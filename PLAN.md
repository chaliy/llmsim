# Implementation Plan

## Phase 1: Project Foundation

### 1.1 Project Setup
- [ ] Initialize Cargo workspace with two crates:
  - `llmsim` (library)
  - `llmsim-server` (binary)
- [ ] Configure `Cargo.toml` with metadata (name, version, license = "MIT", authors)
- [ ] Add initial dependencies:
  - `tokio` (async runtime)
  - `axum` (HTTP framework)
  - `serde` / `serde_json` (serialization)
  - `tiktoken-rs` (token counting)
  - `rand` (latency randomization)
  - `tracing` (logging)
- [ ] Create basic CI workflow (`.github/workflows/ci.yml`): format, lint, test

### 1.2 Core Types
- [ ] Define OpenAI API types in `llmsim/src/openai/types.rs`:
  - `ChatCompletionRequest`
  - `ChatCompletionResponse`
  - `ChatCompletionChunk` (for streaming)
  - `Message`, `Role`, `Usage`
  - `ToolCall`, `Function`
- [ ] Add serde derive macros with proper field naming (`#[serde(rename_all = "snake_case")]`)
- [ ] Write unit tests for serialization/deserialization against real API examples

---

## Phase 2: Core Library (llmsim)

### 2.1 Token Counter
- [ ] Create `llmsim/src/tokens.rs`
- [ ] Implement `count_tokens(text: &str, model: &str) -> usize`
- [ ] Support model-to-encoding mapping (gpt-4, gpt-3.5-turbo, claude, etc.)
- [ ] Add fallback for unknown models
- [ ] Write tests with known token counts

### 2.2 Latency Profiles
- [ ] Create `llmsim/src/latency.rs`
- [ ] Define `LatencyProfile` struct:
  ```rust
  pub struct LatencyProfile {
      pub ttft_mean_ms: u64,      // Time to first token
      pub ttft_stddev_ms: u64,
      pub tbt_mean_ms: u64,       // Time between tokens
      pub tbt_stddev_ms: u64,
  }
  ```
- [ ] Implement preset profiles:
  - `LatencyProfile::gpt4()` - slower, higher quality
  - `LatencyProfile::gpt35_turbo()` - faster
  - `LatencyProfile::claude_opus()` - Anthropic flagship
  - `LatencyProfile::claude_sonnet()` - balanced
  - `LatencyProfile::instant()` - no delay (for fast tests)
- [ ] Implement `LatencyProfile::sample_ttft()` and `sample_tbt()` using normal distribution
- [ ] Write tests for distribution sanity

### 2.3 Response Generator
- [ ] Create `llmsim/src/generator.rs`
- [ ] Implement `ResponseGenerator` trait:
  ```rust
  pub trait ResponseGenerator: Send + Sync {
      fn generate(&self, request: &ChatCompletionRequest) -> String;
  }
  ```
- [ ] Implement `LoremGenerator` - generates lorem ipsum text
- [ ] Implement `EchoGenerator` - echoes back the user message
- [ ] Implement `FixedGenerator` - returns configured fixed response
- [ ] Implement `RandomWordGenerator` - random words to target token count
- [ ] Add configurable response length (target token count)

### 2.4 Streaming Engine
- [ ] Create `llmsim/src/stream.rs`
- [ ] Implement `TokenStream` that yields `ChatCompletionChunk` with delays
- [ ] Support SSE format (`data: {...}\n\n`)
- [ ] Handle `[DONE]` termination message
- [ ] Integrate with latency profiles for inter-token delays
- [ ] Write integration test with mock time

### 2.5 Error Injection
- [ ] Create `llmsim/src/errors.rs`
- [ ] Define `ErrorConfig`:
  ```rust
  pub struct ErrorConfig {
      pub rate_limit_rate: f64,     // 0.0-1.0
      pub server_error_rate: f64,
      pub timeout_rate: f64,
      pub timeout_after_ms: u64,
  }
  ```
- [ ] Implement error decision logic
- [ ] Create proper OpenAI-format error responses
- [ ] Write tests for error rate distribution

### 2.6 Rate Limiter (Optional for Phase 2)
- [ ] Create `llmsim/src/rate_limit.rs`
- [ ] Implement token bucket algorithm
- [ ] Support requests-per-minute and tokens-per-minute limits
- [ ] Return proper 429 responses with `Retry-After` header

---

## Phase 3: Server Binary (llmsim-server)

### 3.1 Basic Server Setup
- [ ] Create `llmsim-server/src/main.rs`
- [ ] Set up Axum router with graceful shutdown
- [ ] Add health check endpoint (`GET /health`)
- [ ] Implement CLI argument parsing (clap):
  - `--port` (default: 8080)
  - `--host` (default: 0.0.0.0)
  - `--config` (optional config file path)
- [ ] Add tracing/logging setup

### 3.2 OpenAI Chat Completions Endpoint
- [ ] Implement `POST /v1/chat/completions`
- [ ] Parse `ChatCompletionRequest`
- [ ] Handle `stream: true` vs `stream: false`
- [ ] Return proper `ChatCompletionResponse` with usage
- [ ] Implement SSE streaming response
- [ ] Add request validation
- [ ] Write integration tests with reqwest

### 3.3 OpenAI Models Endpoint
- [ ] Implement `GET /v1/models`
- [ ] Return list of "available" models with metadata
- [ ] Implement `GET /v1/models/{model_id}`

### 3.4 Configuration
- [ ] Create `llmsim-server/src/config.rs`
- [ ] Support YAML/TOML config file:
  ```yaml
  server:
    port: 8080
    host: "0.0.0.0"

  latency:
    profile: "gpt4"  # or custom values

  response:
    generator: "lorem"
    target_tokens: 100

  errors:
    rate_limit_rate: 0.01
    server_error_rate: 0.001
  ```
- [ ] Support environment variable overrides
- [ ] Validate configuration on startup

### 3.5 Docker Support
- [ ] Create `Dockerfile` (multi-stage build)
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
4. **Workspace structure**: Separates reusable library from binary, enables future crate publication
