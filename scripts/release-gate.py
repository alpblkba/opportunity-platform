#!/usr/bin/env python3
"""Decide whether a candidate release may replace what production actually runs.

Production state comes from `vm-release.sh status` on the VM, never from GitHub Actions history:
the running image's revision label and the database's own SQLx migration checksums. Exit 0 means
deploy, 10 means the candidate is already running, anything else means refuse.
"""
import argparse
import datetime
import hashlib
import json
import pathlib
import re
import subprocess
import sys

ALREADY_DEPLOYED = 10
SHA = re.compile(r"^[0-9a-f]{40}$")
RECOVERY_MAX_AGE = datetime.timedelta(hours=24)


def refuse(reason):
    print(f"REFUSE: {reason}")
    sys.exit(1)


def git(repo, *args):
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)


def production_state(text):
    if "UNRESOLVED" in text:
        refuse("the VM has an unresolved release journal; run deploy-vm.sh --reconcile first")
    revision, applied = None, {}
    for line in text.splitlines():
        if line.startswith("DEPLOYED_REVISION="):
            revision = line.split("=", 1)[1].strip()
        elif line.startswith("MIGRATION="):
            version, _, checksum = line.split("=", 1)[1].strip().partition(":")
            if not version.isdigit() or not re.fullmatch(r"[0-9a-f]{96}", checksum):
                refuse(f"unreadable migration record from production: {line!r}")
            applied[int(version)] = checksum
    if "MIGRATIONS_END" not in text:
        refuse("production did not report its migration history (release in progress, or the database is unreachable)")
    if not revision or not SHA.match(revision):
        refuse(f"production did not report a revision it is running (got {revision!r}); nothing to compare against")
    return revision, applied


def candidate_migrations(repo):
    found = {}
    for path in sorted((repo / "migrations").glob("*.sql")):
        match = re.match(r"^(\d+)_", path.name)
        if not match:
            refuse(f"{path.name} has no SQLx version prefix")
        version = int(match.group(1))
        if version in found:
            refuse(f"two migrations share version {version}")
        # SQLx records SHA-384 of the file bytes; a changed byte makes the new binary refuse to start.
        found[version] = (path.name, hashlib.sha384(path.read_bytes()).hexdigest())
    return found


def check_recovery_point(directory, revision, applied):
    directory = pathlib.Path(directory)
    try:
        manifest_bytes = (directory / "manifest.json").read_bytes()
        manifest = json.loads(manifest_bytes)
    except (OSError, ValueError) as error:
        refuse(f"no readable backup manifest in {directory}: {error}")
    try:
        verified = json.loads((directory / "restore-verified.json").read_text())
    except (OSError, ValueError):
        verified = {}
    if manifest.get("production_revision") != revision:
        refuse("the backup was taken from a different production revision than the one running now")
    if {int(k): v for k, v in manifest.get("migrations", {}).items()} != applied:
        refuse("the backup's migration history differs from production's; take a new backup")
    try:
        created = datetime.datetime.fromisoformat(manifest["created_at"].replace("Z", "+00:00"))
    except (KeyError, ValueError):
        refuse("the backup manifest has no valid created_at")
    if datetime.datetime.now(datetime.timezone.utc) - created > RECOVERY_MAX_AGE:
        refuse("the backup is older than 24 hours; take a fresh one right before a schema change")
    for name, expected in manifest.get("files", {}).items():
        try:
            actual = hashlib.sha256((directory / name).read_bytes()).hexdigest()
        except OSError:
            refuse(f"backup file {name} is missing")
        if actual != expected["sha256"]:
            refuse(f"backup file {name} does not match its manifest")
    if not manifest.get("files"):
        refuse("the backup manifest lists no files")
    if (verified.get("ok") is not True or verified.get("documents_missing") != 0
            or verified.get("manifest_sha256") != hashlib.sha256(manifest_bytes).hexdigest()):
        refuse("this backup has not passed restore-rehearsal.sh; restore it into a throwaway database first")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--status", required=True, help="output of vm-release.sh status")
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--trusted-ref", default="origin/master")
    parser.add_argument("--rollout-manifest", help="verified backup directory; required for schema changes")
    parser.add_argument("--repo", default=".")
    args = parser.parse_args()
    repo = pathlib.Path(args.repo)

    candidate = args.candidate
    if not SHA.match(candidate):
        refuse("the candidate must be a full 40-character commit SHA")
    if git(repo, "rev-parse", "HEAD").stdout.strip() != candidate:
        refuse("the checkout is not the candidate commit")
    if git(repo, "status", "--porcelain", "--", "migrations").stdout.strip():
        refuse("migrations/ has uncommitted changes")
    revision, applied = production_state(pathlib.Path(args.status).read_text())
    print(f"production runs {revision} with {len(applied)} applied migrations; candidate {candidate}")

    if git(repo, "merge-base", "--is-ancestor", candidate, args.trusted_ref).returncode != 0:
        refuse(f"the candidate is not on {args.trusted_ref}")
    if revision == candidate:
        print("ALREADY_DEPLOYED: production already runs the candidate")
        sys.exit(ALREADY_DEPLOYED)
    if git(repo, "cat-file", "-e", revision + "^{commit}").returncode != 0:
        refuse("production runs a revision this repository does not know (an out-of-band build?); investigate before deploying")
    if git(repo, "merge-base", "--is-ancestor", revision, candidate).returncode != 0:
        refuse("the candidate does not descend from production's revision; deploying it would silently drop changes")

    migrations = candidate_migrations(repo)
    for version, checksum in sorted(applied.items()):
        if version not in migrations:
            refuse(f"production applied migration {version}, which the candidate lacks; the candidate would refuse to start")
        if migrations[version][1] != checksum:
            refuse(f"{migrations[version][0]} changed after production applied it; SQLx would refuse to start")
    pending = sorted(set(migrations) - set(applied))
    if applied and pending and pending[0] < max(applied):
        refuse(f"migration {pending[0]} would apply out of order after {max(applied)}")
    if not pending:
        print("DEPLOY: code-only release; the schema is unchanged")
        return
    names = ", ".join(migrations[v][0] for v in pending)
    if not args.rollout_manifest:
        refuse(f"schema change ({names}). An image rollback cannot undo it, so it is never automatic: "
               "take a backup, rehearse its restore, then deploy-vm.sh --migration-rollout <backup dir> <archive>")
    check_recovery_point(args.rollout_manifest, revision, applied)
    print(f"DEPLOY: migration rollout of {names} with a verified recovery point")


if __name__ == "__main__":
    main()
