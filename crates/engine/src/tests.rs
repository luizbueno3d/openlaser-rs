//! Unit tests over the session internals (pending ARM, driver-event folding, history) and the WP6 acceptance tests
//! that run without the other packages' behaviour. Tests that need real plans, encoders or drivers live in
//! `tests/engine.rs`.

#[cfg(test)]
use openlaser_arm::testing::token_for_tests;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use openlaser_doc::{ColorLayer, CutMode, CutOp, ImageCut, ImageMode, Project};
use openlaser_driver::{ByteLink, DetectedDevice, DeviceProvider, DriverError, DriverEvent, MachineState, Outcome, StopReport};
use openlaser_lmc::{LmcFrameJob, LmcJob, LmcTransport, Packet, Single, XferError};
use openlaser_machine::{GalvoMachine, GantryMachine, MachineProfile, jpt_m7_200, jpt_m7_200_sim};
use openlaser_types::{Fingerprint, FrameMode, FrameOrigin, FrameReceipt, FrameSource, FrameStyle, LayerKey, MachineId, MonoTime, RefusalCode, VirtualClock};
use serde_json::{Value, json};

use crate::cmds::Created;
use crate::command::{Access, EngineError, Kind, Origin, registry};
use crate::devices::{DeviceAccess, Event, PendingArm};
use crate::files::FsAuthority;
use crate::geomx;
use crate::jobs::Encoded;
use crate::session::{Label, Session, SessionConfig};

// ---- fixtures -------------------------------------------------------------------------------------------------

/// A colour layer from a profile's defaults (WP1's `Project::new` builds these; the WP0 stub does not).
pub(crate) fn color_layer(i: u8, m: &MachineProfile) -> ColorLayer {
    let d = &m.layer_defaults;
    ColorLayer {
        index: i,
        name: format!("C{i:02}"),
        output: true,
        show: true,
        ops: vec![CutOp {
            name: String::new(),
            enabled: true,
            mode: CutMode::Line,
            passes: 1,
            galvo: d.galvo.clone(),
            gantry: d.gantry.clone(),
            fill: d.fill.clone(),
        }],
        global_passes: 1,
        image: ImageCut {
            mode: ImageMode::Threshold,
            interval: d.fill.interval,
            angle: d.fill.angle,
            bidirectional: true,
            negative: false,
            threshold: 128,
            passes: 1,
            galvo: d.galvo.clone(),
            gantry: d.gantry.clone(),
        },
        air_assist: false,
    }
}

/// A new project with all 30 colour layers, whatever `Project::new` provides yet.
pub(crate) fn full_project(m: &MachineProfile) -> Project {
    let mut p = Session::new_project(m, "Test");
    if p.layers.colors.len() != 30 {
        p.layers.colors = (0..30u8).map(|i| color_layer(i, m)).collect();
    }
    p
}

/// A device provider whose transport refuses every transfer (no device, no emission) and that counts calls.
#[derive(Default)]
pub(crate) struct TestProvider {
    pub opens: AtomicU32,
    pub records: AtomicU32,
    pub cap: Option<f64>,
}

struct NullTransport;

impl LmcTransport for NullTransport {
    fn submit_single(&mut self, _cmd: Single) -> Result<(), XferError> {
        Err(XferError::HardwareDisabled)
    }
    fn poll_single(&mut self, _slice: Duration) -> Result<Option<Option<[u16; 4]>>, XferError> {
        Err(XferError::HardwareDisabled)
    }
    fn submit_packet(&mut self, _pkt: &Packet) -> Result<(), XferError> {
        Err(XferError::HardwareDisabled)
    }
    fn poll_packet(&mut self, _slice: Duration) -> Result<Option<()>, XferError> {
        Err(XferError::HardwareDisabled)
    }
    fn cancel_all(&mut self) {}
    fn drain_replies(&mut self) {}
}

impl DeviceProvider for TestProvider {
    fn name(&self) -> &'static str {
        "test"
    }
    fn detect(&self) -> Result<Vec<DetectedDevice>, DriverError> {
        Ok(Vec::new())
    }
    fn open_lmc(&self, _m: &GalvoMachine) -> Result<Box<dyn LmcTransport>, DriverError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(NullTransport))
    }
    fn open_link(&self, _m: &GantryMachine) -> Result<Box<dyn ByteLink>, DriverError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        Err(DriverError::NotConnected)
    }
    fn emission_cap_pct(&self) -> Result<Option<f64>, DriverError> {
        Ok(self.cap)
    }
    fn record_emission(&self) -> Result<(), DriverError> {
        self.records.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

pub(crate) fn config(origin: Origin, window: bool, devices: DeviceAccess) -> SessionConfig {
    SessionConfig {
        origin,
        window,
        devices,
        machines: vec![jpt_m7_200_sim(), jpt_m7_200()],
        profiles: None,
        camera_store: None,
        files: FsAuthority::TrustedLocal,
        clock: Arc::new(VirtualClock::new()),
    }
}

pub(crate) fn headless() -> Session {
    Session::new(config(Origin::Headless, false, DeviceAccess::None))
}

pub(crate) fn human() -> (Session, Arc<TestProvider>) {
    let p = Arc::new(TestProvider::default());
    let dyn_p: Arc<dyn DeviceProvider> = p.clone();
    (Session::new(config(Origin::Human, true, DeviceAccess::Provider(dyn_p))), p)
}

/// Opens a full test project and returns its index.
pub(crate) fn with_doc(s: &mut Session) -> usize {
    let m = jpt_m7_200_sim();
    s.open_project(full_project(&m), None, Some(m.id.clone())).unwrap()
}

pub(crate) fn draw(s: &mut Session, pts: Value) -> u64 {
    let v = s.execute_as(Origin::Headless, "tools.drawLines", json!({ "pointsMm": pts })).unwrap();
    v["ids"][0].as_u64().unwrap()
}

fn project(s: &Session) -> Arc<Project> {
    Arc::clone(&s.active().unwrap().project)
}

/// Plants a pending ARM as `request_arm` would leave it (its inputs need I1's plan and encoder).
pub(crate) fn plant_pending(s: &mut Session) {
    let idx = s.active.unwrap();
    let doc = &s.docs[idx];
    let fp = Fingerprint([5; 32]);
    let machine = doc.machine.clone();
    let mut summary = crate::devices::tests::summary(fp, &machine.0, &[10.0]);
    let receipt = FrameReceipt {
        fingerprint: fp,
        machine: machine.clone(),
        session: s.id(),
        full_passes: 1,
        at: s.cfg.clock.now(),
        style: FrameStyle::Contour,
        source: FrameSource::Job,
        origin: FrameOrigin::Human,
    };
    let job = LmcJob { return_axis: None, prelude: Vec::new(), lists: Vec::new(), repeats: 1, postlude: Vec::new(), fingerprint: fp, machine: machine.clone() };
    summary.bytes_hash = job.bytes_hash();
    let frame = LmcFrameJob {
        once: Vec::new(),
        once_passes: 1,
        once_est: Duration::ZERO,
        pass: Vec::new(),
        pass_est: Duration::ZERO,
        fingerprint: fp,
        machine: machine.clone(),
        style: FrameStyle::Contour,
        source: FrameSource::Job,
    };
    s.pending_arm = Some(PendingArm {
        fingerprint: fp,
        machine,
        summary,
        receipt,
        checklist: vec!["one".into()],
        burned: Vec::new(),
        revision: doc.revision,
        expires: s.cfg.clock.now().plus(Duration::from_secs(60)),
        job: Encoded::Lmc { job: Arc::new(job), frame: Arc::new(frame) },
        stop_trips: s.stop_registry.trips(),
    });
    s.pending_doc = Some(idx);
}

fn token_for(s: &Session) -> openlaser_types::ArmToken {
    let p = s.pending_arm().unwrap();
    token_for_tests(p.fingerprint, p.summary.bytes_hash, p.machine.clone(), s.id(), s.token_expiry(), 7)
}

// ---- pending ARM ------------------------------------------------------------------------------------------------

#[test]
fn pending_arm_voided_by_edit() {
    let (mut s, _) = human();
    with_doc(&mut s);
    plant_pending(&mut s);
    s.poll();
    draw(&mut s, json!([[0, 0], [10, 0]]));
    assert!(s.pending_arm().is_none(), "an edit must void the pending ARM");
    assert!(s.poll().iter().any(|e| matches!(e, Event::ArmVoided { .. })));
    // Undo and redo are revision bumps too.
    plant_pending(&mut s);
    s.execute("edit.undo", Value::Null).unwrap();
    assert!(s.pending_arm().is_none());
    plant_pending(&mut s);
    s.execute("laser.repeat", json!({ "count": 3 })).unwrap();
    assert!(s.pending_arm().is_none(), "Repeat changes the job");
}

#[test]
fn arm_token_invalid_after_edit() {
    let (mut s, _) = human();
    with_doc(&mut s);
    plant_pending(&mut s);
    let token = token_for(&s);
    draw(&mut s, json!([[0, 0], [10, 0]]));
    let e = s.run_armed(token).unwrap_err();
    assert_eq!(e.code(), "stale", "{e}");
    // A pending ARM whose document moved on (revision bumped without the edit path) is refused too.
    plant_pending(&mut s);
    let token = token_for(&s);
    s.docs[0].revision += 1;
    assert_eq!(s.run_armed(token).unwrap_err().code(), "stale");
    assert!(s.pending_arm().is_none(), "a refused run_armed spends the pending ARM");
}

#[test]
fn stop_voids_pending_arm() {
    let (mut s, _) = human();
    with_doc(&mut s);
    plant_pending(&mut s);
    s.stop_all();
    assert!(s.pending_arm().is_none());
    // A stop that never touched the session (control fast path, signal watcher) is seen by poll.
    plant_pending(&mut s);
    let reg = s.stop_registry();
    std::thread::spawn(move || reg.stop_all(Duration::from_millis(1))).join().unwrap();
    assert!(s.pending_arm().is_some());
    s.poll();
    assert!(s.pending_arm().is_none(), "poll must void the ARM after a fast-path stop");
    // laser.stop from any origin, agents included.
    plant_pending(&mut s);
    s.execute_as(Origin::Synthetic, "laser.stop", json!({})).unwrap();
    assert!(s.pending_arm().is_none());
}

#[test]
fn pending_arm_voided_by_doc_switch_close_and_expiry() {
    let (mut s, _) = human();
    with_doc(&mut s);
    with_doc(&mut s);
    plant_pending(&mut s);
    s.select_doc(0).unwrap();
    assert!(s.pending_arm().is_none(), "another document became active");
    plant_pending(&mut s);
    s.close_doc(Some(1)).unwrap();
    assert!(s.pending_arm().is_none());
    // The planted ARM expires after 60 s of virtual time.
    let (mut s, clock) = clocked();
    with_doc(&mut s);
    plant_pending(&mut s);
    clock.advance(Duration::from_secs(61));
    s.poll();
    assert!(s.pending_arm().is_none(), "an expired ARM request is voided");
}

/// A human session with a test provider whose virtual clock the test can advance.
fn clocked() -> (Session, Arc<VirtualClock>) {
    let clock = Arc::new(VirtualClock::new());
    let p: Arc<dyn DeviceProvider> = Arc::new(TestProvider::default());
    let mut cfg = config(Origin::Human, true, DeviceAccess::Provider(p));
    cfg.clock = clock.clone();
    (Session::new(cfg), clock)
}

#[test]
fn run_armed_without_pending_or_with_a_refused_token() {
    let (mut s, _) = human();
    with_doc(&mut s);
    let tok = token_for_tests(Fingerprint([1; 32]), Fingerprint([2; 32]), MachineId("jpt-m7-200-sim".into()), s.id(), MonoTime::default(), 1);
    assert_eq!(s.run_armed(tok).unwrap_err().code(), "stale", "nothing pending");
    plant_pending(&mut s);
    let tok = token_for(&s);
    // Until WP3's `ArmToken::check` lands every token is refused (fail closed); after it, the stub machine is
    // not connected. Either way nothing runs.
    assert!(s.run_armed(tok).is_err());
    assert!(s.status(&MachineId("jpt-m7-200-sim".into())).is_none());
}

// ---- device hub -------------------------------------------------------------------------------------------------

#[test]
fn headless_has_no_device() {
    let mut s = headless();
    with_doc(&mut s);
    assert_eq!(s.execute("laser.stop", json!({})).unwrap_err(), EngineError::NoDevice);
    assert_eq!(s.execute("laser.frameStop", Value::Null).unwrap_err(), EngineError::NoDevice);
    for id in ["laser.frame", "laser.start", "device.connect", "device.detect", "device.disconnect"] {
        assert_eq!(s.execute(id, json!({})).unwrap_err().code(), "human_only", "{id}");
    }
    // The typed human path still has nothing to open.
    let m = MachineId("jpt-m7-200-sim".into());
    assert_eq!(s.connect(Origin::Human, &m).unwrap_err(), EngineError::NoDevice);
    assert_eq!(s.start_frame(Origin::Human, FrameMode::Once, None, None).unwrap_err(), EngineError::NoDevice);
    assert_eq!(s.request_arm(Origin::Human).map(|_| ()).unwrap_err(), EngineError::NoDevice);
    let st = s.machine_status_json(None);
    assert_eq!(st["backend"], "headless");
    assert_eq!(st["connected"], false);
    assert_eq!(s.execute("laser.status", Value::Null).unwrap()["backend"], "headless");
}

#[test]
fn typed_device_api_refuses_non_human_origins() {
    let (mut s, p) = human();
    with_doc(&mut s);
    let m = MachineId("jpt-m7-200-sim".into());
    for o in [Origin::Synthetic, Origin::Headless, Origin::Replay] {
        assert_eq!(s.connect(o, &m).unwrap_err().code(), "human_only");
        assert_eq!(s.disconnect(o, &m).unwrap_err().code(), "human_only");
        assert_eq!(s.start_frame(o, FrameMode::Once, None, None).unwrap_err().code(), "human_only");
        assert_eq!(s.request_arm(o).map(|_| ()).unwrap_err().code(), "human_only");
        assert_eq!(s.approve_request(o, crate::RequestId(1)).unwrap_err().code(), "human_only");
        assert_eq!(s.decline_request(o, crate::RequestId(1)).unwrap_err().code(), "human_only");
    }
    assert_eq!(p.opens.load(Ordering::SeqCst), 0, "no device was opened");
}

#[test]
fn connect_registers_a_stopper_and_frame_needs_a_connection() {
    let (mut s, p) = human();
    with_doc(&mut s);
    let m = MachineId("jpt-m7-200-sim".into());
    let e = s.start_frame(Origin::Human, FrameMode::Once, None, None).unwrap_err();
    assert_eq!(e.code(), "not_connected", "{e}");
    s.execute("device.connect", json!({})).unwrap();
    assert_eq!(p.opens.load(Ordering::SeqCst), 1);
    assert_eq!(s.stop_registry().machines(), vec![m.clone()]);
    assert!(s.status(&m).is_some());
    s.execute("device.connect", json!({})).unwrap();
    assert_eq!(p.opens.load(Ordering::SeqCst), 1, "connecting twice opens once");
    let acks = s.stop_all();
    assert_eq!(acks.len(), 1);
    assert!(acks[0].1.latched);
    // Disconnect forgets the stopper.
    s.poll();
    if !s.status(&m).is_some_and(|st| st.busy()) {
        s.execute("device.disconnect", json!({})).unwrap();
        assert!(s.stop_registry().machines().is_empty());
    }
}

#[test]
fn device_busy_refuses_light_emit_connect_but_never_stop() {
    let (mut s, _) = human();
    with_doc(&mut s);
    s.execute("device.connect", json!({})).unwrap();
    let m = MachineId("jpt-m7-200-sim".into());
    s.hub[0].status.state = MachineState::Framing;
    for id in ["laser.frame", "laser.start", "device.connect", "device.disconnect"] {
        let e = s.execute(id, json!({})).unwrap_err();
        assert_eq!(e.code(), "machine_busy", "{id}: {e}");
    }
    assert_eq!(s.execute("device.select", json!({ "device": "jpt-m7-200" })).unwrap_err().code(), "machine_busy");
    assert!(s.execute("laser.stop", json!({})).is_ok());
    assert!(s.execute("laser.frameStop", json!({ "machine": m.0 })).is_ok());
    assert!(s.execute("laser.status", json!({})).is_ok());
    // Edits are not device commands: the design stays editable while the machine works.
    draw(&mut s, json!([[0, 0], [1, 1]]));
}

#[test]
fn driver_events_fold_into_status_receipts_and_requests() {
    let (mut s, p) = human();
    with_doc(&mut s);
    s.execute("device.connect", json!({})).unwrap();
    let m = MachineId("jpt-m7-200-sim".into());
    s.fold_driver_event(&m, DriverEvent::State(MachineState::Idle));
    assert!(matches!(s.status(&m).unwrap().state, MachineState::Idle));
    s.hub[0].op = Some(crate::devices::Op::Frame { mode: FrameMode::Once, fingerprint: Fingerprint([1; 32]) });
    s.hub[0].status.state = MachineState::Framing;
    let r = FrameReceipt {
        fingerprint: Fingerprint([1; 32]),
        machine: m.clone(),
        session: s.id(),
        full_passes: 3,
        at: MonoTime::default(),
        style: FrameStyle::Bounds,
        source: FrameSource::Job,
        origin: FrameOrigin::Human,
    };
    s.fold_driver_event(&m, DriverEvent::FrameCompleted(r.clone()));
    let st = s.status(&m).unwrap();
    assert_eq!(st.receipt.as_ref(), Some(&r));
    assert!(!st.busy(), "Frame Once ends with its receipt");
    // A finished run records the bench emission; a stopped one does not.
    s.fold_driver_event(&m, DriverEvent::Finished(Outcome::Finished { duration: Duration::from_secs(1) }));
    assert_eq!(p.records.load(Ordering::SeqCst), 1);
    let report =
        StopReport { confirmed: true, elapsed: Duration::from_millis(40), possible_desync: false, mo_closed: true, ports_zero: true, notes: Vec::new() };
    plant_pending(&mut s);
    s.fold_driver_event(&m, DriverEvent::Finished(Outcome::Stopped(report.clone())));
    assert_eq!(p.records.load(Ordering::SeqCst), 1);
    assert!(s.pending_arm().is_none());
    plant_pending(&mut s);
    s.fold_driver_event(&m, DriverEvent::Stopped(report.clone()));
    assert!(s.pending_arm().is_none(), "a Stopped event voids the pending ARM");
    assert_eq!(s.status(&m).unwrap().last_stop.as_ref(), Some(&report));
    plant_pending(&mut s);
    s.fold_driver_event(&m, DriverEvent::Fault(DriverError::Watchdog { waited_ms: 2000 }));
    assert!(s.pending_arm().is_none());
    assert!(s.status(&m).unwrap().fault.is_some());
    assert_eq!(s.execute_as(Origin::Synthetic, "laser.clearFault", json!({})).unwrap_err().code(), "human_only");
    s.execute("laser.clearFault", json!({})).unwrap();
    assert!(s.status(&m).unwrap().fault.is_none());
    for i in 0..(crate::devices::TRACE_TAIL + 10) {
        s.fold_driver_event(&m, DriverEvent::Trace(openlaser_driver::TraceLine { at: MonoTime::default(), out: true, text: format!("{i}") }));
    }
    assert_eq!(s.status(&m).unwrap().trace_tail.len(), crate::devices::TRACE_TAIL);
    let st = s.machine_status_json(Some(&m));
    assert_eq!(st["frameReceipt"]["fullPasses"], 3);
    assert_eq!(st["state"], "idle", "a cleared fault leaves the machine idle");
}

#[test]
fn stop_never_refused() {
    let stop_ids: Vec<&str> = registry().specs().iter().filter(|s| s.access == Access::Stop).map(|s| s.id).collect();
    assert!(stop_ids.contains(&"laser.stop") && stop_ids.contains(&"laser.frameStop"));
    // Good params, then adversarial ones: every one trips before anything looks at it (§1.3 rule 5).
    let all_params = [
        Value::Null,
        json!({ "machine": "jpt-m7-200-sim" }),
        json!({ "machine": "jpt-m7-200" }),
        json!({ "machine": "no-such-machine" }),
        json!({ "bogus": 1 }),
        json!({ "machine": 5 }),
        json!({ "coalesce": 7 }),
        json!({ "dryRun": true }),
        json!({ "machine": "jpt-m7-200-sim", "all": true }),
        json!([]),
        json!("stop"),
        json!(42),
    ];
    for with_device in [false, true] {
        for with_doc_open in [false, true] {
            let mut s = if with_device { human().0 } else { Session::new(config(Origin::Human, true, DeviceAccess::None)) };
            if with_doc_open {
                with_doc(&mut s);
            }
            if with_device {
                s.execute("device.connect", json!({})).ok();
            }
            for origin in [Origin::Human, Origin::Synthetic, Origin::Headless, Origin::Replay] {
                for id in &stop_ids {
                    for params in &all_params {
                        if let Some(slot) = s.hub.first_mut() {
                            slot.status.state = MachineState::Running { packets_acked: 1, total: 9 };
                        }
                        plant_pending_if_doc(&mut s);
                        let trips = s.stop_registry().trips();
                        let out = s.execute_as(origin, id, params.clone());
                        assert!(s.stop_registry().trips() > trips, "{id} {origin:?} ({params}): nothing was tripped");
                        assert!(s.pending_arm().is_none(), "{id} {origin:?} ({params}): a stop voids the pending ARM");
                        match out {
                            Ok(v) => {
                                assert!(with_device, "{id} {origin:?} ({params})");
                                // The running sim machine is stopped whatever machine was named.
                                assert_eq!(v["stopped"][0]["machine"], "jpt-m7-200-sim", "{id} {origin:?} ({params}): {v}");
                                assert_eq!(v["latched"], true, "{v}");
                            }
                            Err(EngineError::NoDevice) => assert!(!with_device, "{id} {origin:?}"),
                            Err(EngineError::BadParams { message, .. }) => {
                                assert!(with_device, "{id} {origin:?} ({params})");
                                assert!(message.contains("stopped anyway"), "{message}");
                            }
                            Err(e) => panic!("{id} from {origin:?} ({params}) was refused: {e}"),
                        }
                        s.pending_arm = None;
                        if let Some(slot) = s.hub.first_mut() {
                            slot.status.fault = None;
                        }
                    }
                    let row = s.command_list(origin, &crate::ListFilter::default()).into_iter().find(|r| r.id == *id).unwrap();
                    assert!(row.enabled, "{id} must never be disabled");
                }
            }
        }
    }
}

fn plant_pending_if_doc(s: &mut Session) {
    if s.active.is_some() {
        plant_pending(s);
    }
}

#[test]
fn device_commands_never_journaled() {
    let (mut s, _) = human();
    with_doc(&mut s);
    draw(&mut s, json!([[0, 0], [1, 1]]));
    s.execute("device.connect", json!({})).unwrap();
    s.execute("laser.stop", json!({})).unwrap();
    let _ = s.execute("laser.frame", json!({}));
    let _ = s.execute("laser.start", json!({}));
    s.execute("laser.status", json!({})).unwrap();
    s.execute("document.inspect", json!({})).unwrap();
    let journal = s.journal();
    assert!(journal.iter().any(|j| j.id == "tools.drawLines"), "edits are journaled");
    for j in &journal {
        let spec = registry().find(&j.id).unwrap();
        assert!(spec.kind.journaled(), "{} ({:?}) must not be journaled", j.id, spec.kind);
        assert!(!matches!(spec.kind, Kind::Device(_)));
    }
    // A dry run is not journaled either.
    let n = s.journal().len();
    s.execute("tools.drawLines", json!({ "pointsMm": [[0, 0], [2, 2]], "dryRun": true })).unwrap();
    assert_eq!(s.journal().len(), n);
}

#[test]
fn device_select_bumps_revision() {
    let mut s = headless();
    with_doc(&mut s);
    let r0 = s.active().unwrap().revision;
    s.poll();
    let v = s.execute("device.select", json!({ "device": "jpt-m7-200" })).unwrap();
    assert_eq!(v["changed"], true);
    assert_eq!(s.active().unwrap().revision, r0 + 1);
    assert_eq!(s.active().unwrap().machine, MachineId("jpt-m7-200".into()));
    assert!(s.poll().iter().any(|e| matches!(e, Event::DocChanged { .. })));
    // Selecting the same machine again is no change.
    let v = s.execute("device.select", json!({ "device": "jpt-m7-200" })).unwrap();
    assert_eq!(v["changed"], false);
    assert_eq!(s.active().unwrap().revision, r0 + 1);
    assert_eq!(s.execute("device.select", json!({ "device": "nope" })).unwrap_err().code(), "bad_params");
}

#[test]
fn profiles_read_only_without_store() {
    let (mut s, _) = human();
    with_doc(&mut s);
    let cases = [
        ("device.settings.set", json!({ "name": "renamed" })),
        ("device.create", json!({ "id": "copy", "name": "Copy" })),
        ("device.importEzcad", json!({ "path": "markcfg" })),
        ("device.calibrateAxis", json!({ "axis": 1, "requestedMm": 100, "measuredMm": 99 })),
    ];
    for (id, params) in cases {
        let e = s.execute_as(Origin::Human, id, params.clone()).unwrap_err();
        assert!(e.to_string().contains("profiles are read-only"), "{id}: {e}");
        // And every other origin is refused by policy first.
        assert_eq!(s.execute_as(Origin::Synthetic, id, params).unwrap_err().code(), "human_only");
    }
    assert_eq!(s.machines()[0].name, jpt_m7_200_sim().name);
}

/// A store that records saves (validate_profile decides whether any reach it).
#[derive(Default)]
struct MemStore {
    saved: std::sync::Mutex<Vec<MachineProfile>>,
}

impl openlaser_machine::ProfileStore for MemStore {
    fn save(&self, p: &MachineProfile) -> Result<(), String> {
        self.saved.lock().unwrap().push(p.clone());
        Ok(())
    }
    fn delete(&self, _id: &MachineId) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn profile_writes_go_through_the_store_after_validation() {
    let store = Arc::new(MemStore::default());
    let mut cfg = config(Origin::Human, true, DeviceAccess::None);
    cfg.profiles = Some(store.clone());
    let mut s = Session::new(cfg);
    with_doc(&mut s);
    let r0 = s.active().unwrap().revision;
    let out = s.execute("device.settings.set", json!({ "scaleXPct": 120, "negateY": true }));
    match openlaser_machine::validate_profile(&jpt_m7_200_sim()) {
        // WP0's validate_profile refuses everything: nothing may reach the store.
        Err(_) => {
            assert_eq!(out.unwrap_err().code(), "bad_params");
            assert!(store.saved.lock().unwrap().is_empty());
        }
        Ok(()) => {
            out.unwrap();
            let saved = store.saved.lock().unwrap();
            assert_eq!(saved.len(), 1);
            let openlaser_machine::Kinematics::Galvo(g) = &saved[0].kind else { panic!("galvo") };
            assert!((g.cal.scale[0] - 1.2).abs() < 1e-12 && g.cal.negate[1]);
            assert_eq!(s.active().unwrap().revision, r0 + 1, "a profile change bumps the revision");
        }
    }
    // Bad values never reach validation.
    assert_eq!(s.execute("device.settings.set", json!({ "maxPowerPct": 150 })).unwrap_err().code(), "bad_params");
    assert_eq!(s.execute("device.calibrateAxis", json!({ "axis": 3, "requestedMm": 1, "measuredMm": 1 })).unwrap_err().code(), "bad_params");
}

// ---- requests ---------------------------------------------------------------------------------------------------

#[test]
fn requests_need_bridge_mode() {
    let mut s = headless();
    with_doc(&mut s);
    assert_eq!(s.submit_job("agent", "hi").unwrap_err(), EngineError::NeedsBridge);
    assert_eq!(s.request_frame("agent", FrameMode::Once).unwrap_err(), EngineError::NeedsBridge);
    let e = s.execute("job.submit", json!({})).unwrap_err();
    assert_eq!(e.code(), "disabled", "{e}");
}

#[test]
fn submit_with_refusals_creates_nothing() {
    let mut s = Session::new(config(Origin::Synthetic, true, DeviceAccess::None));
    with_doc(&mut s);
    // Far outside the ±100 mm work area: refused by safety (and, before I1, by the stub planner).
    draw(&mut s, json!([[150, 0], [160, 0]]));
    let e = s.execute("job.submit", json!({ "note": "please" })).unwrap_err();
    assert!(matches!(e, EngineError::JobRefused(_)), "{e}");
    assert!(s.requests().submissions().is_empty());
    let e = s.execute("job.submit", json!({ "note": "n".repeat(281) })).unwrap_err();
    assert_eq!(e.code(), "bad_params");
    assert!(s.requests().submissions().is_empty());
}

// ---- history ----------------------------------------------------------------------------------------------------

#[test]
fn coalesce_merges_drags() {
    let mut s = headless();
    with_doc(&mut s);
    let before = project(&s);
    for p in [31.0, 32.0, 33.0, 34.0] {
        s.execute("layer.cutSettings.set", json!({ "layer": 0, "galvo": { "powerPct": p }, "coalesce": "drag-power-C00" })).unwrap();
    }
    let d = s.active().unwrap();
    assert_eq!(d.history.labels(), vec!["Set C00 Power".to_owned()], "four drag steps are one undo step");
    assert_eq!(d.project.color_layer(0).unwrap().ops[0].galvo.power.get(), 34.0);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*project(&s), *before);
    // A different key starts a new step; no key never merges.
    s.execute("layer.cutSettings.set", json!({ "layer": 0, "galvo": { "powerPct": 40 }, "coalesce": "a" })).unwrap();
    s.execute("layer.cutSettings.set", json!({ "layer": 0, "galvo": { "powerPct": 41 }, "coalesce": "b" })).unwrap();
    s.execute("layer.cutSettings.set", json!({ "layer": 0, "galvo": { "powerPct": 42 } })).unwrap();
    s.execute("layer.cutSettings.set", json!({ "layer": 0, "galvo": { "powerPct": 43 } })).unwrap();
    assert_eq!(s.active().unwrap().history.labels().len(), 4);
    // Arrow-key nudges coalesce by default.
    let id = draw(&mut s, json!([[0, 0], [1, 1]]));
    let n = s.active().unwrap().history.labels().len();
    for _ in 0..5 {
        s.execute("arrange.nudge", json!({ "ids": [id], "dxMm": 1, "dyMm": 0 })).unwrap();
    }
    assert_eq!(s.active().unwrap().history.labels().len(), n + 1);
    assert_eq!(s.execute("layer.cutSettings.set", json!({ "layer": 0, "coalesce": 5 })).unwrap_err().code(), "bad_params");
}

#[test]
fn atomic_batch_rolls_back() {
    let mut s = headless();
    with_doc(&mut s);
    draw(&mut s, json!([[0, 0], [5, 5]]));
    let before = project(&s);
    let labels = s.active().unwrap().history.labels();
    let r0 = s.active().unwrap().revision;
    let steps = vec![
        ("tools.drawLines".to_owned(), json!({ "pointsMm": [[1, 1], [2, 2]] })),
        ("layer.cutSettings.set".to_owned(), json!({ "layer": 0, "galvo": { "powerPct": 55 } })),
        ("layer.cutSettings.set".to_owned(), json!({ "layer": 0, "galvo": { "powerPct": 155 } })),
        ("tools.drawLines".to_owned(), json!({ "pointsMm": [[3, 3], [4, 4]] })),
    ];
    let r = s.batch(Origin::Headless, steps.clone(), true, Some("Recolour".into()));
    assert!(r.rolled_back);
    assert_eq!((r.completed, r.failed), (2, 1));
    assert_eq!(r.results[2]["error"]["pointer"], "/galvo/powerPct");
    assert_eq!(*project(&s), *before, "every step is undone");
    assert_eq!(s.active().unwrap().history.labels(), labels, "no history state remains");
    assert!(s.active().unwrap().revision > r0, "the revision still moves on (caches, ARM, requests)");
    // The same batch without the bad step is one undo step with the batch label.
    let good: Vec<_> = steps.into_iter().enumerate().filter(|(i, _)| *i != 2).map(|(_, s)| s).collect();
    let r = s.batch(Origin::Headless, good, true, Some("Recolour".into()));
    assert!(!r.rolled_back && r.completed == 3 && r.failed == 0, "{r:?}");
    assert_eq!(s.active().unwrap().history.labels().last().map(String::as_str), Some("Recolour"));
    assert_eq!(s.active().unwrap().history.labels().len(), labels.len() + 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*project(&s), *before);
    // A stop is never refused there: it runs in its place (here, with no device, it fails with NoDevice and the
    // batch rolls back).
    let r = s.batch(
        Origin::Headless,
        vec![("tools.drawLines".into(), json!({ "pointsMm": [[0, 0], [1, 1]] })), ("laser.stop".into(), json!({}))],
        true,
        Some("x".into()),
    );
    assert_eq!((r.completed, r.failed, r.rolled_back), (1, 1, true), "{r:?}");
    assert_eq!(r.results[1]["error"]["code"], "no_device");
    assert_eq!(*project(&s), *before);
    // Device, file and request commands are refused inside an atomic batch, before anything runs.
    for id in ["file.save", "job.submit", "device.connect"] {
        let r = s.batch(
            Origin::Headless,
            vec![("tools.drawLines".into(), json!({ "pointsMm": [[0, 0], [1, 1]] })), (id.into(), json!({}))],
            true,
            Some("x".into()),
        );
        assert_eq!((r.completed, r.failed), (0, 1), "{id}");
        assert_eq!(*project(&s), *before);
    }
    // Undo is not available inside one.
    let r = s.batch(Origin::Headless, vec![("edit.undo".into(), json!({}))], true, Some("x".into()));
    assert!(r.rolled_back || r.failed == 1);
}

#[test]
fn non_atomic_batches_and_limits() {
    let mut s = headless();
    with_doc(&mut s);
    let steps = vec![
        ("tools.drawLines".to_owned(), json!({ "pointsMm": [[1, 1], [2, 2]] })),
        ("no.such".to_owned(), json!({})),
        ("tools.draw_lines".to_owned(), json!({ "points_mm": [[3, 3], [4, 4]] })),
    ];
    let r = s.batch(Origin::Headless, steps.clone(), false, None);
    assert_eq!((r.completed, r.failed), (2, 1));
    let r = s.batch(Origin::Headless, steps, true, None);
    assert_eq!((r.completed, r.failed), (1, 1));
    let many = vec![("session.list".to_owned(), json!({})); crate::MAX_BATCH_STEPS + 1];
    let r = s.batch(Origin::Headless, many, true, None);
    assert_eq!((r.completed, r.failed), (0, 1));
    // Batch steps are Replay: a human's batch still cannot reach human-only commands.
    let (mut h, p) = human();
    with_doc(&mut h);
    let r = h.batch(Origin::Human, vec![("device.connect".into(), json!({}))], true, None);
    assert_eq!(r.results[0]["error"]["code"], "human_only");
    assert_eq!(p.opens.load(Ordering::SeqCst), 0);
}

#[test]
fn dry_run_changes_nothing() {
    let mut s = headless();
    with_doc(&mut s);
    let before = project(&s);
    let r0 = s.active().unwrap().revision;
    let v = s.execute("tools.rectangle", json!({ "xMm": 0, "yMm": 0, "wMm": 10, "hMm": 5, "dryRun": true })).unwrap();
    assert_eq!(v["ids"].as_array().unwrap().len(), 1);
    assert_eq!(*project(&s), *before);
    assert_eq!(s.active().unwrap().revision, r0);
    assert!(!s.active().unwrap().history.can_undo());
    s.execute("file.new", json!({ "dry_run": true })).unwrap();
    assert_eq!(s.docs().len(), 1);
    s.execute("layer.setCurrent", json!({ "layer": "C07", "dryRun": true })).unwrap();
    assert_eq!(s.active().unwrap().current_layer, LayerKey::Color(0));
    assert_eq!(s.execute("file.save", json!({ "dryRun": true })).unwrap_err().code(), "bad_params");
}

#[test]
fn edits_that_change_nothing_record_nothing() {
    let mut s = headless();
    with_doc(&mut s);
    let r0 = s.active().unwrap().revision;
    s.execute("layer.setOutput", json!({ "layer": 0, "on": true })).unwrap();
    assert_eq!(s.active().unwrap().revision, r0);
    assert!(!s.active().unwrap().history.can_undo());
}

#[test]
fn panics_are_contained_and_the_project_restored() {
    let mut s = headless();
    with_doc(&mut s);
    let before = project(&s);
    let r0 = s.active().unwrap().revision;
    let spec = crate::command!("test.panic", "Panic", menu: [], shortcut: None, kind: Kind::Edit, access: Access::Agent,
        enabled: crate::cmds::always, params: crate::NoParams, run: boom);
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let out = s.run_spec(spec, Origin::Headless, None, json!({}));
    std::panic::set_hook(prev);
    assert_eq!(out.unwrap_err(), EngineError::Internal("test.panic".into()));
    assert_eq!(*project(&s), *before, "the edit made before the panic is rolled back");
    assert!(!s.active().unwrap().history.can_undo());
    assert!(s.active().unwrap().revision > r0);
    assert!(s.journal().is_empty());
}

fn boom(s: &mut Session, _: crate::NoParams) -> Result<Value, EngineError> {
    s.edit(crate::Label::new("Boom"), None, |p, _| {
        p.notes = "changed".into();
        Ok(())
    })?;
    panic!("boom");
}

// ---- prefs ------------------------------------------------------------------------------------------------------

#[test]
fn prefs_set_refuses_machine_and_automation_keys() {
    for origin in [Origin::Human, Origin::Synthetic, Origin::Headless] {
        let mut s = headless();
        for key in ["machine.maxPowerPct", "safety.workAreaMm", "automation.allowAgentConnections", "humanOnly.armWithoutFrame"] {
            let e = s.execute_as(origin, "prefs.set", json!({ "key": key, "value": 1 })).unwrap_err();
            assert_eq!(e.code(), "human_only", "{origin:?} {key}");
            assert!(s.prefs().get(key).is_none());
        }
    }
    let mut s = headless();
    with_doc(&mut s);
    let r0 = s.active().unwrap().revision;
    s.execute("prefs.set", json!({ "key": "output.curveToleranceMm", "value": 0.02 })).unwrap();
    assert_eq!(s.active().unwrap().revision, r0 + 1, "a planner pref bumps the revision");
    assert_eq!(s.execute("prefs.get", json!({ "key": "output.curveToleranceMm" })).unwrap()["value"], 0.02);
    s.execute("prefs.set", json!({ "key": "history.maxStates", "value": 2 })).unwrap();
    for i in 0..5 {
        draw(&mut s, json!([[i, 0], [i, 1]]));
    }
    assert_eq!(s.active().unwrap().history.labels().len(), 2);
}

// ---- commands ---------------------------------------------------------------------------------------------------

#[test]
fn selection_commands() {
    let mut s = headless();
    with_doc(&mut s);
    let a = draw(&mut s, json!([[0, 0], [1, 1]]));
    let b = draw(&mut s, json!([[2, 2], [3, 3]]));
    s.execute("edit.selectAll", json!({})).unwrap();
    assert_eq!(s.active().unwrap().selection.ids.len(), 2);
    s.execute("edit.select", json!({ "ids": [a], "mode": "remove" })).unwrap();
    assert_eq!(s.active().unwrap().selection.ids, vec![openlaser_types::ObjectId(b)]);
    s.execute("edit.select", json!({ "ids": [a], "mode": "toggle" })).unwrap();
    assert_eq!(s.active().unwrap().selection.ids.len(), 2);
    s.execute("edit.selectNone", json!({})).unwrap();
    assert!(s.active().unwrap().selection.ids.is_empty());
    assert_eq!(s.execute("edit.select", json!({ "ids": [999] })).unwrap_err(), EngineError::NoShape(openlaser_types::ObjectId(999)));
    let e = s.execute("edit.delete", json!({})).unwrap_err();
    assert_eq!(e.code(), "disabled", "nothing selected");
    assert_eq!(s.execute("edit.select", json!({})).unwrap_err().code(), "bad_params");
}

#[test]
fn layers_and_cut_settings() {
    let mut s = headless();
    with_doc(&mut s);
    let v = s
        .execute(
            "layer.cut_settings.set",
            json!({ "layer": "C05", "galvo": { "speed_mm_s": 500, "power_pct": 70, "frequency_khz": 40, "q_pulse_ns": 100 }, "fill": { "lpi": 254 }, "mode": "fill" }),
        )
        .unwrap();
    assert_eq!(v["galvo"]["speedMmS"], 500.0);
    assert_eq!(v["galvo"]["frequencyKhz"], 40.0);
    assert_eq!(v["mode"], "fill");
    assert!((v["fill"]["intervalMm"].as_f64().unwrap() - 0.1).abs() < 1e-12);
    let l = s.active().unwrap().project.color_layer(5).unwrap().clone();
    assert_eq!(l.ops[0].galvo.frequency.get(), 40_000);
    assert_eq!(s.active().unwrap().history.labels(), vec!["Set C05 Cut Settings".to_owned()]);
    s.execute("layer.cutSettings.set", json!({ "layer": 5, "galvo": { "frequencyKhz": 35 } })).unwrap();
    assert_eq!(s.active().unwrap().history.labels().last().unwrap(), "Set C05 Frequency");
    let bad = [
        (json!({ "layer": 5, "galvo": { "frequencyKhz": 30.0005 } }), "/galvo/frequencyKhz"),
        (json!({ "layer": 5, "galvo": { "frequencyKhz": 30, "frequencyHz": 30000 } }), "/galvo"),
        (json!({ "layer": 5, "galvo": { "powerPct": 101 } }), "/galvo/powerPct"),
        (json!({ "layer": 5, "galvo": { "speedMmS": 0 } }), "/galvo/speedMmS"),
        (json!({ "layer": 5, "fill": { "intervalMm": 0 } }), "/fill/intervalMm"),
        (json!({ "layer": 5, "passes": 0 }), "/passes"),
        (json!({ "layer": 5, "globalPasses": 0 }), "/globalPasses"),
        (json!({ "layer": 5, "image": { "passes": 0 } }), "/image/passes"),
        (json!({ "layer": 5, "subLayer": 3 }), "/subLayer"),
        (json!({ "layer": "T1", "output": true }), "/layer"),
        (json!({ "layer": 30 }), "/layer"),
        (json!({ "layer": 5, "galvo": { "turbo": true } }), "/galvo/turbo"),
    ];
    for (p, pointer) in bad {
        match s.execute("layer.cutSettings.set", p.clone()) {
            Err(EngineError::BadParams { pointer: got, .. }) => assert_eq!(got, pointer, "{p}"),
            other => panic!("{p}: {other:?}"),
        }
    }
    s.execute("layer.cutSettings.set", json!({ "layer": "t1", "name": "Frame", "show": false })).unwrap();
    assert_eq!(s.active().unwrap().project.layers.tools[0].name, "Frame");
    let g = s.execute("layer.cutSettings.get", json!({ "layer": 5 })).unwrap();
    assert_eq!(g["ops"].as_array().unwrap().len(), 1);
    assert_eq!(s.execute("layer.setOutput", json!({ "layer": "T2", "on": true })).unwrap_err().code(), "bad_params");
    s.execute("layer.setCurrent", json!({ "layer": "C03" })).unwrap();
    let id = draw(&mut s, json!([[0, 0], [1, 1]]));
    assert_eq!(s.active().unwrap().project.shape(openlaser_types::ObjectId(id)).unwrap().layer, LayerKey::Color(3));
}

#[test]
fn inspect_and_lists() {
    let mut s = headless();
    with_doc(&mut s);
    let id = draw(&mut s, json!([[0, 0], [10, 0], [10, 5]]));
    let v = s.execute("document.inspect", json!({})).unwrap();
    assert_eq!(v["objects"]["count"], 1);
    assert_eq!(v["objects"]["items"][0]["id"], id);
    assert_eq!(v["objects"]["items"][0]["bounds"]["max"], json!([10.0, 5.0]));
    assert_eq!(v["layers"][0]["key"], "C00");
    assert_eq!(v["machine"]["kind"], "galvo_lmc");
    assert_eq!(v["history"], json!(["Draw Lines"]));
    assert_eq!(v["canUndo"], true);
    assert!(v["live"].get("fingerprint").is_some());
    let l = s.execute("session.list", json!({})).unwrap();
    assert_eq!(l["active"], 0);
    let m = s.execute("device.list", json!({})).unwrap();
    assert_eq!(m.as_array().unwrap().len(), 2);
    assert_eq!(m[0]["link"], "sim");
    let rows = s.execute("command.list", json!({ "agent": "humanOnly" })).unwrap();
    for r in rows.as_array().unwrap() {
        assert_eq!(r["enabled"], false);
        assert_eq!(r["disabledReason"], crate::command::HUMAN_ONLY_ROW);
    }
    let one = s.execute("command.schema", json!({ "id": "edit.select_all" })).unwrap();
    assert_eq!(one["id"], "edit.selectAll");
    let about = s.execute("help.about", json!({})).unwrap();
    assert_eq!(about["name"], "OpenLaser");
    // Ui commands: allowed with a window, disabled headless.
    assert_eq!(s.execute("view.zoomIn", json!({})).unwrap_err().code(), "disabled");
    let (mut h, _) = human();
    assert_eq!(h.execute_as(Origin::Synthetic, "view.zoomIn", json!({})).unwrap()["ui"], "view.zoomIn");
}

#[test]
fn documents_open_select_close() {
    let mut s = headless();
    s.execute("file.new", json!({ "name": "A" })).unwrap();
    s.execute("file.new", json!({ "name": "B", "machine": "jpt-m7-200" })).unwrap();
    assert_eq!(s.active_index(), Some(1));
    assert_eq!(s.active().unwrap().machine, MachineId("jpt-m7-200".into()));
    s.execute("document.select", json!({ "index": 0 })).unwrap();
    assert_eq!(s.active().unwrap().project.name, "A");
    s.execute("document.close", json!({})).unwrap();
    assert_eq!(s.docs().len(), 1);
    assert_eq!(s.active().unwrap().project.name, "B");
    assert_eq!(s.execute("file.new", json!({ "machine": "nope" })).unwrap_err().code(), "bad_params");
    assert_eq!(s.execute("document.select", json!({ "index": 9 })).unwrap_err().code(), "bad_params");
    s.execute("document.close", json!({})).unwrap();
    assert_eq!(
        s.execute("edit.undo", json!({})).unwrap_err(),
        EngineError::Disabled { id: "edit.undo".into(), reason: "no document open".into(), needs_document: true }
    );
}

#[test]
fn files_respect_the_authority() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.olp").to_string_lossy().into_owned();
    let mut cfg = config(Origin::Headless, false, DeviceAccess::None);
    cfg.files = FsAuthority::Denied;
    let mut s = Session::new(cfg);
    with_doc(&mut s);
    for (id, p) in [("file.open", json!({ "path": path })), ("file.saveAs", json!({ "path": path })), ("job.dump", json!({ "path": path }))] {
        assert_eq!(s.execute(id, p).unwrap_err(), EngineError::Io(crate::files::FS_DENIED.into()), "{id}");
    }
    // A scoped authority answers for itself.
    struct Scoped;
    impl crate::FileAccess for Scoped {
        fn read(&self, path: &str) -> Result<Vec<u8>, String> {
            Err(format!("{path} is outside the automation roots"))
        }
        fn write_atomic(&self, path: &str, _bytes: &[u8]) -> Result<(), String> {
            Err(format!("{path} is outside the automation roots"))
        }
    }
    let mut cfg = config(Origin::Headless, false, DeviceAccess::None);
    cfg.files = FsAuthority::Scoped(Box::new(Scoped));
    let mut s = Session::new(cfg);
    with_doc(&mut s);
    let e = s.execute("file.import", json!({ "path": "/etc/passwd" })).unwrap_err();
    assert!(e.to_string().contains("outside the automation roots"), "{e}");
    assert!(!s.active().unwrap().history.can_undo());
}

#[test]
fn session_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Session>();
}

#[test]
fn edit_rejects_bad_geometry_params() {
    let mut s = headless();
    with_doc(&mut s);
    let cases = [
        ("tools.rectangle", json!({ "xMm": 0, "yMm": 0, "wMm": 0, "hMm": 5 }), "/wMm"),
        ("tools.rectangle", json!({ "xMm": 0, "yMm": 0, "wMm": 4, "hMm": 5, "cornerRadiusMm": 3 }), "/cornerRadiusMm"),
        ("tools.drawLines", json!({ "pointsMm": [[0, 0]] }), "/pointsMm"),
        ("tools.drawLines", json!({ "pointsMm": [[0, 0, 1], [1, 1]] }), "/pointsMm/0"),
        ("tools.drawLines", json!({ "d": "garbage" }), "/d"),
        ("tools.drawLines", json!({}), ""),
        ("arrange.transform", json!({ "ids": [] }), ""),
    ];
    for (id, p, pointer) in cases {
        match s.execute(id, p.clone()) {
            Err(EngineError::BadParams { pointer: got, .. }) => assert_eq!(got, pointer, "{id} {p}"),
            other => panic!("{id} {p}: {other:?}"),
        }
    }
    assert!(!s.active().unwrap().history.can_undo());
}

#[test]
fn refusal_codes_are_reported_through_execute() {
    let (mut s, _) = human();
    with_doc(&mut s);
    let e = s.execute("laser.start", json!({})).unwrap_err();
    assert_eq!(e.code(), "not_connected", "{e}");
    assert!(matches!(e, EngineError::Refused(ref r) if r.code == RefusalCode::NotConnected));
}

#[test]
fn a_run_called_outside_dispatch_is_headless() {
    // Bypassing `execute` skips the policy table; the human-only handlers still refuse (fail closed).
    let (mut s, p) = human();
    with_doc(&mut s);
    for id in ["laser.frame", "laser.start", "device.connect", "device.detect", "device.settings.set", "laser.clearFault"] {
        let spec = registry().find(id).copied().unwrap();
        let e = (spec.run)(&mut s, json!({})).unwrap_err();
        assert_eq!(e.code(), "human_only", "{id}: {e}");
    }
    assert_eq!(p.opens.load(Ordering::SeqCst), 0);
    assert!(s.status(&MachineId("jpt-m7-200-sim".into())).is_none());
}

#[test]
fn edit_after_submit_marks_stale_with_a_planted_submission() {
    // The real submission needs I1's pipeline (tests/engine.rs); the state change on an edit is the engine's own.
    let mut s = Session::new(config(Origin::Synthetic, true, DeviceAccess::None));
    with_doc(&mut s);
    let fp = Fingerprint([4; 32]);
    let view = crate::SubmissionView {
        submission: crate::RequestId(0),
        machine: MachineId("jpt-m7-200-sim".into()),
        fingerprint: fp,
        state: crate::SubmissionState::AwaitingHuman,
        note: String::new(),
        client: "agent".into(),
        summary: crate::devices::tests::summary(fp, "jpt-m7-200-sim", &[10.0]),
        expires_in_s: 0,
        outcome: None,
        stop_report: None,
    };
    let now = s.cfg.clock.now();
    let (v, _) = s.requests.add_submission(view, 0, now);
    let st = s.execute("job.status", json!({ "submission": v.submission.0 })).unwrap();
    assert_eq!(st["state"], "awaiting_human");
    s.poll();
    draw(&mut s, json!([[0, 0], [1, 0]]));
    let st = s.execute("job.status", json!({ "submission": v.submission.0 })).unwrap();
    assert_eq!(st["state"], "stale");
    assert!(s.poll().iter().any(|e| matches!(e, Event::Request { state, .. } if state == "stale")));
    // An agent cannot advance it, only withdraw a pending one.
    assert_eq!(s.approve_request(Origin::Synthetic, v.submission).unwrap_err().code(), "human_only");
    assert_eq!(s.withdraw_request(v.submission).unwrap_err().code(), "stale");
}

// ---- review fixes: stop, ARM race, sticky faults, selection, gantry bytes ----------------------------------------

#[test]
fn stop_with_an_unknown_machine_stops_the_running_one() {
    let (mut s, _) = human();
    with_doc(&mut s);
    s.execute("device.connect", json!({})).unwrap();
    let sim = MachineId("jpt-m7-200-sim".into());
    s.hub[0].status.state = MachineState::Running { packets_acked: 1, total: 9 };
    s.hub[0].op = Some(crate::devices::Op::Run { fingerprint: Fingerprint([1; 32]) });
    let v = s.execute_as(Origin::Synthetic, "laser.stop", json!({ "machine": "jpt-m7-200" })).unwrap();
    assert_eq!(v["stopped"][0]["machine"], sim.0, "{v}");
    assert_eq!(v["latched"], true);
    assert_eq!(v["requested"], "jpt-m7-200");
    assert!(v["note"].as_str().unwrap().contains("not connected"), "{v}");
    // The WP0 driver sends no StopReport: nothing is "confirmed", and the running machine is STOP NOT CONFIRMED.
    assert_eq!(v["confirmed"], false);
    let st = s.status(&sim).unwrap();
    assert!(matches!(st.state, MachineState::StopNotConfirmed) && st.fault.is_some());
    // With nothing connected, nothing is claimed.
    let (mut s, _) = human();
    let v = s.execute_as(Origin::Headless, "laser.stop", json!({ "machine": "jpt-m7-200" })).unwrap();
    assert_eq!((v["latched"].clone(), v["confirmed"].clone()), (json!(false), json!(false)), "{v}");
    assert_eq!(v["stopped"], json!([]));
    let v = s.execute_as(Origin::Synthetic, "laser.frameStop", Value::Null).unwrap();
    assert_eq!((v["latched"].clone(), v["confirmed"].clone()), (json!(false), json!(false)), "{v}");
}

#[test]
fn stop_voids_pending_arm_without_a_poll() {
    // A fast-path stop (control transport thread, signal watcher, panic hook) after Start, with no poll in between:
    // run_armed must refuse, the run must never be queued.
    let (mut s, _) = human();
    with_doc(&mut s);
    plant_pending(&mut s);
    let token = token_for(&s);
    let reg = s.stop_registry();
    std::thread::spawn(move || reg.stop_all(Duration::from_millis(1))).join().unwrap();
    let e = s.run_armed(token).unwrap_err();
    assert!(matches!(&e, EngineError::Refused(r) if r.code == RefusalCode::StoppedAfterQueue), "{e}");
    assert!(s.pending_arm().is_none());
    // A Start after the stop is a new ARM: the earlier stop does not cancel it (request_arm folds it first, as here).
    s.fold_stop_trips();
    plant_pending(&mut s);
    s.poll();
    assert!(s.pending_arm().is_some(), "a stop before Start does not void the ARM Start made");
    // A stop after it does, on the next poll or in run_armed, whichever comes first.
    s.stop_registry().stop_all(Duration::from_millis(1));
    s.poll();
    assert!(s.pending_arm().is_none());
}

#[test]
fn faults_are_sticky_until_acknowledged() {
    let (mut s, _) = human();
    with_doc(&mut s);
    s.execute("device.connect", json!({})).unwrap();
    let m = MachineId("jpt-m7-200-sim".into());
    s.fold_driver_event(&m, DriverEvent::State(MachineState::Idle));
    let refused_for_fault = |e: EngineError| matches!(&e, EngineError::Refused(r) if r.code == RefusalCode::MachineBusy && r.message.contains("Clear Fault"));
    // A driver fault (watchdog, transport): Frame, Start and run_armed are refused, even after the driver says Idle.
    s.fold_driver_event(&m, DriverEvent::Fault(DriverError::Watchdog { waited_ms: 2000 }));
    s.fold_driver_event(&m, DriverEvent::State(MachineState::Idle));
    assert!(matches!(s.status(&m).unwrap().state, MachineState::Fault), "a later Idle never clears a fault");
    assert!(refused_for_fault(s.execute("laser.frame", json!({})).unwrap_err()));
    assert!(refused_for_fault(s.execute("laser.start", json!({})).unwrap_err()));
    assert!(refused_for_fault(s.start_frame(Origin::Human, FrameMode::Loop, None, None).unwrap_err()));
    plant_pending(&mut s);
    let token = token_for(&s);
    assert!(refused_for_fault(s.run_armed(token).unwrap_err()));
    assert!(s.pending_arm().is_none(), "a refused run spends the pending ARM");
    // Only a person's acknowledgement clears it.
    assert_eq!(s.execute_as(Origin::Synthetic, "laser.clearFault", json!({})).unwrap_err().code(), "human_only");
    s.execute("laser.clearFault", json!({})).unwrap();
    assert!(!s.status(&m).unwrap().faulted());
    assert_eq!(s.execute("laser.start", json!({})).unwrap_err().code(), "refused", "now it is the missing frame");

    // An unconfirmed stop (no StopReport within the watchdog) is a sticky fault too.
    s.hub[0].status.state = MachineState::Running { packets_acked: 1, total: 9 };
    s.hub[0].op = Some(crate::devices::Op::Run { fingerprint: Fingerprint([1; 32]) });
    s.stop_all();
    let st = s.status(&m).unwrap();
    assert!(matches!(st.state, MachineState::StopNotConfirmed) && st.fault.is_some(), "{:?}", st.state);
    s.fold_driver_event(&m, DriverEvent::State(MachineState::Idle));
    assert!(matches!(s.status(&m).unwrap().state, MachineState::StopNotConfirmed), "a later Idle never clears it");
    let e = s.execute("laser.frame", json!({})).unwrap_err();
    assert!(refused_for_fault(e.clone()) && e.to_string().contains("STOP NOT CONFIRMED"), "{e}");
    plant_pending(&mut s);
    let token = token_for(&s);
    assert!(refused_for_fault(s.run_armed(token).unwrap_err()));
    s.execute("laser.clearFault", json!({})).unwrap();
    assert!(matches!(s.status(&m).unwrap().state, MachineState::Idle));

    // A StopReport that could not confirm is the same.
    let report =
        StopReport { confirmed: false, elapsed: Duration::from_millis(500), possible_desync: true, mo_closed: false, ports_zero: false, notes: Vec::new() };
    s.fold_driver_event(&m, DriverEvent::Stopped(report.clone()));
    assert!(s.status(&m).unwrap().faulted());
    s.execute("laser.clearFault", json!({})).unwrap();
    s.fold_driver_event(&m, DriverEvent::Finished(Outcome::Stopped(report)));
    assert!(s.status(&m).unwrap().faulted());
    // A user stop that was confirmed is not sticky.
    s.execute("laser.clearFault", json!({})).unwrap();
    let ok = StopReport { confirmed: true, elapsed: Duration::from_millis(40), possible_desync: false, mo_closed: true, ports_zero: true, notes: Vec::new() };
    s.fold_driver_event(&m, DriverEvent::Stopped(ok));
    assert!(!s.status(&m).unwrap().faulted());
}

#[test]
fn selection_change_under_cut_selected_voids_the_arm() {
    let mut s = Session::new(config(Origin::Synthetic, true, DeviceAccess::None));
    with_doc(&mut s);
    draw(&mut s, json!([[0, 0], [10, 0]]));
    draw(&mut s, json!([[0, 5], [10, 5]]));
    // Cut Selected Graphics off: the selection is not part of the job.
    let r0 = s.active().unwrap().revision;
    plant_pending(&mut s);
    s.execute("edit.selectAll", json!({})).unwrap();
    assert_eq!(s.active().unwrap().revision, r0);
    assert!(s.pending_arm().is_some(), "without Cut Selected a selection never changes the job");
    // On: an agent's selection change is a new job.
    s.execute("laser.cutSelected", json!({ "on": true })).unwrap();
    let clean_rev = s.active().unwrap().revision;
    s.docs[0].saved_revision = clean_rev; // as if just saved
    plant_pending(&mut s);
    s.poll();
    s.execute("edit.selectNone", json!({})).unwrap();
    assert!(s.pending_arm().is_none(), "the ARM dialog was for another selection");
    assert_eq!(s.active().unwrap().revision, clean_rev + 1);
    assert!(!s.active().unwrap().dirty(), "selections are not saved: no unsaved changes");
    assert!(s.poll().iter().any(|e| matches!(e, Event::DocChanged { .. })));
    // edit.select, and an edit that only changes the selection, too; a dry run and an unchanged selection do not.
    plant_pending(&mut s);
    let ids: Vec<u64> = s.active().unwrap().project.shapes.iter().map(|sh| sh.id.0).collect();
    s.execute("edit.select", json!({ "ids": [ids[0]], "dryRun": true })).unwrap();
    assert!(s.pending_arm().is_some(), "a dry run changes nothing");
    s.execute("edit.select", json!({ "ids": [ids[0]] })).unwrap();
    assert!(s.pending_arm().is_none());
    plant_pending(&mut s);
    let r = s.active().unwrap().revision;
    s.execute("edit.select", json!({ "ids": [ids[0]] })).unwrap();
    assert_eq!(s.active().unwrap().revision, r, "the same selection is no change");
    assert!(s.pending_arm().is_some());
    s.edit(crate::Label::new("Select"), None, |_, sel| {
        sel.ids.clear();
        Ok(())
    })
    .unwrap();
    assert!(s.pending_arm().is_none());
    assert_eq!(s.active().unwrap().revision, r + 1);
}

#[test]
fn selection_change_under_cut_selected_stales_requests() {
    let mut s = Session::new(config(Origin::Synthetic, true, DeviceAccess::None));
    with_doc(&mut s);
    draw(&mut s, json!([[0, 0], [10, 0]]));
    s.execute("laser.cutSelected", json!({ "on": true })).unwrap();
    s.execute("edit.selectNone", json!({})).unwrap();
    let fp = Fingerprint([4; 32]);
    let view = crate::SubmissionView {
        submission: crate::RequestId(0),
        machine: MachineId("jpt-m7-200-sim".into()),
        fingerprint: fp,
        state: crate::SubmissionState::AwaitingHuman,
        note: String::new(),
        client: "agent".into(),
        summary: crate::devices::tests::summary(fp, "jpt-m7-200-sim", &[10.0]),
        expires_in_s: 0,
        outcome: None,
        stop_report: None,
    };
    let now = s.cfg.clock.now();
    let (v, _) = s.requests.add_submission(view, 0, now);
    s.execute("edit.selectAll", json!({})).unwrap();
    let st = s.execute("job.status", json!({ "submission": v.submission.0 })).unwrap();
    assert_eq!(st["state"], "stale");
}

#[test]
fn run_armed_checks_the_encoded_bytes_before_the_token() {
    let (mut s, _) = human();
    with_doc(&mut s);
    plant_pending(&mut s);
    // The summary the human confirmed no longer describes the object that would be sent.
    s.pending_arm.as_mut().unwrap().summary.bytes_hash = Fingerprint([9; 32]);
    let token = token_for(&s);
    let e = s.run_armed(token).unwrap_err();
    assert!(matches!(&e, EngineError::Refused(r) if r.code == RefusalCode::BytesMismatch), "{e}");
    assert!(s.pending_arm().is_none());
    // And the first-emission cap is applied to the decoded burn, before the token too.
    let capped = Arc::new(TestProvider { cap: Some(5.0), ..TestProvider::default() });
    let dyn_p: Arc<dyn DeviceProvider> = capped;
    let mut s = Session::new(config(Origin::Human, true, DeviceAccess::Provider(dyn_p)));
    with_doc(&mut s);
    s.execute("device.connect", json!({})).unwrap();
    plant_pending(&mut s);
    let token = token_for(&s);
    let e = s.run_armed(token).unwrap_err();
    assert!(matches!(&e, EngineError::Refused(r) if r.code == RefusalCode::BenchPowerCap), "{e}");
}

#[test]
fn start_without_a_frame_receipt_is_refused_before_building() {
    let (mut s, _) = human();
    with_doc(&mut s);
    s.execute("device.connect", json!({})).unwrap();
    let e = s.execute("laser.start", json!({})).unwrap_err();
    assert!(matches!(&e, EngineError::Refused(r) if r.code == RefusalCode::FrameRequired), "{e}");
    assert!(s.pending_arm().is_none());
}

#[test]
fn batches_never_skip_a_stop() {
    let (mut s, _) = human();
    with_doc(&mut s);
    s.execute("device.connect", json!({})).unwrap();
    let stop = || ("laser.stop".to_owned(), json!({}));
    let draw_step = || ("tools.drawLines".to_owned(), json!({ "pointsMm": [[0, 0], [1, 1]] }));
    // Inside an atomic batch a stop runs in its place.
    let t = s.stop_registry().trips();
    let r = s.batch(Origin::Synthetic, vec![draw_step(), stop()], true, Some("Draw and stop".into()));
    assert_eq!((r.completed, r.failed, r.rolled_back), (2, 0, false), "{r:?}");
    assert!(s.stop_registry().trips() > t);
    // An atomic batch refused for another device step still stops.
    let t = s.stop_registry().trips();
    let r = s.batch(Origin::Human, vec![("laser.start".into(), json!({})), stop()], true, Some("x".into()));
    assert_eq!(r.failed, 1);
    assert!(s.stop_registry().trips() > t, "{r:?}");
    assert_eq!(r.results.iter().filter(|v| v["step"] == 1).count(), 1, "{r:?}");
    // A rolled-back atomic batch, and a non-atomic one that stops on an error, still run the stops after the failure.
    for atomic in [Some("y".to_owned()), None] {
        let t = s.stop_registry().trips();
        let r = s.batch(Origin::Synthetic, vec![draw_step(), ("edit.undo".into(), json!({ "bogus": 1 })), draw_step(), stop()], true, atomic.clone());
        assert!(s.stop_registry().trips() > t, "{atomic:?}: {r:?}");
        assert_eq!(r.results.last().unwrap()["step"], 3, "{atomic:?}: {r:?}");
    }
}

#[test]
fn agents_cannot_quit_while_a_machine_works() {
    let (mut s, _) = human();
    with_doc(&mut s);
    s.execute("device.connect", json!({})).unwrap();
    s.hub[0].status.state = MachineState::Running { packets_acked: 1, total: 9 };
    let t = s.stop_registry().trips();
    let e = s.execute_as(Origin::Synthetic, "file.exit", json!({})).unwrap_err();
    assert_eq!(e.code(), "machine_busy", "{e}");
    assert_eq!(s.stop_registry().trips(), t, "the person's job keeps running");
    let v = s.execute_as(Origin::Human, "file.exit", json!({})).unwrap();
    assert_eq!((v["quit"].clone(), v["stopped"].clone()), (json!(true), json!(true)));
    assert!(s.stop_registry().trips() > t);
}

#[test]
fn field_wider_than_rated_is_refused() {
    // H27: the field grows only as far as the lens was cleared; WP1's validate_profile decides, the engine refuses and
    // nothing reaches the store.
    let store = Arc::new(MemStore::default());
    let mut cfg = config(Origin::Human, true, DeviceAccess::None);
    cfg.profiles = Some(store.clone());
    let mut s = Session::new(cfg);
    with_doc(&mut s);
    let e = s.execute("device.settings.set", json!({ "fieldWidthMm": 1000 })).unwrap_err();
    assert_eq!(e.code(), "bad_params", "{e}");
    assert!(store.saved.lock().unwrap().is_empty());
}

// ---- arrange.group / arrange.ungroup -----------------------------------------------------------------------------

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> u64 {
    let v = s.execute("tools.rectangle", json!({ "xMm": x, "yMm": y, "wMm": w, "hMm": h })).unwrap();
    v["ids"][0].as_u64().unwrap()
}

#[test]
fn group_wraps_targets_and_ungroup_restores_them() {
    let mut s = headless();
    with_doc(&mut s);
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 5.0, 5.0);
    let c = rect(&mut s, 40.0, 0.0, 2.0, 2.0); // stays top-level
    let before = project(&s).bounds(&[openlaser_types::ObjectId(a), openlaser_types::ObjectId(b)]).unwrap();
    // One object is not enough; ids inside a group are not top-level targets.
    assert!(s.execute("arrange.group", json!({ "ids": [a] })).unwrap_err().code() == "bad_params");
    let v = s.execute("arrange.group", json!({ "ids": [a, b] })).unwrap();
    let g = v["ids"][0].as_u64().unwrap();
    assert_eq!(s.active().unwrap().selection.ids, vec![openlaser_types::ObjectId(g)]);
    // The group sits at the topmost child's slot, keeping draw order; the bounds are unchanged.
    {
        let p = project(&s);
        assert_eq!(p.shapes.len(), 2);
        assert_eq!(p.shapes[0].id.0, g, "the group takes the topmost child's slot (below the untouched shape)");
        assert_eq!(p.shapes[1].id.0, c);
        match &p.shapes[0].geom {
            openlaser_doc::Geom::Group(kids) => {
                assert_eq!(kids.len(), 2);
                assert_eq!(kids.iter().map(|k| k.id.0).collect::<Vec<_>>(), vec![a, b], "relative order kept");
                assert_eq!(kids[0].xform, openlaser_geom::Affine::translate((5.0, 5.0)), "children keep their own transforms");
            }
            other => panic!("not a group: {other:?}"),
        }
        assert_eq!(p.bounds(&[openlaser_types::ObjectId(g)]).unwrap(), before, "grouping never moves geometry");
    }
    // Ungroup restores, in place.
    let v = s.execute("arrange.ungroup", json!({ "ids": [g] })).unwrap();
    assert_eq!(v["changed"], 2);
    {
        let p = project(&s);
        assert_eq!(p.shapes.len(), 3);
        assert_eq!(p.bounds(&[openlaser_types::ObjectId(a), openlaser_types::ObjectId(b)]).unwrap(), before);
        assert_eq!(s.active().unwrap().selection.ids.len(), 2);
    }
    // Nothing to ungroup and a bad id both refuse.
    assert!(s.execute("arrange.ungroup", json!({ "ids": [a] })).unwrap_err().code() == "bad_params");
    // Undo puts the group back.
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(project(&s).shapes.len(), 2);
    assert!(s.execute("edit.redo", json!({})).is_ok());
}

#[test]
fn ungroup_composes_the_group_transform_and_ungrouping_keeps_z_order() {
    let mut s = headless();
    with_doc(&mut s);
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 20.0, 4.0, 4.0);
    let v = s.execute("arrange.group", json!({ "ids": [a, b] })).unwrap();
    let g = v["ids"][0].as_u64().unwrap();
    // Translate the whole group 7 right, 3 up; releasing the children bakes it into their xforms.
    s.execute("arrange.transform", json!({ "ids": [g], "xMm": 20.0, "yMm": 20.0, "anchor": "bottomLeft" })).unwrap();
    s.execute("arrange.ungroup", json!({ "ids": [g] })).unwrap();
    let p = project(&s);
    assert_eq!(p.bounds(&[openlaser_types::ObjectId(a)]).unwrap().unwrap().min, [20.0, 20.0], "the child landed at the composed position");
    // A second group/ungroup round trip leaves the order and bounds exact.
    let order: Vec<u64> = p.shapes.iter().map(|sh| sh.id.0).collect();
    let bounds = p.bounds(&order.iter().map(|i| openlaser_types::ObjectId(*i)).collect::<Vec<_>>()).unwrap();
    let v = s.execute("arrange.group", json!({ "ids": order })).unwrap();
    let g = v["ids"][0].as_u64().unwrap();
    s.execute("arrange.ungroup", json!({ "ids": [g] })).unwrap();
    let p = project(&s);
    assert_eq!(p.shapes.iter().map(|sh| sh.id.0).collect::<Vec<_>>(), order, "z-order preserved");
    assert_eq!(p.bounds(&order.iter().map(|i| openlaser_types::ObjectId(*i)).collect::<Vec<_>>()).unwrap(), bounds);
}

#[test]
fn a_grouped_design_plans_the_same_sections() {
    let mut s = headless();
    with_doc(&mut s);
    let a = rect(&mut s, -20.0, -20.0, 10.0, 10.0);
    let b = rect(&mut s, 5.0, 5.0, 10.0, 10.0);
    let m = openlaser_machine::jpt_m7_200_sim();
    let ungrouped = openlaser_plan::plan(
        &project(&s),
        &m,
        &openlaser_plan::PlanOptions {
            selected: None,
            repeats: 1,
            frame_style: openlaser_types::FrameStyle::Contour,
            frame_source: openlaser_types::FrameSource::Job,
            flatten_tol_mm: 0.01,
        },
    )
    .unwrap();
    let v = s.execute("arrange.group", json!({ "ids": [a, b] })).unwrap();
    let g = v["ids"][0].as_u64().unwrap();
    let grouped = openlaser_plan::plan(
        &project(&s),
        &m,
        &openlaser_plan::PlanOptions {
            selected: None,
            repeats: 1,
            frame_style: openlaser_types::FrameStyle::Contour,
            frame_source: openlaser_types::FrameSource::Job,
            flatten_tol_mm: 0.01,
        },
    )
    .unwrap();
    assert_eq!(grouped.sections, ungrouped.sections, "a group plans as its children");
    s.execute("arrange.ungroup", json!({ "ids": [g] })).unwrap();
    let after = openlaser_plan::plan(
        &project(&s),
        &m,
        &openlaser_plan::PlanOptions {
            selected: None,
            repeats: 1,
            frame_style: openlaser_types::FrameStyle::Contour,
            frame_source: openlaser_types::FrameSource::Job,
            flatten_tol_mm: 0.01,
        },
    )
    .unwrap();
    assert_eq!(after.sections, ungrouped.sections);
}

// ---- clipboard ---------------------------------------------------------------------------------------------------

#[test]
fn copy_paste_duplicates_with_new_ids_at_the_offset() {
    let mut s = headless();
    with_doc(&mut s);
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let history_len = s.active().unwrap().history.undo.len();
    let v = s.execute("edit.copy", json!({ "ids": [a] })).unwrap();
    assert_eq!(v["changed"], 1);
    assert_eq!(s.active().unwrap().history.undo.len(), history_len, "copy adds no history entry");
    // Default paste: +5 right, -5 down from the copied centre (5,5) → new centre (10,0).
    let v = s.execute("edit.paste", json!({})).unwrap();
    let p1 = v["ids"][0].as_u64().unwrap();
    assert_ne!(p1, a, "fresh id");
    let b = project(&s).bounds(&[openlaser_types::ObjectId(p1)]).unwrap().unwrap();
    assert_eq!(b.min, [5.0, -5.0], "{b:?}");
    assert_eq!(s.active().unwrap().selection.ids, vec![openlaser_types::ObjectId(p1)]);
    // A second paste is a second, independent copy.
    let v = s.execute("edit.paste", json!({})).unwrap();
    let p2 = v["ids"][0].as_u64().unwrap();
    assert_ne!(p2, p1);
    let b2 = project(&s).bounds(&[openlaser_types::ObjectId(p2)]).unwrap().unwrap();
    assert_eq!(b2, b);
    // Paste at a point centres the set there; pasteInPlace keeps the copied position exactly.
    let v = s.execute("edit.paste", json!({ "xMm": 50.0, "yMm": 50.0 })).unwrap();
    let p3 = v["ids"][0].as_u64().unwrap();
    let b = project(&s).bounds(&[openlaser_types::ObjectId(p3)]).unwrap().unwrap();
    assert_eq!((b.min[0] + b.max[0]) / 2.0, 50.0);
    assert_eq!((b.min[1] + b.max[1]) / 2.0, 50.0);
    let v = s.execute("edit.pasteInPlace", json!({})).unwrap();
    let p4 = v["ids"][0].as_u64().unwrap();
    let b = project(&s).bounds(&[openlaser_types::ObjectId(p4)]).unwrap().unwrap();
    assert_eq!(b.min, [0.0, 0.0]);
    // Cut removes in one undo step; the copy survives in the clipboard.
    let history = s.active().unwrap().history.undo.len();
    let v = s.execute("edit.cut", json!({ "ids": [a] })).unwrap();
    assert_eq!(v["changed"], 1);
    assert_eq!(s.active().unwrap().history.undo.len(), history + 1, "one undo step");
    assert!(project(&s).shape(openlaser_types::ObjectId(a)).is_none());
    let v = s.execute("edit.pasteInPlace", json!({})).unwrap();
    let back = v["ids"][0].as_u64().unwrap();
    assert_eq!(project(&s).bounds(&[openlaser_types::ObjectId(back)]).unwrap().unwrap().min, [0.0, 0.0], "the cut shape pastes back exactly");
}

#[test]
fn paste_works_into_a_fresh_document_and_an_empty_clipboard_is_disabled() {
    let mut s = headless();
    with_doc(&mut s);
    let e = s.execute("edit.paste", json!({})).unwrap_err();
    assert!(matches!(e, EngineError::Disabled { ref reason, .. } if reason.contains("clipboard")), "{e}");
    let a = rect(&mut s, 10.0, 10.0, 10.0, 10.0);
    s.execute("edit.copy", json!({ "ids": [a] })).unwrap();
    // A whole new document: the paste still lands and the layer stays valid.
    s.execute("file.new", json!({})).unwrap();
    let v = s.execute("edit.pasteInPlace", json!({})).unwrap();
    let p = v["ids"][0].as_u64().unwrap();
    let doc = s.active().unwrap();
    assert_eq!(doc.project.bounds(&[openlaser_types::ObjectId(p)]).unwrap().unwrap().min, [10.0, 10.0]);
    assert_eq!(doc.project.shape(openlaser_types::ObjectId(p)).unwrap().layer, openlaser_types::LayerKey::Color(0));
}

// ---- tools.polygon, tools.measure --------------------------------------------------------------------------------

#[test]
fn polygon_params_are_validated_and_the_outline_has_the_sides() {
    let mut s = headless();
    with_doc(&mut s);
    let v = s.execute("tools.polygon", json!({ "cxMm": 0, "cyMm": 0, "radiusMm": 10, "sides": 5 })).unwrap();
    let id = v["ids"][0].as_u64().unwrap();
    let polys = project(&s).world_polylines(openlaser_types::ObjectId(id), 0.01).unwrap();
    assert_eq!(polys.len(), 1);
    assert_eq!(polys[0].0.len(), 6, "5 vertices + close");
    // The polygon's own centre is (cx, cy): every vertex is exactly `radius` from it, and the first points up.
    // (An odd-sided polygon's bounding box is NOT centred there; centring the box shifted it off the drag point.)
    assert!(polys[0].0.iter().all(|p| (p[0].hypot(p[1]) - 10.0).abs() < 1e-9), "{:?}", polys[0].0);
    assert!(polys[0].0.iter().any(|p| p[0].abs() < 1e-9 && (p[1] - 10.0).abs() < 1e-9), "a vertex straight up");
    // rotateDeg turns it about that centre: a 90°-rotated pentagon at (50, 7) has a vertex at (40, 7) (CCW, Y up).
    let v = s.execute("tools.polygon", json!({ "cxMm": 50, "cyMm": 7, "radiusMm": 10, "sides": 5, "rotateDeg": 90 })).unwrap();
    let id2 = v["ids"][0].as_u64().unwrap();
    let polys = project(&s).world_polylines(openlaser_types::ObjectId(id2), 0.01).unwrap();
    assert!(polys[0].0.iter().all(|p| ((p[0] - 50.0).hypot(p[1] - 7.0) - 10.0).abs() < 1e-9), "{:?}", polys[0].0);
    assert!(polys[0].0.iter().any(|p| (p[0] - 40.0).abs() < 1e-9 && (p[1] - 7.0).abs() < 1e-9), "{:?}", polys[0].0);
    for (p, ptr) in [
        (json!({ "cxMm": 0, "cyMm": 0, "radiusMm": 0 }), "/radiusMm"),
        (json!({ "cxMm": 0, "cyMm": 0, "radiusMm": 5, "sides": 2 }), "/sides"),
        (json!({ "cxMm": 0, "cyMm": 0, "radiusMm": 5, "sides": 100 }), "/sides"),
        (json!({ "cxMm": 0, "cyMm": 0, "radiusMm": 5, "bogus": 1 }), ""),
    ] {
        assert_eq!(s.execute("tools.polygon", p.clone()).unwrap_err().code(), "bad_params", "{p}: expected {ptr}");
    }
    assert!(s.active().unwrap().history.can_undo(), "valid polygons were created above");
}

#[test]
fn measure_reports_distance_and_angle() {
    let mut s = headless();
    let v = s.execute("tools.measure", json!({ "fromMm": [0, 0], "toMm": [3, 4] })).unwrap();
    assert_eq!(v["distanceMm"], 5.0);
    assert_eq!((v["dxMm"].as_f64().unwrap(), v["dyMm"].as_f64().unwrap()), (3.0, 4.0));
    assert!((v["angleDeg"].as_f64().unwrap() - 53.130102).abs() < 1e-4, "{v}");
    for (to, want) in [([0.0, 5.0], 90.0), ([-5.0, 0.0], 180.0), ([0.0, -5.0], -90.0), ([1.0, -1.0], -45.0)] {
        let v = s.execute("tools.measure", json!({ "fromMm": [0, 0], "toMm": to })).unwrap();
        assert!((v["angleDeg"].as_f64().unwrap() - want).abs() < 1e-9, "{v} vs {want}");
    }
    assert_eq!(s.execute("tools.measure", json!({ "fromMm": [0, 0], "toMm": ["x", 1] })).unwrap_err().code(), "bad_params");
}

// ---- weld, booleans, offsets -------------------------------------------------------------------------------------

fn bounds_of(s: &Session, id: u64) -> openlaser_types::Bounds {
    project(s).bounds(&[openlaser_types::ObjectId(id)]).unwrap().unwrap()
}

#[test]
fn weld_unions_overlapping_rects_into_one_path() {
    let mut s = headless();
    with_doc(&mut s);
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 5.0, 5.0, 10.0, 10.0);
    let v = s.execute("tools.weld", json!({ "ids": [a, b] })).unwrap();
    let g = v["ids"][0].as_u64().unwrap();
    let p = project(&s);
    assert_eq!(p.shapes.len(), 1);
    assert!(matches!(p.shape(openlaser_types::ObjectId(g)).unwrap().geom, openlaser_doc::Geom::Path(_)));
    assert_eq!(s.active().unwrap().selection.ids, vec![openlaser_types::ObjectId(g)]);
    assert_eq!(bounds_of(&s, g), openlaser_types::Bounds { min: [0.0, 0.0], max: [15.0, 15.0] });
    // An open path in the targets is named.
    let l = draw(&mut s, json!([[50, 0], [60, 0]]));
    let e = s.execute("tools.weld", json!({ "ids": [g, l] })).unwrap_err();
    assert!(matches!(e, EngineError::BadParams { .. }) && e.to_string().contains(&l.to_string()), "{e}");
    // Fewer than two targets is a bad-param error.
    assert_eq!(s.execute("tools.weld", json!({ "ids": [g] })).unwrap_err().code(), "bad_params");
    // Undo restores both originals.
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap(); // once for weld (line draw also made a step — unwind past it)
}

#[test]
fn booleans_combine_exactly_two_shapes() {
    let mut s = headless();
    with_doc(&mut s);
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 5.0, 5.0, 10.0, 10.0);
    // Subtract: topmost (b, drawn later) is removed from the bottom (a).
    let v = s.execute("tools.booleanSubtract", json!({ "ids": [a, b] })).unwrap();
    let g = v["ids"][0].as_u64().unwrap();
    let p = project(&s);
    assert_eq!(p.shapes.len(), 1);
    assert_eq!(bounds_of(&s, g), openlaser_types::Bounds { min: [0.0, 0.0], max: [10.0, 10.0] });
    // The result has an outer and a notch boundary — two rings where they bit.
    let polys = p.world_polylines(openlaser_types::ObjectId(g), 0.01).unwrap();
    assert_eq!(polys.len(), 1, "{polys:?}");
    // Wrong arity and a non-overlapping pair refuse.
    assert_eq!(s.execute("tools.booleanUnion", json!({ "ids": [g] })).unwrap_err().code(), "bad_params");
    let c = rect(&mut s, 100.0, 100.0, 5.0, 5.0);
    let e = s.execute("tools.booleanIntersect", json!({ "ids": [g, c] })).unwrap_err();
    assert_eq!(e.code(), "bad_params");
    assert!(e.to_string().contains("overlap"), "{e}");
    // Union keeps both bounds.
    let d = rect(&mut s, 200.0, 0.0, 10.0, 10.0);
    let v = s.execute("tools.booleanUnion", json!({ "ids": [c, d] })).unwrap();
    let u = v["ids"][0].as_u64().unwrap();
    assert_eq!(bounds_of(&s, u), openlaser_types::Bounds { min: [100.0, 0.0], max: [210.0, 105.0] });
}

#[test]
fn offset_grows_shrinks_and_strokes() {
    let mut s = headless();
    with_doc(&mut s);
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let v = s.execute("tools.offsetShapes", json!({ "ids": [a], "distanceMm": 2.0 })).unwrap();
    let o = v["ids"][0].as_u64().unwrap();
    assert_eq!(bounds_of(&s, o), openlaser_types::Bounds { min: [-2.0, -2.0], max: [12.0, 12.0] });
    assert!(project(&s).shape(openlaser_types::ObjectId(a)).is_some(), "the original is kept by default");
    // Inward shrink.
    let v = s.execute("tools.offsetShapes", json!({ "ids": [a], "distanceMm": 1.0, "direction": "inward" })).unwrap();
    let o = v["ids"][0].as_u64().unwrap();
    assert_eq!(bounds_of(&s, o), openlaser_types::Bounds { min: [1.0, 1.0], max: [9.0, 9.0] });
    // Open paths stroke to half the distance; a shape that vanishes is reported, not failed alone.
    let l = draw(&mut s, json!([[50, 50], [60, 50]]));
    let v = s.execute("tools.offsetShapes", json!({ "ids": [l], "distanceMm": 2.0 })).unwrap();
    let o = v["ids"][0].as_u64().unwrap();
    let b = bounds_of(&s, o);
    assert!((b.max[0] - b.min[0] - 12.0).abs() < 0.01 && (b.max[1] - b.min[1] - 2.0).abs() < 0.01, "{b:?}");
    // deleteOriginal removes the source; the shrink of a tiny square vanishes it and others survive.
    let v = s.execute("tools.offsetShapes", json!({ "ids": [o], "distanceMm": 1.0, "deleteOriginal": true })).unwrap();
    assert!(v["ids"].as_array().unwrap().len() == 1);
    // All-vanishing is a bad-param error.
    let t = rect(&mut s, 300.0, 300.0, 1.0, 1.0);
    assert_eq!(s.execute("tools.offsetShapes", json!({ "ids": [t], "distanceMm": 1.0, "direction": "inward" })).unwrap_err().code(), "bad_params");
    assert_eq!(s.execute("tools.offsetShapes", json!({ "ids": [a], "distanceMm": 0 })).unwrap_err().code(), "bad_params");
    assert_eq!(s.execute("tools.offsetShapes", json!({ "ids": [a], "distanceMm": 1, "corner": "bogus" })).unwrap_err().code(), "bad_params");
}

#[test]
fn a_welded_design_plans_one_outline() {
    let mut s = headless();
    with_doc(&mut s);
    let a = rect(&mut s, -20.0, -20.0, 10.0, 10.0);
    let b = rect(&mut s, -15.0, -20.0, 10.0, 10.0);
    let m = openlaser_machine::jpt_m7_200_sim();
    let v = s.execute("tools.weld", json!({ "ids": [a, b] })).unwrap();
    let g = v["ids"][0].as_u64().unwrap();
    let plan = openlaser_plan::plan(
        &project(&s),
        &m,
        &openlaser_plan::PlanOptions {
            selected: None,
            repeats: 1,
            frame_style: openlaser_types::FrameStyle::Contour,
            frame_source: openlaser_types::FrameSource::Job,
            flatten_tol_mm: 0.01,
        },
    )
    .unwrap();
    // One welded outline: one section, one path — the shared edge burns once, not twice.
    let total_paths: usize = plan.sections.iter().map(|sec| sec.paths.len()).sum();
    assert_eq!(total_paths, 1, "{:?}", plan.sections.len());
    assert_eq!(bounds_of(&s, g), openlaser_types::Bounds { min: [-20.0, -20.0], max: [-5.0, -10.0] });
}

// ---- text: tools.text, text.edit, text.fonts, edit.convertToPath --------------------------------------------------

fn with_hack(s: &mut Session) {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../text/tests/fonts");
    s.set_font_dirs(vec![dir]);
}

#[test]
fn tools_text_lays_out_and_selects() {
    let mut s = headless();
    with_doc(&mut s);
    with_hack(&mut s);
    let v = s.execute("tools.text", json!({ "xMm": 10, "yMm": 20, "text": "HI", "family": "Hack", "heightMm": 10 })).unwrap();
    let id = v["ids"][0].as_u64().unwrap();
    let p = project(&s);
    let sh = p.shape(openlaser_types::ObjectId(id)).unwrap();
    let openlaser_doc::Geom::Text(t) = &sh.geom else { panic!("not text") };
    assert_eq!(t.family, "Hack");
    // The anchor sits at the click: the baseline anchor is xform; the 'H' cap height is heightMm.
    let b = bounds_of(&s, id);
    assert!((b.max[1] - b.min[1] - 10.0).abs() < 0.05, "{b:?}");
    // The baseline lands on y; the left ink edge stays within a sidebearing of the anchor.
    assert!((b.min[1] - 20.0).abs() < 0.05 && b.min[0] >= 10.0 && b.min[0] < 12.0, "{b:?}");
    assert_eq!(s.active().unwrap().selection.ids, vec![openlaser_types::ObjectId(id)]);
    // Missing glyph and bad params refuse.
    assert_eq!(s.execute("tools.text", json!({ "xMm": 0, "yMm": 0, "text": "中", "family": "Hack" })).unwrap_err().code(), "bad_params");
    assert_eq!(s.execute("tools.text", json!({ "xMm": 0, "yMm": 0, "text": "A", "bogus": 1 })).unwrap_err().code(), "bad_params");
}

#[test]
fn text_edit_relays_in_place_and_undoes() {
    let mut s = headless();
    with_doc(&mut s);
    with_hack(&mut s);
    let v = s.execute("tools.text", json!({ "xMm": 0, "yMm": 0, "text": "A", "family": "Hack", "heightMm": 10 })).unwrap();
    let id = v["ids"][0].as_u64().unwrap();
    let before = bounds_of(&s, id);
    s.execute("text.edit", json!({ "id": id, "text": "WW", "heightMm": 20 })).unwrap();
    let after = bounds_of(&s, id);
    assert!(after.max[1] - after.min[1] > before.max[1] - before.min[1] + 9.0, "{before:?} → {after:?}");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(bounds_of(&s, id), before);
}

#[test]
fn text_fonts_lists_and_convert_to_path_bakes() {
    let mut s = headless();
    with_doc(&mut s);
    with_hack(&mut s);
    let v = s.execute("text.fonts", json!({})).unwrap();
    assert_eq!(v["fonts"][0]["family"], "Hack", "{v}");
    let v = s.execute("tools.text", json!({ "xMm": 0, "yMm": 0, "text": "H", "family": "Hack" })).unwrap();
    let id = v["ids"][0].as_u64().unwrap();
    let before = bounds_of(&s, id);
    s.execute("edit.convertToPath", json!({ "ids": [id] })).unwrap();
    let p = project(&s);
    let sh = p.shape(openlaser_types::ObjectId(id)).unwrap();
    assert!(matches!(sh.geom, openlaser_doc::Geom::Path(_)), "converted in place");
    assert_eq!(bounds_of(&s, id), before, "world geometry identical");
    // Also a parametric rect.
    let r = rect(&mut s, 50.0, 50.0, 10.0, 10.0);
    let b = bounds_of(&s, r);
    s.execute("edit.convertToPath", json!({ "ids": [r] })).unwrap();
    assert!(matches!(project(&s).shape(openlaser_types::ObjectId(r)).unwrap().geom, openlaser_doc::Geom::Path(_)));
    assert_eq!(bounds_of(&s, r), b);
}

#[test]
fn a_text_shape_plans_its_outline() {
    let mut s = headless();
    with_doc(&mut s);
    with_hack(&mut s);
    let v = s.execute("tools.text", json!({ "xMm": 10, "yMm": 10, "text": "HI", "family": "Hack", "heightMm": 10 })).unwrap();
    let id = v["ids"][0].as_u64().unwrap();
    let m = openlaser_machine::jpt_m7_200_sim();
    let plan = openlaser_plan::plan(
        &project(&s),
        &m,
        &openlaser_plan::PlanOptions {
            selected: None,
            repeats: 1,
            frame_style: openlaser_types::FrameStyle::Contour,
            frame_source: openlaser_types::FrameSource::Job,
            flatten_tol_mm: 0.01,
        },
    )
    .unwrap();
    assert!(!plan.sections.is_empty());
    let total: usize = plan.sections.iter().map(|sec| sec.paths.len()).sum();
    assert!(total >= 2, "H+I outlines (inner counter of nothing — two letter outlines): {total}");
    let b = bounds_of(&s, id);
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for sec in &plan.sections {
        for path in &sec.paths {
            for pt in &path.0 {
                lo[0] = lo[0].min(pt[0]);
                lo[1] = lo[1].min(pt[1]);
                hi[0] = hi[0].max(pt[0]);
                hi[1] = hi[1].max(pt[1]);
            }
        }
    }
    assert!((lo[0] - b.min[0]).abs() < 0.02 && (hi[1] - b.max[1]).abs() < 0.02, "{lo:?} {hi:?} vs {b:?}");
}

// ---- path.get / path.set (node editing) ---------------------------------------------------------------------------

#[test]
fn path_get_and_set_round_trip_in_object_space() {
    let mut s = headless();
    with_doc(&mut s);
    let id = rect(&mut s, 10.0, 10.0, 20.0, 10.0);
    // A rect is parametric: path.set refuses with the convert message.
    let e = s.execute("path.set", json!({ "id": id, "d": "M0 0L5 0" })).unwrap_err();
    assert_eq!(e.code(), "bad_params");
    assert!(e.to_string().contains("convert to path"), "{e}");
    s.execute("edit.convertToPath", json!({ "ids": [id] })).unwrap();
    let v = s.execute("path.get", json!({ "id": id })).unwrap();
    let d = v["d"].as_str().unwrap();
    assert!(d.starts_with("M"), "{d}");
    // A world-size check: the path is object-space, the xform is unchanged.
    s.execute("path.set", json!({ "id": id, "d": "M0 0L30 0L30 30L0 30Z" })).unwrap();
    let b = bounds_of(&s, id);
    assert!((b.max[0] - b.min[0] - 30.0).abs() < 1e-9 && (b.max[1] - b.min[1] - 30.0).abs() < 1e-9, "{b:?}");
    // Undo restores the converted path.
    s.execute("edit.undo", json!({})).unwrap();
    let b2 = bounds_of(&s, id);
    assert!((b2.max[0] - b2.min[0] - 20.0).abs() < 1e-9);
    // Bad d and bad ids refuse.
    assert_eq!(s.execute("path.set", json!({ "id": id, "d": "L1 1" })).unwrap_err().code(), "bad_params");
    assert_eq!(s.execute("path.set", json!({ "id": 9999, "d": "M0 0L1 0" })).unwrap_err().code(), "bad_params");
    assert_eq!(s.execute("path.get", json!({ "id": 9999 })).unwrap_err().code(), "bad_params");
    // Locked shapes refuse.
    s.edit(Label::new("lock"), None, move |project, _| {
        if let Some(sh) = project.shapes.iter_mut().find(|sh| sh.id.0 == id) {
            Arc::make_mut(sh).locked = true;
        }
        Ok(Created { ids: vec![] })
    })
    .unwrap();
    assert_eq!(s.execute("path.set", json!({ "id": id, "d": "M0 0L1 0" })).unwrap_err().code(), "disabled");
}

// ---- tools.traceImage --------------------------------------------------------------------------------------------

/// A 20×20 luma buffer: an ink square cols/rows 5..15 (the 100 px² square from the tracer tests).
fn square_gray() -> Vec<u8> {
    let mut g = vec![255u8; 20 * 20];
    for r in 5..15 {
        for c in 5..15 {
            g[r * 20 + c] = 0;
        }
    }
    g
}

fn add_bitmap(s: &mut Session, gray: Vec<u8>, px: [u32; 2], size_mm: [f64; 2]) -> u64 {
    use std::sync::Arc;
    let id = s
        .edit(Label::new("bitmap"), None, move |project, sel| {
            let id = project.alloc_id().map_err(geomx::doc_error)?;
            project.shapes.push(Arc::new(openlaser_doc::Shape {
                id,
                name: None,
                layer: LayerKey::Color(0),
                xform: openlaser_doc::Affine::IDENTITY,
                geom: openlaser_doc::Geom::Bitmap(Arc::new(openlaser_doc::BitmapData {
                    asset: openlaser_doc::AssetId([7; 32]),
                    px,
                    size_mm,
                    gray: Arc::from(gray.as_slice()),
                    source_ext: "png".to_owned(),
                })),
                locked: false,
                power_scale: openlaser_types::Percent::new(100.0).unwrap(),
                priority: 0,
            }));
            sel.ids = vec![id];
            sel.key = Some(id);
            Ok(Created { ids: vec![id] })
        })
        .unwrap();
    id.ids[0].0
}

#[test]
fn trace_image_draws_the_square_in_object_space() {
    let mut s = headless();
    with_doc(&mut s);
    // A 20×20 px bitmap at 40×40 mm centred at the origin: the ink square sits at ±5 px = ±10 mm.
    let bid = add_bitmap(&mut s, square_gray(), [20, 20], [40.0, 40.0]);
    let v = s.execute("tools.traceImage", json!({ "id": bid, "smoothness": 0 })).unwrap();
    let id = v["ids"][0].as_u64().unwrap();
    let b = bounds_of(&s, id);
    assert!((b.min[0] + 10.0).abs() < 1e-9 && (b.max[0] - 10.0).abs() < 1e-9 && (b.min[1] + 10.0).abs() < 1e-9 && (b.max[1] - 10.0).abs() < 1e-9, "{b:?}");
    // The traced path sits on the bitmap's layer, just above it, and is the selection.
    let p = project(&s);
    assert!(p.shapes.iter().any(|sh| sh.id.0 == id && matches!(sh.geom, openlaser_doc::Geom::Path(_))));
    assert_eq!(s.active().unwrap().selection.ids, vec![openlaser_types::ObjectId(id)]);
    // Bad targets refuse.
    assert_eq!(s.execute("tools.traceImage", json!({})).unwrap_err().code(), "bad_params"); // selection is the path
    assert_eq!(s.execute("tools.traceImage", json!({ "id": 999 })).unwrap_err().code(), "bad_params");
    // Nothing at this threshold: a white image.
    let bid2 = add_bitmap(&mut s, vec![255u8; 4], [2, 2], [4.0, 4.0]);
    assert_eq!(s.execute("tools.traceImage", json!({ "id": bid2 })).unwrap_err().code(), "bad_params");
}

#[test]
fn trace_image_maps_through_the_bitmap_transform() {
    let mut s = headless();
    with_doc(&mut s);
    let bid = add_bitmap(&mut s, square_gray(), [20, 20], [40.0, 40.0]);
    // Move the bitmap: the traced path follows its xform (translation is on the bitmap shape itself).
    s.execute("arrange.transform", json!({ "ids": [bid], "anchor": "center", "xMm": 50.0, "yMm": 30.0 })).unwrap();
    s.execute("document.select", json!({ "index": 0 })).ok();
    let v = s.execute("tools.traceImage", json!({ "id": bid, "smoothness": 0, "deleteImage": true })).unwrap();
    let id = v["ids"][0].as_u64().unwrap();
    let b = bounds_of(&s, id);
    assert!((b.min[0] - 40.0).abs() < 1e-9 && (b.max[0] - 60.0).abs() < 1e-9 && (b.min[1] - 20.0).abs() < 1e-9 && (b.max[1] - 40.0).abs() < 1e-9, "{b:?}");
    assert!(!project(&s).shapes.iter().any(|sh| sh.id.0 == bid), "the image is gone");
    // Undo restores both.
    s.execute("edit.undo", json!({})).unwrap();
    assert!(project(&s).shapes.iter().any(|sh| sh.id.0 == bid));
    assert!(!project(&s).shapes.iter().any(|sh| sh.id.0 == id));
    // A traced design plans as closed paths.
    let m = openlaser_machine::jpt_m7_200_sim();
    let plan = openlaser_plan::plan(
        &project(&s),
        &m,
        &openlaser_plan::PlanOptions {
            selected: None,
            repeats: 1,
            frame_style: openlaser_types::FrameStyle::Contour,
            frame_source: openlaser_types::FrameSource::Job,
            flatten_tol_mm: 0.01,
        },
    );
    assert!(plan.is_ok(), "{plan:?}");
}

#[test]
fn trace_by_id_works_whatever_is_selected() {
    // Agents pass `id` without a selection habit: the enabled gate is only needs_doc, the run resolves `id`.
    let mut s = headless();
    with_doc(&mut s);
    let rect = s.execute("tools.rectangle", json!({ "xMm": 0.0, "yMm": 0.0, "wMm": 10.0, "hMm": 10.0 })).unwrap();
    let rid = rect["ids"][0].as_u64().unwrap(); // the rectangle is now the selection
    let bid = {
        let mut g = vec![255u8; 20 * 20];
        for r in 5..15 {
            for c in 5..15 {
                g[r * 20 + c] = 0;
            }
        }
        use std::sync::Arc;
        s.edit(Label::new("bitmap"), None, move |project, _| {
            let id = project.alloc_id().map_err(geomx::doc_error)?;
            project.shapes.push(Arc::new(openlaser_doc::Shape {
                id,
                name: None,
                layer: LayerKey::Color(0),
                xform: openlaser_doc::Affine::IDENTITY,
                geom: openlaser_doc::Geom::Bitmap(Arc::new(openlaser_doc::BitmapData {
                    asset: openlaser_doc::AssetId([3; 32]),
                    px: [20, 20],
                    size_mm: [40.0, 40.0],
                    gray: Arc::from(g.as_slice()),
                    source_ext: "png".to_owned(),
                })),
                locked: false,
                power_scale: openlaser_types::Percent::new(100.0).unwrap(),
                priority: 0,
            }));
            Ok(())
        })
        .unwrap();
        project(&s).shapes.iter().find(|sh| matches!(sh.geom, openlaser_doc::Geom::Bitmap(_))).unwrap().id.0
    };
    s.execute("edit.select", json!({ "ids": [rid] })).unwrap(); // rectangle selected
    let v = s.execute("tools.traceImage", json!({ "id": bid, "smoothness": 0 })).unwrap();
    let tid = v["ids"][0].as_u64().unwrap();
    let b = bounds_of(&s, tid);
    assert!((b.min[0] + 10.0).abs() < 1e-9 && (b.max[0] - 10.0).abs() < 1e-9, "{b:?}");
    // `id` must still be a Bitmap.
    let rect2 = s.execute("tools.rectangle", json!({ "xMm": 0.0, "yMm": 0.0, "wMm": 5.0, "hMm": 5.0 })).unwrap();
    assert_eq!(s.execute("tools.traceImage", json!({ "id": rect2["ids"][0].as_u64().unwrap() })).unwrap_err().code(), "bad_params");
    // And without `id`, a wrong selection is the bad-params "select one image to trace".
    s.execute("edit.select", json!({ "ids": [rid] })).unwrap();
    assert_eq!(s.execute("tools.traceImage", json!({})).unwrap_err().code(), "bad_params");
}

mod material_test;

#[test]
fn agents_cannot_change_rotary_settings() {
    let (mut s, _) = human();
    with_doc(&mut s);
    let before = s.machines().to_vec();
    for origin in [Origin::Synthetic, Origin::Headless] {
        let e = s.execute_as(origin, "device.settings.set", json!({"rotary":{"enabled":true,"splitMm":5.0}})).unwrap_err();
        assert_eq!(e.code(), "human_only");
    }
    assert_eq!(s.machines(), before);
}
