//! Wires browser events to PTY input (and PTY events to browser side
//! effects) for the five interop features `ghostty-web` exposes hooks for:
//!
//!   - Mouse reporting   (SGR 1006 encoding)
//!   - Bracketed paste   (\e[200~ … \e[201~ wrapping)
//!   - Focus events      (\e[I / \e[O on window focus/blur)
//!   - Window title      (\e]0;…\a → document.title)
//!   - Bell              (\a → CSS flash + Web Audio beep)
//!
//! Each feature is gated by a flag in `settings.rs`. `attach` returns a
//! `BridgeGuard`; dropping it removes every listener.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gloo_events::EventListener;
use js_sys::Reflect;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    AudioContext, ClipboardEvent, GainNode, HtmlElement, MouseEvent, OscillatorNode, WheelEvent,
};

use crate::ghostty::{dispose, GhosttyTerminal};
use crate::settings;

pub struct BridgeGuard {
    _listeners: Vec<EventListener>,
    /// Disposable handles returned by `term.onTitleChange` / `term.onBell`.
    disposables: RefCell<Vec<JsValue>>,
    /// Owned closures that must outlive their JS callers.
    _closures: Vec<JsValue>,
}

impl Drop for BridgeGuard {
    fn drop(&mut self) {
        for d in self.disposables.borrow().iter() {
            dispose(d);
        }
    }
}

/// `send` is invoked with bytes to forward to the PTY (e.g. mouse-encode
/// frames, bracketed paste, focus events). `set_bell_class` reapplies the
/// CSS bell-flash class on the container.
pub fn attach(
    term: GhosttyTerminal,
    container: HtmlElement,
    send: Rc<dyn Fn(&str)>,
) -> BridgeGuard {
    let mut listeners: Vec<EventListener> = Vec::new();
    let mut owned: Vec<JsValue> = Vec::new();
    let disposables: RefCell<Vec<JsValue>> = RefCell::new(Vec::new());

    // Bell false-positive guard: ghostty-web fires `onBell` whenever a write
    // contains 0x07, but bash also emits 0x07 as the OSC terminator after
    // every prompt-title escape. Patch term.write to remember whether the
    // last write was a "real" bell (contains \x07 but NOT \e]).
    let last_was_real_bell = Rc::new(Cell::new(false));
    if let Ok(orig) = Reflect::get(term.as_jsvalue(), &JsValue::from_str("write")) {
        if let Ok(orig_fn) = orig.dyn_into::<js_sys::Function>() {
            let bell_flag = last_was_real_bell.clone();
            let term_js = term.as_jsvalue().clone();
            let orig_fn_clone = orig_fn.clone();
            let new_write = Closure::wrap(Box::new(move |data: JsValue| -> JsValue {
                if let Some(s) = data.as_string() {
                    bell_flag.set(s.contains('\u{07}') && !s.contains('\u{1b}'));
                } else {
                    bell_flag.set(false);
                }
                orig_fn_clone
                    .call1(&term_js, &data)
                    .unwrap_or(JsValue::UNDEFINED)
            }) as Box<dyn FnMut(JsValue) -> JsValue>);
            let _ = Reflect::set(
                term.as_jsvalue(),
                &JsValue::from_str("write"),
                new_write.as_ref().unchecked_ref(),
            );
            owned.push(new_write.into_js_value());
        }
    }

    // ─── Mouse ───────────────────────────────────────────────────────────
    if settings::MOUSE_ENABLED {
        let term_for_mouse = term.clone();
        let container_for_mouse = container.clone();
        let send_mouse = send.clone();
        let button_held = Rc::new(Cell::new(false));

        let cell_at = {
            let term = term_for_mouse.clone();
            let container = container_for_mouse.clone();
            move |e: &MouseEvent| -> Option<(i32, i32)> {
                let (w, h) = term.renderer_metrics()?;
                if w == 0.0 || h == 0.0 {
                    return None;
                }
                let r = container.get_bounding_client_rect();
                let x = (((e.client_x() as f64 - r.left()) / w).floor() as i32 + 1).max(1);
                let y = (((e.client_y() as f64 - r.top()) / h).floor() as i32 + 1).max(1);
                Some((x, y))
            }
        };

        let button_bits = |e: &MouseEvent| -> i32 {
            let mut b = match e.button() {
                0 => 0,
                1 => 1,
                2 => 2,
                _ => 3,
            };
            if e.shift_key() {
                b |= 4;
            }
            if e.alt_key() {
                b |= 8;
            }
            if e.ctrl_key() {
                b |= 16;
            }
            b
        };

        let encode = |cb: i32, x: i32, y: i32, kind: char| -> String {
            format!("\u{1b}[<{cb};{x};{y}{kind}")
        };

        // mousedown
        {
            let term = term_for_mouse.clone();
            let send = send_mouse.clone();
            let cell_at = cell_at.clone();
            let button_bits = button_bits;
            let held = button_held.clone();
            let l = EventListener::new(&container_for_mouse.clone(), "mousedown", move |e| {
                if !term.has_mouse_tracking() {
                    return;
                }
                let me: &MouseEvent = e.dyn_ref().unwrap();
                let Some((x, y)) = cell_at(me) else { return };
                send(&encode(button_bits(me), x, y, 'M'));
                held.set(true);
                me.prevent_default();
                me.stop_propagation();
            });
            listeners.push(l);
        }
        // mouseup
        {
            let term = term_for_mouse.clone();
            let send = send_mouse.clone();
            let cell_at = cell_at.clone();
            let held = button_held.clone();
            let l = EventListener::new(&container_for_mouse.clone(), "mouseup", move |e| {
                if !term.has_mouse_tracking() {
                    return;
                }
                let me: &MouseEvent = e.dyn_ref().unwrap();
                let Some((x, y)) = cell_at(me) else { return };
                send(&encode(button_bits(me), x, y, 'm'));
                held.set(false);
                me.prevent_default();
                me.stop_propagation();
            });
            listeners.push(l);
        }
        // mousemove (only while a button is held).
        {
            let term = term_for_mouse.clone();
            let send = send_mouse.clone();
            let cell_at = cell_at.clone();
            let held = button_held.clone();
            let l = EventListener::new(&container_for_mouse.clone(), "mousemove", move |e| {
                if !term.has_mouse_tracking() {
                    return;
                }
                let me: &MouseEvent = e.dyn_ref().unwrap();
                if !held.get() && me.buttons() == 0 {
                    return;
                }
                let Some((x, y)) = cell_at(me) else { return };
                send(&encode(button_bits(me) | 32, x, y, 'M'));
                me.prevent_default();
                me.stop_propagation();
            });
            listeners.push(l);
        }
        // Wheel via the lib's own custom handler so its built-in scroll
        // (which doesn't know about mouse tracking) is suppressed when the
        // program wants wheel-as-mouse.
        {
            let term_outer = term_for_mouse.clone();
            let send = send_mouse.clone();
            let cell_at = cell_at;
            let cb = Closure::wrap(Box::new(move |ev: JsValue| -> bool {
                if !term_outer.has_mouse_tracking() {
                    return false;
                }
                let we: WheelEvent = ev.unchecked_into();
                let me: &MouseEvent = we.unchecked_ref();
                let Some((x, y)) = cell_at(me) else { return false };
                let dir = if we.delta_y() > 0.0 { 65 } else { 64 };
                send(&encode(dir, x, y, 'M'));
                true
            }) as Box<dyn FnMut(JsValue) -> bool>);
            let _ = term_for_mouse.attach_custom_wheel_handler(&cb);
            owned.push(cb.into_js_value());
        }
    }

    // ─── Bracketed paste ────────────────────────────────────────────────
    if settings::BRACKETED_PASTE_ENABLED {
        let term_p = term.clone();
        let send_p = send.clone();
        let l = EventListener::new(&container.clone(), "paste", move |e| {
            let ce: &ClipboardEvent = e.dyn_ref().unwrap();
            let Some(cd) = ce.clipboard_data() else { return };
            let Ok(text) = cd.get_data("text/plain") else { return };
            if text.is_empty() {
                return;
            }
            ce.prevent_default();
            ce.stop_propagation();
            if term_p.has_bracketed_paste() {
                send_p(&format!("\u{1b}[200~{text}\u{1b}[201~"));
            } else {
                send_p(&text);
            }
        });
        listeners.push(l);
    }

    // ─── Focus events ───────────────────────────────────────────────────
    if settings::FOCUS_EVENTS_ENABLED {
        if let Some(window) = web_sys::window() {
            let term_f = term.clone();
            let send_f = send.clone();
            let l = EventListener::new(&window, "focus", move |_| {
                if term_f.has_focus_events() {
                    send_f("\u{1b}[I");
                }
            });
            listeners.push(l);
            let term_b = term.clone();
            let send_b = send.clone();
            let l = EventListener::new(&web_sys::window().unwrap(), "blur", move |_| {
                if term_b.has_focus_events() {
                    send_b("\u{1b}[O");
                }
            });
            listeners.push(l);
        }
    }

    // ─── Window title ───────────────────────────────────────────────────
    if settings::SET_DOCUMENT_TITLE {
        let cb = Closure::wrap(Box::new(move |t: JsValue| {
            let title = t.as_string().unwrap_or_default();
            if let Some(d) = web_sys::window().and_then(|w| w.document()) {
                let s = if title.is_empty() {
                    "ghostty-web".to_string()
                } else {
                    title
                };
                d.set_title(&s);
            }
        }) as Box<dyn FnMut(JsValue)>);
        if let Ok(d) = term.on_title_change(&cb) {
            disposables.borrow_mut().push(d);
        }
        owned.push(cb.into_js_value());
    }

    // ─── Bell (visual + audible + debug) ────────────────────────────────
    if settings::VISUAL_BELL_ENABLED || settings::AUDIBLE_BELL_ENABLED || settings::DEBUG_BELL_ENABLED {
        let audio_ctx: Rc<RefCell<Option<AudioContext>>> = Rc::new(RefCell::new(None));
        let last_bell_at: Rc<Cell<f64>> = Rc::new(Cell::new(0.0));
        let bell_flag = last_was_real_bell;
        let container_bell = container.clone();
        let cb = Closure::wrap(Box::new(move || {
            if !bell_flag.get() {
                return;
            }
            bell_flag.set(false);

            let now = web_sys::window()
                .and_then(|w| w.performance())
                .map(|p| p.now())
                .unwrap_or(0.0);
            if now - last_bell_at.get() < 250.0 {
                return;
            }
            last_bell_at.set(now);

            if settings::VISUAL_BELL_ENABLED {
                let class_list = container_bell.class_list();
                let _ = class_list.remove_1("ghostty-bell-flash");
                // Force reflow so the animation restarts even when the class
                // is re-added in the same frame.
                let _ = container_bell.offset_width();
                let _ = class_list.add_1("ghostty-bell-flash");
            }
            if settings::AUDIBLE_BELL_ENABLED {
                if audio_ctx.borrow().is_none() {
                    if let Ok(ctx) = AudioContext::new() {
                        *audio_ctx.borrow_mut() = Some(ctx);
                    }
                }
                if let Some(ctx) = audio_ctx.borrow().as_ref() {
                    let _ = beep(ctx);
                }
            }
            if settings::DEBUG_BELL_ENABLED {
                web_sys::console::log_1(&JsValue::from_str("ding"));
            }
        }) as Box<dyn FnMut()>);
        if let Ok(d) = term.on_bell(&cb) {
            disposables.borrow_mut().push(d);
        }
        owned.push(cb.into_js_value());
    }

    BridgeGuard {
        _listeners: listeners,
        disposables,
        _closures: owned,
    }
}

fn beep(ctx: &AudioContext) -> Result<(), JsValue> {
    let t = ctx.current_time();
    let osc: OscillatorNode = ctx.create_oscillator()?;
    let gain: GainNode = ctx.create_gain()?;
    osc.connect_with_audio_node(&gain)?
        .connect_with_audio_node(&ctx.destination())?;
    osc.frequency().set_value(800.0);
    gain.gain().set_value_at_time(0.08, t)?;
    gain.gain()
        .exponential_ramp_to_value_at_time(0.001, t + 0.12)?;
    osc.start_with_when(t)?;
    osc.stop_with_when(t + 0.12)?;
    Ok(())
}

