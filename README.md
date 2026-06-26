# Python vs Bun vs Go vs Rust Performance Comparison

Each backend exposes `POST /chat` on `:8080`, persists nothing by default,
forwards the conversation to the stub, and streams the SSE response back.

## Components

- `stub/` mock LLM at `:9090` (`POST /v1/chat/completions`). Detects
  `generate N paragraphs of lorem ipsum` in the last user message and returns N
  paragraphs (digits or words like `three`); otherwise returns the first
  paragraph. Honors `stream` (SSE token-by-token, default) vs non-stream (single
  JSON). Tune think-time with `TOKEN_DELAY_MS`.
- `servers/` `go-http`, `go-gin`, `go-fiber`, `bun-http`, `bun-hono`,
  `rust-http` (std net only), `rust-actix`, `rust-axum`.
- `k6/`  `baseline`, `stress`, `soak` (and `smoke` for a 6s check).
- `scripts/` orchestration + pidstat/gnuplot measurement.

## Run the benchmark (one command)

```sh
# one backend: backend + stub + k6, with pidstat charts. scenario: baseline|stress|soak|smoke
scripts/run-one.sh go-http baseline

# every backend sequentially + comparison charts
scripts/run-all.sh baseline
```

Results are written to `results/<datetime>_<server>_<scenario>/` (per-run:
`cpu.png`, `memory.png`, `summary.txt`, `k6_summary.json`) and, for `run-all`,
to `results/<datetime>_all_<scenario>/` with `comparison.txt`, `throughput.png`,
`latency_p95.png`, `memory_max.png`.

## With Postgres

```sh
DATABASE_URL=postgres://... ./migrations/migrate.sh   # needs psql
```

Start any server with `--db` (Go/Rust) or `--db` (Bun) and `DATABASE_URL` set;
the orchestration scripts run without a database.

## Run a server manually

```sh
cd stub && go run .                                     # stub on :9090
export STUB_URL=http://localhost:9090/v1/chat/completions
cd servers/go-http   && go build -o /tmp/s . && /tmp/s  # or: go run .
cd servers/bun-hono  && bun install && bun main.ts
cd servers/rust-axum && cargo run --release
k6 run k6/baseline.js
```
