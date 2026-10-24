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

if [[ -f .env ]]; then set -a; source .env; set +a; fi
: "${DATABASE_URL:?DATABASE_URL is not set. Start PostgreSQL (./run.sh or docker compose up -d db) and try again.}"

# ALLOW_SKIP_DB_TESTS would let the integration tests pass without running. Not here.
unset ALLOW_SKIP_DB_TESTS

echo "== the database answers =="
psql_check() { cargo run -q --bin opportunity -- stats >/dev/null; }
psql_check || { echo "cannot reach ${DATABASE_URL%%:*}://... as configured" >&2; exit 1; }
echo "  ok"

echo
echo "== formatting and lints =="
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

echo
echo "== the whole Rust suite, including the HTTP security tests =="
cargo test --workspace

echo
echo "== the HTTP security tests again, named, so the count is visible =="
# Run alone and show each one: this is the list a reviewer reads.
cargo test -p opportunity-api --test native_auth -- --nocapture --test-threads=4 2>&1 | grep -E "^test |test result"

echo
echo "== the frontend =="
(cd apps/web && npx tsc --noEmit && npm run build >/dev/null && npm test -- --run)

echo
echo "== the browser worker =="
npm --prefix apps/browser-worker run build
npm --prefix apps/browser-worker test

echo
echo "== privacy =="
python3 -I scripts/check_privacy.py
python3 -I scripts/test_privacy.py

echo
echo "All of it ran, against PostgreSQL."
