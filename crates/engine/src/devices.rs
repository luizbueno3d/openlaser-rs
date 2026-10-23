//! The device hub and the human emission path (SPEC §3.16; red zone review, §6.8).
//!
//! `connect`, `start_frame`, `request_arm` and `run_armed` are the only ways a document reaches a driver, and each
//! re-checks `Origin::Human` itself (policy is checked in dispatch too: two independent gates). `request_arm` never
//! emits: it cold-builds the job, checks the frame receipt and keeps the encoded object (S22); `run_armed` (typed Rust
//! API only, no command takes a token) sends exactly that object. Every stop voids the pending ARM.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use openlaser_driver::{
    AnyDriver, DeviceProvider, DriverError, DriverEvent, GrblBackend, LmcBackend, MachineState, Outcome, StopAck, StopReport, Stopper, TraceLine, spawn,
};
use openlaser_gcode::Header;
use openlaser_machine::Kinematics;
use openlaser_types::{
    ARM_TOKEN_TTL, ArmToken, EmissionSummary, EmittedSegment, FRAME_RECEIPT_MAX_AGE, Fingerprint, FrameMode, FrameOrigin, FrameReceipt, FrameSource,
    FrameStyle, MachineId, MonoTime, Refusal, RefusalCode, SessionId,
};

use crate::command::{EngineError, Origin};
use crate::jobs::{Encoded, GENERATOR, JobOptions, JobRefused};
use crate::requests::RequestId;
use crate::session::Session;

/// How long `stop_all` waits for each machine's StopReport (§5.4 watchdog).
pub const STOP_WAIT: Duration = Duration::from_millis(500);
/// Trace lines kept per machine for the Console panel.
pub const TRACE_TAIL: usize = 500;
/// A pending ARM lives at most this long (and never past its receipt's age limit).
pub const PENDING_ARM_TTL: Duration = Duration::from_secs(600);
/// Driver events folded per machine per `poll` (the rest wait for the next poll).
const MAX_EVENTS_PER_POLL: usize = 100_000;

pub enum DeviceAccess {
    None,
    Provider(Arc<dyn DeviceProvider>),
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
    /// `StopRegistry::trips()` when Start was pressed. Any stop after it (control fast path, signal watcher, panic hook:
    /// none of them touch the session) changes the count, and `run_armed` then refuses (§3.13: a stop between Confirm
    /// and the driver dequeuing the run cancels the run; §3.16: every stop voids PendingArm).
    pub(crate) stop_trips: u64,
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

impl CachedStatus {
    fn new(provider: &str) -> Self {
        CachedStatus {
            connected: false,
            provider: Some(provider.to_owned()),
            state: MachineState::Connecting,
            status_bits: None,
            fault: None,
            progress: None,
            last_stop: None,
            receipt: None,
            trace_tail: VecDeque::new(),
        }
    }

    /// Framing, running or stopping: no new frame, run or connection change may start (pipeline step 7).
    pub fn busy(&self) -> bool {
        matches!(self.state, MachineState::Framing | MachineState::Running { .. } | MachineState::Stopping)
    }

    /// A fault or an unconfirmed stop that a person has not acknowledged yet (§5.4: faults are sticky until
    /// acknowledged). Frame, Start and run_armed are refused until `laser.clearFault`.
    pub fn faulted(&self) -> bool {
        self.fault.is_some() || matches!(self.state, MachineState::Fault | MachineState::StopNotConfirmed)
    }

    /// Sets the state the driver reports, except that a sticky fault keeps the machine in Fault / STOP NOT CONFIRMED
    /// until it is acknowledged (a later Idle or Stopped from the driver never clears it silently).
    fn settle(&mut self, s: MachineState) {
        if self.faulted() && matches!(s, MachineState::Idle | MachineState::Stopped) {
            return;
        }
        self.state = s;
    }

    /// §5.4 watchdog: no StopReport (or a report that could not confirm) → STOP NOT CONFIRMED + sticky fault.
    fn stop_not_confirmed(&mut self) {
        self.state = MachineState::StopNotConfirmed;
        if self.fault.is_none() {
            self.fault = Some(DriverError::Protocol(STOP_NOT_CONFIRMED.to_owned()));
        }
    }
}

/// The red banner's text (§5.4).
pub const STOP_NOT_CONFIRMED: &str = "STOP NOT CONFIRMED: use the key switch / E-stop, check the machine, then press Clear Fault";

/// Mutex<Vec<(MachineId, Stopper)>>; a poisoned lock is recovered.
#[derive(Default)]
pub struct StopRegistry {
    stoppers: Mutex<Vec<(MachineId, Stopper)>>,
    trips: AtomicU64,
}

impl StopRegistry {
    /// WP0 addition: the device hub registers each driver's stopper here.
    pub fn register(&self, machine: MachineId, stopper: Stopper) {
        let mut all = self.stoppers.lock().unwrap_or_else(|e| e.into_inner());
        all.retain(|(m, _)| *m != machine);
        all.push((machine, stopper));
    }

    /// Forgets a machine's stopper (after disconnect).
    pub fn unregister(&self, machine: &MachineId) {
        let mut all = self.stoppers.lock().unwrap_or_else(|e| e.into_inner());
        all.retain(|(m, _)| m != machine);
    }

    /// Latches every registered machine first, then waits ≤ `max_wait` for each report.
    pub fn stop_all(&self, max_wait: Duration) -> Vec<(MachineId, StopAck)> {
        self.trips.fetch_add(1, Ordering::SeqCst);
        let all: Vec<(MachineId, Stopper)> = self.stoppers.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for (_, s) in &all {
            s.stop();
        }
        all.into_iter().map(|(m, s)| (m, s.stop_and_wait(max_wait))).collect()
    }

    /// Latches one machine and waits ≤ `max_wait` for its report; None if it is not registered.
    pub fn stop_one(&self, machine: &MachineId, max_wait: Duration) -> Option<StopAck> {
        self.trips.fetch_add(1, Ordering::SeqCst);
        let stopper = self.stoppers.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(m, _)| m == machine).map(|(_, s)| s.clone())?;
        Some(stopper.stop_and_wait(max_wait))
    }

    /// How many stops went through this registry (the session compares it in `poll`: a stop that came through the
    /// control fast path or a signal never touched the session, and must still void the pending ARM).
    pub fn trips(&self) -> u64 {
        self.trips.load(Ordering::SeqCst)
    }

    /// Machines with a registered stopper.
    pub fn machines(&self) -> Vec<MachineId> {
        self.stoppers.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(m, _)| m.clone()).collect()
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

/// What the hub last asked a driver to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Op {
    Frame { mode: FrameMode, fingerprint: Fingerprint },
    Run { fingerprint: Fingerprint },
}

/// One connected machine.
pub(crate) struct Slot {
    pub machine: MachineId,
    pub driver: AnyDriver,
    pub status: CachedStatus,
    pub op: Option<Op>,
}

fn stopper_of(d: &AnyDriver) -> Stopper {
    match d {
        AnyDriver::Lmc(h) => h.stopper(),
        AnyDriver::Grbl(h) => h.stopper(),
    }
}

fn drain(d: &AnyDriver) -> Vec<DriverEvent> {
    match d {
        AnyDriver::Lmc(h) => h.events().try_iter().take(MAX_EVENTS_PER_POLL).collect(),
        AnyDriver::Grbl(h) => h.events().try_iter().take(MAX_EVENTS_PER_POLL).collect(),
    }
}

fn require_human(origin: Origin, id: &str) -> Result<(), EngineError> {
    if origin == Origin::Human { Ok(()) } else { Err(EngineError::human_only(id)) }
}

fn not_connected(m: &MachineId) -> EngineError {
    EngineError::refused(RefusalCode::NotConnected, format!("{} is not connected; connect it in the Laser panel first", m.0))
}

fn busy(m: &MachineId) -> EngineError {
    EngineError::refused(RefusalCode::MachineBusy, format!("{} is framing or running; stop it first", m.0))
}

/// The refusal while a machine has an unacknowledged fault or an unconfirmed stop.
fn faulted(m: &MachineId, st: &CachedStatus) -> EngineError {
    let why = match (&st.fault, &st.state) {
        (_, MachineState::StopNotConfirmed) => STOP_NOT_CONFIRMED.to_owned(),
        (Some(e), _) => format!("it reported a fault ({e})"),
        (None, _) => "it reported a fault".to_owned(),
    };
    EngineError::refused(
        RefusalCode::MachineBusy,
        format!("{} cannot start anything: {why}. Check the machine, then press Clear Fault in the Laser panel", m.0),
    )
}

/// The refusal when a stop landed after Start (nothing was sent).
fn stopped_after_start() -> EngineError {
    EngineError::refused(RefusalCode::StoppedAfterQueue, "a stop was pressed after Start, so nothing was sent; press Start again")
}

fn driver_error(machine: &MachineId, e: DriverError) -> EngineError {
    match e {
        DriverError::Refused(r) => EngineError::Refused(r),
        other => EngineError::refused(RefusalCode::NotConnected, format!("cannot open {}: {other}", machine.0)),
    }
}

/// Checks a frame receipt against the job about to be armed (the same rules `mint` and the driver apply; checked
/// here first so Start explains itself before the ARM dialog opens).
pub(crate) fn check_receipt(receipt: Option<&FrameReceipt>, summary: &EmissionSummary, session: SessionId, now: MonoTime) -> Result<(), Refusal> {
    let Some(r) = receipt else {
        return Err(Refusal::new(RefusalCode::FrameRequired, "Frame this job first: Start needs a red-light frame of this exact job from this session"));
    };
    if r.fingerprint != summary.fingerprint {
        return Err(Refusal::new(
            RefusalCode::Stale,
            format!("the last frame ({}) was of a different version of this job ({}); frame it again", r.fingerprint.short(), summary.fingerprint.short()),
        ));
    }
    if r.machine != summary.machine {
        return Err(Refusal::new(RefusalCode::MachineMismatch, format!("the frame was traced on {}, the job is for {}", r.machine.0, summary.machine.0)));
    }
    if r.session != session {
        return Err(Refusal::new(RefusalCode::ArmSessionMismatch, "the frame was traced in another session; frame it again"));
    }
    if r.full_passes < 1 {
        return Err(Refusal::new(RefusalCode::FrameRequired, "the frame has not completed a pass yet; let it finish one pass"));
    }
    if now.saturating_since(r.at) > FRAME_RECEIPT_MAX_AGE {
        return Err(Refusal::new(
            RefusalCode::FrameReceiptExpired,
            format!("the frame is older than {} minutes; frame it again", FRAME_RECEIPT_MAX_AGE.as_secs() / 60),
        ));
    }
    if r.origin != FrameOrigin::Human {
        return Err(Refusal::new(RefusalCode::FrameNotHuman, "only a frame a person started counts toward Start"));
    }
    if !r.covers_job() {
        return Err(Refusal::new(RefusalCode::FrameDoesNotCoverJob, "a frame of the tool layers only does not show where the job burns; frame the job"));
    }
    Ok(())
}

/// S23: every row's power, and every burned segment decoded from the bytes, must stay within the provider's
/// first-emission cap.
pub(crate) fn check_cap(cap: Option<f64>, summary: &EmissionSummary, burned: &[EmittedSegment]) -> Result<(), Refusal> {
    let Some(cap) = cap else { return Ok(()) };
    let over = |p: f64| p.is_nan() || p > cap + 1e-9;
    if let Some(row) = summary.rows.iter().find(|r| over(r.power_pct)) {
        return Err(Refusal::new(
            RefusalCode::BenchPowerCap,
            format!(
                "the first emission of this bench session is capped at {cap} %; {} pass {} uses {:.2} %. Lower it for the first job",
                row.layer, row.pass, row.power_pct
            ),
        )
        .layer(row.layer));
    }
    if let Some(seg) = burned.iter().find(|s| over(s.power_pct)) {
        return Err(Refusal::new(
            RefusalCode::BenchPowerCap,
            format!(
                "the first emission of this bench session is capped at {cap} %; the job's bytes burn at {:.2} %. Lower it for the first job",
                seg.power_pct
            ),
        )
        .at(seg.a));
    }
    Ok(())
}

impl Session {
    pub(crate) fn provider(&self) -> Result<Arc<dyn DeviceProvider>, EngineError> {
        match &self.cfg.devices {
            DeviceAccess::None => Err(EngineError::NoDevice),
            DeviceAccess::Provider(p) => Ok(Arc::clone(p)),
        }
    }

    pub(crate) fn slot(&self, m: &MachineId) -> Option<&Slot> {
        self.hub.iter().find(|s| &s.machine == m)
    }

    fn slot_mut(&mut self, m: &MachineId) -> Option<&mut Slot> {
        self.hub.iter_mut().find(|s| &s.machine == m)
    }

    /// The first machine that is framing, running or stopping.
    pub(crate) fn busy_machine(&self) -> Option<MachineId> {
        self.hub.iter().find(|s| s.status.busy()).map(|s| s.machine.clone())
    }

    /// The active document's machine (else the session's first machine).
    pub(crate) fn current_machine(&self) -> Option<MachineId> {
        self.active().map(|d| d.machine.clone()).or_else(|| self.default_machine())
    }

    // ---- devices: the human path (Access::Human / Stop commands call these) -----------------------------------
    pub fn connect(&mut self, origin: Origin, machine: &MachineId) -> Result<(), EngineError> {
        require_human(origin, "device.connect")?;
        let provider = self.provider()?;
        let profile =
            self.profile(machine).cloned().ok_or_else(|| EngineError::refused(RefusalCode::NoDevice, format!("no machine profile `{}`", machine.0)))?;
        if self.slot(machine).is_some() {
            return Ok(());
        }
        if let Some(m) = self.busy_machine() {
            return Err(busy(&m));
        }
        let driver = match &profile.kind {
            Kinematics::Galvo(g) => {
                let transport = provider.open_lmc(g).map_err(|e| driver_error(machine, e))?;
                AnyDriver::Lmc(spawn(LmcBackend::new(transport, g.clone()), machine.clone(), self.id, Arc::clone(&self.cfg.clock)))
            }
            Kinematics::Gantry(g) => {
                let link = provider.open_link(g).map_err(|e| driver_error(machine, e))?;
                AnyDriver::Grbl(spawn(GrblBackend::new(link, g.clone()), machine.clone(), self.id, Arc::clone(&self.cfg.clock)))
            }
        };
        match &driver {
            AnyDriver::Lmc(h) => h.connect(),
            AnyDriver::Grbl(h) => h.connect(),
        }
        self.stop_registry.register(machine.clone(), stopper_of(&driver));
        self.hub.push(Slot { machine: machine.clone(), driver, status: CachedStatus::new(provider.name()), op: None });
        self.push_event(Event::Status(format!("connecting {}", machine.0)));
        Ok(())
    }

    pub fn disconnect(&mut self, origin: Origin, machine: &MachineId) -> Result<(), EngineError> {
        require_human(origin, "device.disconnect")?;
        self.provider()?;
        let Some(pos) = self.hub.iter().position(|s| &s.machine == machine) else { return Err(not_connected(machine)) };
        if self.hub.get(pos).is_some_and(|s| s.status.busy()) {
            return Err(busy(machine));
        }
        if self.pending_arm.as_ref().is_some_and(|p| &p.machine == machine) {
            self.void_pending_arm("the machine was disconnected");
        }
        self.stop_registry.unregister(machine);
        let slot = self.hub.remove(pos);
        match &slot.driver {
            AnyDriver::Lmc(h) => h.disconnect(),
            AnyDriver::Grbl(h) => h.disconnect(),
        }
        // Dropping the last handle is the dead-man rule: the driver trips its latch and is joined (≤ 500 ms).
        drop(slot);
        self.push_event(Event::Status(format!("disconnected {}", machine.0)));
        Ok(())
    }

    /// `style`/`source` None = the project's JobSettings. Neither changes the fingerprint (S21), so no edit is made.
    pub fn start_frame(&mut self, origin: Origin, mode: FrameMode, style: Option<FrameStyle>, source: Option<FrameSource>) -> Result<Fingerprint, EngineError> {
        self.start_frame_of(origin, mode, style, source, None)
    }

    /// `start_frame`, refused as stale when `expect` is given and the current plan's fingerprint differs from it (an
    /// approved request frames exactly the job the agent asked about, never a later version of it).
    pub(crate) fn start_frame_of(
        &mut self,
        origin: Origin,
        mode: FrameMode,
        style: Option<FrameStyle>,
        source: Option<FrameSource>,
        expect: Option<Fingerprint>,
    ) -> Result<Fingerprint, EngineError> {
        require_human(origin, "laser.frame")?;
        self.provider()?;
        let idx = self.doc_index()?;
        let (project, selection, machine) = {
            let d = self.doc()?;
            (Arc::clone(&d.project), d.selection.clone(), d.machine.clone())
        };
        let profile =
            self.profile(&machine).cloned().ok_or_else(|| EngineError::refused(RefusalCode::NoDevice, format!("no machine profile `{}`", machine.0)))?;
        let Some(slot) = self.slot(&machine) else { return Err(not_connected(&machine)) };
        if slot.status.busy() {
            return Err(busy(&machine));
        }
        if slot.status.faulted() {
            return Err(faulted(&machine, &slot.status));
        }
        let style = style.unwrap_or(project.job.frame_style);
        let source = source.unwrap_or(project.job.frame_source);
        let popts = self.plan_options(&project, &selection, project.job.cut_selected, Some(style), Some(source));
        let plan = openlaser_plan::plan(&project, &profile, &popts).map_err(|e| EngineError::JobRefused(Box::new(JobRefused::error(format!("plan: {e}")))))?;
        if let Some(want) = expect
            && want != plan.fingerprint
        {
            return Err(EngineError::refused(
                RefusalCode::Stale,
                format!("the job changed after the request ({} is now {}); ask again", want.short(), plan.fingerprint.short()),
            ));
        }
        let frame = openlaser_safety::validate_frame(&plan, &profile, source).map_err(|r| {
            EngineError::JobRefused(Box::new(JobRefused { fingerprint: Some(r.fingerprint), refusals: r.refusals, notices: r.notices, error: None }))
        })?;
        let fingerprint = plan.fingerprint;
        let Some(slot) = self.hub.iter_mut().find(|s| s.machine == machine) else { return Err(not_connected(&machine)) };
        match (&profile.kind, &slot.driver) {
            (Kinematics::Galvo(g), AnyDriver::Lmc(h)) => {
                let job = openlaser_lmc::encode_frame(&frame, g)
                    .map_err(|e| EngineError::refused(RefusalCode::NotEncodable, format!("the frame cannot be encoded: {e}")))?;
                h.frame(job, mode, FrameOrigin::Human);
            }
            (Kinematics::Gantry(g), AnyDriver::Grbl(h)) => {
                let header = Header { machine_name: profile.name.clone(), generated: GENERATOR.to_owned() };
                let job = openlaser_gcode::frame_program(&frame, g, &header)
                    .map_err(|e| EngineError::refused(RefusalCode::NotEncodable, format!("the frame cannot be encoded: {e}")))?;
                h.frame(job, mode, FrameOrigin::Human);
            }
            _ => return Err(EngineError::refused(RefusalCode::WrongDeviceKind, format!("{} is connected with another kind of driver", machine.0))),
        }
        slot.status.state = MachineState::Framing;
        slot.op = Some(Op::Frame { mode, fingerprint });
        let moved = self.requests.human_framed(idx, &machine, fingerprint);
        for (id, state) in moved {
            self.push_event(Event::Request { id, state });
        }
        Ok(fingerprint)
    }

    /// laser.start: cold-builds the job, requires a matching FrameReceipt, stores PendingArm (with the exact encoded
    /// job), raises ArmRequested. Never emits. The ARM dialog shows only `PendingArm.summary`, never a submission's.
    pub fn request_arm(&mut self, origin: Origin) -> Result<&PendingArm, EngineError> {
        self.request_arm_of(origin, None)
    }

    /// `request_arm`, refused as stale when `expect` is given and the job built now has another fingerprint (an
    /// approved submission arms exactly the job the agent submitted).
    pub(crate) fn request_arm_of(&mut self, origin: Origin, expect: Option<Fingerprint>) -> Result<&PendingArm, EngineError> {
        require_human(origin, "laser.start")?;
        let provider = self.provider()?;
        let idx = self.doc_index()?;
        self.void_pending_arm("Start was pressed again");
        // A stop the session has not folded yet happened before this Start: fold it now, then remember the count, so
        // that any later stop cancels this ARM (run_armed compares it).
        self.fold_stop_trips();
        let stop_trips = self.stop_registry.trips();
        let (machine, revision) = {
            let d = self.doc()?;
            (d.machine.clone(), d.revision)
        };
        let profile =
            self.profile(&machine).cloned().ok_or_else(|| EngineError::refused(RefusalCode::NoDevice, format!("no machine profile `{}`", machine.0)))?;
        let Some(slot) = self.slot(&machine) else { return Err(not_connected(&machine)) };
        if slot.status.busy() {
            return Err(busy(&machine));
        }
        if slot.status.faulted() {
            return Err(faulted(&machine, &slot.status));
        }
        let receipt = slot.status.receipt.clone();
        if receipt.is_none() {
            // Explained before the build; check_receipt below applies every rule to the built job.
            return Err(EngineError::Refused(Refusal::new(
                RefusalCode::FrameRequired,
                "Frame this job first: Start needs a red-light frame of this exact job from this session",
            )));
        }
        let built =
            self.build_job(Some(idx), &JobOptions { machine: Some(machine.clone()), selected_only: None }).map_err(|r| EngineError::JobRefused(Box::new(r)))?;
        if let Some(want) = expect
            && want != built.summary.fingerprint
        {
            return Err(EngineError::refused(
                RefusalCode::Stale,
                format!("the job changed after it was submitted ({} is now {}); submit it again", want.short(), built.summary.fingerprint.short()),
            ));
        }
        let now = self.cfg.clock.now();
        check_receipt(receipt.as_ref(), &built.summary, self.id, now).map_err(EngineError::Refused)?;
        let cap =
            provider.emission_cap_pct().map_err(|e| EngineError::refused(RefusalCode::BenchPowerCap, format!("the first-emission cap cannot be read: {e}")))?;
        check_cap(cap, &built.summary, &built.burned).map_err(EngineError::Refused)?;
        if profile.checklist.is_empty() {
            return Err(EngineError::refused(RefusalCode::ChecklistIncomplete, "the machine profile has no job checklist; add one in Device Settings"));
        }
        let Some(receipt) = receipt else { return Err(EngineError::refused(RefusalCode::FrameRequired, "Frame this job first")) };
        let expires = receipt.at.plus(FRAME_RECEIPT_MAX_AGE).min(now.plus(PENDING_ARM_TTL));
        self.pending_arm = Some(PendingArm {
            fingerprint: built.summary.fingerprint,
            machine,
            summary: built.summary,
            receipt,
            checklist: profile.checklist.clone(),
            burned: built.burned,
            revision,
            expires,
            job: built.encoded,
            stop_trips,
        });
        self.pending_doc = Some(idx);
        self.push_event(Event::ArmRequested);
        self.pending_arm.as_ref().ok_or_else(|| EngineError::Internal("laser.start".to_owned()))
    }

    pub fn pending_arm(&self) -> Option<&PendingArm> {
        self.pending_arm.as_ref()
    }

    pub fn cancel_arm(&mut self) {
        self.void_pending_arm("cancelled");
    }

    pub(crate) fn void_pending_arm(&mut self, reason: &str) {
        if self.pending_arm.take().is_some() {
            self.pending_doc = None;
            self.push_event(Event::ArmVoided { reason: reason.to_owned() });
        }
    }

    /// Typed Rust API only (no command, no JSON, no control method takes a token). Checks PendingArm is current
    /// (same revision, fingerprint and bytes_hash), token.check, the provider's `emission_cap_pct` (S23),
    /// then moves `PendingArm.job` (the object whose bytes the summary was decoded from) into DriverHandle::run.
    /// It never re-encodes.
    pub fn run_armed(&mut self, token: ArmToken) -> Result<(), EngineError> {
        let Some(pending) = self.pending_arm.take() else {
            drop(token);
            return Err(EngineError::refused(RefusalCode::Stale, "nothing is waiting to be armed; press Start again"));
        };
        let pending_doc = self.pending_doc.take();
        self.push_event(Event::ArmVoided { reason: "armed".to_owned() });
        // A stop since Start that the session has not seen (fast path, signal watcher, panic hook) cancels the run: the
        // driver's generation check only cancels commands queued BEFORE a trip, and this one is not queued yet.
        if self.stop_registry.trips() != pending.stop_trips {
            return Err(stopped_after_start());
        }
        let now = self.cfg.clock.now();
        let current =
            pending_doc.is_some() && pending_doc == self.active && pending_doc.and_then(|i| self.docs.get(i)).is_some_and(|d| d.revision == pending.revision);
        if !current {
            return Err(EngineError::refused(RefusalCode::Stale, "the document changed after Start; press Start again"));
        }
        if now > pending.expires {
            return Err(EngineError::refused(RefusalCode::ArmExpired, "the ARM request expired; press Start again"));
        }
        // The cheap, non-secret checks first: the encoded object is the one the summary describes (S22), the machine
        // can take it, and the first-emission cap (S23) holds for the rows and for the decoded burn.
        if pending.job.bytes_hash() != pending.summary.bytes_hash || pending.job.fingerprint() != pending.fingerprint {
            return Err(EngineError::refused(RefusalCode::BytesMismatch, "the encoded job is not the one whose summary was confirmed"));
        }
        let provider = self.provider()?;
        let machine = pending.machine.clone();
        let Some(slot) = self.slot(&machine) else { return Err(not_connected(&machine)) };
        if slot.status.busy() {
            return Err(busy(&machine));
        }
        if slot.status.faulted() {
            return Err(faulted(&machine, &slot.status));
        }
        let cap =
            provider.emission_cap_pct().map_err(|e| EngineError::refused(RefusalCode::BenchPowerCap, format!("the first-emission cap cannot be read: {e}")))?;
        check_cap(cap, &pending.summary, &pending.burned).map_err(EngineError::Refused)?;
        token.check(&pending.fingerprint, &pending.summary.bytes_hash, &pending.machine, self.id, now).map_err(EngineError::Refused)?;
        if token.fingerprint() != &pending.fingerprint || token.bytes() != &pending.summary.bytes_hash || token.machine() != &pending.machine {
            return Err(EngineError::refused(RefusalCode::BytesMismatch, "the token is for another job"));
        }
        let total = pending.summary.packets;
        let fingerprint = pending.fingerprint;
        let stop_trips = pending.stop_trips;
        let registry = Arc::clone(&self.stop_registry);
        let Some(slot) = self.slot_mut(&machine) else { return Err(not_connected(&machine)) };
        if registry.trips() != stop_trips {
            return Err(stopped_after_start());
        }
        match (pending.job, &slot.driver) {
            (Encoded::Lmc { job, .. }, AnyDriver::Lmc(h)) => {
                let job = Arc::try_unwrap(job).unwrap_or_else(|shared| (*shared).clone());
                h.run(job, token);
            }
            (Encoded::Gcode { job, .. }, AnyDriver::Grbl(h)) => {
                let job = Arc::try_unwrap(job).unwrap_or_else(|shared| (*shared).clone());
                h.run(job, token);
            }
            _ => return Err(EngineError::refused(RefusalCode::WrongDeviceKind, format!("{} is connected with another kind of driver", machine.0))),
        }
        // A stop that landed between the check above and the queueing would not cancel the queued run (it recorded the
        // newer generation): trip this machine's latch again, so the driver refuses it as queued-then-stopped (§5.3).
        if registry.trips() != stop_trips {
            stopper_of(&slot.driver).stop();
        }
        slot.status.state = MachineState::Running { packets_acked: 0, total };
        slot.status.progress = Some((0, total));
        slot.op = Some(Op::Run { fingerprint });
        let moved = self.requests.armed(&machine, fingerprint);
        for (id, state) in moved {
            self.push_event(Event::Request { id, state });
        }
        Ok(())
    }

    /// Also voids PendingArm (closes the ARM dialog) and marks running submissions stopped.
    pub fn stop_all(&mut self) -> Vec<(MachineId, StopAck)> {
        let acks = self.stop_registry.stop_all(STOP_WAIT);
        self.seen_stop_trips = self.stop_registry.trips();
        self.after_stop(&acks, None);
        acks
    }

    /// Stops one machine (`laser.stop {machine}`), with the same side effects as `stop_all` for it.
    pub fn stop_machine(&mut self, machine: &MachineId) -> Option<StopAck> {
        let ack = self.stop_registry.stop_one(machine, STOP_WAIT);
        self.seen_stop_trips = self.stop_registry.trips();
        let acks: Vec<(MachineId, StopAck)> = ack.iter().map(|a| (machine.clone(), a.clone())).collect();
        self.after_stop(&acks, Some(machine));
        ack
    }

    fn after_stop(&mut self, acks: &[(MachineId, StopAck)], only: Option<&MachineId>) {
        self.void_pending_arm("stopped");
        for (m, ack) in acks {
            if let Some(slot) = self.slot_mut(m) {
                // Something was framing or running: its stop must be confirmed (§5.4 watchdog).
                let needed = slot.op.is_some() || slot.status.busy();
                match &ack.report {
                    Some(report) => {
                        slot.status.last_stop = Some(report.clone());
                        slot.status.progress = None;
                        slot.op = None;
                        if report.confirmed {
                            slot.status.settle(MachineState::Stopped);
                        } else {
                            slot.status.stop_not_confirmed();
                        }
                    }
                    None if needed => slot.status.stop_not_confirmed(),
                    None => {}
                }
            }
            let moved = self.requests.stopped(m, ack.report.clone());
            for (id, state) in moved {
                self.push_event(Event::Request { id, state });
            }
        }
        if only.is_none() {
            // A stop with nothing registered still ends every framing request (nothing can be framing then).
            let moved = self.requests.stopped_all();
            for (id, state) in moved {
                self.push_event(Event::Request { id, state });
            }
        }
        self.push_event(Event::Status("stop".to_owned()));
    }

    pub fn stop_registry(&self) -> Arc<StopRegistry> {
        Arc::clone(&self.stop_registry)
    }

    pub fn status(&self, machine: &MachineId) -> Option<&CachedStatus> {
        self.slot(machine).map(|s| &s.status)
    }

    /// Folds driver events into CachedStatus, receipts, requests; expires requests. Call every frame / tool call.
    /// A `Stopped` event or a tripped latch seen here also voids PendingArm (stops that came through the control
    /// fast path or a signal never touched the Session).
    pub fn poll(&mut self) -> Vec<Event> {
        self.fold_stop_trips();
        let batches: Vec<(MachineId, Vec<DriverEvent>)> = self.hub.iter().map(|s| (s.machine.clone(), drain(&s.driver))).collect();
        for (machine, events) in batches {
            for ev in events {
                self.fold_driver_event(&machine, ev);
            }
        }
        let now = self.cfg.clock.now();
        if self.pending_arm.as_ref().is_some_and(|p| now > p.expires) {
            self.void_pending_arm("the ARM request expired");
        }
        let receipts: Vec<(MachineId, Option<MonoTime>)> = self.hub.iter().map(|s| (s.machine.clone(), s.status.receipt.as_ref().map(|r| r.at))).collect();
        let moved = self.requests.tick(now, &receipts);
        for (id, state) in moved {
            self.push_event(Event::Request { id, state });
        }
        self.events.drain(..).collect()
    }

    /// A stop that went through the registry without touching the session (control fast path, signal watcher, panic
    /// hook) voids the pending ARM and ends every framing request.
    pub(crate) fn fold_stop_trips(&mut self) {
        let trips = self.stop_registry.trips();
        if trips != self.seen_stop_trips {
            self.seen_stop_trips = trips;
            self.void_pending_arm("stopped");
            let moved = self.requests.stopped_all();
            for (id, state) in moved {
                self.push_event(Event::Request { id, state });
            }
        }
    }

    /// One driver event → cached status, receipts, requests and the pending ARM.
    pub(crate) fn fold_driver_event(&mut self, machine: &MachineId, ev: DriverEvent) {
        let provider = match &self.cfg.devices {
            DeviceAccess::Provider(p) => Some(Arc::clone(p)),
            DeviceAccess::None => None,
        };
        let mut request_moves = Vec::new();
        let mut void = None;
        if let Some(slot) = self.hub.iter_mut().find(|s| &s.machine == machine) {
            match &ev {
                DriverEvent::Connected(info) => {
                    slot.status.connected = true;
                    slot.status.provider = Some(info.provider.clone());
                    if matches!(slot.status.state, MachineState::Connecting | MachineState::Disconnected) {
                        slot.status.state = MachineState::Idle;
                    }
                }
                DriverEvent::State(s) => {
                    slot.status.settle(s.clone());
                    match s {
                        MachineState::StopNotConfirmed => slot.status.stop_not_confirmed(),
                        MachineState::Fault if slot.status.fault.is_none() => {
                            slot.status.fault = Some(DriverError::Protocol("the driver reported a fault".to_owned()));
                        }
                        _ => {}
                    }
                    if matches!(
                        s,
                        MachineState::Idle | MachineState::Stopped | MachineState::Fault | MachineState::Disconnected | MachineState::StopNotConfirmed
                    ) {
                        slot.op = None;
                        slot.status.progress = None;
                    }
                    if matches!(s, MachineState::Disconnected) {
                        slot.status.connected = false;
                    }
                }
                DriverEvent::FrameCompleted(r) => {
                    slot.status.receipt = Some(r.clone());
                    if let Some(Op::Frame { mode: FrameMode::Once, .. }) = slot.op {
                        slot.op = None;
                        slot.status.settle(MachineState::Idle);
                    }
                    request_moves.extend(self.requests.frame_completed(machine, r));
                }
                DriverEvent::Progress { packets_acked, total, .. } => {
                    slot.status.progress = Some((*packets_acked, *total));
                    if matches!(slot.op, Some(Op::Run { .. })) {
                        slot.status.state = MachineState::Running { packets_acked: *packets_acked, total: *total };
                    }
                }
                DriverEvent::Finished(outcome) => {
                    slot.op = None;
                    slot.status.progress = None;
                    match outcome {
                        Outcome::Finished { .. } => {
                            slot.status.settle(MachineState::Idle);
                            if let Some(p) = &provider
                                && let Err(e) = p.record_emission()
                            {
                                log::error!("record_emission failed for {}: {e}", machine.0);
                            }
                        }
                        Outcome::Stopped(report) => {
                            slot.status.last_stop = Some(report.clone());
                            if report.confirmed {
                                slot.status.settle(MachineState::Stopped);
                            } else {
                                slot.status.stop_not_confirmed();
                            }
                            void = Some("stopped");
                        }
                    }
                    request_moves.extend(self.requests.finished(machine, outcome));
                }
                DriverEvent::Stopped(report) => {
                    slot.op = None;
                    slot.status.progress = None;
                    if report.confirmed {
                        slot.status.settle(MachineState::Stopped);
                    } else {
                        slot.status.stop_not_confirmed();
                    }
                    slot.status.last_stop = Some(report.clone());
                    void = Some("stopped");
                    request_moves.extend(self.requests.stopped(machine, Some(report.clone())));
                }
                DriverEvent::Fault(e) => {
                    slot.op = None;
                    slot.status.progress = None;
                    slot.status.state = MachineState::Fault;
                    slot.status.fault = Some(e.clone());
                    void = Some("the machine reported a fault");
                    request_moves.extend(self.requests.faulted(machine));
                }
                DriverEvent::Trace(line) => {
                    if slot.status.trace_tail.len() >= TRACE_TAIL {
                        slot.status.trace_tail.pop_front();
                    }
                    slot.status.trace_tail.push_back(line.clone());
                }
            }
        }
        if let Some(reason) = void {
            self.void_pending_arm(reason);
        }
        for (id, state) in request_moves {
            self.push_event(Event::Request { id, state });
        }
        if !matches!(ev, DriverEvent::Trace(_)) {
            self.push_event(Event::Driver { machine: machine.clone(), event: ev });
        }
    }

    /// Acknowledges a sticky fault or an unconfirmed stop (the Laser panel's "Clear Fault" button; human-only through
    /// `laser.clearFault`). The only acknowledgement there is: until it, Frame, Start and run_armed are refused.
    pub fn acknowledge_fault(&mut self, origin: Origin, machine: &MachineId) -> Result<(), EngineError> {
        require_human(origin, "laser.clearFault")?;
        let Some(slot) = self.slot_mut(machine) else { return Err(not_connected(machine)) };
        slot.status.fault = None;
        if matches!(slot.status.state, MachineState::Fault | MachineState::StopNotConfirmed) {
            slot.status.state = MachineState::Idle;
        }
        Ok(())
    }

    /// For tests and the ARM dialog: when a token minted now would expire.
    pub fn token_expiry(&self) -> MonoTime {
        self.cfg.clock.now().plus(ARM_TOKEN_TTL)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use openlaser_types::{Bounds, PassRow, SectionMode};

    use super::*;

    pub(crate) fn summary(fp: Fingerprint, machine: &str, powers: &[f64]) -> EmissionSummary {
        EmissionSummary {
            fingerprint: fp,
            bytes_hash: Fingerprint([0; 32]),
            machine: MachineId(machine.into()),
            rows: powers
                .iter()
                .enumerate()
                .map(|(i, p)| PassRow {
                    layer: openlaser_types::LayerKey::Color(u8::try_from(i).unwrap()),
                    layer_name: format!("C{i:02}"),
                    sub: 0,
                    pass: 1,
                    of: 1,
                    mode: SectionMode::Line,
                    power_pct: *p,
                    frequency_khz: Some(30.0),
                    pulse_ns: Some(200),
                    speed_mm_s: 1000.0,
                    galvo: None,
                    paths: 1,
                    mark_mm: 10.0,
                    bounds: Bounds { min: [0.0, 0.0], max: [1.0, 1.0] },
                    estimate: Duration::from_millis(10),
                })
                .collect(),
            notices: Vec::new(),
            repeats: 1,
            estimate: Duration::from_millis(10),
            crash_bound: Duration::from_millis(10),
            packets: 1,
            work_area: Bounds { min: [-100.0, -100.0], max: [100.0, 100.0] },
            negate: [false, false],
            swap_xy: false,
        }
    }

    pub(crate) fn receipt(fp: Fingerprint, machine: &str, session: SessionId, at: MonoTime, source: FrameSource) -> FrameReceipt {
        FrameReceipt {
            fingerprint: fp,
            machine: MachineId(machine.into()),
            session,
            full_passes: 1,
            at,
            style: FrameStyle::Bounds,
            source,
            origin: FrameOrigin::Human,
        }
    }

    #[test]
    fn receipt_rules_in_order() {
        let fp = Fingerprint([7; 32]);
        let s = summary(fp, "m", &[30.0]);
        let sid = SessionId(9);
        let now = MonoTime(Duration::from_secs(1000));
        let ok = receipt(fp, "m", sid, MonoTime(Duration::from_secs(990)), FrameSource::Job);
        assert!(check_receipt(Some(&ok), &s, sid, now).is_ok());
        let code = |r: Option<&FrameReceipt>| check_receipt(r, &s, sid, now).unwrap_err().code;
        assert_eq!(code(None), RefusalCode::FrameRequired);
        assert_eq!(code(Some(&FrameReceipt { fingerprint: Fingerprint([8; 32]), ..ok.clone() })), RefusalCode::Stale);
        assert_eq!(code(Some(&FrameReceipt { machine: MachineId("x".into()), ..ok.clone() })), RefusalCode::MachineMismatch);
        assert_eq!(code(Some(&FrameReceipt { session: SessionId(1), ..ok.clone() })), RefusalCode::ArmSessionMismatch);
        assert_eq!(code(Some(&FrameReceipt { full_passes: 0, ..ok.clone() })), RefusalCode::FrameRequired);
        assert_eq!(code(Some(&FrameReceipt { at: MonoTime(Duration::from_secs(1)), ..ok.clone() })), RefusalCode::FrameReceiptExpired);
        assert_eq!(code(Some(&FrameReceipt { origin: FrameOrigin::Agent, ..ok.clone() })), RefusalCode::FrameNotHuman);
        assert_eq!(code(Some(&FrameReceipt { source: FrameSource::ToolLayersOnly, ..ok.clone() })), RefusalCode::FrameDoesNotCoverJob);
        // S21: the style traced never matters, the source does.
        for style in [FrameStyle::Bounds, FrameStyle::Contour] {
            assert!(check_receipt(Some(&FrameReceipt { style, source: FrameSource::JobAndToolLayers, ..ok.clone() }), &s, sid, now).is_ok());
        }
    }

    #[test]
    fn tool_layer_receipt_does_not_arm() {
        let fp = Fingerprint([3; 32]);
        let s = summary(fp, "m", &[10.0]);
        let r = receipt(fp, "m", SessionId(1), MonoTime::default(), FrameSource::ToolLayersOnly);
        assert_eq!(check_receipt(Some(&r), &s, SessionId(1), MonoTime::default()).unwrap_err().code, RefusalCode::FrameDoesNotCoverJob);
    }

    #[test]
    fn bench_cap_refuses_rows_above_it() {
        let s = summary(Fingerprint([1; 32]), "m", &[5.0, 30.0]);
        assert!(check_cap(None, &s, &[]).is_ok());
        assert!(check_cap(Some(30.0), &s, &[]).is_ok());
        let r = check_cap(Some(10.0), &s, &[]).unwrap_err();
        assert_eq!(r.code, RefusalCode::BenchPowerCap);
        assert_eq!(r.layer, Some(openlaser_types::LayerKey::Color(1)));
        let nan = summary(Fingerprint([1; 32]), "m", &[f64::NAN]);
        assert!(check_cap(Some(10.0), &nan, &[]).is_err(), "NaN power never passes a cap");
        // The burn decoded from the bytes is capped too, whatever the rows say.
        let low = summary(Fingerprint([1; 32]), "m", &[5.0]);
        let seg = |p: f64| EmittedSegment { a: [1.0, 2.0], b: [3.0, 4.0], power_pct: p };
        assert!(check_cap(Some(10.0), &low, &[seg(10.0)]).is_ok());
        let r = check_cap(Some(10.0), &low, &[seg(5.0), seg(40.0)]).unwrap_err();
        assert_eq!(r.code, RefusalCode::BenchPowerCap);
        assert!(r.message.contains("40.00"), "{}", r.message);
        assert!(check_cap(Some(10.0), &low, &[seg(f64::NAN)]).is_err());
    }

    #[test]
    fn sticky_status_rules() {
        let mut st = CachedStatus::new("test");
        st.state = MachineState::Idle;
        assert!(!st.faulted());
        st.stop_not_confirmed();
        assert!(st.faulted() && st.fault.is_some());
        assert!(matches!(st.state, MachineState::StopNotConfirmed));
        // A later Idle or Stopped from the driver never clears it.
        st.settle(MachineState::Idle);
        st.settle(MachineState::Stopped);
        assert!(matches!(st.state, MachineState::StopNotConfirmed));
        // Other states are shown (a machine still running after an unconfirmed stop is still running), still faulted.
        st.settle(MachineState::Running { packets_acked: 1, total: 2 });
        assert!(st.faulted() && st.busy());
        st.fault = None;
        st.state = MachineState::Fault;
        assert!(st.faulted(), "the Fault state alone is sticky too");
    }

    #[test]
    fn stop_registry_counts_trips_and_survives_empty() {
        let r = StopRegistry::default();
        assert_eq!(r.trips(), 0);
        assert!(r.stop_all(Duration::from_millis(1)).is_empty());
        assert_eq!(r.trips(), 1);
        assert!(r.stop_one(&MachineId("none".into()), Duration::from_millis(1)).is_none());
        assert_eq!(r.trips(), 2);
        assert!(r.machines().is_empty());
    }
}
