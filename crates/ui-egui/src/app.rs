//! The OpenLaser window (SPEC §3.19, UI §4): state, the safety core (STOP, Esc, close guard), input provenance and
//! the eframe hooks. Panels live in `panels.rs`, dialogs in `dialogs.rs`, the canvas in `view.rs`.
//!
//! Every product action dispatches an engine command by id with the frame's provenance: `Origin::Synthetic`
//! whenever `services.human.frame_is_synthetic()` says this frame carries automation input, `Origin::Human`
//! otherwise (§4.6 mechanism 3). Human-only widgets act only through `services.human.is_human_click`.

use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::Receiver;

use openlaser_control::server::ControlRequest;
use openlaser_engine::{EmissionSummary, JobRefused, MachineId, MachineState, Origin, Session, StopReport};
use openlaser_types::{FrameReceipt, FrameSource, FrameStyle, LayerKey, Notice, Refusal};

use crate::services::{DialogKind, DialogTicket, Services};

/// Window and panel state that is not document state (the document lives in the engine's `Session`).
#[derive(Debug)]
pub struct UiState {
    /// The layer new shapes go to and the Cuts / Layers row the quick editor shows.
    pub active_layer: LayerKey,
    pub selected_tool: &'static str,
    pub cut_settings_open: bool,
    pub preview_open: bool,
    pub settings_open: bool,
    pub device_settings_open: bool,
    pub live_framing_open: bool,
    pub live_style: FrameStyle,
    pub live_source: FrameSource,
    /// Numeric edits: keep the aspect ratio when W/H/W%/H% change.
    pub lock_ratio: bool,
    /// Numeric edits: which of the 9 points of the selection bounds XPos/YPos name (0 = top-left … 8 = bottom-right).
    pub anchor: u8,
    /// A machine is framing, running or stopping, or the ARM dialog is open: STOP chrome is up, files are refused.
    pub machine_active: bool,
    pub status_message: String,
    pub pointer_mm: Option<[f64; 2]>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            active_layer: LayerKey::Color(0),
            selected_tool: "Select",
            cut_settings_open: false,
            preview_open: false,
            settings_open: false,
            device_settings_open: false,
            live_framing_open: false,
            live_style: FrameStyle::Contour,
            live_source: FrameSource::Job,
            lock_ratio: true,
            anchor: 4,
            machine_active: false,
            status_message: String::new(),
            pointer_mm: None,
        }
    }
}

/// What the planner says about the active document, cached per (document, revision, machine) so the Laser panel,
/// the Cuts table, the status bar and the canvas share one `build_job` per revision (display only, never armed).
#[derive(Debug, Clone)]
pub(crate) enum PlanView {
    Ready(Box<EmissionSummary>),
    Refused(Box<JobRefused>),
}

impl PlanView {
    pub(crate) fn summary(&self) -> Option<&EmissionSummary> {
        match self {
            PlanView::Ready(summary) => Some(summary),
            PlanView::Refused(_) => None,
        }
    }

    pub(crate) fn refusals(&self) -> &[Refusal] {
        match self {
            PlanView::Ready(_) => &[],
            PlanView::Refused(refused) => &refused.refusals,
        }
    }

    pub(crate) fn notices(&self) -> &[Notice] {
        match self {
            PlanView::Ready(summary) => &summary.notices,
            PlanView::Refused(refused) => &refused.notices,
        }
    }
}

type PlanKey = (usize, u64, MachineId);

/// One interactive widget drawn in the last frame, for `ui.elements` (label = its AccessKit label).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Element {
    pub label: String,
    pub role: &'static str,
    /// x, y, width, height in points.
    pub rect: [f32; 4],
    pub enabled: bool,
    pub human_only: bool,
}

/// Waiting for the StopReports of the machines a stop latched.
pub(crate) struct StopWait {
    /// Machines without a report yet, with the report they had before this stop (a new one must differ).
    pending: BTreeMap<MachineId, Option<StopReport>>,
    not_confirmed: bool,
    #[cfg(test)]
    seam_waiting: bool,
}

#[derive(Debug, Default)]
pub(crate) struct CloseState {
    /// A close arrived while a machine was active: stopped, waiting for the report before closing.
    pub after_stop: bool,
    /// The stop was confirmed and `services.quit` asked to close; the next close request passes.
    pub authorized: bool,
}

/// Test seam (unit tests only): stands in for the engine and driver until I2 lands (the WP0 engine has no
/// documents, no plan and no driver status). It never exists in a normal build.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Seam {
    /// The emulated machine's state.
    pub machine_state: Option<MachineState>,
    /// An emulated `PendingArm` (the ARM dialog is open).
    pub pending_arm: bool,
    /// How many times `stop()` latched everything.
    pub stops: usize,
    /// The emulated driver's StopReport after a stop: None = not arrived yet.
    pub stop_report: Option<bool>,
    pub plan: Option<PlanView>,
    pub receipt: Option<FrameReceipt>,
    pub project: Option<std::sync::Arc<openlaser_doc::Project>>,
    pub selection: Vec<openlaser_types::ObjectId>,
    pub polls: usize,
    /// Every widget dispatch: (origin, command id, params).
    pub dispatched: Vec<(Origin, String, serde_json::Value)>,
    /// Human-only actions that passed the provenance gate.
    pub human_actions: Vec<&'static str>,
}

type ScreenshotRequest = (serde_json::Value, u32, std::sync::mpsc::Sender<openlaser_control::wire::Reply>, std::time::Instant);

pub struct OpenLaserApp {
    pub(crate) session: Session,
    pub(crate) ui: UiState,
    pub(crate) services: Services,
    pub(crate) control_rx: Option<Receiver<ControlRequest>>,
    pub(crate) synthetic_input: Vec<egui::Event>,
    pub(crate) input_steps: VecDeque<Vec<egui::Event>>,
    pub(crate) screenshots: Vec<ScreenshotRequest>,
    pub(crate) pending_dialogs: Vec<(DialogTicket, DialogKind, Origin)>,
    pub(crate) close: CloseState,
    pub(crate) stop_wait: Option<StopWait>,
    pub(crate) last_layer_click: Option<(LayerKey, f64)>,
    pub(crate) cut_draft: Option<crate::cut_editor::Draft>,
    pub(crate) device_draft: Option<openlaser_engine::MachineProfile>,
    /// The profile `device_draft` was cloned from. If the session's profile no longer matches (it changed under
    /// the open dialog), the draft is discarded and reloaded with a visible notice, so a human's Save never
    /// persists a change the person did not make.
    pub(crate) device_draft_source: Option<openlaser_engine::MachineProfile>,
    pub(crate) frame_synthetic: bool,
    pub(crate) dock: crate::dock::DockLayout,
    pub(crate) view: crate::view::CanvasView,
    pub(crate) plan_cache: Option<(PlanKey, PlanView)>,
    pub(crate) in_window_menus: bool,
    pub(crate) elements: Vec<Element>,
    pub(crate) last_elements: Vec<Element>,
    pub(crate) stop_overlay_rect: Option<egui::Rect>,
    pub(crate) log: VecDeque<String>,
    #[cfg(test)]
    pub(crate) seam: Seam,
}

const LOG_LINES: usize = 200;

impl OpenLaserApp {
    pub fn new(cc: &eframe::CreationContext<'_>, session: Session, services: Services) -> Self {
        Self::with_context(&cc.egui_ctx, session, services)
    }

    pub(crate) fn with_context(ctx: &egui::Context, session: Session, services: Services) -> Self {
        egui_extras::install_image_loaders(ctx);
        let saved = services.settings.load();
        Self {
            session,
            ui: UiState::default(),
            in_window_menus: saved.in_window_menus,
            dock: crate::dock::decode(&saved.dock_layout),
            services,
            control_rx: None,
            synthetic_input: Vec::new(),
            input_steps: VecDeque::new(),
            screenshots: Vec::new(),
            pending_dialogs: Vec::new(),
            close: CloseState::default(),
            stop_wait: None,
            last_layer_click: None,
            cut_draft: None,
            device_draft: None,
            device_draft_source: None,
            frame_synthetic: true,
            view: crate::view::CanvasView::default(),
            plan_cache: None,
            elements: Vec::new(),
            last_elements: Vec::new(),
            stop_overlay_rect: None,
            log: VecDeque::new(),
            #[cfg(test)]
            seam: Seam::default(),
        }
    }

    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// `--in-window-menus` (and the persisted preference): draw the egui menu bar even on macOS.
    pub fn set_in_window_menus(&mut self, on: bool) {
        self.in_window_menus = on;
    }

    /// Opens the files named on the command line (a person typed them: human origin).
    pub fn open_startup_files(&mut self, files: &[std::path::PathBuf]) {
        for path in files {
            let id = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("olp")) { "file.open" } else { "file.import" };
            if let Err(e) = self.session.execute_as(Origin::Human, id, serde_json::json!({ "path": path })) {
                self.set_status(format!("{}: {e}", path.display()));
            }
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub(crate) fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    pub fn ui_state(&self) -> &UiState {
        &self.ui
    }

    pub(crate) fn set_status(&mut self, message: impl Into<String>) {
        let message = message.into();
        if self.log.back() != Some(&message) {
            self.log.push_back(message.clone());
            while self.log.len() > LOG_LINES {
                self.log.pop_front();
            }
        }
        self.ui.status_message = message;
    }

    // ---- machine state and the safety core ------------------------------------------------------------------------

    /// The state of every machine the session knows (driver events folded by `Session::poll`).
    pub(crate) fn machine_states(&self) -> Vec<(MachineId, MachineState)> {
        #[cfg(test)]
        if let Some(state) = &self.seam.machine_state {
            let id = self.session.machines().first().map(|m| m.id.clone()).unwrap_or_else(|| MachineId("seam".into()));
            return vec![(id, state.clone())];
        }
        self.session.machines().iter().filter_map(|m| self.session.status(&m.id).map(|s| (m.id.clone(), s.state.clone()))).collect()
    }

    /// A machine is framing, running, stopping, or its stop was not confirmed.
    pub(crate) fn is_machine_busy(&self) -> bool {
        self.machine_states().iter().any(|(_, state)| state_requires_stop(state))
    }

    /// The ARM dialog is open (a `PendingArm` exists).
    pub(crate) fn arm_dialog_open(&self) -> bool {
        #[cfg(test)]
        if self.seam.pending_arm {
            return true;
        }
        self.session.pending_arm().is_some()
    }

    /// Framing, armed, marking, stopping or not confirmed: Esc is STOP, the overlay shows, files are refused.
    pub(crate) fn safety_active(&self) -> bool {
        self.is_machine_busy() || self.arm_dialog_open()
    }

    pub(crate) fn human_surface_open(&self) -> bool {
        // The Device Settings draft is human-only too: while its window is open the dispatcher refuses
        // mutating methods and queued synthetic input is dropped, so an agent can never edit the draft a
        // person may later save (§4.6).
        self.services.human.surface_open() || self.ui.device_settings_open
    }

    /// STOP: latch every machine first (the engine voids `PendingArm` and stale submissions), then remember which
    /// reports are outstanding. Never refused, never queued, never behind a permission (§1.3 rule 5).
    pub(crate) fn stop(&mut self) -> Vec<(MachineId, openlaser_engine::StopAck)> {
        let before: BTreeMap<MachineId, Option<StopReport>> =
            self.session.machines().iter().map(|m| (m.id.clone(), self.session.status(&m.id).and_then(|s| s.last_stop.clone()))).collect();
        let acks = self.session.stop_all();
        self.session.cancel_arm();
        let mut wait = StopWait {
            pending: BTreeMap::new(),
            not_confirmed: false,
            #[cfg(test)]
            seam_waiting: false,
        };
        for (id, ack) in &acks {
            match &ack.report {
                Some(report) if report.confirmed => {}
                Some(_) => wait.not_confirmed = true,
                None => {
                    wait.pending.insert(id.clone(), before.get(id).cloned().flatten());
                }
            }
        }
        #[cfg(test)]
        {
            self.seam.stops += 1;
            self.seam.pending_arm = false;
            wait.seam_waiting = self.seam.machine_state.as_ref().is_some_and(state_requires_stop);
        }
        self.stop_wait = Some(wait);
        self.set_status(format!("Stop: {} machine(s) latched", acks.len()));
        acks
    }

    pub(crate) fn stop_from_control(&mut self) -> Vec<(MachineId, openlaser_engine::StopAck)> {
        self.stop()
    }

    /// Some(true): every latched machine reported a confirmed stop (or none was running); Some(false): a report said
    /// the stop is NOT confirmed; None: still waiting.
    pub(crate) fn stop_settled(&mut self) -> Option<bool> {
        let Some(wait) = &mut self.stop_wait else { return Some(true) };
        if wait.not_confirmed {
            return Some(false);
        }
        let session = &self.session;
        let mut failed = false;
        wait.pending.retain(|id, before| match session.status(id).and_then(|s| s.last_stop.as_ref()) {
            Some(report) if Some(report) != before.as_ref() => {
                failed |= !report.confirmed;
                false
            }
            _ => true,
        });
        if failed {
            wait.not_confirmed = true;
            return Some(false);
        }
        #[cfg(test)]
        if wait.seam_waiting {
            match self.seam.stop_report {
                None => return None,
                Some(false) => {
                    wait.not_confirmed = true;
                    return Some(false);
                }
                Some(true) => wait.seam_waiting = false,
            }
        }
        wait.pending.is_empty().then_some(true)
    }

    /// Close guard (UI §7.5): a close while a machine is active is cancelled, STOP is tripped, and the window closes
    /// only after the StopReport confirms the stop. Runs in `logic`; the app calls `craft_quit::on_close_request` after.
    pub(crate) fn guard_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.close.authorized && (self.close.after_stop || self.safety_active()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.close.after_stop {
                self.close.after_stop = true;
                self.stop();
            }
        }
        if self.close.after_stop {
            match self.stop_settled() {
                Some(true) => {
                    self.close.after_stop = false;
                    self.close.authorized = true;
                    self.set_status("Stop confirmed; closing");
                    (self.services.quit)();
                }
                Some(false) => {
                    self.close.after_stop = false;
                    self.set_status("STOP NOT CONFIRMED: the window stays open. Use the E-stop or the key switch.");
                }
                None => {
                    self.set_status("Stopping before closing: waiting for the stop report…");
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
            }
        }
    }

    // ---- provenance and dispatch ----------------------------------------------------------------------------------

    /// The origin of anything a widget does this frame.
    pub(crate) fn origin(&self) -> Origin {
        if self.frame_synthetic { Origin::Synthetic } else { Origin::Human }
    }

    /// Runs an engine command for a widget with this frame's provenance.
    pub(crate) fn execute_widget(&mut self, id: &str, params: serde_json::Value) {
        let origin = self.origin();
        #[cfg(test)]
        self.seam.dispatched.push((origin, id.to_owned(), params.clone()));
        let message = match self.session.execute_as(origin, id, params) {
            Ok(_) => format!("{}: done", crate::menu_catalog::find(id).map(|r| r.label).unwrap_or(id)),
            Err(e) => e.to_string(),
        };
        self.set_status(message);
    }

    /// A menu row, toolbar button, native menu item or shortcut by id. Recomputes provenance from the frame.
    pub fn invoke_menu_command(&mut self, ctx: &egui::Context, id: &str) {
        self.frame_synthetic = self.services.human.frame_is_synthetic(ctx);
        self.invoke_ui_action(id);
    }

    pub(crate) fn invoke_ui_action(&mut self, id: &str) {
        if self.safety_active() && opens_files(id) {
            self.set_status("Stop the active frame or job before opening, importing or saving files.");
            return;
        }
        if let Some(panel) = id.strip_prefix("window.show:") {
            if let Err(e) = self.show_panel(&serde_json::json!({ "panel": panel, "visible": true })) {
                self.set_status(e);
            }
            return;
        }
        match id {
            "edit.settings" => self.ui.settings_open = true,
            "edit.deviceSettings" | "device.settings" | "device.manage" => self.ui.device_settings_open = true,
            "laser.liveFraming" => self.ui.live_framing_open = true,
            "preview.open" => self.ui.preview_open = true,
            "file.exit" => (self.services.quit)(),
            "file.open" | "file.import" => {
                let kind = if id == "file.open" { DialogKind::OpenProject } else { DialogKind::Import };
                let ticket = self.services.files.open(kind);
                let origin = self.origin();
                self.pending_dialogs.push((ticket, kind, origin));
            }
            "view.zoomPage" => self.view.fit(),
            "view.zoomIn" => self.view.zoom_by(std::f64::consts::SQRT_2),
            "view.zoomOut" => self.view.zoom_by(std::f64::consts::FRAC_1_SQRT_2),
            "view.pan" => self.ui.selected_tool = "Pan",
            "tools.select" => {
                self.ui.selected_tool = "Select";
                if openlaser_engine::registry().find(id).is_some() {
                    self.execute_widget(id, serde_json::Value::Null);
                }
            }
            "edit.selectNone" => {
                self.ui.selected_tool = "Select";
                self.execute_widget(id, serde_json::json!({}));
            }
            _ if crate::view::DRAW_TOOLS.iter().any(|(d, _)| *d == id) => {
                // A draw tool is only armed; the canvas drag or clicks produce the command's mm parameters.
                if let Some((_, label)) = crate::view::DRAW_TOOLS.iter().find(|(d, _)| *d == id) {
                    self.ui.selected_tool = label;
                }
            }
            "window.resetLayout" => self.dock = crate::dock::default_layout(),
            _ => self.execute_widget(id, serde_json::Value::Null),
        }
    }

    pub(crate) fn show_panel(&mut self, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        use craft_ui::docking::Action;
        let panel = params.get("panel").and_then(serde_json::Value::as_str).ok_or("panel is required")?.to_owned();
        let visible = params.get("visible").and_then(serde_json::Value::as_bool).ok_or("visible is required")?;
        if !crate::dock::PANELS.contains(&panel.as_str()) || panel == "canvas" {
            return Err(format!("unknown or protected panel `{panel}`"));
        }
        if visible && !self.dock.contains(&panel) {
            self.dock
                .apply_with_permissions(Action::Open { panel: panel.clone(), anchor: Some("cuts".into()) }, crate::dock::permissions)
                .map_err(|e| e.to_string())?;
        }
        if !visible && self.dock.contains(&panel) {
            self.dock.apply_with_permissions(Action::Close { panel: panel.clone() }, crate::dock::permissions).map_err(|e| e.to_string())?;
        }
        if visible {
            self.dock.apply_with_permissions(Action::Activate { panel: panel.clone() }, crate::dock::permissions).map_err(|e| e.to_string())?;
        }
        Ok(serde_json::json!({ "panel": panel, "visible": visible }))
    }

    pub(crate) fn queue_input(&mut self, events: Vec<egui::Event>) {
        self.synthetic_input.extend(events);
    }

    pub(crate) fn queue_drag(&mut self, events: Vec<egui::Event>) {
        self.input_steps.extend(events.into_iter().map(|event| vec![event]));
    }

    // ---- documents, plan, machines ------------------------------------------------------------------------------

    pub(crate) fn project(&self) -> Option<&openlaser_doc::Project> {
        #[cfg(test)]
        if let Some(project) = &self.seam.project {
            return Some(project);
        }
        self.session.active().map(|doc| doc.project.as_ref())
    }

    pub(crate) fn selection(&self) -> Vec<openlaser_types::ObjectId> {
        #[cfg(test)]
        if self.seam.project.is_some() {
            return self.seam.selection.clone();
        }
        self.session.active().map(|doc| doc.selection.ids.clone()).unwrap_or_default()
    }

    /// The device of the active document, else the first profile (the Simulator).
    pub(crate) fn active_machine(&self) -> Option<&openlaser_engine::MachineProfile> {
        let id = self.session.active().map(|doc| doc.machine.clone());
        id.and_then(|id| self.session.machines().iter().find(|m| m.id == id)).or_else(|| self.session.machines().first())
    }

    pub(crate) fn active_is_galvo(&self) -> bool {
        self.active_machine().is_none_or(|m| matches!(m.kind, openlaser_machine::Kinematics::Galvo(_)))
    }

    /// Recomputes the cached plan only when the document, its revision or its machine changed.
    pub(crate) fn refresh_plan(&mut self) {
        #[cfg(test)]
        if let Some(plan) = &self.seam.plan {
            self.plan_cache = Some(((0, 0, MachineId("seam".into())), plan.clone()));
            return;
        }
        let Some((index, revision, machine)) = self.session.active_index().zip(self.session.active()).map(|(i, d)| (i, d.revision, d.machine.clone())) else {
            self.plan_cache = None;
            return;
        };
        let key = (index, revision, machine);
        if self.plan_cache.as_ref().is_some_and(|(cached, _)| *cached == key) {
            return;
        }
        let view = match self.session.build_job(Some(index), &openlaser_engine::JobOptions::default()) {
            Ok(job) => PlanView::Ready(Box::new(job.summary)),
            Err(refused) => PlanView::Refused(Box::new(refused)),
        };
        self.plan_cache = Some((key, view));
    }

    pub(crate) fn plan(&self) -> Option<&PlanView> {
        self.plan_cache.as_ref().map(|(_, plan)| plan)
    }

    /// The machine's last FrameReceipt (from the driver, folded by `Session::poll`).
    pub(crate) fn receipt_for(&self, machine: &MachineId) -> Option<FrameReceipt> {
        #[cfg(test)]
        if let Some(receipt) = &self.seam.receipt {
            return Some(receipt.clone());
        }
        self.session.status(machine).and_then(|s| s.receipt.clone())
    }

    fn poll_session(&mut self) {
        #[cfg(test)]
        {
            self.seam.polls += 1;
        }
        for event in self.session.poll() {
            match event {
                openlaser_engine::Event::Status(message) => self.set_status(message),
                openlaser_engine::Event::ArmVoided { reason } => self.set_status(format!("ARM dialog closed: {reason}")),
                _ => {}
            }
        }
    }

    fn poll_dialogs(&mut self) {
        let mut pending = Vec::new();
        for (ticket, kind, origin) in std::mem::take(&mut self.pending_dialogs) {
            match self.services.files.poll(ticket) {
                None => pending.push((ticket, kind, origin)),
                Some(None) => self.set_status("File selection cancelled"),
                Some(Some((path, _bytes))) => {
                    if self.safety_active() {
                        self.set_status("A file was chosen while a machine is active; it was not opened. Stop first.");
                        continue;
                    }
                    let id = if kind == DialogKind::Import { "file.import" } else { "file.open" };
                    let name = std::path::Path::new(&path).file_name().and_then(|n| n.to_str()).unwrap_or("file").to_owned();
                    let message = match self.session.execute_as(origin, id, serde_json::json!({ "path": path })) {
                        Ok(_) => format!("Opened {name}"),
                        Err(e) => e.to_string(),
                    };
                    self.set_status(message);
                }
            }
        }
        self.pending_dialogs = pending;
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<std::path::PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        for path in dropped {
            if self.safety_active() {
                self.set_status("Stop the active frame or job before importing files.");
                return;
            }
            let id = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("olp")) { "file.open" } else { "file.import" };
            self.execute_widget(id, serde_json::json!({ "path": path }));
        }
    }

    fn drain_control(&mut self, ctx: &egui::Context) {
        if let Some(rx) = self.services.automation.take_requests() {
            self.control_rx = Some(rx);
        }
        let requests: Vec<ControlRequest> = self.control_rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        for request in requests {
            let reply_tx = request.reply.clone();
            if request.deadline <= std::time::Instant::now() || !request.dispatch.try_dispatch() {
                let _ = reply_tx.send(crate::control::error_reply(request.request.id.clone(), "control request expired or was already settled"));
                continue;
            }
            match crate::control::handle(self, ctx, request) {
                crate::control::Outcome::Done(reply) | crate::control::Outcome::AfterInput(reply) => {
                    let _ = reply_tx.send(reply);
                }
                crate::control::Outcome::Screenshot { id, max_side, reply } => self.screenshots.push((id, max_side, reply, std::time::Instant::now())),
            }
        }
    }

    fn answer_screenshots(&mut self, ctx: &egui::Context) {
        let image = ctx.input(|i| {
            i.raw.events.iter().rev().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(std::sync::Arc::clone(image)) } else { None })
        });
        if let Some(image) = image {
            for (id, side, reply, _) in self.screenshots.drain(..) {
                let _ = reply.send(match crate::screenshot::encode(&image, side) {
                    Ok(result) => openlaser_control::wire::Reply { id, ok: true, result: Some(result), error: None },
                    Err(error) => crate::control::error_reply(id, &error),
                });
            }
        }
        self.screenshots.retain(|(id, _, reply, started)| {
            let expired = started.elapsed() > std::time::Duration::from_secs(5);
            if expired {
                let _ = reply.send(crate::control::error_reply(id.clone(), "screenshot timed out"));
            }
            !expired
        });
        if !self.screenshots.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn persist_ui_settings(&mut self) {
        let mut settings = self.services.settings.load();
        let dock = crate::services::normalize_dock_layout(&crate::dock::encode(&self.dock));
        let show_laser = self.dock.contains(&"laser".to_owned());
        let show_cuts = self.dock.contains(&"cuts".to_owned());
        if settings.dock_layout != dock || settings.show_laser != show_laser || settings.show_cuts != show_cuts {
            settings.dock_layout = dock;
            settings.show_laser = show_laser;
            settings.show_cuts = show_cuts;
            self.services.settings.save(&settings);
        }
    }

    // ---- elements (`ui.elements`) -------------------------------------------------------------------------------

    /// Records an interactive widget drawn this frame under its AccessKit label.
    pub(crate) fn track(&mut self, response: &egui::Response, role: &'static str, label: impl Into<String>, human_only: bool) {
        let r = response.rect;
        self.elements.push(Element { label: label.into(), role, rect: [r.min.x, r.min.y, r.width(), r.height()], enabled: response.enabled(), human_only });
    }

    /// Sets the AccessKit label of a widget whose visible content is an icon or a short glyph, and records it.
    pub(crate) fn labeled(&mut self, response: &egui::Response, kind: egui::WidgetType, label: &str, human_only: bool) {
        let enabled = response.enabled();
        let text = label.to_owned();
        response.widget_info(|| egui::WidgetInfo::labeled(kind, enabled, text.clone()));
        let role = match kind {
            egui::WidgetType::Checkbox => "checkBox",
            egui::WidgetType::DragValue | egui::WidgetType::TextEdit => "input",
            egui::WidgetType::ComboBox => "comboBox",
            _ => "button",
        };
        self.track(response, role, label, human_only);
    }

    pub(crate) fn last_frame_elements(&self) -> &[Element] {
        &self.last_elements
    }
}

impl Drop for OpenLaserApp {
    /// Fail-safe stop (UI §7.5): Dock Quit, logout and `terminate:` end the event loop through
    /// `applicationWillTerminate`, which bypasses `CloseRequested`, so the close guard never ran. Dropping the app
    /// still owns a live session with live drivers, so run the same stop path (latch every machine, void a pending
    /// arm). A repeated stop is a no-op.
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

impl eframe::App for OpenLaserApp {
    /// (1) Esc while a machine is framing, armed or marking, or while the ARM dialog is open → STOP (latch first,
    /// PendingArm voided) and the Esc events are removed before any widget sees them; when idle, Esc passes on (a
    /// field reverts, a menu closes) and `logic` clears the selection. (2) Native menu key equivalents: none (the
    /// native items carry no accelerators; egui's shortcut table handles keys). (3) Queued synthetic input goes in
    /// only through `services.human.inject` (§2.3 rule 16), and is dropped while a human-only surface is open.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let esc_pressed = raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. }));
        if esc_pressed && self.safety_active() {
            self.stop();
            raw.events.retain(|e| !matches!(e, egui::Event::Key { key: egui::Key::Escape, .. }));
        }
        if self.human_surface_open() {
            self.synthetic_input.clear();
            self.input_steps.clear();
        }
        let mut queued = std::mem::take(&mut self.synthetic_input);
        if queued.is_empty()
            && let Some(step) = self.input_steps.pop_front()
        {
            queued = step;
        }
        self.services.human.inject(ctx, raw, queued);
        if !self.input_steps.is_empty() {
            ctx.request_repaint();
        }
    }

    /// Every frame, and while hidden whenever a repaint was requested: poll the session; drain the control channel;
    /// shortcuts (Synthetic if the frame is); idle Esc; file drops and dialogs; the plan cache; the 250 ms repaint
    /// while active; screenshots; the close guard last (the app then calls craft_quit).
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_synthetic = self.services.human.frame_is_synthetic(ctx);
        self.poll_session();
        self.drain_control(ctx);
        let text_focus = ctx.egui_wants_keyboard_input();
        let table = crate::shortcuts::table();
        let fired = ctx.input_mut(|input| crate::shortcuts::take_pressed(input, &table, text_focus));
        for id in fired {
            self.invoke_menu_command(ctx, id);
        }
        let idle_esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if idle_esc && !self.safety_active() && !text_focus && !ctx.any_popup_open() && ctx.memory(|m| m.top_modal_layer().is_none()) {
            self.invoke_ui_action("edit.selectNone");
        }
        self.handle_dropped_files(ctx);
        if !self.safety_active() {
            self.poll_dialogs();
        }
        self.refresh_plan();
        self.ui.machine_active = self.safety_active();
        if self.ui.machine_active {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        self.answer_screenshots(ctx);
        self.persist_ui_settings();
        self.guard_close(ctx);
    }

    /// The LightBurn layout (UI §4.1), the STOP overlay last at `Order::Tooltip`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame_synthetic = self.services.human.frame_is_synthetic(&ctx);
        self.ui.machine_active = self.safety_active();
        self.elements.clear();
        if show_in_window_menus(self.in_window_menus, cfg!(target_os = "macos")) {
            egui::Panel::top("menu_bar").show(ui, |ui| self.menu_bar(ui));
        }
        egui::Panel::top("main_toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::top("numeric_edits").show(ui, |ui| self.numeric_edits(ui));
        egui::Panel::bottom("status_bar").show(ui, |ui| self.status_bar(ui));
        egui::Panel::bottom("palette_bar").show(ui, |ui| self.palette_bar(ui));
        egui::Panel::bottom("arrange_bar").show(ui, |ui| self.arrange_bar(ui));
        egui::Panel::left("tools").resizable(false).exact_size(44.0).show(ui, |ui| self.tool_palette(ui));
        egui::CentralPanel::default().show(ui, |ui| self.dock_area(ui));
        self.dialogs(&ctx);
        self.draw_arm_dialog(&ctx);
        self.ui.machine_active = self.safety_active();
        self.stop_overlay(&ctx);
        self.last_elements = std::mem::take(&mut self.elements);
    }
}

/// Anything that opens a file dialog or replaces the document: refused while a machine is active (UI §7.2).
pub(crate) fn opens_files(id: &str) -> bool {
    matches!(id, "file.new" | "file.open" | "file.import" | "file.save" | "file.saveAs" | "file.export" | "file.saveMachineFile" | "file.openRecent")
}

pub(crate) fn state_requires_stop(state: &MachineState) -> bool {
    matches!(state, MachineState::Framing | MachineState::Running { .. } | MachineState::Stopping | MachineState::StopNotConfirmed)
}

/// macOS uses the native (muda) menu unless `--in-window-menus` (or the preference) asks for the egui one;
/// Windows and Linux always draw the in-window menu bar (SPEC §2.6).
pub(crate) fn show_in_window_menus(requested: bool, macos: bool) -> bool {
    requested || !macos
}

#[cfg(test)]
#[path = "tests/app_kittests.rs"]
mod tests;
