use uuid::Uuid;

use config_control_plane::ingest::IngestService;

mod common;

use common::*;

/// E2E test: WebSocket receives realtime events after ingest.
/// This test verifies that after ingesting a change event,
/// a WebSocket subscriber can receive the broadcast message.
#[tokio::test]
async fn e2e_websocket_broadcast_after_ingest() {
    let pool = setup_test_db().await;
    sqlx::query("DELETE FROM change_events")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM file_queries")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM files").execute(&pool).await.ok();
    sqlx::query("DELETE FROM watch_roots")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM hosts").execute(&pool).await.ok();

    let state = make_app_state_with_broadcast_capacity(pool, "ws-secret", 256);
    let mut rx = state.broadcast_tx.subscribe();

    // Register host
    let host_id = Uuid::new_v4();
    db_helpers::seed_host(state.db.pool(), host_id, "ws-host", "production")
        .await
        .unwrap();

    // Ingest a change event directly via IngestService
    let body = make_change_event_json(&host_id, "/etc/ws-app/config.yaml", "modified", "ws-key-1");
    let _outcome = IngestService::ingest_change(
        state.db.pool(),
        &state.broadcast_tx,
        &state.snapshot_store,
        body,
    )
    .await
    .unwrap();

    // Verify the broadcast was received
    let msg = rx.try_recv().unwrap();
    assert_eq!(msg.host_id, host_id);
    assert_eq!(msg.path, "/etc/ws-app/config.yaml");
    assert_eq!(msg.event_kind, "modified");
}

/// E2E test: SubscriptionFilter matches events by environment.
#[tokio::test]
async fn e2e_websocket_filter_by_environment() {
    let pool = setup_test_db().await;
    sqlx::query("DELETE FROM change_events")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM file_queries")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM files").execute(&pool).await.ok();
    sqlx::query("DELETE FROM watch_roots")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM hosts").execute(&pool).await.ok();

    let state = make_app_state_with_broadcast_capacity(pool, "ws-secret", 256);

    let host_prod = Uuid::new_v4();
    let host_dev = Uuid::new_v4();
    db_helpers::seed_host(state.db.pool(), host_prod, "prod-host", "production")
        .await
        .unwrap();
    db_helpers::seed_host(state.db.pool(), host_dev, "dev-host", "development")
        .await
        .unwrap();

    let filter = config_control_plane::realtime::SubscriptionFilter {
        environment: Some("production".to_string()),
        host_id: None,
        path_prefix: None,
        severity: None,
    };

    let mut rx = state.broadcast_tx.subscribe();

    // Ingest event for production host
    let body_prod = make_change_event_json(
        &host_prod,
        "/etc/prod/config.yaml",
        "modified",
        "ws-prod-key",
    );
    IngestService::ingest_change(
        state.db.pool(),
        &state.broadcast_tx,
        &state.snapshot_store,
        body_prod,
    )
    .await
    .unwrap();

    // Ingest event for development host
    let body_dev =
        make_change_event_json(&host_dev, "/etc/dev/config.yaml", "modified", "ws-dev-key");
    IngestService::ingest_change(
        state.db.pool(),
        &state.broadcast_tx,
        &state.snapshot_store,
        body_dev,
    )
    .await
    .unwrap();

    // Read messages and filter
    let mut prod_count = 0;
    let mut dev_count = 0;
    while let Ok(msg) = rx.try_recv() {
        if filter.matches(&msg) {
            prod_count += 1;
        } else {
            dev_count += 1;
        }
    }
    assert!(
        prod_count >= 1,
        "Expected at least one production event to match filter"
    );
    assert!(
        dev_count >= 1,
        "Expected at least one development event to not match filter"
    );
}

/// E2E test: SubscriptionFilter matches events by severity.
#[tokio::test]
async fn e2e_websocket_filter_by_severity() {
    let pool = setup_test_db().await;
    sqlx::query("DELETE FROM change_events")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM file_queries")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM files").execute(&pool).await.ok();
    sqlx::query("DELETE FROM watch_roots")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM hosts").execute(&pool).await.ok();

    let state = make_app_state_with_broadcast_capacity(pool, "ws-secret", 256);

    let host_id = Uuid::new_v4();
    db_helpers::seed_host(state.db.pool(), host_id, "sev-host", "default")
        .await
        .unwrap();

    let filter = config_control_plane::realtime::SubscriptionFilter {
        environment: None,
        host_id: None,
        path_prefix: None,
        severity: Some("warning".to_string()),
    };

    let mut rx = state.broadcast_tx.subscribe();

    // Ingest warning event (severity: warning via event body)
    let body_warning = make_change_event_json(&host_id, "/etc/warn.yaml", "deleted", "ws-warn-key");
    IngestService::ingest_change(
        state.db.pool(),
        &state.broadcast_tx,
        &state.snapshot_store,
        body_warning,
    )
    .await
    .unwrap();

    // Read messages
    let mut warning_count = 0;
    while let Ok(msg) = rx.try_recv() {
        if filter.matches(&msg) {
            warning_count += 1;
        }
    }
    assert!(
        warning_count >= 1,
        "Expected at least one warning event to match severity filter"
    );
}
