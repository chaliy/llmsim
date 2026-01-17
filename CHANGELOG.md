# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - Unreleased

Initial release of LLMSim - a lightweight, high-performance LLM API simulator.

### Added

- OpenAI Chat Completions API (`/openai/v1/chat/completions`)
- OpenAI Responses API (`/openai/v1/responses`)
- OpenAI Models API (`/openai/v1/models`, `/openai/v1/models/:id`)
- Streaming support with Server-Sent Events (SSE)
- Realistic token-by-token streaming with configurable delays
- Realistic model profiles based on models.dev data
- Configurable latency simulation (constant, uniform, normal, log-normal)
- Token counting using tiktoken
- Health check endpoint (`/health`)
- Statistics endpoint (`/llmsim/stats`)
- Terminal UI (TUI) with real-time stats display
- CLI with configurable host, port, and latency parameters
- Usage examples for Rust, Python, and JavaScript
- Load testing benchmarks with k6

[Unreleased]: https://github.com/llmsim/llmsim/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/llmsim/llmsim/releases/tag/v0.1.0
