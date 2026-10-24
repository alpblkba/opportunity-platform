#!/usr/bin/env bash
# Restore a dump into a named database.
#
# Refuses to touch an existing database unless told to, because the usual reason to run this is a rehearsal into a throwaway database, and the usual disaster is pointing it at the real one.
set -euo pipefail

cd "$(dirname "$0")/.."

DUMP="${1:?usage: restore.sh <dump file> [target database]}"
TARGET="${2:-opportunity_restore_rehearsal}"
COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.prod.yml}"
ENV_FILE="${ENV_FILE:-.env.prod}"
ALLOW_OVERWRITE="${ALLOW_OVERWRITE:-no}"

compose() { docker compose -f "$COMPOSE_FILE" ${ENV_FILE:+--env-file "$ENV_FILE"} "$@"; }

if [[ -f "$ENV_FILE" ]]; then set -a; source "$ENV_FILE"; set +a; fi
DB_USER="${POSTGRES_USER:-opportunity}"

[[ -f "$DUMP" ]] || { echo "no such dump: $DUMP" >&2; exit 1; }

if [[ -f "${DUMP}.sha256" ]]; then
  echo "verifying the dump checksum"
  if command -v shasum >/dev/null; then shasum -a 256 -c "${DUMP}.sha256"; else sha256sum -c "${DUMP}.sha256"; fi
fi

# An encrypted dump is decrypted to a file only this user can read, for as long as the restore takes.
PLAINTEXT="$DUMP"
if [[ "$DUMP" == *.gpg ]]; then
  echo "decrypting"
  PLAINTEXT="$(mktemp -t opportunity-restore)"
  chmod 600 "$PLAINTEXT"
  trap 'shred -u "$PLAINTEXT" 2>/dev/null || rm -f "$PLAINTEXT"' EXIT
  gpg --batch --yes --quiet --decrypt --output "$PLAINTEXT" "$DUMP"
fi

exists=$(compose exec -T db psql -U "$DB_USER" -d postgres -tAc \
  "select 1 from pg_database where datname = '${TARGET}'" || true)
if [[ "$exists" == "1" && "$ALLOW_OVERWRITE" != "yes" ]]; then
  echo "database '${TARGET}' already exists. Refusing to overwrite it." >&2
  echo "Rerun with ALLOW_OVERWRITE=yes only if you are certain, or choose a different target name." >&2
  exit 1
fi

if [[ "$exists" == "1" ]]; then
  compose exec -T db psql -U "$DB_USER" -d postgres -c "drop database \"${TARGET}\" with (force)"
fi
compose exec -T db psql -U "$DB_USER" -d postgres -c "create database \"${TARGET}\""
compose exec -T db pg_restore -U "$DB_USER" -d "$TARGET" --no-owner < "$PLAINTEXT"

echo
echo "restored into '${TARGET}'. Row counts:"
compose exec -T db psql -U "$DB_USER" -d "$TARGET" -tAc \
  "select 'opportunities: '||count(*) from opportunities
   union all select 'postings: '||count(*) from postings
   union all select 'sources: '||count(*) from sources
   union all select 'profiles: '||count(*) from profiles
   union all select 'applications: '||count(*) from applications"
