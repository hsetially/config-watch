-- Add base_branch column, drop path_mappings_json (replaced by repo filename search)

ALTER TABLE workflows ADD COLUMN base_branch TEXT NOT NULL DEFAULT 'main';
ALTER TABLE workflows DROP COLUMN path_mappings_json;