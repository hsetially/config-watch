use std::rc::Rc;
use yew::{function_component, html, Callback, Html, Properties, TargetCast};
use web_sys::{Event, HtmlInputElement, HtmlSelectElement};

use crate::models::{FilterState, HostInfo, ViewMode, WatchRootInfo};

#[derive(Properties, PartialEq)]
pub struct FilterBarProps {
    pub filters: FilterState,
    pub hosts: Rc<Vec<HostInfo>>,
    pub watch_roots: Rc<Vec<WatchRootInfo>>,
    pub view_mode: ViewMode,
    pub page_size: u32,
    pub on_change: Callback<FilterState>,
    pub on_connect: Callback<()>,
    pub on_refresh: Callback<()>,
    pub on_mode_change: Callback<ViewMode>,
    pub on_fetch_hosts: Callback<()>,
    pub on_page_size_change: Callback<u32>,
}

#[function_component(FilterBar)]
pub fn filter_bar(props: &FilterBarProps) -> Html {
    let on_env_change = {
        let on_change = props.on_change.clone();
        let filters = props.filters.clone();
        Callback::from(move |e: Event| {
            let input: HtmlInputElement = e.target_unchecked_into();
            let mut new_filters = filters.clone();
            let val = input.value();
            new_filters.environment = if val.is_empty() { None } else { Some(val) };
            on_change.emit(new_filters);
        })
    };

    let on_host_change = {
        let on_change = props.on_change.clone();
        let filters = props.filters.clone();
        Callback::from(move |e: Event| {
            let select: HtmlSelectElement = e.target_unchecked_into();
            let mut new_filters = filters.clone();
            let val = select.value();
            new_filters.host_id = if val.is_empty() { None } else { Some(val) };
            on_change.emit(new_filters);
        })
    };

    let on_path_change = {
        let on_change = props.on_change.clone();
        let filters = props.filters.clone();
        Callback::from(move |e: Event| {
            let input: HtmlInputElement = e.target_unchecked_into();
            let mut new_filters = filters.clone();
            let val = input.value();
            new_filters.path_prefix = if val.is_empty() { None } else { Some(val) };
            on_change.emit(new_filters);
        })
    };

    let on_filename_change = {
        let on_change = props.on_change.clone();
        let filters = props.filters.clone();
        Callback::from(move |e: Event| {
            let input: HtmlInputElement = e.target_unchecked_into();
            let mut new_filters = filters.clone();
            let val = input.value();
            new_filters.filename = if val.is_empty() { None } else { Some(val) };
            on_change.emit(new_filters);
        })
    };

    let on_severity_change = {
        let on_change = props.on_change.clone();
        let filters = props.filters.clone();
        Callback::from(move |e: Event| {
            let select: HtmlSelectElement = e.target_unchecked_into();
            let mut new_filters = filters.clone();
            let val = select.value();
            new_filters.severity = if val.is_empty() { None } else { Some(val) };
            on_change.emit(new_filters);
        })
    };

    let on_since_change = {
        let on_change = props.on_change.clone();
        let filters = props.filters.clone();
        Callback::from(move |e: Event| {
            let input: HtmlInputElement = e.target_unchecked_into();
            let mut new_filters = filters.clone();
            let val = input.value();
            new_filters.since = if val.is_empty() { None } else { Some(val) };
            on_change.emit(new_filters);
        })
    };

    let on_until_change = {
        let on_change = props.on_change.clone();
        let filters = props.filters.clone();
        Callback::from(move |e: Event| {
            let input: HtmlInputElement = e.target_unchecked_into();
            let mut new_filters = filters.clone();
            let val = input.value();
            new_filters.until = if val.is_empty() { None } else { Some(val) };
            on_change.emit(new_filters);
        })
    };

    let on_page_size_change = {
        let on_change = props.on_page_size_change.clone();
        Callback::from(move |e: Event| {
            let select: HtmlSelectElement = e.target_unchecked_into();
            if let Ok(size) = select.value().parse::<u32>() {
                on_change.emit(size);
            }
        })
    };

    let on_root_change = {
        let on_change = props.on_change.clone();
        let filters = props.filters.clone();
        Callback::from(move |e: Event| {
            let select: HtmlSelectElement = e.target_unchecked_into();
            let mut new_filters = filters.clone();
            let val = select.value();
            new_filters.path_prefix = if val.is_empty() { None } else { Some(val) };
            on_change.emit(new_filters);
        })
    };

    let on_connect = props.on_connect.clone();
    let on_refresh = props.on_refresh.clone();
    let on_fetch_hosts = props.on_fetch_hosts.clone();

    let on_mode_stream = {
        let on_mode = props.on_mode_change.clone();
        Callback::from(move |_: ()| on_mode.emit(ViewMode::Stream))
    };

    let on_mode_history = {
        let on_mode = props.on_mode_change.clone();
        Callback::from(move |_: ()| on_mode.emit(ViewMode::History))
    };

    let on_mode_compare = {
        let on_mode = props.on_mode_change.clone();
        Callback::from(move |_: ()| on_mode.emit(ViewMode::Compare))
    };

    let selected_host_id = props.filters.host_id.clone().unwrap_or_default();
    let selected_host_id_for_compare = selected_host_id.clone();

    let stream_active = props.view_mode == ViewMode::Stream;
    let history_active = props.view_mode == ViewMode::History;
    let compare_active = props.view_mode == ViewMode::Compare;

    let action_label = match props.view_mode {
        ViewMode::Stream => "Connect",
        ViewMode::History => "Fetch",
        ViewMode::Compare => "",
    };

    let action_callback = match props.view_mode {
        ViewMode::Stream => on_connect.clone(),
        ViewMode::History => on_refresh.clone(),
        ViewMode::Compare => on_connect.clone(),
    };

    // Build path prefix datalist options from watch_roots
    let path_prefix_value = props.filters.path_prefix.clone().unwrap_or_default();
    let has_roots = !props.watch_roots.is_empty();

    html! {
        <div class="filter-bar">
            // Mode toggle
            <div class="filter-title">{"Mode"}</div>
            <div class="mode-toggle">
                <button
                    class={if stream_active { "mode-btn mode-btn-active" } else { "mode-btn" }}
                    onclick={move |_| on_mode_stream.emit(())}
                >
                    {"Stream"}
                </button>
                <button
                    class={if history_active { "mode-btn mode-btn-active" } else { "mode-btn" }}
                    onclick={move |_| on_mode_history.emit(())}
                >
                    {"History"}
                </button>
                <button
                    class={if compare_active { "mode-btn mode-btn-active" } else { "mode-btn" }}
                    onclick={move |_| on_mode_compare.emit(())}
                >
                    {"Compare"}
                </button>
            </div>

            if !compare_active {
                <div class="filter-title" style="margin-top: 8px">{"Filters"}</div>
                <div class="filter-group">
                    <label for="filter-env">{"Environment"}</label>
                    <input
                        id="filter-env"
                        type="text"
                        placeholder="e.g. production"
                        value={props.filters.environment.clone().unwrap_or_default()}
                        onchange={on_env_change}
                    />
                </div>
                <div class="filter-group">
                    <div class="filter-group-header">
                        <label for="filter-host">{"Host"}</label>
                        <button class="link-btn" onclick={move |_| on_fetch_hosts.emit(())}>{"reload"}</button>
                    </div>
                    <select
                        id="filter-host"
                        value={selected_host_id}
                        onchange={on_host_change}
                    >
                        <option value="">{"All hosts"}</option>
                        { for props.hosts.iter().map(|h| {
                            let val = h.host_id.to_string();
                            let label = format!("{} ({})", h.hostname, h.status);
                            let selected = val == selected_host_id_for_compare;
                            html! { <option value={val} selected={selected}>{ label }</option> }
                        })}
                    </select>
                </div>
                if props.view_mode == ViewMode::Stream && props.filters.host_id.is_some() && !props.watch_roots.is_empty() {
                    <div class="filter-group">
                        <label for="filter-root">{"Watch Root"}</label>
                        <select
                            id="filter-root"
                            value={props.filters.path_prefix.clone().unwrap_or_default()}
                            onchange={on_root_change}
                        >
                            <option value="">{"All roots"}</option>
                            { for props.watch_roots.iter().filter(|r| r.active).map(|r| {
                                let selected = props.filters.path_prefix.as_deref() == Some(r.root_path.as_str());
                                html! { <option value={r.root_path.clone()} selected={selected}>{ r.root_path.clone() }</option> }
                            })}
                        </select>
                    </div>
                }
                <div class="filter-group">
                    <label for="filter-path">{"Path Prefix"}</label>
                    <input
                        id="filter-path"
                        type="text"
                        placeholder={if has_roots { "Type or select a watch root..." } else { "/etc/app/" }}
                        value={path_prefix_value}
                        onchange={on_path_change}
                        list="watch-roots-list"
                    />
                    if has_roots {
                        <datalist id="watch-roots-list">
                            { for props.watch_roots.iter().map(|r| {
                                html! { <option value={r.root_path.clone()} /> }
                            })}
                        </datalist>
                    }
                </div>
                <div class="filter-group">
                    <label for="filter-filename">{"File Name"}</label>
                    <input
                        id="filter-filename"
                        type="text"
                        placeholder="e.g. config.yaml"
                        value={props.filters.filename.clone().unwrap_or_default()}
                        onchange={on_filename_change}
                    />
                </div>
                <div class="filter-group">
                    <label for="filter-severity">{"Severity"}</label>
                    <select
                        id="filter-severity"
                        value={props.filters.severity.clone().unwrap_or_default()}
                        onchange={on_severity_change}
                    >
                        <option value="">{"All"}</option>
                        <option value="info">{"Info"}</option>
                        <option value="critical">{"Critical"}</option>
                    </select>
                </div>
                if props.view_mode == ViewMode::History {
                    <div class="filter-group">
                        <label for="filter-since">{"From"}</label>
                        <input
                            id="filter-since"
                            type="datetime-local"
                            value={props.filters.since.clone().unwrap_or_default()}
                            onchange={on_since_change}
                        />
                    </div>
                    <div class="filter-group">
                        <label for="filter-until">{"To"}</label>
                        <input
                            id="filter-until"
                            type="datetime-local"
                            value={props.filters.until.clone().unwrap_or_default()}
                            onchange={on_until_change}
                        />
                    </div>
                    <div class="filter-group">
                        <label for="filter-page-size">{"Per Page"}</label>
                        <select
                            id="filter-page-size"
                            value={props.page_size.to_string()}
                            onchange={on_page_size_change}
                        >
                            <option value="10">{"10"}</option>
                            <option value="25">{"25"}</option>
                            <option value="50">{"50"}</option>
                            <option value="100">{"100"}</option>
                        </select>
                    </div>
                }
                <button class="connect-btn" onclick={move |_| action_callback.emit(())}>
                    { action_label }
                </button>
            }
        </div>
    }
}