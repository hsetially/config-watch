-- config-watch initial schema

CREATE TABLE hosts (
    host_id UUID PRIMARY KEY,
    hostname TEXT NOT NULL,
    environment TEXT NOT NULL DEFAULT 'default',
    labels_json JSONB NOT NULL DEFAULT '{}',
    agent_version TEXT NOT NULL DEFAULT '0.1.0',
    registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'registering'
        CHECK (status IN ('registering','healthy','degraded','offline','revoked'))
);

CREATE TABLE watch_roots (
    watch_root_id UUID PRIMARY KEY,
    host_id UUID NOT NULL REFERENCES hosts(host_id),
    root_path TEXT NOT NULL,
    include_globs TEXT[] NOT NULL DEFAULT '{"**/*.yaml","**/*.yml"}',
    exclude_globs TEXT[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    active BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE TABLE files (
    file_id UUID PRIMARY KEY,
    host_id UUID NOT NULL REFERENCES hosts(host_id),
    watch_root_id UUID NOT NULL REFERENCES watch_roots(watch_root_id),
    canonical_path TEXT NOT NULL,
    last_hash TEXT,
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    exists_now BOOLEAN NOT NULL DEFAULT TRUE,
    last_snapshot_id UUID,
    UNIQUE(host_id, canonical_path)
);

CREATE TABLE snapshots (
    snapshot_id UUID PRIMARY KEY,
    content_hash TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    storage_uri TEXT NOT NULL,
    compression TEXT NOT NULL DEFAULT 'none'
        CHECK (compression IN ('none','zstd')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE change_events (
    event_id UUID PRIMARY KEY,
    idempotency_key TEXT NOT NULL UNIQUE,
    host_id UUID NOT NULL REFERENCES hosts(host_id),
    file_id UUID REFERENCES files(file_id),
    event_time TIMESTAMPTZ NOT NULL,
    event_kind TEXT NOT NULL
        CHECK (event_kind IN ('created','modified','deleted','metadata_only','permission_changed')),
    previous_snapshot_id UUID REFERENCES snapshots(snapshot_id),
    current_snapshot_id UUID REFERENCES snapshots(snapshot_id),
    diff_artifact_uri TEXT,
    diff_summary_json JSONB,
    author_name TEXT,
    author_source TEXT,
    author_confidence TEXT NOT NULL DEFAULT 'unknown'
        CHECK (author_confidence IN ('exact','probable','weak','unknown')),
    process_hint TEXT,
    severity TEXT NOT NULL DEFAULT 'info'
        CHECK (severity IN ('info','warning','critical')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE file_queries (
    query_id UUID PRIMARY KEY,
    requester_id TEXT NOT NULL,
    host_id UUID NOT NULL REFERENCES hosts(host_id),
    canonical_path TEXT NOT NULL,
    query_kind TEXT NOT NULL
        CHECK (query_kind IN ('stat','preview')),
    result_status TEXT NOT NULL
        CHECK (result_status IN ('success','denied','error','timeout')),
    requested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ
);

-- Indexes for query performance
CREATE INDEX idx_change_events_event_time ON change_events(event_time DESC);
CREATE INDEX idx_change_events_host_time ON change_events(host_id, event_time DESC);
CREATE INDEX idx_change_events_idempotency ON change_events(idempotency_key);
CREATE INDEX idx_change_events_severity_time ON change_events(severity, event_time DESC);
CREATE INDEX idx_files_host_path ON files(host_id, canonical_path);
CREATE INDEX idx_hosts_status ON hosts(status);