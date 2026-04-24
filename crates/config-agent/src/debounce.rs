use std::collections::HashMap;

use chrono::{DateTime, Utc};
use camino::Utf8PathBuf;

use config_shared::events::ChangeKind;

use crate::watcher::{RawEventKind, RawWatchEvent};

#[derive(Debug, Clone)]
pub struct DebouncedEvent {
    pub canonical_path: Utf8PathBuf,
    pub event_kind: ChangeKind,
    pub observed_at: DateTime<Utc>,
    pub raw_event_count: usize,
}

struct DebounceWindowEntry {
    first_seen: DateTime<Utc>,
    latest_kind: RawEventKind,
    raw_count: usize,
    exists_before: bool,
}

pub struct DebounceWindow {
    window_ms: u64,
    pending: HashMap<Utf8PathBuf, DebounceWindowEntry>,
}

impl DebounceWindow {
    pub fn new(window_ms: u64) -> Self {
        Self {
            window_ms,
            pending: HashMap::new(),
        }
    }

    pub fn ingest(&mut self, event: RawWatchEvent, file_exists: bool) {
        let entry = self.pending.entry(event.raw_path.clone()).or_insert_with(|| {
            DebounceWindowEntry {
                first_seen: event.observed_at,
                latest_kind: event.event_kind.clone(),
                raw_count: 0,
                exists_before: file_exists,
            }
        });
        entry.latest_kind = event.event_kind;
        entry.raw_count += 1;
        if file_exists {
            entry.exists_before = true;
        }
    }

    pub fn flush_expired(&mut self) -> Vec<DebouncedEvent> {
        let now = Utc::now();
        let window_duration = chrono::Duration::milliseconds(self.window_ms as i64);
        let mut expired = Vec::new();

        let keys: Vec<Utf8PathBuf> = self.pending.keys().cloned().collect();

        for key in keys {
            if let Some(entry) = self.pending.get(&key) {
                if now - entry.first_seen >= window_duration {
                    if let Some(entry) = self.pending.remove(&key) {
                        let event_kind = map_debounced_kind(&entry.latest_kind, entry.exists_before);
                        expired.push(DebouncedEvent {
                            canonical_path: key,
                            event_kind,
                            observed_at: entry.first_seen,
                            raw_event_count: entry.raw_count,
                        });
                    }
                }
            }
        }

        expired
    }

    pub fn flush_all(&mut self) -> Vec<DebouncedEvent> {
        let mut result = Vec::new();
        let entries: Vec<(Utf8PathBuf, DebounceWindowEntry)> = self.pending.drain().collect();
        for (key, entry) in entries {
            let event_kind = map_debounced_kind(&entry.latest_kind, entry.exists_before);
            result.push(DebouncedEvent {
                canonical_path: key,
                event_kind,
                observed_at: entry.first_seen,
                raw_event_count: entry.raw_count,
            });
        }
        result
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

fn map_debounced_kind(latest: &RawEventKind, existed_before: bool) -> ChangeKind {
    match latest {
        RawEventKind::Created => ChangeKind::Created,
        RawEventKind::Deleted => ChangeKind::Deleted,
        RawEventKind::Modified => {
            if existed_before {
                ChangeKind::Modified
            } else {
                ChangeKind::Created
            }
        }
        RawEventKind::Other => ChangeKind::MetadataOnly,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_raw_event(path: &str, kind: RawEventKind) -> RawWatchEvent {
        RawWatchEvent {
            raw_path: Utf8PathBuf::from(path),
            event_kind: kind,
            observed_at: Utc::now(),
        }
    }

    #[test]
    fn burst_suppression_single_output() {
        let mut window = DebounceWindow::new(500);
        for _ in 0..10 {
            window.ingest(make_raw_event("/test.yaml", RawEventKind::Modified), true);
        }
        let flushed = window.flush_all();
        assert_eq!(flushed.len(), 1);
        assert_eq!(flushed[0].raw_event_count, 10);
    }

    #[test]
    fn different_paths_debounced_independently() {
        let mut window = DebounceWindow::new(500);
        window.ingest(make_raw_event("/a.yaml", RawEventKind::Modified), true);
        window.ingest(make_raw_event("/b.yaml", RawEventKind::Modified), true);
        let flushed = window.flush_all();
        assert_eq!(flushed.len(), 2);
    }

    #[test]
    fn created_maps_correctly() {
        let mut window = DebounceWindow::new(500);
        window.ingest(make_raw_event("/new.yaml", RawEventKind::Created), false);
        let flushed = window.flush_all();
        assert_eq!(flushed[0].event_kind, ChangeKind::Created);
    }

    #[test]
    fn deleted_maps_correctly() {
        let mut window = DebounceWindow::new(500);
        window.ingest(make_raw_event("/gone.yaml", RawEventKind::Deleted), true);
        let flushed = window.flush_all();
        assert_eq!(flushed[0].event_kind, ChangeKind::Deleted);
    }

    #[test]
    fn flush_expired_respects_window() {
        let mut window = DebounceWindow::new(500);
        window.ingest(make_raw_event("/test.yaml", RawEventKind::Modified), true);
        let flushed = window.flush_expired();
        assert!(flushed.is_empty());
    }

    #[test]
    fn pending_count_tracks_entries() {
        let mut window = DebounceWindow::new(500);
        assert_eq!(window.pending_count(), 0);
        window.ingest(make_raw_event("/a.yaml", RawEventKind::Modified), true);
        assert_eq!(window.pending_count(), 1);
        window.ingest(make_raw_event("/b.yaml", RawEventKind::Modified), true);
        assert_eq!(window.pending_count(), 2);
    }
}