# Plan: WebSocket Mode for OpenAI Responses API

## Overview

Implement WebSocket transport for the Responses API endpoint (`/openai/v1/responses`), mirroring OpenAI's WebSocket mode. This allows clients to connect via WebSocket and send `response.create` events, receiving the same streaming events the HTTP SSE endpoint already produces.

## Key Behaviors

- **Connection URL**: `ws://localhost:{port}/openai/v1/responses` (WebSocket upgrade on the existing path)
- **Client → Server**: JSON messages with `type: "response.create"` containing the same body as the HTTP Responses API (minus `stream`/`background`)
- **Server → Client**: Same event types as the existing SSE streaming (response.created, response.output_text.delta, response.completed, etc.), sent as JSON WebSocket text frames
- **Connection-local cache**: The most recent response is kept in memory per connection, enabling fast continuations via `previous_response_id`
- **Sequential execution**: One in-flight response per connection (no multiplexing)
- **60-minute connection timeout**: Connections close after 60 minutes

## Implementation Steps

### Step 1: Add WebSocket dependency
- Add `axum` WebSocket feature (axum has built-in WebSocket support via `axum::extract::ws`)
- No new crate needed — axum's `ws` feature uses tokio-tungstenite under the hood

**File**: `Cargo.toml`

### Step 2: Define WebSocket message types
- Create `src/openai/websocket.rs` module with:
  - `WebSocketClientEvent` — wraps `response.create` with optional `generate: false`
  - `WebSocketServerEvent` — wraps existing response streaming event types for WS framing
  - Error event types (`previous_response_not_found`, `websocket_connection_limit_reached`)

**Files**: `src/openai/websocket.rs`, `src/openai/mod.rs`

### Step 3: Implement WebSocket handler
- Create `ws_responses` handler in `src/cli/handlers.rs` that:
  1. Upgrades the HTTP connection to WebSocket
  2. Maintains a connection-local `HashMap<String, ResponsesResponse>` (or just a single `Option<ResponsesResponse>` for the last response)
  3. Reads `response.create` messages from the client
  4. Reuses existing response generation logic (generators, latency profiles, token counting)
  5. Sends back streaming events as JSON text frames (same event types as SSE)
  6. Tracks stats via the existing `SharedStats`
  7. Enforces one-at-a-time sequential processing
  8. Closes connection after 60 minutes

**File**: `src/cli/handlers.rs` (or a dedicated `src/cli/ws_handler.rs` to keep handlers.rs manageable)

### Step 4: Register WebSocket route
- Add the WebSocket handler to the router in `src/cli/mod.rs`
- The route needs to handle both POST (existing HTTP handler) and GET with WebSocket upgrade on `/openai/v1/responses`
- Axum can distinguish via the `WebSocketUpgrade` extractor — if the request is a WS upgrade, route to the WS handler; otherwise fall through to the POST handler

**File**: `src/cli/mod.rs`

### Step 5: Wire up response generation for WebSocket
- Extract the core response generation logic from the existing `create_response` handler into a shared function that both HTTP and WS handlers can call
- This function takes a `ResponsesRequest` and returns the generated content, usage stats, and event sequence
- The WS handler converts these events to JSON text frames instead of SSE

**File**: `src/cli/handlers.rs` (refactor)

### Step 6: Add WebSocket stats tracking
- Add `websocket_connections` counter to `Stats` (active WS connections)
- Add `EndpointType::WebSocketResponses` variant for tracking
- Track WS-specific metrics: connections opened, closed, messages sent/received

**File**: `src/stats.rs`

### Step 7: Integration tests
- Add WebSocket integration tests in `tests/`
- Test: connect, send `response.create`, receive events, verify event sequence
- Test: `previous_response_id` continuation within a connection
- Test: sequential execution (second request waits for first)
- Test: connection timeout behavior
- Test: error events for invalid messages

**File**: `tests/websocket_test.rs`

### Step 8: Update specs and docs
- Update `specs/api-endpoints.md` with WebSocket endpoint
- Update `specs/responses-api.md` with WebSocket mode section
- Update `specs/architecture.md` with new module
- Create/update `docs/` with WebSocket usage examples
- Update `AGENTS.md` conventions section

**Files**: `specs/api-endpoints.md`, `specs/responses-api.md`, `specs/architecture.md`, `docs/`

## Dependency Changes

```toml
# Only change: enable axum's "ws" feature
axum = { version = "0.8", features = ["macros", "ws"] }
```

## Event Format (WebSocket frames)

Client sends:
```json
{
  "type": "response.create",
  "response": {
    "model": "gpt-5",
    "input": [{"role": "user", "content": "Hello"}],
    "tools": [],
    "temperature": 0.7
  }
}
```

Server sends (sequence of JSON text frames):
```json
{"type": "response.created", "response": {"id": "resp_...", ...}}
{"type": "response.output_item.added", ...}
{"type": "response.output_text.delta", "delta": "Hello", ...}
{"type": "response.output_text.done", ...}
{"type": "response.output_item.done", ...}
{"type": "response.completed", "response": {"id": "resp_...", "usage": {...}, ...}}
```

## Risk Assessment

- **Low risk**: Axum has mature WebSocket support; the response generation logic already exists
- **Medium risk**: Routing both POST and WS upgrade on the same path needs care (axum handles this cleanly with extractors)
- **Low risk**: Connection-local state is straightforward (single-connection, no shared mutable state)

## Checklist

- [ ] `cargo fmt` passes
- [ ] `cargo clippy` has no warnings
- [ ] `cargo test` passes (including new WS tests)
- [ ] Specs updated
- [ ] Docs updated
