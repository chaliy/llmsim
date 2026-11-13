---
type: Specification
title: "Prompt Caching and Thinking Realism"
description: "Cached-token usage the way OpenAI and Anthropic report it, effort-scaled reasoning latency, and redacted thinking."
tags:
  - llmsim
  - simulation
  - caching
  - reasoning
---
# Prompt Caching and Thinking Realism

## Abstract

Agent loops resend a growing conversation every turn, and real providers bill
and serve the repeated prefix from a cache. Clients that track cost or show
cache hit rates read the cached-token fields, so a simulator that always
reports zero hides bugs in that accounting. llmsim keeps a short-lived set of
prompt prefixes and reports cache hits in each provider's shape. Reasoning
models also get slower with higher effort, and Anthropic occasionally returns
`redacted_thinking` blocks that clients must pass back untouched. Code:
[`src/cli/prompt_cache.rs`](../../src/cli/prompt_cache.rs),
[`src/latency.rs`](../../src/latency.rs) (`with_reasoning_effort`).

## Requirements

### Prompt caching

- R1: Prompt caching is on by default (`[response] prompt_caching = true`) and
  is per server, keyed by model.
- R2: Prefix boundaries are message, system block and tool boundaries. A hit is
  the longest previously seen prefix, so appending messages to a conversation
  keeps hitting.
- R3: OpenAI (chat and Responses) caches automatically for prompts of at least
  1024 tokens; the cached count is rounded down to 1024 + n·128. It is reported
  in `usage.prompt_tokens_details.cached_tokens` (chat) and
  `usage.input_tokens_details.cached_tokens` (Responses).
- R4: Anthropic caches only up to `cache_control` breakpoints (blocks on
  tools, system or messages, or the last block when the request has top-level
  `cache_control`), each at least 1024 tokens. A seen breakpoint is
  `cache_read_input_tokens`; the rest up to the last breakpoint is
  `cache_creation_input_tokens`; `input_tokens` is what is left. Streams carry
  the same fields in `message_start`.
- R5: Entries live 5 minutes and are refreshed on use.
- R6: Cached tokens skip prompt prefill latency, so a hit starts sooner.

### Thinking realism

- R7: For reasoning models, `reasoning_effort` (chat) and `reasoning.effort`
  (Responses) scale time to first token, from 0.35× for `none` to 4.5× for
  `max`; `medium` is 1×.
- R8: Hidden Anthropic thinking still takes time: the stream waits about one
  token interval per thinking token before the text starts.
- R9: `[[llmsim:redacted-thinking]]` in the last user message turns the
  thinking block into `redacted_thinking` with opaque `data`, in both
  non-streaming and streaming responses.
