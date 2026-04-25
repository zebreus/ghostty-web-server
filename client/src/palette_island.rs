//! Command palette (Cmd/Ctrl+K). Mirrors `src/islands/PaletteIsland.tsx`.

use std::rc::Rc;

use gloo_events::EventListener;
use gloo_timers::callback::Interval;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{HtmlInputElement, KeyboardEvent};
use yew::prelude::*;

use crate::palette::{load_commands, Command};

#[function_component(PaletteIsland)]
pub fn palette_island() -> Html {
    let open = use_state(|| false);
    let commands: UseStateHandle<Vec<Command>> = use_state(Vec::new);
    let query = use_state(String::new);
    let selected = use_state(|| 0usize);
    let input_ref = use_node_ref();

    // Cmd/Ctrl+K toggles, Esc closes — capture phase so the terminal
    // doesn't swallow the keystroke.
    {
        let open = open.clone();
        use_effect_with((), move |_| {
            let document = web_sys::window().unwrap().document().unwrap();
            let target: web_sys::EventTarget = document.into();
            let open_for_listener = open.clone();
            let listener = EventListener::new_with_options(
                &target,
                "keydown",
                gloo_events::EventListenerOptions::enable_prevent_default(),
                move |e| {
                    let ke: &KeyboardEvent = e.dyn_ref().unwrap();
                    if (ke.meta_key() || ke.ctrl_key()) && ke.key() == "k" {
                        ke.prevent_default();
                        ke.stop_propagation();
                        open_for_listener.set(!*open_for_listener);
                    } else if ke.key() == "Escape" && *open_for_listener {
                        ke.prevent_default();
                        ke.stop_propagation();
                        open_for_listener.set(false);
                    }
                },
            );
            move || drop(listener)
        });
    }

    // Load + poll commands while open.
    {
        let open = open.clone();
        let commands = commands.clone();
        let query = query.clone();
        let selected = selected.clone();
        use_effect_with(*open, move |is_open| {
            let cleanup: Box<dyn FnOnce()> = if *is_open {
                query.set(String::new());
                selected.set(0);
                let cancelled = Rc::new(std::cell::Cell::new(false));
                let refresh = {
                    let commands = commands.clone();
                    let cancelled = cancelled.clone();
                    move || {
                        let commands = commands.clone();
                        let cancelled = cancelled.clone();
                        spawn_local(async move {
                            let list = load_commands().await;
                            if !cancelled.get() {
                                commands.set(list);
                            }
                        });
                    }
                };
                refresh();
                let interval = {
                    let refresh = refresh.clone();
                    Interval::new(2000, move || refresh())
                };
                Box::new(move || {
                    cancelled.set(true);
                    drop(interval);
                })
            } else {
                Box::new(|| {})
            };
            let _ = open;
            cleanup
        });
    }

    // Focus the search box on open.
    {
        let open = open.clone();
        let input_ref = input_ref.clone();
        use_effect_with(*open, move |is_open| {
            if *is_open {
                if let Some(el) = input_ref.cast::<HtmlInputElement>() {
                    let _ = el.focus();
                }
            }
            || ()
        });
    }

    if !*open {
        return Html::default();
    }

    let q = query.trim().to_lowercase();
    let filtered: Vec<Command> = if q.is_empty() {
        (*commands).clone()
    } else {
        commands
            .iter()
            .filter(|c| {
                let mut hay = c.label.to_lowercase();
                if let Some(d) = &c.detail {
                    hay.push(' ');
                    hay.push_str(&d.to_lowercase());
                }
                if let Some(h) = &c.hint {
                    hay.push(' ');
                    hay.push_str(&h.to_lowercase());
                }
                if let Some(g) = &c.group {
                    hay.push(' ');
                    hay.push_str(&g.to_lowercase());
                }
                hay.contains(&q)
            })
            .cloned()
            .collect()
    };

    // Clamp selection.
    {
        let selected = selected.clone();
        let len = filtered.len();
        let cur = *selected;
        if cur >= len && len > 0 {
            selected.set(len - 1);
        } else if len == 0 && cur != 0 {
            selected.set(0);
        }
    }

    let move_sel = {
        let selected = selected.clone();
        let filtered_len = filtered.len();
        let filtered_for_move = filtered.clone();
        Rc::new(move |delta: i32| {
            if filtered_len == 0 {
                return;
            }
            let mut next = *selected as i32;
            for _ in 0..filtered_len {
                next = (next + delta + filtered_len as i32) % filtered_len as i32;
                if !filtered_for_move[next as usize].disabled {
                    break;
                }
            }
            selected.set(next as usize);
        })
    };

    let invoke = {
        let open = open.clone();
        Rc::new(move |cmd: Option<Command>| {
            let Some(cmd) = cmd else { return };
            if cmd.disabled {
                return;
            }
            open.set(false);
            (cmd.on_select)();
        })
    };

    let on_input_key = {
        let move_sel = move_sel.clone();
        let invoke = invoke.clone();
        let filtered = filtered.clone();
        let selected = selected.clone();
        Callback::from(move |e: KeyboardEvent| match e.key().as_str() {
            "ArrowDown" => {
                e.prevent_default();
                move_sel(1);
            }
            "ArrowUp" => {
                e.prevent_default();
                move_sel(-1);
            }
            "Enter" => {
                e.prevent_default();
                let cmd = filtered.get(*selected).cloned();
                invoke(cmd);
            }
            _ => {}
        })
    };

    let on_input = {
        let query = query.clone();
        let selected = selected.clone();
        Callback::from(move |e: InputEvent| {
            let target: HtmlInputElement = e.target().unwrap().dyn_into().unwrap();
            query.set(target.value());
            selected.set(0);
        })
    };

    let close = {
        let open = open.clone();
        Callback::from(move |_| open.set(false))
    };
    let stop_click = Callback::from(|e: MouseEvent| e.stop_propagation());

    // Group rows for rendering while preserving global selection index.
    let mut groups: Vec<(String, Vec<(usize, Command)>)> = Vec::new();
    for (idx, cmd) in filtered.iter().enumerate() {
        let label = cmd.group.clone().unwrap_or_default();
        if let Some(b) = groups.iter_mut().find(|(l, _)| *l == label) {
            b.1.push((idx, cmd.clone()));
        } else {
            groups.push((label, vec![(idx, cmd.clone())]));
        }
    }

    let groups_html = if groups.is_empty() {
        html! { <div class="ghostty-palette-empty">{ "No matches" }</div> }
    } else {
        groups
            .into_iter()
            .map(|(label, items)| {
                let items_html = items.into_iter().map(|(idx, cmd)| {
                    let mut cls = String::from("ghostty-palette-row");
                    if idx == *selected { cls.push_str(" ghostty-palette-row-selected"); }
                    if cmd.disabled { cls.push_str(" ghostty-palette-row-disabled"); }
                    let on_enter = {
                        let selected = selected.clone();
                        let disabled = cmd.disabled;
                        Callback::from(move |_| {
                            if !disabled { selected.set(idx); }
                        })
                    };
                    let on_click_row = {
                        let invoke = invoke.clone();
                        let cmd = cmd.clone();
                        Callback::from(move |_| invoke(Some(cmd.clone())))
                    };
                    let detail = cmd.detail.clone();
                    let hint = cmd.hint.clone();
                    html! {
                        <div key={cmd.id.clone()} class={cls}
                             onmouseenter={on_enter}
                             onclick={on_click_row}>
                            <span class="ghostty-palette-label">{ &cmd.label }</span>
                            { detail.map(|d| html!{ <span class="ghostty-palette-detail">{d}</span> }).unwrap_or_default() }
                            { hint.map(|h| html!{ <span class="ghostty-palette-hint-cell">{h}</span> }).unwrap_or_default() }
                        </div>
                    }
                }).collect::<Html>();
                html! {
                    <div key={label.clone()} class="ghostty-palette-group">
                        { if !label.is_empty() { html!{ <div class="ghostty-palette-group-label">{label.clone()}</div> } } else { Html::default() } }
                        { items_html }
                    </div>
                }
            })
            .collect::<Html>()
    };

    html! {
        <div class="ghostty-palette" onclick={close}>
            <div class="ghostty-palette-card" onclick={stop_click}>
                <div class="ghostty-palette-search">
                    <span class="ghostty-palette-search-icon">{ "›" }</span>
                    <input
                        ref={input_ref}
                        type="text"
                        placeholder="Search commands…"
                        value={(*query).clone()}
                        oninput={on_input}
                        onkeydown={on_input_key}
                        spellcheck="false"
                        autocomplete="off"
                    />
                </div>
                <div class="ghostty-palette-list">
                    { groups_html }
                </div>
                <div class="ghostty-palette-footer">
                    <span>{ "↑↓ navigate" }</span>
                    <span>{ "⏎ select" }</span>
                    <span>{ "esc close" }</span>
                </div>
            </div>
        </div>
    }
}
