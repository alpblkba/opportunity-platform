#!/usr/bin/env bash
# Build, check and package the production image for a machine that cannot compile.
#
# The target host has 1 GiB of memory and a quarter of a shared core. It runs images; it never
# builds one. Everything that needs a toolchain happens here, and what leaves this script is a file
# plus a checksum.
#
#   ./scripts/release-image.sh                 # linux/amd64, runtime target, no browser
#   PLATFORM=linux/arm64 ./scripts/release-image.sh
#   TARGET=browser ./scripts/release-image.sh   # the browser-capable image, which the 1 GiB
#                                               # profile does not use
set -euo pipefail

cd "$(dirname "$0")/.."

PLATFORM="${PLATFORM:-linux/amd64}"
TARGET="${TARGET:-runtime}"
OUT_DIR="${RELEASE_DIR:-dist}"
ARCH="${PLATFORM##*/}"

# The version is what a rollback asks for by name, so two builds must never answer to the same name.
#
# A commit alone cannot do that here: this repository is deliberately not committed between builds,
# so several images would share one revision and a rollback would land on whichever was loaded last.
# The timestamp makes every build distinct; the revision says what it came from; `dirty` says the
# tree had changes that no revision describes. Nothing requires a commit.
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
REVISION="$(git rev-parse --short HEAD 2>/dev/null || echo nogit)"
if ! git diff --quiet 2>/dev/null || ! git diff --cached --quiet 2>/dev/null \
   || [[ -n "$(git ls-files --others --exclude-standard 2>/dev/null)" ]]; then
  REVISION="${REVISION}-dirty"
fi
VERSION="${VERSION:-${STAMP}-${REVISION}}"
TAG="opportunity-app:${VERSION}-${ARCH}"
if [[ "$TARGET" != "runtime" ]]; then
  TAG="opportunity-app:${VERSION}-${TARGET}-${ARCH}"
fi
ARCHIVE="${OUT_DIR}/$(echo "$TAG" | tr ':/' '--').tar.gz"

echo "building ${TAG} for ${PLATFORM} (target: ${TARGET})"
docker buildx build --platform "$PLATFORM" --target "$TARGET" -t "$TAG" --load .

# What the image claims and what it is have to agree, because the wrong architecture fails on the
# far side of a slow transfer rather than here.
built_arch="$(docker image inspect "$TAG" --format '{{.Architecture}}')"
[[ "$built_arch" == "$ARCH" ]] || { echo "built ${built_arch}, expected ${ARCH}" >&2; exit 1; }

echo "checking the image"
docker run --rm --platform "$PLATFORM" --entrypoint sh "$TAG" -c '
  set -e
  opportunity --version
  test -f /app/apps/web/dist/index.html || { echo "no built frontend in the image" >&2; exit 1; }
  # Nothing private may ride along. The build context excludes it, and this is the check that the
  # exclusion still holds.
  found=$(find / -xdev \( -name ".env" -o -name ".env.prod" -o -name "*.dump" -o -name "*.p12" \) 2>/dev/null | head -5)
  if [ -n "$found" ]; then echo "secrets in the image: $found" >&2; exit 1; fi
  test -z "$(ls -A /app/private 2>/dev/null | grep -v browser)" || { echo "private tree is not empty" >&2; exit 1; }
'

mkdir -p "$OUT_DIR"
echo "packaging ${ARCHIVE}"
docker save "$TAG" | gzip -9 > "$ARCHIVE"
if command -v shasum >/dev/null; then
  shasum -a 256 "$ARCHIVE" > "${ARCHIVE}.sha256"
else
  sha256sum "$ARCHIVE" > "${ARCHIVE}.sha256"
fi

extracted="$(docker image inspect "$TAG" --format '{{.Size}}')"
printf '\n%s\n' "ready to ship:"
printf '  tag        %s\n' "$TAG"
printf '  archive    %s (%s)\n' "$ARCHIVE" "$(du -h "$ARCHIVE" | cut -f1)"
printf '  checksum   %s\n' "$(cut -d' ' -f1 "${ARCHIVE}.sha256")"
# `docker image inspect` reports the manifest size; `docker system df -v` on the target reports what
# the layers actually occupy, which for this image has measured about four times larger.
printf '  manifest   %s MB (expect roughly 150-180 MB unpacked on the target)\n' "$((extracted / 1000000))"
printf '\nTransfer it with scripts/deploy-vm.sh, which verifies this checksum on the far side.\n'
