use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WsMessageType {
    Change,
    Gap,
    Heartbeat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsMessage {
    pub msg_type: WsMessageType,
    pub event: Option<RealtimeMessage>,
    pub gap_from: Option<Uuid>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeMessage {
    pub event_id: Uuid,
    pub host_id: Uuid,
    pub environment: String,
    pub path: String,
    pub event_kind: String,
    pub event_time: String,
    pub severity: String,
    pub author_display: Option<String>,
    pub summary: Option<DiffSummary>,
    pub diff_render: Option<String>,
    #[serde(default)]
    pub pr_url: Option<String>,
    #[serde(default)]
    pub pr_number: Option<i64>,
}

/// Fetched from GET /v1/hosts
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    pub host_id: Uuid,
    pub hostname: String,
    pub environment: String,
    pub labels_json: serde_json::Value,
    pub agent_version: String,
    pub registered_at: String,
    pub last_heartbeat_at: Option<String>,
    pub status: String,
}

/// Fetched from GET /v1/hosts/:host_id/roots
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchRootInfo {
    pub watch_root_id: Uuid,
    pub host_id: Uuid,
    pub root_path: String,
    #[serde(default)]
    pub include_globs: Vec<String>,
    #[serde(default)]
    pub exclude_globs: Vec<String>,
    pub created_at: String,
    pub active: bool,
}

/// Fetched from GET /v1/changes (list strips diff_render)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeEventRow {
    pub event_id: Uuid,
    pub idempotency_key: String,
    pub host_id: Uuid,
    pub file_id: Option<Uuid>,
    pub event_time: String,
    pub event_kind: String,
    pub previous_snapshot_id: Option<Uuid>,
    pub current_snapshot_id: Option<Uuid>,
    pub diff_artifact_uri: Option<String>,
    pub diff_summary_json: Option<DiffSummary>,
    pub author_name: Option<String>,
    pub author_source: Option<String>,
    pub author_confidence: String,
    pub process_hint: Option<String>,
    pub severity: String,
    pub created_at: String,
    #[serde(default)]
    pub diff_render: Option<String>,
    pub canonical_path: Option<String>,
    #[serde(default)]
    pub pr_url: Option<String>,
    #[serde(default)]
    pub pr_number: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Stream,
    History,
    Compare,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FilterState {
    pub environment: Option<String>,
    pub host_id: Option<String>,
    pub path_prefix: Option<String>,
    pub filename: Option<String>,
    pub severity: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
}

impl FilterState {
    pub fn to_query_string(&self) -> String {
        let mut params = Vec::new();
        if let Some(ref v) = self.environment {
            params.push(format!("environment={}", url_encode(v)));
        }
        if let Some(ref v) = self.host_id {
            params.push(format!("host_id={}", url_encode(v)));
        }
        if let Some(ref v) = self.path_prefix {
            params.push(format!("path_prefix={}", url_encode(v)));
        }
        if let Some(ref v) = self.filename {
            params.push(format!("filename={}", url_encode(v)));
        }
        if let Some(ref v) = self.severity {
            params.push(format!("severity={}", url_encode(v)));
        }
        if params.is_empty() {
            String::new()
        } else {
            format!("?{}", params.join("&"))
        }
    }

    pub fn to_changes_query_string(&self, pagination: &PaginationState) -> String {
        let mut params = Vec::new();
        if let Some(ref v) = self.host_id {
            params.push(format!("host_id={}", url_encode(v)));
        }
        if let Some(ref v) = self.path_prefix {
            params.push(format!("path_prefix={}", url_encode(v)));
        }
        if let Some(ref v) = self.filename {
            params.push(format!("filename={}", url_encode(v)));
        }
        if let Some(ref v) = self.severity {
            params.push(format!("severity={}", url_encode(v)));
        }
        if let Some(ref v) = self.since {
            if !v.is_empty() {
                let ts = if v.contains('T') {
                    format!("{}%3A00Z", url_encode(v))
                } else {
                    format!("{}T00%3A00%3A00Z", url_encode(v))
                };
                params.push(format!("since={}", ts));
            }
        }
        if let Some(ref v) = self.until {
            if !v.is_empty() {
                let ts = if v.contains('T') {
                    format!("{}%3A59Z", url_encode(v))
                } else {
                    format!("{}T23%3A59%3A59Z", url_encode(v))
                };
                params.push(format!("until={}", ts));
            }
        }
        params.push(format!("limit={}", pagination.page_size));
        params.push(format!("offset={}", (pagination.page - 1) * pagination.page_size));
        format!("?{}", params.join("&"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaginationState {
    pub page: u32,
    pub page_size: u32,
    pub total: u32,
}

impl Default for PaginationState {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: 25,
            total: 0,
        }
    }
}

impl PaginationState {
    pub fn total_pages(&self) -> u32 {
        if self.total == 0 {
            0
        } else {
            self.total.div_ceil(self.page_size)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangesPage {
    pub changes: Vec<ChangeEventRow>,
    pub total: u32,
}

fn url_encode(s: &str) -> String {
    let mut result = String::new();
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Header,
    HunkMeta,
    Added,
    Removed,
    Context,
}

pub fn format_event_time(time_str: &str) -> String {
    time_str
        .replace('T', " ")
        .trim_end_matches('Z')
        .split('.')
        .next()
        .unwrap_or(time_str)
        .to_string()
}

pub fn severity_class(severity: &str) -> &'static str {
    match severity {
        "critical" => "severity-critical",
        _ => "severity-info",
    }
}

/// Build a tooltip explaining why this severity was assigned.
pub fn severity_tooltip(severity: &str, summary: &Option<DiffSummary>) -> String {
    match severity {
        "critical" => {
            let mut reasons: Vec<String> = Vec::new();
            if let Some(s) = summary {
                for finding in &s.yaml_lint_findings {
                    let line_info = finding.line.map(|l| format!(" (line {})", l)).unwrap_or_default();
                    reasons.push(format!("{}: {}{}", finding.check, finding.message, line_info));
                }
            }
            if reasons.is_empty() {
                "YAML structure error".to_string()
            } else {
                reasons.join("; ")
            }
        }
        _ => "Routine config change".to_string(),
    }
}

pub fn event_kind_icon(kind: &str) -> &'static str {
    match kind {
        "created" => "[+]",
        "modified" => "[~]",
        "deleted" => "[-]",
        "metadata_only" => "[m]",
        "metadataonly" => "[m]",
        "permission_changed" => "[p]",
        "permissionchanged" => "[p]",
        _ => "[?]",
    }
}

/// Convert a ChangeEventRow (from REST API) into a RealtimeMessage (used by EventList).
/// History events from the list endpoint lack diff_render; it's fetched on expand.
pub fn history_row_to_message(row: &ChangeEventRow) -> RealtimeMessage {
    RealtimeMessage {
        event_id: row.event_id,
        host_id: row.host_id,
        environment: String::new(),
        path: row.canonical_path.clone().unwrap_or_default(),
        event_kind: row.event_kind.clone(),
        event_time: row.event_time.clone(),
        severity: row.severity.clone(),
        author_display: row.author_name.clone(),
        summary: row.diff_summary_json.clone(),
        diff_render: row.diff_render.clone(),
        pr_url: row.pr_url.clone(),
        pr_number: row.pr_number,
    }
}

// --- Workflow types ---

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChangeRequest {
    pub canonical_path: String,
    pub content_hash: Option<String>,
    pub event_kind: String,
    pub repo_filename: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCreateRequest {
    pub repo_url: String,
    pub branch_name: String,
    pub base_branch: Option<String>,
    pub pr_title: String,
    pub pr_description: Option<String>,
    pub file_changes: Vec<FileChangeRequest>,
    pub reviewers: Option<Vec<String>>,
    pub github_token: Option<String>,
    #[serde(default)]
    pub event_ids: Vec<uuid::Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCreateResponse {
    pub workflow_id: uuid::Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStatusResponse {
    pub workflow: WorkflowStatusRow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStatusRow {
    pub workflow_id: uuid::Uuid,
    pub status: String,
    pub repo_url: String,
    pub branch_name: String,
    pub base_branch: String,
    pub pr_title: String,
    pub pr_description: Option<String>,
    pub reviewers: Option<Vec<String>>,
    pub error_message: Option<String>,
    pub pr_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDefaults {
    pub repo_url: String,
    pub base_branch: String,
    pub pr_title: String,
    pub github_token: String,
}

// --- File content retrieval ---

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileContentResponse {
    pub path: String,
    pub exists: bool,
    pub size_bytes: u64,
    pub content_b64: Option<String>,
    pub offset: u64,
    pub chunk_length: u64,
    pub last_chunk: bool,
    pub content_hash: Option<String>,
}

impl FileContentResponse {
    pub fn decoded_content(&self) -> Option<String> {
        self.content_b64.as_ref().and_then(|b64| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(b64)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
        })
    }
}

// --- File comparison types ---

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColumnSource {
    Agent { host_id: String, hostname: String },
    Github { url: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompareColumn {
    pub source: ColumnSource,
    pub label: String,
    pub file_path: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CompareResult {
    pub source_label: String,
    pub exists: bool,
    pub content: Option<String>,
    pub size_bytes: u64,
    pub content_hash: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubFileContentResponse {
    pub path: String,
    pub content: String,
    pub size_bytes: u64,
    pub sha: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WordSegment {
    pub content: String,
    pub changed: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DiffLine {
    pub line_num: Option<u32>,
    pub kind: DiffLineKind,
    pub content: String,
    pub words: Vec<WordSegment>,
}