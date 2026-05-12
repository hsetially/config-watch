use std::collections::HashSet;
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use web_sys::{HtmlInputElement, HtmlSelectElement};
use yew::{
    function_component, html, use_effect_with, use_node_ref, Callback, Html, NodeRef, Properties,
    TargetCast, UseStateHandle,
};

use crate::api;
use crate::models::{
    ColumnSource, CompareColumn, CompareResult, DiffLine, DiffLineKind, HostInfo, WordSegment,
};
use crate::storage;

#[derive(Properties, PartialEq)]
pub struct FileCompareProps {
    pub hosts: Rc<Vec<HostInfo>>,
    pub server_url: String,
    pub csrf_token: Option<String>,
    pub on_fetch_hosts: Callback<()>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComparePhase {
    Idle,
    Fetching,
    Done,
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColumnType {
    Agent,
    Github,
}

#[derive(Debug, Clone)]
struct ChangeGroup {
    start: usize,
    end: usize,
    #[allow(dead_code)]
    added: usize,
    #[allow(dead_code)]
    removed: usize,
}

fn default_columns() -> Vec<CompareColumn> {
    vec![
        CompareColumn {
            source: ColumnSource::Agent {
                host_id: String::new(),
                hostname: String::new(),
            },
            label: String::new(),
            file_path: String::new(),
        },
        CompareColumn {
            source: ColumnSource::Agent {
                host_id: String::new(),
                hostname: String::new(),
            },
            label: String::new(),
            file_path: String::new(),
        },
    ]
}

fn load_columns() -> Vec<CompareColumn> {
    let window = web_sys::window().expect("no window");
    let storage = match window.local_storage() {
        Ok(Some(s)) => s,
        _ => return default_columns(),
    };
    match storage.get_item("config_watch_compare_columns") {
        Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_else(|_| default_columns()),
        _ => default_columns(),
    }
}

fn load_column_types() -> Vec<ColumnType> {
    let window = web_sys::window().expect("no window");
    let storage = match window.local_storage() {
        Ok(Some(s)) => s,
        _ => return vec![ColumnType::Agent, ColumnType::Agent],
    };
    match storage.get_item("config_watch_compare_types") {
        Ok(Some(json)) => serde_json::from_str(&json)
            .unwrap_or_else(|_| vec![ColumnType::Agent, ColumnType::Agent]),
        _ => vec![ColumnType::Agent, ColumnType::Agent],
    }
}

fn save_columns(cols: &[CompareColumn]) {
    if let Ok(json) = serde_json::to_string(cols) {
        let window = web_sys::window().expect("no window");
        if let Ok(Some(storage)) = window.local_storage() {
            let _ = storage.set_item("config_watch_compare_columns", &json);
        }
    }
}

fn save_column_types(types: &[ColumnType]) {
    if let Ok(json) = serde_json::to_string(types) {
        let window = web_sys::window().expect("no window");
        if let Ok(Some(storage)) = window.local_storage() {
            let _ = storage.set_item("config_watch_compare_types", &json);
        }
    }
}

#[function_component(FileCompare)]
pub fn file_compare(props: &FileCompareProps) -> Html {
    let columns: UseStateHandle<Vec<CompareColumn>> = yew::use_state(load_columns);
    let column_types: UseStateHandle<Vec<ColumnType>> = yew::use_state(load_column_types);
    let results: UseStateHandle<Vec<Option<CompareResult>>> = yew::use_state(Vec::new);
    let diff_lines_left: UseStateHandle<Vec<DiffLine>> = yew::use_state(Vec::new);
    let diff_lines_right: UseStateHandle<Vec<DiffLine>> = yew::use_state(Vec::new);
    let phase: UseStateHandle<ComparePhase> = yew::use_state(|| ComparePhase::Idle);
    let github_token: UseStateHandle<String> = yew::use_state(storage::load_github_token);
    let diff_format: UseStateHandle<String> = yew::use_state(|| "side_by_side".to_string());
    let change_groups: UseStateHandle<Vec<ChangeGroup>> = yew::use_state(Vec::new);
    let active_change: UseStateHandle<Option<usize>> = yew::use_state(|| None);
    let unified_lines: UseStateHandle<Vec<DiffLine>> = yew::use_state(Vec::new);
    let diff_content_ref: NodeRef = use_node_ref();

    // Persist columns and types to localStorage whenever they change.
    {
        let cols = (*columns).clone();
        use_effect_with(cols, move |cols| {
            save_columns(cols);
        });
    }
    {
        let types = (*column_types).clone();
        use_effect_with(types, move |types| {
            save_column_types(types);
        });
    }

    // Auto-select the first available host for any Agent column that has no host selected yet.
    {
        let columns = columns.clone();
        let column_types = column_types.clone();
        let hosts = props.hosts.clone();
        use_effect_with(hosts, move |hosts| {
            if hosts.is_empty() {
                return;
            }
            let first = &hosts[0];
            let mut cols = (*columns).clone();
            let mut changed = false;
            for (idx, col) in cols.iter_mut().enumerate() {
                let is_agent = (*column_types).get(idx) == Some(&ColumnType::Agent);
                let host_empty = matches!(
                    &col.source,
                    ColumnSource::Agent { host_id, .. } if host_id.is_empty()
                );
                if is_agent && host_empty {
                    col.source = ColumnSource::Agent {
                        host_id: first.host_id.to_string(),
                        hostname: first.hostname.clone(),
                    };
                    col.label = format!("{} ({})", first.hostname, first.environment);
                    changed = true;
                }
            }
            if changed {
                columns.set(cols);
            }
        });
    }

    let on_compare = {
        let server_url = props.server_url.clone();
        let columns = columns.clone();
        let results = results.clone();
        let diff_lines_left = diff_lines_left.clone();
        let diff_lines_right = diff_lines_right.clone();
        let phase = phase.clone();
        let github_token = github_token.clone();
        let unified_lines = unified_lines.clone();
        let change_groups = change_groups.clone();
        let csrf_token = props.csrf_token.clone();
        Callback::from(move |_: ()| {
            let cols = (*columns).clone();

            let active_cols: Vec<_> = cols
                .iter()
                .filter(|c| match &c.source {
                    ColumnSource::Agent { host_id, .. } => {
                        !host_id.is_empty() && !c.file_path.is_empty()
                    }
                    ColumnSource::Github { url } => !url.is_empty(),
                })
                .cloned()
                .collect();

            if active_cols.len() < 2 {
                let msg = if cols.iter().all(|c| {
                    matches!(&c.source,
                        ColumnSource::Agent { host_id, .. } if host_id.is_empty()
                    )
                }) {
                    "Select at least one agent (host) in each column. If the list is empty, click Reload hosts."
                } else if cols.iter().all(|c| c.file_path.is_empty()) {
                    "Enter a file path for each agent column."
                } else {
                    "Configure at least 2 columns with valid sources and file paths."
                };
                phase.set(ComparePhase::Error(msg.into()));
                return;
            }

            phase.set(ComparePhase::Fetching);
            results.set(Vec::new());
            diff_lines_left.set(Vec::new());
            diff_lines_right.set(Vec::new());

            let server = server_url.clone();
            let num = active_cols.len();
            let fetched: Rc<std::cell::RefCell<Vec<Option<CompareResult>>>> =
                Rc::new(std::cell::RefCell::new(vec![None; num]));
            let counter: Rc<std::cell::RefCell<usize>> = Rc::new(std::cell::RefCell::new(0));

            for (i, col) in active_cols.iter().enumerate() {
                let source = col.source.clone();
                let label = col.label.clone();
                let server = server.clone();
                let path = col.file_path.clone();
                let fetched = fetched.clone();
                let counter = counter.clone();
                let results_state = results.clone();
                let diff_left = diff_lines_left.clone();
                let diff_right = diff_lines_right.clone();
                let phase = phase.clone();
                let unified = unified_lines.clone();
                let groups = change_groups.clone();
                let num_cols = num;

                match &source {
                    ColumnSource::Agent { host_id, .. } => {
                        let host_id = host_id.clone();
                        let label = label.clone();
                        let on_result = Callback::from(
                            move |resp: Option<crate::models::FileContentResponse>| {
                                let result = match resp {
                                    Some(r) if r.exists => CompareResult {
                                        source_label: label.clone(),
                                        exists: true,
                                        content: r.decoded_content(),
                                        size_bytes: r.size_bytes,
                                        content_hash: r.content_hash,
                                        error: None,
                                    },
                                    Some(_) => CompareResult {
                                        source_label: label.clone(),
                                        exists: false,
                                        content: None,
                                        size_bytes: 0,
                                        content_hash: None,
                                        error: Some("File not found".into()),
                                    },
                                    None => CompareResult {
                                        source_label: label.clone(),
                                        exists: false,
                                        content: None,
                                        size_bytes: 0,
                                        content_hash: None,
                                        error: Some("Failed to fetch".into()),
                                    },
                                };
                                collect_result(
                                    i,
                                    result,
                                    fetched.clone(),
                                    counter.clone(),
                                    num_cols,
                                    results_state.clone(),
                                    diff_left.clone(),
                                    diff_right.clone(),
                                    phase.clone(),
                                    unified.clone(),
                                    groups.clone(),
                                );
                            },
                        );
                        api::fetch_file_content(&server, &host_id, &path, None, None, csrf_token.clone(), on_result, None);
                    }
                    ColumnSource::Github { url } => {
                        let label = label.clone();
                        let token = (*github_token).clone();
                        let on_result = Callback::from(
                            move |resp: Option<crate::models::GitHubFileContentResponse>| {
                                let result = match resp {
                                    Some(r) => CompareResult {
                                        source_label: label.clone(),
                                        exists: true,
                                        content: Some(r.content.clone()),
                                        size_bytes: r.size_bytes,
                                        content_hash: r.sha.clone(),
                                        error: None,
                                    },
                                    None => CompareResult {
                                        source_label: label.clone(),
                                        exists: false,
                                        content: None,
                                        size_bytes: 0,
                                        content_hash: None,
                                        error: Some("GitHub fetch failed".into()),
                                    },
                                };
                                collect_result(
                                    i,
                                    result,
                                    fetched.clone(),
                                    counter.clone(),
                                    num_cols,
                                    results_state.clone(),
                                    diff_left.clone(),
                                    diff_right.clone(),
                                    phase.clone(),
                                    unified.clone(),
                                    groups.clone(),
                                );
                            },
                        );
                        let token_ref = if token.is_empty() {
                            None
                        } else {
                            Some(token.as_str())
                        };
                        api::fetch_github_file_content(&server, url, token_ref, csrf_token.clone(), on_result, None);
                    }
                }
            }
        })
    };

    let add_column = {
        let columns = columns.clone();
        let column_types = column_types.clone();
        Callback::from(move |_: ()| {
            let mut cols = (*columns).clone();
            let mut types = (*column_types).clone();
            if cols.len() < 4 {
                cols.push(CompareColumn {
                    source: ColumnSource::Agent {
                        host_id: String::new(),
                        hostname: String::new(),
                    },
                    label: String::new(),
                    file_path: String::new(),
                });
                types.push(ColumnType::Agent);
                columns.set(cols);
                column_types.set(types);
            }
        })
    };

    let add_github_column = {
        let columns = columns.clone();
        let column_types = column_types.clone();
        Callback::from(move |_: ()| {
            let mut cols = (*columns).clone();
            let mut types = (*column_types).clone();
            if cols.len() < 4 {
                cols.push(CompareColumn {
                    source: ColumnSource::Github { url: String::new() },
                    label: "GitHub".to_string(),
                    file_path: String::new(),
                });
                types.push(ColumnType::Github);
                columns.set(cols);
                column_types.set(types);
            }
        })
    };

    let remove_column = {
        let columns = columns.clone();
        let column_types = column_types.clone();
        Callback::from(move |idx: usize| {
            let mut cols = (*columns).clone();
            let mut types = (*column_types).clone();
            if cols.len() > 2 {
                cols.remove(idx);
                types.remove(idx);
                columns.set(cols);
                column_types.set(types);
            }
        })
    };

    let host_options = &*props.hosts;
    let has_hosts = !host_options.is_empty();
    let has_github_column = (*column_types).contains(&ColumnType::Github);

    let current_format = (*diff_format).clone();
    let num_changes = (*change_groups).len();

    let on_format_change = {
        let diff_format = diff_format.clone();
        let unified_lines = unified_lines.clone();
        let change_groups = change_groups.clone();
        let results_state = results.clone();
        let active_change = active_change.clone();
        Callback::from(move |e: yew::Event| {
            let select: HtmlSelectElement = e.target_unchecked_into();
            let val = select.value();
            // When format changes, recompute the unified diff if needed
            if val != "side_by_side" {
                let all = (*results_state).clone();
                let left_content = all
                    .first()
                    .and_then(|r| r.as_ref())
                    .and_then(|r| r.content.clone())
                    .unwrap_or_default();
                let right_content = all
                    .get(1)
                    .and_then(|r| r.as_ref())
                    .and_then(|r| r.content.clone())
                    .unwrap_or_default();
                if !left_content.is_empty() || !right_content.is_empty() {
                    let (uni, groups) = compute_unified(&left_content, &right_content);
                    unified_lines.set(uni);
                    change_groups.set(groups);
                }
            }
            diff_format.set(val);
            active_change.set(None);
        })
    };

    let scroll_to_change = {
        let diff_content_ref = diff_content_ref.clone();
        let active_change = active_change.clone();
        let num_changes = num_changes;
        Callback::from(move |direction: i32| {
            let current = (*active_change).unwrap_or(if direction < 0 { num_changes } else { usize::MAX });
            let next = if direction < 0 {
                if current == 0 { num_changes.saturating_sub(1) } else { current.saturating_sub(1) }
            } else {
                if current >= num_changes.saturating_sub(1) { 0 } else { current + 1 }
            };
            if num_changes == 0 {
                return;
            }
            active_change.set(Some(next));
            // Scroll to the change group element
            if let Some(el) = diff_content_ref.cast::<web_sys::Element>() {
                let id = format!("change-group-{}", next);
                if let Some(target) = el.query_selector(&format!("#{}", id)).ok().flatten() {
                    target.scroll_into_view_with_bool(true);
                }
            }
        })
    };

    let prev_change = {
        let scroll = scroll_to_change.clone();
        Callback::from(move |_: ()| scroll.emit(-1))
    };
    let next_change = {
        let scroll = scroll_to_change.clone();
        Callback::from(move |_: ()| scroll.emit(1))
    };

    html! {
        <div class="compare-panel">
            <div class="compare-panel-header">
                <h2>{"File Comparison"}</h2>
                <div class="compare-header-controls">
                    if matches!(*phase, ComparePhase::Done) && num_changes > 0 {
                        <div class="compare-nav">
                            <button class="compare-nav-btn" onclick={move |_| prev_change.emit(())} title="Previous change">{"<"}</button>
                            <span class="compare-nav-info">
                                { format!("{}/{}", (*active_change).map(|a| a + 1).unwrap_or(1), num_changes) }
                            </span>
                            <button class="compare-nav-btn" onclick={move |_| next_change.emit(())} title="Next change">{">"}</button>
                        </div>
                    }
                    <div class="compare-format-select">
                        <label for="compare-diff-format">{"Format:"}</label>
                        <select
                            id="compare-diff-format"
                            value={current_format.clone()}
                            onchange={on_format_change}
                        >
                            <option value="side_by_side">{"Side by Side"}</option>
                            <option value="unified">{"Unified"}</option>
                            <option value="context">{"Context"}</option>
                            <option value="full_file">{"Full File"}</option>
                        </select>
                    </div>
                </div>
                <button
                    class="link-btn"
                    onclick={{
                        let on_fetch = props.on_fetch_hosts.clone();
                        move |_| on_fetch.emit(())
                    }}
                >
                    {"Reload hosts"}
                </button>
            </div>

            <div class="compare-panel-body">
                if has_github_column {
                    <div class="compare-token-row">
                        <label for="compare-github-token">{"GitHub Token"}</label>
                        <input
                            id="compare-github-token"
                            class="compare-col-path-input"
                            type="password"
                            placeholder="ghp_xxxxxxxxxxxx"
                            value={(*github_token).clone()}
                            oninput={{
                                let github_token = github_token.clone();
                                move |e: yew::InputEvent| {
                                    let input: HtmlInputElement = e.target_unchecked_into();
                                    let val = input.value();
                                    storage::save_github_token(&val);
                                    github_token.set(val);
                                }
                            }}
                        />
                    </div>
                }
                <div class="compare-agents-row">
                    { for (*columns).iter().enumerate().map(|(idx, col)| {
                        let col_type = (*column_types).get(idx).copied().unwrap_or(ColumnType::Agent);

                        let on_toggle_type = {
                            let columns = columns.clone();
                            let column_types = column_types.clone();
                            let host_options = host_options.clone();
                            move |new_type: ColumnType| {
                                let mut cols = (*columns).clone();
                                let mut types = (*column_types).clone();
                                let prev_file_path = cols[idx].file_path.clone();
                                types[idx] = new_type;
                                cols[idx] = match new_type {
                                    ColumnType::Agent => {
                                        let first_host = host_options.first();
                                        if let Some(h) = first_host {
                                            CompareColumn {
                                                source: ColumnSource::Agent {
                                                    host_id: h.host_id.to_string(),
                                                    hostname: h.hostname.clone(),
                                                },
                                                label: format!("{} ({})", h.hostname, h.environment),
                                                file_path: prev_file_path,
                                            }
                                        } else {
                                            CompareColumn {
                                                source: ColumnSource::Agent { host_id: String::new(), hostname: String::new() },
                                                label: String::new(),
                                                file_path: prev_file_path,
                                            }
                                        }
                                    }
                                    ColumnType::Github => CompareColumn {
                                        source: ColumnSource::Github { url: String::new() },
                                        label: "GitHub".to_string(),
                                        file_path: String::new(),
                                    },
                                };
                                columns.set(cols);
                                column_types.set(types);
                            }
                        };

                        let toggle_agent = {
                            let on_toggle_type = on_toggle_type.clone();
                            move |_| on_toggle_type(ColumnType::Agent)
                        };
                        let toggle_github = {
                            let on_toggle_type = on_toggle_type.clone();
                            move |_| on_toggle_type(ColumnType::Github)
                        };

                        let remove = {
                            let remove_column = remove_column.clone();
                            move |_| remove_column.emit(idx)
                        };

                        let col_content = match (&col_type, &col.source) {
                            (ColumnType::Agent, ColumnSource::Agent { host_id, .. }) => {
                                let col_host_id = host_id.clone();
                                let col_file_path = col.file_path.clone();
                                let on_change = {
                                    let columns = columns.clone();
                                    let host_options = host_options.clone();
                                    move |e: yew::Event| {
                                        let select: web_sys::HtmlSelectElement = e.target_unchecked_into();
                                        let val = select.value();
                                        let hostname = host_options.iter()
                                            .find(|h| h.host_id.to_string() == val)
                                            .map(|h| h.hostname.clone())
                                            .unwrap_or_default();
                                        let label = host_options.iter()
                                            .find(|h| h.host_id.to_string() == val)
                                            .map(|h| format!("{} ({})", h.hostname, h.environment))
                                            .unwrap_or_default();
                                        let mut cols = (*columns).clone();
                                        cols[idx] = CompareColumn {
                                            source: ColumnSource::Agent { host_id: val, hostname },
                                            label,
                                            file_path: cols[idx].file_path.clone(),
                                        };
                                        columns.set(cols);
                                    }
                                };
                                let on_path_input = {
                                    let columns = columns.clone();
                                    move |e: yew::InputEvent| {
                                        let input: HtmlInputElement = e.target_unchecked_into();
                                        let val = input.value();
                                        let mut cols = (*columns).clone();
                                        cols[idx].file_path = val;
                                        columns.set(cols);
                                    }
                                };
                                let host_missing = col_host_id.is_empty();
                                let path_missing = col_file_path.is_empty();
                                html! {
                                    <>
                                        if !has_hosts {
                                            <span class="compare-hint compare-hint-error">{"No hosts loaded. Click Reload hosts."}</span>
                                        }
                                        <select onchange={on_change} value={col_host_id.clone()}>
                                            <option value="">{"— Select agent —"}</option>
                                            { for host_options.iter().map(|h| {
                                                let hid = h.host_id.to_string();
                                                html! {
                                                    <option value={hid}>
                                                        { format!("{} ({})", h.hostname, h.environment) }
                                                    </option>
                                                }
                                            })}
                                        </select>
                                        if host_missing && has_hosts {
                                            <span class="compare-hint compare-hint-error">{"Select an agent"}</span>
                                        }
                                        <input
                                            class="compare-col-path-input"
                                            type="text"
                                            placeholder="/etc/myapp/config.yaml"
                                            value={col_file_path}
                                            oninput={on_path_input}
                                        />
                                        if path_missing && !host_missing {
                                            <span class="compare-hint compare-hint-error">{"Enter a file path"}</span>
                                        }
                                    </>
                                }
                            }
                            (ColumnType::Github, ColumnSource::Github { url }) => {
                                let url_val = url.clone();
                                let on_input = {
                                    let columns = columns.clone();
                                    move |e: yew::InputEvent| {
                                        let input: HtmlInputElement = e.target_unchecked_into();
                                        let val = input.value();
                                        let label = if val.is_empty() {
                                            "GitHub".to_string()
                                        } else {
                                            val.rsplit('/').next().unwrap_or("GitHub").to_string()
                                        };
                                        let mut cols = (*columns).clone();
                                        cols[idx] = CompareColumn {
                                            source: ColumnSource::Github { url: val },
                                            label,
                                            file_path: String::new(),
                                        };
                                        columns.set(cols);
                                    }
                                };
                                html! {
                                    <input
                                        class="compare-github-url-input"
                                        type="text"
                                        placeholder="https://github.com/owner/repo/blob/main/path/to/file.yaml"
                                        value={url_val}
                                        oninput={on_input}
                                    />
                                }
                            }
                            _ => html! {},
                        };

                        html! {
                            <div class="compare-agent-select" key={idx.to_string()}>
                                <label>{ format!("Column {}", idx + 1) }</label>
                                <div class="compare-column-type-toggle">
                                    <button
                                        class={if col_type == ColumnType::Agent { "type-btn type-btn-active" } else { "type-btn" }}
                                        onclick={toggle_agent}
                                    >{"Agent"}</button>
                                    <button
                                        class={if col_type == ColumnType::Github { "type-btn type-btn-active" } else { "type-btn" }}
                                        onclick={toggle_github}
                                    >{"GitHub"}</button>
                                </div>
                                <div class="compare-agent-select-inner">
                                    { col_content }
                                    if idx >= 2 {
                                        <button class="compare-remove-col-btn" onclick={remove}>{"x"}</button>
                                    }
                                </div>
                            </div>
                        }
                    }).collect::<Vec<Html>>() }

                    if (*columns).len() < 4 {
                        <div class="compare-add-btns">
                            <button class="compare-add-col-btn" onclick={{
                                let add_column = add_column.clone();
                                move |_| add_column.emit(())
                            }}>{"+ Agent"}</button>
                            <button class="compare-add-col-btn compare-add-github-btn" onclick={{
                                let add_github_column = add_github_column.clone();
                                move |_| add_github_column.emit(())
                            }}>{"+ GitHub"}</button>
                        </div>
                    }
                </div>

                <button
                    class="workflow-submit-btn"
                    disabled={matches!(*phase, ComparePhase::Fetching)}
                    onclick={{
                        let on_compare = on_compare.clone();
                        move |_| on_compare.emit(())
                    }}
                >
                    {"Compare"}
                </button>

                { match &*phase {
                    ComparePhase::Fetching => html! {
                        <div class="workflow-status">
                            <span class="workflow-spinner"></span>
                            {"Fetching file contents..."}
                        </div>
                    },
                    ComparePhase::Error(msg) => html! {
                        <div class="workflow-status workflow-status-error">
                            { msg.clone() }
                        </div>
                    },
                    ComparePhase::Done => html! {},
                    ComparePhase::Idle => html! {},
                }}

                if matches!(*phase, ComparePhase::Done) {
                    if current_format == "side_by_side" {
                        <div class="compare-columns">
                            <div class="compare-column">
                                <div class="compare-column-header">
                                    { (*columns).first().map(|c| if c.label.is_empty() { "Column 1".to_string() } else { c.label.clone() }).unwrap_or_default() }
                                </div>
                                <div class="compare-column-content" ref={diff_content_ref.clone()}>
                                    { render_diff_lines(&diff_lines_left, true, (*change_groups).clone()) }
                                </div>
                            </div>
                            <div class="compare-column">
                                <div class="compare-column-header">
                                    { (*columns).get(1).map(|c| if c.label.is_empty() { "Column 2".to_string() } else { c.label.clone() }).unwrap_or_default() }
                                </div>
                                <div class="compare-column-content">
                                    { render_diff_lines(&diff_lines_right, false, (*change_groups).clone()) }
                                </div>
                            </div>
                        </div>
                    } else {
                        { render_unified_view(
                            &columns,
                            &unified_lines,
                            current_format.as_str(),
                            &change_groups,
                            num_changes,
                            diff_content_ref.clone(),
                        )}
                    }

                    { for (*columns).iter().skip(2).enumerate().map(|(extra_idx, col)| {
                        let result = (*results).get(extra_idx + 2).and_then(|r| r.clone());
                        let header = if col.label.is_empty() {
                            format!("Column {}", extra_idx + 3)
                        } else {
                            col.label.clone()
                        };
                        html! {
                            <div class="compare-extra-column" key={(extra_idx + 2).to_string()}>
                                <div class="compare-column-header">{ header }</div>
                                <div class="compare-column-content">
                                    { match result {
                                        None => html! { <div class="compare-line compare-line-context">{"No data"}</div> },
                                        Some(r) if !r.exists => html! { <div class="compare-line compare-line-removed">{"File not found"}</div> },
                                        Some(r) => html! {
                                            <pre class="compare-raw-content">{ r.content.unwrap_or_default() }</pre>
                                        },
                                    }}
                                </div>
                            </div>
                        }
                    }).collect::<Vec<Html>>() }
                }
            </div>
        </div>
    }
}

fn render_unified_view(
    columns: &[CompareColumn],
    unified: &[DiffLine],
    format: &str,
    change_groups: &[ChangeGroup],
    num_changes: usize,
    diff_content_ref: NodeRef,
) -> Html {
    let col1_label = columns
        .first()
        .map(|c| {
            if c.label.is_empty() {
                "Column 1".to_string()
            } else {
                c.label.clone()
            }
        })
        .unwrap_or_default();
    let col2_label = columns
        .get(1)
        .map(|c| {
            if c.label.is_empty() {
                "Column 2".to_string()
            } else {
                c.label.clone()
            }
        })
        .unwrap_or_default();
    html! {
        <div class="compare-unified-panel">
            <div class="compare-unified-header">
                <span class="compare-unified-label-old">{ col1_label }</span>
                <span class="compare-unified-arrow">{" > "}</span>
                <span class="compare-unified-label-new">{ col2_label }</span>
                <span class="compare-change-count">
                    { format!("{} change{}", num_changes, if num_changes == 1 { "" } else { "s" }) }
                </span>
            </div>
            <div class="compare-column-content compare-unified-content" ref={diff_content_ref}>
                { render_unified_lines(unified, format, change_groups) }
            </div>
        </div>
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_result(
    idx: usize,
    result: CompareResult,
    fetched: Rc<std::cell::RefCell<Vec<Option<CompareResult>>>>,
    counter: Rc<std::cell::RefCell<usize>>,
    num_cols: usize,
    results_state: UseStateHandle<Vec<Option<CompareResult>>>,
    diff_left: UseStateHandle<Vec<DiffLine>>,
    diff_right: UseStateHandle<Vec<DiffLine>>,
    phase: UseStateHandle<ComparePhase>,
    unified: UseStateHandle<Vec<DiffLine>>,
    change_groups: UseStateHandle<Vec<ChangeGroup>>,
) {
    fetched.borrow_mut()[idx] = Some(result);
    *counter.borrow_mut() += 1;

    if *counter.borrow() == num_cols {
        let all: Vec<Option<CompareResult>> = fetched.borrow().iter().cloned().collect();
        results_state.set(all.clone());

        let left_content = all
            .first()
            .and_then(|r| r.as_ref())
            .and_then(|r| r.content.clone())
            .unwrap_or_default();
        let right_content = all
            .get(1)
            .and_then(|r| r.as_ref())
            .and_then(|r| r.content.clone())
            .unwrap_or_default();

        let (left_lines, right_lines) = compute_diff(&left_content, &right_content);
        let (uni_lines, groups) = compute_unified(&left_content, &right_content);
        diff_left.set(left_lines);
        diff_right.set(right_lines);
        unified.set(uni_lines);
        change_groups.set(groups);
        phase.set(ComparePhase::Done);
    }
}

fn render_diff_lines(
    lines: &[DiffLine],
    is_left: bool,
    change_groups: Vec<ChangeGroup>,
) -> Html {
    // Build a set of line indices that start change groups
    let change_starts: HashSet<usize> = if is_left {
        change_groups.iter().map(|g| g.start).collect()
    } else {
        HashSet::new()
    };

    html! {
        for lines.iter().enumerate().map(|(i, line)| {
            let kind_class = match line.kind {
                DiffLineKind::Context | DiffLineKind::Header | DiffLineKind::HunkMeta => "compare-line-context",
                DiffLineKind::Added => "compare-line-added",
                DiffLineKind::Removed => "compare-line-removed",
            };
            let skip = (is_left && line.kind == DiffLineKind::Added)
                || (!is_left && line.kind == DiffLineKind::Removed);

            // Determine which change group this line belongs to
            let group_id = if is_left {
                change_groups.iter().enumerate()
                    .find(|(_, g)| i >= g.start && i <= g.end)
                    .map(|(idx, _)| idx)
            } else {
                None
            };

            let is_group_start = change_starts.contains(&i);
            let marker_id = if is_group_start { group_id.map(|gid| format!("change-group-{}", gid)) } else { None };

            if skip {
                html! { <div class="compare-line compare-line-placeholder" key={i.to_string()} id={marker_id}></div> }
            } else {
                html! {
                    <div class={format!("compare-line {}", kind_class)} key={i.to_string()} id={marker_id}>
                        <span class="compare-line-num">
                            { line.line_num.map(|n| n.to_string()).unwrap_or_default() }
                        </span>
                        <span class="compare-line-content">
                            { for line.words.iter().map(|seg| {
                                if seg.changed {
                                    html! { <span class="compare-word-hl">{ &seg.content }</span> }
                                } else {
                                    html! { <span>{ &seg.content }</span> }
                                }
                            }).collect::<Vec<Html>>() }
                        </span>
                    </div>
                }
            }
        })
    }
}

fn compute_diff(left: &str, right: &str) -> (Vec<DiffLine>, Vec<DiffLine>) {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_lines(left, right);

    let mut left_lines: Vec<DiffLine> = Vec::new();
    let mut right_lines: Vec<DiffLine> = Vec::new();
    let mut left_num: u32 = 0;
    let mut right_num: u32 = 0;

    let mut change_vec: Vec<(ChangeTag, String)> = Vec::new();
    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            change_vec.push((change.tag(), change.to_string_lossy().to_string()));
        }
    }

    let mut i = 0;

    while i < change_vec.len() {
        let (tag, ref content) = change_vec[i];
        match tag {
            ChangeTag::Equal => {
                left_num += 1;
                right_num += 1;
                left_lines.push(DiffLine {
                    line_num: Some(left_num),
                    kind: DiffLineKind::Context,
                    content: content.clone(),
                    words: vec![WordSegment {
                        content: content.clone(),
                        changed: false,
                    }],
                });
                right_lines.push(DiffLine {
                    line_num: Some(right_num),
                    kind: DiffLineKind::Context,
                    content: content.clone(),
                    words: vec![WordSegment {
                        content: content.clone(),
                        changed: false,
                    }],
                });
                i += 1;
            }
            ChangeTag::Delete => {
                let mut deleted_texts = vec![content.clone()];
                let mut j = i + 1;
                while j < change_vec.len() && change_vec[j].0 == ChangeTag::Delete {
                    deleted_texts.push(change_vec[j].1.clone());
                    j += 1;
                }
                let mut inserted_texts: Vec<String> = Vec::new();
                while j < change_vec.len() && change_vec[j].0 == ChangeTag::Insert {
                    inserted_texts.push(change_vec[j].1.clone());
                    j += 1;
                }

                let paired_count = deleted_texts.len().min(inserted_texts.len());
                for k in 0..paired_count {
                    left_num += 1;
                    right_num += 1;
                    let left_words = word_diff(&deleted_texts[k], &inserted_texts[k], true);
                    let right_words = word_diff(&deleted_texts[k], &inserted_texts[k], false);
                    left_lines.push(DiffLine {
                        line_num: Some(left_num),
                        kind: DiffLineKind::Removed,
                        content: deleted_texts[k].clone(),
                        words: left_words,
                    });
                    right_lines.push(DiffLine {
                        line_num: Some(right_num),
                        kind: DiffLineKind::Added,
                        content: inserted_texts[k].clone(),
                        words: right_words,
                    });
                }
                for text in deleted_texts.iter().skip(paired_count) {
                    left_num += 1;
                    let content = text.clone();
                    left_lines.push(DiffLine {
                        line_num: Some(left_num),
                        kind: DiffLineKind::Removed,
                        content: content.clone(),
                        words: vec![WordSegment {
                            content,
                            changed: true,
                        }],
                    });
                    right_lines.push(DiffLine {
                        line_num: None,
                        kind: DiffLineKind::Removed,
                        content: String::new(),
                        words: Vec::new(),
                    });
                }
                for text in inserted_texts.iter().skip(paired_count) {
                    right_num += 1;
                    left_lines.push(DiffLine {
                        line_num: None,
                        kind: DiffLineKind::Added,
                        content: String::new(),
                        words: Vec::new(),
                    });
                    let content = text.clone();
                    right_lines.push(DiffLine {
                        line_num: Some(right_num),
                        kind: DiffLineKind::Added,
                        content: content.clone(),
                        words: vec![WordSegment {
                            content,
                            changed: true,
                        }],
                    });
                }

                i = j;
            }
            ChangeTag::Insert => {
                right_num += 1;
                left_lines.push(DiffLine {
                    line_num: None,
                    kind: DiffLineKind::Added,
                    content: String::new(),
                    words: Vec::new(),
                });
                right_lines.push(DiffLine {
                    line_num: Some(right_num),
                    kind: DiffLineKind::Added,
                    content: content.clone(),
                    words: vec![WordSegment {
                        content: content.clone(),
                        changed: true,
                    }],
                });
                i += 1;
            }
        }
    }

    (left_lines, right_lines)
}

fn compute_unified(left: &str, right: &str) -> (Vec<DiffLine>, Vec<ChangeGroup>) {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_lines(left, right);

    let mut lines: Vec<DiffLine> = Vec::new();
    let mut groups: Vec<ChangeGroup> = Vec::new();
    let mut left_num: u32 = 0;
    let mut right_num: u32 = 0;

    let mut change_vec: Vec<(ChangeTag, String)> = Vec::new();
    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            change_vec.push((change.tag(), change.to_string_lossy().to_string()));
        }
    }

    let mut i = 0;
    while i < change_vec.len() {
        let (tag, ref content) = change_vec[i];
        match tag {
            ChangeTag::Equal => {
                left_num += 1;
                right_num += 1;
                lines.push(DiffLine {
                    line_num: Some(left_num),
                    kind: DiffLineKind::Context,
                    content: content.clone(),
                    words: vec![WordSegment {
                        content: content.clone(),
                        changed: false,
                    }],
                });
                i += 1;
            }
            ChangeTag::Delete => {
                let mut deleted_texts = vec![content.clone()];
                let mut j = i + 1;
                while j < change_vec.len() && change_vec[j].0 == ChangeTag::Delete {
                    deleted_texts.push(change_vec[j].1.clone());
                    j += 1;
                }
                let mut inserted_texts: Vec<String> = Vec::new();
                while j < change_vec.len() && change_vec[j].0 == ChangeTag::Insert {
                    inserted_texts.push(change_vec[j].1.clone());
                    j += 1;
                }

                let change_start = lines.len();
                let mut added = 0usize;
                let mut removed = 0usize;

                // Emit removed lines
                for text in &deleted_texts {
                    left_num += 1;
                    removed += 1;
                    lines.push(DiffLine {
                        line_num: Some(left_num),
                        kind: DiffLineKind::Removed,
                        content: text.clone(),
                        words: vec![WordSegment {
                            content: text.clone(),
                            changed: true,
                        }],
                    });
                }
                // Emit added lines
                for text in &inserted_texts {
                    right_num += 1;
                    added += 1;
                    lines.push(DiffLine {
                        line_num: Some(right_num),
                        kind: DiffLineKind::Added,
                        content: text.clone(),
                        words: vec![WordSegment {
                            content: text.clone(),
                            changed: true,
                        }],
                    });
                }
                // Pair word diffs for equal-length blocks
                let paired = deleted_texts.len().min(inserted_texts.len());
                for k in 0..paired {
                    let left_words = word_diff(&deleted_texts[k], &inserted_texts[k], true);
                    let right_words = word_diff(&deleted_texts[k], &inserted_texts[k], false);
                    if let Some(l) = lines.get_mut(change_start + k) {
                        l.words = left_words;
                    }
                    if let Some(l) = lines.get_mut(change_start + deleted_texts.len() + k) {
                        l.words = right_words;
                    }
                }

                let change_end = lines.len().saturating_sub(1);
                groups.push(ChangeGroup {
                    start: change_start,
                    end: change_end,
                    added,
                    removed,
                });

                i = j;
            }
            ChangeTag::Insert => {
                right_num += 1;
                let start = lines.len();
                lines.push(DiffLine {
                    line_num: Some(right_num),
                    kind: DiffLineKind::Added,
                    content: content.clone(),
                    words: vec![WordSegment {
                        content: content.clone(),
                        changed: true,
                    }],
                });
                groups.push(ChangeGroup {
                    start,
                    end: start,
                    added: 1,
                    removed: 0,
                });
                i += 1;
            }
        }
    }

    (lines, groups)
}

fn render_unified_lines(
    lines: &[DiffLine],
    format: &str,
    change_groups: &[ChangeGroup],
) -> Html {
    let context_lines: usize = 3;
    let mut visible_indices: HashSet<usize> = HashSet::new();

    if format == "full_file" {
        for idx in 0..lines.len() {
            visible_indices.insert(idx);
        }
    } else if format == "context" {
        for group in change_groups {
            let ctx_start = group.start.saturating_sub(context_lines);
            let ctx_end = (group.end + context_lines).min(lines.len().saturating_sub(1));
            for idx in ctx_start..=ctx_end {
                visible_indices.insert(idx);
            }
        }
    } else {
        // unified: show all change lines
        for group in change_groups {
            for idx in group.start..=group.end {
                visible_indices.insert(idx);
            }
        }
    }

    let mut html_lines: Vec<Html> = Vec::new();
    let mut last_shown: Option<usize> = None;
    let mut skip_count: usize = 0;

    for (i, line) in lines.iter().enumerate() {
        let visible = visible_indices.contains(&i);

        if !visible {
            skip_count += 1;
            continue;
        }

        // Before showing a visible line, emit a skip marker if we skipped lines
        if skip_count > 0 {
            let collapse_key = if let Some(prev) = last_shown {
                prev + 1
            } else {
                0
            };
            html_lines.push(html! {
                <div class="compare-skip-marker" key={format!("skip-{}", collapse_key)}>
                    { format!("... {} unchanged lines hidden ...", skip_count) }
                </div>
            });
            skip_count = 0;
        }

        // Determine group membership for marker IDs
        let group_id = change_groups
            .iter()
            .enumerate()
            .find(|(_, g)| i >= g.start && i <= g.end)
            .map(|(idx, _)| idx);

        let is_group_start = group_id.is_some()
            && change_groups
                .get(group_id.unwrap())
                .map(|g| g.start == i)
                .unwrap_or(false);
        let marker_id = if is_group_start {
            group_id.map(|gid| format!("change-group-{}", gid))
        } else {
            None
        };

        let kind_class = match line.kind {
            DiffLineKind::Added => "compare-line-added",
            DiffLineKind::Removed => "compare-line-removed",
            _ => "compare-line-context",
        };

        let prefix = match line.kind {
            DiffLineKind::Added => "+",
            DiffLineKind::Removed => "-",
            _ => " ",
        };

        html_lines.push(html! {
            <div class={format!("compare-line {}", kind_class)} key={i.to_string()} id={marker_id}>
                <span class="compare-line-num">
                    { line.line_num.map(|n| n.to_string()).unwrap_or_default() }
                </span>
                <span class="compare-line-prefix">{ prefix }</span>
                <span class="compare-line-content">
                    { for line.words.iter().map(|seg| {
                        if seg.changed {
                            html! { <span class="compare-word-hl">{ &seg.content }</span> }
                        } else {
                            html! { <span>{ &seg.content }</span> }
                        }
                    }).collect::<Vec<Html>>() }
                </span>
            </div>
        });

        last_shown = Some(i);
    }

    // Emit trailing skip if needed
    if skip_count > 0 {
        let collapse_key = if let Some(prev) = last_shown {
            prev + 1
        } else {
            0
        };
        html_lines.push(html! {
            <div class="compare-skip-marker" key={format!("skip-{}", collapse_key)}>
                { format!("... {} unchanged lines hidden ...", skip_count) }
            </div>
        });
    }

    html! { for html_lines }
}

fn word_diff(left_line: &str, right_line: &str, is_left: bool) -> Vec<WordSegment> {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_words(left_line, right_line);
    let mut segments = Vec::new();

    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            let content = change.to_string_lossy().to_string();
            match change.tag() {
                ChangeTag::Equal => {
                    segments.push(WordSegment {
                        content,
                        changed: false,
                    });
                }
                ChangeTag::Delete => {
                    if is_left {
                        segments.push(WordSegment {
                            content,
                            changed: true,
                        });
                    }
                }
                ChangeTag::Insert => {
                    if !is_left {
                        segments.push(WordSegment {
                            content,
                            changed: true,
                        });
                    }
                }
            }
        }
    }

    if segments.is_empty() {
        vec![WordSegment {
            content: if is_left {
                left_line.to_string()
            } else {
                right_line.to_string()
            },
            changed: true,
        }]
    } else {
        segments
    }
}
