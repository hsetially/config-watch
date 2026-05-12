use std::num::NonZeroUsize;
use std::sync::Mutex;

use config_diff::difftastic::{DiffEngine, DiffFormat, DiffOutput};
use config_diff::DiffConfig;
use lru::LruCache;
use uuid::Uuid;

/// Bound on the in-memory render cache. With ~10 hosts and humans clicking on
/// changes, the working set is small; a thousand entries is comfortable.
const RENDER_CACHE_CAPACITY: usize = 1024;

/// Owns the single difftastic-backed `DiffEngine` and an LRU of rendered diffs
/// keyed by `(event_id, format_label)`. All construction-time validation
/// (difftastic availability) happens once at startup, not per request.
pub struct DiffService {
    engine: DiffEngine,
    cache: Mutex<LruCache<CacheKey, CachedRender>>,
    default_format_label: String,
}

#[derive(Hash, Eq, PartialEq, Clone)]
struct CacheKey {
    event_id: Uuid,
    format_label: String,
}

#[derive(Clone)]
pub struct CachedRender {
    pub render: String,
    pub added: u64,
    pub removed: u64,
    pub format_label: String,
}

impl DiffService {
    /// Build the service from the control-plane `[diff]` config.
    ///
    /// Logs at `error!` (not `warn!`) if difftastic is missing — without it
    /// the engine silently falls back to a unified line diff that ignores
    /// `format`, which previously masked real misconfiguration. Operators
    /// must see this in the startup log.
    pub fn new(config: DiffConfig) -> Self {
        let label = format_label(&config);
        let engine = DiffEngine::with_config(config);
        if !engine.is_difftastic_available() {
            tracing::error!(
                "difftastic not found on the control plane; non-unified diff formats will not work. \
                 Install difftastic on this host (e.g. `cargo install difftastic` or the prebuilt \
                 release binary) and restart."
            );
        } else {
            tracing::info!(format = %label, "diff service initialized");
        }
        Self {
            engine,
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(RENDER_CACHE_CAPACITY).unwrap(),
            )),
            default_format_label: label,
        }
    }

    pub fn default_format_label(&self) -> &str {
        &self.default_format_label
    }

    /// Look up a previously rendered diff. Returns `None` on miss.
    pub fn cache_get(&self, event_id: Uuid, format_label: &str) -> Option<CachedRender> {
        let key = CacheKey {
            event_id,
            format_label: format_label.to_string(),
        };
        let mut cache = self.cache.lock().expect("diff cache mutex poisoned");
        cache.get(&key).cloned()
    }

    pub fn cache_put(&self, event_id: Uuid, format_label: &str, render: CachedRender) {
        let key = CacheKey {
            event_id,
            format_label: format_label.to_string(),
        };
        let mut cache = self.cache.lock().expect("diff cache mutex poisoned");
        cache.put(key, render);
    }

    /// Run difftastic on two byte slices using the engine's configured format.
    /// `path` is used by difftastic to pick the language parser.
    pub async fn render(
        &self,
        previous: &str,
        current: &str,
        path: &camino::Utf8Path,
    ) -> anyhow::Result<DiffOutput> {
        self.engine.compute_diff(previous, current, path).await
    }

    /// Render using an explicit format override (from a `?format=` query param).
    pub async fn render_with_format(
        &self,
        previous: &str,
        current: &str,
        path: &camino::Utf8Path,
        format: DiffFormat,
    ) -> anyhow::Result<DiffOutput> {
        self.engine
            .compute_diff_with_format(previous, current, path, format)
            .await
    }

    /// Parse a format label string (from a `?format=` query param) into a
    /// `DiffFormat`. Returns `None` for unrecognised / empty labels so the
    /// caller can fall back to the configured default.
    pub fn parse_format_label(label: &str) -> Option<DiffFormat> {
        match label {
            "unified" => Some(DiffFormat::Unified),
            "context" => Some(DiffFormat::Context),
            "full_file" => Some(DiffFormat::FullFile),
            "side_by_side" => Some(DiffFormat::SideBySide),
            "raw" => Some(DiffFormat::Raw),
            _ => None,
        }
    }
}

/// Stable string label for a `DiffConfig.format`, suitable for cache keys and
/// logs. Matches the labels the dashboard already uses in `diff_viewer.rs`.
pub fn format_label(config: &DiffConfig) -> String {
    format_label_for(&config.format).to_string()
}

/// Stable string label for a `DiffFormat` value. Returns the same labels the
/// dashboard uses in `diff_viewer.rs`.
pub fn format_label_for(format: &DiffFormat) -> &'static str {
    match format {
        DiffFormat::Unified => "unified",
        DiffFormat::Context => "context",
        DiffFormat::FullFile => "full_file",
        DiffFormat::SideBySide => "side_by_side",
        DiffFormat::Raw => "raw",
    }
}
