//! The engine (SPEC §3.16; owned by WP6): every user-visible action is a command dispatched by id.
//!
//! WP0 stubs: the data types are complete; dispatch, editing, the job pipeline and the device hub return
//! `EngineError::NotImplemented`. `StopRegistry::stop_all` works (stop always works, SPEC §1.3 rule 5).
#![forbid(unsafe_code)]

mod command;
mod requests;
mod session;

pub use command::{
    Access, AgentClass, CommandRow, CommandSpec, DeviceClass, Disabled, EngineError, Kind, ListFilter, Origin, Registry, RunFn, decode, registry,
};
pub use requests::{FrameRequestState, FrameRequestView, RequestId, Requests, SubmissionState, SubmissionView};
pub use session::{
    BatchResult, BuiltJob, CachedStatus, DeviceAccess, DocState, Encoded, Event, FileAccess, FsAuthority, History, HistoryState, JobOptions, JobRefused, Label,
    LiveCheck, PendingArm, Session, SessionConfig, StopRegistry,
};

// Re-exports so callers can name these without a direct dependency (SPEC §3.0).
pub use openlaser_doc::{Project, Selection};
pub use openlaser_driver::{DeviceProvider, DriverError, DriverEvent, MachineState, Outcome, StopAck, StopReport, Stopper, TraceLine};
pub use openlaser_gcode::{GcodeFrame, GcodeProgram};
pub use openlaser_lmc::{Listing, LmcFrameJob, LmcJob};
pub use openlaser_machine::{MachineProfile, ProfileStore};
pub use openlaser_plan::JobPlan;
pub use openlaser_safety::Validated;
pub use openlaser_types::{
    ArmToken, Clock, EmissionSummary, EmittedSegment, Fingerprint, FrameMode, FrameReceipt, FrameSource, FrameStyle, LayerKey, MachineId, MonoTime, Notice,
    ObjectId, Refusal, SessionId,
};

pub type Result<T, E = EngineError> = std::result::Result<T, E>;
