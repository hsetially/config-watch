use anyhow::{bail, Context};
use camino::{Utf8Path, Utf8PathBuf};

pub fn normalize_path(path: &str) -> anyhow::Result<Utf8PathBuf> {
    let p = Utf8Path::new(path);

    for component in p.components() {
        let s = component.as_str();
        if s == ".." {
            bail!("path traversal detected: {}", path);
        }
    }

    let canonical = std::fs::canonicalize(p)
        .with_context(|| format!("failed to canonicalize path: {}", path))?;

    Utf8PathBuf::from_path_buf(canonical)
        .map_err(|_| anyhow::anyhow!("non-UTF-8 path after canonicalization: {}", path))
}

pub fn canonicalize_watch_path(
    path: &Utf8Path,
    roots: &[Utf8PathBuf],
) -> anyhow::Result<Utf8PathBuf> {
    let canonical = normalize_path(path.as_str())?;

    for root in roots {
        let canonical_root = normalize_path(root.as_str())?;
        if canonical.starts_with(&canonical_root) {
            return Ok(canonical);
        }
    }

    bail!("path {:?} is not under any configured watch root", path)
}

pub fn is_yaml_file(path: &Utf8Path) -> bool {
    match path.extension() {
        Some(ext) => ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"),
        None => false,
    }
}

pub fn strip_watch_root(path: &Utf8Path, root: &Utf8Path) -> anyhow::Result<Utf8PathBuf> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| anyhow::anyhow!("path {:?} does not start with root {:?}", path, root))?;
    Ok(relative.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn is_yaml_file_detects_yaml() {
        assert!(is_yaml_file(Utf8Path::new("config.yaml")));
        assert!(is_yaml_file(Utf8Path::new("config.yml")));
        assert!(is_yaml_file(Utf8Path::new("CONFIG.YAML")));
    }

    #[test]
    fn is_yaml_file_rejects_non_yaml() {
        assert!(!is_yaml_file(Utf8Path::new("config.json")));
        assert!(!is_yaml_file(Utf8Path::new("config.toml")));
        assert!(!is_yaml_file(Utf8Path::new("config")));
    }

    #[test]
    fn reject_path_traversal() {
        let result = normalize_path("../etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn strip_watch_root_basic() {
        let path = Utf8Path::new("/etc/myapp/config.yaml");
        let root = Utf8Path::new("/etc/myapp");
        assert_eq!(
            strip_watch_root(path, root).unwrap(),
            Utf8PathBuf::from("config.yaml")
        );
    }

    #[test]
    fn strip_watch_root_mismatch() {
        let path = Utf8Path::new("/other/config.yaml");
        let root = Utf8Path::new("/etc/myapp");
        assert!(strip_watch_root(path, root).is_err());
    }

    #[test]
    fn normalize_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.yaml");
        fs::write(&path, "key: value").unwrap();
        let result = normalize_path(path.to_str().unwrap());
        assert!(result.is_ok());
        assert!(result.unwrap().ends_with("test.yaml"));
    }
}
