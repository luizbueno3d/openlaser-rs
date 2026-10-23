//! The loopback control server (std::net on 127.0.0.1 only). WP0 stubs: it never starts.

use std::sync::Arc;
use std::sync::atomic::AtomicU8;

use crate::{Error, token, wire};

pub struct ControlRequest {
    pub conn: u64,
    pub client: String,
    pub request: wire::Request,
    pub reply: std::sync::mpsc::Sender<wire::Reply>,
    pub deadline: std::time::Instant,
    pub dispatch: Arc<Dispatch>,
}

/// QUEUED → DISPATCHED (UI thread starts it) or QUEUED → ABANDONED (deadline passed), settled once, so a
/// timeout says either "not run, safe to resend" or "may still apply".
#[derive(Debug, Default)]
pub struct Dispatch {
    state: AtomicU8,
}

impl Dispatch {
    /// WP0 stub: false.
    pub fn try_dispatch(&self) -> bool {
        let _ = &self.state;
        false
    }

    /// WP0 stub: false.
    pub fn abandon(&self) -> bool {
        false
    }
}

/// `machine.stop` is answered on the transport thread through this, before anything touches the UI queue.
pub type FastStop = Arc<dyn Fn(Option<&str>) -> serde_json::Value + Send + Sync>;

pub struct ServerConfig {
    /// 0 = free port.
    pub port: u16,
    pub token: token::Token,
    pub fast_stop: FastStop,
    pub wake: Arc<dyn Fn() + Send + Sync>,
}

/// std::net on 127.0.0.1 only; ≤ 16 connections; 30 s idle; first frame must be
/// {"method":"auth","params":{"token","client":{"name","version"}}}.
pub struct ControlServer {
    port: u16,
}

impl ControlServer {
    pub fn start(cfg: ServerConfig) -> Result<(Self, std::sync::mpsc::Receiver<ControlRequest>), Error> {
        let _ = cfg;
        Err(Error::NotImplemented("openlaser_control::server::ControlServer::start"))
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn clients(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn disconnect_all(&self) {}

    pub fn shutdown(self) {}
}
