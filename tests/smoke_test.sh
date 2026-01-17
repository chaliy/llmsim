#!/bin/bash
# Smoke tests for LLMSim server
# Run with: ./tests/smoke_test.sh

set -e

PORT=${PORT:-8888}
BASE_URL="http://127.0.0.1:$PORT"
LLMSIM_PID=""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo_pass() {
    echo -e "${GREEN}✓ $1${NC}"
}

echo_fail() {
    echo -e "${RED}✗ $1${NC}"
}

echo_info() {
    echo -e "${YELLOW}→ $1${NC}"
}

cleanup() {
    if [ -n "$LLMSIM_PID" ]; then
        echo_info "Stopping server (PID: $LLMSIM_PID)..."
        kill $LLMSIM_PID 2>/dev/null || true
        wait $LLMSIM_PID 2>/dev/null || true
    fi
}

trap cleanup EXIT

# Build the project
echo_info "Building LLMSim..."
cargo build --release --quiet

# Start the server
echo_info "Starting server on port $PORT..."
./target/release/llmsim serve --port $PORT &
LLMSIM_PID=$!

# Wait for server to start
sleep 2

# Check if server is running
if ! kill -0 $LLMSIM_PID 2>/dev/null; then
    echo_fail "Server failed to start"
    exit 1
fi

echo_pass "Server started"

# Test 1: Health check
echo_info "Testing /health endpoint..."
HEALTH=$(curl -s "$BASE_URL/health")
if echo "$HEALTH" | grep -q '"status":"ok"'; then
    echo_pass "Health check passed"
else
    echo_fail "Health check failed: $HEALTH"
    exit 1
fi

# Test 2: Stats endpoint
echo_info "Testing /llmsim/stats endpoint..."
STATS=$(curl -s "$BASE_URL/llmsim/stats")
if echo "$STATS" | grep -q '"total_requests"'; then
    echo_pass "Stats endpoint returned valid JSON"
else
    echo_fail "Stats endpoint failed: $STATS"
    exit 1
fi

# Test 3: Models list
echo_info "Testing /openai/v1/models endpoint..."
MODELS=$(curl -s "$BASE_URL/openai/v1/models")
if echo "$MODELS" | grep -q '"object":"list"'; then
    echo_pass "Models endpoint passed"
else
    echo_fail "Models endpoint failed: $MODELS"
    exit 1
fi

# Test 4: Chat completion (non-streaming)
echo_info "Testing /openai/v1/chat/completions (non-streaming)..."
RESPONSE=$(curl -s -X POST "$BASE_URL/openai/v1/chat/completions" \
    -H "Content-Type: application/json" \
    -d '{
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello!"}],
        "stream": false
    }')
if echo "$RESPONSE" | grep -q '"choices"'; then
    echo_pass "Non-streaming chat completion passed"
else
    echo_fail "Non-streaming chat completion failed: $RESPONSE"
    exit 1
fi

# Test 5: Chat completion (streaming)
echo_info "Testing /openai/v1/chat/completions (streaming)..."
STREAM_RESPONSE=$(curl -s -X POST "$BASE_URL/openai/v1/chat/completions" \
    -H "Content-Type: application/json" \
    -d '{
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello!"}],
        "stream": true
    }')
if echo "$STREAM_RESPONSE" | grep -q 'data:'; then
    echo_pass "Streaming chat completion passed"
else
    echo_fail "Streaming chat completion failed: $STREAM_RESPONSE"
    exit 1
fi

# Test 6: Verify stats updated
echo_info "Verifying stats updated after requests..."
STATS_AFTER=$(curl -s "$BASE_URL/llmsim/stats")
TOTAL_REQUESTS=$(echo "$STATS_AFTER" | grep -o '"total_requests":[0-9]*' | cut -d: -f2)
if [ "$TOTAL_REQUESTS" -ge 2 ]; then
    echo_pass "Stats correctly tracking requests (total: $TOTAL_REQUESTS)"
else
    echo_fail "Stats not updating correctly: $STATS_AFTER"
    exit 1
fi

# Test 7: Token counting in stats
TOTAL_TOKENS=$(echo "$STATS_AFTER" | grep -o '"total_tokens":[0-9]*' | cut -d: -f2)
if [ "$TOTAL_TOKENS" -gt 0 ]; then
    echo_pass "Token counting working (total: $TOTAL_TOKENS)"
else
    echo_fail "Token counting not working"
    exit 1
fi

# Test 8: OpenAI Responses API
echo_info "Testing /openai/v1/responses endpoint..."
RESPONSES_API=$(curl -s -X POST "$BASE_URL/openai/v1/responses" \
    -H "Content-Type: application/json" \
    -d '{
        "model": "gpt-4",
        "input": "Hello!",
        "stream": false
    }')
if echo "$RESPONSES_API" | grep -q '"status":"completed"'; then
    echo_pass "OpenAI Responses API passed"
else
    echo_fail "OpenAI Responses API failed: $RESPONSES_API"
    exit 1
fi

# Test 9: OpenResponses API
echo_info "Testing /openresponses/v1/responses endpoint..."
OPENRESPONSES=$(curl -s -X POST "$BASE_URL/openresponses/v1/responses" \
    -H "Content-Type: application/json" \
    -d '{
        "model": "gpt-4",
        "input": [{"role": "user", "content": "Test"}],
        "stream": false
    }')
if echo "$OPENRESPONSES" | grep -q '"status":"completed"'; then
    echo_pass "OpenResponses API passed"
else
    echo_fail "OpenResponses API failed: $OPENRESPONSES"
    exit 1
fi

# Test 10: OpenResponses streaming
echo_info "Testing /openresponses/v1/responses (streaming)..."
OPENRESPONSES_STREAM=$(curl -s -X POST "$BASE_URL/openresponses/v1/responses" \
    -H "Content-Type: application/json" \
    -d '{
        "model": "gpt-4",
        "input": "Hello!",
        "stream": true
    }')
if echo "$OPENRESPONSES_STREAM" | grep -q 'response.completed'; then
    echo_pass "OpenResponses streaming passed"
else
    echo_fail "OpenResponses streaming failed: $OPENRESPONSES_STREAM"
    exit 1
fi

echo ""
echo -e "${GREEN}All smoke tests passed!${NC}"
