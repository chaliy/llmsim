# LLMSim Knowledge Update Log

## 2026-11-12

* **Added the Realtime API.** New [OpenAI Realtime API Specification](apis/realtime-api.md):
  voice sessions over WebSocket with server VAD on the audio and barge-in.

* **Added the Gemini API.** New [Gemini API Specification](apis/gemini-api.md): native
  `generateContent` with thinking, tools, JSON, streaming, caching and embeddings.

* **Added batch and background APIs.** New [Batch and Background APIs](apis/batch-and-background.md):
  background Responses, OpenAI Files and Batches, Anthropic Message Batches.

* **Added the Embeddings API.** New [OpenAI Embeddings API Specification](apis/embeddings-api.md):
  `/openai/v1/embeddings` with similarity-preserving vectors.

* **Scenarios on the Responses API and Anthropic Messages.** [Scenarios](simulation/scenarios.md)
  now play on both, in each API's wire shape.

* **Added prompt caching and thinking realism.** New
  [Prompt Caching and Thinking Realism](simulation/prompt-caching-and-thinking.md) concept:
  cached-token usage for OpenAI and Anthropic, effort-scaled reasoning latency, redacted thinking.

* **Added structured outputs and forced tool calls.** New
  [Structured Outputs and Forced Tool Calls](simulation/structured-outputs.md) concept;
  Anthropic streams now carry `tool_use` blocks with `input_json_delta`.

* **Added rate limits, overload bursts and request limits.** New
  [Rate Limits, Overload Bursts and Request Limits](simulation/limits.md) concept.

* **Added Decisions APIs.** New [Decisions APIs Specification](apis/decisions-api.md):
  OpenAI `/v1/decisions`, Gemini `/v1beta/decisions` and Microsoft Foundry System One,
  all on the System One answer engine. Jev aliases now resolve to Jev 1.14.

* **Added the Latency Model.** New [Latency Model](simulation/latency.md) spec: measured
  GPT-6 tier profiles (Luna, Sol) and Muse Spark, log-normal TTFT, prompt-size prefill.
* **Built-in scenarios.** [Scenarios](simulation/scenarios.md) are compiled in and on by
  default; six agent scenarios from everruns joined the starter library.

## 2026-11-10

* **Added Python and Node bindings.** New [Python and Node Bindings](foundations/language-bindings.md)
  spec: PyPI and npm packages named `llmsim` embed the server in-process. The repo became a
  Cargo workspace; [Release Process](project/release-process.md) now publishes PyPI and npm.

## 2026-10-10

* **Added TypeSafe System One.** New [TypeSafe System One API Specification](apis/typesafe-api.md)
  for `/typesafe/v1/systemone` and `/typesafe/v1/models` (Jev model).
* **Adopted OKF v0.2.** The former `specs/` folder moved into this bundle, grouped into
  domains, with OKF frontmatter on every concept. `scripts/check_okf.py` and the
  `Knowledge Bundle (OKF v0.2)` CI job keep it conformant. See
  [Knowledge Maintenance Contract](knowledge-contract.md).
