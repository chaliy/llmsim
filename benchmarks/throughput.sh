#!/bin/bash
# llmsim Throughput Benchmark
#
# Measures llmsim's peak sustained request rate (req/s) and how it scales with
# parallelism (async worker threads / CPU cores). See specs/throughput-benchmark.md.
#
# Generator: oha (Rust, MIT) -- chosen over k6 because this benchmark saturates
# the server (instant latency), where the load generator competes for CPU. oha's
# per-request overhead is near zero, so the number reflects llmsim, not the tool.
#
# Method:
#   for each worker-thread count W (sweep, default powers of two up to nproc):
#     start a fresh server pinned to W via TOKIO_WORKER_THREADS
#     for each connection concurrency C (sweep): run oha for DURATION, record RPS
#     pick the best result that still meets the success criteria (R5)
#   emit a scaling table + a single headline RPS line + JSON.
#
# Usage:
#   ./benchmarks/throughput.sh [options]
#
# Options:
#   --port PORT          Port to use (default: 8899)
#   --workers LIST       Comma-separated worker-thread counts (default: powers of 2 up to nproc)
#   --concurrency LIST   Comma-separated oha -c steps (default: 16,32,64,128,256)
#   --duration DUR       Measured run duration per step (default: 30s)
#   --warmup DUR         Warm-up duration per worker count, discarded (default: 5s)
#   --output FILE        Write machine-readable JSON results to FILE
#   --no-server          Use an already-running server (skips the worker sweep; single run)
#   --help               Show this help

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CONFIG_FILE="$SCRIPT_DIR/config/throughput.toml"

PORT=8899
WORKERS=""
CONCURRENCY="16,32,64,128,256"
DURATION="30s"
WARMUP="5s"
OUTPUT_FILE=""
START_SERVER=true
LLMSIM_PID=""

# Success criteria (R5)
MAX_ERROR_PCT="0.1"
MAX_P99_MS="50"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_pass() { echo -e "${GREEN}[PASS]${NC} $1"; }
log_fail() { echo -e "${RED}[FAIL]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }

show_help() { grep '^#' "$0" | sed 's/^# \?//'; exit 0; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --port) PORT="$2"; shift 2 ;;
        --workers) WORKERS="$2"; shift 2 ;;
        --concurrency) CONCURRENCY="$2"; shift 2 ;;
        --duration) DURATION="$2"; shift 2 ;;
        --warmup) WARMUP="$2"; shift 2 ;;
        --output) OUTPUT_FILE="$2"; shift 2 ;;
        --no-server) START_SERVER=false; shift ;;
        --help|-h) show_help ;;
        *) log_fail "Unknown option: $1"; exit 1 ;;
    esac
done

# --- Dependency checks ------------------------------------------------------
if ! command -v oha &> /dev/null; then
    log_fail "oha is not installed (the throughput benchmark's load generator)."
    echo ""
    echo "Install options:"
    echo "  cargo install oha          # any platform with Rust"
    echo "  brew install oha           # macOS / Linuxbrew"
    echo "  See https://github.com/hatoo/oha for binaries"
    exit 1
fi
if ! command -v jq &> /dev/null; then
    log_fail "jq is required to parse oha JSON output. Install jq and retry."
    exit 1
fi

URL="http://127.0.0.1:$PORT"
ENDPOINT="$URL/openai/v1/chat/completions"
BODY='{"model":"gpt-5","messages":[{"role":"user","content":"Hi"}],"stream":false,"max_tokens":16}'

# --- Environment capture (R6) ----------------------------------------------
detect_cores() { command -v nproc &>/dev/null && nproc || sysctl -n hw.ncpu 2>/dev/null || echo 1; }
detect_cpu_model() {
    if command -v lscpu &>/dev/null; then lscpu | awk -F: '/Model name/{gsub(/^ +/,"",$2); print $2; exit}'
    elif [ -r /proc/cpuinfo ]; then awk -F: '/model name/{gsub(/^ +/,"",$2); print $2; exit}' /proc/cpuinfo
    else sysctl -n machdep.cpu.brand_string 2>/dev/null || echo "unknown"; fi
}
detect_ram() {
    if command -v free &>/dev/null; then free -h | awk '/^Mem:/{print $2}'
    elif command -v sysctl &>/dev/null; then echo "$(( $(sysctl -n hw.memsize 2>/dev/null || echo 0) / 1024 / 1024 / 1024 )) GiB"
    else echo "unknown"; fi
}

CORES="$(detect_cores)"
CPU_MODEL="$(detect_cpu_model)"
RAM="$(detect_ram)"
OS="$(uname -srm)"
RUST_VERSION="$(rustc --version 2>/dev/null || echo unknown)"
OHA_VERSION="$(oha --version 2>/dev/null || echo unknown)"
GIT_COMMIT="$(git -C "$PROJECT_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
LLMSIM_VERSION="$(awk -F\" '/^version *=/{print $2; exit}' "$PROJECT_ROOT/Cargo.toml" 2>/dev/null || echo unknown)"

# Default worker sweep: powers of two up to core count, plus core count itself.
if [ -z "$WORKERS" ]; then
    WORKERS="$(python3 - "$CORES" <<'PY'
import sys
n = int(sys.argv[1])
s, p = set(), 1
while p <= n:
    s.add(p); p *= 2
s.add(n)
print(",".join(str(x) for x in sorted(s)))
PY
)"
fi

echo ""
echo "================= llmsim Throughput Benchmark ================="
echo "CPU:            $CPU_MODEL ($CORES logical cores)"
echo "RAM:            $RAM"
echo "OS:             $OS"
echo "Rust:           $RUST_VERSION"
echo "oha:            $OHA_VERSION"
echo "llmsim:         v$LLMSIM_VERSION ($GIT_COMMIT)"
echo "Config:         $(basename "$CONFIG_FILE") (instant latency, non-streaming, no errors)"
echo "Worker sweep:   $WORKERS"
echo "Concurrency:    $CONCURRENCY"
echo "Duration/step:  $DURATION (warmup $WARMUP)"
echo "Generator:      oha on loopback (same host as server)"
echo "Success crit.:  error < ${MAX_ERROR_PCT}% AND p99 < ${MAX_P99_MS}ms"
echo "=============================================================="
echo ""
log_warn "Absolute numbers are hardware-dependent. The scaling table (RPS vs workers), not the single peak, is the portable result."
echo ""

# --- Server lifecycle -------------------------------------------------------
RESULTS_TSV="$(mktemp)"
ENV_JSON="$(mktemp)"
jq -n \
    --arg cpu "$CPU_MODEL" --argjson cores "$CORES" --arg ram "$RAM" --arg os "$OS" \
    --arg rust "$RUST_VERSION" --arg oha "$OHA_VERSION" \
    --arg version "$LLMSIM_VERSION" --arg commit "$GIT_COMMIT" \
    --arg duration "$DURATION" --arg warmup "$WARMUP" \
    --arg workers "$WORKERS" --arg concurrency "$CONCURRENCY" \
    '{cpu_model:$cpu, logical_cores:$cores, ram:$ram, os:$os, rust:$rust, oha:$oha,
      llmsim_version:$version, git_commit:$commit, duration_per_step:$duration,
      warmup:$warmup, worker_sweep:$workers, concurrency_sweep:$concurrency,
      generator:"oha", colocated:"loopback (same host as server)"}' > "$ENV_JSON"

trap 'cleanup' EXIT
cleanup() {
    [ -n "$LLMSIM_PID" ] && kill "$LLMSIM_PID" 2>/dev/null || true
    [ -n "$LLMSIM_PID" ] && wait "$LLMSIM_PID" 2>/dev/null || true
    rm -f "$RESULTS_TSV" "$ENV_JSON"
}

start_server() {
    local workers="$1"
    log_info "Starting llmsim with TOKIO_WORKER_THREADS=$workers ..."
    TOKIO_WORKER_THREADS="$workers" \
        "$PROJECT_ROOT/target/release/llmsim" serve \
        --host 127.0.0.1 --port "$PORT" --config "$CONFIG_FILE" \
        >/dev/null 2>&1 &
    LLMSIM_PID=$!
    for _ in $(seq 1 20); do
        if curl -s "$URL/health" | grep -q '"status":"ok"'; then return 0; fi
        if ! kill -0 "$LLMSIM_PID" 2>/dev/null; then log_fail "Server exited during startup"; return 1; fi
        sleep 0.5
    done
    log_fail "Server did not become healthy on $URL"; return 1
}

stop_server() {
    [ -n "$LLMSIM_PID" ] && kill "$LLMSIM_PID" 2>/dev/null || true
    [ -n "$LLMSIM_PID" ] && wait "$LLMSIM_PID" 2>/dev/null || true
    LLMSIM_PID=""
}

# Run oha once; echo "rps p50ms p95ms p99ms errpct total" (tab-separated).
run_oha() {
    local conc="$1" dur="$2" json
    json="$(oha -z "$dur" -c "$conc" --no-tui --output-format json \
        -m POST \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer bench" \
        -d "$BODY" \
        "$ENDPOINT" 2>/dev/null)" || { echo ""; return 1; }
    echo "$json" | jq -r '
        (.statusCodeDistribution // {}) as $s
        | ([$s | to_entries[] | .value] | add // 0) as $ok
        | ([$s | to_entries[] | select((.key|tonumber) < 200 or (.key|tonumber) >= 300) | .value] | add // 0) as $non2xx
        | ((.errorDistribution // {}) | [.[]] | add // 0) as $cerr
        | ($ok + $cerr) as $attempted
        | [ (.summary.requestsPerSec // 0),
            ((.latencyPercentiles.p50 // 0) * 1000),
            ((.latencyPercentiles.p95 // 0) * 1000),
            ((.latencyPercentiles.p99 // 0) * 1000),
            (if $attempted > 0 then ($non2xx + $cerr) / $attempted * 100 else 100 end),
            $attempted ] | @tsv'
}

# Run the concurrency sweep against the live server for worker count $1.
sweep_concurrency() {
    local workers="$1" c line
    log_info "  warm-up ($WARMUP, -c 32) ..."
    run_oha 32 "$WARMUP" >/dev/null || true
    IFS=',' read -ra CSTEPS <<< "$CONCURRENCY"
    for c in "${CSTEPS[@]}"; do
        line="$(run_oha "$c" "$DURATION")" || { log_warn "  oha failed at -c $c"; continue; }
        [ -z "$line" ] && { log_warn "  no result at -c $c"; continue; }
        # workers concurrency rps p50 p95 p99 errpct total
        printf '%s\t%s\t%s\n' "$workers" "$c" "$line" >> "$RESULTS_TSV"
        awk -v w="$workers" -v c="$c" -F'\t' \
            '{printf "  workers=%s c=%-4s -> %10.1f req/s  p99=%6.2fms  err=%.3f%%\n", w, c, $1, $4, $5}' \
            <<< "$line"
    done
}

# --- Run --------------------------------------------------------------------
if [ "$START_SERVER" = true ]; then
    log_info "Building llmsim (release) ..."
    cargo build --release --quiet --manifest-path "$PROJECT_ROOT/Cargo.toml"
    IFS=',' read -ra WSTEPS <<< "$WORKERS"
    for w in "${WSTEPS[@]}"; do
        echo ""
        log_info "=== Worker threads: $w ==="
        start_server "$w" || { stop_server; continue; }
        sweep_concurrency "$w"
        stop_server
    done
else
    log_info "Using existing server at $URL (no worker sweep; reporting one row)."
    if ! curl -s "$URL/health" | grep -q '"status":"ok"'; then
        log_fail "No healthy server at $URL"; exit 1
    fi
    sweep_concurrency "external"
fi

# --- Report (selection R4/R5, table R2, headline + JSON R7) ------------------
echo ""
python3 - "$RESULTS_TSV" "$MAX_ERROR_PCT" "$MAX_P99_MS" "${OUTPUT_FILE:-}" "$ENV_JSON" <<'PY'
import sys, json, collections

tsv, max_err, max_p99, out, env_path = (
    sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), sys.argv[4], sys.argv[5])
with open(env_path) as f:
    environment = json.load(f)

rows = []
with open(tsv) as f:
    for ln in f:
        parts = ln.rstrip("\n").split("\t")
        if len(parts) != 8:
            continue
        w, c, rps, p50, p95, p99, errpct, total = parts
        rows.append(dict(workers=w, concurrency=int(c), rps=float(rps),
                         p50=float(p50), p95=float(p95), p99=float(p99),
                         errpct=float(errpct), total=float(total)))

if not rows:
    print("No results were collected — check that oha ran successfully.")
    sys.exit(1)

# Best result per worker count: prefer runs meeting R5, by max RPS; else best-effort max RPS.
by_w = collections.OrderedDict()
for r in rows:
    by_w.setdefault(r["workers"], []).append(r)

def qualifies(r):
    return r["errpct"] < max_err and r["p99"] < max_p99

def pick(cands):
    ok = [r for r in cands if qualifies(r)]
    pool = ok if ok else cands
    best = max(pool, key=lambda r: r["rps"])
    best = dict(best); best["qualified"] = bool(ok)
    return best

# Numeric sort for worker counts where possible.
def wkey(w):
    try: return (0, int(w))
    except ValueError: return (1, w)

selected = [pick(by_w[w]) for w in sorted(by_w, key=wkey)]

base = next((s["rps"] for s in selected if str(s["workers"]) in ("1",)), selected[0]["rps"])

# Markdown scaling table (R2)
print("### Throughput scaling\n")
print("| workers | best -c | RPS | p50 (ms) | p95 (ms) | p99 (ms) | error % | scaling vs 1 |")
print("|--------:|--------:|----:|---------:|---------:|---------:|--------:|-------------:|")
for s in selected:
    scale = f"{s['rps']/base:.2f}x" if base else "-"
    flag = "" if s["qualified"] else " ⚠"
    print(f"| {s['workers']}{flag} | {s['concurrency']} | {s['rps']:,.0f} | "
          f"{s['p50']:.2f} | {s['p95']:.2f} | {s['p99']:.2f} | {s['errpct']:.3f} | {scale} |")
print("\n⚠ = did not meet success criteria (error<{:.1f}% and p99<{:.0f}ms); shown best-effort.".format(max_err, max_p99))

# Headline (R7): peak qualifying RPS across the sweep
qual = [s for s in selected if s["qualified"]]
peak = max(qual or selected, key=lambda s: s["rps"])
print("\n" + "=" * 62)
print("PEAK: {:,.0f} req/s sustained at {} workers "
      "(p99 {:.2f}ms, {:.3f}% errors){}".format(
          peak["rps"], peak["workers"], peak["p99"], peak["errpct"],
          "" if peak["qualified"] else "  [best-effort, criteria not met]"))
print("=" * 62)

if out:
    with open(out, "w") as f:
        json.dump({"environment": environment, "selected": selected,
                   "peak": peak, "all_runs": rows}, f, indent=2)
    print(f"\nJSON written to {out}")
PY
