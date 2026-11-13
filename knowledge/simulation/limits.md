---
type: Specification
title: "Rate Limits, Overload Bursts and Request Limits"
description: "Per-key RPM/TPM buckets with provider headers, clustered overload failures, and context/output/model validation."
tags:
  - llmsim
  - simulation
  - errors
---
# Rate Limits, Overload Bursts and Request Limits

## Abstract

Random per-request error rates (`[errors]`) test retries, but real failures
have structure: a key runs out of its per-minute budget and the provider says
when it refills, incidents fail every call for a while, and oversized
requests are rejected up front. This concept defines those behaviours. Code:
[`src/cli/limits.rs`](../../src/cli/limits.rs) (router middleware) and
[`src/cli/validation.rs`](../../src/cli/validation.rs); user docs in
[docs/api.md](../../docs/api.md#rate-limits).

## Requirements

### R1: Rate limits

**R1.1**: `[rate_limits] requests_per_minute` and `tokens_per_minute` (0 =
off) are token buckets per API key, refilled continuously over a minute.

**R1.2**: Token cost is charged on arrival: body bytes / 4 plus the
requested output cap (1000 when absent). Estimates are enough; the handler
still counts real usage.

**R1.3**: With a limit set, every generation response carries the
provider's headers (OpenAI `x-ratelimit-*`, Anthropic
`anthropic-ratelimit-*`). An exhausted key gets 429 in the provider's
envelope with `retry-after` and `retry-after-ms`. Model lists, token
counting, health and stats are never limited.

### R2: Overload bursts

**R2.1**: `[errors] overload_burst_rate` starts a burst with that chance per
request; for `overload_burst_ms` every generation request fails (529
`overloaded_error` on Anthropic and TypeSafe shapes, 503 elsewhere) with
`retry-after` set to the time left.

### R3: Request limits

**R3.1**: With `[validation] enforce_limits` (default on), chat, Responses
and Messages requests whose prompt (plus output cap) exceeds the model's
context window, or whose output cap exceeds its max output tokens, fail with
the provider's 400. Models without a profile are never checked.

**R3.2**: `[validation] strict_models` (default off) rejects model ids
without a profile with the provider's 404. Scenario model ids always pass.

## Non-goals

- Organization-level limits shared across keys, or limit tiers.
- Charging actual (rather than estimated) usage back to the bucket.
