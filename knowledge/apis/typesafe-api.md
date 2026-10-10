---
type: Specification
title: "TypeSafe System One API Specification"
description: "Wire-compatible simulation of the TypeSafe System One evaluation and models APIs with the Jev model."
tags:
  - llmsim
  - apis
---
# TypeSafe System One API Specification

## Abstract

This specification defines LLMSim's simulation of the [TypeSafe System One
API](https://docs.typesafe.ai/api), served by TypeSafe's Jev models. System One
does not generate text: it answers typed questions about a `state` (yes/no
"noul", single-selection "choice", graded "score") with calibrated
probabilities. The goal is wire-format compatibility, so the official
`typesafe-sdk` clients and integrations such as everruns' System One decision
driver work unchanged when pointed at `{base_url}/typesafe`, and verdict-driven
code paths (guardrails, routing, rating) can be tested without API cost.

The wire shapes follow TypeSafe's published OpenAPI schema
(`https://api.typesafe.ai/openapi.json`) and the `typesafe-sdk` Python client.

## Requirements

### R1: Evaluation Endpoint

**R1.1**: Implement `POST /typesafe/v1/systemone`. Non-streaming only; the real
API has no streaming mode.

**R1.2**: The request body:

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `state` | string \| object \| array | yes | The content every question refers to |
| `model` | string | yes | Model name or alias, e.g. `jev-latest` |
| `questions` | object | yes | Question name → question; at least one |

**R1.3**: Each question carries a `type` discriminator and optional
`instructions` (string, object, array, or null):

| `type` | `criteria` | Rules |
|--------|-----------|-------|
| `noul` | optional `{"true": ..., "false": ...}` | needs `instructions` or a non-null criteria value |
| `choice` | required object, option → description (description may be `null`) | 1–255 options |
| `score` | required array of level descriptions, lowest first | 1–10 levels; entries may not be `null` |

**R1.4**: The response body:

```json
{
  "model": "jev-1.13.0",
  "answers": {
    "is_urgent": {"type": "noul", "noul": 0.95},
    "team": {"type": "choice", "choice": "billing", "confidence": 0.62,
             "probabilities": {"billing": 0.81, "technical": 0.19}},
    "severity": {"type": "score", "score": 1.4, "confidence": 0.31,
                 "legend": {"0": "low", "1": "medium", "2": "high"},
                 "probabilities": {"0": 0.2, "1": 0.2, "2": 0.6}}
  },
  "usage": {"input_tokens": 88, "output_tokens": 60}
}
```

- `model` is the versioned ID that answered: aliases resolve to it (R3).
- Every question gets exactly one answer, under its own name, of its own type.
- `noul` is in 0..=1.
- `probabilities` cover every option (choice) or every level index (score),
  each in 0..=1, summing to 1.
- `choice` is the highest-probability option.
- `score` is the probability-weighted level, in `0..=levels-1`; `legend` maps
  each level index to the description it was given.
- `confidence` is in 0..=1, computed as one minus the normalized entropy of the
  distribution (1 for a certain answer, 0 for a uniform one).

**R1.5**: Answers are a deterministic function of the state, the question name,
the question type, and its instructions. Identical requests always get
identical answers; different questions spread across the probability range.

**R1.6**: `usage.input_tokens` counts the state, every question's instructions
and criteria, and a small framing overhead. `usage.output_tokens` is 20 per
question (the API reference example). The real API bills input tokens only.

**R1.7**: Latency is a single wait (no token stream). Without a configured
latency profile, `jev-*` models use the `jev` profile (~150 ms mean).

### R2: Errors

**R2.1**: Invalid requests return `422` with the FastAPI validation body,
reporting every problem found and its path:

```json
{"detail": [{"type": "too_short", "loc": ["body", "questions", "u", "score", "criteria"],
             "msg": "List should have at least 1 item after validation, not 0", "input": []}]}
```

Malformed JSON returns `422` with `type: "json_invalid"`.

**R2.2**: Other errors, including injected ones, use TypeSafe's envelope
`{"detail": {"error_type": "...", "message": "..."}}`, with `error_type`
derived from the status (`rate_limit_error`, `authentication_error`,
`overloaded_error`, `api_error`, ...). Injected `429`s carry `Retry-After`.

**R2.3**: The API key is not checked (see [API Endpoints](../foundations/api-endpoints.md)
Non-Requirements).

### R3: Models

**R3.1**: `GET /typesafe/v1/models` returns
`{"models": [{"name", "description", "release_date"}]}`, listing the aliases
`jev-latest` and `jev-preview` like the real API.

**R3.2**: `jev-latest`, `jev-preview`, and `jev-1.13` resolve to `jev-1.13.0`
(Jev 1.13, released 2026-09-17). Any other model name, including future
versioned `jev-*` IDs, is accepted and echoed unchanged in `model`.

### R4: Scope

**R4.1**: Scripted mode and scenarios produce chat turns and do not apply to
System One. Error injection and stats apply: requests count under
`systemone_requests` in `/llmsim/stats`.
