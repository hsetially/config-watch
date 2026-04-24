use std::fs;

use anyhow::Context;
use camino::Utf8Path;

use crate::store::SnapshotStore;

#[derive(Debug, Clone)]
pub struct RetentionConfig {
    pub max_snapshots_per_file: usize,
    pub max_total_bytes: u64,
    pub max_age_days: u32,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            max_snapshots_per_file: 10,
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
    let mut total_size: u64 = 0;
    let mut entries: Vec<(String, u64)> = Vec::new();

    for entry in walk_snapshot_files(base_dir)? {
        let meta = fs::metadata(&entry).with_context(|| format!("stat {}", entry))?;
        total_size += meta.len();
        entries.push((entry, meta.len()));
    }

    if total_size <= config.max_total_bytes {
        return Ok(RetentionStats {
            snapshots_removed: 0,
            bytes_freed: 0,
        });
    }

    entries.sort_by_key(|(_, size)| std::cmp::Reverse(*size));

    let mut removed = 0;
    let mut freed: u64 = 0;
    for (path, size) in &entries {
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

    #[test]
    fn default_config_values() {
        let cfg = RetentionConfig::default();
        assert_eq!(cfg.max_snapshots_per_file, 10);
        assert_eq!(cfg.max_age_days, 90);
    }
}
