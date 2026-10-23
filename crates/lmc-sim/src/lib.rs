//! The simulator (SPEC §3.14; owned by WP4). Every device test runs here (SPEC §1.3 rule 1).
//!
//! WP0 stubs: the types are complete; the fake board refuses every transfer with `NotImplemented`.
#![forbid(unsafe_code)]

mod fake;
pub mod grbl;
mod provider;

pub use fake::{FakeLmc, Fault, Recorder, Recording, SimConfig, SimWire, WireHook, simulate_job};
pub use provider::SimProvider;

// Re-exports for callers that name these through the simulator (SPEC §3.0).
pub use openlaser_driver::{ByteLink, DetectedDevice, DeviceProvider, LinkError};
pub use openlaser_lmc::{LmcJob, LmcTransport, XferError};
pub use openlaser_types::{Clock, EmittedSegment, MonoTime, SystemClock, VirtualClock};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}
