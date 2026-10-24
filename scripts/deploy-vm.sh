#!/usr/bin/env bash
# Put a built image on the VM and start it. Nothing here compiles anything on the far side.
#
#   ./scripts/deploy-vm.sh --preflight                    # check everything, change nothing
#   ./scripts/deploy-vm.sh --dry-run <archive>            # print every command, touch nothing
#   ./scripts/deploy-vm.sh dist/opportunity-app-<tag>.tar.gz
#   ./scripts/deploy-vm.sh --rollback opportunity-app:<previous tag>
#   ./scripts/deploy-vm.sh --status
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

# What the application container is actually running, which is the only answer that matters when
# deciding what to go back to.
running_image() {
  { remote "${DOCKER_PRELUDE} \$D inspect -f '{{.Config.Image}}' \$(cd ${REMOTE_DIR} && $(compose ps -q app) 2>/dev/null | head -1) 2>/dev/null" 2>/dev/null | tr -d '\r\n' | tail -1; } || true
}

# What .env.prod names, for the case where nothing is running yet.
configured_image() {
  { remote "grep '^APP_IMAGE=' ${REMOTE_DIR}/.env.prod 2>/dev/null | cut -d= -f2-" 2>/dev/null | tr -d '\r\n' | tail -1; } || true
}

# Which image to run is written into .env.prod rather than exported into the command.
#
# sudo resets the environment, so `APP_IMAGE=x sudo docker compose` loses it and compose quietly
# falls back to its default tag: the wrong version, started successfully, which is the worst kind of
# failure. The env file crosses that boundary, and a reboot then brings back the same image rather
# than whatever `latest` happens to be.
set_app_image() {
  local tag="$1"
  remote "cd ${REMOTE_DIR} && if grep -q '^APP_IMAGE=' .env.prod; then sed -i 's|^APP_IMAGE=.*|APP_IMAGE=${tag}|' .env.prod; else printf 'APP_IMAGE=%s\\n' '${tag}' >> .env.prod; fi && grep '^APP_IMAGE=' .env.prod"
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
    if copy "$probe" "${REMOTE_DIR}/.scp-probe" >/dev/null 2>&1; then
      echo "  ok    scp over IAP works"
      remote "rm -f ${REMOTE_DIR}/.scp-probe" >/dev/null 2>&1 || true
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
    if [ -n \"\$stray\" ]; then say note \"the bootstrap connector \$stray is running; deploying stops it so one connector runs, not two\"; fi
    if systemctl is-active --quiet cloudflared 2>/dev/null; then say note 'a cloudflared systemd service is active; deploying stops and disables it'; fi

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

# Stop whatever connector was started by hand, so the one in the compose file is the only one.
retire_bootstrap_connector() {
  remote "${DOCKER_PRELUDE}
    if \$D ps -a --filter name=${BOOTSTRAP_CONNECTOR} --format '{{.Names}}' | grep -q .; then
      echo '  stopping the bootstrap connector ${BOOTSTRAP_CONNECTOR}'
      \$D rm -f ${BOOTSTRAP_CONNECTOR} >/dev/null
    fi
    if systemctl is-active --quiet cloudflared 2>/dev/null; then
      echo '  stopping and disabling the cloudflared service'
      sudo -n systemctl disable --now cloudflared
    fi
    remaining=\$(\$D ps --filter ancestor=cloudflare/cloudflared --format '{{.Names}}' | tr '\\n' ' ')
    echo \"  cloudflared containers now: \${remaining:-none}\""
}

case "$MODE" in
  preflight)
    preflight
    exit $?
    ;;
  status)
    echo "== what is running =="
    remote "${DOCKER_PRELUDE} cd ${REMOTE_DIR} && $(compose ps)"
    remote "${DOCKER_PRELUDE} \$D stats --no-stream --format '{{.Name}} {{.MemUsage}} {{.CPUPerc}}'"
    remote "grep '^APP_IMAGE=' ${REMOTE_DIR}/.env.prod; free -m | head -2; df -h / | tail -1"
    exit 0
    ;;
  rollback)
    echo "== rolling back to ${ROLLBACK_TAG} =="
    # The previous image is still on the disk unless someone pruned it, which is the whole reason a
    # rollback is a restart and not a rebuild.
    remote "${DOCKER_PRELUDE} \$D image inspect ${ROLLBACK_TAG} >/dev/null"
    set_app_image "$ROLLBACK_TAG"
    remote "${DOCKER_PRELUDE} cd ${REMOTE_DIR} && $(compose up -d --no-build app)"
    remote "${DOCKER_PRELUDE} cd ${REMOTE_DIR} && $(compose ps)"
    echo
    echo "A rollback across a migration that changed the schema needs the backup taken before the"
    echo "update: an older binary does not understand a newer schema. See docs/GCP_DEPLOYMENT.md."
    exit 0
    ;;
esac

[[ -n "$ARCHIVE" ]] || { echo "usage: deploy-vm.sh <image archive> | --preflight | --rollback <tag> | --status [--dry-run]" >&2; exit 2; }
[[ -f "$ARCHIVE" ]] || { echo "no such archive: $ARCHIVE" >&2; exit 1; }
[[ -f "${ARCHIVE}.sha256" ]] || { echo "no checksum beside ${ARCHIVE}; build it with release-image.sh" >&2; exit 1; }

CHECKSUM="$(cut -d' ' -f1 "${ARCHIVE}.sha256")"
BASENAME="$(basename "$ARCHIVE")"
TAG="${APP_IMAGE:-opportunity-app:$(echo "$BASENAME" | sed -e 's/^opportunity-app-//' -e 's/\.tar\.gz$//')}"

if [[ "$DRY_RUN" != "yes" ]]; then
  echo "== preflight =="
  preflight || { echo "Refusing to deploy onto a machine that is not ready." >&2; exit 1; }
  echo
fi

echo "== sending ${BASENAME} to ${INSTANCE} over ${TRANSPORT} =="
copy "$ARCHIVE" "${REMOTE_DIR}/${BASENAME}"
copy docker-compose.prod.yml "${REMOTE_DIR}/docker-compose.prod.yml"
copy docker-compose.e2micro.yml "${REMOTE_DIR}/docker-compose.e2micro.yml"
copy scripts/vm-check.sh "${REMOTE_DIR}/vm-check.sh"

echo "== verifying the transfer =="
# A truncated image would otherwise be found by a container that will not start, on a machine that
# is awkward to look at.
remote "cd ${REMOTE_DIR} && echo '${CHECKSUM}  ${BASENAME}' | sha256sum -c -"

echo "== loading =="
remote "${DOCKER_PRELUDE} \$D load -i ${REMOTE_DIR}/${BASENAME} && rm -f ${REMOTE_DIR}/${BASENAME}"

# What to go back to, read before anything is changed.
#
# This used to be read *after* `set_app_image "$TAG"`, so it always came back as the new tag, the
# "is there something to go back to" test was always false, and the automatic rollback never ran
# once. The running container is asked first because that is what is actually serving; .env.prod is
# the fallback for the case where nothing is running yet.
PREVIOUS_RUNNING="$(running_image)"
PREVIOUS_CONFIGURED="$(configured_image)"
PREVIOUS="${PREVIOUS_RUNNING:-$PREVIOUS_CONFIGURED}"
if [[ -n "$PREVIOUS" ]]; then
  echo "== currently running ${PREVIOUS} =="
  # A file naming one image while another is serving means someone edited .env.prod without
  # restarting, or a previous deployment half-finished. Worth saying before adding another change
  # on top, and worth saying which one a rollback would return to.
  if [[ -n "$PREVIOUS_CONFIGURED" && "$PREVIOUS_CONFIGURED" != "$PREVIOUS_RUNNING" ]]; then
    echo "   NOTE: .env.prod names ${PREVIOUS_CONFIGURED} but ${PREVIOUS_RUNNING} is running." >&2
    echo "   A rollback would return to ${PREVIOUS_RUNNING}, which is what is actually serving." >&2
  fi
else
  echo "== nothing is running yet, so there is nothing to roll back to =="
fi

echo "== the tunnel =="
retire_bootstrap_connector

echo "== recording which image to run =="
set_app_image "$TAG"
remote "chmod 600 ${REMOTE_DIR}/.env.prod"

# Start it and check it. Both steps live in one function called from an `if`, so neither can end
# the script under `set -e` before the recovery below gets a chance: a container that refuses to
# start is as much a failed release as one that starts and fails its checks, and before this they
# were handled by completely different amounts of code - one of them none.
start_and_check() {
  echo "== starting ${TAG} =="
  # --no-build is belt and braces: the compose file carries a build section for the development
  # machine, and this host must never try to use it.
  if ! remote "${DOCKER_PRELUDE} cd ${REMOTE_DIR} && $(compose up -d --no-build)"; then
    echo >&2
    echo "COMPOSE COULD NOT START ${TAG}." >&2
    return 1
  fi
  echo "== checking =="
  # A failed check is a failed release. Without this the script printed "Deployed" over the top of
  # a broken one and exited zero, which is the only way a bad release goes unnoticed.
  if ! remote "cd ${REMOTE_DIR} && sh vm-check.sh"; then
    echo >&2
    echo "THE POST-DEPLOYMENT CHECK FAILED. ${TAG} is running and did not pass." >&2
    return 1
  fi
}

# The one way back, whatever went wrong. Returns non-zero when the recovery itself did not work,
# which is a different and worse situation than a failed release.
recover() {
  if [[ -z "$PREVIOUS" || "$PREVIOUS" == "$TAG" ]]; then
    echo "No earlier image is recorded, so nothing was rolled back automatically." >&2
    echo "Fix it, or deploy a known-good archive." >&2
    return 1
  fi
  echo "Going back to ${PREVIOUS}." >&2
  # The image has to still be here. It is only ever removed by a prune, which this script never
  # runs, but saying so when it is missing beats restarting into nothing.
  if ! remote "${DOCKER_PRELUDE} \$D image inspect ${PREVIOUS} >/dev/null 2>&1"; then
    echo "${PREVIOUS} is no longer on the machine; it cannot be restored automatically." >&2
    echo "Load a known-good archive and deploy it." >&2
    return 1
  fi
  set_app_image "$PREVIOUS"
  remote "${DOCKER_PRELUDE} cd ${REMOTE_DIR} && $(compose up -d --no-build app)" || true

  # Verified against the container, not against the file this script just wrote. The file saying
  # the right thing proves only that the file was written.
  local restored configured health
  restored="$(running_image)"
  configured="$(configured_image)"
  echo >&2
  if [[ "$restored" != "$PREVIOUS" || "$configured" != "$PREVIOUS" ]]; then
    echo "RECOVERY FAILED. Running: ${restored:-nothing}. Configured: ${configured:-nothing}." >&2
    echo "Expected ${PREVIOUS}. Fix this by hand before anything else." >&2
    return 1
  fi
  echo "Rolled back: the running container and .env.prod both say ${PREVIOUS}." >&2

  # And it has to actually work, which is the same bar the release had to clear: every check in
  # vm-check.sh, not /health alone. This result used to be discarded with `|| true`, so a recovery
  # that restored a version answering /health while refusing to authenticate anyone, or running
  # without an identity layer, was still announced as a successful rollback. It was discarded
  # because the rehearsal's failure injection also failed this run; the marker now names the image
  # it applies to, so the restored version is judged on its own.
  if ! remote "cd ${REMOTE_DIR} && sh vm-check.sh"; then
    echo >&2
    echo "RECOVERY FAILED: ${PREVIOUS} is running but does not pass its checks." >&2
    return 1
  fi
  echo "The restored version passes every check." >&2
}

if start_and_check; then
  echo
  echo "Deployed ${TAG}. Keep the previous image on the VM until this one has been used:"
  echo "  ./scripts/deploy-vm.sh --rollback <previous tag>"
else
  recovered=yes
  recover || recovered=no
  echo >&2
  echo "What a rollback does NOT undo:" >&2
  echo "  - migrations. They ran at startup and are not reversed; an older binary may not" >&2
  echo "    understand the newer schema. If the restored version does not come up, restore the" >&2
  echo "    backup taken before this deployment." >&2
  echo "  - Cloudflare policy. Nothing here touches it. If this deployment was part of a" >&2
  echo "    cutover, put the Access policy back by hand. See docs/AUTHENTICATION.md." >&2
  if [[ "$recovered" == "no" ]]; then
    echo >&2
    echo "THE MACHINE IS NOT IN A KNOWN-GOOD STATE. Deal with this before anything else." >&2
    exit 2
  fi
  # A failed release is a failure even when the recovery worked.
  exit 1
fi
