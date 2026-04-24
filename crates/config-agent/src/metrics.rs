use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct AgentMetrics {
    pub watcher_events_received: AtomicU64,
    pub events_normalized: AtomicU64,
    pub events_suppressed_unchanged: AtomicU64,
    pub events_published: AtomicU64,
    pub events_publish_failed: AtomicU64,
    pub diff_latency_us: AtomicU64,
    pub spool_depth: AtomicU64,
    pub snapshot_read_failures: AtomicU64,
    pub last_control_plane_contact: AtomicU64,
}

impl AgentMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn increment(&self, counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "watcher_events_received": self.watcher_events_received.load(Ordering::Relaxed),
            "events_normalized": self.events_normalized.load(Ordering::Relaxed),
            "events_suppressed_unchanged": self.events_suppressed_unchanged.load(Ordering::Relaxed),
            "events_published": self.events_published.load(Ordering::Relaxed),
            "events_publish_failed": self.events_publish_failed.load(Ordering::Relaxed),
            "diff_latency_us": self.diff_latency_us.load(Ordering::Relaxed),
            "spool_depth": self.spool_depth.load(Ordering::Relaxed),
            "snapshot_read_failures": self.snapshot_read_failures.load(Ordering::Relaxed),
            "last_control_plane_contact": self.last_control_plane_contact.load(Ordering::Relaxed),
        })
    }
}
