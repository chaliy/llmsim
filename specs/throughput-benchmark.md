# Throughput Benchmark Specification

## Abstract

This specification defines a dedicated **throughput benchmark** that measures the
peak sustained request rate llmsim can serve, and how that rate scales with
parallelism (CPU cores / async worker threads).

It is distinct from the general load-testing framework (`specs/load-testing.md`),
which validates behavior under realistic, latency-shaped traffic using k6. The
throughput benchmark deliberately strips away simulated latency and large
payloads so the number it reports reflects llmsim's own request-handling
ceiling — the figure suitable for headline claims (e.g. "sustains ~X req/s on
N cores").

The benchmark MUST produce two things:

1. A single quotable headline line (peak sustained RPS + the hardware it ran on).
2. A **parallelisation scaling table**: RPS as a function of server worker-thread
   count, showing how throughput scales as cores are added.

## Motivation

A simulator's most honest performance metric is how much traffic it can absorb,
because that is exactly the property a user relies on when substituting it for a
real API under load. "High-performance" is only credible with a measured number
and a stated methodology. The parallelisation sweep additionally demonstrates
that llmsim actually uses available cores rather than being single-threaded —
the core justification for it being written in Rust on a multi-threaded async
runtime.

## Tool Selection: oha (not k6)

The general load-testing framework uses **k6** (`specs/load-testing.md`) and that
remains the reference tool for scenario-based, latency-shaped tests. The
throughput benchmark instead uses **[`oha`](https://github.com/hatoo/oha)**, and
the distinction is deliberate:

- **The throughput benchmark saturates the server with `instant` latency.** There
  the load generator and the server compete for the same CPU cores. k6 runs each
  iteration through an embedded JS interpreter (goja), which has comparatively
  high per-request overhead — at saturation you risk measuring *k6's* ceiling,
  not llmsim's. (Note: k6 is a Go binary running JS scripts, **not** Node.js.)
- **`oha` is a Rust, MIT-licensed HTTP load generator** purpose-built for maximum
  request rate, with latency histograms (p50/p95/p99) and JSON output. Its
  per-request overhead is near zero, so the measured rate reflects the server.
- For latency-shaped load tests the generator is mostly idle (waiting on
  simulated TTFT), so k6's overhead is irrelevant there — hence k6 stays for
  `specs/load-testing.md` and oha is scoped to throughput only.

`oha` is MIT-licensed (permissive, compatible with this repo's dependency
policy).

## Requirements

### R1: Isolate the server's request-handling ceiling

The benchmark MUST minimize every cost that is not llmsim's own request handling:

- Latency profile MUST be `instant` (0ms TTFT, 0ms inter-token) so the measured
  rate is the server's ceiling, not the latency it was configured to simulate.
- Requests MUST be **non-streaming** `POST /openai/v1/chat/completions`.
- Response size MUST be small and fixed (`target_tokens` ≤ 32, and the request
  body MUST cap output similarly, e.g. `max_tokens` ≤ 32) to minimize
  serialization and token-counting cost.
- Error injection MUST be disabled (all rates `0.0`).

A dedicated config file `benchmarks/config/throughput.toml` MUST encode these
settings so runs are reproducible.

### R2: Parallelisation sweep (server-side)

The benchmark MUST measure throughput across a range of server worker-thread
counts and report each result.

- The server's async runtime worker-thread count MUST be controlled via the
  `TOKIO_WORKER_THREADS` environment variable. The server uses `#[tokio::main]`
  (multi-threaded runtime), which honors this variable with no code changes
  required.
- The sweep MUST cover, at minimum, the powers of two from `1` up to the host's
  logical core count, plus the core count itself if it is not a power of two
  (e.g. on a 4-core host: `1, 2, 4`; on a 6-core host: `1, 2, 4, 6`).
- For each worker-thread count, the runner MUST start a fresh server pinned to
  that count, run the load stage (R3/R4), record peak sustained RPS, then shut
  the server down before the next step.
- Results MUST be emitted as a markdown table with columns:
  `workers | RPS | p50 (ms) | p95 (ms) | p99 (ms) | error %`, plus a derived
  `scaling vs 1 worker` column (RPS at N / RPS at 1) so near-linear scaling is
  visible at a glance.

### R3: Client-side parallelism must not be the bottleneck

The reported RPS must reflect the server, not the load generator. The benchmark
MUST:

- Drive load with `oha` using enough concurrent connections (`-c`) to saturate
  the server under test. `oha` is a closed-loop generator (fixed in-flight
  concurrency), so saturation is reached by raising `-c` until RPS plateaus
  (see R4), not by setting a target arrival rate.
- Reuse connections (HTTP keep-alive, oha default) so the measurement is not
  dominated by TCP setup.
- Run the load generator on the same host as the server (loopback) for the
  headline number, to remove network variance. Remote runs are permitted but
  MUST be labeled as such.
- Ensure the generator itself is not CPU-starved by the server. When sweeping
  low worker-thread counts the host has spare cores for oha; at full-core counts,
  oha and the server share cores — this is acceptable and MUST be noted as a
  caveat (the full-core number is a realistic floor, not an upper bound). Where
  feasible, the operator MAY pin oha and the server to disjoint core sets
  (e.g. `taskset`) for the full-core measurement; if done, it MUST be recorded.

A k6-based cross-check is permitted but MUST be additive and clearly labeled;
oha is the reference tool for this benchmark.

### R4: Finding peak sustained RPS

For each worker-thread count the runner MUST locate the sustainable rate, not a
momentary burst:

- Sweep oha concurrency `-c` upward in steps (e.g. 16, 32, 64, 128, 256) and pick
  the highest achieved RPS that still meets the success criteria in R5. RPS will
  rise then plateau (and eventually latency degrades) — report the plateau.
- Report the RPS **actually achieved** by oha (its `Requests/sec` summary over
  the measured run), not a target.
- Each measured run MUST last ≥ 30s (oha `-z 30s`) to average out scheduler
  jitter and allocator noise. A short warm-up run (≥ 5s) before measurement MUST
  be discarded from the reported figure.

### R5: Success criteria (what counts as "sustained")

A rate qualifies as sustained only if, during the measured run:

- Error/non-2xx rate < 0.1% (oha reports status-code distribution).
- p99 response latency < 50ms. With the `instant` profile there is no simulated
  latency, so anything above this indicates the server is saturating/queuing.

The highest qualifying concurrency step is the reported peak for that worker
count.

### R6: Environment capture (methodology honesty)

Every run MUST record and print the conditions, so the headline number is never
quoted without context:

- CPU model and logical core count (`nproc`, and CPU model where available)
- Total RAM
- OS / kernel and architecture
- Rust toolchain version and llmsim version/commit
- oha version (and k6 version if a cross-check was run)
- Whether load generator and server shared a host (loopback) or were remote, and
  any core pinning applied

This block MUST appear in stdout and in the JSON output (R7).

### R7: Output

The benchmark MUST emit:

- Human-readable stdout: environment block (R6), the per-worker scaling table
  (R2), and a final single headline line, e.g.:
  `PEAK: 18,400 req/s sustained at 4 workers (4 vCPU, p99 2ms, 0.0% errors)`
- A machine-readable JSON file (path via `--output`) containing the environment
  block, an array of per-worker results (each with the winning concurrency,
  achieved RPS, latency percentiles, error rate), and the selected peak. oha's
  own `--json` output for each step SHOULD be captured/aggregated rather than
  re-parsing text. This enables pasting numbers into docs/posts and future
  run-over-run comparison.

### R8: Invocation

The benchmark MUST be runnable as a single command via the existing runner
conventions, e.g.:

```bash
./benchmarks/run-benchmark.sh throughput
./benchmarks/run-benchmark.sh throughput --output throughput.json
./benchmarks/run-benchmark.sh throughput --workers 1,2,4,8     # override sweep
./benchmarks/run-benchmark.sh throughput --concurrency 16,64,256  # override -c steps
```

It MUST follow the existing runner contract from `specs/load-testing.md`:
auto-build (`cargo build --release`), start/stop the server, `--no-server` to
target an existing server, `--output` for JSON, graceful cleanup on exit, and a
clear failure with install guidance if `oha` is not installed
(`cargo install oha`, or the project's package manager).

### R9: Reproducibility and caveats

The benchmark MUST be deterministic in configuration (fixed profile, fixed
payload) so two runs on the same hardware are comparable. The runner MUST print
a one-line caveat reminding the operator that absolute numbers are
hardware-dependent and that the parallelisation table — not the single peak — is
the portable result.

## Implementation Notes

These are guidance for the implementing agent, not binding requirements.

### Files to add

```
benchmarks/
├── config/
│   └── throughput.toml          # instant profile, non-streaming, target_tokens<=32, no errors
├── throughput.sh                # or extend run-benchmark.sh: worker sweep + concurrency sweep + env capture
└── run-benchmark.sh             # extend: add `throughput` profile dispatch
```

`oha` drives requests directly from the CLI, so no JS script is needed (unlike
the k6 scripts in `benchmarks/k6/`). The request body is passed via
`oha --method POST -H 'Content-Type: application/json' -d '<json>'`.

### Worker-thread sweep mechanics

- Determine the sweep set from `nproc` unless `--workers a,b,c` is given.
- For each `W` in the set:
  - `TOKIO_WORKER_THREADS=$W target/release/llmsim serve --host 127.0.0.1 \
     --port $PORT --config benchmarks/config/throughput.toml &`
  - Wait for `/health` to return `{"status":"ok"}`.
  - Warm-up: `oha -z 5s -c 32 ...` (discarded).
  - For each concurrency `C` in the `-c` sweep: run
    `oha -z 30s -c $C --json --method POST -H ... -d '<body>' \
      http://127.0.0.1:$PORT/openai/v1/chat/completions`
    and capture RPS + latency percentiles + status distribution from oha's JSON.
  - Select the best qualifying `C` per R4/R5; record it for this `W`.
  - Kill the server, free the port, proceed to next `W`.

### Request body

```json
{"model":"gpt-5","messages":[{"role":"user","content":"Hi"}],"stream":false,"max_tokens":16}
```

### Headline parsing

The selected peak for the post is `max(RPS)` across the worker sweep that still
meets R5 — typically (but not necessarily) at `workers == nproc`. The scaling
column makes it clear whether llmsim is bottlenecked before saturating all cores.

## Future Enhancements

1. CI regression gate: fail if peak RPS at full cores regresses > X% vs a stored
   baseline for the runner's core count.
2. Streaming-throughput variant: tokens/sec ceiling under the `instant` profile
   with streaming enabled.
3. k6 cross-check harness to quantify client-side overhead differences.
4. Per-endpoint throughput (responses API, openresponses) alongside chat
   completions.
