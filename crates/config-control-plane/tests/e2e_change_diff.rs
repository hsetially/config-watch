//! Failure-mode contract tests for the lazy `/v1/changes/{id}/diff` endpoint
//! introduced in the diff-render re-architecture.
//!
//! The happy path (CP successfully fetches snapshots from a real agent and
//! renders) requires a running agent with snapshots on disk; that gets
//! validated on the actual canary deploy. These tests pin the failure modes
//! the dashboard depends on (404, 503, 410-via-agent), so a regression in
//! routing or status mapping fails CI rather than only showing up at click
//! time on the canary.

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

async fn send(state: AppState, req: Request<Body>) -> (StatusCode, Value) {
    let auth = make_test_auth().await;
    let app = build_router(state, auth);
    let response = app.oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

fn diff_request(event_id: Uuid) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(format!("/v1/changes/{}/diff", event_id))
        .body(Body::empty())
        .unwrap()
}

async fn setup() -> AppState {
    let pool = setup_test_db().await;
    make_app_state(pool, "diff-e2e-secret").await
}

#[tokio::test]
async fn diff_endpoint_unknown_event_returns_404() {
    let state = setup().await;
    let (status, json) = send(state, diff_request(Uuid::new_v4())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(json
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .contains("event not found"));
}

#[tokio::test]
async fn diff_endpoint_offline_host_returns_503() {
    // The dashboard relies on 503 here to render the "host offline" banner;
    // anything else (500, 404) makes the failure look like a CP bug and
    // suppresses the user-actionable message.
    let state = setup().await;
    let host_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();

    db_helpers::seed_host(state.db.pool(), host_id, "diff-offline-host", "default")
        .await
        .unwrap();
    db_helpers::set_host_status(state.db.pool(), host_id, "offline")
        .await
        .unwrap();
    db_helpers::seed_change_event(
        state.db.pool(),
        event_id,
        &format!("idem-{}", event_id),
        host_id,
        "modified",
        "info",
    )
    .await
    .unwrap();
    // Backfill canonical_path so the handler doesn't 422 first.
    sqlx::query("UPDATE change_events SET canonical_path = $1 WHERE event_id = $2")
        .bind("/etc/config/foo.yaml")
        .bind(event_id)
        .execute(state.db.pool())
        .await
        .unwrap();

    let (status, json) = send(state, diff_request(event_id)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(json
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .contains("offline"));
}

#[tokio::test]
async fn diff_endpoint_event_without_canonical_path_returns_422() {
    // change_events.canonical_path was added in migration 0003; older rows can
    // legitimately be NULL. Without it the CP can't ask the agent for any
    // file, so 422 communicates "the event itself is incomplete" instead of
    // misleading callers with 503 (host trouble) or 500 (CP bug).
    let state = setup().await;
    let host_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();

    db_helpers::seed_host(state.db.pool(), host_id, "diff-no-path-host", "default")
        .await
        .unwrap();
    db_helpers::seed_change_event(
        state.db.pool(),
        event_id,
        &format!("idem-{}", event_id),
        host_id,
        "modified",
        "info",
    )
    .await
    .unwrap();

    let (status, json) = send(state, diff_request(event_id)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(json
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .contains("canonical_path"));
}

#[tokio::test]
async fn diff_endpoint_unreachable_agent_returns_503() {
    // Healthy host record but the hostname doesn't resolve / port closed.
    // The CP's agent-query fetch fails with a connection error, which
    // `unwrap_snapshot_fetch` maps to FetchStatus::Unreachable → 503.
    let state = setup().await;
    let host_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();

    // Hostname that won't resolve; reqwest will fail fast with a DNS error.
    db_helpers::seed_host(
        state.db.pool(),
        host_id,
        "host-that-will-never-resolve.invalid",
        "default",
    )
    .await
    .unwrap();
    db_helpers::seed_change_event(
        state.db.pool(),
        event_id,
        &format!("idem-{}", event_id),
        host_id,
        "modified",
        "info",
    )
    .await
    .unwrap();
    sqlx::query("UPDATE change_events SET canonical_path = $1 WHERE event_id = $2")
        .bind("/etc/config/foo.yaml")
        .bind(event_id)
        .execute(state.db.pool())
        .await
        .unwrap();

    let (status, json) = send(state, diff_request(event_id)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(json
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .contains("agent unreachable"));
}
