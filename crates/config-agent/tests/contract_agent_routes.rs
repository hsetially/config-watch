use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

use config_agent::api::{build_agent_router, AgentState};
use config_agent::query_handler::QueryHandler;

fn make_agent_state(temp_dir: &std::path::Path) -> AgentState {
    let watch_roots = vec![temp_dir.to_string_lossy().to_string()];
    let redaction_patterns = vec![
        "password".to_string(),
        "secret".to_string(),
        "token".to_string(),
        "key".to_string(),
        "credential".to_string(),
    ];
    let query_handler = QueryHandler::new(watch_roots, redaction_patterns, 4096);
    AgentState {
        query_handler: Arc::new(query_handler),
    }
}

async fn send_request(app: axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let response = app.oneshot(req).await.unwrap();
    let status = response.status();
    let body = response.into_body();
    let bytes = body.collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

#[tokio::test]
async fn file_metadata_missing_path_returns_400() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = make_agent_state(temp_dir.path());
    let app = build_agent_router(state);
    let body = serde_json::json!({});
    let req = Request::builder()
        .method("POST")
        .uri("/v1/query/file-metadata")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_request(app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("missing path"));
}

#[tokio::test]
async fn file_metadata_denied_path_returns_403() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = make_agent_state(temp_dir.path());
    let app = build_agent_router(state);
    let body = serde_json::json!({ "path": "/etc/ssl/cert.pem" });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/query/file-metadata")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_request(app, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("denied by security policy"));
}

#[tokio::test]
async fn file_metadata_outside_watch_root_returns_403() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = make_agent_state(temp_dir.path());
    let app = build_agent_router(state);
    let body = serde_json::json!({ "path": "/etc/some-other-config.yaml" });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/query/file-metadata")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_request(app, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("not in watch roots"));
}

#[tokio::test]
async fn file_metadata_valid_yaml_returns_200() {
    let temp_dir = tempfile::tempdir().unwrap();
    let yaml_path = temp_dir.path().join("config.yaml");
    std::fs::write(&yaml_path, "key: value\n").unwrap();

    let state = make_agent_state(temp_dir.path());
    let app = build_agent_router(state);
    let body = serde_json::json!({ "path": yaml_path.to_string_lossy() });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/query/file-metadata")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_request(app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.get("exists").unwrap().as_bool().unwrap());
    assert!(json.get("is_yaml").unwrap().as_bool().unwrap());
    assert!(json.get("content_hash").is_some());
}

#[tokio::test]
async fn file_metadata_nonexistent_file_returns_200_exists_false() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = make_agent_state(temp_dir.path());
    let app = build_agent_router(state);
    let nonexistent = temp_dir.path().join("nonexistent.yaml");
    let body = serde_json::json!({ "path": nonexistent.to_string_lossy() });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/query/file-metadata")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_request(app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!json.get("exists").unwrap().as_bool().unwrap());
}

#[tokio::test]
async fn file_preview_missing_path_returns_400() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = make_agent_state(temp_dir.path());
    let app = build_agent_router(state);
    let body = serde_json::json!({});
    let req = Request::builder()
        .method("POST")
        .uri("/v1/query/file-preview")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_request(app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("missing path"));
}

#[tokio::test]
async fn file_preview_denied_path_returns_403() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = make_agent_state(temp_dir.path());
    let app = build_agent_router(state);
    let body = serde_json::json!({ "path": "/etc/ssl/cert.pem" });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/query/file-preview")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_request(app, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("denied by security policy"));
}

#[tokio::test]
async fn file_preview_valid_returns_200_with_redaction() {
    let temp_dir = tempfile::tempdir().unwrap();
    let yaml_path = temp_dir.path().join("secrets.yaml");
    std::fs::write(
        &yaml_path,
        "database_password: supersecret\ndatabase_host: localhost\n",
    )
    .unwrap();

    let state = make_agent_state(temp_dir.path());
    let app = build_agent_router(state);
    let body = serde_json::json!({ "path": yaml_path.to_string_lossy() });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/query/file-preview")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_request(app, req).await;
    assert_eq!(status, StatusCode::OK);
    let content = json.get("content").unwrap().as_str().unwrap();
    assert!(
        content.contains("[REDACTED]"),
        "Expected password value to be redacted, got: {}",
        content
    );
    assert!(
        content.contains("database_password"),
        "Key name should still be present"
    );
}
