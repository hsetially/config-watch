use serde::Deserialize;
use wasm_bindgen_futures::spawn_local;
use yew::Callback;

use crate::models::{
    ChangeEventRow, ChangesPage, FileContentResponse, GitHubFileContentResponse, HostInfo,
    WatchRootInfo, WorkflowCreateRequest, WorkflowCreateResponse, WorkflowStatusResponse,
};

pub fn fetch_hosts(base_url: &str, on_result: Callback<Vec<HostInfo>>) {
    let url = format!("https://{}/v1/hosts", base_url);
    let on_result = on_result.clone();

    spawn_local(async move {
        match gloo_net::http::Request::get(&url).send().await {
            Ok(resp) => {
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
                    gloo::console::warn!(
                        "Hosts request failed with status:",
                        &resp.status().to_string()
                    );
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

pub fn fetch_watch_roots(base_url: &str, host_id: &str, on_result: Callback<Vec<WatchRootInfo>>) {
    let url = format!("https://{}/v1/hosts/{}/roots", base_url, host_id);
    let on_result = on_result.clone();

    spawn_local(async move {
        match gloo_net::http::Request::get(&url).send().await {
            Ok(resp) => {
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
                        &resp.status().to_string()
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

pub fn fetch_changes(base_url: &str, query: &str, on_result: Callback<ChangesPage>) {
    let url = format!("https://{}/v1/changes{}", base_url, query);
    let on_result = on_result.clone();

    spawn_local(async move {
        match gloo_net::http::Request::get(&url).send().await {
            Ok(resp) => {
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
                        &resp.status().to_string()
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

pub fn fetch_event_detail(
    base_url: &str,
    event_id: &str,
    on_result: Callback<Option<ChangeEventRow>>,
) {
    let url = format!("https://{}/v1/changes/{}", base_url, event_id);
    let on_result = on_result.clone();

    spawn_local(async move {
        match gloo_net::http::Request::get(&url).send().await {
            Ok(resp) => {
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

pub fn create_workflow(
    base_url: &str,
    body: &WorkflowCreateRequest,
    on_result: Callback<Option<WorkflowCreateResponse>>,
) {
    let url = format!("https://{}/v1/workflows", base_url);
    let on_result = on_result.clone();
    let json_body = serde_json::to_string(body).unwrap_or_default();

    spawn_local(async move {
        match gloo_net::http::Request::post(&url)
            .header("Content-Type", "application/json")
            .body(json_body)
        {
            Ok(req) => match req.send().await {
                Ok(resp) => {
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

pub fn fetch_file_content(
    base_url: &str,
    host_id: &str,
    path: &str,
    offset: Option<u64>,
    limit: Option<u64>,
    on_result: Callback<Option<FileContentResponse>>,
) {
    let url = format!("https://{}/v1/file/content", base_url);
    let on_result = on_result.clone();
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
        match gloo_net::http::Request::post(&url)
            .header("Content-Type", "application/json")
            .body(json_body)
        {
            Ok(req) => match req.send().await {
                Ok(resp) => {
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
    base_url: &str,
    workflow_id: &str,
    on_result: Callback<Option<WorkflowStatusResponse>>,
) {
    let url = format!("https://{}/v1/workflows/{}", base_url, workflow_id);
    let on_result = on_result.clone();

    spawn_local(async move {
        match gloo_net::http::Request::get(&url).send().await {
            Ok(resp) => {
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
    base_url: &str,
    github_url: &str,
    github_token: Option<&str>,
    on_result: Callback<Option<GitHubFileContentResponse>>,
) {
    let url = format!("https://{}/v1/github/file-content", base_url);
    let on_result = on_result.clone();
    let mut body = serde_json::json!({ "url": github_url });
    if let Some(t) = github_token {
        body["github_token"] = serde_json::Value::String(t.to_string());
    }
    let json_body = serde_json::to_string(&body).unwrap_or_default();

    spawn_local(async move {
        match gloo_net::http::Request::post(&url)
            .header("Content-Type", "application/json")
            .body(json_body)
        {
            Ok(req) => match req.send().await {
                Ok(resp) => {
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
