# Examples

Usage examples for LLMSim with various SDKs and languages.

For API documentation, see [docs/api.md](../docs/api.md).

## Prerequisites

Start the llmsim server first:

```bash
# Headless
llmsim serve --port 8080

# Or with TUI dashboard
llmsim serve --port 8080 --tui

# Or from source
cargo run --release -- serve --port 8080
```

## Rust

Library usage: token counting, generators, latency profiles, streaming.

```bash
cargo run --example basic_usage
```

## Python

### OpenAI SDK

```bash
uv run examples/openai_client.py
```

### OpenResponses API

```bash
uv run examples/openresponses_client.py
```

### LangChain

```bash
uv run examples/langchain_client.py
```

## TypeScript

```bash
npm install openai
npx tsx examples/openai_client.ts
```

## Custom Server URL

```bash
LLMSIM_URL=http://localhost:9000/openai/v1 uv run examples/openai_client.py
LLMSIM_URL=http://localhost:9000 uv run examples/openresponses_client.py
```
