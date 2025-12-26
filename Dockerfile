# Build stage
FROM rust:1.83-slim-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY llmsim/Cargo.toml ./llmsim/
COPY llmsim-server/Cargo.toml ./llmsim-server/

# Create dummy source files for dependency caching
RUN mkdir -p llmsim/src llmsim-server/src \
    && echo "pub fn dummy() {}" > llmsim/src/lib.rs \
    && echo "fn main() {}" > llmsim-server/src/main.rs

# Build dependencies (this layer is cached)
RUN cargo build --release --package llmsim-server

# Remove dummy source files
RUN rm -rf llmsim/src llmsim-server/src

# Copy actual source code
COPY llmsim/src ./llmsim/src
COPY llmsim-server/src ./llmsim-server/src

# Build the actual application
RUN touch llmsim/src/lib.rs llmsim-server/src/main.rs \
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
