//! The driver runtime (SPEC §3.13, §5.3–§5.4; owned by WP5, red zone).
//!
//! WP0 stubs: the data types and traits are complete. `StopLatch`, `Stopper` and `Cx` work (a stop must always be
//! deliverable); `spawn` starts no thread yet, and the backends refuse everything with `NotImplemented`.
#![forbid(unsafe_code)]

mod backend;
mod grbl;
mod lmc_backend;
mod runtime;
mod stop;

pub use backend::{Backend, Capabilities, DeviceInfo, DriverError, DriverEvent, EncodedJob, FrameOutcome, MachineState, MachineStatus, Outcome, TraceLine};
pub use grbl::{ByteLink, GrblBackend, LinkError};
pub use lmc_backend::LmcBackend;
pub use runtime::{AnyDriver, DetectedDevice, DeviceProvider, DriverHandle, spawn};
pub use stop::{Cx, StopAck, StopLatch, StopReport, Stopped, Stopper};

// Re-exports for callers that name these through the driver (SPEC §3.0).
pub use crossbeam_channel::{Receiver, Sender};
pub use openlaser_gcode::{GcodeFrame, GcodeProgram};
pub use openlaser_lmc::{LmcFrameJob, LmcJob, LmcTransport, XferError};
pub use openlaser_machine::{GalvoMachine, GantryMachine};
pub use openlaser_types::{ArmToken, Clock, DeviceKind, Fingerprint, FrameMode, FrameOrigin, FrameReceipt, MachineId, MonoTime, Refusal, SessionId};

pub type Result<T, E = DriverError> = std::result::Result<T, E>;
