//! The engine (SPEC §3.16; owned by WP6): every user-visible action is a command dispatched by id.
//!
//! - `command.rs`: the registry, the `command!` macro, the access × origin policy, `command_list` rows.
//! - `params.rs`: strict params decoding (snake → camel, universal keys, JSON-pointer errors) and shared params.
//! - `session.rs`: documents, snapshot undo, dispatch, batches. `jobs.rs`: plan → validate → encode → summary.
//! - `devices.rs`: the device hub, framing, the pending ARM and `run_armed` (red zone). `requests.rs`: agent frame
//!   requests and job submissions (red zone). `inspect.rs`: the JSON views. `cmds/`: the Appendix A commands.
//!
//! Safety (AGENTS.md §1): nothing here emits on its own. `laser.start` only stores a `PendingArm`; the only call that
//! hands bytes to a driver's `run` is `Session::run_armed`, which takes an `ArmToken` that only `openlaser-arm` mints
//! (and only a human can make it mint). Human-only commands are refused for every other origin before their params
//! are decoded, and the typed device methods check the origin again.
#![forbid(unsafe_code)]

mod cmds;
mod command;
mod devices;
mod files;
mod geomx;
mod inspect;
mod jobs;
mod params;
pub mod prefs;
mod requests;
mod session;
#[cfg(test)]
mod tests;

pub use command::{
    Access, AgentClass, CommandRow, CommandSpec, DeviceClass, Disabled, EngineError, Kind, ListFilter, Origin, Registry, RunFn, params_schema, registry,
    run_typed,
};
pub use devices::{PENDING_ARM_TTL, STOP_WAIT};
pub use devices::{PendingArm, StopRegistry};
pub use files::FsAuthority;
pub use files::MAX_READ_BYTES;
pub use inspect::INSPECT_OBJECTS;
pub use jobs::GENERATOR;
pub use jobs::LiveCheck;
pub use params::{IdsParams, LayerKeySchema, MAX_PARAMS_DEPTH, NoParams, UNIVERSAL_KEYS, decode, normalise_id, snake_to_camel};
pub use requests::{
    FRAME_REQUEST_TTL, FrameRequestState, FrameRequestView, MAX_NOTE_CHARS, RequestId, Requests, SUBMISSION_TTL, SubmissionState, SubmissionView,
};
pub use session::{
    BatchResult, BuiltJob, CachedStatus, DeviceAccess, DocState, Encoded, Event, FileAccess, History, HistoryState, JobOptions, JobRefused, JournalEntry,
    Label, MAX_BATCH_STEPS, Session, SessionConfig,
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
