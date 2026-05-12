use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use std::sync::RwLock;

use crate::metrics::AgentMetrics;
use crate::query_handler::QueryHandler;

#[derive(Clone)]
pub struct ConfigInfo {
    pub agent_id: String,
    pub environment: String,
    pub watch_mode: String,
    pub poll_interval_secs: u64,
    pub watch_roots: Vec<String>,
}

#[derive(Clone)]
pub struct AgentState {
    pub query_handler: Arc<QueryHandler>,
    /// HMAC secret for authenticating requests. If empty, auth is skipped
    /// (only safe when bound to 127.0.0.1).
    pub agent_secret: String,
    pub metrics: Arc<AgentMetrics>,
    pub watch_backend: Arc<RwLock<String>>,
    pub spool_dir: camino::Utf8PathBuf,
    pub snapshot_dir: camino::Utf8PathBuf,
    pub config_info: Arc<ConfigInfo>,
}

pub fn build_agent_router(state: AgentState) -> Router {
    Router::new()
        .route("/v1/agent/health", get(health_handler))
        .route("/v1/query/file-metadata", post(file_metadata_handler))
        .route("/v1/query/file-preview", post(file_preview_handler))
        .route("/v1/query/file-content", post(file_content_handler))
        .with_state(state)
}

/// Validate X-Agent-Token HMAC header. Returns Ok(()) if:
/// - secret is empty (local-only mode, bound to 127.0.0.1)
/// - the token verifies against the stored secret
fn verify_agent_token(secret: &str, headers: &axum::http::HeaderMap) -> Result<(), StatusCode> {
    if secret.is_empty() {
        return Ok(());
    }
    let token = headers
        .get("X-Agent-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if token.is_empty() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    match config_auth::tokens::AgentCredential::verify(secret, token) {
        Ok(_) => Ok(()),
        Err(e) => {
            tracing::warn!(error = %e, "agent API auth failed");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

async fn health_handler(State(state): State<AgentState>) -> (StatusCode, Json<serde_json::Value>) {
    let metrics = state.metrics.snapshot();
    let watch_backend = state.watch_backend.read().unwrap().clone();
    let spool_writable = check_dir_writable(&state.spool_dir);
    let snapshot_writable = check_dir_writable(&state.snapshot_dir);
    let (difft_path, difft_available) = config_diff::find_difft_binary();

    let health = serde_json::json!({
        "status": "ok",
        "agent_id": state.config_info.agent_id,
        "environment": state.config_info.environment,
        "watch_backend": watch_backend,
        "watch_mode_config": state.config_info.watch_mode,
        "poll_interval_secs": state.config_info.poll_interval_secs,
        "watch_roots": state.config_info.watch_roots,
        "metrics": metrics,
        "storage": {
            "spool_dir": state.spool_dir,
            "spool_writable": spool_writable,
            "snapshot_dir": state.snapshot_dir,
            "snapshot_writable": snapshot_writable,
        },
        "difftastic": {
            "available": difft_available,
            "path": difft_path.to_string_lossy(),
        },
    });

    (StatusCode::OK, Json(health))
}

fn check_dir_writable(dir: &camino::Utf8Path) -> bool {
    let test_file = dir.join(".health_check_write_test");
    match std::fs::write(&test_file, b"test") {
        Ok(_) => {
            let _ = std::fs::remove_file(&test_file);
            true
        }
        Err(_) => false,
    }
}

async fn file_metadata_handler(
    State(state): State<AgentState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Err(status) = verify_agent_token(&state.agent_secret, &headers) {
        return (status, Json(serde_json::json!({"error": "invalid agent token"})));
    }

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
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Err(status) = verify_agent_token(&state.agent_secret, &headers) {
        return (status, Json(serde_json::json!({"error": "invalid agent token"})));
    }

    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "missing path"})),
            )
        }
    };

    // The `revision` field is optional. Old clients omit it and get the
    // legacy disk-read behavior via the serde default (`Current`).
    let revision: config_transport::agent_query::PreviewRevision = match body.get("revision") {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(r) => r,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": format!("invalid revision: {}", e)})),
                );
            }
        },
        None => Default::default(),
    };

    let result = state.query_handler.preview_revision(&path, &revision).await;
    match result {
        Ok(resp) => (
            StatusCode::OK,
            Json(serde_json::to_value(resp).unwrap_or_default()),
        ),
        Err(e) => {
            // Check for the typed Gone error first — anyhow erases the type but
            // preserves the chain via downcast.
            if e.downcast_ref::<config_transport::agent_query::SnapshotGone>()
                .is_some()
            {
                return (
                    StatusCode::GONE,
                    Json(serde_json::json!({"error": e.to_string()})),
                );
            }
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
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Err(status) = verify_agent_token(&state.agent_secret, &headers) {
        return (status, Json(serde_json::json!({"error": "invalid agent token"})));
    }

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