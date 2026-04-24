use std::collections::HashMap;
use std::sync::Arc;

use config_snapshot::store::SnapshotStore;
use config_storage::db::Database;
use config_transport::websocket::RealtimeMessage;
use tokio::sync::broadcast;

use crate::metrics::ControlPlaneMetrics;
use crate::tunnel::AgentRegistry;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub broadcast_tx: broadcast::Sender<RealtimeMessage>,
    pub secret: String,
    pub operator_keys: Arc<HashMap<String, (String, String)>>,
    pub metrics: Arc<ControlPlaneMetrics>,
    pub tunnel_registry: Arc<AgentRegistry>,
    pub query_timeout_secs: u64,
    pub snapshot_store: Arc<SnapshotStore>,
    pub repos_dir: String,
    pub github_token: Option<String>,
}

impl AppState {
    pub fn new(db: Database, secret: String, snapshot_store: SnapshotStore) -> Self {
        let metrics = ControlPlaneMetrics::new();
        let (broadcast_tx, _) = broadcast::channel(256);
        let tunnel_registry = Arc::new(AgentRegistry::new(metrics.clone()));
        Self {
            db: Arc::new(db),
            broadcast_tx,
            secret,
            operator_keys: Arc::new(HashMap::new()),
            metrics,
            tunnel_registry,
            query_timeout_secs: 10,
            snapshot_store: Arc::new(snapshot_store),
            repos_dir: "./data/repos".to_string(),
            github_token: None,
        }
    }

    pub fn with_operator_keys(mut self, keys: HashMap<String, (String, String)>) -> Self {
        self.operator_keys = Arc::new(keys);
        self
    }

    pub fn with_broadcast_capacity(db: Database, secret: String, capacity: usize, snapshot_store: SnapshotStore) -> Self {
        let metrics = ControlPlaneMetrics::new();
        let (broadcast_tx, _) = broadcast::channel(capacity);
        let tunnel_registry = Arc::new(AgentRegistry::new(metrics.clone()));
        Self {
            db: Arc::new(db),
            broadcast_tx,
            secret,
            operator_keys: Arc::new(HashMap::new()),
            metrics,
            tunnel_registry,
            query_timeout_secs: 10,
            snapshot_store: Arc::new(snapshot_store),
            repos_dir: "./data/repos".to_string(),
            github_token: None,
        }
    }

    pub fn with_query_timeout(mut self, timeout_secs: u64) -> Self {
        self.query_timeout_secs = timeout_secs;
        self
    }

    pub fn with_repos_dir(mut self, repos_dir: String) -> Self {
        self.repos_dir = repos_dir;
        self
    }

    pub fn with_github_token(mut self, token: Option<String>) -> Self {
        self.github_token = token.filter(|t| !t.is_empty());
        self
    }
}