# Throughput Benchmark Specification

## Abstract

This specification defines a dedicated **throughput benchmark** that measures the
peak sustained request rate llmsim can serve, and how that rate scales with
parallelism (CPU cores / async worker threads).

It is distinct from the general load-testing framework (`specs/load-testing.md`),
which validates behavior under realistic, latency-shaped traffic. The throughput
benchmark deliberately strips away simulated latency and large payloads so the
number it reports reflects llmsim's own request-handling ceiling — the figure
suitable for headline claims (e.g. "sustains ~X req/s on N cores").

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

## Requirements

### R1: Isolate the server's request-handling ceiling

The benchmark MUST minimize every cost that is not llmsim's own request handling:

- Latency profile MUST be `instant` (0ms TTFT, 0ms inter-token) so the measured
  rate is the server's ceiling, not the latency it was configured to simulate.
- Requests MUST be **non-streaming** `POST /openai/v1/chat/completions`.
- Response size MUST be small and fixed (`target_tokens` ≤ 32, `max_tokens` ≤ 32)
  to minimize serialization and token-counting cost.
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
  that count, run the load stage (R3), record peak sustained RPS, then shut the
  server down before the next step.
- Results MUST be emitted as a markdown table with columns:
  `workers | RPS | p50 (ms) | p95 (ms) | p99 (ms) | error %`, plus a derived
  `scaling vs 1 worker` column (RPS at N / RPS at 1) so near-linear scaling is
  visible at a glance.

### R3: Client-side parallelism must not be the bottleneck

The reported RPS must reflect the server, not the load generator. The benchmark
MUST:

- Drive load with an **open model** (k6 `ramping-arrival-rate` or
  `constant-arrival-rate`), so offered load is independent of response time and
  the true saturation point is found rather than masked by a closed VU loop.
- Provision enough `preAllocatedVUs` / `maxVUs` that the generator never starves
  (k6 MUST NOT emit "insufficient VUs" / dropped-iteration warnings; if it does,
  the run is invalid and MUST be retried with a higher cap).
- Reuse connections (HTTP keep-alive; k6 default) so the measurement is not
  dominated by TCP/TLS setup.
- Run the load generator on the same host as the server (loopback) for the
  headline number, to remove network variance. Remote runs are permitted but
  MUST be labeled as such.

If a non-k6 generator with lower per-request overhead is used as a cross-check
(e.g. `oha`, `wrk`), it MUST be additive — k6 remains the reference tool per
`specs/load-testing.md`. Any such cross-check MUST be clearly labeled.

### R4: Finding peak sustained RPS

For each worker-thread count the runner MUST locate the sustainable rate, not a
momentary burst:

- Ramp offered request rate upward in stages and identify the highest stage at
  which the run still meets the success criteria in R5 (the "knee").
- Report the RPS **actually achieved** (completed requests / wall-clock of the
  held stage), not merely the offered target rate.
- The held measurement window MUST be ≥ 30s to average out scheduler jitter and
  GC/allocator noise. A short warm-up stage (≥ 5s) before measurement MUST be
  discarded from the reported figure.

### R5: Success criteria (what counts as "sustained")

A rate qualifies as sustained only if, during the held window:

- `http_req_failed` rate < 0.1%
- p99 response latency < 50ms (with the `instant` profile there is no simulated
  latency, so anything above this indicates the server is saturating/queuing)

The highest qualifying stage is the reported peak for that worker count.

### R6: Environment capture (methodology honesty)

Every run MUST record and print the conditions, so the headline number is never
quoted without context:

- CPU model and logical core count (`nproc`, and CPU model where available)
- Total RAM
- OS / kernel and architecture
- Rust toolchain version and llmsim version/commit
- k6 version
- Whether load generator and server shared a host (loopback) or were remote

This block MUST appear in stdout and in the JSON output (R7).

### R7: Output

The benchmark MUST emit:

- Human-readable stdout: environment block (R6), the per-worker scaling table
  (R2), and a final single headline line, e.g.:
  `PEAK: 18,400 req/s sustained at 4 workers (4 vCPU, p99 2ms, 0.0% errors)`
- A machine-readable JSON file (path via `--output`) containing the environment
  block, an array of per-worker results, and the selected peak. This enables
  pasting numbers into docs/posts and future run-over-run comparison.

### R8: Invocation

The benchmark MUST be runnable as a single command via the existing runner
conventions, e.g.:

```bash
./benchmarks/run-benchmark.sh throughput
./benchmarks/run-benchmark.sh throughput --output throughput.json
./benchmarks/run-benchmark.sh throughput --workers 1,2,4,8   # override sweep
```

It MUST follow the existing runner contract from `specs/load-testing.md`:
auto-build (`cargo build --release`), start/stop the server, `--no-server` to
target an existing server, `--output` for JSON, graceful cleanup on exit, and a
clear failure if `k6` is not installed.

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
├── k6/
│   └── throughput.js            # open-model arrival-rate script, non-streaming only
└── run-benchmark.sh             # extend: add `throughput` profile + worker sweep + env capture
```

### Worker-thread sweep mechanics

- Determine the sweep set from `nproc` unless `--workers a,b,c` is given.
- For each `W` in the set:
  - `TOKIO_WORKER_THREADS=$W target/release/llmsim serve --host 127.0.0.1 \
     --port $PORT --config benchmarks/config/throughput.toml`
  - Wait for `/health` to return `{"status":"ok"}`.
  - Run `k6 run --env K6_TARGET_URL=... --env PROFILE=throughput benchmarks/k6/throughput.js`.
  - Parse achieved RPS + latency percentiles (prefer k6 `--summary-export` /
    `handleSummary`, or read `/llmsim/stats`, consistent with the existing
    high-concurrency teardown).
  - Kill the server, free the port, proceed to next `W`.

### k6 script shape (`throughput.js`)

- Single scenario, `executor: 'ramping-arrival-rate'`.
- Stages step the target rate upward (e.g. 2k → 5k → 10k → 20k → 40k rps, scaled
  by host capability), each held ≥ 30s after a short warm-up.
- `preAllocatedVUs` / `maxVUs` generous (e.g. 200–1000) so offered rate is met.
- Thresholds encode R5 (`http_req_failed: ['rate<0.001']`,
  `http_req_duration: ['p(99)<50']`).
- `handleSummary` writes the JSON described in R7.
- Payload built via `buildChatRequest({ stream:false, maxTokens:16, prompt:'Hi' })`.

### Headline parsing

The selected peak for the post is `max(RPS)` across the sweep that still meets
R5 — typically (but not necessarily) at `workers == nproc`. The scaling column
makes it clear whether llmsim is bottlenecked before saturating all cores.

## Future Enhancements

1. CI regression gate: fail if peak RPS at full cores regresses > X% vs a stored
   baseline for the runner's core count.
2. Streaming-throughput variant: tokens/sec ceiling under the `instant` profile
   with streaming enabled.
3. `oha`/`wrk` cross-check harness to bound k6's own client-side overhead.
4. Per-endpoint throughput (responses API, openresponses) alongside chat
   completions.
