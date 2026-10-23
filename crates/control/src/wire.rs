//! The line envelope. WP0 stubs for the reader and writer.

use serde::{Deserialize, Serialize};

use crate::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: serde_json::Value,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub id: serde_json::Value,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Reads a whole line before returning any byte; a line over `max` is an error (the caller closes or skips).
pub fn read_line_bounded(r: &mut impl std::io::BufRead, max: usize) -> Result<Option<Vec<u8>>, Error> {
    let _ = (r, max);
    Err(Error::NotImplemented("openlaser_control::wire::read_line_bounded"))
}

/// Oversized replies are replaced by a small error reply with the same id ("…operation may have completed").
pub fn write_reply(w: &mut impl std::io::Write, reply: &Reply) -> std::io::Result<()> {
    let _ = (w, reply);
    Err(std::io::Error::other("not implemented: openlaser_control::wire::write_reply"))
}
