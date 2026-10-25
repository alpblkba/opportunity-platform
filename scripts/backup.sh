#!/usr/bin/env bash
# Back up the database and the private tree.
#
# The dump is custom-format so it can be restored selectively and in parallel. The private tree holds the CV, raw source snapshots and browser screenshots, which are not in the database and would otherwise be lost.
set -euo pipefail
umask 077

cd "$(dirname "$0")/.."

COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.prod.yml}"
ENV_FILE="${ENV_FILE:-.env.prod}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
# On the host, beside the repository, because a dump that has to leave the machine is easier to copy from a directory than from inside a named volume. Override with BACKUP_DIR.
OUT_DIR="${BACKUP_DIR:-backups}"

# Encryption, and who can undo it.
#
# BACKUP_ENCRYPT_TO names an OpenPGP recipient. Public-key rather than a passphrase on purpose: the
# machine being backed up holds only the public half, so whoever takes the machine cannot read its
# backups, and the owner's private key never goes near the server.
ENCRYPT_TO="${BACKUP_ENCRYPT_TO:-}"
REQUIRE_ENCRYPTION="${BACKUP_REQUIRE_ENCRYPTION:-no}"
if [[ -z "$ENCRYPT_TO" && "$REQUIRE_ENCRYPTION" == "yes" ]]; then
  echo "BACKUP_REQUIRE_ENCRYPTION=yes but BACKUP_ENCRYPT_TO names no recipient. Refusing to write a readable copy of the CV and the application history." >&2
  exit 1
fi

if [[ -n "$ENCRYPT_TO" ]]; then
  # An encryption failure must not leave the plaintext backup behind.
  trap 'rm -f "${OUT_DIR}/opportunity-${STAMP}.dump" "${OUT_DIR}/private-${STAMP}.tar.gz" "${OUT_DIR}/opportunity-${STAMP}.dump.gpg.tmp" "${OUT_DIR}/private-${STAMP}.tar.gz.gpg.tmp"' EXIT
fi

# Encrypt in place when a recipient is configured, and checksum whatever is actually going to be
# copied off the machine, so verification on the far side checks the file that exists there.
protect() {
  local file="$1"
  if [[ -n "$ENCRYPT_TO" ]]; then
    gpg --batch --yes --trust-model always --encrypt --recipient "$ENCRYPT_TO" --output "${file}.gpg.tmp" "$file" || return 1
    mv "${file}.gpg.tmp" "${file}.gpg" || return 1
    shred -u "$file" 2>/dev/null || rm -f "$file"
    file="${file}.gpg"
  fi
  if command -v shasum >/dev/null; then
    shasum -a 256 "$file" > "${file}.sha256" || return 1
  else
    sha256sum "$file" > "${file}.sha256" || return 1
  fi
  printf '%s' "$file"
}

compose() { docker compose -f "$COMPOSE_FILE" ${ENV_FILE:+--env-file "$ENV_FILE"} "$@"; }

if [[ -f "$ENV_FILE" ]]; then set -a; source "$ENV_FILE"; set +a; fi
DB_USER="${POSTGRES_USER:-opportunity}"
DB_NAME="${POSTGRES_DB:-opportunity}"

mkdir -p "$OUT_DIR"
echo "backing up database ${DB_NAME} as ${DB_USER}"
compose exec -T db pg_dump -U "$DB_USER" -d "$DB_NAME" --format=custom --no-owner \
  > "${OUT_DIR}/opportunity-${STAMP}.dump"

# The checksum is what a restore rehearsal verifies against, so a silently truncated dump is caught before it is relied on.
DUMP="$(protect "${OUT_DIR}/opportunity-${STAMP}.dump")"

if compose ps --services 2>/dev/null | grep -q '^app$'; then
  echo "archiving the private tree"
  if compose exec -T app tar -czf - -C /app private 2>/dev/null > "${OUT_DIR}/private-${STAMP}.tar.gz"; then
    protect "${OUT_DIR}/private-${STAMP}.tar.gz" > /dev/null
  else
    rm -f "${OUT_DIR}/private-${STAMP}.tar.gz"
    echo "The private tree could not be archived; the backup is incomplete." >&2
    exit 1
  fi
fi

echo
echo "wrote:"
ls -la "${OUT_DIR}" | grep "$STAMP"
echo
if [[ -n "$ENCRYPT_TO" ]]; then
  echo "Encrypted to ${ENCRYPT_TO}. Copy them off this machine; only your private key opens them."
else
  echo "NOT ENCRYPTED. These are your CV, your answers and your application history in the clear."
  echo "Set BACKUP_ENCRYPT_TO to an OpenPGP recipient before this leaves the machine."
fi
echo "A dump sitting on the same disk as the database it came from protects against a mistake, not a loss."
