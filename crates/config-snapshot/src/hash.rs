use anyhow::Context;
use camino::Utf8Path;

pub fn compute_blake3(content: &[u8]) -> String {
    blake3::hash(content).to_hex().to_string()
}

pub async fn compute_blake3_file(path: &Utf8Path) -> anyhow::Result<String> {
    let content = tokio::fs::read(path)
        .await
        .with_context(|| format!("failed to read file for hashing: {}", path))?;
    Ok(compute_blake3(&content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_blake3_deterministic() {
        let h1 = compute_blake3(b"hello world");
        let h2 = compute_blake3(b"hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn compute_blake3_different_content() {
        let h1 = compute_blake3(b"hello");
        let h2 = compute_blake3(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn compute_blake3_empty() {
        let h = compute_blake3(b"");
        assert!(!h.is_empty());
    }

    #[tokio::test]
    async fn compute_blake3_file_reads_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.yaml");
        std::fs::write(&path, "key: value").unwrap();
        let path = camino::Utf8PathBuf::from_path_buf(path).unwrap();
        let hash = compute_blake3_file(&path).await.unwrap();
        assert!(!hash.is_empty());
    }
}
