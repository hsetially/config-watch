use serde::Deserialize;

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