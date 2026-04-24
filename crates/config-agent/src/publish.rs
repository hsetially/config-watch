use config_shared::events::{ChangeEvent, ChangeEventEnvelope};
use config_shared::ids::IdempotencyKey;
use config_transport::client::{
    ControlPlaneClient, HeartbeatRequest, RegisterRequest, RegisterResponse,
};

pub struct EventPublisher {
    client: ControlPlaneClient,
    host_id: uuid::Uuid,
    hostname: String,
    environment: String,
    agent_version: String,
}

impl EventPublisher {
    pub fn new(
        base_url: &str,
        auth_token: &str,
        host_id: uuid::Uuid,
        hostname: &str,
        environment: &str,
        agent_version: &str,
    ) -> Self {
        Self {
            client: ControlPlaneClient::new(base_url, auth_token),
            host_id,
            hostname: hostname.to_string(),
            environment: environment.to_string(),
            agent_version: agent_version.to_string(),
        }
    }

    pub async fn register(&self, labels: serde_json::Value) -> anyhow::Result<RegisterResponse> {
        let request = RegisterRequest {
            host_id: self.host_id,
            hostname: self.hostname.clone(),
            environment: self.environment.clone(),
            labels,
            agent_version: self.agent_version.clone(),
            watch_roots: vec![],
        };
        self.client.register(&request).await
    }

    pub async fn heartbeat(
        &self,
        spool_depth: usize,
        watched_file_count: usize,
    ) -> anyhow::Result<()> {
        let request = HeartbeatRequest {
            host_id: self.host_id,
            status: "healthy".into(),
            spool_depth,
            watched_file_count,
        };
        self.client.heartbeat(&request).await
    }

    pub fn clone_spool_depth_handle(&self) -> EventPublisher {
        EventPublisher {
            client: self.client.clone(),
            host_id: self.host_id,
            hostname: self.hostname.clone(),
            environment: self.environment.clone(),
            agent_version: self.agent_version.clone(),
        }
    }

    pub async fn publish(
        &self,
        event: &ChangeEvent,
        idempotency_key: &IdempotencyKey,
    ) -> anyhow::Result<bool> {
        let envelope = ChangeEventEnvelope::wrap(event.clone());
        match self.client.publish_change(&envelope, idempotency_key).await {
            Ok(resp) => Ok(resp.accepted),
            Err(e) => {
                tracing::warn!(error = %e, "publish failed");
                Err(e)
            }
        }
    }

    pub fn current_token(&self) -> String {
        self.client.current_token()
    }
}
