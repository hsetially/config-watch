use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use std::sync::Arc;

use crate::query_handler::QueryHandler;

#[derive(Clone)]
pub struct AgentState {
    pub query_handler: Arc<QueryHandler>,
}

pub fn build_agent_router(state: AgentState) -> Router<()> {
    Router::new()
        .route("/v1/query/file-metadata", post(file_metadata_handler))
        .route("/v1/query/file-preview", post(file_preview_handler))
        .route("/v1/query/file-content", post(file_content_handler))
        .with_state(state)
}

async fn file_metadata_handler(
    State(state): State<AgentState>,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "missing path"})),
            )
        }
    };

    match state.query_handler.stat(&path).await {
        Ok(resp) => (
            StatusCode::OK,
            Json(serde_json::to_value(resp).unwrap_or_default()),
        ),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("denied by security policy") || msg.contains("not in watch roots") {
                (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": msg})),
                )
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": msg})),
                )
            }
        }
    }
}

async fn file_preview_handler(
    State(state): State<AgentState>,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "missing path"})),
            )
        }
    };

    match state.query_handler.preview(&path) {
        Ok(resp) => (
            StatusCode::OK,
            Json(serde_json::to_value(resp).unwrap_or_default()),
        ),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("denied by security policy") || msg.contains("not in watch roots") {
                (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": msg})),
                )
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": msg})),
                )
            }
        }
    }
}

async fn file_content_handler(
    State(state): State<AgentState>,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "missing path"})),
            )
        }
    };
    let offset = body.get("offset").and_then(|v| v.as_u64());
    let limit = body.get("limit").and_then(|v| v.as_u64());

    match state.query_handler.content(&path, offset, limit) {
        Ok(resp) => (
            StatusCode::OK,
            Json(serde_json::to_value(resp).unwrap_or_default()),
        ),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("denied by security policy") || msg.contains("not in watch roots") {
                (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": msg})),
                )
            } else if msg.contains("too large") {
                (
                    StatusCode::PAYLOAD_TOO_LARGE,
                    Json(serde_json::json!({"error": msg})),
                )
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": msg})),
                )
            }
        }
    }
}
