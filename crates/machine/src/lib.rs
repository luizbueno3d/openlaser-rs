//! Machine profiles (SPEC §3.4; owned by WP1). No I/O.
//!
//! WP0 stubs: the data types are complete; `jpt_m7_200`/`_sim` carry the SPEC's numbers (WP1 checks them against
//! the OM profile); the queries and importers return neutral values or `Error::NotImplemented`.
#![forbid(unsafe_code)]

mod builtin;
pub mod markcfg;
pub mod om;
mod profile;

pub use builtin::{jpt_m7_200, jpt_m7_200_sim, om_gantries};
pub use profile::{
    FieldCalibration, FieldRect, FirstJump, GalvoLink, GalvoMachine, GalvoTimingDefaults, GantryLink, GantryMachine, GantryTool, JobLimits, Kinematics,
    LaserSource, LmcBoardFilter, MachineProfile, PortMap, ProfileStore, RedDotCal, RunAhead, SourceKind, validate_profile,
};

// Re-exports for callers that name these through the profile (SPEC §3.0).
pub use openlaser_doc::{LayerDefaults, WorkspaceOrigin};
pub use openlaser_types::{DeviceKind, MachineId, Mm, MmPerS, Us};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}
