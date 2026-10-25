#!/usr/bin/env bash
# Build, check and package the production image for a machine that cannot compile.
#
# The target host has 1 GiB of memory and a quarter of a shared core. It runs images; it never
# builds one. Everything that needs a toolchain happens here, and what leaves this script is a file
# plus a checksum.
#
#   ./scripts/release-image.sh                 # linux/amd64, runtime target, no browser
set -euo pipefail

cd "$(dirname "$0")/.."

PLATFORM="${PLATFORM:-linux/amd64}"
TARGET="${TARGET:-runtime}"
[[ "$PLATFORM" == linux/amd64 && "$TARGET" == runtime ]] || { echo 'Production releases require linux/amd64 and target runtime' >&2; exit 1; }
REVISION="$(git rev-parse HEAD)"
[[ "${REVIEWED_SHA:-}" == "$REVISION" && "$REVISION" =~ ^[0-9a-f]{40}$ ]] || { echo 'Set REVIEWED_SHA to the complete reviewed HEAD' >&2; exit 1; }
[[ -z "$(git status --porcelain --untracked-files=all)" ]] || { echo 'Production image creation requires a clean reviewed checkout' >&2; exit 1; }
[[ "${VERSION:-$REVISION}" == "$REVISION" ]] || { echo 'VERSION must equal REVIEWED_SHA' >&2; exit 1; }
BUILD_LOCK="${TMPDIR:-/tmp}/opportunity-image-${REVISION}.lock"
umask 077
mkdir "$BUILD_LOCK" || { echo 'A build lock exists for this revision; inspect it before retrying' >&2; exit 1; }
trap 'rmdir "$BUILD_LOCK"' EXIT
OUT_DIR="${RELEASE_DIR:-dist}"
ARCH=amd64
TAG="opportunity-app:${REVISION}-amd64"
ARCHIVE="${OUT_DIR}/opportunity-app-${REVISION}-amd64.tar.gz"
if docker image inspect "$TAG" >/dev/null 2>&1 || [[ -e "$ARCHIVE" || -e "${ARCHIVE}.sha256" ]]; then
  echo 'Release tag or archive already exists; do not replace a reviewed artifact' >&2
  exit 1
fi

echo "building ${TAG} for ${PLATFORM} (target: ${TARGET})"
docker buildx build --platform linux/amd64 --target runtime --provenance=false --sbom=false \
  --build-arg "GIT_REVISION=$REVISION" -t "$TAG" --load .
[[ -z "$(git status --porcelain --untracked-files=all)" && "$(git rev-parse HEAD)" == "$REVISION" ]] || { echo 'Checkout changed during build; discard this artifact' >&2; exit 1; }
[[ "$(docker image inspect "$TAG" --format '{{index .Config.Labels "org.opencontainers.image.revision"}}')" == "$REVISION" ]] || { echo 'Wrong built revision' >&2; exit 1; }

BUILT_ID="$(docker image inspect "$TAG" --format '{{.Id}}')"

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

ARTIFACT="$(python3 scripts/release-artifact.py "$ARCHIVE" --revision "$REVISION")"
read -r verified_tag identities checksum <<< "$ARTIFACT"
[[ "$verified_tag" == "$TAG" && ",$identities," == *",$BUILT_ID,"* \
   && "$(docker image inspect "$TAG" --format '{{.Id}}')" == "$BUILT_ID" ]] || { echo 'Built and packaged image identities differ' >&2; exit 1; }
printf '%s\n' "$ARTIFACT"

extracted="$(docker image inspect "$TAG" --format '{{.Size}}')"
printf '\n%s\n' "ready to ship:"
printf '  tag        %s\n' "$TAG"
printf '  archive    %s (%s)\n' "$ARCHIVE" "$(du -h "$ARCHIVE" | cut -f1)"
printf '  checksum   %s\n' "$(cut -d' ' -f1 "${ARCHIVE}.sha256")"
# `docker image inspect` reports the manifest size; `docker system df -v` on the target reports what
# the layers actually occupy, which for this image has measured about four times larger.
printf '  manifest   %s MB (expect roughly 150-180 MB unpacked on the target)\n' "$((extracted / 1000000))"
printf '\nTransfer it with scripts/deploy-vm.sh, which verifies this checksum on the far side.\n'
