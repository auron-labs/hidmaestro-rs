//! Wire protocol shared with the `HIDMaestro.Bridge` host process.
//!
//! Requests and responses are newline-delimited JSON on the child's
//! stdin/stdout. Events are interleaved unsolicited lines:
//!
//! ```text
//! -> {"id":1,"method":"create_controller","params":{"profile_id":"xbox-360-wired"}}
//! <- {"event":"output","data":{...}}            (any time, any controller)
//! <- {"id":1,"ok":true,"result":{...}}
//! <- {"id":2,"ok":false,"error":"profile 'foo' not found"}
//! ```

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct Request<P: Serialize> {
    pub id: u64,
    pub method: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<P>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Inbound {
    Response {
        id: u64,
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        result: Option<serde_json::Value>,
        #[serde(default)]
        error: Option<String>,
    },
    Event {
        #[serde(rename = "event")]
        _kind: String,
        data: serde_json::Value,
    },
}
