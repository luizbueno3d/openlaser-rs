//! The control protocol (SPEC §3.17, §4.5; owned by WP7). std only; loopback only.
//!
//! Token, discovery file, bounded wire, method table, `AuthorizedWorkspace` and the loopback `ControlServer` with
//! the `machine.stop` fast path. Every gate fails closed.
#![forbid(unsafe_code)]

pub mod budgets;
pub mod discovery;
mod fsutil;
pub mod methods;
pub mod server;
pub mod token;
pub mod wire;
pub mod workspace;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("line longer than {max} bytes")]
    LineTooLong { max: usize },
    #[error("bad token: {0}")]
    BadToken(String),
    #[error("path refused: {0}")]
    PathRefused(String),
    /// A secret file other users can read, or a file that is not what it claims to be.
    #[error("insecure: {0}")]
    Insecure(String),
    /// The discovery file names a process that is gone or is not the OpenLaser that wrote it.
    #[error("stale discovery: {0}")]
    Stale(String),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}
