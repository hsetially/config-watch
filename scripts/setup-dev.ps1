# Setup script for Windows (PowerShell)
# Run this once after cloning the repo

# Copy .env.example to .env if .env doesn't exist
if (-not (Test-Path .env)) {
    Copy-Item .env.example .env
    Write-Host "Created .env from .env.example"
} else {
    Write-Host ".env already exists, skipping"
}

# Create fixture directory
if (-not (Test-Path fixtures\yaml)) {
    New-Item -ItemType Directory -Path fixtures\yaml -Force | Out-Null
}

# Create sample YAML if it doesn't exist
if (-not (Test-Path fixtures\yaml\app.yaml)) {
    @"
service:
  name: demo
  replicas: 2
"@ | Set-Content -Path fixtures\yaml\app.yaml
    Write-Host "Created fixtures/yaml/app.yaml"
} else {
    Write-Host "fixtures/yaml/app.yaml already exists, skipping"
}

# Create tmp directories for agent
if (-not (Test-Path tmp\snapshots)) {
    New-Item -ItemType Directory -Path tmp\snapshots -Force | Out-Null
}
if (-not (Test-Path tmp\spool)) {
    New-Item -ItemType Directory -Path tmp\spool -Force | Out-Null
}

# Install difftastic (syntax-aware diff engine used by config-diff)
if (-not (Get-Command difft -ErrorAction SilentlyContinue)) {
    Write-Host "Installing difftastic..."
    cargo install difftastic
} else {
    Write-Host "difftastic already installed"
}

Write-Host ""
Write-Host "Setup complete. Next steps:"
Write-Host "  1. Start Postgres:  docker-compose up -d"
Write-Host "  2. Start control plane:  cargo run -p config-control-plane -- --config deploy/dev/control-plane.toml"
Write-Host "  3. Start agent (separate terminal):  cargo run -p config-agent -- --config deploy/dev/agent.toml"