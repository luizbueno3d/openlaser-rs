//! I2: shipping app, engine, Human and SimProvider, with no token witnesses or emulated app state.
//! Kittest 0.36 omits raw_input_hook; every step below explicitly runs it, as native eframe does.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use eframe::App as _;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use openlaser_automation::{Bridge, BridgeClient, BridgeTarget, ClientInfo, TokenSource, ToolBackend, ToolOutput};
use openlaser_control::server::{ControlServer, ServerConfig};
use openlaser_engine::{DeviceAccess, Encoded, FsAuthority, JobOptions, MachineId, MachineState, Origin, Session, SessionConfig};
use openlaser_human::{CONFIRM_LABEL, Human, format};
use openlaser_lmc_sim::{Recording, SimConfig, SimProvider, SimWire};
use openlaser_types::{Clock, FrameOrigin, SystemClock};
use openlaser_ui_egui::serde_json::{Value, json};
use openlaser_ui_egui::{
    ArmModel, ArmOutcome, AutomationToggle, Clipboard, ControlInfo, DialogKind, DialogTicket, FileDialogs, HumanSurfaces, OpenLaserApp, Services,
    SettingsStore, UiSettings,
};

type Harness = egui_kittest::Harness<'static, OpenLaserApp>;

struct Memory;
impl FileDialogs for Memory {
    fn open(&mut self, _: DialogKind) -> DialogTicket {
        DialogTicket(0)
    }
    fn poll(&mut self, _: DialogTicket) -> Option<Option<(String, Vec<u8>)>> {
        None
    }
    fn save(&mut self, _: DialogKind, _: Vec<u8>) -> DialogTicket {
        DialogTicket(0)
    }
}
impl SettingsStore for Memory {
    fn load(&self) -> UiSettings {
        UiSettings { in_window_menus: true, ..UiSettings::default() }
    }
    fn save(&mut self, _: &UiSettings) {}
}
impl Clipboard for Memory {
    fn get(&mut self) -> Option<String> {
        None
    }
    fn set(&mut self, _: &str) {}
}
impl AutomationToggle for Memory {
    fn set(&mut self, _: bool) -> Result<Option<ControlInfo>, String> {
        Ok(None)
    }
    fn info(&self) -> Option<ControlInfo> {
        None
    }
}

/// Observation only: all decisions and tokens still come from the production Human and real mint.
/// The injection queue lets an adversarial test reach Human::inject even while the dispatcher refuses input.
struct ObservedHuman {
    human: Human,
    injected: Arc<Mutex<Vec<egui::Event>>>,
    armed: Arc<Mutex<usize>>,
}
impl HumanSurfaces for ObservedHuman {
    fn inject(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput, mut events: Vec<egui::Event>) {
        events.append(&mut self.injected.lock().unwrap());
        self.human.inject(ctx, raw, events);
    }
    fn frame_is_synthetic(&self, ctx: &egui::Context) -> bool {
        self.human.frame_is_synthetic(ctx)
    }
    fn is_human_click(&mut self, ctx: &egui::Context, r: &egui::Response) -> bool {
        self.human.is_human_click(ctx, r)
    }
    fn arm_dialog(&mut self, ctx: &egui::Context, m: &ArmModel) -> ArmOutcome {
        let outcome = self.human.arm_dialog(ctx, m);
        if matches!(outcome, ArmOutcome::Armed(_)) {
            *self.armed.lock().unwrap() += 1;
        }
        outcome
    }
    fn surface_open(&self) -> bool {
        self.human.surface_open()
    }
}

struct Rig {
    h: Harness,
    provider: Arc<SimProvider>,
    machine: MachineId,
    target: BridgeTarget,
    injected: Arc<Mutex<Vec<egui::Event>>>,
    armed: Arc<Mutex<usize>>,
    // Dropped before the app/session; sockets cannot outlive the rig.
    server: Option<ControlServer>,
}
impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            server.shutdown();
        }
    }
}
fn client_info() -> ClientInfo {
    ClientInfo { name: "I2 agent".into(), version: "1".into() }
}
fn card_path() -> std::path::PathBuf {
    openlaser_testkit::golden_dir().join("inputs").join("workspace.json")
}

fn rig(setup: impl FnOnce(&mut Session)) -> Rig {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let provider = Arc::new(SimProvider::new(SimConfig {
        buffer_depth: 3,
        transfer_latency: Duration::from_millis(2),
        status_extra_bits: 0,
        pid: 0x9899,
        faults: vec![],
        emission_cap_pct: None,
    }));
    let profile = openlaser_ui_egui::simulator_profile();
    let machine = profile.id.clone();
    let mut session = Session::new(SessionConfig {
        origin: Origin::Human,
        window: true,
        devices: DeviceAccess::Provider(provider.clone()),
        machines: vec![profile],
        profiles: None,
        camera_store: None,
        files: FsAuthority::TrustedLocal,
        clock: clock.clone(),
    });
    session.execute("file.new", json!({})).unwrap();
    setup(&mut session);
    let registry = session.stop_registry();
    let token = openlaser_control::token::Token::generate().unwrap();
    let secret = token.to_hex();
    let (server, requests) = ControlServer::start(ServerConfig {
        port: 0,
        token,
        fast_stop: Arc::new(move |machine| openlaser::control_runtime::fast_stop(&registry, machine)),
        wake: Arc::new(|| {}),
    })
    .unwrap();
    let target = BridgeTarget::Addr { addr: ([127, 0, 0, 1], server.port()).into(), token: TokenSource::Inline(secret) };
    let injected = Arc::new(Mutex::new(Vec::new()));
    let armed = Arc::new(Mutex::new(0));
    let services = Services {
        clock: clock.clone(),
        human: Box::new(ObservedHuman { human: Human::new(clock), injected: injected.clone(), armed: armed.clone() }),
        files: Box::new(Memory),
        settings: Box::new(Memory),
        clipboard: Box::new(Memory),
        automation: Box::new(Memory),
        quit: Box::new(|| {}),
    };
    let h = Harness::builder()
        .with_size(egui::vec2(1800.0, 1400.0))
        .with_step_dt(0.016)
        .build_eframe(move |cc| OpenLaserApp::new(cc, session, services).with_control(requests));
    let mut rig = Rig { h, provider, machine, target, injected, armed, server: Some(server) };
    settle(&mut rig);
    rig
}
fn card(s: &mut Session) {
    let result = s.execute("file.import", json!({"path": card_path()})).unwrap();
    assert_eq!(result["objects"].as_array().unwrap().len(), 403);
}
fn square(s: &mut Session) {
    s.execute("tools.drawLines", json!({"pointsMm": [[-5,-5],[5,-5],[5,5],[-5,5]], "closed": true})).unwrap();
}
fn step(r: &mut Rig) {
    let ctx = r.h.ctx.clone();
    let mut raw = std::mem::take(r.h.input_mut());
    r.h.state_mut().raw_input_hook(&ctx, &mut raw);
    *r.h.input_mut() = raw;
    r.h.step();
}
fn settle(r: &mut Rig) {
    // The production Human requires 300 ms of continuous visibility (real monotonic time).
    let until = Instant::now() + Duration::from_millis(340);
    while Instant::now() < until {
        step(r);
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
}
fn click_at(r: &mut Rig, pos: egui::Pos2) {
    assert!(r.h.ctx.content_rect().contains(pos), "human target {pos:?} is outside viewport {:?}", r.h.ctx.content_rect());
    r.h.input_mut().events.extend([egui::Event::PointerMoved(pos), pointer(pos, true)]);
    step(r);
    r.h.input_mut().events.push(pointer(pos, false));
    step(r);
}
fn click(r: &mut Rig, label: &str) {
    let node = r.h.get_by_role_and_label(Role::Button, label);
    assert!(!node.accesskit_node().is_disabled(), "{label} disabled: {}", r.h.state().ui_state().status_message);
    let pos = node.rect().center();
    click_at(r, pos);
}
fn wait(r: &mut Rig, what: &str, done: impl Fn(&Rig) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(180);
    while !done(r) {
        assert!(Instant::now() < deadline, "timeout: {what}; {}", r.h.state().ui_state().status_message);
        step(r);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn connect(r: &mut Rig) {
    click(r, "Connect");
    wait(r, "connect", |r| r.h.state().session().status(&r.machine).is_some_and(|s| s.connected && matches!(s.state, MachineState::Idle)));
    settle(r);
}
fn frame(r: &mut Rig) {
    click(r, "Frame");
    wait(r, "human frame receipt", |r| r.h.state().session().status(&r.machine).is_some_and(|s| s.receipt.is_some() && !s.busy()));
    let receipt = r.h.state().session().status(&r.machine).unwrap().receipt.as_ref().unwrap();
    assert_eq!(receipt.origin, FrameOrigin::Human);
    assert!(receipt.full_passes >= 1 && receipt.covers_job());
    settle(r);
}
fn start(r: &mut Rig) {
    click(r, "Start");
    assert!(r.h.state().session().pending_arm().is_some(), "{}", r.h.state().ui_state().status_message);
    settle(r);
}
fn fill_arm(r: &mut Rig) {
    let checklist = r.h.state().session().pending_arm().unwrap().checklist.clone();
    assert!(!checklist.is_empty());
    for label in checklist {
        let pos = r.h.get_by_role_and_label(Role::CheckBox, &label).rect().center();
        click_at(r, pos);
    }
    let pos = r.h.get_by_role(Role::TextInput).rect().center();
    click_at(r, pos);
    r.h.input_mut().events.push(egui::Event::Text("ARM".into()));
    step(r);
    assert_eq!(r.h.get_by_role(Role::TextInput).value().as_deref(), Some("ARM"));
}
fn arm(r: &mut Rig) {
    fill_arm(r);
    let before = *r.armed.lock().unwrap();
    click(r, CONFIRM_LABEL);
    assert_eq!(*r.armed.lock().unwrap(), before + 1, "real Human must return exactly one ArmToken");
    assert!(r.h.state().session().pending_arm().is_none());
}
fn recording(r: &Rig) -> Recording {
    r.provider.last_recorder().unwrap().snapshot()
}
fn finish(r: &mut Rig) {
    wait(r, "marking completed", |r| r.h.state().session().status(&r.machine).is_some_and(|s| !s.busy()) && !recording(r).burned.is_empty());
    let rec = recording(r);
    assert!(!rec.mo && rec.port == 0 && rec.writes_while_not_ready == 0, "{rec:?}");
}
fn assert_burn(r: &Rig, expected: &[[[f64; 2]; 2]]) {
    let rec = recording(r);
    assert_eq!(rec.burned.len(), expected.len());
    let unit = 236.68 / 65535.0;
    for (got, want) in rec.burned.iter().zip(expected) {
        for k in 0..2 {
            assert!((got.a[k] - want[0][k]).abs() <= unit && (got.b[k] - want[1][k]).abs() <= unit, "{got:?} != {want:?}");
        }
    }
}
fn plan_segments(r: &Rig) -> Vec<[[f64; 2]; 2]> {
    r.h.state()
        .session()
        .build_job(None, &JobOptions::default())
        .unwrap()
        .plan
        .sections
        .iter()
        .flat_map(|s| s.paths.iter().flat_map(|p| p.0.windows(2).map(|w| [w[0], w[1]])))
        .collect()
}

#[test]
fn kittest_business_card_flow() {
    let mut r = rig(card);
    let project = &r.h.state().session().active().unwrap().project;
    assert_eq!(project.shapes.len(), 403);
    assert!(project.layers.colors.iter().any(|l| l.output));
    connect(&mut r);
    frame(&mut r);
    let expected = plan_segments(&r);
    start(&mut r);
    let pending = r.h.state().session().pending_arm().unwrap();
    assert_eq!(pending.summary.bytes_hash, pending.job.bytes_hash());
    let built = r.h.state().session().build_job(None, &JobOptions::default()).unwrap();
    let Encoded::Lmc { job, .. } = &built.encoded else { panic!("LMC job") };
    let openlaser_machine::Kinematics::Galvo(machine) = openlaser_ui_egui::simulator_profile().kind else { panic!("galvo") };
    let decoded = openlaser_lmc::emission_summary(job, &machine, &built.plan, built.validated.notices()).unwrap();
    assert_eq!(pending.summary, decoded);
    for row in &decoded.rows {
        for value in [format::power(row.power_pct), format::frequency(row.frequency_khz), format::pulse(row.pulse_ns), format::speed(row.speed_mm_s)] {
            assert!(r.h.query_all_by_label(&value).next().is_some(), "ARM must show decoded {value}");
        }
    }
    arm(&mut r);
    finish(&mut r);
    assert_burn(&r, &expected);

    settle(&mut r);
    frame(&mut r);
    start(&mut r);
    let before = recording(&r).burned.len();
    arm(&mut r);
    wait(&mut r, "second run marking", |r| recording(r).burned.len() > before);
    let started = Instant::now();
    r.h.input_mut().events.extend(openlaser_ui_egui::control::input::events_for("ui.key", &json!({"key":"Escape"})).unwrap());
    step(&mut r);
    wait(&mut r, "Esc stop report", |r| r.h.state().session().status(&r.machine).unwrap().last_stop.is_some());
    assert!(started.elapsed() <= Duration::from_millis(100), "Esc stop took {:?}", started.elapsed());
    assert_stopped(&r);
}

/// Runs the blocking bridge client off the UI thread while the real app drains the server queue.
fn remote<T: Send + 'static>(r: &mut Rig, f: impl FnOnce(BridgeTarget) -> T + Send + 'static) -> T {
    let target = r.target.clone();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || tx.send(f(target)).unwrap());
    let deadline = Instant::now() + Duration::from_secs(30);
    let result = loop {
        if let Ok(result) = rx.try_recv() {
            break result;
        }
        assert!(Instant::now() < deadline, "bridge call timed out");
        step(r);
        std::thread::sleep(Duration::from_millis(2));
    };
    worker.join().unwrap();
    result
}
fn tool(r: &mut Rig, name: &'static str, params: Value) -> Value {
    remote(r, move |target| match Bridge::new(target, &client_info()).call(name, params, &client_info()) {
        ToolOutput::Json(v) => v,
        ToolOutput::Error { message, data } => panic!("{name}: {message}: {data:?}"),
        ToolOutput::Png { .. } => panic!("{name}: unexpected PNG"),
    })
}
#[test]
fn mcp_bridge_end_to_end() {
    let mut r = rig(|_| {});
    let imported = tool(&mut r, "doc_import", json!({"path": card_path()}));
    assert_eq!(imported["objects"].as_array().unwrap().len(), 403);
    tool(&mut r, "command_run", json!({"id":"edit.selectAll"}));
    let planned = tool(&mut r, "job_plan", json!({}));
    assert_eq!(planned["ok"], true, "{planned}");
    let submitted = tool(&mut r, "job_submit", json!({"note":"I2 business card"}));
    assert_eq!(submitted["state"], "awaiting_human", "{submitted}");
    step(&mut r);
    assert!(r.h.query_by_label("Job from I2 agent 1").is_some());
    connect(&mut r);
    click(&mut r, "Approve framing");
    wait(&mut r, "submission frame", |r| r.h.state().session().status(&r.machine).is_some_and(|s| s.receipt.is_some() && !s.busy()));
    settle(&mut r);
    start(&mut r);
    arm(&mut r);
    finish(&mut r);
    let status = remote(&mut r, move |target| {
        BridgeClient::connect(&target, &client_info()).unwrap().call_read("job.status", json!({"submission":submitted["submission"]})).unwrap()
    });
    assert_eq!(status["submissions"][0]["state"], "finished", "{status}");
}

#[test]
fn frame_request_without_approval_sends_no_lighting_packets() {
    let mut r = rig(square);
    connect(&mut r);
    let before = recording(&r).wire.len();
    let requested = tool(&mut r, "frame_request", json!({"mode":"once"}));
    assert_eq!(requested["state"], "awaiting_human");
    settle(&mut r);
    let rec = recording(&r);
    assert!(rec.lit.is_empty() && rec.burned.is_empty());
    assert!(!rec.wire[before..].iter().any(|w| matches!(w, SimWire::Packet { .. })));
    assert!(r.h.state().session().status(&r.machine).unwrap().receipt.is_none());
    assert_eq!(*r.armed.lock().unwrap(), 0);
}
fn assert_stopped(r: &Rig) {
    let report = r.h.state().session().status(&r.machine).unwrap().last_stop.as_ref().unwrap();
    assert!(report.confirmed && report.mo_closed && report.ports_zero, "{report:?}");
    let rec = recording(r);
    assert!(!rec.mo && rec.port == 0);
}
#[test]
fn mcp_stop_latches_while_ui_thread_blocked() {
    let mut r = rig(card);
    connect(&mut r);
    frame(&mut r);
    start(&mut r);
    arm(&mut r);
    wait(&mut r, "marking", |r| !recording(r).burned.is_empty());
    let client = BridgeClient::connect(&r.target, &client_info()).unwrap();
    let stop = client.stop_handle().unwrap();
    // Queue a normal call and leave it unanswered. No app frames run until STOP's reply arrives.
    let target = r.target.clone();
    let queued = std::thread::spawn(move || BridgeClient::connect(&target, &client_info()).unwrap().call_read("doc.inspect", json!({})));
    std::thread::sleep(Duration::from_millis(30));
    let before = r.h.ctx.cumulative_pass_nr();
    let started = Instant::now();
    let ToolOutput::Json(report) = stop.stop(None) else { panic!("stop refused") };
    assert_eq!(r.h.ctx.cumulative_pass_nr(), before);
    assert!(started.elapsed() < Duration::from_millis(100), "stop: {:?}", started.elapsed());
    assert_eq!(report["latched"], true);
    assert_eq!(report["confirmed"], true);
    assert!(!recording(&r).mo && recording(&r).port == 0);
    step(&mut r);
    wait(&mut r, "StopReport", |r| r.h.state().session().status(&r.machine).unwrap().last_stop.is_some());
    assert_stopped(&r);
    wait(&mut r, "queued read", |_| queued.is_finished());
    queued.join().unwrap().unwrap();
}
#[test]
fn stop_served_while_human_surface_open() {
    let mut r = rig(square);
    connect(&mut r);
    frame(&mut r);
    start(&mut r);
    let refused = remote(&mut r, |target| {
        BridgeClient::connect(&target, &client_info()).unwrap().call("engine.execute", json!({"command":"edit.selectAll"})).unwrap_err()
    });
    assert!(format!("{refused:?}").contains("human_confirmation_open"), "{refused:?}");
    let report = remote(&mut r, |target| BridgeClient::connect(&target, &client_info()).unwrap().call("machine.stop", json!({})).unwrap());
    assert_eq!(report["latched"], true);
    assert_eq!(report["confirmed"], true);
    wait(&mut r, "stop", |r| r.h.state().session().status(&r.machine).unwrap().last_stop.is_some());
    assert_stopped(&r);
    assert!(r.h.state().session().pending_arm().is_none());
    assert_eq!(*r.armed.lock().unwrap(), 0);
}
fn inject_click(r: &mut Rig, pos: egui::Pos2) {
    r.injected.lock().unwrap().extend([egui::Event::PointerMoved(pos), pointer(pos, true)]);
    step(r);
    r.injected.lock().unwrap().push(pointer(pos, false));
    step(r);
}
fn clear_taint(r: &mut Rig) {
    let until = Instant::now() + Duration::from_millis(1100);
    while Instant::now() < until {
        step(r);
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn synthetic_input_cannot_arm_end_to_end() {
    let mut r = rig(square);
    connect(&mut r);
    let baseline = recording(&r).wire.len();
    let pos = r.h.get_by_role_and_label(Role::Button, "Frame").rect().center();
    inject_click(&mut r, pos);
    settle(&mut r);
    assert!(r.h.state().session().status(&r.machine).unwrap().receipt.is_none());
    let pos = r.h.get_by_role_and_label(Role::Button, "Start").rect().center();
    inject_click(&mut r, pos);
    assert!(r.h.state().session().pending_arm().is_none());
    assert!(!recording(&r).wire[baseline..].iter().any(|w| matches!(w, SimWire::Packet { .. })));
    clear_taint(&mut r);
    frame(&mut r); // positive control: an actual person's click still produces the receipt.
    let pos = r.h.get_by_role_and_label(Role::Button, "Start").rect().center();
    inject_click(&mut r, pos);
    assert!(r.h.state().session().pending_arm().is_none(), "synthetic Start with a valid receipt");
    clear_taint(&mut r);
    start(&mut r);
    let checklist = r.h.state().session().pending_arm().unwrap().checklist.clone();
    for label in checklist {
        let pos = r.h.get_by_role_and_label(Role::CheckBox, &label).rect().center();
        inject_click(&mut r, pos);
    }
    let pos = r.h.get_by_role(Role::TextInput).rect().center();
    inject_click(&mut r, pos);
    r.injected.lock().unwrap().push(egui::Event::Text("ARM".into()));
    step(&mut r);
    let pos = r.h.get_by_role_and_label(Role::Button, CONFIRM_LABEL).rect().center();
    inject_click(&mut r, pos);
    clear_taint(&mut r);
    assert_eq!(r.h.get_by_role(Role::TextInput).value().unwrap_or_default(), "");
    assert_eq!(*r.armed.lock().unwrap(), 0);
    assert!(recording(&r).burned.is_empty());
    // Even an otherwise completed human form cannot be confirmed by automation.
    fill_arm(&mut r);
    let pos = r.h.get_by_role_and_label(Role::Button, CONFIRM_LABEL).rect().center();
    inject_click(&mut r, pos);
    assert_eq!(*r.armed.lock().unwrap(), 0);
    assert!(recording(&r).burned.is_empty());
    clear_taint(&mut r);
    arm(&mut r); // same real gate remains usable after the attack.
    finish(&mut r);
}

#[test]
fn grouped_shape_lands_where_designed() {
    let mut r = rig(|s| {
        // Three segments create two levels of real document groups via commands, not pre-flattened SVG.
        s.execute("tools.drawLines", json!({"pointsMm":[[0,0],[4,0],[4,2]],"closed":false})).unwrap();
        s.execute("tools.drawLines", json!({"pointsMm":[[0,2],[1,2]],"closed":false})).unwrap();
        s.execute("edit.selectAll", json!({})).unwrap();
        s.execute("arrange.group", json!({})).unwrap();
        s.execute("arrange.transform", json!({"scalePct":200,"anchor":"bottomLeft"})).unwrap();
        s.execute("arrange.rotateBy", json!({"deg":90,"anchor":"bottomLeft"})).unwrap();
        s.execute("tools.drawLines", json!({"pointsMm":[[0,0],[1,0]],"closed":false})).unwrap();
        s.execute("edit.selectAll", json!({})).unwrap();
        s.execute("arrange.group", json!({})).unwrap();
        s.execute("arrange.nudge", json!({"dxMm":30,"dyMm":20})).unwrap();
    });
    let project = &r.h.state().session().active().unwrap().project;
    let openlaser_doc::Geom::Group(outer) = &project.shapes[0].geom else { panic!("outer group") };
    assert!(outer.iter().any(|s| matches!(s.geom, openlaser_doc::Geom::Group(_))));
    // Independently composed: (x,y) -> (30 - 2*y, 20 + 2*x).
    let expected = [[[30., 20.], [30., 28.]], [[30., 28.], [26., 28.]], [[26., 20.], [26., 22.]], [[30., 20.], [31., 20.]]];
    let planned = plan_segments(&r);
    assert_eq!(planned.len(), expected.len());
    for (got, want) in planned.iter().zip(expected) {
        for k in 0..2 {
            for axis in 0..2 {
                assert!((got[k][axis] - want[k][axis]).abs() < 1e-9, "{got:?} != {want:?}");
            }
        }
    }
    connect(&mut r);
    frame(&mut r);
    start(&mut r);
    arm(&mut r);
    finish(&mut r);
    assert_burn(&r, &expected);
}

#[test]
fn image_summary_uses_the_human_image_label() {
    assert_eq!(format::mode(openlaser_types::SectionMode::Image), "Image");
}
