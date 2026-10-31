//! The machine session behind the CLI: the engine's `Session` with `Origin::Human` (SPEC §2.3 rule 13: openlaser-hw
//! is a human surface), the injected `DirProfileStore` (S24), and a device provider: `SimProvider` for `--sim`, or
//! `transport::UsbProvider` in a build with feature `hw` (S4, S23). Without `hw`, every real-device path is refused
//! with `HardwareDisabled` before anything is opened.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use openlaser_driver::Single;
use openlaser_engine::{
    ArmToken, DeviceAccess, DeviceProvider, DriverEvent, EmissionSummary, EngineError, Event, Fingerprint, FrameMode, FrameReceipt, FrameSource, FrameStyle,
    FsAuthority, JobOptions, MachineId, MachineProfile, MonoTime, Origin, Outcome, ProfileStore, Session, SessionConfig, SessionId, StopReport,
};
use openlaser_format::machines::DirProfileStore;
use openlaser_lmc_sim::{SimConfig, SimProvider};
use openlaser_machine::{FieldRect, GalvoLink, GalvoMachine, Kinematics};
use openlaser_types::{Clock, RefusalCode};
use serde_json::{Value, json};

use crate::args::{DeviceOpts, JobSource};
use crate::stop::StopTarget;
use crate::trace::{TraceWriter, WireEvent};
use crate::{HwError, Terminal};

/// The real-USB profile, and the simulator twin `--sim` defaults to.
pub const DEFAULT_USB_MACHINE: &str = "jpt-m7-200";
pub const DEFAULT_SIM_MACHINE: &str = "jpt-m7-200-sim";

/// What the flows learn from the machine.
#[derive(Debug, Clone, PartialEq)]
pub enum RigEvent {
    /// The board executed a frame pass (S20).
    FrameCompleted(FrameReceipt),
    Progress {
        packets_acked: u32,
        total: u32,
        eta_ms: u64,
    },
    Finished(Duration),
    Stopped(StopReport),
    Fault(String),
    /// The engine voided the pending ARM (a stop, an edit).
    ArmVoided(String),
    /// Something the operator should read that does not change the machine (a trace file that failed).
    Warning(String),
}

/// What the ARM step shows and mints from: the engine's `PendingArm` (S22: its job is the one that will be sent).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmView {
    pub summary: EmissionSummary,
    pub receipt: FrameReceipt,
    pub checklist: Vec<String>,
}

/// Why the rig is being built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Status,
    Frame,
    Run,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RigSpec {
    pub purpose: Purpose,
    pub device: DeviceOpts,
    /// `frame --builtin corners --extent <mm>`: frame past the work area, red light only (H27).
    pub extent: Option<f64>,
}

/// One machine session. Every method runs on the main thread; only [`Rig::stopper`] crosses threads.
pub trait Rig {
    fn prepare_rotary(&mut self) -> Result<(), HwError> {
        Err(HwError::Failed("rotary control is unavailable on this rig".into()))
    }
    fn rotary(&mut self, _degrees: Option<f64>) -> Result<i32, HwError> {
        Err(HwError::Failed("rotary control is unavailable on this rig".into()))
    }
    fn machine(&self) -> &MachineId;
    fn session(&self) -> SessionId;
    fn now(&self) -> MonoTime;
    /// Opens the design and applies the run overrides (`--layers`, `--repeats`, `--power-pct`).
    fn load(&mut self, job: &JobSource, layers: Option<&Path>, repeats: Option<u16>, power_pct: Option<f64>, extent: Option<f64>) -> Result<(), HwError>;
    /// plan → validate → encode → decode: the summary of the bytes (display only; ARM uses `request_arm`).
    fn summary(&mut self) -> Result<EmissionSummary, HwError>;
    fn connect(&mut self) -> Result<(), HwError>;
    fn start_frame(&mut self, mode: FrameMode, style: Option<FrameStyle>, source: Option<FrameSource>) -> Result<Fingerprint, HwError>;
    fn poll(&mut self) -> Vec<RigEvent>;
    /// Every machine of this session (the key thread, the signal watcher and the panic hook hold it).
    fn stopper(&self) -> Arc<dyn StopTarget>;
    /// `laser.start`: cold-builds the job and stores the PendingArm. Never emits.
    fn request_arm(&mut self) -> Result<ArmView, HwError>;
    /// S23: the provider's first-emission cap (bench session; `SimConfig.emission_cap_pct` on the simulator).
    fn emission_cap_pct(&self) -> Result<Option<f64>, HwError>;
    /// Sends exactly the PendingArm's job with the token (typed API only; never re-encodes).
    fn run_armed(&mut self, token: ArmToken) -> Result<(), HwError>;
    /// `status`: GetSerialNo + GetVersion only, no init (H12).
    fn probe(&mut self) -> Result<Vec<String>, HwError>;
    fn disconnect(&mut self);
}

/// Builds rigs (the binary: [`EngineRigs`]; tests: scripted).
pub trait RigFactory {
    fn open(&mut self, spec: &RigSpec) -> Result<Box<dyn Rig>, HwError>;
}

fn engine(e: EngineError) -> HwError {
    match e {
        EngineError::Refused(r) => HwError::Refused(r),
        EngineError::JobRefused(j) => job_refused(*j),
        other => HwError::Failed(other.to_string()),
    }
}

fn job_refused(j: openlaser_engine::JobRefused) -> HwError {
    if j.refusals.is_empty() { HwError::Failed(j.error.unwrap_or_else(|| "the job could not be built".to_owned())) } else { HwError::Refusals(j.refusals) }
}

/// `status` (H12): GetSerialNo and GetVersion, each answered within 250 ms or a fault (never retried).
const PROBE_GET_SERIAL_NO: u16 = 0x0009;
const PROBE_GET_VERSION: u16 = 0x0007;
const PROBE_SLICE: Duration = Duration::from_millis(20);
const PROBE_BUDGET: Duration = Duration::from_millis(250);
/// GetVersion word 3 (R05 §1.3).
const STATUS_READY: u16 = 0x0020;
const STATUS_BUSY: u16 = 0x0004;

fn driver_error(e: openlaser_driver::DriverError) -> HwError {
    match e {
        openlaser_driver::DriverError::Refused(r) => HwError::Refused(r),
        other => HwError::Failed(other.to_string()),
    }
}

/// The real device provider: `hw` builds only.
#[cfg(feature = "hw")]
fn usb_provider() -> Result<Arc<dyn DeviceProvider>, HwError> {
    Ok(Arc::new(openlaser_transport::UsbProvider::new()))
}

#[cfg(not(feature = "hw"))]
fn usb_provider() -> Result<Arc<dyn DeviceProvider>, HwError> {
    Err(HwError::refused(
        RefusalCode::HardwareDisabled,
        "this openlaser-hw was built without the `hw` feature, so it cannot open the real laser; add --sim to drive the simulator",
    ))
}

/// `openlaser-hw detect`: read-only enumeration (`DeviceProvider::detect` never opens a device).
pub fn detect(term: &mut dyn Terminal) -> Result<(), HwError> {
    let provider = usb_provider()?;
    let found = provider.detect().map_err(|e| HwError::Failed(e.to_string()))?;
    if found.is_empty() {
        term.write("No LMC board found.\n");
    }
    for d in found {
        term.write(&format!(
            "{:04x}:{:04x} {} at {}{}: {}\n",
            d.vid,
            d.pid,
            d.serial.as_deref().map_or_else(|| "(no serial)".to_owned(), |s| crate::plain::field(s, crate::plain::NAME_MAX_CHARS)),
            crate::plain::field(&d.location, crate::plain::MESSAGE_MAX_CHARS),
            if d.supported { "" } else { " (not supported)" },
            crate::plain::field(&d.note, crate::plain::MESSAGE_MAX_CHARS)
        ));
    }
    Ok(())
}

/// Builds [`EngineRig`]s.
pub struct EngineRigs {
    profiles_dir: Option<PathBuf>,
    sim: SimConfig,
    clock: Arc<dyn Clock>,
}

impl EngineRigs {
    pub fn new(profiles_dir: Option<PathBuf>, sim: SimConfig, clock: Arc<dyn Clock>) -> Self {
        EngineRigs { profiles_dir, sim, clock }
    }
}

impl RigFactory for EngineRigs {
    fn open(&mut self, spec: &RigSpec) -> Result<Box<dyn Rig>, HwError> {
        Ok(Box::new(EngineRig::open(spec, self.profiles_dir.as_deref(), &self.sim, Arc::clone(&self.clock))?))
    }
}

/// The engine-backed rig.
pub struct EngineRig {
    rotary: Option<openlaser_driver::RotarySession<Box<dyn openlaser_driver::LmcTransport>>>,
    session: Session,
    clock: Arc<dyn Clock>,
    machine: MachineId,
    provider: Arc<dyn DeviceProvider>,
    sim: Option<Arc<SimProvider>>,
    trace: Option<TraceState>,
    /// `--trace` on the real USB device: the recording transport's file (see [`crate::usb_trace`]).
    usb_trace: Option<crate::usb_trace::SharedSink>,
    /// The machine's galvo settings, for `status` (None for a gantry).
    galvo: Option<GalvoMachine>,
}

struct TraceState {
    writer: TraceWriter,
    /// The FakeLmc recorder being copied (a new connection opens a new board) and its events already written.
    recorder: Option<Arc<openlaser_lmc_sim::Recorder>>,
    seen: usize,
    opened: bool,
    last_pump: Option<MonoTime>,
    /// The first write failure, reported once.
    failed: Option<String>,
    reported: bool,
}

/// How often the simulator's recording is copied into the trace (a snapshot clones the whole recording).
const TRACE_PUMP_EVERY: Duration = Duration::from_millis(250);

/// `--extent`: the in-memory profile frames up to ±extent mm (never saved; the frame command never marks).
fn widen_for_extent(p: &mut MachineProfile, extent: f64) -> Result<(), HwError> {
    let Kinematics::Galvo(g) = &mut p.kind else {
        return Err(HwError::refused(RefusalCode::WrongDeviceKind, "--extent is for galvo machines"));
    };
    let f = g.field_f();
    let half = f[0].min(f[1]) / 2.0;
    if !(extent.is_finite() && extent > 0.0 && extent < half) {
        return Err(HwError::refused(
            RefusalCode::NotEncodable,
            format!("--extent {extent} mm is not strictly inside the encodable square (±{half:.3} mm at F = {:.2} mm)", f[0].min(f[1])),
        ));
    }
    let rect = FieldRect { min: [-extent, -extent], max: [extent, extent] };
    g.work_area = rect;
    g.h27_cleared = Some(rect);
    Ok(())
}

/// A saved profile file that could not be loaded leaves its machine on the built-in settings (calibration, axis
/// reversal, work area), so a device command refuses until the file is fixed or removed, never runs on the
/// built-in in its place (AGENTS.md §1 rule 3).
pub fn refuse_skipped_profiles(skipped: &[String]) -> Result<(), HwError> {
    if skipped.is_empty() {
        return Ok(());
    }
    Err(HwError::Failed(format!(
        "machine profile files were skipped, so their machines would run on the built-in settings: {}. Fix or remove them \
         (`openlaser-hw profile show` lists them) and run again; nothing was opened",
        crate::plain::field(&skipped.join("; "), crate::plain::MESSAGE_MAX_CHARS)
    )))
}

impl EngineRig {
    pub fn open(spec: &RigSpec, profiles_dir: Option<&Path>, sim_cfg: &SimConfig, clock: Arc<dyn Clock>) -> Result<Self, HwError> {
        // The hardware gate comes first: without `hw` nothing past this line runs for a real device.
        let usb = if spec.device.sim { None } else { Some(usb_provider()?) };
        let loaded = openlaser_format::machines::load_all(profiles_dir);
        Self::open_loaded(spec, usb, loaded, profiles_dir, sim_cfg, clock)
    }

    /// [`EngineRig::open`] after the hardware gate, with the profiles `format::machines::load_all` returned.
    fn open_loaded(
        spec: &RigSpec,
        usb: Option<Arc<dyn DeviceProvider>>,
        (mut machines, skipped): (Vec<MachineProfile>, Vec<String>),
        profiles_dir: Option<&Path>,
        sim_cfg: &SimConfig,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, HwError> {
        refuse_skipped_profiles(&skipped)?;
        let default = if spec.device.sim { DEFAULT_SIM_MACHINE } else { DEFAULT_USB_MACHINE };
        let machine = spec.device.machine.clone().unwrap_or_else(|| MachineId(default.to_owned()));
        let profile = machines.iter_mut().find(|m| m.id == machine).ok_or_else(|| {
            HwError::refused(RefusalCode::NoDevice, format!("no machine profile `{}` (built-ins: {DEFAULT_USB_MACHINE}, {DEFAULT_SIM_MACHINE})", machine.0))
        })?;
        if let Some(e) = spec.extent {
            widen_for_extent(profile, e)?;
        }
        let link_is_sim = matches!(&profile.kind, Kinematics::Galvo(g) if g.link == GalvoLink::Sim);
        let galvo = match &profile.kind {
            Kinematics::Galvo(g) => Some(g.clone()),
            Kinematics::Gantry(_) => None,
        };
        let (mut provider, sim): (Arc<dyn DeviceProvider>, Option<Arc<SimProvider>>) = match usb {
            None => {
                let s = Arc::new(SimProvider::new(sim_cfg.clone()));
                (Arc::clone(&s) as Arc<dyn DeviceProvider>, Some(s))
            }
            Some(_) if link_is_sim => return Err(HwError::Usage(format!("`{}` is a simulator profile: add --sim", machine.0))),
            Some(u) => (u, None),
        };
        let mut usb_trace = None;
        let trace = match &spec.device.trace {
            None => None,
            Some(path) if sim.is_some() => Some(TraceState {
                writer: TraceWriter::create(path).map_err(HwError::Failed)?,
                recorder: None,
                seen: 0,
                opened: false,
                last_pump: None,
                failed: None,
                reported: false,
            }),
            Some(path) => {
                let sink = crate::usb_trace::sink(path, Arc::clone(&clock)).map_err(HwError::Failed)?;
                usb_trace = Some(Arc::clone(&sink));
                provider = Arc::new(crate::usb_trace::TracingProvider::new(provider, sink));
                None
            }
        };
        let profiles: Option<Arc<dyn ProfileStore>> = profiles_dir.map(|d| Arc::new(DirProfileStore::new(d.to_path_buf())) as Arc<dyn ProfileStore>);
        let camera_store = openlaser_format::camera_cal::calibration_path()
            .map(|path| Arc::new(openlaser_format::camera_cal::DirCalibrationStore::new(path)) as Arc<dyn openlaser_engine::CameraCalibrationStore>);
        let session = Session::new(SessionConfig {
            origin: Origin::Human,
            window: false,
            devices: DeviceAccess::Provider(Arc::clone(&provider)),
            machines,
            profiles,
            camera_store,
            files: FsAuthority::TrustedLocal,
            clock: Arc::clone(&clock),
        });
        Ok(EngineRig { rotary: None, session, clock, machine, provider, sim, trace, usb_trace, galvo })
    }

    fn exec(&mut self, id: &str, params: Value) -> Result<Value, HwError> {
        self.session.execute_as(Origin::Human, id, params).map_err(engine)
    }

    /// Copies new FakeLmc wire events into the trace file (every 250 ms, and at the end).
    fn pump_trace(&mut self, closing: bool) {
        let (Some(t), Some(sim)) = (self.trace.as_mut(), self.sim.as_ref()) else { return };
        let now = self.clock.now();
        if !closing && t.last_pump.is_some_and(|p| now.saturating_since(p) < TRACE_PUMP_EVERY) {
            return;
        }
        t.last_pump = Some(now);
        let write = |t: &mut TraceState, ev: &WireEvent, at: MonoTime| {
            if t.failed.is_none()
                && let Err(e) = t.writer.write(ev, at)
            {
                t.failed = Some(e);
            }
        };
        if let Some(rec) = sim.last_recorder() {
            if !t.recorder.as_ref().is_some_and(|r| Arc::ptr_eq(r, &rec)) {
                // A new connection: close the previous board's stream, start the new one.
                if t.opened {
                    write(t, &WireEvent::Close, now);
                    t.opened = false;
                }
                t.recorder = Some(Arc::clone(&rec));
                t.seen = 0;
            }
            let snap = rec.snapshot();
            for w in snap.wire.iter().skip(t.seen) {
                let (at, ev) = crate::trace::from_sim(w);
                if !t.opened {
                    write(t, &WireEvent::Open, at);
                    t.opened = true;
                }
                write(t, &ev, at);
            }
            t.seen = snap.wire.len();
        }
        if closing && t.opened {
            write(t, &WireEvent::Close, now);
            t.opened = false;
        }
    }

    /// The first trace failure, once (the flows print it; the machine is not stopped for a record).
    fn trace_failure(&mut self) -> Option<String> {
        if let Some(sink) = &self.usb_trace {
            return crate::usb_trace::take_failure(sink);
        }
        let t = self.trace.as_mut()?;
        if t.reported {
            return None;
        }
        let f = t.failed.clone()?;
        t.reported = true;
        Some(f)
    }
}

struct RotaryStop(openlaser_driver::Stopper, MachineId);
impl StopTarget for RotaryStop {
    fn stop_now(&self) {
        self.0.stop();
    }
    fn stop_and_wait(&self, max: Duration) -> Vec<(MachineId, openlaser_driver::StopAck)> {
        vec![(self.1.clone(), self.0.stop_and_wait(max))]
    }
}

impl Rig for EngineRig {
    fn prepare_rotary(&mut self) -> Result<(), HwError> {
        let g = self.galvo.as_ref().ok_or_else(|| HwError::refused(RefusalCode::WrongDeviceKind, "rotary requires an LMC galvo"))?;
        let r =
            g.rotary.as_ref().filter(|r| r.enabled).ok_or_else(|| {
                HwError::refused(RefusalCode::UnsupportedMode, "rotary.enabled is required; enable rotary in the human-only profile settings")
            })?;
        r.validate(g.work_area).map_err(HwError::Failed)?;
        let transport = self.provider.open_lmc(g).map_err(driver_error)?;
        self.rotary = Some(openlaser_driver::RotarySession::new(transport, g.clone(), self.clock.clone()).map_err(driver_error)?);
        Ok(())
    }
    fn rotary(&mut self, degrees: Option<f64>) -> Result<i32, HwError> {
        self.rotary.as_mut().ok_or_else(|| HwError::Failed("rotary session is not prepared".into()))?.execute(degrees).map_err(driver_error)
    }

    fn machine(&self) -> &MachineId {
        &self.machine
    }

    fn session(&self) -> SessionId {
        self.session.id()
    }

    fn now(&self) -> MonoTime {
        self.clock.now()
    }

    fn load(&mut self, job: &JobSource, layers: Option<&Path>, repeats: Option<u16>, power_pct: Option<f64>, extent: Option<f64>) -> Result<(), HwError> {
        match job {
            JobSource::Builtin(b) => {
                for (id, params) in b.pattern(extent).commands(b.name(), &self.machine.clone(), power_pct) {
                    self.exec(id, params)?;
                }
            }
            JobSource::Project(path) => match layers {
                None => {
                    self.exec("file.open", json!({ "path": path.to_string_lossy() }))?;
                }
                Some(l) => {
                    let staged = stage_om_workspace(path, l)?;
                    let r = self.exec("file.open", json!({ "path": staged.workspace.to_string_lossy() }));
                    drop(staged);
                    r?;
                }
            },
        }
        self.exec("device.select", json!({ "device": self.machine.0 }))?;
        if let Some(n) = repeats {
            self.exec("laser.repeat", json!({ "count": n }))?;
        }
        Ok(())
    }

    fn summary(&mut self) -> Result<EmissionSummary, HwError> {
        let opts = JobOptions { machine: Some(self.machine.clone()), selected_only: None };
        self.session.build_job(None, &opts).map(|b| b.summary).map_err(job_refused)
    }

    fn connect(&mut self) -> Result<(), HwError> {
        let m = self.machine.clone();
        self.session.connect(Origin::Human, &m).map_err(engine)
    }

    fn start_frame(&mut self, mode: FrameMode, style: Option<FrameStyle>, source: Option<FrameSource>) -> Result<Fingerprint, HwError> {
        self.session.start_frame(Origin::Human, mode, style, source).map_err(engine)
    }

    fn poll(&mut self) -> Vec<RigEvent> {
        let events = self.session.poll();
        self.pump_trace(false);
        let mut out = Vec::new();
        if let Some(f) = self.trace_failure() {
            out.push(RigEvent::Warning(format!("the trace file stopped recording: {f}")));
        }
        for ev in events {
            match ev {
                Event::Driver { machine, event } if machine == self.machine => match event {
                    DriverEvent::FrameCompleted(r) => out.push(RigEvent::FrameCompleted(r)),
                    DriverEvent::Progress { packets_acked, total, eta_ms } => out.push(RigEvent::Progress { packets_acked, total, eta_ms }),
                    DriverEvent::Finished(Outcome::Finished { duration }) => out.push(RigEvent::Finished(duration)),
                    DriverEvent::Finished(Outcome::Stopped(r)) | DriverEvent::Stopped(r) => out.push(RigEvent::Stopped(r)),
                    DriverEvent::Fault(e) => out.push(RigEvent::Fault(e.to_string())),
                    DriverEvent::Connected(_) | DriverEvent::State(_) | DriverEvent::Trace(_) => {}
                },
                Event::ArmVoided { reason } => out.push(RigEvent::ArmVoided(reason)),
                _ => {}
            }
        }
        out
    }

    fn stopper(&self) -> Arc<dyn StopTarget> {
        if let Some(r) = &self.rotary {
            return Arc::new(RotaryStop(r.stopper(), self.machine.clone()));
        }
        self.session.stop_registry()
    }

    fn request_arm(&mut self) -> Result<ArmView, HwError> {
        let p = self.session.request_arm(Origin::Human).map_err(engine)?;
        Ok(ArmView { summary: p.summary.clone(), receipt: p.receipt.clone(), checklist: p.checklist.clone() })
    }

    fn emission_cap_pct(&self) -> Result<Option<f64>, HwError> {
        self.provider.emission_cap_pct().map_err(|e| HwError::Failed(format!("cannot read the first-emission cap: {e}")))
    }

    fn run_armed(&mut self, token: ArmToken) -> Result<(), HwError> {
        self.session.run_armed(token).map_err(engine)
    }

    fn probe(&mut self) -> Result<Vec<String>, HwError> {
        // GetSerialNo + GetVersion on a freshly opened transport: no init, no Reset, nothing that moves or fires (H12).
        let Some(g) = self.galvo.clone() else {
            return Err(HwError::refused(RefusalCode::WrongDeviceKind, "`status` reads LMC (galvo) boards; this machine is a gantry"));
        };
        let mut t = self.provider.open_lmc(&g).map_err(driver_error)?;
        let mut lines = Vec::new();
        for (name, op) in [("GetSerialNo", PROBE_GET_SERIAL_NO), ("GetVersion", PROBE_GET_VERSION)] {
            t.submit_single(Single([op, 0, 0, 0, 0, 0])).map_err(|e| HwError::Failed(format!("{name}: {e}")))?;
            let mut polls = 0u32;
            let reply = loop {
                match t.poll_single(PROBE_SLICE).map_err(|e| HwError::Failed(format!("{name}: {e}")))? {
                    Some(r) => break r,
                    None => {
                        polls += 1;
                        if PROBE_SLICE * polls >= PROBE_BUDGET {
                            return Err(HwError::Failed(format!("{name}: no reply within {} ms (not retried)", PROBE_BUDGET.as_millis())));
                        }
                    }
                }
            };
            match reply {
                Some(w) => {
                    let mut line = format!("{name}: {:04x} {:04x} {:04x} {:04x}", w[0], w[1], w[2], w[3]);
                    if op == PROBE_GET_VERSION {
                        let ready = if w[3] & STATUS_READY != 0 { "READY" } else { "not READY" };
                        let busy = if w[3] & STATUS_BUSY != 0 { "BUSY" } else { "idle" };
                        line.push_str(&format!(" (status 0x{:04x}: {ready}, {busy})", w[3]));
                    }
                    lines.push(line);
                }
                None => lines.push(format!("{name}: no reply")),
            }
        }
        drop(t);
        if let Some(f) = self.trace_failure() {
            lines.push(format!("warning: the trace file stopped recording: {f}"));
        }
        Ok(lines)
    }

    fn disconnect(&mut self) {
        let m = self.machine.clone();
        let _ = self.session.disconnect(Origin::Human, &m);
        let _ = self.session.poll();
        self.pump_trace(true);
    }
}

/// `run <workspace.json> --layers <layer_settings.json>`: the engine reads `layer_settings.json` from the
/// workspace's folder (SPEC Appendix B), so both files are copied side by side into a private temporary folder.
struct Staged {
    dir: PathBuf,
    workspace: PathBuf,
}

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn stage_om_workspace(workspace: &Path, layers: &Path) -> Result<Staged, HwError> {
    if workspace.extension().and_then(|e| e.to_str()) != Some("json") {
        return Err(HwError::Usage("--layers applies to an OM workspace.json".to_owned()));
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("openlaser-hw-{}-{nanos}", std::process::id()));
    std::fs::create_dir(&dir).map_err(|e| HwError::Failed(format!("cannot stage the workspace: {e}")))?;
    let staged = Staged { workspace: dir.join("workspace.json"), dir };
    std::fs::copy(workspace, &staged.workspace).map_err(|e| HwError::Failed(format!("cannot read {}: {e}", workspace.display())))?;
    std::fs::copy(layers, staged.dir.join("layer_settings.json")).map_err(|e| HwError::Failed(format!("cannot read {}: {e}", layers.display())))?;
    Ok(staged)
}

#[cfg(test)]
mod tests {
    use openlaser_machine::{jpt_m7_200, jpt_m7_200_sim};

    use super::*;

    fn galvo(p: &MachineProfile) -> &openlaser_machine::GalvoMachine {
        match &p.kind {
            Kinematics::Galvo(g) => g,
            Kinematics::Gantry(_) => panic!("galvo profile"),
        }
    }

    #[test]
    fn a_skipped_profile_file_refuses_the_device_command() {
        assert!(refuse_skipped_profiles(&[]).is_ok());
        let err = refuse_skipped_profiles(&["machines/jpt-m7-200.json: cal.scale must be finite".to_owned()]).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("machines/jpt-m7-200.json: cal.scale must be finite"), "{text}");
        assert!(text.contains("nothing was opened"), "{text}");
        assert_eq!(err.exit_code(), crate::EXIT_FAIL);
    }

    #[test]
    fn open_refuses_while_a_profile_file_is_skipped() {
        let spec = RigSpec { purpose: Purpose::Frame, device: DeviceOpts { machine: None, sim: true, trace: None }, extent: None };
        let clock: Arc<dyn Clock> = Arc::new(openlaser_types::VirtualClock::new());
        let sim = crate::default_sim_config();
        let loaded = (vec![jpt_m7_200_sim()], vec!["machines/jpt-m7-200-sim.json: not valid JSON".to_owned()]);
        let err = EngineRig::open_loaded(&spec, None, loaded, None, &sim, Arc::clone(&clock)).err().unwrap();
        assert!(err.to_string().contains("machines/jpt-m7-200-sim.json: not valid JSON"), "{err}");
        // The same profiles with nothing skipped open (no device is touched by opening).
        let rig = EngineRig::open_loaded(&spec, None, (vec![jpt_m7_200_sim()], Vec::new()), None, &sim, clock).unwrap();
        assert_eq!(rig.machine(), &MachineId(DEFAULT_SIM_MACHINE.to_owned()));
    }

    /// The USB path (`status`, `--trace`) with the simulator standing in for the board: the provider is handed in
    /// as `usb`, so everything after the hardware gate runs exactly as it would on the real device.
    #[test]
    fn status_on_the_usb_path_reads_serial_and_version_and_traces_every_wire_event() {
        let dir = std::env::temp_dir().join(format!("openlaser-hw-usb-trace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("status.jsonl");
        let _ = std::fs::remove_file(&path);
        let spec = RigSpec {
            purpose: Purpose::Status,
            device: DeviceOpts { machine: Some(MachineId(DEFAULT_USB_MACHINE.to_owned())), sim: false, trace: Some(path.clone()) },
            extent: None,
        };
        let clock: Arc<dyn Clock> = Arc::new(openlaser_types::VirtualClock::new());
        let sim = crate::default_sim_config();
        let board: Arc<dyn DeviceProvider> = Arc::new(SimProvider::new(sim.clone()));
        let mut rig = EngineRig::open_loaded(&spec, Some(board), (vec![jpt_m7_200()], Vec::new()), None, &sim, clock).unwrap();
        let lines = rig.probe().unwrap();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("GetSerialNo: "), "{lines:?}");
        assert!(lines[1].starts_with("GetVersion: ") && lines[1].contains("READY"), "{lines:?}");
        let text = std::fs::read_to_string(&path).unwrap();
        let kinds: Vec<String> = text
            .lines()
            .map(|l| {
                let v: Value = serde_json::from_str(l).unwrap();
                format!("{} {}", v["kind"].as_str().unwrap(), v["op"].as_str().unwrap_or(""))
            })
            .collect();
        assert_eq!(kinds, ["open ", "single GetSerialNo", "read ", "single GetVersion", "read ", "close "]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extent_widens_the_frame_only_inside_the_encodable_square() {
        let mut p = jpt_m7_200();
        widen_for_extent(&mut p, 110.0).unwrap();
        let rect = FieldRect { min: [-110.0, -110.0], max: [110.0, 110.0] };
        assert_eq!(galvo(&p).work_area, rect);
        assert_eq!(galvo(&p).h27_cleared, Some(rect));
        // F = 200 mm × 1.1834: ±118.34 mm is the edge of the encodable square, never inside it.
        for bad in [118.34, 150.0, 0.0, -5.0, f64::NAN, f64::INFINITY] {
            let mut p = jpt_m7_200();
            let err = widen_for_extent(&mut p, bad).unwrap_err();
            assert_eq!(err.code(), Some(RefusalCode::NotEncodable), "{bad}");
            assert_eq!(p, jpt_m7_200(), "a refused extent changes nothing ({bad})");
        }
    }
}
