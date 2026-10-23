//! The real-device edge (SPEC §3.15, §5.1, §2.6; owned by WP5). Compiled, never run by agents (AGENTS.md §1).
//!
//! Hardware is default-deny (S23): every USB/serial/TCP open and every `detect` first requires `OPENLASER_NO_HW`
//! unset and `OPENLASER_HW_PRESENT=1`, then (S17) a valid bench session, then the board lock. Only the apps' `hw`
//! builds link this crate. No USB reset, no retry, no unbounded blocking transfer (§2.3 rule 15): every transfer
//! error surfaces as an `XferError` and the driver stops the job.
#![forbid(unsafe_code)]

use std::ffi::OsStr;
use std::path::PathBuf;

use openlaser_types::RefusalCode;

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
/// The per-user data folder name (`craft_settings::paths::data_dir`), shared with the prototype's board lock.
pub const APP_DIR: &str = "OpenLaser";

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("hardware access is disabled in this build or environment")]
    HardwareDisabled,
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

/// The S23 rule on two environment values: disabled when `OPENLASER_NO_HW` is set (to anything, even empty) or
/// `OPENLASER_HW_PRESENT` is not exactly "1". Nothing missing ever enables hardware.
pub fn hardware_disabled_by(no_hw: Option<&OsStr>, hw_present: Option<&OsStr>) -> bool {
    no_hw.is_some() || hw_present.is_none_or(|v| v != "1")
}

/// Default deny (S23), read from the process environment. Checked before anything else.
pub fn hardware_disabled() -> bool {
    hardware_disabled_by(std::env::var_os(ENV_NO_HW).as_deref(), std::env::var_os(ENV_HW_PRESENT).as_deref())
}

/// The refusal every open and detect returns while hardware is disabled.
pub fn hardware_refusal() -> Refusal {
    Refusal::new(
        RefusalCode::HardwareDisabled,
        "hardware access is disabled: it needs a `hw` build, OPENLASER_HW_PRESENT=1, OPENLASER_NO_HW unset and a bench session (openlaser-hw bench start)",
    )
}

/// `<data_dir>/OpenLaser` (§2.6: `%LOCALAPPDATA%\OpenLaser` on Windows, `~/Library/Application Support/OpenLaser`
/// on macOS), where the board lock and the bench session live.
pub fn data_dir() -> std::result::Result<PathBuf, Refusal> {
    craft_settings::paths::data_dir(APP_DIR)
        .ok_or_else(|| Refusal::new(RefusalCode::BenchSessionRequired, "this system has no per-user data folder for the board lock and bench session"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_hw_env_refuses() {
        let one = Some(OsStr::new("1"));
        assert!(hardware_disabled_by(Some(OsStr::new("1")), one));
        assert!(hardware_disabled_by(Some(OsStr::new("")), one), "set to anything, even empty");
        assert!(hardware_disabled_by(Some(OsStr::new("0")), one));
    }

    #[test]
    fn hw_present_unset_refuses() {
        assert!(hardware_disabled_by(None, None));
        assert!(hardware_disabled_by(None, Some(OsStr::new("true"))));
        assert!(hardware_disabled_by(None, Some(OsStr::new(" 1"))));
        assert!(hardware_disabled_by(None, Some(OsStr::new("0"))));
        assert!(!hardware_disabled_by(None, Some(OsStr::new("1"))), "the only enabling combination");
    }

    #[test]
    fn data_dir_is_per_user_and_named() {
        if let Ok(d) = data_dir() {
            assert!(d.ends_with(APP_DIR));
        }
    }
}
