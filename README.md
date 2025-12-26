# LLMSim

**LLM Traffic Simulator** - A lightweight, high-performance LLM API simulator for load testing, CI/CD, and local development.

## Overview

LLMSim replicates realistic LLM API behavior without running actual models. It solves common challenges when testing LLM-integrated applications:

- **Cost**: Real API calls during load tests are expensive
- **Rate Limits**: Production APIs prevent realistic load testing
- **Reproducibility**: Real models produce variable responses
- **Traffic Realism**: LLM responses have unique characteristics (streaming, variable latency, token-based billing)

## Features

- **Realistic Latency Simulation** - Time-to-first-token (TTFT) and inter-token delays with normal distribution
- **Streaming Support** - Server-Sent Events (SSE) for OpenAI-compatible streaming
- **Accurate Token Counting** - Uses tiktoken-rs (OpenAI's tokenizer implementation)
- **Error Injection** - Rate limits (429), server errors (500/503), timeouts
- **Multiple Response Generators** - Lorem ipsum, echo, fixed, random, sequence
- **Model-Specific Profiles** - GPT-5, GPT-4, Claude, Gemini latency profiles

## Installation

```bash
cargo install --git https://github.com/llmsim/llmsim.git
```

## Usage

### CLI Server

```bash
# Start with defaults (port 8080, lorem generator, gpt-5 latency)
llmsim serve

# Custom port and latency profile
llmsim serve --port 3000 --latency-profile claude-sonnet

# All options
llmsim serve \
  --port 8080 \
  --host 0.0.0.0 \
  --generator lorem \
  --target-tokens 150

# Using config file
llmsim serve --config config.yaml
```

### As a Library

```rust
use llmsim::{
    openai::{ChatCompletionRequest, Message},
    generator::LoremGenerator,
    latency::LatencyProfile,
};

// Create a latency profile
let latency = LatencyProfile::gpt5();

// Count tokens
let tokens = llmsim::count_tokens("Hello, world!", "gpt-5").unwrap();

// Generate responses
let generator = LoremGenerator::new(100);
let response = generator.generate(&request);
```

## API Endpoints

OpenAI-compatible endpoints:

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Health check |
| `/v1/chat/completions` | POST | Chat completions (streaming & non-streaming) |
| `/v1/models` | GET | List available models |
| `/v1/models/{model_id}` | GET | Get specific model details |

## Configuration

### YAML Config File

```yaml
server:
  port: 8080
  host: "0.0.0.0"

latency:
  profile: "gpt5"
  # Custom values (optional):
  # ttft_mean_ms: 600
  # ttft_stddev_ms: 150
  # tbt_mean_ms: 40
  # tbt_stddev_ms: 12

response:
  generator: "lorem"
  target_tokens: 100

errors:
  rate_limit_rate: 0.01
  server_error_rate: 0.001
  timeout_rate: 0.0
  timeout_after_ms: 30000

models:
  available:
    - "gpt-5"
    - "gpt-5-mini"
    - "gpt-4o"
    - "claude-opus"
```

## Supported Models

| Family | Models |
|--------|--------|
| GPT-5 | gpt-5, gpt-5-mini, gpt-5.1, gpt-5.2, gpt-5-codex |
| O-Series | o3, o3-mini, o4, o4-mini |
| GPT-4 | gpt-4, gpt-4-turbo, gpt-4o, gpt-4o-mini, gpt-4.1 |
| Claude | claude-opus, claude-sonnet, claude-haiku (with versions) |
| Gemini | gemini-pro |

## Latency Profiles

| Profile | TTFT Mean | TBT Mean |
|---------|-----------|----------|
| gpt-5 | 600ms | 40ms |
| gpt-5-mini | 300ms | 20ms |
| gpt-4 | 800ms | 50ms |
| gpt-4o | 400ms | 25ms |
| o-series | 2000ms | 30ms |
| claude-opus | 1000ms | 60ms |
| claude-sonnet | 500ms | 30ms |
| claude-haiku | 200ms | 15ms |
| instant | 0ms | 0ms |
| fast | 10ms | 1ms |

## Use Cases

- **Load Testing** - Simulate thousands of concurrent LLM requests
- **CI/CD Pipelines** - Fast, deterministic tests for LLM integrations
- **Local Development** - Develop without API keys or costs
- **Chaos Engineering** - Test behavior under failure scenarios
- **Cost Estimation** - Estimate token usage before production

## Requirements

- Rust 1.83+ (for building from source)
- OR Docker

## License

MIT License - see [LICENSE](LICENSE) for details.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for contribution guidelines.
