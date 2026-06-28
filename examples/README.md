# Examples

## API Endpoints

LLMSim provides two API providers:

| Provider | Base Path | Description |
|----------|-----------|-------------|
| **OpenAI** | `/openai/v1/` | OpenAI-compatible Chat Completions and Responses API |
| **OpenResponses** | `/openresponses/v1/` | [OpenResponses](https://www.openresponses.org) specification |
| **Anthropic** | `/anthropic/v1/` | [Anthropic Messages API](https://docs.anthropic.com/en/api/messages) |

### OpenAI API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/openai/v1/chat/completions` | POST | Chat completions (streaming supported) |
| `/openai/v1/responses` | POST | Responses API (streaming supported) |
| `/openai/v1/responses` | WS | WebSocket mode for Responses API |
| `/openai/v1/models` | GET | List available models |
| `/openai/v1/models/:id` | GET | Get model details |

### OpenResponses API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/openresponses/v1/responses` | POST | Create response (streaming supported) |

### Anthropic API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/anthropic/v1/messages` | POST | Messages API (streaming supported) |
| `/anthropic/v1/models` | GET | List available Claude models |
| `/anthropic/v1/models/:id` | GET | Get model details |

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

### WebSocket Mode

Using the WebSocket transport for the Responses API:

```bash
uv run examples/openai_websocket_client.py
```

### LangChain

Using LangChain's OpenAI-compatible client:

```bash
uv run examples/langchain_client.py
```

### Pydantic AI

Using [Pydantic AI](https://ai.pydantic.dev) via its OpenAI-compatible provider:

```bash
uv run examples/pydantic_ai_client.py
```

> Structured `output_type` results and tool calling require the server to be
> running in [scripted mode](../specs/scripted-mode.md); a default server
> returns simulated text.

### Anthropic SDK

Direct usage of the official Anthropic Python SDK (messages, streaming, tools,
models):

```bash
uv run examples/anthropic_client.py
```

### Anthropic via LangChain

Using LangChain's `ChatAnthropic` client:

```bash
uv run examples/anthropic_langchain.py
```

### Scripted mode

Drive a deterministic multi-turn script (tool calls, errors, mixed
text+calls) for agent scenario tests:

```bash
# Boot the server with a script file
cargo run -- serve --config examples/scripted_demo/scripted_demo.toml

# In another shell
uv run examples/scripted_demo/scripted_demo.py
```

See [`specs/scripted-mode.md`](../specs/scripted-mode.md) for the full
script JSON format.

## TypeScript Examples

The TypeScript examples live in their own folder with a pinned
`package.json`/`package-lock.json`:

```bash
cd examples/node
npm install
```

### OpenAI SDK

Direct usage of the official OpenAI Node.js library:

```bash
npx tsx openai_client.ts
```

### Vercel AI SDK

Using the [Vercel AI SDK](https://sdk.vercel.ai) via its OpenAI-compatible provider:

```bash
npx tsx vercel_ai_client.ts
```

> `generateObject` and tool calling require the server to be running in
> [scripted mode](../specs/scripted-mode.md); a default server returns
> simulated text.

### Anthropic SDK

Direct usage of the official Anthropic Node.js SDK:

```bash
npx tsx anthropic_client.ts
```

## Go Example

Direct usage of the official Anthropic Go SDK (the module is committed under
`examples/go`, so no setup is needed):

```bash
cd examples/go
LLMSIM_URL=http://localhost:8080/anthropic/ go run anthropic_client.go
```

## Shell / curl Example

Raw HTTP against the Anthropic Messages API:

```bash
./examples/anthropic_curl.sh
```

## Custom Server URL

```bash
# OpenAI examples
LLMSIM_URL=http://localhost:9000/openai/v1 uv run examples/openai_client.py

# OpenResponses example
LLMSIM_URL=http://localhost:9000 uv run examples/openresponses_client.py

# Anthropic example
LLMSIM_URL=http://localhost:9000/anthropic uv run examples/anthropic_client.py
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

### Anthropic Messages API

```bash
curl http://localhost:8080/anthropic/v1/messages \
  -H "content-type: application/json" \
  -H "x-api-key: not-needed" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model": "claude-opus-4-8",
    "max_tokens": 64,
    "messages": [{"role": "user", "content": "Hello, Claude!"}]
  }'
```

### Streaming (Anthropic)

```bash
curl -N http://localhost:8080/anthropic/v1/messages \
  -H "content-type: application/json" \
  -d '{
    "model": "claude-haiku-4-5",
    "max_tokens": 64,
    "stream": true,
    "messages": [{"role": "user", "content": "Tell me a story"}]
  }'
```
