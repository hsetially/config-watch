use std::time::Duration;

use axum::{
    Router,
    routing::{get, post},
    extract::{State, WebSocketUpgrade, ws::{Message, WebSocket}},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use futures::{SinkExt, StreamExt};
use uuid::Uuid;

use crate::http::extractors::{AgentAuth, ChangeFilters, CorrelationId, Pagination};
use crate::realtime::SubscriptionFilter;
use crate::services::AppState;

pub fn build_router(state: AppState) -> Router<()> {
    Router::new()
        .route("/v1/agents/register", post(register_handler))
        .route("/v1/agents/heartbeat", post(heartbeat_handler))
        .route("/v1/agents/tunnel", get(agent_tunnel_handler))
        .route("/v1/events/change", post(change_ingest_handler))
        .route("/v1/hosts", get(hosts_list_handler))
        .route("/v1/hosts/:host_id", get(host_detail_handler))
        .route("/v1/hosts/:host_id/roots", get(host_roots_handler))
        .route("/v1/changes", get(changes_list_handler))
        .route("/v1/changes/:event_id", get(change_detail_handler))
        .route("/v1/changes/stream", get(changes_stream_handler))
        .route("/v1/file/stat", post(file_stat_handler))
        .route("/v1/file/preview", post(file_preview_handler))
        .route("/v1/file/content", post(file_content_handler))
        .route("/v1/github/file-content", post(github_file_content_handler))
        .route("/v1/workflows", post(create_workflow_handler).get(list_workflows_handler))
        .route("/v1/workflows/:workflow_id", get(get_workflow_handler))
        .route("/v1/metrics", get(metrics_handler))
        .with_state(state)
}

type HandlerResponse = (StatusCode, Json<serde_json::Value>);

async fn register_handler(
    State(state): State<AppState>,
    _cid: CorrelationId,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> HandlerResponse {
    // Verify enrollment token
    let enrollment_token = headers
        .get("X-Enrollment-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();

    if enrollment_token.is_empty() {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "missing enrollment token"})));
    }

    // In v1, enrollment token = control plane secret (simple approach)
    if enrollment_token != state.secret {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "invalid enrollment token"})));
    }

    let host_id = body
        .get("host_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok());

    let Some(host_id) = host_id else {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "missing host_id"})));
    };

    let hostname = body.get("hostname").and_then(|v| v.as_str()).unwrap_or("unknown");
    let environment = body.get("environment").and_then(|v| v.as_str()).unwrap_or("default");
    let agent_version = body.get("agent_version").and_then(|v| v.as_str()).unwrap_or("0.1.0");
    let labels = body.get("labels").cloned().unwrap_or(serde_json::json!({}));

    match config_storage::repositories::hosts::HostsRepo::register(
        state.db.pool(),
        host_id,
        hostname,
        environment,
        labels,
        agent_version,
    )
    .await
    {
        Ok(row) => {
            let credential = config_auth::tokens::AgentCredential::issue(
                &state.secret,
                &host_id.to_string(),
                chrono::Duration::hours(24),
            );
            let response = serde_json::json!({
                "agent_credential": credential.token,
                "credential_expires_at": credential.expires_at.to_rfc3339(),
                "host": row,
            });
            (StatusCode::CREATED, Json(response))
        }
        Err(e) => {
            tracing::error!(error = %e, "register failed");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "register failed"})))
        }
    }
}

async fn heartbeat_handler(
    State(state): State<AppState>,
    _auth: AgentAuth,
    _cid: CorrelationId,
    Json(body): Json<serde_json::Value>,
) -> HandlerResponse {
    let host_id = body
        .get("host_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok());

    let Some(host_id) = host_id else {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "missing host_id"})));
    };

    match config_storage::repositories::hosts::HostsRepo::heartbeat(state.db.pool(), host_id).await {
        Ok(()) => (StatusCode::NO_CONTENT, Json(serde_json::json!({}))),
        Err(e) => {
            tracing::error!(error = %e, "heartbeat failed");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "heartbeat failed"})))
        }
    }
}

async fn agent_tunnel_handler(
    State(state): State<AppState>,
    auth: AgentAuth,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let host_id: Uuid = auth.host_id.parse().unwrap_or_else(|_| Uuid::nil());
    ws.on_upgrade(move |socket| {
        crate::tunnel::handle_agent_tunnel(socket, state, host_id)
    })
}

async fn change_ingest_handler(
    State(state): State<AppState>,
    _auth: AgentAuth,
    _cid: CorrelationId,
    Json(body): Json<serde_json::Value>,
) -> HandlerResponse {
    match crate::ingest::IngestService::ingest_change(state.db.pool(), &state.broadcast_tx, &state.snapshot_store, body).await {
        Ok(crate::ingest::IngestOutcome::Accepted { event_id }) => {
            (StatusCode::CREATED, Json(serde_json::json!({"accepted": true, "event_id": event_id})))
        }
        Ok(crate::ingest::IngestOutcome::Duplicate { event_id }) => {
            (StatusCode::CONFLICT, Json(serde_json::json!({"accepted": true, "event_id": event_id, "message": "duplicate"})))
        }
        Ok(crate::ingest::IngestOutcome::Rejected { reason }) => {
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({"accepted": false, "error": reason})))
        }
        Err(e) => {
            tracing::error!(error = %e, "ingest failed");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"accepted": false, "error": "ingest failed"})))
        }
    }
}

async fn hosts_list_handler(
    State(state): State<AppState>,
    axum::extract::Query(pagination): axum::extract::Query<Pagination>,
) -> HandlerResponse {
    let hosts = config_storage::repositories::hosts::HostsRepo::list(
        state.db.pool(),
        None,
        pagination.limit,
        pagination.offset,
    )
    .await
    .unwrap_or_default();

    (StatusCode::OK, Json(serde_json::json!({"hosts": hosts})))
}

async fn host_detail_handler(
    State(state): State<AppState>,
    axum::extract::Path(host_id): axum::extract::Path<Uuid>,
) -> HandlerResponse {
    match config_storage::repositories::hosts::HostsRepo::get(state.db.pool(), host_id).await {
        Ok(Some(row)) => (StatusCode::OK, Json(serde_json::json!({"host": row}))),
        Ok(None) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "host not found"}))),
        Err(e) => {
            tracing::error!(error = %e, "get host failed");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "internal error"})))
        }
    }
}

async fn host_roots_handler(
    State(state): State<AppState>,
    axum::extract::Path(host_id): axum::extract::Path<Uuid>,
) -> HandlerResponse {
    match config_storage::repositories::watch_roots::WatchRootsRepo::list_by_host(state.db.pool(), host_id).await {
        Ok(roots) => (StatusCode::OK, Json(serde_json::json!({"roots": roots}))),
        Err(e) => {
            tracing::error!(error = %e, "get host roots failed");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "internal error"})))
        }
    }
}

async fn changes_list_handler(
    State(state): State<AppState>,
    axum::extract::Query(pagination): axum::extract::Query<Pagination>,
    axum::extract::Query(filters): axum::extract::Query<ChangeFilters>,
) -> HandlerResponse {
    let since = filters.since.as_deref().and_then(|s| {
        let s = if s.len() == 10 && s.chars().all(|c| c.is_ascii_digit() || c == '-') {
            format!("{}T00:00:00Z", s)
        } else {
            s.to_string()
        };
        s.parse::<DateTime<Utc>>().ok()
    });

    let until = filters.until.as_deref().and_then(|s| {
        let s = if s.len() == 10 && s.chars().all(|c| c.is_ascii_digit() || c == '-') {
            format!("{}T23:59:59Z", s)
        } else {
            s.to_string()
        };
        s.parse::<DateTime<Utc>>().ok()
    });

    let f = config_storage::repositories::change_events::ChangeEventFilters {
        host_id: filters.host_id.as_ref().and_then(|s| Uuid::parse_str(s).ok()),
        path_prefix: filters.path_prefix.clone(),
        filename: filters.filename.clone(),
        author: filters.author.clone(),
        severity: filters.severity.clone(),
        since,
        until,
    };

    let total = config_storage::repositories::change_events::ChangeEventsRepo::count(
        state.db.pool(),
        &f,
    )
    .await
    .unwrap_or(0);

    let events = config_storage::repositories::change_events::ChangeEventsRepo::list(
        state.db.pool(),
        &f,
        pagination.limit,
        pagination.offset,
    )
    .await
    .unwrap_or_default();

    // Strip diff_render from list responses to keep payloads small.
    // Use GET /v1/changes/:event_id for the full diff text.
    let changes: Vec<serde_json::Value> = events
        .into_iter()
        .map(|row| {
            let mut v = serde_json::to_value(row).unwrap_or_default();
            v.as_object_mut().map(|o| o.remove("diff_render"));
            v
        })
        .collect();

    (StatusCode::OK, Json(serde_json::json!({"changes": changes, "total": total})))
}

async fn change_detail_handler(
    State(state): State<AppState>,
    axum::extract::Path(event_id): axum::extract::Path<Uuid>,
) -> HandlerResponse {
    match config_storage::repositories::change_events::ChangeEventsRepo::get(state.db.pool(), event_id).await {
        Ok(Some(row)) => (StatusCode::OK, Json(serde_json::json!({"event": row}))),
        Ok(None) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "event not found"}))),
        Err(e) => {
            tracing::error!(error = %e, "get event failed");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "internal error"})))
        }
    }
}

async fn changes_stream_handler(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let filter = SubscriptionFilter {
        environment: params.get("environment").cloned(),
        host_id: params.get("host_id").and_then(|s| Uuid::parse_str(s).ok()),
        path_prefix: params.get("path_prefix").cloned(),
        severity: params.get("severity").cloned(),
    };

    ws.on_upgrade(move |socket| handle_ws(socket, state, filter))
}

async fn handle_ws(socket: WebSocket, state: AppState, filter: SubscriptionFilter) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.broadcast_tx.subscribe();

    tracing::info!("websocket client connected");

    let mut send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if !filter.matches(&msg) {
                        continue;
                    }
                    let ws_msg = config_transport::websocket::WsMessage {
                        msg_type: config_transport::websocket::WsMessageType::Change,
                        event: Some(msg),
                        gap_from: None,
                    };
                    let json = match serde_json::to_string(&ws_msg) {
                        Ok(j) => j,
                        Err(e) => {
                            tracing::error!(error = %e, "failed to serialize ws message");
                            continue;
                        }
                    };
                    if sender.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    let gap_msg = config_transport::websocket::WsMessage {
                        msg_type: config_transport::websocket::WsMessageType::Gap,
                        event: None,
                        gap_from: Some(Uuid::nil()),
                    };
                    let json = serde_json::to_string(&gap_msg).unwrap_or_default();
                    if sender.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                    tracing::warn!(skipped = n, "websocket client lagging");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if msg == Message::Close(None) || matches!(msg, Message::Text(_) | Message::Binary(_)) {
                // Client sent a message or closed
            }
        }
    });

    tokio::select! {
        _ = (&mut send_task) => { recv_task.abort(); }
        _ = (&mut recv_task) => { send_task.abort(); }
    }

    tracing::info!("websocket client disconnected");
}

async fn file_stat_handler(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> HandlerResponse {
    let host_id = match body.get("host_id").and_then(|v| v.as_str()).and_then(|s| Uuid::parse_str(s).ok()) {
        Some(id) => id,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "missing host_id"}))),
    };
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "missing path"}))),
    };

    let host = match config_storage::repositories::hosts::HostsRepo::get(state.db.pool(), host_id).await {
        Ok(Some(h)) => h,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "host not found"}))),
        Err(e) => {
            tracing::error!(error = %e, "failed to lookup host");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "internal error"})));
        }
    };

    if host.status == "offline" {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error": "host is offline"})));
    }

    // Try tunnel first
    if state.tunnel_registry.is_connected(host_id) {
        let request_id = Uuid::new_v4().to_string();
        let tunnel_msg = config_transport::tunnel::TunnelMessage::query_request(
            request_id.clone(),
            config_transport::tunnel::QueryKind::Stat,
            path.to_string(),
        );

        match state.tunnel_registry.send_query(host_id, request_id, &tunnel_msg) {
            Ok(rx) => {
                match tokio::time::timeout(
                    Duration::from_secs(state.query_timeout_secs),
                    rx,
                ).await {
                    Ok(Ok(response)) => {
                        state.metrics.tunnel_queries_routed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let _ = config_storage::repositories::file_queries::FileQueriesRepo::insert(
                            state.db.pool(),
                            Uuid::new_v4(),
                            "operator",
                            host_id,
                            path,
                            "stat",
                            &response.status,
                        ).await;
                        return match response.status.as_str() {
                            "success" => (StatusCode::OK, Json(response.data.unwrap_or_default())),
                            "denied" => (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": response.error.unwrap_or_default()}))),
                            _ => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": response.error.unwrap_or_default()}))),
                        };
                    }
                    Ok(Err(_)) | Err(_) => {
                        state.metrics.tunnel_queries_fallback.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tracing::warn!(host_id = %host_id, "tunnel query failed or timed out, falling back to HTTP");
                    }
                }
            }
            Err(e) => {
                state.metrics.tunnel_queries_fallback.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tracing::warn!(host_id = %host_id, error = %e, "tunnel send failed, falling back to HTTP");
            }
        }
    }

    // Fall back to direct HTTP
    let agent_addr = format!("{}:9090", host.hostname);
    let query_client = config_transport::agent_query::AgentQueryClient::new();

    match query_client.query_stat(&agent_addr, path).await {
        Ok(result) => {
            let _ = config_storage::repositories::file_queries::FileQueriesRepo::insert(
                state.db.pool(),
                Uuid::new_v4(),
                "operator",
                host_id,
                path,
                "stat",
                "success",
            ).await;
            (StatusCode::OK, Json(result))
        }
        Err(e) => {
            let msg = e.to_string();
            let _ = config_storage::repositories::file_queries::FileQueriesRepo::insert(
                state.db.pool(),
                Uuid::new_v4(),
                "operator",
                host_id,
                path,
                "stat",
                "denied",
            ).await;
            if msg.contains("denied") {
                (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": msg})))
            } else {
                (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": format!("agent query failed: {}", msg)})))
            }
        }
    }
}

async fn file_preview_handler(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> HandlerResponse {
    let host_id = match body.get("host_id").and_then(|v| v.as_str()).and_then(|s| Uuid::parse_str(s).ok()) {
        Some(id) => id,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "missing host_id"}))),
    };
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "missing path"}))),
    };

    let host = match config_storage::repositories::hosts::HostsRepo::get(state.db.pool(), host_id).await {
        Ok(Some(h)) => h,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "host not found"}))),
        Err(e) => {
            tracing::error!(error = %e, "failed to lookup host");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "internal error"})));
        }
    };

    if host.status == "offline" {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error": "host is offline"})));
    }

    // Try tunnel first
    if state.tunnel_registry.is_connected(host_id) {
        let request_id = Uuid::new_v4().to_string();
        let tunnel_msg = config_transport::tunnel::TunnelMessage::query_request(
            request_id.clone(),
            config_transport::tunnel::QueryKind::Preview,
            path.to_string(),
        );

        match state.tunnel_registry.send_query(host_id, request_id, &tunnel_msg) {
            Ok(rx) => {
                match tokio::time::timeout(
                    Duration::from_secs(state.query_timeout_secs),
                    rx,
                ).await {
                    Ok(Ok(response)) => {
                        state.metrics.tunnel_queries_routed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let _ = config_storage::repositories::file_queries::FileQueriesRepo::insert(
                            state.db.pool(),
                            Uuid::new_v4(),
                            "operator",
                            host_id,
                            path,
                            "preview",
                            &response.status,
                        ).await;
                        return match response.status.as_str() {
                            "success" => (StatusCode::OK, Json(response.data.unwrap_or_default())),
                            "denied" => (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": response.error.unwrap_or_default()}))),
                            _ => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": response.error.unwrap_or_default()}))),
                        };
                    }
                    Ok(Err(_)) | Err(_) => {
                        state.metrics.tunnel_queries_fallback.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tracing::warn!(host_id = %host_id, "tunnel query failed or timed out, falling back to HTTP");
                    }
                }
            }
            Err(e) => {
                state.metrics.tunnel_queries_fallback.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tracing::warn!(host_id = %host_id, error = %e, "tunnel send failed, falling back to HTTP");
            }
        }
    }

    // Fall back to direct HTTP
    let agent_addr = format!("{}:9090", host.hostname);
    let query_client = config_transport::agent_query::AgentQueryClient::new();

    match query_client.query_preview(&agent_addr, path).await {
        Ok(result) => {
            let _ = config_storage::repositories::file_queries::FileQueriesRepo::insert(
                state.db.pool(),
                Uuid::new_v4(),
                "operator",
                host_id,
                path,
                "preview",
                "success",
            ).await;
            (StatusCode::OK, Json(result))
        }
        Err(e) => {
            let msg = e.to_string();
            let _ = config_storage::repositories::file_queries::FileQueriesRepo::insert(
                state.db.pool(),
                Uuid::new_v4(),
                "operator",
                host_id,
                path,
                "preview",
                "denied",
            ).await;
            if msg.contains("denied") {
                (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": msg})))
            } else {
                (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": format!("agent query failed: {}", msg)})))
            }
        }
    }
}

async fn file_content_handler(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> HandlerResponse {
    let host_id = match body.get("host_id").and_then(|v| v.as_str()).and_then(|s| Uuid::parse_str(s).ok()) {
        Some(id) => id,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "missing host_id"}))),
    };
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "missing path"}))),
    };
    let offset = body.get("offset").and_then(|v| v.as_u64());
    let limit = body.get("limit").and_then(|v| v.as_u64());

    let host = match config_storage::repositories::hosts::HostsRepo::get(state.db.pool(), host_id).await {
        Ok(Some(h)) => h,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "host not found"}))),
        Err(e) => {
            tracing::error!(error = %e, "failed to lookup host");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "internal error"})));
        }
    };

    if host.status == "offline" {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error": "host is offline"})));
    }

    // Try tunnel first
    if state.tunnel_registry.is_connected(host_id) {
        let request_id = Uuid::new_v4().to_string();
        let tunnel_msg = config_transport::tunnel::TunnelMessage::content_query_request(
            request_id.clone(),
            path.to_string(),
            offset,
            limit,
        );

        match state.tunnel_registry.send_query(host_id, request_id, &tunnel_msg) {
            Ok(rx) => {
                match tokio::time::timeout(
                    Duration::from_secs(state.query_timeout_secs.max(30)),
                    rx,
                ).await {
                    Ok(Ok(response)) => {
                        state.metrics.tunnel_queries_routed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        return match response.status.as_str() {
                            "success" => (StatusCode::OK, Json(response.data.unwrap_or_default())),
                            "denied" => (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": response.error.unwrap_or_default()}))),
                            _ => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": response.error.unwrap_or_default()}))),
                        };
                    }
                    Ok(Err(_)) | Err(_) => {
                        state.metrics.tunnel_queries_fallback.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tracing::warn!(host_id = %host_id, "tunnel content query failed or timed out, falling back to HTTP");
                    }
                }
            }
            Err(e) => {
                state.metrics.tunnel_queries_fallback.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tracing::warn!(host_id = %host_id, error = %e, "tunnel send failed, falling back to HTTP");
            }
        }
    }

    // Fall back to direct HTTP
    let agent_addr = format!("{}:9090", host.hostname);
    let query_client = config_transport::agent_query::AgentQueryClient::new();

    match query_client.query_content(&agent_addr, path, offset, limit).await {
        Ok(result) => (StatusCode::OK, Json(result)),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("denied") {
                (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": msg})))
            } else if msg.contains("too large") {
                (StatusCode::PAYLOAD_TOO_LARGE, Json(serde_json::json!({"error": msg})))
            } else {
                (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": format!("agent query failed: {}", msg)})))
            }
        }
    }
}

async fn github_file_content_handler(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> HandlerResponse {
    let url = match body.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "missing url"}))),
    };

    let token = body
        .get("github_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| state.github_token.clone());

    let (owner, repo, branch, path) = match config_workflow::github_client::parse_github_blob_url(url) {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))),
    };

    match config_workflow::github_client::fetch_file_contents(
        token.as_deref(),
        &owner,
        &repo,
        &path,
        &branch,
    ).await {
        Ok(file) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "path": file.path,
                "content": file.content,
                "size_bytes": file.size_bytes,
                "sha": file.sha,
            })),
        ),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": msg})))
            } else if msg.contains("auth") {
                (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": msg})))
            } else {
                (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": format!("github request failed: {}", msg)})))
            }
        }
    }
}

async fn metrics_handler(
    State(state): State<AppState>,
) -> HandlerResponse {
    (StatusCode::OK, Json(state.metrics.snapshot()))
}

// --- Workflow handlers ---

#[derive(serde::Deserialize)]
struct CreateWorkflowRequest {
    repo_url: String,
    branch_name: String,
    base_branch: Option<String>,
    pr_title: String,
    pr_description: Option<String>,
    file_changes: Vec<config_workflow::models::FileChange>,
    reviewers: Option<Vec<String>>,
    github_token: Option<String>,
    event_ids: Option<Vec<Uuid>>,
}

async fn create_workflow_handler(
    State(state): State<AppState>,
    Json(body): Json<CreateWorkflowRequest>,
) -> HandlerResponse {
    let base_branch = body.base_branch.clone().unwrap_or_else(|| "main".to_string());
    let workflow_id = Uuid::new_v4();
    let run = config_workflow::models::WorkflowRun {
        workflow_id,
        repo_url: body.repo_url.clone(),
        branch_name: body.branch_name.clone(),
        base_branch: base_branch.clone(),
        pr_title: body.pr_title.clone(),
        pr_description: body.pr_description.clone(),
        file_changes: body.file_changes.clone(),
        reviewers: body.reviewers.clone(),
        repos_dir: state.repos_dir.clone(),
        github_token: body.github_token.clone(),
        event_ids: body.event_ids.clone().unwrap_or_default(),
    };

    let workflow_row = config_storage::models::WorkflowRow {
        workflow_id,
        status: "pending".to_string(),
        repo_url: body.repo_url,
        branch_name: body.branch_name,
        base_branch,
        pr_title: body.pr_title,
        pr_description: body.pr_description,
        file_changes_json: serde_json::to_value(&body.file_changes).unwrap_or_default(),
        error_message: None,
        pr_url: None,
        reviewers_json: body.reviewers.as_ref().map(|r| serde_json::to_value(r).unwrap_or_default()),
        event_ids_json: body.event_ids.as_ref().map(|ids| serde_json::to_value(ids).unwrap_or_default()),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    match config_storage::repositories::workflows::WorkflowsRepo::insert(state.db.pool(), &workflow_row).await {
        Ok(_) => {}
        Err(e) => {
            tracing::error!(error = %e, "failed to insert workflow");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "failed to create workflow"})));
        }
    }

    // Spawn background task
    let pool = state.db.pool().clone();
    let resolver = std::sync::Arc::new(config_workflow::content_resolver::SnapshotContentResolver::new(
        state.snapshot_store.clone(),
    ));

    tokio::spawn(async move {
        config_workflow::executor::run_workflow(run, pool, resolver).await;
    });

    (StatusCode::ACCEPTED, Json(serde_json::json!({"workflow_id": workflow_id})))
}

async fn get_workflow_handler(
    State(state): State<AppState>,
    axum::extract::Path(workflow_id): axum::extract::Path<Uuid>,
) -> HandlerResponse {
    match config_storage::repositories::workflows::WorkflowsRepo::get(state.db.pool(), workflow_id).await {
        Ok(Some(row)) => (StatusCode::OK, Json(serde_json::json!({"workflow": row}))),
        Ok(None) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "workflow not found"}))),
        Err(e) => {
            tracing::error!(error = %e, "get workflow failed");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "internal error"})))
        }
    }
}

async fn list_workflows_handler(
    State(state): State<AppState>,
    axum::extract::Query(pagination): axum::extract::Query<Pagination>,
) -> HandlerResponse {
    match config_storage::repositories::workflows::WorkflowsRepo::list(
        state.db.pool(),
        pagination.limit,
        pagination.offset,
    )
    .await
    {
        Ok(workflows) => (StatusCode::OK, Json(serde_json::json!({"workflows": workflows}))),
        Err(e) => {
            tracing::error!(error = %e, "list workflows failed");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "internal error"})))
        }
    }
}