#!/bin/sh
# Runs inside a freshly built release image, as root, before release-image.sh packages it:
#
#   docker run --rm -i --user 0 --entrypoint sh <tag> -s < scripts/image-check.sh
#
# The build context is the reviewed commit's tracked files, and this is the check that nothing
# private rode along anyway. It names what it finds, never what is in it.
set -eu

opportunity --version
test -f /app/apps/web/dist/index.html || { echo "no built frontend in the image" >&2; exit 1; }
test -d /app/config || { echo "no configuration in the image" >&2; exit 1; }

# Environment files, deploy credentials and dumps have no business anywhere on the image.
found=$(find / -xdev \( -name '.env' -o -name '.env.*' -o -name 'gha-creds-*.json' -o -name '*.dump' \
  -o -name '*.p12' -o -name '*.pfx' \) ! -name '.env.example' ! -name '.env.prod.example' 2>/dev/null | head -5)
if [ -n "$found" ]; then echo "secrets in the image: $found" >&2; exit 1; fi
# Keys and documents only under /app: the base image's CA bundle is supposed to hold .pem files.
found=$(find /app \( -name '*.pem' -o -name '*.key' -o -name '*.pdf' -o -name '*.docx' -o -name '*.log' \) 2>/dev/null | head -5)
if [ -n "$found" ]; then echo "private files in the image: $found" >&2; exit 1; fi
if [ -n "$(ls -A /app/private 2>/dev/null | grep -v '^browser$')" ]; then echo "private tree is not empty" >&2; exit 1; fi
echo "image contents checked"
