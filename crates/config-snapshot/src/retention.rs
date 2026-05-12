use std::fs;
use std::time::SystemTime;

use anyhow::Context;
use camino::Utf8Path;

use crate::store::SnapshotStore;

/// Snapshot retention policy. Two independent caps run on every sweep:
///
/// 1. **Age cap** (`max_age_days`): any snapshot file older than N days by mtime
///    is deleted. This is the primary cap operators reason about — it
///    determines how far back the dashboard's lazy-diff endpoint can render.
/// 2. **Size cap** (`max_total_bytes`): a safety net for runaway growth. If the
///    age sweep didn't bring total size under the cap, the largest remaining
///    snapshots are deleted until it does.
///
/// `max_snapshots_per_file` was removed — it overlapped with the byte cap and
/// was never actually enforced, so leaving it in the API would be misleading.
#[derive(Debug, Clone)]
pub struct RetentionConfig {
    pub max_total_bytes: u64,
    pub max_age_days: u32,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            max_total_bytes: 1024 * 1024 * 1024,
            max_age_days: 90,
        }
    }
}

#[derive(Debug)]
pub struct RetentionStats {
    pub snapshots_removed: usize,
    pub bytes_freed: u64,
}

pub async fn enforce_retention(
    store: &SnapshotStore,
    config: &RetentionConfig,
) -> anyhow::Result<RetentionStats> {
    let base_dir = store_base_dir(store);
    let mut entries: Vec<(String, u64, SystemTime)> = Vec::new();

    for entry in walk_snapshot_files(base_dir)? {
        let meta = fs::metadata(&entry).with_context(|| format!("stat {}", entry))?;
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        entries.push((entry, meta.len(), mtime));
    }

    let mut removed: usize = 0;
    let mut freed: u64 = 0;

    // Pass 1: age-based deletion. Skipped when max_age_days == 0 (treated as
    // "no age cap") so callers can opt out of age-based eviction explicitly.
    if config.max_age_days > 0 {
        let cutoff = SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(
                config.max_age_days as u64 * 24 * 3600,
            ))
            .unwrap_or(SystemTime::UNIX_EPOCH);
        entries.retain(|(path, size, mtime)| {
            if *mtime < cutoff {
                if let Err(e) = fs::remove_file(path) {
                    tracing::warn!(path = %path, error = %e, "failed to remove aged snapshot");
                    true // keep in list so the size pass can still try
                } else {
                    removed += 1;
                    freed += size;
                    false
                }
            } else {
                true
            }
        });
    }

    // Pass 2: size cap. Largest remaining files first.
    let mut total_size: u64 = entries.iter().map(|(_, s, _)| *s).sum();
    if total_size > config.max_total_bytes {
        entries.sort_by_key(|(_, size, _)| std::cmp::Reverse(*size));
        for (path, size, _) in &entries {
            if total_size <= config.max_total_bytes {
                break;
            }
            if let Err(e) = fs::remove_file(path) {
                tracing::warn!(path = %path, error = %e, "failed to remove snapshot during retention");
            } else {
                total_size -= size;
                freed += size;
                removed += 1;
            }
        }
    }

    Ok(RetentionStats {
        snapshots_removed: removed,
        bytes_freed: freed,
    })
}

fn store_base_dir(store: &SnapshotStore) -> &Utf8Path {
    store.base_dir()
}

fn walk_snapshot_files(base: &Utf8Path) -> anyhow::Result<Vec<String>> {
    let mut files = Vec::new();
    if !base.exists() {
        return Ok(files);
    }
    for entry in walkdir::WalkDir::new(base.as_std_path()) {
        let entry = entry.with_context(|| "walking snapshot directory")?;
        if entry.file_type().is_file() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name != "current_state.json" {
                files.push(entry.path().to_string_lossy().to_string());
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;

    #[test]
    fn default_config_values() {
        let cfg = RetentionConfig::default();
        assert_eq!(cfg.max_age_days, 90);
        assert_eq!(cfg.max_total_bytes, 1024 * 1024 * 1024);
    }

    #[tokio::test]
    async fn age_cap_deletes_old_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let base = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let store = SnapshotStore::new(&base).unwrap();

        // Old snapshot — backdate its mtime to 100 days ago.
        let old_data = b"old: snapshot";
        let old_hash = crate::hash::compute_blake3(old_data);
        store.write_snapshot(&old_hash, old_data).await.unwrap();
        let old_path = base.join(&old_hash[..2]).join(&old_hash);
        let one_hundred_days_ago = SystemTime::now()
            - std::time::Duration::from_secs(100 * 24 * 3600);
        filetime::set_file_mtime(
            old_path.as_std_path(),
            filetime::FileTime::from_system_time(one_hundred_days_ago),
        )
        .unwrap();

        // Fresh snapshot — should survive.
        let new_data = b"new: snapshot";
        let new_hash = crate::hash::compute_blake3(new_data);
        store.write_snapshot(&new_hash, new_data).await.unwrap();

        let cfg = RetentionConfig {
            max_total_bytes: u64::MAX, // disable size cap to isolate the age path
            max_age_days: 30,
        };
        let stats = enforce_retention(&store, &cfg).await.unwrap();
        assert_eq!(stats.snapshots_removed, 1);
        assert!(!store.content_exists(&old_hash));
        assert!(store.content_exists(&new_hash));
    }

    #[tokio::test]
    async fn size_cap_runs_after_age_cap() {
        // Both files are fresh, so age does nothing; the size cap then evicts
        // the larger one until total <= cap.
        let dir = tempfile::tempdir().unwrap();
        let base = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let store = SnapshotStore::new(&base).unwrap();

        let big = vec![b'x'; 4096];
        let small = vec![b'y'; 100];
        let big_hash = crate::hash::compute_blake3(&big);
        let small_hash = crate::hash::compute_blake3(&small);
        store.write_snapshot(&big_hash, &big).await.unwrap();
        store.write_snapshot(&small_hash, &small).await.unwrap();

        let cfg = RetentionConfig {
            max_total_bytes: 200, // forces the big one out
            max_age_days: 0,      // age pass disabled
        };
        let stats = enforce_retention(&store, &cfg).await.unwrap();
        assert_eq!(stats.snapshots_removed, 1);
        assert!(!store.content_exists(&big_hash));
        assert!(store.content_exists(&small_hash));
    }

    #[tokio::test]
    async fn no_op_when_under_caps() {
        let dir = tempfile::tempdir().unwrap();
        let base = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let store = SnapshotStore::new(&base).unwrap();
        let data = b"k: v";
        let hash = crate::hash::compute_blake3(data);
        store.write_snapshot(&hash, data).await.unwrap();

        let stats = enforce_retention(&store, &RetentionConfig::default())
            .await
            .unwrap();
        assert_eq!(stats.snapshots_removed, 0);
        assert!(store.content_exists(&hash));
    }
}
