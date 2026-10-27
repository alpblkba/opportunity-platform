#!/usr/bin/env bash
# Compile the worker and reject missing tests or a successful runner that executed nothing.
set -euo pipefail

cd "$(dirname "$0")/../apps/browser-worker"
unset BROWSER_WORKER_ALLOW_EXTERNAL

npm run typecheck
npm run build

shopt -s nullglob
tests=(dist/*.test.js)
if (( ${#tests[@]} == 0 )); then
  echo "No compiled browser-worker tests found" >&2
  exit 1
fi

report="$(mktemp)"
trap 'rm -f "$report"' EXIT
npm test -- --test-reporter=tap | tee "$report"
python3 - "$report" <<'PY'
import pathlib
import re
import sys

# TAP ("# tests 19") where the reporter flag takes effect; Node 26 ignores a reporter flag placed after
# the test files and prints the spec summary ("ℹ tests 19") instead. Either way every count is checked.
counts = dict(re.findall(r"^(?:#|ℹ) (tests|pass|fail|cancelled|skipped|todo) (\d+)$", pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"), re.M))
if set(counts) != {"tests", "pass", "fail", "cancelled", "skipped", "todo"} or int(counts["tests"]) == 0 or int(counts["pass"]) == 0 or int(counts["tests"]) != int(counts["pass"]) or any(int(counts[k]) for k in ("fail", "cancelled", "skipped", "todo")):
    sys.exit("Browser-worker tests must execute, pass and leave no skipped, cancelled or todo tests")
PY
