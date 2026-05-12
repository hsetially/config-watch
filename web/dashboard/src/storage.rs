use crate::models::{RealtimeMessage, WorkflowDefaults};

const STORAGE_KEY_PREFIX: &str = "config_watch_stream_";
const DEFAULTS_KEY_PREFIX: &str = "config_watch_defaults_";
const MAX_STORED_EVENTS: usize = 200;

/// Build a localStorage key from the server URL and optional host filter.
pub fn storage_key(server: &str, host_id: Option<&str>) -> String {
    match host_id {
        Some(hid) => format!("{}{}_{}", STORAGE_KEY_PREFIX, server, hid),
        None => format!("{}{}", STORAGE_KEY_PREFIX, server),
    }
}

/// Save stream events to localStorage, keyed by server+host.
pub fn save_events(key: &str, events: &[RealtimeMessage]) {
    let trimmed = if events.len() > MAX_STORED_EVENTS {
        &events[..MAX_STORED_EVENTS]
    } else {
        events
    };
    match serde_json::to_string(trimmed) {
        Ok(json) => {
            let window = web_sys::window().expect("no window");
            if let Ok(Some(storage)) = window.local_storage() {
                if storage.set_item(key, &json).is_err() {
                    gloo::console::warn!("Failed to write localStorage for key:", key);
                }
            }
        }
        Err(e) => {
            gloo::console::warn!("Failed to serialize events:", &e.to_string());
        }
    }
}

/// Load stream events from localStorage. Returns an empty vec on failure.
pub fn load_events(key: &str) -> Vec<RealtimeMessage> {
    let window = web_sys::window().expect("no window");
    let storage = match window.local_storage() {
        Ok(Some(s)) => s,
        _ => return Vec::new(),
    };
    match storage.get_item(key) {
        Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Remove stored events for a given key.
#[allow(dead_code)]
pub fn clear_events(key: &str) {
    let window = web_sys::window().expect("no window");
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = storage.remove_item(key);
    }
}

/// Remove all stream event entries from localStorage.
pub fn clear_all_stream_events() {
    let window = web_sys::window().expect("no window");
    if let Ok(Some(storage)) = window.local_storage() {
        let keys_to_remove: Vec<String> = (0..storage.length().unwrap_or(0))
            .filter_map(|i| storage.key(i).ok().flatten())
            .filter(|k| k.starts_with(STORAGE_KEY_PREFIX))
            .collect();
        for key in keys_to_remove {
            let _ = storage.remove_item(&key);
        }
    }
}

/// Build a localStorage key for workflow defaults.
pub fn defaults_key(server: &str) -> String {
    format!("{}{}", DEFAULTS_KEY_PREFIX, server)
}

/// Save workflow form defaults to localStorage.
pub fn save_workflow_defaults(key: &str, defaults: &WorkflowDefaults) {
    match serde_json::to_string(defaults) {
        Ok(json) => {
            let window = web_sys::window().expect("no window");
            if let Ok(Some(storage)) = window.local_storage() {
                if storage.set_item(key, &json).is_err() {
                    gloo::console::warn!("Failed to write localStorage for defaults key:", key);
                }
            }
        }
        Err(e) => {
            gloo::console::warn!("Failed to serialize workflow defaults:", &e.to_string());
        }
    }
}

/// Load workflow form defaults from localStorage.
pub fn load_workflow_defaults(key: &str) -> Option<WorkflowDefaults> {
    let window = web_sys::window().expect("no window");
    let storage = match window.local_storage() {
        Ok(Some(s)) => s,
        _ => return None,
    };
    match storage.get_item(key) {
        Ok(Some(json)) => serde_json::from_str(&json).ok(),
        _ => None,
    }
}

const GITHUB_TOKEN_KEY: &str = "config_watch_github_token";

const AUTH_DATA_KEY: &str = "config_watch_auth_data";

/// CSRF cookie name — must match the name set by the auth proxy on sign-in.
const CSRF_COOKIE_NAME: &str = "config_watch_csrf";

/// Auth data persisted in localStorage (user identity only, no session token).
/// The session token is stored in an HttpOnly cookie set by the server.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthData {
    pub user_id: String,
    pub email: Option<String>,
}

pub fn save_auth_data(data: &AuthData) {
    match serde_json::to_string(data) {
        Ok(json) => {
            let window = web_sys::window().expect("no window");
            if let Ok(Some(storage)) = window.local_storage() {
                let _ = storage.set_item(AUTH_DATA_KEY, &json);
            }
        }
        Err(e) => {
            gloo::console::warn!("Failed to serialize auth data:", &e.to_string());
        }
    }
}

pub fn load_auth_data() -> Option<AuthData> {
    let window = web_sys::window().expect("no window");
    let storage = match window.local_storage() {
        Ok(Some(s)) => s,
        _ => return None,
    };
    match storage.get_item(AUTH_DATA_KEY) {
        Ok(Some(json)) => serde_json::from_str(&json).ok(),
        _ => None,
    }
}

pub fn clear_auth_data() {
    let window = web_sys::window().expect("no window");
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = storage.remove_item(AUTH_DATA_KEY);
    }
}

/// Read the CSRF token from the config_watch_csrf cookie.
/// Returns None if the cookie is not present.
pub fn load_csrf_token() -> Option<String> {
    let window = web_sys::window().expect("no window");
    let cookie_str = js_sys::Reflect::get(&window, &wasm_bindgen::JsValue::from_str("document"))
        .ok()
        .and_then(|doc| js_sys::Reflect::get(&doc, &wasm_bindgen::JsValue::from_str("cookie")).ok())
        .and_then(|v| v.as_string());
    let cookie_str = cookie_str?;
    cookie_str.split(';').map(|s| s.trim()).find_map(|part| {
        let prefix = format!("{}=", CSRF_COOKIE_NAME);
        part.strip_prefix(&prefix).map(|v| v.to_string())
    })
}

/// Clear the CSRF cookie by setting it to an expired value.
pub fn clear_csrf_cookie() {
    let _ = js_sys::eval(
        "document.cookie = 'config_watch_csrf=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/'",
    );
}

pub fn save_github_token(token: &str) {
    let window = web_sys::window().expect("no window");
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = storage.set_item(GITHUB_TOKEN_KEY, token);
    }
}

pub fn load_github_token() -> String {
    let window = web_sys::window().expect("no window");
    let storage = match window.local_storage() {
        Ok(Some(s)) => s,
        _ => return String::new(),
    };
    match storage.get_item(GITHUB_TOKEN_KEY) {
        Ok(Some(t)) => t,
        _ => String::new(),
    }
}

const LAZY_DIFF_FLAG_KEY: &str = "lazy_diff_endpoint";

/// Feature flag for the lazy server-side diff endpoint introduced in the
/// re-architecture. Defaults to `false` to preserve the legacy
/// `fetch_event_detail` path. To opt in from a browser:
///
/// ```js
/// localStorage.setItem('lazy_diff_endpoint', '1')
/// ```
///
/// Removed once the rollout is validated and the legacy path is deleted.
pub fn lazy_diff_endpoint_enabled() -> bool {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return false,
    };
    let storage = match window.local_storage() {
        Ok(Some(s)) => s,
        _ => return false,
    };
    matches!(
        storage.get_item(LAZY_DIFF_FLAG_KEY),
        Ok(Some(ref v)) if v == "1" || v == "true"
    )
}

// --- Diff render cache (survives mode switches and page reloads) ---

const DIFF_CACHE_KEY: &str = "config_watch_diff_cache";

/// Cache of rendered diffs keyed by `"event_id:format"`.
type DiffCacheMap = std::collections::HashMap<String, String>;

fn load_diff_cache() -> DiffCacheMap {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return DiffCacheMap::new(),
    };
    let storage = match window.local_storage() {
        Ok(Some(s)) => s,
        _ => return DiffCacheMap::new(),
    };
    match storage.get_item(DIFF_CACHE_KEY) {
        Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_default(),
        _ => DiffCacheMap::new(),
    }
}

fn save_diff_cache(cache: &DiffCacheMap) {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return,
    };
    let storage = match window.local_storage() {
        Ok(Some(s)) => s,
        _ => return,
    };
    if let Ok(json) = serde_json::to_string(cache) {
        let _ = storage.set_item(DIFF_CACHE_KEY, &json);
    }
}

fn diff_cache_key(event_id: &uuid::Uuid, format: &str) -> String {
    format!("{}:{}", event_id, format)
}

/// Store a rendered diff in localStorage so it survives mode switches.
pub fn cache_diff_render(event_id: &uuid::Uuid, format: &str, render: &str) {
    let mut cache = load_diff_cache();
    cache.insert(diff_cache_key(event_id, format), render.to_string());
    save_diff_cache(&cache);
}

/// Look up a cached diff render. Returns `None` on miss.
pub fn get_cached_diff(event_id: &uuid::Uuid, format: &str) -> Option<String> {
    let cache = load_diff_cache();
    cache.get(&diff_cache_key(event_id, format)).cloned()
}
