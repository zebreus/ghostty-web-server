//! Yew client entrypoint. Mounts the three islands into the static shell.

mod bridge;
mod ghostty;
mod palette;
mod palette_island;
mod protocol;
mod session_commands;
mod settings;
mod status;
mod status_island;
mod terminal_island;

use wasm_bindgen::JsCast;
use yew::prelude::*;

use crate::palette_island::PaletteIsland;
use crate::status_island::StatusIsland;
use crate::terminal_island::TerminalIsland;

fn main() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Info);

    // Self-register session/action providers (matches the import-side-effect
    // of `import './session-commands'` in the original client.tsx).
    session_commands::register_all();

    let document = web_sys::window().unwrap().document().unwrap();

    if let Some(el) = document
        .get_element_by_id("root")
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        yew::Renderer::<TerminalIsland>::with_root(el.into()).render();
    }
    if let Some(el) = document
        .get_element_by_id("palette-root")
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        yew::Renderer::<PaletteIsland>::with_root(el.into()).render();
    }
    if let Some(el) = document
        .get_element_by_id("status-root")
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        yew::Renderer::<StatusIsland>::with_root(el.into()).render();
    }
}
