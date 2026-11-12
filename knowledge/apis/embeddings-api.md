---
type: Specification
title: "OpenAI Embeddings API Specification"
description: "Deterministic, similarity-preserving embedding vectors with OpenAI's request shapes, limits and errors."
tags:
  - llmsim
  - api
  - embeddings
---
# OpenAI Embeddings API Specification

## Abstract

Retrieval pipelines, semantic caches and dedup jobs call an embeddings API
far more often than a chat model, and they break in ways random vectors
cannot show: a ranking that is always noise hides bugs in the ranking code.
llmsim answers `POST /openai/v1/embeddings` with vectors whose cosine
similarity tracks word overlap, so related texts rank above unrelated ones,
while staying deterministic and free. Code:
[`src/embeddings.rs`](../../src/embeddings.rs) (vectors, request types) and
[`src/cli/embeddings_handlers.rs`](../../src/cli/embeddings_handlers.rs).

## Requirements

- R1: Request and response shapes match OpenAI: `input` as a string, strings,
  a token array or token arrays; `data[]` of `{object: "embedding", index,
  embedding}`; `usage.prompt_tokens` and `total_tokens`.
- R2: Vectors are unit length and a pure function of the text and dimension
  count. Each distinct word (case-folded) contributes a fixed pseudo-random
  direction weighted by the square root of its count; a small per-text
  component keeps texts with the same words apart.
- R3: Models: `text-embedding-3-small` (1536), `text-embedding-3-large`
  (3072), `text-embedding-ada-002` (1536). Unknown ids use the 3-small shape,
  or 404 `model_not_found` when `strict_models` is on.
- R4: `dimensions` on v3 models returns the renormalized prefix of the full
  vector; on ada-002, or outside 1..native, it is a 400.
- R5: `encoding_format` is `float` (default) or `base64` (little-endian
  float32).
- R6: Limits: 8192 tokens per input, 2048 inputs and 300,000 tokens per
  request, no empty inputs; errors use OpenAI's `invalid_request_error`
  wording.
- R7: Latency uses an embedding profile (about 120 ms, 2 µs per input token);
  a configured latency profile overrides it. Rate limits and error injection
  apply as on the other OpenAI endpoints.

## Non-goals

- Real semantics (synonyms, paraphrase). Only shared words bring texts close.
