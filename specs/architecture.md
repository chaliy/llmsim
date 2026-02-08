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
│   │   └── state.rs    # Application state (config + stats)
│   ├── tui/            # Terminal UI dashboard
│   │   ├── mod.rs      # TUI module entry point
│   │   ├── app.rs      # Application state and event loop
│   │   └── ui.rs       # Ratatui widget rendering
│   ├── openai/         # OpenAI API types
│   │   ├── mod.rs
│   │   └── types.rs
│   ├── openresponses/  # OpenResponses API types (https://www.openresponses.org)
│   │   ├── mod.rs
│   │   ├── types.rs    # Request/response types
│   │   └── stream.rs   # OpenResponses-specific streaming
│   ├── stats.rs        # Real-time statistics tracking
│   ├── tokens.rs       # Token counting with tiktoken
│   ├── latency.rs      # Latency profile simulation
│   ├── generator.rs    # Response generators
│   ├── stream.rs       # SSE streaming engine
│   └── errors.rs       # Error injection
├── benchmarks/         # Load testing benchmarks (k6)
│   ├── run-benchmark.sh    # Main benchmark runner
│   ├── smoke-test.sh       # Quick smoke test
│   └── k6/                 # k6 test scripts
│       ├── config.js       # Shared configuration
│       ├── chat-completions.js
│       ├── endpoints.js
│       └── high-concurrency.js
└── Dockerfile          # Multi-stage build
```

## Usage

### As a CLI Tool

```bash
# Start the server
llmsim serve --port 8080

# Start with real-time stats dashboard
llmsim serve --tui

# With a config file
llmsim serve --config config.yaml

# Full options
llmsim serve \
  --port 8080 \
  --host 0.0.0.0 \
  --generator lorem \
  --target-tokens 150 \
  --tui
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

## API Endpoints

Provider-specific endpoints mirror their original API paths, prefixed with the provider name.
See `specs/api-endpoints.md` for the full specification.

### OpenAI Chat Completions API
- `POST /openai/v1/chat/completions` - Create a chat completion

### OpenAI Models API
- `GET /openai/v1/models` - List available models
- `GET /openai/v1/models/:model_id` - Get model details

### OpenAI Responses API
- `POST /openai/v1/responses` - Create a response (streaming and non-streaming)

See `specs/responses-api.md` for detailed Responses API specification.

### Module Organization

- **Public modules** (`openai`, `openresponses`, `generator`, `latency`, `stream`, `tokens`, `errors`, `stats`): Core library functionality, re-exported from `lib.rs`
- **CLI modules** (`cli/*`): Server-specific code, HTTP handlers and configuration
- **TUI modules** (`tui/*`): Terminal dashboard, built with Ratatui

### API Support

The server implements two LLM API specifications with provider-namespaced routes:

1. **OpenAI API** (`/openai/v1/...`)
   - `/openai/v1/chat/completions` - Chat completions (streaming & non-streaming)
   - `/openai/v1/models` - List available models
   - `/openai/v1/models/:model_id` - Get specific model
   - `/openai/v1/responses` - Responses API (streaming & non-streaming)

2. **OpenResponses API** (`/openresponses/v1/...`) - [openresponses.org](https://www.openresponses.org)
   - `/openresponses/v1/responses` - Create response (streaming & non-streaming)
   - Open-source specification for interoperable LLM interfaces
   - Supports text and message-based input
   - Full streaming with lifecycle events (response.created, response.output_text.delta, etc.)
   - Tool support, reasoning configuration, and metadata

### Stats Module

The `stats` module provides thread-safe metrics collection using atomic counters:

- **Request metrics**: total, active, streaming, non-streaming, per-model counts
- **Token metrics**: prompt tokens, completion tokens, total tokens
- **Error tracking**: by status code (429, 5xx, 504)
- **Latency**: average, min, max response times
- **RPS**: rolling 60-second window calculation

Stats are exposed via `/llmsim/stats` endpoint and consumed by the TUI dashboard.

### TUI Module

The `tui` module provides a real-time terminal dashboard built with [Ratatui](https://ratatui.rs/):

- **app.rs**: Event loop, state management, HTTP polling
- **ui.rs**: Widget layout and rendering (tables, sparklines, bar charts)

## Model Profiles

Model specifications are sourced from [models.dev](https://models.dev) and baked into the codebase at compile time. Each model profile includes:

- **Context Window**: Maximum input token limit
- **Max Output Tokens**: Maximum response length
- **Capabilities**: Function calling, vision, JSON mode, reasoning
- **Knowledge Cutoff**: Training data cutoff date
- **Release Date**: Model availability timestamp

### OpenAI Models

The `/openai/v1/models` endpoint returns realistic model data including context windows and output limits:

```json
{
  "object": "list",
  "data": [
    {
      "id": "gpt-5",
      "object": "model",
      "created": 1754524800,
      "owned_by": "openai",
      "context_window": 400000,
      "max_output_tokens": 128000
    }
  ]
}
```

### GPT-5 Family
| Model | Context | Max Output | Capabilities |
|-------|---------|------------|--------------|
| gpt-5 | 400K | 128K | Vision, Reasoning, Tools, JSON |
| gpt-5-pro | 400K | 272K | Vision, Reasoning, Tools, JSON |
| gpt-5-mini | 400K | 128K | Vision, Reasoning, Tools, JSON |
| gpt-5-nano | 400K | 128K | Vision, Reasoning, Tools, JSON |
| gpt-5-codex | 400K | 128K | Vision, Reasoning, Tools, JSON |
| gpt-5.1 | 400K | 128K | Vision, Reasoning, Tools, JSON |
| gpt-5.2 | 400K | 128K | Vision, Reasoning, Tools, JSON |
| gpt-5.3-codex | 400K | 128K | Vision, Reasoning, Tools, JSON |

### O-Series Reasoning Models
| Model | Context | Max Output | Capabilities |
|-------|---------|------------|--------------|
| o1 | 200K | 100K | Vision, Reasoning, Tools, JSON |
| o1-mini | 128K | 65K | Reasoning, JSON |
| o3 | 200K | 100K | Vision, Reasoning, Tools, JSON |
| o3-mini | 200K | 100K | Vision, Reasoning, Tools, JSON |
| o4-mini | 200K | 100K | Vision, Reasoning, Tools, JSON |

### GPT-4 Family
| Model | Context | Max Output | Capabilities |
|-------|---------|------------|--------------|
| gpt-4o | 128K | 16K | Vision, Tools, JSON |
| gpt-4o-mini | 128K | 16K | Vision, Tools, JSON |
| gpt-4-turbo | 128K | 4K | Tools, JSON |
| gpt-4 | 8K | 8K | Tools, JSON |
| gpt-4.1 | 1M | 32K | Vision, Tools, JSON |
| gpt-4.1-mini | 1M | 32K | Vision, Tools, JSON |
| gpt-4.1-nano | 1M | 32K | Vision, Tools, JSON |

### Claude Family
| Model | Context | Max Output | Capabilities |
|-------|---------|------------|--------------|
| claude-3.5-sonnet | 200K | 8K | Vision, Tools, JSON |
| claude-3.7-sonnet | 200K | 64K | Vision, Reasoning, Tools, JSON |
| claude-sonnet-4 | 200K | 64K | Vision, Tools, JSON |
| claude-sonnet-4.5 | 200K | 64K | Vision, Reasoning, Tools, JSON |
| claude-opus-4 | 200K | 64K | Vision, Tools, JSON |
| claude-opus-4.5 | 200K | 64K | Vision, Reasoning, Tools, JSON |
| claude-opus-4.6 | 1M | 128K | Vision, Reasoning, Tools, JSON |
| claude-haiku-4.5 | 200K | 64K | Vision, Reasoning, Tools, JSON |

### Gemini Family
| Model | Context | Max Output | Capabilities |
|-------|---------|------------|--------------|
| gemini-2.0-flash | 1M | 8K | Vision, Tools, JSON |
| gemini-2.5-flash | 1M | 65K | Vision, Tools, JSON |
| gemini-2.5-pro | 1M | 65K | Vision, Reasoning, Tools, JSON |

### DeepSeek Family
| Model | Context | Max Output | Capabilities |
|-------|---------|------------|--------------|
| deepseek-chat | 128K | 8K | Tools, JSON |
| deepseek-reasoner | 128K | 128K | Reasoning, Tools, JSON |

### Context Window Emulation (Future)

The context window information is included to support future context window limit emulation, where requests exceeding a model's context window will return appropriate error responses.
