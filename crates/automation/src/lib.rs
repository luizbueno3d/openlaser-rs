//! Agent automation (SPEC §3.18, §4; owned by WP7): the MCP server, the headless backend and the GUI bridge.
//! This crate can never link `arm`, `human`, `transport` or a USB crate (§2.3 rule 5): agents can plan, simulate,
//! submit for a person's approval, and stop — never arm, emit, connect or frame by themselves.
#![forbid(unsafe_code)]

mod backend;
mod bridge;
mod mcp;
mod serve;
mod tools;

use std::io::Write;
use std::path::PathBuf;

pub use backend::{Bridge, Headless, ToolBackend, ToolOutput, png_from_result};
pub use bridge::{BridgeClient, BridgeError, StopHandle};
pub use tools::{ClientInfo, Hints, Mode, ToolAccess, ToolDef, check as check_arguments, strict_schema, tools};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
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

#[derive(Clone)]
pub enum BackendChoice {
    Headless,
    Bridge(BridgeTarget),
}

#[derive(Clone)]
pub enum BridgeTarget {
    /// Discovery file.
    Auto,
    File(PathBuf),
    Addr {
        addr: std::net::SocketAddr,
        token: TokenSource,
    },
}

#[derive(Clone)]
pub enum TokenSource {
    Inline(String),
    File(PathBuf),
    /// OPENLASER_CONTROL_TOKEN[_FILE].
    Env,
}

/// stdio MCP behind PhotoCraft's `serve_io` admission rules: 1 MiB line cap ends the session (after an error reply),
/// -32700 recovery, single writer, `_meta` 2026-07-28 list fields. Server info {name: "openlaser", version}. In bridge
/// mode nothing connects until the first call, and nothing ever falls back to headless.
pub fn run_mcp_stdio(cfg: McpConfig) -> Result<(), Error> {
    let client = ClientInfo { name: "openlaser-cli".to_owned(), version: env!("CARGO_PKG_VERSION").to_owned() };
    let backend: Box<dyn ToolBackend> = match cfg.backend {
        BackendChoice::Headless => Box::new(Headless::new(cfg.read_root.as_deref(), cfg.write_root.as_deref())?),
        BackendChoice::Bridge(target) => Box::new(Bridge::new(target, &client)),
    };
    mcp::serve(backend, cfg.compact, client, std::io::stdin().lock(), std::io::stdout())
}

pub struct ServeConfig {
    pub port: Option<u16>,
    pub token: Option<TokenSource>,
    pub read_root: Option<PathBuf>,
    pub write_root: Option<PathBuf>,
}

/// `openlaser-cli serve`: the control protocol over a headless Session (stdio: no auth; TCP: auth frame). With a
/// port, the token comes from `--control-token`, `--control-token-file` (created private if missing) or the
/// environment, or is generated and printed once on stderr.
pub fn run_serve(cfg: ServeConfig) -> Result<(), Error> {
    let mut backend = Headless::new(cfg.read_root.as_deref(), cfg.write_root.as_deref())?;
    if let Some(port) = cfg.port {
        let (token, generated) = serve::serve_token(cfg.token)?;
        let hex = token.to_hex();
        let (server, requests) = openlaser_control::server::ControlServer::start(serve::server_config(port, token))?;
        let mut err = std::io::stderr().lock();
        let _ = writeln!(err, "openlaser-cli serve: listening on 127.0.0.1:{}", server.port());
        if generated {
            let _ = writeln!(err, "openlaser-cli serve: control token {hex} (use --control-token-file <path> to keep one in a private file)");
        }
        drop(err);
        serve::serve_requests(&mut backend, &requests);
        server.shutdown();
        return Ok(());
    }
    serve::serve_lines(&mut backend, std::io::stdin().lock(), std::io::stdout().lock())
}
