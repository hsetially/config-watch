use std::collections::HashMap;

use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use config_shared::ids::SnapshotId;
use config_shared::snapshots::{CompressionKind, SnapshotRef};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CurrentStateEntry {
    content_hash: String,
    snapshot_id: uuid::Uuid,
    last_seen_at: String,
}

type CurrentStateMap = HashMap<String, CurrentStateEntry>;

pub struct SnapshotStore {
    base_dir: Utf8PathBuf,
    state: std::sync::RwLock<CurrentStateMap>,
}

impl SnapshotStore {
    pub fn base_dir(&self) -> &Utf8Path {
        &self.base_dir
    }

    pub fn new(base_dir: &Utf8Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(base_dir)
            .with_context(|| format!("failed to create snapshot dir: {}", base_dir))?;

        let state = Self::load_state(base_dir)?;
        Ok(Self {
            base_dir: base_dir.to_path_buf(),
            state: std::sync::RwLock::new(state),
        })
    }

    pub async fn read_content(&self, content_hash: &str) -> anyhow::Result<Vec<u8>> {
        let hash_prefix = &content_hash[..2.min(content_hash.len())];
        let path = self.base_dir.join(hash_prefix).join(content_hash);
        let data = tokio::fs::read(&path)
            .await
            .with_context(|| format!("failed to read snapshot: {}", path))?;
        Ok(data)
    }

    pub async fn write_snapshot(
        &self,
        content_hash: &str,
        data: &[u8],
    ) -> anyhow::Result<SnapshotRef> {
        let snapshot_id = SnapshotId::new();
        let hash_prefix = &content_hash[..2.min(content_hash.len())];
        let dir = self.base_dir.join(hash_prefix);
        tokio::fs::create_dir_all(&dir).await?;

        let storage_path = dir.join(content_hash);
        tokio::fs::write(&storage_path, data).await?;

        let snapshot_ref = SnapshotRef {
            snapshot_id,
            content_hash: content_hash.to_string(),
            size_bytes: data.len() as u64,
            compression: CompressionKind::None,
        };

        self.persist_state_entry(content_hash, &snapshot_id)?;

        Ok(snapshot_ref)
    }

    pub fn get_current_hash(&self, path: &Utf8Path) -> Option<String> {
        let state = self.state.read().unwrap();
        state.get(path.as_str()).map(|e| e.content_hash.clone())
    }

    pub fn set_current_hash(&self, path: &Utf8Path, hash: &str) -> anyhow::Result<()> {
        {
            let mut state = self.state.write().unwrap();
            state.insert(
                path.to_string(),
                CurrentStateEntry {
                    content_hash: hash.to_string(),
                    snapshot_id: uuid::Uuid::nil(),
                    last_seen_at: Utc::now().to_rfc3339(),
                },
            );
        }
        self.save_state()
    }

    pub fn get_last_snapshot_id(&self, path: &Utf8Path) -> Option<SnapshotId> {
        let state = self.state.read().unwrap();
        state
            .get(path.as_str())
            .map(|e| SnapshotId::from(e.snapshot_id))
    }

    fn load_state(base_dir: &Utf8Path) -> anyhow::Result<CurrentStateMap> {
        let state_path = base_dir.join("current_state.json");
        if !state_path.exists() {
            return Ok(HashMap::new());
        }
        let content = std::fs::read_to_string(&state_path)
            .with_context(|| format!("failed to read state file: {}", state_path))?;
        let state: CurrentStateMap = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse state file: {}", state_path))?;
        Ok(state)
    }

    fn save_state(&self) -> anyhow::Result<()> {
        let state_path = self.base_dir.join("current_state.json");
        let state = self.state.read().unwrap();
        let content = serde_json::to_string_pretty(&*state)?;
        std::fs::write(&state_path, content)
            .with_context(|| format!("failed to write state file: {}", state_path))?;
        Ok(())
    }

    fn persist_state_entry(&self, _hash: &str, _id: &SnapshotId) -> anyhow::Result<()> {
        self.save_state()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::compute_blake3;

    #[test]
    fn new_creates_directory() {
        let dir = tempfile::tempdir().unwrap();
        let base = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let _store = SnapshotStore::new(&base).unwrap();
        assert!(base.exists());
    }

    #[test]
    fn set_and_get_current_hash() {
        let dir = tempfile::tempdir().unwrap();
        let base = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let store = SnapshotStore::new(&base).unwrap();
        let path = Utf8Path::new("/etc/myapp/config.yaml");
        store.set_current_hash(path, "abc123").unwrap();
        assert_eq!(store.get_current_hash(path), Some("abc123".to_string()));
    }

    #[test]
    fn get_current_hash_missing() {
        let dir = tempfile::tempdir().unwrap();
        let base = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let store = SnapshotStore::new(&base).unwrap();
        assert!(store
            .get_current_hash(Utf8Path::new("/nonexistent.yaml"))
            .is_none());
    }

    #[tokio::test]
    async fn write_and_read_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let base = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let store = SnapshotStore::new(&base).unwrap();

        let data = b"key: value";
        let hash = compute_blake3(data);
        let snap_ref = store.write_snapshot(&hash, data).await.unwrap();

        let read_data = store.read_content(&snap_ref.content_hash).await.unwrap();
        assert_eq!(read_data, data);
    }

    #[test]
    fn state_persists_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let base = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();

        let path = Utf8Path::new("/etc/myapp/config.yaml");
        {
            let store = SnapshotStore::new(&base).unwrap();
            store.set_current_hash(path, "abc123").unwrap();
        }

        let store2 = SnapshotStore::new(&base).unwrap();
        assert_eq!(store2.get_current_hash(path), Some("abc123".to_string()));
    }
}
