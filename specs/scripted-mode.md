# Scripted response mode

## Abstract

llmsim's default generators (`lorem`, `echo`, `fixed`, `random`, `sequence`)
produce one response shape per session — fine for boot-up smoke tests, not
enough to drive **agent scenario tests** which need multi-turn loops,
tool calls, error recovery, and model switches.

Scripted mode lets a caller provide a JSON file describing an ordered list
of assistant turns. The server replays the script across requests, advancing
an atomic cursor on each call. When the cursor outruns the script, behaviour
is configurable (repeat last, error, loop).

This unlocks tests like "agent calls `bash`, then `write_file`, then
reports done" — driven against a real HTTP server without an API key,
without flakiness, and without paying for tokens.

## Requirements

### Configuration

- A new optional field `response.script_path` in `Config` points at a JSON
  file. When set, the server loads the script at startup and ignores
  `response.generator` and `response.target_tokens`.
- An invalid script (missing file, malformed JSON, empty turns array) fails
  startup with a descriptive error.

### Script file format

The file is a JSON object:

```json
{
  "on_exhausted": "repeat_last",
  "turns": [
    {"type": "assistant", "text": "Hello"},
    {"type": "tool_calls", "calls": [
      {"name": "bash", "arguments": {"command": "ls"}}
    ]},
    {"type": "mixed", "text": "Running ls", "calls": [
      {"name": "bash", "arguments": {"command": "ls"}, "id": "call_42"}
    ]},
    {"type": "error", "kind": "rate_limit"},
    {"type": "error", "kind": "invalid_request", "message": "bad args"},
    {"type": "error", "kind": "other", "message": "boom", "status_code": 502}
  ]
}
```

#### Turn variants

- **`assistant`** — plain text response. `finish_reason` is `stop`.
- **`tool_calls`** — one or more tool calls in a single assistant turn.
  Assistant message has no text content; `finish_reason` is `tool_calls`.
- **`mixed`** — text plus tool calls in the same turn. `finish_reason`
  is `tool_calls`.
- **`error`** — return an HTTP error for this turn. The cursor still
  advances so the next request gets the next turn (agent retry behaviour
  is testable). Supported `kind`s:
  - `rate_limit` → HTTP 429
  - `timeout` → HTTP 504 (returns immediately; no actual sleep)
  - `invalid_request` → HTTP 400 with `message`
  - `other` → defaults to HTTP 500; override with `status_code`

#### Tool call structure

```json
{"name": "<function_name>", "arguments": <json_value>, "id": "<optional>"}
```

`arguments` is any JSON value; llmsim serialises it to a string on the
wire to match OpenAI's `function.arguments` shape.

`id` is optional — when absent, llmsim generates one of the form
`call_llmsim_<turn_index>_<call_index>` so they are stable and unique
across the whole script (useful for correlating function_call_output
items in subsequent requests).

### `on_exhausted`

What happens after the last turn has been consumed and another request
arrives:

| Value | Behaviour |
|-------|-----------|
| `repeat_last` (default) | Replay the final turn forever. Matches the existing `fixed` generator. |
| `error` | Return HTTP 500 with a message that the script is exhausted. Useful for asserting the agent stopped on its own. |
| `loop` | Cycle back to the start of the script. |

### Endpoint coverage

Scripted mode is wired into all three response endpoints with these
guarantees:

| Endpoint | Text turns | Tool calls | Error turns | Streaming |
|----------|------------|------------|-------------|-----------|
| `POST /openai/v1/chat/completions` | ✓ | ✓ | ✓ | ✓ (incl. tool call deltas) |
| `POST /openai/v1/responses` | ✓ | ✓ (non-streaming only) | ✓ (non-streaming only) | text only |
| `POST /openresponses/v1/responses` | ✓ | reduced to text | ✓ | text only |

Streaming `Responses API` tool-call events (`response.output_item.added`
for `function_call`, `response.function_call_arguments.delta`, etc.) are
intentionally deferred — the existing `ResponsesTokenStream` is text-only
and lifting tool-call support is a much larger change than the scenario
test use case requires.

### Streaming format

For chat completions, scripted streaming follows the OpenAI wire shape:

1. Role chunk (`{"delta": {"role": "assistant"}}`)
2. Word-boundary content deltas (one chunk per whitespace-bounded token)
3. For each tool call: one "announce" chunk (name + id + empty args)
followed by one "arguments" chunk (full JSON args string)
4. Finish chunk with `finish_reason` = `"stop"` or `"tool_calls"`
5. `data: [DONE]\n\n`

Latency between chunks uses the configured `LatencyProfile`, so streaming
behaviour mirrors what the agent would see from a real provider.

### Concurrency

The cursor is an atomic counter. Concurrent requests serialise through
`fetch_add` — a 2-turn script driven by 2 in-flight requests yields
`turn[0]` then `turn[1]` regardless of which task lands first. The unit
test `script::cursor_is_thread_safe` asserts this with 10 threads racing
over a 100-turn script.

### Stats integration

Scripted requests participate in the normal stats tracking
(`record_request_start`, `record_request_end`, `record_error`). Error
turns increment the appropriate status-code counters so the dashboard
shows scripted errors the same as injected ones.

### Public Rust API

Re-exported from the crate root for embedded use:

```rust
use llmsim::{
    OnExhausted, Script, ScriptSpec, ScriptedResponse,
    SimError, SimToolCall, SimTurn,
};

let script = Script::new(vec![
    SimTurn::ToolCalls {
        calls: vec![SimToolCall {
            name: "bash".into(),
            arguments: serde_json::json!({"command": "ls"}),
            id: None,
        }],
    },
    SimTurn::Assistant { text: "done".into() },
])
.with_on_exhausted(OnExhausted::Error);
```

Or load from a file:

```rust
let script = Script::from_file("/path/to/script.json")?;
```

Wire into an `AppState` via `AppState::with_script(Arc::new(script))`.

## Non-goals

- No request-side matching (no `SimMatcher`). v1 advances strictly on
  request count; the script does not inspect outgoing messages. The
  proposal's stretch `When { matches, respond }` variant is deferred.
- No token-level latency or partial-failure recovery beyond what
  `LatencyProfile` already provides.
- Not a wire-format mock of any specific provider — this is a fixture
  for agent loops, not a recording proxy.

## Future work

- Add `SimMatcher` for request-aware scripting (assert the agent sent
  the right tool result before responding).
- Add streaming tool-call events to the Responses API path.
- Add a small CLI helper (`llmsim script validate <path>`) to lint a
  script file without booting the server.
