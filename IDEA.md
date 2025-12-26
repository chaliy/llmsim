# LLMSim - LLM Traffic Simulator

## Problem Statement

Load testing applications that integrate with LLMs (OpenAI, Anthropic, Google) is challenging because:

1. **Cost**: Real API calls during load tests are expensive
2. **Rate Limits**: Production APIs have rate limits that prevent realistic load testing
3. **Inconsistency**: Real models produce variable responses, making test reproducibility difficult
4. **Traffic Shape**: LLM responses have unique characteristics (streaming, variable latency, token-based billing) that generic mock servers don't replicate

## Solution

**LLMSim** is a lightweight, high-performance LLM API simulator that replicates the traffic shape of real LLM APIs without running actual models.

### Core Capabilities

1. **Realistic Latency Simulation**
   - Time-to-first-token (TTFT) delays
   - Inter-token latency (simulating token generation speed)
   - Model-specific latency profiles (GPT-4 is slower than GPT-3.5, etc.)

2. **Streaming Support**
   - Server-Sent Events (SSE) for OpenAI-compatible streaming
   - Chunk-by-chunk delivery with realistic timing

3. **Token Counting & Usage**
   - Accurate token counting for requests/responses
   - Usage metadata in responses (prompt_tokens, completion_tokens, total_tokens)

4. **Tool/Function Calling**
   - Support for OpenAI function calling format
   - Configurable tool call responses

5. **Error Simulation**
   - Rate limit errors (429)
   - Server errors (500, 503)
   - Timeout simulation
   - Configurable error rates

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                      llmsim                             │
├─────────────────────────────────────────────────────────┤
│  Binary Server (llmsim-server)                          │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐       │
│  │   OpenAI    │ │  Anthropic  │ │   Gemini    │       │
│  │  /v1/chat   │ │  /v1/msg    │ │  /v1/models │       │
│  └──────┬──────┘ └──────┬──────┘ └──────┬──────┘       │
│         │               │               │               │
│         └───────────────┼───────────────┘               │
│                         ▼                               │
│  ┌─────────────────────────────────────────────────┐   │
│  │              Core Library (llmsim)               │   │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────────────┐ │   │
│  │  │ Latency  │ │ Response │ │ Token Counter    │ │   │
│  │  │ Profiles │ │ Generator│ │ (tiktoken-rs)    │ │   │
│  │  └──────────┘ └──────────┘ └──────────────────┘ │   │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────────────┐ │   │
│  │  │ Streaming│ │  Error   │ │ Rate Limiter     │ │   │
│  │  │  Engine  │ │ Injector │ │                  │ │   │
│  │  └──────────┘ └──────────┘ └──────────────────┘ │   │
│  └─────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
```

## Use Cases

1. **Load Testing**: Simulate thousands of concurrent LLM requests without hitting real APIs
2. **CI/CD Pipelines**: Fast, deterministic tests for LLM integrations
3. **Local Development**: Develop against LLM APIs without API keys or costs
4. **Chaos Engineering**: Test application behavior under various failure scenarios
5. **Cost Estimation**: Estimate token usage before deploying to production

## Design Principles

1. **Single Binary**: Easy deployment, no runtime dependencies
2. **Low Resource Usage**: Efficient enough to run alongside the system under test
3. **API Compatibility**: Drop-in replacement for real LLM APIs
4. **Configurable**: Model profiles, error rates, and latency can be tuned
5. **Observable**: Metrics and logging for understanding simulator behavior

## Technology Choice: Rust

- **Performance**: Handle thousands of concurrent connections with minimal resources
- **Single Binary**: Easy distribution and deployment
- **Memory Safety**: Reliable under load without GC pauses
- **Ecosystem**: Strong async ecosystem (tokio, axum) for HTTP servers
- **tiktoken-rs**: Native token counting compatible with OpenAI's tokenizer

## License

MIT License - Free for commercial and open-source use.

## Future Extensions

- Response mocking (predefined responses for specific prompts)
- Record/replay mode (capture real API traffic, replay in tests)
- Prometheus metrics endpoint
- WebSocket support for real-time protocols
- Embeddings API simulation
- Image generation API simulation (DALL-E compatible)
