#!/bin/bash
# llmsim Quick Smoke Test for Load Testing
#
# A fast validation that the load testing setup works correctly.
# Runs minimal iterations to verify:
#   1. Server starts correctly
#   2. k6 can connect and run
#   3. Basic chat completion works
#   4. Thresholds pass under light load
#
# Usage:
#   ./benchmarks/smoke-test.sh [--quick] [--port PORT]
#
# Options:
#   --quick    Ultra-fast mode (3 iterations instead of 5)
#   --port     Port to use (default: 8889 to avoid conflicts)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PORT=8889
PROFILE="smoke"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_pass() { echo -e "${GREEN}[PASS]${NC} $1"; }
log_fail() { echo -e "${RED}[FAIL]${NC} $1"; }

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --quick)
            PROFILE="quick-smoke"
            shift
            ;;
        --port)
            PORT="$2"
            shift 2
            ;;
        *)
            shift
            ;;
    esac
done

echo ""
echo "=========================================="
echo "  llmsim Load Test Smoke Test"
echo "=========================================="
echo "Profile: $PROFILE"
echo "Port: $PORT"
echo ""

# Check for k6
if ! command -v k6 &> /dev/null; then
    log_fail "k6 is not installed"
    echo "Install k6: https://k6.io/docs/get-started/installation/"
    exit 1
fi

log_pass "k6 is installed: $(k6 version 2>/dev/null | head -1)"

# Run the benchmark
log_info "Running smoke test..."
echo ""

"$SCRIPT_DIR/run-benchmark.sh" $PROFILE --port $PORT

EXIT_CODE=$?

echo ""
echo "=========================================="
if [ $EXIT_CODE -eq 0 ]; then
    echo -e "${GREEN}  SMOKE TEST PASSED${NC}"
else
    echo -e "${RED}  SMOKE TEST FAILED${NC}"
fi
echo "=========================================="
echo ""

exit $EXIT_CODE
