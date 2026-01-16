# Examples

## API Endpoints

LLMSim provides two API providers:

| Provider | Base Path | Description |
|----------|-----------|-------------|
| **OpenAI** | `/openai/v1/` | OpenAI-compatible Chat Completions and Responses API |
| **OpenResponses** | `/openresponses/v1/` | [OpenResponses](https://www.openresponses.org) specification |

### OpenAI API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/openai/v1/chat/completions` | POST | Chat completions (streaming supported) |
| `/openai/v1/responses` | POST | Responses API (streaming supported) |
| `/openai/v1/models` | GET | List available models |
| `/openai/v1/models/:id` | GET | Get model details |

### OpenResponses API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/openresponses/v1/responses` | POST | Create response (streaming supported) |

## Running the Examples

All examples require the server to be running first:

```bash
# Start server (headless)
cargo run --release -- serve --port 8080

# Or start with TUI dashboard to watch stats in real-time
cargo run --release -- serve --port 8080 --tui
```

## Rust Example

Demonstrates library usage: token counting, generators, latency profiles, and streaming.

```bash
cargo run --example basic_usage
```

## Python Examples

### OpenAI SDK

Direct usage of the official OpenAI Python library:

```bash
uv run examples/openai_client.py
```

### OpenResponses API

Using the OpenResponses specification with httpx:

```bash
uv run examples/openresponses_client.py
```

### LangChain

Using LangChain's OpenAI-compatible client:

```bash
uv run examples/langchain_client.py
```

## TypeScript Example

Direct usage of the official OpenAI Node.js library:

```bash
npm install openai
npx tsx examples/openai_client.ts
```

## Custom Server URL

```bash
# OpenAI examples
LLMSIM_URL=http://localhost:9000/openai/v1 uv run examples/openai_client.py

# OpenResponses example
LLMSIM_URL=http://localhost:9000 uv run examples/openresponses_client.py
```

## Stats API

You can fetch real-time server statistics:

```bash
curl http://localhost:8080/llmsim/stats | jq
```

Response includes:
- Request counts (total, active, streaming, non-streaming)
- Token usage (prompt, completion, total)
- Error breakdown (rate limits, server errors, timeouts)
- Latency metrics (avg, min, max)
- Requests per second
- Per-model request distribution

## Quick API Test

### OpenAI Chat Completions

```bash
curl http://localhost:8080/openai/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

### OpenAI Responses API

```bash
curl http://localhost:8080/openai/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5",
    "input": "What is 2+2?"
  }'
```

### OpenResponses API

```bash
curl http://localhost:8080/openresponses/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5",
    "input": "Hello, world!"
  }'
```

### Streaming (OpenResponses)

```bash
curl http://localhost:8080/openresponses/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5",
    "input": "Tell me a story",
    "stream": true
  }'
```
