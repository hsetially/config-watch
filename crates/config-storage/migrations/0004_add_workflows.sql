-- Workflow tracking for Create PR and similar operations

CREATE TABLE workflows (
    workflow_id  UUID PRIMARY KEY,
    status       TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending','cloning','applying','committing','pushing','creating_pr','completed','failed')),
    repo_url     TEXT NOT NULL,
    branch_name  TEXT NOT NULL,
    pr_title     TEXT NOT NULL,
    pr_description TEXT,
    file_changes_json  JSONB NOT NULL,
    path_mappings_json JSONB NOT NULL,
    error_message  TEXT,
    pr_url       TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_workflows_status ON workflows(status);
CREATE INDEX idx_workflows_created_at ON workflows(created_at DESC);