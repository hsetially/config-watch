ALTER TABLE change_events ADD COLUMN canonical_path TEXT;

-- Backfill from files table where file_id is set
UPDATE change_events
SET canonical_path = f.canonical_path
FROM files f
WHERE change_events.file_id = f.file_id
  AND change_events.canonical_path IS NULL;

-- Index for path prefix queries
CREATE INDEX idx_change_events_canonical_path ON change_events(canonical_path);