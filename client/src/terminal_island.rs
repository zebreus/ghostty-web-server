//! TerminalIsland — bootstraps `ghostty-web`, opens the WebSocket, wires
//! the bridge, and handles ACK-based resize. Mirrors
//! `src/islands/TerminalIsland.tsx`.

use std::cell::RefCell;
use std::rc::Rc;

use gloo_storage::{SessionStorage, Storage};
use gloo_timers::callback::Interval;
use js_sys::{Function, Reflect};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{
    BinaryType, CloseEvent, Element, HtmlElement, MessageEvent, ResizeObserver, WebSocket,
};
use yew::prelude::*;

use crate::bridge::{attach as attach_bridge, BridgeGuard};
use crate::ghostty::{build_terminal_options, dyn_import, GhosttyModule, GhosttyTerminal};
use crate::protocol::{ClientMsg, ServerMsg};
use crate::session_commands::switch_session;
use crate::settings;
use crate::status::{set_status, Status};

fn session_id() -> String {
    if let Ok(id) = SessionStorage::get::<String>("ghosttySessionId") {
        return id;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let _ = SessionStorage::set("ghosttySessionId", id.clone());
    id
}

fn consume_take_flag() -> bool {
    let v: Option<String> = SessionStorage::get("ghosttyTake").ok();
    let taken = v.as_deref() == Some("1");
    if taken {
        SessionStorage::delete("ghosttyTake");
    }
    taken
}

#[function_component(TerminalIsland)]
pub fn terminal_island() -> Html {
    let node = use_node_ref();

    {
        let node = node.clone();
        use_effect_with((), move |_| {
            let cancelled = Rc::new(std::cell::Cell::new(false));
            let state: Rc<RefCell<Option<TerminalState>>> = Rc::new(RefCell::new(None));

            let node_for_async = node.clone();
            let cancelled_for_async = cancelled.clone();
            let state_for_async = state.clone();
            spawn_local(async move {
                if cancelled_for_async.get() {
                    return;
                }
                let Some(container) = node_for_async.cast::<HtmlElement>() else { return };
                if let Err(e) = bootstrap(container, cancelled_for_async.clone(), state_for_async).await {
                    web_sys::console::error_1(&e);
                    set_status(Status::Error {
                        message: "Failed to load terminal".into(),
                    });
                }
            });

            // Cleanup
            let cancelled_for_cleanup = cancelled.clone();
            let state_for_cleanup = state.clone();
            move || {
                cancelled_for_cleanup.set(true);
                if let Some(s) = state_for_cleanup.borrow_mut().take() {
                    s.shutdown();
                }
            }
        });
    }

    html! { <div ref={node} id="terminal" /> }
}

struct TerminalState {
    ws: Rc<RefCell<Option<WebSocket>>>,
    term: GhosttyTerminal,
    bridge: Option<BridgeGuard>,
    resize_observer: Option<ResizeObserver>,
    reconnect_timer: Rc<RefCell<Option<Interval>>>,
    /// JS Closures that must outlive their JS callers.
    _retained: Vec<JsValue>,
}

impl TerminalState {
    fn shutdown(self) {
        if let Some(t) = self.reconnect_timer.borrow_mut().take() {
            drop(t);
        }
        drop(self.bridge);
        if let Some(ro) = self.resize_observer {
            ro.disconnect();
        }
        if let Some(ws) = self.ws.borrow_mut().take() {
            let _ = ws.close();
        }
        self.term.dispose();
    }
}

async fn bootstrap(
    container: HtmlElement,
    cancelled: Rc<std::cell::Cell<bool>>,
    state_slot: Rc<RefCell<Option<TerminalState>>>,
) -> Result<(), JsValue> {
    let origin = web_sys::window()
        .and_then(|w| w.location().origin().ok())
        .unwrap_or_default();
    let url = format!("{origin}/dist/ghostty-web.js");

    let module_js = JsFuture::from(dyn_import(&url)).await?;
    if cancelled.get() {
        return Ok(());
    }
    let module = GhosttyModule::from_module(module_js);

    // Silence ghostty-web's noisy `[ghostty-vt]` log output (mostly OSC
    // warnings the WASM parser doesn't fully implement). Replace
    // console.log with a wrapper that drops messages whose first arg is
    // exactly `'[ghostty-vt]'`.
    silence_ghostty_vt_logs();

    module.init().await?;
    if cancelled.get() {
        return Ok(());
    }

    let opts = build_terminal_options(80, 24);
    let term = module.new_terminal(&opts)?;
    term.open(&container).await?;
    if cancelled.get() {
        return Ok(());
    }

    // Outgoing-message helper, shared with bridge + onData + resize.
    let ws_holder: Rc<RefCell<Option<WebSocket>>> = Rc::new(RefCell::new(None));
    let send_input: Rc<dyn Fn(&str)> = {
        let ws_holder = ws_holder.clone();
        Rc::new(move |data: &str| {
            send_msg(
                &ws_holder,
                &ClientMsg::Input {
                    value: data.to_string(),
                },
            );
        })
    };

    // Mouse / paste / focus / title / bell.
    let bridge = attach_bridge(term.clone(), container.clone(), send_input.clone());

    // ── Resize state (ACK-based, single in-flight) ────────────────────────
    let in_flight = Rc::new(std::cell::Cell::new(false));
    let desired: Rc<RefCell<Option<(u16, u16)>>> = Rc::new(RefCell::new(None));

    let send_next_resize: Rc<dyn Fn()> = {
        let ws_holder = ws_holder.clone();
        let in_flight = in_flight.clone();
        let desired = desired.clone();
        Rc::new(move || {
            let Some((cols, rows)) = *desired.borrow() else { return };
            in_flight.set(true);
            send_msg(&ws_holder, &ClientMsg::Resize { cols, rows });
        })
    };

    let on_ack: Rc<dyn Fn(u16, u16)> = {
        let in_flight = in_flight.clone();
        let desired = desired.clone();
        let term_for_ack = term.clone();
        let send_next_resize = send_next_resize.clone();
        Rc::new(move |cols, rows| {
            in_flight.set(false);
            if cols != term_for_ack.cols() || rows != term_for_ack.rows() {
                let _ = term_for_ack.resize(cols, rows);
            }
            let mut d = desired.borrow_mut();
            if let Some((dc, dr)) = *d {
                if dc != cols || dr != rows {
                    drop(d);
                    send_next_resize();
                    return;
                }
            }
            *d = None;
        })
    };

    // ── ResizeObserver → fit() ────────────────────────────────────────────
    let fit: Rc<dyn Fn()> = {
        let term_for_fit = term.clone();
        let container_for_fit = container.clone();
        let in_flight = in_flight.clone();
        let desired = desired.clone();
        let send_next_resize = send_next_resize.clone();
        let ws_holder_for_fit = ws_holder.clone();
        Rc::new(move || {
            let Some((mw, mh)) = term_for_fit.renderer_metrics() else { return };
            if mw == 0.0 || mh == 0.0 {
                return;
            }
            let cw = container_for_fit.client_width() as f64;
            let ch = container_for_fit.client_height() as f64;
            let cols = (cw / mw).floor().max(settings::MIN_COLS as f64) as u16;
            let rows = (ch / mh).floor().max(settings::MIN_ROWS as f64) as u16;
            if cols == term_for_fit.cols()
                && rows == term_for_fit.rows()
                && desired.borrow().is_none()
            {
                return;
            }
            *desired.borrow_mut() = Some((cols, rows));
            if !in_flight.get() {
                send_next_resize();
            }
            if settings::RESIZE_AUTO_REDRAW_MS > 0 {
                send_msg(
                    &ws_holder_for_fit,
                    &ClientMsg::Input {
                        value: "\u{0c}".to_string(),
                    },
                );
            }
        })
    };

    let fit_cb = {
        let fit = fit.clone();
        Closure::wrap(Box::new(move |_entries: JsValue, _obs: JsValue| {
            fit();
        }) as Box<dyn FnMut(JsValue, JsValue)>)
    };
    let ro = ResizeObserver::new(fit_cb.as_ref().unchecked_ref())?;
    ro.observe(container.dyn_ref::<Element>().unwrap());
    let mut retained: Vec<JsValue> = Vec::new();
    retained.push(fit_cb.into_js_value());

    // ── term.onData → input ───────────────────────────────────────────────
    let on_data_cb = {
        let send_input = send_input.clone();
        Closure::wrap(Box::new(move |d: JsValue| {
            if let Some(s) = d.as_string() {
                send_input(&s);
            }
        }) as Box<dyn FnMut(JsValue)>)
    };
    term.on_data(&on_data_cb)?;
    retained.push(on_data_cb.into_js_value());

    // ── WebSocket connect / reconnect loop ────────────────────────────────
    let session_id_str = session_id();
    let take_initial = Rc::new(std::cell::Cell::new(consume_take_flag()));
    let reconnect_timer: Rc<RefCell<Option<Interval>>> = Rc::new(RefCell::new(None));

    let connect: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let connect_outer = connect.clone();

    let connect_fn: Rc<dyn Fn()> = {
        let cancelled = cancelled.clone();
        let ws_holder = ws_holder.clone();
        let term_for_conn = term.clone();
        let take_initial = take_initial.clone();
        let reconnect_timer = reconnect_timer.clone();
        let in_flight = in_flight.clone();
        let desired = desired.clone();
        let send_next_resize = send_next_resize.clone();
        let on_ack = on_ack.clone();
        let fit = fit.clone();
        let session_id_str = session_id_str.clone();
        let connect = connect.clone();

        Rc::new(move || {
            if cancelled.get() {
                return;
            }
            set_status(Status::Connecting);

            let proto = web_sys::window()
                .and_then(|w| w.location().protocol().ok())
                .unwrap_or_default();
            let host = web_sys::window()
                .and_then(|w| w.location().host().ok())
                .unwrap_or_default();
            let scheme = if proto == "https:" { "wss:" } else { "ws:" };
            let mut params = format!(
                "sessionId={}&cols={}&rows={}",
                urlencode(&session_id_str),
                term_for_conn.cols(),
                term_for_conn.rows()
            );
            if take_initial.get() {
                params.push_str("&take=1");
                take_initial.set(false);
            }
            let ws_url = format!("{scheme}//{host}/ws?{params}");

            let ws = match WebSocket::new(&ws_url) {
                Ok(w) => w,
                Err(_) => {
                    set_status(Status::Error {
                        message: "Failed to open WebSocket".into(),
                    });
                    return;
                }
            };
            ws.set_binary_type(BinaryType::Arraybuffer);

            // onopen
            {
                let in_flight = in_flight.clone();
                let desired = desired.clone();
                let send_next_resize = send_next_resize.clone();
                let fit = fit.clone();
                let onopen = Closure::wrap(Box::new(move |_: JsValue| {
                    set_status(Status::Connected);
                    in_flight.set(false);
                    if desired.borrow().is_none() {
                        fit();
                    } else {
                        send_next_resize();
                    }
                }) as Box<dyn FnMut(JsValue)>);
                ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
                onopen.forget();
            }
            // onmessage
            {
                let term_for_msg = term_for_conn.clone();
                let on_ack = on_ack.clone();
                let onmessage = Closure::wrap(Box::new(move |e: MessageEvent| {
                    let Some(text) = e.data().as_string() else { return };
                    let Ok(parsed) = serde_json::from_str::<ServerMsg>(&text) else { return };
                    match parsed {
                        ServerMsg::Data { value } => {
                            let _ = term_for_msg.write(&value);
                        }
                        ServerMsg::Ack { cols, rows } => on_ack(cols, rows),
                    }
                }) as Box<dyn FnMut(MessageEvent)>);
                ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
                onmessage.forget();
            }
            // onclose
            {
                let cancelled = cancelled.clone();
                let session_id_for_close = session_id_str.clone();
                let reconnect_timer = reconnect_timer.clone();
                let connect = connect.clone();
                let onclose = Closure::wrap(Box::new(move |e: CloseEvent| {
                    if cancelled.get() {
                        return;
                    }
                    if e.code() == 4002 {
                        let session_id_for_take = session_id_for_close.clone();
                        set_status(Status::Busy {
                            on_take: yew::Callback::from(move |_| {
                                switch_session(&session_id_for_take, true);
                            }),
                        });
                        return;
                    }
                    let n = Rc::new(std::cell::Cell::new(2u32));
                    set_status(Status::Reconnecting { remaining: n.get() });
                    let timer_slot = reconnect_timer.clone();
                    let connect_inner = connect.clone();
                    let n_for_timer = n.clone();
                    let interval = Interval::new(1000, move || {
                        let cur = n_for_timer.get();
                        if cur <= 1 {
                            *timer_slot.borrow_mut() = None;
                            if let Some(c) = connect_inner.borrow().clone() {
                                c();
                            }
                        } else {
                            n_for_timer.set(cur - 1);
                            set_status(Status::Reconnecting {
                                remaining: n_for_timer.get(),
                            });
                        }
                    });
                    *reconnect_timer.borrow_mut() = Some(interval);
                }) as Box<dyn FnMut(CloseEvent)>);
                ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
                onclose.forget();
            }

            *ws_holder.borrow_mut() = Some(ws);
        })
    };

    *connect_outer.borrow_mut() = Some(connect_fn.clone());
    connect_fn();

    *state_slot.borrow_mut() = Some(TerminalState {
        ws: ws_holder,
        term,
        bridge: Some(bridge),
        resize_observer: Some(ro),
        reconnect_timer,
        _retained: retained,
    });

    Ok(())
}

fn send_msg(ws_holder: &Rc<RefCell<Option<WebSocket>>>, msg: &ClientMsg) {
    let Some(ws) = ws_holder.borrow().as_ref().cloned() else { return };
    if ws.ready_state() != WebSocket::OPEN {
        return;
    }
    let Ok(text) = serde_json::to_string(msg) else { return };
    let _ = ws.send_with_str(&text);
}

fn urlencode(s: &str) -> String {
    js_sys::encode_uri_component(s).into()
}

fn silence_ghostty_vt_logs() {
    let Some(window) = web_sys::window() else { return };
    let console = match Reflect::get(&window, &JsValue::from_str("console")) {
        Ok(c) => c,
        Err(_) => return,
    };
    let orig_log: Function = match Reflect::get(&console, &JsValue::from_str("log")) {
        Ok(v) => match v.dyn_into() {
            Ok(f) => f,
            Err(_) => return,
        },
        Err(_) => return,
    };
    let console_for_closure = console.clone();
    let orig_for_closure = orig_log.clone();
    let wrapper = Closure::wrap(Box::new(move |args: JsValue| {
        // `args` here is undefined — we use Reflect to get arguments via
        // Function.prototype.apply with the current arguments object. To
        // keep this simple, get the first argument by indexing into the
        // arguments-like Array we receive when called via .apply below.
        let arr: js_sys::Array = match args.dyn_into() {
            Ok(a) => a,
            Err(_) => return,
        };
        if arr.length() > 0 {
            let first = arr.get(0);
            if first.as_string().as_deref() == Some("[ghostty-vt]") {
                return;
            }
        }
        let _ = orig_for_closure.apply(&console_for_closure, &arr);
    }) as Box<dyn FnMut(JsValue)>);
    // Wrap in a JS shim that converts variadic args → Array, since
    // wasm-bindgen closures can't take variadic args directly.
    let shim_src = "return function(){ return f(Array.from(arguments)); };";
    let shim_factory = js_sys::Function::new_with_args("f", shim_src);
    let shim = match shim_factory.call1(&JsValue::NULL, wrapper.as_ref().unchecked_ref()) {
        Ok(s) => s,
        Err(_) => return,
    };
    let _ = Reflect::set(&console, &JsValue::from_str("log"), &shim);
    wrapper.forget();
}
