#!/usr/bin/env python3
"""End-to-end backup and restore with real tools and no real data.

A disposable PostgreSQL container stands in for the VM's database (migrated by the real binary, with
one synthetic CV row), a temporary directory for its private volume, and a fresh throwaway OpenPGP
key for the owner's. deploy-vm.sh --fetch-backup runs over a loopback "ssh" that executes locally,
then restore-rehearsal.sh restores the result into a second disposable container. Also checks that a
truncated archive, a missing private archive and the wrong key are all refused.

Needs docker, gpg and target/debug/opportunity (cargo build --bin opportunity).
"""
import hashlib
import json
import os
import pathlib
import shutil
import socket
import subprocess
import sys
import tempfile
import time

BASE = pathlib.Path(__file__).resolve().parents[1]
CONTAINER = f"opportunity-roundtrip-{os.getpid()}"
REVISION = "c" * 40
CV = b"Synthetic Person\nFirmware engineer\n"

FAKE_DOCKER = r'''#!/usr/bin/env python3
import os, subprocess, sys
a = sys.argv[1:]; real = os.environ["REAL_DOCKER"]; db = os.environ["ROUNDTRIP_DB"]
if a[:1] == ["info"]: sys.exit(0)
if a[:1] == ["compose"]:
    if "exec" in a:
        rest = a[a.index("exec") + 1:]
        rest = [v for v in rest if v != "-T"]
        if rest[0] == "db":
            sys.exit(subprocess.run([real, "exec", "-i", db] + rest[1:]).returncode)
        if rest[0] == "app" and rest[1] == "tar":
            sys.exit(subprocess.run(["tar", "-czf", "-", "-C", os.environ["ROUNDTRIP_APP"], "private"]).returncode)
    if "ps" in a: print("app"); sys.exit(0)
if a[:1] == ["inspect"]:
    print("ACCESS_MODE=native" if "Env" in a[-1] else "sha256:" + "d" * 64); sys.exit(0)
if a[:2] == ["image", "inspect"]: print(os.environ["ROUNDTRIP_REVISION"]); sys.exit(0)
sys.exit(f"unexpected docker call: {a}")
'''


def run(*args, check=True, **kwargs):
    result = subprocess.run(args, capture_output=True, text=True, **kwargs)
    if check and result.returncode != 0:
        raise SystemExit(f"{' '.join(map(str, args))} failed:\n{result.stdout}\n{result.stderr}")
    return result


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def new_key(home):
    home.mkdir(mode=0o700)
    env = dict(os.environ, GNUPGHOME=str(home))
    run("gpg", "--batch", "--passphrase", "", "--quick-generate-key", "Backup Roundtrip <roundtrip@example.test>", "default", "default", "never", env=env)
    listing = run("gpg", "--batch", "--with-colons", "--list-keys", env=env).stdout
    return env, next(line.split(":")[9] for line in listing.splitlines() if line.startswith("fpr:"))


def main():
    real_docker = shutil.which("docker") or sys.exit("docker is required")
    binary = BASE / "target/debug/opportunity"
    binary.exists() or sys.exit("build the server first: cargo build --bin opportunity")
    # A short path: gpg-agent's socket lives in GNUPGHOME and Unix socket paths are short.
    work = pathlib.Path(tempfile.mkdtemp(prefix="obr", dir="/tmp"))
    port = free_port()
    try:
        run(real_docker, "run", "-d", "--rm", "--name", CONTAINER, "-e", "POSTGRES_USER=roundtrip", "-e", "POSTGRES_PASSWORD=roundtrip-only",
            "-e", "POSTGRES_DB=roundtrip", "-p", f"127.0.0.1:{port}:5432", "postgres:16-alpine")
        for _ in range(60):
            if run(real_docker, "exec", CONTAINER, "pg_isready", "-U", "roundtrip", "-d", "roundtrip", check=False).returncode == 0:
                break
            time.sleep(1)
        time.sleep(1)
        url = f"postgres://roundtrip:roundtrip-only@127.0.0.1:{port}/roundtrip"
        run(str(binary), "migrate", env=dict(os.environ, DATABASE_URL=url))
        app = work / "app"; (app / "private/documents").mkdir(parents=True)
        (app / "private/documents/synthetic.txt").write_bytes(CV)
        run(real_docker, "exec", CONTAINER, "psql", "-U", "roundtrip", "-d", "roundtrip", "-v", "ON_ERROR_STOP=1", "-c",
            "insert into documents (owner_id, kind, label, file_path, content_hash, byte_size, media_type) select id, 'cv', 'synthetic.txt', "
            f"'private/documents/synthetic.txt', '{hashlib.sha256(CV).hexdigest()}', {len(CV)}, 'text/plain' from owners limit 1")

        bin_dir = work / "bin"; bin_dir.mkdir()
        (bin_dir / "docker").write_text(FAKE_DOCKER); (bin_dir / "docker").chmod(0o755)
        (bin_dir / "ssh").write_text('#!/bin/sh\nshift\nexec bash -c "$1"\n'); (bin_dir / "ssh").chmod(0o755)
        vm = work / "vm"; vm.mkdir(); (vm / ".env.prod").write_text("APP_IMAGE=fixture\n")
        owner_env, fingerprint = new_key(work / "k1")
        out = work / "backups"
        fetch_env = dict(owner_env, PATH=f"{bin_dir}{os.pathsep}{os.environ['PATH']}", REAL_DOCKER=real_docker, ROUNDTRIP_DB=CONTAINER,
                         ROUNDTRIP_APP=str(app), ROUNDTRIP_REVISION=REVISION, TRANSPORT="ssh", SSH_HOST="loopback.invalid",
                         REMOTE_DIR=str(vm), BACKUP_OUT_DIR=str(out), BACKUP_ENCRYPT_TO=fingerprint)
        run("bash", "scripts/deploy-vm.sh", "--fetch-backup", cwd=BASE, env=fetch_env)
        [backup] = [p for p in out.iterdir() if p.name.startswith("opportunity-")]
        manifest = json.loads((backup / "manifest.json").read_text())
        assert manifest["production_revision"] == REVISION, manifest
        assert len(manifest["migrations"]) == len(list((BASE / "migrations").glob("*.sql"))), manifest["migrations"]
        assert not any(p.name.startswith(".incomplete") for p in out.iterdir())

        run("bash", "scripts/restore-rehearsal.sh", str(backup), cwd=BASE, env=owner_env)
        verified = json.loads((backup / "restore-verified.json").read_text())
        assert verified["ok"] and verified["documents_checked"] == 1 and verified["documents_missing"] == 0, verified
        assert verified["tables"] > 10, verified
        print(f"round trip: {verified['tables']} tables, {verified['migrations']} migrations, {verified['documents_checked']} CV file restored")

        def refused(name, mutate, env):
            copy = work / name; shutil.copytree(backup, copy); (copy / "restore-verified.json").unlink()
            mutate(copy)
            result = run("bash", "scripts/restore-rehearsal.sh", str(copy), cwd=BASE, env=env, check=False)
            assert result.returncode != 0 and not (copy / "restore-verified.json").exists(), (name, result.stdout, result.stderr)
            print(f"refused as expected: {name}")
        refused("truncated", lambda d: (d / "database.dump.gpg").write_bytes((d / "database.dump.gpg").read_bytes()[:-100]), owner_env)
        refused("missing-private-archive", lambda d: (d / "private.tar.gz.gpg").unlink(), owner_env)
        other_env, _ = new_key(work / "k2")
        refused("wrong-key", lambda d: None, other_env)
        print("PASS backup round trip")
    finally:
        subprocess.run([real_docker, "rm", "-f", CONTAINER], capture_output=True)
        for home in ("k1", "k2"):
            subprocess.run(["gpgconf", "--kill", "gpg-agent"], env=dict(os.environ, GNUPGHOME=str(work / home)), capture_output=True)
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
