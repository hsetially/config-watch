#!/usr/bin/env bash
# Build config-watch binaries for Linux inside a Docker container.
# Works from any host OS — no cross-compilation toolchain required.
#
# Usage:
#   bash scripts/build-linux.sh                # build all binaries
#   bash scripts/build-linux.sh config-agent     # build only config-agent
#   bash scripts/build-linux.sh config-control-plane
#
# Artifacts are written to ./dist/ in the project root.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$PROJECT_DIR/dist"

BINARY="${1:-}"

mkdir -p "$DIST_DIR"

if [ -n "$BINARY" ]; then
  IMAGE_NAME="config-watch-build-${BINARY}"
  echo "Building Docker image for binary: $BINARY ..."
  docker build --build-arg BINARY="$BINARY" -t "$IMAGE_NAME" "$PROJECT_DIR"
  echo "Copying binary to $DIST_DIR/ ..."
  docker run --rm -v "$DIST_DIR":/out "$IMAGE_NAME"
else
  IMAGE_NAME="config-watch-build"
  echo "Building Docker image for all binaries ..."
  docker build -t "$IMAGE_NAME" "$PROJECT_DIR"
  echo "Copying binaries to $DIST_DIR/ ..."
  docker run --rm -v "$DIST_DIR":/out "$IMAGE_NAME"
fi

echo "Artifacts in $DIST_DIR/:"
ls -lh "$DIST_DIR/"