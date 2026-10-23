//! Agent automation (SPEC §3.18, §4; owned by WP7): the MCP server, the headless backend and the GUI bridge.
//! This crate can never link `arm`, `human`, `transport` or a USB crate (§2.3 rule 5).
//!
//! WP0 stubs: the tool table is empty; servers and backends return `Error::NotImplemented`.
#![forbid(unsafe_code)]

mod backend;
mod bridge;
mod tools;

use std::path::PathBuf;

pub use backend::{Bridge, Headless, StopHandle, ToolBackend, ToolOutput};
pub use bridge::{BridgeClient, BridgeError};
pub use tools::{ClientInfo, Hints, Mode, ToolAccess, ToolDef, strict_schema, tools};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("control: {0}")]
    Control(#[from] openlaser_control::Error),
    #[error("bridge: {0:?}")]
    Bridge(BridgeError),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

pub struct McpConfig {
    pub backend: BackendChoice,
    pub read_root: Option<PathBuf>,
    pub write_root: Option<PathBuf>,
    pub compact: bool,
}

pub enum BackendChoice {
    Headless,
    Bridge(BridgeTarget),
}

pub enum BridgeTarget {
    /// Discovery file.
    Auto,
    File(PathBuf),
    Addr {
        addr: std::net::SocketAddr,
        token: TokenSource,
    },
}

pub enum TokenSource {
    Inline(String),
    File(PathBuf),
    /// OPENLASER_CONTROL_TOKEN[_FILE].
    Env,
}

/// stdio MCP (rmcp 3.5) behind PhotoCraft's `serve_io` pre-reader: 1 MiB line cap ends the session, -32700
/// recovery, single writer, `_meta` 2026-07-28 list fields. Server info {name: "openlaser", version}.
pub fn run_mcp_stdio(cfg: McpConfig) -> Result<(), Error> {
    let _ = cfg;
    Err(Error::NotImplemented("openlaser_automation::run_mcp_stdio"))
}

pub struct ServeConfig {
    pub port: Option<u16>,
    pub token: Option<TokenSource>,
    pub read_root: Option<PathBuf>,
    pub write_root: Option<PathBuf>,
}

/// `openlaser-cli serve`: the control protocol over a headless Session (stdio: no auth; TCP: auth frame).
pub fn run_serve(cfg: ServeConfig) -> Result<(), Error> {
    let _ = cfg;
    Err(Error::NotImplemented("openlaser_automation::run_serve"))
}
