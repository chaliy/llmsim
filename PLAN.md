# Implementation Plan

## Phase 1: Project Foundation

### 1.1 Project Setup
- [x] Initialize single Cargo crate with library + binary:
  - Library: `llmsim` (src/lib.rs)
  - Binary: `llmsim` with `serve` subcommand (src/main.rs)
- [x] Configure `Cargo.toml` with metadata (name, version, license = "MIT", authors)
- [x] Add initial dependencies:
  - `tokio` (async runtime)
  - `axum` (HTTP framework)
  - `serde` / `serde_json` (serialization)
  - `tiktoken-rs` (token counting)
  - `rand` (latency randomization)
  - `tracing` (logging)
  - `clap` (CLI argument parsing)
- [x] Create basic CI workflow (`.github/workflows/ci.yml`): format, lint, test

### 1.2 Core Types
- [x] Define OpenAI API types in `src/openai/types.rs`:
  - `ChatCompletionRequest`
  - `ChatCompletionResponse`
  - `ChatCompletionChunk` (for streaming)
  - `Message`, `Role`, `Usage`
  - `ToolCall`, `Function`
- [x] Add serde derive macros with proper field naming (`#[serde(rename_all = "snake_case")]`)
- [x] Write unit tests for serialization/deserialization against real API examples

---

## Phase 2: Core Library

### 2.1 Token Counter
- [x] Create `src/tokens.rs`
- [x] Implement `count_tokens(text: &str, model: &str) -> usize`
- [x] Support model-to-encoding mapping (gpt-4, gpt-5, claude, etc.)
- [x] Add fallback for unknown models
- [x] Write tests with known token counts

### 2.2 Latency Profiles
- [x] Create `src/latency.rs`
- [x] Define `LatencyProfile` struct:
  ```rust
  pub struct LatencyProfile {
      pub ttft_mean_ms: u64,      // Time to first token
      pub ttft_stddev_ms: u64,
      pub tbt_mean_ms: u64,       // Time between tokens
      pub tbt_stddev_ms: u64,
  }
  ```
- [x] Implement preset profiles:
  - `LatencyProfile::gpt5()` - flagship model
  - `LatencyProfile::gpt5_mini()` - faster
  - `LatencyProfile::o_series()` - reasoning models (o3, o4)
  - `LatencyProfile::gpt4()` - GPT-4 family
  - `LatencyProfile::claude_opus()` - Anthropic flagship
  - `LatencyProfile::claude_sonnet()` - balanced
  - `LatencyProfile::instant()` - no delay (for fast tests)
- [x] Implement `LatencyProfile::sample_ttft()` and `sample_tbt()` using normal distribution
- [x] Write tests for distribution sanity

### 2.3 Response Generator
- [x] Create `src/generator.rs`
- [x] Implement `ResponseGenerator` trait:
  ```rust
  pub trait ResponseGenerator: Send + Sync {
      fn generate(&self, request: &ChatCompletionRequest) -> String;
  }
  ```
- [x] Implement `LoremGenerator` - generates lorem ipsum text
- [x] Implement `EchoGenerator` - echoes back the user message
- [x] Implement `FixedGenerator` - returns configured fixed response
- [x] Implement `RandomWordGenerator` - random words to target token count
- [x] Add configurable response length (target token count)

### 2.4 Streaming Engine
- [x] Create `src/stream.rs`
- [x] Implement `TokenStream` that yields `ChatCompletionChunk` with delays
- [x] Support SSE format (`data: {...}\n\n`)
- [x] Handle `[DONE]` termination message
- [x] Integrate with latency profiles for inter-token delays
- [x] Write integration tests

### 2.5 Error Injection
- [x] Create `src/errors.rs`
- [x] Define `ErrorConfig`:
  ```rust
  pub struct ErrorConfig {
      pub rate_limit_rate: f64,     // 0.0-1.0
      pub server_error_rate: f64,
      pub timeout_rate: f64,
      pub timeout_after_ms: u64,
  }
  ```
- [x] Implement error decision logic
- [x] Create proper OpenAI-format error responses
- [x] Write tests for error rate distribution

### 2.6 Rate Limiter (Optional for Phase 2)
- [ ] Create `src/rate_limit.rs`
- [ ] Implement token bucket algorithm
- [ ] Support requests-per-minute and tokens-per-minute limits
- [ ] Return proper 429 responses with `Retry-After` header

---

## Phase 3: Server CLI (`llmsim serve`)

### 3.1 Basic Server Setup
- [x] Create `src/main.rs` with clap subcommand structure
- [x] Implement `llmsim serve` subcommand with CLI options:
  - `--port` (default: 8080)
  - `--host` (default: 0.0.0.0)
  - `--config` (optional config file path)
  - `--generator` (lorem, echo, random, fixed:text)
  - `--target-tokens` (default: 100)
  - Note: Latency is auto-derived from model in each request
- [x] Create `src/cli/` module for server functionality
- [x] Set up Axum router with graceful shutdown
- [x] Add health check endpoint (`GET /health`)
- [x] Add tracing/logging setup

### 3.2 OpenAI Chat Completions Endpoint
- [x] Implement `POST /v1/chat/completions`
- [x] Parse `ChatCompletionRequest`
- [x] Handle `stream: true` vs `stream: false`
- [x] Return proper `ChatCompletionResponse` with usage
- [x] Implement SSE streaming response
- [x] Add request validation
- [ ] Write integration tests with reqwest

### 3.3 OpenAI Models Endpoint
- [x] Implement `GET /v1/models`
- [x] Return list of "available" models with metadata (GPT-5, o-series, Claude, etc.)
- [x] Implement `GET /v1/models/{model_id}`

### 3.4 Configuration
- [x] Create `src/cli/config.rs`
- [x] Support YAML config file:
  ```yaml
  server:
    port: 8080
    host: "0.0.0.0"

  latency:
    profile: "gpt5"  # or custom values

  response:
    generator: "lorem"
    target_tokens: 100

  errors:
    rate_limit_rate: 0.01
    server_error_rate: 0.001
  ```
- [x] CLI arguments override config file values
- [x] Validate configuration on startup

### 3.5 Docker Support
- [x] Create `Dockerfile` (multi-stage build)
- [ ] Create `docker-compose.yml` for easy local testing
- [ ] Document Docker usage in README

---

## Phase 4: Tool Calling Support

### 4.1 Function/Tool Definitions
- [ ] Extend types with `Tool`, `ToolChoice`, `FunctionCall`
- [ ] Parse tool definitions from request
- [ ] Validate tool call format

### 4.2 Tool Call Response Generation
- [ ] Implement `ToolCallGenerator`:
  - Random tool selection from available tools
  - Generate plausible arguments based on parameter schema
- [ ] Support `tool_choice: "auto"`, `"none"`, `{"type": "function", "function": {"name": "..."}}`
- [ ] Return proper `tool_calls` array in response

### 4.3 Multi-turn Tool Conversations
- [ ] Handle `role: "tool"` messages in conversation
- [ ] Track tool call IDs
- [ ] Generate appropriate follow-up responses

---

## Phase 5: Additional API Support

### 5.1 Anthropic Messages API
- [ ] Create `llmsim/src/anthropic/types.rs`
- [ ] Implement Anthropic message format
- [ ] Add `/v1/messages` endpoint
- [ ] Support Anthropic streaming format (different from OpenAI)
- [ ] Handle Anthropic-specific headers (`x-api-key`, `anthropic-version`)

### 5.2 OpenAI Responses API (Assistants)
- [ ] Implement `/v1/threads` endpoints
- [ ] Implement `/v1/threads/{thread_id}/messages`
- [ ] Implement `/v1/threads/{thread_id}/runs`
- [ ] Support run streaming

### 5.3 Google Gemini API
- [ ] Create `llmsim/src/gemini/types.rs`
- [ ] Implement Gemini message format
- [ ] Add `/v1beta/models/{model}:generateContent` endpoint
- [ ] Add `/v1beta/models/{model}:streamGenerateContent` endpoint

---

## Phase 6: Advanced Features

### 6.1 Response Mocking
- [ ] Create mock configuration format:
  ```yaml
  mocks:
    - match:
        content_contains: "weather"
      response:
        content: "The weather is sunny and 72°F."
    - match:
        model: "gpt-4"
        system_contains: "json"
      response:
        content: '{"result": "mocked"}'
  ```
- [ ] Implement pattern matching engine
- [ ] Support regex patterns
- [ ] Add mock priority/ordering

### 6.2 Metrics & Observability
- [ ] Add Prometheus metrics endpoint (`/metrics`)
- [ ] Track:
  - Request count by endpoint and model
  - Response latency histograms
  - Token counts (input/output)
  - Error rates
  - Active connections
- [ ] Add structured logging with request IDs

### 6.3 Record/Replay Mode
- [ ] Implement proxy mode to real APIs
- [ ] Record requests/responses to file
- [ ] Replay recorded sessions
- [ ] Anonymize sensitive data in recordings

---

## Phase 7: Polish & Release

### 7.1 Documentation
- [ ] Write comprehensive README.md:
  - Quick start
  - Installation (cargo, binary, Docker)
  - Configuration reference
  - API compatibility matrix
  - Examples
- [ ] Add `docs/` folder with detailed guides
- [ ] Generate API documentation with rustdoc

### 7.2 Testing & Quality
- [ ] Achieve 80%+ code coverage
- [ ] Add load tests using k6 or similar
- [ ] Test against real client libraries (openai-python, anthropic-sdk)
- [ ] Fuzz testing for parser robustness

### 7.3 Release
- [ ] Set up GitHub releases with binaries (Linux, macOS, Windows)
- [ ] Publish to crates.io
- [ ] Create Homebrew formula
- [ ] Announce on relevant communities

---

## Milestone Summary

| Milestone | Description | Target Deliverable |
|-----------|-------------|-------------------|
| M1 | Foundation | Compiling workspace with types |
| M2 | Core Library | Token counting, latency, generators work |
| M3 | Basic Server | OpenAI chat completions endpoint works |
| M4 | Tool Calling | Function calling support |
| M5 | Multi-API | Anthropic + Gemini support |
| M6 | Advanced | Mocking, metrics, record/replay |
| M7 | Release | Published, documented, tested |

---

## Technical Decisions Log

Document significant technical decisions here as implementation progresses:

1. **Axum over Actix-web**: Axum is simpler, well-integrated with tokio, and has good streaming support
2. **tiktoken-rs**: Direct port of OpenAI's tokenizer, ensures accurate token counts
3. **YAML for config**: More readable than JSON, better for complex configurations
4. **Single crate with lib + bin**: Simpler structure with `llmsim` as library and `llmsim serve` as CLI subcommand. Avoids workspace complexity while still exposing library for programmatic use
5. **Clap subcommands**: Using `llmsim serve` pattern allows future expansion with additional commands (e.g., `llmsim mock`, `llmsim record`)
6. **Model list from models.dev**: GPT-5 family, o-series reasoning models, and Claude models based on current production models
