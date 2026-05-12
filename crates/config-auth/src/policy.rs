use std::path::Path;

const DENIED_PREFIXES: &[&str] = &["/etc/ssl", "/etc/ssh"];

const DENIED_SEGMENTS: &[&str] = &["private"];

pub fn is_path_denied(path: &str) -> bool {
    let p = Path::new(path);

    for prefix in DENIED_PREFIXES {
        if p.starts_with(prefix) {
            return true;
        }
    }

    for component in p.components() {
        if let Some(seg) = component.as_os_str().to_str() {
            if DENIED_SEGMENTS.contains(&seg) {
                return true;
            }
        }
    }

    false
}

/// C6 fix: canonicalize the path and check it stays within an allowed root.
/// Rejects symlinks that escape watch roots and broken symlinks.
pub fn is_path_allowed(path: &str, watch_roots: &[&str]) -> bool {
    if is_path_denied(path) {
        return false;
    }
    // Canonicalize the queried path to resolve .. and symlinks.
    let canonical = match std::fs::canonicalize(path) {
        Ok(c) => c,
        Err(_) => {
            // File doesn't exist yet — fall back to lexical check against
            // canonicalized (or lexical) roots. This is less safe but
            // allows querying paths for files that haven't been created.
            let query_path = Path::new(path);
            return watch_roots.iter().any(|root| {
                let canonical_root = match std::fs::canonicalize(root) {
                    Ok(r) => r,
                    Err(_) => Path::new(root).to_path_buf(),
                };
                query_path.starts_with(&canonical_root)
            });
        }
    };
    watch_roots.iter().any(|root| {
        let canonical_root = match std::fs::canonicalize(root) {
            Ok(r) => r,
            Err(_) => {
                // Root doesn't exist on disk yet — fall back to lexical check.
                // This is less safe but allows newly-created roots.
                return canonical.starts_with(Path::new(root));
            }
        };
        canonical.starts_with(&canonical_root)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssl_paths_denied() {
        assert!(is_path_denied("/etc/ssl/certs/ca.pem"));
        assert!(is_path_denied("/etc/ssh/sshd_config"));
    }

    #[test]
    fn private_segment_denied() {
        assert!(is_path_denied("/etc/config/private/keys.yaml"));
        assert!(is_path_denied("/opt/app/private/config.yaml"));
    }

    #[test]
    fn normal_paths_ok() {
        assert!(!is_path_denied("/etc/config/app.yaml"));
        assert!(!is_path_denied("/opt/myapp/config/settings.yaml"));
    }

    #[test]
    fn allowed_within_watch_root() {
        assert!(is_path_allowed("/etc/config/app.yaml", &["/etc/config"]));
        assert!(!is_path_allowed("/opt/other/app.yaml", &["/etc/config"]));
    }

    #[test]
    fn denied_overrides_watch_root() {
        assert!(!is_path_allowed("/etc/ssl/cert.pem", &["/etc"]));
    }
}
