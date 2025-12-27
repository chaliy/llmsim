#!/bin/bash
# llmsim Load Testing Benchmark Runner
#
# This script manages llmsim server lifecycle and runs k6 load tests.
#
# Usage:
#   ./benchmarks/run-benchmark.sh [profile] [options]
#
# Profiles:
#   smoke       - Quick validation (5 iterations, ~10s)
#   quick-smoke - Ultra-fast validation (3 iterations)
#   load        - Normal load test (~4 minutes)
#   stress      - Stress test (~7 minutes)
#   spike       - Spike test (~2 minutes)
#   soak        - Long-running soak test (~32 minutes)
#   high-concurrency - High VU count test
#
# Options:
#   --no-server     Don't start llmsim (use existing server)
#   --port PORT     Port to use (default: 8888)
#   --output FILE   Output results to file
#   --vus NUM       Override VU count (high-concurrency only)
#   --chaos         Enable error injection (rate limits, server errors)
#   --config FILE   Use custom llmsim config file
#   --help          Show this help

set -e

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DEFAULT_PORT=8888
PORT=$DEFAULT_PORT
PROFILE="smoke"
START_SERVER=true
OUTPUT_FILE=""
MAX_VUS=""
LLMSIM_PID=""
CHAOS_MODE=false
CUSTOM_CONFIG=""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_pass() { echo -e "${GREEN}[PASS]${NC} $1"; }
log_fail() { echo -e "${RED}[FAIL]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }

show_help() {
    head -30 "$0" | grep '^#' | sed 's/^# \?//'
    exit 0
}

cleanup() {
    if [ -n "$LLMSIM_PID" ] && [ "$START_SERVER" = true ]; then
        log_info "Stopping llmsim server (PID: $LLMSIM_PID)..."
        kill $LLMSIM_PID 2>/dev/null || true
        wait $LLMSIM_PID 2>/dev/null || true
    fi
}

trap cleanup EXIT

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --no-server)
            START_SERVER=false
            shift
            ;;
        --port)
            PORT="$2"
            shift 2
            ;;
        --output)
            OUTPUT_FILE="$2"
            shift 2
            ;;
        --vus)
            MAX_VUS="$2"
            shift 2
            ;;
        --chaos)
            CHAOS_MODE=true
            shift
            ;;
        --config)
            CUSTOM_CONFIG="$2"
            shift 2
            ;;
        --help|-h)
            show_help
            ;;
        smoke|quick-smoke|load|stress|spike|soak|high-concurrency)
            PROFILE="$1"
            shift
            ;;
        *)
            log_fail "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Check for k6
if ! command -v k6 &> /dev/null; then
    log_fail "k6 is not installed. Install it from https://k6.io/docs/get-started/installation/"
    echo ""
    echo "Quick install options:"
    echo "  macOS:  brew install k6"
    echo "  Linux:  sudo gpg -k && sudo gpg --no-default-keyring --keyring /usr/share/keyrings/k6-archive-keyring.gpg --keyserver hkp://keyserver.ubuntu.com:80 --recv-keys C5AD17C747E3415A3642D57D77C6C491D6AC1D69 && echo 'deb [signed-by=/usr/share/keyrings/k6-archive-keyring.gpg] https://dl.k6.io/deb stable main' | sudo tee /etc/apt/sources.list.d/k6.list && sudo apt-get update && sudo apt-get install k6"
    echo "  Docker: docker run -i grafana/k6 run -"
    exit 1
fi

log_info "llmsim Load Testing Benchmark"
echo "=============================="
echo "Profile: $PROFILE"
echo "Port: $PORT"
echo "Start server: $START_SERVER"
echo "Chaos mode: $CHAOS_MODE"
echo ""

# Start server if needed
if [ "$START_SERVER" = true ]; then
    log_info "Building llmsim..."
    cargo build --release --quiet --manifest-path "$PROJECT_ROOT/Cargo.toml"

    # Determine which config to use
    if [ -n "$CUSTOM_CONFIG" ]; then
        CONFIG_FILE="$CUSTOM_CONFIG"
    elif [ "$CHAOS_MODE" = true ]; then
        CONFIG_FILE="$SCRIPT_DIR/config/chaos.yaml"
        log_warn "Chaos mode enabled - error injection active"
    else
        CONFIG_FILE="$SCRIPT_DIR/config/benchmark.yaml"
    fi

    log_info "Starting llmsim server on port $PORT (config: $(basename $CONFIG_FILE))..."
    "$PROJECT_ROOT/target/release/llmsim" serve --port $PORT --config "$CONFIG_FILE" &
    LLMSIM_PID=$!

    # Wait for server to start
    sleep 2

    if ! kill -0 $LLMSIM_PID 2>/dev/null; then
        log_fail "Server failed to start"
        exit 1
    fi

    # Verify server is responding
    for i in {1..10}; do
        if curl -s "http://127.0.0.1:$PORT/health" | grep -q '"status":"ok"'; then
            log_pass "Server started successfully"
            break
        fi
        if [ $i -eq 10 ]; then
            log_fail "Server not responding after 10 attempts"
            exit 1
        fi
        sleep 1
    done
fi

# Build k6 command
K6_CMD="k6 run"
K6_CMD="$K6_CMD --env K6_TARGET_URL=http://127.0.0.1:$PORT"
K6_CMD="$K6_CMD --env PROFILE=$PROFILE"

if [ -n "$MAX_VUS" ]; then
    K6_CMD="$K6_CMD --env MAX_VUS=$MAX_VUS"
fi

if [ -n "$OUTPUT_FILE" ]; then
    K6_CMD="$K6_CMD --out json=$OUTPUT_FILE"
fi

# Select appropriate test script
if [ "$PROFILE" = "high-concurrency" ]; then
    TEST_SCRIPT="$SCRIPT_DIR/k6/high-concurrency.js"
else
    TEST_SCRIPT="$SCRIPT_DIR/k6/chat-completions.js"
fi

log_info "Running k6 benchmark..."
echo ""

# Run the benchmark
$K6_CMD "$TEST_SCRIPT"

EXIT_CODE=$?

echo ""
if [ $EXIT_CODE -eq 0 ]; then
    log_pass "Benchmark completed successfully"
else
    log_warn "Benchmark completed with threshold failures (exit code: $EXIT_CODE)"
fi

exit $EXIT_CODE
