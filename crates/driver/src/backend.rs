//! The backend trait, the driver's state and events.

use std::time::Duration;

use openlaser_machine::DeviceKind;
use openlaser_types::{Fingerprint, FrameMode, FrameOrigin, FrameReceipt, MachineId, MonoTime, Refusal, RefusalCode, SessionId};
use serde::Serialize;

use crate::stop::{Cx, StopReport};

pub trait EncodedJob: Send + 'static {
    fn fingerprint(&self) -> &Fingerprint;
    fn machine(&self) -> &MachineId;
    fn emits(&self) -> bool;
    /// S22; recomputed by the runtime from the object it was handed.
    fn bytes_hash(&self) -> Fingerprint;
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub kind: DeviceKind,
    pub frame: bool,
    pub run: bool,
    pub pause: bool,
    pub jog: bool,
    pub home: bool,
    pub inputs: bool,
}

/// The device side of one machine, driven only from its driver thread.
///
/// Error convention (WP5): `connect`, `frame` and `run` return `Err(DriverError::Refused(..))` only for a refusal
/// decided before anything reached the wire. When the latch trips during an operation, the backend runs its own
/// [`Backend::emergency_stop`] and returns the `Stopped(report)` outcome (`connect` returns `Err` and the runtime
/// stops it). Any other `Err` is a fault: the backend returns at once WITHOUT stopping, and the runtime calls
/// `emergency_stop`, raises the sticky fault and posts the `StopReport`.
pub trait Backend: Send + 'static {
    type Job: EncodedJob;
    type Frame: EncodedJob;
    fn capabilities(&self) -> Capabilities;
    fn connect(&mut self, cx: &mut Cx) -> Result<DeviceInfo, DriverError>;
    /// S20: emits FrameCompleted(receipt) only after the board has EXECUTED the traced list (status !BUSY after it).
    /// Once: one list, one receipt with `full_passes = once_passes`. Loop: one list per pass, a receipt after each
    /// (`full_passes` counts executed passes), until stopped. A stop before !BUSY gives no receipt for that list.
    fn frame(&mut self, f: &Self::Frame, mode: FrameMode, origin: FrameOrigin, session: SessionId, cx: &mut Cx) -> Result<FrameOutcome, DriverError>;
    /// The runtime has already checked the token, the nonce and its own receipt record.
    fn run(&mut self, job: &Self::Job, cx: &mut Cx) -> Result<Outcome, DriverError>;
    /// Never returns early; always ends with MO closed and ports at 0, or reports that it could not confirm.
    fn emergency_stop(&mut self, cx: &mut Cx) -> StopReport;
    /// A failed read is Err, never bits.
    fn status(&mut self, cx: &mut Cx) -> Result<MachineStatus, DriverError>;
    fn disconnect(&mut self, cx: &mut Cx);
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum MachineState {
    Disconnected,
    Connecting,
    Idle,
    Framing,
    Running { packets_acked: u32, total: u32 },
    Stopping,
    Stopped,
    Fault,
    StopNotConfirmed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineStatus {
    pub state: MachineState,
    pub status_bits: Option<u16>,
    pub ready: bool,
    pub busy: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub vid: u16,
    pub pid: u16,
    pub serial: Option<String>,
    pub version: Option<[u16; 4]>,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum FrameOutcome {
    Completed { passes: u32 },
    Stopped(StopReport),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Outcome {
    Finished {
        #[serde(rename = "durationS", with = "openlaser_types::serde_secs")]
        duration: Duration,
    },
    Stopped(StopReport),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error, Serialize)]
pub enum DriverError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("board not READY within the progress budget; stopped")]
    Watchdog { waited_ms: u64 },
    #[error("refused: {0:?}")]
    Refused(Refusal),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("driver thread panicked; stop was sent")]
    PanicContained,
    #[error("not connected")]
    NotConnected,
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

impl DriverError {
    /// A refusal with its code and one actionable sentence.
    pub fn refused(code: RefusalCode, message: impl Into<String>) -> Self {
        DriverError::Refused(Refusal::new(code, message))
    }

    /// True for a refusal (nothing was sent); false for a fault of the machine or the link.
    pub fn is_refusal(&self) -> bool {
        matches!(self, DriverError::Refused(_))
    }
}

/// What the driver thread reports. A command the driver thread refuses (`StoppedAfterQueue`, `MachineBusy`,
/// `NotConnected`, ARM re-checks …) arrives as `Fault(DriverError::Refused(..))`: nothing was sent, and the
/// machine's state does not change.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum DriverEvent {
    Connected(DeviceInfo),
    State(MachineState),
    FrameCompleted(FrameReceipt),
    Progress {
        packets_acked: u32,
        total: u32,
        eta_ms: u64,
    },
    Finished(Outcome),
    Stopped(StopReport),
    /// Sticky until acknowledged.
    Fault(DriverError),
    Trace(TraceLine),
}

/// Disassembled, for the Console panel.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceLine {
    pub at: MonoTime,
    pub out: bool,
    pub text: String,
}
