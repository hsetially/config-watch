use std::collections::HashMap;

use camino::Utf8PathBuf;
use serde::Deserialize;

use config_diff::DiffConfig;
use config_shared::ids::HostId;

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    pub agent_id: String,
    pub environment: String,
    #[serde(default)]
    pub host_labels: HashMap<String, String>,
    pub control_plane_base_url: String,
    pub watch_roots: Vec<WatchRootConfig>,
    #[serde(default = "default_include_globs")]
    pub include_globs: Vec<String>,
    #[serde(default = "default_exclude_globs")]
    pub exclude_globs: Vec<String>,
    #[serde(default = "default_debounce_window_ms")]
    pub debounce_window_ms: u64,
    #[serde(default = "default_snapshot_dir")]
    pub snapshot_dir: Utf8PathBuf,
    #[serde(default = "default_spool_dir")]
    pub spool_dir: Utf8PathBuf,
    #[serde(default)]
    pub enrollment_token: String,
    #[serde(default = "default_content_preview_max_bytes")]
    pub content_preview_max_bytes: usize,
    #[serde(default = "default_redaction_patterns")]
    pub redaction_patterns: Vec<String>,
    #[serde(default = "default_heartbeat_interval_secs")]
    pub heartbeat_interval_secs: u64,
    #[serde(default = "default_query_timeout_secs")]
    pub query_timeout_secs: u64,
    #[serde(default = "default_max_spool_events")]
    pub max_spool_events: usize,
    #[serde(default = "default_max_spool_bytes")]
    pub max_spool_bytes: u64,
    #[serde(default = "default_max_file_size_bytes")]
    pub max_file_size_bytes: u64,
    #[serde(default = "default_agent_api_bind_addr")]
    pub agent_api_bind_addr: String,
    /// HMAC secret for authenticating queries to the agent API.
    /// If empty (default), auth is skipped — only safe when bound to 127.0.0.1.
    /// Set CONFIG_WATCH_AGENT_API_SECRET to enable HMAC auth on the query API.
    #[serde(default)]
    pub agent_api_secret: String,
    #[serde(default = "default_true")]
    pub tunnel_enabled: bool,
    #[serde(default = "default_tunnel_reconnect_base_secs")]
    pub tunnel_reconnect_base_secs: u64,
    #[serde(default = "default_tunnel_reconnect_max_secs")]
    pub tunnel_reconnect_max_secs: u64,
    /// When true (the production default), the control-plane HTTP client
    /// enforces https:// + TLS 1.2 and `validate()` rejects an http:// base URL.
    /// Set to false in local dev only.
    #[serde(default = "default_true")]
    pub tls_required: bool,
    /// Watcher backend: "auto" (detect NFS → poll), "poll" (force PollWatcher),
    /// or "inotify" (force RecommendedWatcher). Default: "auto".
    #[serde(default = "default_watch_mode")]
    pub watch_mode: String,
    /// Poll interval in seconds when using PollWatcher. Default: 2.
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    #[serde(default)]
    pub diff: DiffConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatchRootConfig {
    pub root_path: Utf8PathBuf,
    #[serde(default = "default_true")]
    pub recursive: bool,
}

impl AgentConfig {
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let settings = config::Config::builder()
            .add_source(config::File::with_name(path))
            .add_source(
                config::Environment::with_prefix("CONFIG_WATCH")
                    .separator("_")
                    .try_parsing(true),
            )
            .build()?;

        let cfg: Self = settings.try_deserialize()?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn host_id(&self) -> anyhow::Result<HostId> {
        let uuid = uuid::Uuid::parse_str(&self.agent_id)?;
        Ok(HostId::from(uuid))
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.watch_roots.is_empty() {
            anyhow::bail!("watch_roots must not be empty");
        }
        if !matches!(self.watch_mode.as_str(), "auto" | "poll" | "inotify") {
            anyhow::bail!(
                "watch_mode must be one of: auto, poll, inotify (got {})",
                self.watch_mode
            );
        }
        if self.poll_interval_secs == 0 {
            anyhow::bail!("poll_interval_secs must be > 0");
        }
        if self.tls_required && !self.control_plane_base_url.starts_with("https://") {
            anyhow::bail!(
                "tls_required is true but control_plane_base_url is not https:// \
                 (got {}). Use https:// in production, or set tls_required = false for local dev.",
                self.control_plane_base_url
            );
        }
        for root in &self.watch_roots {
            if !root.root_path.exists() {
                tracing::warn!(path = %root.root_path, "watch root does not exist yet");
            }
        }
        Ok(())
    }
}

fn default_include_globs() -> Vec<String> {
    vec!["**/*.yaml".into(), "**/*.yml".into()]
}

fn default_exclude_globs() -> Vec<String> {
    vec!["**/.*".into(), "**/*.sw?".into(), "**/~*".into()]
}

fn default_debounce_window_ms() -> u64 {
    500
}

fn default_snapshot_dir() -> Utf8PathBuf {
    Utf8PathBuf::from("/var/lib/config-watch/snapshots")
}

fn default_spool_dir() -> Utf8PathBuf {
    Utf8PathBuf::from("/var/lib/config-watch/spool")
}

fn default_content_preview_max_bytes() -> usize {
    4096
}

fn default_redaction_patterns() -> Vec<String> {
    vec![r"(?i)(token|secret|password|key|credential)".into()]
}

fn default_heartbeat_interval_secs() -> u64 {
    30
}

fn default_query_timeout_secs() -> u64 {
    10
}

fn default_max_spool_events() -> usize {
    10_000
}

fn default_max_spool_bytes() -> u64 {
    500 * 1024 * 1024
}

fn default_max_file_size_bytes() -> u64 {
    1024 * 1024
}

fn default_agent_api_bind_addr() -> String {
    "127.0.0.1:9090".into()
}

fn default_tunnel_reconnect_base_secs() -> u64 {
    1
}

fn default_tunnel_reconnect_max_secs() -> u64 {
    30
}

fn default_true() -> bool {
    true
}

fn default_watch_mode() -> String {
    "auto".into()
}

fn default_poll_interval_secs() -> u64 {
    2
}
