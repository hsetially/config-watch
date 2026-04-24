use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

use config_control_plane::http::routes::build_router;
use config_control_plane::services::AppState;

mod common;

use common::*;

async fn setup_app() -> AppState {
    let pool = setup_test_db().await;
    make_app_state(pool, "test-secret")
}

// Helper: send request and return (status, json)
async fn send_req(state: AppState, req: Request<Body>) -> (StatusCode, Value) {
    let app = build_router(state);
    let response = app.oneshot(req).await.unwrap();
    let status = response.status();
    let body = response.into_body();
    let bytes = body.collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

// --- Registration tests ---

#[tokio::test]
async fn register_missing_enrollment_token_returns_401() {
    let state = setup_app().await;
    let body = register_body(&Uuid::new_v4(), "test-host", "default");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/agents/register")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, _) = send_req(state, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn register_invalid_enrollment_token_returns_401() {
    let state = setup_app().await;
    let body = register_body(&Uuid::new_v4(), "test-host", "default");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/agents/register")
        .header("content-type", "application/json")
        .header("X-Enrollment-Token", "wrong-token")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("invalid enrollment token"));
}

#[tokio::test]
async fn register_missing_host_id_returns_400() {
    let state = setup_app().await;
    let body = serde_json::json!({
        "hostname": "test-host",
        "environment": "default"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/agents/register")
        .header("content-type", "application/json")
        .header("X-Enrollment-Token", "test-secret")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("missing host_id"));
}

#[tokio::test]
async fn register_valid_returns_201_with_credential() {
    let state = setup_app().await;
    let host_id = Uuid::new_v4();
    let body = register_body(&host_id, "test-host", "default");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/agents/register")
        .header("content-type", "application/json")
        .header("X-Enrollment-Token", "test-secret")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(json.get("agent_credential").is_some());
    assert!(json.get("credential_expires_at").is_some());
    assert!(json.get("host").is_some());
}

#[tokio::test]
async fn register_upserts_on_duplicate_host_id() {
    let state = setup_app().await;
    let host_id = Uuid::new_v4();
    let body1 = register_body(&host_id, "host-one", "default");
    let body2 = register_body(&host_id, "host-two", "production");

    let req1 = Request::builder()
        .method("POST")
        .uri("/v1/agents/register")
        .header("content-type", "application/json")
        .header("X-Enrollment-Token", "test-secret")
        .body(Body::from(serde_json::to_vec(&body1).unwrap()))
        .unwrap();
    let (status1, _) = send_req(state.clone(), req1).await;
    assert_eq!(status1, StatusCode::CREATED);

    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/agents/register")
        .header("content-type", "application/json")
        .header("X-Enrollment-Token", "test-secret")
        .body(Body::from(serde_json::to_vec(&body2).unwrap()))
        .unwrap();
    let (status2, json2) = send_req(state, req2).await;
    assert_eq!(status2, StatusCode::CREATED);
    let host = json2.get("host").unwrap();
    assert_eq!(host.get("hostname").unwrap().as_str().unwrap(), "host-two");
}

// --- Heartbeat tests ---

#[tokio::test]
async fn heartbeat_missing_agent_token_returns_401() {
    let state = setup_app().await;
    let host_id = Uuid::new_v4();
    let body = heartbeat_body(&host_id);
    let req = Request::builder()
        .method("POST")
        .uri("/v1/agents/heartbeat")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, _) = send_req(state, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn heartbeat_invalid_agent_token_returns_401() {
    let state = setup_app().await;
    let host_id = Uuid::new_v4();
    let body = heartbeat_body(&host_id);
    let req = Request::builder()
        .method("POST")
        .uri("/v1/agents/heartbeat")
        .header("content-type", "application/json")
        .header("X-Agent-Token", "invalid-token")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, _) = send_req(state, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn heartbeat_valid_token_returns_204() {
    let state = setup_app().await;
    let host_id = Uuid::new_v4();

    // Register the host first
    let register_body = register_body(&host_id, "test-host", "default");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/agents/register")
        .header("content-type", "application/json")
        .header("X-Enrollment-Token", "test-secret")
        .body(Body::from(serde_json::to_vec(&register_body).unwrap()))
        .unwrap();
    let (reg_status, _) = send_req(state.clone(), req).await;
    assert_eq!(reg_status, StatusCode::CREATED);

    let token = make_agent_credential("test-secret", &host_id.to_string());
    let body = heartbeat_body(&host_id);
    let req = Request::builder()
        .method("POST")
        .uri("/v1/agents/heartbeat")
        .header("content-type", "application/json")
        .header("X-Agent-Token", &token)
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, _) = send_req(state, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

// --- Ingest tests ---

#[tokio::test]
async fn change_ingest_missing_agent_token_returns_401() {
    let state = setup_app().await;
    let host_id = Uuid::new_v4();
    let body = make_change_event_json(&host_id, "/etc/test.yaml", "modified", "test-key-1");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/events/change")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, _) = send_req(state, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn change_ingest_wrong_schema_version_returns_400() {
    let state = setup_app().await;
    let host_id = Uuid::new_v4();
    let token = make_agent_credential("test-secret", &host_id.to_string());
    let body = serde_json::json!({
        "schema_version": "2.0",
        "event": { "idempotency_key": "key1" }
    });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/events/change")
        .header("content-type", "application/json")
        .header("X-Agent-Token", &token)
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("unsupported schema version"));
}

#[tokio::test]
async fn change_ingest_missing_event_returns_400() {
    let state = setup_app().await;
    let host_id = Uuid::new_v4();
    let token = make_agent_credential("test-secret", &host_id.to_string());
    let body = serde_json::json!({ "schema_version": "1.0" });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/events/change")
        .header("content-type", "application/json")
        .header("X-Agent-Token", &token)
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("missing event"));
}

#[tokio::test]
async fn change_ingest_missing_idempotency_key_returns_400() {
    let state = setup_app().await;
    let host_id = Uuid::new_v4();
    let token = make_agent_credential("test-secret", &host_id.to_string());
    let body = serde_json::json!({
        "schema_version": "1.0",
        "event": {
            "event_id": Uuid::new_v4().to_string(),
            "host_id": host_id.to_string(),
            "event_kind": "modified"
        }
    });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/events/change")
        .header("content-type", "application/json")
        .header("X-Agent-Token", &token)
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("missing idempotency_key"));
}

#[tokio::test]
async fn change_ingest_valid_returns_201() {
    let state = setup_app().await;
    let host_id = Uuid::new_v4();

    // Register the host first
    let register_body = register_body(&host_id, "ingest-host", "default");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/agents/register")
        .header("content-type", "application/json")
        .header("X-Enrollment-Token", "test-secret")
        .body(Body::from(serde_json::to_vec(&register_body).unwrap()))
        .unwrap();
    let (reg_status, _) = send_req(state.clone(), req).await;
    assert_eq!(reg_status, StatusCode::CREATED);

    let token = make_agent_credential("test-secret", &host_id.to_string());
    let body = make_change_event_json(
        &host_id,
        "/etc/test.yaml",
        "modified",
        &format!("key-{}", host_id),
    );
    let req = Request::builder()
        .method("POST")
        .uri("/v1/events/change")
        .header("content-type", "application/json")
        .header("X-Agent-Token", &token)
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(json.get("event_id").is_some());
}

#[tokio::test]
async fn change_ingest_duplicate_returns_409() {
    let state = setup_app().await;
    let host_id = Uuid::new_v4();

    // Register host
    let register_body = register_body(&host_id, "dupe-host", "default");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/agents/register")
        .header("content-type", "application/json")
        .header("X-Enrollment-Token", "test-secret")
        .body(Body::from(serde_json::to_vec(&register_body).unwrap()))
        .unwrap();
    send_req(state.clone(), req).await;

    let token = make_agent_credential("test-secret", &host_id.to_string());
    let body = make_change_event_json(
        &host_id,
        "/etc/dupe.yaml",
        "modified",
        &format!("dupe-key-{}", host_id),
    );

    // First request: 201
    let req1 = Request::builder()
        .method("POST")
        .uri("/v1/events/change")
        .header("content-type", "application/json")
        .header("X-Agent-Token", &token)
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status1, _) = send_req(state.clone(), req1).await;
    assert_eq!(status1, StatusCode::CREATED);

    // Second request with same idempotency key: 409
    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/events/change")
        .header("content-type", "application/json")
        .header("X-Agent-Token", &token)
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status2, json2) = send_req(state, req2).await;
    assert_eq!(status2, StatusCode::CONFLICT);
    assert!(json2
        .get("message")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("duplicate"));
}

// --- Hosts tests ---

#[tokio::test]
async fn hosts_list_returns_200_with_hosts_array() {
    let state = setup_app().await;
    let req = Request::builder()
        .method("GET")
        .uri("/v1/hosts")
        .body(Body::empty())
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.get("hosts").is_some());
    assert!(json.get("hosts").unwrap().is_array());
}

#[tokio::test]
async fn hosts_list_custom_limit_offset() {
    let state = setup_app().await;
    let req = Request::builder()
        .method("GET")
        .uri("/v1/hosts?limit=10&offset=5")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send_req(state, req).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn host_detail_nonexistent_returns_404() {
    let state = setup_app().await;
    let random_id = Uuid::new_v4();
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/hosts/{}", random_id))
        .body(Body::empty())
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("host not found"));
}

#[tokio::test]
async fn host_detail_existing_returns_200() {
    let state = setup_app().await;
    let host_id = Uuid::new_v4();

    // Register a host first
    let register_body = register_body(&host_id, "detail-host", "staging");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/agents/register")
        .header("content-type", "application/json")
        .header("X-Enrollment-Token", "test-secret")
        .body(Body::from(serde_json::to_vec(&register_body).unwrap()))
        .unwrap();
    let (reg_status, _reg_json) = send_req(state.clone(), req).await;
    assert_eq!(reg_status, StatusCode::CREATED);

    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/hosts/{}", host_id))
        .body(Body::empty())
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json.get("host")
            .unwrap()
            .get("hostname")
            .unwrap()
            .as_str()
            .unwrap(),
        "detail-host"
    );
}

// --- Changes tests ---

#[tokio::test]
async fn changes_list_returns_200_with_changes_array() {
    let state = setup_app().await;
    let req = Request::builder()
        .method("GET")
        .uri("/v1/changes")
        .body(Body::empty())
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.get("changes").is_some());
    assert!(json.get("changes").unwrap().is_array());
}

#[tokio::test]
async fn change_detail_nonexistent_returns_404() {
    let state = setup_app().await;
    let random_id = Uuid::new_v4();
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/changes/{}", random_id))
        .body(Body::empty())
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("event not found"));
}

// --- Metrics tests ---

#[tokio::test]
async fn metrics_returns_200_with_snapshot() {
    let state = setup_app().await;
    let req = Request::builder()
        .method("GET")
        .uri("/v1/metrics")
        .body(Body::empty())
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.get("events_ingested").is_some());
}

// --- File query tests ---

#[tokio::test]
async fn file_stat_missing_host_id_returns_400() {
    let state = setup_app().await;
    let body = serde_json::json!({ "path": "/etc/test.yaml" });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/file/stat")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("missing host_id"));
}

#[tokio::test]
async fn file_stat_missing_path_returns_400() {
    let state = setup_app().await;
    let host_id = Uuid::new_v4();
    let body = serde_json::json!({ "host_id": host_id.to_string() });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/file/stat")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("missing path"));
}

#[tokio::test]
async fn file_stat_host_not_found_returns_404() {
    let state = setup_app().await;
    let host_id = Uuid::new_v4();
    let body = file_stat_body(&host_id, "/etc/test.yaml");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/file/stat")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("host not found"));
}

#[tokio::test]
async fn file_stat_host_offline_returns_503() {
    let state = setup_app().await;
    let host_id = Uuid::new_v4();
    let pool = state.db.pool().clone();

    db_helpers::seed_host(&pool, host_id, "offline-host", "default")
        .await
        .unwrap();
    db_helpers::set_host_status(&pool, host_id, "offline")
        .await
        .unwrap();

    let body = file_stat_body(&host_id, "/etc/test.yaml");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/file/stat")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("host is offline"));
}

#[tokio::test]
async fn file_preview_missing_host_id_returns_400() {
    let state = setup_app().await;
    let body = serde_json::json!({ "path": "/etc/test.yaml" });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/file/preview")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("missing host_id"));
}
