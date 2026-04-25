//! Generic command-palette registry. Mirrors `src/palette.ts`.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

#[derive(Clone)]
pub struct Command {
    pub id: String,
    pub label: String,
    pub group: Option<String>,
    pub detail: Option<String>,
    pub hint: Option<String>,
    pub disabled: bool,
    pub on_select: Rc<dyn Fn()>,
}

pub type ProviderFut = Pin<Box<dyn Future<Output = Vec<Command>>>>;
pub type Provider = Rc<dyn Fn() -> ProviderFut>;

thread_local! {
    static PROVIDERS: RefCell<Vec<Provider>> = RefCell::new(Vec::new());
}

pub fn register_provider(p: Provider) {
    PROVIDERS.with(|ps| ps.borrow_mut().push(p));
}

pub async fn load_commands() -> Vec<Command> {
    let providers: Vec<Provider> = PROVIDERS.with(|ps| ps.borrow().clone());
    let mut out = Vec::new();
    for p in providers {
        out.extend(p().await);
    }
    out
}
