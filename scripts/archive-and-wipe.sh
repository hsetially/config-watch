#!/usr/bin/env bash
set -euo pipefail

# Archive the current database to a timestamped file, then truncate all tables.
# Usage: scripts/archive-and-wipe.sh [--db-url DB_URL]
# Default DB URL: postgres://postgres:postgres@localhost:5432/config_watch

DB_URL="postgres://postgres:postgres@localhost:5432/config_watch"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --db-url)
            DB_URL="$2"
            shift 2
            ;;
        *)
            echo "Unknown argument: $1" >&2
            echo "Usage: $0 [--db-url DB_URL]" >&2
            exit 1
            ;;
    esac
done

ARCHIVE_DIR="$(cd "$(dirname "$0")/.." && pwd)/archives"
mkdir -p "$ARCHIVE_DIR"

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
ARCHIVE_FILE="$ARCHIVE_DIR/archive_${TIMESTAMP}.sql.gz"

echo "=== Config-Watch DB Archive & Wipe ==="
echo ""

# --- Record counts before ---
echo "Current record counts:"
psql "$DB_URL" -t -A -c "
SELECT '  workflows', COUNT(*) FROM workflows
UNION ALL SELECT '  file_queries', COUNT(*) FROM file_queries
UNION ALL SELECT '  change_events', COUNT(*) FROM change_events
UNION ALL SELECT '  files', COUNT(*) FROM files
UNION ALL SELECT '  watch_roots', COUNT(*) FROM watch_roots
UNION ALL SELECT '  snapshots', COUNT(*) FROM snapshots
UNION ALL SELECT '  hosts', COUNT(*) FROM hosts;
"
echo ""

# --- Archive ---
echo "Archiving database to: $ARCHIVE_FILE"
pg_dump "$DB_URL" | gzip > "$ARCHIVE_FILE"
ARCHIVE_SIZE=$(du -h "$ARCHIVE_FILE" | cut -f1)
echo "Archive created: $ARCHIVE_SIZE"
echo ""

# --- Confirm ---
echo "WARNING: This will DELETE ALL RECORDS from every table."
read -rp "Type 'yes' to proceed: " CONFIRM
if [[ "$CONFIRM" != "yes" ]]; then
    echo "Aborted."
    exit 0
fi

# --- Truncate ---
echo "Truncating all tables..."
psql "$DB_URL" -c "
TRUNCATE workflows, file_queries, change_events, files, watch_roots, snapshots, hosts CASCADE;
"
echo ""

# --- Record counts after ---
echo "Record counts after wipe:"
psql "$DB_URL" -t -A -c "
SELECT '  workflows', COUNT(*) FROM workflows
UNION ALL SELECT '  file_queries', COUNT(*) FROM file_queries
UNION ALL SELECT '  change_events', COUNT(*) FROM change_events
UNION ALL SELECT '  files', COUNT(*) FROM files
UNION ALL SELECT '  watch_roots', COUNT(*) FROM watch_roots
UNION ALL SELECT '  snapshots', COUNT(*) FROM snapshots
UNION ALL SELECT '  hosts', COUNT(*) FROM hosts;
"
echo ""
echo "Done. Archive saved at: $ARCHIVE_FILE"
echo "To restore: gunzip -c $ARCHIVE_FILE | psql \"$DB_URL\""