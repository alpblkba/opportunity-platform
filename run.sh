#!/usr/bin/env bash
# Start the whole application locally: database, migrations, built frontend and API, on one port.
set -euo pipefail

cd "$(dirname "$0")"

if [[ ! -f .env ]]; then
  cp .env.example .env
  echo "created .env from .env.example"
fi
# shellcheck disable=SC1091
set -a; source .env; set +a

API_HOST="${API_HOST:-127.0.0.1}"
API_PORT="${API_PORT:-8787}"
DATABASE_URL="${DATABASE_URL:?DATABASE_URL must be set in .env}"

missing=()
command -v cargo >/dev/null || missing+=("cargo (https://rustup.rs)")
command -v npm >/dev/null || missing+=("npm (Node.js 22 or newer)")
if ((${#missing[@]})); then
  printf 'missing prerequisite: %s\n' "${missing[@]}" >&2
  exit 1
fi

# A database the owner already runs is left alone; the bundled container is only started when nothing answers on the port.
if ! (exec 3<>/dev/tcp/127.0.0.1/5432) 2>/dev/null; then
  if command -v docker >/dev/null && docker info >/dev/null 2>&1; then
    echo "starting the bundled PostgreSQL container"
    docker compose up -d db
    for _ in $(seq 1 40); do
      docker compose exec -T db pg_isready -U opportunity -d opportunity >/dev/null 2>&1 && break
      sleep 1
    done
  else
    echo "no PostgreSQL on 127.0.0.1:5432 and no running Docker daemon. Start Docker Desktop, or point DATABASE_URL at your own PostgreSQL." >&2
    exit 1
  fi
fi

echo "building the frontend"
npm --prefix apps/web ci --silent
npm --prefix apps/web run build --silent

echo "building the server"
cargo build --release --bin opportunity

./target/release/opportunity migrate

# Registering the verified seed sources is skipped once any source exists, so an owner's edits are never overwritten.
if [[ "$(./target/release/opportunity source list | wc -l | tr -d ' ')" == "0" ]]; then
  echo "importing verified seed sources"
  ./target/release/opportunity source import config/sources.seed.toml
fi

echo
echo "open http://${API_HOST}:${API_PORT}"
echo "press ctrl-c to stop the server; the database container keeps running until ./stop.sh"
exec ./target/release/opportunity serve --host "$API_HOST" --port "$API_PORT"
