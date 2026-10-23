//! The bridge client: one authenticated loopback connection to a running OpenLaser window. WP0 stubs.

use crate::BridgeTarget;
use crate::backend::StopHandle;
use crate::tools::ClientInfo;

/// One authenticated loopback connection.
pub struct BridgeClient {
    _private: (),
}

impl BridgeClient {
    pub fn connect(target: &BridgeTarget, client: &ClientInfo) -> Result<Self, BridgeError> {
        let _ = (target, client);
        Err(BridgeError::NotImplemented("openlaser_automation::BridgeClient::connect"))
    }

    /// Never resent.
    pub fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, BridgeError> {
        let _ = (method, params);
        Err(BridgeError::NotImplemented("openlaser_automation::BridgeClient::call"))
    }

    /// One retry.
    pub fn call_read(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, BridgeError> {
        let _ = (method, params);
        Err(BridgeError::NotImplemented("openlaser_automation::BridgeClient::call_read"))
    }

    /// Opens a SECOND authenticated connection reserved for `machine.stop` (reconnects on use if idle-dropped).
    pub fn stop_handle(&self) -> Result<StopHandle, BridgeError> {
        let _ = StopHandle::new(None);
        Err(BridgeError::NotImplemented("openlaser_automation::BridgeClient::stop_handle"))
    }
}

/// Wording per AUT A8: NotSent "the request was not sent"; Transport "operation may have completed; inspect state
/// before retrying"; TimedOut read "may be retried safely" / write "may have completed"; Definite = the app's error.
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeError {
    NotSent(String),
    Transport(String),
    TimedOut {
        read: bool,
    },
    Definite(String),
    /// Build-only (SPEC §3.0); removed at I3.
    NotImplemented(&'static str),
}
