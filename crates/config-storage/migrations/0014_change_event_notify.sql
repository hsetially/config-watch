-- Trigger: notify control-plane pods of new change events via LISTEN/NOTIFY.
-- Each pod's PgListener task receives the event_id and re-fetches the full row
-- to broadcast to its local WebSocket clients.

CREATE OR REPLACE FUNCTION notify_change_event() RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify('config_watch_changes', NEW.event_id::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER change_event_notify
    AFTER INSERT ON change_events
    FOR EACH ROW
    EXECUTE FUNCTION notify_change_event();