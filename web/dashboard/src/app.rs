use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use uuid::Uuid;
use web_sys::HtmlInputElement;
use yew::{
    function_component, html, use_effect_with, use_node_ref, Callback, Html, NodeRef, TargetCast,
    UseStateHandle,
};

use crate::api;
use crate::components::event_list::EventList;
use crate::components::file_compare::FileCompare;
use crate::components::filters::FilterBar;
use crate::components::login::Login;
use crate::components::selection_bar::SelectionBar;
use crate::components::workflow_panel::WorkflowPanel;
use crate::models::{
    history_row_to_message, ChangeEventRow, ChangesPage, ConnectionStatus, FilterState, HostInfo,
    PaginationState, RealtimeMessage, ViewMode, WatchRootInfo,
};
use crate::storage;
use crate::ws;

const MAX_EVENTS: usize = 200;

fn storage_key_for(server: &str, filters: &FilterState) -> String {
    storage::storage_key(server, filters.host_id.as_deref())
}

/// Tracks scroll state to preserve position when new events are prepended.
struct ScrollAnchor {
    scroll_top: f64,
    scroll_height: f64,
}

#[function_component(App)]
pub fn app() -> Html {
    let auth_data: UseStateHandle<Option<storage::AuthData>> =
        yew::use_state(|| storage::load_auth_data());
    let approval_pending: UseStateHandle<bool> = yew::use_state(|| false);
    let stream_events: UseStateHandle<Vec<RealtimeMessage>> = yew::use_state(Vec::new);
    let history_events: UseStateHandle<Vec<RealtimeMessage>> = yew::use_state(Vec::new);
    let connection: UseStateHandle<ConnectionStatus> =
        yew::use_state(|| ConnectionStatus::Disconnected);
    let filters: UseStateHandle<FilterState> = yew::use_state(FilterState::default);
    let expanded: UseStateHandle<Option<Uuid>> = yew::use_state(|| None);
    let server_url: UseStateHandle<String> = yew::use_state(|| "localhost:8082".to_string());
    let view_mode: UseStateHandle<ViewMode> = yew::use_state(|| ViewMode::Stream);
    let hosts: UseStateHandle<Vec<HostInfo>> = yew::use_state(Vec::new);
    let watch_roots: UseStateHandle<Vec<WatchRootInfo>> = yew::use_state(Vec::new);
    let loading: UseStateHandle<bool> = yew::use_state(|| false);
    let selected_events: UseStateHandle<HashSet<Uuid>> = yew::use_state(HashSet::new);
    let workflow_panel_open: UseStateHandle<bool> = yew::use_state(|| false);
    let pagination: UseStateHandle<PaginationState> = yew::use_state(PaginationState::default);
    // Tracks event IDs already lazy-fetched this session so the on-expand
    // callback fires exactly once per event and doesn't loop after writing
    // the result back into `diff_render`.
    let lazy_fetched: Rc<RefCell<HashSet<Uuid>>> = yew::use_mut_ref(HashSet::new);

    // Track the last storage key so we can detect host/server changes
    let last_storage_key: UseStateHandle<Option<String>> = yew::use_state(|| None);

    // CSRF token for mutating API calls — read from cookie each render
    let csrf_token: Option<String> = storage::load_csrf_token();

    // 401 handler — clears auth data and forces re-render to login
    let on_unauthorized = {
        let auth_data = auth_data.clone();
        Callback::from(move |_: ()| {
            storage::clear_auth_data();
            storage::clear_csrf_cookie();
            auth_data.set(None);
        })
    };

    // Load stored events on mount so they appear before connecting
    {
        let stream_events = stream_events.clone();
        let server_url = server_url.clone();
        let filters = filters.clone();
        use_effect_with((), move |_| {
            let key = storage_key_for(&server_url, &filters);
            let stored = storage::load_events(&key);
            if !stored.is_empty() {
                stream_events.set(stored);
            }
        });
    }

    // Scroll position preservation for stream events
    let main_ref: NodeRef = use_node_ref();
    let scroll_anchor: Rc<RefCell<Option<ScrollAnchor>>> = yew::use_mut_ref(|| None);

    // --- Stream mode: WebSocket ---

    let on_message = {
        let stream_events = stream_events.clone();
        let server_url = server_url.clone();
        let filters = filters.clone();
        let main_ref = main_ref.clone();
        let scroll_anchor = scroll_anchor.clone();
        Callback::from(move |msg: RealtimeMessage| {
            let mut current = (*stream_events).clone();
            // Deduplicate by event_id
            if current.iter().any(|e| e.event_id == msg.event_id) {
                return;
            }

            // Capture scroll state BEFORE the update
            if let Some(el) = main_ref.cast::<web_sys::Element>() {
                *scroll_anchor.borrow_mut() = Some(ScrollAnchor {
                    scroll_top: el.scroll_top() as f64,
                    scroll_height: el.scroll_height() as f64,
                });
            }

            current.insert(0, msg);
            if current.len() > MAX_EVENTS {
                current.truncate(MAX_EVENTS);
            }
            let key = storage_key_for(&server_url, &filters);
            storage::save_events(&key, &current);
            stream_events.set(current);
        })
    };

    let on_status = {
        let connection = connection.clone();
        let stream_events = stream_events.clone();
        let server_url = server_url.clone();
        let filters = filters.clone();
        let last_storage_key = last_storage_key.clone();
        let scroll_anchor = scroll_anchor.clone();
        Callback::from(move |status: ConnectionStatus| {
            let prev_key = (*last_storage_key).clone();
            let new_key = storage_key_for(&server_url, &filters);

            let key_changed = match &prev_key {
                Some(pk) => pk != &new_key,
                None => true,
            };

            match &status {
                ConnectionStatus::Connected => {
                    if key_changed {
                        last_storage_key.set(Some(new_key.clone()));
                    }
                    let stored = storage::load_events(&new_key);
                    stream_events.set(stored);
                }
                ConnectionStatus::Disconnected | ConnectionStatus::Error(_) => {
                    // Keep events in memory and localStorage — they're already saved by on_message
                    last_storage_key.set(None);
                }
                ConnectionStatus::Connecting => {}
            }

            *scroll_anchor.borrow_mut() = None;
            connection.set(status);
        })
    };

    // After stream events change, restore scroll position adjusted for new content
    {
        let main_ref = main_ref.clone();
        let scroll_anchor = scroll_anchor.clone();
        let event_count = (*stream_events).len();
        use_effect_with(event_count, move |_| {
            if let Some(el) = main_ref.cast::<web_sys::Element>() {
                if let Some(anchor) = scroll_anchor.borrow_mut().take() {
                    let new_height = el.scroll_height() as f64;
                    let height_delta = new_height - anchor.scroll_height;
                    if height_delta > 0.0 {
                        // New content was added above — shift scroll down by the delta
                        el.set_scroll_top((anchor.scroll_top + height_delta) as i32);
                    }
                }
            }
        });
    }

    // --- History mode: REST fetch ---

    let on_history_result = {
        let history_events = history_events.clone();
        let loading = loading.clone();
        let pagination = pagination.clone();
        let filters = filters.clone();
        Callback::from(move |page: ChangesPage| {
            let fmt = (*filters)
                .diff_format
                .clone()
                .unwrap_or_else(|| "context".to_string());
            let messages: Vec<RealtimeMessage> = page
                .changes
                .iter()
                .map(|row| {
                    let mut msg = history_row_to_message(row);
                    // Restore cached diff_render if we have one.
                    if msg.diff_render.is_none() {
                        msg.diff_render = storage::get_cached_diff(&msg.event_id, &fmt);
                    }
                    msg
                })
                .collect();
            history_events.set(messages);
            pagination.set(PaginationState {
                page: pagination.page,
                page_size: pagination.page_size,
                total: page.total,
            });
            loading.set(false);
        })
    };

    let on_fetch_diff = {
        let server_url = server_url.clone();
        let history_events = history_events.clone();
        let stream_events = stream_events.clone();
        let view_mode = view_mode.clone();
        let lazy_fetched = lazy_fetched.clone();
        let filters = filters.clone();
        let _auth_data = auth_data.clone();
        let on_unauthorized = on_unauthorized.clone();
        Callback::from(move |event_id: Uuid| {
            let server = (*server_url).clone();
            let id_str = event_id.to_string();
            let mode = *view_mode;
            let diff_format = (*filters).diff_format.clone();

            if crate::storage::lazy_diff_endpoint_enabled() {
                if !lazy_fetched.borrow_mut().insert(event_id) {
                    return;
                }
                let history = history_events.clone();
                let stream = stream_events.clone();
                // Compute the storage key now (in the outer Fn closure) so we
                // don't move `filters` into the inner FnOnce callback below.
                let stream_key = storage_key_for(&server, &filters);
                let fmt_label = diff_format.clone().unwrap_or_else(|| "context".to_string());
                let on_unauth = on_unauthorized.clone();
                api::fetch_change_diff(
                    &server,
                    &id_str,
                    diff_format.as_deref(),
                    Callback::from(move |result: api::DiffFetch| {
                        match result {
                            api::DiffFetch::Unauthorized => {
                                on_unauth.emit(());
                                return;
                            }
                            _ => {}
                        }
                        let diff = match result {
                            api::DiffFetch::Ok { render, previous_unavailable } => {
                                if previous_unavailable {
                                    format!("@@warn:previous_unavailable@@\n{}", render)
                                } else {
                                    render
                                }
                            }
                            api::DiffFetch::HostOffline => {
                                "@@error:host_offline@@\nThe agent for this host is offline; cannot render the diff.".to_string()
                            }
                            api::DiffFetch::BothEvicted => {
                                "@@error:snapshots_evicted@@\nBoth previous and current snapshot bytes have been evicted by retention.".to_string()
                            }
                            api::DiffFetch::Failed(msg) => {
                                format!("@@error:fetch_failed@@\n{}", msg)
                            }
                            api::DiffFetch::Unauthorized => unreachable!(),
                        };
                        let update = |events: &mut Vec<RealtimeMessage>| {
                            if let Some(e) = events.iter_mut().find(|e| e.event_id == event_id) {
                                e.diff_render = Some(diff.clone());
                            }
                        };
                        match mode {
                            ViewMode::History => {
                                let mut v = (*history).clone();
                                update(&mut v);
                                storage::cache_diff_render(&event_id, &fmt_label, &diff);
                                history.set(v);
                            }
                            ViewMode::Stream => {
                                let mut v = (*stream).clone();
                                update(&mut v);
                                storage::save_events(&stream_key, &v);
                                stream.set(v);
                            }
                            ViewMode::Compare => {}
                        }
                    }),
                );
            } else {
                let history = history_events.clone();
                let stream = stream_events.clone();
                let on_unauth = on_unauthorized.clone();
                api::fetch_event_detail(
                    &server,
                    &id_str,
                    Callback::from(move |detail: Option<ChangeEventRow>| {
                        if let Some(row) = detail {
                            if let Some(diff) = &row.diff_render {
                                let update = |events: &mut Vec<RealtimeMessage>| {
                                    if let Some(e) =
                                        events.iter_mut().find(|e| e.event_id == event_id)
                                    {
                                        e.diff_render = Some(diff.clone());
                                    }
                                };
                                match mode {
                                    ViewMode::History => {
                                        let mut v = (*history).clone();
                                        update(&mut v);
                                        history.set(v);
                                    }
                                    ViewMode::Stream => {
                                        let mut v = (*stream).clone();
                                        update(&mut v);
                                        stream.set(v);
                                    }
                                    ViewMode::Compare => {}
                                }
                            }
                        }
                    }),
                    Some(on_unauth),
                );
            }
        })
    };

    // --- Host fetching ---

    let on_hosts_result = {
        let hosts = hosts.clone();
        Callback::from(move |list: Vec<HostInfo>| {
            hosts.set(list);
        })
    };

    let on_roots_result = {
        let watch_roots = watch_roots.clone();
        Callback::from(move |roots: Vec<WatchRootInfo>| {
            watch_roots.set(roots);
        })
    };

    // Clear lazy-fetched set when diff_format changes so expanded events
    // re-fetch with the new format.
    {
        let lazy_fetched = lazy_fetched.clone();
        let diff_format = filters.diff_format.clone();
        use_effect_with(diff_format, move |_| {
            lazy_fetched.borrow_mut().clear();
        });
    }

    // Fetch watch_roots when host filter changes
    {
        let server_url = server_url.clone();
        let filters = filters.clone();
        let on_roots_result = on_roots_result.clone();
        let watch_roots_state = watch_roots.clone();
        let _auth_data = auth_data.clone();
        let on_unauthorized = on_unauthorized.clone();
        use_effect_with(filters.host_id.clone(), move |_| {
            if let Some(ref host_id) = filters.host_id {
                api::fetch_watch_roots(
                    &server_url,
                    host_id,
                    on_roots_result,
                    Some(on_unauthorized),
                );
            } else {
                watch_roots_state.set(Vec::new());
            }
        });
    }

    // --- Callbacks ---

    let on_filter_change = {
        let filters = filters.clone();
        let pagination = pagination.clone();
        Callback::from(move |new_filters: FilterState| {
            filters.set(new_filters);
            pagination.set(PaginationState {
                page: 1,
                page_size: pagination.page_size,
                total: 0,
            });
        })
    };

    let on_connect = {
        let server_url = server_url.clone();
        let filters = filters.clone();
        let on_message = on_message.clone();
        let on_status = on_status.clone();
        let _auth_data = auth_data.clone();
        Callback::from(move |_: ()| {
            let csrf = storage::load_csrf_token();
            ws::connect(
                &server_url,
                &filters,
                csrf.as_deref(),
                on_message.clone(),
                on_status.clone(),
            );
        })
    };

    let on_refresh = {
        let server_url = server_url.clone();
        let filters = filters.clone();
        let loading = loading.clone();
        let on_history_result = on_history_result.clone();
        let pagination = pagination.clone();
        let on_unauthorized = on_unauthorized.clone();
        Callback::from(move |_: ()| {
            loading.set(true);
            let query = filters.to_changes_query_string(&pagination);
            api::fetch_changes(
                &server_url,
                &query,
                on_history_result.clone(),
                Some(on_unauthorized.clone()),
            );
        })
    };

    let on_page_change = {
        let pagination = pagination.clone();
        let server_url = server_url.clone();
        let filters = filters.clone();
        let loading = loading.clone();
        let on_history_result = on_history_result.clone();
        let on_unauthorized = on_unauthorized.clone();
        Callback::from(move |new_page: u32| {
            loading.set(true);
            pagination.set(PaginationState {
                page: new_page,
                page_size: pagination.page_size,
                total: pagination.total,
            });
            let query_paginator = PaginationState {
                page: new_page,
                page_size: pagination.page_size,
                total: pagination.total,
            };
            let query = filters.to_changes_query_string(&query_paginator);
            api::fetch_changes(
                &server_url,
                &query,
                on_history_result.clone(),
                Some(on_unauthorized.clone()),
            );
        })
    };

    let on_page_size_change = {
        let pagination = pagination.clone();
        Callback::from(move |new_size: u32| {
            pagination.set(PaginationState {
                page: 1,
                page_size: new_size,
                total: 0,
            });
        })
    };

    let on_mode_change = {
        let view_mode = view_mode.clone();
        let server_url = server_url.clone();
        let on_hosts_result = on_hosts_result.clone();
        let selected_events = selected_events.clone();
        let on_refresh = on_refresh.clone();
        let lazy_fetched = lazy_fetched.clone();
        let on_unauthorized = on_unauthorized.clone();
        Callback::from(move |mode: ViewMode| {
            view_mode.set(mode);
            selected_events.set(HashSet::new());
            lazy_fetched.borrow_mut().clear();
            api::fetch_hosts(
                &server_url,
                on_hosts_result.clone(),
                Some(on_unauthorized.clone()),
            );
            if mode == ViewMode::History {
                on_refresh.emit(());
            }
        })
    };

    let on_fetch_hosts = {
        let server_url = server_url.clone();
        let on_hosts_result = on_hosts_result.clone();
        let on_unauthorized = on_unauthorized.clone();
        Callback::from(move |_: ()| {
            api::fetch_hosts(
                &server_url,
                on_hosts_result.clone(),
                Some(on_unauthorized.clone()),
            );
        })
    };

    let on_toggle = {
        let expanded = expanded.clone();
        Callback::from(move |event_id: Uuid| {
            let current = *expanded;
            expanded.set(if current == Some(event_id) {
                None
            } else {
                Some(event_id)
            });
        })
    };

    let on_toggle_select = {
        let selected_events = selected_events.clone();
        Callback::from(move |event_id: Uuid| {
            let mut current = (*selected_events).clone();
            if current.contains(&event_id) {
                current.remove(&event_id);
            } else {
                current.insert(event_id);
            }
            selected_events.set(current);
        })
    };

    let _on_select_all = {
        let selected_events = selected_events.clone();
        let active_events = match *view_mode {
            ViewMode::Stream => Rc::new((*stream_events).clone()),
            ViewMode::History => Rc::new((*history_events).clone()),
            ViewMode::Compare => Rc::new(Vec::new()),
        };
        Callback::from(move |_: ()| {
            let all_ids: HashSet<Uuid> = (*active_events).iter().map(|e| e.event_id).collect();
            selected_events.set(all_ids);
        })
    };

    let on_deselect_all = {
        let selected_events = selected_events.clone();
        Callback::from(move |_: ()| {
            selected_events.set(HashSet::new());
        })
    };

    let on_open_workflow = {
        let workflow_panel_open = workflow_panel_open.clone();
        Callback::from(move |_: ()| {
            workflow_panel_open.set(true);
        })
    };

    let on_close_workflow = {
        let workflow_panel_open = workflow_panel_open.clone();
        Callback::from(move |_: ()| {
            workflow_panel_open.set(false);
        })
    };

    let on_pr_created = {
        let on_close_workflow = on_close_workflow.clone();
        let on_deselect_all = on_deselect_all.clone();
        let on_refresh = on_refresh.clone();
        Callback::from(move |_: ()| {
            on_close_workflow.emit(());
            on_deselect_all.emit(());
            on_refresh.emit(());
        })
    };

    let on_clear_storage = {
        let stream_events = stream_events.clone();
        Callback::from(move |_: ()| {
            storage::clear_all_stream_events();
            stream_events.set(Vec::new());
        })
    };

    let on_server_url_change = {
        let server_url = server_url.clone();
        Callback::from(move |e: yew::Event| {
            let input: HtmlInputElement = e.target_unchecked_into();
            server_url.set(input.value());
        })
    };

    let on_logout = {
        let auth_data = auth_data.clone();
        Callback::from(move |_: web_sys::MouseEvent| {
            let auth_data_clone = auth_data.clone();
            crate::auth::sign_out(Callback::from(move |()| {
                auth_data_clone.set(None);
            }));
        })
    };

    // --- Rendering ---

    // If approval is pending, show the pending approval screen
    if *approval_pending {
        let on_back_to_login = {
            let approval_pending = approval_pending.clone();
            Callback::from(move |_| {
                approval_pending.set(false);
            })
        };
        return html! {
            <div class="login-container">
                <div class="login-card">
                    <h1 class="login-title">{ "Config Watch" }</h1>
                    <h2 class="login-subtitle">{ "Account Pending Approval" }</h2>
                    <p class="login-error" style="background: var(--bg-card, #1e1e2e); color: var(--text-muted, #888);">
                        { "Your account has been created and is awaiting admin approval. You will be able to sign in once an administrator approves your account." }
                    </p>
                    <button class="login-submit" onclick={on_back_to_login}>
                        { "Back to Sign In" }
                    </button>
                </div>
            </div>
        };
    }

    // If not authenticated, show the login form
    if auth_data.is_none() {
        let on_login_success = {
            let auth_data = auth_data.clone();
            Callback::from(move |data: crate::storage::AuthData| {
                storage::save_auth_data(&data);
                auth_data.set(Some(data));
            })
        };
        let on_approval_pending = {
            let approval_pending = approval_pending.clone();
            Callback::from(move |()| {
                approval_pending.set(true);
            })
        };
        return html! {
            <Login on_success={on_login_success} on_approval_pending={on_approval_pending} />
        };
    }

    let status_indicator = match &*connection {
        ConnectionStatus::Connected => html! {
            <span class="status-dot status-connected"></span>
        },
        ConnectionStatus::Connecting => html! {
            <span class="status-dot status-connecting"></span>
        },
        ConnectionStatus::Disconnected => html! {
            <span class="status-dot status-disconnected"></span>
        },
        ConnectionStatus::Error(msg) => html! {
            <span class="status-dot status-error" title={msg.clone()}></span>
        },
    };

    let status_text = match &*connection {
        ConnectionStatus::Connected => "Connected",
        ConnectionStatus::Connecting => "Connecting...",
        ConnectionStatus::Disconnected => "Disconnected",
        ConnectionStatus::Error(_) => "Error",
    };

    let mode_label = match *view_mode {
        ViewMode::Stream => "Stream",
        ViewMode::History => "History",
        ViewMode::Compare => "Compare",
    };

    let active_events: Rc<Vec<RealtimeMessage>> = match *view_mode {
        ViewMode::Stream => Rc::new((*stream_events).clone()),
        ViewMode::History => Rc::new((*history_events).clone()),
        ViewMode::Compare => Rc::new(Vec::new()),
    };

    // History view loads events from the DB without their diff_render (or with
    // a stale agent-rendered one); expanding triggers a fetch. Stream view
    // gets diff_render via WebSocket so no on-expand fetch is needed.
    let fetch_on_expand = *view_mode == ViewMode::History;
    let event_count = active_events.len();

    let empty_msg = match *view_mode {
        ViewMode::Stream => {
            "Configure filters and connect to start receiving real-time change events."
        }
        ViewMode::History => "Click Fetch to load change events from the database.",
        ViewMode::Compare => "Use the controls above to compare files across agents.",
    };

    let user_email = auth_data.as_ref().and_then(|d| d.email.clone()).unwrap_or_default();

    html! {
        <div class="app-layout">
            <header class="app-header">
                <div class="header-left">
                    <h1>{"Config Watch"}</h1>
                    <span class="header-subtitle">{"Real-time Configuration Change Monitor"}</span>
                </div>
                <div class="header-right">
                    if *view_mode == ViewMode::Stream {
                        <div class="connection-info">
                            { status_indicator }
                            <span class="status-text">{ status_text }</span>
                        </div>
                    }
                    <div class="mode-badge">{ mode_label }</div>
                    <div class="server-url-input">
                        <label for="server-url">{"Server:"}</label>
                        <input
                            id="server-url"
                            type="text"
                            value={(*server_url).clone()}
                            onchange={on_server_url_change}
                            placeholder="localhost:8082"
                        />
                    </div>
                    <div class="user-info">
                        <span class="user-email">{ &user_email }</span>
                        <button class="logout-btn" onclick={on_logout}>{"Sign out"}</button>
                    </div>
                </div>
            </header>
            <div class="app-body">
                <aside class="sidebar">
                    <FilterBar
                        filters={(*filters).clone()}
                        hosts={Rc::new((*hosts).clone())}
                        watch_roots={Rc::new((*watch_roots).clone())}
                        view_mode={*view_mode}
                        page_size={pagination.page_size}
                        on_change={on_filter_change}
                        on_connect={on_connect}
                        on_refresh={on_refresh}
                        on_mode_change={on_mode_change}
                        on_fetch_hosts={on_fetch_hosts.clone()}
                        on_page_size_change={on_page_size_change}
                    />
                    <div class="sidebar-stats">
                        <div class="stat-row">
                            <span class="stat-label">{"Events"}</span>
                            <span class="stat-value">{ event_count }</span>
                        </div>
                        if *view_mode == ViewMode::History && *loading {
                            <div class="stat-row">
                                <span class="stat-label">{"Status"}</span>
                                <span class="stat-value stat-loading">{"Loading..."}</span>
                            </div>
                        }
                        <button class="clear-cache-btn" onclick={{
                            let on_clear = on_clear_storage.clone();
                            move |_| on_clear.emit(())
                        }}>{"Clear Cache"}</button>
                    </div>
                </aside>
                <main ref={main_ref} class="main-content">
                    { match *view_mode {
                        ViewMode::Stream | ViewMode::History => {
                            if active_events.is_empty() {
                                html! {
                                    <div class="empty-state">
                                        <h2>{"No changes yet"}</h2>
                                        <p>{ empty_msg }</p>
                                    </div>
                                }
                            } else {
                                html! {
                                    <EventList
                                        events={active_events.clone()}
                                        expanded_id={*expanded}
                                        on_toggle={on_toggle}
                                        fetch_on_expand={fetch_on_expand}
                                        on_fetch_diff={on_fetch_diff}
                                        selected_events={Rc::new((*selected_events).clone())}
                                        on_toggle_select={on_toggle_select.clone()}
                                        pagination={(*pagination).clone()}
                                        on_page_change={on_page_change.clone()}
                                        view_mode={*view_mode}
                                        hosts={Rc::new((*hosts).clone())}
                                    />
                                }
                            }
                        }
                        ViewMode::Compare => html! {
                            <FileCompare
                                hosts={Rc::new((*hosts).clone())}
                                server_url={(*server_url).clone()}
                                csrf_token={csrf_token.clone()}
                                on_fetch_hosts={on_fetch_hosts.clone()}
                            />
                        },
                    }}
                </main>
                if *view_mode == ViewMode::History && !(*selected_events).is_empty() {
                    <SelectionBar
                        count={(*selected_events).len()}
                        pr_disabled={(*active_events).iter()
                            .filter(|e| (*selected_events).contains(&e.event_id))
                            .any(|e| e.pr_url.is_some())}
                        on_open_workflow={on_open_workflow.clone()}
                        on_deselect_all={on_deselect_all.clone()}
                    />
                }
            </div>
            if *workflow_panel_open {
                <WorkflowPanel
                    events={active_events.clone()}
                    selected_events={Rc::new((*selected_events).clone())}
                    server_url={(*server_url).clone()}
                    csrf_token={csrf_token.clone()}
                    on_close={on_close_workflow}
                    on_pr_created={on_pr_created.clone()}
                />
            }
        </div>
    }
}