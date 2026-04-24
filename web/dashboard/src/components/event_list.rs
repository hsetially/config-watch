use std::collections::HashSet;
use std::rc::Rc;
use yew::{function_component, html, Callback, Html, Properties};

use crate::components::diff_viewer::DiffViewer;
use crate::models::{
    event_kind_icon, format_event_time, severity_class, severity_tooltip, HostInfo,
    PaginationState, RealtimeMessage, ViewMode,
};

#[derive(Properties, PartialEq)]
pub struct EventListProps {
    pub events: Rc<Vec<RealtimeMessage>>,
    pub expanded_id: Option<uuid::Uuid>,
    pub on_toggle: Callback<uuid::Uuid>,
    #[prop_or_default]
    pub lazy_diff: bool,
    #[prop_or_default]
    pub on_fetch_diff: Callback<uuid::Uuid>,
    #[prop_or_default]
    pub selected_events: Rc<HashSet<uuid::Uuid>>,
    #[prop_or_default]
    pub on_toggle_select: Callback<uuid::Uuid>,
    #[prop_or_default]
    pub pagination: PaginationState,
    #[prop_or_default]
    pub on_page_change: Callback<u32>,
    #[prop_or_default]
    pub view_mode: ViewMode,
    #[prop_or_default]
    pub hosts: Rc<Vec<HostInfo>>,
}

#[function_component(EventList)]
pub fn event_list(props: &EventListProps) -> Html {
    let total_pages = props.pagination.total_pages();
    let current_page = props.pagination.page;
    let show_pagination = props.view_mode == ViewMode::History && total_pages > 0;

    html! {
        <div class="event-list">
            { for props.events.iter().map(|event| {
                render_event(event, props.expanded_id, &props.on_toggle, props.lazy_diff, &props.on_fetch_diff, &props.selected_events, &props.on_toggle_select, &props.hosts)
            })}
            if show_pagination {
                <div class="pagination">
                    <button
                        class="pagination-btn"
                        disabled={current_page <= 1}
                        onclick={{
                            let on_page_change = props.on_page_change.clone();
                            move |_| on_page_change.emit(current_page - 1)
                        }}
                    >
                        {"Previous"}
                    </button>
                    <span class="pagination-info">
                        {format!("Page {} of {}", current_page, total_pages)}
                    </span>
                    <button
                        class="pagination-btn"
                        disabled={current_page >= total_pages}
                        onclick={{
                            let on_page_change = props.on_page_change.clone();
                            move |_| on_page_change.emit(current_page + 1)
                        }}
                    >
                        {"Next"}
                    </button>
                </div>
            }
        </div>
    }
}

#[allow(clippy::too_many_arguments)]
fn render_event(
    event: &RealtimeMessage,
    expanded_id: Option<uuid::Uuid>,
    on_toggle: &Callback<uuid::Uuid>,
    lazy_diff: bool,
    on_fetch_diff: &Callback<uuid::Uuid>,
    selected_events: &HashSet<uuid::Uuid>,
    on_toggle_select: &Callback<uuid::Uuid>,
    hosts: &Rc<Vec<HostInfo>>,
) -> Html {
    let is_expanded = expanded_id == Some(event.event_id);
    let is_selected = selected_events.contains(&event.event_id);
    let sev_class = severity_class(&event.severity);
    let icon = event_kind_icon(&event.event_kind);
    let time = format_event_time(&event.event_time);
    let toggle = on_toggle.clone();
    let toggle2 = on_toggle.clone();
    let toggle3 = on_toggle.clone();
    let event_id = event.event_id;

    let select_cb = on_toggle_select.clone();
    let select_id = event.event_id;

    let summary_text = event
        .summary
        .as_ref()
        .map(|s| format!("{} lines changed", s.changed_line_estimate))
        .unwrap_or_else(|| "No summary".to_string());

    let author = event.author_display.as_deref().unwrap_or("unknown");
    let host_display = hosts
        .iter()
        .find(|h| h.host_id == event.host_id)
        .map(|h| h.hostname.clone())
        .unwrap_or_else(|| event.host_id.to_string());

    // Extract file name from path for the primary display
    let file_name = event
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&event.path)
        .to_string();

    // When expanding a lazy-diff event that has no diff_render, fetch it
    if is_expanded && lazy_diff && event.diff_render.is_none() {
        on_fetch_diff.emit(event_id);
    }

    html! {
        <div class="event-card" key={event.event_id.to_string()}>
            <div class={format!("event-header {}", sev_class)}>
                <div class="event-select">
                    <input
                        type="checkbox"
                        checked={is_selected}
                        onclick={move |e: yew::MouseEvent| {
                            e.stop_propagation();
                            select_cb.emit(select_id);
                        }}
                    />
                </div>
                <div class="event-header-left" onclick={move |_| toggle.emit(event_id)}>
                    <span class="event-icon">{ icon }</span>
                    <span class="event-file-name">{ file_name }</span>
                    <span class={format!("event-severity {}", sev_class)} data-tooltip={severity_tooltip(&event.severity, &event.summary)}>{ &event.severity }</span>
                    {
                        if let Some(ref pr_url) = event.pr_url {
                            let num = event.pr_number.map(|n| format!("#{}", n)).unwrap_or_else(|| "PR".to_string());
                            html! {
                                <a class="event-pr-badge" href={pr_url.clone()} target="_blank"
                                   onclick={|e: yew::MouseEvent| e.stop_propagation()}>
                                    { num }
                                </a>
                            }
                        } else {
                            html! {}
                        }
                    }
                </div>
                <div class="event-header-right" onclick={move |_| toggle2.emit(event_id)}>
                    <span class="event-time">{ time }</span>
                </div>
            </div>
            <div class="event-meta" onclick={move |_| toggle3.emit(event_id)}>
                <span class="event-path-full">{ &event.path }</span>
                <span class="event-kind-label">{ &event.event_kind }</span>
                <span class="event-author">{ format!("by {}", author) }</span>
                <span class="event-env">{ &event.environment }</span>
                <span class="event-host">{ format!("host: {}", host_display) }</span>
                <span class="event-summary">{ summary_text }</span>
            </div>
            { if is_expanded {
                render_expanded_content(event, lazy_diff)
            } else {
                html! {}
            }}
        </div>
    }
}

fn render_expanded_content(event: &RealtimeMessage, lazy_diff: bool) -> Html {
    match &event.diff_render {
        Some(diff) if !diff.is_empty() => html! {
            <div class="event-diff">
                <DiffViewer diff_text={diff.clone()} />
            </div>
        },
        _ if lazy_diff => html! {
            <div class="event-no-diff">
                <p>{"Loading diff..."}</p>
            </div>
        },
        _ => html! {
            <div class="event-no-diff">
                <p>{"No diff available for this event."}</p>
            </div>
        },
    }
}
