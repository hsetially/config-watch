use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub struct AgentQueryClient {
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStatRequest {
    pub path: String,
}

/// Which revision of a file to preview. Default is the live disk read; a
/// snapshot revision is read from the agent's local snapshot store by hash.
/// Old agents (pre-revision) ignore the field and behave as `Current`, so
/// `Current` MUST stay the serde default.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewRevision {
    #[default]
    Current,
    Snapshot {
        content_hash: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePreviewRequest {
    pub path: String,
    #[serde(default, skip_serializing_if = "PreviewRevision::is_default")]
    pub revision: PreviewRevision,
}

impl PreviewRevision {
    fn is_default(&self) -> bool {
        matches!(self, PreviewRevision::Current)
    }
}

/// Returned when the snapshot bytes the caller asked for are no longer in the
/// agent's local store (evicted by retention). Mapped to HTTP 410 Gone over the
/// wire so the control plane can render a graceful "previous unavailable"
/// fallback instead of treating it as an internal error.
#[derive(Debug, thiserror::Error)]
#[error("snapshot not available: {0}")]
pub struct SnapshotGone(pub String);

impl Default for AgentQueryClient {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentQueryClient {
    pub fn new() -> Self {
        // Agent-to-agent queries are internal (localhost), so we don't enforce HTTPS.
        // But we still set a minimum TLS version for any external calls.
        let http = reqwest::Client::builder()
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { http }
    }

    pub async fn query_stat(&self, agent_addr: &str, path: &str) -> Result<serde_json::Value> {
        let url = format!("http://{}/v1/query/file-metadata", agent_addr);
        let body = FileStatRequest {
            path: path.to_string(),
        };

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("file stat request to agent failed")?;

        let status = resp.status();
        if status == reqwest::StatusCode::FORBIDDEN {
            anyhow::bail!("path denied by agent security policy");
        }
        if !status.is_success() {
            anyhow::bail!("agent stat query failed: HTTP {}", status);
        }

        resp.json::<serde_json::Value>()
            .await
            .context("failed to parse stat response")
    }

    pub async fn query_preview(&self, agent_addr: &str, path: &str) -> Result<serde_json::Value> {
        self.query_preview_revision(agent_addr, path, PreviewRevision::Current)
            .await
    }

    pub async fn query_preview_revision(
        &self,
        agent_addr: &str,
        path: &str,
        revision: PreviewRevision,
    ) -> Result<serde_json::Value> {
        let url = format!("http://{}/v1/query/file-preview", agent_addr);
        let body = FilePreviewRequest {
            path: path.to_string(),
            revision,
        };

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("file preview request to agent failed")?;

        let status = resp.status();
        if status == reqwest::StatusCode::FORBIDDEN {
            anyhow::bail!("path denied by agent security policy");
        }
        if status == reqwest::StatusCode::GONE {
            // Snapshot was retention-evicted on the agent; bubble up as a typed
            // error so the control plane can render "previous unavailable"
            // instead of treating it as a generic failure.
            let detail = resp
                .text()
                .await
                .unwrap_or_else(|_| "snapshot evicted".to_string());
            return Err(SnapshotGone(detail).into());
        }
        if !status.is_success() {
            anyhow::bail!("agent preview query failed: HTTP {}", status);
        }

        resp.json::<serde_json::Value>()
            .await
            .context("failed to parse preview response")
    }

    pub async fn query_content(
        &self,
        agent_addr: &str,
        path: &str,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> Result<serde_json::Value> {
        let url = format!("http://{}/v1/query/file-content", agent_addr);
        let mut body = serde_json::json!({ "path": path });
        if let Some(off) = offset {
            body["offset"] = serde_json::Value::Number(off.into());
        }
        if let Some(lim) = limit {
            body["limit"] = serde_json::Value::Number(lim.into());
        }

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("file content request to agent failed")?;

        let status = resp.status();
        if status == reqwest::StatusCode::FORBIDDEN {
            anyhow::bail!("path denied by agent security policy");
        }
        if status == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
            anyhow::bail!("file too large");
        }
        if !status.is_success() {
            anyhow::bail!("agent content query failed: HTTP {}", status);
        }

        resp.json::<serde_json::Value>()
            .await
            .context("failed to parse content response")
    }
}
