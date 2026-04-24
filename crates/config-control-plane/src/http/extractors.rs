use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use uuid::Uuid;

use crate::services::AppState;

pub struct AgentAuth {
    pub host_id: String,
    pub token: String,
}

#[async_trait::async_trait]
impl FromRequestParts<AppState> for AgentAuth {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get("X-Agent-Token")
            .or_else(|| parts.headers.get("X-Enrollment-Token"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_default();

        if token.is_empty() {
            return Err((StatusCode::UNAUTHORIZED, "missing agent token").into_response());
        }

        // Verify the HMAC credential
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

#[derive(Debug, Clone)]
pub enum OperatorRole {
    Viewer,
    Investigator,
    Operator,
    Admin,
}

pub struct OperatorAuth {
    pub operator_id: String,
    pub role: OperatorRole,
}

#[async_trait::async_trait]
impl FromRequestParts<AppState> for OperatorAuth {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(|s| s.trim())
            .unwrap_or_default();

        if token.is_empty() {
            return Err((StatusCode::UNAUTHORIZED, "missing bearer token").into_response());
        }

        match state.operator_keys.get(token) {
            Some((operator_id, role)) => {
                let role = match role.as_str() {
                    "admin" => OperatorRole::Admin,
                    "operator" => OperatorRole::Operator,
                    "investigator" => OperatorRole::Investigator,
                    _ => OperatorRole::Viewer,
                };
                Ok(Self {
                    operator_id: operator_id.clone(),
                    role,
                })
            }
            None => Err((StatusCode::UNAUTHORIZED, "invalid operator key").into_response()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CorrelationId(pub Uuid);

#[async_trait::async_trait]
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
}
