#!/usr/bin/env python3
"""Check Git's index (not only working tree) for local or sensitive paths."""
from __future__ import annotations
import os
import pathlib
import subprocess
import sys
from urllib.parse import parse_qsl, unquote, urlsplit

root = pathlib.Path(__file__).resolve().parents[1]
ignore = (root / ".gitignore").read_text()
required = [
    "/private/",
    "**/private/",
    "*.pdf",
    "*.docx",
    "!/tests/fixtures/**/*.pdf",
    "!/tests/fixtures/**/*.docx",
    "!**/tests/fixtures/**/*.pdf",
    "!**/tests/fixtures/**/*.docx",
    "/google-cloud-sdk/",
    "gha-creds-*.json",
]
missing = [pattern for pattern in required if pattern not in ignore.splitlines()]
if missing:
    sys.exit("FAIL: required ignore/fixture override missing: " + repr(missing))
# Fresh clones do not contain an ignored private/ tree; that's expected.
if not (root / ".git").exists():
    print("PASS: ignore policy present (Git index unavailable before git init)")
    sys.exit(0)
try:
    result = subprocess.run(["git", "ls-files", "-z", "--cached"], cwd=root, capture_output=True, check=True)
except (OSError, subprocess.CalledProcessError) as exc:
    sys.exit(f"FAIL: cannot inspect Git index: {exc}")
paths = [os.fsdecode(p) for p in result.stdout.split(b"\0") if p]
unsafe = []
for name in paths:
    path = pathlib.PurePosixPath(name)
    name_lower = path.name.lower()
    # At any depth: a worker that writes screenshots into its own private/ directory is holding the
    # same kind of thing as the one at the root.
    if "private" in path.parts:
        unsafe.append((name, "private data path"))
        continue
    # A sanitized .env*.example is the documented way to publish what the deployment needs, and
    # .gitignore un-ignores it on purpose. The guard used to allow .env.example alone and would have
    # failed the first commit that included .env.prod.example. Being named .example is not enough,
    # though: every secret-ish key in one has to be empty or an obvious placeholder.
    if name_lower == ".env" or name_lower.startswith(".env."):
        if not name_lower.endswith(".example"):
            unsafe.append((name, "environment secret path"))
            continue
        try:
            # Validate the blob that would be committed, even when the working copy differs or is absent.
            blob = subprocess.run(["git", "show", f":{name}"], cwd=root, capture_output=True, check=True)
            content = blob.stdout.decode("utf-8", errors="replace")
        except (OSError, subprocess.CalledProcessError):
            unsafe.append((name, "cannot read the Git index blob, so its contents cannot be cleared"))
            continue
        for line in content.splitlines():
            key, _, value = line.partition("=")
            key, value = key.strip(), value.strip()
            if not _ or key.startswith("#"):
                continue
            if any(word in key.upper() for word in ("PASSWORD", "TOKEN", "SECRET", "_KEY", "CREDENTIAL")):
                if value and not value.startswith("REPLACE_WITH"):
                    unsafe.append((name, f"{key} has a value; an example file must leave secrets empty or REPLACE_WITH_*"))
            if key.upper().removeprefix("EXPORT ").endswith("DATABASE_URL") and value and not value.startswith("REPLACE_WITH"):
                url = value.strip("\"'")
                # This exact public loopback default is already shipped in .env.example.
                if url == "postgres://opportunity:opportunity_local_dev@127.0.0.1:5432/opportunity":
                    continue
                try:
                    parsed = urlsplit(url)
                    credentials = [parsed.password or ""]
                    credentials.extend(v for k, v in parse_qsl(parsed.query) if any(
                        word in k.upper() for word in ("PASSWORD", "TOKEN", "SECRET", "KEY", "CREDENTIAL")
                    ))
                    if parsed.scheme not in ("postgres", "postgresql") or any(
                        v and not unquote(v).startswith("REPLACE_WITH") for v in credentials
                    ):
                        unsafe.append((name, f"{key} must use a PostgreSQL URL with empty or REPLACE_WITH_* credentials"))
                except ValueError:
                    unsafe.append((name, f"{key} is not a valid example PostgreSQL URL"))
        continue
    if name_lower.startswith("gha-creds-") and name_lower.endswith(".json"):
        unsafe.append((name, "Google Cloud credential written by google-github-actions/auth"))
        continue
    if name_lower.endswith((".sqlite", ".sqlite3", ".db", ".dump", ".pem", ".key", ".p12", ".pfx", ".pages", ".rtf")):
        unsafe.append((name,"sensitive binary/file type"))
        continue
    if name_lower.endswith((".pdf", ".docx")):
        parts = path.parts
        in_fixtures = any(
            parts[i] == "tests" and parts[i + 1] == "fixtures"
            for i in range(len(parts) - 1)
        )
        if not in_fixtures:
            unsafe.append((name, "CV/document outside synthetic fixtures"))
        elif not name_lower.startswith(("synthetic-", "sanitized-")):
            unsafe.append((name, "fixture document must have synthetic-/sanitized- prefix"))
if unsafe:
    for name, reason in unsafe:
        print(f"FAIL: {name}: {reason}")
    sys.exit(1)
print(f"PASS: Git index privacy paths OK ({len(paths)} paths)")
