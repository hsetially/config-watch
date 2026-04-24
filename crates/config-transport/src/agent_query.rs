use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub struct AgentQueryClient {
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStatRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePreviewRequest {
    pub path: String,
}

impl Default for AgentQueryClient {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentQueryClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    pub async fn query_stat(&self, agent_addr: &str, path: &str) -> Result<serde_json::Value> {
        let url = format!("http://{}/v1/query/file-metadata", agent_addr);
        let body = FileStatRequest { path: path.to_string() };

        let resp = self.http
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

        resp.json::<serde_json::Value>().await.context("failed to parse stat response")
    }

    pub async fn query_preview(&self, agent_addr: &str, path: &str) -> Result<serde_json::Value> {
        let url = format!("http://{}/v1/query/file-preview", agent_addr);
        let body = FilePreviewRequest { path: path.to_string() };

        let resp = self.http
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("file preview request to agent failed")?;

        let status = resp.status();
        if status == reqwest::StatusCode::FORBIDDEN {
            anyhow::bail!("path denied by agent security policy");
        }
        if !status.is_success() {
            anyhow::bail!("agent preview query failed: HTTP {}", status);
        }

        resp.json::<serde_json::Value>().await.context("failed to parse preview response")
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

        let resp = self.http
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

        resp.json::<serde_json::Value>().await.context("failed to parse content response")
    }
}