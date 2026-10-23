//! The two tool backends. WP0 stubs.

use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::Error;
use crate::bridge::BridgeClient;
use crate::tools::{ClientInfo, Mode};

pub enum ToolOutput {
    Json(serde_json::Value),
    Png { png: Vec<u8>, note: serde_json::Value },
    Error { message: String, data: Option<serde_json::Value> },
}

/// The MCP layer is identical over both backends. Tool calls run one at a time behind a mutex (PhotoCraft), EXCEPT
/// `stop`, which the server routes to `stop_handle()` without taking that mutex, so a long `job_simulate` or a
/// 55 s bridge timeout can never delay a stop.
pub trait ToolBackend: Send {
    fn mode(&self) -> Mode;
    fn call(&mut self, tool: &str, args: serde_json::Value, client: &ClientInfo) -> ToolOutput;
    /// Bridge: a handle with its own authenticated connection, used only for `machine.stop`. Headless: None (the
    /// `stop` tool then returns the `needs_bridge:` error).
    fn stop_handle(&self) -> Option<StopHandle>;
}

/// Arc<Mutex<dedicated BridgeClient connection>>.
#[derive(Clone)]
pub struct StopHandle {
    conn: Arc<Mutex<Option<BridgeClient>>>,
}

impl StopHandle {
    /// WP0 addition: wraps the dedicated stop connection (`BridgeClient::stop_handle` builds it).
    pub(crate) fn new(conn: Option<BridgeClient>) -> Self {
        StopHandle { conn: Arc::new(Mutex::new(conn)) }
    }

    /// Waits ≤ STOP_WAIT for the StopAck. WP0 stub: reports that nothing was sent.
    pub fn stop(&self, machine: Option<&str>) -> ToolOutput {
        let connected = self.conn.lock().map(|c| c.is_some()).unwrap_or(false);
        ToolOutput::Error {
            message: format!("not implemented: StopHandle::stop (machine {machine:?}, connected {connected}); the request was not sent"),
            data: None,
        }
    }
}

/// engine::Session with origin Headless, window false, DeviceAccess::None; files through AuthorizedWorkspace.
pub struct Headless {
    _private: (),
}

impl Headless {
    pub fn new(read_root: Option<&Path>, write_root: Option<&Path>) -> Result<Self, Error> {
        let _ = (read_root, write_root);
        Err(Error::NotImplemented("openlaser_automation::Headless::new"))
    }
}

impl ToolBackend for Headless {
    fn mode(&self) -> Mode {
        Mode::Headless
    }

    fn call(&mut self, tool: &str, args: serde_json::Value, client: &ClientInfo) -> ToolOutput {
        let _ = (args, client);
        ToolOutput::Error { message: format!("not implemented: headless tool `{tool}`"), data: None }
    }

    fn stop_handle(&self) -> Option<StopHandle> {
        None
    }
}

pub struct Bridge {
    client: BridgeClient,
}

impl ToolBackend for Bridge {
    fn mode(&self) -> Mode {
        Mode::Bridge
    }

    fn call(&mut self, tool: &str, args: serde_json::Value, client: &ClientInfo) -> ToolOutput {
        let _ = (args, client, &self.client);
        ToolOutput::Error { message: format!("not implemented: bridge tool `{tool}`"), data: None }
    }

    /// WP0 stub: None until the bridge opens its dedicated stop connection.
    fn stop_handle(&self) -> Option<StopHandle> {
        None
    }
}
