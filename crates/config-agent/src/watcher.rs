use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::sync::{Arc, RwLock};
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

/// Resolved watcher backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WatchBackend {
    Inotify,
    Poll,
}

impl std::fmt::Display for WatchBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WatchBackend::Inotify => write!(f, "inotify"),
            WatchBackend::Poll => write!(f, "poll"),
        }
    }
}

/// Mount information for a watch root path.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MountInfo {
    pub path: String,
    pub mount_point: Option<String>,
    pub fs_type: Option<String>,
    pub is_nfs: bool,
}

/// Detect mount information for a path by reading /proc/mounts on Linux.
/// On non-Linux platforms, returns MountInfo with is_nfs=false and no mount details.
pub fn detect_mount_info(path: &str) -> MountInfo {
    #[cfg(target_os = "linux")]
    {
        let mounts = match std::fs::read_to_string("/proc/mounts") {
            Ok(m) => m,
            Err(_) => {
                return MountInfo {
                    path: path.to_string(),
                    mount_point: None,
                    fs_type: None,
                    is_nfs: false,
                }
            }
        };

        let mut best_match: Option<(&str, &str, bool)> = None;
        let mut best_match_len = 0usize;

        for line in mounts.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                continue;
            }
            let mount_point = parts[1];
            let fs_type = parts[2];
            let is_nfs = matches!(fs_type, "nfs" | "nfs4" | "cifs" | "smbfs");

            if path.starts_with(mount_point) && mount_point.len() > best_match_len {
                best_match_len = mount_point.len();
                best_match = Some((mount_point, fs_type, is_nfs));
            }
        }

        match best_match {
            Some((mp, fs, is_nfs)) => MountInfo {
                path: path.to_string(),
                mount_point: Some(mp.to_string()),
                fs_type: Some(fs.to_string()),
                is_nfs,
            },
            None => MountInfo {
                path: path.to_string(),
                mount_point: None,
                fs_type: None,
                is_nfs: false,
            },
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        MountInfo {
            path: path.to_string(),
            mount_point: None,
            fs_type: None,
            is_nfs: false,
        }
    }
}

/// Detect whether a path is on an NFS/CIFS/SMB mount.
fn is_nfs_mount(path: &str) -> bool {
    detect_mount_info(path).is_nfs
}

pub struct FileWatcher {
    config: AgentConfig,
    event_tx: mpsc::Sender<RawWatchEvent>,
    watch_backend: Arc<RwLock<String>>,
}

impl FileWatcher {
    pub fn new(
        config: AgentConfig,
        event_tx: mpsc::Sender<RawWatchEvent>,
        watch_backend: Arc<RwLock<String>>,
    ) -> Self {
        Self {
            config,
            event_tx,
            watch_backend,
        }
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

        // Determine the watcher backend based on config and mount detection.
        let backend = match self.config.watch_mode.as_str() {
            "poll" => {
                tracing::info!("watch_mode=poll — using PollWatcher (forced by config)");
                WatchBackend::Poll
            }
            "inotify" => {
                let any_nfs = roots.iter().any(|r| is_nfs_mount(r));
                if any_nfs {
                    tracing::warn!(
                        "watch_mode=inotify but NFS mount detected — inotify will NOT detect \
                         changes on NFS shares. Consider setting watch_mode=\"poll\" or watch_mode=\"auto\"."
                    );
                }
                WatchBackend::Inotify
            }
            _ => {
                // "auto" — detect from mount info
                let any_nfs = roots.iter().any(|r| is_nfs_mount(r));
                if any_nfs {
                    tracing::info!("NFS mount detected — using PollWatcher for reliable change detection");
                    WatchBackend::Poll
                } else {
                    tracing::info!("Using inotify (RecommendedWatcher) for local filesystem");
                    WatchBackend::Inotify
                }
            }
        };

        // Store resolved backend name so health endpoint can read it.
        {
            let mut guard = self.watch_backend.write().unwrap();
            *guard = backend.to_string();
        }

        // Log per-root mount detection details.
        for root in &roots {
            let mount_info = detect_mount_info(root);
            tracing::info!(
                path = %mount_info.path,
                mount_point = ?mount_info.mount_point,
                fs_type = ?mount_info.fs_type,
                is_nfs = mount_info.is_nfs,
                backend = %backend,
                "watch root mount detection"
            );
        }

        let poll_interval = std::time::Duration::from_secs(self.config.poll_interval_secs);

        tokio::spawn(async move {
            match backend {
                WatchBackend::Poll => {
                    use notify::PollWatcher;
                    use notify::Watcher;
                    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<notify::Event>();

                    let mut watcher = match PollWatcher::new(
                        move |res: Result<notify::Event, notify::Error>| {
                            if let Ok(event) = res {
                                let _ = notify_tx.send(event);
                            }
                        },
                        notify::Config::default()
                            .with_poll_interval(poll_interval)
                            .with_compare_contents(true),
                    ) {
                        Ok(w) => w,
                        Err(e) => {
                            tracing::error!(error = %e, "failed to create poll watcher");
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
                        if let Err(e) = watcher.watch(&canonical, notify::RecursiveMode::Recursive) {
                            tracing::error!(path = %canonical.display(), error = %e, "failed to watch root");
                        } else {
                            tracing::info!(path = %canonical.display(), "watching root (poll mode)");
                        }
                    }

                    while let Some(event) = notify_rx.recv().await {
                        Self::process_event(event, &include_set, &exclude_set, &tx).await;
                    }
                }
                WatchBackend::Inotify => {
                    use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

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
                            tracing::info!(path = %canonical.display(), "watching root (inotify mode)");
                        }
                    }

                    while let Some(event) = notify_rx.recv().await {
                        Self::process_event(event, &include_set, &exclude_set, &tx).await;
                    }
                }
            }
        });

        Ok(())
    }

    async fn process_event(
        event: notify::Event,
        include_set: &GlobSet,
        exclude_set: &GlobSet,
        tx: &mpsc::Sender<RawWatchEvent>,
    ) {
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
                notify::EventKind::Create(_) => RawEventKind::Created,
                notify::EventKind::Modify(_) => RawEventKind::Modified,
                notify::EventKind::Remove(_) => RawEventKind::Deleted,
                _ => RawEventKind::Other,
            };

            let watch_event = RawWatchEvent {
                raw_path: path_buf,
                event_kind,
                observed_at: Utc::now(),
            };

            tracing::debug!(path = %watch_event.raw_path, kind = ?watch_event.event_kind, "file change detected");
            if tx.send(watch_event).await.is_err() {
                break;
            }
        }
    }
}