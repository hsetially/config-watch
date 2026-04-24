use std::sync::Arc;

use config_snapshot::store::SnapshotStore;

pub trait ContentResolver: Send + Sync {
    fn resolve(&self, path: &str, content_hash: Option<&str>) -> anyhow::Result<Option<Vec<u8>>>;
}

pub struct SnapshotContentResolver {
    store: Arc<SnapshotStore>,
}

impl SnapshotContentResolver {
    pub fn new(store: Arc<SnapshotStore>) -> Self {
        Self { store }
    }
}

impl ContentResolver for SnapshotContentResolver {
    fn resolve(&self, _path: &str, content_hash: Option<&str>) -> anyhow::Result<Option<Vec<u8>>> {
        let hash = match content_hash {
            Some(h) => h,
            None => return Ok(None),
        };

        let hash_prefix = &hash[..2.min(hash.len())];
        let path = self.store.base_dir().join(hash_prefix).join(hash);

        match std::fs::read(&path) {
            Ok(data) => Ok(Some(data)),
            Err(e) => {
                tracing::warn!(content_hash = hash, path = %path, error = %e, "snapshot content not found");
                Ok(None)
            }
        }
    }
}

/// A resolver that always returns None — used when no snapshot store is available.
pub struct NullContentResolver;

impl ContentResolver for NullContentResolver {
    fn resolve(&self, _path: &str, _content_hash: Option<&str>) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(None)
    }
}