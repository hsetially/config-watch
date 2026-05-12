-- Add 'initial_snapshot' as a valid event_kind value for the agent startup scan feature.
ALTER TABLE change_events DROP CONSTRAINT IF EXISTS change_events_event_kind_check;
ALTER TABLE change_events ADD CONSTRAINT change_events_event_kind_check
    CHECK (event_kind IN ('created','modified','deleted','metadata_only','permission_changed','initial_snapshot'));