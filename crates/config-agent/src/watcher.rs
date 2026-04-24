use chrono::{DateTime, Utc};
use camino::Utf8PathBuf;
use globset::{Glob, GlobSet, GlobSetBuilder};
use tokio::sync::mpsc;

use crate::config::AgentConfig;

fn build_glob_set(patterns: &[String]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        match Glob::new(pattern) {
            Ok(g) => {
                builder.add(g);
            }
            Err(e) => {
                tracing::warn!(pattern = %pattern, error = %e, "invalid glob pattern, skipping");
            }
        }
    }
    builder.build().unwrap_or_else(|_| GlobSet::empty())
}

#[derive(Debug, Clone)]
pub enum RawEventKind {
    Created,
    Modified,
    Deleted,
    Other,
}

#[derive(Debug, Clone)]
pub struct RawWatchEvent {
    pub raw_path: Utf8PathBuf,
    pub event_kind: RawEventKind,
    pub observed_at: DateTime<Utc>,
}

pub struct FileWatcher {
    config: AgentConfig,
    event_tx: mpsc::Sender<RawWatchEvent>,
}

impl FileWatcher {
    pub fn new(config: AgentConfig, event_tx: mpsc::Sender<RawWatchEvent>) -> Self {
        Self { config, event_tx }
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        let roots: Vec<String> = self
            .config
            .watch_roots
            .iter()
            .map(|r| r.root_path.to_string())
            .collect();

        tracing::info!(?roots, "file watcher starting");

        let tx = self.event_tx.clone();
        let include_set = build_glob_set(&self.config.include_globs);
        let exclude_set = build_glob_set(&self.config.exclude_globs);

        tokio::spawn(async move {
            use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

            let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<Event>();

            let mut watcher = match RecommendedWatcher::new(
                move |res: Result<Event, notify::Error>| {
                    if let Ok(event) = res {
                        let _ = notify_tx.send(event);
                    }
                },
                notify::Config::default(),
            ) {
                Ok(w) => w,
                Err(e) => {
                    tracing::error!(error = %e, "failed to create file watcher");
                    return;
                }
            };

            for root in &roots {
                let raw_path = std::path::Path::new(root);
                if !raw_path.exists() {
                    tracing::error!(path = %root, "watch root does not exist, cannot watch");
                    continue;
                }
                let canonical = match std::fs::canonicalize(raw_path) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!(path = %root, error = %e, "failed to canonicalize watch root");
                        continue;
                    }
                };
                if let Err(e) = watcher.watch(&canonical, RecursiveMode::Recursive) {
                    tracing::error!(path = %canonical.display(), error = %e, "failed to watch root");
                } else {
                    tracing::info!(path = %canonical.display(), "watching root");
                }
            }

            while let Some(event) = notify_rx.recv().await {
                tracing::debug!(kind = ?event.kind, paths = ?event.paths, "raw notify event");
                for path in event.paths {
                    let path_str = match path.to_str() {
                        Some(s) => s.to_string(),
                        None => continue,
                    };

                    let normalized = path_str
                        .strip_prefix(r"\\?\")
                        .unwrap_or(&path_str)
                        .replace('\\', "/");
                    let path_buf = Utf8PathBuf::from(normalized);

                    if !include_set.is_empty() && !include_set.is_match(path_buf.as_std_path()) {
                        continue;
                    }
                    if exclude_set.is_match(path_buf.as_std_path()) {
                        tracing::debug!(path = %path_buf, "excluded by glob");
                        continue;
                    }
                    let event_kind = match event.kind {
                        EventKind::Create(_) => RawEventKind::Created,
                        EventKind::Modify(_) => RawEventKind::Modified,
                        EventKind::Remove(_) => RawEventKind::Deleted,
                        _ => RawEventKind::Other,
                    };

                    let watch_event = RawWatchEvent {
                        raw_path: path_buf,
                        event_kind,
                        observed_at: Utc::now(),
                    };

                    tracing::info!(path = %watch_event.raw_path, kind = ?watch_event.event_kind, "file change detected");
                    if tx.send(watch_event).await.is_err() {
                        break;
                    }
                }
            }
        });

        Ok(())
    }
}