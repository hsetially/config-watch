# Quick database query script for Windows
# Usage:
#   pwsh scripts/query-db.ps1                    # Interactive psql session
#   pwsh scripts/query-db.ps1 -Query "SELECT * FROM change_events ORDER BY event_time DESC LIMIT 10"
#   pwsh scripts/query-db.ps1 -Action events     # Recent change events
#   pwsh scripts/query-db.ps1 -Action hosts      # Registered hosts
#   pwsh scripts/query-db.ps1 -Action counts     # Event counts by kind

param(
    [string]$Query,
    [ValidateSet("events","hosts","counts","interactive")]
    [string]$Action = "interactive"
)

$DbUrl = if ($env:DATABASE_URL) { $env:DATABASE_URL } else { "postgres://postgres:postgres@localhost:5432/config_watch" }

# Try psql first (if on PATH via Postgres install), fall back to docker
$PsqlAvailable = Get-Command psql -ErrorAction SilentlyContinue

function Invoke-DbQuery {
    param([string]$Sql)
    if ($PsqlAvailable) {
        psql $DbUrl -c $Sql
    } else {
        $Container = docker ps --filter "ancestor=postgres:16" --format "{{.Names}}" | Select-Object -First 1
        if (-not $Container) {
            $Container = docker ps --filter "name=postgres" --format "{{.Names}}" | Select-Object -First 1
        }
        if ($Container) {
            docker exec -it $Container psql -U postgres -d config_watch -c $Sql
        } else {
            Write-Host "ERROR: No psql on PATH and no running Postgres container found." -ForegroundColor Red
            Write-Host "Start the database first: docker-compose up -d" -ForegroundColor Yellow
            exit 1
        }
    }
}

if ($Query) {
    Invoke-DbQuery $Query
} else {
    switch ($Action) {
        "events" {
            Invoke-DbQuery "SELECT event_id, event_kind, severity, event_time, idempotency_key FROM change_events ORDER BY event_time DESC LIMIT 10;"
        }
        "hosts" {
            Invoke-DbQuery "SELECT host_id, hostname, status, last_heartbeat_at FROM hosts;"
        }
        "counts" {
            Invoke-DbQuery "SELECT event_kind, count(*) FROM change_events GROUP BY event_kind;"
        }
        "interactive" {
            if ($PsqlAvailable) {
                psql $DbUrl
            } else {
                $Container = docker ps --filter "ancestor=postgres:16" --format "{{.Names}}" | Select-Object -First 1
                if (-not $Container) {
                    $Container = docker ps --filter "name=postgres" --format "{{.Names}}" | Select-Object -First 1
                }
                if ($Container) {
                    docker exec -it $Container psql -U postgres -d config_watch
                } else {
                    Write-Host "ERROR: No psql on PATH and no running Postgres container found." -ForegroundColor Red
                    Write-Host "Start the database first: docker-compose up -d" -ForegroundColor Yellow
                    exit 1
                }
            }
        }
    }
}