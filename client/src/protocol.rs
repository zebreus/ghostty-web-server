//! Wire-protocol envelopes. Mirror of `server/src/protocol.rs`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClientMsg {
    Input { value: String },
    Resize { cols: u16, rows: u16 },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerMsg {
    Data { value: String },
    Ack { cols: u16, rows: u16 },
}
