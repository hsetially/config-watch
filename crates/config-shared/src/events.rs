use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::attribution::Attribution;
use crate::ids::{EventId, HostId, IdempotencyKey, SnapshotId};
use crate::snapshots::DiffSummary;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Created,
    Modified,
    Deleted,
    MetadataOnly,
    PermissionChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEvent {
    pub event_id: EventId,
    pub idempotency_key: IdempotencyKey,
    pub host_id: HostId,
    pub canonical_path: Utf8PathBuf,
    pub event_time: DateTime<Utc>,
    pub event_kind: ChangeKind,
    pub previous_snapshot_id: Option<SnapshotId>,
    pub current_snapshot_id: Option<SnapshotId>,
    pub diff_summary: Option<DiffSummary>,
    pub diff_render: Option<String>,
    pub attribution: Attribution,
    pub severity: Severity,
    /// Base64-encoded current file content (for created/modified events).
    /// Used by the control plane to populate its snapshot store.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_b64: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEventEnvelope {
    pub event: ChangeEvent,
    pub schema_version: String,
}

impl ChangeEventEnvelope {
    pub fn wrap(event: ChangeEvent) -> Self {
        Self {
            event,
            schema_version: "1.0".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEventSummary {
    pub event_id: EventId,
    pub host_id: HostId,
    pub path: Utf8PathBuf,
    pub event_kind: ChangeKind,
    pub event_time: DateTime<Utc>,
    pub severity: Severity,
    pub author_name: Option<String>,
    pub diff_summary: Option<DiffSummary>,
}
