# Scenarios

Scenarios replay realistic agent conversations for load tests: tool calls,
long answers, reasoning, slow first tokens, mid-stream stalls, a 429 halfway,
or a dropped stream. Many sessions can share one llmsim server, because the
step to play is worked out from each request's own message history.

Scenarios currently play on `POST /openai/v1/chat/completions` (streaming and
non-streaming).

## Enable

Point the server at a scenario file or a directory of them:

```toml
# config.toml
[response]
scenarios_path = "examples/scenarios"
```

```bash
llmsim serve --config config.toml
```

The repository ships a starter library in `examples/scenarios/`:

| Scenario | Steps | Exercises |
|----------|-------|-----------|
| `chat-short` | 1 answer, 40-80 tokens, TTFT 0.6-1.0 s | Plain chat |
| `research-3-tools` | 3 tool rounds, then a 600-token answer | Tool phases holding a turn open |
| `long-answer` | 1 answer, 1500 tokens at 50 tokens/s | Long streams, SSE throughput |
| `slow-first-token` | 1 answer, TTFT 4-8 s | Idle connections |
| `parallel-tools` | 1 step with 4 tool calls, then an answer | Parallel tool execution |
| `flaky-429` | 429 with Retry-After 1 s, then `research-3-tools` | Retry and backoff |
| `stream-stall` | 300 tokens with a 20 s stall after 100 | Keep-alives, proxy idle timeouts |
| `broken-stream` | stream cut after 50 tokens, full answer on retry | Mid-stream failure recovery |
| `thinking` | 400 reasoning tokens, then a 300-token answer | Reasoning deltas |

## Select a scenario

Append a marker to the user message:

```text
Summarise the open issues. [[llmsim:research-3-tools]]
Summarise the open issues. [[llmsim:research-3-tools seed=7 speed=0.5]]
```

- `seed` changes the random draws (timing ranges, lorem text).
- `speed` scales all timing; `0.5` runs twice as fast.
- The marker applies until the next user message. A user message without a
  marker gets llmsim's normal behaviour.
- If you cannot edit messages, use the model id `llmsim-scenario-<name>`.
- An unknown name returns HTTP 400 listing the known scenarios.

Keep a session index in the message text (for example `session 17`) so each
session draws its own timing while reruns stay reproducible.

## Write a scenario

```json
{
  "name": "my-scenario",
  "defaults": { "timing": { "ttft_ms": [800, 1300], "tokens_per_sec": 65 } },
  "steps": [
    { "turn": { "type": "mixed", "text": "Let me check.",
                "calls": [{ "tool": "any_read" }] } },
    { "fail_first": [{ "kind": "rate_limit", "retry_after_ms": 1000 }],
      "turn": { "type": "tool_calls", "calls": [{ "name": "bash", "arguments": { "command": "ls" } }] } },
    { "timing": { "stall": { "after_tokens": 100, "ms": 5000 } },
      "turn": { "type": "assistant", "lorem_tokens": 300, "reasoning_tokens": 100 } }
  ],
  "on_exhausted": "repeat_last"
}
```

- **Turns:** `assistant`, `tool_calls`, `mixed` (text + calls) or `error`.
  Text is either `text` or `lorem_tokens` (a number or `[min, max]`).
  `reasoning_tokens` streams as `delta.reasoning_content` before the answer.
- **Tool calls:** `name` must be a tool the request offers (else HTTP 400).
  `tool` is a placeholder (`any_read`, `any_write`, `any`) that picks an
  offered tool. Without `arguments`, required parameters are filled from the
  tool's JSON schema.
- **Timing:** `ttft_ms`, `tokens_per_sec`, `stall {after_tokens, ms}`,
  `cut_after_tokens` (drop the connection; the retry then succeeds). Unset
  values fall back to `defaults.timing`, then to the latency profile.
- **Errors:** `fail_first` lists errors for the first attempts of a step.
  `retry_after_ms` sets `Retry-After` and `retry-after-ms`.
  `defaults.error_rate` (for example `{"rate_limit": 0.02}`) adds background
  errors.
- Unknown fields and invalid values fail at startup.

Responses carry `x-llmsim-scenario`, `x-llmsim-step` and `x-llmsim-attempt`
headers to help debug a run.

Attempt counts for `fail_first` and cuts are kept per server process, so a
retry that lands on a different llmsim instance may see the error again.

The full specification is in `knowledge/simulation/scenarios.md`.
