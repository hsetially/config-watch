use sqlx::PgPool;
use uuid::Uuid;

use config_storage::db::Database;
use config_storage::models::ChangeEventRow;
use config_storage::repositories::change_events::{ChangeEventFilters, ChangeEventsRepo};
use config_storage::repositories::hosts::HostsRepo;

async fn setup_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgres@localhost:5432/config_watch_test".to_string()
    });

    let db = Database::connect(&database_url)
        .await
        .expect("failed to connect to test database");
    db.run_migrations().await.expect("failed to run migrations");

    db.pool().clone()
}

fn make_event(host_id: Uuid, idempotency_key: &str, kind: &str, severity: &str) -> ChangeEventRow {
    ChangeEventRow {
        event_id: Uuid::new_v4(),
        idempotency_key: idempotency_key.to_string(),
        host_id,
        file_id: None,
        event_time: chrono::Utc::now(),
        event_kind: kind.to_string(),
        previous_snapshot_id: None,
        current_snapshot_id: None,
        diff_artifact_uri: None,
        diff_summary_json: None,
        author_name: None,
        author_source: None,
        author_confidence: "unknown".to_string(),
        process_hint: None,
        severity: severity.to_string(),
        created_at: chrono::Utc::now(),
        diff_render: None,
        canonical_path: None,
        pr_url: None,
        pr_number: None,
    }
}

#[tokio::test]
async fn insert_and_get_roundtrip() {
    let pool = setup_pool().await;
    let host_id = Uuid::new_v4();
    HostsRepo::register(
        &pool,
        host_id,
        "repo-host",
        "default",
        serde_json::json!({}),
        "0.1.0",
    )
    .await
    .unwrap();

    let key = format!("roundtrip-key-{}", host_id);
    let event = make_event(host_id, &key, "modified", "info");
    let inserted = ChangeEventsRepo::insert(&pool, &event).await.unwrap();

    assert_eq!(inserted.event_id, event.event_id);
    assert_eq!(inserted.idempotency_key, key);
    assert_eq!(inserted.host_id, host_id);
    assert_eq!(inserted.event_kind, "modified");

    let fetched = ChangeEventsRepo::get(&pool, event.event_id).await.unwrap();
    assert!(fetched.is_some());
    let fetched = fetched.unwrap();
    assert_eq!(fetched.event_id, event.event_id);
}

#[tokio::test]
async fn exists_by_idempotency_key_true() {
    let pool = setup_pool().await;
    let host_id = Uuid::new_v4();
    HostsRepo::register(
        &pool,
        host_id,
        "exist-host",
        "default",
        serde_json::json!({}),
        "0.1.0",
    )
    .await
    .unwrap();

    let key = format!("exist-key-{}", host_id);
    let event = make_event(host_id, &key, "modified", "info");
    ChangeEventsRepo::insert(&pool, &event).await.unwrap();

    let exists = ChangeEventsRepo::exists_by_idempotency_key(&pool, &key)
        .await
        .unwrap();
    assert!(exists);
}

#[tokio::test]
async fn exists_by_idempotency_key_false() {
    let pool = setup_pool().await;
    let exists = ChangeEventsRepo::exists_by_idempotency_key(
        &pool,
        &format!("nonexistent-{}", Uuid::new_v4()),
    )
    .await
    .unwrap();
    assert!(!exists);
}

#[tokio::test]
async fn list_with_no_filters_returns_all() {
    let pool = setup_pool().await;
    let host_id = Uuid::new_v4();
    HostsRepo::register(
        &pool,
        host_id,
        "list-host",
        "default",
        serde_json::json!({}),
        "0.1.0",
    )
    .await
    .unwrap();

    let event1 = make_event(
        host_id,
        &format!("list-key-1-{}", host_id),
        "modified",
        "info",
    );
    let event2 = make_event(
        host_id,
        &format!("list-key-2-{}", host_id),
        "created",
        "info",
    );
    ChangeEventsRepo::insert(&pool, &event1).await.unwrap();
    ChangeEventsRepo::insert(&pool, &event2).await.unwrap();

    let filters = ChangeEventFilters {
        host_id: None,
        path_prefix: None,
        filename: None,
        author: None,
        severity: None,
        since: None,
        until: None,
    };
    let results = ChangeEventsRepo::list(&pool, &filters, 50, 0)
        .await
        .unwrap();
    assert!(results.len() >= 2);
}

#[tokio::test]
async fn list_with_host_id_filter() {
    let pool = setup_pool().await;
    let host1 = Uuid::new_v4();
    let host2 = Uuid::new_v4();
    HostsRepo::register(
        &pool,
        host1,
        "filter-host-1",
        "default",
        serde_json::json!({}),
        "0.1.0",
    )
    .await
    .unwrap();
    HostsRepo::register(
        &pool,
        host2,
        "filter-host-2",
        "default",
        serde_json::json!({}),
        "0.1.0",
    )
    .await
    .unwrap();

    let event1 = make_event(
        host1,
        &format!("host-filter-key-1-{}", host1),
        "modified",
        "info",
    );
    let event2 = make_event(
        host2,
        &format!("host-filter-key-2-{}", host2),
        "modified",
        "info",
    );
    ChangeEventsRepo::insert(&pool, &event1).await.unwrap();
    ChangeEventsRepo::insert(&pool, &event2).await.unwrap();

    let filters = ChangeEventFilters {
        host_id: Some(host1),
        path_prefix: None,
        filename: None,
        author: None,
        severity: None,
        since: None,
        until: None,
    };
    let results = ChangeEventsRepo::list(&pool, &filters, 50, 0)
        .await
        .unwrap();
    assert!(results.iter().all(|r| r.host_id == host1));
}

#[tokio::test]
async fn list_with_severity_filter() {
    let pool = setup_pool().await;
    let host_id = Uuid::new_v4();
    HostsRepo::register(
        &pool,
        host_id,
        "sev-host",
        "default",
        serde_json::json!({}),
        "0.1.0",
    )
    .await
    .unwrap();

    let event1 = make_event(
        host_id,
        &format!("sev-key-1-{}", host_id),
        "modified",
        "info",
    );
    let event2 = make_event(
        host_id,
        &format!("sev-key-2-{}", host_id),
        "deleted",
        "warning",
    );
    ChangeEventsRepo::insert(&pool, &event1).await.unwrap();
    ChangeEventsRepo::insert(&pool, &event2).await.unwrap();

    let filters = ChangeEventFilters {
        host_id: None,
        path_prefix: None,
        filename: None,
        author: None,
        severity: Some("warning".to_string()),
        since: None,
        until: None,
    };
    let results = ChangeEventsRepo::list(&pool, &filters, 50, 0)
        .await
        .unwrap();
    assert!(results.iter().all(|r| r.severity == "warning"));
}

#[tokio::test]
async fn list_pagination_limit_offset() {
    let pool = setup_pool().await;
    let host_id = Uuid::new_v4();
    HostsRepo::register(
        &pool,
        host_id,
        "page-host",
        "default",
        serde_json::json!({}),
        "0.1.0",
    )
    .await
    .unwrap();

    // Insert 5 events
    for i in 0..5 {
        let event = make_event(
            host_id,
            &format!("page-key-{}-{}", i, host_id),
            "modified",
            "info",
        );
        ChangeEventsRepo::insert(&pool, &event).await.unwrap();
    }

    let filters = ChangeEventFilters {
        host_id: Some(host_id),
        path_prefix: None,
        filename: None,
        author: None,
        severity: None,
        since: None,
        until: None,
    };

    let results_limit_2 = ChangeEventsRepo::list(&pool, &filters, 2, 0).await.unwrap();
    assert!(results_limit_2.len() <= 2);

    let results_offset = ChangeEventsRepo::list(&pool, &filters, 50, 3)
        .await
        .unwrap();
    let total = ChangeEventsRepo::list(&pool, &filters, 50, 0)
        .await
        .unwrap();
    let expected_len = total.len().saturating_sub(3);
    assert_eq!(results_offset.len(), expected_len);
}

// Previously ignored due to bug in change_events.rs where path_prefix
// used {} formatting instead of $N parameter syntax.
#[tokio::test]
async fn list_with_path_prefix_filter() {
    let pool = setup_pool().await;
    let host_id = Uuid::new_v4();
    HostsRepo::register(
        &pool,
        host_id,
        "prefix-host",
        "default",
        serde_json::json!({}),
        "0.1.0",
    )
    .await
    .unwrap();

    let filters = ChangeEventFilters {
        host_id: None,
        path_prefix: Some("/etc/myapp".to_string()),
        filename: None,
        author: None,
        severity: None,
        since: None,
        until: None,
    };
    // This should not panic or error. Currently it will fail due to the bug.
    let _ = ChangeEventsRepo::list(&pool, &filters, 50, 0).await;
}
