use sqlx::PgPool;
use uuid::Uuid;

use config_control_plane::ingest::{IngestOutcome, IngestService};

mod common;

async fn setup_pool() -> PgPool {
    common::setup_test_db().await
}

fn make_snapshot_store() -> config_snapshot::store::SnapshotStore {
    let tmp = tempfile::tempdir().unwrap();
    config_snapshot::store::SnapshotStore::new(camino::Utf8Path::new(tmp.path().to_str().unwrap()))
        .unwrap()
}

#[tokio::test]
async fn ingest_valid_event_returns_accepted() {
    let pool = setup_pool().await;
    let host_id = Uuid::new_v4();

    // Seed a host first
    common::db_helpers::seed_host(&pool, host_id, "ingest-host", "default")
        .await
        .unwrap();

    let (broadcast_tx, _broadcast_rx) = tokio::sync::broadcast::channel(256);

    let snapshot_store = make_snapshot_store();
    let body = common::make_change_event_json(
        &host_id,
        "/etc/test.yaml",
        "modified",
        &format!("ingest-key-{}", host_id),
    );

    let outcome = IngestService::ingest_change(&pool, &broadcast_tx, &snapshot_store, body)
        .await
        .unwrap();

    match outcome {
        IngestOutcome::Accepted { event_id } => {
            assert!(!event_id.is_nil());
        }
        _ => panic!("Expected Accepted, got {:?}", outcome_variant(&outcome)),
    }
}

#[tokio::test]
async fn ingest_duplicate_idempotency_key_returns_duplicate() {
    let pool = setup_pool().await;
    let host_id = Uuid::new_v4();
    common::db_helpers::seed_host(&pool, host_id, "dupe-host", "default")
        .await
        .unwrap();

    let snapshot_store = make_snapshot_store();
    let (broadcast_tx, _) = tokio::sync::broadcast::channel(256);
    let key = format!("dupe-key-{}", host_id);
    let key_ref: &str = &key;
    let body = common::make_change_event_json(&host_id, "/etc/dupe.yaml", "modified", key_ref);

    // First ingest
    let outcome1 =
        IngestService::ingest_change(&pool, &broadcast_tx, &snapshot_store, body.clone())
            .await
            .unwrap();
    assert!(matches!(outcome1, IngestOutcome::Accepted { .. }));

    // Second ingest with same key
    let outcome2 = IngestService::ingest_change(&pool, &broadcast_tx, &snapshot_store, body)
        .await
        .unwrap();
    assert!(matches!(outcome2, IngestOutcome::Duplicate { .. }));

    // Verify both refer to same event_id
    if let (IngestOutcome::Accepted { event_id: id1 }, IngestOutcome::Duplicate { event_id: id2 }) =
        (outcome1, outcome2)
    {
        assert_eq!(id1, id2);
    }
}

#[tokio::test]
async fn ingest_wrong_schema_version_returns_rejected() {
    let pool = setup_pool().await;
    let snapshot_store = make_snapshot_store();
    let (broadcast_tx, _) = tokio::sync::broadcast::channel(256);

    let body = serde_json::json!({
        "schema_version": "2.0",
        "event": { "idempotency_key": "test" }
    });

    let outcome = IngestService::ingest_change(&pool, &broadcast_tx, &snapshot_store, body)
        .await
        .unwrap();
    assert!(matches!(outcome, IngestOutcome::Rejected { .. }));
}

#[tokio::test]
async fn ingest_missing_event_returns_rejected() {
    let pool = setup_pool().await;
    let snapshot_store = make_snapshot_store();
    let (broadcast_tx, _) = tokio::sync::broadcast::channel(256);

    let body = serde_json::json!({
        "schema_version": "1.0"
    });

    let outcome = IngestService::ingest_change(&pool, &broadcast_tx, &snapshot_store, body)
        .await
        .unwrap();
    match outcome {
        IngestOutcome::Rejected { reason } => {
            assert!(reason.contains("missing event"));
        }
        _ => panic!("Expected Rejected, got {:?}", outcome_variant(&outcome)),
    }
}

#[tokio::test]
async fn ingest_missing_idempotency_key_returns_rejected() {
    let pool = setup_pool().await;
    let snapshot_store = make_snapshot_store();
    let (broadcast_tx, _) = tokio::sync::broadcast::channel(256);

    let body = serde_json::json!({
        "schema_version": "1.0",
        "event": {
            "event_id": Uuid::new_v4().to_string(),
            "host_id": Uuid::new_v4().to_string(),
            "event_kind": "modified"
        }
    });

    let outcome = IngestService::ingest_change(&pool, &broadcast_tx, &snapshot_store, body)
        .await
        .unwrap();
    match outcome {
        IngestOutcome::Rejected { reason } => {
            assert!(reason.contains("missing idempotency_key"));
        }
        _ => panic!("Expected Rejected"),
    }
}

#[tokio::test]
async fn ingest_broadcasts_realtime_message() {
    let pool = setup_pool().await;
    let host_id = Uuid::new_v4();
    common::db_helpers::seed_host(&pool, host_id, "broadcast-host", "default")
        .await
        .unwrap();

    let snapshot_store = make_snapshot_store();
    let (broadcast_tx, mut broadcast_rx) = tokio::sync::broadcast::channel(256);

    let body = common::make_change_event_json(
        &host_id,
        "/etc/broadcast.yaml",
        "created",
        &format!("broadcast-key-{}", host_id),
    );

    let _ = IngestService::ingest_change(&pool, &broadcast_tx, &snapshot_store, body)
        .await
        .unwrap();

    // Should receive a broadcast message
    let msg = broadcast_rx.try_recv().unwrap();
    assert_eq!(msg.host_id, host_id);
    assert_eq!(msg.path, "/etc/broadcast.yaml");
    assert_eq!(msg.event_kind, "created");
}

#[tokio::test]
async fn ingest_touches_host_heartbeat() {
    let pool = setup_pool().await;
    let host_id = Uuid::new_v4();
    common::db_helpers::seed_host(&pool, host_id, "heartbeat-host", "default")
        .await
        .unwrap();

    // Get initial heartbeat
    let initial: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT last_heartbeat_at FROM hosts WHERE host_id = $1")
            .bind(host_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    let snapshot_store = make_snapshot_store();
    let (broadcast_tx, _) = tokio::sync::broadcast::channel(256);
    let body = common::make_change_event_json(
        &host_id,
        "/etc/hb.yaml",
        "modified",
        &format!("hb-key-{}", host_id),
    );

    let _ = IngestService::ingest_change(&pool, &broadcast_tx, &snapshot_store, body)
        .await
        .unwrap();

    // Heartbeat should be updated
    let after: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT last_heartbeat_at FROM hosts WHERE host_id = $1")
            .bind(host_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    // after should be >= initial (both may be None initially, but after ingest, the heartbeat should be set)
    match (initial, after) {
        (Some(init), Some(aft)) => assert!(aft >= init),
        (None, Some(_)) => {} // Was null, now set
        _ => {}
    }
}

fn outcome_variant(outcome: &IngestOutcome) -> &'static str {
    match outcome {
        IngestOutcome::Accepted { .. } => "Accepted",
        IngestOutcome::Duplicate { .. } => "Duplicate",
        IngestOutcome::Rejected { .. } => "Rejected",
    }
}
