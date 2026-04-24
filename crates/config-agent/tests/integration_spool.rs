use camino::Utf8PathBuf;
use chrono::Utc;
use config_agent::spool::{DeliveryStatus, SpoolWriter};
use config_shared::attribution::Attribution;
use config_shared::events::{ChangeEvent, ChangeKind, Severity};
use config_shared::ids::{EventId, HostId, IdempotencyKey};

fn make_event(id: &str) -> ChangeEvent {
    ChangeEvent {
        event_id: EventId(uuid::Uuid::parse_str(id).unwrap()),
        idempotency_key: IdempotencyKey("test".into()),
        host_id: HostId::new(),
        canonical_path: Utf8PathBuf::from("/test/config.yaml"),
        event_time: Utc::now(),
        event_kind: ChangeKind::Modified,
        previous_snapshot_id: None,
        current_snapshot_id: None,
        diff_summary: None,
        diff_render: None,
        attribution: Attribution::unknown(),
        severity: Severity::Info,
        content_b64: None,
    }
}

#[tokio::test]
async fn spool_append_creates_pending_entry() {
    let dir = tempfile::tempdir().unwrap();
    let spool_dir = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let writer = SpoolWriter::new(&spool_dir, 100, 1024 * 1024).unwrap();

    let event = make_event("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    let entry = writer.append(&event).await.unwrap();
    assert_eq!(entry.status, DeliveryStatus::Pending);

    let pending = writer.pending_entries().await.unwrap();
    assert_eq!(pending.len(), 1);
}

#[tokio::test]
async fn spool_mark_delivered_removes_from_pending() {
    let dir = tempfile::tempdir().unwrap();
    let spool_dir = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let writer = SpoolWriter::new(&spool_dir, 100, 1024 * 1024).unwrap();

    let event = make_event("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    writer.append(&event).await.unwrap();
    writer.mark_delivered(&event.event_id).await.unwrap();

    let pending = writer.pending_entries().await.unwrap();
    assert!(pending.is_empty());
}

#[tokio::test]
async fn spool_mark_failed_removes_from_pending() {
    let dir = tempfile::tempdir().unwrap();
    let spool_dir = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let writer = SpoolWriter::new(&spool_dir, 100, 1024 * 1024).unwrap();

    let event = make_event("cccccccc-cccc-cccc-cccc-cccccccccccc");
    writer.append(&event).await.unwrap();
    writer
        .mark_failed(&event.event_id, "permanent error")
        .await
        .unwrap();

    let pending = writer.pending_entries().await.unwrap();
    assert!(pending.is_empty());
}

#[test]
fn spool_increment_attempts_tracks_count() {
    let dir = tempfile::tempdir().unwrap();
    let spool_dir = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let writer = SpoolWriter::new(&spool_dir, 100, 1024 * 1024).unwrap();

    // Need async block for append, but increment_attempts is sync
    let rt = tokio::runtime::Runtime::new().unwrap();
    let event = make_event("dddddddd-dddd-dddd-dddd-dddddddddddd");
    rt.block_on(writer.append(&event)).unwrap();

    let count = writer.increment_attempts(&event.event_id).unwrap();
    assert_eq!(count, 1);
    let count2 = writer.increment_attempts(&event.event_id).unwrap();
    assert_eq!(count2, 2);
}

#[test]
fn spool_pending_count_matches() {
    let dir = tempfile::tempdir().unwrap();
    let spool_dir = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let writer = SpoolWriter::new(&spool_dir, 100, 1024 * 1024).unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let event1 = make_event("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee");
    let event2 = make_event("ffffffff-ffff-ffff-ffff-ffffffffffff");
    rt.block_on(writer.append(&event1)).unwrap();
    rt.block_on(writer.append(&event2)).unwrap();

    assert_eq!(writer.pending_count(), 2);
}

#[tokio::test]
async fn spool_replay_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let spool_dir = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();

    // Create writer, append events
    let writer1 = SpoolWriter::new(&spool_dir, 100, 1024 * 1024).unwrap();
    let event1 = make_event("11111111-1111-1111-1111-111111111111");
    let event2 = make_event("22222222-2222-2222-2222-222222222222");
    writer1.append(&event1).await.unwrap();
    writer1.append(&event2).await.unwrap();

    // Mark event1 as delivered
    writer1.mark_delivered(&event1.event_id).await.unwrap();
    drop(writer1);

    // Create new writer from same dir — should find event2 still pending
    let writer2 = SpoolWriter::new(&spool_dir, 100, 1024 * 1024).unwrap();
    let pending = writer2.pending_entries().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].event.event_id, event2.event_id);
}

#[test]
fn spool_index_persists_status() {
    let dir = tempfile::tempdir().unwrap();
    let spool_dir = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let writer = SpoolWriter::new(&spool_dir, 100, 1024 * 1024).unwrap();

    let event = make_event("33333333-3333-3333-3333-333333333333");
    rt.block_on(writer.append(&event)).unwrap();

    // Verify index file exists
    let index_path = spool_dir.join("index.json");
    assert!(index_path.exists(), "Index file should exist after append");

    let content = std::fs::read_to_string(&index_path).unwrap();
    assert!(
        content.contains(&event.event_id.to_string()),
        "Index should contain event ID"
    );
}
