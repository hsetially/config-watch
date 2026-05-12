use anyhow::Result;
use base64::Engine;

use config_shared::events::{ChangeEvent, ChangeKind, Severity};
use config_shared::ids::{EventId, HostId, SnapshotId};
use config_shared::snapshots::{DiffSummary, YamlLintSeverity};
use config_shared::validation::derive_idempotency_key;

use crate::attribution::AttributionResolver;
use crate::config::AgentConfig;
use crate::debounce::DebouncedEvent;

fn build_glob_set(patterns: &[String]) -> globset::GlobSet {
    use globset::{Glob, GlobSetBuilder};
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        match Glob::new(pattern) {
            Ok(g) => {
                builder.add(g);
            }
            Err(e) => {
                tracing::warn!(pattern = %pattern, error = %e, "baseline: invalid glob, skipping");
            }
        }
    }
    builder.build().unwrap_or_else(|_| globset::GlobSet::empty())
}

/// Scan all watch roots and snapshot existing files that don't yet have a baseline.
/// This ensures the first real modification has a proper previous version to diff against.
/// Does not publish any events — only populates the snapshot store.
pub async fn baseline_scan(
    config: &AgentConfig,
    snapshot_store: &config_snapshot::store::SnapshotStore,
) -> Result<BaselineStats> {
    let mut stats = BaselineStats::default();
    let include_set = build_glob_set(&config.include_globs);
    let exclude_set = build_glob_set(&config.exclude_globs);

    for root in &config.watch_roots {
        if !root.root_path.exists() {
            continue;
        }
        let entries = tokio::task::spawn_blocking({
            let root = root.root_path.clone();
            move || -> Vec<std::path::PathBuf> {
                walkdir::WalkDir::new(root.as_std_path())
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_file())
                    .map(|e| e.into_path())
                    .collect()
            }
        })
        .await?;

        stats.files_scanned += entries.len();

        for entry in entries {
            let path_str = match entry.to_str() {
                Some(s) => s,
                None => continue,
            };
            let normalized = path_str
                .strip_prefix(r"\\?\")
                .unwrap_or(path_str)
                .replace('\\', "/");
            let path = camino::Utf8PathBuf::from(normalized);

            if !config_shared::paths::is_yaml_file(&path) {
                continue;
            }
            if !include_set.is_empty() && !include_set.is_match(path.as_std_path()) {
                continue;
            }
            if exclude_set.is_match(path.as_std_path()) {
                continue;
            }

            // Check if we already have a state entry for this file
            if let Some(existing_hash) = snapshot_store.get_current_hash(&path) {
                // Verify the content file actually exists on disk
                if snapshot_store.content_exists(&existing_hash) {
                    stats.skipped_existing += 1;
                    continue;
                }
                // Hash in state but content file missing — re-create baseline
                tracing::info!(path = %path, hash = %existing_hash, "baseline: state entry exists but content file missing, re-creating");
            }

            let content = match tokio::fs::read(&path).await {
                Ok(c) => c,
                Err(_) => continue,
            };
            // H11: cap file reads at 5 MiB to prevent YAML alias amplification or oversize reads
            if content.len() > 5 * 1024 * 1024 {
                tracing::warn!(path = %path, size = content.len(), "baseline: skipping file exceeding 5 MiB cap");
                continue;
            }
            let hash = config_snapshot::hash::compute_blake3(&content);

            if let Err(e) = snapshot_store.write_snapshot(&hash, &content).await {
                tracing::warn!(path = %path, error = %e, "baseline: failed to write snapshot");
                continue;
            }
            if let Err(e) = snapshot_store.set_current_hash(&path, &hash) {
                tracing::warn!(path = %path, error = %e, "baseline: failed to set hash");
                continue;
            }

            stats.baselines_created += 1;
        }
    }

    tracing::info!(
        scanned = stats.files_scanned,
        baselines = stats.baselines_created,
        skipped = stats.skipped_existing,
        "baseline scan complete"
    );

    Ok(stats)
}

#[derive(Debug, Default)]
pub struct BaselineStats {
    pub files_scanned: usize,
    pub baselines_created: usize,
    pub skipped_existing: usize,
}

pub enum SnapshotDecision {
    Unchanged,
    Changed {
        previous_hash: String,
        current_hash: String,
        previous_data: Vec<u8>,
        current_data: Vec<u8>,
    },
    FileCreated {
        current_hash: String,
        current_data: Vec<u8>,
    },
    FileDeleted {
        previous_hash: String,
        previous_data: Vec<u8>,
    },
}

pub struct Pipeline {
    config: AgentConfig,
    host_id: HostId,
    diff_engine: config_diff::difftastic::DiffEngine,
}

impl Pipeline {
    pub fn new(config: AgentConfig, host_id: HostId) -> Self {
        let diff_engine = config_diff::difftastic::DiffEngine::with_config(config.diff.clone());
        Self {
            config,
            host_id,
            diff_engine,
        }
    }

    pub fn canonicalize_and_filter(
        &self,
        event: &crate::watcher::RawWatchEvent,
    ) -> Option<camino::Utf8PathBuf> {
        if !config_shared::paths::is_yaml_file(&event.raw_path) {
            return None;
        }

        let excluded = self.config.exclude_globs.iter().any(|pattern| {
            event
                .raw_path
                .as_str()
                .contains(&pattern.replace("**/", "").replace("*", ""))
        });
        if excluded {
            return None;
        }

        Some(event.raw_path.clone())
    }

    pub async fn snapshot_acquire(
        &self,
        event: &DebouncedEvent,
        snapshot_store: &config_snapshot::store::SnapshotStore,
    ) -> Result<SnapshotDecision> {
        let path = &event.canonical_path;

        match event.event_kind {
            ChangeKind::Deleted => {
                let prev_hash = snapshot_store.get_current_hash(path).unwrap_or_default();
                if prev_hash.is_empty() {
                    return Ok(SnapshotDecision::Unchanged);
                }
                let previous_data = snapshot_store
                    .read_content(&prev_hash)
                    .await
                    .unwrap_or_default();
                Ok(SnapshotDecision::FileDeleted {
                    previous_hash: prev_hash,
                    previous_data,
                })
            }
            ChangeKind::Created
            | ChangeKind::Modified
            | ChangeKind::MetadataOnly
            | ChangeKind::PermissionChanged
            | ChangeKind::InitialSnapshot => {
                if !path.exists() {
                    return Ok(SnapshotDecision::Unchanged);
                }

                let content = tokio::fs::read(path).await?;
                // H11: cap file reads at 5 MiB
                if content.len() > 5 * 1024 * 1024 {
                    anyhow::bail!("file exceeds 5 MiB size cap: {} ({} bytes)", path, content.len());
                }
                let current_hash = config_snapshot::hash::compute_blake3(&content);

                let prev_hash = snapshot_store.get_current_hash(path).unwrap_or_default();

                if prev_hash == current_hash {
                    return Ok(SnapshotDecision::Unchanged);
                }

                if prev_hash.is_empty() {
                    return Ok(SnapshotDecision::FileCreated {
                        current_hash,
                        current_data: content,
                    });
                }

                let previous_data = match snapshot_store.read_content(&prev_hash).await {
                    Ok(data) => data,
                    Err(e) => {
                        tracing::warn!(
                            path = %path,
                            hash = %prev_hash,
                            error = %e,
                            "previous snapshot missing, treating as file creation"
                        );
                        return Ok(SnapshotDecision::FileCreated {
                            current_hash,
                            current_data: content,
                        });
                    }
                };

                Ok(SnapshotDecision::Changed {
                    previous_hash: prev_hash,
                    current_hash,
                    previous_data,
                    current_data: content,
                })
            }
        }
    }

    pub async fn diff_generate(
        &self,
        previous: &str,
        current: &str,
        path: &camino::Utf8Path,
    ) -> Result<config_diff::difftastic::DiffOutput> {
        self.diff_engine.compute_diff(previous, current, path).await
    }

    pub fn enrich_attribution(
        &self,
        event: &DebouncedEvent,
    ) -> config_shared::attribution::Attribution {
        let resolver = AttributionResolver::new(&self.config);
        resolver.resolve(&event.canonical_path, &event.event_kind)
    }

    pub fn build_change_event(
        &self,
        event: &DebouncedEvent,
        decision: &SnapshotDecision,
        diff_summary: Option<DiffSummary>,
        diff_render: Option<String>,
        attribution: config_shared::attribution::Attribution,
    ) -> ChangeEvent {
        let (prev_hash, curr_hash, prev_snap, curr_snap, current_data) = match decision {
            SnapshotDecision::Unchanged => (String::new(), String::new(), None, None, None),
            SnapshotDecision::Changed {
                previous_hash,
                current_hash,
                current_data,
                ..
            } => (
                previous_hash.clone(),
                current_hash.clone(),
                Some(SnapshotId::new()),
                Some(SnapshotId::new()),
                Some(current_data),
            ),
            SnapshotDecision::FileCreated {
                current_hash,
                current_data,
            } => (
                String::new(),
                current_hash.clone(),
                None,
                Some(SnapshotId::new()),
                Some(current_data),
            ),
            SnapshotDecision::FileDeleted { previous_hash, .. } => (
                previous_hash.clone(),
                String::new(),
                Some(SnapshotId::new()),
                None,
                None,
            ),
        };

        // Run YAML linter on current file data for Changed/FileCreated
        let lint_findings = current_data
            .as_ref()
            .map(|data| crate::yaml_lint::lint_yaml(data))
            .unwrap_or_default();

        let has_critical_lint = lint_findings
            .iter()
            .any(|f| f.severity == YamlLintSeverity::Critical);

        // Merge lint findings into diff_summary
        let diff_summary = if lint_findings.is_empty() {
            diff_summary
        } else if let Some(mut ds) = diff_summary {
            ds.yaml_lint_findings = lint_findings;
            Some(ds)
        } else {
            // FileCreated with no diff yet — create synthetic DiffSummary for lint findings
            let file_size_after = current_data.map(|d| d.len() as u64).unwrap_or(0);
            Some(DiffSummary {
                changed_line_estimate: 0,
                file_size_before: 0,
                file_size_after,
                comment_only_hint: false,
                syntax_equivalent_hint: false,
                yaml_lint_findings: lint_findings,
            })
        };

        let severity = if has_critical_lint {
            Severity::Critical
        } else {
            Severity::Info
        };

        let idempotency_key = derive_idempotency_key(
            &self.host_id,
            &event.canonical_path,
            &prev_hash,
            &curr_hash,
            event.observed_at,
        );

        let content_b64 = current_data.as_ref().map(|data| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(data)
        });

        ChangeEvent {
            event_id: EventId::new(),
            idempotency_key,
            host_id: self.host_id,
            canonical_path: event.canonical_path.clone(),
            event_time: event.observed_at,
            event_kind: event.event_kind.clone(),
            previous_snapshot_id: prev_snap,
            current_snapshot_id: curr_snap,
            diff_summary,
            diff_render,
            attribution,
            severity,
            content_b64,
        }
    }
}

/// Publish initial snapshots for all tracked files to the control plane.
/// Called after baseline scan and registration so the control plane has the
/// full current state of each file, not just future deltas.
pub async fn publish_initial_snapshots(
    publisher: &crate::publish::EventPublisher,
    snapshot_store: &config_snapshot::store::SnapshotStore,
    host_id: HostId,
) -> Result<usize> {
    let tracked = snapshot_store.list_tracked_files();
    let mut published = 0usize;

    for (canonical_path, current_hash) in &tracked {
        let content = match snapshot_store.read_content(current_hash).await {
            Ok(data) => data,
            Err(e) => {
                tracing::warn!(path = %canonical_path, error = %e, "initial snapshot: failed to read content, skipping");
                continue;
            }
        };

        let content_b64 = base64::engine::general_purpose::STANDARD.encode(&content);
        let file_size_after = content.len() as u64;

        let diff_summary = config_shared::snapshots::DiffSummary {
            changed_line_estimate: 0,
            file_size_before: 0,
            file_size_after,
            comment_only_hint: false,
            syntax_equivalent_hint: false,
            yaml_lint_findings: vec![],
        };

        let idempotency_key = config_shared::validation::derive_idempotency_key(
            &host_id,
            canonical_path,
            "",
            current_hash,
            chrono::Utc::now(),
        );

        let event = ChangeEvent {
            event_id: EventId::new(),
            idempotency_key,
            host_id,
            canonical_path: canonical_path.clone(),
            event_time: chrono::Utc::now(),
            event_kind: ChangeKind::InitialSnapshot,
            previous_snapshot_id: None,
            current_snapshot_id: Some(SnapshotId::new()),
            diff_summary: Some(diff_summary),
            diff_render: None,
            attribution: config_shared::attribution::Attribution::unknown(),
            severity: Severity::Info,
            content_b64: Some(content_b64),
        };

        match publisher.publish(&event, &event.idempotency_key).await {
            Ok(_) => {
                published += 1;
            }
            Err(e) => {
                tracing::warn!(path = %canonical_path, error = %e, "initial snapshot: publish failed");
            }
        }

        // Rate-limit to avoid overwhelming the control plane
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    tracing::info!(
        published = published,
        total = tracked.len(),
        "initial snapshots published"
    );

    Ok(published)
}
