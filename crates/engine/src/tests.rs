//! Unit tests over the session internals (pending ARM, driver-event folding, history) and the WP6 acceptance tests
//! that run before the other packages' behaviour lands. Tests that need real plans, encoders or drivers live in
//! `tests/engine.rs` and are `#[ignore = "needs I1"]` until the integrator merges them.

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

use crate::command::{Access, EngineError, Kind, Origin, registry};
use crate::devices::{DeviceAccess, Event, PendingArm};
use crate::files::FsAuthority;
use crate::jobs::Encoded;
use crate::session::{Session, SessionConfig};

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
    let summary = crate::devices::tests::summary(fp, &machine.0, &[10.0]);
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
    let job = LmcJob { prelude: Vec::new(), lists: Vec::new(), repeats: 1, postlude: Vec::new(), fingerprint: fp, machine: machine.clone() };
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
