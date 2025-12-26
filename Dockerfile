# Build stage
FROM rust:1.83-slim-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Create dummy source files for dependency caching
RUN mkdir -p src/cli src/openai \
    && echo "pub fn dummy() {}" > src/lib.rs \
    && echo "fn main() {}" > src/main.rs \
    && echo "pub mod config; pub mod handlers; pub mod state; pub use config::{Config, ConfigError}; pub use state::AppState; pub async fn run_server(_config: Config) -> Result<(), Box<dyn std::error::Error>> { Ok(()) }" > src/cli/mod.rs \
    && echo "pub struct Config; pub enum ConfigError {}" > src/cli/config.rs \
    && echo "" > src/cli/handlers.rs \
    && echo "pub struct AppState;" > src/cli/state.rs \
    && echo "" > src/openai/mod.rs

# Build dependencies (this layer is cached)
RUN cargo build --release

# Remove dummy source files
RUN rm -rf src

# Copy actual source code
COPY src ./src

# Build the actual application
RUN touch src/lib.rs src/main.rs \
    && cargo build --release

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary
COPY --from=builder /app/target/release/llmsim /usr/local/bin/

# Create non-root user
RUN useradd -m -s /bin/bash llmsim
USER llmsim

# Default configuration
ENV LLMSIM_HOST=0.0.0.0
ENV LLMSIM_PORT=8080

EXPOSE 8080

ENTRYPOINT ["llmsim", "serve"]
