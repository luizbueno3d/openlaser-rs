//! Machine profiles (SPEC §3.4; owned by WP1). No I/O.
//!
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
    #[error("invalid machine profile: {0}")]
    InvalidProfile(String),
    #[error("invalid markcfg: {0}")]
    InvalidMarkCfg(String),
    #[error("invalid OM calibration: {0}")]
    InvalidCalibration(String),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}
