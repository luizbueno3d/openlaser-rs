//! The session: documents, history, machines, jobs and devices. WP0 stubs.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use openlaser_doc::{Project, Selection};
use openlaser_driver::{DeviceProvider, DriverError, DriverEvent, MachineState, StopAck, StopReport, Stopper, TraceLine};
use openlaser_gcode::{GcodeFrame, GcodeProgram};
use openlaser_lmc::{Listing, LmcFrameJob, LmcJob};
use openlaser_machine::{MachineProfile, ProfileStore};
use openlaser_plan::JobPlan;
use openlaser_safety::Validated;
use openlaser_types::{
    ArmToken, Clock, EmissionSummary, EmittedSegment, Fingerprint, FrameMode, FrameReceipt, FrameSource, FrameStyle, LayerKey, MachineId, MonoTime, Notice,
    Refusal, RefusalCode, SessionId,
};
use serde::Serialize;

use crate::command::{CommandRow, EngineError, ListFilter, Origin};
use crate::requests::{FrameRequestView, RequestId, Requests, SubmissionView};

/// Automation adapts control::AuthorizedWorkspace to this.
pub trait FileAccess: Send + Sync {
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;
    fn write_atomic(&self, path: &str, bytes: &[u8]) -> Result<(), String>;
}

pub enum FsAuthority {
    Denied,
    Scoped(Box<dyn FileAccess>),
    TrustedLocal,
}

pub enum DeviceAccess {
    None,
    Provider(Arc<dyn DeviceProvider>),
}

pub struct SessionConfig {
    /// Default origin of execute(): Human (GUI, openlaser-hw), Headless (CLI/MCP).
    pub origin: Origin,
    /// A GUI hosts this session (bridge requests possible).
    pub window: bool,
    /// None in openlaser-cli; Sim (+ Usb with `hw`) in the GUI and openlaser-hw.
    pub devices: DeviceAccess,
    /// format::machines::load_all (built-ins shadowed by saved profiles).
    pub machines: Vec<MachineProfile>,
    /// S24: Some only in the GUI and openlaser-hw. `device.settings.set`/`create`/`importEzcad`/`calibrateAxis`
    /// write through it and fail with NoDevice-style "profiles are read-only in this session" when None.
    pub profiles: Option<Arc<dyn ProfileStore>>,
    pub files: FsAuthority,
    pub clock: Arc<dyn Clock>,
}

pub struct Session {
    cfg: SessionConfig,
    id: SessionId,
    docs: Vec<DocState>,
    active: Option<usize>,
    stop_registry: Arc<StopRegistry>,
    requests: Requests,
}

const NI: fn(&'static str) -> EngineError = EngineError::NotImplemented;

impl Session {
    pub fn new(cfg: SessionConfig) -> Self {
        Session { cfg, id: SessionId(1), docs: Vec::new(), active: None, stop_registry: Arc::new(StopRegistry::default()), requests: Requests::default() }
    }

    pub fn id(&self) -> SessionId {
        self.id
    }

    // ---- dispatch (pipeline: SPEC §3.16) --------------------------------------------------------------------
    pub fn execute(&mut self, id: &str, params: serde_json::Value) -> Result<serde_json::Value, EngineError> {
        let origin = self.cfg.origin;
        self.execute_as(origin, id, params)
    }

    pub fn execute_as(&mut self, origin: Origin, id: &str, params: serde_json::Value) -> Result<serde_json::Value, EngineError> {
        let _ = (origin, id, params);
        Err(NI("openlaser_engine::Session::execute_as"))
    }

    /// Non-atomic by default (PhotoCraft); `atomic: Some(label)` = one undo step, rolled back on any error;
    /// device commands are refused inside an atomic batch.
    pub fn batch(&mut self, origin: Origin, steps: Vec<(String, serde_json::Value)>, stop_on_error: bool, atomic: Option<String>) -> BatchResult {
        let _ = (origin, stop_on_error, atomic);
        if steps.is_empty() {
            return BatchResult { completed: 0, failed: 0, rolled_back: false, results: Vec::new() };
        }
        let error = NI("openlaser_engine::Session::batch").to_string();
        BatchResult { completed: 0, failed: 1, rolled_back: false, results: vec![serde_json::json!({ "ok": false, "error": error })] }
    }

    pub fn command_list(&self, origin: Origin, f: &ListFilter) -> Vec<CommandRow> {
        let _ = (origin, f);
        Vec::new()
    }

    pub fn command_schema(&self, id: &str) -> Result<serde_json::Value, EngineError> {
        let _ = id;
        Err(NI("openlaser_engine::Session::command_schema"))
    }

    // ---- documents ------------------------------------------------------------------------------------------
    pub fn docs(&self) -> &[DocState] {
        &self.docs
    }

    pub fn active(&self) -> Option<&DocState> {
        self.active.and_then(|i| self.docs.get(i))
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    /// The one way a document changes: runs `f` on a copy; on Ok swaps it in and records one history state
    /// (or merges into the previous one when `coalesce` matches); on Err nothing changes. Bumps the revision,
    /// voids PendingArm, marks submissions/frame requests stale, invalidates the live check.
    pub fn edit<R>(
        &mut self,
        label: Label,
        coalesce: Option<&str>,
        f: impl FnOnce(&mut Project, &mut Selection) -> Result<R, EngineError>,
    ) -> Result<R, EngineError> {
        let _ = (label, coalesce, f);
        Err(NI("openlaser_engine::Session::edit"))
    }

    pub fn undo(&mut self) -> Result<(), EngineError> {
        Err(NI("openlaser_engine::Session::undo"))
    }

    pub fn redo(&mut self) -> Result<(), EngineError> {
        Err(NI("openlaser_engine::Session::redo"))
    }

    /// doc_inspect JSON (§4.3).
    pub fn inspect(&self, index: Option<usize>, all_objects: bool) -> Result<serde_json::Value, EngineError> {
        let _ = (index, all_objects);
        Err(NI("openlaser_engine::Session::inspect"))
    }

    // ---- machines and jobs ----------------------------------------------------------------------------------
    pub fn machines(&self) -> &[MachineProfile] {
        &self.cfg.machines
    }

    pub fn machine_for(&self, index: Option<usize>) -> Option<&MachineProfile> {
        let _ = index;
        None
    }

    /// plan → validate → encode → disassemble → summary. Pure; never touches a device.
    pub fn build_job(&self, index: Option<usize>, opts: &JobOptions) -> Result<BuiltJob, JobRefused> {
        let _ = (index, opts);
        Err(JobRefused {
            fingerprint: None,
            refusals: vec![Refusal::new(RefusalCode::NotImplemented, "not implemented: openlaser_engine::Session::build_job")],
            notices: Vec::new(),
            error: None,
        })
    }

    /// Cached build_job of the active doc for the canvas, Cuts badges and status bar; recomputed when stale.
    pub fn live_check(&mut self) -> Option<&LiveCheck> {
        None
    }

    // ---- devices: the human path (Access::Human / Stop commands call these) -----------------------------------
    pub fn connect(&mut self, origin: Origin, machine: &MachineId) -> Result<(), EngineError> {
        let _ = (origin, machine, &self.cfg.devices, &self.cfg.files, &self.cfg.profiles, self.cfg.window);
        Err(NI("openlaser_engine::Session::connect"))
    }

    pub fn disconnect(&mut self, origin: Origin, machine: &MachineId) -> Result<(), EngineError> {
        let _ = (origin, machine);
        Err(NI("openlaser_engine::Session::disconnect"))
    }

    /// `style`/`source` None = the project's JobSettings. Neither changes the fingerprint (S21), so no edit is made.
    pub fn start_frame(&mut self, origin: Origin, mode: FrameMode, style: Option<FrameStyle>, source: Option<FrameSource>) -> Result<Fingerprint, EngineError> {
        let _ = (origin, mode, style, source);
        Err(NI("openlaser_engine::Session::start_frame"))
    }

    /// laser.start: cold-builds the job, requires a matching FrameReceipt, stores PendingArm (with the exact encoded
    /// job), raises ArmRequested. Never emits. The ARM dialog shows only `PendingArm.summary`, never a submission's.
    pub fn request_arm(&mut self, origin: Origin) -> Result<&PendingArm, EngineError> {
        let _ = origin;
        Err(NI("openlaser_engine::Session::request_arm"))
    }

    pub fn pending_arm(&self) -> Option<&PendingArm> {
        None
    }

    pub fn cancel_arm(&mut self) {}

    /// Typed Rust API only (no command, no JSON, no control method takes a token). Checks PendingArm is current
    /// (same revision, fingerprint and bytes_hash), token.check, the provider's `emission_cap_pct` (S23),
    /// then moves `PendingArm.job` (the object whose bytes the summary was decoded from) into DriverHandle::run.
    /// It never re-encodes.
    pub fn run_armed(&mut self, token: ArmToken) -> Result<(), EngineError> {
        drop(token);
        Err(NI("openlaser_engine::Session::run_armed"))
    }

    /// Also voids PendingArm (closes the ARM dialog) and marks running submissions stopped. WP0 stub: latches every
    /// registered machine (there is no PendingArm yet).
    pub fn stop_all(&mut self) -> Vec<(MachineId, StopAck)> {
        self.stop_registry.stop_all(Duration::from_millis(500))
    }

    pub fn stop_registry(&self) -> Arc<StopRegistry> {
        Arc::clone(&self.stop_registry)
    }

    pub fn status(&self, machine: &MachineId) -> Option<&CachedStatus> {
        let _ = machine;
        None
    }

    /// Folds driver events into CachedStatus, receipts, requests; expires requests. Call every frame / tool call.
    /// A `Stopped` event or a tripped latch seen here also voids PendingArm (stops that came through the control
    /// fast path or a signal never touched the Session).
    pub fn poll(&mut self) -> Vec<Event> {
        let _ = self.cfg.clock.now();
        Vec::new()
    }

    // ---- agent requests (bridge mode; NeedsBridge when !window) ---------------------------------------------
    pub fn submit_job(&mut self, client: &str, note: &str) -> Result<SubmissionView, EngineError> {
        let _ = (client, note);
        Err(NI("openlaser_engine::Session::submit_job"))
    }

    pub fn request_frame(&mut self, client: &str, mode: FrameMode) -> Result<FrameRequestView, EngineError> {
        let _ = (client, mode);
        Err(NI("openlaser_engine::Session::request_frame"))
    }

    /// Human only.
    pub fn approve_request(&mut self, origin: Origin, id: RequestId) -> Result<(), EngineError> {
        let _ = (origin, id);
        Err(NI("openlaser_engine::Session::approve_request"))
    }

    /// Human only.
    pub fn decline_request(&mut self, origin: Origin, id: RequestId) -> Result<(), EngineError> {
        let _ = (origin, id);
        Err(NI("openlaser_engine::Session::decline_request"))
    }

    pub fn withdraw_request(&mut self, id: RequestId) -> Result<(), EngineError> {
        let _ = id;
        Err(NI("openlaser_engine::Session::withdraw_request"))
    }

    pub fn requests(&self) -> &Requests {
        &self.requests
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchResult {
    pub completed: usize,
    pub failed: usize,
    pub rolled_back: bool,
    pub results: Vec<serde_json::Value>,
}

pub struct DocState {
    pub project: Arc<Project>,
    pub history: History,
    pub selection: Selection,
    pub current_layer: LayerKey,
    pub path: Option<String>,
    pub machine: MachineId,
    pub revision: u64,
    pub saved_revision: u64,
}

/// undo: VecDeque<HistoryState>, redo: Vec<HistoryState>, max_states (default 100).
pub struct History {
    undo: VecDeque<HistoryState>,
    redo: Vec<HistoryState>,
    max_states: usize,
}

impl Default for History {
    fn default() -> Self {
        History { undo: VecDeque::new(), redo: Vec::new(), max_states: 100 }
    }
}

impl History {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn labels(&self) -> Vec<String> {
        let _ = self.max_states;
        self.undo.iter().map(|s| s.label.to_string()).collect()
    }
}

pub struct HistoryState {
    pub label: Label,
    pub project: Arc<Project>,
    pub selection: Selection,
}

/// "Set {name} Frequency".
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub wording: &'static str,
    pub value: Option<String>,
}

impl std::fmt::Display for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.value {
            Some(v) => f.write_str(&self.wording.replace("{name}", v)),
            None => f.write_str(self.wording),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct JobOptions {
    pub machine: Option<MachineId>,
    /// None = project setting.
    pub selected_only: Option<bool>,
}

pub enum Encoded {
    Lmc { job: Arc<LmcJob>, frame: Arc<LmcFrameJob> },
    Gcode { job: Arc<GcodeProgram>, frame: Arc<GcodeFrame> },
}

pub struct BuiltJob {
    pub plan: Arc<JobPlan>,
    pub validated: Validated,
    pub encoded: Encoded,
    pub summary: EmissionSummary,
    pub listing: Listing,
    pub burned: Vec<EmittedSegment>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRefused {
    pub fingerprint: Option<Fingerprint>,
    pub refusals: Vec<Refusal>,
    pub notices: Vec<Notice>,
    pub error: Option<String>,
}

/// Display only: never armed from.
pub struct LiveCheck {
    pub revision: u64,
    pub fingerprint: Option<Fingerprint>,
    pub refusals: Vec<Refusal>,
    pub notices: Vec<Notice>,
    pub estimate: Option<Duration>,
    pub summary: Option<EmissionSummary>,
}

pub struct PendingArm {
    pub fingerprint: Fingerprint,
    pub machine: MachineId,
    pub summary: EmissionSummary,
    pub receipt: FrameReceipt,
    pub checklist: Vec<String>,
    pub burned: Vec<EmittedSegment>,
    pub revision: u64,
    pub expires: MonoTime,
    /// S22: the encoded job `summary` was decoded from; `run_armed` sends exactly this (Arc → owned clone of the
    /// same value, re-checked by bytes_hash in the driver).
    pub job: Encoded,
}

pub struct CachedStatus {
    pub connected: bool,
    pub provider: Option<String>,
    pub state: MachineState,
    pub status_bits: Option<u16>,
    pub fault: Option<DriverError>,
    pub progress: Option<(u32, u32)>,
    pub last_stop: Option<StopReport>,
    pub receipt: Option<FrameReceipt>,
    pub trace_tail: VecDeque<TraceLine>,
}

/// Mutex<Vec<(MachineId, Stopper)>>; a poisoned lock is recovered.
#[derive(Default)]
pub struct StopRegistry {
    stoppers: Mutex<Vec<(MachineId, Stopper)>>,
}

impl StopRegistry {
    /// WP0 addition: the device hub registers each driver's stopper here.
    pub fn register(&self, machine: MachineId, stopper: Stopper) {
        let mut all = self.stoppers.lock().unwrap_or_else(|e| e.into_inner());
        all.retain(|(m, _)| *m != machine);
        all.push((machine, stopper));
    }

    /// Latches every registered machine first, then waits ≤ `max_wait` for each report.
    pub fn stop_all(&self, max_wait: Duration) -> Vec<(MachineId, StopAck)> {
        let all: Vec<(MachineId, Stopper)> = self.stoppers.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for (_, s) in &all {
            s.stop();
        }
        all.into_iter().map(|(m, s)| (m, s.stop_and_wait(max_wait))).collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    DocChanged { index: usize, revision: u64 },
    ArmRequested,
    ArmVoided { reason: String },
    Driver { machine: MachineId, event: DriverEvent },
    Request { id: RequestId, state: String },
    Status(String),
}
