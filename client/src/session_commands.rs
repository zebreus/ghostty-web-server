//! Session-related commands for the palette. Self-registers via
//! `register_all`. Mirrors `src/session-commands.ts`.

use std::rc::Rc;

use gloo_net::http::Request;
use gloo_storage::{SessionStorage, Storage};
use serde::Deserialize;

use crate::palette::{register_provider, Command, ProviderFut};

#[derive(Debug, Clone, Deserialize)]
struct SessionRow {
    id: String,
    #[serde(rename = "startedAt")]
    started_at: f64,
    attached: bool,
    #[serde(rename = "activeProcess")]
    active_process: String,
}

#[derive(Debug, Clone, Deserialize)]
struct SessionsResp {
    #[serde(default)]
    sessions: Vec<SessionRow>,
}

pub fn switch_session(id: &str, take: bool) {
    let _ = SessionStorage::set("ghosttySessionId", id.to_string());
    if take {
        let _ = SessionStorage::set("ghosttyTake", "1".to_string());
    }
    if let Some(w) = web_sys::window() {
        let _ = w.location().reload();
    }
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

fn rel_time(ts_ms: f64) -> String {
    let diff = (now_ms() - ts_ms).max(0.0);
    let s = (diff / 1000.0).round() as i64;
    if s < 5 {
        return "just now".into();
    }
    if s < 60 {
        return format!("{s}s ago");
    }
    let m = (s as f64 / 60.0).round() as i64;
    if m < 60 {
        return format!("{m}m ago");
    }
    let h = (m as f64 / 60.0).round() as i64;
    if h < 24 {
        return format!("{h}h ago");
    }
    format!("{}d ago", (h as f64 / 24.0).round() as i64)
}

pub fn register_all() {
    // Sessions provider — fetched live each time the palette opens / polls.
    register_provider(Rc::new(|| -> ProviderFut {
        Box::pin(async move {
            let resp = match Request::get("/api/sessions").send().await {
                Ok(r) => r,
                Err(_) => return Vec::new(),
            };
            let parsed: SessionsResp = match resp.json().await {
                Ok(p) => p,
                Err(_) => return Vec::new(),
            };
            let current_id: String = SessionStorage::get("ghosttySessionId").unwrap_or_default();
            parsed
                .sessions
                .into_iter()
                .map(|s| {
                    let is_current = s.id == current_id;
                    let is_orphan = !s.attached;
                    let group = if is_orphan {
                        "Orphan"
                    } else if is_current {
                        "Current"
                    } else {
                        "Other tabs"
                    };
                    let hint = if is_current {
                        "this tab"
                    } else if is_orphan {
                        "switch"
                    } else {
                        "take"
                    };
                    let label = format!(
                        "{} · {}",
                        s.id.chars().take(8).collect::<String>(),
                        s.active_process
                    );
                    let detail = rel_time(s.started_at);
                    let id_for_select = s.id.clone();
                    Command {
                        id: format!("session:{}", s.id),
                        group: Some(group.to_string()),
                        label,
                        detail: Some(detail),
                        hint: Some(hint.to_string()),
                        disabled: is_current,
                        on_select: Rc::new(move || {
                            if is_current {
                                return;
                            }
                            switch_session(&id_for_select, !is_orphan);
                        }),
                    }
                })
                .collect()
        })
    }));

    // Static actions.
    register_provider(Rc::new(|| -> ProviderFut {
        Box::pin(async move {
            vec![
                Command {
                    id: "action:new-tab".into(),
                    group: Some("Actions".into()),
                    label: "New session in new tab".into(),
                    detail: None,
                    hint: Some("⏎".into()),
                    disabled: false,
                    on_select: Rc::new(|| {
                        if let Some(w) = web_sys::window() {
                            let origin = w
                                .location()
                                .origin()
                                .unwrap_or_else(|_| String::from("/"));
                            let _ = w.open_with_url_and_target(&format!("{origin}/"), "_blank");
                        }
                    }),
                },
                Command {
                    id: "action:reload".into(),
                    group: Some("Actions".into()),
                    label: "Reload".into(),
                    detail: None,
                    hint: Some("⏎".into()),
                    disabled: false,
                    on_select: Rc::new(|| {
                        if let Some(w) = web_sys::window() {
                            let _ = w.location().reload();
                        }
                    }),
                },
            ]
        })
    }));
}
