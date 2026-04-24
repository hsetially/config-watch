# Multi-stage Dockerfile for building config-watch Linux binaries
# from a Windows host without needing a cross-compilation toolchain.
#
# Usage:
#   # Build all binaries:
#   docker build -t config-watch-build .
#   docker run --rm -v "$(pwd)/dist":/out config-watch-build
#
#   # Build a single binary (faster, skips unused crates):
#   docker build --build-arg BINARY=config-agent -t config-watch-build-agent .
#   docker run --rm -v "$(pwd)/dist":/out config-watch-build-agent
#
# Or via the Makefile / build script:
#   make build-linux-agent
#   bash scripts/build-linux.sh config-agent

# ── Stage 1: Build ────────────────────────────────────────────────────────────
FROM rust:1.82-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .

# Build all workspace binaries in release mode by default.
# Pass --build-arg BINARY=<name> to build only one binary (faster).
ARG BINARY=""
RUN if [ -n "$BINARY" ]; then \
      cargo build --release --bin "$BINARY"; \
    else \
      cargo build --release --bins; \
    fi

# ── Stage 2: Extract ────────────────────────────────────────────────────────
# Copy the release dir and extract only the executable binaries to /out
# so the host can mount a volume and pull them out.

FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y ca-certificates libssl3 \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/ /opt/release/

# Copy only regular files that are executable (the built binaries, not .d files)
# to the mounted /out directory.
CMD ["sh", "-c", "mkdir -p /out && find /opt/release/ -maxdepth 1 -type f -executable -exec cp {} /out/ \\;"]