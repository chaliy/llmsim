# Scenarios for load testing

## Abstract

Named scenarios let a load test replay realistic agent conversations (tool
calls, long answers, reasoning, slow first tokens, stalls, a 429 halfway,
a dropped stream) across thousands of concurrent sessions with no shared
state. A scenario is a list of steps. A marker in the user message picks the
scenario, and the step to play is counted from the conversation itself: the
number of assistant replies since that user message.

Scripted mode (`specs/scripted-mode.md`) plays one script per server through
a single global cursor. That breaks as soon as two sessions share the server,
a turn retries, or a worker restarts. Counting steps from the conversation
fixes all three, because every request carries its own history. Scripted mode
stays as it is for single-agent tests.

The engine is a library module (`llmsim::scenario`) so embedders (Everruns'
llmsim driver, yolop) can call the pure `resolve` function directly; the
server wires it into the chat completions endpoint.

## Requirements

### Scenario format

A scenario is a JSON object: a name (`[a-z0-9-]+`) plus an ordered list of
steps. Each step is one model call.

```json
{
  "name": "research-3-tools",
  "description": "Three tool rounds, then a long answer. 429 on the second call.",
  "defaults": { "timing": { "ttft_ms": [800, 1300], "tokens_per_sec": 65 } },
  "steps": [
    { "turn": { "type": "mixed", "text": "Let me check the repo.",
                "calls": [{ "tool": "any_read", "arguments": {} }] } },
    { "fail_first": [{ "kind": "rate_limit", "retry_after_ms": 1000 }],
      "turn": { "type": "tool_calls",
                "calls": [{ "name": "bash", "arguments": { "command": "echo step 2" } }] } },
    { "timing": { "ttft_ms": 2500 },
      "turn": { "type": "tool_calls", "calls": [{ "tool": "any_read" }, { "tool": "any_read" }] } },
    { "timing": { "tokens_per_sec": 40, "stall": { "after_tokens": 120, "ms": 3000 } },
      "turn": { "type": "assistant", "lorem_tokens": 600 } }
  ],
  "on_exhausted": "repeat_last"
}
```

| Field | Type | Meaning |
|-------|------|---------|
| `turn` | object | What the model returns: `assistant`, `tool_calls`, `mixed` or `error`. |
| `turn.text` | string | Fixed text. Mutually exclusive with `lorem_tokens`; one is required for `assistant` and `mixed`. |
| `turn.lorem_tokens` | number or [min, max] | Generated lorem text of this many words. |
| `turn.reasoning_tokens` | number or [min, max] | Reasoning text emitted before the answer. |
| `turn.calls[].name` | string | An exact tool name. If the request does not offer it, the call fails with a 400. |
| `turn.calls[].tool` | placeholder | `any_read`, `any_write` or `any`: picks a tool the request offers (read-like or write-like by name; any tool when none match). A request offering no tools fails with a 400. |
| `turn.calls[].arguments` | any | Arguments. When absent, the tool's required JSON-schema parameters are filled with placeholders (`"."` for path-like strings, `"llmsim"` for other strings, `1`, `false`, `[]`, first `enum` value). |
| `turn` (error) | `{"type":"error","kind":...}` | Same kinds as scripted mode (`rate_limit`, `timeout`, `invalid_request`, `other`), plus optional `retry_after_ms`. Every attempt fails. |
| `timing.ttft_ms` | number or [min, max] | Time to first token. A range is sampled per step. |
| `timing.tokens_per_sec` | number | Streaming speed after the first token. |
| `timing.stall` | {after_tokens, ms} | Pause mid-stream, to exercise idle timeouts and SSE keep-alives. |
| `timing.cut_after_tokens` | number | Drop the connection after this many tokens without a finish event. Applies to the first attempt that `fail_first` does not fail, so the retry succeeds. |
| `fail_first` | list of errors | Errors returned on the first attempts of this step, in order, before it succeeds. Each may set `retry_after_ms`. |
| `defaults.timing` | object | Scenario-wide timing; each step overrides it field by field. |
| `defaults.error_rate` | {kind: rate} | Background error rate per attempt; kinds `rate_limit`, `timeout`, `server_error`. |
| `on_exhausted` | enum | `repeat_last` (default), `loop` or `error` (HTTP 500), as in scripted mode. |

- Unknown fields are rejected, so a typo fails at startup rather than being
  ignored.
- Timing not set by the step or `defaults` falls back to the server's latency
  profile.
- `retry_after_ms` is sent as `Retry-After` (seconds, rounded up) and
  `retry-after-ms`. It lives on a `StepError` wrapper that flattens
  `SimError`, so the public `SimError` enum is unchanged.

### Selecting a scenario

```text
Summarise the open issues. [[llmsim:research-3-tools]]
Summarise the open issues. [[llmsim:research-3-tools seed=7 speed=0.5]]
```

- **Syntax:** `[[llmsim:<name> key=value ...]]`, anywhere in the text. The
  first marker in the message is used. Unknown keys and malformed values are
  ignored.
- **Parameters:** `seed` (integer) changes the random draws without editing
  the scenario. `speed` scales all timing (TTFT, token gaps, stalls, and the
  latency-profile fallback): `0.5` runs twice as fast. Both are optional.
  `Retry-After` is not scaled.
- **Scope:** a marker applies to the user message that carries it and every
  model call until the next user message. A later user message without a
  marker leaves the scenario; each user message can pick a different one.
- **Precedence:** a marker in the last user message wins, then a model id of
  the form `llmsim-scenario-<name>` (for channels where the driver cannot
  edit messages). Without either, today's behaviour applies (script, then
  generator).
- **Unknown name:** HTTP 400 whose message names the scenario and lists the
  known ones.
- **Marker text** stays in the message; llmsim does not strip it.

### Stateless step resolution

`resolve(messages, tools, model, set) -> Result<Option<Resolution>, ResolveError>`
is a pure function:

1. The triggering user message is the last message with role `user`. Tool
   results do not count.
2. Read its marker; with none, use the model id; with neither, return `None`.
3. Steps already played = assistant messages after that user message.
4. Cross-check tool call ids. Every call a scenario makes gets the id
   `call_llmsim_<scenario>_<step>_<n>_<nonce>`. If ids after the user message
   (on assistant tool calls or tool results) show a higher step for the same
   scenario, the ids win. That recovers history compaction that dropped an
   assistant message.
5. Play `steps[step]`, or apply `on_exhausted` past the end.
6. Seed the random draws (timing ranges, lorem text, placeholder picks) with
   a hash of scenario name, `seed`, the user message text, the number of user
   messages, and the step. Hashing is FNV-1a feeding SplitMix64, stable
   across processes and versions.

### Attempts and errors

A failed call adds no assistant message, so a retry resolves to the same
step. The attempt number comes from an in-process `AttemptTracker` keyed by
the step fingerprint (bounded to 50k entries, 10-minute TTL). Attempt `a`:

1. `a < len(fail_first)`: return `fail_first[a]`.
2. Error step: return its error.
3. `defaults.error_rate`: draw from fingerprint + attempt; may return an
   error.
4. Otherwise respond; when `a == len(fail_first)` and `cut_after_tokens` is
   set, drop the connection after that many tokens.

A retry that lands on a different worker or after a restart sees the errors
again: an extra error, never a wrong answer.

### Server mode

- `[response] scenarios_path = "..."` loads one JSON file (an object or an
  array of objects) or every `*.json` file in a directory. Invalid scenarios
  or duplicate names fail startup. `script_path` keeps working unchanged.
- Wired into `POST /openai/v1/chat/completions`, streaming and
  non-streaming:
  - Streaming: role chunk, `delta.reasoning_content` deltas, content deltas
    (word tokens with leading space), tool call deltas (announce + arguments
    chunk), finish chunk with usage, `[DONE]`. Paced by the resolved timing.
    A cut aborts the HTTP body after the given number of tokens.
  - Non-streaming: waits as long as streaming the whole answer would take
    (TTFT + gaps + stall), then returns the message with `tool_calls`. A cut
    sends headers and then aborts the body.
  - Usage counts words for text and reasoning; reasoning is reported in
    `completion_tokens_details.reasoning_tokens`.
  - Responses carry `x-llmsim-scenario`, `x-llmsim-step` and
    `x-llmsim-attempt` headers for debugging.
- Error injection from `[errors]` still applies first.
- Other endpoints (Responses, OpenResponses, Anthropic Messages) do not play
  scenarios yet; they keep their current behaviour.

### Starter library

`examples/scenarios/` ships `chat-short`, `research-3-tools`, `long-answer`,
`slow-first-token`, `parallel-tools`, `flaky-429`, `stream-stall`,
`broken-stream` and `thinking`, with timing from production observations
(TTFT 0.8-1.3 s, about 65 tokens/s). They use tool placeholders only, so
they work with any agent. A test keeps them loadable.

### Public Rust API

```rust
use llmsim::scenario::{resolve, AttemptTracker, AttemptOutcome, ConversationMessage,
                       MessageRole, ScenarioSet, ToolSpec};

let set = ScenarioSet::from_path("examples/scenarios")?;
let tracker = AttemptTracker::default();
let messages = [ConversationMessage::new(MessageRole::User, "hi [[llmsim:chat-short]]")];
if let Some(res) = resolve(&messages, &[], Some("gpt-5"), &set)? {
    match res.outcome(tracker.next_attempt(res.fingerprint)) {
        AttemptOutcome::Fail(err) => { /* return err */ }
        AttemptOutcome::Respond { cut_after_tokens } => { /* play res.turn with res.timing */ }
    }
}
```

`ConversationMessage` and `ToolSpec` convert from the OpenAI request types.
The pacing and SSE engine is `llmsim::scenario_stream`.

## Non-goals

- Content that makes sense; answers are lorem text or fixed strings.
- Token counts that match a real tokenizer.
- Replacing scripted mode.

## Future work

- Scenarios on the Responses, OpenResponses and Anthropic endpoints.
- Custom scenarios supplied per request or via an admin endpoint.
