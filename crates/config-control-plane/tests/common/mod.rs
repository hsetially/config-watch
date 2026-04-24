pub mod db_helpers;

use sqlx::PgPool;
use uuid::Uuid;

use config_control_plane::services::AppState;
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

#[allow(dead_code)]
pub fn make_app_state(pool: sqlx::PgPool, secret: &str) -> AppState {
    let db = Database::from_pool(pool);
    let tmp = tempfile::tempdir().expect("create temp dir");
    let snapshot_store = config_snapshot::store::SnapshotStore::new(camino::Utf8Path::new(
        tmp.path().join("snapshots").to_str().unwrap(),
    ))
    .expect("create snapshot store");
    AppState::new(db, secret.to_string(), snapshot_store)
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
    AppState::with_broadcast_capacity(db, secret.to_string(), capacity, snapshot_store)
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
    serde_json::json!({
        "schema_version": "1.0",
        "event": {
            "event_id": Uuid::new_v4().to_string(),
            "idempotency_key": idempotency_key,
            "host_id": host_id.to_string(),
            "canonical_path": path,
            "event_kind": event_kind,
            "severity": "info",
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
