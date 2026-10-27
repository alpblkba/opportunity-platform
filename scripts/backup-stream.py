#!/usr/bin/env python3
"""Pieces of the encrypted backup that are easier to get right in Python than in a shell pipe.

check-recipient FPR          the key must be named by its full fingerprint, exist exactly once, be
                             valid and able to encrypt; a name or an email address is refused
encrypt --expect KIND ...    stdin -> gpg; refuses a stream that is empty or not a pg_dump/gzip
                             stream, records the plaintext SHA-256 and size beside the output
manifest DIR FPR             writes manifest.json and per-file .sha256 once both streams succeeded
verify DIR                   checks the manifest and every ciphertext checksum (no key needed)
attest DIR WORK              after restore-rehearsal.sh restored DIR into WORK: plaintext hashes,
                             migration history and every referenced CV file, then restore-verified.json
"""
import datetime
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import tarfile

MAGIC = {"pgdump": b"PGDMP", "gzip": b"\x1f\x8b"}
FINGERPRINT = re.compile(r"^(?:[0-9A-F]{40}|[0-9A-F]{64})$")
STREAMS = {"database.dump.gpg": "pgdump", "private.tar.gz.gpg": "gzip"}


def fail(message):
    print(f"backup: {message}", file=sys.stderr)
    sys.exit(1)


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def check_recipient(value):
    fingerprint = value.replace(" ", "").upper()
    if not FINGERPRINT.match(fingerprint):
        fail("BACKUP_ENCRYPT_TO must be the key's full fingerprint (gpg --fingerprint), not a name, email or short id")
    listing = subprocess.run(["gpg", "--batch", "--with-colons", "--fixed-list-mode", "--list-keys", "--", fingerprint],
                             capture_output=True, text=True)
    if listing.returncode != 0:
        fail(f"no public key {fingerprint} in the keyring; import the owner's public key first")
    keys, current = [], None
    for line in listing.stdout.splitlines():
        fields = line.split(":")
        if fields[0] == "pub":
            current = {"validity": fields[1], "capabilities": fields[11], "fingerprint": None}
            keys.append(current)
        elif fields[0] == "fpr" and current is not None and current["fingerprint"] is None:
            current["fingerprint"] = fields[9]
    if len(keys) != 1 or keys[0]["fingerprint"] != fingerprint:
        fail(f"{fingerprint} does not name exactly one primary key")
    if keys[0]["validity"] in ("r", "e", "d", "i", "n"):
        fail(f"{fingerprint} is revoked, expired, disabled or invalid")
    if "E" not in keys[0]["capabilities"]:
        fail(f"{fingerprint} has no usable encryption key")
    return fingerprint


def encrypt(expect, recipient, output):
    os.umask(0o077)
    output = pathlib.Path(output)
    gpg = subprocess.Popen(["gpg", "--batch", "--yes", "--no-tty", "--quiet", "--trust-model", "always",
                            "--encrypt", "--recipient", recipient, "--output", str(output)],
                           stdin=subprocess.PIPE)
    # Trust is anchored on the fingerprint check-recipient pinned, not on gpg's web of trust.
    digest, size, head = hashlib.sha256(), 0, b""
    try:
        for block in iter(lambda: sys.stdin.buffer.read(1 << 16), b""):
            if len(head) < 8:
                head += block[:8]
            digest.update(block)
            size += len(block)
            gpg.stdin.write(block)
    except BrokenPipeError:
        pass
    finally:
        gpg.stdin.close()
    if gpg.wait() != 0:
        fail(f"gpg could not encrypt {output.name}")
    if size == 0:
        fail(f"{output.name}: the remote stream was empty")
    if not head.startswith(MAGIC[expect]):
        fail(f"{output.name}: the remote stream is not a {expect} stream")
    meta = {"plaintext_sha256": digest.hexdigest(), "plaintext_bytes": size, "kind": expect}
    pathlib.Path(str(output) + ".meta.json").write_text(json.dumps(meta))


def production_state(path):
    revision, image, migrations = None, None, {}
    text = path.read_text() if path.exists() else ""
    complete = "MIGRATIONS_END" in text and "UNRESOLVED" not in text
    for line in text.splitlines():
        if line.startswith("DEPLOYED_REVISION="):
            revision = line.split("=", 1)[1].strip() or None
        elif line.startswith("DEPLOYED_IMAGE="):
            image = line.split("=", 1)[1].strip() or None
        elif line.startswith("MIGRATION="):
            version, _, checksum = line.split("=", 1)[1].strip().partition(":")
            migrations[version] = checksum
    return (revision, image, migrations) if complete else (None, None, {})


def manifest(directory, recipient):
    directory = pathlib.Path(directory)
    status = directory / "production-status.txt"
    revision, image, migrations = production_state(status)
    files = {}
    for name, kind in STREAMS.items():
        path = directory / name
        meta_path = pathlib.Path(str(path) + ".meta.json")
        if not path.is_file() or not meta_path.is_file():
            fail(f"{name} is missing; the backup is incomplete")
        meta = json.loads(meta_path.read_text())
        if meta["kind"] != kind:
            fail(f"{name} holds a {meta['kind']} stream")
        files[name] = {"sha256": sha256(path), "bytes": path.stat().st_size,
                       "plaintext_sha256": meta["plaintext_sha256"], "plaintext_bytes": meta["plaintext_bytes"]}
        # restore.sh reads the same per-file checksum format it always has.
        pathlib.Path(str(path) + ".sha256").write_text(f"{files[name]['sha256']}  {name}\n")
        meta_path.unlink()
    status.unlink(missing_ok=True)
    document = {
        "format": 1,
        "created_at": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
        "production_revision": revision,
        "production_image": image,
        "migrations": migrations,
        "recipient": recipient,
        "files": files,
        "consistency": "pg_dump snapshot first, then the private tree; uploads only add files, so every referenced CV exists",
    }
    (directory / "manifest.json").write_text(json.dumps(document, indent=2) + "\n")


def load_manifest(directory):
    try:
        data = (directory / "manifest.json").read_bytes()
        return data, json.loads(data)
    except (OSError, ValueError) as error:
        fail(f"no readable manifest in {directory}: {error}")


def verify(directory):
    directory = pathlib.Path(directory)
    _, document = load_manifest(directory)
    if set(document.get("files", {})) != set(STREAMS):
        fail("the manifest does not list both the database and the private tree")
    for name, entry in document["files"].items():
        path = directory / name
        if not path.is_file():
            fail(f"{name} is missing")
        if sha256(path) != entry["sha256"]:
            fail(f"{name} does not match its manifest checksum (truncated or altered)")
    print(f"verified checksums of {', '.join(sorted(document['files']))}")


def attest(directory, work):
    directory, work = pathlib.Path(directory), pathlib.Path(work)
    manifest_bytes, document = load_manifest(directory)
    plain = {"database.dump.gpg": work / "database.dump", "private.tar.gz.gpg": work / "private.tar.gz"}
    for name, path in plain.items():
        if sha256(path) != document["files"][name]["plaintext_sha256"]:
            fail(f"{name} decrypted to different bytes than were backed up")
    restored = {}
    for line in (work / "migrations.txt").read_text().splitlines():
        version, _, checksum = line.partition(":")
        restored[version] = checksum
    if document.get("migrations") and restored != document["migrations"]:
        fail("the restored migration history differs from the one recorded at backup time")
    tables = int((work / "tables.txt").read_text().strip() or 0)
    if tables == 0:
        fail("the restored database has no tables")
    archived = {}
    with tarfile.open(plain["private.tar.gz.gpg"], "r:gz") as archive:
        for member in archive:
            if member.isfile():
                handle = archive.extractfile(member)
                archived[member.name.lstrip("./")] = hashlib.sha256(handle.read()).hexdigest()
    missing, checked = [], 0
    for line in (work / "documents.tsv").read_text().splitlines():
        if not line.strip():
            continue
        file_path, content_hash = line.split("\t")
        relative = file_path[len("/app/"):] if file_path.startswith("/app/") else file_path.lstrip("./")
        checked += 1
        if archived.get(relative) != content_hash:
            missing.append(relative)
    for path in missing:
        print(f"missing or altered in the private archive: {path}", file=sys.stderr)
    result = {"ok": not missing, "verified_at": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
              "manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(), "tables": tables,
              "migrations": len(restored), "documents_checked": checked, "documents_missing": len(missing),
              "private_files": len(archived)}
    if missing:
        fail(f"{len(missing)} referenced CV file(s) are not in the backup")
    (directory / "restore-verified.json").write_text(json.dumps(result, indent=2) + "\n")
    print(f"restore verified: {tables} tables, {len(restored)} migrations, {checked} CV files, {len(archived)} private files")


def main():
    args = sys.argv[1:]
    if args[:1] == ["check-recipient"] and len(args) == 2:
        print(f"encrypting to {check_recipient(args[1])}")
    elif args[:1] == ["encrypt"] and len(args) == 7 and args[1] == "--expect" and args[3] == "--recipient" and args[5] == "--output" and args[2] in MAGIC:
        encrypt(args[2], check_recipient(args[4]), args[6])
    elif args[:1] == ["manifest"] and len(args) == 3:
        manifest(args[1], check_recipient(args[2]))
    elif args[:1] == ["verify"] and len(args) == 2:
        verify(args[1])
    elif args[:1] == ["attest"] and len(args) == 3:
        attest(args[1], args[2])
    else:
        fail(__doc__)


if __name__ == "__main__":
    main()
