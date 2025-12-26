# Build stage
FROM rust:1.83-slim-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY crates/llmsim/Cargo.toml ./crates/llmsim/
COPY crates/llmsim-server/Cargo.toml ./crates/llmsim-server/

# Create dummy source files for dependency caching
RUN mkdir -p crates/llmsim/src crates/llmsim-server/src \
    && echo "pub fn dummy() {}" > crates/llmsim/src/lib.rs \
    && echo "fn main() {}" > crates/llmsim-server/src/main.rs

# Build dependencies (this layer is cached)
RUN cargo build --release --package llmsim-server

# Remove dummy source files
RUN rm -rf crates/llmsim/src crates/llmsim-server/src

# Copy actual source code
COPY crates/llmsim/src ./crates/llmsim/src
COPY crates/llmsim-server/src ./crates/llmsim-server/src

# Build the actual application
RUN touch crates/llmsim/src/lib.rs crates/llmsim-server/src/main.rs \
    && cargo build --release --package llmsim-server

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary
COPY --from=builder /app/target/release/llmsim-server /usr/local/bin/

# Create non-root user
RUN useradd -m -s /bin/bash llmsim
USER llmsim

# Default configuration
ENV LLMSIM_HOST=0.0.0.0
ENV LLMSIM_PORT=8080

EXPOSE 8080

ENTRYPOINT ["llmsim-server"]
