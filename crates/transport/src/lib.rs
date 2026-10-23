//! The real-device edge (SPEC §3.15, §5.1, §2.6; owned by WP5). Compiled, never run by agents (AGENTS.md §1).
//!
//! WP0 stubs touch no device: nothing here enumerates or opens USB, a serial port or a socket. Every open and
//! every transfer is refused (`HardwareDisabled` unless `OPENLASER_NO_HW` is unset and `OPENLASER_HW_PRESENT=1`,
//! S23; else `NotImplemented`).
#![forbid(unsafe_code)]

pub mod bench;
mod links;
mod lock;
mod usb;

pub use links::{SerialLink, TcpLink};
pub use lock::BoardLock;
pub use usb::{DetectedBoard, UsbLmc, UsbProvider, detect};

// Re-exports for callers that name these through the transport (SPEC §3.0).
pub use openlaser_driver::{ByteLink, DetectedDevice, DeviceProvider, DriverError, LinkError};
pub use openlaser_lmc::{LmcTransport, XferError};
pub use openlaser_types::Refusal;

/// Set → every open/detect returns HardwareDisabled.
pub const ENV_NO_HW: &str = "OPENLASER_NO_HW";
/// Must equal "1", else HardwareDisabled (S23, default deny).
pub const ENV_HW_PRESENT: &str = "OPENLASER_HW_PRESENT";

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("hardware access is disabled in this build or environment")]
    HardwareDisabled,
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

/// Default deny (S23): disabled when `OPENLASER_NO_HW` is set (to anything) or `OPENLASER_HW_PRESENT` is not exactly
/// "1". Checked before anything else.
pub(crate) fn hardware_disabled() -> bool {
    std::env::var_os(ENV_NO_HW).is_some() || std::env::var_os(ENV_HW_PRESENT).is_none_or(|v| v != "1")
}
