use uuid::Uuid;

use config_transport::websocket::RealtimeMessage;

#[derive(Debug, Clone, Default)]
pub struct SubscriptionFilter {
    pub environment: Option<String>,
    pub host_id: Option<Uuid>,
    pub path_prefix: Option<String>,
    pub severity: Option<String>,
}

impl SubscriptionFilter {
    pub fn matches(&self, msg: &RealtimeMessage) -> bool {
        if let Some(ref env) = self.environment {
            if msg.environment != *env {
                return false;
            }
        }
        if let Some(ref host_id) = self.host_id {
            if msg.host_id != *host_id {
                return false;
            }
        }
        if let Some(ref prefix) = self.path_prefix {
            if !msg.path.starts_with(prefix) {
                return false;
            }
        }
        if let Some(ref sev) = self.severity {
            if msg.severity != *sev {
                return false;
            }
        }
        true
    }
}

pub struct RealtimeService {
    pub broadcast_tx: tokio::sync::broadcast::Sender<RealtimeMessage>,
}

impl RealtimeService {
    pub fn new(tx: tokio::sync::broadcast::Sender<RealtimeMessage>) -> Self {
        Self { broadcast_tx: tx }
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<RealtimeMessage> {
        self.broadcast_tx.subscribe()
    }

    pub fn publish(&self, msg: RealtimeMessage) {
        let _ = self.broadcast_tx.send(msg);
    }
}
