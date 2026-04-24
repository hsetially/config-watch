use yew::{function_component, html, Callback, Html, Properties};

#[derive(Properties, PartialEq)]
pub struct SelectionBarProps {
    pub count: usize,
    pub pr_disabled: bool,
    pub on_open_workflow: Callback<()>,
    pub on_deselect_all: Callback<()>,
}

#[function_component(SelectionBar)]
pub fn selection_bar(props: &SelectionBarProps) -> Html {
    let on_open = props.on_open_workflow.clone();
    let on_clear = props.on_deselect_all.clone();

    html! {
        <div class="selection-bar">
            <span class="selection-count">{ format!("{} files selected", props.count) }</span>
            <button
                class="selection-action-btn"
                disabled={props.pr_disabled}
                onclick={move |_| on_open.emit(())}
            >
                { if props.pr_disabled { "PR already exists" } else { "Create PR" } }
            </button>
            <button class="selection-clear-btn" onclick={move |_| on_clear.emit(())}>
                {"Clear"}
            </button>
        </div>
    }
}
