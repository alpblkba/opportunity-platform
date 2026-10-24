#!/usr/bin/env bash
# Move the local development database to the deployed one.
#
# Reads from the local Compose stack and writes into the production stack. It never drops or truncates the local database: the source is only ever read.
set -euo pipefail

cd "$(dirname "$0")/.."

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
DUMP="backups/local-export-${STAMP}.dump"
LOCAL_USER="${LOCAL_POSTGRES_USER:-opportunity}"
LOCAL_DB="${LOCAL_POSTGRES_DB:-opportunity}"
ENV_FILE="${ENV_FILE:-.env.prod}"

mkdir -p backups

echo "1/4 exporting the local database, read only"
docker compose -f docker-compose.yml exec -T db \
  pg_dump -U "$LOCAL_USER" -d "$LOCAL_DB" --format=custom --no-owner > "$DUMP"
echo "    wrote $DUMP ($(wc -c < "$DUMP" | tr -d ' ') bytes)"

echo "2/4 starting the production database"
docker compose -f docker-compose.prod.yml --env-file "$ENV_FILE" up -d db
until docker compose -f docker-compose.prod.yml --env-file "$ENV_FILE" exec -T db pg_isready -q; do sleep 2; done

echo "3/4 loading it into the production database"
set -a; source "$ENV_FILE"; set +a
docker compose -f docker-compose.prod.yml --env-file "$ENV_FILE" exec -T db \
  pg_restore -U "$POSTGRES_USER" -d "${POSTGRES_DB:-opportunity}" --no-owner --clean --if-exists < "$DUMP"

echo "4/4 comparing row counts"
for table in opportunities postings sources profiles applications; do
  local_count=$(docker compose -f docker-compose.yml exec -T db psql -U "$LOCAL_USER" -d "$LOCAL_DB" -tAc "select count(*) from $table" 2>/dev/null || echo "n/a")
  cloud_count=$(docker compose -f docker-compose.prod.yml --env-file "$ENV_FILE" exec -T db psql -U "$POSTGRES_USER" -d "${POSTGRES_DB:-opportunity}" -tAc "select count(*) from $table" 2>/dev/null || echo "n/a")
  printf '    %-16s local=%-8s cloud=%-8s %s\n' "$table" "$local_count" "$cloud_count" \
    "$([[ "$local_count" == "$cloud_count" ]] && echo match || echo DIFFERENT)"
done

echo
echo "The local database was only read. It is unchanged."
