#!/usr/bin/env bash
# Executed over the existing SSH transport. The private journal is also the durable release lock.
set -euo pipefail
umask 077
ACTION="${1:?action}"; TOKEN="${2:?release token}"; REQUEST_TOKEN="$TOKEN"
[[ "$TOKEN" =~ ^[a-zA-Z0-9-]+$ ]] || exit 2
trap 'rc=$?; case "$rc" in 124|137|143) rc=2;; esac; trap - EXIT; printf "RELEASE_RESULT %s %s\n" "$REQUEST_TOKEN" "$rc"; exit 0' EXIT
trap 'exit 2' HUP INT TERM
command -v flock >/dev/null && command -v timeout >/dev/null || { echo 'VM requires flock and GNU timeout before release' >&2; exit 2; }
exec 9>.release-operation.lock
flock -n 9 || { echo 'Another release operation is still active; state is unresolved' >&2; exit 2; }
if docker info >/dev/null 2>&1; then D=(docker); else sudo -n docker info >/dev/null; D=(sudo -n docker); fi
VM_READY_TIMEOUT="${VM_READY_TIMEOUT:-60}"
[[ "$VM_READY_TIMEOUT" =~ ^[0-9]+$ && "$VM_READY_TIMEOUT" -ge 1 && "$VM_READY_TIMEOUT" -le 120 ]] || exit 2
export VM_READY_TIMEOUT
# Compose must use its recorded env file, not inherited interpolation overrides.
unset APP_IMAGE ACCESS_MODE POSTGRES_USER POSTGRES_PASSWORD POSTGRES_DB CF_ACCESS_TEAM_DOMAIN CF_ACCESS_AUD ACCESS_ALLOWED_EMAILS CLOUDFLARE_TUNNEL_TOKEN EMBEDDINGS_PROVIDER EMBEDDINGS_ENDPOINT EMBEDDINGS_MODEL DATABASE_MAX_CONNECTIONS MATCH_SHORTLIST RUST_LOG
STATE=.release-state
dc() { "${D[@]}" compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod "$@"; }
value() { sed -n "s/^$1=//p" .env.prod | head -1; }
phase() { printf '%s\n' "$1" > "$STATE/phase.next"; mv "$STATE/phase.next" "$STATE/phase"; }
image_id() { "${D[@]}" image inspect "$1" --format '{{.Id}}'; }
contains_id() { [[ ",$1," == *",$2,"* ]]; }
app_id() { dc ps -aq app | head -1; }
mode() {
  local setting
  setting="$("${D[@]}" inspect "$1" --format '{{range .Config.Env}}{{if eq (index (split . "=") 0) "ACCESS_MODE"}}{{.}}{{end}}{{end}}')" || return 1
  setting="${setting#ACCESS_MODE=}"; printf '%s' "${setting:-cloudflare}"
}
valid_id() { [[ "$1" =~ ^sha256:[a-f0-9]{64}$ ]]; }
check() {
  local expected="$1" expected_mode="$2" app actual configured
  app="$(app_id)"; [[ -n "$app" ]] || return 1
  actual="$("${D[@]}" inspect "$app" --format '{{.Image}}')" || return 1
  [[ "$actual" == "$expected" ]] || { echo 'Running immutable image identity differs' >&2; return 1; }
  configured="$(value APP_IMAGE)"; configured="${configured:-opportunity-app:latest}"
  [[ "$(image_id "$configured")" == "$expected" ]] || return 1
  local configured_mode; configured_mode="$(value ACCESS_MODE)"; configured_mode="${configured_mode:-cloudflare}"
  [[ "${expected_mode}" == "$(mode "$app")" && "${expected_mode}" == "$configured_mode" ]] || return 1
  # The timeout also bounds older retained checkers which have no readiness polling.
  local deadline=$((SECONDS + VM_READY_TIMEOUT)) code
  while :; do
    code="$("${D[@]}" exec "$app" curl -s --connect-timeout 2 --max-time 2 -o /dev/null -w '%{http_code}' http://127.0.0.1:8787/ready 2>/dev/null)" || code=000
    [[ "$code" == 200 ]] && break
    (( SECONDS < deadline )) || { echo "Readiness deadline expired (HTTP $code)" >&2; return 1; }
    sleep 1
  done
  # vm-check.sh exits with how many checks failed. That count is not a result code: two failed
  # checks would otherwise read as 2, "unresolved", and leave a failed release running unrecovered.
  EXPECTED_IMAGE_ID="$expected" timeout 150 sh vm-check.sh || { local rc=$?; [[ "$rc" == 124 ]] && return 124; return 1; }
}
configuration_matches() {
  local outcome="$1" expected file
  expected="$STATE/after.env"
  [[ "$outcome" != recovered ]] || expected="$STATE/recovery.env"
  cmp -s .env.prod "$expected" || return 1
  for file in docker-compose.prod.yml docker-compose.e2micro.yml vm-check.sh; do
    expected="$STATE/before/$file"
    if [[ "$outcome" != recovered && "$(cat "$STATE/kind")" == deploy ]]; then expected="$STATE/incoming-$TOKEN/$file"; fi
    cmp -s "$file" "$expected" || return 1
  done
}
archive_state() {
  local outcome="$1"
  phase "$outcome"
  mkdir -p .release-history
  [[ ! -e ".release-history/$TOKEN" ]]
  mv "$STATE" ".release-history/$TOKEN"
}
require_state() {
  [[ -d "$STATE" && "$(cat "$STATE/token")" == "$TOKEN" ]] || { echo 'Release journal is absent or belongs to another invocation' >&2; exit 2; }
}
recover() {
  require_state
  [[ -f "$STATE/prepared" ]] || { echo 'Preparation was interrupted; preserve journal for inspection' >&2; exit 2; }
  # Do not overwrite an operator edit made outside this journal.
  [[ "$(cat "$STATE/phase")" != external-change ]] || exit 2
  for file in .env.prod docker-compose.prod.yml docker-compose.e2micro.yml vm-check.sh; do
    if cmp -s "$file" "$STATE/before/$file"; then continue; fi
    if [[ "$file" == .env.prod ]]; then
      cmp -s "$file" "$STATE/after.env" || cmp -s "$file" "$STATE/recovery.env" || { phase external-change; exit 2; }
    else
      cmp -s "$file" "$STATE/incoming-$TOKEN/$file" || { phase external-change; exit 2; }
    fi
  done
  phase recovering
  local previous previous_mode
  previous="$(cat "$STATE/previous-id")"; previous_mode="$(cat "$STATE/previous-mode")"
  [[ "$(image_id "$(cat "$STATE/rollback-ref")")" == "$previous" ]]
  for file in .env.prod docker-compose.prod.yml docker-compose.e2micro.yml vm-check.sh; do
    cp "$STATE/before/$file" "$file.release-next"; mv "$file.release-next" "$file"
  done
  chmod 600 .env.prod
  # A tag may have been moved outside this script; use the retained immutable rollback reference.
  local ref; ref="$(cat "$STATE/rollback-ref")"
  sed "s|^APP_IMAGE=.*|APP_IMAGE=$ref|" .env.prod > "$STATE/recovery.env"
  if ! grep -q '^APP_IMAGE=' "$STATE/recovery.env"; then printf '\nAPP_IMAGE=%s\n' "$ref" >> "$STATE/recovery.env"; fi
  cp "$STATE/recovery.env" .env.prod.release-next; mv .env.prod.release-next .env.prod
  timeout 90 "${D[@]}" compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod up -d --no-build --no-deps app
  check "$previous" "$previous_mode"
  archive_state recovered
  echo 'Previous immutable image and configuration pass complete verification; database was not restored'
}
case "$ACTION" in
  begin)
    [[ $# == 6 ]]
    TAG="$3"; IDS="$4"; REVISION="$5"; KIND="$6"
    [[ "$TAG" =~ ^[a-zA-Z0-9][a-zA-Z0-9:._/-]+$ && "$KIND" =~ ^(deploy|rollback)$ ]]
    if [[ -e "$STATE" ]]; then echo 'Unresolved release journal exists; use --reconcile, do not retry deployment' >&2; exit 2; fi
    mkdir "$STATE"; printf '%s\n' "$TOKEN" > "$STATE/token"; phase preparing
    mkdir "$STATE/before" "$STATE/incoming-$TOKEN"
    for file in .env.prod docker-compose.prod.yml docker-compose.e2micro.yml vm-check.sh; do test -f "$file"; cp "$file" "$STATE/before/$file"; done
    chmod 600 "$STATE/before/"*
    app="$(app_id)"; test -n "$app"
    previous="$("${D[@]}" inspect "$app" --format '{{.Image}}')"; valid_id "$previous"
    previous_mode="$(mode "$app")"; configured_mode="$(value ACCESS_MODE)"; configured_mode="${configured_mode:-cloudflare}"
    [[ "$previous_mode" == "$configured_mode" && "$configured_mode" =~ ^(cloudflare|both|native)$ ]]
    configured="$(value APP_IMAGE)"; configured="${configured:-opportunity-app:latest}"
    [[ "$(image_id "$configured")" == "$previous" ]] || { phase external-change; echo 'Configured/running image mismatch before release' >&2; exit 2; }
    ref="opportunity-app:rollback-$TOKEN"
    if image_id "$ref" >/dev/null 2>&1; then exit 2; fi
    "${D[@]}" tag "$previous" "$ref"
    printf '%s\n' "$previous" > "$STATE/previous-id"
    printf '%s\n' "$previous_mode" > "$STATE/previous-mode"
    printf '%s\n' "$ref" > "$STATE/rollback-ref"
    printf '%s\n' "$TAG" > "$STATE/target-tag"
    printf '%s\n' "$IDS" > "$STATE/target-ids"
    printf '%s\n' "$REVISION" > "$STATE/revision"
    printf '%s\n' "$KIND" > "$STATE/kind"
    touch "$STATE/prepared"; phase prepared
    echo "Prepared durable release journal $TOKEN; previous image $previous retained as $ref"
    ;;
  load)
    require_state
    [[ "$(cat "$STATE/phase")" == prepared ]]
    [[ "$3" =~ ^opportunity-app-[a-f0-9]{40}-amd64.tar.gz$ && "$4" =~ ^[a-f0-9]{64}$ ]]
    tag="$(cat "$STATE/target-tag")"; ids="$(cat "$STATE/target-ids")"
    current="$(image_id "$tag" 2>/dev/null)" || current=''
    if [[ -n "$current" ]] && ! contains_id "$ids" "$current"; then echo 'Refusing mutable release tag collision before load' >&2; exit 1; fi
    printf '%s  %s\n' "$4" "$STATE/incoming-$TOKEN/$3" | sha256sum -c -
    phase loading
    timeout 120 "${D[@]}" load -i "$STATE/incoming-$TOKEN/$3"
    loaded="$(image_id "$tag")"; contains_id "$ids" "$loaded" || { echo 'Loaded image identity differs from verified archive' >&2; exit 1; }
    [[ "$("${D[@]}" image inspect "$tag" --format '{{.Architecture}} {{.Os}}')" == 'amd64 linux' ]]
    [[ "$("${D[@]}" image inspect "$tag" --format '{{index .Config.Labels "org.opencontainers.image.revision"}}')" == "$(cat "$STATE/revision")" ]]
    [[ "$("${D[@]}" image inspect "$tag" --format '{{index .Config.Labels "org.opencontainers.image.title"}}')" == opportunity-runtime ]]
    printf '%s\n' "$loaded" > "$STATE/target-id"
    # Docker holds the image now. The archive is tens of megabytes that recovery never reads, and
    # kept in .release-history it would fill this disk one release at a time.
    printf '%s\n' "$4" > "$STATE/archive-sha256"; rm -f "$STATE/incoming-$TOKEN/$3"
    phase loaded
    ;;
  apply)
    require_state
    kind="$(cat "$STATE/kind")"; tag="$(cat "$STATE/target-tag")"
    if [[ "$kind" == rollback ]]; then
      [[ "$(cat "$STATE/phase")" == prepared ]]
      id="$(image_id "$tag")"; valid_id "$id"; printf '%s\n' "$id" > "$STATE/target-id"
    else [[ "$(cat "$STATE/phase")" == loaded ]]; fi
    for file in .env.prod docker-compose.prod.yml docker-compose.e2micro.yml vm-check.sh; do
      cmp -s "$file" "$STATE/before/$file" || { phase external-change; echo 'Configuration changed outside this release; reconcile manually' >&2; exit 2; }
    done
    phase configuring
    if [[ "$kind" == deploy ]]; then
      for file in docker-compose.prod.yml docker-compose.e2micro.yml vm-check.sh; do cp "$STATE/incoming-$TOKEN/$file" "$file.release-next"; mv "$file.release-next" "$file"; done
    fi
    sed "s|^APP_IMAGE=.*|APP_IMAGE=$tag|" .env.prod > "$STATE/after.env"
    if ! grep -q '^APP_IMAGE=' "$STATE/after.env"; then printf '\nAPP_IMAGE=%s\n' "$tag" >> "$STATE/after.env"; fi
    cp "$STATE/after.env" .env.prod.release-next; mv .env.prod.release-next .env.prod; chmod 600 .env.prod; phase configured
    timeout 90 "${D[@]}" compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod up -d --no-build --no-deps app
    phase started
    check "$(cat "$STATE/target-id")" "$(cat "$STATE/previous-mode")"
    configuration_matches committed
    phase verified
    ;;
  finish)
    require_state
    [[ "$(cat "$STATE/phase")" == verified ]]
    configuration_matches committed
    check "$(cat "$STATE/target-id")" "$(cat "$STATE/previous-mode")"
    archive_state committed
    echo 'Release identity, configuration, mode, readiness and complete checks verified'
    ;;
  recover) recover ;;
  reconcile)
    if [[ ! -d "$STATE" ]]; then
      STATE=".release-history/$TOKEN"
      [[ -d "$STATE" ]] || { echo 'No pending journal; supply its recorded token to verify completed history' >&2; exit 2; }
      if [[ "$(cat "$STATE/phase")" == committed ]]; then
        configuration_matches committed
        check "$(cat "$STATE/target-id")" "$(cat "$STATE/previous-mode")"
      elif [[ "$(cat "$STATE/phase")" == recovered ]]; then
        configuration_matches recovered
        check "$(cat "$STATE/previous-id")" "$(cat "$STATE/previous-mode")"
      else exit 2; fi
      echo 'Completed release history independently verified'; exit 0
    fi
    [[ "$TOKEN" == pending || "$TOKEN" == "$(cat "$STATE/token")" ]] || exit 2
    TOKEN="$(cat "$STATE/token")"; [[ "$TOKEN" =~ ^[a-zA-Z0-9-]+$ ]]; require_state
    [[ "$(cat "$STATE/phase")" != external-change ]] || { echo 'External configuration change requires operator review' >&2; exit 2; }
    # Accept a completed intended release only after independently checking it; otherwise restore prior state.
    if [[ -f "$STATE/target-id" ]] && configuration_matches committed && check "$(cat "$STATE/target-id")" "$(cat "$STATE/previous-mode")"; then
      archive_state committed; echo 'Reconciled and verified the intended release'
    else recover; fi
    ;;
  status)
    if [[ -d "$STATE" ]]; then printf 'UNRESOLVED release=%s phase=%s\n' "$(cat "$STATE/token")" "$(cat "$STATE/phase")"; exit 2; fi
    app="$(app_id)"; test -n "$app"
    running="$("${D[@]}" inspect "$app" --format '{{.Image}}')"
    printf 'Configured image: %s\nRunning immutable image: %s\nActual access mode: %s\n' "$(value APP_IMAGE)" "$running" "$(mode "$app")"
    echo 'No pending release journal'
    # Machine-readable, for release-gate.py: what actually runs, what a restart would run, and SQLx's
    # own record of the schema, including a migration that failed part-way.
    configured="$(value APP_IMAGE)"; configured="${configured:-opportunity-app:latest}"
    printf 'DEPLOYED_REVISION=%s\nDEPLOYED_IMAGE=%s\nCONFIGURED_IMAGE_ID=%s\n' "$("${D[@]}" image inspect "$running" --format '{{index .Config.Labels "org.opencontainers.image.revision"}}')" "$running" "$(image_id "$configured" 2>/dev/null || true)"
    dc exec -T db sh -c 'psql -XAt -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d "$POSTGRES_DB"' <<'SQL'
select case when success then 'MIGRATION=' else 'MIGRATION_FAILED=' end || version || ':' || encode(checksum, 'hex') from _sqlx_migrations order by version;
SQL
    echo MIGRATIONS_END
    ;;
  *) exit 2 ;;
esac
