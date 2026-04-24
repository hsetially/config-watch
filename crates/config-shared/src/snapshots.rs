use serde::{Deserialize, Serialize};

use crate::ids::SnapshotId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompressionKind {
    None,
    Zstd,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRef {
    pub snapshot_id: SnapshotId,
    pub content_hash: String,
    pub size_bytes: u64,
    pub compression: CompressionKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffSummary {
    pub changed_line_estimate: u64,
    pub file_size_before: u64,
    pub file_size_after: u64,
    pub comment_only_hint: bool,
    pub syntax_equivalent_hint: bool,
    #[serde(default)]
    pub yaml_lint_findings: Vec<YamlLintFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YamlLintFinding {
    pub severity: YamlLintSeverity,
    pub check: String,
    pub message: String,
    #[serde(default)]
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YamlLintSeverity {
    Critical,
    Warning,
}
