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

async fn send_req(state: AppState, req: Request<Body>) -> (StatusCode, Value) {
    let app = build_router(state);
    let response = app.oneshot(req).await.unwrap();
    let status = response.status();
    let body = response.into_body();
    let bytes = body.collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

async fn setup_e2e() -> AppState {
    let pool = setup_test_db().await;
    make_app_state(pool, "e2e-secret")
}

#[tokio::test]
async fn e2e_agent_registers_with_control_plane() {
    let state = setup_e2e().await;
    let host_id = Uuid::new_v4();
    let body = register_body(&host_id, "e2e-host", "production");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/agents/register")
        .header("content-type", "application/json")
        .header("X-Enrollment-Token", "e2e-secret")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_req(state.clone(), req).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(json.get("agent_credential").is_some());

    // Verify host appears in list
    let req = Request::builder()
        .method("GET")
        .uri("/v1/hosts")
        .body(Body::empty())
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::OK);
    let hosts = json.get("hosts").unwrap().as_array().unwrap();
    assert!(hosts.iter().any(|h| h.get("hostname").unwrap().as_str().unwrap() == "e2e-host"));
}

#[tokio::test]
async fn e2e_change_event_appears_in_changes_list() {
    let state = setup_e2e().await;
    let host_id = Uuid::new_v4();

    // Register host
    let body = register_body(&host_id, "changes-host", "default");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/agents/register")
        .header("content-type", "application/json")
        .header("X-Enrollment-Token", "e2e-secret")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    send_req(state.clone(), req).await;

    // Ingest a change event
    let token = make_agent_credential("e2e-secret", &host_id.to_string());
    let event_body = make_change_event_json(&host_id, "/etc/myapp/config.yaml", "modified", &format!("e2e-change-{}", host_id));
    let req = Request::builder()
        .method("POST")
        .uri("/v1/events/change")
        .header("content-type", "application/json")
        .header("X-Agent-Token", &token)
        .body(Body::from(serde_json::to_vec(&event_body).unwrap()))
        .unwrap();
    let (status, _) = send_req(state.clone(), req).await;
    assert_eq!(status, StatusCode::CREATED);

    // Verify change appears in list
    let req = Request::builder()
        .method("GET")
        .uri("/v1/changes")
        .body(Body::empty())
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::OK);
    let changes = json.get("changes").unwrap().as_array().unwrap();
    assert!(!changes.is_empty());
}

#[tokio::test]
async fn e2e_duplicate_change_event_returns_409() {
    let state = setup_e2e().await;
    let host_id = Uuid::new_v4();

    // Register host
    let body = register_body(&host_id, "dupe-e2e-host", "default");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/agents/register")
        .header("content-type", "application/json")
        .header("X-Enrollment-Token", "e2e-secret")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    send_req(state.clone(), req).await;

    let token = make_agent_credential("e2e-secret", &host_id.to_string());
    let event_body = make_change_event_json(&host_id, "/etc/dupe.yaml", "modified", &format!("e2e-dupe-{}", host_id));

    let req1 = Request::builder()
        .method("POST")
        .uri("/v1/events/change")
        .header("content-type", "application/json")
        .header("X-Agent-Token", &token)
        .body(Body::from(serde_json::to_vec(&event_body).unwrap()))
        .unwrap();
    let (status1, _) = send_req(state.clone(), req1).await;
    assert_eq!(status1, StatusCode::CREATED);

    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/events/change")
        .header("content-type", "application/json")
        .header("X-Agent-Token", &token)
        .body(Body::from(serde_json::to_vec(&event_body).unwrap()))
        .unwrap();
    let (status2, json2) = send_req(state, req2).await;
    assert_eq!(status2, StatusCode::CONFLICT);
    assert!(json2.get("message").unwrap().as_str().unwrap().contains("duplicate"));
}

#[tokio::test]
async fn e2e_metrics_shows_ingested_count() {
    let state = setup_e2e().await;
    let req = Request::builder()
        .method("GET")
        .uri("/v1/metrics")
        .body(Body::empty())
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.get("events_ingested").is_some());
}

#[tokio::test]
async fn e2e_host_detail_shows_registered_host() {
    let state = setup_e2e().await;
    let host_id = Uuid::new_v4();
    let body = register_body(&host_id, "detail-e2e-host", "staging");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/agents/register")
        .header("content-type", "application/json")
        .header("X-Enrollment-Token", "e2e-secret")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    send_req(state.clone(), req).await;

    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/hosts/{}", host_id))
        .body(Body::empty())
        .unwrap();
    let (status, json) = send_req(state, req).await;
    assert_eq!(status, StatusCode::OK);
    let host = json.get("host").unwrap();
    assert_eq!(host.get("hostname").unwrap().as_str().unwrap(), "detail-e2e-host");
    assert_eq!(host.get("environment").unwrap().as_str().unwrap(), "staging");
}