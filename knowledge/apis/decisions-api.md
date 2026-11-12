---
type: Specification
title: "Decisions APIs Specification"
description: "OpenAI Decisions, Gemini Decisions and Microsoft Foundry System One: typed judgments simulated on the System One engine."
tags:
  - llmsim
  - apis
  - decisions
---
# Decisions APIs Specification

## Abstract

Several providers now sell typed judgments: a question about some input is
answered with a calibrated probability, a choice distribution, or a score,
instead of prose. LLMSim simulates the ones everruns' decision drivers call,
so guardrails, routing and rating code can be tested against each wire shape
without API cost. All of them reuse the [TypeSafe System One](typesafe-api.md)
answer synthesis: the same input and question get the same answer, whichever
envelope asked. Handlers live in
[`src/cli/decisions_handlers.rs`](../../src/cli/decisions_handlers.rs).

## Requirements

### R1: OpenAI Decisions

**R1.1**: `POST /openai/v1/decisions` (OpenAI's Decisions API, public beta).
Body: optional `model` (default `gpt-6-luna`), `input` text, and `questions`,
an array of `predicate`, `choice` (`choices[].value`, optional `description`)
and `score` (`levels[].label`, optional `description`) questions, each with a
`name` and `instructions`.

**R1.2**: The response carries `model`, `answers` in question order (each
with its `name` and `type`: `probability` for predicates; `choice`,
`confidence` and `probabilities[{value, probability}]` for choices; `score`,
`confidence` and `probabilities[{value: level index, probability}]` for
scores) and `usage {input_tokens, output_tokens}`.

**R1.3**: Limits the live API enforces: at most 24 questions, unique names,
at least two choices, two to ten score levels. Violations return 400 with
OpenAI's `{"error": {"message", "type", "param", "code"}}` envelope.

**R1.4**: `[[llmsim:refuse]]` in `input` makes every answer
`{"type": "refusal", "name"}` with HTTP 200, the way the real API declines.

### R2: Gemini Decisions

**R2.1**: `POST /gemini/v1beta/decisions` (Gemini Decision 1). Body: optional
`model` (default `gemini-decision-1`, `models/` prefix accepted), `input` (a
string or Content parts of type `text`, `image`, `audio`, `video`,
`document`), and `questions`, an object keyed by `[A-Za-z0-9_-]{1,64}`.

**R2.2**: Questions are System One's primitives with Google's criteria:
`noul` takes `true_description`/`false_description`, `choice` maps options to
descriptions, `score` lists 2 to 255 levels.

**R2.3**: The response has `answers` keyed like the request (System One answer
fields; a noul also carries `confidence`) and `usage` with
`total_input_tokens` and `total_output_tokens` (always 0). It names no model.
Errors use Google's `{"error": {"code", "message", "status"}}` envelope.

### R3: Microsoft Foundry

**R3.1**: `POST /foundry/providers/microsoft/v1/systemone` serves the System
One wire unchanged, at the path Foundry uses under a resource root, so a
client's base URL is `{llmsim}/foundry`. The model (a deployment name such as
`Microsoft-Decision-1`) is echoed.

### R4: Shared behaviour

**R4.1**: Error injection and the latency profile apply as for System One:
one first-token wait, plus prefill for the input tokens.

## Non-goals

- OpenRouter's System One route.
- Reading media content; media parts only count as placeholders.
