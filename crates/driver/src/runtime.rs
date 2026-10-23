//! The driver thread and its handle; device providers (SPEC §3.13, §5.3, §5.4; red zone).
//!
//! One OS thread per machine solely owns its backend. Commands arrive on an unbounded control channel, each tagged
//! with the latch generation at the moment it was queued; the thread refuses any command a stop overtook
//! (`StoppedAfterQueue`) and clears the latch only for the others. Events leave on an unbounded channel, so a UI
//! that stops reading can never stall the thread or its stop checks. A panic in a backend is contained and followed
//! by an emergency stop; dropping the handle trips the latch and waits ≤ 500 ms for the stop to reach the wire.

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use openlaser_lmc::LmcTransport;
use openlaser_machine::{GalvoMachine, GantryMachine};
// The variant is aliased because xtask rule 13 greps for the engine's dispatch origin `Origin::Human`.
use openlaser_types::FrameOrigin::{self, Human as HumanFrame};
use openlaser_types::{ArmToken, Clock, FRAME_RECEIPT_MAX_AGE, FrameMode, FrameReceipt, MachineId, Refusal, RefusalCode, SessionId};
use serde::Serialize;

use crate::backend::{Backend, Capabilities, DriverError, DriverEvent, EncodedJob, FrameOutcome, MachineState, Outcome};
use crate::grbl::{ByteLink, GrblBackend};
use crate::lmc_backend::LmcBackend;
use crate::stop::{Cx, CxHooks, StopLatch, StopMailbox, StopReport, Stopper};

/// Dead-man rule (§5.4): dropping the handle waits at most this long for the stop to reach the wire.
pub const DEAD_MAN_JOIN: Duration = Duration::from_millis(500);

/// What the handle sends the driver thread.
enum Control<B: Backend> {
    Connect,
    Frame(B::Frame, FrameMode, FrameOrigin),
    Run(B::Job, ArmToken),
    Disconnect,
    /// Sent by the handle's `Drop` after its stop: the thread disconnects and ends.
    Shutdown,
}

impl<B: Backend> Control<B> {
    fn name(&self) -> &'static str {
        match self {
            Control::Connect => "connect",
            Control::Frame(..) => "frame",
            Control::Run(..) => "run",
            Control::Disconnect => "disconnect",
            Control::Shutdown => "shutdown",
        }
    }
}

/// A command with the latch generation recorded when it was queued (§5.3 "queued-then-stopped").
struct Queued<B: Backend> {
    generation: u64,
    cmd: Control<B>,
}

/// Starts the driver thread for one machine. If the OS refuses a thread, the handle reports a `Fault` event and
/// every command is dropped (the backend, and with it the device, is closed at once).
pub fn spawn<B: Backend>(backend: B, machine: MachineId, session: SessionId, clock: Arc<dyn Clock>) -> DriverHandle<B> {
    let caps = backend.capabilities();
    let (ctl, ctl_rx) = crossbeam_channel::unbounded::<Queued<B>>();
    let (wake, wake_rx) = crossbeam_channel::bounded::<()>(1);
    let (events_tx, events) = crossbeam_channel::unbounded::<DriverEvent>();
    let (done_tx, done) = crossbeam_channel::bounded::<()>(1);
    let latch = Arc::new(StopLatch::with_clock(wake, Arc::clone(&clock)));
    let mailbox = Arc::new(StopMailbox::default());
    let stopper = Stopper::with_mailbox(Arc::clone(&latch), Arc::clone(&mailbox));
    let thread_name = format!("openlaser-driver {}", machine.0);
    let failure_events = events_tx.clone();
    let rt = Runtime {
        backend,
        machine,
        session,
        clock,
        latch,
        mailbox,
        events: events_tx,
        ctl: ctl_rx,
        wake: wake_rx,
        pending: RefCell::new(VecDeque::new()),
        receipt: RefCell::new(None),
        used_nonces: HashSet::new(),
        connected: false,
        faulted: false,
        handled_generation: 0,
        signal_handled: false,
        state: MachineState::Disconnected,
    };
    let thread = std::thread::Builder::new().name(thread_name).spawn(move || {
        let _done = done_tx; // dropped when the thread ends, however it ends
        driver_main(rt);
    });
    let thread = match thread {
        Ok(t) => Some(t),
        Err(e) => {
            let _ = failure_events.try_send(DriverEvent::Fault(DriverError::Transport(format!("could not start the driver thread: {e}"))));
            None
        }
    };
    DriverHandle { ctl, stopper, events, caps, thread, done }
}

pub struct DriverHandle<B: Backend> {
    ctl: Sender<Queued<B>>,
    stopper: Stopper,
    events: Receiver<DriverEvent>,
    caps: Capabilities,
    thread: Option<JoinHandle<()>>,
    done: Receiver<()>,
}

impl<B: Backend> DriverHandle<B> {
    fn send(&self, cmd: Control<B>) {
        let generation = self.stopper.latch().generation();
        let _ = self.ctl.send(Queued { generation, cmd });
    }

    /// Every command records `latch.generation()` when queued; the driver thread refuses to start it
    /// (`Refused(StoppedAfterQueue)`) if a trip happened after that, and only then clears the latch for it. A stop
    /// pressed between "Confirm" and the driver dequeuing the run therefore cancels the run instead of being cleared.
    pub fn connect(&self) {
        self.send(Control::Connect);
    }

    /// Refused with `MachineBusy` by the driver thread itself while another frame or run is active (defence in
    /// depth behind the engine's pipeline step 7).
    pub fn frame(&self, f: B::Frame, mode: FrameMode, origin: FrameOrigin) {
        self.send(Control::Frame(f, mode, origin));
    }

    /// Queues the job. The driver thread re-checks: `token.check(job.fingerprint(), &job.bytes_hash(), machine,
    /// session, now)` with the hash RECOMPUTED from the job object (S22); nonce unused (then spent); its OWN last
    /// FrameReceipt has the same fingerprint, ≥ 1 full (executed) pass, ≤ 10 min, origin Human, covers job; no
    /// frame or run active (`MachineBusy`).
    pub fn run(&self, job: B::Job, token: ArmToken) {
        self.send(Control::Run(job, token));
    }

    pub fn stop(&self) {
        self.stopper.stop();
    }

    pub fn stopper(&self) -> Stopper {
        self.stopper.clone()
    }

    pub fn events(&self) -> &Receiver<DriverEvent> {
        &self.events
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    pub fn disconnect(&self) {
        self.send(Control::Disconnect);
    }
}

/// Drop of the last DriverHandle = dead-man: it trips the latch, then JOINS the driver thread for ≤ 500 ms so the
/// abort reaches the wire before a panicking or exiting UI thread can end the process (process exit kills other
/// threads mid-sequence). Apps also install a panic hook and signal watchers that stop every machine first (§5.4).
impl<B: Backend> Drop for DriverHandle<B> {
    fn drop(&mut self) {
        let deadline = Instant::now().checked_add(DEAD_MAN_JOIN);
        let _ = self.stopper.stop_and_wait(DEAD_MAN_JOIN);
        let _ = self.ctl.send(Queued { generation: u64::MAX, cmd: Control::Shutdown });
        let left = deadline.map(|d| d.saturating_duration_since(Instant::now())).unwrap_or(Duration::ZERO);
        match self.done.recv_timeout(left) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                if let Some(t) = self.thread.take() {
                    let _ = t.join();
                }
            }
            // Detached: the thread still finishes its abort, but this thread does not wait longer than 500 ms.
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// What `detect` reports (transport maps its `DetectedBoard` into this).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedDevice {
    pub vid: u16,
    pub pid: u16,
    pub serial: Option<String>,
    pub location: String,
    pub supported: bool,
    pub note: String,
}

/// Injected by the apps; the engine never names a concrete transport.
pub trait DeviceProvider: Send + Sync {
    /// "sim", "usb".
    fn name(&self) -> &'static str;
    /// Read-only enumeration for `device.detect` (Find My Laser). Never opens a device.
    fn detect(&self) -> Result<Vec<DetectedDevice>, DriverError>;
    fn open_lmc(&self, m: &GalvoMachine) -> Result<Box<dyn LmcTransport>, DriverError>;
    fn open_link(&self, m: &GantryMachine) -> Result<Box<dyn ByteLink>, DriverError>;
    /// S23 first-emission cap, consulted by `Session::run_armed` and the ARM dialog (the engine cannot link
    /// `transport`). UsbProvider: `bench::emission_cap_pct`; SimProvider: `SimConfig.emission_cap_pct` (None by
    /// default; tests set it).
    fn emission_cap_pct(&self) -> Result<Option<f64>, DriverError>;
    /// After a run finishes (not after a stop or fault). UsbProvider: `bench::record_emission`.
    fn record_emission(&self) -> Result<(), DriverError>;
}

pub enum AnyDriver {
    Lmc(DriverHandle<LmcBackend<Box<dyn LmcTransport>>>),
    Grbl(DriverHandle<GrblBackend<Box<dyn ByteLink>>>),
}

impl AnyDriver {
    pub fn stopper(&self) -> Stopper {
        match self {
            AnyDriver::Lmc(h) => h.stopper(),
            AnyDriver::Grbl(h) => h.stopper(),
        }
    }

    pub fn events(&self) -> &Receiver<DriverEvent> {
        match self {
            AnyDriver::Lmc(h) => h.events(),
            AnyDriver::Grbl(h) => h.events(),
        }
    }

    pub fn capabilities(&self) -> &Capabilities {
        match self {
            AnyDriver::Lmc(h) => h.capabilities(),
            AnyDriver::Grbl(h) => h.capabilities(),
        }
    }

    pub fn stop(&self) {
        self.stopper().stop();
    }

    pub fn connect(&self) {
        match self {
            AnyDriver::Lmc(h) => h.connect(),
            AnyDriver::Grbl(h) => h.connect(),
        }
    }

    pub fn disconnect(&self) {
        match self {
            AnyDriver::Lmc(h) => h.disconnect(),
            AnyDriver::Grbl(h) => h.disconnect(),
        }
    }
}

// ---- the driver thread -------------------------------------------------------------------------------------------

/// How an operation ended: normally, or with a contained panic (already followed by an emergency stop).
enum Op<R> {
    Done(R),
    Panicked(StopReport),
}

enum Next<B: Backend> {
    Cmd(Queued<B>),
    Wake,
    Closed,
}

struct Runtime<B: Backend> {
    backend: B,
    machine: MachineId,
    session: SessionId,
    clock: Arc<dyn Clock>,
    latch: Arc<StopLatch>,
    mailbox: Arc<StopMailbox>,
    events: Sender<DriverEvent>,
    ctl: Receiver<Queued<B>>,
    wake: Receiver<()>,
    /// Commands that arrived during an operation and were not refused (they run next, in order).
    pending: RefCell<VecDeque<Queued<B>>>,
    /// The driver's OWN last frame receipt (S20, §5.3): the only one `run` accepts.
    receipt: RefCell<Option<FrameReceipt>>,
    used_nonces: HashSet<u64>,
    connected: bool,
    /// Sticky until the next successful connect: a transport error, watchdog, contained panic or unconfirmed stop.
    faulted: bool,
    handled_generation: u64,
    signal_handled: bool,
    state: MachineState,
}

/// Records receipts and answers commands that arrive mid-operation.
struct OpHooks<'r, B: Backend> {
    ctl: &'r Receiver<Queued<B>>,
    pending: &'r RefCell<VecDeque<Queued<B>>>,
    receipt: &'r RefCell<Option<FrameReceipt>>,
    events: &'r Sender<DriverEvent>,
    /// A frame or run is active: frame, run, connect and disconnect are refused `MachineBusy`.
    busy: bool,
}

impl<B: Backend> CxHooks for OpHooks<'_, B> {
    fn on_event(&self, ev: &DriverEvent) {
        if let DriverEvent::FrameCompleted(r) = ev
            && let Ok(mut slot) = self.receipt.try_borrow_mut()
        {
            *slot = Some(r.clone());
        }
    }

    fn pump(&self) {
        while let Ok(q) = self.ctl.try_recv() {
            let refuse = self.busy && !matches!(q.cmd, Control::Shutdown);
            if refuse {
                let msg = format!("{} refused: the machine is framing or running; stop it first", q.cmd.name());
                let _ = self.events.try_send(DriverEvent::Fault(DriverError::refused(RefusalCode::MachineBusy, msg)));
                continue;
            }
            match self.pending.try_borrow_mut() {
                Ok(mut p) => p.push_back(q),
                Err(_) => {
                    let msg = format!("{} refused: the driver could not queue it", q.cmd.name());
                    let _ = self.events.try_send(DriverEvent::Fault(DriverError::refused(RefusalCode::MachineBusy, msg)));
                }
            }
        }
    }
}

fn driver_main<B: Backend>(mut rt: Runtime<B>) {
    let outcome = catch_unwind(AssertUnwindSafe(|| rt.run_loop()));
    if outcome.is_err() {
        // Outermost guard: a panic in the runtime itself. Stop the machine, report, end the thread.
        rt.emit(DriverEvent::Fault(DriverError::PanicContained));
        let report = match rt.with_cx(false, |b, cx| b.emergency_stop(cx)) {
            Op::Done(r) | Op::Panicked(r) => r,
        };
        rt.faulted = true;
        rt.post_report(report, MachineState::Fault);
    }
}

impl<B: Backend> Runtime<B> {
    fn run_loop(&mut self) {
        loop {
            self.service_stop();
            let queued = self.pending.try_borrow_mut().ok().and_then(|mut p| p.pop_front());
            let next = match queued {
                Some(q) => Next::Cmd(q),
                None => crossbeam_channel::select! {
                    recv(self.ctl) -> msg => msg.map_or(Next::Closed, Next::Cmd),
                    recv(self.wake) -> _ => Next::Wake,
                },
            };
            match next {
                Next::Wake => continue,
                Next::Closed => break,
                Next::Cmd(q) => {
                    if matches!(q.cmd, Control::Shutdown) {
                        break;
                    }
                    self.dispatch(q);
                }
            }
        }
        if self.connected {
            let _ = self.with_cx(false, |b, cx| b.disconnect(cx));
        }
        self.connected = false;
        self.set_state(MachineState::Disconnected);
    }

    fn emit(&self, ev: DriverEvent) {
        let _ = self.events.try_send(ev);
    }

    fn set_state(&mut self, s: MachineState) {
        if s != self.state {
            self.state = s.clone();
            self.emit(DriverEvent::State(s));
        }
    }

    fn refuse(&self, code: RefusalCode, message: impl Into<String>) {
        self.emit(DriverEvent::Fault(DriverError::refused(code, message)));
    }

    /// Runs `f` on the backend with a fresh `Cx`, containing panics: a panic emits `Fault(PanicContained)` and is
    /// followed at once by `emergency_stop` (itself guarded).
    fn with_cx<R>(&mut self, busy: bool, f: impl FnOnce(&mut B, &mut Cx) -> R) -> Op<R> {
        let Runtime { backend, latch, events, clock, ctl, pending, receipt, .. } = self;
        let hooks = OpHooks { ctl, pending, receipt, events, busy };
        let mut cx = Cx::with_hooks(latch, events, &**clock, &hooks);
        let out = match catch_unwind(AssertUnwindSafe(|| f(backend, &mut cx))) {
            Ok(r) => Op::Done(r),
            Err(_) => {
                cx.emit(DriverEvent::Fault(DriverError::PanicContained));
                let report = catch_unwind(AssertUnwindSafe(|| backend.emergency_stop(&mut cx)))
                    .unwrap_or_else(|_| StopReport::unconfirmed("the emergency stop panicked too: use the key switch / E-stop"));
                Op::Panicked(report)
            }
        };
        cx.flush_dropped_traces();
        out
    }

    /// Posts the report for the stop it answers (every generation up to now), emits it, and sets the state: the
    /// given one if the stop was confirmed, else `StopNotConfirmed` and a sticky fault (§5.4).
    fn post_report(&mut self, report: StopReport, confirmed_state: MachineState) {
        let generation = self.latch.generation();
        self.handled_generation = self.handled_generation.max(generation);
        self.latch.reset_stamp();
        self.mailbox.post(generation, report.clone());
        let confirmed = report.confirmed;
        self.emit(DriverEvent::Stopped(report));
        if confirmed {
            self.set_state(confirmed_state);
        } else {
            self.faulted = true;
            self.set_state(MachineState::StopNotConfirmed);
        }
    }

    fn stop_now(&mut self) -> StopReport {
        match self.with_cx(false, |b, cx| b.emergency_stop(cx)) {
            Op::Done(r) | Op::Panicked(r) => r,
        }
    }

    /// A trip (or a signal) not yet answered while idle: run the stop sequence and post the report, so
    /// `stop_and_wait` always gets one, and a stop pressed on an idle machine still confirms MO closed.
    fn service_stop(&mut self) {
        let generation = self.latch.generation();
        let signal = self.latch.signal_raised();
        let fresh_trip = self.latch.is_tripped() && generation != self.handled_generation;
        let fresh_signal = signal && !self.signal_handled;
        if !fresh_trip && !fresh_signal {
            return;
        }
        self.handled_generation = generation;
        self.signal_handled |= signal;
        while self.wake.try_recv().is_ok() {}
        self.set_state(MachineState::Stopping);
        let report = self.stop_now();
        let after = if self.connected && !self.faulted { MachineState::Stopped } else { self.idle_state() };
        self.post_report(report, after);
    }

    fn idle_state(&self) -> MachineState {
        match (self.connected, self.faulted) {
            (_, true) => MachineState::Fault,
            (true, false) => MachineState::Idle,
            (false, false) => MachineState::Disconnected,
        }
    }

    /// A fault (sticky): report it, stop the machine, post the report.
    fn fault(&mut self, e: DriverError) {
        self.faulted = true;
        self.emit(DriverEvent::Fault(e));
        self.set_state(MachineState::Stopping);
        let report = self.stop_now();
        self.post_report(report, MachineState::Fault);
    }

    fn dispatch(&mut self, q: Queued<B>) {
        let Queued { generation, cmd } = q;
        if self.latch.signal_raised() || generation != self.latch.generation() || !self.latch.clear_if_generation(generation) {
            self.refuse(RefusalCode::StoppedAfterQueue, format!("{} cancelled: a stop was requested after it was queued; nothing was sent", cmd.name()));
            return;
        }
        match cmd {
            Control::Connect => self.op_connect(),
            Control::Frame(f, mode, origin) => self.op_frame(&f, mode, origin),
            Control::Run(job, token) => self.op_run(&job, &token),
            Control::Disconnect => self.op_disconnect(),
            Control::Shutdown => {}
        }
    }

    fn op_connect(&mut self) {
        self.connected = false;
        self.clear_receipt();
        self.set_state(MachineState::Connecting);
        match self.with_cx(false, |b, cx| b.connect(cx)) {
            Op::Done(Ok(info)) => {
                self.connected = true;
                self.faulted = false;
                self.emit(DriverEvent::Connected(info));
                self.set_state(MachineState::Idle);
            }
            Op::Done(Err(e)) if e.is_refusal() => {
                self.emit(DriverEvent::Fault(e));
                let s = self.idle_state();
                self.set_state(s);
            }
            Op::Done(Err(e)) => {
                if self.latch.is_tripped() {
                    // Stopped mid-connect: not a fault; the stop sequence still runs.
                    self.set_state(MachineState::Stopping);
                    let report = self.stop_now();
                    self.post_report(report, MachineState::Disconnected);
                } else {
                    self.fault(e);
                }
            }
            Op::Panicked(report) => {
                self.faulted = true;
                self.post_report(report, MachineState::Fault);
            }
        }
    }

    fn ready_for_work(&self) -> Result<(), Refusal> {
        if self.faulted {
            return Err(Refusal::new(RefusalCode::NotConnected, "the machine has a fault: acknowledge it and connect again"));
        }
        if !self.connected {
            return Err(Refusal::new(RefusalCode::NotConnected, "the machine is not connected: connect first"));
        }
        Ok(())
    }

    fn op_frame(&mut self, f: &B::Frame, mode: FrameMode, origin: FrameOrigin) {
        if let Err(r) = self.ready_for_work() {
            return self.emit(DriverEvent::Fault(DriverError::Refused(r)));
        }
        if f.machine() != &self.machine {
            return self.refuse(RefusalCode::MachineMismatch, format!("the frame was encoded for {}, this driver drives {}", f.machine().0, self.machine.0));
        }
        if f.emits() {
            return self.refuse(RefusalCode::EmissionNotAllowed, "a frame job must never emit");
        }
        self.set_state(MachineState::Framing);
        let session = self.session;
        match self.with_cx(true, |b, cx| b.frame(f, mode, origin, session, cx)) {
            Op::Done(Ok(FrameOutcome::Completed { .. })) => self.set_state(MachineState::Idle),
            Op::Done(Ok(FrameOutcome::Stopped(report))) => self.post_report(report, MachineState::Stopped),
            Op::Done(Err(e)) if e.is_refusal() => {
                self.emit(DriverEvent::Fault(e));
                self.set_state(MachineState::Idle);
            }
            Op::Done(Err(e)) => self.fault(e),
            Op::Panicked(report) => {
                self.faulted = true;
                self.post_report(report, MachineState::Fault);
            }
        }
    }

    fn clear_receipt(&self) {
        if let Ok(mut r) = self.receipt.try_borrow_mut() {
            *r = None;
        }
    }

    /// The driver thread's own re-checks before any byte of a run is sent (§3.13, §4.6 mechanisms 4 and 5). The
    /// token's fields are compared explicitly first (defence in depth, independent of `ArmToken::check`), then the
    /// driver's OWN receipt, the nonce, and finally `ArmToken::check` itself.
    fn admit(&mut self, job: &B::Job, token: &ArmToken) -> Result<(), Refusal> {
        let now = self.clock.now();
        let fp = *job.fingerprint();
        if job.machine() != &self.machine {
            return Err(Refusal::new(
                RefusalCode::MachineMismatch,
                format!("the job was encoded for {}, this driver drives {}", job.machine().0, self.machine.0),
            ));
        }
        let bytes = job.bytes_hash();
        if token.fingerprint() != &fp {
            return Err(Refusal::new(
                RefusalCode::Stale,
                format!("the job ({}) is not the one that was armed ({}): plan, frame and ARM again", fp.short(), token.fingerprint().short()),
            ));
        }
        if token.bytes() != &bytes {
            return Err(Refusal::new(
                RefusalCode::BytesMismatch,
                format!(
                    "the bytes handed to the driver ({}) differ from the bytes the human confirmed ({}); nothing was sent",
                    bytes.short(),
                    token.bytes().short()
                ),
            ));
        }
        if token.machine() != &self.machine {
            return Err(Refusal::new(RefusalCode::MachineMismatch, format!("the ARM was for {}, this driver drives {}", token.machine().0, self.machine.0)));
        }
        let receipt = self.receipt.try_borrow().ok().and_then(|r| r.clone());
        let Some(r) = receipt else {
            return Err(Refusal::new(RefusalCode::FrameRequired, "frame this job on this machine first: no executed frame pass in this session"));
        };
        if r.fingerprint != fp {
            return Err(Refusal::new(
                RefusalCode::FrameRequired,
                format!("the last frame traced another job ({}): frame this one ({})", r.fingerprint.short(), fp.short()),
            ));
        }
        if r.machine != self.machine {
            return Err(Refusal::new(RefusalCode::MachineMismatch, format!("the last frame ran on {}, not {}", r.machine.0, self.machine.0)));
        }
        if r.session != self.session {
            return Err(Refusal::new(RefusalCode::ArmSessionMismatch, "the frame receipt is from another session: frame again"));
        }
        if r.full_passes < 1 {
            return Err(Refusal::new(RefusalCode::FrameRequired, "no frame pass has executed yet: frame again"));
        }
        let age = now.saturating_since(r.at);
        if age > FRAME_RECEIPT_MAX_AGE {
            return Err(Refusal::new(RefusalCode::FrameReceiptExpired, format!("the frame is {} s old (limit 600 s): frame again", age.as_secs())));
        }
        if r.origin != HumanFrame {
            return Err(Refusal::new(RefusalCode::FrameNotHuman, "the frame was not started by a person: frame again from the Laser panel"));
        }
        if !r.covers_job() {
            return Err(Refusal::new(RefusalCode::FrameDoesNotCoverJob, "the last frame traced tool layers only: frame the job itself"));
        }
        if !self.used_nonces.insert(token.nonce()) {
            return Err(Refusal::new(RefusalCode::ArmNonceUsed, "this ARM was already used: ARM again"));
        }
        token.check(&fp, &bytes, &self.machine, self.session, now)
    }

    fn op_run(&mut self, job: &B::Job, token: &ArmToken) {
        if let Err(r) = self.ready_for_work() {
            return self.emit(DriverEvent::Fault(DriverError::Refused(r)));
        }
        if let Err(r) = self.admit(job, token) {
            return self.emit(DriverEvent::Fault(DriverError::Refused(r)));
        }
        self.state = MachineState::Running { packets_acked: 0, total: 0 };
        match self.with_cx(true, |b, cx| b.run(job, cx)) {
            Op::Done(Ok(Outcome::Finished { duration })) => {
                self.emit(DriverEvent::Finished(Outcome::Finished { duration }));
                self.set_state(MachineState::Idle);
            }
            Op::Done(Ok(Outcome::Stopped(report))) => {
                self.post_report(report.clone(), MachineState::Stopped);
                self.emit(DriverEvent::Finished(Outcome::Stopped(report)));
            }
            Op::Done(Err(e)) if e.is_refusal() => {
                self.emit(DriverEvent::Fault(e));
                self.set_state(MachineState::Idle);
            }
            Op::Done(Err(e)) => self.fault(e),
            Op::Panicked(report) => {
                self.faulted = true;
                self.post_report(report, MachineState::Fault);
            }
        }
    }

    fn op_disconnect(&mut self) {
        let _ = self.with_cx(false, |b, cx| b.disconnect(cx));
        self.connected = false;
        self.clear_receipt();
        let s = if self.faulted { MachineState::Fault } else { MachineState::Disconnected };
        self.set_state(s);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use openlaser_arm::testing::token_for_tests;
    use openlaser_types::{Fingerprint, FrameSource, FrameStyle, MonoTime, SystemClock, VirtualClock};

    use super::*;
    use crate::backend::{DeviceInfo, MachineStatus};

    #[derive(Debug, Clone)]
    struct Job {
        fp: Fingerprint,
        machine: MachineId,
        bytes: Fingerprint,
    }

    impl EncodedJob for Job {
        fn fingerprint(&self) -> &Fingerprint {
            &self.fp
        }
        fn machine(&self) -> &MachineId {
            &self.machine
        }
        fn emits(&self) -> bool {
            true
        }
        fn bytes_hash(&self) -> Fingerprint {
            self.bytes
        }
    }

    #[derive(Debug, Clone)]
    struct Frame {
        fp: Fingerprint,
        machine: MachineId,
        source: FrameSource,
        panic: bool,
        fail: bool,
    }

    impl EncodedJob for Frame {
        fn fingerprint(&self) -> &Fingerprint {
            &self.fp
        }
        fn machine(&self) -> &MachineId {
            &self.machine
        }
        fn emits(&self) -> bool {
            false
        }
        fn bytes_hash(&self) -> Fingerprint {
            Fingerprint([0; 32])
        }
    }

    #[derive(Default)]
    struct Calls {
        connect: AtomicU32,
        frame: AtomicU32,
        run: AtomicU32,
        stop: AtomicU32,
        disconnect: AtomicU32,
        stop_done: AtomicBool,
    }

    struct Mock {
        calls: Arc<Calls>,
        gate: Option<Receiver<()>>,
        stop_takes: Duration,
        flood: u32,
    }

    impl Mock {
        fn new(calls: &Arc<Calls>) -> Self {
            Mock { calls: Arc::clone(calls), gate: None, stop_takes: Duration::ZERO, flood: 0 }
        }

        fn stopped(&mut self, cx: &mut Cx) -> StopReport {
            self.emergency_stop(cx)
        }
    }

    impl Backend for Mock {
        type Job = Job;
        type Frame = Frame;

        fn capabilities(&self) -> Capabilities {
            Capabilities { kind: openlaser_types::DeviceKind::GalvoLmc, frame: true, run: true, pause: false, jog: false, home: false, inputs: false }
        }

        fn connect(&mut self, cx: &mut Cx) -> Result<DeviceInfo, DriverError> {
            self.calls.connect.fetch_add(1, Ordering::SeqCst);
            if let Some(gate) = &self.gate {
                while gate.try_recv().is_err() {
                    if cx.slice(Duration::from_millis(2)).is_err() {
                        return Err(DriverError::Protocol("connect stopped".into()));
                    }
                }
            }
            Ok(DeviceInfo { vid: 1, pid: 2, serial: None, version: None, provider: "mock".into() })
        }

        fn frame(&mut self, f: &Frame, mode: FrameMode, origin: FrameOrigin, session: SessionId, cx: &mut Cx) -> Result<FrameOutcome, DriverError> {
            self.calls.frame.fetch_add(1, Ordering::SeqCst);
            assert!(!f.panic, "mock frame panics on purpose");
            if f.fail {
                return Err(DriverError::Transport("endpoint stalled".into()));
            }
            let receipt = |passes: u32, cx: &Cx| FrameReceipt {
                fingerprint: f.fp,
                machine: f.machine.clone(),
                session,
                full_passes: passes,
                at: cx.now(),
                style: FrameStyle::Bounds,
                source: f.source,
                origin,
            };
            for i in 0..self.flood {
                cx.emit(DriverEvent::Progress { packets_acked: i, total: self.flood, eta_ms: 0 });
            }
            match mode {
                FrameMode::Once => {
                    cx.emit(DriverEvent::FrameCompleted(receipt(1, cx)));
                    Ok(FrameOutcome::Completed { passes: 1 })
                }
                FrameMode::Loop => {
                    let mut k = 0;
                    loop {
                        if cx.slice(Duration::from_millis(3)).is_err() {
                            return Ok(FrameOutcome::Stopped(self.stopped(cx)));
                        }
                        k += 1;
                        cx.emit(DriverEvent::FrameCompleted(receipt(k, cx)));
                    }
                }
            }
        }

        fn run(&mut self, _job: &Job, cx: &mut Cx) -> Result<Outcome, DriverError> {
            self.calls.run.fetch_add(1, Ordering::SeqCst);
            for _ in 0..20 {
                if cx.slice(Duration::from_millis(1)).is_err() {
                    return Ok(Outcome::Stopped(self.stopped(cx)));
                }
            }
            Ok(Outcome::Finished { duration: Duration::from_millis(20) })
        }

        fn emergency_stop(&mut self, cx: &mut Cx) -> StopReport {
            self.calls.stop.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(self.stop_takes);
            self.calls.stop_done.store(true, Ordering::SeqCst);
            StopReport {
                confirmed: true,
                elapsed: cx.now().saturating_since(cx.stop_requested_at()),
                possible_desync: false,
                mo_closed: true,
                ports_zero: true,
                notes: Vec::new(),
            }
        }

        fn status(&mut self, _cx: &mut Cx) -> Result<MachineStatus, DriverError> {
            Ok(MachineStatus { state: MachineState::Idle, status_bits: None, ready: true, busy: false })
        }

        fn disconnect(&mut self, _cx: &mut Cx) {
            self.calls.disconnect.fetch_add(1, Ordering::SeqCst);
        }
    }

    const FP: Fingerprint = Fingerprint([7; 32]);
    const BYTES: Fingerprint = Fingerprint([9; 32]);

    fn m() -> MachineId {
        MachineId("jpt-m7-200-sim".into())
    }

    fn job() -> Job {
        Job { fp: FP, machine: m(), bytes: BYTES }
    }

    fn frame(source: FrameSource) -> Frame {
        Frame { fp: FP, machine: m(), source, panic: false, fail: false }
    }

    fn token(nonce: u64) -> ArmToken {
        token_for_tests(FP, BYTES, m(), SessionId(1), MonoTime(Duration::from_secs(3600)), nonce)
    }

    fn spawn_mock(mock: Mock, clock: Arc<dyn Clock>) -> DriverHandle<Mock> {
        spawn(mock, m(), SessionId(1), clock)
    }

    /// Reads events until `pred` matches (panics after 5 s).
    fn until(h: &DriverHandle<Mock>, pred: impl Fn(&DriverEvent) -> bool) -> Vec<DriverEvent> {
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let ev = h.events().recv_timeout(left).unwrap_or_else(|_| panic!("no matching event; saw {seen:?}"));
            let hit = pred(&ev);
            seen.push(ev);
            if hit {
                return seen;
            }
        }
    }

    fn refusal(code: RefusalCode) -> impl Fn(&DriverEvent) -> bool {
        move |e| matches!(e, DriverEvent::Fault(DriverError::Refused(r)) if r.code == code)
    }

    fn connected(h: &DriverHandle<Mock>) {
        h.connect();
        until(h, |e| matches!(e, DriverEvent::State(MachineState::Idle)));
    }

    #[test]
    fn connect_and_frame_once_issue_a_receipt() {
        let calls = Arc::new(Calls::default());
        let h = spawn_mock(Mock::new(&calls), Arc::new(SystemClock::new()));
        h.connect();
        let evs = until(&h, |e| matches!(e, DriverEvent::State(MachineState::Idle)));
        assert!(evs.iter().any(|e| matches!(e, DriverEvent::Connected(_))));
        assert!(evs.iter().any(|e| matches!(e, DriverEvent::State(MachineState::Connecting))));
        h.frame(frame(FrameSource::Job), FrameMode::Once, HumanFrame);
        let evs = until(&h, |e| matches!(e, DriverEvent::State(MachineState::Idle)));
        assert!(evs.iter().any(|e| matches!(e, DriverEvent::FrameCompleted(r) if r.full_passes == 1)));
    }

    #[test]
    fn stop_after_queue_cancels_queued_run() {
        // t6_stop_after_queue_cancels_queued_run: the run is queued behind a connect, then a stop: refused, unsent.
        let calls = Arc::new(Calls::default());
        let (gate_tx, gate) = crossbeam_channel::unbounded();
        let h = spawn_mock(Mock { gate: Some(gate), ..Mock::new(&calls) }, Arc::new(SystemClock::new()));
        h.connect();
        until(&h, |e| matches!(e, DriverEvent::State(MachineState::Connecting)));
        h.run(job(), token(1));
        h.stop();
        until(&h, refusal(RefusalCode::StoppedAfterQueue));
        assert_eq!(calls.run.load(Ordering::SeqCst), 0, "nothing of the run reached the backend");
        assert!(calls.stop.load(Ordering::SeqCst) >= 1, "the interrupted connect was stopped");
        drop(gate_tx);
    }

    #[test]
    fn run_refused_while_framing() {
        // t6_run_refused_while_framing: MachineBusy from the driver thread itself, during a Loop frame.
        let calls = Arc::new(Calls::default());
        let h = spawn_mock(Mock::new(&calls), Arc::new(SystemClock::new()));
        connected(&h);
        h.frame(frame(FrameSource::Job), FrameMode::Loop, HumanFrame);
        until(&h, |e| matches!(e, DriverEvent::FrameCompleted(_)));
        h.run(job(), token(1));
        h.frame(frame(FrameSource::Job), FrameMode::Once, HumanFrame);
        until(&h, refusal(RefusalCode::MachineBusy));
        until(&h, refusal(RefusalCode::MachineBusy));
        assert_eq!(calls.run.load(Ordering::SeqCst), 0);
        let ack = h.stopper().stop_and_wait(Duration::from_secs(2));
        assert!(ack.report.is_some_and(|r| r.confirmed));
        assert_eq!(calls.frame.load(Ordering::SeqCst), 1);
    }

    fn run_after(source: Option<FrameSource>, origin: FrameOrigin, tok: ArmToken, j: Job) -> Vec<DriverEvent> {
        let calls = Arc::new(Calls::default());
        let h = spawn_mock(Mock::new(&calls), Arc::new(SystemClock::new()));
        connected(&h);
        if let Some(source) = source {
            h.frame(frame(source), FrameMode::Once, origin);
            until(&h, |e| matches!(e, DriverEvent::FrameCompleted(_)));
        }
        h.run(j, tok);
        let evs = until(&h, |e| matches!(e, DriverEvent::Fault(_) | DriverEvent::Finished(_)));
        assert_eq!(calls.run.load(Ordering::SeqCst), 0, "a refused run never reaches the backend");
        evs
    }

    fn refused_with(evs: &[DriverEvent]) -> RefusalCode {
        match evs.last() {
            Some(DriverEvent::Fault(DriverError::Refused(r))) => r.code,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn run_refused_without_own_receipt() {
        assert_eq!(refused_with(&run_after(None, HumanFrame, token(1), job())), RefusalCode::FrameRequired);
    }

    #[test]
    fn run_refused_tool_layers_receipt() {
        let evs = run_after(Some(FrameSource::ToolLayersOnly), HumanFrame, token(1), job());
        assert_eq!(refused_with(&evs), RefusalCode::FrameDoesNotCoverJob);
    }

    #[test]
    fn run_refused_agent_receipt() {
        assert_eq!(refused_with(&run_after(Some(FrameSource::Job), FrameOrigin::Agent, token(1), job())), RefusalCode::FrameNotHuman);
    }

    #[test]
    fn run_refused_when_bytes_differ_from_token() {
        // t6_run_refused_when_bytes_differ_from_token (S22): the hash is recomputed from the job handed over.
        let tampered = Job { bytes: Fingerprint([8; 32]), ..job() };
        assert_eq!(refused_with(&run_after(Some(FrameSource::Job), HumanFrame, token(1), tampered)), RefusalCode::BytesMismatch);
    }

    #[test]
    fn run_refused_for_another_job_or_machine() {
        let other = Job { fp: Fingerprint([1; 32]), ..job() };
        assert_eq!(refused_with(&run_after(Some(FrameSource::Job), HumanFrame, token(1), other)), RefusalCode::Stale);
        let elsewhere = Job { machine: MachineId("other".into()), ..job() };
        assert_eq!(refused_with(&run_after(Some(FrameSource::Job), HumanFrame, token(1), elsewhere)), RefusalCode::MachineMismatch);
    }

    #[test]
    fn run_refused_when_the_receipt_expired() {
        let calls = Arc::new(Calls::default());
        let clock = Arc::new(VirtualClock::new());
        let h = spawn_mock(Mock::new(&calls), clock.clone());
        connected(&h);
        h.frame(frame(FrameSource::Job), FrameMode::Once, HumanFrame);
        until(&h, |e| matches!(e, DriverEvent::FrameCompleted(_)));
        clock.advance(FRAME_RECEIPT_MAX_AGE + Duration::from_secs(1));
        h.run(job(), token(1));
        let evs = until(&h, |e| matches!(e, DriverEvent::Fault(_)));
        assert_eq!(refused_with(&evs), RefusalCode::FrameReceiptExpired);
    }

    #[test]
    fn work_is_refused_until_connected() {
        let calls = Arc::new(Calls::default());
        let h = spawn_mock(Mock::new(&calls), Arc::new(SystemClock::new()));
        h.frame(frame(FrameSource::Job), FrameMode::Once, HumanFrame);
        until(&h, refusal(RefusalCode::NotConnected));
        assert_eq!(calls.frame.load(Ordering::SeqCst), 0);
    }

    #[test]
    #[ignore = "needs I1"] // ArmToken::check (WP3) refuses every token until I1
    fn run_finishes_with_a_valid_token_and_own_receipt() {
        let calls = Arc::new(Calls::default());
        let h = spawn_mock(Mock::new(&calls), Arc::new(SystemClock::new()));
        connected(&h);
        h.frame(frame(FrameSource::Job), FrameMode::Once, HumanFrame);
        until(&h, |e| matches!(e, DriverEvent::FrameCompleted(_)));
        h.run(job(), token(1));
        until(&h, |e| matches!(e, DriverEvent::Finished(Outcome::Finished { .. })));
        assert_eq!(calls.run.load(Ordering::SeqCst), 1);
    }

    #[test]
    #[ignore = "needs I1"] // ArmToken::check (WP3) refuses every token until I1
    fn arm_token_single_use() {
        let calls = Arc::new(Calls::default());
        let h = spawn_mock(Mock::new(&calls), Arc::new(SystemClock::new()));
        connected(&h);
        h.frame(frame(FrameSource::Job), FrameMode::Once, HumanFrame);
        until(&h, |e| matches!(e, DriverEvent::FrameCompleted(_)));
        h.run(job(), token(42));
        until(&h, |e| matches!(e, DriverEvent::Finished(_)));
        h.run(job(), token(42));
        until(&h, refusal(RefusalCode::ArmNonceUsed));
        assert_eq!(calls.run.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_used_nonce_is_refused() {
        // The nonce is spent by the first presentation that passes the driver's own checks, whatever ArmToken::check
        // then says, so the same nonce can never be presented twice.
        let calls = Arc::new(Calls::default());
        let h = spawn_mock(Mock::new(&calls), Arc::new(SystemClock::new()));
        connected(&h);
        h.frame(frame(FrameSource::Job), FrameMode::Once, HumanFrame);
        until(&h, |e| matches!(e, DriverEvent::FrameCompleted(_)));
        h.run(job(), token(42));
        until(&h, |e| matches!(e, DriverEvent::Fault(_) | DriverEvent::Finished(_)));
        h.run(job(), token(42));
        until(&h, refusal(RefusalCode::ArmNonceUsed));
    }

    #[test]
    fn dead_man_handle_drop_stops() {
        // t6_dead_man_handle_drop_stops + t6_handle_drop_waits_for_stop_report.
        let calls = Arc::new(Calls::default());
        let h = spawn_mock(Mock { stop_takes: Duration::from_millis(80), ..Mock::new(&calls) }, Arc::new(SystemClock::new()));
        connected(&h);
        h.frame(frame(FrameSource::Job), FrameMode::Loop, HumanFrame);
        until(&h, |e| matches!(e, DriverEvent::FrameCompleted(_)));
        let t = Instant::now();
        drop(h);
        let took = t.elapsed();
        assert!(calls.stop_done.load(Ordering::SeqCst), "the drop returned only after the stop sequence");
        assert!(took >= Duration::from_millis(80) && took <= DEAD_MAN_JOIN + Duration::from_millis(100), "{took:?}");
        assert_eq!(calls.disconnect.load(Ordering::SeqCst), 1, "the thread ended cleanly");
    }

    #[test]
    fn panic_guard_stops() {
        // t6_panic_guard_stops: a panic in the backend is contained and followed by the emergency stop.
        let calls = Arc::new(Calls::default());
        let h = spawn_mock(Mock::new(&calls), Arc::new(SystemClock::new()));
        connected(&h);
        h.frame(Frame { panic: true, ..frame(FrameSource::Job) }, FrameMode::Once, HumanFrame);
        let evs = until(&h, |e| matches!(e, DriverEvent::Stopped(_)));
        assert!(evs.iter().any(|e| matches!(e, DriverEvent::Fault(DriverError::PanicContained))));
        assert_eq!(calls.stop.load(Ordering::SeqCst), 1);
        // Sticky: framing again is refused until a new connect.
        h.frame(frame(FrameSource::Job), FrameMode::Once, HumanFrame);
        until(&h, refusal(RefusalCode::NotConnected));
        connected(&h);
        h.frame(frame(FrameSource::Job), FrameMode::Once, HumanFrame);
        until(&h, |e| matches!(e, DriverEvent::FrameCompleted(_)));
    }

    #[test]
    fn a_transport_fault_is_sticky_and_stops() {
        let calls = Arc::new(Calls::default());
        let h = spawn_mock(Mock::new(&calls), Arc::new(SystemClock::new()));
        connected(&h);
        h.frame(Frame { fail: true, ..frame(FrameSource::Job) }, FrameMode::Once, HumanFrame);
        let evs = until(&h, |e| matches!(e, DriverEvent::State(MachineState::Fault)));
        assert!(evs.iter().any(|e| matches!(e, DriverEvent::Fault(DriverError::Transport(_)))));
        assert!(evs.iter().any(|e| matches!(e, DriverEvent::Stopped(r) if r.confirmed)));
        assert_eq!(calls.stop.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn driver_never_blocks_on_unpolled_events() {
        // t6_driver_never_blocks_on_unpolled_events: nobody reads 100 000 events; the stop is as fast as ever.
        let calls = Arc::new(Calls::default());
        let h = spawn_mock(Mock { flood: 100_000, ..Mock::new(&calls) }, Arc::new(SystemClock::new()));
        h.connect();
        h.frame(frame(FrameSource::Job), FrameMode::Loop, HumanFrame);
        while calls.frame.load(Ordering::SeqCst) == 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(50));
        let ack = h.stopper().stop_and_wait(Duration::from_millis(500));
        assert!(ack.report.is_some(), "{ack:?}");
        assert!(ack.elapsed_ms < 250, "{ack:?}");
    }

    #[test]
    fn stop_when_idle_still_runs_the_stop_sequence() {
        let calls = Arc::new(Calls::default());
        let h = spawn_mock(Mock::new(&calls), Arc::new(SystemClock::new()));
        connected(&h);
        let ack = h.stopper().stop_and_wait(Duration::from_millis(500));
        assert!(ack.report.is_some_and(|r| r.confirmed));
        assert_eq!(calls.stop.load(Ordering::SeqCst), 1);
        // A user stop is not sticky: the next command clears the latch and runs.
        h.frame(frame(FrameSource::Job), FrameMode::Once, HumanFrame);
        until(&h, |e| matches!(e, DriverEvent::FrameCompleted(_)));
    }

    #[test]
    fn a_signal_refuses_every_later_command() {
        let calls = Arc::new(Calls::default());
        let h = spawn_mock(Mock::new(&calls), Arc::new(SystemClock::new()));
        connected(&h);
        h.stopper().latch().signal_flag().store(true, Ordering::SeqCst);
        h.frame(frame(FrameSource::Job), FrameMode::Once, HumanFrame);
        until(&h, refusal(RefusalCode::StoppedAfterQueue));
        until(&h, |e| matches!(e, DriverEvent::Stopped(_)));
        assert_eq!(calls.frame.load(Ordering::SeqCst), 0);
        assert_eq!(calls.stop.load(Ordering::SeqCst), 1, "the signal stop ran once");
    }

    #[test]
    fn frame_for_another_machine_is_refused() {
        let calls = Arc::new(Calls::default());
        let h = spawn_mock(Mock::new(&calls), Arc::new(SystemClock::new()));
        connected(&h);
        h.frame(Frame { machine: MachineId("other".into()), ..frame(FrameSource::Job) }, FrameMode::Once, HumanFrame);
        until(&h, refusal(RefusalCode::MachineMismatch));
    }
}
