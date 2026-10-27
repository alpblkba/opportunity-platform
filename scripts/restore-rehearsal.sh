#!/usr/bin/env bash
# Prove that a backup from deploy-vm.sh --fetch-backup restores, without going near a real database:
# checksums first, then decryption with the owner's key, a full pg_restore into a throwaway
# PostgreSQL container, the private tree unpacked into a throwaway directory, and every CV the
# database references found in it with the bytes it recorded. Success writes restore-verified.json
# beside the backup, which deploy-vm.sh --migration-rollout requires.
#
#   ./scripts/restore-rehearsal.sh ~/Backups/opportunity/opportunity-<stamp>
set -euo pipefail
umask 077
cd "$(dirname "$0")/.."
DIR="${1:?usage: restore-rehearsal.sh <backup directory>}"
IMAGE="${REHEARSAL_POSTGRES_IMAGE:-postgres:16-alpine}"
NAME="opportunity-rehearsal-$$-${RANDOM}"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/opportunity-rehearsal.XXXXXX")"
cleanup() { docker rm -f "$NAME" >/dev/null 2>&1 || true; rm -rf "$WORK"; }
trap cleanup EXIT

rm -f "$DIR/restore-verified.json"
python3 scripts/backup-stream.py verify "$DIR"
gpg --batch --quiet --decrypt --output "$WORK/database.dump" "$DIR/database.dump.gpg"
gpg --batch --quiet --decrypt --output "$WORK/private.tar.gz" "$DIR/private.tar.gz.gpg"

docker run -d --rm --name "$NAME" --network none -e POSTGRES_USER=rehearsal -e POSTGRES_PASSWORD=rehearsal-only \
  -e POSTGRES_DB=rehearsal "$IMAGE" >/dev/null
for _ in $(seq 1 60); do
  docker exec "$NAME" pg_isready -U rehearsal -d rehearsal >/dev/null 2>&1 && break
  sleep 1
done
docker exec -i "$NAME" pg_restore -U rehearsal -d rehearsal --no-owner --single-transaction --exit-on-error < "$WORK/database.dump"
sql() { docker exec "$NAME" psql -U rehearsal -d rehearsal -XAt -v ON_ERROR_STOP=1 -F "$(printf '\t')" -c "$1"; }
sql "select version || ':' || encode(checksum, 'hex') from _sqlx_migrations where success order by version" > "$WORK/migrations.txt"
sql "select count(*) from information_schema.tables where table_schema = 'public'" > "$WORK/tables.txt"
sql "select file_path, content_hash from documents" > "$WORK/documents.tsv"
mkdir "$WORK/files"
python3 -c 'import sys, tarfile; tarfile.open(sys.argv[1]).extractall(sys.argv[2], filter="data")' "$WORK/private.tar.gz" "$WORK/files"
python3 scripts/backup-stream.py attest "$DIR" "$WORK"
