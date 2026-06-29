# OpenAI Image Generation API Specification

## Abstract

This specification defines the simulated OpenAI image generation endpoint for
LLMSim, replicating the traffic shape of the gpt-image family (the "ChatGPT
Images" capability) without running a real diffusion model. The endpoint
produces a synthetic but valid PNG of the exact requested size that renders the
request prompt and a clear "LLMSIM SIMULATED IMAGE" watermark, so generated
bytes are unambiguously identifiable as simulated. Both the non-streaming JSON
response and the SSE streaming flow (with progressive partial images) are
supported, along with realistic, quality- and size-scaled latency.

## Requirements

### R1: Core Endpoint

**R1.1**: Implement `POST /openai/v1/images/generations` accepting the OpenAI
image generation request body and returning a simulated image response.

**R1.2**: The endpoint MUST be compatible with the official OpenAI SDKs when
configured with `{base_url}/openai/v1` as the base URL (e.g.
`client.images.generate(...)`).

### R2: Request Parameters

**R2.1**: Accept the following request fields:

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `prompt` | string | (required) | Text description of the image |
| `model` | string | `gpt-image-1` | Image model id |
| `n` | integer | `1` | Number of images, clamped to 1–10 |
| `size` | string | `1024x1024` | `WxH`, or `auto` (resolved to default) |
| `quality` | string | `high` | `low` \| `medium` \| `high` \| `auto` |
| `output_format` | string | `png` | `png` \| `jpeg` \| `webp` (echoed back) |
| `background` | string | `opaque` | `transparent` \| `opaque` \| `auto` |
| `response_format` | string | — | Accepted for DALL·E compatibility |
| `stream` | boolean | `false` | Stream partial images via SSE |
| `partial_images` | integer | `0` | Progressive previews, clamped to 0–3 |
| `moderation` | string | — | Accepted and ignored |
| `user` | string | — | Accepted and ignored |

**R2.2**: Supported sizes mirror the gpt-image models: `1024x1024`,
`1536x1024`, `1024x1536`. Arbitrary `WxH` values are accepted and clamped to
the 16–4096 px range per axis; unparseable sizes fall back to `1024x1024`.

### R3: Non-Streaming Response

**R3.1**: Return a JSON object with:
- `created`: Unix timestamp
- `data`: array of `n` objects, each with `b64_json` (base64 PNG)
- `usage`: token usage (see R5)
- `size`, `quality`, `output_format`, `background`: the resolved parameters

**R3.2**: Each `data[i].b64_json` MUST decode to a valid PNG of the requested
size.

### R4: Streaming Response

**R4.1**: When `stream` is `true`, respond with `text/event-stream` emitting:
- `partial_images` events of type `image_generation.partial_image`, each with a
  `partial_image_index` (0-based) and a `b64_json` preview frame
- a final `image_generation.completed` event with the full image and `usage`

**R4.2**: Partial frames MUST be progressively refined: earlier partials are
coarser (pixelated previews), converging to a crisp final frame.

**R4.3**: Each streamed event payload includes `type`, `b64_json`, `created_at`,
`size`, `quality`, `background`, and `output_format`. Partial events also
include `partial_image_index`; the completed event also includes `usage`.

**R4.4**: Events are framed as SSE: `event: <type>\ndata: <json>\n\n`.

### R5: Usage Accounting

**R5.1**: `usage` reports:
- `input_tokens`: token count of the prompt text
- `output_tokens`: estimated image tokens, scaled by size and quality
- `total_tokens`: sum of the above
- `input_tokens_details`: `{ text_tokens, image_tokens }` (image_tokens is 0 for
  text-only prompts)

**R5.2**: Output image token baselines are anchored to gpt-image-1 at
1024x1024 (low ≈ 272, medium ≈ 1056, high ≈ 4160) and scale linearly with pixel
area.

### R6: Latency

**R6.1**: Generation time is simulated and scales with quality and image area.
It is anchored to the configured latency profile: model-derived latency (the
default) yields realistic multi-second waits, while the `instant` and `fast`
profiles collapse the wait so tests and load runs stay fast.

**R6.2**: For streaming, partial and completed frames are spaced evenly across
the total generation time.

### R7: Models

**R7.1**: Register the gpt-image model family in the model registry and default
model list so they appear in `GET /openai/v1/models`:
`gpt-image-1`, `gpt-image-1-mini`, `gpt-image-1.5`.

**R7.2**: Image models advertise `vision` capability (they accept image inputs)
and do not advertise text tool-calling, JSON mode, or reasoning.

### R8: Error Injection

**R8.1**: The endpoint participates in the shared error-injection model
(rate-limit and server errors) like the other OpenAI endpoints.

### R9: Statistics

**R9.1**: Image requests are counted under a dedicated `image_requests` stat and
contribute to total/streaming/non-streaming counters, token totals, and latency
tracking exposed by `GET /llmsim/stats`.

## Non-Requirements

- Real image synthesis or any resemblance between the prompt and the rendered
  content beyond the literal prompt text drawn onto the placeholder.
- Image edits (`/v1/images/edits`) and variations (`/v1/images/variations`).
  These may be added later following the same pattern.
- The Responses API `image_generation` tool. The dedicated images endpoint is
  the primary, SDK-compatible surface; tool-based generation is a future
  extension.
- Honoring `output_format`/`background` in the actual encoded bytes (the
  simulator always emits an indexed PNG); these parameters are echoed back for
  client compatibility.

## Rationale

### Why a self-contained PNG encoder?

LLMSim avoids non-permissive and heavyweight dependencies. The endpoint ships a
tiny dependency-free indexed-PNG encoder and 5x7 bitmap font, producing a valid,
readable, watermarked image. Uncompressed ("stored") DEFLATE blocks keep the
encoder trivial; combined with an 8-bit palette this yields realistic payload
sizes (~1 MB for 1024x1024) without a compression crate.

### Why progressive partial images?

Real gpt-image streaming returns increasingly refined previews. Simulating this
with decreasing pixelation lets clients exercise the same partial-image handling
code paths they would use against the real API.
