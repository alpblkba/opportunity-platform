#!/usr/bin/env bash
# Put a built image on the VM and start it. Nothing here compiles anything on the far side.
#
#   ./scripts/deploy-vm.sh --preflight                    # check everything, change nothing
#   ./scripts/deploy-vm.sh --dry-run <archive>            # print every command, touch nothing
#   ./scripts/deploy-vm.sh dist/opportunity-app-<tag>.tar.gz
#   ./scripts/deploy-vm.sh --rollback opportunity-app:<previous tag>
#   ./scripts/deploy-vm.sh --status
#   ./scripts/deploy-vm.sh --reconcile                   # inspect and resolve a pending journal
#
# Transport is Google's IAP tunnel by default, which needs no public address on the VM and no open
# firewall: the connection is authorized by IAM and tunnelled over Google's front end. Set
# TRANSPORT=ssh with SSH_HOST to use an ordinary SSH host instead, which is how this script is
# rehearsed locally against a 1 GiB VM.
set -euo pipefail

cd "$(dirname "$0")/.."

PROJECT="${GCP_PROJECT:-project-484f9dc3-1a36-4b55-9e8}"
INSTANCE="${GCP_INSTANCE:-opportunity-platform}"
ZONE="${GCP_ZONE:-us-central1-a}"
TRANSPORT="${TRANSPORT:-iap}"
REMOTE_DIR="${REMOTE_DIR:-/opt/opportunity}"
# The connector started by hand to prove the tunnel works. Cloudflare is happy with several
# connectors on one tunnel; a 1 GiB machine is not, and one nobody manages is one nobody updates.
BOOTSTRAP_CONNECTOR="${BOOTSTRAP_CONNECTOR:-opportunity-tunnel-bootstrap}"
DRY_RUN="no"
MODE="deploy"
RECONCILE_TOKEN="pending"
ARCHIVE=""
ROLLBACK_TAG=""
# What the machine has to be. Taken from the archive's own name when there is one, so a rehearsal on
# other hardware checks itself honestly rather than being told to skip the check.
EXPECTED_MACHINE="x86_64"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN="yes"; shift ;;
    --preflight) MODE="preflight"; shift ;;
    --rollback) MODE="rollback"; ROLLBACK_TAG="${2:?--rollback needs the image tag to go back to}"; shift 2 ;;
    --status) MODE="status"; shift ;;
    --reconcile) MODE="reconcile"; shift; if [[ $# -gt 0 && "$1" != -* ]]; then RECONCILE_TOKEN="$1"; shift; fi ;;
    -*) echo "unknown option: $1" >&2; exit 2 ;;
    *) ARCHIVE="$1"; shift ;;
  esac
done

case "${ARCHIVE:-}" in
  *-arm64.tar.gz) EXPECTED_MACHINE="aarch64" ;;
  *-amd64.tar.gz) EXPECTED_MACHINE="x86_64" ;;
esac

# Whether this user can reach the Docker socket is decided on the VM, every time, because it is a
# property of that machine and not of this one. A Debian user with passwordless sudo and no docker
# group membership is the normal case here. `sudo -n` means a machine that would have asked for a
# password fails loudly instead of hanging on one nobody is there to type.
DOCKER_PRELUDE='if docker info >/dev/null 2>&1; then D="docker"; elif sudo -n docker info >/dev/null 2>&1; then D="sudo -n docker"; else echo "cannot reach the Docker socket, with or without passwordless sudo" >&2; exit 1; fi;'

# Everything that touches the far side goes through these two, so --dry-run is honest by
# construction and the local rehearsal needs no second code path.
remote() {
  local command="$*"
  if [[ "$DRY_RUN" == "yes" ]]; then
    printf '  [dry run] run: %s\n' "$command"
    return 0
  fi
  case "$TRANSPORT" in
    iap) gcloud compute ssh "$INSTANCE" --project "$PROJECT" --zone "$ZONE" --tunnel-through-iap --command "$command" ;;
    ssh) ssh "${SSH_HOST:?set SSH_HOST for TRANSPORT=ssh}" "$command" ;;
    lima) limactl shell "${LIMA_INSTANCE:?set LIMA_INSTANCE}" -- sh -c "$command" ;;
    *) echo "unknown transport: $TRANSPORT" >&2; exit 2 ;;
  esac
}

copy() {
  local source="$1" destination="$2"
  if [[ "$DRY_RUN" == "yes" ]]; then
    printf '  [dry run] copy: %s -> %s\n' "$source" "$destination"
    return 0
  fi
  case "$TRANSPORT" in
    iap) gcloud compute scp "$source" "${INSTANCE}:${destination}" --project "$PROJECT" --zone "$ZONE" --tunnel-through-iap ;;
    ssh) scp "$source" "${SSH_HOST:?set SSH_HOST for TRANSPORT=ssh}:${destination}" ;;
    lima) limactl shell "${LIMA_INSTANCE:?set LIMA_INSTANCE}" -- sh -c "cat > ${destination}" < "$source" ;;
    *) echo "unknown transport: $TRANSPORT" >&2; exit 2 ;;
  esac
}

# Just the compose invocation. The caller supplies the directory and the `$D` the prelude chose.
compose() {
  echo "\$D compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod $*"
}

preflight() {
  local problems=0
  echo "== here =="
  if [[ "$TRANSPORT" == "iap" ]]; then
    if command -v gcloud >/dev/null; then
      local account
      account="$(gcloud auth list --filter=status:ACTIVE --format='value(account)' 2>/dev/null | head -1)"
      if [[ -n "$account" ]]; then
        echo "  ok    gcloud is authenticated as ${account}"
      else
        echo "  FAIL  no active gcloud account: run 'gcloud auth login'"; problems=$((problems + 1))
      fi
      echo "  note  project ${PROJECT}, instance ${INSTANCE}, zone ${ZONE}"
    else
      echo "  FAIL  gcloud is not installed"; problems=$((problems + 1))
    fi
  else
    echo "  note  transport ${TRANSPORT}, so no gcloud is involved"
  fi
  if [[ -n "$ARCHIVE" ]]; then
    if [[ -f "$ARCHIVE" && -f "${ARCHIVE}.sha256" ]]; then
      echo "  ok    ${ARCHIVE} and its checksum are here"
    else
      echo "  FAIL  ${ARCHIVE} or its checksum is missing; build it with release-image.sh"; problems=$((problems + 1))
    fi
  fi

  echo
  echo "== reaching the machine =="
  if ! remote "true" >/dev/null 2>&1; then
    echo "  FAIL  cannot run a command on ${INSTANCE} over ${TRANSPORT}"
    echo "        over IAP this needs roles/iap.tunnelResourceAccessor, OS Login on this instance,"
    echo "        and the firewall already allowing 35.235.240.0/20 to TCP 22"
    echo
    echo "Nothing else can be checked without a shell."
    return 1
  fi
  echo "  ok    a command runs on the machine"
  if [[ "$TRANSPORT" == "iap" ]]; then
    # scp uses a second code path inside gcloud, and finding that out during a 38 MB transfer is
    # worse than finding it out now with an empty file.
    local probe
    probe="$(mktemp)"
    if copy "$probe" "${REMOTE_DIR}/.scp-probe-${TOKEN}" >/dev/null 2>&1; then
      echo "  ok    scp over IAP works"
      remote "rm -f ${REMOTE_DIR}/.scp-probe-${TOKEN}" >/dev/null 2>&1 || true
    else
      echo "  FAIL  scp over IAP failed; ssh works, so this is usually ${REMOTE_DIR} not being writable"
      problems=$((problems + 1))
    fi
    rm -f "$probe"
  fi

  echo
  echo "== on the machine =="
  # One remote shell for all of it, so a slow tunnel is crossed once rather than a dozen times.
  local report
  report="$(remote "
    set -u
    problems=0
    say() { printf '  %-5s %s\n' \"\$1\" \"\$2\"; }
    bad() { say FAIL \"\$1\"; problems=\$((problems + 1)); }

    for tool in bash timeout flock sha256sum; do
      command -v \"\$tool\" >/dev/null 2>&1 || bad \"missing required release tool: \$tool\"
    done
    arch=\$(uname -m)
    if [ \"\$arch\" = ${EXPECTED_MACHINE} ]; then say ok \"architecture \$arch, which the image needs\"; else bad \"architecture \$arch, but the image is for ${EXPECTED_MACHINE}\"; fi

    if sudo -n true 2>/dev/null; then say ok 'passwordless sudo works'; else say note 'no passwordless sudo, which only matters if docker needs it'; fi
    if docker info >/dev/null 2>&1; then say ok 'docker is reachable without sudo'
    elif sudo -n docker info >/dev/null 2>&1; then say ok 'docker is reachable through sudo'
    else bad 'cannot reach the Docker socket, with or without passwordless sudo'; fi

    ${DOCKER_PRELUDE}
    say note \"\$(\$D --version 2>/dev/null | head -1)\"
    if \$D compose version >/dev/null 2>&1; then say ok \"compose \$(\$D compose version --short 2>/dev/null)\"; else bad 'docker compose is not available'; fi

    total_mb=\$(free -m | awk '/^Mem:/ {print \$2}')
    say note \"\${total_mb} MB of memory, limits in the profile sum to 512 MiB\"
    free_disk=\$(df -BG --output=avail / | tail -1 | tr -dc '0-9')
    if [ \"\${free_disk:-0}\" -ge 3 ]; then say ok \"\${free_disk} GB free on /\"; else bad \"\${free_disk} GB free on /, and the images alone want about 1.5 GB\"; fi

    if [ -d ${REMOTE_DIR} ] && [ -w ${REMOTE_DIR} ]; then
      say ok '${REMOTE_DIR} exists and is writable'
    else
      bad '${REMOTE_DIR} is missing or not writable: sudo mkdir -p ${REMOTE_DIR} && sudo chown \$USER ${REMOTE_DIR}'
    fi

    if [ -f ${REMOTE_DIR}/.env.prod ]; then
      mode=\$(stat -c '%a' ${REMOTE_DIR}/.env.prod)
      if [ \"\$mode\" = 600 ]; then say ok '.env.prod is mode 600'; else bad \".env.prod is mode \$mode, expected 600\"; fi
      missing=''
      for key in POSTGRES_USER POSTGRES_PASSWORD POSTGRES_DB ACCESS_MODE CF_ACCESS_TEAM_DOMAIN CF_ACCESS_AUD ACCESS_ALLOWED_EMAILS CLOUDFLARE_TUNNEL_TOKEN; do
        grep -q \"^\${key}=..*\" ${REMOTE_DIR}/.env.prod || missing=\"\$missing \$key\"
        grep -q \"^\${key}=REPLACE_WITH\" ${REMOTE_DIR}/.env.prod && missing=\"\$missing \$key(placeholder)\"
      done
      if [ -z \"\$missing\" ]; then say ok 'every required variable is set'; else bad \"unset or still a placeholder:\$missing\"; fi
      # DATABASE_URL embeds this password in a URL, so a character that needs escaping there is a
      # connection failure at first start rather than a warning anywhere.
      password=\$(grep '^POSTGRES_PASSWORD=' ${REMOTE_DIR}/.env.prod | cut -d= -f2-)
      case \"\$password\" in
        '') : ;;
        *[!A-Za-z0-9._~-]*) bad 'POSTGRES_PASSWORD has characters that are not safe inside a URL; regenerate with: openssl rand -hex 24' ;;
        *) say ok 'POSTGRES_PASSWORD is URL-safe' ;;
      esac
    else
      bad 'no .env.prod on the machine yet; create it from .env.prod.example'
    fi

    stray=\$(\$D ps --filter name=${BOOTSTRAP_CONNECTOR} --format '{{.Names}}' 2>/dev/null)
    if [ -n \"\$stray\" ]; then bad 'a bootstrap connector is running; resolve it before release'; fi
    if systemctl is-active --quiet cloudflared 2>/dev/null; then bad 'a standalone cloudflared service is active; resolve it before release'; fi

    echo \"REMOTE_PROBLEMS=\$problems\"
  ")"
  printf '%s\n' "$report" | grep -v '^REMOTE_PROBLEMS='
  local remote_problems
  remote_problems="$(printf '%s\n' "$report" | sed -n 's/^REMOTE_PROBLEMS=//p' | tail -1)"
  problems=$((problems + ${remote_problems:-0}))

  echo
  if [[ "$problems" -eq 0 ]]; then
    echo "Preflight passed."
  else
    echo "${problems} check(s) failed."
  fi
  return "$problems"
}

# All mutations run through one bounded VM helper. Its owner-only journal survives client loss.
[[ "$REMOTE_DIR" =~ ^/[a-zA-Z0-9_/-]+$ && "$REMOTE_DIR" != / && "$REMOTE_DIR" != *..* ]] || { echo 'Unsafe remote directory' >&2; exit 2; }
TOKEN="$(date -u +%Y%m%dT%H%M%S)-$$-${RANDOM}"
ACTIVE=no
UNCERTAIN=no
IN_FLIGHT=no
LAST_RESULT=0
VM_READY_TIMEOUT="${VM_READY_TIMEOUT:-60}"
[[ "$VM_READY_TIMEOUT" =~ ^[0-9]+$ && "$VM_READY_TIMEOUT" -ge 1 && "$VM_READY_TIMEOUT" -le 120 ]] || exit 2
vm_action() {
  local action="$1" output transport_rc marker request result tail
  shift
  IN_FLIGHT=yes
  if output="$(remote "cd ${REMOTE_DIR} && VM_READY_TIMEOUT=${VM_READY_TIMEOUT} timeout 420 bash -s -- ${action} ${TOKEN} $*" < scripts/vm-release.sh)"; then transport_rc=0; else transport_rc=$?; fi
  IN_FLIGHT=no
  tail="$(printf '%s\n' "$output" | tail -1)"
  read -r marker request result <<< "$tail" || true
  printf '%s\n' "$output" | sed '$d'
  if [[ "$transport_rc" != 0 || "$marker" != RELEASE_RESULT || "$request" != "$TOKEN" || ! "$result" =~ ^[0-9]+$ ]]; then
    UNCERTAIN=yes; LAST_RESULT=2
    echo "Transport completion is uncertain; journal ${TOKEN} must be reconciled before retry" >&2
    return 2
  fi
  case "$result" in 0) LAST_RESULT=0;; 2) LAST_RESULT=2;; *) LAST_RESULT=1;; esac
  return "$LAST_RESULT"
}
on_exit() {
  local result=$?
  trap - EXIT INT TERM HUP
  if [[ "$ACTIVE" == yes && "$result" != 0 ]]; then
    if [[ "$UNCERTAIN" == yes || "$LAST_RESULT" == 2 ]]; then
      echo 'UNRESOLVED RELEASE: preserve the VM journal and run --reconcile; do not retry blindly' >&2
      exit 2
    fi
    if vm_action recover; then
      echo 'Release failed; previous image/configuration passed complete recovery checks' >&2
      exit 1
    fi
    echo 'UNRESOLVED RELEASE: recovery could not be proved; database restoration may be required' >&2
    exit 2
  fi
  exit "$result"
}
on_signal() {
  echo 'Deployment interrupted; verifying recovery only if remote completion is known' >&2
  if [[ "$ACTIVE" == yes && "$IN_FLIGHT" == no ]]; then exit 1; fi
  UNCERTAIN=yes
  exit 2
}
trap on_exit EXIT
trap on_signal INT TERM HUP

case "$MODE" in
  preflight) preflight; exit $? ;;
  status) vm_action status; exit $? ;;
  reconcile)
    TOKEN="$RECONCILE_TOKEN"
    [[ "$TOKEN" =~ ^[a-zA-Z0-9-]+$ ]] || exit 2
    [[ "$DRY_RUN" == no ]] || { echo 'Would reconcile the pending VM journal'; exit 0; }
    if vm_action reconcile; then exit 0; else exit 2; fi
    ;;
esac

if [[ "$MODE" == deploy ]]; then
  [[ -n "$ARCHIVE" ]] || { echo 'Supply the reviewed image archive' >&2; exit 1; }
  ARTIFACT="$(python3 scripts/release-artifact.py "$ARCHIVE" --revision "${REVIEWED_SHA:-}")"
  read -r TAG IDENTITIES CHECKSUM <<< "$ARTIFACT"
  [[ -z "${APP_IMAGE:-}" || "$APP_IMAGE" == "$TAG" ]] || { echo 'APP_IMAGE conflicts with the verified archive' >&2; exit 1; }
  [[ "$(git rev-parse HEAD)" == "$REVIEWED_SHA" ]] || { echo 'Operator checkout differs from the reviewed image' >&2; exit 1; }
else
  TAG="$ROLLBACK_TAG"; IDENTITIES=-
  [[ "$TAG" =~ ^[a-zA-Z0-9][a-zA-Z0-9:._/-]+$ ]] || { echo 'Unsafe rollback reference' >&2; exit 1; }
fi
if [[ "$DRY_RUN" == yes ]]; then
  echo "Would lock/journal ${REMOTE_DIR}, retain the previous immutable image, verify/load ${TAG}, start app and fully check identity/mode/readiness"
  exit 0
fi
if [[ "$MODE" == deploy ]]; then preflight; fi
ACTIVE=yes
vm_action begin "$TAG" "$IDENTITIES" "${REVIEWED_SHA:--}" "$MODE"
if [[ "$MODE" == deploy ]]; then
  INCOMING="${REMOTE_DIR}/.release-state/incoming-${TOKEN}"
  copy "$ARCHIVE" "${INCOMING}/$(basename "$ARCHIVE")"
  copy docker-compose.prod.yml "${INCOMING}/docker-compose.prod.yml"
  copy docker-compose.e2micro.yml "${INCOMING}/docker-compose.e2micro.yml"
  copy scripts/vm-check.sh "${INCOMING}/vm-check.sh"
  vm_action load "$(basename "$ARCHIVE")" "$CHECKSUM"
fi
vm_action apply
vm_action finish
ACTIVE=no
echo "Verified ${MODE}: ${TAG}. The retained rollback reference and prior configuration are in .release-history/${TOKEN}."
echo 'No database restore or Cloudflare policy change was performed.'
