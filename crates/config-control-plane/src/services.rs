use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use better_auth::adapters::SqlxAdapter;
use better_auth::core::auth::BetterAuth;
use config_diff::DiffConfig;
use config_snapshot::store::SnapshotStore;
use config_storage::db::Database;
use config_transport::websocket::RealtimeMessage;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::diff_service::DiffService;
use crate::metrics::ControlPlaneMetrics;
use crate::tunnel::AgentRegistry;

/// Type alias for the auth state used throughout the application.
pub type AuthState = Arc<BetterAuth<SqlxAdapter>>;

const LOCAL_EVENT_DEDUP_CAPACITY: usize = 64;

/// Shared deduplication set for recently ingested event IDs.
/// Used by the PgListener to skip events that the local pod already
/// broadcast via `broadcast_tx.send()`, preventing double-delivery
/// to WebSocket clients.
pub type LocalEventDedup = Arc<std::sync::Mutex<VecDeque<Uuid>>>;

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
    /// Server-side diff renderer. Single instance per process — owns the only
    /// `DiffEngine` and the lazy render cache.
    pub diff_service: Arc<DiffService>,
    /// BetterAuth instance for user authentication (sessions, API keys, etc.)
    pub auth: AuthState,
    /// Secret token for admin API endpoints (approve/reject/list pending users).
    /// Falls back to `secret` if not configured.
    pub admin_api_secret: Option<String>,
    /// Whether new users require admin approval before they can sign in.
    pub require_approval: bool,
    /// Whether HTTPS is required (mirrors `auth.tls_required`). Used by the
    /// auth-proxy handler to decide whether the CSRF cookie carries `Secure`.
    pub tls_required: bool,
    /// Deduplication set for event IDs recently ingested by this pod.
    /// The PgListener checks this before forwarding remote events to avoid
    /// double-broadcasting events that the local ingest handler already sent.
    pub local_event_dedup: LocalEventDedup,
}

impl AppState {
    pub fn new(db: Database, secret: String, snapshot_store: SnapshotStore, auth: AuthState) -> Self {
        Self::new_with_diff_config(db, secret, snapshot_store, DiffConfig::default(), auth)
    }

    pub fn new_with_diff_config(
        db: Database,
        secret: String,
        snapshot_store: SnapshotStore,
        diff_config: DiffConfig,
        auth: AuthState,
    ) -> Self {
        let metrics = ControlPlaneMetrics::new();
        let (broadcast_tx, _) = broadcast::channel(256);
        let tunnel_registry = Arc::new(AgentRegistry::new(metrics.clone()));
        let diff_service = Arc::new(DiffService::new(diff_config));
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
            diff_service,
            auth,
            admin_api_secret: None,
            require_approval: true,
            tls_required: true,
            local_event_dedup: Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(LOCAL_EVENT_DEDUP_CAPACITY))),
        }
    }

    pub fn with_operator_keys(mut self, keys: HashMap<String, (String, String)>) -> Self {
        self.operator_keys = Arc::new(keys);
        self
    }

    pub fn with_broadcast_capacity(
        db: Database,
        secret: String,
        capacity: usize,
        snapshot_store: SnapshotStore,
        auth: AuthState,
    ) -> Self {
        let metrics = ControlPlaneMetrics::new();
        let (broadcast_tx, _) = broadcast::channel(capacity);
        let tunnel_registry = Arc::new(AgentRegistry::new(metrics.clone()));
        let diff_service = Arc::new(DiffService::new(DiffConfig::default()));
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
            diff_service,
            auth,
            admin_api_secret: None,
            require_approval: true,
            tls_required: true,
            local_event_dedup: Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(LOCAL_EVENT_DEDUP_CAPACITY))),
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

    pub fn with_admin_api_secret(mut self, secret: Option<String>) -> Self {
        self.admin_api_secret = secret;
        self
    }

    pub fn with_require_approval(mut self, require: bool) -> Self {
        self.require_approval = require;
        self
    }

    pub fn with_tls_required(mut self, tls_required: bool) -> Self {
        self.tls_required = tls_required;
        self
    }
}
