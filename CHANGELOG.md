# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-01-17

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

- feat(models): Add realistic model profiles from models.dev ([#12](https://github.com/chaliy/llmsim/pull/12)) by @chaliy
- feat(api): add OpenResponses API provider with provider-namespaced routes ([#11](https://github.com/chaliy/llmsim/pull/11)) by @chaliy
- docs: improve README and AGENTS.md documentation ([#10](https://github.com/chaliy/llmsim/pull/10)) by @chaliy
- Add OpenAI responses endpoint support ([#6](https://github.com/chaliy/llmsim/pull/6)) by @chaliy
- Add load and stress testing benchmarks ([#7](https://github.com/chaliy/llmsim/pull/7)) by @chaliy
- Add real-time stats display to console UI ([#5](https://github.com/chaliy/llmsim/pull/5)) by @chaliy
- Create simple JavaScript AI SDK example ([#4](https://github.com/chaliy/llmsim/pull/4)) by @chaliy
- Add usage examples for Rust and Python ([#2](https://github.com/chaliy/llmsim/pull/2)) by @chaliy
- Create README, license, and contribution files ([#3](https://github.com/chaliy/llmsim/pull/3)) by @chaliy
- LLMSim Library and Server ([#1](https://github.com/chaliy/llmsim/pull/1)) by @chaliy

[Unreleased]: https://github.com/chaliy/llmsim/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/chaliy/llmsim/releases/tag/v0.2.0
