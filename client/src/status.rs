//! Tiny pub/sub for the connection-status overlay.
//!
//! Mirrors `src/status.ts` from the original implementation.

use std::cell::RefCell;
use std::rc::Rc;

use yew::Callback;

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Idle,
    Connecting,
    Connected,
    Reconnecting { remaining: u32 },
    /// Another tab owns the session. `on_take` reloads with `take=1`.
    Busy { on_take: Callback<()> },
    Error { message: String },
}

impl Status {
    pub fn is_overlay_visible(&self) -> bool {
        !matches!(self, Status::Idle | Status::Connected)
    }
}

type Listener = Rc<dyn Fn(&Status)>;

#[derive(Default)]
struct Bus {
    current: Status,
    listeners: Vec<Listener>,
}

impl Default for Status {
    fn default() -> Self {
        Status::Idle
    }
}

thread_local! {
    static BUS: RefCell<Bus> = RefCell::new(Bus { current: Status::Idle, listeners: Vec::new() });
}

pub fn set_status(s: Status) {
    BUS.with(|b| {
        let mut bus = b.borrow_mut();
        bus.current = s.clone();
        // Clone listener handles before dispatch to allow re-entrance.
        let listeners: Vec<Listener> = bus.listeners.clone();
        drop(bus);
        for l in listeners {
            l(&s);
        }
    });
}

pub fn current() -> Status {
    BUS.with(|b| b.borrow().current.clone())
}

/// Subscribe to status changes. The listener fires immediately with the
/// current status. Returns an unsubscribe handle.
pub fn subscribe(cb: impl Fn(&Status) + 'static) -> Unsubscribe {
    let cb: Listener = Rc::new(cb);
    let now = BUS.with(|b| {
        let mut bus = b.borrow_mut();
        bus.listeners.push(cb.clone());
        bus.current.clone()
    });
    cb(&now);
    Unsubscribe { cb }
}

pub struct Unsubscribe {
    cb: Listener,
}

impl Drop for Unsubscribe {
    fn drop(&mut self) {
        BUS.with(|b| {
            let mut bus = b.borrow_mut();
            // Remove the matching Rc by pointer identity.
            bus.listeners.retain(|l| !Rc::ptr_eq(l, &self.cb));
        });
    }
}
