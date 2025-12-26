# LLMSim Architecture

## Project Structure

LLMSim is organized as a single Rust crate with both library and binary targets:

```
llmsim/
├── Cargo.toml          # Single package with [lib] and [[bin]] sections
├── src/
│   ├── lib.rs          # Library entry point (public API)
│   ├── main.rs         # Binary entry point (CLI)
│   ├── cli/            # CLI-specific modules
│   │   ├── mod.rs      # Server runner
│   │   ├── config.rs   # Configuration loading
│   │   ├── handlers.rs # HTTP request handlers
│   │   └── state.rs    # Application state
│   ├── openai/         # OpenAI API types
│   │   ├── mod.rs
│   │   └── types.rs
│   ├── tokens.rs       # Token counting with tiktoken
│   ├── latency.rs      # Latency profile simulation
│   ├── generator.rs    # Response generators
│   ├── stream.rs       # SSE streaming engine
│   └── errors.rs       # Error injection
└── Dockerfile          # Multi-stage build
```

## Usage

### As a CLI Tool

```bash
# Start the server
llmsim serve --port 8080 --latency-profile gpt5

# With a config file
llmsim serve --config config.yaml

# Full options
llmsim serve \
  --port 8080 \
  --host 0.0.0.0 \
  --latency-profile claude-sonnet \
  --generator lorem \
  --target-tokens 150
```

### As a Library

```rust
use llmsim::{
    openai::{ChatCompletionRequest, Message},
    generator::LoremGenerator,
    latency::LatencyProfile,
    TokenStreamBuilder,
};

// Create a latency profile
let latency = LatencyProfile::gpt5();

// Generate a response
let generator = LoremGenerator::new(100);
let response = generator.generate(&request);

// Count tokens
let tokens = llmsim::count_tokens("Hello, world!", "gpt-5").unwrap();
```

## Design Decisions

### Single Crate Structure

The project uses a single crate with both `[lib]` and `[[bin]]` targets rather than a Cargo workspace:

**Pros:**
- Simpler project structure
- Easier to maintain
- Single `Cargo.toml` to manage
- Library API exposed via `llmsim` crate
- Binary available as `llmsim` command

**Cons:**
- CLI dependencies included in library (minimal overhead)
- Less separation of concerns

### CLI Subcommand Pattern

Using `llmsim serve` allows for future expansion:
- `llmsim serve` - Run the HTTP server
- `llmsim mock` (future) - Run with mock configuration
- `llmsim record` (future) - Proxy and record real API calls

### Module Organization

- **Public modules** (`openai`, `generator`, `latency`, `stream`, `tokens`, `errors`): Core library functionality, re-exported from `lib.rs`
- **CLI modules** (`cli/*`): Server-specific code, only used by the binary

## Supported Models

Based on [models.dev](https://models.dev/api.json):

### GPT-5 Family
- gpt-5, gpt-5-mini, gpt-5-codex
- gpt-5.1, gpt-5.1-codex, gpt-5.1-codex-mini, gpt-5.1-codex-max
- gpt-5.2

### O-Series Reasoning
- o3, o3-mini, o4-mini

### GPT-4 Family
- gpt-4, gpt-4-turbo, gpt-4o, gpt-4o-mini, gpt-4.1

### Claude Family
- claude-3.5-sonnet, claude-3.7-sonnet
- claude-sonnet-4, claude-sonnet-4.5
- claude-opus-4, claude-opus-4.5
- claude-haiku-4.5
