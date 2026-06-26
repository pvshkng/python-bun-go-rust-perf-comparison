#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

K6NAME="${1:-baseline}"
SERVERS="${SERVERS:-$ALL_SERVERS}"

trap cleanup EXIT INT TERM

TS="$(date +%Y%m%d_%H%M%S)"
RUNDIR="$ROOT/results/${TS}_all_${K6NAME}"
mkdir -p "$RUNDIR"

for s in $SERVERS; do
    run_bench "$s" "$K6NAME" "$RUNDIR/$s" || echo "!! $s failed, continuing"
    sleep 2
done

DAT="$RUNDIR/comparison.dat"
echo "# server rate avg_ms p95_ms fail_pct mem_mb" >"$DAT"
for s in $SERVERS; do
    j="$RUNDIR/$s/k6_summary.json"
    [ -f "$j" ] || continue
    mem=$(awk '{if($2>m)m=$2}END{printf "%.1f", m}' "$RUNDIR/$s/mem.dat" 2>/dev/null || echo 0)
    jq -r --arg s "$s" --arg mem "$mem" '
        def r2: .*100|round/100;
        [$s,
         (.metrics.http_reqs.rate // 0 | r2),
         (.metrics.http_req_duration.avg // 0 | r2),
         (.metrics.http_req_duration["p(95)"] // 0 | r2),
         ((.metrics.http_req_failed.value // 0) * 100 | r2),
         ($mem|tonumber)] | @tsv' "$j"
done >>"$DAT"

sed 's/^# //' "$DAT" | column -t >"$RUNDIR/comparison.txt"

gnuplot <<EOF
set terminal pngcairo size 1100,600
set style data histograms
set style fill solid 0.75 border -1
set boxwidth 0.7
set grid ytics
set xtics rotate by -30
set output "${RUNDIR}/throughput.png"
set title "Throughput by server - ${K6NAME}"
set ylabel "req/s"
plot "${DAT}" using 2:xtic(1) lc rgb "#3366cc" notitle
set output "${RUNDIR}/latency_p95.png"
set title "p95 latency by server - ${K6NAME}"
set ylabel "ms"
plot "${DAT}" using 4:xtic(1) lc rgb "#cc3333" notitle
set output "${RUNDIR}/memory_max.png"
set title "Max RSS by server - ${K6NAME}"
set ylabel "MB"
plot "${DAT}" using 6:xtic(1) lc rgb "#33aa55" notitle
EOF

echo
echo "=== comparison (${K6NAME}) ==="
cat "$RUNDIR/comparison.txt"
echo
echo "results: $RUNDIR"
