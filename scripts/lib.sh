#!/usr/bin/env bash

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

ALL_SERVERS="go-http go-gin go-fiber bun-http bun-hono rust-http rust-actix rust-axum"

STUB_PORT="${STUB_PORT:-9090}"
SERVER_PORT=8080
export STUB_URL="http://localhost:${STUB_PORT}/v1/chat/completions"
export TOKEN_DELAY_MS="${TOKEN_DELAY_MS:-0}"

STUB_PID=""
SERVER_PID=""
PIDSTAT_PID=""

cleanup() {
    [ -n "$PIDSTAT_PID" ] && kill "$PIDSTAT_PID" 2>/dev/null
    [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null
    [ -n "$STUB_PID" ] && kill "$STUB_PID" 2>/dev/null
    wait 2>/dev/null
    STUB_PID=""; SERVER_PID=""; PIDSTAT_PID=""
}

build_stub() {
    ( cd "$ROOT/stub" && go build -o "$ROOT/.bin/stub" . )
}

build_server() {
    local s="$1"
    case "$s" in
        bun-*)  ( cd "$ROOT/servers/$s" && bun install >/dev/null 2>&1 ) ;;
        go-*)   ( cd "$ROOT/servers/$s" && go build -o "$ROOT/.bin/$s" . ) ;;
        rust-*) ( cd "$ROOT/servers/$s" && cargo build --release >/dev/null 2>&1 ) ;;
        *) echo "unknown server: $s" >&2; return 1 ;;
    esac
}

start_stub() {
    LOREM_PATH="$ROOT/stub/data/lorem_ipsum.json" PORT="$STUB_PORT" \
        "$ROOT/.bin/stub" >"$1" 2>&1 &
    STUB_PID=$!
}

start_server() {
    local s="$1" log="$2"
    case "$s" in
        bun-*)  ( cd "$ROOT/servers/$s" && exec bun main.ts ) >"$log" 2>&1 & ;;
        go-*)   "$ROOT/.bin/$s" >"$log" 2>&1 & ;;
        rust-*) "$ROOT/servers/$s/target/release/$s" >"$log" 2>&1 & ;;
    esac
    SERVER_PID=$!
}

wait_port() {
    local port="$1" tries=0
    while [ $tries -lt 100 ]; do
        if curl -s -o /dev/null --max-time 2 "localhost:${port}/chat" -d '{"message":"warmup"}' 2>/dev/null; then
            return 0
        fi
        sleep 0.2; tries=$((tries + 1))
    done
    return 1
}

parse_pidstat() {
    local raw="$1" cpu="$2" mem="$3"
    awk '$1 ~ /^[0-9][0-9]:[0-9][0-9]:[0-9][0-9]$/ {
        print c, $8 >> "'"$cpu"'";
        print c, $13/1024 >> "'"$mem"'";
        c++;
    }' c=0 "$raw"
}

plot_run() {
    local outdir="$1" server="$2" k6name="$3"
    gnuplot <<EOF
set terminal pngcairo size 1000,500
set grid
set xlabel "elapsed seconds"
set output "${outdir}/cpu.png"
set title "CPU usage - ${server} (${k6name})"
set ylabel "%CPU (100% = 1 core)"
plot "${outdir}/cpu.dat" using 1:2 with lines lw 2 lc rgb "#cc3333" title "cpu"
set output "${outdir}/memory.png"
set title "Memory (RSS) - ${server} (${k6name})"
set ylabel "RSS MB"
plot "${outdir}/mem.dat" using 1:2 with lines lw 2 lc rgb "#3366cc" title "rss"
EOF
}

write_summary() {
    local outdir="$1" server="$2" k6name="$3" json="$outdir/k6_summary.json"
    {
        echo "server:   $server"
        echo "scenario: $k6name"
        if [ -f "$json" ]; then
            jq -r '
              "requests: \(.metrics.http_reqs.count // 0)",
              "req/s:    \(.metrics.http_reqs.rate // 0 | (.*100|round/100))",
              "avg ms:   \(.metrics.http_req_duration.avg // 0 | (.*100|round/100))",
              "p95 ms:   \(.metrics.http_req_duration["p(95)"] // 0 | (.*100|round/100))",
              "max ms:   \(.metrics.http_req_duration.max // 0 | (.*100|round/100))",
              "fail %:   \((.metrics.http_req_failed.value // 0) * 100 | (.*100|round/100))"
            ' "$json"
        fi
        if [ -f "$outdir/cpu.dat" ]; then
            awk '{s+=$2; if($2>m)m=$2; n++} END{if(n)printf "cpu avg:  %.1f\ncpu max:  %.1f\n", s/n, m}' "$outdir/cpu.dat"
        fi
        if [ -f "$outdir/mem.dat" ]; then
            awk '{if($2>m)m=$2} END{printf "mem max:  %.1f MB\n", m}' "$outdir/mem.dat"
        fi
    } | tee "$outdir/summary.txt"
}

run_bench() {
    local server="$1" k6name="$2" outdir="$3"
    mkdir -p "$outdir"

    echo ">> building $server"
    build_stub
    build_server "$server" || return 1

    echo ">> starting stub + $server"
    start_stub "$outdir/stub.log"
    sleep 0.5
    start_server "$server" "$outdir/server.log"

    if ! wait_port "$SERVER_PORT"; then
        echo "!! $server did not become ready" >&2
        cat "$outdir/server.log" >&2
        cleanup
        return 1
    fi

    echo ">> measuring with pidstat + running k6/$k6name"
    S_TIME_FORMAT=ISO pidstat -h -u -r -p "$SERVER_PID" 1 >"$outdir/pidstat.raw" 2>/dev/null &
    PIDSTAT_PID=$!

    k6 run --summary-export "$outdir/k6_summary.json" \
        "$ROOT/k6/${k6name}.js" 2>&1 | tee "$outdir/k6_stdout.txt"

    cleanup

    : >"$outdir/cpu.dat"; : >"$outdir/mem.dat"
    parse_pidstat "$outdir/pidstat.raw" "$outdir/cpu.dat" "$outdir/mem.dat"
    plot_run "$outdir" "$server" "$k6name"
    write_summary "$outdir" "$server" "$k6name"
    echo ">> done: $outdir"
}
