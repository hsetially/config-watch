use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};

use crate::ids::{HostId, IdempotencyKey};

pub fn validate_non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

pub fn validate_path_in_roots(path: &Utf8Path, roots: &[Utf8PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

pub fn validate_yaml_content(content: &[u8]) -> bool {
    serde_yaml::from_slice::<serde_yaml::Value>(content).is_ok()
}

pub fn validate_idempotency_key(key: &IdempotencyKey) -> bool {
    !key.0.is_empty()
}

pub fn derive_idempotency_key(
    host_id: &HostId,
    path: &Utf8Path,
    prev_hash: &str,
    curr_hash: &str,
    time_bucket: DateTime<Utc>,
) -> IdempotencyKey {
    use std::fmt::Write;

    let bucket = time_bucket.format("%Y%m%d%H%M");
    let mut s = String::with_capacity(128);
    write!(
        s,
        "{}:{}:{}:{}:{}",
        host_id, path, prev_hash, curr_hash, bucket
    )
    .unwrap();
    IdempotencyKey(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::HostId;
    use uuid::Uuid;

    #[test]
    fn validate_non_empty_works() {
        assert!(validate_non_empty("hello"));
        assert!(!validate_non_empty(""));
        assert!(!validate_non_empty("   "));
    }

    #[test]
    fn validate_path_in_roots_matches() {
        let roots = vec![Utf8PathBuf::from("/etc/myapp")];
        assert!(validate_path_in_roots(
            Utf8Path::new("/etc/myapp/config.yaml"),
            &roots
        ));
    }

    #[test]
    fn validate_path_in_roots_rejects() {
        let roots = vec![Utf8PathBuf::from("/etc/myapp")];
        assert!(!validate_path_in_roots(
            Utf8Path::new("/etc/other/config.yaml"),
            &roots
        ));
    }

    #[test]
    fn validate_yaml_content_valid() {
        assert!(validate_yaml_content(b"key: value"));
    }

    #[test]
    fn validate_yaml_content_invalid() {
        assert!(!validate_yaml_content(b": : invalid"));
    }

    #[test]
    fn validate_idempotency_key_works() {
        assert!(validate_idempotency_key(&IdempotencyKey("abc".into())));
        assert!(!validate_idempotency_key(&IdempotencyKey(String::new())));
    }

    #[test]
    fn derive_idempotency_key_deterministic() {
        let host_id = HostId(Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap());
        let path = Utf8Path::new("/etc/myapp/config.yaml");
        let time = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let k1 = derive_idempotency_key(&host_id, path, "abc", "def", time);
        let k2 = derive_idempotency_key(&host_id, path, "abc", "def", time);
        assert_eq!(k1, k2);
    }

    #[test]
    fn derive_idempotency_key_differs_on_inputs() {
        let host_id = HostId(Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap());
        let path = Utf8Path::new("/etc/myapp/config.yaml");
        let time = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let k1 = derive_idempotency_key(&host_id, path, "abc", "def", time);
        let k2 = derive_idempotency_key(&host_id, path, "abc", "xyz", time);
        assert_ne!(k1, k2);
    }
}
