//! Connection-status overlay. Mirrors `src/islands/StatusIsland.tsx`.

use yew::prelude::*;

use crate::status::{subscribe, Status, Unsubscribe};

#[function_component(StatusIsland)]
pub fn status_island() -> Html {
    let status = use_state(Status::default);

    {
        let status = status.clone();
        use_effect_with((), move |_| {
            let cb_status = status.clone();
            let unsub: Unsubscribe = subscribe(move |s| cb_status.set(s.clone()));
            move || drop(unsub)
        });
    }

    if !status.is_overlay_visible() {
        return Html::default();
    }

    html! {
        <div class="ghostty-status" role="status" aria-live="polite">
            <div class="ghostty-status-card">
                { render_card(&status) }
            </div>
        </div>
    }
}

fn render_card(s: &Status) -> Html {
    match s {
        Status::Connecting => html! {
            <>
                <Spinner />
                <span class="ghostty-status-label">{ "Connecting…" }</span>
            </>
        },
        Status::Reconnecting { remaining } => html! {
            <>
                <Spinner />
                <span class="ghostty-status-label">
                    { format!("Reconnecting in {remaining}s…") }
                </span>
            </>
        },
        Status::Busy { on_take } => {
            let on_click = {
                let on_take = on_take.clone();
                Callback::from(move |_| on_take.emit(()))
            };
            html! {
                <>
                    <Icon kind={IconKind::Warn} />
                    <div class="ghostty-status-text">
                        <strong>{ "Session attached in another tab" }</strong>
                        <span>{ "Take it over to continue here." }</span>
                    </div>
                    <button class="ghostty-status-action" onclick={on_click}>
                        { "Take session" }
                    </button>
                </>
            }
        }
        Status::Error { message } => html! {
            <>
                <Icon kind={IconKind::Error} />
                <span class="ghostty-status-label">{ message }</span>
            </>
        },
        // Idle / Connected branches are filtered above.
        _ => Html::default(),
    }
}

#[function_component(Spinner)]
fn spinner() -> Html {
    html! {
        <svg class="ghostty-spinner" viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="12" cy="12" r="9" />
        </svg>
    }
}

#[derive(Clone, PartialEq)]
enum IconKind {
    Warn,
    Error,
}

#[derive(Properties, PartialEq)]
struct IconProps {
    kind: IconKind,
}

#[function_component(Icon)]
fn icon(props: &IconProps) -> Html {
    let cls = match props.kind {
        IconKind::Warn => "ghostty-icon ghostty-icon-warn",
        IconKind::Error => "ghostty-icon ghostty-icon-error",
    };
    html! {
        <svg class={cls} viewBox="0 0 24 24" aria-hidden="true">
            { match props.kind {
                IconKind::Warn => html! {
                    <path d="M12 3 L22 20 L2 20 Z M12 10 V14 M12 17 V17.01" />
                },
                IconKind::Error => html! {
                    <>
                        <circle cx="12" cy="12" r="9" />
                        <path d="M8 8 L16 16 M16 8 L8 16" />
                    </>
                },
            } }
        </svg>
    }
}
