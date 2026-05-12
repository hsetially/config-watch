use config_diff::DiffConfig;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    #[serde(default = "default_auth_secret")]
    pub secret: String,
    #[serde(default = "default_auth_base_url")]
    pub base_url: String,
    /// Origins trusted for CSRF checks (e.g. the dashboard URL).
    /// Defaults to deriving from base_url plus common dev ports.
    #[serde(default = "default_trusted_origins")]
    pub trusted_origins: Vec<String>,
    /// Whether new users require admin approval before they can sign in.
    /// When true, new users are created with `banned=true` and `role='pending_approval'`.
    #[serde(default = "default_true")]
    pub require_approval: bool,
    /// When true (the production default), session and CSRF cookies are issued
    /// with the `Secure` attribute so they only travel over HTTPS. Set to false
    /// in local dev (plain HTTP) so the browser actually stores the cookies.
    #[serde(default = "default_true")]
    pub tls_required: bool,
    /// Secret token for admin API endpoints (approve/reject/list pending users).
    /// Falls back to `control_plane_secret` if not set.
    #[serde(default)]
    pub admin_api_secret: Option<String>,
    #[serde(default)]
    pub github_client_id: Option<String>,
    #[serde(default)]
    pub github_client_secret: Option<String>,
    #[serde(default)]
    pub google_client_id: Option<String>,
    #[serde(default)]
    pub google_client_secret: Option<String>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            secret: default_auth_secret(),
            base_url: default_auth_base_url(),
            trusted_origins: default_trusted_origins(),
            require_approval: true,
            tls_required: true,
            admin_api_secret: None,
            github_client_id: None,
            github_client_secret: None,
            google_client_id: None,
            google_client_secret: None,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_auth_secret() -> String {
    std::env::var("CONFIG_WATCH_AUTH_SECRET")
        .unwrap_or_else(|_| "change-me-in-production-at-least-32-characters-long".into())
}

fn default_auth_base_url() -> String {
    std::env::var("CONFIG_WATCH_AUTH_BASE_URL").unwrap_or_else(|_| "http://localhost:8082".into())
}

fn default_trusted_origins() -> Vec<String> {
    // M8: No default trusted origins in production.
    // Set CONFIG_WATCH_AUTH_TRUSTED_ORIGINS explicitly to allow CORS/CSRF origins.
    // For local development, use a config file or env var like:
    //   CONFIG_WATCH_AUTH_TRUSTED_ORIGINS=http://localhost:8080,http://localhost:8082
    Vec::new()
}

/// Broadcast mode for cross-pod event fan-out.
///
/// - `Local`: Use in-process broadcast only (single-pod, no Postgres LISTEN/NOTIFY).
/// - `Postgres`: Use Postgres LISTEN/NOTIFY for cross-pod event fan-out (multi-pod deployment).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BroadcastMode {
    #[default]
    Local,
    Postgres,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ControlPlaneConfig {
    pub bind_addr: String,
    pub database_url: String,
    #[serde(default = "default_secret")]
    pub control_plane_secret: String,
    #[serde(default = "default_query_timeout_secs")]
    pub query_timeout_secs: u64,
    #[serde(default = "default_snapshot_data_dir")]
    pub snapshot_data_dir: String,
    #[serde(default = "default_repos_dir")]
    pub repos_dir: String,
    #[serde(default)]
    pub github_token: Option<String>,
    /// Diff rendering settings. Diffs are rendered server-side now (lazy, on
    /// dashboard request) so this lives on the control plane rather than per
    /// agent. Defaults match the legacy agent defaults.
    #[serde(default)]
    pub diff: DiffConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    /// Broadcast mode: "local" (single-pod, default) or "postgres" (multi-pod with LISTEN/NOTIFY).
    #[serde(default)]
    pub broadcast_mode: BroadcastMode,
}

impl ControlPlaneConfig {
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

        // C2: Reject default/weak secrets at startup — fail-closed.
        const DEFAULT_CP_SECRET: &str = "dev-secret-change-me";
        const DEFAULT_AUTH_SECRET: &str = "change-me-in-production-at-least-32-characters-long";
        if cfg.control_plane_secret == DEFAULT_CP_SECRET {
            anyhow::bail!(
                "control_plane_secret is set to the default value. \
                 Set CONFIG_WATCH_CONTROL_PLANE_SECRET to a strong random value (≥32 chars)"
            );
        }
        if cfg.control_plane_secret.len() < 32 {
            anyhow::bail!(
                "control_plane_secret is too short ({} chars). Must be ≥32 characters.",
                cfg.control_plane_secret.len()
            );
        }
        if cfg.auth.secret == DEFAULT_AUTH_SECRET {
            anyhow::bail!(
                "auth.secret is set to the default value. \
                 Set CONFIG_WATCH_AUTH_SECRET to a strong random value (≥32 chars)"
            );
        }
        if cfg.auth.secret.len() < 32 {
            anyhow::bail!(
                "auth.secret is too short ({} chars). Must be ≥32 characters.",
                cfg.auth.secret.len()
            );
        }

        Ok(cfg)
    }
}

fn default_secret() -> String {
    "dev-secret-change-me".into()
}

fn default_query_timeout_secs() -> u64 {
    10
}

fn default_snapshot_data_dir() -> String {
    "./data/snapshots".into()
}

fn default_repos_dir() -> String {
    "./data/repos".into()
}
