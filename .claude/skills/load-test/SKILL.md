---
name: load-test
description: Run load and stress tests for llmsim using k6. Use this skill when you need to benchmark performance, validate high concurrency handling, or run smoke tests for CI validation.
---

# Load Testing Skill

## Environment Setup

### Prerequisites

**k6** must be installed. Install options:

```bash
# Linux (binary)
curl -sLO https://github.com/grafana/k6/releases/download/v0.54.0/k6-v0.54.0-linux-amd64.tar.gz
tar -xzf k6-v0.54.0-linux-amd64.tar.gz
sudo mv k6-v0.54.0-linux-amd64/k6 /usr/local/bin/

# macOS
brew install k6

# Verify installation
k6 version
```

### Build llmsim

```bash
cargo build --release
```

## Running Tests

### Quick Smoke Test (Recommended for CI)

Validates the load testing setup works correctly:

```bash
./benchmarks/smoke-test.sh
```

Ultra-fast variant (3 iterations):
```bash
./benchmarks/smoke-test.sh --quick
```

### Load Test Profiles

| Profile | Command | Duration | VUs | Purpose |
|---------|---------|----------|-----|---------|
| Smoke | `./benchmarks/run-benchmark.sh smoke` | ~10s | 1 | Quick validation |
| Load | `./benchmarks/run-benchmark.sh load` | ~4 min | 10-50 | Normal load |
| Stress | `./benchmarks/run-benchmark.sh stress` | ~7 min | 50-500 | Breaking points |
| Spike | `./benchmarks/run-benchmark.sh spike` | ~2 min | 10-500 | Burst traffic |
| Soak | `./benchmarks/run-benchmark.sh soak` | ~32 min | 50 | Memory leaks |
| High Concurrency | `./benchmarks/run-benchmark.sh high-concurrency` | ~4 min | 500 | Max throughput |

### Common Options

```bash
# Use existing server (don't start new one)
./benchmarks/run-benchmark.sh load --no-server --port 8080

# Custom port
./benchmarks/run-benchmark.sh smoke --port 9000

# Output results to JSON
./benchmarks/run-benchmark.sh load --output results.json

# Custom VU count for high-concurrency
./benchmarks/run-benchmark.sh high-concurrency --vus 1000

# Chaos mode - enable error injection (rate limits, server errors)
./benchmarks/run-benchmark.sh load --chaos

# Custom llmsim config file
./benchmarks/run-benchmark.sh load --config path/to/config.yaml
```

### Error Injection (Chaos Mode)

By default, benchmarks run with error injection **disabled** for consistent results.

Use `--chaos` to enable error injection:
- 10% rate limit errors (429)
- 5% server errors (500/503)
- 2% timeout errors (504)

This tests client error handling and retry logic under realistic conditions.

### Remote Testing

```bash
export K6_TARGET_URL="https://llmsim.example.com"
./benchmarks/run-benchmark.sh load --no-server
```

### Direct k6 Usage

```bash
# Run specific test script
k6 run benchmarks/k6/chat-completions.js

# With custom profile
k6 run --env PROFILE=stress benchmarks/k6/chat-completions.js

# With custom target
k6 run --env K6_TARGET_URL=http://localhost:9000 benchmarks/k6/endpoints.js
```

## Test Scripts

| Script | Description |
|--------|-------------|
| `chat-completions.js` | Main test - streaming/non-streaming chat completions |
| `endpoints.js` | All endpoints with weighted distribution |
| `high-concurrency.js` | Maximum concurrent connections |

## Success Criteria

Tests pass if thresholds are met:

| Profile | Max Error Rate | P95 Latency |
|---------|----------------|-------------|
| smoke | 1% | 2s |
| load | 5% | 3s |
| stress | 15% | 10s |

## Troubleshooting

**Server not responding**: Check port availability and build status
**High error rates in stress tests**: Expected behavior at capacity limits
**k6 not found**: Install k6 following instructions above

## Files

- `benchmarks/run-benchmark.sh` - Main runner script
- `benchmarks/smoke-test.sh` - Quick smoke test
- `benchmarks/config/benchmark.yaml` - Default config (no errors)
- `benchmarks/config/chaos.yaml` - Chaos mode config (error injection)
- `benchmarks/k6/config.js` - k6 test configuration
- `benchmarks/k6/*.js` - k6 test scripts
- `specs/load-testing.md` - Full specification
