use std::path::Path;

use anyhow::{Context, Result};
use base64::Engine;
use serde::Serialize;

use crate::redaction::RedactionEngine;
use config_transport::tunnel::FileContentResponse;

const MAX_CONTENT_BYTES: u64 = 10 * 1024 * 1024; // 10 MB limit
const DEFAULT_CHUNK_BYTES: u64 = 256 * 1024; // 256 KB per chunk

#[derive(Debug, Serialize)]
pub struct FileStatResponse {
    pub path: String,
    pub exists: bool,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
    pub permissions: Option<String>,
    pub content_hash: Option<String>,
    pub is_yaml: bool,
}

#[derive(Debug, Serialize)]
pub struct FilePreviewResponse {
    pub path: String,
    pub exists: bool,
    pub content: Option<String>,
    pub truncated: bool,
    pub redacted_keys: Vec<String>,
}

pub struct QueryHandler {
    redaction: RedactionEngine,
    watch_roots: Vec<String>,
}

impl QueryHandler {
    pub fn new(watch_roots: Vec<String>, redaction_patterns: Vec<String>, preview_max_bytes: usize) -> Self {
        // Resolve relative watch roots to absolute paths so that incoming absolute query paths
        // match correctly on Windows (an absolute path cannot start_with a relative path).
        let resolved_roots: Vec<String> = watch_roots
            .into_iter()
            .map(|r| {
                let p = std::path::Path::new(&r);
                if p.is_absolute() {
                    r
                } else {
                    std::env::current_dir()
                        .map(|cwd| cwd.join(p).to_string_lossy().to_string())
                        .unwrap_or(r)
                }
            })
            .collect();
        Self {
            redaction: RedactionEngine::new(&redaction_patterns, preview_max_bytes),
            watch_roots: resolved_roots,
        }
    }

    fn check_path(&self, path: &str) -> Result<()> {
        if config_auth::policy::is_path_denied(path) {
            anyhow::bail!("path denied by security policy: {}", path);
        }
        let resolved_query = {
            let p = std::path::Path::new(path);
            if p.is_absolute() {
                path.to_string()
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(p).to_string_lossy().to_string())
                    .unwrap_or_else(|_| path.to_string())
            }
        };
        let watch_root_refs: Vec<&str> = self.watch_roots.iter().map(|s| s.as_str()).collect();
        if !config_auth::policy::is_path_allowed(&resolved_query, &watch_root_refs) {
            anyhow::bail!("path not in watch roots: {}", path);
        }
        Ok(())
    }

    pub async fn stat(&self, path: &str) -> Result<FileStatResponse> {
        self.check_path(path)?;

        let p = Path::new(path);
        if !p.exists() {
            return Ok(FileStatResponse {
                path: path.to_string(),
                exists: false,
                size_bytes: None,
                modified_at: None,
                permissions: None,
                content_hash: None,
                is_yaml: false,
            });
        }

        let metadata = std::fs::metadata(p)
            .with_context(|| format!("failed to read metadata for: {}", path))?;

        let modified_at = metadata.modified().ok().map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.to_rfc3339()
        });

        let content_hash = if metadata.is_file() {
            match camino::Utf8PathBuf::from_path_buf(p.to_path_buf()) {
                Ok(utf8_path) => config_snapshot::hash::compute_blake3_file(&utf8_path).await.ok(),
                Err(_) => None,
            }
        } else {
            None
        };

        let is_yaml = config_shared::paths::is_yaml_file(camino::Utf8Path::new(path));

        Ok(FileStatResponse {
            path: path.to_string(),
            exists: true,
            size_bytes: Some(metadata.len()),
            modified_at,
            permissions: None,
            content_hash,
            is_yaml,
        })
    }

    pub fn preview(&self, path: &str) -> Result<FilePreviewResponse> {
        self.check_path(path)?;

        let p = Path::new(path);
        if !p.exists() {
            return Ok(FilePreviewResponse {
                path: path.to_string(),
                exists: false,
                content: None,
                truncated: false,
                redacted_keys: Vec::new(),
            });
        }

        let raw_content = std::fs::read_to_string(p)
            .with_context(|| format!("failed to read file: {}", path))?;

        let redacted = self.redaction.redact_yaml(&raw_content);
        let truncated = redacted.len() > self.redaction.max_preview_bytes;
        let preview = self.redaction.truncate(&redacted).to_string();

        let redacted_keys = find_redacted_keys(&raw_content, &self.redaction);

        Ok(FilePreviewResponse {
            path: path.to_string(),
            exists: true,
            content: Some(preview),
            truncated,
            redacted_keys,
        })
    }

    /// Read a chunk of raw file content (no redaction, no truncation).
    /// Returns the content base64-encoded for safe JSON transport over WebSocket.
    /// `offset` is the byte offset to start reading from. `limit` is the max bytes to read.
    /// If limit is None, reads up to DEFAULT_CHUNK_BYTES.
    pub fn content(&self, path: &str, offset: Option<u64>, limit: Option<u64>) -> Result<FileContentResponse> {
        self.check_path(path)?;

        let p = Path::new(path);
        if !p.exists() {
            return Ok(FileContentResponse {
                path: path.to_string(),
                exists: false,
                size_bytes: 0,
                content_b64: None,
                offset: 0,
                chunk_length: 0,
                last_chunk: true,
                content_hash: None,
            });
        }

        let metadata = std::fs::metadata(p)
            .with_context(|| format!("failed to read metadata for: {}", path))?;

        let file_size = metadata.len();
        if file_size > MAX_CONTENT_BYTES {
            anyhow::bail!(
                "file too large ({} bytes, max {} bytes)",
                file_size,
                MAX_CONTENT_BYTES
            );
        }

        let offset = offset.unwrap_or(0);
        if offset > file_size {
            anyhow::bail!("offset {} exceeds file size {}", offset, file_size);
        }

        let max_read = limit.unwrap_or(DEFAULT_CHUNK_BYTES);
        let remaining = file_size - offset;
        let read_len = remaining.min(max_read) as usize;

        let mut file = std::fs::File::open(p)
            .with_context(|| format!("failed to open file: {}", path))?;
        std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(offset))
            .with_context(|| format!("failed to seek to offset {}: {}", offset, path))?;

        let mut buf = vec![0u8; read_len];
        let bytes_read = std::io::Read::read(&mut file, &mut buf)
            .with_context(|| format!("failed to read file: {}", path))?;
        buf.truncate(bytes_read);

        let content_b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
        let last_chunk = offset + bytes_read as u64 >= file_size;

        Ok(FileContentResponse {
            path: path.to_string(),
            exists: true,
            size_bytes: file_size,
            content_b64: Some(content_b64),
            offset,
            chunk_length: bytes_read as u64,
            last_chunk,
            content_hash: None,
        })
    }
}

fn find_redacted_keys(content: &str, engine: &RedactionEngine) -> Vec<String> {
    let mut keys = Vec::new();
    for line in content.lines() {
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim();
            if engine.key_patterns.iter().any(|p| p.is_match(key)) {
                keys.push(key.to_string());
            }
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_query_path_matches_resolved_watch_root() {
        // Simulate what happens on Windows: watch root is relative (e.g. "./fixtures/yaml")
        // but query arrives as an absolute path. After resolution in QueryHandler::new,
        // the watch root becomes absolute and Path::starts_with works.
        let resolved_root = std::env::current_dir().unwrap().join("fixtures").join("yaml");
        let handler = QueryHandler::new(
            vec!["./fixtures/yaml".to_string()],
            vec![],
            4096,
        );

        // The watch root should now be absolute
        assert!(
            std::path::Path::new(&handler.watch_roots[0]).is_absolute(),
            "watch root should be resolved to absolute path"
        );

        // An absolute path under that root should pass check_path
        let absolute_file = resolved_root.join("sample.yaml");
        let absolute_file_str = absolute_file.to_string_lossy();
        let result = handler.content(&absolute_file_str, None, None);

        // File may not exist in test, but check_path should NOT reject it for being outside roots
        if let Err(e) = result {
            let msg = e.to_string();
            assert!(
                !msg.contains("not in watch roots"),
                "absolute path was rejected as outside watch roots: {}",
                msg
            );
        }
    }
}