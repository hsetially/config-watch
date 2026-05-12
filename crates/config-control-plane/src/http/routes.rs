use std::collections::HashMap;
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, State, WebSocketUpgrade,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use better_auth::HttpMethod;
use chrono::{DateTime, Utc};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::http::extractors::{
    AgentAuth, AuthenticatedUser, ChangeFilters, CorrelationId, CsrfProtected, Pagination,
    WsAuthenticatedUser,
};
use crate::realtime::SubscriptionFilter;
use crate::services::{AppState, AuthState};

/// Constant-time comparison for secrets. Falls back to standard comparison
/// for empty/unequal-length strings (which leaks only length, not content).
fn verify_secret_constant_time(provided: &str, expected: &str) -> bool {
    if provided.is_empty() || expected.is_empty() {
        return false;
    }
    constant_time_eq::constant_time_eq(provided.as_bytes(), expected.as_bytes())
}

pub fn build_router(state: AppState, auth_state: AuthState) -> Router {
    let auth_router = Router::new()
        .fallback(auth_proxy_handler)
        .with_state((state.clone(), auth_state.clone()));

    Router::new()
        .route("/v1/agents/register", post(register_handler))
        .route("/v1/agents/heartbeat", post(heartbeat_handler))
        .route("/v1/agents/tunnel", get(agent_tunnel_handler))
        .route("/v1/events/change", post(change_ingest_handler))
        .route("/v1/hosts", get(hosts_list_handler))
        .route("/v1/hosts/{host_id}", get(host_detail_handler))
        .route("/v1/hosts/{host_id}/roots", get(host_roots_handler))
        .route("/v1/changes", get(changes_list_handler))
        .route("/v1/changes/{event_id}", get(change_detail_handler))
        .route("/v1/changes/{event_id}/diff", get(change_diff_handler))
        .route("/v1/changes/stream", get(changes_stream_handler))
        .route("/v1/file/stat", post(file_stat_handler))
        .route("/v1/file/preview", post(file_preview_handler))
        .route("/v1/file/content", post(file_content_handler))
        .route("/v1/github/file-content", post(github_file_content_handler))
        .route("/v1/ws-ticket", post(ws_ticket_handler))
        .route(
            "/v1/workflows",
            post(create_workflow_handler).get(list_workflows_handler),
        )
        // L8: unauthenticated health endpoint for k8s/Istio probes
        .route("/healthz", get(healthz_handler))
        .route("/v1/workflows/{workflow_id}", get(get_workflow_handler))
        .route("/v1/metrics", get(metrics_handler))
        // Admin approval endpoints
        .route("/v1/admin/approve-user", post(approve_user_handler))
        .route("/v1/admin/reject-user", post(reject_user_handler))
        .route("/v1/admin/pending-users", get(pending_users_handler))
        .route("/v1/admin/set-role", post(set_role_handler))
        .with_state(state)
        .nest("/auth", auth_router)
}

/// Proxy handler that forwards all /auth/* requests to BetterAuth.
/// Post-processes sign-up responses to mark new users as pending approval,
/// and sign-in responses to track last login info.
async fn auth_proxy_handler(
    State((state, auth)): State<(AppState, AuthState)>,
    req: axum::extract::Request,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();

    // Convert HTTP method
    let method = match parts.method {
        axum::http::Method::GET => HttpMethod::Get,
        axum::http::Method::POST => HttpMethod::Post,
        axum::http::Method::PUT => HttpMethod::Put,
        axum::http::Method::DELETE => HttpMethod::Delete,
        axum::http::Method::PATCH => HttpMethod::Patch,
        axum::http::Method::OPTIONS => HttpMethod::Options,
        axum::http::Method::HEAD => HttpMethod::Head,
        _ => {
            return (StatusCode::METHOD_NOT_ALLOWED, "Unsupported method").into_response();
        }
    };

    // Strip /auth prefix from path
    let path = parts
        .uri
        .path()
        .strip_prefix("/auth")
        .unwrap_or(parts.uri.path());

    // Capture request metadata for login tracking
    let request_ip = parts
        .headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .or_else(|| {
            parts
                .headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        });
    let request_ua = parts
        .headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Convert headers — validate Origin for mutating routes (C3 fix).
    // We no longer rewrite Origin/Referer to suppress BetterAuth's CSRF defense.
    let trusted_origins = &auth.config().trusted_origins;
    let request_origin = parts
        .headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let is_signup = method == HttpMethod::Post && path == "/sign-up/email";
    let is_signin = method == HttpMethod::Post && path == "/sign-in/email";

    // For non-GET mutating requests, require Origin to be in trusted_origins.
    let is_mutating = matches!(
        method,
        HttpMethod::Post | HttpMethod::Put | HttpMethod::Delete | HttpMethod::Patch
    );
    if is_mutating {
        if let Some(ref origin) = request_origin {
            if !trusted_origins.iter().any(|t| t == origin) {
                tracing::warn!(origin = %origin, "rejecting request with untrusted origin");
                return (
                    StatusCode::FORBIDDEN,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    serde_json::json!({"error": "untrusted_origin", "message": "Request origin is not allowed"}).to_string(),
                )
                    .into_response();
            }
        } else if !is_signup && !is_signin {
            // Non-login mutating requests must have an Origin header.
            tracing::warn!("rejecting mutating request without origin header");
            return (
                StatusCode::FORBIDDEN,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"error": "missing_origin", "message": "Origin header required"})
                    .to_string(),
            )
                .into_response();
        }
    }

    let mut headers = HashMap::new();
    for (name, value) in parts.headers.iter() {
        if let Ok(value_str) = value.to_str() {
            headers.insert(name.to_string(), value_str.to_string());
        }
    }

    // Convert query parameters
    let mut query = HashMap::new();
    if let Some(query_str) = parts.uri.query() {
        for (key, value) in url::form_urlencoded::parse(query_str.as_bytes()) {
            query.insert(key.to_string(), value.to_string());
        }
    }

    // Read body
    let max_bytes = if auth.body_limit_config().enabled {
        auth.body_limit_config().max_bytes
    } else {
        1024 * 1024
    };
    let body_bytes = match axum::body::to_bytes(body, max_bytes).await {
        Ok(bytes) => {
            if bytes.is_empty() {
                None
            } else {
                Some(bytes.to_vec())
            }
        }
        Err(_) => return (StatusCode::BAD_REQUEST, "Failed to read request body").into_response(),
    };

    let auth_req =
        better_auth::AuthRequest::from_parts(method, path.to_string(), headers, body_bytes, query);

    match auth.handle_request(auth_req).await {
        Ok(auth_response) => {
            let status = StatusCode::from_u16(auth_response.status)
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

            // Post-process successful sign-up: mark user as pending approval
            // H2 fix: also delete the auto-created session when approval is required.
            // Return 403 so the frontend shows the "Account Pending Approval" screen
            // instead of letting the 200 response through and showing a broken dashboard.
            if is_signup && state.require_approval && status == StatusCode::OK {
                if let Ok(body_json) =
                    serde_json::from_slice::<serde_json::Value>(&auth_response.body)
                {
                    if let Some(user_id) = body_json
                        .get("user")
                        .and_then(|u| u.get("id"))
                        .and_then(|id| id.as_str())
                    {
                        let pool = state.db.pool();
                        let user_id_owned = user_id.to_string();
                        // Mark user as banned with pending_approval role
                        if let Err(e) = sqlx::query(
                            "UPDATE users SET banned = true, role = 'pending_approval', updated_at = NOW() WHERE id = $1",
                        )
                        .bind(&user_id_owned)
                        .execute(pool)
                        .await
                        {
                            tracing::warn!(error = %e, user_id = %user_id_owned, "failed to mark new user as pending approval");
                        } else {
                            tracing::info!(user_id = %user_id_owned, "new user marked as pending approval");
                            // Delete the auto-created session so the pending user
                            // cannot use it — they must wait for admin approval.
                            let _ = sqlx::query("DELETE FROM sessions WHERE user_id = $1")
                                .bind(&user_id_owned)
                                .execute(pool)
                                .await;
                        }

                        // Always return 403 for approval-required sign-ups so the
                        // frontend shows the "Account Pending Approval" screen
                        // instead of redirecting to a broken dashboard.
                        let forbidden_body = serde_json::json!({
                            "error": "approval_pending",
                            "message": "Account awaiting admin approval"
                        });
                        return (
                            StatusCode::FORBIDDEN,
                            [(axum::http::header::CONTENT_TYPE, "application/json")],
                            serde_json::to_string(&forbidden_body).unwrap_or_default(),
                        )
                            .into_response();
                    }
                }
            }

            // Post-process successful sign-in: check banned/role, track login info
            if is_signin && status == StatusCode::OK {
                if let Ok(body_json) =
                    serde_json::from_slice::<serde_json::Value>(&auth_response.body)
                {
                    if let Some(user_id) = body_json
                        .get("user")
                        .and_then(|u| u.get("id"))
                        .and_then(|id| id.as_str())
                    {
                        let pool = state.db.pool();
                        let user_id_owned = user_id.to_string();

                        // Check if user is banned or lacks required role
                        let user_check = sqlx::query_as::<_, (bool, Option<String>)>(
                            "SELECT banned, role FROM users WHERE id = $1",
                        )
                        .bind(&user_id_owned)
                        .fetch_optional(pool)
                        .await;

                        if let Ok(Some((banned, role))) = user_check {
                            if banned {
                                let error_type = if role.as_deref() == Some("pending_approval") {
                                    "approval_pending"
                                } else {
                                    "banned"
                                };
                                let message = if role.as_deref() == Some("pending_approval") {
                                    "Account awaiting admin approval".to_string()
                                } else {
                                    "Account has been banned".to_string()
                                };

                                // Revoke the session BetterAuth just created
                                let _ = sqlx::query("DELETE FROM sessions WHERE user_id = $1")
                                    .bind(&user_id_owned)
                                    .execute(pool)
                                    .await;

                                tracing::info!(user_id = %user_id_owned, role = ?role, "sign-in blocked: user is banned");
                                let body =
                                    serde_json::json!({"error": error_type, "message": message});
                                return (
                                    StatusCode::FORBIDDEN,
                                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                                    serde_json::to_string(&body).unwrap_or_default(),
                                )
                                    .into_response();
                            }
                            if !matches!(role.as_deref(), Some("admin") | Some("user")) {
                                // Revoke the session BetterAuth just created
                                let _ = sqlx::query("DELETE FROM sessions WHERE user_id = $1")
                                    .bind(&user_id_owned)
                                    .execute(pool)
                                    .await;

                                tracing::info!(user_id = %user_id_owned, role = ?role, "sign-in blocked: insufficient role");
                                let body = serde_json::json!({"error": "insufficient_role", "message": "Account does not have required role"});
                                return (
                                    StatusCode::FORBIDDEN,
                                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                                    serde_json::to_string(&body).unwrap_or_default(),
                                )
                                    .into_response();
                            }
                        }

                        // Track last login info
                        let ip = request_ip.as_deref().unwrap_or("unknown");
                        let ua = request_ua.as_deref().unwrap_or("");
                        if let Err(e) = sqlx::query(
                            "UPDATE users SET last_login_at = NOW(), last_login_ip = $1, last_login_user_agent = $2, updated_at = NOW() WHERE id = $3",
                        )
                        .bind(ip)
                        .bind(ua)
                        .bind(&user_id_owned)
                        .execute(pool)
                        .await
                        {
                            tracing::warn!(error = %e, user_id = %user_id_owned, "failed to update last login info");
                        }
                    }
                }
            }

            let mut response = axum::http::Response::builder().status(status);

            // H2 fix: strip Set-Cookie headers when returning 403 so the
            // browser never receives a valid session cookie for banned/pending users.
            let is_forbidden = status == StatusCode::FORBIDDEN;

            for (name, value) in auth_response.headers {
                if let (Ok(header_name), Ok(header_value)) = (
                    axum::http::HeaderName::from_bytes(name.as_bytes()),
                    axum::http::HeaderValue::from_str(&value),
                ) {
                    if is_forbidden && header_name == axum::http::header::SET_COOKIE {
                        continue;
                    }
                    response = response.header(header_name, header_value);
                }
            }

            // C4: On successful sign-in/sign-up, inject a CSRF double-submit cookie.
            // Also strip the "token" field from the response body so JS never sees
            // the session token (it's now in an HttpOnly cookie set by BetterAuth).
            let is_success = status == StatusCode::OK;
            let should_inject_csrf = is_success && (is_signin || is_signup);
            let should_strip_token = is_success && (is_signin || is_signup);

            let response_body = if should_strip_token {
                if let Ok(mut body_json) =
                    serde_json::from_slice::<serde_json::Value>(&auth_response.body)
                {
                    if let Some(obj) = body_json.as_object_mut() {
                        obj.remove("token");
                    }
                    serde_json::to_vec(&body_json).unwrap_or(auth_response.body)
                } else {
                    auth_response.body
                }
            } else {
                auth_response.body
            };

            if should_inject_csrf {
                let csrf_token = Uuid::new_v4().to_string();
                let csrf_cookie = if state.tls_required {
                    format!(
                        "config_watch_csrf={}; Secure; SameSite=Strict; Path=/",
                        csrf_token
                    )
                } else {
                    format!("config_watch_csrf={}; SameSite=Strict; Path=/", csrf_token)
                };
                if let (Ok(header_name), Ok(header_value)) = (
                    axum::http::HeaderName::from_bytes(b"set-cookie"),
                    axum::http::HeaderValue::from_str(&csrf_cookie),
                ) {
                    response = response.header(header_name, header_value);
                }
            }

            response
                .body(axum::body::Body::from(response_body))
                .unwrap_or_else(|_| {
                    axum::http::Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(axum::body::Body::from("Internal server error"))
                        .unwrap()
                })
                .into_response()
        }
        Err(err) => {
            let status = StatusCode::from_u16(err.status_code())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let message = match err.status_code() {
                500 => "Internal server error".to_string(),
                _ => err.to_string(),
            };
            (status, Json(serde_json::json!({"message": message}))).into_response()
        }
    }
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
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "missing enrollment token"})),
        );
    }

    // In v1, enrollment token = control plane secret (simple approach)
    // L6 fix: use constant-time comparison
    if !verify_secret_constant_time(enrollment_token, &state.secret) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid enrollment token"})),
        );
    }

    let host_id = body
        .get("host_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok());

    let Some(host_id) = host_id else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "missing host_id"})),
        );
    };

    let hostname = body
        .get("hostname")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let environment = body
        .get("environment")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let agent_version = body
        .get("agent_version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.1.0");
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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "register failed"})),
            )
        }
    }
}

async fn heartbeat_handler(
    State(state): State<AppState>,
    auth: AgentAuth,
    _cid: CorrelationId,
    Json(body): Json<serde_json::Value>,
) -> HandlerResponse {
    let host_id = body
        .get("host_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok());

    // H10 fix: reject if body.host_id doesn't match the authenticated identity
    if let Some(id) = host_id {
        if id.to_string() != auth.host_id {
            let msg = format!(
                "host_id mismatch: authenticated as {} but requested {}",
                auth.host_id, id
            );
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": msg})),
            );
        }
    }

    let Some(host_id) = host_id else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "missing host_id"})),
        );
    };

    match config_storage::repositories::hosts::HostsRepo::heartbeat(state.db.pool(), host_id).await
    {
        Ok(()) => (StatusCode::NO_CONTENT, Json(serde_json::json!({}))),
        Err(e) => {
            tracing::error!(error = %e, "heartbeat failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "heartbeat failed"})),
            )
        }
    }
}

async fn agent_tunnel_handler(
    State(state): State<AppState>,
    auth: AgentAuth,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // M4 fix: reject malformed host_id instead of falling back to nil UUID
    let host_id: Uuid = match auth.host_id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid host_id in authentication"})),
            )
                .into_response();
        }
    };
    ws.on_upgrade(move |socket| crate::tunnel::handle_agent_tunnel(socket, state, host_id))
}

async fn change_ingest_handler(
    State(state): State<AppState>,
    _auth: AgentAuth,
    _cid: CorrelationId,
    Json(body): Json<serde_json::Value>,
) -> HandlerResponse {
    match crate::ingest::IngestService::ingest_change(
        state.db.pool(),
        &state.broadcast_tx,
        &state.local_event_dedup,
        &state.snapshot_store,
        body,
    )
    .await
    {
        Ok(crate::ingest::IngestOutcome::Accepted { event_id }) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"accepted": true, "event_id": event_id})),
        ),
        Ok(crate::ingest::IngestOutcome::Duplicate { event_id }) => (
            StatusCode::CONFLICT,
            Json(
                serde_json::json!({"accepted": true, "event_id": event_id, "message": "duplicate"}),
            ),
        ),
        Ok(crate::ingest::IngestOutcome::Rejected { reason }) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"accepted": false, "error": reason})),
        ),
        Err(e) => {
            tracing::error!(error = %e, "ingest failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"accepted": false, "error": "ingest failed"})),
            )
        }
    }
}

async fn hosts_list_handler(
    State(state): State<AppState>,
    _user: AuthenticatedUser,
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
    _user: AuthenticatedUser,
    axum::extract::Path(host_id): axum::extract::Path<Uuid>,
) -> HandlerResponse {
    match config_storage::repositories::hosts::HostsRepo::get(state.db.pool(), host_id).await {
        Ok(Some(row)) => (StatusCode::OK, Json(serde_json::json!({"host": row}))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "host not found"})),
        ),
        Err(e) => {
            tracing::error!(error = %e, "get host failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            )
        }
    }
}

async fn host_roots_handler(
    State(state): State<AppState>,
    _user: AuthenticatedUser,
    axum::extract::Path(host_id): axum::extract::Path<Uuid>,
) -> HandlerResponse {
    match config_storage::repositories::watch_roots::WatchRootsRepo::list_by_host(
        state.db.pool(),
        host_id,
    )
    .await
    {
        Ok(roots) => (StatusCode::OK, Json(serde_json::json!({"roots": roots}))),
        Err(e) => {
            tracing::error!(error = %e, "get host roots failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            )
        }
    }
}

async fn changes_list_handler(
    State(state): State<AppState>,
    _user: AuthenticatedUser,
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
        host_id: filters
            .host_id
            .as_ref()
            .and_then(|s| Uuid::parse_str(s).ok()),
        path_prefix: filters.path_prefix.clone(),
        filename: filters.filename.clone(),
        author: filters.author.clone(),
        severity: filters.severity.clone(),
        since,
        until,
        event_kind_exclude: if filters.initial.as_deref() == Some("true") {
            vec![]
        } else {
            vec!["initial_snapshot".to_string()]
        },
    };

    let total =
        config_storage::repositories::change_events::ChangeEventsRepo::count(state.db.pool(), &f)
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

    (
        StatusCode::OK,
        Json(serde_json::json!({"changes": changes, "total": total})),
    )
}

async fn change_detail_handler(
    State(state): State<AppState>,
    _user: AuthenticatedUser,
    axum::extract::Path(event_id): axum::extract::Path<Uuid>,
) -> HandlerResponse {
    match config_storage::repositories::change_events::ChangeEventsRepo::get(
        state.db.pool(),
        event_id,
    )
    .await
    {
        Ok(Some(row)) => (StatusCode::OK, Json(serde_json::json!({"event": row}))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "event not found"})),
        ),
        Err(e) => {
            tracing::error!(error = %e, "get event failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            )
        }
    }
}

/// Lazy server-side diff. Looks up the change event, fetches both snapshot
/// revisions from the agent's snapshot store via the agent-query API, and
/// renders with the configured `DiffEngine`. Subsequent calls for the same
/// `(event_id, format)` are served from the in-memory LRU.
///
/// Failure modes the dashboard relies on:
/// - 404: event_id unknown
/// - 503: host offline or unreachable
/// - 410: both snapshots evicted by retention (no diff possible)
/// - 200 with `previous_unavailable=true`: previous evicted, current shown
///
/// The configured format is the only one rendered for now; a per-request
/// `?format=` override comes with the dashboard format dropdown work.
///
/// Query parameters:
/// - `format` (optional): override the server-configured diff format.
///   Accepted values: `unified`, `context`, `full_file`, `side_by_side`, `raw`.
#[derive(Deserialize)]
struct DiffQuery {
    #[serde(default)]
    format: Option<String>,
}

async fn change_diff_handler(
    State(state): State<AppState>,
    _user: AuthenticatedUser,
    axum::extract::Path(event_id): axum::extract::Path<Uuid>,
    Query(query): Query<DiffQuery>,
) -> HandlerResponse {
    let requested_format = query
        .format
        .as_deref()
        .and_then(crate::diff_service::DiffService::parse_format_label);

    let format_label = match requested_format {
        Some(ref fmt) => crate::diff_service::format_label_for(fmt).to_string(),
        None => state.diff_service.default_format_label().to_string(),
    };

    if let Some(cached) = state.diff_service.cache_get(event_id, &format_label) {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "event_id": event_id,
                "render": cached.render,
                "added": cached.added,
                "removed": cached.removed,
                "format": cached.format_label,
                "cached": true,
            })),
        );
    }

    let event = match config_storage::repositories::change_events::ChangeEventsRepo::get(
        state.db.pool(),
        event_id,
    )
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "event not found"})),
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "get event failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            );
        }
    };

    let path = match event.canonical_path.as_deref() {
        Some(p) => p.to_string(),
        None => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"error": "event has no canonical_path"})),
            )
        }
    };

    let hashes = match config_storage::repositories::change_events::ChangeEventsRepo::
        get_content_hashes_by_event_ids(state.db.pool(), &[event_id])
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "fetch content hashes failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            );
        }
    };
    let (curr_hash_opt, prev_hash_opt) = match hashes.into_iter().next() {
        Some((_path, curr, prev)) => (curr, prev),
        None => (None, None),
    };

    let host =
        match config_storage::repositories::hosts::HostsRepo::get(state.db.pool(), event.host_id)
            .await
        {
            Ok(Some(h)) => h,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "host not found"})),
                )
            }
            Err(e) => {
                tracing::error!(error = %e, "lookup host failed");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "internal error"})),
                );
            }
        };
    if host.status == "offline" {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "host is offline", "host_id": event.host_id})),
        );
    }

    let host_id = event.host_id;

    // Fetch both revisions in parallel, trying tunnel first then falling back
    // to direct HTTP. Each may independently come back as SnapshotGone.
    let prev_fut =
        fetch_snapshot_via_tunnel_or_http(&state, host_id, &path, prev_hash_opt.as_deref());
    let curr_fut =
        fetch_snapshot_via_tunnel_or_http(&state, host_id, &path, curr_hash_opt.as_deref());
    let (prev_res, curr_res) = tokio::join!(prev_fut, curr_fut);

    let (previous, prev_status) = unwrap_snapshot_fetch(prev_res, "previous");
    let (current, curr_status) = unwrap_snapshot_fetch(curr_res, "current");

    if matches!(prev_status, FetchStatus::Unreachable)
        || matches!(curr_status, FetchStatus::Unreachable)
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "agent unreachable", "host_id": event.host_id})),
        );
    }

    if matches!(prev_status, FetchStatus::Gone) && matches!(curr_status, FetchStatus::Gone) {
        return (
            StatusCode::GONE,
            Json(serde_json::json!({
                "error": "both snapshots evicted by retention; cannot render diff",
                "event_id": event_id,
            })),
        );
    }

    let path_utf8 = camino::Utf8PathBuf::from(&path);
    let render_result = match requested_format {
        Some(fmt) => {
            state
                .diff_service
                .render_with_format(&previous, &current, path_utf8.as_path(), fmt)
                .await
        }
        None => {
            state
                .diff_service
                .render(&previous, &current, path_utf8.as_path())
                .await
        }
    };

    let (render, added, removed) = match render_result {
        Ok(config_diff::difftastic::DiffOutput::Changed {
            render,
            added,
            removed,
        }) => (render, added, removed),
        Ok(config_diff::difftastic::DiffOutput::Unchanged) => {
            // Equal bytes — return an empty diff with the format prefix so the
            // dashboard renders consistently rather than a "no diff" stub.
            (String::new(), 0, 0)
        }
        Ok(config_diff::difftastic::DiffOutput::Error { message }) => {
            tracing::warn!(event_id = %event_id, message = %message, "diff render error");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": message})),
            );
        }
        Err(e) => {
            tracing::error!(event_id = %event_id, error = %e, "diff render panic");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            );
        }
    };

    state.diff_service.cache_put(
        event_id,
        &format_label,
        crate::diff_service::CachedRender {
            render: render.clone(),
            added,
            removed,
            format_label: format_label.clone(),
        },
    );

    let previous_unavailable = matches!(prev_status, FetchStatus::Gone);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "event_id": event_id,
            "render": render,
            "added": added,
            "removed": removed,
            "format": format_label,
            "previous_unavailable": previous_unavailable,
            "cached": false,
        })),
    )
}

#[derive(Debug)]
enum FetchStatus {
    Ok,
    Gone,
    Unreachable,
    NoHash,
}

/// Convert a snapshot-fetch result into a `(text, status)` pair used by the
/// diff handler. `Gone` and `NoHash` both render as empty strings — the diff
/// engine treats that as a created/deleted event, which is what we want.
fn unwrap_snapshot_fetch(
    res: Result<Option<String>, anyhow::Error>,
    label: &'static str,
) -> (String, FetchStatus) {
    match res {
        Ok(Some(text)) => (text, FetchStatus::Ok),
        Ok(None) => (String::new(), FetchStatus::NoHash),
        Err(e) => {
            if e.downcast_ref::<config_transport::agent_query::SnapshotGone>()
                .is_some()
            {
                tracing::info!(side = label, "snapshot evicted on agent");
                (String::new(), FetchStatus::Gone)
            } else {
                tracing::warn!(side = label, error = %e, "snapshot fetch failed");
                (String::new(), FetchStatus::Unreachable)
            }
        }
    }
}

/// Returns `Ok(Some(text))` on success, `Ok(None)` if the event has no hash on
/// that side (created/deleted event), or `Err` for any agent failure.
async fn fetch_snapshot_text(
    client: &config_transport::agent_query::AgentQueryClient,
    agent_addr: &str,
    path: &str,
    content_hash: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let Some(hash) = content_hash else {
        return Ok(None);
    };
    let revision = config_transport::agent_query::PreviewRevision::Snapshot {
        content_hash: hash.to_string(),
    };
    let value = client
        .query_preview_revision(agent_addr, path, revision)
        .await?;
    let text = value
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(Some(text))
}

/// Fetch a snapshot revision by trying the tunnel first, falling back to direct
/// HTTP. Returns the snapshot text wrapped in the same `Result<Option<String>>`
/// format as `fetch_snapshot_text`, but maps tunnel-level errors (including
/// `SnapshotGone`) into `anyhow::Error` so that `unwrap_snapshot_fetch` can
/// classify them uniformly.
async fn fetch_snapshot_via_tunnel_or_http(
    state: &AppState,
    host_id: Uuid,
    path: &str,
    content_hash: Option<&str>,
) -> Result<Option<String>, anyhow::Error> {
    let Some(hash) = content_hash else {
        return Ok(None);
    };

    // Try tunnel first if the agent is connected.
    if state.tunnel_registry.is_connected(host_id) {
        let request_id = Uuid::new_v4().to_string();
        let tunnel_msg = config_transport::tunnel::TunnelMessage::preview_revision_query_request(
            request_id.clone(),
            path.to_string(),
            hash.to_string(),
        );

        match state
            .tunnel_registry
            .send_query(host_id, request_id, &tunnel_msg)
        {
            Ok(rx) => {
                match tokio::time::timeout(Duration::from_secs(state.query_timeout_secs), rx).await
                {
                    Ok(Ok(response)) => {
                        match response.status.as_str() {
                            "success" => {
                                state
                                    .metrics
                                    .tunnel_queries_routed
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                let text = response
                                    .data
                                    .as_ref()
                                    .and_then(|v| v.get("content"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                return Ok(Some(text));
                            }
                            "gone" => {
                                state
                                    .metrics
                                    .tunnel_queries_routed
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                let detail = response.error.unwrap_or_default();
                                return Err(
                                    config_transport::agent_query::SnapshotGone(detail).into()
                                );
                            }
                            "denied" => {
                                state
                                    .metrics
                                    .tunnel_queries_routed
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                return Err(anyhow::anyhow!(
                                    "path denied by agent security policy"
                                ));
                            }
                            _ => {
                                // Unknown status from agent — fall through to HTTP
                                state
                                    .metrics
                                    .tunnel_queries_fallback
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                tracing::warn!(
                                    host_id = %host_id,
                                    status = %response.status,
                                    "unexpected tunnel response status, falling back to HTTP"
                                );
                            }
                        }
                    }
                    Ok(Err(_)) | Err(_) => {
                        state
                            .metrics
                            .tunnel_queries_fallback
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tracing::warn!(
                            host_id = %host_id,
                            "tunnel snapshot query failed or timed out, falling back to HTTP"
                        );
                    }
                }
            }
            Err(e) => {
                state
                    .metrics
                    .tunnel_queries_fallback
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tracing::warn!(
                    host_id = %host_id,
                    error = %e,
                    "tunnel send failed for snapshot query, falling back to HTTP"
                );
            }
        }
    }

    // Fall back to direct HTTP.
    let agent_addr = {
        let host = config_storage::repositories::hosts::HostsRepo::get(state.db.pool(), host_id)
            .await
            .map_err(|e| anyhow::anyhow!("failed to lookup host for HTTP fallback: {}", e))?;
        let host = host.ok_or_else(|| anyhow::anyhow!("host not found for HTTP fallback"))?;
        format!("{}:9090", host.hostname)
    };
    let client = config_transport::agent_query::AgentQueryClient::new();
    fetch_snapshot_text(&client, &agent_addr, path, Some(hash)).await
}

async fn changes_stream_handler(
    State(state): State<AppState>,
    _user: WsAuthenticatedUser,
    ws: WebSocketUpgrade,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let filter = SubscriptionFilter {
        environment: params.get("environment").cloned(),
        host_id: params.get("host_id").and_then(|s| Uuid::parse_str(s).ok()),
        path_prefix: params.get("path_prefix").cloned(),
        severity: params.get("severity").cloned(),
    };

    // H9: limit max message size to 1 MiB on WebSocket
    ws.max_message_size(1 << 20)
        .on_upgrade(move |socket| handle_ws(socket, state, filter))
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
                    if sender.send(Message::Text(json.into())).await.is_err() {
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
                    if sender.send(Message::Text(json.into())).await.is_err() {
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
            match msg {
                Message::Close(_) => break,
                Message::Text(_) | Message::Binary(_) => {}
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = (&mut send_task) => { recv_task.abort(); }
        _ = (&mut recv_task) => { send_task.abort(); }
    }

    tracing::info!("websocket client disconnected");
}

/// One-shot WebSocket ticket endpoint. Returns a stateless HMAC-signed ticket
/// that the dashboard can use in a `?ticket=` query parameter for WS authentication,
/// avoiding the need to send the session token in the URL (which leaks in logs).
/// The ticket is signed with the control-plane secret and works across pods.
async fn ws_ticket_handler(State(state): State<AppState>, user: CsrfProtected) -> HandlerResponse {
    let ticket = crate::ws_ticket::generate_ticket(&user.0.user_id, &state.secret);
    (StatusCode::OK, Json(serde_json::json!({"ticket": ticket})))
}

async fn file_stat_handler(
    State(state): State<AppState>,
    _user: CsrfProtected,
    Json(body): Json<serde_json::Value>,
) -> HandlerResponse {
    let host_id = match body
        .get("host_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
    {
        Some(id) => id,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "missing host_id"})),
            )
        }
    };
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "missing path"})),
            )
        }
    };

    let host =
        match config_storage::repositories::hosts::HostsRepo::get(state.db.pool(), host_id).await {
            Ok(Some(h)) => h,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "host not found"})),
                )
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to lookup host");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "internal error"})),
                );
            }
        };

    if host.status == "offline" {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "host is offline"})),
        );
    }

    // Try tunnel first
    if state.tunnel_registry.is_connected(host_id) {
        let request_id = Uuid::new_v4().to_string();
        let tunnel_msg = config_transport::tunnel::TunnelMessage::query_request(
            request_id.clone(),
            config_transport::tunnel::QueryKind::Stat,
            path.to_string(),
        );

        match state
            .tunnel_registry
            .send_query(host_id, request_id, &tunnel_msg)
        {
            Ok(rx) => {
                match tokio::time::timeout(Duration::from_secs(state.query_timeout_secs), rx).await
                {
                    Ok(Ok(response)) => {
                        state
                            .metrics
                            .tunnel_queries_routed
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let _ =
                            config_storage::repositories::file_queries::FileQueriesRepo::insert(
                                state.db.pool(),
                                Uuid::new_v4(),
                                "operator",
                                host_id,
                                path,
                                "stat",
                                &response.status,
                            )
                            .await;
                        return match response.status.as_str() {
                            "success" => (StatusCode::OK, Json(response.data.unwrap_or_default())),
                            "denied" => (
                                StatusCode::FORBIDDEN,
                                Json(
                                    serde_json::json!({"error": response.error.unwrap_or_default()}),
                                ),
                            ),
                            _ => (
                                StatusCode::BAD_GATEWAY,
                                Json(
                                    serde_json::json!({"error": response.error.unwrap_or_default()}),
                                ),
                            ),
                        };
                    }
                    Ok(Err(_)) | Err(_) => {
                        state
                            .metrics
                            .tunnel_queries_fallback
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tracing::warn!(host_id = %host_id, "tunnel query failed or timed out, falling back to HTTP");
                    }
                }
            }
            Err(e) => {
                state
                    .metrics
                    .tunnel_queries_fallback
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
            )
            .await;
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
            )
            .await;
            if msg.contains("denied") {
                (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": msg})),
                )
            } else {
                (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": format!("agent query failed: {}", msg)})),
                )
            }
        }
    }
}

async fn file_preview_handler(
    State(state): State<AppState>,
    _user: CsrfProtected,
    Json(body): Json<serde_json::Value>,
) -> HandlerResponse {
    let host_id = match body
        .get("host_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
    {
        Some(id) => id,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "missing host_id"})),
            )
        }
    };
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "missing path"})),
            )
        }
    };

    let host =
        match config_storage::repositories::hosts::HostsRepo::get(state.db.pool(), host_id).await {
            Ok(Some(h)) => h,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "host not found"})),
                )
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to lookup host");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "internal error"})),
                );
            }
        };

    if host.status == "offline" {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "host is offline"})),
        );
    }

    // Try tunnel first
    if state.tunnel_registry.is_connected(host_id) {
        let request_id = Uuid::new_v4().to_string();
        let tunnel_msg = config_transport::tunnel::TunnelMessage::query_request(
            request_id.clone(),
            config_transport::tunnel::QueryKind::Preview,
            path.to_string(),
        );

        match state
            .tunnel_registry
            .send_query(host_id, request_id, &tunnel_msg)
        {
            Ok(rx) => {
                match tokio::time::timeout(Duration::from_secs(state.query_timeout_secs), rx).await
                {
                    Ok(Ok(response)) => {
                        state
                            .metrics
                            .tunnel_queries_routed
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let _ =
                            config_storage::repositories::file_queries::FileQueriesRepo::insert(
                                state.db.pool(),
                                Uuid::new_v4(),
                                "operator",
                                host_id,
                                path,
                                "preview",
                                &response.status,
                            )
                            .await;
                        return match response.status.as_str() {
                            "success" => (StatusCode::OK, Json(response.data.unwrap_or_default())),
                            "denied" => (
                                StatusCode::FORBIDDEN,
                                Json(
                                    serde_json::json!({"error": response.error.unwrap_or_default()}),
                                ),
                            ),
                            _ => (
                                StatusCode::BAD_GATEWAY,
                                Json(
                                    serde_json::json!({"error": response.error.unwrap_or_default()}),
                                ),
                            ),
                        };
                    }
                    Ok(Err(_)) | Err(_) => {
                        state
                            .metrics
                            .tunnel_queries_fallback
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tracing::warn!(host_id = %host_id, "tunnel query failed or timed out, falling back to HTTP");
                    }
                }
            }
            Err(e) => {
                state
                    .metrics
                    .tunnel_queries_fallback
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
            )
            .await;
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
            )
            .await;
            if msg.contains("denied") {
                (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": msg})),
                )
            } else {
                (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": format!("agent query failed: {}", msg)})),
                )
            }
        }
    }
}

async fn file_content_handler(
    State(state): State<AppState>,
    _user: CsrfProtected,
    Json(body): Json<serde_json::Value>,
) -> HandlerResponse {
    let host_id = match body
        .get("host_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
    {
        Some(id) => id,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "missing host_id"})),
            )
        }
    };
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "missing path"})),
            )
        }
    };
    let offset = body.get("offset").and_then(|v| v.as_u64());
    let limit = body.get("limit").and_then(|v| v.as_u64());

    let host =
        match config_storage::repositories::hosts::HostsRepo::get(state.db.pool(), host_id).await {
            Ok(Some(h)) => h,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "host not found"})),
                )
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to lookup host");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "internal error"})),
                );
            }
        };

    if host.status == "offline" {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "host is offline"})),
        );
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

        match state
            .tunnel_registry
            .send_query(host_id, request_id, &tunnel_msg)
        {
            Ok(rx) => {
                match tokio::time::timeout(
                    Duration::from_secs(state.query_timeout_secs.max(30)),
                    rx,
                )
                .await
                {
                    Ok(Ok(response)) => {
                        state
                            .metrics
                            .tunnel_queries_routed
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        return match response.status.as_str() {
                            "success" => (StatusCode::OK, Json(response.data.unwrap_or_default())),
                            "denied" => (
                                StatusCode::FORBIDDEN,
                                Json(
                                    serde_json::json!({"error": response.error.unwrap_or_default()}),
                                ),
                            ),
                            _ => (
                                StatusCode::BAD_GATEWAY,
                                Json(
                                    serde_json::json!({"error": response.error.unwrap_or_default()}),
                                ),
                            ),
                        };
                    }
                    Ok(Err(_)) | Err(_) => {
                        state
                            .metrics
                            .tunnel_queries_fallback
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tracing::warn!(host_id = %host_id, "tunnel content query failed or timed out, falling back to HTTP");
                    }
                }
            }
            Err(e) => {
                state
                    .metrics
                    .tunnel_queries_fallback
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tracing::warn!(host_id = %host_id, error = %e, "tunnel send failed, falling back to HTTP");
            }
        }
    }

    // Fall back to direct HTTP
    let agent_addr = format!("{}:9090", host.hostname);
    let query_client = config_transport::agent_query::AgentQueryClient::new();

    match query_client
        .query_content(&agent_addr, path, offset, limit)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(result)),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("denied") {
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
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": format!("agent query failed: {}", msg)})),
                )
            }
        }
    }
}

async fn github_file_content_handler(
    State(state): State<AppState>,
    _user: CsrfProtected,
    Json(body): Json<serde_json::Value>,
) -> HandlerResponse {
    let url = match body.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "missing url"})),
            )
        }
    };

    // M11: SSRF protection — only allow github.com URLs
    if let Ok(parsed) = url::Url::parse(url) {
        if parsed.host_str() != Some("github.com") && parsed.host_str() != Some("www.github.com") {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "only github.com URLs are allowed"})),
            );
        }
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid URL"})),
        );
    }

    // M11: never accept github_token from the request body — only use the server-side configured one
    let token = state.github_token.clone();

    let (owner, repo, branch, path) =
        match config_workflow::github_client::parse_github_blob_url(url) {
            Ok(r) => r,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
            }
        };

    match config_workflow::github_client::fetch_file_contents(
        token.as_deref(),
        &owner,
        &repo,
        &path,
        &branch,
    )
    .await
    {
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
                (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": msg})),
                )
            } else if msg.contains("auth") {
                (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": msg})),
                )
            } else {
                (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": format!("github request failed: {}", msg)})),
                )
            }
        }
    }
}

async fn metrics_handler(
    State(state): State<AppState>,
    _user: AuthenticatedUser,
) -> HandlerResponse {
    (StatusCode::OK, Json(state.metrics.snapshot()))
}

/// L8: unauthenticated health endpoint for k8s/Istio probes.
async fn healthz_handler() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
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
    _user: CsrfProtected,
    Json(body): Json<CreateWorkflowRequest>,
) -> HandlerResponse {
    let base_branch = body
        .base_branch
        .clone()
        .unwrap_or_else(|| "main".to_string());
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
        reviewers_json: body
            .reviewers
            .as_ref()
            .map(|r| serde_json::to_value(r).unwrap_or_default()),
        event_ids_json: body
            .event_ids
            .as_ref()
            .map(|ids| serde_json::to_value(ids).unwrap_or_default()),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    match config_storage::repositories::workflows::WorkflowsRepo::insert(
        state.db.pool(),
        &workflow_row,
    )
    .await
    {
        Ok(_) => {}
        Err(e) => {
            tracing::error!(error = %e, "failed to insert workflow");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "failed to create workflow"})),
            );
        }
    }

    // Spawn background task
    let pool = state.db.pool().clone();
    let resolver = std::sync::Arc::new(
        config_workflow::content_resolver::SnapshotContentResolver::new(
            state.snapshot_store.clone(),
        ),
    );

    tokio::spawn(async move {
        config_workflow::executor::run_workflow(run, pool, resolver).await;
    });

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"workflow_id": workflow_id})),
    )
}

async fn get_workflow_handler(
    State(state): State<AppState>,
    _user: AuthenticatedUser,
    axum::extract::Path(workflow_id): axum::extract::Path<Uuid>,
) -> HandlerResponse {
    match config_storage::repositories::workflows::WorkflowsRepo::get(state.db.pool(), workflow_id)
        .await
    {
        Ok(Some(row)) => (StatusCode::OK, Json(serde_json::json!({"workflow": row}))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "workflow not found"})),
        ),
        Err(e) => {
            tracing::error!(error = %e, "get workflow failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            )
        }
    }
}

async fn list_workflows_handler(
    State(state): State<AppState>,
    _user: AuthenticatedUser,
    axum::extract::Query(pagination): axum::extract::Query<Pagination>,
) -> HandlerResponse {
    match config_storage::repositories::workflows::WorkflowsRepo::list(
        state.db.pool(),
        pagination.limit,
        pagination.offset,
    )
    .await
    {
        Ok(workflows) => (
            StatusCode::OK,
            Json(serde_json::json!({"workflows": workflows})),
        ),
        Err(e) => {
            tracing::error!(error = %e, "list workflows failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Admin approval endpoints
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ApproveUserRequest {
    user_id: Option<String>,
    email: Option<String>,
    /// Role to assign on approval. Defaults to "user" if omitted.
    #[serde(default = "default_approve_role")]
    role: Option<String>,
}

fn default_approve_role() -> Option<String> {
    Some("user".to_string())
}

#[derive(Deserialize)]
struct RejectUserRequest {
    user_id: Option<String>,
    email: Option<String>,
    reason: Option<String>,
}

#[derive(Deserialize)]
struct SetRoleRequest {
    user_id: Option<String>,
    email: Option<String>,
    role: String,
}

#[derive(Serialize)]
struct PendingUser {
    id: String,
    email: Option<String>,
    name: Option<String>,
    created_at: Option<DateTime<Utc>>,
}

/// Verify the admin secret from the request header.
/// Returns the effective secret string if valid.
async fn approve_user_handler(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<ApproveUserRequest>,
) -> HandlerResponse {
    // Verify admin secret
    let secret = state.admin_api_secret.as_deref().unwrap_or(&state.secret);
    let provided = headers
        .get("X-Admin-Secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_secret_constant_time(provided, secret) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid admin secret"})),
        );
    }

    // Find user by ID or email
    let user_id = match (&body.user_id, &body.email) {
        (Some(id), _) => id.clone(),
        (None, Some(email)) => {
            match sqlx::query_scalar::<_, String>("SELECT id FROM users WHERE email = $1")
                .bind(email)
                .fetch_optional(state.db.pool())
                .await
            {
                Ok(Some(id)) => id,
                Ok(None) => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": "user not found"})),
                    )
                }
                Err(e) => {
                    tracing::error!(error = %e, "user lookup failed");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"error": "internal error"})),
                    );
                }
            }
        }
        (None, None) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "provide user_id or email"})),
            );
        }
    };

    let role = body.role.as_deref().unwrap_or("user");
    // Only allow valid dashboard roles
    if role != "admin" && role != "user" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "role must be 'admin' or 'user'"})),
        );
    }

    match sqlx::query(
        "UPDATE users SET banned = false, role = $1, updated_at = NOW() WHERE id = $2",
    )
    .bind(role)
    .bind(&user_id)
    .execute(state.db.pool())
    .await
    {
        Ok(result) => {
            if result.rows_affected() == 0 {
                (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "user not found"})),
                )
            } else {
                tracing::info!(user_id = %user_id, role = role, "user approved");
                (
                    StatusCode::OK,
                    Json(
                        serde_json::json!({"message": "user approved", "user_id": user_id, "role": role}),
                    ),
                )
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "approve user failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            )
        }
    }
}

async fn reject_user_handler(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<RejectUserRequest>,
) -> HandlerResponse {
    // Verify admin secret
    let secret = state.admin_api_secret.as_deref().unwrap_or(&state.secret);
    let provided = headers
        .get("X-Admin-Secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_secret_constant_time(provided, secret) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid admin secret"})),
        );
    }

    // Find user by ID or email
    let user_id = match (&body.user_id, &body.email) {
        (Some(id), _) => id.clone(),
        (None, Some(email)) => {
            match sqlx::query_scalar::<_, String>("SELECT id FROM users WHERE email = $1")
                .bind(email)
                .fetch_optional(state.db.pool())
                .await
            {
                Ok(Some(id)) => id,
                Ok(None) => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": "user not found"})),
                    )
                }
                Err(e) => {
                    tracing::error!(error = %e, "user lookup failed");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"error": "internal error"})),
                    );
                }
            }
        }
        (None, None) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "provide user_id or email"})),
            );
        }
    };

    let reason = body.reason.as_deref().unwrap_or("rejected");

    match sqlx::query(
        "UPDATE users SET banned = true, ban_reason = $1, updated_at = NOW() WHERE id = $2",
    )
    .bind(reason)
    .bind(&user_id)
    .execute(state.db.pool())
    .await
    {
        Ok(result) => {
            if result.rows_affected() == 0 {
                (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "user not found"})),
                )
            } else {
                // Also revoke all sessions for this user
                let _ = sqlx::query("DELETE FROM sessions WHERE user_id = $1")
                    .bind(&user_id)
                    .execute(state.db.pool())
                    .await;
                tracing::info!(user_id = %user_id, "user rejected and sessions revoked");
                (
                    StatusCode::OK,
                    Json(serde_json::json!({"message": "user rejected", "user_id": user_id})),
                )
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "reject user failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            )
        }
    }
}

async fn pending_users_handler(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> HandlerResponse {
    // Verify admin secret
    let secret = state.admin_api_secret.as_deref().unwrap_or(&state.secret);
    let provided = headers
        .get("X-Admin-Secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_secret_constant_time(provided, secret) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid admin secret"})),
        );
    }

    match sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<DateTime<Utc>>)>(
        "SELECT id, email, name, created_at FROM users WHERE role = 'pending_approval' ORDER BY created_at DESC",
    )
    .fetch_all(state.db.pool())
    .await
    {
        Ok(rows) => {
            let users: Vec<PendingUser> = rows
                .into_iter()
                .map(|(id, email, name, created_at)| PendingUser { id, email, name, created_at })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"users": users})))
        }
        Err(e) => {
            tracing::error!(error = %e, "list pending users failed");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "internal error"})))
        }
    }
}

async fn set_role_handler(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<SetRoleRequest>,
) -> HandlerResponse {
    // Verify admin secret
    let secret = state.admin_api_secret.as_deref().unwrap_or(&state.secret);
    let provided = headers
        .get("X-Admin-Secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_secret_constant_time(provided, secret) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid admin secret"})),
        );
    }

    // Only allow valid dashboard roles
    if body.role != "admin" && body.role != "user" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "role must be 'admin' or 'user'"})),
        );
    }

    // Find user by ID or email
    let user_id = match (&body.user_id, &body.email) {
        (Some(id), _) => id.clone(),
        (None, Some(email)) => {
            match sqlx::query_scalar::<_, String>("SELECT id FROM users WHERE email = $1")
                .bind(email)
                .fetch_optional(state.db.pool())
                .await
            {
                Ok(Some(id)) => id,
                Ok(None) => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": "user not found"})),
                    )
                }
                Err(e) => {
                    tracing::error!(error = %e, "user lookup failed");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"error": "internal error"})),
                    );
                }
            }
        }
        (None, None) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "provide user_id or email"})),
            );
        }
    };

    match sqlx::query("UPDATE users SET role = $1, updated_at = NOW() WHERE id = $2")
        .bind(&body.role)
        .bind(&user_id)
        .execute(state.db.pool())
        .await
    {
        Ok(result) => {
            if result.rows_affected() == 0 {
                (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "user not found"})),
                )
            } else {
                tracing::info!(user_id = %user_id, role = %body.role, "user role updated");
                (
                    StatusCode::OK,
                    Json(
                        serde_json::json!({"message": "role updated", "user_id": user_id, "role": body.role}),
                    ),
                )
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "set role failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            )
        }
    }
}
