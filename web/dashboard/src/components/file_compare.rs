use serde::{Deserialize, Serialize};
use std::rc::Rc;
use web_sys::HtmlInputElement;
use yew::{
    function_component, html, use_effect_with, Callback, Html, Properties, TargetCast,
    UseStateHandle,
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
                                );
                            },
                        );
                        api::fetch_file_content(&server, &host_id, &path, None, None, on_result);
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
                                );
                            },
                        );
                        let token_ref = if token.is_empty() {
                            None
                        } else {
                            Some(token.as_str())
                        };
                        api::fetch_github_file_content(&server, url, token_ref, on_result);
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

    html! {
        <div class="compare-panel">
            <div class="compare-panel-header">
                <h2>{"File Comparison"}</h2>
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
                    <div class="compare-columns">
                        <div class="compare-column">
                            <div class="compare-column-header">
                                { (*columns).first().map(|c| if c.label.is_empty() { "Column 1".to_string() } else { c.label.clone() }).unwrap_or_default() }
                            </div>
                            <div class="compare-column-content">
                                { render_diff_lines(&diff_lines_left, true) }
                            </div>
                        </div>
                        <div class="compare-column">
                            <div class="compare-column-header">
                                { (*columns).get(1).map(|c| if c.label.is_empty() { "Column 2".to_string() } else { c.label.clone() }).unwrap_or_default() }
                            </div>
                            <div class="compare-column-content">
                                { render_diff_lines(&diff_lines_right, false) }
                            </div>
                        </div>
                    </div>

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
        diff_left.set(left_lines);
        diff_right.set(right_lines);
        phase.set(ComparePhase::Done);
    }
}

fn render_diff_lines(lines: &[DiffLine], is_left: bool) -> Html {
    html! {
        for lines.iter().enumerate().map(|(i, line)| {
            let kind_class = match line.kind {
                DiffLineKind::Context | DiffLineKind::Header | DiffLineKind::HunkMeta => "compare-line-context",
                DiffLineKind::Added => "compare-line-added",
                DiffLineKind::Removed => "compare-line-removed",
            };
            let skip = (is_left && line.kind == DiffLineKind::Added)
                || (!is_left && line.kind == DiffLineKind::Removed);
            if skip {
                html! { <div class="compare-line compare-line-placeholder" key={i.to_string()}></div> }
            } else {
                html! {
                    <div class={format!("compare-line {}", kind_class)} key={i.to_string()}>
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
