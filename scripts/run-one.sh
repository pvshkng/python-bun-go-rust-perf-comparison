#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

SERVER="${1:-}"
K6NAME="${2:-baseline}"

if [ -z "$SERVER" ]; then
    echo "usage: scripts/run-one.sh <server> [baseline|stress|soak]" >&2
    echo "servers: $ALL_SERVERS" >&2
    exit 1
fi

trap cleanup EXIT INT TERM

TS="$(date +%Y%m%d_%H%M%S)"
OUTDIR="$ROOT/results/${TS}_${SERVER}_${K6NAME}"

run_bench "$SERVER" "$K6NAME" "$OUTDIR"
