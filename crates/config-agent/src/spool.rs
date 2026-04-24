use std::collections::HashMap;

use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use config_shared::events::ChangeEvent;
use config_shared::ids::EventId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryStatus {
    Pending,
    Delivered,
    FailedPermanently { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpoolEntry {
    pub event: ChangeEvent,
    pub status: DeliveryStatus,
    pub created_at: DateTime<Utc>,
    pub attempts: u32,
    pub last_attempt_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct SpoolIndex {
    entries: HashMap<String, DeliveryStatus>,
}

pub struct SpoolWriter {
    spool_dir: Utf8PathBuf,
    max_events: usize,
    max_bytes: u64,
}

impl SpoolWriter {
    pub fn new(spool_dir: &Utf8PathBuf, max_events: usize, max_bytes: u64) -> Result<Self> {
        let pending_dir = spool_dir.join("pending");
        std::fs::create_dir_all(spool_dir)
            .with_context(|| format!("failed to create spool dir: {}", spool_dir))?;
        std::fs::create_dir_all(&pending_dir)
            .with_context(|| format!("failed to create pending dir: {}", pending_dir))?;
        Ok(Self {
            spool_dir: spool_dir.clone(),
            max_events,
            max_bytes,
        })
    }

    pub async fn append(&self, event: &ChangeEvent) -> Result<SpoolEntry> {
        self.enforce_limits()?;

        let entry = SpoolEntry {
            event: event.clone(),
            status: DeliveryStatus::Pending,
            created_at: Utc::now(),
            attempts: 0,
            last_attempt_at: None,
        };

        let file_path = self.event_path(&event.event_id);
        let json =
            serde_json::to_string(&entry).with_context(|| "failed to serialize spool entry")?;
        tokio::fs::write(&file_path, json)
            .await
            .with_context(|| format!("failed to write spool entry: {}", file_path))?;

        self.update_index(&event.event_id, &DeliveryStatus::Pending)?;

        Ok(entry)
    }

    pub async fn mark_delivered(&self, event_id: &EventId) -> Result<()> {
        self.update_status(event_id, DeliveryStatus::Delivered)?;
        let path = self.event_path(event_id);
        if path.exists() {
            let data = tokio::fs::read_to_string(&path).await?;
            if let Ok(mut entry) = serde_json::from_str::<SpoolEntry>(&data) {
                entry.status = DeliveryStatus::Delivered;
                let json = serde_json::to_string(&entry)?;
                tokio::fs::write(&path, json).await?;
            }
        }
        Ok(())
    }

    pub async fn mark_failed(&self, event_id: &EventId, reason: &str) -> Result<()> {
        let status = DeliveryStatus::FailedPermanently {
            reason: reason.to_string(),
        };
        self.update_status(event_id, status.clone())?;
        let path = self.event_path(event_id);
        if path.exists() {
            let data = tokio::fs::read_to_string(&path).await?;
            if let Ok(mut entry) = serde_json::from_str::<SpoolEntry>(&data) {
                entry.status = status;
                let json = serde_json::to_string(&entry)?;
                tokio::fs::write(&path, json).await?;
            }
        }
        Ok(())
    }

    pub async fn pending_entries(&self) -> Result<Vec<SpoolEntry>> {
        let pending_dir = self.spool_dir.join("pending");
        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&pending_dir)
            .await
            .with_context(|| format!("failed to read pending dir: {}", pending_dir))?;

        while let Some(entry) = read_dir.next_entry().await? {
            let path = entry.path();
            if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                if let Ok(data) = tokio::fs::read_to_string(&path).await {
                    if let Ok(spool_entry) = serde_json::from_str::<SpoolEntry>(&data) {
                        if spool_entry.status == DeliveryStatus::Pending {
                            entries.push(spool_entry);
                        }
                    }
                }
            }
        }

        entries.sort_by_key(|e| e.created_at);
        Ok(entries)
    }

    fn event_path(&self, event_id: &EventId) -> Utf8PathBuf {
        self.spool_dir
            .join("pending")
            .join(format!("{}.jsonl", event_id))
    }

    fn index_path(&self) -> Utf8PathBuf {
        self.spool_dir.join("index.json")
    }

    fn update_index(&self, event_id: &EventId, status: &DeliveryStatus) -> Result<()> {
        let mut index = self.load_index();
        index.entries.insert(event_id.to_string(), status.clone());
        self.save_index(&index)
    }

    fn update_status(&self, event_id: &EventId, status: DeliveryStatus) -> Result<()> {
        let mut index = self.load_index();
        index.entries.insert(event_id.to_string(), status);
        self.save_index(&index)
    }

    fn load_index(&self) -> SpoolIndex {
        let path = self.index_path();
        if !path.exists() {
            return SpoolIndex::default();
        }
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    }

    fn save_index(&self, index: &SpoolIndex) -> Result<()> {
        let json = serde_json::to_string(index)?;
        std::fs::write(self.index_path(), json)?;
        Ok(())
    }

    pub fn pending_count(&self) -> usize {
        let pending_dir = self.spool_dir.join("pending");
        std::fs::read_dir(&pending_dir)
            .map(|rd| rd.filter_map(|e| e.ok()).count())
            .unwrap_or(0)
    }

    pub fn increment_attempts(&self, event_id: &EventId) -> Result<u32> {
        let path = self.event_path(event_id);
        if !path.exists() {
            return Ok(0);
        }
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("reading spool entry: {}", path))?;
        let mut entry: SpoolEntry = serde_json::from_str(&data)
            .with_context(|| format!("parsing spool entry: {}", path))?;
        entry.attempts += 1;
        entry.last_attempt_at = Some(Utc::now());
        let json = serde_json::to_string(&entry)?;
        std::fs::write(&path, json)?;
        Ok(entry.attempts)
    }

    pub fn clone_for_heartbeat(&self) -> SpoolWriter {
        SpoolWriter {
            spool_dir: self.spool_dir.clone(),
            max_events: self.max_events,
            max_bytes: self.max_bytes,
        }
    }

    fn enforce_limits(&self) -> Result<()> {
        let pending_dir = self.spool_dir.join("pending");
        let entries: Vec<_> = std::fs::read_dir(&pending_dir)
            .with_context(|| format!("reading pending dir: {}", pending_dir))?
            .filter_map(|e| e.ok())
            .collect();

        if entries.len() >= self.max_events {
            tracing::warn!(
                current = entries.len(),
                max = self.max_events,
                "spool event limit reached"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    async fn append_and_mark_delivered() {
        let dir = tempfile::tempdir().unwrap();
        let spool_dir = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let writer = SpoolWriter::new(&spool_dir, 100, 1024 * 1024).unwrap();

        let event = make_event("11111111-1111-1111-1111-111111111111");
        let entry = writer.append(&event).await.unwrap();
        assert_eq!(entry.status, DeliveryStatus::Pending);

        writer.mark_delivered(&event.event_id).await.unwrap();

        let pending = writer.pending_entries().await.unwrap();
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn append_and_mark_failed() {
        let dir = tempfile::tempdir().unwrap();
        let spool_dir = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let writer = SpoolWriter::new(&spool_dir, 100, 1024 * 1024).unwrap();

        let event = make_event("22222222-2222-2222-2222-222222222222");
        writer.append(&event).await.unwrap();
        writer
            .mark_failed(&event.event_id, "permanent error")
            .await
            .unwrap();

        let pending = writer.pending_entries().await.unwrap();
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn pending_entries_returns_only_pending() {
        let dir = tempfile::tempdir().unwrap();
        let spool_dir = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let writer = SpoolWriter::new(&spool_dir, 100, 1024 * 1024).unwrap();

        let event1 = make_event("33333333-3333-3333-3333-333333333333");
        let event2 = make_event("44444444-4444-4444-4444-444444444444");
        writer.append(&event1).await.unwrap();
        writer.append(&event2).await.unwrap();
        writer.mark_delivered(&event1.event_id).await.unwrap();

        let pending = writer.pending_entries().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].event.event_id, event2.event_id);
    }
}
