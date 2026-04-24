-- Track PR association on change events and event IDs on workflows

ALTER TABLE change_events ADD COLUMN pr_url TEXT;
ALTER TABLE change_events ADD COLUMN pr_number BIGINT;

ALTER TABLE workflows ADD COLUMN event_ids_json JSONB;