//! Whole-window kittests (SPEC §6.4, §7.9 acceptance). They use `NoHuman` (nothing is human, every frame is
//! synthetic, a human-only surface counts as open) unless a test says why it needs another double. The WP0 engine has
//! no documents, plan or driver yet, so the app's test seam (`Seam`) stands in for the engine's state; tests that
//! need the real engine round trip (edit, undo, safety) are `#[ignore = "needs I2"]` and say which piece.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use eframe::App as _;
use egui_kittest::kittest::{NodeT, Queryable};
use openlaser_engine::{DeviceAccess, FsAuthority, MachineState, Origin, SessionConfig};
use openlaser_types::{Clock, LayerKey, SystemClock};

use super::*;
use crate::services::{ControlInfo, DialogKind, DialogTicket, FileDialogs, SettingsStore, UiSettings};

type H = egui_kittest::Harness<'static, OpenLaserApp>;

// ---- doubles ----------------------------------------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Files(Arc<AtomicUsize>);
impl FileDialogs for Files {
    fn open(&mut self, _kind: DialogKind) -> DialogTicket {
        DialogTicket(self.0.fetch_add(1, Ordering::SeqCst) as u64 + 1)
    }
    fn poll(&mut self, _ticket: DialogTicket) -> Option<Option<(String, Vec<u8>)>> {
        None
    }
    fn save(&mut self, _kind: DialogKind, _bytes: Vec<u8>) -> DialogTicket {
        DialogTicket(0)
    }
}

#[derive(Clone)]
struct Settings(Arc<std::sync::Mutex<UiSettings>>);
impl SettingsStore for Settings {
    fn load(&self) -> UiSettings {
        self.0.lock().map(|s| s.clone()).unwrap_or_default()
    }
    fn save(&mut self, value: &UiSettings) {
        if let Ok(mut s) = self.0.lock() {
            *s = value.clone();
        }
    }
}

struct Clip;
impl crate::Clipboard for Clip {
    fn get(&mut self) -> Option<String> {
        None
    }
    fn set(&mut self, _text: &str) {}
}

struct Automation;
impl crate::AutomationToggle for Automation {
    fn set(&mut self, _on: bool) -> Result<Option<ControlInfo>, String> {
        Ok(None)
    }
    fn info(&self) -> Option<ControlInfo> {
        None
    }
}

/// NoHuman without the fail-closed "a surface is open": needed where a test must reach the not-refused path (a
/// mutating control method when idle, the toolbar STOP when nothing else is up). Still never human.
struct NoSurface;
impl crate::HumanSurfaces for NoSurface {
    fn inject(&mut self, _ctx: &egui::Context, _raw: &mut egui::RawInput, _events: Vec<egui::Event>) {}
    fn frame_is_synthetic(&self, _ctx: &egui::Context) -> bool {
        true
    }
    fn is_human_click(&mut self, _ctx: &egui::Context, _r: &egui::Response) -> bool {
        false
    }
    fn arm_dialog(&mut self, _ctx: &egui::Context, _m: &crate::ArmModel) -> crate::ArmOutcome {
        crate::ArmOutcome::Cancelled
    }
    fn surface_open(&self) -> bool {
        false
    }
}

/// An adversarial surface: it injects queued events and calls ANY click "human". The app must still refuse to act in
/// a synthetic frame (it checks `frame_is_synthetic` itself).
struct SyntheticInjector;
impl crate::HumanSurfaces for SyntheticInjector {
    fn inject(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput, events: Vec<egui::Event>) {
        raw.events.extend(events);
    }
    fn frame_is_synthetic(&self, _ctx: &egui::Context) -> bool {
        true
    }
    fn is_human_click(&mut self, _ctx: &egui::Context, r: &egui::Response) -> bool {
        r.clicked()
    }
    fn arm_dialog(&mut self, _ctx: &egui::Context, _m: &crate::ArmModel) -> crate::ArmOutcome {
        crate::ArmOutcome::Cancelled
    }
    fn surface_open(&self) -> bool {
        false
    }
}

/// Frames are human (the harness's own events): shows a path is otherwise live, so a refusal test is not vacuous.
struct HumanFrames;
impl crate::HumanSurfaces for HumanFrames {
    fn inject(&mut self, _ctx: &egui::Context, _raw: &mut egui::RawInput, _events: Vec<egui::Event>) {}
    fn frame_is_synthetic(&self, _ctx: &egui::Context) -> bool {
        false
    }
    fn is_human_click(&mut self, _ctx: &egui::Context, r: &egui::Response) -> bool {
        r.clicked_by(egui::PointerButton::Primary)
    }
    fn arm_dialog(&mut self, _ctx: &egui::Context, _m: &crate::ArmModel) -> crate::ArmOutcome {
        crate::ArmOutcome::Open
    }
    fn surface_open(&self) -> bool {
        false
    }
}

// ---- harness ------------------------------------------------------------------------------------------------------------

struct Options {
    size: egui::Vec2,
    ppp: f32,
    human: Box<dyn crate::HumanSurfaces>,
    machines: Vec<openlaser_engine::MachineProfile>,
    settings: UiSettings,
    files: Files,
    quit: Box<dyn Fn() + Send>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            size: egui::vec2(1440.0, 900.0),
            ppp: 1.0,
            human: Box::new(crate::NoHuman),
            machines: vec![openlaser_machine::jpt_m7_200_sim()],
            settings: UiSettings { in_window_menus: true, ..UiSettings::default() },
            files: Files::default(),
            quit: Box::new(|| {}),
        }
    }
}

fn harness_with(options: Options) -> H {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let session = Session::new(SessionConfig {
        origin: Origin::Human,
        window: true,
        devices: DeviceAccess::None,
        machines: options.machines,
        profiles: None,
        files: FsAuthority::Denied,
        clock: Arc::clone(&clock),
    });
    let services = Services {
        clock,
        human: options.human,
        files: Box::new(options.files),
        settings: Box::new(Settings(Arc::new(std::sync::Mutex::new(options.settings)))),
        clipboard: Box::new(Clip),
        automation: Box::new(Automation),
        quit: options.quit,
    };
    egui_kittest::Harness::builder()
        .with_size(options.size)
        .with_pixels_per_point(options.ppp)
        .with_step_dt(0.05)
        .with_max_steps(400)
        .build_eframe(move |cc| OpenLaserApp::new(cc, session, services))
}

fn harness() -> H {
    harness_with(Options::default())
}

fn with_human(human: impl crate::HumanSurfaces + 'static) -> H {
    harness_with(Options { human: Box::new(human), ..Options::default() })
}

/// One eframe frame as the native loop runs it: `raw_input_hook` first, then `logic` + `ui`.
fn hooked_step(h: &mut H, events: Vec<egui::Event>) {
    let ctx = h.ctx.clone();
    let mut raw = std::mem::take(h.input_mut());
    raw.events.extend(events);
    h.state_mut().raw_input_hook(&ctx, &mut raw);
    *h.input_mut() = raw;
    h.step();
}

fn esc() -> Vec<egui::Event> {
    crate::control::input::events_for("ui.key", &serde_json::json!({ "key": "Escape" })).unwrap()
}

fn layer(index: u8, defaults: &openlaser_doc::LayerDefaults) -> openlaser_doc::ColorLayer {
    let op = openlaser_doc::CutOp {
        name: "Line".into(),
        enabled: true,
        mode: openlaser_doc::CutMode::Line,
        passes: 1,
        galvo: defaults.galvo.clone(),
        gantry: defaults.gantry.clone(),
        fill: defaults.fill.clone(),
    };
    openlaser_doc::ColorLayer {
        index,
        name: format!("C{index:02}"),
        output: true,
        show: true,
        ops: vec![op],
        global_passes: 1,
        air_assist: false,
        image: openlaser_doc::ImageCut {
            mode: openlaser_doc::ImageMode::Threshold,
            interval: defaults.fill.interval,
            angle: defaults.fill.angle,
            bidirectional: true,
            negative: false,
            threshold: 128,
            passes: 1,
            galvo: defaults.galvo.clone(),
            gantry: defaults.gantry.clone(),
        },
    }
}

/// A project with all 30 colour layers, one rectangle on C00 and one on T1.
fn used_project() -> Arc<openlaser_doc::Project> {
    let defaults = openlaser_machine::jpt_m7_200_sim().layer_defaults;
    let mut project = openlaser_doc::Project::new(
        openlaser_doc::Workspace { kind: openlaser_doc::DeviceKind::GalvoLmc, size_mm: [200.0, 200.0], origin: openlaser_doc::WorkspaceOrigin::Center },
        &defaults,
    );
    project.layers.colors = (0..30).map(|i| layer(i, &defaults)).collect();
    for (id, key) in [(1, LayerKey::Color(0)), (2, LayerKey::Tool(1))] {
        project.shapes.push(Arc::new(openlaser_doc::Shape {
            id: openlaser_types::ObjectId(id),
            name: None,
            layer: key,
            xform: Default::default(),
            geom: openlaser_doc::Geom::Rect {
                w: openlaser_types::Mm::new(10.0).unwrap(),
                h: openlaser_types::Mm::new(10.0).unwrap(),
                corner_radius: openlaser_types::Mm::new(0.0).unwrap(),
            },
            locked: false,
            power_scale: openlaser_types::Percent::new(100.0).unwrap(),
            priority: 0,
        }));
    }
    Arc::new(project)
}

fn summary(rows: Vec<openlaser_types::PassRow>) -> openlaser_engine::EmissionSummary {
    openlaser_engine::EmissionSummary {
        fingerprint: openlaser_types::Fingerprint([7; 32]),
        bytes_hash: openlaser_types::Fingerprint([8; 32]),
        machine: openlaser_machine::jpt_m7_200_sim().id,
        rows,
        notices: Vec::new(),
        repeats: 1,
        estimate: std::time::Duration::from_secs(42),
        crash_bound: std::time::Duration::from_secs(2),
        packets: 3,
        work_area: openlaser_types::Bounds { min: [-100.0; 2], max: [100.0; 2] },
        negate: [false; 2],
        swap_xy: false,
    }
}

fn pass_row(key: LayerKey, actual: f64) -> openlaser_types::PassRow {
    openlaser_types::PassRow {
        layer: key,
        layer_name: key.to_string(),
        sub: 0,
        pass: 1,
        of: 1,
        mode: openlaser_types::SectionMode::Line,
        power_pct: 29.99,
        frequency_khz: Some(29.99),
        pulse_ns: Some(200),
        speed_mm_s: actual,
        galvo: None,
        paths: 1,
        mark_mm: 40.0,
        bounds: openlaser_types::Bounds { min: [0.0; 2], max: [10.0; 2] },
        estimate: std::time::Duration::from_secs(3),
    }
}

fn ready_plan(h: &mut H) -> openlaser_engine::EmissionSummary {
    let s = summary(vec![pass_row(LayerKey::Color(0), 996.8)]);
    h.state_mut().seam.plan = Some(PlanView::Ready(Box::new(s.clone())));
    s
}

fn fresh_receipt(h: &H, s: &openlaser_engine::EmissionSummary) -> openlaser_types::FrameReceipt {
    openlaser_types::FrameReceipt {
        fingerprint: s.fingerprint,
        machine: s.machine.clone(),
        session: h.state().session.id(),
        full_passes: 1,
        at: h.state().services.clock.now(),
        style: openlaser_types::FrameStyle::Contour,
        source: openlaser_types::FrameSource::Job,
        origin: openlaser_types::FrameOrigin::Human,
    }
}

fn refusal_plan(h: &mut H) {
    h.state_mut().seam.plan = Some(PlanView::Refused(Box::new(openlaser_engine::JobRefused {
        fingerprint: None,
        refusals: vec![
            openlaser_types::Refusal::new(openlaser_types::RefusalCode::PulseNotInTable, "pulse 7 ns is not in the JPT M7 pulse table")
                .layer(LayerKey::Color(0)),
        ],
        notices: Vec::new(),
        error: None,
    })));
}

fn has_text(h: &H, text: &str) -> bool {
    h.query_all_by_label_contains(text).next().is_some()
}

/// A node whose label starts with `text` (tooltips are exactly the reason; other labels may quote it later).
fn has_prefix(h: &H, text: &str) -> bool {
    h.root().children_recursive().any(|n| {
        let node = n.accesskit_node();
        // AccessKit keeps a Label's text in `value` (as kittest's own label queries do).
        let label = if node.role() == egui::accesskit::Role::Label { node.value() } else { node.label() };
        label.is_some_and(|l| l.starts_with(text))
    })
}

fn hover_shows(h: &mut H, label: &str, reason: &str) {
    assert!(!has_prefix(h, reason), "`{reason}` is visible before hovering `{label}`");
    h.get_by_label(label).hover();
    h.run_steps(20);
    assert!(has_prefix(h, reason), "hovering `{label}` did not show `{reason}`");
    h.hover_at(egui::pos2(1.0, 1.0));
    h.run_steps(5);
}

fn control(h: &mut H, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let (tx, _) = std::sync::mpsc::channel();
    let ctx = h.ctx.clone();
    let request = ControlRequest {
        conn: 1,
        client: "test".into(),
        request: openlaser_control::wire::Request { id: serde_json::json!(1), method: method.into(), params },
        reply: tx,
        deadline: std::time::Instant::now() + std::time::Duration::from_secs(1),
        dispatch: Arc::new(Default::default()),
    };
    match crate::control::handle(h.state_mut(), &ctx, request) {
        crate::control::Outcome::Done(reply) | crate::control::Outcome::AfterInput(reply) => {
            if reply.ok {
                Ok(reply.result.unwrap_or_default())
            } else {
                Err(reply.error.unwrap_or_default())
            }
        }
        crate::control::Outcome::Screenshot { .. } => Ok(serde_json::json!({ "screenshot": "pending" })),
    }
}

// ---- layout ----------------------------------------------------------------------------------------------------------------

#[test]
fn kittest_lightburn_regions_present() {
    let mut h = harness();
    h.run();
    for label in [
        // menu bar
        "File",
        "Edit",
        "Tools",
        "Arrange",
        "Laser Tools",
        "Window",
        "Language",
        "Help",
        // main toolbar
        "New",
        "Open",
        "Import",
        "Save",
        "Undo",
        "Redo",
        "Cut",
        "Copy",
        "Paste",
        "Zoom In",
        "Zoom to Page",
        "Settings",
        "Device Settings",
        "Preview",
        // numeric edits
        "XPos",
        "YPos",
        "Width",
        "Height",
        "W%",
        "H%",
        "Rotate",
        "Lock aspect ratio",
        "Anchor top-left",
        // left tools
        "Select",
        "Rectangle",
        "Ellipse",
        "Measure",
        // dock: Cuts / Layers and the Laser / Library / Move / Console tab group
        "Cuts / Layers",
        "Laser",
        "Library",
        "Move",
        "Console",
        // canvas
        "Zoom to fit",
        // Laser panel
        "Devices…",
        "Connect",
        "Pause",
        "Stop",
        "Start",
        "Frame",
        "Send",
        "Home",
        "Go to Origin",
        "Cut Selected Graphics",
        "Repeat count",
        // Arrange toolbar and palette
        "Group",
        "Flip Horizontal",
        "Layer 00",
        "Layer 29",
        "T1",
        "T2",
        // STOP (the overlay is up: NoHuman reports a human-only surface open)
        "Stop laser",
        "Stop laser (toolbar)",
    ] {
        assert!(h.query_all_by_label(label).next().is_some(), "missing region or control: {label}");
    }
    assert!(has_text(&h, "agent: none"), "status bar agent indicator");
    assert!(has_text(&h, "JPT M7"), "machine chip");
}

#[test]
fn kittest_palette_has_30_plus_tools() {
    for width in [1440.0, 1024.0, 760.0] {
        let mut h = harness_with(Options { size: egui::vec2(width, 700.0), ..Options::default() });
        h.run();
        let mut previous = f32::NEG_INFINITY;
        let expected: Vec<String> = (0..30).map(|n| format!("Layer {n:02}")).chain(["T1".to_owned(), "T2".to_owned()]).collect();
        for label in expected {
            let rect = h.get_by_label(&label).rect();
            assert!(rect.left() >= 0.0 && rect.right() <= width, "{label} is off-window at {width}: {rect:?}");
            assert!(rect.width() >= 12.0 && rect.height() >= 20.0, "{label} too small at {width}: {rect:?}");
            assert!(rect.left() > previous, "{label} is out of order at {width}");
            previous = rect.left();
        }
    }
}

#[test]
fn kittest_palette_click_assigns_selection() {
    let mut h = harness();
    h.state_mut().seam.project = Some(used_project());
    h.state_mut().seam.selection = vec![openlaser_types::ObjectId(1)];
    h.run();
    h.get_by_label("Layer 05").click();
    h.run();
    let last = h.state().seam.dispatched.last().cloned().unwrap();
    assert_eq!(last.1, "layer.assign");
    assert_eq!(last.2, serde_json::json!({ "layer": "C05", "ids": [1] }));
    assert_eq!(h.state().ui.active_layer, LayerKey::Color(5));
    h.state_mut().seam.selection.clear();
    h.get_by_label("T2").click();
    h.run();
    let last = h.state().seam.dispatched.last().cloned().unwrap();
    assert_eq!((last.1.as_str(), last.2), ("layer.setCurrent", serde_json::json!({ "layer": "T2" })));
}

#[test]
#[ignore = "needs I2: WP6's layer.assign and undo (the WP0 engine has no documents)"]
fn kittest_palette_click_assigns_selection_end_to_end() {
    let mut h = harness();
    h.state_mut().session.execute_as(Origin::Synthetic, "file.new", serde_json::json!({})).unwrap();
    h.state_mut().session.execute_as(Origin::Synthetic, "tools.rectangle", serde_json::json!({ "rectMm": [[0, 0], [10, 10]] })).unwrap();
    h.run();
    h.get_by_label("Layer 05").click();
    h.run();
    assert_eq!(h.state().session.active().unwrap().project.shapes[0].layer, LayerKey::Color(5));
    h.state_mut().session.undo().unwrap();
    assert_eq!(h.state().session.active().unwrap().project.shapes[0].layer, LayerKey::Color(0));
}

fn header_order(h: &H) -> Vec<String> {
    let mut headers: Vec<(f32, String)> = ["#", "Layer", "Mode", "Spd/Pwr", "Output", "Show", "Air"]
        .iter()
        .filter_map(|label| h.query_all_by_label(label).next().map(|n| (n.rect().left(), (*label).to_owned())))
        .collect();
    headers.sort_by(|a, b| a.0.total_cmp(&b.0));
    headers.into_iter().map(|(_, l)| l).collect()
}

#[test]
fn kittest_cuts_columns_and_actual_speed() {
    let mut h = harness();
    h.state_mut().seam.project = Some(used_project());
    h.run();
    assert_eq!(header_order(&h), ["#", "Layer", "Mode", "Spd/Pwr", "Output", "Show"], "galvo columns, in LightBurn's order, no Air");
    // Only layers in use are listed; the # cell is a usable target; Output/Show are real checkboxes.
    assert!(h.query_by_label("Cut layer C00").is_some() && h.query_by_label("Cut layer T1").is_some());
    assert!(h.query_by_label("Cut layer C01").is_none(), "an unused layer is listed");
    let target = h.get_by_label("Cut layer C00").rect();
    assert!(target.width() >= 24.0 && target.height() >= 24.0, "{target:?}");
    for label in ["Output C00", "Show C00", "Show T1"] {
        let node = h.get_by_label(label);
        assert_eq!(node.accesskit_node().role(), egui::accesskit::Role::CheckBox, "{label}");
        assert!(node.accesskit_node().toggled().is_some(), "{label}");
    }
    assert!(h.get_by_label("Output T1").accesskit_node().is_disabled(), "tool layers never output");
    // No plan: the requested speed is shown and is never called actual.
    assert!(!has_text(&h, "(actual)"), "a requested speed is labelled actual");
    assert!(has_text(&h, "mm/s requested"));
    // With a plan: the decoded actual speed, labelled.
    ready_plan(&mut h);
    h.run();
    assert!(has_text(&h, "996.80 mm/s (actual)"), "quick editor shows the actual speed");
    assert!(has_text(&h, "996.8 /"), "Spd/Pwr shows the actual speed");
    // Clicking Output dispatches the command for that layer.
    h.get_by_label("Output C00").click();
    h.run();
    let last = h.state().seam.dispatched.last().cloned().unwrap();
    assert_eq!((last.1.as_str(), last.2), ("layer.setOutput", serde_json::json!({ "layer": "C00", "on": false })));
}

fn gantry_profile() -> openlaser_engine::MachineProfile {
    let mut profile = openlaser_machine::jpt_m7_200_sim();
    profile.id = openlaser_types::MachineId("om-diode".into());
    profile.name = "Diode · G-code".into();
    profile.kind = openlaser_machine::Kinematics::Gantry(openlaser_machine::GantryMachine {
        bed_mm: [400.0, 400.0],
        origin: openlaser_doc::WorkspaceOrigin::BottomLeft,
        max_speed: openlaser_types::MmPerS::new(6000.0).unwrap(),
        travel_speed: openlaser_types::MmPerS::new(3000.0).unwrap(),
        s_max: 1000,
        tool: openlaser_machine::GantryTool::Laser { dynamic: false },
        air_assist: true,
        frame_before_mark: true,
        home_on_connect: false,
        link: openlaser_machine::GantryLink::ExportOnly,
    });
    profile
}

#[test]
fn kittest_air_hidden_for_galvo() {
    let mut h = harness();
    h.state_mut().seam.project = Some(used_project());
    h.run();
    assert!(h.query_by_label("Air").is_none() && h.query_by_label("Air C00").is_none(), "Air shows for a galvo");
    let mut h = harness_with(Options { machines: vec![gantry_profile()], ..Options::default() });
    h.state_mut().seam.project = Some(used_project());
    h.run();
    assert_eq!(header_order(&h), ["#", "Layer", "Mode", "Spd/Pwr", "Output", "Show", "Air"]);
    let air = h.get_by_label("Air C00");
    assert_eq!(air.accesskit_node().role(), egui::accesskit::Role::CheckBox);
    assert!(!air.accesskit_node().is_disabled());
    air.click();
    h.run();
    let last = h.state().seam.dispatched.last().cloned().unwrap();
    assert_eq!((last.1.as_str(), last.2), ("layer.setAir", serde_json::json!({ "layer": "C00", "on": true })));
}

#[test]
fn kittest_double_click_opens_cut_settings() {
    let mut h = harness();
    h.state_mut().seam.project = Some(used_project());
    h.run();
    h.get_by_label("Cut layer C00").click();
    h.run();
    assert!(!h.state().ui.cut_settings_open, "one click selects");
    h.get_by_label("Cut layer C00").click();
    h.run();
    assert!(h.state().ui.cut_settings_open, "a double click opens the editor");
    assert!(has_text(&h, "Cut Settings Editor: C00"));
    for field in ["Speed (mm/s)", "Max Power (%)", "Frequency (kHz)", "Q-Pulse Width (ns)", "Passes"] {
        assert!(h.query_by_label(field).is_some(), "editor field {field}");
    }
}

#[test]
fn kittest_cut_settings_ok_is_one_command() {
    let mut h = harness();
    h.state_mut().seam.project = Some(used_project());
    h.state_mut().ui.cut_settings_open = true;
    h.run();
    for (field, value) in
        [("speedMmS", serde_json::json!(1234)), ("frequencyKhz", serde_json::json!(60)), ("qPulseNs", serde_json::json!(200)), ("passes", serde_json::json!(2))]
    {
        h.state_mut().cut_draft.as_mut().unwrap().set_field(field, &value).unwrap();
    }
    let before = h.state().seam.dispatched.len();
    h.get_by_label("OK").click();
    h.run();
    let new: Vec<_> = h.state().seam.dispatched[before..].to_vec();
    assert_eq!(new.len(), 1, "OK must dispatch exactly one command (one undo step): {new:?}");
    assert_eq!(new[0].1, "layer.cutSettings.set");
    assert_eq!(
        new[0].2["galvo"],
        serde_json::json!({ "speedMmS": 1234.0, "powerPct": new[0].2["galvo"]["powerPct"], "frequencyKhz": 60.0, "qPulseNs": 200.0 })
    );
    assert_eq!(new[0].2["passes"], 2);
    // The engine refuses the command without a session document (the seam's project is the UI's view, not an open
    // document), so the dialog stays open with the error instead of pretending.
    assert!(h.state().ui.cut_settings_open);
    assert!(h.state().ui.status_message.contains("no document open"), "{}", h.state().ui.status_message);
}

#[test]
#[ignore = "needs I2: WP6's layer.cutSettings.set and history (the WP0 engine edit/undo return NotImplemented)"]
fn kittest_cut_settings_ok_is_one_undo_step() {
    let mut h = harness();
    h.state_mut().session.execute_as(Origin::Synthetic, "file.new", serde_json::json!({})).unwrap();
    let before = h.state().session.active().unwrap().history.labels().len();
    h.state_mut().ui.cut_settings_open = true;
    h.run();
    h.state_mut().cut_draft.as_mut().unwrap().speed = 1234.0;
    h.state_mut().cut_draft.as_mut().unwrap().passes = 3;
    h.get_by_label("OK").click();
    h.run();
    assert!(!h.state().ui.cut_settings_open);
    assert_eq!(h.state().session.active().unwrap().history.labels().len(), before + 1);
    h.state_mut().session.undo().unwrap();
    assert_ne!(h.state().session.active().unwrap().project.color_layer(0).unwrap().ops[0].galvo.speed.get(), 1234.0);
}

#[test]
fn kittest_refusal_badge_for_off_table_pulse() {
    let mut h = harness();
    h.state_mut().seam.project = Some(used_project());
    refusal_plan(&mut h);
    h.run();
    hover_shows(&mut h, "Refusals C00", "pulse_not_in_table");
    assert!(has_text(&h, "1 refusal(s)"), "status bar refusal chip");
    assert!(has_text(&h, "pulse 7 ns is not in the JPT M7 pulse table"), "Laser panel list");
}

#[test]
#[ignore = "needs I2: WP3's safety::validate behind WP6's live_check (the WP0 build_job refuses everything as not implemented)"]
fn kittest_refusal_badge_for_off_table_pulse_end_to_end() {
    let mut h = harness();
    h.state_mut().session.execute_as(Origin::Synthetic, "file.new", serde_json::json!({})).unwrap();
    h.state_mut().session.execute_as(Origin::Synthetic, "layer.cutSettings.set", serde_json::json!({ "layer": 0, "galvo": { "qPulseNs": 7 } })).unwrap();
    h.run();
    assert!(h.query_all_by_label_contains("Refusals C00").next().is_some());
}

// ---- Laser panel ---------------------------------------------------------------------------------------------------------

#[test]
fn kittest_laser_panel_galvo_disabled_reasons() {
    let mut h = harness();
    h.state_mut().seam.project = Some(used_project());
    h.run();
    for (label, reason) in [
        ("Pause", "Pause is not available on galvo machines"),
        ("Send", "Not available for this device: Send uploads to DSP controllers only."),
        ("Home", "A galvo has no homing"),
        ("Go to Origin", "A galvo has no head to move"),
        ("Start From", "Galvos always use absolute coordinates"),
        ("Job Origin", "Job Origin applies to User Origin"),
        ("Use Selection Origin", "Not used on galvos"),
        ("Start", "No job: open, import or draw a design first."),
        ("Frame", "No job: open, import or draw a design first."),
    ] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{label} must be disabled");
        hover_shows(&mut h, label, reason);
    }
    assert!(!h.get_by_label("Stop").accesskit_node().is_disabled(), "the panel's Stop is always enabled");
}

#[test]
fn kittest_start_without_receipt_says_frame_first() {
    let mut h = harness();
    h.state_mut().seam.project = Some(used_project());
    let s = ready_plan(&mut h);
    h.run();
    assert!(h.get_by_label("Start").accesskit_node().is_disabled());
    assert!(!h.get_by_label("Frame").accesskit_node().is_disabled(), "a planned job can be framed");
    hover_shows(&mut h, "Start", "Frame this job first. A human must approve every marking job.");
    let receipt = fresh_receipt(&h, &s);
    h.state_mut().seam.receipt = Some(receipt);
    h.run();
    assert!(!h.get_by_label("Start").accesskit_node().is_disabled(), "with a fresh human receipt Start is enabled");
}

#[test]
fn kittest_mark_disabled_while_refusals() {
    let mut h = harness();
    h.state_mut().seam.project = Some(used_project());
    refusal_plan(&mut h);
    let receipt = fresh_receipt(&h, &summary(Vec::new()));
    h.state_mut().seam.receipt = Some(receipt);
    h.run();
    assert!(h.get_by_label("Start").accesskit_node().is_disabled());
    assert!(h.get_by_label("Frame").accesskit_node().is_disabled());
    hover_shows(&mut h, "Start", "1 refusal(s) block this job");
    assert!(has_text(&h, "pulse_not_in_table"));
}

#[test]
fn synthetic_click_on_frame_button_does_not_frame() {
    // Frame is ENABLED (a planned job), and the click really reaches the button: queued through the control
    // channel's input builder and the injector. The adversarial injector calls it human; the app must not.
    let mut h = with_human(SyntheticInjector);
    h.state_mut().seam.project = Some(used_project());
    ready_plan(&mut h);
    h.run();
    let frame = h.get_by_label("Frame");
    assert!(!frame.accesskit_node().is_disabled());
    let p = frame.rect().center();
    for event in crate::control::input::events_for("ui.click", &serde_json::json!({ "x": p.x, "y": p.y })).unwrap() {
        h.state_mut().queue_input(vec![event]);
        hooked_step(&mut h, Vec::new());
    }
    h.run();
    assert!(h.state().seam.human_actions.is_empty(), "a synthetic click framed: {:?}", h.state().seam.human_actions);
    assert!(h.state().ui.status_message.contains("human-only"), "{}", h.state().ui.status_message);
    assert!(!h.state().ui.live_framing_open);
    // The same button with human frames does act: the refusal above is the provenance gate, not a dead button.
    let mut h = with_human(HumanFrames);
    h.state_mut().seam.project = Some(used_project());
    ready_plan(&mut h);
    h.run();
    h.get_by_label("Frame").click();
    h.run();
    assert_eq!(h.state().seam.human_actions, ["Frame"]);
}

#[test]
fn synthetic_request_card_click_never_approves() {
    let mut app = with_human(SyntheticInjector).into_state();
    app.services.human = Box::new(SyntheticInjector);
    let mut h = egui_kittest::Harness::builder().build_ui_state(
        |ui, app: &mut OpenLaserApp| {
            let ctx = ui.ctx().clone();
            app.request_card(ui, &ctx, openlaser_engine::RequestId(1), "Job from test", "Ready for human review");
        },
        app,
    );
    h.run();
    for label in ["Approve framing", "Reject request"] {
        h.get_by_label(label).click();
        h.run();
        assert!(h.state().seam.human_actions.is_empty(), "{label}");
        assert!(h.state().ui.status_message.contains("human-only"), "{label}: {}", h.state().ui.status_message);
    }
}

// ---- STOP ------------------------------------------------------------------------------------------------------------------

const STATES: [MachineState; 9] = [
    MachineState::Disconnected,
    MachineState::Connecting,
    MachineState::Idle,
    MachineState::Framing,
    MachineState::Running { packets_acked: 1, total: 4 },
    MachineState::Stopping,
    MachineState::Stopped,
    MachineState::Fault,
    MachineState::StopNotConfirmed,
];

#[test]
fn kittest_stop_always_visible_and_enabled() {
    for state in STATES {
        // Spelled out here, not taken from the code under test (UI §7.1: framing, marking, stopping, not confirmed).
        let active = matches!(state, MachineState::Framing | MachineState::Running { .. } | MachineState::Stopping | MachineState::StopNotConfirmed);
        for layout in ["default", "laser closed", "laser floated", "modal open"] {
            for ppp in [1.0, 2.0] {
                for no_human in [true, false] {
                    let human: Box<dyn crate::HumanSurfaces> = if no_human { Box::new(crate::NoHuman) } else { Box::new(NoSurface) };
                    let mut h = harness_with(Options { size: egui::vec2(1024.0, 700.0), ppp, human, ..Options::default() });
                    h.state_mut().seam.project = Some(used_project());
                    h.state_mut().seam.machine_state = Some(state.clone());
                    match layout {
                        "laser closed" => drop(h.state_mut().show_panel(&serde_json::json!({ "panel": "laser", "visible": false })).unwrap()),
                        "laser floated" => drop(control(&mut h, "ui.dock.float", serde_json::json!({ "panel": "laser" }))),
                        // A real egui::Modal: the Cut Settings Editor.
                        "modal open" => h.state_mut().ui.cut_settings_open = true,
                        _ => {}
                    }
                    if layout == "laser floated" {
                        // Docking is refused while busy through control; float it directly.
                        let _ = h.state_mut().dock.apply_with_permissions(
                            craft_ui::docking::Action::Float { panel: "laser".into(), rect: [80.0, 80.0, 320.0, 420.0] },
                            crate::dock::permissions,
                        );
                    }
                    h.run();
                    let case = format!("{state:?} / {layout} / {ppp}x / {}", if no_human { "NoHuman" } else { "no surface" });
                    assert_eq!(h.state().ui.machine_active, active, "{case}");
                    let overlay = active || no_human || layout == "modal open";
                    assert_eq!(h.query_by_label("Stop laser (toolbar)").is_some(), overlay, "overlay presence: {case}");
                    let stop = h.get_by_label("Stop laser");
                    assert!(!stop.accesskit_node().is_disabled(), "{case}");
                    let r = stop.rect();
                    assert!(r.min.x >= 0.0 && r.max.x <= 1024.0 && r.min.y >= 0.0 && r.max.y <= 700.0 && r.width() >= 40.0, "STOP off-window: {case}: {r:?}");
                    let before = h.state().seam.stops;
                    stop.click();
                    h.run();
                    assert_eq!(h.state().seam.stops, before + 1, "STOP click did not stop: {case}");
                }
            }
        }
    }
}

#[test]
fn stop_overlay_is_above_the_arm_dialog_state() {
    let mut h = with_human(NoSurface);
    h.state_mut().seam.machine_state = Some(MachineState::Idle);
    h.state_mut().seam.pending_arm = true;
    h.run();
    assert!(h.state().ui.machine_active, "a pending ARM is an active state");
    assert!(has_text(&h, "Armed"), "chip says Armed");
    h.get_by_label("Stop laser").click();
    h.run();
    assert_eq!(h.state().seam.stops, 1);
    assert!(!h.state().seam.pending_arm, "STOP voids the pending ARM");
    assert!(!h.state().ui.machine_active);
}

/// A text field inside the app's own frame, to watch what Esc does to focused widgets.
fn esc_harness(human: impl crate::HumanSurfaces + 'static) -> egui_kittest::Harness<'static, (OpenLaserApp, String)> {
    let app = with_human(human).into_state();
    egui_kittest::Harness::builder().with_size(egui::vec2(400.0, 200.0)).build_ui_state(
        |ui, (app, text): &mut (OpenLaserApp, String)| {
            let r = ui.add(egui::TextEdit::singleline(text).id_salt("field"));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Field"));
            let _ = app;
        },
        (app, String::new()),
    )
}

fn esc_step(h: &mut egui_kittest::Harness<'static, (OpenLaserApp, String)>) {
    let ctx = h.ctx.clone();
    let mut raw = std::mem::take(h.input_mut());
    raw.events.extend(esc());
    h.state_mut().0.raw_input_hook(&ctx, &mut raw);
    *h.input_mut() = raw;
    h.step();
}

#[test]
fn kittest_esc_trips_latch_before_widgets() {
    let mut h = esc_harness(NoSurface);
    h.state_mut().0.seam.machine_state = Some(MachineState::Running { packets_acked: 0, total: 2 });
    h.run();
    h.get_by_label("Field").click();
    h.run();
    h.get_by_label("Field").type_text("keep this text");
    h.run();
    assert!(h.get_by_label("Field").is_focused());
    esc_step(&mut h);
    h.run();
    assert_eq!(h.state().0.seam.stops, 1, "Esc while active must trip STOP");
    assert_eq!(h.state().1, "keep this text");
    assert!(h.get_by_label("Field").is_focused(), "Esc reached the focused field: it must be consumed before widgets");
    // Idle: Esc is not STOP and goes on to the widget (the field gives up focus as egui does).
    h.state_mut().0.seam.machine_state = Some(MachineState::Idle);
    esc_step(&mut h);
    h.run();
    assert_eq!(h.state().0.seam.stops, 1, "idle Esc must not stop");
    assert!(!h.get_by_label("Field").is_focused(), "idle Esc should reach widgets");
}

#[test]
fn idle_esc_clears_the_selection() {
    let mut h = harness();
    h.state_mut().seam.project = Some(used_project());
    h.state_mut().seam.selection = vec![openlaser_types::ObjectId(1)];
    h.state_mut().ui.selected_tool = "Pan";
    h.run();
    hooked_step(&mut h, esc());
    h.run();
    assert_eq!(h.state().seam.stops, 0);
    assert_eq!(h.state().ui.selected_tool, "Select");
    assert!(h.state().seam.dispatched.iter().any(|(_, id, _)| id == "edit.selectNone"), "{:?}", h.state().seam.dispatched);
}

#[test]
fn kittest_esc_closes_arm_dialog() {
    let mut h = harness();
    h.state_mut().seam.pending_arm = true;
    h.run();
    assert!(h.state().arm_dialog_open());
    hooked_step(&mut h, esc());
    assert!(!h.state().arm_dialog_open());
    assert_eq!(h.state().seam.stops, 1);
}

#[test]
fn kittest_no_file_dialog_while_active() {
    let files = Files::default();
    let mut h = harness_with(Options { files: files.clone(), ..Options::default() });
    h.state_mut().seam.machine_state = Some(MachineState::Framing);
    h.run();
    hover_shows(&mut h, "Open", "Open: Stop the active frame or job");
    assert!(h.get_by_label("Import").accesskit_node().is_disabled());
    h.state_mut().invoke_ui_action("file.open");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::O);
    h.run();
    assert_eq!(files.0.load(Ordering::SeqCst), 0, "a file dialog opened while framing");
    h.state_mut().seam.machine_state = Some(MachineState::Idle);
    h.run();
    h.get_by_label("Open").click();
    h.run();
    assert_eq!(files.0.load(Ordering::SeqCst), 1, "Open works when idle");
}

#[test]
fn kittest_quit_while_marking_stops_first() {
    let quit = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&quit);
    let mut h = harness_with(Options { quit: Box::new(move || observed.store(true, Ordering::SeqCst)), ..Options::default() });
    h.state_mut().seam.machine_state = Some(MachineState::Running { packets_acked: 1, total: 9 });
    h.run();
    h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    h.step();
    assert!(h.output().viewport_output[&egui::ViewportId::ROOT].commands.contains(&egui::ViewportCommand::CancelClose), "the close must be cancelled");
    assert_eq!(h.state().seam.stops, 1, "closing while marking must stop first");
    assert!(!quit.load(Ordering::SeqCst), "closed before the StopReport");
    h.run_steps(3);
    assert!(!quit.load(Ordering::SeqCst), "closed before the StopReport");
    // The driver reports a confirmed stop: now the window may close (craft_quit runs after, in the app).
    h.state_mut().seam.stop_report = Some(true);
    h.state_mut().seam.machine_state = Some(MachineState::Stopped);
    h.step();
    assert!(quit.load(Ordering::SeqCst), "the window must close once the stop is confirmed");
    assert_eq!(h.state().seam.stops, 1);
}

#[test]
fn quit_refused_when_the_stop_is_not_confirmed_and_idle_close_is_not_held() {
    let quit = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&quit);
    let mut h = harness_with(Options { quit: Box::new(move || observed.store(true, Ordering::SeqCst)), ..Options::default() });
    h.state_mut().seam.machine_state = Some(MachineState::Framing);
    h.state_mut().seam.stop_report = Some(false);
    h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    h.step();
    h.run_steps(2);
    assert!(!quit.load(Ordering::SeqCst));
    assert_eq!(h.state().seam.stops, 1, "the close tripped a stop before anything may quit");
    // The close guard reports the unconfirmed stop; the engine's own `stop` status event may then replace that
    // line in the status bar, so the log is the record that it was shown.
    assert!(h.state().log.iter().any(|m| m.contains("STOP NOT CONFIRMED")), "{:?}", h.state().log);
    // Idle: a close is not held and nothing is stopped.
    let mut h = harness();
    h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    h.step();
    assert!(!h.output().viewport_output[&egui::ViewportId::ROOT].commands.contains(&egui::ViewportCommand::CancelClose));
    assert_eq!(h.state().seam.stops, 0);
}

#[test]
fn draw_tools_arm_then_the_canvas_dispatch_mm_params() {
    // Regression: a tool click used to run the command with no parameters ("tools.rectangle: missing field 'xMm'").
    let mut h = with_human(HumanFrames);
    h.run();
    h.state_mut().session.execute_as(Origin::Human, "file.new", serde_json::json!({})).unwrap();
    let fit = h.get_by_label("Zoom to fit").rect();
    let a = egui::pos2(fit.left() - 400.0, fit.top() - 300.0);
    let b = a + egui::vec2(80.0, 60.0);
    // Arming the tool never dispatches; the drag does, with real millimetres, then the tool is Select again.
    h.get_by_label("Rectangle").click();
    h.run();
    assert_eq!(h.state().ui.selected_tool, "Rectangle");
    assert!(h.state().seam.dispatched.iter().all(|(_, id, _)| id != "tools.rectangle"), "arming must not run the command");
    h.event(egui::Event::PointerButton { pos: a, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    h.event(egui::Event::PointerMoved(b));
    h.step();
    h.event(egui::Event::PointerButton { pos: b, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run();
    let (_, id, params) = h.state().seam.dispatched.last().cloned().unwrap();
    assert_eq!(id, "tools.rectangle");
    assert!(params["wMm"].as_f64().is_some_and(|w| w > 1.0) && params["hMm"].as_f64().is_some_and(|v| v > 1.0), "{params}");
    assert!(params["xMm"].as_f64().is_some() && params["yMm"].as_f64().is_some(), "{params}");
    assert_eq!(h.state().ui.selected_tool, "Select", "a placed shape returns to Select");
    // Draw Lines: clicks collect points (no dispatch yet), Enter finishes open.
    h.get_by_label("Draw Lines").click();
    h.run();
    for p in [a, b, a + egui::vec2(120.0, 0.0)] {
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.step();
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    assert!(h.state().seam.dispatched.iter().all(|(_, id, _)| id != "tools.drawLines"), "no dispatch before finish");
    h.event(egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    h.run();
    let (_, id, params) = h.state().seam.dispatched.last().cloned().unwrap();
    assert_eq!(id, "tools.drawLines");
    assert_eq!(params["pointsMm"].as_array().map(|p| p.len()), Some(3), "{params}");
    assert_eq!(params["closed"], serde_json::Value::Bool(false));
    // Esc while armed cancels and returns to Select.
    h.get_by_label("Ellipse").click();
    h.run();
    h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    h.run();
    assert_eq!(h.state().ui.selected_tool, "Select");
}

// ---- canvas selection drags (real engine) ---------------------------------------------------------------------------------

/// Screen position of a world-mm point through the live view transform.
fn w2s(h: &H, world: [f64; 2]) -> egui::Pos2 {
    crate::canvas::ViewXform {
        zoom: h.state().view.zoom.unwrap(),
        center: h.state().view.center,
        rect: h.state().view.last_canvas.unwrap_or(egui::Rect::NOTHING),
    }
    .world_to_screen(world)
}

fn s2w(h: &H, p: egui::Pos2) -> [f64; 2] {
    crate::canvas::ViewXform {
        zoom: h.state().view.zoom.unwrap(),
        center: h.state().view.center,
        rect: h.state().view.last_canvas.unwrap_or(egui::Rect::NOTHING),
    }
    .screen_to_world(p)
}

fn pointer(h: &mut H, p: egui::Pos2, down: bool, mods: egui::Modifiers) {
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: down, modifiers: mods });
    h.step();
}

/// Press → moves → release (the drag crosses egui's threshold on the first move).
fn drag_to(h: &mut H, a: egui::Pos2, b: egui::Pos2, mods: egui::Modifiers) {
    h.event(egui::Event::ModifiersChanged(mods));
    pointer(h, a, true, mods);
    h.event(egui::Event::PointerMoved(a + (b - a) * 0.5));
    h.step();
    h.event(egui::Event::PointerMoved(b));
    h.step();
    pointer(h, b, false, mods);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run();
}

/// A new document with one axis-aligned rectangle; returns its object id.
fn doc_with_rect(h: &mut H, x: f64, y: f64, w: f64, ht: f64) -> openlaser_types::ObjectId {
    h.state_mut().session.execute_as(Origin::Human, "file.new", serde_json::json!({})).unwrap();
    let v = h.state_mut().session.execute_as(Origin::Human, "tools.rectangle", serde_json::json!({ "xMm": x, "yMm": y, "wMm": w, "hMm": ht })).unwrap();
    h.run();
    openlaser_types::ObjectId(v["ids"][0].as_u64().unwrap())
}

fn sel_bounds(h: &H) -> openlaser_types::Bounds {
    h.state().session.active().unwrap().project.bounds(&h.state().session.active().unwrap().selection.ids).unwrap().unwrap()
}

#[test]
fn move_drag_commits_one_transform_and_undo_restores() {
    let mut h = with_human(HumanFrames);
    h.run();
    doc_with_rect(&mut h, 0.0, 0.0, 10.0, 10.0);
    let before = sel_bounds(&h);
    let dispatches = h.state().seam.dispatched.len();
    let (a, b) = (w2s(&h, [5.0, 5.0]), w2s(&h, [25.0, 15.0]));
    // Press, move, hold (no dispatch mid-drag), release.
    pointer(&mut h, a, true, egui::Modifiers::NONE);
    h.event(egui::Event::PointerMoved(a + (b - a) * 0.4));
    h.step();
    h.event(egui::Event::PointerMoved(b));
    h.step();
    assert!(h.state().seam.dispatched.iter().skip(dispatches).all(|(_, id, _)| !id.starts_with("arrange.")), "no dispatch during the drag");
    let delta = [s2w(&h, b)[0] - s2w(&h, a)[0], s2w(&h, b)[1] - s2w(&h, a)[1]];
    pointer(&mut h, b, false, egui::Modifiers::NONE);
    h.run();
    let new: Vec<_> = h.state().seam.dispatched.iter().skip(dispatches).cloned().collect();
    let moves: Vec<_> = new.iter().filter(|(_, id, _)| id == "arrange.transform").collect();
    assert_eq!(moves.len(), 1, "one drag is one command: {new:?}");
    assert_eq!(moves[0].2["anchor"], "center");
    for v in [moves[0].2["xMm"].as_f64().unwrap() - 5.0 - delta[0], moves[0].2["yMm"].as_f64().unwrap() - 5.0 - delta[1]] {
        assert!(v.abs() < 1e-6, "{v}");
    }
    let after = sel_bounds(&h);
    assert!((after.min[0] - before.min[0] - delta[0]).abs() < 1e-6 && (after.min[1] - before.min[1] - delta[1]).abs() < 1e-6, "{after:?}");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run();
    assert_eq!(sel_bounds(&h), before, "Cmd+Z restores the bounds");
}

#[test]
fn handle_drags_scale_from_the_opposite_side_and_clamp() {
    let mut h = with_human(HumanFrames);
    h.run();
    doc_with_rect(&mut h, 0.0, 0.0, 10.0, 10.0);
    h.state_mut().ui.lock_ratio = false;
    // Bottom-right corner handle (world [max.x, min.y]); the top-left corner (min.x, max.y) must not move.
    let (a, b) = (w2s(&h, [10.0, 0.0]), w2s(&h, [16.0, -4.0]));
    let count = h.state().seam.dispatched.len();
    drag_to(&mut h, a, b, egui::Modifiers::NONE);
    let after = sel_bounds(&h);
    assert_eq!((after.min[0], after.max[1]), (0.0, 10.0), "the top-left corner is the anchor: {after:?}");
    assert!((after.max[0] - 16.0).abs() < 1e-4 && (after.min[1] - -4.0).abs() < 1e-4, "{after:?}");
    let last = &h.state().seam.dispatched.last().unwrap().2;
    assert_eq!(last["anchor"], "topLeft");
    assert!(last["wMm"].is_f64() && last["hMm"].is_f64(), "{last}");
    // Right edge handle (index 3): the midpoint of the right side, one axis only in the committed params.
    let (a, b) = (w2s(&h, [16.0, 3.0]), w2s(&h, [20.0, 3.0]));
    drag_to(&mut h, a, b, egui::Modifiers::NONE);
    let last = &h.state().seam.dispatched.last().unwrap().2;
    assert_eq!(last["anchor"], "left");
    assert!(last["wMm"].is_f64() && last["hMm"].is_null(), "an edge handle commits one axis: {last}");
    assert!((last["wMm"].as_f64().unwrap() - 20.0).abs() < 1e-4);
    // Shift on a corner: proportional.
    let before = sel_bounds(&h);
    let (a, b) = (w2s(&h, before.max), w2s(&h, [before.max[0] + 10.0, before.max[1]]));
    drag_to(&mut h, a, b, egui::Modifiers::SHIFT);
    let after = sel_bounds(&h);
    let (bw, bh, aw, ah) = (before.max[0] - before.min[0], before.max[1] - before.min[1], after.max[0] - after.min[0], after.max[1] - after.min[1]);
    assert!((aw / ah - bw / bh).abs() < 1e-6, "Shift scales proportionally: {before:?} → {after:?}");
    // Dragging past the opposite side clamps at the 0.1 mm minimum, never flips.
    let before = sel_bounds(&h);
    let (a, b) = (w2s(&h, [before.min[0], before.max[1]]), w2s(&h, [before.max[0] + 50.0, before.min[1] - 50.0]));
    drag_to(&mut h, a, b, egui::Modifiers::NONE);
    let after = sel_bounds(&h);
    assert!((after.max[0] - after.min[0] - 0.1).abs() < 1e-6 && (after.max[1] - after.min[1] - 0.1).abs() < 1e-6, "{after:?}");
    assert!(after.min[0] < before.max[0] && after.max[1] > before.min[1], "no flip: {after:?}");
    assert!(h.state().seam.dispatched.iter().skip(count).all(|(_, id, _)| !id.starts_with("arrange.nudge")), "drags never use nudge");
}

#[test]
fn clicking_selected_toggles_rotate_handles_and_rotate_drag_commits() {
    let mut h = with_human(HumanFrames);
    h.run();
    doc_with_rect(&mut h, 0.0, 0.0, 10.0, 20.0);
    assert!(!h.state().view.rotate_handles);
    // A click (press + release, no drag) on the selected shape toggles the handles.
    let c = w2s(&h, [5.0, 10.0]);
    pointer(&mut h, c, true, egui::Modifiers::NONE);
    pointer(&mut h, c, false, egui::Modifiers::NONE);
    h.run();
    assert!(h.state().view.rotate_handles, "click on the selected shape shows rotate handles");
    // Drag the bottom-right corner handle to the top-right: the segment to the centre turns
    // atan2-style counter-clockwise.
    let (a, b) = (w2s(&h, [10.0, 0.0]), w2s(&h, [10.0, 20.0]));
    drag_to(&mut h, a, b, egui::Modifiers::NONE);
    let last = h.state().seam.dispatched.last().cloned().unwrap();
    assert_eq!(last.1, "arrange.rotateBy");
    let expected = (10.0f64.atan2(5.0) - (-10.0f64.atan2(5.0))).to_degrees();
    assert!((last.2["deg"].as_f64().unwrap() - expected).abs() < 0.5, "{:?} vs {expected}", last.2);
    assert_eq!(last.2["anchor"], "center");
    // Shift snaps to 15°: rotate back to axis-aligned.
    let after = sel_bounds(&h);
    let (a, b) = (w2s(&h, [after.min[0], after.max[1]]), w2s(&h, [after.max[0], after.max[1]]));
    drag_to(&mut h, a, b, egui::Modifiers::SHIFT);
    let last = h.state().seam.dispatched.last().cloned().unwrap();
    let deg = last.2["deg"].as_f64().unwrap();
    assert_eq!(deg % 15.0, 0.0, "Shift snaps to 15°: {deg}");
    // Clicking elsewhere (empty canvas) resets to scale handles.
    let p = w2s(&h, [-80.0, -80.0]);
    pointer(&mut h, p, true, egui::Modifiers::NONE);
    pointer(&mut h, p, false, egui::Modifiers::NONE);
    h.run();
    assert!(!h.state().view.rotate_handles);
}

#[test]
fn the_window_opens_on_an_empty_document() {
    // Regression: launching with no file left the session without a document, so every edit tool was disabled.
    let mut h = harness();
    assert!(h.state().session.active().is_none());
    h.state_mut().open_startup_files(&[]);
    assert!(h.state().session.active().is_some(), "an empty document is open");
    assert!(h.state_mut().session.execute_as(Origin::Human, "tools.rectangle", serde_json::json!({ "xMm": 0, "yMm": 0, "wMm": 5, "hMm": 5 })).is_ok());
}

#[test]
fn clicking_an_unselected_shape_inside_the_box_selects_it() {
    // Regression: a stroke hit on another shape inside the selection's box toggled the handles instead.
    let mut h = with_human(HumanFrames);
    h.run();
    let big = doc_with_rect(&mut h, 0.0, 0.0, 40.0, 40.0);
    let v =
        h.state_mut().session.execute_as(Origin::Human, "tools.rectangle", serde_json::json!({ "xMm": 10.0, "yMm": 10.0, "wMm": 5.0, "hMm": 5.0 })).unwrap();
    let small = openlaser_types::ObjectId(v["ids"][0].as_u64().unwrap());
    h.state_mut().session.execute_as(Origin::Human, "edit.select", serde_json::json!({ "ids": [big.0], "mode": "replace" })).unwrap();
    h.run();
    let p = w2s(&h, [10.0, 12.5]);
    pointer(&mut h, p, true, egui::Modifiers::NONE);
    pointer(&mut h, p, false, egui::Modifiers::NONE);
    h.run();
    assert_eq!(h.state().session.active().unwrap().selection.ids, vec![small]);
    assert!(!h.state().view.rotate_handles);
}

#[test]
fn esc_mid_drag_dispatches_nothing_and_keeps_the_document() {
    let mut h = with_human(HumanFrames);
    h.run();
    doc_with_rect(&mut h, 0.0, 0.0, 10.0, 10.0);
    let before = sel_bounds(&h);
    let count = h.state().seam.dispatched.len();
    let (a, b) = (w2s(&h, [5.0, 5.0]), w2s(&h, [40.0, 30.0]));
    pointer(&mut h, a, true, egui::Modifiers::NONE);
    h.event(egui::Event::PointerMoved(b));
    h.step();
    h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    h.step();
    pointer(&mut h, b, false, egui::Modifiers::NONE);
    h.run();
    assert!(h.state().seam.dispatched.iter().skip(count).all(|(_, id, _)| !id.starts_with("arrange.")), "Esc cancelled the commit");
    assert_eq!(sel_bounds(&h), before, "the document is untouched");
    assert_eq!(h.state().session.active().unwrap().selection.ids, vec![openlaser_types::ObjectId(1)], "the selection is kept");
}

#[test]
fn arrow_keys_nudge_the_selection() {
    let mut h = with_human(HumanFrames);
    h.run();
    doc_with_rect(&mut h, 0.0, 0.0, 10.0, 10.0);
    h.event(egui::Event::Key { key: egui::Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    h.run();
    let last = &h.state().seam.dispatched.last().unwrap().2;
    assert_eq!((last["dxMm"].as_f64().unwrap(), last["dyMm"].as_f64().unwrap()), (1.0, 0.0));
    assert!((sel_bounds(&h).min[0] - 1.0).abs() < 1e-9);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowDown);
    h.run();
    let last = &h.state().seam.dispatched.last().unwrap().2;
    assert_eq!((last["dxMm"].as_f64().unwrap(), last["dyMm"].as_f64().unwrap()), (0.0, -10.0));
    assert!((sel_bounds(&h).min[1] - -10.0).abs() < 1e-9);
    // Delete, duplicate, select-all and undo work through the same plumbing on the real selection.
    h.event(egui::Event::Key { key: egui::Key::Backspace, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    h.run();
    assert!(h.state().session.active().unwrap().project.shapes.is_empty(), "Backspace deletes");
}

#[test]
fn numeric_edits_reach_the_engine() {
    let mut h = with_human(HumanFrames);
    h.run();
    doc_with_rect(&mut h, 0.0, 0.0, 10.0, 20.0);
    // Anchor defaults to centre: XPos names the bounding-box centre.
    h.get_by_label("XPos").focus();
    h.get_by_label("XPos").type_text("30");
    h.event(egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    h.run();
    let b = sel_bounds(&h);
    assert!(((b.min[0] + b.max[0]) / 2.0 - 30.0).abs() < 1e-6, "XPos moved the centre: {b:?}");
    h.get_by_label("Width").focus();
    h.get_by_label("Width").type_text("40");
    h.event(egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    h.run();
    assert!((sel_bounds(&h).max[0] - sel_bounds(&h).min[0] - 40.0).abs() < 1e-6);
    h.get_by_label("Rotate").focus();
    h.get_by_label("Rotate").type_text("90");
    h.event(egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    h.run();
    let b = sel_bounds(&h);
    assert!((b.max[1] - b.min[1] - 40.0).abs() < 0.1, "90° about the centre swaps the axes: {b:?}");
    assert!(h.state().seam.dispatched.iter().any(|(_, id, p)| id == "arrange.transform" && p["anchor"] == "center" && p["xMm"].is_f64()));
}

#[test]
fn a_new_drawn_shape_is_selected_and_shift_constrains_the_drag() {
    let mut h = with_human(HumanFrames);
    h.run();
    h.state_mut().session.execute_as(Origin::Human, "file.new", serde_json::json!({})).unwrap();
    h.get_by_label("Rectangle").click();
    h.run();
    let (a, b) = (w2s(&h, [0.0, 0.0]), w2s(&h, [40.0, 10.0]));
    drag_to(&mut h, a, b, egui::Modifiers::SHIFT);
    let b_ = sel_bounds(&h);
    assert!((b_.max[0] - b_.min[0] - (b_.max[1] - b_.min[1])).abs() < 1e-6, "Shift draws a square: {b_:?}");
    assert_eq!(
        h.state().session.active().unwrap().selection.ids,
        vec![h.state().session.active().unwrap().project.shapes[0].id],
        "the new shape is the selection"
    );
}

#[test]
fn hidden_window_still_polls_while_active() {
    let mut h = harness();
    h.state_mut().seam.machine_state = Some(MachineState::Framing);
    h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().minimized = Some(true);
    let before = h.state().seam.polls;
    h.run_steps(3);
    assert!(h.state().seam.polls >= before + 3);
    assert!(h.state().ui.machine_active);
    assert!(h.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay <= std::time::Duration::from_millis(250));
}

// ---- provenance ----------------------------------------------------------------------------------------------------------

/// Item 5: every widget path dispatches with the frame's provenance: Synthetic in a synthetic frame, Human otherwise.
#[test]
fn widgets_dispatch_with_the_frames_origin() {
    for (synthetic, expected) in [(true, Origin::Synthetic), (false, Origin::Human)] {
        let files = Files::default();
        let human: Box<dyn crate::HumanSurfaces> = if synthetic { Box::new(crate::NoHuman) } else { Box::new(HumanFrames) };
        let mut h = harness_with(Options { human, files: files.clone(), ..Options::default() });
        h.state_mut().seam.project = Some(used_project());
        h.run();
        // Palette swatch, Cuts checkbox, left tool, Cut Settings OK.
        h.get_by_label("Layer 03").click();
        h.run();
        h.get_by_label("Show C00").click();
        h.run();
        h.state_mut().ui.active_layer = LayerKey::Color(0);
        h.state_mut().ui.cut_settings_open = true;
        h.run();
        h.get_by_label("OK").click();
        h.run();
        // The WP0 engine refuses the edit, so the editor stays open: close it.
        h.get_by_label("Cancel").click();
        h.run();
        assert!(!h.state().ui.cut_settings_open);
        // Toolbar button, in-window menu row and shortcut: each opens a file dialog recorded with its origin.
        h.get_by_label("Open").click();
        h.run();
        h.get_by_label("File").click();
        h.run();
        h.query_all_by_label_contains("Import…").next().unwrap().click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::O);
        h.run();
        let dispatched: Vec<(Origin, String)> = h.state().seam.dispatched.iter().map(|(o, id, _)| (*o, id.clone())).collect();
        for id in ["layer.setCurrent", "layer.setShow", "layer.cutSettings.set"] {
            assert!(dispatched.iter().any(|(_, d)| d == id), "{id} was not dispatched: {dispatched:?}");
        }
        assert!(dispatched.iter().all(|(o, _)| *o == expected), "synthetic={synthetic}: {dispatched:?}");
        let dialogs: Vec<Origin> = h.state().pending_dialogs.iter().map(|(_, _, o)| *o).collect();
        assert_eq!(dialogs.len(), 3, "toolbar, menu and shortcut each open a dialog: {dialogs:?}");
        assert!(dialogs.iter().all(|o| *o == expected), "synthetic={synthetic}: {dialogs:?}");
        assert_eq!(files.0.load(Ordering::SeqCst), 3);
    }
}

// ---- control dispatcher ----------------------------------------------------------------------------------------------------

#[test]
fn control_refuses_human_only() {
    let mut ids: Vec<&str> =
        vec!["device.connect", "device.settings.set", "laser.frame", "laser.start", "laser.arm", "laser.run", "laser.redLight", "hw:tools.positionLaser"];
    ids.extend(openlaser_engine::registry().specs().iter().filter(|s| s.access == openlaser_engine::Access::Human).map(|s| s.id));
    // NoHuman (a surface counts as open) and NoSurface: the human-only refusal comes first either way.
    for no_human in [true, false] {
        let mut h = if no_human { harness() } else { with_human(NoSurface) };
        for id in &ids {
            for (method, params) in [
                ("engine.execute", serde_json::json!({ "command": id })),
                ("ui.menu.invoke", serde_json::json!({ "id": id })),
                ("engine.execute", serde_json::json!({ "command": "command.batch", "params": { "steps": [{ "id": "layer.setShow" }, { "id": id }] } })),
            ] {
                let error = control(&mut h, method, params.clone()).unwrap_err();
                assert!(error.starts_with("human-only"), "{method} {params} → {error}");
            }
        }
    }
}

#[test]
fn control_mutations_refused_while_busy_or_human_surface_open() {
    let mut h = with_human(NoSurface);
    h.state_mut().seam.machine_state = Some(MachineState::Framing);
    h.run();
    for (method, params) in [
        ("engine.execute", serde_json::json!({ "command": "file.new" })),
        ("ui.menu.invoke", serde_json::json!({ "id": "window.show:console" })),
        ("ui.click", serde_json::json!({ "x": 10, "y": 10 })),
        ("ui.key", serde_json::json!({ "key": "A" })),
        ("ui.dialog.open", serde_json::json!({ "dialog": "cutSettings" })),
        ("ui.dock.close", serde_json::json!({ "panel": "laser" })),
        ("app.quit", serde_json::json!({})),
        ("app.open", serde_json::json!({ "path": "a.olp" })),
    ] {
        let error = control(&mut h, method, params).unwrap_err();
        assert!(error.starts_with("machine_busy"), "{method}: {error}");
    }
    for method in ["ui.inspect", "ui.elements", "engine.commands", "ui.screenshot", "machine.list", "methods"] {
        assert!(control(&mut h, method, serde_json::json!({})).is_ok(), "{method} must be served while busy");
    }
    let before = h.state().seam.stops;
    let stop = control(&mut h, "machine.stop", serde_json::json!({ "machine": "no-such-machine" })).unwrap();
    assert!(stop["latched"].is_boolean() && stop["stoppedAll"] == true, "{stop}");
    assert_eq!(h.state().seam.stops, before + 1, "machine.stop with an unknown id still stops everything");
    // A human-only surface open: NoHuman (fail closed) and a pending ARM.
    let mut h = harness();
    assert!(control(&mut h, "engine.execute", serde_json::json!({ "command": "file.new" })).unwrap_err().starts_with("human_confirmation_open"));
    assert!(control(&mut h, "ui.inspect", serde_json::json!({})).is_ok());
    let mut h = with_human(NoSurface);
    h.state_mut().seam.pending_arm = true;
    assert!(control(&mut h, "ui.dock.close", serde_json::json!({ "panel": "console" })).unwrap_err().starts_with("human_confirmation_open"));
    // Idle and nothing open: the same mutation goes through to the dispatcher.
    let mut h = with_human(NoSurface);
    assert_eq!(control(&mut h, "ui.dock.close", serde_json::json!({ "panel": "console" })).map(|_| ()), Ok(()));
    // The merged engine builds documents: the mutation reaches the dispatcher and succeeds.
    let v = control(&mut h, "engine.execute", serde_json::json!({ "command": "file.new" })).unwrap_or_else(|e| panic!("idle mutation refused: {e}"));
    assert_eq!(v["index"], 0, "{v}");
}

#[test]
fn every_control_method_has_a_handler() {
    let mut h = with_human(NoSurface);
    h.run();
    for method in crate::control::ROUTES.iter().chain(openlaser_control::methods::METHODS.iter()) {
        if *method == "app.quit" {
            continue; // would close the harness window; its route is exercised by the busy test
        }
        let answer = control(&mut h, method, serde_json::json!({}));
        if let Err(error) = &answer {
            assert!(!error.contains("unknown method"), "{method} fell through: {error}");
        }
    }
    assert!(control(&mut h, "connect", serde_json::json!({})).unwrap_err().contains("unknown method"));
    assert!(control(&mut h, "laser.arm", serde_json::json!({})).unwrap_err().contains("unknown method"));
}

#[test]
fn ui_elements_and_inspect_report_the_real_window() {
    let mut h = with_human(NoSurface);
    h.state_mut().seam.project = Some(used_project());
    h.run();
    let elements = control(&mut h, "ui.elements", serde_json::json!({})).unwrap();
    let list = elements["elements"].as_array().unwrap();
    let find = |label: &str| list.iter().find(|e| e["label"] == label).cloned();
    for label in ["Stop laser", "XPos", "Layer 05", "T2", "Output C00", "Open", "Zoom to fit", "Device"] {
        assert!(find(label).is_some(), "ui.elements lacks {label}");
    }
    assert_eq!(find("Frame").unwrap()["humanOnly"], true);
    assert_eq!(find("Start").unwrap()["enabled"], false);
    let rect = find("Stop laser").unwrap()["rect"].clone();
    assert!(rect[2].as_f64().unwrap() > 0.0);
    let filtered = control(&mut h, "ui.elements", serde_json::json!({ "query": "layer 0", "limit": 3 })).unwrap();
    assert_eq!(filtered["elements"].as_array().unwrap().len(), 3);
    let inspect = control(&mut h, "ui.inspect", serde_json::json!({})).unwrap();
    assert_eq!(inspect["armDialog"], "closed");
    h.state_mut().seam.pending_arm = true;
    let inspect = control(&mut h, "ui.inspect", serde_json::json!({})).unwrap();
    assert_eq!(inspect["armDialog"], "open");
    assert_eq!(inspect["machineActive"], true);
    assert!(inspect["visiblePanels"].as_array().unwrap().iter().any(|p| p == "laser"));
}

#[test]
fn ordinary_dialog_is_editable_but_never_a_human_surface() {
    let mut h = with_human(NoSurface);
    h.state_mut().seam.project = Some(used_project());
    control(&mut h, "ui.dialog.open", serde_json::json!({ "dialog": "cutSettings", "layer": "C00" })).unwrap();
    control(&mut h, "ui.dialog.set", serde_json::json!({ "dialog": "cutSettings", "field": "speedMmS", "value": 1234 })).unwrap();
    assert_eq!(h.state().cut_draft.as_ref().unwrap().speed, 1234.0);
    assert!(control(&mut h, "ui.dialog.set", serde_json::json!({ "dialog": "cutSettings", "field": "force", "value": true })).is_err());
    for kind in ["arm", "deviceSettings", "settings"] {
        assert!(control(&mut h, "ui.dialog.open", serde_json::json!({ "dialog": kind })).unwrap_err().contains("human-only dialog"), "{kind}");
    }
    assert!(control(&mut h, "ui.set", serde_json::json!({ "fields": { "automation": true } })).unwrap_err().contains("unknown or human-only"));
    control(&mut h, "ui.set", serde_json::json!({ "fields": { "tool": "Pan", "activeLayer": "T1", "zoom": 3.0 } })).unwrap();
    assert_eq!((h.state().ui.selected_tool, h.state().ui.active_layer, h.state().view.zoom), ("Pan", LayerKey::Tool(1), Some(3.0)));
    control(&mut h, "ui.drag", serde_json::json!({ "from": { "x": 10, "y": 20 }, "to": { "x": 50, "y": 70 } })).unwrap();
    assert_eq!(h.state().input_steps.len(), 4);
}

#[test]
fn control_refuses_conflicting_command_identifiers() {
    let mut h = with_human(NoSurface);
    // Two command-identifying fields are refused even when they name the same command: nothing may be
    // authorized against one id and executed as the other.
    for params in [
        serde_json::json!({ "id": "file.new", "command": "file.new" }),
        serde_json::json!({ "id": "layer.setShow", "command": "file.new" }),
        serde_json::json!({ "command": "file.new", "id": "layer.setShow" }),
    ] {
        for method in ["engine.execute", "ui.menu.invoke"] {
            let error = control(&mut h, method, params.clone()).unwrap_err();
            assert!(error.contains("conflicting command identifiers"), "{method} {params} → {error}");
        }
    }
    // The dispatcher-bypass shape: a harmless `command` must not let a mutating `id` through while busy.
    h.state_mut().seam.machine_state = Some(MachineState::Framing);
    let error = control(&mut h, "ui.menu.invoke", serde_json::json!({ "command": "machine.status", "id": "file.new" })).unwrap_err();
    assert!(error.contains("conflicting command identifiers"), "{error}");
    // Single-field requests still reach the dispatcher.
    let mut h = with_human(NoSurface);
    control(&mut h, "engine.execute", serde_json::json!({ "command": "file.new" }))
        .unwrap_or_else(|e| panic!("a single-field request must reach the dispatcher: {e}"));
    let error = control(&mut h, "ui.menu.invoke", serde_json::json!({ "id": "window.show:console" })).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(error["panel"], "console");
}

// ---- Device Settings is a human-only surface --------------------------------------------------------------------------------

#[test]
fn device_settings_draft_is_untouchable_by_agents() {
    // With the Device Settings window open, every mutating control method is refused and queued synthetic
    // input is dropped: the draft is only ever mutated by this dialog's own widgets, driven by a person.
    let mut h = with_human(SyntheticInjector);
    h.state_mut().ui.device_settings_open = true;
    h.run();
    let draft = h.state().device_draft.clone().unwrap();
    for (method, params) in [
        ("engine.execute", serde_json::json!({ "command": "file.new" })),
        ("ui.menu.invoke", serde_json::json!({ "id": "window.show:console" })),
        ("ui.set", serde_json::json!({ "fields": { "tool": "Pan" } })),
        ("ui.dialog.open", serde_json::json!({ "dialog": "cutSettings" })),
        ("ui.dock.close", serde_json::json!({ "panel": "console" })),
        ("app.quit", serde_json::json!({})),
    ] {
        let error = control(&mut h, method, params).unwrap_err();
        assert!(error.starts_with("human_confirmation_open"), "{method}: {error}");
    }
    // ui.click on the draft's own widgets never reaches egui: raw_input_hook drops queued synthetic events
    // while a human-only surface is open.
    for label in ["Reverse X direction", "Reverse Y direction", "Galvo 1 is X Axis"] {
        let center = h.get_by_label(label).rect().center();
        for event in crate::control::input::events_for("ui.click", &serde_json::json!({ "x": center.x, "y": center.y })).unwrap() {
            h.state_mut().queue_input(vec![event]);
            hooked_step(&mut h, Vec::new());
        }
    }
    h.run();
    assert_eq!(h.state().device_draft.as_ref(), Some(&draft), "synthetic input reached the human-only draft");
    // Reads are still served, and the surface is reported open.
    let inspect = control(&mut h, "ui.inspect", serde_json::json!({})).unwrap();
    assert_eq!(inspect["humanSurfaces"]["open"], true);
}

#[test]
fn device_settings_draft_reloads_when_the_profile_changes() {
    let mut h = with_human(NoSurface);
    h.state_mut().ui.device_settings_open = true;
    h.run();
    assert!(h.state().device_draft.is_some());
    // The profile changes under the open draft (an agent-side device command, another device selected): it
    // is reloaded with a visible notice, so a person's later Save cannot persist an edit they did not make.
    let mut changed = openlaser_machine::jpt_m7_200_sim();
    if let openlaser_machine::Kinematics::Galvo(g) = &mut changed.kind {
        g.cal.negate = [true, true];
    }
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    h.state_mut().session = Session::new(SessionConfig {
        origin: Origin::Human,
        window: true,
        devices: DeviceAccess::None,
        machines: vec![changed.clone()],
        profiles: None,
        files: FsAuthority::Denied,
        clock,
    });
    h.run();
    assert_eq!(h.state().device_draft.as_ref(), Some(&changed), "the stale draft must be discarded and reloaded");
    assert!(h.state().ui.status_message.contains("reloaded"), "the reload must be visible: {}", h.state().ui.status_message);
    // Closing drops both draft and source; reopening clones a fresh draft.
    h.state_mut().ui.device_settings_open = false;
    h.run();
    h.state_mut().ui.device_settings_open = true;
    h.run();
    assert_eq!(h.state().device_draft.as_ref(), Some(&changed));
}

// ---- menus -----------------------------------------------------------------------------------------------------------------

#[test]
fn submenus_are_nested() {
    let mut h = harness();
    h.run();
    h.get_by_label("File").click();
    h.run();
    assert!(h.query_by_label("Import Prefs…").is_none(), "a submenu row is flattened into File");
    h.query_all_by_label_contains("Preferences").next().unwrap().hover();
    h.run_steps(20);
    assert!(h.query_by_label("Import Prefs…").is_some());
    assert!(h.query_all_by_label_contains("Save As Template").next().unwrap().accesskit_node().is_disabled(), "unbuilt rows render disabled");
}

#[test]
fn in_window_menu_flag_is_respected_on_both_platforms() {
    assert!(!show_in_window_menus(false, true));
    assert!(show_in_window_menus(true, true));
    assert!(show_in_window_menus(false, false));
    assert!(show_in_window_menus(true, false));
    let mut h = harness_with(Options { settings: UiSettings { in_window_menus: false, ..UiSettings::default() }, ..Options::default() });
    h.run();
    assert_eq!(h.query_by_label("Laser Tools").is_some(), !cfg!(target_os = "macos"), "macOS uses the native menu unless asked");
    h.state_mut().set_in_window_menus(true);
    h.run();
    assert!(h.query_by_label("Laser Tools").is_some(), "--in-window-menus draws the menu bar on every platform");
}

/// The disabled reason must name the predicate that actually failed in `live && !blocked`, not a catch-all.
#[test]
fn disabled_menu_items_name_the_predicate_that_failed() {
    fn item<'h>(h: &'h mut H, menu: &str, text: &'h str) -> egui_kittest::Node<'h> {
        if h.query_all_by_label_contains(text).next().is_none() {
            h.get_by_label(menu).click();
            h.run();
        }
        h.query_all_by_label_contains(text).next().unwrap_or_else(|| panic!("{menu} has no row {text}"))
    }
    let mut h = with_human(NoSurface);
    h.state_mut().seam.machine_state = Some(MachineState::Framing);
    h.run();
    for (menu, text, reason) in [
        // `Save As Template…` is not implemented and files are blocked: the honest reason is "not in OpenLaser yet".
        ("File", "Save As Template…", "Not in OpenLaser yet"),
        // `Import…` is implemented: an active machine is the actual blocker.
        ("File", "Import…", "Stop the active frame or job"),
        // A hardware row keeps its human-only reason.
        ("Tools", "Position Laser", "Hardware actions are human-only"),
    ] {
        let node = item(&mut h, menu, text);
        assert!(node.accesskit_node().is_disabled(), "{text} must be disabled");
        node.hover();
        h.run_steps(20);
        assert!(has_prefix(&h, reason), "hovering `{text}` did not show `{reason}`");
        h.hover_at(egui::pos2(1.0, 1.0));
        h.run_steps(5);
    }
}

// ---- accessibility, canvas, dock ---------------------------------------------------------------------------------------

#[test]
fn kittest_accessibility_labels() {
    use egui::accesskit::Role;
    let mut h = with_human(NoSurface);
    h.state_mut().seam.project = Some(used_project());
    ready_plan(&mut h);
    h.state_mut().ui.settings_open = true;
    h.state_mut().ui.device_settings_open = true;
    h.state_mut().ui.live_framing_open = true;
    h.state_mut().ui.preview_open = true;
    h.run();
    let mut interactive = 0;
    let mut roles = std::collections::BTreeSet::new();
    for node in h.root().children_recursive() {
        let node = node.accesskit_node();
        let role = node.role();
        if matches!(
            role,
            Role::Button
                | Role::CheckBox
                | Role::TextInput
                | Role::NumberInput
                | Role::SpinButton
                | Role::Slider
                | Role::RadioButton
                | Role::Switch
                | Role::MenuItem
                | Role::MenuItemCheckBox
                | Role::MenuItemRadio
                | Role::ComboBox
                | Role::Tab
                | Role::Link
        ) {
            interactive += 1;
            roles.insert(format!("{role:?}"));
            assert!(node.label().is_some_and(|s| !s.trim().is_empty()), "unlabelled interactive node: {node:?}");
        }
    }
    assert!(interactive > 120, "the whole tree was not walked: {interactive}");
    for role in ["Button", "CheckBox", "ComboBox", "SpinButton", "RadioButton"] {
        assert!(roles.contains(role), "no {role} was checked: {roles:?}");
    }
}

#[test]
fn canvas_draws_rulers_grid_field_work_area_and_origin() {
    let mut h = harness();
    h.run();
    let colors = crate::view::palette(h.ctx.theme() == egui::Theme::Dark);
    let check = |h: &H| {
        let shapes = &h.output().shapes;
        let segments = |color: egui::Color32| -> Vec<[egui::Pos2; 2]> {
            shapes
                .iter()
                .filter_map(|s| if let egui::Shape::LineSegment { points, stroke } = &s.shape { (stroke.color == color).then_some(*points) } else { None })
                .collect()
        };
        // Rulers: numeric labels along the top strip (one y) and the left strip (one x).
        let labels: Vec<egui::Pos2> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                // Ruler labels only (monospace 9 pt): the palette's "00".."29" are numbers too.
                egui::Shape::Text(t) if t.galley.job.sections.first().is_some_and(|f| f.format.font_id == egui::FontId::monospace(9.0)) => {
                    t.galley.text().parse::<f64>().is_ok().then_some(t.pos)
                }
                _ => None,
            })
            .collect();
        let most = |key: &dyn Fn(&egui::Pos2) -> i32| {
            let mut counts = std::collections::BTreeMap::new();
            for p in &labels {
                *counts.entry(key(p)).or_insert(0) += 1;
            }
            counts.values().copied().max().unwrap_or(0)
        };
        assert!(most(&|p| p.y.round() as i32) >= 3, "top ruler labels: {labels:?}");
        assert!(most(&|p| p.x.round() as i32) >= 3, "left ruler labels: {labels:?}");
        // Grid: minor and major lines in the work area.
        assert!(segments(colors.grid_minor).len() >= 10, "minor grid");
        assert!(segments(colors.grid_major).len() >= 4, "major grid");
        // The encodable field: dashed (many short segments) and labelled.
        assert!(segments(colors.field).len() >= 16, "dashed encodable field");
        assert!(shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "encodable")));
        // The work area outline.
        assert!(shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.stroke.color == colors.work_area)), "work area");
        // The centre cross sits at world (0, 0).
        let cross = segments(colors.cross);
        assert_eq!(cross.len(), 2, "centre cross");
        (cross[0][0] + (cross[0][1] - cross[0][0]) * 0.5, cross[1][0] + (cross[1][1] - cross[1][0]) * 0.5)
    };
    let (a, b) = check(&h);
    assert!(a.distance(b) < 0.01, "the two cross strokes share a centre");
    // Pan 10 mm right: the origin cross moves 10 mm × zoom to the left.
    let zoom = h.state().view.zoom.unwrap();
    h.state_mut().view.center[0] += 10.0;
    h.run();
    let (moved, _) = check(&h);
    assert!(((a.x - moved.x) - (10.0 * zoom) as f32).abs() < 0.5, "cross at {a:?} → {moved:?} (zoom {zoom})");
    let fit = h.state().view.zoom.unwrap();
    assert!(fit > 0.5 && fit < 5.0, "{fit}");
}

#[test]
fn canvas_wheel_pinch_zoom_pan_and_fit() {
    let mut h = harness();
    h.run();
    let canvas = h.get_by_label("Zoom to fit").rect();
    let p = egui::pos2(canvas.left() - 300.0, canvas.top() - 200.0);
    h.hover_at(p);
    h.run();
    h.get_by_label("Zoom to fit").click();
    h.run();
    h.hover_at(p);
    h.run();
    let fit = h.state().view.zoom.unwrap();
    let center = h.state().view.center;
    let world_before = h.state().ui.pointer_mm.unwrap();
    // Mouse wheel (lines) zooms about the cursor.
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        phase: egui::TouchPhase::Move,
        delta: egui::vec2(0.0, 2.0),
        modifiers: egui::Modifiers::NONE,
    });
    h.run();
    let zoomed = h.state().view.zoom.unwrap();
    assert!(zoomed > fit * 1.2, "wheel zoom: {fit} → {zoomed}");
    h.hover_at(p + egui::vec2(0.5, 0.0));
    h.run();
    let world_after = h.state().ui.pointer_mm.unwrap();
    assert!((world_before[0] - world_after[0]).abs() < 0.2 && (world_before[1] - world_after[1]).abs() < 0.2, "zoom must keep the point under the cursor");
    // Pinch.
    h.event(egui::Event::Zoom(1.5));
    h.run();
    assert!(h.state().view.zoom.unwrap() > zoomed * 1.4);
    // Trackpad scroll (points) pans.
    let before = h.state().view.center;
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        phase: egui::TouchPhase::Move,
        delta: egui::vec2(40.0, 0.0),
        modifiers: egui::Modifiers::NONE,
    });
    h.run();
    assert!(h.state().view.center[0] < before[0], "trackpad pan");
    // Middle-drag pans.
    let before = h.state().view.center;
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Middle, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    h.event(egui::Event::PointerMoved(p + egui::vec2(60.0, 30.0)));
    h.step();
    h.event(egui::Event::PointerButton {
        pos: p + egui::vec2(60.0, 30.0),
        button: egui::PointerButton::Middle,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run();
    assert!(h.state().view.center[0] < before[0] && h.state().view.center[1] > before[1], "middle-drag pan");
    // Zoom to fit restores the fitted view.
    h.get_by_label("Zoom to fit").click();
    h.run();
    h.hover_at(p);
    h.run();
    let refit = h.state().view.zoom.unwrap();
    assert!((refit - fit).abs() < 1e-9, "fit {fit} → refit {refit}");
    assert_eq!(h.state().view.center, center);
}

#[test]
fn dock_layout_persists_and_bad_layouts_fall_back() {
    let settings = Arc::new(std::sync::Mutex::new(UiSettings { in_window_menus: true, ..UiSettings::default() }));
    let make = |settings: &Arc<std::sync::Mutex<UiSettings>>| {
        let s = settings.lock().unwrap().clone();
        let mut h = harness_with(Options { settings: s, ..Options::default() });
        h.state_mut().services.settings = Box::new(Settings(Arc::clone(settings)));
        h
    };
    let mut h = make(&settings);
    h.run();
    h.state_mut().show_panel(&serde_json::json!({ "panel": "laser", "visible": false })).unwrap();
    h.run();
    assert!(h.query_by_label("Connect").is_none());
    assert!(!settings.lock().unwrap().show_laser);
    let mut h = make(&settings);
    h.run();
    assert!(h.query_by_label("Connect").is_none(), "the closed Laser panel stays closed after a restart");
    settings.lock().unwrap().dock_layout = "{not json".into();
    let mut h = make(&settings);
    h.run();
    assert!(h.query_by_label("Connect").is_some(), "a malformed layout falls back to the default");
}

// ---- offscreen screenshot -----------------------------------------------------------------------------------------------

/// Writes a PNG of the default layout (CPU rasterised from egui's own tessellation) to `$OPENLASER_LAYOUT_PNG`.
#[test]
#[ignore = "writes a review PNG; run with OPENLASER_LAYOUT_PNG=<path> cargo test … -- --ignored layout_screenshot"]
fn layout_screenshot() {
    let Ok(path) = std::env::var("OPENLASER_LAYOUT_PNG") else { return };
    let textures = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let session = Session::new(SessionConfig {
        origin: Origin::Human,
        window: true,
        devices: DeviceAccess::None,
        machines: vec![openlaser_machine::jpt_m7_200_sim()],
        profiles: None,
        files: FsAuthority::Denied,
        clock: Arc::clone(&clock),
    });
    let services = Services {
        clock,
        human: Box::new(NoSurface),
        files: Box::new(Files::default()),
        settings: Box::new(Settings(Arc::new(std::sync::Mutex::new(UiSettings { in_window_menus: true, ..UiSettings::default() })))),
        clipboard: Box::new(Clip),
        automation: Box::new(Automation),
        quit: Box::new(|| {}),
    };
    let size = egui::vec2(1440.0, 900.0);
    let mut h = egui_kittest::Harness::builder()
        .with_size(size)
        .with_step_dt(0.05)
        .renderer(crate::screenshot::tests_raster::Recorder(Arc::clone(&textures)))
        .build_eframe(move |cc| OpenLaserApp::new(cc, session, services));
    // OPENLASER_LAYOUT_THEME=light renders the light theme (the default render is dark).
    if std::env::var("OPENLASER_LAYOUT_THEME").is_ok_and(|t| t == "light") {
        h.ctx.set_theme(egui::Theme::Light);
    }
    h.state_mut().seam.project = Some(used_project());
    ready_plan(&mut h);
    h.run();
    h.run_steps(5);
    let shapes = h.output().shapes.clone();
    let ppp = h.output().pixels_per_point;
    let primitives = h.ctx.tessellate(shapes, ppp);
    let image = crate::screenshot::tests_raster::rasterize(&primitives, &textures.lock().unwrap(), size, ppp);
    let png = crate::screenshot::png_bytes(&image).unwrap();
    std::fs::write(&path, png).unwrap();
}
