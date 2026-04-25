//! WebSocket wire protocol — JSON envelopes in both directions.
//!
//! Mirrors the original TypeScript:
//!
//! ```ts
//! type ServerMsg = { type: 'data'; value: string }
//!                | { type: 'ack'; cols: number; rows: number };
//! type ClientMsg = { type: 'input'; value: string }
//!                | { type: 'resize'; cols: number; rows: number };
//! ```

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerMsg {
    Data { value: String },
    Ack { cols: u16, rows: u16 },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClientMsg {
    Input { value: String },
    Resize { cols: u16, rows: u16 },
}
