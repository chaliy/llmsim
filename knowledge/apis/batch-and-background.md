---
type: Specification
title: "Batch and Background APIs"
description: "Background Responses, OpenAI Files and Batches, and Anthropic Message Batches: asynchronous work whose status advances with time."
tags:
  - llmsim
  - api
  - batch
---
# Batch and Background APIs

## Abstract

Long agent runs and offline jobs use the providers' asynchronous APIs:
background Responses that clients poll, and batch APIs that take thousands of
requests and return a results file later. Code that polls, backs off, cancels
and parses results files needs a server that moves through the same states.
Code: [`src/cli/jobs.rs`](../../src/cli/jobs.rs).

## Requirements

- R1: Work runs at creation through an internal router with instant timing,
  no injected errors and no rate limits, so each request returns what the
  synchronous endpoint would (scenarios and forced tools included). Only the
  reported status waits, as a function of time since creation, so polling
  sees realistic transitions without a background worker.
- R2: Background responses (`background: true`): create returns `queued`;
  retrieval shows `in_progress` until the model would have finished (latency
  profile, prefill and output tokens), then the stored response with its id.
  Cancel before then gives `cancelled`; cancelling a completed response is a
  400; `DELETE` removes it; `store: false` is rejected. Background with
  `stream: true` streams as usual.
- R3: Files: multipart upload with `file` and `purpose`; metadata and content
  retrieval; delete.
- R4: OpenAI batches: endpoints `/v1/chat/completions`, `/v1/responses`,
  `/v1/embeddings`, window `24h`. Every line is validated first (JSON,
  `custom_id`, `url` equal to the endpoint); any invalid line fails the batch
  with `errors`. Status goes `validating`, `in_progress`, `completed` after
  `[batch] completion_ms`; 200 results go to `output_file_id`, others to
  `error_file_id`, in OpenAI's line shape. Cancel before completion gives
  `cancelled`.
- R5: Anthropic message batches: `custom_id` must match
  `^[a-zA-Z0-9_-]{1,64}$` and be unique. `processing_status` is `in_progress`,
  then `ended` with `request_counts`; `results_url` is absolute (built from the
  request's `Host`) because the SDK fetches it directly. Results are JSONL of
  `succeeded` / `errored` (the Messages error body) / `canceled`. Results and
  delete before the batch ends are 400s.
- R6: State is in memory, capped at 10,000 entries per kind (oldest dropped),
  and lost on restart.

## Non-goals

- Batch pricing, expiry, or results that outlive the process.
