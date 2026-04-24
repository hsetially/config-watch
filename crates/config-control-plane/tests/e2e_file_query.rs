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
    // Tests in this file run in parallel and each uses fresh Uuid::new_v4() host
    // ids, so we rely on UUID scoping for isolation rather than truncating shared
    // tables (which races with other tests' seeded rows).
    let pool = setup_test_db().await;
    make_app_state(pool, "e2e-secret")
}

#[tokio::test]
async fn e2e_file_stat_host_not_found() {
    let state = setup_e2e().await;
    let nonexistent_host = Uuid::new_v4();
    let body = file_stat_body(&nonexistent_host, "/etc/config.yaml");
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
async fn e2e_file_stat_offline_host() {
    let state = setup_e2e().await;
    let host_id = Uuid::new_v4();

    db_helpers::seed_host(state.db.pool(), host_id, "offline-host", "default")
        .await
        .unwrap();
    db_helpers::set_host_status(state.db.pool(), host_id, "offline")
        .await
        .unwrap();

    let body = file_stat_body(&host_id, "/etc/config.yaml");
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
async fn e2e_file_preview_host_not_found() {
    let state = setup_e2e().await;
    let nonexistent_host = Uuid::new_v4();
    let body = serde_json::json!({
        "host_id": nonexistent_host.to_string(),
        "path": "/etc/config.yaml"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/file/preview")
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
async fn e2e_file_preview_offline_host() {
    let state = setup_e2e().await;
    let host_id = Uuid::new_v4();

    db_helpers::seed_host(state.db.pool(), host_id, "offline-preview-host", "default")
        .await
        .unwrap();
    db_helpers::set_host_status(state.db.pool(), host_id, "offline")
        .await
        .unwrap();

    let body = serde_json::json!({
        "host_id": host_id.to_string(),
        "path": "/etc/config.yaml"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/file/preview")
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
