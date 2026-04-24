# Distributed YAML Configuration Watcher

A Rust workspace for an agent-based YAML configuration change monitoring system. Each VM runs a local agent that watches YAML files, computes syntax-aware diffs, and publishes normalized change events to a central control plane for storage, querying, and realtime streaming.

## Purpose

The system is designed for three outcomes:

- **Detect** configuration changes reliably across many machines.
- **Attribute** changes as accurately as the platform allows.
- **Answer** live and historical questions from one place.

## Architecture

```
┌─────────────────┐         ┌──────────────────────────┐
│  config-agent    │ ──HTTP──▶  config-control-plane     │
│  (per VM)       │         │                          │
│                 │ ◀──HTTP──│  ingest  /v1/events/change│
│  file watcher   │  ═══WS══▶│  hosts   /v1/hosts        │
│  debounce       │ (tunnel) │  query   /v1/changes      │
│  snapshot+diff  │         │  stream  /v1/changes/stream│
│  spool+publish  │         │  metrics /v1/metrics       │
│  query API :9090│         │  workflow /v1/workflows    │
└─────────────────┘         │  file    /v1/file/content  │
                            │  github  /v1/github/file-content│
                            └────────┬─────────────────┘
                                     │
                            ┌────────┼────────┐
                            │        │         │
                            ▼        ▼         ▼
                   ┌────────────┐ ┌──────────────┐
                   │ PostgreSQL │ │  Browser      │
                   │ (migrations)│ │  config-dash  │
                   └────────────┘ │  (WASM/Yew)  │
                                  │  WebSocket   │
                                  │  Stream/History/Compare│
                                  │  Line Numbers│
                                  │  Multi-select│
                                  │  Create PR   │
                                  │  File Compare│
                                  │  GitHub Proxy│
                                  └──────────────┘
```

Agent watches YAML files under configured roots, debounces rapid changes, snapshots file content (BLAKE3 hash), generates syntax-aware diffs (difftastic with line-diff fallback), persists events to a local spool (crash recovery), then publishes to the control plane. The control plane verifies idempotency, stores events, broadcasts to WebSocket subscribers, and exposes REST endpoints for querying.

Agents also maintain a **persistent WebSocket tunnel** (agent-initiated) to the control plane, enabling bidirectional query routing through NAT and firewalls. File-stat, file-preview, and file-content requests are routed through this tunnel first, falling back to direct HTTP if the tunnel is unavailable.

The **workflow system** lets operators select change events from **History mode** in the dashboard and create a pull request: the control plane clones the target repo, resolves each event's previous and current snapshot content, writes the exact event changes into the working tree, commits, pushes a branch, and opens a PR via the GitHub API. Events that already have a PR cannot be re-selected for another workflow.

## Workspace crates

| Crate | Type | Description |
|---|---|---|
| `config-shared` | Library | Domain types: `ChangeEvent`, `HostId`, `SnapshotRef`, `DiffSummary`, `Attribution`, path validation, idempotency keys |
| `config-agent` | Binary | VM-local agent: file watcher (`notify`), debounce window, snapshot pipeline, diff engine, spool-before-publish, query API |
| `config-control-plane` | Binary | Central server: Axum HTTP routes, ingest service, host registry, query service, WebSocket realtime stream, metrics |
| `config-storage` | Library | Postgres persistence: `Database` wrapper, migrations, 7 repository modules (`hosts`, `watch_roots`, `files`, `snapshots`, `change_events`, `file_queries`, `workflows`) |
| `config-transport` | Library | HTTP client with retry, agent query client, WebSocket message types, tunnel protocol, idempotency header helpers |
| `config-diff` | Library | Difftastic syntax-aware diff with configurable output formats (unified, context, full_file, side_by_side, raw), line-diff fallback, diff summary parsing, severity classification |
| `config-snapshot` | Library | BLAKE3 content hashing, disk-based snapshot store with sharded layout, retention enforcement |
| `config-auth` | Library | HMAC-SHA256 credential issuance/verification, enrollment token verification, path security policy (deny `/etc/ssl`, `/etc/ssh`, `private` segments) |
| `config-cli` | Binary | Operator CLI: `hosts`, `tail` (WebSocket), `stat` subcommands |
| `config-workflow` | Library | Workflow executor: clone repo, apply file changes, commit, push branch, create GitHub PR; content resolver with snapshot store fallback |
| `config-dashboard` | Binary (WASM) | Browser dashboard: Yew/WASM UI with real-time WebSocket subscription, `similar`-based word-level diff viewer with scroll/collapse, multi-select events, multi-agent and GitHub file comparison, and Create PR workflow panel |

### Module map by crate

**`config-shared`** — `ids` (typed UUIDs), `events` (ChangeEvent, ChangeKind, Severity), `attribution` (author attribution with confidence), `snapshots` (SnapshotRef, DiffSummary, CompressionKind), `errors` (AppError with HTTP status mapping), `paths` (normalize, canonicalize, is_yaml), `validation` (derive_idempotency_key), `queries` (FileMetadataQuery)

**`config-agent`** — `watcher` (RawWatchEvent → notify), `debounce` (DebounceWindow collapsing bursts), `pipeline` (SnapshotDecision, diff_generate, build_change_event), `spool` (SpoolWriter: append/deliver/fail/replay), `publish` (ControlPlaneClient retry), `query_handler` (stat/preview with redaction), `redaction` (regex-based secret masking), `attribution` (filesystem metadata hints), `tunnel` (AgentTunnel: persistent WebSocket to control plane, exponential backoff reconnection, query dispatch)

**`config-control-plane`** — `http/routes` (16 REST endpoints + WebSocket), `http/extractors` (AgentAuth HMAC, OperatorAuth bearer, CorrelationId, Pagination, ChangeFilters), `http/middleware` (CORS, 1 MB body limit, tracing), `ingest` (IngestService: schema validation, idempotency check, DB insert, broadcast), `registry` (derive_host_status), `query` (paginated list, detail), `realtime` (SubscriptionFilter matching), `tunnel` (AgentRegistry: WebSocket tunnel, DashMap-based connection tracking, oneshot query correlation), `metrics` (atomic counters including tunnel metrics)

**`config-storage`** — `db` (PgPool wrapper, migrations), `models` (HostRow, WatchRootRow, FileRow, SnapshotRow, ChangeEventRow with canonical_path, FileQueryRow, WorkflowRow), `repositories/*` (CRUD repos including workflows), `tx` (transaction helper)

**`config-transport`** — `client` (register, heartbeat, publish_change with exponential backoff), `agent_query` (stat/preview proxy), `websocket` (WsMessage, RealtimeMessage), `tunnel` (TunnelMessage protocol: QueryRequest/QueryResponse/Ping/Pong, oneshot correlation), `idempotency` (header generation/parsing)

**`config-auth`** — `tokens` (AgentCredential HMAC-SHA256 issue/verify), `enrollment` (EnrollmentVerifier), `policy` (is_path_denied, is_path_allowed)

**`config-diff`** — `difftastic` (DiffEngine: difftastic binary detection, DiffConfig with 5 output formats, fallback to line diff, diff render output), `summary` (parse_diff_summary, classify_severity)

**`config-dashboard`** — `app` (root component, tri-mode state: Stream/History/Compare, event buffers, selection state, layout), `models` (RealtimeMessage, HostInfo, ChangeEventRow, DiffSummary, FilterState, ViewMode, WatchRootInfo, DiffLine, WordSegment, DiffLineKind, ColumnSource, CompareColumn, CompareResult, GitHubFileContentResponse, WorkflowCreateRequest, WorkflowStatusRow, severity/event_kind helpers), `api` (REST clients: fetch_hosts, fetch_changes, fetch_event_detail, fetch_watch_roots, fetch_file_content, fetch_github_file_content, create_workflow, get_workflow via gloo-net HTTP), `ws` (WebSocket client via gloo-net, connects to `/v1/changes/stream` with filter query params), `storage` (localStorage persistence for events), `components/diff_viewer` (`similar`-based word-level diff renderer with line numbers in gutter for unified/context/full_file modes, color-coded added/removed/hunk/header lines, scroll container with expand/collapse toggle), `components/event_list` (event feed with expandable cards, lazy diff fetch, multi-select checkboxes), `components/filters` (Stream/History/Compare mode toggle, host dropdown, path prefix with watch roots datalist, filename search, severity filter), `components/file_compare` (multi-column file comparison: agent or GitHub source per column, per-column file path inputs, `similar`-based side-by-side diff with word-level highlights), `components/selection_bar` (bottom bar with selection count, Create PR button — **History mode only**), `components/workflow_panel` (slide-in panel: repo/branch/PR form, GitHub token, submit, status polling, auto-refreshes history on success)

**`config-workflow`** — `models` (WorkflowStatus state machine, FileChange, PathMapping, WorkflowRun), `git_ops` (clone with token auth, apply changes, commit, push via git2), `github_client` (create PR via REST API, parse_owner_repo, fetch_file_contents via GitHub Contents API, parse_github_blob_url), `content_resolver` (trait: resolve by content_hash from snapshot store), `executor` (async state machine: pending → cloning → applying → committing → pushing → creating_pr → completed/failed, DB status updates)

**`config-snapshot`** — `hash` (BLAKE3 compute), `store` (SnapshotStore: read/write sharded files, current_state.json), `retention` (enforce_retention with max_snapshots_per_file, max_total_bytes, max_age_days)

## Prerequisites

- **Rust** 1.78+ (`rustup.rs` or your distro's toolchain)
- **Docker** (for Postgres)
- **difftastic** (installed automatically by `setup-dev.ps1`; or run `cargo install difftastic`)
- **wasm32-unknown-unknown** target (for web dashboard: `rustup target add wasm32-unknown-unknown`)
- **Trunk** (for web dashboard: `cargo install trunk`)

## Quick start

> **Windows users:** `make` is not available by default. Use the PowerShell setup script and `cargo` commands directly (shown below).

```bash
# 1. Clone and enter the repo
git clone <repo-url> && cd config-watch

# 2. Set up environment
#    Linux/macOS:
make setup-dev
#    Windows PowerShell:
pwsh scripts/setup-dev.ps1

# 3. Start Postgres
docker-compose up -d

# 4. Seed sample YAML files (agent watches ./fixtures/yaml/)
bash scripts/seed-dev.sh
#    Windows PowerShell:
pwsh scripts/setup-dev.ps1   # already creates fixtures/yaml/app.yaml

# 5. Start the control plane (port 8082)
#    Linux/macOS:
make run-control
#    Windows or direct:
cargo run -p config-control-plane -- --config deploy/dev/control-plane.toml

# 6. Start the agent (port 9090) in another terminal
#    Linux/macOS:
make run-agent
#    Windows or direct:
cargo run -p config-agent -- --config deploy/dev/agent.toml
```

**7. Start the web dashboard (optional)**

```bash
# One-time setup
rustup target add wasm32-unknown-unknown
cargo install trunk

# Build and serve on http://localhost:3000
cd web/dashboard && trunk serve
```

Open `http://localhost:3000`, set the Server field to your control plane address (default: `localhost:8082`), then:
- **Stream mode** — click **Connect** to subscribe to live WebSocket events with inline line numbers
- **History mode** — click **Fetch** to load stored events from the database. Select events with checkboxes and click **Create PR** to open the workflow panel. Events that already have a PR are disabled for selection
- **Compare mode** — select **Compare** from the mode toggle. Add columns from different agents or paste a GitHub blob URL to compare files side-by-side with word-level diff highlights. Each column has its own file path input
- **Create PR workflow** (History mode only) — select events, click **Create PR**, fill in repo URL, branch name, PR title/description, and GitHub token. The control plane clones the repo, resolves previous/current snapshots for each event to produce clean per-event diffs, commits, pushes a branch, and opens a GitHub PR. The history list auto-refreshes after a successful PR so events show their PR link

### Download pre-built binaries from CI

Every push and pull request triggers a GitHub Actions workflow that builds release binaries for Linux and Windows. You can download these without installing Rust:

1. Go to **Actions** → **CI** → select the latest successful run
2. Scroll to **Artifacts** and download:
   - `binaries-ubuntu-latest` — Linux x86_64 binaries
   - `binaries-windows-latest` — Windows x86_64 binaries
   - `config-dashboard` — Static web dashboard files

See [docs/11-download-from-ci.md](docs/11-download-from-ci.md) for detailed screenshots and steps.

Select a host from the dropdown (click reload to fetch registered hosts). See [docs/10-web-dashboard.md](docs/10-web-dashboard.md) for full documentation.

The control plane listens on `127.0.0.1:8082` and auto-migrates the database on startup. The agent listens on `0.0.0.0:9090` for query requests and heartbeats to the control plane every 30 seconds.

## Remote deployment (local control plane + cloud agents)

You can run the control plane on your local machine and agents on remote VMs by using **Cloudflare Tunnel** to expose the control plane to the internet. The tunnel direction is inbound-only: cloud agents make outbound HTTP requests to the tunnel URL, which works through any NAT or firewall.

### Setup

**1. Start the control plane locally**

```bash
# Start Postgres
docker compose up -d

# Start the control plane
cargo run -p config-control-plane -- --config deploy/dev/control-plane.toml
```

**2. Start a Cloudflare Tunnel**

```bash
# Install cloudflared (one-time): https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/

# Start a quick tunnel (no account needed)
cloudflared tunnel --url http://localhost:8082
```

This outputs a public URL like `https://random-words.trycloudflare.com`. The URL changes each restart. For a stable URL, [create a named tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/configure-tunnels/).

**3. Configure the remote agent**

On the remote VM (e.g., GCP, AWS, Azure), create an agent config:

```toml
# agent.toml
agent_id = "<unique-uuid>"
environment = "production"
control_plane_base_url = "https://random-words.trycloudflare.com"
enrollment_token = "dev-secret-change-me"

[[watch_roots]]
root_path = "/etc/myapp"
recursive = true
```

Or override via environment variable:

```bash
export CONFIG_WATCH_CONTROL_PLANE_BASE_URL="https://random-words.trycloudflare.com"
```

**4. Build and run the agent on the remote VM**

```bash
cargo build --release --bin config-agent
./target/release/config-agent --config agent.toml
```

### Architecture

```
┌──────────────────┐                  ┌─────────────────────┐
│  Local PC         │                  │  Cloudflare Edge    │
│                   │                  │                     │
│  config-control-  │◄─── tunnel ──────│  https://xxx.try-   │
│  plane :8082      │                  │  cloudflare.com     │
│                   │                  └─────────┬───────────┘
│  PostgreSQL       │                            │
└──────────────────┘                            │
                                      ┌─────────▼───────────┐
                                      │  GCP / AWS / Azure   │
                                      │                     │
                                      │  config-agent :9090 │
                                      │  (outbound HTTPS)   │
                                      └─────────────────────┘
```

### Current limitations

- **File proxy queries** (`/v1/file/stat`, `/v1/file/preview`, `/v1/file/content`) — Routed through the agent WebSocket tunnel first, falling back to direct HTTP if the tunnel is disconnected. Direct HTTP (`hostname:9090`) still won't resolve from a local PC to a cloud VM without the tunnel.
- **Tunnel URL changes on restart** — Quick tunnels get a new URL each time. Use a named Cloudflare tunnel for a stable URL in production.
- **WebSocket** — Cloudflare Tunnel supports WebSocket, so `/v1/changes/stream` works through the tunnel (`wss://`).
- **Workflow content resolution** — The Create PR workflow resolves file content from the snapshot store by content hash. If no snapshot exists (e.g., the event has no content_hash), the file cannot be included in the PR.

### Security considerations

- The control plane now accepts CORS requests from any origin (required for unpredictable tunnel URLs). In production, restrict `allow_origin` to your known tunnel domain.
- All agent-to-control-plane communication is authenticated (enrollment token + HMAC credentials).
- Cloudflare Tunnel terminates TLS at their edge and forwards plain HTTP to your local control plane. The tunnel itself is encrypted end-to-end between the agent and Cloudflare.

### Verify it's running

```bash
# List registered hosts
curl http://localhost:8082/v1/hosts

# Check metrics
curl http://localhost:8082/v1/metrics

# Watch realtime changes (WebSocket)
# Use config-cli or wscat:
#   config-cli tail --env development
```

> **Windows PowerShell** — use `Invoke-WebRequest` instead of `curl`:
> ```powershell
> # List registered hosts
> (Invoke-WebRequest -Uri http://localhost:8082/v1/hosts).Content
>
> # Check metrics
> (Invoke-WebRequest -Uri http://localhost:8082/v1/metrics).Content
> ```

### Verify the control plane received a change event

After editing a watched YAML file (e.g., `fixtures/yaml/app.yaml`), the agent detects the change, diffs it, and POSTs to the control plane. Here's how to verify the event landed:

**1. Check via the REST API**

```bash
# List recent change events
curl -s http://localhost:8082/v1/changes | python3 -m json.tool

# Filter by path prefix
curl -s "http://localhost:8082/v1/changes?path_prefix=fixtures/yaml" | python3 -m json.tool

# Filter by severity
curl -s "http://localhost:8082/v1/changes?severity=warning" | python3 -m json.tool

# Get a specific event by ID (includes diff_render with actual diff text)
curl -s http://localhost:8082/v1/changes/<event_id> | python3 -m json.tool
```

```powershell
# Windows PowerShell — list recent changes
(Invoke-WebRequest -Uri http://localhost:8082/v1/changes).Content | ConvertFrom-Json | ConvertTo-Json -Depth 5

# Filter by path prefix
(Invoke-WebRequest -Uri "http://localhost:8082/v1/changes?path_prefix=fixtures/yaml").Content
```

**2. Check via the database directly**

```bash
# Connect to Postgres (password: postgres)
psql -h localhost -U postgres -d config_watch

# List the 10 most recent change events
SELECT event_id, event_kind, canonical_path, severity, event_time, idempotency_key
FROM change_events ORDER BY event_time DESC LIMIT 10;

# See full event details including diff summary, file path, and actual diff text
SELECT event_id, event_kind, canonical_path, severity, diff_summary_json, diff_render, author_name, author_confidence
FROM change_events ORDER BY event_time DESC LIMIT 5;

# Show just the diff content for a specific event
SELECT diff_render FROM change_events WHERE event_id = '<event_id>';

# Count events by kind
SELECT event_kind, count(*) FROM change_events GROUP BY event_kind;

# Check registered hosts and their status
SELECT host_id, hostname, status, last_heartbeat_at FROM hosts;

# Exit psql
\q
```

```powershell
# Windows PowerShell — connect via docker
docker exec -it config-watch-postgres-1 psql -U postgres -d config_watch

# Then run the same SQL queries above inside the psql session
# Or use a single command:
docker exec -it config-watch-postgres-1 psql -U postgres -d config_watch -c `
  "SELECT event_id, event_kind, canonical_path, severity, diff_render FROM change_events ORDER BY event_time DESC LIMIT 10;"
```

**3. Stream changes in real time**

```bash
# Using config-cli (WebSocket client)
cargo run -p config-cli -- tail --env development

# Show diff content alongside each event
cargo run -p config-cli -- tail --env development --diff

# Or with the web dashboard (browser-based WASM UI)
cd web/dashboard && trunk serve
# Opens at http://localhost:3000 — connect and see live diffs

# Or with wscat
wscat -c ws://localhost:8082/v1/changes/stream
```

**4. Check agent spool status**

If events aren't reaching the control plane, check the agent's local spool:

```bash
# Pending events (not yet delivered)
ls tmp/spool/pending/

# Delivered events
ls tmp/spool/delivered/

# Failed events
ls tmp/spool/failed/
```

A pending event that never moves to `delivered/` indicates a connectivity or auth issue between agent and control plane.

## Development

### Build and lint

```bash
# Linux/macOS (via Makefile)           # Windows / direct cargo
make fmt                              # cargo fmt --all
make lint                             # cargo clippy --workspace --all-targets -- -D warnings
make test                             # cargo test --workspace
```

### Building Linux binaries from Windows

Cross-compiling from Windows to Linux requires a Linux cross-compilation toolchain (gcc, OpenSSL headers, etc.) that is cumbersome to set up. Instead, use Docker to build inside a Linux container — no cross-compilation toolchain needed on the host.

**Prerequisites:** Docker must be running.

```bash
# Build all Linux binaries (agent, control-plane, CLI)
make build-linux

# Build a single binary
make build-linux-agent
make build-linux-control
make build-linux-cli

# Or use the script directly
bash scripts/build-linux.sh                # all binaries
bash scripts/build-linux.sh config-agent    # single binary
```

Artifacts are written to `./dist/` in the project root. These are statically linked against the Linux OpenSSL built inside the container.

You can also run the Docker commands directly:

```bash
# Build the image
docker build -t config-watch-build .

# Extract all binaries to ./dist/
docker run --rm -v "$(pwd)/dist":/out config-watch-build

# Build only config-agent (faster, skips unused crates)
docker run --rm -e BINARY=config-agent -v "$(pwd)/dist":/out config-watch-build
```

> **Windows PowerShell** — replace `$(pwd)` with `${PWD}`:
> ```powershell
> docker run --rm -v "${PWD}/dist":/out config-watch-build
> ```

### Web dashboard (WASM)

```bash
# Linux/macOS (via Makefile)           # Windows / direct
make dashboard-serve                   # cd web/dashboard && trunk serve
make dashboard-build                   # cd web/dashboard && trunk build --release
```

The dashboard compiles Rust to WebAssembly via Yew. Development builds serve at `http://localhost:3000` with hot-reload on source/CSS changes. Production builds output to `web/dashboard/dist/`.

### Running tests

Most tests run without external dependencies. Tests that require Postgres use the `DATABASE_URL` environment variable (defaults to `postgres://postgres:postgres@localhost:5432/config_watch_test`).

```bash
# Unit and logic tests (no DB required)
cargo test --workspace --lib

# Contract tests for control-plane routes (requires DB)
DATABASE_URL=postgres://postgres:postgres@localhost:5432/config_watch_test \
  cargo test -p config-control-plane --test contract_control_plane

# Contract tests for agent routes (no DB, uses temp directories)
cargo test -p config-agent --test contract_agent_routes

# Extractor tests (no DB)
cargo test -p config-control-plane --test contract_extractors

# Integration tests (some require DB)
cargo test -p config-auth --test integration_auth           # No DB
cargo test -p config-agent --test integration_pipeline       # No DB
cargo test -p config-agent --test integration_spool          # No DB
cargo test -p config-control-plane --test integration_subscription_filter  # No DB
cargo test -p config-control-plane --test integration_realtime             # No DB
cargo test -p config-storage --test integration_derive_host_status         # No DB

# E2E tests (require DB)
DATABASE_URL=postgres://postgres:postgres@localhost:5432/config_watch_test \
  cargo test -p config-control-plane --test e2e_agent_to_control_plane
```

### Test structure

```
tests/                          # Integration/e2e tests (legacy, mostly empty)
crates/*/tests/                 # Per-crate integration and contract tests
crates/*/src/**/tests.rs         # Inline unit tests (53 total)
```

| Tier | Count | What they cover | DB needed |
|---|---|---|---|
| Unit | 53 | Pure logic: hashing, validation, debounce, spool, auth tokens, policy, diff summary, retention defaults | No |
| Contract | 28 | HTTP request/response shapes: all 13 routes, auth extractors, status codes, error messages | Yes (control-plane) |
| Integration | 42 | Cross-crate interactions: ingest idempotency, pipeline stages, spool lifecycle, subscription filter, realtime broadcast, host status, auth flow, repo queries | Some |
| E2E | 10 | Full flow: register → ingest → query → WebSocket → file forwarding | Yes |

### Known issue

None currently. The previously documented `path_prefix` filter bug has been fixed — it now uses the `canonical_path` column directly with `$N` parameter syntax.

## Configuration

### Control plane (`deploy/dev/control-plane.toml`)

| Key | Default | Description |
|---|---|---|
| `bind_addr` | `127.0.0.1:8082` | Listen address |
| `database_url` | — | Postgres connection string |
| `control_plane_secret` | `dev-secret-change-me` | HMAC key for agent credentials |
| `query_timeout_secs` | `10` | Timeout for agent tunnel queries |
| `snapshot_data_dir` | `./data/snapshots` | Directory for content snapshots (used by workflow content resolver) |
| `repos_dir` | `./data/repos` | Directory for cloned git repositories (used by workflow executor) |
| `github_token` | — | GitHub personal access token for file-content proxy and workflow PR creation (optional for public repos) |

All keys support `CONFIG_WATCH_*` environment variable overrides (e.g., `CONFIG_WATCH_DATABASE_URL`).

### Agent (`deploy/dev/agent.toml`)

| Key | Default | Description |
|---|---|---|
| `agent_id` | — | UUID identifying this agent instance |
| `environment` | `default` | Environment label (prod, staging, etc.) |
| `control_plane_base_url` | — | Control plane URL |
| `watch_roots` | — | Array of `{ root_path, recursive }` |
| `debounce_window_ms` | `500` | Burst suppression window |
| `snapshot_dir` | — | Directory for content snapshots |
| `spool_dir` | — | Directory for outbound event spool |
| `heartbeat_interval_secs` | `30` | Heartbeat frequency |
| `content_preview_max_bytes` | `4096` | Max preview size |
| `redaction_patterns` | `[(?i)(token\|secret\|password\|key\|credential)]` | Regex patterns for value redaction |
| `diff.format` | `unified` | Diff output format: `unified`, `context`, `full_file`, `side_by_side`, or `raw` |
| `diff.context_lines` | `3` | Context lines around changes (only for `context` format) |
| `diff.side_by_side_width` | `120` | Column width for side-by-side format |
| `tunnel_enabled` | `true` | Enable persistent WebSocket tunnel to control plane |
| `tunnel_reconnect_base_secs` | `1` | Base delay for exponential backoff reconnection |
| `tunnel_reconnect_max_secs` | `30` | Maximum delay between reconnection attempts |

## API reference

### Control plane (port 8082)

| Method | Path | Auth | Description |
|---|---|---|---|
| POST | `/v1/agents/register` | Enrollment token | Register host, receive HMAC credential |
| POST | `/v1/agents/heartbeat` | Agent token | Update heartbeat timestamp |
| GET | `/v1/agents/tunnel` | Agent token | WebSocket tunnel for bidirectional query routing |
| POST | `/v1/events/change` | Agent token | Ingest change event (idempotent) |
| GET | `/v1/hosts` | — | List hosts (paginated) |
| GET | `/v1/hosts/{host_id}` | — | Get host detail |
| GET | `/v1/hosts/{host_id}/roots` | — | List watch roots for a host |
| GET | `/v1/changes` | — | List change events (filtered, paginated; includes `canonical_path`) |
| GET | `/v1/changes/{event_id}` | — | Get change event detail (includes `diff_render` and `canonical_path`) |
| GET | `/v1/changes/stream` | — | WebSocket realtime stream |
| POST | `/v1/file/content` | — | Proxy file content fetch to agent (tunnel-first, HTTP fallback) |
| POST | `/v1/file/stat` | — | Proxy file stat query to agent (tunnel-first, HTTP fallback) |
| POST | `/v1/file/preview` | — | Proxy file preview query to agent (tunnel-first, HTTP fallback) |
| POST | `/v1/github/file-content` | — | Fetch file contents from GitHub (server-side proxy, uses `github_token` from config) |
| POST | `/v1/workflows` | — | Create workflow (returns 202 + workflow_id, runs in background) |
| GET | `/v1/workflows/{workflow_id}` | — | Get workflow status |
| GET | `/v1/workflows` | — | List workflows (paginated) |
| GET | `/v1/metrics` | — | Atomic counter snapshot |

### Agent query API (port 9090)

| Method | Path | Description |
|---|---|---|
| POST | `/v1/query/file-metadata` | Return file size, hash, is_yaml, permissions |
| POST | `/v1/query/file-preview` | Return redacted file content |

## Data flow

```
1. File changes → FileWatcher (notify) → RawWatchEvent
2. DebounceWindow collapses bursts → DebouncedEvent
3. Pipeline.snapshot_acquire (BLAKE3 hash, compare to stored)
   → Unchanged (suppressed) | Changed | FileCreated | FileDeleted
4. Pipeline.diff_generate (difftastic or line-diff fallback)
5. Pipeline.enrich_attribution (filesystem metadata hints)
6. SpoolWriter.append (persist to disk before network)
7. EventPublisher.publish (POST /v1/events/change with idempotency key)
8. Control plane: schema validate → idempotency check → insert (with `canonical_path`) → broadcast → heartbeat update
9. AgentTunnel: persistent WebSocket to control plane (agent-initiated, traverses NAT)
10. Query routing: control plane → tunnel → agent (stat/preview/content), HTTP fallback if tunnel down
11. GitHub proxy: dashboard → POST /v1/github/file-content → control plane → GitHub Contents API (server-side token, CORS-free)
12. File comparison: dashboard fetches content from agent(s) and/or GitHub → similar-based line + word diff → side-by-side rendered
13. Workflow: dashboard multi-select → POST /v1/workflows → clone → apply → commit → push → GitHub PR
```

## Security model

- **Agent authentication**: HMAC-SHA256 credentials issued on registration. Token format: `{host_id}|{expires_utc}|{hmac_hex}`. Verified via `X-Agent-Token` header.
- **Enrollment**: New agents present an enrollment token (`X-Enrollment-Token`) matching the control plane secret.
- **Path deny list**: `/etc/ssl`, `/etc/ssh`, and any path containing `private` are always denied, regardless of watch roots.
- **Content redaction**: File previews mask values matching `(?i)(token|secret|password|key|credential)` patterns with `[REDACTED]`.
- **Request body limit**: 1 MB on control plane endpoints.

## Core product principles

1. **Every raw file event is not a business event.** Debounce and normalize before storage.
2. **Authorship is probabilistic.** Store attribution source and confidence, not fake certainty.
3. **History is append-only.** Never mutate audit records after ingest.
4. **Realtime comes after durability.** Persist first, then fan out to subscribers.
5. **Remote access is scoped.** This is an observability system, not a general-purpose file explorer.

## In scope

- Linux VMs only (agents)
- Remote agents connecting via Cloudflare Tunnel or direct network
- Agent-to-control-plane WebSocket tunnel (NAT/firewall traversal)
- Agent installed on each VM
- Recursive watching of configured parent directories
- YAML-only change tracking (`.yaml`, `.yml`)
- Snapshotting and syntax-aware diff generation
- Central event ingest and realtime subscriptions
- Metadata and content-preview queries (tunnel-first, HTTP fallback)
- Best-effort author attribution with confidence score
- Multi-select change events in dashboard
- Multi-column file comparison (agent and GitHub sources, `similar`-based word-level diff)
- GitHub file content proxy (server-side, avoids CORS, keeps token out of browser)
- Create PR workflow (clone, apply, commit, push, open GitHub PR)

## Out of scope for v1

- Windows/macOS agents
- Full Git correlation (beyond the Create PR workflow)
- Full-blown approval workflows
- Arbitrary remote shell execution
- Secret material retrieval without explicit authorization
- Deep semantic policy validation of YAML contents