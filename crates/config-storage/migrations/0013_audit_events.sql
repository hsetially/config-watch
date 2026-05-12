-- H3: audit_events table for admin action logging.
-- Required for admin approval/rejection/role-change audit trail.

CREATE TABLE IF NOT EXISTS audit_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_id TEXT NOT NULL,
    action TEXT NOT NULL,
    target_user_id TEXT,
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_audit_events_actor ON audit_events(actor_id);
CREATE INDEX IF NOT EXISTS idx_audit_events_action ON audit_events(action);
CREATE INDEX IF NOT EXISTS idx_audit_events_created_at ON audit_events(created_at);