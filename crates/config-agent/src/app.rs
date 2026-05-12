use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::api::{build_agent_router, AgentState};
use crate::config::AgentConfig;
use crate::debounce::DebounceWindow;
use crate::pipeline::Pipeline;
use crate::publish::EventPublisher;
use crate::query_handler::QueryHandler;
use crate::spool::SpoolWriter;
use crate::watcher::FileWatcher;

pub async fn run(cfg: AgentConfig) -> anyhow::Result<()> {
    let host_id = cfg.host_id()?;

    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    tracing::info!(
        agent_id = %cfg.agent_id,
        environment = %cfg.environment,
        roots = ?cfg.watch_roots,
        watch_mode = %cfg.watch_mode,
        poll_interval_secs = cfg.poll_interval_secs,
        "agent starting"
    );

    // Structured startup diagnostics
    for root in &cfg.watch_roots {
        let mount_info = crate::watcher::detect_mount_info(root.root_path.as_ref());
        tracing::info!(
            path = %mount_info.path,
            mount_point = ?mount_info.mount_point,
            fs_type = ?mount_info.fs_type,
            is_nfs = mount_info.is_nfs,
            exists = root.root_path.exists(),
            "watch root mount info"
        );
    }
    let (difft_path, difft_available) = config_diff::find_difft_binary();
    tracing::info!(
        difftastic_available = difft_available,
        difftastic_path = %difft_path.display(),
        "difftastic detection"
    );
    tracing::info!(
        snapshot_dir = %cfg.snapshot_dir,
        spool_dir = %cfg.spool_dir,
        snapshot_dir_exists = cfg.snapshot_dir.exists(),
        spool_dir_exists = cfg.spool_dir.exists(),
        "storage directories"
    );

    let snapshot_store = Arc::new(config_snapshot::store::SnapshotStore::new(
        &cfg.snapshot_dir,
    )?);
    let spool = SpoolWriter::new(&cfg.spool_dir, cfg.max_spool_events, cfg.max_spool_bytes)?;
    let pipeline = Pipeline::new(cfg.clone(), host_id);

    // Baseline scan: snapshot all existing files so the first modification
    // has a proper previous version to diff against.
    match crate::pipeline::baseline_scan(&cfg, snapshot_store.as_ref()).await {
        Ok(stats) => tracing::info!(
            scanned = stats.files_scanned,
            baselines = stats.baselines_created,
            skipped = stats.skipped_existing,
            "baseline scan complete"
        ),
        Err(e) => tracing::warn!(error = %e, "baseline scan failed, continuing without baselines"),
    }

    // Register with control plane
    let publisher = EventPublisher::new(
        &cfg.control_plane_base_url,
        &cfg.enrollment_token,
        host_id.0,
        &hostname,
        &cfg.environment,
        "0.1.0",
        cfg.tls_required,
    );

    match publisher.register(serde_json::json!({})).await {
        Ok(resp) => {
            tracing::info!(credential_expires = ?resp.credential_expires_at, "registered with control plane")
        }
        Err(e) => tracing::warn!(error = %e, "registration failed, will retry on heartbeat"),
    }

    // Publish initial snapshots for all tracked files
    if let Err(e) =
        crate::pipeline::publish_initial_snapshots(&publisher, snapshot_store.as_ref(), host_id)
            .await
    {
        tracing::warn!(error = %e, "initial snapshot publishing failed, continuing without initial state sync");
    }

    // Replay pending spool entries
    let pending = spool.pending_entries().await?;
    for entry in &pending {
        tracing::info!(event_id = %entry.event.event_id, "replaying spool entry");
        match publisher
            .publish(&entry.event, &entry.event.idempotency_key)
            .await
        {
            Ok(_) => {
                if let Err(e) = spool.mark_delivered(&entry.event.event_id).await {
                    tracing::warn!(error = %e, "failed to mark replayed event delivered");
                }
            }
            Err(e) => tracing::warn!(error = %e, "replay publish failed, will retry later"),
        }
    }

    // Spawn agent query API server
    let watch_roots: Vec<String> = cfg
        .watch_roots
        .iter()
        .map(|r| r.root_path.to_string())
        .collect();
    let query_handler = Arc::new(QueryHandler::with_snapshot_store(
        watch_roots.clone(),
        cfg.redaction_patterns.clone(),
        cfg.content_preview_max_bytes,
        Some(snapshot_store.clone()),
    ));

    // Create shared state for watcher backend reporting
    let watch_backend: Arc<RwLock<String>> = Arc::new(RwLock::new(String::new()));
    let metrics = crate::metrics::AgentMetrics::new();

    // Start tunnel if enabled (requires successful registration to have a valid token)
    let tunnel_handle = if cfg.tunnel_enabled {
        let tunnel = crate::tunnel::AgentTunnel::new(cfg.clone(), query_handler.clone());
        let tunnel_token = publisher.current_token();
        Some(tokio::spawn(async move {
            tunnel.run(tunnel_token).await;
        }))
    } else {
        None
    };

    let config_info = Arc::new(crate::api::ConfigInfo {
        agent_id: cfg.agent_id.clone(),
        environment: cfg.environment.clone(),
        watch_mode: cfg.watch_mode.clone(),
        poll_interval_secs: cfg.poll_interval_secs,
        watch_roots,
    });

    let agent_state = AgentState {
        query_handler,
        agent_secret: cfg.agent_api_secret.clone(),
        metrics: metrics.clone(),
        watch_backend: watch_backend.clone(),
        spool_dir: cfg.spool_dir.clone(),
        snapshot_dir: cfg.snapshot_dir.clone(),
        config_info: config_info.clone(),
    };
    let agent_router = build_agent_router(agent_state);
    let api_bind = cfg.agent_api_bind_addr.clone();
    let api_handle = tokio::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(&api_bind).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(addr = %api_bind, error = %e, "failed to bind agent API");
                return;
            }
        };
        tracing::info!(addr = %api_bind, "agent query API listening");
        if let Err(e) = axum::serve(listener, agent_router).await {
            tracing::error!(error = %e, "agent API server error");
        }
    });

    // Heartbeat timer
    let heartbeat_interval = Duration::from_secs(cfg.heartbeat_interval_secs);
    let heartbeat_publisher = publisher.clone_spool_depth_handle();
    let heartbeat_spool = spool.clone_for_heartbeat();
    let heartbeat_metrics = metrics.clone();
    let heartbeat_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(heartbeat_interval);
        loop {
            interval.tick().await;
            let depth = heartbeat_spool.pending_count();
            heartbeat_metrics
                .spool_depth
                .store(depth as u64, std::sync::atomic::Ordering::Relaxed);
            if let Err(e) = heartbeat_publisher.heartbeat(depth, 0).await {
                tracing::warn!(error = %e, "heartbeat failed");
            } else {
                heartbeat_metrics.last_control_plane_contact.store(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                    std::sync::atomic::Ordering::Relaxed,
                );
            }
        }
    });

    // Periodic retention enforcement (every 6 hours)
    let retention_store = config_snapshot::store::SnapshotStore::new(&cfg.snapshot_dir)?;
    let retention_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(6 * 3600));
        let config = config_snapshot::retention::RetentionConfig::default();
        loop {
            interval.tick().await;
            match config_snapshot::retention::enforce_retention(&retention_store, &config).await {
                Ok(stats) if stats.snapshots_removed > 0 => {
                    tracing::info!(
                        removed = stats.snapshots_removed,
                        freed_bytes = stats.bytes_freed,
                        "retention enforcement completed"
                    );
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "retention enforcement failed"),
            }
        }
    });

    let (raw_tx, mut raw_rx) = mpsc::channel::<crate::watcher::RawWatchEvent>(256);
    let (debounced_tx, mut debounced_rx) = mpsc::channel::<crate::debounce::DebouncedEvent>(64);

    let watcher = FileWatcher::new(cfg.clone(), raw_tx, watch_backend.clone());
    watcher.start().await?;

    let debounce_window_ms = cfg.debounce_window_ms;
    let debounce_handle = tokio::spawn(async move {
        let mut window = DebounceWindow::new(debounce_window_ms);
        let mut interval = tokio::time::interval(Duration::from_millis(debounce_window_ms));

        loop {
            tokio::select! {
                Some(event) = raw_rx.recv() => {
                    let exists = event.raw_path.exists();
                    window.ingest(event, exists);
                }
                _ = interval.tick() => {
                    let expired = window.flush_expired();
                    for event in expired {
                        if debounced_tx.send(event).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    });

    let shutdown = tokio::signal::ctrl_c();

    tokio::pin!(shutdown);

    let max_publish_retries: u32 = 5;

    loop {
        tokio::select! {
            Some(event) = debounced_rx.recv() => {
                tracing::debug!(path = %event.canonical_path, kind = ?event.event_kind, "processing debounced event");

                let decision = pipeline.snapshot_acquire(&event, snapshot_store.as_ref()).await?;

                match decision {
                    crate::pipeline::SnapshotDecision::Unchanged => {
                        metrics.increment(&metrics.events_suppressed_unchanged);
                        tracing::info!(path = %event.canonical_path, "unchanged, skipping");
                        continue;
                    }
                    decision => {
                        let attribution = pipeline.enrich_attribution(&event);

                        let (diff_summary, diff_render) = match &decision {
                            crate::pipeline::SnapshotDecision::Changed { previous_data, current_data, .. } => {
                                let prev_str = String::from_utf8_lossy(previous_data).to_string();
                                let curr_str = String::from_utf8_lossy(current_data).to_string();
                                match pipeline.diff_generate(&prev_str, &curr_str, &event.canonical_path).await {
                                    Ok(config_diff::difftastic::DiffOutput::Changed { render, added, removed }) => {
                                        let file_size_before = previous_data.len() as u64;
                                        let file_size_after = current_data.len() as u64;
                                        let summary = config_diff::summary::build_diff_summary(
                                            added, removed, file_size_before, file_size_after, &render,
                                        );
                                        (Some(summary), Some(render))
                                    }
                                    Ok(config_diff::difftastic::DiffOutput::Unchanged) => (None, None),
                                    Ok(config_diff::difftastic::DiffOutput::Error { message }) => {
                                        tracing::warn!(path = %event.canonical_path, error = %message, "diff generation failed");
                                        (None, None)
                                    }
                                    Err(e) => {
                                        tracing::warn!(path = %event.canonical_path, error = %e, "diff generation error");
                                        (None, None)
                                    }
                                }
                            }
                            crate::pipeline::SnapshotDecision::FileCreated { current_data, .. } => {
                                let curr_str = String::from_utf8_lossy(current_data).to_string();
                                match pipeline.diff_generate("", &curr_str, &event.canonical_path).await {
                                    Ok(config_diff::difftastic::DiffOutput::Changed { render, added, removed }) => {
                                        let file_size_after = current_data.len() as u64;
                                        let summary = config_diff::summary::build_diff_summary(
                                            added, removed, 0, file_size_after, &render,
                                        );
                                        (Some(summary), Some(render))
                                    }
                                    Ok(config_diff::difftastic::DiffOutput::Unchanged) => (None, None),
                                    Ok(config_diff::difftastic::DiffOutput::Error { message }) => {
                                        tracing::warn!(path = %event.canonical_path, error = %message, "diff generation failed for created file");
                                        (None, None)
                                    }
                                    Err(e) => {
                                        tracing::warn!(path = %event.canonical_path, error = %e, "diff generation error for created file");
                                        (None, None)
                                    }
                                }
                            }
                            crate::pipeline::SnapshotDecision::FileDeleted { previous_data, .. } => {
                                let prev_str = String::from_utf8_lossy(previous_data).to_string();
                                match pipeline.diff_generate(&prev_str, "", &event.canonical_path).await {
                                    Ok(config_diff::difftastic::DiffOutput::Changed { render, added, removed }) => {
                                        let file_size_before = previous_data.len() as u64;
                                        let summary = config_diff::summary::build_diff_summary(
                                            added, removed, file_size_before, 0, &render,
                                        );
                                        (Some(summary), Some(render))
                                    }
                                    Ok(config_diff::difftastic::DiffOutput::Unchanged) => (None, None),
                                    Ok(config_diff::difftastic::DiffOutput::Error { message }) => {
                                        tracing::warn!(path = %event.canonical_path, error = %message, "diff generation failed for deleted file");
                                        (None, None)
                                    }
                                    Err(e) => {
                                        tracing::warn!(path = %event.canonical_path, error = %e, "diff generation error for deleted file");
                                        (None, None)
                                    }
                                }
                            }
                            crate::pipeline::SnapshotDecision::Unchanged => (None, None),
                        };

                        let change_event = pipeline.build_change_event(&event, &decision, diff_summary, diff_render, attribution);

                        if let Err(e) = spool.append(&change_event).await {
                            tracing::error!(error = %e, "failed to spool event");
                            continue;
                        }

                        match publisher.publish(&change_event, &change_event.idempotency_key).await {
                            Ok(_) => {
                                metrics.increment(&metrics.events_published);
                                metrics.last_control_plane_contact.store(
                                    std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs(),
                                    std::sync::atomic::Ordering::Relaxed,
                                );
                                if let Err(e) = spool.mark_delivered(&change_event.event_id).await {
                                    tracing::warn!(error = %e, "failed to mark event delivered");
                                }
                            }
                            Err(_) => {
                                metrics.increment(&metrics.events_publish_failed);
                                let attempts = spool.increment_attempts(&change_event.event_id)?;
                                if attempts >= max_publish_retries {
                                    if let Err(e) = spool.mark_failed(&change_event.event_id, "max retries exceeded").await {
                                        tracing::warn!(error = %e, "failed to mark event as permanently failed");
                                    }
                                    tracing::error!(
                                        event_id = %change_event.event_id,
                                        attempts,
                                        "event permanently failed after max retries"
                                    );
                                }
                            }
                        }

                        if let crate::pipeline::SnapshotDecision::Changed { current_hash, current_data, .. }
                        | crate::pipeline::SnapshotDecision::FileCreated { current_hash, current_data } = &decision {
                            if let Err(e) = snapshot_store.write_snapshot(current_hash, current_data).await {
                                tracing::warn!(error = %e, "failed to write snapshot");
                            }
                            if let Err(e) = snapshot_store.set_current_hash(&event.canonical_path, current_hash) {
                                tracing::warn!(error = %e, "failed to update current hash");
                            }
                        }
                    }
                }
            }
            _ = &mut shutdown => {
                tracing::info!("shutdown signal received");
                break;
            }
        }
    }

    heartbeat_handle.abort();
    debounce_handle.abort();
    api_handle.abort();
    retention_handle.abort();
    if let Some(h) = tunnel_handle {
        h.abort();
    }
    tracing::info!("agent stopped");
    Ok(())
}
