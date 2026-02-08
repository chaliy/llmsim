# Routine Maintenance Specification

## Abstract

This specification defines the routine maintenance process for llmsim. Regular maintenance ensures dependencies stay current, model profiles reflect the latest LLM landscape, code quality remains high, and documentation stays aligned with the implementation.

## Requirements

### R1: Dependency Updates

**R1.1**: Update all Rust dependencies to their latest compatible versions using `cargo update`.

**R1.2**: Evaluate major version upgrades for all dependencies. Apply major upgrades when:
- The new version has no breaking changes affecting this codebase, OR
- Breaking changes are straightforward to adapt

**R1.3**: Run `cargo audit` to check for known security vulnerabilities in dependencies.

**R1.4**: Verify license compliance with `cargo deny check licenses` after updates.

### R2: Model Profile Updates

**R2.1**: Review model profiles against [models.dev](https://models.dev) for accuracy:
- Context window sizes
- Max output token limits
- Capabilities (function calling, vision, JSON mode, reasoning)
- Knowledge cutoff dates
- Release timestamps

**R2.2**: Add newly released models from major providers:
- OpenAI (GPT, O-series)
- Anthropic (Claude)
- Google (Gemini)

**R2.3**: Update latency profiles in `src/latency.rs` to reflect new model families when applicable.

**R2.4**: Update the default model list in `src/cli/config.rs` to include new models.

### R3: Code Quality Review

**R3.1**: Check for and resolve all TODO, FIXME, HACK, and XXX comments in the codebase.

**R3.2**: Review code for security issues, focusing on:
- Input validation on API endpoints
- Error handling (no panicking on user input)
- Resource limits (body sizes, collection growth bounds)
- CORS configuration appropriateness

**R3.3**: Run the full lint and test suite:
- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`

### R4: Test Coverage

**R4.1**: Review test coverage across modules. Each public module should have unit tests for core functionality.

**R4.2**: Integration tests should cover all API endpoints.

**R4.3**: Tests should exercise both streaming and non-streaming paths.

### R5: Specification Alignment

**R5.1**: Verify all specs in `specs/` accurately reflect the current implementation:
- Model tables match the code
- API endpoints match the router
- Configuration options match the config struct

**R5.2**: Update specs when the code has diverged.

### R6: AGENTS.md Review

**R6.1**: Verify AGENTS.md reflects current project structure and conventions.

**R6.2**: Ensure the specs list is complete and accurate.

**R6.3**: Keep AGENTS.md concise and actionable for coding agents.

## Cadence

Routine maintenance should be performed:
- Monthly, or
- Before each minor/major release, or
- When significant new models are released by major providers
