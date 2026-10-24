#!/usr/bin/env bash
# Prove that a failed deployment actually rolls back.
#
#   TRANSPORT=lima LIMA_INSTANCE=e2micro ./scripts/rollback-rehearsal.sh
#
# Deploys A, writes a row into the database, then fails a release twice over - once with a version
# Compose cannot start at all, once with a version that starts and fails its checks - because those
# take different paths through the deployment script. After each, the same six things have to be
# true: the command exited non-zero, the running container is A, .env.prod names A, the restored
# version passed the complete check rather than only /health, the row survived, and /health answers.
#
# This exists because the rollback was written, looked right, and could not have worked: the
# previous image was read after it had already been overwritten, so the "is there something to go
# back to" test compared the new tag against itself and was always false. Nothing but a rehearsal
# catches that.
set -uo pipefail

cd "$(dirname "$0")/.."

TRANSPORT="${TRANSPORT:-lima}"
REMOTE_DIR="${REMOTE_DIR:-/opt/opportunity}"
PLATFORM="${PLATFORM:-linux/arm64}"
export TRANSPORT REMOTE_DIR

failures=0
step() { printf '\n=== %s ===\n' "$1"; }
pass() { printf '  ok    %s\n' "$1"; }
fail() { printf '  FAIL  %s\n' "$1"; failures=$((failures + 1)); }

remote() {
  case "$TRANSPORT" in
    lima) limactl shell "${LIMA_INSTANCE:?set LIMA_INSTANCE}" -- sh -c "$1" ;;
    ssh) ssh "${SSH_HOST:?set SSH_HOST}" "$1" ;;
    *) echo "this rehearsal does not run over ${TRANSPORT}" >&2; exit 2 ;;
  esac
}

docker_on_vm() {
  remote 'if docker info >/dev/null 2>&1; then D="docker"; else D="sudo -n docker"; fi; '"$1"
}

compose_on_vm() {
  docker_on_vm "cd ${REMOTE_DIR} && \$D compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod $1"
}

running_image() {
  docker_on_vm "cd ${REMOTE_DIR} && \$D inspect -f '{{.Config.Image}}' \$(\$D compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod ps -q app 2>/dev/null | head -1) 2>/dev/null" | tr -d '\r\n' | tail -1
}

configured_image() {
  remote "grep '^APP_IMAGE=' ${REMOTE_DIR}/.env.prod | cut -d= -f2-" | tr -d '\r\n' | tail -1
}

rows() {
  compose_on_vm "exec -T db psql -U opportunity -d opportunity -tAc 'select count(*) from organizations'" 2>/dev/null | tr -d '\r\n ' | tail -1
}

# What has to be true after any failed release, whatever failed. Asserted twice, because a version
# that cannot start and a version that starts and fails its checks take different paths through the
# deployment script and only one of them used to reach the rollback at all.
assert_rolled_back() {
  log="$1"; code="$2"
  [ "$code" -ne 0 ] && pass "the deployment command exited ${code}, not zero" || fail "a failed release exited 0"
  [ "$(running_image)" = "$A_TAG" ] && pass "the running container is A again" || fail "running $(running_image), expected ${A_TAG}"
  [ "$(configured_image)" = "$A_TAG" ] && pass ".env.prod names A again" || fail ".env.prod names $(configured_image), expected ${A_TAG}"
  # Not merely /health: the recovery is only successful once the restored version passes every
  # check vm-check.sh makes. Before this the result of that check was thrown away.
  if grep -q "The restored version passes every check" "$log"; then
    pass "the restored version passed the complete post-deployment check"
  else
    fail "the recovery did not report a complete passing check; see ${log}"
  fi
  now="$(rows)"
  if [ -n "$now" ] && [ "$now" = "$BEFORE_ROWS" ]; then
    pass "the database still holds ${now} organizations; the volume was never touched"
  else
    fail "organizations went from ${BEFORE_ROWS} to ${now:-unknown}"
  fi
  health="$(docker_on_vm "cd ${REMOTE_DIR} && \$D compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod exec -T app curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8787/health" 2>/dev/null | tr -d '\r\n' | tail -1)"
  [ "$health" = "200" ] && pass "the restored version serves /health" || fail "/health answered ${health:-nothing}"
}

step "building two versions"
VERSION_A="rehearsal-a" VERSION="rehearsal-a" PLATFORM="$PLATFORM" ./scripts/release-image.sh >/dev/null || { echo "could not build A" >&2; exit 1; }
VERSION="rehearsal-b" PLATFORM="$PLATFORM" ./scripts/release-image.sh >/dev/null || { echo "could not build B" >&2; exit 1; }
ARCH="${PLATFORM##*/}"
A_ARCHIVE="dist/opportunity-app-rehearsal-a-${ARCH}.tar.gz"
B_ARCHIVE="dist/opportunity-app-rehearsal-b-${ARCH}.tar.gz"
A_TAG="opportunity-app:rehearsal-a-${ARCH}"
B_TAG="opportunity-app:rehearsal-b-${ARCH}"
echo "  A: ${A_TAG}"
echo "  B: ${B_TAG}"

step "1. deploying A, which should pass"
if ./scripts/deploy-vm.sh "$A_ARCHIVE" >/dev/null 2>&1; then
  pass "A deployed and passed its checks"
else
  fail "A did not deploy; the rehearsal cannot continue"
  exit 1
fi
[ "$(running_image)" = "$A_TAG" ] && pass "A is the running image" || fail "running $(running_image), expected ${A_TAG}"

step "2. writing a row, to see whether it survives"
remote "true" >/dev/null
compose_on_vm "exec -T db psql -U opportunity -d opportunity -c \"insert into organizations (display_name, normalized_name) values ('Rollback Rehearsal','rollback rehearsal') on conflict do nothing\"" >/dev/null 2>&1
BEFORE_ROWS="$(rows)"
echo "  organizations before: ${BEFORE_ROWS}"

step "3a. deploying a version Compose cannot start at all"
# An archive whose filename implies a tag the archive does not contain: deploy-vm loads it, derives
# APP_IMAGE from the name, and `up -d` fails because pull_policy is never and that tag is not here.
# A real mistake rather than an injected one, and the failure happens after .env.prod was changed,
# which is the path that used to end the script before the rollback could run.
MISNAMED="dist/opportunity-app-rehearsal-missing-${ARCH}.tar.gz"
cp "$B_ARCHIVE" "$MISNAMED"
if command -v shasum >/dev/null; then shasum -a 256 "$MISNAMED" > "${MISNAMED}.sha256"; else sha256sum "$MISNAMED" > "${MISNAMED}.sha256"; fi
set +e
./scripts/deploy-vm.sh "$MISNAMED" > /tmp/rollback-rehearsal-startup.log 2>&1
STARTUP_EXIT=$?
set -e
rm -f "$MISNAMED" "${MISNAMED}.sha256"
grep -E "COMPOSE COULD NOT START|Going back to|Rolled back|RECOVERY FAILED|passes every check" /tmp/rollback-rehearsal-startup.log | sed 's/^/  /'
assert_rolled_back /tmp/rollback-rehearsal-startup.log "$STARTUP_EXIT"

step "3b. deploying B, which starts but fails its health check"
# Named, not blanket: the deployment script runs this same check against the restored version, and
# a marker that failed every image is why that result used to be ignored.
remote "printf '%s' '${B_TAG}' > ${REMOTE_DIR}/.force-check-failure"
set +e
./scripts/deploy-vm.sh "$B_ARCHIVE" > /tmp/rollback-rehearsal.log 2>&1
DEPLOY_EXIT=$?
set -e
remote "rm -f ${REMOTE_DIR}/.force-check-failure"
grep -E "THE POST-DEPLOYMENT CHECK FAILED|Going back to|Rolled back|RECOVERY FAILED|passes every check" /tmp/rollback-rehearsal.log | sed 's/^/  /'

step "4. what the script actually did"
assert_rolled_back /tmp/rollback-rehearsal.log "$DEPLOY_EXIT"

if docker_on_vm "\$D image inspect ${B_TAG} >/dev/null 2>&1"; then
  pass "B is still on the machine, so it can be examined or retried"
else
  fail "B was removed, which makes a second attempt a fresh transfer"
fi

printf '\n'
if [ "$failures" -eq 0 ]; then
  echo "The rollback works: a failed check restored the previous version and the release still failed."
else
  echo "${failures} check(s) failed. The log is /tmp/rollback-rehearsal.log"
fi
exit "$failures"
