---
type: Specification
title: "Anthropic Messages API Specification"
description: "Wire-compatible simulation of the Anthropic Messages, token counting, and models APIs."
tags:
  - llmsim
  - apis
---
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
| `thinking` | object | no | `{"type": "adaptive" \| "enabled" \| "disabled" \| "between_tools", "budget_tokens"?, "display"?}` (see R7) |
| `output_config` | object | no | `{"effort": "low" \| "medium" \| "high" \| "xhigh" \| "max", "format"?}` (see R7) |

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

**R1.6**: When a thinking block is produced (R7), the non-streaming `content`
array starts with `{"type": "thinking", "thinking": "...", "signature": "..."}`
followed by the `text` block.

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

**R2.1.1**: When a thinking block is produced (R7), it streams first at
`index` 0: `content_block_start` with `{"type": "thinking", "thinking": "",
"signature": ""}`, zero or more `thinking_delta` deltas (none when display is
omitted), one `signature_delta`, then `content_block_stop`. The text block
follows at `index` 1.

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
`claude-opus-5-5`, `claude-sonnet-5-5`, `claude-haiku-5-5`, `claude-fable-5-1`).
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

**R5.1**: When the server runs with a script (see [scripted-mode](../simulation/scripted-mode.md)), the
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

### R7: Extended Thinking and Effort

**R7.1**: Generated (non-scripted) responses carry a `thinking` block when
thinking is on: always on Claude 5.x / Fable models (`claude-fable-*`,
`claude-opus-5*`, `claude-sonnet-5*`, `claude-haiku-5*`) unless `thinking.type`
is `disabled` or `between_tools`; on other models only when `thinking.type` is
`adaptive` or `enabled`. Scripted turns never add a thinking block.

**R7.2**: Thinking tokens scale with `output_config.effort` (default `high`,
`medium` on Opus 5.5 / Haiku 5.5), are capped by `thinking.budget_tokens` when
set, and are billed inside `usage.output_tokens`.

**R7.3**: The thinking text is a synthetic summary when `thinking.display` is
`summarized`, and empty when it is `omitted` or `updates`. Without `display`,
Opus 4.7+ and 5.x / Fable models default to omitted; older models to
summarized. Each block carries a random opaque `signature`.

**R7.4**: The endpoint MUST reject with `400 invalid_request_error`:
- an `output_config.effort` outside `low`/`medium`/`high`/`xhigh`/`max`;
- `thinking.type: "enabled"` on models where `budget_tokens` was removed
  (Opus 4.7+, Sonnet 5.x, Haiku 5.5, Fable);
- `thinking.type: "disabled"` on Fable, Opus 5.5, and Sonnet 5.5;
- `thinking.type: "enabled"` without `budget_tokens`, with `budget_tokens`
  below 1024, or with `budget_tokens >= max_tokens`;
- an unknown `thinking.type`.

Unknown/custom model IDs are never rejected for model-specific rules.

### R8: Token Counting

**R8.1**: Implement `POST /anthropic/v1/messages/count_tokens`, accepting the
Messages request shape without `max_tokens`/`stream`, and returning
`{"input_tokens": N}`.

**R8.2**: `input_tokens` MUST equal the `usage.input_tokens` a Messages call
with the same `model`, `system`, and `messages` reports.

**R8.3**: `thinking` / `output_config` are validated with the R7.4 rules
(except the `max_tokens` bound).

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
- Real token-budget enforcement, real chain-of-thought content, prompt
  caching, the Batches API, the Files API, or Managed Agents.
- Per-model rules beyond R7.4 (forced `tool_choice` rejection, sampling
  parameter rejection, preserved-thinking history checks).
- A WebSocket transport for Messages (the real API has none).
