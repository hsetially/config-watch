use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct ControlPlaneMetrics {
    pub events_ingested: AtomicU64,
    pub events_duplicate: AtomicU64,
    pub events_rejected: AtomicU64,
    pub ingest_latency_us: AtomicU64,
    pub active_websocket_subscriptions: AtomicU64,
    pub file_queries_stat: AtomicU64,
    pub file_queries_preview: AtomicU64,
    pub db_write_failures: AtomicU64,
    pub tunnel_connections_active: AtomicU64,
    pub tunnel_queries_routed: AtomicU64,
    pub tunnel_queries_fallback: AtomicU64,
}

impl ControlPlaneMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn increment(&self, counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "events_ingested": self.events_ingested.load(Ordering::Relaxed),
            "events_duplicate": self.events_duplicate.load(Ordering::Relaxed),
            "events_rejected": self.events_rejected.load(Ordering::Relaxed),
            "ingest_latency_us": self.ingest_latency_us.load(Ordering::Relaxed),
            "active_websocket_subscriptions": self.active_websocket_subscriptions.load(Ordering::Relaxed),
            "file_queries_stat": self.file_queries_stat.load(Ordering::Relaxed),
            "file_queries_preview": self.file_queries_preview.load(Ordering::Relaxed),
            "db_write_failures": self.db_write_failures.load(Ordering::Relaxed),
            "tunnel_connections_active": self.tunnel_connections_active.load(Ordering::Relaxed),
            "tunnel_queries_routed": self.tunnel_queries_routed.load(Ordering::Relaxed),
            "tunnel_queries_fallback": self.tunnel_queries_fallback.load(Ordering::Relaxed),
        })
    }
}