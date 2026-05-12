use axum::body::Body;
use axum::extract::FromRequestParts;
use axum::http::{Request, StatusCode};
use std::collections::VecDeque;
use std::sync::Arc;
use uuid::Uuid;

use config_control_plane::http::extractors::{AgentAuth, CorrelationId};
use config_control_plane::services::AppState;

mod common;

fn make_state(secret: &str) -> AppState {
    // Use a dummy pool URL that won't be connected for extractor tests
    // since extractors don't touch the DB
    let (tx, _) = tokio::sync::broadcast::channel(256);
    let metrics = config_control_plane::metrics::ControlPlaneMetrics::new();
    let tmp = tempfile::tempdir().expect("create temp dir");
    let snapshot_store = config_snapshot::store::SnapshotStore::new(camino::Utf8Path::new(
        tmp.path().join("snapshots").to_str().unwrap(),
    ))
    .expect("create snapshot store");

    // Create a dummy auth state for testing
    let auth_config = better_auth::AuthConfig::new(
        "test-secret-key-that-is-at-least-32-characters-long",
    )
    .base_url("http://localhost:3000");
    let db = better_auth::adapters::SqlxAdapter::from_pool(
        sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent").unwrap(),
    );
    let auth_state = std::sync::Arc::new(tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            better_auth::AuthBuilder::new(auth_config)
                .database(db)
                .plugin(better_auth::plugins::EmailPasswordPlugin::new().enable_signup(true))
                .build()
                .await
                .expect("failed to build test auth")
        })
    }));

    AppState {
        db: std::sync::Arc::new(config_storage::db::Database::from_pool(
            sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent").unwrap(),
        )),
        broadcast_tx: tx,
        secret: secret.to_string(),
        operator_keys: std::sync::Arc::new(std::collections::HashMap::new()),
        metrics: metrics.clone(),
        tunnel_registry: std::sync::Arc::new(config_control_plane::tunnel::AgentRegistry::new(
            metrics,
        )),
        query_timeout_secs: 10,
        snapshot_store: std::sync::Arc::new(snapshot_store),
        repos_dir: "./data/repos".to_string(),
        github_token: None,
        diff_service: std::sync::Arc::new(
            config_control_plane::diff_service::DiffService::new(
                config_diff::DiffConfig::default(),
            ),
        ),
        auth: auth_state,
        admin_api_secret: None,
        require_approval: true,
        local_event_dedup: Arc::new(std::sync::Mutex::new(VecDeque::new())),
    }
}

async fn extract_from_request<T: FromRequestParts<AppState>>(
    state: &AppState,
    request: Request<Body>,
) -> Result<T, T::Rejection> {
    let (mut parts, _body) = request.into_parts();
    T::from_request_parts(&mut parts, state).await
}

#[tokio::test]
async fn agent_auth_missing_token_returns_401() {
    let state = make_state("test-secret");
    let req = Request::builder()
        .method("POST")
        .uri("/test")
        .body(Body::empty())
        .unwrap();
    let result = extract_from_request::<AgentAuth>(&state, req).await;
    assert!(result.is_err());
    let response = result.err().unwrap();
    let status = response.status();
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn agent_auth_valid_token_extracts_host_id() {
    let secret = "test-secret";
    let host_id = Uuid::new_v4();
    let state = make_state(secret);
    let token = config_auth::tokens::AgentCredential::issue(
        secret,
        &host_id.to_string(),
        chrono::Duration::hours(24),
    )
    .token;

    let req = Request::builder()
        .method("POST")
        .uri("/test")
        .header("X-Agent-Token", &token)
        .body(Body::empty())
        .unwrap();
    let result = extract_from_request::<AgentAuth>(&state, req).await;
    assert!(result.is_ok(), "Expected Ok, got err: {:?}", result.err());
    let auth = result.unwrap();
    assert_eq!(auth.host_id, host_id.to_string());
}

#[tokio::test]
async fn agent_auth_expired_token_returns_401() {
    let secret = "test-secret";
    let host_id = Uuid::new_v4();
    let state = make_state(secret);
    let token = config_auth::tokens::AgentCredential::issue(
        secret,
        &host_id.to_string(),
        chrono::Duration::seconds(-1),
    )
    .token;

    let req = Request::builder()
        .method("POST")
        .uri("/test")
        .header("X-Agent-Token", &token)
        .body(Body::empty())
        .unwrap();
    let result = extract_from_request::<AgentAuth>(&state, req).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn agent_auth_wrong_secret_returns_401() {
    let host_id = Uuid::new_v4();
    let state = make_state("correct-secret");
    let token = config_auth::tokens::AgentCredential::issue(
        "wrong-secret",
        &host_id.to_string(),
        chrono::Duration::hours(24),
    )
    .token;

    let req = Request::builder()
        .method("POST")
        .uri("/test")
        .header("X-Agent-Token", &token)
        .body(Body::empty())
        .unwrap();
    let result = extract_from_request::<AgentAuth>(&state, req).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn agent_auth_malformed_token_returns_401() {
    let state = make_state("test-secret");
    let req = Request::builder()
        .method("POST")
        .uri("/test")
        .header("X-Agent-Token", "not-a-valid-token-format")
        .body(Body::empty())
        .unwrap();
    let result = extract_from_request::<AgentAuth>(&state, req).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn correlation_id_missing_generates_uuid() {
    let state = make_state("test-secret");
    let req = Request::builder()
        .method("GET")
        .uri("/test")
        .body(Body::empty())
        .unwrap();
    let result = extract_from_request::<CorrelationId>(&state, req).await;
    assert!(result.is_ok());
    let cid = result.unwrap();
    // Should be a valid UUID (auto-generated)
    assert!(Uuid::parse_str(&cid.0.to_string()).is_ok());
}

#[tokio::test]
async fn correlation_id_present_uses_provided() {
    let state = make_state("test-secret");
    let provided_id = Uuid::new_v4();
    let req = Request::builder()
        .method("GET")
        .uri("/test")
        .header("X-Correlation-ID", provided_id.to_string())
        .body(Body::empty())
        .unwrap();
    let result = extract_from_request::<CorrelationId>(&state, req).await;
    assert!(result.is_ok());
    let cid = result.unwrap();
    assert_eq!(cid.0, provided_id);
}