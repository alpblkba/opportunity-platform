#!/usr/bin/env python3
"""Check Git's index (not only working tree) for local or sensitive paths."""
from __future__ import annotations
import pathlib
import subprocess
import sys

root = pathlib.Path(__file__).resolve().parents[1]
ignore = (root / ".gitignore").read_text()
required = [
    "/private/",
    "*.pdf",
    "*.docx",
    "!/tests/fixtures/**/*.pdf",
    "!/tests/fixtures/**/*.docx",
    "!**/tests/fixtures/**/*.pdf",
    "!**/tests/fixtures/**/*.docx",
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
paths = [p.decode("utf-8", errors="replace") for p in result.stdout.split(b"\0") if p]
unsafe = []
for name in paths:
    path = pathlib.PurePosixPath(name)
    name_lower = path.name.lower()
    if path.parts[0] == "private":
        unsafe.append((name,"private data path"))
        continue
    if name_lower == ".env" or (name_lower.startswith(".env.") and name_lower != ".env.example"):
        unsafe.append((name,"environment secret path"))
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
