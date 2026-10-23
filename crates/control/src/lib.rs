//! The control protocol (SPEC §3.17, §4.5; owned by WP7). std only; loopback only.
//!
//! WP0 stubs: the data types, budgets and wire envelope are complete; token, discovery, workspace and server
//! functions return `Error::NotImplemented` (or fail closed: `Token::matches` is always false).
#![forbid(unsafe_code)]

pub mod budgets;
pub mod discovery;
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
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}
