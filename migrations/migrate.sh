#!/usr/bin/env bash
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
for f in "$DIR"/*.sql; do
    psql "$DATABASE_URL" -f "$f"
done
