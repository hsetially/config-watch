use std::collections::HashMap;

use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use config_agent::debounce::{DebouncedEvent, DebounceWindow};
use config_agent::pipeline::{Pipeline, SnapshotDecision};
use config_agent::config::{AgentConfig, WatchRootConfig};
use config_agent::watcher::{RawEventKind, RawWatchEvent};
use config_shared::events::{ChangeKind, Severity};
use config_shared::ids::HostId;

fn make_config(watch_root: &str) -> AgentConfig {
    AgentConfig {
        agent_id: "00000000-0000-0000-0000-000000000001".to_string(),
        environment: "default".to_string(),
        host_labels: HashMap::new(),
        control_plane_base_url: "http://localhost:8080".to_string(),
        watch_roots: vec![WatchRootConfig {
            root_path: Utf8PathBuf::from(watch_root),
            recursive: true,
        }],
        include_globs: vec!["**/*.yaml".to_string(), "**/*.yml".to_string()],
        exclude_globs: vec!["**/.*".to_string(), "**/*.sw?".to_string()],
        debounce_window_ms: 500,
        snapshot_dir: Utf8PathBuf::from("/tmp/test-snapshots"),
        spool_dir: Utf8PathBuf::from("/tmp/test-spool"),
        enrollment_token: "test-token".to_string(),
        content_preview_max_bytes: 4096,
        redaction_patterns: vec![],
        heartbeat_interval_secs: 30,
        query_timeout_secs: 10,
        max_spool_events: 10000,
        max_spool_bytes: 500 * 1024 * 1024,
        max_file_size_bytes: 1024 * 1024,
        agent_api_bind_addr: "0.0.0.0:9090".to_string(),
        tunnel_enabled: true,
        tunnel_reconnect_base_secs: 1,
        tunnel_reconnect_max_secs: 30,
        diff: config_diff::DiffConfig::default(),
    }
}

fn make_debounced_event(path: &str, kind: ChangeKind) -> DebouncedEvent {
    DebouncedEvent {
        canonical_path: Utf8PathBuf::from(path),
        event_kind: kind,
        observed_at: Utc::now(),
        raw_event_count: 1,
    }
}

#[tokio::test]
async fn debounce_suppresses_burst_to_single_event() {
    let mut window = DebounceWindow::new(500);
    let path = "/etc/app/config.yaml";

    for _ in 0..10 {
        window.ingest(
            RawWatchEvent {
                raw_path: Utf8PathBuf::from(path),
                event_kind: RawEventKind::Modified,
                observed_at: Utc::now(),
            },
            true,
        );
    }

    let flushed = window.flush_all();
    assert!(flushed.len() <= 1, "Expected at most 1 debounced event from burst, got {}", flushed.len());
}

#[test]
fn debounce_maps_created_correctly() {
    let mut window = DebounceWindow::new(500);
    window.ingest(
        RawWatchEvent {
            raw_path: Utf8PathBuf::from("/etc/new.yaml"),
            event_kind: RawEventKind::Created,
            observed_at: Utc::now(),
        },
        false,
    );

    let flushed = window.flush_all();
    assert_eq!(flushed.len(), 1);
    assert!(matches!(flushed[0].event_kind, ChangeKind::Created));
}

#[test]
fn debounce_maps_modified_on_existing_file() {
    let mut window = DebounceWindow::new(500);
    window.ingest(
        RawWatchEvent {
            raw_path: Utf8PathBuf::from("/etc/existing.yaml"),
            event_kind: RawEventKind::Modified,
            observed_at: Utc::now(),
        },
        true,
    );

    let flushed = window.flush_all();
    assert_eq!(flushed.len(), 1);
    assert!(matches!(flushed[0].event_kind, ChangeKind::Modified));
}

#[tokio::test]
async fn pipeline_snapshot_acquire_detects_new_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("new-file.yaml");
    std::fs::write(&file_path, "key: value\n").unwrap();

    let config = make_config(temp_dir.path().to_str().unwrap());
    let host_id = HostId::new();
    let pipeline = Pipeline::new(config, host_id);

    let snapshot_dir = Utf8PathBuf::from_path_buf(temp_dir.path().join("snapshots")).unwrap();
    std::fs::create_dir_all(snapshot_dir.as_std_path()).unwrap();
    let store = config_snapshot::store::SnapshotStore::new(&snapshot_dir).unwrap();

    let event = make_debounced_event(file_path.to_str().unwrap(), ChangeKind::Created);
    let decision = pipeline.snapshot_acquire(&event, &store).await.unwrap();
    match decision {
        SnapshotDecision::FileCreated { current_hash, .. } => {
            assert!(!current_hash.is_empty());
        }
        _ => panic!("Expected FileCreated, got {:?}", decision_variant(&decision)),
    }
}

#[tokio::test]
async fn pipeline_snapshot_acquire_detects_changed_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("changed.yaml");
    std::fs::write(&file_path, "key: value\n").unwrap();

    let config = make_config(temp_dir.path().to_str().unwrap());
    let host_id = HostId::new();
    let pipeline = Pipeline::new(config, host_id);

    let snapshot_dir = Utf8PathBuf::from_path_buf(temp_dir.path().join("snapshots")).unwrap();
    std::fs::create_dir_all(snapshot_dir.as_std_path()).unwrap();
    let store = config_snapshot::store::SnapshotStore::new(&snapshot_dir).unwrap();

    let event = make_debounced_event(file_path.to_str().unwrap(), ChangeKind::Created);
    let decision = pipeline.snapshot_acquire(&event, &store).await.unwrap();
    match decision {
        SnapshotDecision::FileCreated { current_hash, current_data } => {
            let _ = store.write_snapshot(&current_hash, &current_data).await.unwrap();
            store.set_current_hash(Utf8Path::new(file_path.to_str().unwrap()), &current_hash).unwrap();
        }
        _ => panic!("Expected FileCreated, got {:?}", decision_variant(&decision)),
    }

    std::fs::write(&file_path, "key: new_value\n").unwrap();
    let event2 = make_debounced_event(file_path.to_str().unwrap(), ChangeKind::Modified);
    let decision = pipeline.snapshot_acquire(&event2, &store).await.unwrap();
    match decision {
        SnapshotDecision::Changed { previous_hash, current_hash, previous_data, .. } => {
            assert_ne!(previous_hash, current_hash);
            assert_eq!(previous_data, b"key: value\n", "previous_data should contain the old file content");
        }
        _ => panic!("Expected Changed, got {:?}", decision_variant(&decision)),
    }
}

#[tokio::test]
async fn pipeline_snapshot_acquire_detects_unchanged_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("unchanged.yaml");
    std::fs::write(&file_path, "key: value\n").unwrap();

    let config = make_config(temp_dir.path().to_str().unwrap());
    let host_id = HostId::new();
    let pipeline = Pipeline::new(config, host_id);

    let snapshot_dir = Utf8PathBuf::from_path_buf(temp_dir.path().join("snapshots")).unwrap();
    std::fs::create_dir_all(snapshot_dir.as_std_path()).unwrap();
    let store = config_snapshot::store::SnapshotStore::new(&snapshot_dir).unwrap();

    let event = make_debounced_event(file_path.to_str().unwrap(), ChangeKind::Created);
    let decision = pipeline.snapshot_acquire(&event, &store).await.unwrap();
    match decision {
        SnapshotDecision::FileCreated { current_hash, current_data } => {
            let _ = store.write_snapshot(&current_hash, &current_data).await.unwrap();
            store.set_current_hash(Utf8Path::new(file_path.to_str().unwrap()), &current_hash).unwrap();
        }
        _ => panic!("Expected FileCreated"),
    }

    let event2 = make_debounced_event(file_path.to_str().unwrap(), ChangeKind::Modified);
    let decision2 = pipeline.snapshot_acquire(&event2, &store).await.unwrap();
    assert!(matches!(decision2, SnapshotDecision::Unchanged));
}

#[tokio::test]
async fn pipeline_snapshot_acquire_detects_deleted_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("to-delete.yaml");

    let config = make_config(temp_dir.path().to_str().unwrap());
    let host_id = HostId::new();
    let pipeline = Pipeline::new(config, host_id);

    let snapshot_dir = Utf8PathBuf::from_path_buf(temp_dir.path().join("snapshots")).unwrap();
    std::fs::create_dir_all(snapshot_dir.as_std_path()).unwrap();
    let store = config_snapshot::store::SnapshotStore::new(&snapshot_dir).unwrap();

    std::fs::write(&file_path, "key: value\n").unwrap();
    let event = make_debounced_event(file_path.to_str().unwrap(), ChangeKind::Created);
    let decision = pipeline.snapshot_acquire(&event, &store).await.unwrap();
    match decision {
        SnapshotDecision::FileCreated { current_hash, current_data } => {
            let _ = store.write_snapshot(&current_hash, &current_data).await.unwrap();
            store.set_current_hash(Utf8Path::new(file_path.to_str().unwrap()), &current_hash).unwrap();
        }
        _ => panic!("Expected FileCreated"),
    }

    std::fs::remove_file(&file_path).unwrap();
    let event2 = make_debounced_event(file_path.to_str().unwrap(), ChangeKind::Deleted);
    let decision = pipeline.snapshot_acquire(&event2, &store).await.unwrap();
    assert!(matches!(decision, SnapshotDecision::FileDeleted { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_diff_generate_produces_diff_output() {
    let config = make_config("/tmp");
    let host_id = HostId::new();
    let pipeline = Pipeline::new(config, host_id);
    let path = Utf8PathBuf::from("/etc/test.yaml");

    let result = pipeline.diff_generate("old: content\n", "new: content\n", &path).await;
    assert!(result.is_ok());
}

#[test]
fn pipeline_build_change_event_includes_idempotency_key() {
    let config = make_config("/tmp");
    let host_id = HostId::new();
    let pipeline = Pipeline::new(config, host_id);

    let event = make_debounced_event("/etc/test.yaml", ChangeKind::Modified);
    let decision = SnapshotDecision::Changed {
        previous_hash: "abc123".to_string(),
        current_hash: "def456".to_string(),
        previous_data: Vec::new(),
        current_data: Vec::new(),
    };
    let attribution = config_shared::attribution::Attribution::unknown();

    let change_event = pipeline.build_change_event(&event, &decision, None, None, attribution);
    assert!(!change_event.idempotency_key.to_string().is_empty());
}

#[test]
fn pipeline_build_change_event_sets_severity_info_on_delete() {
    let config = make_config("/tmp");
    let host_id = HostId::new();
    let pipeline = Pipeline::new(config, host_id);

    let event = make_debounced_event("/etc/test.yaml", ChangeKind::Deleted);
    let decision = SnapshotDecision::FileDeleted {
        previous_hash: "abc123".to_string(),
        previous_data: Vec::new(),
    };
    let attribution = config_shared::attribution::Attribution::unknown();

    let change_event = pipeline.build_change_event(&event, &decision, None, None, attribution);
    assert_eq!(change_event.severity, Severity::Info);
}

#[test]
fn pipeline_build_change_event_sets_severity_info_on_large_diff() {
    let config = make_config("/tmp");
    let host_id = HostId::new();
    let pipeline = Pipeline::new(config, host_id);

    let event = make_debounced_event("/etc/test.yaml", ChangeKind::Modified);
    let decision = SnapshotDecision::Changed {
        previous_hash: "abc123".to_string(),
        current_hash: "def456".to_string(),
        previous_data: Vec::new(),
        current_data: Vec::new(),
    };
    let attribution = config_shared::attribution::Attribution::unknown();
    let diff_summary = Some(config_shared::snapshots::DiffSummary {
        changed_line_estimate: 100,
        file_size_before: 500,
        file_size_after: 1000,
        comment_only_hint: false,
        syntax_equivalent_hint: false,
        yaml_lint_findings: vec![],
    });

    let change_event = pipeline.build_change_event(&event, &decision, diff_summary, None, attribution);
    assert_eq!(change_event.severity, Severity::Info);
}

fn decision_variant(d: &SnapshotDecision) -> &'static str {
    match d {
        SnapshotDecision::Unchanged => "Unchanged",
        SnapshotDecision::Changed { .. } => "Changed",
        SnapshotDecision::FileCreated { .. } => "FileCreated",
        SnapshotDecision::FileDeleted { .. } => "FileDeleted",
    }
}