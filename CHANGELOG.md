# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **TypeSafe System One API support**: new `POST /typesafe/v1/systemone` and
  `GET /typesafe/v1/models` endpoints, wire-compatible with the official
  `typesafe-sdk` clients when pointed at `{base_url}/typesafe`. Simulates the
  Jev model (`jev-latest` / `jev-preview` → `jev-1.13.0`) answering noul,
  choice, and score questions with deterministic, well-formed probabilities,
  FastAPI-style `422` validation errors, TypeSafe's error envelope for injected
  errors, a `jev` latency profile, and a `systemone_requests` stat counter. New
  Python SDK example. See `knowledge/apis/typesafe-api.md`.
- **Anthropic Messages API support**: new `/anthropic/v1/messages` endpoint
  (streaming and non-streaming) plus `/anthropic/v1/models` and
  `/anthropic/v1/models/:id`, wire-compatible with the official Anthropic SDKs
  when pointed at `{base_url}/anthropic`. Includes realistic Claude model
  profiles (sourced from models.dev) using the real Anthropic API model IDs
  (`claude-opus-4-8`, `claude-sonnet-4-6`, `claude-haiku-4-5`, `claude-fable-5`,
  …) with dated-snapshot and `-latest` aliases, the Anthropic SSE event
  sequence, the Anthropic error envelope, scripted `tool_use` support, and a
  `messages_requests` stat counter. New examples for Python, TypeScript, Go,
  curl, and LangChain. See `knowledge/apis/anthropic-api.md`.
- **New models**: GPT-5.6 (Sol/Terra/Luna), GPT-6 Astra/Sol/Luna, GPT-6.1 Sol,
  gpt-image-2, Claude Opus 5 / 5.5, Sonnet 5 / 5.5, Haiku 5.5, Fable 5.1,
  Gemini 3.5 Flash / Flash Lite and 3.6 / 3.7 / 3.8 Flash, DeepSeek V4 Pro and
  V4.1 Flash (`deepseek-flash`), with profiles from models.dev.
- **Anthropic extended thinking**: `thinking` and `output_config.effort` are
  honored. Claude 5.x / Fable models (and any model sent
  `thinking: {type: "adaptive"}`) return a `thinking` block with a signature
  before the text, streamed as `thinking_delta` / `signature_delta`; thinking
  tokens scale with effort and count in `usage.output_tokens`. Configurations
  the real API rejects return `400 invalid_request_error`.
- **Anthropic token counting**: `POST /anthropic/v1/messages/count_tokens`.
- **Chat Completions `reasoning_effort`**: reasoning models report
  `completion_tokens_details.reasoning_tokens` (included in
  `completion_tokens`). New `max` effort level on chat and Responses.

### Changed

- Reasoning-model detection uses model profiles, so GPT-5.6/GPT-6 produce
  reasoning tokens and `gpt-5.3-chat-latest` no longer does.
- Claude Opus 4.8 and Fable 5 release dates corrected; Opus 4.8 now carries
  its published knowledge cutoff.
- Rust toolchain pinned to 1.99; `tuika` 0.13, `tokio-tungstenite` 0.30.

## [0.5.1] - 2026-06-28

### Highlights

- **~1100x throughput improvement**: tokenizers are now cached and stats are
  tracked lock-free, dramatically increasing peak requests/sec. Includes a new
  throughput benchmark to measure parallelisation scaling.
- The CLI now preserves the port configured in the config file when the
  `--port` flag is absent, matching the existing host-handling behavior.

### What's Changed

* fix(cli): preserve config port when --port flag is absent ([#68](https://github.com/chaliy/llmsim/pull/68)) by @chaliy
* perf: cache tokenizers + lock-free stats (~1100x throughput), add throughput benchmark ([#66](https://github.com/chaliy/llmsim/pull/66)) by @chaliy

**Full Changelog**: https://github.com/chaliy/llmsim/compare/v0.5.0...v0.5.1

## [0.5.0] - 2026-06-22

### Highlights

- **Optional Cargo features** let library consumers slim their builds. New
  features `tokens` (tiktoken-rs), `server` (axum/tower-http, implies
  `tokens`), and `cli` (clap/tracing-subscriber + binary, implies `server`),
  with `tui` now implying `cli`. The default feature set is `["cli"]`, so the
  binary and `cargo test` are unchanged; consumers can use
  `llmsim = { default-features = false }` to drop `axum`, `tower-http`,
  `tiktoken-rs`, `clap`, websockets, and `tracing-subscriber`.
- **Claude Opus 4.8** added to the model registry, default model list, and
  API docs as the current flagship Anthropic model.
- Major dependency upgrades: `tiktoken-rs` 0.11 → 0.12 and `tower-http`
  0.6 → 0.7 (plus transitive bumps).

### What's Changed

* feat(build): gate server, tokens, and CLI deps behind Cargo features ([#63](https://github.com/chaliy/llmsim/pull/63)) by @chaliy

**Full Changelog**: https://github.com/chaliy/llmsim/compare/v0.4.0...v0.5.0

## [0.4.0] - 2026-05-28

### Highlights

- **Scripted response mode** for agent scenario tests — replay an ordered
  list of assistant turns (text, tool calls, mixed, errors) with a
  configurable `on_exhausted` policy. See `knowledge/simulation/scripted-mode.md`.
- CLI now honors `LLMSIM_HOST` and preserves the host configured in the
  config file when not overridden on the command line.
- Hardening across the streaming and WebSocket paths: stats callback
  now finalizes on stream drop, WebSocket capacity is reserved before
  upgrade, and a connection cap protects against runaway clients.
- Security and reliability hardening in CI release/publish workflows
  (commit-message injection, tag validation, action tampering) and
  per-model stats tracking is bounded to prevent memory DoS.

### What's Changed

* docs(examples): add Rust scripted-mode example and CI smoke-test ([#61](https://github.com/chaliy/llmsim/pull/61)) by @chaliy
* feat(script): add scripted response mode for agent scenario tests ([#60](https://github.com/chaliy/llmsim/pull/60)) by @chaliy
* fix(cli): honor LLMSIM_HOST and preserve config host ([#59](https://github.com/chaliy/llmsim/pull/59)) by @chaliy
* docs(specs): document operational defaults policy ([#58](https://github.com/chaliy/llmsim/pull/58)) by @chaliy
* fix(ci): remove unpinned Python example execution from CI ([#56](https://github.com/chaliy/llmsim/pull/56)) by @chaliy
* fix(benchmarks): bind local benchmark server to loopback ([#55](https://github.com/chaliy/llmsim/pull/55)) by @chaliy
* fix(responses): finalize stats callback on stream drop ([#54](https://github.com/chaliy/llmsim/pull/54)) by @chaliy
* fix(ws): reserve websocket capacity before upgrade ([#53](https://github.com/chaliy/llmsim/pull/53)) by @chaliy
* fix(tui): reject invalid chars in stats endpoint URL ([#52](https://github.com/chaliy/llmsim/pull/52)) by @chaliy
* fix(ci): restrict publish paths to release-tagged flows ([#51](https://github.com/chaliy/llmsim/pull/51)) by @chaliy
* fix(stats): bound per-model request tracking to prevent memory DoS ([#50](https://github.com/chaliy/llmsim/pull/50)) by @chaliy
* fix(ci): harden crates publish job against action tampering ([#49](https://github.com/chaliy/llmsim/pull/49)) by @chaliy
* fix(ci): prevent release workflow commit message injection ([#48](https://github.com/chaliy/llmsim/pull/48)) by @chaliy
* fix(ci): validate release tag safely in publish workflow ([#46](https://github.com/chaliy/llmsim/pull/46)) by @chaliy
* fix(ws): enforce websocket connection cap ([#45](https://github.com/chaliy/llmsim/pull/45)) by @chaliy

**Full Changelog**: https://github.com/chaliy/llmsim/compare/v0.3.0...v0.4.0

## [0.3.0] - 2026-05-19

### Highlights

- Config file format migrated from YAML to TOML (breaking change)
- Refreshed model catalog: Claude Opus 4.7, Sonnet 4.6, latest GPT-5.4/5.5, Gemini 3/3.1
- Reduced default dependency graph for faster builds and smaller binaries
- Rust toolchain pinned to 1.95 across CI and local dev
- New maintenance skill for the routine maintenance workflow

### Breaking Changes

- **Config file format moved from YAML to TOML.** The upstream `serde_yaml`
  crate is archived (released as `0.9.34+deprecated`) and every successor
  fork is stale, while `toml` is actively maintained by the Cargo team and
  is the idiomatic format for Rust CLIs. To migrate an existing
  `config.yaml`, replace section headers like `server:` with `[server]`,
  change `key: value` to `key = value`, quote strings, and convert lists
  to TOML arrays. See `benchmarks/config/*.toml` for working examples.
  - `Config::from_yaml` is renamed to `Config::from_toml`.
  - `Config::from_file` now expects TOML content (the `--config` flag is
    unchanged).
  - The bundled `benchmarks/config/benchmark.yaml` and `chaos.yaml`
    examples have been replaced with `.toml` versions.

### What's Changed

* feat(models): refresh model catalog for maintenance ([#43](https://github.com/chaliy/llmsim/pull/43)) by @chaliy
* perf(deps): reduce default dependency graph ([#42](https://github.com/chaliy/llmsim/pull/42)) by @chaliy
* chore: remove stale inception docs PLAN.md and IDEA.md ([#41](https://github.com/chaliy/llmsim/pull/41)) by @chaliy
* docs(skill): add maintenance skill for routine maintenance workflow ([#40](https://github.com/chaliy/llmsim/pull/40)) by @chaliy
* ci(examples): cover openai_websocket_client and openresponses_client ([#39](https://github.com/chaliy/llmsim/pull/39)) by @chaliy
* ci: pin Rust toolchain to 1.95 across CI and local dev ([#38](https://github.com/chaliy/llmsim/pull/38)) by @chaliy
* chore(config)!: migrate config file format from YAML to TOML ([#37](https://github.com/chaliy/llmsim/pull/37)) by @chaliy
* docs(specs): document OpenResponses endpoints and fix default port ([#36](https://github.com/chaliy/llmsim/pull/36)) by @chaliy
* feat(models): add Claude Opus 4.7 and Sonnet 4.6 ([#35](https://github.com/chaliy/llmsim/pull/35)) by @chaliy
* chore(deps): bump rand, rand_distr, tiktoken-rs, tokio-tungstenite ([#34](https://github.com/chaliy/llmsim/pull/34)) by @chaliy
* chore: remove agent attribution with identity enforcement ([#33](https://github.com/chaliy/llmsim/pull/33)) by @chaliy
* docs: add README badges for CI, crates.io, and agent friendly ([#32](https://github.com/chaliy/llmsim/pull/32)) by @chaliy

**Full Changelog**: https://github.com/chaliy/llmsim/compare/v0.2.3...v0.3.0

## [0.2.3] - 2026-03-20

### Highlights

- WebSocket mode for Responses API streaming
- OpenAI thinking/reasoning emulation support
- Fixed repository URLs for crates.io listing

### What's Changed

* chore: routine maintenance - update deps and align specs ([#30](https://github.com/chaliy/llmsim/pull/30)) by @chaliy
* fix: correct repository URLs for crates.io listing ([#29](https://github.com/chaliy/llmsim/pull/29)) by @chaliy
* chore: add attribution settings and agent guidance for commits/PRs ([#28](https://github.com/chaliy/llmsim/pull/28)) by @chaliy
* feat: add /ship command for full shipping workflow ([#27](https://github.com/chaliy/llmsim/pull/27)) by @chaliy
* feat(api): add WebSocket mode for Responses API ([#26](https://github.com/chaliy/llmsim/pull/26)) by @chaliy
* feat(api): add OpenAI thinking/reasoning emulation ([#25](https://github.com/chaliy/llmsim/pull/25)) by @chaliy

**Full Changelog**: https://github.com/chaliy/llmsim/compare/v0.2.2...v0.2.3

## [0.2.2] - 2026-02-08

### Highlights

- Updated dependencies and model profiles for routine maintenance
- Fixed CI release workflow to call publish workflow directly

### What's Changed

* chore: routine maintenance - update deps, models, and specs ([#23](https://github.com/chaliy/llmsim/pull/23)) by @chaliy
* fix(ci): call publish workflow directly from release workflow ([#22](https://github.com/chaliy/llmsim/pull/22)) by @chaliy

**Full Changelog**: https://github.com/chaliy/llmsim/compare/v0.2.1...v0.2.2

## [0.2.1] - 2026-02-08

### Highlights

- New model profiles: Claude Opus 4.6, GPT-5.3 Codex

### What's Changed

* docs: adopt changelog format with highlights and full changelog link ([#21](https://github.com/chaliy/llmsim/pull/21)) by @chaliy
* fix(ci): fix failing build and add CI merge policy ([#20](https://github.com/chaliy/llmsim/pull/20)) by @chaliy
* feat(models): add Claude Opus 4.6, GPT-5.3 Codex, and update model profiles ([#17](https://github.com/chaliy/llmsim/pull/17)) by @chaliy
* fix(ci): wrap if condition in expression syntax for YAML parsing ([#16](https://github.com/chaliy/llmsim/pull/16)) by @chaliy

**Full Changelog**: https://github.com/chaliy/llmsim/compare/v0.2.0...v0.2.1

## [0.2.0] - 2026-01-17

### Highlights

- Provider-namespaced API routes for multi-provider support and SDK compatibility
- Realistic model profiles sourced from models.dev
- OpenAI Responses API endpoint support
- Real-time stats display in the console UI

### Breaking Changes

- **API endpoints now require provider prefix**: All provider-specific API endpoints are now prefixed with the provider name. This change improves multi-provider support and SDK compatibility.
  - `/v1/chat/completions` → `/openai/v1/chat/completions`
  - `/v1/responses` → `/openai/v1/responses`
  - `/v1/models` → `/openai/v1/models`
  - When using official SDKs, configure the base URL with the provider prefix:
    ```python
    # OpenAI Python SDK
    client = OpenAI(base_url="http://localhost:3000/openai/v1", api_key="not-needed")
    ```

### What's Changed

* feat(models): Add realistic model profiles from models.dev ([#12](https://github.com/chaliy/llmsim/pull/12)) by @chaliy
* feat(api): add OpenResponses API provider with provider-namespaced routes ([#11](https://github.com/chaliy/llmsim/pull/11)) by @chaliy
* docs: improve README and AGENTS.md documentation ([#10](https://github.com/chaliy/llmsim/pull/10)) by @chaliy
* feat(api): Add load and stress testing benchmarks ([#7](https://github.com/chaliy/llmsim/pull/7)) by @chaliy
* feat(api): Add OpenAI responses endpoint support ([#6](https://github.com/chaliy/llmsim/pull/6)) by @chaliy
* feat(tui): Add real-time stats display to console UI ([#5](https://github.com/chaliy/llmsim/pull/5)) by @chaliy
* docs: Create simple JavaScript AI SDK example ([#4](https://github.com/chaliy/llmsim/pull/4)) by @chaliy
* docs: Create README, license, and contribution files ([#3](https://github.com/chaliy/llmsim/pull/3)) by @chaliy
* docs: Add usage examples for Rust and Python ([#2](https://github.com/chaliy/llmsim/pull/2)) by @chaliy
* feat: LLMSim Library and Server ([#1](https://github.com/chaliy/llmsim/pull/1)) by @chaliy

**Full Changelog**: https://github.com/chaliy/llmsim/commits/v0.2.0

[Unreleased]: https://github.com/chaliy/llmsim/compare/v0.5.1...HEAD
[0.5.1]: https://github.com/chaliy/llmsim/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/chaliy/llmsim/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/chaliy/llmsim/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/chaliy/llmsim/compare/v0.2.3...v0.3.0
[0.2.3]: https://github.com/chaliy/llmsim/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/chaliy/llmsim/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/chaliy/llmsim/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/chaliy/llmsim/releases/tag/v0.2.0
