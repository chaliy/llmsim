# LLMSim API Reference

LLMSim provides multiple API providers with provider-namespaced routes.

## Providers

| Provider | Base Path | Description |
|----------|-----------|-------------|
| **OpenAI** | `/openai/v1/` | OpenAI-compatible Chat Completions and Responses API |
| **OpenResponses** | `/openresponses/v1/` | [OpenResponses](https://www.openresponses.org) specification |
| **Anthropic** | `/anthropic/v1/` | [Anthropic Messages API](https://docs.anthropic.com/en/api/messages) |

## OpenAI API (`/openai/v1/...`)

### Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/openai/v1/chat/completions` | POST | Chat completions (streaming & non-streaming) |
| `/openai/v1/responses` | POST | Responses API (streaming & non-streaming) |
| `/openai/v1/responses` | WS | WebSocket mode for Responses API |
| `/openai/v1/models` | GET | List available models |
| `/openai/v1/models/:id` | GET | Get model details |

### Chat Completions

```bash
curl http://localhost:8080/openai/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5",
    "messages": [
      {"role": "system", "content": "You are a helpful assistant."},
      {"role": "user", "content": "Hello!"}
    ],
    "stream": false
  }'
```

#### Request Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `model` | string | Yes | Model ID (e.g., "gpt-5", "claude-opus-4.5") |
| `messages` | array | Yes | Array of message objects |
| `stream` | boolean | No | Enable streaming (default: false) |
| `temperature` | number | No | Sampling temperature (0-2) |
| `max_tokens` | integer | No | Maximum tokens to generate |
| `top_p` | number | No | Nucleus sampling parameter |

#### Multimodal (image) input

A message's `content` may be a plain string or an array of content parts, matching the OpenAI Chat Completions format:

```bash
curl http://localhost:8080/openai/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [
      {"role": "user", "content": [
        {"type": "text", "text": "What is in this image?"},
        {"type": "image_url", "image_url": {"url": "https://example.com/cat.png", "detail": "auto"}}
      ]}
    ]
  }'
```

Image parts require a vision-capable model. Sending an `image_url` part to a model whose profile reports no vision support (e.g. `gpt-4`) returns `400 invalid_request_error`. Custom model ids with no profile are accepted. Image content does not yet affect generated output.

#### Response

```json
{
  "id": "chatcmpl-abc123",
  "object": "chat.completion",
  "created": 1234567890,
  "model": "gpt-5",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "Hello! How can I help you today?"
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 10,
    "completion_tokens": 20,
    "total_tokens": 30
  }
}
```

### Responses API

```bash
curl http://localhost:8080/openai/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5",
    "input": "What is the capital of France?",
    "stream": false
  }'
```

#### Reasoning (Thinking) Emulation

For reasoning models (o-series and GPT-5 family), LLMSim emulates the `reasoning` output item with optional summary text. Pass a `reasoning` configuration to control effort and summary:

```bash
curl http://localhost:8080/openai/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "o3",
    "input": "What is 2+2?",
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    }
  }'
```

The response includes a `reasoning` output item before the `message`:

```json
{
  "output": [
    {
      "type": "reasoning",
      "id": "rs_abc123",
      "status": "completed",
      "summary": [{"type": "summary_text", "text": "The model considered..."}]
    },
    {
      "type": "message",
      "id": "msg_xyz789",
      "role": "assistant",
      "status": "completed",
      "content": [{"type": "output_text", "text": "2 + 2 = 4."}]
    }
  ],
  "usage": {
    "input_tokens": 5,
    "output_tokens": 8,
    "total_tokens": 37,
    "output_tokens_details": {"reasoning_tokens": 24}
  }
}
```

| Parameter | Values | Description |
|-----------|--------|-------------|
| `reasoning.effort` | `none`, `minimal`, `low`, `medium`, `high`, `xhigh` | Controls reasoning token count |
| `reasoning.summary` | `auto`, `concise`, `detailed` | Controls summary text generation |

When streaming, additional SSE events are emitted for the reasoning item (`response.reasoning_summary_text.delta`, etc.) before the message text deltas.

### WebSocket Mode

The Responses API also supports WebSocket transport for persistent connections, ideal for multi-turn agentic workflows with many tool calls.

**Connect:**
```
ws://localhost:8080/openai/v1/responses
```

**Send a `response.create` event (flat format, used by the OpenAI SDK):**
```json
{
  "type": "response.create",
  "model": "gpt-5",
  "input": [{"role": "user", "content": "Hello!"}]
}
```

The server sends back the same streaming events as the SSE format, but as plain JSON text frames (no `event:`/`data:` envelope).

**Multi-turn continuations:**
```json
{
  "type": "response.create",
  "model": "gpt-5",
  "input": [{"role": "user", "content": "Follow up question"}],
  "previous_response_id": "resp_abc123"
}
```

The most recent completed response is cached per connection. If `previous_response_id` doesn't match the cached response, a `previous_response_not_found` error is returned.

**Connection behavior:**
- Sequential execution (one response at a time per connection)
- 60-minute connection limit
- Supports `generate: false` for warmup requests

**Python example:**
```bash
uv run examples/openai_websocket_client.py
```

### List Models

```bash
curl http://localhost:8080/openai/v1/models
```

### Get Model

```bash
curl http://localhost:8080/openai/v1/models/gpt-5
```

## OpenResponses API (`/openresponses/v1/...`)

[OpenResponses](https://www.openresponses.org) is an open-source specification for building multi-provider, interoperable LLM interfaces.

### Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/openresponses/v1/responses` | POST | Create response (streaming & non-streaming) |

### Create Response

#### Text Input

```bash
curl http://localhost:8080/openresponses/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5",
    "input": "What is the capital of France?",
    "stream": false
  }'
```

#### Message-based Input

```bash
curl http://localhost:8080/openresponses/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5",
    "input": [
      {"role": "system", "content": "You are a helpful assistant."},
      {"role": "user", "content": "Hello!"}
    ],
    "stream": false
  }'
```

#### Request Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `model` | string | Yes | Model ID |
| `input` | string or array | Yes | Text or array of messages |
| `stream` | boolean | No | Enable streaming (default: false) |
| `temperature` | number | No | Sampling temperature |
| `max_output_tokens` | integer | No | Maximum tokens to generate |
| `top_p` | number | No | Nucleus sampling parameter |
| `tools` | array | No | Tool definitions |
| `reasoning` | object | No | Reasoning configuration (for o-series) |
| `metadata` | object | No | Custom metadata |

#### Response

```json
{
  "id": "resp_abc123",
  "object": "response",
  "created_at": 1234567890,
  "completed_at": 1234567891,
  "model": "gpt-5",
  "status": "completed",
  "output": [
    {
      "type": "message",
      "id": "msg_xyz789",
      "role": "assistant",
      "content": [
        {
          "type": "output_text",
          "text": "The capital of France is Paris."
        }
      ],
      "status": "completed"
    }
  ],
  "usage": {
    "input_tokens": 10,
    "output_tokens": 15,
    "total_tokens": 25
  }
}
```

### Streaming

When `stream: true`, the response is sent as Server-Sent Events (SSE):

```bash
curl http://localhost:8080/openresponses/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5",
    "input": "Tell me a story",
    "stream": true
  }'
```

#### Stream Events

| Event Type | Description |
|------------|-------------|
| `response.created` | Response object created |
| `response.in_progress` | Response generation started |
| `response.output_item.added` | New output item added |
| `response.reasoning_summary_part.added` | Reasoning summary part added (reasoning models) |
| `response.reasoning_summary_text.delta` | Reasoning summary text chunk (reasoning models) |
| `response.reasoning_summary_text.done` | Reasoning summary text complete (reasoning models) |
| `response.reasoning_summary_part.done` | Reasoning summary part complete (reasoning models) |
| `response.content_part.added` | New content part added |
| `response.output_text.delta` | Text chunk (content delta) |
| `response.output_text.done` | Text generation complete |
| `response.content_part.done` | Content part complete |
| `response.output_item.done` | Output item complete |
| `response.completed` | Response complete with usage |

### Reasoning Configuration

For o-series models (o3, o4), you can configure reasoning:

```bash
curl http://localhost:8080/openresponses/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "o3",
    "input": "Solve this complex problem",
    "reasoning": {
      "effort": "high",
      "summary": "detailed"
    }
  }'
```

## Anthropic API (`/anthropic/v1/...`)

Simulates the [Anthropic Messages API](https://docs.anthropic.com/en/api/messages).
When using an Anthropic SDK, set the base URL to `http://localhost:8080/anthropic`.

### Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/anthropic/v1/messages` | POST | Messages API (streaming & non-streaming) |
| `/anthropic/v1/models` | GET | List available Claude models |
| `/anthropic/v1/models/:id` | GET | Get model details |

### Messages

```bash
curl http://localhost:8080/anthropic/v1/messages \
  -H "content-type: application/json" \
  -H "x-api-key: not-needed" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model": "claude-opus-4-8",
    "max_tokens": 64,
    "system": "You are a helpful assistant.",
    "messages": [{"role": "user", "content": "What is the capital of France?"}]
  }'
```

#### Request Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `model` | string | yes | Anthropic model ID (e.g. `claude-opus-4-8`) |
| `max_tokens` | integer | yes | Maximum tokens to generate |
| `messages` | array | yes | Conversation turns (`user`/`assistant`); content is a string or content blocks |
| `system` | string \| array | no | System prompt |
| `temperature` | number | no | |
| `top_p` | number | no | |
| `top_k` | integer | no | |
| `stop_sequences` | array | no | |
| `stream` | boolean | no | Stream Server-Sent Events |
| `tools` | array | no | Tool definitions |
| `tool_choice` | object | no | |
| `metadata` | object | no | E.g. `{"user_id": "..."}` |

#### Response

```json
{
  "id": "msg_abc123",
  "type": "message",
  "role": "assistant",
  "model": "claude-opus-4-8",
  "content": [{"type": "text", "text": "The capital of France is Paris."}],
  "stop_reason": "end_turn",
  "stop_sequence": null,
  "usage": {"input_tokens": 10, "output_tokens": 8}
}
```

### Streaming

When `stream: true`, the response is the Anthropic Server-Sent Event sequence.
Each event carries an explicit `event:` line, and the stream ends after
`message_stop` with **no** `[DONE]` sentinel.

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

#### Stream Events

| Event Type | Description |
|------------|-------------|
| `message_start` | Message object created (seeds `usage.input_tokens`) |
| `content_block_start` | Text content block opened at `index` 0 |
| `ping` | Keep-alive |
| `content_block_delta` | Text chunk (`delta.type == "text_delta"`) |
| `content_block_stop` | Content block complete |
| `message_delta` | Final `stop_reason` + cumulative `usage.output_tokens` |
| `message_stop` | Stream complete |

### List Models

```bash
curl http://localhost:8080/anthropic/v1/models
```

### Get Model

```bash
curl http://localhost:8080/anthropic/v1/models/claude-opus-4-8
```

### Errors

Errors use the Anthropic error envelope:

```json
{"type": "error", "error": {"type": "rate_limit_error", "message": "..."}}
```

## LLMSim Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Health check |
| `/llmsim/stats` | GET | Real-time server statistics |

### Health Check

```bash
curl http://localhost:8080/health
```

### Server Statistics

```bash
curl http://localhost:8080/llmsim/stats
```

#### Response

```json
{
  "uptime_secs": 3600,
  "total_requests": 15000,
  "active_requests": 5,
  "streaming_requests": 12000,
  "non_streaming_requests": 3000,
  "prompt_tokens": 500000,
  "completion_tokens": 1500000,
  "total_tokens": 2000000,
  "total_errors": 150,
  "rate_limit_errors": 100,
  "server_errors": 30,
  "timeout_errors": 20,
  "requests_per_second": 4.2,
  "avg_latency_ms": 245.5,
  "min_latency_ms": 50.0,
  "max_latency_ms": 2500.0,
  "model_requests": {
    "gpt-5": 10000,
    "gpt-4o": 5000
  }
}
```

## Supported Models

| Family | Models |
|--------|--------|
| GPT-5 | gpt-5, gpt-5-pro, gpt-5-mini, gpt-5-nano, gpt-5-codex, gpt-5.1, gpt-5.2, gpt-5.3-codex, gpt-5.3-codex-spark, gpt-5.3-chat-latest, gpt-5.4, gpt-5.4-pro, gpt-5.4-mini, gpt-5.4-nano, gpt-5.5, gpt-5.5-pro |
| O-Series | o1, o1-mini, o3, o3-mini, o4-mini |
| GPT-4 | gpt-4, gpt-4-turbo, gpt-4o, gpt-4o-mini, gpt-4.1, gpt-4.1-mini, gpt-4.1-nano |
| Claude | claude-3.5-sonnet, claude-3.7-sonnet, claude-sonnet-4, claude-sonnet-4.5, claude-sonnet-4.6, claude-opus-4, claude-opus-4.1, claude-opus-4.5, claude-opus-4.6, claude-opus-4.7, claude-opus-4.8, claude-haiku-4.5 |
| Gemini | gemini-2.0-flash, gemini-2.5-flash, gemini-2.5-pro, gemini-3-pro-preview, gemini-3-flash-preview, gemini-3.1-pro-preview, gemini-3.1-flash-lite |
| DeepSeek | deepseek-chat, deepseek-reasoner |

## Scripted Mode

For agent scenario tests, llmsim can replay a deterministic
multi-turn script (text, tool calls, mixed turns, errors) instead of
running a generator. Enable it in your config:

```toml
[response]
script_path = "/path/to/script.json"
```

The script JSON has an `on_exhausted` policy (`repeat_last` /
`error` / `loop`) and a `turns` array. Example:

```json
{
  "on_exhausted": "error",
  "turns": [
    {"type": "tool_calls", "calls": [
      {"name": "bash", "arguments": {"command": "ls"}}
    ]},
    {"type": "assistant", "text": "done"}
  ]
}
```

See [`specs/scripted-mode.md`](../specs/scripted-mode.md) for the
full format, turn variants (`assistant` / `tool_calls` / `mixed` /
`error`), and per-endpoint coverage. Example script and clients live
in [`examples/scripted_demo/`](../examples/scripted_demo/).

## Error Responses

Errors follow OpenAI/OpenResponses format:

```json
{
  "error": {
    "message": "Rate limit exceeded. Please retry after some time.",
    "type": "rate_limit_error",
    "code": "rate_limit_exceeded"
  }
}
```

### Error Codes

| Status | Type | Description |
|--------|------|-------------|
| 429 | `rate_limit_error` | Rate limit exceeded |
| 500 | `server_error` | Internal server error |
| 503 | `server_error` | Service unavailable |
| 504 | `timeout_error` | Gateway timeout |
