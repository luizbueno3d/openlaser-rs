//! The driver thread and its handle; device providers.

use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use openlaser_lmc::LmcTransport;
use openlaser_machine::{GalvoMachine, GantryMachine};
use openlaser_types::{ArmToken, Clock, FrameMode, FrameOrigin, MachineId, SessionId};
use serde::Serialize;

use crate::backend::{Backend, Capabilities, DriverError, DriverEvent};
use crate::grbl::{ByteLink, GrblBackend};
use crate::lmc_backend::LmcBackend;
use crate::stop::{StopLatch, Stopper};

/// What the handle sends the driver thread.
// The WP0 stub has no driver thread to read these yet (WP5).
#[allow(dead_code)]
enum Control<B: Backend> {
    Connect,
    Frame(B::Frame, FrameMode, FrameOrigin),
    Run(B::Job, ArmToken),
    Disconnect,
}

/// Starts the driver thread for one machine. WP0 stub: no thread yet; every queued request is dropped (nothing
/// can reach a device), and the stopper still latches.
pub fn spawn<B: Backend>(backend: B, machine: MachineId, session: SessionId, clock: Arc<dyn Clock>) -> DriverHandle<B> {
    let caps = backend.capabilities();
    let _ = (backend, machine, session, clock);
    let (ctl, ctl_rx) = crossbeam_channel::unbounded::<Control<B>>();
    drop(ctl_rx);
    let (wake, _wake_rx) = crossbeam_channel::bounded::<()>(1);
    let (_events_tx, events) = crossbeam_channel::unbounded::<DriverEvent>();
    let stopper = Stopper::new(Arc::new(StopLatch::new(wake)));
    DriverHandle { ctl, stopper, events, caps }
}

pub struct DriverHandle<B: Backend> {
    ctl: Sender<Control<B>>,
    stopper: Stopper,
    events: Receiver<DriverEvent>,
    caps: Capabilities,
}

impl<B: Backend> DriverHandle<B> {
    /// Every command records `latch.generation()` when queued; the driver thread refuses to start it
    /// (`Refused(StoppedAfterQueue)`) if a trip happened after that, and only then clears the latch for it. A stop
    /// pressed between "Confirm" and the driver dequeuing the run therefore cancels the run instead of being cleared.
    pub fn connect(&self) {
        let _ = self.ctl.send(Control::Connect);
    }

    /// Refused with `MachineBusy` by the driver thread itself while another frame or run is active (defence in
    /// depth behind the engine's pipeline step 7).
    pub fn frame(&self, f: B::Frame, mode: FrameMode, origin: FrameOrigin) {
        let _ = self.ctl.send(Control::Frame(f, mode, origin));
    }

    /// Queues the job. The driver thread re-checks: `token.check(job.fingerprint(), &job.bytes_hash(), machine,
    /// session, now)` with the hash RECOMPUTED from the job object (S22); nonce unused (then spent); its OWN last
    /// FrameReceipt has the same fingerprint, ≥ 1 full (executed) pass, ≤ 10 min, origin Human, covers job; no
    /// frame or run active (`MachineBusy`).
    pub fn run(&self, job: B::Job, token: ArmToken) {
        let _ = self.ctl.send(Control::Run(job, token));
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
        let _ = self.ctl.send(Control::Disconnect);
    }
}
// Drop of the last DriverHandle = dead-man: it trips the latch, then JOINS the driver thread for ≤ 500 ms so the
// abort reaches the wire before a panicking or exiting UI thread can end the process (WP5).

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
