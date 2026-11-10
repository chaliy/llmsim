---
type: Specification
title: "Python and Node Bindings"
description: "In-process llmsim server for Python (PyPI `llmsim`) and Node/TypeScript (npm `llmsim`): scope, API shape, config merge, lifecycle, packaging."
tags:
  - llmsim
  - foundations
  - python
  - javascript
---
# Python and Node Bindings

## Abstract

`llmsim` ships as native packages for Python (PyO3, PyPI `llmsim`) and
Node/TypeScript (NAPI-RS, npm `llmsim`), modeled on bashkit's packaging. Each
package embeds the same Rust HTTP server as the CLI and starts it inside the
caller's process, so tests point official OpenAI/Anthropic SDKs at a local
URL without installing a binary or running a sidecar. The bindings expose the
server, not the internal library modules: the wire API is the product, and the
SDKs already speak it.

## Requirements

### R1: Scope

**R1.1**: The packages expose one concept, a running server, with start,
stop, its URL, provider base URLs (`/openai/v1`, `/anthropic`,
`/openresponses/v1`), the effective config, and live stats
(the `GET /llmsim/stats` shape).

**R1.2**: Every feature of `llmsim serve` (generators, latency profiles, error
injection, scripted mode, scenarios, models) is reachable through config,
with no per-feature binding code.

### R2: Configuration

**R2.1**: Config is the TOML file's shape. Sources merge in a fixed order,
later wins: a TOML file or text, then a dict/object shaped like it, then
shortcut options (`host`, `port`, `generator`, `target_tokens`, `latency`,
`script`, `scenarios`).

**R2.2**: The deep merge lives in the core crate (`Config::merged_with_json`)
so both bindings share it. Objects merge per key; arrays and scalars replace.

**R2.3**: Bindings default to `127.0.0.1` and port 0 (the OS picks a free
port), so parallel test workers never collide. The CLI default (`0.0.0.0:8080`)
is unchanged.

**R2.4**: Invalid config fails before a server starts: at construction in
Python, at `start()` / `resolveConfig()` in Node.

### R3: Lifecycle

**R3.1**: `start` returns only after the listener is bound (core
`start_server`), so the reported port is real and requests never race the
bind. Bind conflicts and unreadable script/scenario files are errors.

**R3.2**: `stop` drains in-flight requests and is idempotent. Dropping the
native handle signals shutdown without blocking.

**R3.3**: Python: each server owns a private two-worker Tokio runtime and
every blocking call releases the GIL, so sync SDK clients in the same thread
work. `Server` is a sync and async context manager.

**R3.4**: Node: servers run on napi's shared Tokio runtime; `start` and
`stop` return Promises and never block the event loop. A forgotten server
does not keep the process alive. `Server` implements `Symbol.asyncDispose`.

### R4: Packaging and versioning

**R4.1**: Binding crates are workspace members under `crates/`, excluded from
the crates.io package and from default cargo members.

**R4.2**: One version for crates.io, PyPI and npm: `[workspace.package]` in
`Cargo.toml`. maturin reads it; `crates/llmsim-js/package.json` must match,
which the `Bindings` CI workflow enforces.

**R4.3**: Python ships abi3 (`cp39-abi3`) wheels for Linux glibc and musl
(x86_64, aarch64), macOS (x86_64, arm64) and Windows x64, plus an sdist.
npm ships one package carrying all five platform `.node` binaries, not
per-platform packages.

**R4.4**: Publishing uses trusted publishing (OIDC) for both registries.
See [Release Process](../project/release-process.md).
