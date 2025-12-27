# llmsim Load Testing Benchmarks

Load and stress testing benchmarks for llmsim using [k6](https://k6.io/).

## Quick Start

```bash
# Install k6 (if not already installed)
# macOS
brew install k6

# Linux (Debian/Ubuntu)
sudo gpg -k
sudo gpg --no-default-keyring --keyring /usr/share/keyrings/k6-archive-keyring.gpg \
  --keyserver hkp://keyserver.ubuntu.com:80 --recv-keys C5AD17C747E3415A3642D57D77C6C491D6AC1D69
echo "deb [signed-by=/usr/share/keyrings/k6-archive-keyring.gpg] https://dl.k6.io/deb stable main" \
  | sudo tee /etc/apt/sources.list.d/k6.list
sudo apt-get update && sudo apt-get install k6

# Run smoke test (quick validation)
./benchmarks/smoke-test.sh

# Run full load test
./benchmarks/run-benchmark.sh load
```

## Test Profiles

| Profile | Duration | VUs | Purpose |
|---------|----------|-----|---------|
| `quick-smoke` | ~5s | 1 | Ultra-fast CI validation |
| `smoke` | ~10s | 1 | Quick validation |
| `load` | ~4 min | 10→50 | Normal load testing |
| `stress` | ~7 min | 50→500 | Find breaking points |
| `spike` | ~2 min | 10→500→10 | Burst traffic handling |
| `soak` | ~32 min | 50 | Long-running stability |
| `high-concurrency` | ~4 min | 100→500 | Maximum throughput |

## Usage

### Basic Usage

```bash
# Smoke test (quick validation)
./benchmarks/smoke-test.sh

# Quick smoke test (even faster)
./benchmarks/smoke-test.sh --quick

# Run specific profile
./benchmarks/run-benchmark.sh load
./benchmarks/run-benchmark.sh stress
./benchmarks/run-benchmark.sh spike
```

### Advanced Options

```bash
# Use existing server (don't start new one)
./benchmarks/run-benchmark.sh load --no-server --port 8080

# Custom port
./benchmarks/run-benchmark.sh smoke --port 9000

# Output results to JSON file
./benchmarks/run-benchmark.sh load --output results.json

# High concurrency with custom VU count
./benchmarks/run-benchmark.sh high-concurrency --vus 1000
```

### Remote Testing

```bash
# Test against remote server
export K6_TARGET_URL="https://llmsim.example.com"
./benchmarks/run-benchmark.sh load --no-server
```

## Test Scripts

### chat-completions.js
Main load test targeting the `/v1/chat/completions` endpoint:
- 70% streaming requests, 30% non-streaming
- Variable request sizes and models
- Measures TTFT, response time, token throughput

### endpoints.js
Tests all llmsim endpoints with weighted distribution:
- 80% chat completions
- 5% health checks
- 5% model listings
- 5% stats endpoint
- 5% model detail

### high-concurrency.js
Optimized for maximum concurrent connections:
- Ramps up to high VU counts (default 500)
- Minimal request payload for speed
- Measures connection errors and timeouts

## Thresholds

Tests will fail if thresholds are exceeded:

| Profile | Max Error Rate | P95 Latency |
|---------|----------------|-------------|
| smoke | 1% | 2s |
| load | 5% | 3s |
| stress | 15% | 10s |
| spike | 20% | - |

## Output

Example output from a load test:

```
=== llmsim Load Test ===
Profile: load
Target: http://127.0.0.1:8888
========================

     ✓ status is 200
     ✓ has choices
     ✓ has usage info

     checks.........................: 100.00%
     http_req_duration..............: avg=245ms p(95)=890ms
     http_req_failed................: 0.00%
     streaming_requests.............: 1523
     non_streaming_requests.........: 677
     tokens_processed...............: 156789

=== Test Complete ===
```

## CI Integration

### GitHub Actions

```yaml
- name: Install k6
  run: |
    sudo gpg -k
    sudo gpg --no-default-keyring --keyring /usr/share/keyrings/k6-archive-keyring.gpg \
      --keyserver hkp://keyserver.ubuntu.com:80 \
      --recv-keys C5AD17C747E3415A3642D57D77C6C491D6AC1D69
    echo "deb [signed-by=/usr/share/keyrings/k6-archive-keyring.gpg] https://dl.k6.io/deb stable main" \
      | sudo tee /etc/apt/sources.list.d/k6.list
    sudo apt-get update && sudo apt-get install k6

- name: Run smoke test
  run: ./benchmarks/smoke-test.sh --quick
```

## Troubleshooting

### k6 not found
Install k6 following the instructions at https://k6.io/docs/get-started/installation/

### Server not responding
- Check if another process is using the port
- Verify llmsim builds successfully: `cargo build --release`
- Check server logs for errors

### High error rates during stress tests
- This is expected behavior during stress testing
- Adjust thresholds in `k6/config.js` if needed
- Consider increasing server resources

## See Also

- [Load Testing Specification](../specs/load-testing.md)
- [k6 Documentation](https://k6.io/docs/)
