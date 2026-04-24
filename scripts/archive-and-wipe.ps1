# Archive the current database to a timestamped file, then truncate all tables.
# Usage:
#   pwsh scripts/archive-and-wipe.ps1
#   pwsh scripts/archive-and-wipe.ps1 -DbUrl "postgres://user:pass@host:5432/db"
# Default DB URL: postgres://postgres:postgres@localhost:5432/config_watch

param(
    [string]$DbUrl = "postgres://postgres:postgres@localhost:5432/config_watch"
)

$ErrorActionPreference = "Stop"

# Try psql first (if on PATH via Postgres install), fall back to docker
$PsqlAvailable = Get-Command psql -ErrorAction SilentlyContinue
$PgDumpAvailable = Get-Command pg_dump -ErrorAction SilentlyContinue

function Find-Container {
    $Container = docker ps --filter "ancestor=postgres:16" --format "{{.Names}}" | Select-Object -First 1
    if (-not $Container) {
        $Container = docker ps --filter "name=postgres" --format "{{.Names}}" | Select-Object -First 1
    }
    if (-not $Container) {
        Write-Host "ERROR: No psql on PATH and no running Postgres container found." -ForegroundColor Red
        Write-Host "Start the database first: docker-compose up -d" -ForegroundColor Yellow
        exit 1
    }
    $Container
}

function Invoke-DbQuery {
    param([string]$Sql)
    if ($PsqlAvailable) {
        psql $DbUrl -t -A -c $Sql
    } else {
        $Container = Find-Container
        docker exec $Container psql -U postgres -d config_watch -t -A -c $Sql
    }
}

function Invoke-DbCommand {
    param([string]$Sql)
    if ($PsqlAvailable) {
        psql $DbUrl -c $Sql
    } else {
        $Container = Find-Container
        docker exec $Container psql -U postgres -d config_watch -c $Sql
    }
}

# --- Setup ---
$ProjectDir = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$ArchiveDir = Join-Path $ProjectDir "archives"
if (-not (Test-Path $ArchiveDir)) {
    New-Item -ItemType Directory -Path $ArchiveDir | Out-Null
}

$Timestamp = Get-Date -Format "yyyyMMdd_HHmmss"
$ArchiveFile = Join-Path $ArchiveDir "archive_$Timestamp.sql.gz"

Write-Host "=== Config-Watch DB Archive & Wipe ===" -ForegroundColor Cyan
Write-Host ""

# --- Record counts before ---
Write-Host "Current record counts:"
$CountQuery = @"
SELECT '  workflows', COUNT(*) FROM workflows
UNION ALL SELECT '  file_queries', COUNT(*) FROM file_queries
UNION ALL SELECT '  change_events', COUNT(*) FROM change_events
UNION ALL SELECT '  files', COUNT(*) FROM files
UNION ALL SELECT '  watch_roots', COUNT(*) FROM watch_roots
UNION ALL SELECT '  snapshots', COUNT(*) FROM snapshots
UNION ALL SELECT '  hosts', COUNT(*) FROM hosts;
"@
Invoke-DbQuery $CountQuery
Write-Host ""

# --- Archive ---
Write-Host "Archiving database to: $ArchiveFile"
if ($PsqlAvailable -and $PgDumpAvailable) {
    pg_dump $DbUrl | gzip > $ArchiveFile
} else {
    $Container = Find-Container
    # pg_dump inside container, pipe through gzip on host
    docker exec $Container pg_dump -U postgres -d config_watch | gzip > $ArchiveFile
}

if (-not (Test-Path $ArchiveFile) -or (Get-Item $ArchiveFile).Length -eq 0) {
    Write-Host "ERROR: Archive file was not created or is empty." -ForegroundColor Red
    exit 1
}

$ArchiveSize = "{0:N1} MB" -f ((Get-Item $ArchiveFile).Length / 1MB)
Write-Host "Archive created: $ArchiveSize" -ForegroundColor Green
Write-Host ""

# --- Confirm ---
Write-Host "WARNING: This will DELETE ALL RECORDS from every table." -ForegroundColor Yellow
$Confirm = Read-Host "Type 'yes' to proceed"
if ($Confirm -ne "yes") {
    Write-Host "Aborted." -ForegroundColor Yellow
    exit 0
}

# --- Truncate ---
Write-Host "Truncating all tables..."
Invoke-DbCommand "TRUNCATE workflows, file_queries, change_events, files, watch_roots, snapshots, hosts CASCADE;"
Write-Host ""

# --- Record counts after ---
Write-Host "Record counts after wipe:"
Invoke-DbQuery $CountQuery
Write-Host ""
Write-Host "Done. Archive saved at: $ArchiveFile" -ForegroundColor Green
Write-Host "To restore: gunzip -c $ArchiveFile | psql `"$DbUrl`""