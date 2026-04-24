use camino::Utf8Path;

use config_shared::attribution::{Attribution, AttributionConfidence, AttributionSource};
use config_shared::events::ChangeKind;

use crate::config::AgentConfig;

pub struct AttributionResolver {
    #[allow(dead_code)]
    config: AgentConfig,
}

impl AttributionResolver {
    pub fn new(config: &AgentConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    pub fn resolve(&self, path: &Utf8Path, event_kind: &ChangeKind) -> Attribution {
        let mut attribution = Attribution::unknown();

        if matches!(event_kind, ChangeKind::Deleted) {
            attribution.author_source = AttributionSource::FileSystemMetadata;
            attribution.confidence = AttributionConfidence::Weak;
            return attribution;
        }

        if let Ok(metadata) = path.metadata() {
            attribution.author_source = AttributionSource::FileSystemMetadata;
            attribution.confidence = AttributionConfidence::Weak;

            if let Ok(modified) = metadata.modified() {
                let _modified_time = modified;
            }
        }

        attribution
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentConfig, WatchRootConfig};
    use camino::Utf8PathBuf;

    fn test_config() -> AgentConfig {
        AgentConfig::from_file("nonexistent").unwrap_or_else(|_| AgentConfig {
            agent_id: "test".into(),
            environment: "test".into(),
            host_labels: Default::default(),
            control_plane_base_url: "http://localhost:8080".into(),
            watch_roots: vec![WatchRootConfig {
                root_path: Utf8PathBuf::from("/tmp"),
                recursive: true,
            }],
            include_globs: vec!["**/*.yaml".into()],
            exclude_globs: vec![],
            debounce_window_ms: 500,
            snapshot_dir: Utf8PathBuf::from("/tmp/snapshots"),
            spool_dir: Utf8PathBuf::from("/tmp/spool"),
            enrollment_token: String::new(),
            content_preview_max_bytes: 4096,
            redaction_patterns: vec![],
            heartbeat_interval_secs: 30,
            query_timeout_secs: 10,
            max_spool_events: 10000,
            max_spool_bytes: 524288000,
            max_file_size_bytes: 1048576,
            agent_api_bind_addr: "0.0.0.0:9090".into(),
            tunnel_enabled: true,
            tunnel_reconnect_base_secs: 1,
            tunnel_reconnect_max_secs: 30,
            diff: config_diff::DiffConfig::default(),
        })
    }

    #[test]
    fn resolve_deleted_returns_weak() {
        let config = test_config();
        let resolver = AttributionResolver::new(&config);
        let result = resolver.resolve(Utf8Path::new("/tmp/config.yaml"), &ChangeKind::Deleted);
        assert_eq!(result.confidence, AttributionConfidence::Weak);
    }

    #[test]
    fn resolve_default_is_unknown() {
        let config = test_config();
        let resolver = AttributionResolver::new(&config);
        let result = resolver.resolve(Utf8Path::new("/nonexistent.yaml"), &ChangeKind::Modified);
        assert_eq!(result.confidence, AttributionConfidence::Unknown);
    }
}