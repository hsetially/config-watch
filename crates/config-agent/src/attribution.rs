use camino::Utf8Path;

use config_shared::attribution::{Attribution, AttributionConfidence, AttributionSource};
use config_shared::events::ChangeKind;

use crate::config::AgentConfig;

fn resolve_author_name(path: &Utf8Path) -> Option<String> {
    for var in &["SUDO_USER", "USER", "LOGNAME", "USERNAME"] {
        if let Ok(user) = std::env::var(var) {
            if !user.is_empty() {
                return Some(user);
            }
        }
    }

    #[cfg(unix)]
    {
        if let Some(name) = file_owner_name(path) {
            return Some(name);
        }
    }

    let _ = path; // used on unix
    None
}

#[cfg(unix)]
fn file_owner_name(path: &Utf8Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let meta = path.metadata().ok()?;
    let uid = meta.uid();
    resolve_uid(uid)
}

#[cfg(unix)]
fn resolve_uid(uid: u32) -> Option<String> {
    let content = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in content.lines() {
        let mut parts = line.splitn(4, ':');
        let name = parts.next()?;
        parts.next()?; // password
        if let Some(uid_str) = parts.next() {
            if uid_str.parse::<u32>().ok()? == uid {
                return Some(name.to_string());
            }
        }
    }
    None
}

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
        let author_name = resolve_author_name(path);

        if matches!(event_kind, ChangeKind::Deleted) {
            return Attribution {
                author_name,
                author_source: AttributionSource::FileSystemMetadata,
                confidence: AttributionConfidence::Weak,
                process_hint: None,
                ssh_session_hint: None,
                deployment_hint: None,
            };
        }

        if path.exists() {
            Attribution {
                author_name,
                author_source: AttributionSource::FileSystemMetadata,
                confidence: AttributionConfidence::Weak,
                process_hint: None,
                ssh_session_hint: None,
                deployment_hint: None,
            }
        } else {
            Attribution {
                author_name,
                author_source: AttributionSource::Unknown,
                confidence: AttributionConfidence::Unknown,
                process_hint: None,
                ssh_session_hint: None,
                deployment_hint: None,
            }
        }
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

    #[test]
    fn resolve_existing_file_sets_weak_confidence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.yaml");
        std::fs::write(&path, "key: value").unwrap();
        let utf8_path = Utf8PathBuf::from_path_buf(path).unwrap();

        let config = test_config();
        let resolver = AttributionResolver::new(&config);
        let result = resolver.resolve(&utf8_path, &ChangeKind::Modified);
        assert_eq!(result.confidence, AttributionConfidence::Weak);
        assert_eq!(result.author_source, AttributionSource::FileSystemMetadata);
    }

    #[test]
    fn resolve_populates_author_name() {
        let saved_user = std::env::var("USER").ok();
        std::env::set_var("USER", "configadmin");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.yaml");
        std::fs::write(&path, "key: value").unwrap();
        let utf8_path = Utf8PathBuf::from_path_buf(path).unwrap();

        let config = test_config();
        let resolver = AttributionResolver::new(&config);
        let result = resolver.resolve(&utf8_path, &ChangeKind::Modified);

        if let Some(v) = saved_user {
            std::env::set_var("USER", v);
        } else {
            std::env::remove_var("USER");
        }

        assert_eq!(result.author_name, Some("configadmin".to_string()));
    }
}
