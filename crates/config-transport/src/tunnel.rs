use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TunnelMessageType {
    QueryRequest,
    QueryResponse,
    Ping,
    Pong,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum QueryKind {
    Stat,
    Preview,
    Content,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRequestPayload {
    pub kind: QueryKind,
    pub path: String,
    /// For Content queries: byte offset to start reading from (0-based).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    /// For Content queries: max bytes to read. None = read to end.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

/// Response payload for a Content query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContentResponse {
    pub path: String,
    pub exists: bool,
    /// Total file size in bytes.
    pub size_bytes: u64,
    /// Base64-encoded file content for this chunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_b64: Option<String>,
    /// Byte offset this chunk starts at.
    pub offset: u64,
    /// Length of the decoded content in this chunk.
    pub chunk_length: u64,
    /// Whether this is the final chunk (offset + chunk_length >= size_bytes).
    pub last_chunk: bool,
    /// BLAKE3 hash of the full file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponsePayload {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelMessage {
    pub msg_type: TunnelMessageType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

impl TunnelMessage {
    pub fn ping() -> Self {
        Self {
            msg_type: TunnelMessageType::Ping,
            request_id: None,
            payload: None,
        }
    }

    pub fn pong() -> Self {
        Self {
            msg_type: TunnelMessageType::Pong,
            request_id: None,
            payload: None,
        }
    }

    pub fn query_request(request_id: String, kind: QueryKind, path: String) -> Self {
        let payload = QueryRequestPayload { kind, path, offset: None, limit: None };
        Self {
            msg_type: TunnelMessageType::QueryRequest,
            request_id: Some(request_id),
            payload: Some(serde_json::to_value(payload).unwrap_or_default()),
        }
    }

    pub fn content_query_request(request_id: String, path: String, offset: Option<u64>, limit: Option<u64>) -> Self {
        let payload = QueryRequestPayload { kind: QueryKind::Content, path, offset, limit };
        Self {
            msg_type: TunnelMessageType::QueryRequest,
            request_id: Some(request_id),
            payload: Some(serde_json::to_value(payload).unwrap_or_default()),
        }
    }

    pub fn query_response(request_id: String, payload: QueryResponsePayload) -> Self {
        Self {
            msg_type: TunnelMessageType::QueryResponse,
            request_id: Some(request_id),
            payload: Some(serde_json::to_value(payload).unwrap_or_default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn roundtrip_query_request() {
        let msg = TunnelMessage::query_request(
            "test-id".into(),
            QueryKind::Stat,
            "/etc/app/config.yaml".into(),
        );
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: TunnelMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.msg_type, TunnelMessageType::QueryRequest);
        assert_eq!(decoded.request_id.as_deref(), Some("test-id"));
    }

    #[test]
    fn roundtrip_query_response() {
        let resp = QueryResponsePayload {
            status: "success".into(),
            data: Some(serde_json::json!({"key": "value"})),
            error: None,
        };
        let msg = TunnelMessage::query_response("req-1".into(), resp);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: TunnelMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.msg_type, TunnelMessageType::QueryResponse);
        assert_eq!(decoded.request_id.as_deref(), Some("req-1"));
    }

    #[test]
    fn roundtrip_ping_pong() {
        let ping = TunnelMessage::ping();
        let json = serde_json::to_string(&ping).unwrap();
        let decoded: TunnelMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.msg_type, TunnelMessageType::Ping);
        assert!(decoded.request_id.is_none());

        let pong = TunnelMessage::pong();
        let json = serde_json::to_string(&pong).unwrap();
        let decoded: TunnelMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.msg_type, TunnelMessageType::Pong);
    }

    #[test]
    fn roundtrip_content_query_request() {
        let msg = TunnelMessage::content_query_request(
            "test-id".into(),
            "/etc/app/big-config.yaml".into(),
            Some(0),
            Some(65536),
        );
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: TunnelMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.msg_type, TunnelMessageType::QueryRequest);
        let payload: QueryRequestPayload = decoded.payload.and_then(|v| serde_json::from_value(v).ok()).unwrap();
        assert_eq!(payload.kind, QueryKind::Content);
        assert_eq!(payload.path, "/etc/app/big-config.yaml");
        assert_eq!(payload.offset, Some(0));
        assert_eq!(payload.limit, Some(65536));
    }

    #[test]
    fn roundtrip_file_content_response() {
        let resp = FileContentResponse {
            path: "/etc/app/config.yaml".into(),
            exists: true,
            size_bytes: 1024,
            content_b64: Some(base64::engine::general_purpose::STANDARD.encode(b"hello")),
            offset: 0,
            chunk_length: 5,
            last_chunk: true,
            content_hash: Some("abc123".into()),
        };
        let data = serde_json::to_value(&resp).unwrap();
        let decoded: FileContentResponse = serde_json::from_value(data).unwrap();
        assert!(decoded.last_chunk);
        assert_eq!(decoded.size_bytes, 1024);
    }
}