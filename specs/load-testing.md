# Load Testing Specification

## Abstract

This specification defines the load testing and stress testing framework for llmsim. It provides standardized benchmarks to validate llmsim's performance characteristics under various load conditions, ensuring the simulator can handle high concurrency with low resource usage.

## Requirements

### R1: Test Profiles

The framework MUST support the following test profiles:

| Profile | Purpose | Duration | VUs | Use Case |
|---------|---------|----------|-----|----------|
| `quick-smoke` | Ultra-fast validation | 3 iterations | 1 | CI pre-commit checks |
| `smoke` | Quick validation | ~10s | 1 | CI pipeline validation |
| `load` | Normal load testing | ~4 min | 10-50 | Performance baseline |
| `stress` | Beyond capacity testing | ~7 min | 50-500 | Find breaking points |
| `spike` | Sudden traffic bursts | ~2 min | 10-500 | Burst handling |
| `soak` | Extended duration | ~32 min | 50 | Memory leak detection |
| `high-concurrency` | Maximum throughput | ~4 min | 100-500+ | Concurrency limits |

### R2: Metrics Collection

The framework MUST collect and report the following metrics:

**Latency Metrics:**
- Response time percentiles (p50, p95, p99)
- Time to first token (TTFT) for streaming
- Time between tokens (TBT) for streaming

**Throughput Metrics:**
- Requests per second (RPS)
- Tokens per second
- Concurrent connection count

**Reliability Metrics:**
- Error rate (by type: 4xx, 5xx, timeouts)
- Success rate
- Connection failures

### R3: Thresholds

Each profile MUST define pass/fail thresholds:

| Profile | Max Error Rate | P95 Latency | P99 Latency |
|---------|----------------|-------------|-------------|
| smoke | <1% | <2000ms | - |
| load | <5% | <3000ms | <5000ms |
| stress | <15% | <10000ms | - |
| spike | <20% | - | - |
| soak | <1% | <3000ms | - |

### R4: Smoke Test Mode

The framework MUST provide a quick smoke test option that:
- Completes in under 30 seconds
- Validates all endpoints respond correctly
- Verifies streaming functionality works
- Can be run in CI without timeout concerns

### R5: Server Management

The benchmark runner MUST:
- Automatically build and start llmsim if not running
- Support connecting to an existing server (`--no-server`)
- Gracefully shut down the server after tests
- Handle server startup failures

### R6: Cloud Readiness

The framework MUST support:
- Configurable target URL for remote testing
- Environment variable configuration
- JSON output for CI/CD integration
- No hard-coded localhost dependencies

## Implementation

### Tool Selection: k6

**Rationale:** k6 was selected as the primary load testing tool because:

1. **JavaScript Scripting**: Complex test scenarios with conditions, loops, and custom logic
2. **Low Resource Usage**: Go-based, efficient memory and CPU usage
3. **Cloud Ready**: Can target remote endpoints, integrates with k6 Cloud
4. **CI/CD Integration**: JSON output, exit codes based on thresholds
5. **Active Development**: Well-maintained, good documentation
6. **Built-in Features**: Thresholds, stages, scenarios, metrics

**Alternatives Considered:**
- `wrk`: Higher raw throughput but limited scripting, no thresholds
- `oha`: Rust-based, fast, but limited scenario support
- `locust`: Python-based, higher resource usage
- `vegeta`: Good for constant rate, limited scenarios

### Directory Structure

```
benchmarks/
├── run-benchmark.sh      # Main runner script
├── smoke-test.sh         # Quick smoke test wrapper
└── k6/
    ├── config.js         # Shared configuration
    ├── chat-completions.js   # Chat endpoint tests
    ├── endpoints.js      # All endpoints test
    └── high-concurrency.js   # Max throughput test
```

### Configuration

Test configuration is centralized in `benchmarks/k6/config.js`:

```javascript
export const PROFILES = {
    smoke: { vus: 1, duration: '10s', ... },
    load: { stages: [...], thresholds: {...} },
    // ...
};
```

### Usage Examples

```bash
# Quick smoke test
./benchmarks/smoke-test.sh

# Full load test
./benchmarks/run-benchmark.sh load

# Stress test against existing server
./benchmarks/run-benchmark.sh stress --no-server --port 8080

# High concurrency with custom VUs
./benchmarks/run-benchmark.sh high-concurrency --vus 1000

# Output results to file
./benchmarks/run-benchmark.sh load --output results.json
```

## Cloud Deployment

### Local to Cloud Migration

For cloud testing, set the target URL:

```bash
export K6_TARGET_URL="https://llmsim.example.com"
k6 run benchmarks/k6/chat-completions.js
```

### Docker Support

```bash
docker run -i grafana/k6 run - < benchmarks/k6/chat-completions.js
```

### CI/CD Integration

```yaml
# GitHub Actions example
- name: Run load tests
  run: |
    ./benchmarks/run-benchmark.sh smoke --no-server
  env:
    K6_TARGET_URL: ${{ secrets.LLMSIM_URL }}
```

## Future Enhancements

1. **Distributed Testing**: k6 Cloud or custom k6 operator for Kubernetes
2. **Grafana Integration**: Real-time dashboards during tests
3. **Comparison Reports**: Compare runs across commits/versions
4. **oha Integration**: Optional fast local benchmarks with oha
5. **Custom Token Counting**: Accurate token throughput measurement
