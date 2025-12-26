# Examples

## Rust Example

Demonstrates library usage: token counting, generators, latency profiles, and streaming.

```bash
cargo run --example basic_usage
```

## Python Examples

Both Python examples require the server to be running first:

```bash
cargo run --release -- serve --port 8080
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
