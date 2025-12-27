# Examples

## Rust Example

Demonstrates library usage: token counting, generators, latency profiles, and streaming.

```bash
cargo run --example basic_usage
```

## Python Examples

Both Python examples require the server to be running first:

```bash
# Start server (headless)
cargo run --release -- serve --port 8080

# Or start with TUI dashboard to watch stats in real-time
cargo run --release -- serve --port 8080 --tui
```

### OpenAI SDK

Direct usage of the official OpenAI Python library:

```bash
uv run examples/openai_client.py
```

### LangChain

Using LangChain's OpenAI-compatible client:

```bash
uv run examples/langchain_client.py
```

### Custom Server URL

```bash
LLMSIM_URL=http://localhost:9000/openai uv run examples/openai_client.py
```

## Stats API

You can fetch real-time server statistics:

```bash
curl http://localhost:8080/llmsim/stats | jq
```

Response includes:
- Request counts (total, active, streaming, non-streaming)
- Token usage (prompt, completion, total)
- Error breakdown (rate limits, server errors, timeouts)
- Latency metrics (avg, min, max)
- Requests per second
- Per-model request distribution
