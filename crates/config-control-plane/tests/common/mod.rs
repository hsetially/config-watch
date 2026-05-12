pub mod db_helpers;

use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use config_control_plane::services::{AppState, AuthState};
use config_storage::db::Database;

#[allow(dead_code)]
pub async fn setup_test_db() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgres@localhost:5432/config_watch_test".to_string()
    });

    let db = Database::connect(&database_url)
        .await
        .expect("failed to connect to test database");
    db.run_migrations().await.expect("failed to run migrations");
    db.pool().clone()
}

pub fn make_test_auth() -> AuthState {
    let config =
        better_auth::AuthConfig::new("test-secret-key-that-is-at-least-32-characters-long")
            .base_url("http://localhost:3000");
    let db = better_auth::adapters::SqlxAdapter::from_pool(
        sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent").unwrap(),
    );
    Arc::new(tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            better_auth::AuthBuilder::new(config)
                .database(db)
                .plugin(better_auth::plugins::EmailPasswordPlugin::new().enable_signup(true))
                .build()
                .await
                .expect("failed to build test auth")
        })
    }))
}

#[allow(dead_code)]
pub fn make_app_state(pool: sqlx::PgPool, secret: &str) -> AppState {
    let db = Database::from_pool(pool);
    let tmp = tempfile::tempdir().expect("create temp dir");
    let snapshot_store = config_snapshot::store::SnapshotStore::new(camino::Utf8Path::new(
        tmp.path().join("snapshots").to_str().unwrap(),
    ))
    .expect("create snapshot store");
    AppState::new(db, secret.to_string(), snapshot_store, make_test_auth())
}

#[allow(dead_code)]
pub fn make_app_state_with_broadcast_capacity(
    pool: sqlx::PgPool,
    secret: &str,
    capacity: usize,
) -> AppState {
    let db = Database::from_pool(pool);
    let tmp = tempfile::tempdir().expect("create temp dir");
    let snapshot_store = config_snapshot::store::SnapshotStore::new(camino::Utf8Path::new(
        tmp.path().join("snapshots").to_str().unwrap(),
    ))
    .expect("create snapshot store");
    AppState::with_broadcast_capacity(
        db,
        secret.to_string(),
        capacity,
        snapshot_store,
        make_test_auth(),
    )
}

#[allow(dead_code)]
pub fn make_agent_credential(secret: &str, host_id: &str) -> String {
    config_auth::tokens::AgentCredential::issue(secret, host_id, chrono::Duration::hours(24)).token
}

#[allow(dead_code)]
pub fn make_expired_credential(secret: &str, host_id: &str) -> String {
    config_auth::tokens::AgentCredential::issue(secret, host_id, chrono::Duration::seconds(-1))
        .token
}

#[allow(dead_code)]
pub fn make_change_event_json(
    host_id: &Uuid,
    path: &str,
    event_kind: &str,
    idempotency_key: &str,
) -> serde_json::Value {
    make_change_event_json_with_severity(host_id, path, event_kind, idempotency_key, "info")
}

#[allow(dead_code)]
pub fn make_change_event_json_with_severity(
    host_id: &Uuid,
    path: &str,
    event_kind: &str,
    idempotency_key: &str,
    severity: &str,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "1.0",
        "event": {
            "event_id": Uuid::new_v4().to_string(),
            "idempotency_key": idempotency_key,
            "host_id": host_id.to_string(),
            "canonical_path": path,
            "event_kind": event_kind,
            "severity": severity,
            "attribution": {
                "author_name": "test-user",
                "confidence": "unknown"
            },
            "diff_summary": {
                "changed_line_estimate": 5,
                "file_size_before": 100,
                "file_size_after": 120,
                "comment_only_hint": false,
                "syntax_equivalent_hint": false
            }
        }
    })
}

#[allow(dead_code)]
pub fn register_body(host_id: &Uuid, hostname: &str, environment: &str) -> serde_json::Value {
    serde_json::json!({
        "host_id": host_id.to_string(),
        "hostname": hostname,
        "environment": environment,
        "labels": {},
        "agent_version": "0.1.0"
    })
}

#[allow(dead_code)]
pub fn heartbeat_body(host_id: &Uuid) -> serde_json::Value {
    serde_json::json!({
        "host_id": host_id.to_string(),
        "status": "healthy",
        "spool_depth": 0,
        "watched_file_count": 5
    })
}

#[allow(dead_code)]
pub fn file_stat_body(host_id: &Uuid, path: &str) -> serde_json::Value {
    serde_json::json!({
        "host_id": host_id.to_string(),
        "path": path
    })
}
