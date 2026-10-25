#!/usr/bin/env bash
# Local regression rehearsal with synthetic Docker/transport fixtures. This does not certify a VM.
# Actual image/database recovery evidence belongs in the owner's isolated rollout rehearsal.
set -euo pipefail
cd "$(dirname "$0")/.."
echo 'Running local synthetic release/recovery regressions; no VM is contacted.'
exec python3 -I scripts/test_release.py
