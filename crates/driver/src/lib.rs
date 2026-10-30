//! The driver runtime (SPEC §3.13, §5.3–§5.4; owned by WP5, red zone).
//!
//! - [`spawn`] starts one OS thread per machine that alone owns its [`Backend`]; [`DriverHandle`] queues commands to
//!   it, and its `Drop` is the dead-man rule. [`StopLatch`] / [`Stopper`] are the stop path: an atomic store and a
//!   wake, readable between every transfer, with `stop_and_wait` for the [`StopReport`].
//! - [`LmcBackend`] is the LMC session of §5.3 (flow control, run-ahead window, watchdog, executed-pass receipts,
//!   emergency stop); [`GrblBackend`] streams G-code to GRBL (§5.7).
//! - [`DeviceProvider`] and [`ByteLink`] are the seams the apps inject; only `openlaser-transport` touches devices.
#![forbid(unsafe_code)]

mod backend;
mod grbl;
mod lmc_backend;
mod lmc_ops;
mod lmc_session;
mod runtime;
mod stop;

pub use backend::{Backend, Capabilities, DeviceInfo, DriverError, DriverEvent, EncodedJob, FrameOutcome, MachineState, MachineStatus, Outcome, TraceLine};
pub use grbl::{ByteLink, DEFAULT_RX_SIZE, GrblBackend, LinkError};
pub use lmc_backend::LmcBackend;
pub use runtime::{AnyDriver, DEAD_MAN_JOIN, DetectedDevice, DeviceProvider, DriverHandle, spawn};
pub use stop::{Cx, MAX_SLICE, StopAck, StopLatch, StopReport, Stopped, Stopper, TRACE_LINES_PER_OPERATION};

// Re-exports for callers that name these through the driver (SPEC §3.0).
pub use crossbeam_channel::{Receiver, Sender};
pub use openlaser_gcode::{GcodeFrame, GcodeProgram};
pub use openlaser_lmc::{LmcFrameJob, LmcJob, LmcTransport, Packet, Single, XferError};
pub use openlaser_machine::{GalvoMachine, GantryMachine};
pub use openlaser_types::{ArmToken, Clock, DeviceKind, Fingerprint, FrameMode, FrameOrigin, FrameReceipt, MachineId, MonoTime, Refusal, SessionId};

pub type Result<T, E = DriverError> = std::result::Result<T, E>;
