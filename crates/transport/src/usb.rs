//! USB (nusb 0.2.8; WinUSB on Windows, §2.6 item 4). WP0 stubs: no enumeration, no open.

use std::time::Duration;

use openlaser_driver::{ByteLink, DetectedDevice, DeviceProvider, DriverError};
use openlaser_lmc::{LmcTransport, Packet, Single, XferError};
use openlaser_machine::{GalvoMachine, GantryMachine};
use openlaser_types::{Refusal, RefusalCode};
use serde::Serialize;

use crate::hardware_disabled;

/// The real-device provider. open_lmc requires, in order: OPENLASER_NO_HW unset and OPENLASER_HW_PRESENT=1; a valid
/// bench session (S17); the board lock; vid/pid match (0x9980 → CloneBoard refusal, never firmware); the device
/// configured (macOS `claim_interface` cannot find interface 0 on an unconfigured device: call
/// `set_configuration(1)` only if the profile's `usb_set_configuration` is true, else refuse `UsbNotConfigured`);
/// then claims interface 0, bulk OUT 0x02, bulk IN 0x88. Holds a `keepawake` assertion for the session (§2.6).
#[derive(Debug, Default)]
pub struct UsbProvider {
    _private: (),
}

impl UsbProvider {
    pub fn new() -> Self {
        UsbProvider { _private: () }
    }
}

fn refused(what: &'static str) -> DriverError {
    if hardware_disabled() {
        DriverError::Refused(Refusal::new(
            RefusalCode::HardwareDisabled,
            "hardware access is disabled (needs OPENLASER_HW_PRESENT=1 and OPENLASER_NO_HW unset, a hw build and a bench session)",
        ))
    } else {
        DriverError::NotImplemented(what)
    }
}

/// detect() also needs OPENLASER_NO_HW unset and OPENLASER_HW_PRESENT=1. WP0 stub: everything is refused.
impl DeviceProvider for UsbProvider {
    fn name(&self) -> &'static str {
        "usb"
    }

    fn detect(&self) -> Result<Vec<DetectedDevice>, DriverError> {
        Err(refused("openlaser_transport::UsbProvider::detect"))
    }

    fn open_lmc(&self, m: &GalvoMachine) -> Result<Box<dyn LmcTransport>, DriverError> {
        let _ = m;
        Err(refused("openlaser_transport::UsbProvider::open_lmc"))
    }

    fn open_link(&self, m: &GantryMachine) -> Result<Box<dyn ByteLink>, DriverError> {
        let _ = m;
        Err(refused("openlaser_transport::UsbProvider::open_link"))
    }

    /// `bench::emission_cap_pct`.
    fn emission_cap_pct(&self) -> Result<Option<f64>, DriverError> {
        Err(refused("openlaser_transport::UsbProvider::emission_cap_pct"))
    }

    /// `bench::record_emission`.
    fn record_emission(&self) -> Result<(), DriverError> {
        Err(refused("openlaser_transport::UsbProvider::record_emission"))
    }
}

/// nusb 0.2.8. Writes are exactly 12 or 3072 bytes; reads 8 bytes; `submit` + `wait_next_complete(≤ 20 ms)` slices;
/// each transfer has a total budget of `transfer_timeout_ms` (250), after which it is `XferError::Timeout`. NO
/// reset, NO retry, NO `transfer_blocking` (§2.3 rule 15): every error is mapped to XferError and surfaces as a sticky
/// fault plus an emergency stop.
#[derive(Debug)]
pub struct UsbLmc {
    _private: (),
}

fn xfer_refused(what: &'static str) -> XferError {
    if hardware_disabled() { XferError::HardwareDisabled } else { XferError::NotImplemented(what) }
}

impl LmcTransport for UsbLmc {
    fn submit_single(&mut self, cmd: Single) -> Result<(), XferError> {
        let _ = cmd;
        Err(xfer_refused("openlaser_transport::UsbLmc::submit_single"))
    }

    fn poll_single(&mut self, slice: Duration) -> Result<Option<Option<[u16; 4]>>, XferError> {
        let _ = slice;
        Err(xfer_refused("openlaser_transport::UsbLmc::poll_single"))
    }

    fn submit_packet(&mut self, pkt: &Packet) -> Result<(), XferError> {
        let _ = pkt;
        Err(xfer_refused("openlaser_transport::UsbLmc::submit_packet"))
    }

    fn poll_packet(&mut self, slice: Duration) -> Result<Option<()>, XferError> {
        let _ = slice;
        Err(xfer_refused("openlaser_transport::UsbLmc::poll_packet"))
    }

    fn cancel_all(&mut self) {}

    fn drain_replies(&mut self) {}
}

/// Read-only enumeration (descriptors only; never opens a device). WP0 stub: refuses.
pub fn detect() -> Result<Vec<DetectedBoard>, XferError> {
    Err(xfer_refused("openlaser_transport::detect"))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedBoard {
    pub vid: u16,
    pub pid: u16,
    pub serial: Option<String>,
    pub location: String,
    pub supported: bool,
    pub note: String,
}
