use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsMessage {
    pub msg_type: WsMessageType,
    pub event: Option<RealtimeMessage>,
    pub gap_from: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WsMessageType {
    Change,
    Gap,
    Heartbeat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealtimeMessage {
    pub event_id: Uuid,
    pub host_id: Uuid,
    pub environment: String,
    pub path: String,
    pub event_kind: String,
    pub event_time: String,
    pub severity: String,
    pub author_display: Option<String>,
    pub summary: Option<serde_json::Value>,
    pub diff_render: Option<String>,
    #[serde(default)]
    pub pr_url: Option<String>,
    #[serde(default)]
    pub pr_number: Option<i64>,
}
