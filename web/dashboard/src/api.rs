use serde::Deserialize;
use wasm_bindgen_futures::spawn_local;
use yew::Callback;

use crate::models::{
    ChangeEventRow, ChangesPage, FileContentResponse, GitHubFileContentResponse, HostInfo,
    WatchRootInfo, WorkflowCreateRequest, WorkflowCreateResponse, WorkflowStatusResponse,
};
use crate::url;

/// Check if a response is 401 Unauthorized or 403 Forbidden and emit logout if so.
fn check_unauthorized(status: u16, on_unauthorized: &Option<Callback<()>>) {
    if status == 401 || status == 403 {
        if let Some(cb) = on_unauthorized {
            cb.emit(());
        }
    }
}

// --- GET endpoints: session cookie sent automatically, no auth header needed ---

pub fn fetch_hosts(
    _base_url: &str,
    on_result: Callback<Vec<HostInfo>>,
    on_unauthorized: Option<Callback<()>>,
) {
    let url = url::api_url("/v1/hosts");
    let on_result = on_result.clone();
    let on_unauthorized = on_unauthorized.clone();

    spawn_local(async move {
        let req =
            gloo_net::http::Request::get(&url).credentials(web_sys::RequestCredentials::Include);
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                check_unauthorized(status, &on_unauthorized);
                if resp.ok() {
                    match resp.json::<HostsResponse>().await {
                        Ok(body) => {
                            on_result.emit(body.hosts);
                        }
                        Err(e) => {
                            gloo::console::warn!("Failed to parse hosts response:", &e.to_string());
                            on_result.emit(Vec::new());
                        }
                    }
                } else {
                    gloo::console::warn!("Hosts request failed with status:", &status.to_string());
                    on_result.emit(Vec::new());
                }
            }
            Err(e) => {
                gloo::console::warn!("Hosts fetch error:", &e.to_string());
                on_result.emit(Vec::new());
            }
        }
    });
}

pub fn fetch_watch_roots(
    _base_url: &str,
    host_id: &str,
    on_result: Callback<Vec<WatchRootInfo>>,
    on_unauthorized: Option<Callback<()>>,
) {
    let url = url::api_url(&format!("/v1/hosts/{}/roots", host_id));
    let on_result = on_result.clone();
    let on_unauthorized = on_unauthorized.clone();

    spawn_local(async move {
        let req =
            gloo_net::http::Request::get(&url).credentials(web_sys::RequestCredentials::Include);
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                check_unauthorized(status, &on_unauthorized);
                if resp.ok() {
                    match resp.json::<WatchRootsResponse>().await {
                        Ok(body) => {
                            on_result.emit(body.roots);
                        }
                        Err(e) => {
                            gloo::console::warn!(
                                "Failed to parse watch roots response:",
                                &e.to_string()
                            );
                            on_result.emit(Vec::new());
                        }
                    }
                } else {
                    gloo::console::warn!(
                        "Watch roots request failed with status:",
                        &status.to_string()
                    );
                    on_result.emit(Vec::new());
                }
            }
            Err(e) => {
                gloo::console::warn!("Watch roots fetch error:", &e.to_string());
                on_result.emit(Vec::new());
            }
        }
    });
}

pub fn fetch_changes(
    _base_url: &str,
    query: &str,
    on_result: Callback<ChangesPage>,
    on_unauthorized: Option<Callback<()>>,
) {
    let url = url::api_url(&format!("/v1/changes{}", query));
    let on_result = on_result.clone();
    let on_unauthorized = on_unauthorized.clone();

    spawn_local(async move {
        let req =
            gloo_net::http::Request::get(&url).credentials(web_sys::RequestCredentials::Include);
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                check_unauthorized(status, &on_unauthorized);
                if resp.ok() {
                    match resp.json::<ChangesResponse>().await {
                        Ok(body) => {
                            on_result.emit(ChangesPage {
                                changes: body.changes,
                                total: body.total,
                            });
                        }
                        Err(e) => {
                            gloo::console::warn!(
                                "Failed to parse changes response:",
                                &e.to_string()
                            );
                            on_result.emit(ChangesPage {
                                changes: Vec::new(),
                                total: 0,
                            });
                        }
                    }
                } else {
                    gloo::console::warn!(
                        "Changes request failed with status:",
                        &status.to_string()
                    );
                    on_result.emit(ChangesPage {
                        changes: Vec::new(),
                        total: 0,
                    });
                }
            }
            Err(e) => {
                gloo::console::warn!("Changes fetch error:", &e.to_string());
                on_result.emit(ChangesPage {
                    changes: Vec::new(),
                    total: 0,
                });
            }
        }
    });
}

/// Result of the lazy server-side diff fetch.
///
/// The control plane endpoint may legitimately return success with the
/// previous snapshot evicted by retention — callers should still render the
/// payload (it's a "current only" view) but show a subtle warning.
#[derive(Debug, Clone)]
pub enum DiffFetch {
    Ok {
        render: String,
        previous_unavailable: bool,
    },
    HostOffline,
    BothEvicted,
    Failed(String),
    Unauthorized,
}

#[derive(Debug, Clone, Deserialize)]
struct DiffEndpointResponse {
    render: String,
    #[serde(default)]
    previous_unavailable: bool,
}

/// Hit the lazy `/v1/changes/{id}/diff` endpoint.
pub fn fetch_change_diff(
    _base_url: &str,
    event_id: &str,
    format: Option<&str>,
    on_result: Callback<DiffFetch>,
) {
    let url = match format {
        Some(f) => url::api_url(&format!("/v1/changes/{}/diff?format={}", event_id, f)),
        None => url::api_url(&format!("/v1/changes/{}/diff", event_id)),
    };
    let on_result = on_result.clone();
    spawn_local(async move {
        let req =
            gloo_net::http::Request::get(&url).credentials(web_sys::RequestCredentials::Include);
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                if status == 401 || status == 403 {
                    on_result.emit(DiffFetch::Unauthorized);
                } else if resp.ok() {
                    match resp.json::<DiffEndpointResponse>().await {
                        Ok(body) => on_result.emit(DiffFetch::Ok {
                            render: body.render,
                            previous_unavailable: body.previous_unavailable,
                        }),
                        Err(e) => on_result.emit(DiffFetch::Failed(e.to_string())),
                    }
                } else if status == 503 {
                    on_result.emit(DiffFetch::HostOffline);
                } else if status == 410 {
                    on_result.emit(DiffFetch::BothEvicted);
                } else {
                    on_result.emit(DiffFetch::Failed(format!("HTTP {}", status)));
                }
            }
            Err(e) => on_result.emit(DiffFetch::Failed(e.to_string())),
        }
    });
}

pub fn fetch_event_detail(
    _base_url: &str,
    event_id: &str,
    on_result: Callback<Option<ChangeEventRow>>,
    on_unauthorized: Option<Callback<()>>,
) {
    let url = url::api_url(&format!("/v1/changes/{}", event_id));
    let on_result = on_result.clone();
    let on_unauthorized = on_unauthorized.clone();

    spawn_local(async move {
        let req =
            gloo_net::http::Request::get(&url).credentials(web_sys::RequestCredentials::Include);
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                check_unauthorized(status, &on_unauthorized);
                if resp.ok() {
                    match resp.json::<EventDetailResponse>().await {
                        Ok(body) => {
                            on_result.emit(Some(body.event));
                        }
                        Err(e) => {
                            gloo::console::warn!("Failed to parse event detail:", &e.to_string());
                            on_result.emit(None);
                        }
                    }
                } else {
                    on_result.emit(None);
                }
            }
            Err(_) => {
                on_result.emit(None);
            }
        }
    });
}

#[derive(Debug, Clone, Deserialize)]
struct HostsResponse {
    hosts: Vec<HostInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct WatchRootsResponse {
    roots: Vec<WatchRootInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct ChangesResponse {
    changes: Vec<ChangeEventRow>,
    #[serde(default)]
    total: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct EventDetailResponse {
    event: ChangeEventRow,
}

// --- Mutating endpoints: require CSRF token in x-csrf-token header ---
// Session cookie is sent automatically by the browser.

pub fn create_workflow(
    _base_url: &str,
    body: &WorkflowCreateRequest,
    csrf_token: Option<String>,
    on_result: Callback<Option<WorkflowCreateResponse>>,
    on_unauthorized: Option<Callback<()>>,
) {
    let url = url::api_url("/v1/workflows");
    let on_result = on_result.clone();
    let on_unauthorized = on_unauthorized.clone();
    let json_body = serde_json::to_string(body).unwrap_or_default();

    spawn_local(async move {
        let mut req = gloo_net::http::Request::post(&url)
            .credentials(web_sys::RequestCredentials::Include)
            .header("Content-Type", "application/json");
        if let Some(ref csrf) = csrf_token {
            req = req.header("x-csrf-token", csrf);
        }
        match req.body(json_body) {
            Ok(req) => match req.send().await {
                Ok(resp) => {
                    check_unauthorized(resp.status(), &on_unauthorized);
                    if resp.status() == 202 {
                        match resp.json::<WorkflowCreateResponse>().await {
                            Ok(body) => on_result.emit(Some(body)),
                            Err(_) => on_result.emit(None),
                        }
                    } else {
                        on_result.emit(None);
                    }
                }
                Err(_) => on_result.emit(None),
            },
            Err(_) => on_result.emit(None),
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub fn fetch_file_content(
    _base_url: &str,
    host_id: &str,
    path: &str,
    offset: Option<u64>,
    limit: Option<u64>,
    csrf_token: Option<String>,
    on_result: Callback<Option<FileContentResponse>>,
    on_unauthorized: Option<Callback<()>>,
) {
    let url = url::api_url("/v1/file/content");
    let on_result = on_result.clone();
    let on_unauthorized = on_unauthorized.clone();
    let mut body = serde_json::json!({
        "host_id": host_id,
        "path": path,
    });
    if let Some(off) = offset {
        body["offset"] = serde_json::Value::Number(off.into());
    }
    if let Some(lim) = limit {
        body["limit"] = serde_json::Value::Number(lim.into());
    }
    let json_body = serde_json::to_string(&body).unwrap_or_default();

    spawn_local(async move {
        let mut req = gloo_net::http::Request::post(&url)
            .credentials(web_sys::RequestCredentials::Include)
            .header("Content-Type", "application/json");
        if let Some(ref csrf) = csrf_token {
            req = req.header("x-csrf-token", csrf);
        }
        match req.body(json_body) {
            Ok(req) => match req.send().await {
                Ok(resp) => {
                    check_unauthorized(resp.status(), &on_unauthorized);
                    if resp.ok() {
                        match resp.json::<FileContentResponse>().await {
                            Ok(body) => on_result.emit(Some(body)),
                            Err(e) => {
                                gloo::console::warn!(
                                    "Failed to parse file content response:",
                                    &e.to_string()
                                );
                                on_result.emit(None);
                            }
                        }
                    } else {
                        on_result.emit(None);
                    }
                }
                Err(_) => on_result.emit(None),
            },
            Err(_) => on_result.emit(None),
        }
    });
}

pub fn get_workflow(
    _base_url: &str,
    workflow_id: &str,
    on_result: Callback<Option<WorkflowStatusResponse>>,
    on_unauthorized: Option<Callback<()>>,
) {
    let url = url::api_url(&format!("/v1/workflows/{}", workflow_id));
    let on_result = on_result.clone();
    let on_unauthorized = on_unauthorized.clone();

    spawn_local(async move {
        let req =
            gloo_net::http::Request::get(&url).credentials(web_sys::RequestCredentials::Include);
        match req.send().await {
            Ok(resp) => {
                check_unauthorized(resp.status(), &on_unauthorized);
                if resp.ok() {
                    match resp.json::<WorkflowStatusResponse>().await {
                        Ok(body) => on_result.emit(Some(body)),
                        Err(_) => on_result.emit(None),
                    }
                } else {
                    on_result.emit(None);
                }
            }
            Err(_) => on_result.emit(None),
        }
    });
}

pub fn fetch_github_file_content(
    _base_url: &str,
    github_url: &str,
    github_token: Option<&str>,
    csrf_token: Option<String>,
    on_result: Callback<Option<GitHubFileContentResponse>>,
    on_unauthorized: Option<Callback<()>>,
) {
    let url = url::api_url("/v1/github/file-content");
    let on_result = on_result.clone();
    let on_unauthorized = on_unauthorized.clone();
    let mut body = serde_json::json!({ "url": github_url });
    if let Some(t) = github_token {
        body["github_token"] = serde_json::Value::String(t.to_string());
    }
    let json_body = serde_json::to_string(&body).unwrap_or_default();

    spawn_local(async move {
        let mut req = gloo_net::http::Request::post(&url)
            .credentials(web_sys::RequestCredentials::Include)
            .header("Content-Type", "application/json");
        if let Some(ref csrf) = csrf_token {
            req = req.header("x-csrf-token", csrf);
        }
        match req.body(json_body) {
            Ok(req) => match req.send().await {
                Ok(resp) => {
                    check_unauthorized(resp.status(), &on_unauthorized);
                    if resp.ok() {
                        match resp.json::<GitHubFileContentResponse>().await {
                            Ok(body) => on_result.emit(Some(body)),
                            Err(e) => {
                                gloo::console::warn!(
                                    "Failed to parse github file content:",
                                    &e.to_string()
                                );
                                on_result.emit(None);
                            }
                        }
                    } else {
                        on_result.emit(None);
                    }
                }
                Err(_) => on_result.emit(None),
            },
            Err(_) => on_result.emit(None),
        }
    });
}
