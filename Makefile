# Makefile for config-watch
# On Windows without `make`, use the direct cargo commands instead:
#   cargo fmt --all
#   cargo clippy --workspace --all-targets -- -D warnings
#   cargo test --workspace
#   cargo run -p config-control-plane -- --config deploy/dev/control-plane.toml
#   cargo run -p config-agent -- --config deploy/dev/agent.toml
# Or run: pwsh scripts/setup-dev.ps1

.PHONY: fmt lint test run-agent run-control setup-dev install-difftastic db psql dashboard dashboard-serve dashboard-build build-linux build-linux-agent build-linux-control build-linux-cli

fmt:
	cargo fmt --all

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

run-agent:
	cargo run -p config-agent -- --config deploy/dev/agent.toml

run-control:
	cargo run -p config-control-plane -- --config deploy/dev/control-plane.toml

setup-dev:
	cp -n .env.example .env 2>/dev/null || copy .env.example .env 2>nul || true

install-difftastic:
	cargo install difftastic

# Connect to the dev Postgres database
psql:
	psql -h localhost -U postgres -d config_watch

db:
	psql -h localhost -U postgres -d config_watch

# WASM dashboard (requires trunk: cargo install trunk)
dashboard-serve:
	cd web/dashboard && trunk serve

dashboard-build:
	cd web/dashboard && trunk build --release

# ── Docker-based Linux builds (no cross-compilation toolchain needed) ─────
# Requires Docker. Artifacts are placed in ./dist/
#   make build-linux          — all binaries
#   make build-linux-agent    — config-agent only
#   make build-linux-control  — config-control-plane only
#   make build-linux-cli      — config-cli only

build-linux:
	bash scripts/build-linux.sh

build-linux-agent:
	bash scripts/build-linux.sh config-agent

build-linux-control:
	bash scripts/build-linux.sh config-control-plane

build-linux-cli:
	bash scripts/build-linux.sh config-cli