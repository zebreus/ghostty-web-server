//! PTY session manager — port of the session logic in the original
//! `src/server.tsx`.
//!
//! Each `Session` owns a spawned shell PTY. PTY output is read on a
//! dedicated blocking thread (portable-pty's reader is synchronous) and
//! pushed onto an mpsc channel: the bytes are decoded as UTF-8, appended to
//! a capped scrollback buffer, and forwarded (when present) to the
//! currently attached WebSocket via the attachment's mpsc sender.
//!
//! Reattach / take semantics:
//!   - At most one WS attached at a time.
//!   - `take=1` query closes the previous attachment with code 1000.
//!   - Otherwise a second connection is rejected with close code 4002.
//!   - `attach_key` is bumped on every successful attach so a late close()
//!     handler can tell whether it owns the current attachment.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use tokio::sync::{mpsc, Mutex};

use crate::protocol::ServerMsg;

const SCROLLBACK_CAP: usize = 256_000;
const SCROLLBACK_KEEP: usize = 192_000;

pub type Tx = mpsc::UnboundedSender<ServerMsg>;

pub struct Attachment {
    pub tx: Tx,
    pub key: u64,
}

pub struct Session {
    pub id: String,
    pub started_at: u128,
    pub pid: u32,
    master: StdMutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    pub scrollback: Mutex<String>,
    pub attached: Mutex<Option<Attachment>>,
    pub attach_counter: Mutex<u64>,
}

impl Session {
    pub async fn write_input(&self, data: &[u8]) -> std::io::Result<()> {
        let mut w = self.writer.lock().await;
        w.write_all(data)?;
        w.flush()
    }

    pub fn resize(&self, cols: u16, rows: u16) -> std::io::Result<()> {
        self.master
            .lock()
            .unwrap()
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }

    pub async fn send_to_attached(&self, msg: ServerMsg) {
        if let Some(a) = self.attached.lock().await.as_ref() {
            let _ = a.tx.send(msg);
        }
    }
}

#[derive(Clone, Default)]
pub struct Sessions {
    inner: Arc<Mutex<HashMap<String, Arc<Session>>>>,
}

pub struct AttachOutcome {
    pub session: Arc<Session>,
    pub attach_key: u64,
}

pub enum AttachResult {
    Attached {
        outcome: AttachOutcome,
        scrollback: Option<String>,
    },
    Busy,
}

impl Sessions {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn snapshot(&self) -> Vec<Arc<Session>> {
        self.inner.lock().await.values().cloned().collect()
    }

    pub async fn attach(
        &self,
        id: &str,
        cols: u16,
        rows: u16,
        take: bool,
        tx: Tx,
    ) -> AttachResult {
        let (session, existed) = {
            let mut map = self.inner.lock().await;
            if let Some(s) = map.get(id).cloned() {
                (s, true)
            } else {
                let s = spawn_session(id.to_string(), cols, rows);
                map.insert(id.to_string(), s.clone());
                (s, false)
            }
        };

        if existed {
            let mut current = session.attached.lock().await;
            if current.is_some() {
                if take {
                    // Drop the previous tx so its WS task closes.
                    *current = None;
                } else {
                    return AttachResult::Busy;
                }
            }
        }

        let attach_key = {
            let mut counter = session.attach_counter.lock().await;
            *counter = counter.wrapping_add(1);
            *counter
        };
        {
            let mut current = session.attached.lock().await;
            *current = Some(Attachment {
                tx,
                key: attach_key,
            });
        }

        let _ = session.resize(cols, rows);

        let scrollback = if existed {
            let s = session.scrollback.lock().await;
            if s.is_empty() {
                None
            } else {
                Some(s.clone())
            }
        } else {
            None
        };

        AttachResult::Attached {
            outcome: AttachOutcome {
                session,
                attach_key,
            },
            scrollback,
        }
    }

    pub async fn detach_if_current(&self, id: &str, attach_key: u64) {
        if let Some(session) = self.inner.lock().await.get(id).cloned() {
            let mut current = session.attached.lock().await;
            if let Some(a) = current.as_ref() {
                if a.key == attach_key {
                    *current = None;
                }
            }
        }
    }

    pub async fn remove(&self, id: &str) {
        self.inner.lock().await.remove(id);
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Background reader / shell-exit dispatch.
//
// portable-pty's reader is a blocking std::io::Read. We run it on a
// dedicated thread per session and use the stashed Tokio runtime handle to
// schedule async work (appending scrollback, fanning out to the attached
// WS, removing the session on shell exit).
// ---------------------------------------------------------------------------

static RUNTIME: Lazy<StdMutex<Option<tokio::runtime::Handle>>> = Lazy::new(|| StdMutex::new(None));
static PUBLIC_SESSIONS: Lazy<StdMutex<Option<Sessions>>> = Lazy::new(|| StdMutex::new(None));

pub fn install_runtime_handle(h: tokio::runtime::Handle) {
    *RUNTIME.lock().unwrap() = Some(h);
}

pub fn install_public_sessions(s: Sessions) {
    *PUBLIC_SESSIONS.lock().unwrap() = Some(s);
}

fn spawn_session(id: String, cols: u16, rows: u16) -> Arc<Session> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty failed");

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let mut cmd = CommandBuilder::new(&shell);
    if let Ok(home) = std::env::var("HOME") {
        cmd.cwd(home);
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .expect("failed to spawn shell");

    let writer = pair.master.take_writer().expect("take_writer failed");
    let reader = pair.master.try_clone_reader().expect("clone_reader failed");
    let pid = child.process_id().unwrap_or(0);

    // Drop the slave end on the parent — the child inherited it on spawn,
    // and keeping it open here would block reader-EOF after shell exit.
    drop(pair.slave);

    let session = Arc::new(Session {
        id: id.clone(),
        started_at: now_ms(),
        pid,
        master: StdMutex::new(pair.master),
        writer: Mutex::new(writer),
        scrollback: Mutex::new(String::new()),
        attached: Mutex::new(None),
        attach_counter: Mutex::new(0),
    });

    let session_for_reader = session.clone();
    std::thread::spawn(move || run_reader(session_for_reader, reader));

    let session_for_exit = session.clone();
    let id_for_exit = id;
    std::thread::spawn(move || {
        let _ = child.wait();
        notify_shell_exit(id_for_exit, session_for_exit);
    });

    session
}

fn run_reader(session: Arc<Session>, mut reader: Box<dyn Read + Send>) {
    let mut buf = [0u8; 8192];
    let mut leftover: Vec<u8> = Vec::new();
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                leftover.extend_from_slice(&buf[..n]);
                let (decoded, rest) = decode_utf8_lossy_streaming(&leftover);
                leftover = rest;
                if decoded.is_empty() {
                    continue;
                }
                let Some(handle) = RUNTIME.lock().unwrap().clone() else {
                    continue;
                };
                let session = session.clone();
                handle.spawn(async move {
                    {
                        let mut sb = session.scrollback.lock().await;
                        sb.push_str(&decoded);
                        if sb.len() > SCROLLBACK_CAP {
                            let cut = sb.len() - SCROLLBACK_KEEP;
                            let mut idx = cut;
                            while idx < sb.len() && !sb.is_char_boundary(idx) {
                                idx += 1;
                            }
                            *sb = sb[idx..].to_string();
                        }
                    }
                    session
                        .send_to_attached(ServerMsg::Data { value: decoded })
                        .await;
                });
            }
            Err(_) => break,
        }
    }
}

/// Decode as much UTF-8 as possible from `buf`. Returns the decoded string
/// and any trailing incomplete byte sequence to retry on the next read.
fn decode_utf8_lossy_streaming(buf: &[u8]) -> (String, Vec<u8>) {
    match std::str::from_utf8(buf) {
        Ok(s) => (s.to_string(), Vec::new()),
        Err(e) => {
            let valid_up_to = e.valid_up_to();
            // SAFETY: 0..valid_up_to is valid UTF-8 by definition of `valid_up_to`.
            let head = std::str::from_utf8(&buf[..valid_up_to]).unwrap().to_string();
            let tail = &buf[valid_up_to..];
            match e.error_len() {
                Some(len) => {
                    let mut s = head;
                    s.push('\u{FFFD}');
                    let rest = tail[len..].to_vec();
                    let (more, last) = decode_utf8_lossy_streaming(&rest);
                    s.push_str(&more);
                    (s, last)
                }
                None => (head, tail.to_vec()),
            }
        }
    }
}

fn notify_shell_exit(id: String, session: Arc<Session>) {
    let Some(handle) = RUNTIME.lock().unwrap().clone() else {
        return;
    };
    handle.spawn(async move {
        session
            .send_to_attached(ServerMsg::Data {
                value: "\r\n\u{1b}[33mShell exited\u{1b}[0m\r\n".to_string(),
            })
            .await;
        {
            let mut a = session.attached.lock().await;
            *a = None;
        }
        if let Some(public) = {
            let g = PUBLIC_SESSIONS.lock().unwrap();
            g.clone()
        } {
            public.remove(&id).await;
        }
    });
}
