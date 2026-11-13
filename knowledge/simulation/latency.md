---
type: Specification
title: "Latency Model"
description: "How llmsim times a response: per-model profiles, a log-normal first-token tail, and prefill that grows with the prompt."
tags:
  - llmsim
  - simulation
  - latency
---
# Latency Model

## Abstract

Every simulated response waits a time to first token (TTFT), then streams with
a gap between tokens (TBT). The goal is timing that looks like production to a
client under test: per-model speeds, occasional slow starts, and long prompts
that start later than short ones. Profiles live in
[`src/latency.rs`](../../src/latency.rs); the public table is in the
[README](../../README.md#latency-profiles).

## Requirements

### R1: Profile selection

**R1.1**: A request uses `[latency] profile` or custom `ttft_*`/`tbt_*`
values when configured; otherwise the profile matching its model id
(`LatencyProfile::from_model`).

**R1.2**: GPT-5.6+ and GPT-6 tiers have their own profiles, matched by the
tier word in the id: `luna`, `terra`, `astra`, and `sol` (bare `gpt-5.6`,
`gpt-6.x` ids are Sol aliases). `muse-spark-*` has its own profile.

**R1.3**: Tier numbers come from measured production agent turns (everruns,
2026-10-28; TTFT includes default reasoning): Luna 2.9 s and 108 tokens/s,
Sol 2.0 s and 46 tokens/s, Muse Spark 12.7 s and 136 tokens/s. Terra and Astra
were not measured and are placed by tier. Replace estimates with measurements
when they exist, and record the date here.

### R2: Distribution

**R2.1**: TTFT is drawn from a log-normal distribution with the profile's mean
and stddev, so the median sits below the mean and the slowest 1% take well over
1.5x the mean. TBT stays normal.

**R2.2**: `instant` (all zeros) never sleeps.

### R3: Prefill

**R3.1**: Each profile carries `prefill_us_per_token` (default 50 us, about
20k tokens/s). Handlers add `input_tokens x prefill` to the mean TTFT once the
prompt is counted. `instant` and `fast` have no prefill.

**R3.2**: `[latency] prefill_us_per_token` overrides the rate for every
model; `0` turns prefill off.

## Non-goals

- Queueing delay that grows with server load.
- Per-provider time-of-day variation.
