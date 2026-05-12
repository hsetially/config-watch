use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use std::sync::{Arc, RwLock};
use tower::ServiceExt;

use config_agent::api::{build_agent_router, AgentState, ConfigInfo};
use config_agent::metrics::AgentMetrics;
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
        agent_secret: String::new(),
        metrics: AgentMetrics::new(),
        watch_backend: Arc::new(RwLock::new("unknown".to_string())),
        spool_dir: camino::Utf8PathBuf::from("/tmp/test-spool"),
        snapshot_dir: camino::Utf8PathBuf::from("/tmp/test-snapshots"),
        config_info: Arc::new(ConfigInfo {
            agent_id: "test-agent".to_string(),
            environment: "test".to_string(),
            watch_mode: "auto".to_string(),
            poll_interval_secs: 2,
            watch_roots: vec!["/tmp".to_string()],
        }),
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

// --- Revision-aware preview tests ---

fn make_agent_state_with_snapshots(
    temp_dir: &std::path::Path,
    snapshot_dir: &std::path::Path,
) -> (AgentState, Arc<config_snapshot::store::SnapshotStore>) {
    let watch_roots = vec![temp_dir.to_string_lossy().to_string()];
    let redaction_patterns = vec!["password".to_string(), "secret".to_string()];
    let snapshot_dir_utf8 = camino::Utf8PathBuf::from_path_buf(snapshot_dir.to_path_buf()).unwrap();
    let store = Arc::new(config_snapshot::store::SnapshotStore::new(&snapshot_dir_utf8).unwrap());
    let query_handler = QueryHandler::with_snapshot_store(
        watch_roots,
        redaction_patterns,
        4096,
        Some(store.clone()),
    );
    let state = AgentState {
        query_handler: Arc::new(query_handler),
        agent_secret: String::new(),
        metrics: AgentMetrics::new(),
        watch_backend: Arc::new(RwLock::new("unknown".to_string())),
        spool_dir: camino::Utf8PathBuf::from("/tmp/test-spool"),
        snapshot_dir: snapshot_dir_utf8.clone(),
        config_info: Arc::new(ConfigInfo {
            agent_id: "test-agent".to_string(),
            environment: "test".to_string(),
            watch_mode: "auto".to_string(),
            poll_interval_secs: 2,
            watch_roots: vec![temp_dir.to_string_lossy().to_string()],
        }),
    };
    (state, store)
}

#[tokio::test]
async fn file_preview_explicit_current_revision_reads_from_disk() {
    // Confirms the new `revision: { kind: "current" }` payload behaves the
    // same as omitting `revision` entirely — backwards compatibility for old
    // callers must be byte-identical.
    let temp_dir = tempfile::tempdir().unwrap();
    let snap_dir = tempfile::tempdir().unwrap();
    let yaml_path = temp_dir.path().join("config.yaml");
    // Avoid field names that collide with the built-in redaction defaults
    // (token, secret, password, key, credential, ...).
    std::fs::write(&yaml_path, "app_name: hello\n").unwrap();

    let (state, _store) = make_agent_state_with_snapshots(temp_dir.path(), snap_dir.path());
    let app = build_agent_router(state);
    let body = serde_json::json!({
        "path": yaml_path.to_string_lossy(),
        "revision": { "kind": "current" },
    });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/query/file-preview")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_request(app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json
        .get("content")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("app_name: hello"));
}

#[tokio::test]
async fn file_preview_snapshot_revision_returns_stored_bytes() {
    // Write a snapshot, then mutate the on-disk file. Asking for the snapshot
    // revision must return the stored bytes, not the live ones — this is the
    // load-bearing behavior for the lazy diff plan.
    let temp_dir = tempfile::tempdir().unwrap();
    let snap_dir = tempfile::tempdir().unwrap();
    let yaml_path = temp_dir.path().join("config.yaml");
    let original = b"old: 1\n";
    std::fs::write(&yaml_path, original).unwrap();

    let (state, store) = make_agent_state_with_snapshots(temp_dir.path(), snap_dir.path());
    let hash = config_snapshot::hash::compute_blake3(original);
    store.write_snapshot(&hash, original).await.unwrap();

    // Mutate the live file so disk and snapshot diverge.
    std::fs::write(&yaml_path, b"new: 2\n").unwrap();

    let app = build_agent_router(state);
    let body = serde_json::json!({
        "path": yaml_path.to_string_lossy(),
        "revision": { "kind": "snapshot", "content_hash": hash },
    });
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
        content.contains("old: 1"),
        "snapshot revision should return stored bytes, got: {}",
        content
    );
    assert!(
        !content.contains("new: 2"),
        "snapshot revision must not return live disk bytes",
    );
}

#[tokio::test]
async fn file_preview_snapshot_revision_missing_returns_410() {
    // The CP relies on this status to render "previous unavailable" when
    // retention has evicted the bytes. Returning 500 here would mask the
    // common case as a generic agent failure.
    let temp_dir = tempfile::tempdir().unwrap();
    let snap_dir = tempfile::tempdir().unwrap();
    let yaml_path = temp_dir.path().join("config.yaml");
    std::fs::write(&yaml_path, b"k: v\n").unwrap();

    let (state, _store) = make_agent_state_with_snapshots(temp_dir.path(), snap_dir.path());
    let app = build_agent_router(state);
    let body = serde_json::json!({
        "path": yaml_path.to_string_lossy(),
        "revision": {
            "kind": "snapshot",
            "content_hash": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        },
    });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/query/file-preview")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, json) = send_request(app, req).await;
    assert_eq!(status, StatusCode::GONE);
    assert!(json
        .get("error")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("not present"));
}
