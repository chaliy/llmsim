---
type: Specification
title: "Gemini API Specification"
description: "Google Gemini v1beta generateContent, streaming, token counting, embeddings and models, with Gemini's thinking, tool and caching behavior."
tags:
  - llmsim
  - api
  - gemini
---
# Gemini API Specification

## Abstract

Agents that call Gemini through the Google Gen AI SDK need Gemini's own wire
format: `contents` and `parts`, `usageMetadata`, `thoughtsTokenCount`,
`functionCall` parts and Google's error envelope. llmsim serves it under
`/gemini/v1beta/models/{model}:{method}`. Code:
[`src/cli/gemini_handlers.rs`](../../src/cli/gemini_handlers.rs), with
scenarios in [`src/cli/scenario_handlers.rs`](../../src/cli/scenario_handlers.rs).

## Requirements

- R1: Requests are read as JSON accepting camelCase and snake_case names.
  `contents` is required; roles are `user` and `model`. Text parts are
  counted with the tokenizer; each inline or file part costs 258 tokens.
- R2: Responses carry one candidate (`content.role: model`, `parts`,
  `finishReason`, `index`), `usageMetadata` (`promptTokenCount`,
  `candidatesTokenCount`, `thoughtsTokenCount` when thinking,
  `cachedContentTokenCount` when cached, `totalTokenCount` = prompt +
  candidates + thoughts), `modelVersion` and `responseId`.
- R3: Thinking is on by default for 2.5 (except Flash-Lite), 3.x and 4.x
  models. `thinkingBudget: 0` turns it off except on Pro (400); a positive
  budget caps it; `thinkingLevel` scales it; `includeThoughts` returns a
  `thought: true` summary part. Thinking counts against `maxOutputTokens`;
  when it uses the whole cap the candidate has no parts and `MAX_TOKENS`.
- R4: `functionCallingConfig.mode: ANY` forces a `functionCall` with
  schema-valid args (narrowed by `allowedFunctionNames`); `AUTO` calls only
  with `auto_tool_calls`; `NONE` never calls. Thinking models attach a
  `thoughtSignature`. `responseMimeType: application/json` returns JSON valid
  against `responseSchema` (upper-case types are lower-cased) or
  `responseJsonSchema`.
- R5: `streamGenerateContent` sends about 12 words per chunk: SSE frames with
  `alt=sse`, otherwise one JSON array. The last chunk has `finishReason` and
  `usageMetadata`.
- R6: Implicit caching reports `cachedContentTokenCount` for a repeated
  prefix of at least 1024 tokens (4096 on Pro); cached tokens skip prefill.
- R7: Limits from the model profile: `maxOutputTokens` above the model's cap
  and prompts above its context window are 400s with Google's wording;
  unknown models are 404 only with `strict_models`.
- R8: Scenarios play with the same rules as the other APIs; `functionCall`
  parts carry `call_llmsim_...` ids and `functionResponse.id` links results.
- R9: `embedContent` / `batchEmbedContents` reuse the embeddings engine:
  `gemini-embedding-001` (3072; `outputDimensionality` 128-3072, prefix not
  renormalized) and `text-embedding-004` (768).
- R10: Errors use `{"error": {"code", "message", "status"}}`; rate limits and
  error injection apply to generation calls.

## Non-goals

- The Vertex AI path layout, cached-content resources and file uploads.
