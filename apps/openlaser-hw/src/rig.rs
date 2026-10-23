//! The machine session behind the CLI: the engine's `Session` with `Origin::Human` (SPEC §2.3 rule 13: openlaser-hw
//! is a human surface), the injected `DirProfileStore` (S24), and a device provider: `SimProvider` for `--sim`, or
//! `transport::UsbProvider` in a build with feature `hw` (S4, S23). Without `hw`, every real-device path is refused
//! with `HardwareDisabled` before anything is opened.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use openlaser_engine::{
    ArmToken, DeviceAccess, DeviceProvider, DriverEvent, EmissionSummary, EngineError, Event, Fingerprint, FrameMode, FrameReceipt, FrameSource, FrameStyle,
    FsAuthority, JobOptions, MachineId, MachineProfile, MonoTime, Origin, Outcome, ProfileStore, Session, SessionConfig, SessionId, StopReport,
};
use openlaser_format::machines::DirProfileStore;
use openlaser_lmc_sim::{SimConfig, SimProvider};
use openlaser_machine::{FieldRect, GalvoLink, Kinematics};
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
            d.serial.as_deref().unwrap_or("(no serial)"),
            d.location,
            if d.supported { "" } else { " (not supported)" },
            d.note
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
    session: Session,
    clock: Arc<dyn Clock>,
    machine: MachineId,
    provider: Arc<dyn DeviceProvider>,
    sim: Option<Arc<SimProvider>>,
    trace: Option<TraceState>,
}

struct TraceState {
    writer: TraceWriter,
    /// Recorder events already written.
    seen: usize,
    opened: bool,
}

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

impl EngineRig {
    pub fn open(spec: &RigSpec, profiles_dir: Option<&Path>, sim_cfg: &SimConfig, clock: Arc<dyn Clock>) -> Result<Self, HwError> {
        // The hardware gate comes first: without `hw` nothing past this line runs for a real device.
        let usb = if spec.device.sim { None } else { Some(usb_provider()?) };
        let (mut machines, _skipped) = openlaser_format::machines::load_all(profiles_dir);
        let default = if spec.device.sim { DEFAULT_SIM_MACHINE } else { DEFAULT_USB_MACHINE };
        let machine = spec.device.machine.clone().unwrap_or_else(|| MachineId(default.to_owned()));
        let profile = machines.iter_mut().find(|m| m.id == machine).ok_or_else(|| {
            HwError::refused(RefusalCode::NoDevice, format!("no machine profile `{}` (built-ins: {DEFAULT_USB_MACHINE}, {DEFAULT_SIM_MACHINE})", machine.0))
        })?;
        if let Some(e) = spec.extent {
            widen_for_extent(profile, e)?;
        }
        let link_is_sim = matches!(&profile.kind, Kinematics::Galvo(g) if g.link == GalvoLink::Sim);
        let (provider, sim): (Arc<dyn DeviceProvider>, Option<Arc<SimProvider>>) = match usb {
            None => {
                let s = Arc::new(SimProvider::new(sim_cfg.clone()));
                (Arc::clone(&s) as Arc<dyn DeviceProvider>, Some(s))
            }
            Some(_) if link_is_sim => return Err(HwError::Usage(format!("`{}` is a simulator profile: add --sim", machine.0))),
            Some(u) => (u, None),
        };
        let trace = match &spec.device.trace {
            None => None,
            Some(path) if sim.is_some() => Some(TraceState { writer: TraceWriter::create(path).map_err(HwError::Failed)?, seen: 0, opened: false }),
            Some(_) => {
                return Err(HwError::Failed(
                    "--trace on the real USB device needs openlaser-driver to re-export openlaser_lmc::{Single, Packet} so the transport can be \
                     wrapped (integrator request); nothing was opened. --trace works with --sim"
                        .to_owned(),
                ));
            }
        };
        let profiles: Option<Arc<dyn ProfileStore>> = profiles_dir.map(|d| Arc::new(DirProfileStore::new(d.to_path_buf())) as Arc<dyn ProfileStore>);
        let session = Session::new(SessionConfig {
            origin: Origin::Human,
            window: false,
            devices: DeviceAccess::Provider(Arc::clone(&provider)),
            machines,
            profiles,
            files: FsAuthority::TrustedLocal,
            clock: Arc::clone(&clock),
        });
        Ok(EngineRig { session, clock, machine, provider, sim, trace })
    }

    fn exec(&mut self, id: &str, params: Value) -> Result<Value, HwError> {
        self.session.execute_as(Origin::Human, id, params).map_err(engine)
    }

    /// Copies new FakeLmc wire events into the trace file.
    fn pump_trace(&mut self, closing: bool) {
        let (Some(t), Some(sim)) = (self.trace.as_mut(), self.sim.as_ref()) else { return };
        let now = self.clock.now();
        if let Some(rec) = sim.last_recorder() {
            let snap = rec.snapshot();
            if let Some(first) = snap.wire.first()
                && !t.opened
            {
                let (at, _) = crate::trace::from_sim(first);
                let _ = t.writer.write(&WireEvent::Open, at);
                t.opened = true;
            }
            for w in snap.wire.iter().skip(t.seen) {
                let (at, ev) = crate::trace::from_sim(w);
                let _ = t.writer.write(&ev, at);
            }
            t.seen = snap.wire.len();
            if closing && t.opened {
                let at = snap.wire.last().map(|w| crate::trace::from_sim(w).0).unwrap_or(now);
                let _ = t.writer.write(&WireEvent::Close, at);
                t.opened = false;
            }
        }
    }
}

impl Rig for EngineRig {
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
        // GetSerialNo/GetVersion without init needs a transport the CLI can drive directly; LmcTransport takes
        // `openlaser_lmc::Single`, which no crate this binary may depend on re-exports yet (integrator request).
        Err(HwError::Failed(
            "`status` needs openlaser-driver to re-export openlaser_lmc::Single (integrator request); nothing was sent. Use `detect` for the \
             descriptors"
                .to_owned(),
        ))
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
