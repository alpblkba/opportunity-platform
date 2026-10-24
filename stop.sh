#!/usr/bin/env bash
# Stop the bundled database container. Data survives in the named volume opportunity-db-data.
set -euo pipefail
cd "$(dirname "$0")"
docker compose stop db
echo "database stopped. Data is kept in the opportunity-db-data volume; 'docker compose down -v' would delete it."
