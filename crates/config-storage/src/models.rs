use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct HostRow {
    pub host_id: Uuid,
    pub hostname: String,
    pub environment: String,
    pub labels_json: serde_json::Value,
    pub agent_version: String,
    pub registered_at: DateTime<Utc>,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WatchRootRow {
    pub watch_root_id: Uuid,
    pub host_id: Uuid,
    pub root_path: String,
    pub include_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct FileRow {
    pub file_id: Uuid,
    pub host_id: Uuid,
    pub watch_root_id: Uuid,
    pub canonical_path: String,
    pub last_hash: Option<String>,
    pub last_seen_at: DateTime<Utc>,
    pub exists_now: bool,
    pub last_snapshot_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SnapshotRow {
    pub snapshot_id: Uuid,
    pub content_hash: String,
    pub size_bytes: i64,
    pub storage_uri: String,
    pub compression: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChangeEventRow {
    pub event_id: Uuid,
    pub idempotency_key: String,
    pub host_id: Uuid,
    pub file_id: Option<Uuid>,
    pub event_time: DateTime<Utc>,
    pub event_kind: String,
    pub previous_snapshot_id: Option<Uuid>,
    pub current_snapshot_id: Option<Uuid>,
    pub diff_artifact_uri: Option<String>,
    pub diff_summary_json: Option<serde_json::Value>,
    pub author_name: Option<String>,
    pub author_source: Option<String>,
    pub author_confidence: String,
    pub process_hint: Option<String>,
    pub severity: String,
    pub created_at: DateTime<Utc>,
    pub diff_render: Option<String>,
    pub canonical_path: Option<String>,
    pub pr_url: Option<String>,
    pub pr_number: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct FileQueryRow {
    pub query_id: Uuid,
    pub requester_id: String,
    pub host_id: Uuid,
    pub canonical_path: String,
    pub query_kind: String,
    pub result_status: String,
    pub requested_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WorkflowRow {
    pub workflow_id: Uuid,
    pub status: String,
    pub repo_url: String,
    pub branch_name: String,
    pub base_branch: String,
    pub pr_title: String,
    pub pr_description: Option<String>,
    pub file_changes_json: serde_json::Value,
    pub error_message: Option<String>,
    pub pr_url: Option<String>,
    pub reviewers_json: Option<serde_json::Value>,
    pub event_ids_json: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
