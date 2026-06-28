# Anthropic Messages API Specification

## Abstract

This specification defines LLMSim's simulation of the [Anthropic Messages
API](https://docs.anthropic.com/en/api/messages). The goal is wire-format
compatibility: the official Anthropic SDKs (Python `anthropic`,
`@anthropic-ai/sdk`, `anthropic-sdk-go`, ...) work unchanged when pointed at
`{base_url}/anthropic`, so agentic workflows and integrations can be tested
without API cost or running a real model.

## Requirements

### R1: Messages Endpoint

**R1.1**: Implement `POST /anthropic/v1/messages` accepting Messages API
requests and returning simulated responses.

**R1.2**: The request body MUST support at least these fields:

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `model` | string | yes | Anthropic model ID (see R4) |
| `max_tokens` | integer | yes | Maximum tokens to generate |
| `messages` | array | yes | Conversation turns (`user`/`assistant`) |
| `system` | string \| array | no | System prompt (string or text blocks) |
| `temperature` | number | no | |
| `top_p` | number | no | |
| `top_k` | integer | no | |
| `stop_sequences` | array | no | |
| `stream` | boolean | no | Defaults to `false` |
| `tools` | array | no | Tool definitions |
| `tool_choice` | object | no | |
| `metadata` | object | no | E.g. `{"user_id": "..."}` |

**R1.3**: `messages[].content` MUST accept either a bare string or an array of
content blocks. The simulator MUST tolerate (without erroring) content block
types it does not interpret, e.g. `image`, `document`, `thinking`, `tool_use`,
and `tool_result` — only the embedded text contributes to the prompt.

**R1.4**: A non-streaming response MUST have this shape:

```json
{
  "id": "msg_<hex>",
  "type": "message",
  "role": "assistant",
  "model": "claude-opus-4-8",
  "content": [{"type": "text", "text": "..."}],
  "stop_reason": "end_turn",
  "stop_sequence": null,
  "usage": {"input_tokens": 10, "output_tokens": 25}
}
```

**R1.5**: `stop_reason` MUST be one of `end_turn`, `max_tokens`,
`stop_sequence`, `tool_use`, `pause_turn`, `refusal`. Default text responses use
`end_turn`; scripted tool-call turns use `tool_use`.

### R2: Streaming

**R2.1**: When `stream: true`, the endpoint MUST emit the Anthropic streaming
event sequence as Server-Sent Events, in order:

1. `message_start`
2. `content_block_start`
3. `ping` (optional keep-alive; the simulator emits one)
4. `content_block_delta` (one per token, `delta.type == "text_delta"`)
5. `content_block_stop`
6. `message_delta` (carries final `stop_reason` and cumulative `usage.output_tokens`)
7. `message_stop`

**R2.2**: Each event MUST carry both an `event:` line and a `data:` line.

**R2.3**: The stream MUST terminate after `message_stop` with **no** `[DONE]`
sentinel (unlike the OpenAI SSE format).

**R2.4**: `message_start` MUST seed `usage.input_tokens`; `message_delta` MUST
report the final `usage.output_tokens`.

### R3: Errors

**R3.1**: Errors MUST use the Anthropic error envelope:

```json
{"type": "error", "error": {"type": "invalid_request_error", "message": "..."}}
```

**R3.2**: The inner `error.type` MUST be derived from the HTTP status:

| Status | `error.type` |
|--------|--------------|
| 400 | `invalid_request_error` |
| 401 | `authentication_error` |
| 403 | `permission_error` |
| 404 | `not_found_error` |
| 413 | `request_too_large` |
| 429 | `rate_limit_error` |
| 500 | `api_error` |
| 529 | `overloaded_error` |

**R3.3**: Injected errors (via the error-injection config) and scripted error
turns MUST both render through this envelope. A `429` SHOULD include a
`Retry-After` header.

### R4: Models

**R4.1**: Implement `GET /anthropic/v1/models` and
`GET /anthropic/v1/models/:model_id`.

**R4.2**: Model IDs MUST use the real Anthropic API form (dash-separated, e.g.
`claude-opus-4-8`, `claude-sonnet-4-6`, `claude-haiku-4-5`, `claude-fable-5`).
Dated snapshot IDs (e.g. `claude-haiku-4-5-20251001`) and `-latest` aliases
(e.g. `claude-3-5-sonnet-latest`) MUST resolve to the same profile.

**R4.3**: A model object MUST have:

```json
{
  "type": "model",
  "id": "claude-opus-4-8",
  "display_name": "Claude Opus 4.8",
  "created_at": "2026-05-20T00:00:00Z",
  "max_input_tokens": 1000000,
  "max_tokens": 128000
}
```

**R4.4**: `GET /anthropic/v1/models` MUST return the paginated list envelope
(`data`, `first_id`, `last_id`, `has_more`).

**R4.5**: `GET /anthropic/v1/models/:id` for an unknown model MUST return `404`
with the Anthropic error envelope (`error.type == "not_found_error"`).

**R4.6**: Model profiles are sourced from [models.dev](https://models.dev) and
the Anthropic model documentation. Each profile carries a realistic context
window, max output tokens, capabilities, and (where published) a knowledge
cutoff.

### R5: Scripted Mode

**R5.1**: When the server runs with a script (see `specs/scripted-mode.md`), the
Messages endpoint MUST replay scripted turns:

- `assistant` turns → a single `text` content block, `stop_reason: end_turn`.
- `tool_calls` turns → one `tool_use` content block per call,
  `stop_reason: tool_use`. Missing IDs are auto-assigned a `toolu_`-prefixed id.
- `mixed` turns → a `text` block followed by `tool_use` blocks,
  `stop_reason: tool_use`.
- `error` turns → the Anthropic error envelope with the mapped status.

### R6: Latency and Stats

**R6.1**: Absent an explicit latency override, the endpoint MUST select a
model-derived latency profile (Opus/Sonnet/Haiku) from the model ID.

**R6.2**: Each request MUST be recorded in stats under a dedicated
`messages_requests` counter, in addition to the shared request/token counters.

## Rationale

- **SDK compatibility**: Using the exact Anthropic wire shape (including the
  `event:`-line SSE format and the no-`[DONE]` termination) means the official
  SDKs' streaming helpers (`text_stream`, `get_final_message()`, `finalMessage()`)
  work without modification.
- **Real model IDs**: The OpenAI-oriented registry uses dotted IDs
  (`claude-opus-4.8`); the Anthropic API uses dashed IDs (`claude-opus-4-8`).
  A separate Anthropic registry keyed on the real IDs (plus aliases) lets
  SDK-issued model strings resolve.

## Non-Requirements

- Authentication/authorization (LLMSim is for local testing; the `x-api-key`
  and `anthropic-version` headers are accepted but ignored).
- Real token-budget enforcement, extended-thinking content, prompt caching,
  the Batches API, the Files API, or Managed Agents.
- A WebSocket transport for Messages (the real API has none).
