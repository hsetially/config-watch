use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use config_shared::events::ChangeEventEnvelope;
use config_shared::ids::IdempotencyKey;

use crate::idempotency::generate_idempotency_header;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub host_id: Uuid,
    pub hostname: String,
    pub environment: String,
    pub labels: serde_json::Value,
    pub agent_version: String,
    pub watch_roots: Vec<WatchRootRegistration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchRootRegistration {
    pub root_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub agent_credential: String,
    pub credential_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    pub host_id: Uuid,
    pub status: String,
    pub spool_depth: usize,
    pub watched_file_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishChangeResponse {
    pub accepted: bool,
    pub event_id: Option<Uuid>,
    pub message: Option<String>,
}

#[derive(Clone)]
pub struct ControlPlaneClient {
    http: reqwest::Client,
    base_url: String,
    auth_token: std::sync::Arc<std::sync::Mutex<String>>,
}

impl ControlPlaneClient {
    pub fn new(base_url: &str, auth_token: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.to_string(),
            auth_token: std::sync::Arc::new(std::sync::Mutex::new(auth_token.to_string())),
        }
    }

    pub fn current_token(&self) -> String {
        self.auth_token.lock().unwrap().clone()
    }

    pub async fn register(&self, request: &RegisterRequest) -> Result<RegisterResponse> {
        let url = format!("{}/v1/agents/register", self.base_url);
        let enrollment_token = self.auth_token.lock().unwrap().clone();
        let resp = self.http
            .post(&url)
            .header("X-Enrollment-Token", &enrollment_token)
            .json(request)
            .send()
            .await
            .context("register request failed")?;

        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("register failed: HTTP {}", status);
        }

        let response = resp.json::<RegisterResponse>()
            .await
            .context("failed to parse register response")?;

        // Switch from enrollment token to HMAC credential for subsequent requests
        *self.auth_token.lock().unwrap() = response.agent_credential.clone();

        Ok(response)
    }

    pub async fn heartbeat(&self, request: &HeartbeatRequest) -> Result<()> {
        let url = format!("{}/v1/agents/heartbeat", self.base_url);
        let resp = self.retry_post(&url, request).await?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("heartbeat failed: HTTP {}", status);
        }
        Ok(())
    }

    pub async fn publish_change(
        &self,
        event: &ChangeEventEnvelope,
        idempotency_key: &IdempotencyKey,
    ) -> Result<PublishChangeResponse> {
        let url = format!("{}/v1/events/change", self.base_url);
        let key_header = generate_idempotency_header(idempotency_key);
        let token = self.auth_token.lock().unwrap().clone();

        let resp = self.http
            .post(&url)
            .header("X-Agent-Token", &token)
            .header("X-Idempotency-Key", key_header)
            .json(event)
            .send()
            .await
            .context("publish change request failed")?;

        let status = resp.status();
        if status == StatusCode::CONFLICT {
            return Ok(PublishChangeResponse {
                accepted: true,
                event_id: None,
                message: Some("duplicate".into()),
            });
        }
        if !status.is_success() {
            anyhow::bail!("publish failed: HTTP {}", status);
        }

        resp.json::<PublishChangeResponse>()
            .await
            .context("failed to parse publish response")
    }

    async fn retry_post<T: Serialize>(&self, url: &str, body: &T) -> Result<reqwest::Response> {
        let mut attempts = 0u32;
        let max_retries = 3;
        let token = self.auth_token.lock().unwrap().clone();

        loop {
            let resp = self.http
                .post(url)
                .header("X-Agent-Token", &token)
                .json(body)
                .send()
                .await;

            match resp {
                Ok(r) => {
                    if r.status().is_server_error() && attempts < max_retries {
                        attempts += 1;
                        let delay = std::time::Duration::from_millis(100 * 2u64.pow(attempts));
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Ok(r);
                }
                Err(e) if attempts < max_retries => {
                    attempts += 1;
                    let delay = std::time::Duration::from_millis(100 * 2u64.pow(attempts));
                    tracing::warn!(error = %e, attempt = attempts, "request failed, retrying");
                    tokio::time::sleep(delay).await;
                    continue;
                }
                Err(e) => return Err(e).context("request failed after retries"),
            }
        }
    }
}