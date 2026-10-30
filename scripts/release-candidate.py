#!/usr/bin/env python3
"""What the deploy workflow checks before, and reports after, release-gate.py and deploy-vm.sh.

  config       the repository variables the deploy needs are present and well formed
  check        this run is for the exact current master HEAD of this repository, started by a
               successful push CI run (or by hand on master), and CI passed every required job on
               that same SHA. Exit 0 eligible, 11 stale (a newer commit is master HEAD), 1 refused.
  master-head  print the current master HEAD, for release-gate.py --master-head
  summary      the job summary, from the steps' outcomes and production's own status

GitHub is asked through `gh api` with the job's read-only token. Nothing here reaches GCP.
"""
import argparse
import json
import os
import pathlib
import re
import subprocess
import sys

SHA = re.compile(r"^[0-9a-f]{40}$")
BRANCH = "master"
CI_WORKFLOW = ".github/workflows/hygiene.yml"
# Every job in hygiene.yml, by name. A run that skipped one is not a pass.
REQUIRED_JOBS = (
    "Repository security and privacy",
    "Rust checks with PostgreSQL",
    "Product acceptance journey",
    "Frontend validation",
    "Browser-worker validation",
)
ELIGIBLE, STALE = 0, 11
# Values are checked for shape only and never printed: a malformed one is named, not echoed.
CONFIG = {
    "GCP_PROJECT": r"[a-z][a-z0-9-]{4,28}[a-z0-9]",
    "GCP_INSTANCE": r"[a-z]([-a-z0-9]{0,61}[a-z0-9])?",
    "GCP_ZONE": r"[a-z]+-[a-z]+[0-9]+-[a-z]",
    "GCP_WORKLOAD_IDENTITY_PROVIDER": r"projects/[0-9]+/locations/global/workloadIdentityPools/[a-z0-9-]{4,32}/providers/[a-z0-9-]{4,32}",
    "GCP_DEPLOY_SERVICE_ACCOUNT": r"[a-z][a-z0-9-]{4,28}[a-z0-9]@[a-z][a-z0-9-]{4,28}[a-z0-9]\.iam\.gserviceaccount\.com",
    # Required here, optional for deploy-vm.sh: the job logs in as the service account's own OS Login
    # user, and /opt/opportunity belongs to the owner.
    "REMOTE_RUN_AS": r"[a-z_][a-z0-9_.-]{0,31}",
}


def out(**fields):
    path = os.environ.get("GITHUB_OUTPUT")
    if path:
        with open(path, "a") as f:
            for key, value in fields.items():
                f.write(f"{key}={' '.join(str(value).split())}\n")


def refuse(reason):
    print(f"REFUSED: {reason}", file=sys.stderr)
    out(outcome="REFUSED", reason=reason)
    sys.exit(1)


def gh(path):
    result = subprocess.run(["gh", "api", "-H", "Accept: application/vnd.github+json", path],
                            capture_output=True, text=True)
    if result.returncode != 0:
        refuse(f"GitHub API request failed ({path.split('?')[0]}): {result.stderr.strip()[:200]}")
    try:
        return json.loads(result.stdout)
    except ValueError:
        refuse(f"GitHub API returned something that is not JSON for {path.split('?')[0]}")


def master_head(repo):
    sha = (gh(f"repos/{repo}/git/ref/heads/{BRANCH}").get("object") or {}).get("sha", "")
    if not SHA.match(sha):
        refuse("could not read the current master HEAD")
    return sha


def config(_args):
    problems = []
    for name, shape in CONFIG.items():
        value = os.environ.get(name, "")
        if not value:
            problems.append(f"{name} is not set")
        elif not re.fullmatch(shape, value):
            problems.append(f"{name} is not well formed")
    if problems:
        refuse("deploy configuration is incomplete: " + "; ".join(problems) + " (repository variables, docs/RELEASE_OPERATIONS.md)")
    print("deploy configuration is present and well formed")


def started_for(env, repo):
    """The SHA this run was started for, after checking the event that started it."""
    name = env.get("GITHUB_EVENT_NAME")
    if name == "workflow_run":
        try:
            run = json.loads(pathlib.Path(env["GITHUB_EVENT_PATH"]).read_text())["workflow_run"]
        except (KeyError, OSError, ValueError, TypeError):
            refuse("the workflow_run event payload is unreadable")
        checks = [
            (run.get("event") == "push", "CI ran for a pull request or other event, not a push to master"),
            (run.get("conclusion") == "success", f"CI concluded {run.get('conclusion')!r}, not success"),
            (run.get("head_branch") == BRANCH, "CI ran on another branch"),
            ((run.get("path") or "").split("@")[0] == CI_WORKFLOW, "the workflow that finished is not continuous integration"),
            ((run.get("repository") or {}).get("full_name") == repo, "CI ran in another repository"),
            ((run.get("head_repository") or {}).get("full_name") == repo, "CI ran on a fork"),
        ]
        sha = run.get("head_sha", "")
    elif name == "workflow_dispatch":
        checks = [(env.get("GITHUB_REF") == f"refs/heads/{BRANCH}", "a manual deploy must be started on master")]
        sha = env.get("GITHUB_SHA", "")
    else:
        refuse(f"deploys start from CI completion or a manual run on master, not from {name!r}")
    for ok, reason in checks:
        if not ok:
            refuse(reason)
    return sha


def check(_args):
    env = os.environ
    repo = env.get("GITHUB_REPOSITORY", "")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        refuse("GITHUB_REPOSITORY is missing")
    candidate = env.get("REVIEWED_SHA", "")
    if not SHA.match(candidate):
        refuse("the candidate must be a full 40-character commit SHA")
    if started_for(env, repo) != candidate:
        refuse("the candidate differs from the commit this run was started for")
    head = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    if head != candidate:
        refuse("the checkout is not the candidate commit")

    current = master_head(repo)
    if current != candidate:
        print(f"SKIPPED_STALE_REVISION\nCandidate: {candidate}\nCurrent master: {current}\nNo production changes performed.")
        out(outcome="SKIPPED_STALE_REVISION", master_head=current)
        sys.exit(STALE)

    runs = gh(f"repos/{repo}/actions/workflows/{CI_WORKFLOW.rsplit('/', 1)[1]}/runs"
              f"?head_sha={candidate}&event=push&branch={BRANCH}&status=success&per_page=20").get("workflow_runs") or []
    # The query is a filter, not a guarantee: every field that matters is checked again here.
    green = [r for r in runs if r.get("head_sha") == candidate and r.get("event") == "push"
             and r.get("head_branch") == BRANCH and r.get("status") == "completed"
             and r.get("conclusion") == "success" and (r.get("path") or "").split("@")[0] == CI_WORKFLOW
             and (r.get("repository") or {}).get("full_name") == repo
             and (r.get("head_repository") or {}).get("full_name") == repo]
    if not green:
        refuse(f"continuous integration has not passed on a push of {candidate} to master")
    passed = None
    for run in sorted(green, key=lambda r: r.get("id", 0), reverse=True):
        jobs = gh(f"repos/{repo}/actions/runs/{run['id']}/jobs?filter=latest&per_page=100").get("jobs") or []
        conclusions = {j.get("name"): j.get("conclusion") for j in jobs}
        if all(conclusions.get(name) == "success" for name in REQUIRED_JOBS) and all(c == "success" for c in conclusions.values()):
            passed = run
            break
    if not passed:
        refuse("no CI run on this SHA passed every required job: " + ", ".join(REQUIRED_JOBS))
    print(f"ELIGIBLE: {candidate} is master HEAD and CI run {passed['id']} passed every required job")
    out(outcome="ELIGIBLE", ci_run=passed["id"])


def read_fields(*paths):
    fields = {}
    for path in paths:
        if path and pathlib.Path(path).is_file():
            for line in pathlib.Path(path).read_text().splitlines():
                key, sep, value = line.partition("=")
                if sep and re.fullmatch(r"[A-Z_]+", key):
                    fields[key] = value.strip()
    return fields


def production_status(path):
    status = {"revision": None, "migrations": None, "mode": None}
    if path and pathlib.Path(path).is_file():
        text = pathlib.Path(path).read_text()
        if "MIGRATIONS_END" in text and "UNRESOLVED" not in text:
            versions = [int(m) for m in re.findall(r"^MIGRATION=(\d+):", text, re.M)]
            status["migrations"] = f"{max(versions):04d}" if versions else "none"
            found = re.search(r"^DEPLOYED_REVISION=([0-9a-f]{40})$", text, re.M)
            status["revision"] = found.group(1) if found else None
            mode = re.search(r"^Actual access mode: (\S+)$", text, re.M)
            status["mode"] = mode.group(1) if mode else None
    return status


def decide(env, fields):
    """(result, failed stage, production changed?) from the steps' outcomes, most specific first."""
    steps = {name: env.get(f"STEP_{name.upper()}", "") for name in ("config", "candidate", "auth", "gate", "build", "preflight", "deploy")}
    if env.get("CANDIDATE_RESULT") == "SKIPPED_STALE_REVISION":
        return "SKIPPED_STALE_REVISION", None, "no"
    for name, stage in (("config", "configuration"), ("candidate", "candidate verification (master HEAD and CI)"), ("auth", "GCP authentication")):
        if steps[name] in ("failure", "cancelled"):
            return ("REFUSED" if name == "candidate" else "FAILED"), stage, "no"
    outcome = fields.get("OUTCOME")
    if steps["deploy"] in ("success", "failure", "cancelled") or outcome in ("DEPLOYED", "ROLLED_BACK", "RECOVERY_REQUIRED"):
        if outcome in ("DEPLOYED", "ROLLED_BACK", "RECOVERY_REQUIRED", "ALREADY_DEPLOYED", "SKIPPED_STALE_REVISION", "MIGRATION_REQUIRED", "REFUSED"):
            changed = {"DEPLOYED": "yes", "ROLLED_BACK": "rolled back", "RECOVERY_REQUIRED": "unknown"}.get(outcome, fields.get("CHANGED", "no"))
            return outcome, fields.get("STAGE"), changed
        if outcome == "FAILED":
            return "FAILED", fields.get("STAGE", "deployment"), fields.get("CHANGED", "unknown")
        # The deploy step ran and left no outcome: whatever it did is unknown, never a success.
        return "RECOVERY_REQUIRED", "deployment (no outcome recorded)", "unknown"
    if outcome in ("ALREADY_DEPLOYED", "SKIPPED_STALE_REVISION", "MIGRATION_REQUIRED", "REFUSED", "RECOVERY_REQUIRED"):
        return outcome, "release gate" if outcome in ("REFUSED", "RECOVERY_REQUIRED") else None, "no"
    for name, stage in (("gate", "release gate"), ("build", "image build"), ("preflight", "preflight")):
        if steps[name] in ("failure", "cancelled"):
            return "FAILED", stage, "no"
    if all(v in ("skipped", "") for v in steps.values()):
        return "NOT_RUN", None, "no"
    return "FAILED", "unknown (no step reported an outcome)", "unknown"


def summary(args):
    env = os.environ
    fields = read_fields(args.gate_outcome, args.deploy_outcome)
    status = production_status(args.status)
    candidate = env.get("REVIEWED_SHA", "")
    result, stage, changed = decide(env, fields)
    if result == "DEPLOYED" and status["revision"] != candidate:
        # A reported success is believed only when production says the same thing afterwards.
        result, stage, changed = "FAILED", "post-deploy verification (production does not report the candidate)", "unknown"
    title = {"DEPLOYED": "SUCCESS"}.get(result, result)
    lines = ["## Opportunity Platform — production deployment", "", f"**Result: {title}**", "",
             "| | |", "| --- | --- |", f"| Candidate SHA | `{candidate or 'unknown'}` |"]
    if fields.get("PREVIOUS_REVISION"):
        lines.append(f"| Previously deployed SHA | `{fields['PREVIOUS_REVISION']}` |")
    lines.append(f"| Running SHA | `{status['revision']}` |" if status["revision"] else "| Running SHA | not read |")
    if status["migrations"]:
        lines.append(f"| Database migrations | {status['migrations']} |")
    if result == "DEPLOYED":
        lines += [f"| Release type | {fields.get('RELEASE_TYPE', 'code-only')} |",
                  "| Image verified | yes (archive identity here, loaded identity on the VM) |",
                  "| Transfer checksum verified | yes (on the VM, before docker load) |",
                  "| Application ready | yes (/ready and vm-check.sh) |",
                  f"| Authentication enforced | yes (ACCESS_MODE={status['mode'] or 'unknown'}, private API refuses anonymous callers) |",
                  f"| Rollback image retained | yes (`{fields.get('ROLLBACK_REF', 'see .release-history')}`) |"]
    else:
        lines.append(f"| Production changed | {changed} |")
        if stage:
            lines.append(f"| Failed or stopping stage | {stage} |")
        if fields.get("ROLLBACK"):
            lines.append(f"| Rollback | {fields['ROLLBACK']} |")
        lines.append(f"| Recovery required | {'yes' if result == 'RECOVERY_REQUIRED' else 'no'} |")
    if fields.get("JOURNAL"):
        lines.append(f"| Release journal | `{fields['JOURNAL']}` |")
    lines.append("")
    newer = env.get("MASTER_HEAD")
    explain = {
        "SKIPPED_STALE_REVISION": f"The candidate is no longer master HEAD{f' (now `{newer}`)' if newer else ''}. "
                                  "No deployment was attempted; the newer commit's own run deploys it.",
        "ALREADY_DEPLOYED": "Production already runs the candidate. Nothing was transferred or restarted.",
        "MIGRATION_REQUIRED": "Automatic deployment did not proceed: the candidate adds migrations.\n\nPending migrations: "
                              + ", ".join(f"`{m}`" for m in fields.get("PENDING_MIGRATIONS", "").split(",") if m)
                              + "\n\nAction: the owner completes the verified manual migration rollout (docs/RELEASE_OPERATIONS.md).",
        "ROLLED_BACK": "The new release failed verification. The previous image and configuration were restored and passed the complete checks again. No database restore was needed: this release changed no schema.",
        "RECOVERY_REQUIRED": "The outcome on the VM is not proven. Do not re-run the deploy. Run `deploy-vm.sh --status`, then `deploy-vm.sh --reconcile` from the owner's machine.",
        "REFUSED": f"Refused before production was changed: {fields.get('REASON') or env.get('CANDIDATE_REASON') or 'see the step log'}.",
        "NOT_RUN": "Deployment is disabled (DEPLOY_ENABLED is not true) or this run was not eligible.",
    }
    if result in explain:
        lines += [explain[result], ""]
    elif result == "FAILED":
        lines += [f"Failed at {stage}. Production changed: {changed}. See that step's log.", ""]
    print("\n".join(lines))
    return 0 if result in ("DEPLOYED", "ALREADY_DEPLOYED", "SKIPPED_STALE_REVISION", "NOT_RUN") else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("config")
    commands.add_parser("check")
    commands.add_parser("master-head")
    report = commands.add_parser("summary")
    report.add_argument("--status")
    report.add_argument("--gate-outcome")
    report.add_argument("--deploy-outcome")
    args = parser.parse_args()
    if args.command == "config":
        config(args)
    elif args.command == "check":
        check(args)
    elif args.command == "master-head":
        print(master_head(os.environ.get("GITHUB_REPOSITORY", "")))
    else:
        sys.exit(summary(args))


if __name__ == "__main__":
    main()
