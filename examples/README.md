# Examples

## Rust Example

Demonstrates library usage: token counting, generators, latency profiles, and streaming.

```bash
cargo run --example basic_usage
```

## Python (LangChain) Example

Shows how to connect to llmsim using LangChain's OpenAI-compatible client.

1. Start the server:
   ```bash
   cargo run --release -- serve --port 8080
   ```

2. Run the client (in another terminal):
   ```bash
   uv run examples/langchain_client.py
   ```

To use a different server URL:
```bash
LLMSIM_URL=http://localhost:9000/v1 uv run examples/langchain_client.py
```
