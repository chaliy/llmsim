# LLMSim API Reference

LLMSim provides two API providers with provider-namespaced routes.

## Providers

| Provider | Base Path | Description |
|----------|-----------|-------------|
| **OpenAI** | `/openai/v1/` | OpenAI-compatible Chat Completions and Responses API |
| **OpenResponses** | `/openresponses/v1/` | [OpenResponses](https://www.openresponses.org) specification |

## OpenAI API (`/openai/v1/...`)

### Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/openai/v1/chat/completions` | POST | Chat completions (streaming & non-streaming) |
| `/openai/v1/responses` | POST | Responses API (streaming & non-streaming) |
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
| GPT-5 | gpt-5, gpt-5-pro, gpt-5-mini, gpt-5-nano, gpt-5-codex, gpt-5.1, gpt-5.2, gpt-5.3-codex |
| O-Series | o1, o1-mini, o3, o3-mini, o4-mini |
| GPT-4 | gpt-4, gpt-4-turbo, gpt-4o, gpt-4o-mini, gpt-4.1, gpt-4.1-mini, gpt-4.1-nano |
| Claude | claude-3.5-sonnet, claude-3.7-sonnet, claude-sonnet-4, claude-sonnet-4.5, claude-opus-4, claude-opus-4.1, claude-opus-4.5, claude-opus-4.6, claude-haiku-4.5 |
| Gemini | gemini-2.0-flash, gemini-2.5-flash, gemini-2.5-pro |
| DeepSeek | deepseek-chat, deepseek-reasoner |

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
