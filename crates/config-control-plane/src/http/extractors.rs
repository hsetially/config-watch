use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use better_auth::types_mod::ApiKeyOps;
use better_auth::{AuthSession, AuthUser, UserOps};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::services::AppState;

/// CSRF cookie name — must match the name set by the auth proxy on sign-in.
const CSRF_COOKIE_NAME: &str = "config_watch_csrf";

pub struct AgentAuth {
    pub host_id: String,
    pub token: String,
}

impl FromRequestParts<AppState> for AgentAuth {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Try X-API-Key header first (BetterAuth API key)
        if let Some(api_key) = parts
            .headers
            .get("X-API-Key")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
        {
            if !api_key.is_empty() {
                return validate_api_key(&api_key, state).await;
            }
        }

        // Fall back to X-Agent-Token (HMAC credential)
        let token = parts
            .headers
            .get("X-Agent-Token")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_default();

        if token.is_empty() {
            return Err(
                (StatusCode::UNAUTHORIZED, "missing agent token or API key").into_response()
            );
        }

        match config_auth::tokens::AgentCredential::verify(&state.secret, &token) {
            Ok(cred) => Ok(Self {
                host_id: cred.host_id,
                token,
            }),
            Err(e) => {
                tracing::warn!(error = e, "agent auth failed");
                Err((StatusCode::UNAUTHORIZED, e).into_response())
            }
        }
    }
}

async fn validate_api_key(key: &str, state: &AppState) -> Result<AgentAuth, Response> {
    // Hash the key using SHA-256 (same algorithm as BetterAuth)
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let hash = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize());

    let api_key = state
        .auth
        .database()
        .get_api_key_by_hash(&hash)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "API key lookup error");
            (StatusCode::INTERNAL_SERVER_ERROR, "API key lookup failed").into_response()
        })?;

    match api_key {
        Some(k) => {
            if !k.enabled {
                return Err((StatusCode::UNAUTHORIZED, "API key is disabled").into_response());
            }
            // Check expiry if set
            if let Some(ref expires) = k.expires_at {
                if !expires.is_empty() {
                    if let Ok(expiry) = expires.parse::<chrono::DateTime<chrono::Utc>>() {
                        if expiry < chrono::Utc::now() {
                            return Err(
                                (StatusCode::UNAUTHORIZED, "API key has expired").into_response()
                            );
                        }
                    }
                }
            }
            Ok(AgentAuth {
                host_id: k.user_id.to_string(),
                token: key.to_string(),
            })
        }
        None => Err((StatusCode::UNAUTHORIZED, "invalid API key").into_response()),
    }
}

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub role: Option<String>,
    pub banned: bool,
    pub two_factor_enabled: bool,
}

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let cookie_name = &state.auth.config().session.cookie_name;
        let token = extract_session_token(parts, cookie_name)
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, "missing session token").into_response())?;

        let session = state
            .auth
            .session_manager()
            .get_session(&token)
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "session validation error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "session validation failed",
                )
                    .into_response()
            })?
            .ok_or_else(|| {
                (StatusCode::UNAUTHORIZED, "invalid or expired session").into_response()
            })?;

        let user = state
            .auth
            .database()
            .get_user_by_id(session.user_id())
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "user lookup error");
                (StatusCode::INTERNAL_SERVER_ERROR, "user lookup failed").into_response()
            })?
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, "user not found").into_response())?;

        if user.banned() {
            let role = user.role().map(|s| s.to_string());
            let error_type = if role.as_deref() == Some("pending_approval") {
                "approval_pending"
            } else {
                "banned"
            };
            let message = if role.as_deref() == Some("pending_approval") {
                "Account awaiting admin approval".to_string()
            } else {
                user.ban_reason()
                    .unwrap_or("Account has been banned")
                    .to_string()
            };
            let body = serde_json::json!({
                "error": error_type,
                "message": message,
            });
            return Err((
                StatusCode::FORBIDDEN,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                serde_json::to_string(&body).unwrap_or_else(|_| {
                    r#"{"error":"banned","message":"Account suspended"}"#.to_string()
                }),
            )
                .into_response());
        }

        // Only users with 'admin' or 'user' role may access the dashboard
        let role = user.role().map(|s| s.to_string());
        if !matches!(role.as_deref(), Some("admin") | Some("user")) {
            let body = serde_json::json!({
                "error": "insufficient_role",
                "message": "Account does not have required role",
            });
            return Err((
                StatusCode::FORBIDDEN,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                serde_json::to_string(&body).unwrap_or_else(|_| {
                    r#"{"error":"insufficient_role","message":"Access denied"}"#.to_string()
                }),
            )
                .into_response());
        }

        // H6: Admins must have 2FA enabled to access admin endpoints.
        // Access to non-admin dashboard pages is still allowed.
        let two_factor_enabled = user.two_factor_enabled();

        Ok(Self {
            user_id: user.id().to_string(),
            email: user.email().map(|s| s.to_string()),
            name: user.name().map(|s| s.to_string()),
            role,
            banned: false,
            two_factor_enabled,
        })
    }
}

/// Optional session extractor — returns `None` instead of 401 when unauthenticated.
#[derive(Debug, Clone)]
pub struct MaybeAuthenticated(pub Option<AuthenticatedUser>);

impl FromRequestParts<AppState> for MaybeAuthenticated {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match AuthenticatedUser::from_request_parts(parts, state).await {
            Ok(user) => Ok(MaybeAuthenticated(Some(user))),
            Err(_) => Ok(MaybeAuthenticated(None)),
        }
    }
}

fn extract_session_token(parts: &Parts, cookie_name: &str) -> Option<String> {
    // Try Bearer token from Authorization header
    if let Some(auth_header) = parts.headers.get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ") {
                return Some(token.to_string());
            }
        }
    }

    // Fall back to session cookie
    if let Some(cookie_header) = parts.headers.get("cookie") {
        if let Ok(cookie_str) = cookie_header.to_str() {
            for part in cookie_str.split(';') {
                let part = part.trim();
                let prefix = format!("{}=", cookie_name);
                if let Some(value) = part.strip_prefix(&prefix) {
                    if !value.is_empty() {
                        return Some(value.to_string());
                    }
                }
            }
        }
    }

    // M9 fix: ?token= query parameter is NOT accepted here.
    // Use WsAuthenticatedUser for WebSocket upgrade routes only.
    None
}

/// Extract a named cookie value from the Cookie header.
fn extract_cookie_value(parts: &Parts, cookie_name: &str) -> Option<String> {
    let cookie_header = parts.headers.get("cookie")?;
    let cookie_str = cookie_header.to_str().ok()?;
    for part in cookie_str.split(';') {
        let part = part.trim();
        let prefix = format!("{}=", cookie_name);
        if let Some(value) = part.strip_prefix(&prefix) {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// CSRF-protected extractor for mutating requests (POST/PUT/DELETE/PATCH).
/// Validates that the `x-csrf-token` header matches the `config_watch_csrf` cookie
/// using constant-time comparison. Requires an authenticated session (cookie or Bearer).
#[derive(Debug, Clone)]
pub struct CsrfProtected(pub AuthenticatedUser);

impl FromRequestParts<AppState> for CsrfProtected {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthenticatedUser::from_request_parts(parts, state).await?;

        let header_token = parts
            .headers
            .get("x-csrf-token")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| (StatusCode::FORBIDDEN, "missing CSRF token").into_response())?;

        let cookie_token = extract_cookie_value(parts, CSRF_COOKIE_NAME)
            .ok_or_else(|| (StatusCode::FORBIDDEN, "missing CSRF cookie").into_response())?;

        if !constant_time_eq::constant_time_eq(header_token.as_bytes(), cookie_token.as_bytes()) {
            return Err((StatusCode::FORBIDDEN, "CSRF token mismatch").into_response());
        }

        Ok(CsrfProtected(user))
    }
}

/// WebSocket-only authenticator that accepts ?ticket= (one-shot), ?token= (legacy),
/// Bearer header, or session cookie. Prefer ?ticket= as ?token= leaks in logs.
/// This extractor MUST only be used on the WS upgrade route — never on
/// regular REST endpoints where tokens would leak into proxy logs.
#[derive(Debug, Clone)]
pub struct WsAuthenticatedUser(pub AuthenticatedUser);

impl FromRequestParts<AppState> for WsAuthenticatedUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let cookie_name = &state.auth.config().session.cookie_name;

        // Try one-shot ticket first (preferred for WS)
        if let Some(query) = parts.uri.query() {
            for (key, value) in form_urlencoded::parse(query.as_bytes()) {
                if key == "ticket" && !value.is_empty() {
                    let ticket_str = value.to_string();
                    match crate::ws_ticket::verify_ticket(&ticket_str, &state.secret) {
                        Ok(user_id) => {
                            let user = state
                                .auth
                                .database()
                                .get_user_by_id(&user_id)
                                .await
                                .map_err(|e| {
                                    tracing::warn!(error = %e, "user lookup error for WS ticket");
                                    (StatusCode::INTERNAL_SERVER_ERROR, "user lookup failed")
                                        .into_response()
                                })?
                                .ok_or_else(|| {
                                    (StatusCode::UNAUTHORIZED, "user not found").into_response()
                                })?;

                            if user.banned() {
                                return Err(
                                    (StatusCode::FORBIDDEN, "account suspended").into_response()
                                );
                            }
                            let role = user.role().map(|s| s.to_string());
                            if !matches!(role.as_deref(), Some("admin") | Some("user")) {
                                return Err(
                                    (StatusCode::FORBIDDEN, "insufficient role").into_response()
                                );
                            }

                            return Ok(WsAuthenticatedUser(AuthenticatedUser {
                                user_id: user.id().to_string(),
                                email: user.email().map(|s| s.to_string()),
                                name: user.name().map(|s| s.to_string()),
                                role,
                                banned: false,
                                two_factor_enabled: user.two_factor_enabled(),
                            }));
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "WS ticket verification failed");
                            return Err((StatusCode::UNAUTHORIZED, "invalid or expired ticket")
                                .into_response());
                        }
                    }
                }
            }
        }

        // Try Bearer header
        if let Some(auth_header) = parts.headers.get("authorization") {
            if let Ok(auth_str) = auth_header.to_str() {
                if let Some(token) = auth_str.strip_prefix("Bearer ") {
                    if let Some(user) = validate_session(token, state, cookie_name).await? {
                        return Ok(WsAuthenticatedUser(user));
                    }
                }
            }
        }

        // Try session cookie
        if let Some(cookie_header) = parts.headers.get("cookie") {
            if let Ok(cookie_str) = cookie_header.to_str() {
                for part in cookie_str.split(';') {
                    let part = part.trim();
                    let prefix = format!("{}=", cookie_name);
                    if let Some(value) = part.strip_prefix(&prefix) {
                        if !value.is_empty() {
                            if let Some(user) = validate_session(value, state, cookie_name).await? {
                                return Ok(WsAuthenticatedUser(user));
                            }
                        }
                    }
                }
            }
        }

        // Legacy: ?token= for backward compatibility (leaks in logs — prefer ?ticket=)
        if let Some(query) = parts.uri.query() {
            for (key, value) in form_urlencoded::parse(query.as_bytes()) {
                if key == "token" && !value.is_empty() {
                    if let Some(user) = validate_session(&value, state, cookie_name).await? {
                        return Ok(WsAuthenticatedUser(user));
                    }
                }
            }
        }

        Err((StatusCode::UNAUTHORIZED, "missing session token").into_response())
    }
}

/// Shared session validation helper.
async fn validate_session(
    token: &str,
    state: &AppState,
    _cookie_name: &str,
) -> Result<Option<AuthenticatedUser>, Response> {
    let session = match state.auth.session_manager().get_session(token).await {
        Ok(Some(s)) => s,
        Ok(None) => return Ok(None),
        Err(e) => {
            tracing::warn!(error = %e, "session validation error");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "session validation failed",
            )
                .into_response());
        }
    };

    let user = match state
        .auth
        .database()
        .get_user_by_id(session.user_id())
        .await
    {
        Ok(Some(u)) => u,
        Ok(None) => return Ok(None),
        Err(e) => {
            tracing::warn!(error = %e, "user lookup error");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "user lookup failed").into_response());
        }
    };

    if user.banned() {
        return Ok(None);
    }

    let role = user.role().map(|s| s.to_string());
    if !matches!(role.as_deref(), Some("admin") | Some("user")) {
        return Ok(None);
    }

    Ok(Some(AuthenticatedUser {
        user_id: user.id().to_string(),
        email: user.email().map(|s| s.to_string()),
        name: user.name().map(|s| s.to_string()),
        role,
        banned: false,
        two_factor_enabled: user.two_factor_enabled(),
    }))
}

/// Admin user extractor: requires authenticated user with admin role and 2FA enabled.
/// H6: admins must have 2FA enabled before accessing admin endpoints.
#[derive(Debug, Clone)]
pub struct AdminUser(pub AuthenticatedUser);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthenticatedUser::from_request_parts(parts, state).await?;

        if user.role.as_deref() != Some("admin") {
            return Err((
                StatusCode::FORBIDDEN,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"error": "forbidden", "message": "Admin role required"})
                    .to_string(),
            )
                .into_response());
        }

        if !user.two_factor_enabled {
            return Err((
                StatusCode::FORBIDDEN,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"error": "2fa_required", "message": "Two-factor authentication must be enabled for admin access"}).to_string(),
            )
                .into_response());
        }

        Ok(AdminUser(user))
    }
}

#[derive(Debug, Clone)]
pub struct CorrelationId(pub Uuid);

impl FromRequestParts<AppState> for CorrelationId {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let id = parts
            .headers
            .get("X-Correlation-ID")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_else(Uuid::new_v4);

        Ok(Self(id))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pagination {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChangeFilters {
    pub host_id: Option<String>,
    pub path_prefix: Option<String>,
    pub filename: Option<String>,
    pub author: Option<String>,
    pub severity: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    /// If "true", include initial_snapshot events. Default: exclude them.
    pub initial: Option<String>,
}
