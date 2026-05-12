use std::sync::OnceLock;

/// Cached control-plane base URL, read once from the
/// `<meta name="api-base-url">` tag in `index.html`.
static API_BASE_URL: OnceLock<String> = OnceLock::new();

/// Read the control-plane base URL from the `<meta name="api-base-url">`
/// tag. Panics on first access if the tag is missing or empty — fail-loud
/// so a deploy-time substitution miss is caught immediately, not silently
/// routed to the wrong host.
pub fn api_base_url() -> String {
    API_BASE_URL
        .get_or_init(|| {
            let document = web_sys::window()
                .and_then(|w| w.document())
                .expect("no document available");
            let meta = document
                .query_selector("meta[name=\"api-base-url\"]")
                .ok()
                .flatten()
                .expect("required <meta name=\"api-base-url\"> tag missing from index.html");
            let content = meta
                .get_attribute("content")
                .filter(|s| !s.is_empty())
                .expect("<meta name=\"api-base-url\"> has empty content attribute");
            if !content.starts_with("http://") && !content.starts_with("https://") {
                panic!(
                    "api-base-url must start with http:// or https://: got {}",
                    content
                );
            }
            content.trim_end_matches('/').to_string()
        })
        .clone()
}

/// Build an HTTP(S) URL for an API endpoint.
/// `path` should start with `/` (e.g. `/v1/hosts`).
pub fn api_url(path: &str) -> String {
    format!("{}{}", api_base_url(), path)
}

/// Build a WebSocket URL for the streaming endpoint, deriving the scheme
/// from the configured API base URL (`http` → `ws`, `https` → `wss`).
pub fn ws_url(path: &str, query: &str) -> String {
    let base = api_base_url();
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{}", rest)
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{}", rest)
    } else {
        panic!(
            "api-base-url must start with http:// or https://: got {}",
            base
        );
    };
    if query.is_empty() {
        format!("{}{}", ws_base, path)
    } else {
        format!("{}{}?{}", ws_base, path, query)
    }
}
