#!/bin/sh
# Run on the VM after a deployment. Answers the only questions worth asking there: is it up, is it
# protected, and does it fit.
#
#   sh vm-check.sh
#
# Exits non-zero when something that must be true is not, so it can gate a deployment rather than
# just describe one.
set -u

# This user may or may not be in the docker group. On the VM it is not, and reaches the socket
# through passwordless sudo; `sudo -n` keeps a machine that would prompt from hanging here.
if docker info >/dev/null 2>&1; then
  D="docker"
elif sudo -n docker info >/dev/null 2>&1; then
  D="sudo -n docker"
else
  echo "cannot reach the Docker socket, with or without passwordless sudo" >&2
  exit 1
fi

COMPOSE="$D compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod"
APP="$($COMPOSE ps -q app 2>/dev/null | head -1)"
DB="$($COMPOSE ps -q db 2>/dev/null | head -1)"
problems=0
VM_READY_TIMEOUT="${VM_READY_TIMEOUT:-60}"
case "$VM_READY_TIMEOUT" in ''|*[!0-9]*) echo 'VM_READY_TIMEOUT must be 1..120 seconds' >&2; exit 1;; esac
[ "$VM_READY_TIMEOUT" -ge 1 ] && [ "$VM_READY_TIMEOUT" -le 120 ] || exit 1

# Read rather than source: this file holds the tunnel token and the database password, and nothing
# here needs to execute it.
value_of() { [ -f .env.prod ] && grep "^$1=" .env.prod | cut -d= -f2- | head -1; }
DB_USER="$(value_of POSTGRES_USER)"
DB_USER="${DB_USER:-opportunity}"
DB_NAME="$(value_of POSTGRES_DB)"
DB_NAME="${DB_NAME:-opportunity}"
MODE="$(value_of ACCESS_MODE)"
MODE="${MODE:-cloudflare}"

fail() { printf '  FAIL  %s\n' "$1"; problems=$((problems + 1)); }
pass() { printf '  ok    %s\n' "$1"; }

# A deliberate failure, for scripts/rollback-rehearsal.sh to prove the deployment script actually
# rolls back. A file rather than a flag so it travels through the plain `sh vm-check.sh` the
# deployment runs, and so a forgotten one is visible in a directory listing. Never present in a real
# deployment; the rehearsal creates it for one run and removes it afterwards.
#
# It holds the image tag it applies to. The rollback runs this same script against the restored
# version, and an unconditional marker failed that run too - which is why the deployment script used
# to have to ignore the result of its own post-recovery check. An empty file still fails any image,
# for the manual case.
if [ -f .force-check-failure ]; then
  forced=$(tr -d '\r\n ' < .force-check-failure)
  if [ -z "$forced" ] || [ "$forced" = "$(value_of APP_IMAGE)" ]; then
    fail "a rollback rehearsal asked this check to fail for ${forced:-any image}"
  else
    pass "a rehearsal marker names ${forced}, which is not the image deployed now"
  fi
fi

echo "== containers =="
$COMPOSE ps --format '  {{.Service}}  {{.Status}}' 2>/dev/null || $COMPOSE ps
[ -n "$APP" ] || fail "the application container is not running"
[ -n "$DB" ] || fail "the database container is not running"

# Wait before inspecting startup logs or account state; migration/startup may still be in progress.
if [ -n "$APP" ]; then
  deadline=$(( $(date +%s) + VM_READY_TIMEOUT ))
  while :; do
    health=$($D exec "$APP" curl -s --connect-timeout 2 --max-time 2 -o /dev/null -w '%{http_code}' http://127.0.0.1:8787/health 2>/dev/null) || health=000
    ready=$($D exec "$APP" curl -s --connect-timeout 2 --max-time 2 -o /dev/null -w '%{http_code}' http://127.0.0.1:8787/ready 2>/dev/null) || ready=000
    [ "$health" = 200 ] && [ "$ready" = 200 ] && break
    if [ "$(date +%s)" -ge "$deadline" ]; then
      fail "startup deadline expired: /health=${health}, /ready=${ready}"
      break
    fi
    printf '  waiting for startup: /health=%s /ready=%s\n' "$health" "$ready"
    sleep 1
  done
fi

echo
echo "== which image is running =="
wanted="$(value_of APP_IMAGE)"
running="$([ -n "$APP" ] && $D inspect "$APP" --format '{{.Config.Image}}' 2>/dev/null)"
printf '  wanted  %s\n' "${wanted:-unset in .env.prod}"
printf '  running %s\n' "${running:-nothing}"
actual_id="$([ -n "$APP" ] && $D inspect "$APP" --format '{{.Image}}' 2>/dev/null)"
if [ -n "${EXPECTED_IMAGE_ID:-}" ] && [ "$actual_id" != "$EXPECTED_IMAGE_ID" ]; then
  fail "the running immutable image ID differs from the verified release"
fi
if [ -n "$wanted" ]; then
  configured_id=$($D image inspect "$wanted" --format '{{.Id}}' 2>/dev/null)
  [ -n "$actual_id" ] && [ "$configured_id" = "$actual_id" ] || fail "configured and running immutable image IDs differ"
fi
if [ -n "$wanted" ] && [ -n "$running" ] && [ "$wanted" != "$running" ]; then
  fail "the running container is not the image .env.prod names; a restart would change versions"
fi

echo
echo "== one tunnel connector, not two =="
# A bootstrap connector left running costs memory this machine does not have and would keep serving
# after the managed one is replaced.
connectors=$($D ps --filter ancestor=cloudflare/cloudflared --format '{{.Names}}' 2>/dev/null | tr '\n' ' ')
count=$(printf '%s' "$connectors" | wc -w | tr -d ' ')
if [ "$count" -le 1 ]; then
  pass "cloudflared containers: ${connectors:-none}"
else
  fail "more than one cloudflared container is running: ${connectors}"
fi
if systemctl is-active --quiet cloudflared 2>/dev/null; then
  fail "a cloudflared systemd service is also running alongside the compose tunnel"
fi

echo
echo "== what the application says about itself =="
if [ -n "$APP" ]; then
  said=$($D logs "$APP" 2>&1 | grep -E "access:|semantic reranking|assisted application|listening|migrations" | tail -6)
  if [ -n "$said" ]; then
    printf '%s\n' "$said" | sed 's/^/  /'
  else
    # It never got as far as saying anything about itself, so the last thing it did say is what matters.
    echo "  it has not reported a startup line. Its last output:"
    $D logs "$APP" 2>&1 | tail -5 | sed 's/^/    /'
  fi
  # Three production modes, all of which check identity in the application: cloudflare (today),
  # both (Cloudflare and a native session, the cutover rung), native (sessions alone). Only
  # loopback means nothing is checking, and that must never be what is running here.
  if $D logs "$APP" 2>&1 | grep -q "access: loopback only"; then
    fail "the application is running with no identity layer"
  elif $D logs "$APP" 2>&1 | grep -qE "access: (Cloudflare Access|native sessions)"; then
    pass "identity is enforced in the application: $($D logs "$APP" 2>&1 | grep -m1 'access:' | sed 's/.*access: //' | cut -c1-60)"
  else
    fail "the application did not say how it decides who is calling"
  fi
  if $D logs "$APP" 2>&1 | grep -q "assisted application is off"; then
    pass "the browser is off, as the low-memory profile intends"
  else
    fail "a browser is configured in a profile measured not to have room for one"
  fi
fi

echo
echo "== the mode the application is actually running =="
# .env.prod is what someone wrote; this is what the process was started with. In both and native
# modes the unauthenticated status codes are identical, so the file alone cannot tell them apart -
# and the difference between them is whether Cloudflare Access is still required. Getting that
# wrong in the permissive direction, right before switching Access to Bypass, is the mistake this
# check exists to prevent.
#
# Only this one variable is read. The container's environment also holds the database password and
# the tunnel token, and neither belongs in any output.
if [ -n "$APP" ]; then
  RUNNING_MODE=$($D inspect "$APP" --format '{{range .Config.Env}}{{println .}}{{end}}' 2>/dev/null \
    | grep '^ACCESS_MODE=' | cut -d= -f2- | tr -d '\r' | head -1)
  RUNNING_MODE="${RUNNING_MODE:-cloudflare}"
  if [ "$RUNNING_MODE" = "$MODE" ]; then
    pass "the container is running ACCESS_MODE=${RUNNING_MODE}, which is what .env.prod says"
  else
    fail "the container is running ACCESS_MODE=${RUNNING_MODE} but .env.prod says ${MODE}: restart the application before trusting either"
  fi
  # Everything below is judged against what is actually running.
  MODE="$RUNNING_MODE"
fi

echo
echo "== who can log in (mode: ${MODE}) =="
# Asked of the database, not of the startup log: the log says what was true when the process
# started, and the first account is created after that by design. Asked after the running mode is
# known, because it is only a requirement in the modes that use native sessions - in cloudflare mode
# there are deliberately no accounts, and failing the deployment over that made the check useless on
# the configuration production actually runs.
#
# `users` only has to hold an account someone can log in as: role owner (the only role permitted
# until per-owner query isolation exists), not disabled, and with a password set. An agent row with
# no password hash is not somebody who can log in.
if [ -n "$DB" ]; then
  owners=$($D exec "$DB" psql -U "$DB_USER" -d "$DB_NAME" -tAc \
    "select count(*) from users where role = 'owner' and disabled_at is null and password_hash is not null" 2>/dev/null)
  asked=$?
  owners=$(printf '%s' "$owners" | tr -d '\r\n ')
  if [ "$asked" -ne 0 ] || [ -z "$owners" ]; then
    # Not the same thing as an answer of zero, and the difference matters: a failed query means the
    # question was never answered, which could be a missing migration or a database that is not up.
    fail "could not ask the database how many accounts exist, so this is unknown rather than none; check that the migrations ran"
  else
    case "$MODE" in
      cloudflare)
        if [ "$owners" = "0" ]; then
          pass "no native account, which is correct in cloudflare mode: Cloudflare Access decides"
        else
          pass "${owners} owner account(s) exist, unused while Cloudflare Access is the boundary"
        fi
        ;;
      both|native)
        if [ "$owners" = "0" ]; then
          fail "${MODE} mode needs a native login and no usable owner account exists: create one with 'opportunity user create'"
        else
          pass "an owner account exists, so there is someone to log in as"
        fi
        ;;
      *)
        pass "${owners} owner account(s); ${MODE} mode does not use them" ;;
    esac
  fi
fi

echo
echo "== health and access (mode: ${MODE}) =="
if [ -n "$APP" ]; then
  health=$($D exec "$APP" curl -s --connect-timeout 2 --max-time 5 -o /dev/null -w '%{http_code}' http://127.0.0.1:8787/health 2>/dev/null)
  [ "$health" = "200" ] && pass "/health answers 200" || fail "/health answered ${health:-nothing}"
  ready=$($D exec "$APP" curl -s --connect-timeout 2 --max-time 2 -o /dev/null -w '%{http_code}' http://127.0.0.1:8787/ready 2>/dev/null) || ready=000
  [ "$ready" = "200" ] && pass "/ready answers 200" || fail "/ready answered ${ready:-nothing}"

  # True in every production mode: no private data without an identity. This is the check that
  # matters, and it does not vary.
  for path in /api/stats /api/applications /api/facts /api/profiles /api/auth/session; do
    code=$($D exec "$APP" curl -s --connect-timeout 2 --max-time 5 -o /dev/null -w '%{http_code}' "http://127.0.0.1:8787${path}" 2>/dev/null)
    if [ "$code" = "401" ]; then
      pass "${path} refuses an unauthenticated caller"
    else
      fail "${path} answered ${code:-nothing} to an unauthenticated caller, expected 401"
    fi
  done

  # The interface itself differs by design. Where Cloudflare is the boundary it sits behind the
  # gate; where a native session is, the login page has to load before anyone can log in.
  root=$($D exec "$APP" curl -s --connect-timeout 2 --max-time 5 -o /dev/null -w '%{http_code}' http://127.0.0.1:8787/ 2>/dev/null)
  login=$($D exec "$APP" curl -s --connect-timeout 2 --max-time 5 -o /dev/null -w '%{http_code}' http://127.0.0.1:8787/login 2>/dev/null)
  case "$MODE" in
    cloudflare)
      [ "$root" = "401" ] && pass "/ stays behind Cloudflare Access" || fail "/ answered ${root:-nothing}, expected 401 in cloudflare mode"
      ;;
    both|native)
      [ "$login" = "200" ] && pass "/login is served so it can be logged in to" || fail "/login answered ${login:-nothing}, expected 200 in ${MODE} mode"
      [ "$root" = "200" ] && pass "/ serves the public landing page" || fail "/ answered ${root:-nothing}, expected 200 in ${MODE} mode"
      # A deep link into the private side is the same shell; what it shows is decided by the API,
      # which is checked above. What matters here is that it resolves rather than 404ing.
      workspace=$($D exec "$APP" curl -s --connect-timeout 2 --max-time 5 -o /dev/null -w '%{http_code}' http://127.0.0.1:8787/workspace 2>/dev/null)
      [ "$workspace" = "200" ] && pass "/workspace deep-links into the application" || fail "/workspace answered ${workspace:-nothing}"
      # The shell is public here, so it had better be a shell: one bundle, no data baked in.
      for path in / /login /workspace; do
        body=$($D exec "$APP" curl -s --connect-timeout 2 --max-time 5 "http://127.0.0.1:8787${path}" 2>/dev/null | head -c 8000)
        case "$body" in
          *@*) fail "${path} contains an address; the public shell must carry no personal data" ;;
          *) pass "${path} carries no address" ;;
        esac
      done

      ;;
    loopback)
      fail "ACCESS_MODE=loopback on a deployed machine means nothing is checking identity"
      ;;
    *)
      fail "unknown ACCESS_MODE '${MODE}' in .env.prod"
      ;;
  esac
fi

echo
echo "== the database is not reachable from outside =="
if [ -n "$DB" ]; then
  published=$($D port "$DB" 2>/dev/null)
  [ -z "$published" ] && pass "no published ports on the database" || fail "the database publishes: $published"
  ready=$($D exec "$DB" pg_isready -U "$DB_USER" 2>/dev/null | tail -1)
  printf '  %s\n' "$ready"
fi

echo
echo "== secrets =="
if [ -f .env.prod ]; then
  mode=$(stat -c '%a' .env.prod 2>/dev/null || stat -f '%Lp' .env.prod 2>/dev/null)
  [ "$mode" = "600" ] && pass ".env.prod is readable only by its owner" || fail ".env.prod is mode ${mode}, expected 600"
  missing=""
  for key in POSTGRES_PASSWORD CF_ACCESS_TEAM_DOMAIN CF_ACCESS_AUD ACCESS_ALLOWED_EMAILS CLOUDFLARE_TUNNEL_TOKEN; do
    grep -q "^${key}=.\+" .env.prod || missing="${missing} ${key}"
    grep -q "^${key}=REPLACE_WITH" .env.prod && missing="${missing} ${key}(placeholder)"
  done
  [ -z "$missing" ] && pass "every required secret is set" || fail "unset or still a placeholder:${missing}"
else
  fail "no .env.prod in $(pwd)"
fi

echo
echo "== what it is using =="
$D stats --no-stream --format '  {{.Name}}  {{.MemUsage}}  cpu {{.CPUPerc}}' 2>/dev/null
printf '  host  '; free -m | awk '/^Mem:/ {printf "%s MB used, %s MB available of %s MB\n", $3, $7, $2}'
printf '  swap  '; free -m | awk '/^Swap:/ {printf "%s MB used of %s MB\n", $3, $2}'
printf '  disk  '; df -h / | awk 'NR==2 {printf "%s used of %s, %s free\n", $3, $2, $4}'
printf '  load  '; cut -d' ' -f1-3 /proc/loadavg

available=$(free -m | awk '/^Mem:/ {print $7}')
if [ "${available:-0}" -lt 120 ]; then
  fail "only ${available} MB of memory available; a limit reached is a container killed"
else
  pass "${available} MB of memory available"
fi

echo
echo "== kernel memory pressure since boot =="
if dmesg >/dev/null 2>&1; then
  oom=$(dmesg | grep -c "Out of memory")
  if [ "$oom" -eq 0 ]; then
    pass "no out-of-memory kills"
  else
    fail "${oom} out-of-memory kills since boot"
  fi
else
  printf '  note  reading the kernel log needs root here: sudo dmesg | grep -i "out of memory"\n'
fi
restarts=$($D inspect "$APP" "$DB" --format '{{.Name}}={{.RestartCount}}' 2>/dev/null | tr '\n' ' ')
printf '  restarts %s\n' "$restarts"

echo
if [ "$problems" -eq 0 ]; then
  echo "All checks passed."
else
  echo "${problems} check(s) failed."
fi
exit "$problems"
