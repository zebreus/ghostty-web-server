//! `wasm-bindgen` extern declarations for the third-party `ghostty-web`
//! JS+WASM library. We treat almost every value as a generic `JsValue` and
//! call methods through `Reflect`/`js_sys` because the library's surface
//! evolves frequently and full bindings would be high-maintenance for low
//! gain — this is a thin shim, not a full Rust port.
//!
//! The browser fetches `/dist/ghostty-web.js` lazily via dynamic `import()`
//! (see `terminal::load_module`). The module exports `init()` and the
//! `Terminal` constructor we need.

use js_sys::{Function, Object, Promise, Reflect};
use wasm_bindgen::prelude::*;
use web_sys::HtmlElement;

#[wasm_bindgen(inline_js = "export function dyn_import(url) { return import(url); }")]
extern "C" {
    /// `await import(url)` from within wasm. The body uses an async-friendly
    /// `Promise` so callers get a real `Promise<Module>`.
    pub fn dyn_import(url: &str) -> Promise;
}

/// Wrapper around the imported `ghostty-web` JS module object.
#[derive(Clone)]
pub struct GhosttyModule {
    inner: JsValue,
}

impl GhosttyModule {
    pub fn from_module(m: JsValue) -> Self {
        Self { inner: m }
    }

    /// Call the module-level `init()` and await the returned Promise.
    pub async fn init(&self) -> Result<(), JsValue> {
        let init: Function = Reflect::get(&self.inner, &JsValue::from_str("init"))?.into();
        let p: Promise = init.call0(&self.inner)?.dyn_into()?;
        wasm_bindgen_futures::JsFuture::from(p).await?;
        Ok(())
    }

    /// Construct a new `Terminal` with the given options object.
    pub fn new_terminal(&self, opts: &JsValue) -> Result<GhosttyTerminal, JsValue> {
        let ctor: Function = Reflect::get(&self.inner, &JsValue::from_str("Terminal"))?.into();
        let term = js_sys::Reflect::construct(&ctor, &js_sys::Array::of1(opts))?;
        Ok(GhosttyTerminal { inner: term.into() })
    }
}

#[derive(Clone)]
pub struct GhosttyTerminal {
    inner: JsValue,
}

impl GhosttyTerminal {
    pub fn as_jsvalue(&self) -> &JsValue {
        &self.inner
    }

    pub async fn open(&self, el: &HtmlElement) -> Result<(), JsValue> {
        let f: Function = Reflect::get(&self.inner, &JsValue::from_str("open"))?.into();
        let p: Promise = f.call1(&self.inner, el)?.dyn_into()?;
        wasm_bindgen_futures::JsFuture::from(p).await?;
        Ok(())
    }

    pub fn cols(&self) -> u16 {
        Reflect::get(&self.inner, &JsValue::from_str("cols"))
            .ok()
            .and_then(|v| v.as_f64())
            .map(|n| n as u16)
            .unwrap_or(80)
    }

    pub fn rows(&self) -> u16 {
        Reflect::get(&self.inner, &JsValue::from_str("rows"))
            .ok()
            .and_then(|v| v.as_f64())
            .map(|n| n as u16)
            .unwrap_or(24)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), JsValue> {
        let f: Function = Reflect::get(&self.inner, &JsValue::from_str("resize"))?.into();
        f.call2(&self.inner, &JsValue::from_f64(cols as f64), &JsValue::from_f64(rows as f64))?;
        Ok(())
    }

    pub fn write(&self, data: &str) -> Result<(), JsValue> {
        let f: Function = Reflect::get(&self.inner, &JsValue::from_str("write"))?.into();
        f.call1(&self.inner, &JsValue::from_str(data))?;
        Ok(())
    }

    pub fn renderer_metrics(&self) -> Option<(f64, f64)> {
        let renderer = Reflect::get(&self.inner, &JsValue::from_str("renderer")).ok()?;
        let f: Function = Reflect::get(&renderer, &JsValue::from_str("getMetrics")).ok()?.into();
        let m = f.call0(&renderer).ok()?;
        let w = Reflect::get(&m, &JsValue::from_str("width")).ok()?.as_f64()?;
        let h = Reflect::get(&m, &JsValue::from_str("height")).ok()?.as_f64()?;
        Some((w, h))
    }

    pub fn has_mouse_tracking(&self) -> bool {
        self.call_bool("hasMouseTracking")
    }
    pub fn has_bracketed_paste(&self) -> bool {
        self.call_bool("hasBracketedPaste")
    }
    pub fn has_focus_events(&self) -> bool {
        self.call_bool("hasFocusEvents")
    }

    fn call_bool(&self, name: &str) -> bool {
        Reflect::get(&self.inner, &JsValue::from_str(name))
            .ok()
            .and_then(|f| f.dyn_into::<Function>().ok())
            .and_then(|f| f.call0(&self.inner).ok())
            .map(|v| v.is_truthy())
            .unwrap_or(false)
    }

    /// `term.onData(cb)` — fired for every keystroke the lib wants to send
    /// to the PTY.
    pub fn on_data(&self, cb: &Closure<dyn FnMut(JsValue)>) -> Result<(), JsValue> {
        let f: Function = Reflect::get(&self.inner, &JsValue::from_str("onData"))?.into();
        f.call1(&self.inner, cb.as_ref().unchecked_ref())?;
        Ok(())
    }

    /// `term.onTitleChange(cb)` — returns a `{dispose}` object.
    pub fn on_title_change(&self, cb: &Closure<dyn FnMut(JsValue)>) -> Result<JsValue, JsValue> {
        let f: Function = Reflect::get(&self.inner, &JsValue::from_str("onTitleChange"))?.into();
        f.call1(&self.inner, cb.as_ref().unchecked_ref())
    }

    /// `term.onBell(cb)` — returns a `{dispose}` object.
    pub fn on_bell(&self, cb: &Closure<dyn FnMut()>) -> Result<JsValue, JsValue> {
        let f: Function = Reflect::get(&self.inner, &JsValue::from_str("onBell"))?.into();
        f.call1(&self.inner, cb.as_ref().unchecked_ref())
    }

    /// `term.attachCustomWheelEventHandler(cb)` — returning `true` from
    /// `cb` tells the lib to skip its own wheel handling.
    pub fn attach_custom_wheel_handler(
        &self,
        cb: &Closure<dyn FnMut(JsValue) -> bool>,
    ) -> Result<(), JsValue> {
        let f: Function = Reflect::get(&self.inner, &JsValue::from_str("attachCustomWheelEventHandler"))?.into();
        f.call1(&self.inner, cb.as_ref().unchecked_ref())?;
        Ok(())
    }

    /// `term.dispose()` if present.
    pub fn dispose(&self) {
        if let Ok(v) = Reflect::get(&self.inner, &JsValue::from_str("dispose")) {
            if let Ok(f) = v.dyn_into::<Function>() {
                let _ = f.call0(&self.inner);
            }
        }
    }
}

/// Build the `{cols, rows, fontFamily, fontSize, theme:{…}}` options object
/// the `Terminal` constructor expects.
pub fn build_terminal_options(cols: u16, rows: u16) -> JsValue {
    let opts = Object::new();
    let _ = Reflect::set(&opts, &"cols".into(), &JsValue::from_f64(cols as f64));
    let _ = Reflect::set(&opts, &"rows".into(), &JsValue::from_f64(rows as f64));
    let _ = Reflect::set(
        &opts,
        &"fontFamily".into(),
        &JsValue::from_str("JetBrains Mono, Menlo, Monaco, monospace"),
    );
    let _ = Reflect::set(&opts, &"fontSize".into(), &JsValue::from_f64(14.0));

    let theme = Object::new();
    let _ = Reflect::set(&theme, &"background".into(), &JsValue::from_str("#1e1e1e"));
    let _ = Reflect::set(&theme, &"foreground".into(), &JsValue::from_str("#d4d4d4"));
    let _ = Reflect::set(&opts, &"theme".into(), &theme);

    opts.into()
}

/// Dispose object returned by `term.onTitleChange` / `term.onBell`.
pub fn dispose(d: &JsValue) {
    if let Ok(f) = Reflect::get(d, &JsValue::from_str("dispose")) {
        if let Ok(f) = f.dyn_into::<Function>() {
            let _ = f.call0(d);
        }
    }
}
