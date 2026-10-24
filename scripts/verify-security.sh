#!/usr/bin/env bash
# The command that proves the security suite actually ran.
#
#   ./scripts/verify-security.sh
#
# Everything here is checked against a real PostgreSQL. It fails, loudly, if the database is not
# reachable: a security suite that passes because it skipped itself is worse than one that fails,
# and the HTTP tests are the only evidence that the gate is where it is supposed to be.
set -euo pipefail

cd "$(dirname "$0")/.."

# Set VERIFY_ENV_FILE to an empty string to use only explicit environment variables.
verification_env_file="${VERIFY_ENV_FILE-.env}"
if [[ -f "$verification_env_file" ]]; then set -a; source "$verification_env_file"; set +a; fi
: "${DATABASE_URL:?DATABASE_URL is not set. Start PostgreSQL (./run.sh or docker compose up -d db) and try again.}"

# ALLOW_SKIP_DB_TESTS would let the integration tests pass without running. Not here.
unset ALLOW_SKIP_DB_TESTS

echo "== the database answers =="
psql_check() { cargo run --locked -q --bin opportunity -- stats >/dev/null; }
psql_check || { echo "cannot reach ${DATABASE_URL%%:*}://... as configured" >&2; exit 1; }
echo "  ok"

echo
echo "== formatting and lints =="
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings

echo
echo "== the whole Rust suite, including the HTTP security tests =="
cargo test --locked --workspace

echo
echo "== the frontend =="
(cd apps/web && npm run typecheck && npm test -- --passWithNoTests=false && npm run build)

echo
echo "== the browser worker =="
bash scripts/verify-browser-worker.sh

echo
echo "== privacy =="
python3 -I scripts/check_privacy.py
python3 -I scripts/test_privacy.py
python3 -I scripts/test_ci.py

echo
echo "All of it ran, against PostgreSQL."
