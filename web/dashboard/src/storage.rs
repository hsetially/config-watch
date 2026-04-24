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
