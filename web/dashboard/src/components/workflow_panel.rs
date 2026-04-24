use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use uuid::Uuid;
use web_sys::{HtmlInputElement, HtmlTextAreaElement};
use yew::{
    function_component, html, use_effect_with, Callback, Html, Properties, TargetCast,
    UseStateHandle,
};

use crate::api;
use crate::models::{FileChangeRequest, RealtimeMessage, WorkflowCreateRequest, WorkflowDefaults};
use crate::storage;

#[derive(Properties, PartialEq)]
pub struct WorkflowPanelProps {
    pub events: Rc<Vec<RealtimeMessage>>,
    pub selected_events: Rc<HashSet<Uuid>>,
    pub server_url: String,
    pub on_close: Callback<()>,
    pub on_pr_created: Callback<()>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PanelState {
    Editing,
    Submitting,
    Polling { workflow_id: Uuid },
    Completed { pr_url: String },
    Failed { error: String },
}

fn today_branch_name() -> String {
    let date = js_sys::Date::new_0();
    format!(
        "config-watch-{}-{:02}-{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date()
    )
}

#[function_component(WorkflowPanel)]
pub fn workflow_panel(props: &WorkflowPanelProps) -> Html {
    let selected: Vec<RealtimeMessage> = props
        .events
        .iter()
        .filter(|e| props.selected_events.contains(&e.event_id))
        .cloned()
        .collect();

    // Form state
    let repo_url: UseStateHandle<String> = yew::use_state(String::new);
    let branch_name: UseStateHandle<String> = yew::use_state(today_branch_name);
    let base_branch: UseStateHandle<String> = yew::use_state(|| "main".to_string());
    let pr_title: UseStateHandle<String> =
        yew::use_state(|| "config-watch: apply configuration changes".to_string());
    let pr_description: UseStateHandle<String> = yew::use_state(String::new);
    let reviewers: UseStateHandle<String> = yew::use_state(String::new);
    let github_token: UseStateHandle<String> = yew::use_state(String::new);
    let panel_state: UseStateHandle<PanelState> = yew::use_state(|| PanelState::Editing);

    // Per-file custom search filenames (event_id → repo filename to search for)
    let repo_filenames: UseStateHandle<HashMap<Uuid, String>> = {
        let initial: HashMap<Uuid, String> = selected
            .iter()
            .map(|e| {
                let basename = e.path.rsplit('/').next().unwrap_or(&e.path).to_string();
                (e.event_id, basename)
            })
            .collect();
        yew::use_state(|| initial)
    };

    // Load defaults from localStorage on mount
    {
        let repo_url = repo_url.clone();
        let base_branch = base_branch.clone();
        let pr_title = pr_title.clone();
        let github_token = github_token.clone();
        let server_url = props.server_url.clone();
        use_effect_with((), move |_| {
            let key = storage::defaults_key(&server_url);
            if let Some(defaults) = storage::load_workflow_defaults(&key) {
                if !defaults.repo_url.is_empty() {
                    repo_url.set(defaults.repo_url);
                }
                if !defaults.base_branch.is_empty() {
                    base_branch.set(defaults.base_branch);
                }
                if !defaults.pr_title.is_empty() {
                    pr_title.set(defaults.pr_title);
                }
                if !defaults.github_token.is_empty() {
                    github_token.set(defaults.github_token);
                }
            }
            // branch_name is already initialized with today_branch_name()
        });
    }

    // Polling effect for workflow status
    {
        let panel_state = panel_state.clone();
        let server_url = props.server_url.clone();
        let interval: Rc<RefCell<Option<gloo::timers::callback::Interval>>> =
            yew::use_mut_ref(|| None);

        use_effect_with((*panel_state).clone(), move |state: &PanelState| {
            if let PanelState::Polling { workflow_id } = state {
                let id_str = workflow_id.to_string();
                let server = server_url.clone();
                let ps = panel_state.clone();
                let on_result =
                    Callback::from(move |resp: Option<crate::models::WorkflowStatusResponse>| {
                        if let Some(r) = resp {
                            let row = r.workflow;
                            match row.status.as_str() {
                                "completed" => {
                                    ps.set(PanelState::Completed {
                                        pr_url: row.pr_url.unwrap_or_default(),
                                    });
                                }
                                "failed" => {
                                    ps.set(PanelState::Failed {
                                        error: row
                                            .error_message
                                            .unwrap_or_else(|| "unknown error".to_string()),
                                    });
                                }
                                _ => {}
                            }
                        }
                    });

                api::get_workflow(&server, &id_str, on_result.clone());

                let id_str2 = id_str.clone();
                let server2 = server.clone();
                let interval_handle = gloo::timers::callback::Interval::new(2000, move || {
                    api::get_workflow(&server2, &id_str2, on_result.clone());
                });
                *interval.borrow_mut() = Some(interval_handle);
            }
            || {}
        });
    }

    // Notify parent when PR is successfully created (one-shot)
    {
        let panel_state = panel_state.clone();
        let on_pr_created = props.on_pr_created.clone();
        let fired: Rc<RefCell<bool>> = yew::use_mut_ref(|| false);
        use_effect_with((*panel_state).clone(), move |state: &PanelState| {
            if matches!(state, PanelState::Completed { .. }) && !*fired.borrow() {
                *fired.borrow_mut() = true;
                on_pr_created.emit(());
            }
            || {}
        });
    }

    // Submit handler
    let on_submit = {
        let repo_url = repo_url.clone();
        let branch_name = branch_name.clone();
        let base_branch = base_branch.clone();
        let pr_title = pr_title.clone();
        let pr_description = pr_description.clone();
        let reviewers = reviewers.clone();
        let github_token = github_token.clone();
        let panel_state = panel_state.clone();
        let server_url = props.server_url.clone();
        let selected_events = selected.clone();
        let repo_filenames = repo_filenames.clone();
        Callback::from(move |_: ()| {
            let filenames = (*repo_filenames).clone();
            let file_changes: Vec<FileChangeRequest> = selected_events
                .iter()
                .map(|e| {
                    let default_basename = e.path.rsplit('/').next().unwrap_or(&e.path).to_string();
                    let custom = filenames.get(&e.event_id).cloned();
                    FileChangeRequest {
                        canonical_path: e.path.clone(),
                        content_hash: None,
                        event_kind: e.event_kind.clone(),
                        repo_filename: if custom.as_deref() == Some(&default_basename)
                            || custom.is_none()
                        {
                            None
                        } else {
                            custom
                        },
                    }
                })
                .collect();

            let event_ids: Vec<Uuid> = selected_events.iter().map(|e| e.event_id).collect();

            let reviewers_list: Option<Vec<String>> = {
                let r = (*reviewers).clone();
                if r.trim().is_empty() {
                    None
                } else {
                    Some(
                        r.split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect(),
                    )
                }
            };

            let body = WorkflowCreateRequest {
                repo_url: (*repo_url).clone(),
                branch_name: (*branch_name).clone(),
                base_branch: if (*base_branch).is_empty() {
                    None
                } else {
                    Some((*base_branch).clone())
                },
                pr_title: (*pr_title).clone(),
                pr_description: if pr_description.is_empty() {
                    None
                } else {
                    Some((*pr_description).clone())
                },
                file_changes,
                reviewers: reviewers_list,
                github_token: if github_token.is_empty() {
                    None
                } else {
                    Some((*github_token).clone())
                },
                event_ids,
            };

            // Save current values as defaults
            {
                let key = storage::defaults_key(&server_url);
                let defaults = WorkflowDefaults {
                    repo_url: (*repo_url).clone(),
                    base_branch: (*base_branch).clone(),
                    pr_title: (*pr_title).clone(),
                    github_token: (*github_token).clone(),
                };
                storage::save_workflow_defaults(&key, &defaults);
            }

            let ps = panel_state.clone();
            api::create_workflow(
                &server_url,
                &body,
                Callback::from(move |resp: Option<crate::models::WorkflowCreateResponse>| {
                    if let Some(r) = resp {
                        ps.set(PanelState::Polling {
                            workflow_id: r.workflow_id,
                        });
                    } else {
                        ps.set(PanelState::Failed {
                            error: "Failed to create workflow".to_string(),
                        });
                    }
                }),
            );

            panel_state.set(PanelState::Submitting);
        })
    };

    let is_editable = matches!(*panel_state, PanelState::Editing);

    // Status display
    let status_html = match &*panel_state {
        PanelState::Editing => html! {},
        PanelState::Submitting => html! {
            <div class="workflow-status">
                <span class="workflow-spinner"></span>
                {"Creating workflow..."}
            </div>
        },
        PanelState::Polling { .. } => html! {
            <div class="workflow-status">
                <span class="workflow-spinner"></span>
                {"Running workflow... (pulling, searching files, committing, pushing, creating PR)"}
            </div>
        },
        PanelState::Completed { pr_url } => html! {
            <div class="workflow-status workflow-status-success">
                <span>{"PR created: "}</span>
                <a href={pr_url.clone()} target="_blank">{ pr_url.clone() }</a>
            </div>
        },
        PanelState::Failed { error } => html! {
            <div class="workflow-status workflow-status-error">
                <span>{ format!("Failed: {}", error) }</span>
            </div>
        },
    };

    html! {
        <div class="workflow-panel-overlay" onclick={{
            let on_close = props.on_close.clone();
            move |_| on_close.emit(())
        }}>
            <div class="workflow-panel" onclick={|e: yew::MouseEvent| e.stop_propagation()}>
                <div class="workflow-panel-header">
                    <h2>{"Create Pull Request"}</h2>
                    <button class="workflow-close-btn" onclick={{
                        let on_close = props.on_close.clone();
                        move |_| on_close.emit(())
                    }}>{"x"}</button>
                </div>

                <div class="workflow-panel-body">
                    { status_html }

                    <div class="workflow-section">
                        <h3>{"Selected Files"}</h3>
                        <p class="workflow-files-hint">{"Files will be matched by name in the repo"}</p>
                        <div class="workflow-files-list">
                            { for selected.iter().map(|e| {
                                let eid = e.event_id;
                                let current_filename = (*repo_filenames).get(&eid).cloned().unwrap_or_else(|| {
                                    e.path.rsplit('/').next().unwrap_or(&e.path).to_string()
                                });
                                let on_change = {
                                    let repo_filenames = repo_filenames.clone();
                                    Callback::from(move |ev: yew::Event| {
                                        let input: HtmlInputElement = ev.target_unchecked_into();
                                        let val = input.value();
                                        let mut map = (*repo_filenames).clone();
                                        map.insert(eid, val);
                                        repo_filenames.set(map);
                                    })
                                };
                                html! {
                                    <div class="workflow-file-row" key={eid.to_string()}>
                                        <span class="workflow-file-icon">{ crate::models::event_kind_icon(&e.event_kind) }</span>
                                        <input
                                            class="workflow-file-search-input"
                                            type="text"
                                            value={current_filename}
                                            disabled={!is_editable}
                                            onchange={on_change}
                                        />
                                        <span class="workflow-file-host-path">{ &e.path }</span>
                                    </div>
                                }
                            })}
                        </div>
                    </div>

                    <div class="workflow-section">
                        <h3>{"Repository"}</h3>
                        <div class="workflow-field">
                            <label>{"Repo URL"}</label>
                            <input
                                type="text"
                                placeholder="https://github.com/owner/repo"
                                value={(*repo_url).clone()}
                                disabled={!is_editable}
                                onchange={{
                                    let repo_url = repo_url.clone();
                                    move |e: yew::Event| {
                                        let input: HtmlInputElement = e.target_unchecked_into();
                                        repo_url.set(input.value());
                                    }
                                }}
                            />
                        </div>
                        <div class="workflow-field">
                            <label>{"New Branch"}</label>
                            <input
                                type="text"
                                value={(*branch_name).clone()}
                                disabled={!is_editable}
                                onchange={{
                                    let branch_name = branch_name.clone();
                                    move |e: yew::Event| {
                                        let input: HtmlInputElement = e.target_unchecked_into();
                                        branch_name.set(input.value());
                                    }
                                }}
                            />
                        </div>
                        <div class="workflow-field">
                            <label>{"Base Branch"}</label>
                            <input
                                type="text"
                                value={(*base_branch).clone()}
                                placeholder="main"
                                disabled={!is_editable}
                                onchange={{
                                    let base_branch = base_branch.clone();
                                    move |e: yew::Event| {
                                        let input: HtmlInputElement = e.target_unchecked_into();
                                        base_branch.set(input.value());
                                    }
                                }}
                            />
                        </div>
                    </div>

                    <div class="workflow-section">
                        <h3>{"Pull Request"}</h3>
                        <div class="workflow-field">
                            <label>{"Title"}</label>
                            <input
                                type="text"
                                value={(*pr_title).clone()}
                                disabled={!is_editable}
                                onchange={{
                                    let pr_title = pr_title.clone();
                                    move |e: yew::Event| {
                                        let input: HtmlInputElement = e.target_unchecked_into();
                                        pr_title.set(input.value());
                                    }
                                }}
                            />
                        </div>
                        <div class="workflow-field">
                            <label>{"Description"}</label>
                            <textarea
                                value={(*pr_description).clone()}
                                disabled={!is_editable}
                                onchange={{
                                    let pr_description = pr_description.clone();
                                    move |e: yew::Event| {
                                        let input: HtmlTextAreaElement = e.target_unchecked_into();
                                        pr_description.set(input.value());
                                    }
                                }}
                            />
                        </div>
                        <div class="workflow-field">
                            <label>{"Reviewers"}</label>
                            <input
                                type="text"
                                placeholder="username1, username2 (comma-separated)"
                                value={(*reviewers).clone()}
                                disabled={!is_editable}
                                onchange={{
                                    let reviewers = reviewers.clone();
                                    move |e: yew::Event| {
                                        let input: HtmlInputElement = e.target_unchecked_into();
                                        reviewers.set(input.value());
                                    }
                                }}
                            />
                        </div>
                    </div>

                    <div class="workflow-section">
                        <h3>{"Authentication"}</h3>
                        <div class="workflow-field">
                            <label>{"GitHub Token"}</label>
                            <input
                                type="password"
                                placeholder="ghp_xxxx (optional for public repos)"
                                value={(*github_token).clone()}
                                disabled={!is_editable}
                                onchange={{
                                    let github_token = github_token.clone();
                                    move |e: yew::Event| {
                                        let input: HtmlInputElement = e.target_unchecked_into();
                                        github_token.set(input.value());
                                    }
                                }}
                            />
                        </div>
                    </div>

                    if is_editable {
                        <button
                            class="workflow-submit-btn"
                            disabled={(*repo_url).is_empty()}
                            onclick={move |_| on_submit.emit(())}
                        >
                            {"Create PR"}
                        </button>
                    }
                </div>
            </div>
        </div>
    }
}
