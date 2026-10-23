//! Native editor shell. Product-specific operations dispatch through the engine; window and view state live here.

use std::sync::mpsc::Receiver;

use openlaser_control::server::ControlRequest;
use openlaser_engine::{MachineState, Origin, Session};

use crate::canvas::CanvasRenderer;
use crate::services::Services;

#[derive(Debug, Default)]
pub struct UiState {
    pub active_layer: usize,
    pub selected_tool: &'static str,
    pub active_panel: &'static str,
    pub laser_floated: bool,
    pub cut_settings_open: bool,
    pub preview_open: bool,
    pub settings_open: bool,
    pub device_settings_open: bool,
    pub live_framing_open: bool,
    live_style: Option<openlaser_types::FrameStyle>,
    live_source: Option<openlaser_types::FrameSource>,
    pub lock_ratio: bool,
    pub anchor: u8,
    pub machine_active: bool,
    pub status_message: String,
    pub pointer_mm: Option<[f64; 2]>,
}

type ScreenshotRequest = (serde_json::Value, u32, std::sync::mpsc::Sender<openlaser_control::wire::Reply>, std::time::Instant);

type PlanCache = Option<(Option<(usize, u64)>, Result<openlaser_engine::BuiltJob, openlaser_engine::JobRefused>)>;

pub struct OpenLaserApp {
    session: Session,
    ui: UiState,
    services: Services,
    control_rx: Option<Receiver<ControlRequest>>,
    synthetic_input: Vec<egui::Event>,
    input_steps: std::collections::VecDeque<Vec<egui::Event>>,
    screenshots: Vec<ScreenshotRequest>,
    pending_dialogs: Vec<(crate::services::DialogTicket, crate::services::DialogKind, Origin)>,
    close_after_stop: bool,
    close_authorized: bool,
    last_layer_click: Option<(usize, f64)>,
    cut_draft: Option<crate::cut_editor::Draft>,
    device_draft: Option<openlaser_engine::MachineProfile>,
    stop_confirmed: bool,
    stop_waiting: std::collections::BTreeSet<openlaser_types::MachineId>,
    frame_synthetic: bool,
    dock: crate::dock::DockLayout,
    renderer: crate::canvas::PainterRenderer,
    canvas_zoom: Option<f64>,
    canvas_center: [f64; 2],
    plan_cache: PlanCache,
    #[cfg(test)]
    machine_state: Option<MachineState>,
    #[cfg(test)]
    stop_probe: Option<Box<dyn FnMut() -> bool + Send>>,
    #[cfg(test)]
    frame_ready: bool,
    #[cfg(test)]
    human_actions: usize,
    #[cfg(test)]
    pending_arm: bool,
    #[cfg(test)]
    polls: usize,
    #[cfg(test)]
    test_project: Option<std::sync::Arc<openlaser_doc::Project>>,
    #[cfg(test)]
    dispatched: Vec<(Origin, String, serde_json::Value)>,
}

impl OpenLaserApp {
    pub fn new(cc: &eframe::CreationContext<'_>, session: Session, services: Services) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut ui = UiState { selected_tool: "Select", active_panel: "Laser", ..UiState::default() };
        let saved = services.settings.load();
        ui.active_panel = if saved.show_laser { "Laser" } else { "Library" };
        Self {
            session,
            ui,
            services,
            control_rx: None,
            synthetic_input: Vec::new(),
            input_steps: Default::default(),
            screenshots: Vec::new(),
            pending_dialogs: Vec::new(),
            close_after_stop: false,
            close_authorized: false,
            last_layer_click: None,
            cut_draft: None,
            device_draft: None,
            stop_confirmed: false,
            stop_waiting: Default::default(),
            frame_synthetic: true,
            dock: crate::dock::decode(&saved.dock_layout),
            renderer: Default::default(),
            canvas_zoom: None,
            canvas_center: [0.0; 2],
            plan_cache: None,
            #[cfg(test)]
            machine_state: None,
            #[cfg(test)]
            stop_probe: None,
            #[cfg(test)]
            frame_ready: false,
            #[cfg(test)]
            human_actions: 0,
            #[cfg(test)]
            pending_arm: false,
            #[cfg(test)]
            polls: 0,
            #[cfg(test)]
            test_project: None,
            #[cfg(test)]
            dispatched: Vec::new(),
        }
    }

    pub fn open_startup_files(&mut self, files: &[std::path::PathBuf]) {
        for path in files {
            let result = self.session.execute_as(Origin::Human, "file.open", serde_json::json!({"path":path}));
            if let Err(e) = result {
                self.ui.status_message = e.to_string();
                break;
            }
        }
    }

    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
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

    fn stop(&mut self) {
        let was_busy = self.is_machine_busy();
        let reports = self.session.stop_all();
        self.stop_waiting = reports.iter().filter(|(_, ack)| !ack.report.as_ref().is_some_and(|report| report.confirmed)).map(|(id, _)| id.clone()).collect();
        self.session.cancel_arm();
        #[cfg(test)]
        {
            self.pending_arm = false;
        }
        self.stop_confirmed = (!was_busy && reports.is_empty())
            || (!reports.is_empty() && reports.iter().all(|(_, ack)| ack.report.as_ref().is_some_and(|report| report.confirmed)));
        #[cfg(test)]
        if let Some(probe) = &mut self.stop_probe {
            self.stop_confirmed = probe();
        }
        self.ui.status_message = "Stop requested".to_owned();
    }

    pub(crate) fn stop_from_control(&mut self) {
        self.stop();
    }

    pub(crate) fn is_machine_busy(&self) -> bool {
        #[cfg(test)]
        if let Some(state) = &self.machine_state {
            return state_requires_stop(state);
        }
        self.session.machines().iter().filter_map(|machine| self.session.status(&machine.id)).any(|status| state_requires_stop(&status.state))
    }

    pub(crate) fn arm_dialog_open(&self) -> bool {
        #[cfg(test)]
        if self.pending_arm {
            return true;
        }
        self.session.pending_arm().is_some()
    }
    pub(crate) fn human_surface_open(&self) -> bool {
        self.services.human.surface_open()
    }

    pub fn invoke_menu_command(&mut self, ctx: &egui::Context, id: &str) {
        self.frame_synthetic = self.services.human.frame_is_synthetic(ctx);
        if let Some(panel) = id.strip_prefix("window.show:") {
            let _ = self.show_panel(&serde_json::json!({"panel":panel,"visible":true}));
        } else {
            self.invoke_ui_action(id);
        }
    }

    pub(crate) fn show_panel(&mut self, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        use craft_ui::docking::Action;
        let panel = params.get("panel").and_then(serde_json::Value::as_str).ok_or("panel is required")?.to_owned();
        let visible = params.get("visible").and_then(serde_json::Value::as_bool).ok_or("visible is required")?;
        if !crate::dock::PANELS.contains(&panel.as_str()) || panel == "canvas" {
            return Err("unknown or protected panel".into());
        }
        if visible && !self.dock.contains(&panel) {
            self.dock
                .apply_with_permissions(Action::Open { panel: panel.clone(), anchor: Some("canvas".into()) }, crate::dock::permissions)
                .map_err(|e| e.to_string())?;
        }
        if !visible && self.dock.contains(&panel) {
            self.dock.apply_with_permissions(Action::Close { panel: panel.clone() }, crate::dock::permissions).map_err(|e| e.to_string())?;
        }
        if visible {
            self.dock.apply_with_permissions(Action::Activate { panel }, crate::dock::permissions).map_err(|e| e.to_string())?;
        }
        Ok(serde_json::json!({"visible":visible}))
    }

    fn guard_close(&mut self, ctx: &egui::Context) {
        if self.close_authorized {
            return;
        }
        if ctx.input(|input| input.viewport().close_requested()) && (self.close_after_stop || self.is_machine_busy() || self.arm_dialog_open()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.close_after_stop {
                self.close_after_stop = true;
                self.stop();
            }
        }
        if self.close_after_stop {
            // A state transition alone is insufficient: require a confirmed StopReport.
            if self.stop_confirmed {
                self.close_after_stop = false;
                self.close_authorized = true;
                (self.services.quit)();
            } else {
                self.ui.status_message = "Stop not confirmed; close refused. Check the machine and E-stop.".to_owned();
            }
        }
    }

    pub(crate) fn queue_input(&mut self, events: Vec<egui::Event>) {
        self.synthetic_input.extend(events);
    }
    pub(crate) fn queue_drag(&mut self, events: Vec<egui::Event>) {
        self.input_steps.extend(events.into_iter().map(|event| vec![event]));
    }
    pub(crate) fn element_labels(&self) -> Vec<String> {
        let mut labels: Vec<String> = ["Stop laser", "XPos", "YPos", "Width", "Height", "W%", "H%", "Rotate °"].into_iter().map(str::to_owned).collect();
        labels.extend((0..30).map(|n| format!("Layer {n:02}")));
        labels.extend(["T1".into(), "T2".into()]);
        let visible = crate::dock::visible_panels(&self.dock);
        labels.extend(visible.iter().map(|p| crate::dock::label(p)));
        if visible.iter().any(|p| p == "laser") {
            labels.extend(
                [
                    "Connect",
                    "Frame",
                    "Start",
                    "Device",
                    "Pause",
                    "Send",
                    "Home",
                    "Go to Origin",
                    "Repeat",
                    "Repeat count",
                    "Start From: Absolute · galvo uses field centre",
                    "Cut Selected Graphics",
                ]
                .into_iter()
                .map(str::to_owned),
            );
        }
        if visible.iter().any(|p| p == "cuts") {
            labels.extend(["#", "Layer", "Mode", "Spd/Pwr", "Output", "Show", "Quick edit", "Edit Cut Settings…"].into_iter().map(str::to_owned));
            if let Some(project) = self.project() {
                for index in used_layer_indices(project) {
                    let name = project.color_layer(index as u8).map(|l| l.name.clone()).unwrap_or_else(|| format!("T{}", index.saturating_sub(29)));
                    labels.extend([format!("Cut layer {name}"), format!("Output {name}"), format!("Show {name}")]);
                }
            }
        }

        labels
    }
    pub(crate) fn resize_window(&mut self, ctx: &egui::Context, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        let width = params.get("width").and_then(serde_json::Value::as_f64).ok_or("width is required")?;
        let height = params.get("height").and_then(serde_json::Value::as_f64).ok_or("height is required")?;
        if !(760.0..=16384.0).contains(&width) || !(480.0..=16384.0).contains(&height) {
            return Err("window size outside 760×480 to 16384×16384".into());
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(width as f32, height as f32)));
        Ok(serde_json::json!({}))
    }
    pub(crate) fn control_set(&mut self, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        let field = params.get("field").and_then(serde_json::Value::as_str).ok_or("field is required")?;
        let value = params.get("value").ok_or("value is required")?;
        match field {
            "activeLayer" => self.ui.active_layer = value.as_u64().filter(|n| *n < 32).ok_or("activeLayer must be 0 to 31")? as usize,
            "tool" => {
                self.ui.selected_tool = match value.as_str() {
                    Some("Select") => "Select",
                    Some("Pan") => "Pan",
                    Some("Rectangle") => "Rectangle",
                    Some("Ellipse") => "Ellipse",
                    _ => return Err("unknown tool".into()),
                }
            }
            _ => return Err("unknown or human-only UI field".into()),
        }
        Ok(serde_json::json!({"field":field,"value":value}))
    }
    pub(crate) fn control_dialog(&mut self, method: &str, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        if params.get("dialog").and_then(serde_json::Value::as_str) != Some("cutSettings") {
            return Err("unknown or human-only dialog".into());
        }
        match method {
            "ui.dialog.open" => {
                self.ui.cut_settings_open = true;
                self.cut_draft =
                    self.project().and_then(|p| p.color_layer(self.ui.active_layer as u8)).and_then(|l| l.ops.first()).map(crate::cut_editor::Draft::from_op);
            }
            "ui.dialog.cancel" => {
                self.ui.cut_settings_open = false;
                self.cut_draft = None;
            }
            "ui.dialog.set" => {
                let draft = self.cut_draft.as_mut().filter(|_| self.ui.cut_settings_open).ok_or("cut settings dialog has no editable layer")?;
                let value = params.get("value").ok_or("value is required")?;
                match params.get("field").and_then(serde_json::Value::as_str).ok_or("field is required")? {
                    "speedMmS" => draft.speed = value.as_f64().ok_or("speed must be a number")?,
                    "powerPct" => draft.power = value.as_f64().ok_or("power must be a number")?,
                    "frequencyKhz" => draft.frequency = value.as_f64().ok_or("frequency must be a number")?,
                    "qPulseNs" => draft.pulse = value.as_f64().ok_or("pulse must be a number")?,
                    "intervalMm" => draft.interval = value.as_f64().ok_or("interval must be a number")?,
                    "angleDeg" => draft.angle = value.as_f64().ok_or("angle must be a number")?,
                    "passes" => draft.passes = value.as_u64().and_then(|v| u16::try_from(v).ok()).ok_or("passes must be an integer in 0 to 65535")?,
                    "crossHatch" => draft.cross_hatch = value.as_bool().ok_or("crossHatch must be boolean")?,
                    _ => return Err("unknown cut settings field".into()),
                }
            }
            "ui.dialog.confirm" => {
                let draft = self.cut_draft.as_ref().filter(|_| self.ui.cut_settings_open).ok_or("cut settings dialog has no editable layer")?;
                self.session.execute_as(Origin::Synthetic, "layer.cutSettings.set", draft.params(self.ui.active_layer)).map_err(|e| e.to_string())?;
                self.ui.cut_settings_open = false;
                self.cut_draft = None;
            }
            _ => return Err("unknown dialog operation".into()),
        }
        Ok(serde_json::json!({"open":self.ui.cut_settings_open}))
    }

    fn poll_session(&mut self) {
        let events = self.session.poll();
        #[cfg(test)]
        {
            self.polls += 1;
        }
        for event in events {
            if let openlaser_engine::Event::Status(message) = event {
                self.ui.status_message = message;
            }
        }
        if self.close_after_stop && !self.stop_waiting.is_empty() {
            self.stop_waiting.retain(|id| !self.session.status(id).and_then(|status| status.last_stop.as_ref()).is_some_and(|report| report.confirmed));
            if self.stop_waiting.is_empty() {
                self.stop_confirmed = true;
            }
        }
    }

    fn draw_arm_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.session.pending_arm() else { return };
        let Some(machine) = self.session.machines().iter().find(|m| m.id == pending.machine) else {
            self.session.cancel_arm();
            return;
        };
        let model = crate::ArmModel {
            summary: pending.summary.clone(),
            receipt: pending.receipt.clone(),
            checklist: pending.checklist.clone(),
            session: self.session.id(),
            now: self.services.clock.now(),
            thumbnail: openlaser_render::emission_list(&pending.burned, &[], machine),
            emission_cap_pct: None,
        };
        match self.services.human.arm_dialog(ctx, &model) {
            crate::ArmOutcome::Open => {}
            crate::ArmOutcome::Cancelled => self.session.cancel_arm(),
            crate::ArmOutcome::Armed(token) => {
                if let Err(e) = self.session.run_armed(token) {
                    self.ui.status_message = e.to_string();
                }
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            for menu in crate::menu_catalog::MENUS {
                ui.menu_button(*menu, |ui| self.menu_branch(ui, &[*menu]));
            }
        });
    }

    fn menu_branch(&mut self, ui: &mut egui::Ui, path: &[&str]) {
        let mut seen = std::collections::BTreeSet::new();
        for row in crate::menu_catalog::CATALOG.iter().filter(|row| row.path.starts_with(path)) {
            if let Some(next) = row.path.get(path.len()) {
                if seen.insert(*next) {
                    let mut nested = path.to_vec();
                    nested.push(next);
                    ui.menu_button(*next, |ui| self.menu_branch(ui, &nested));
                }
            } else if crate::menu_catalog::is_separator(row) {
                ui.separator();
            } else {
                let live = crate::menu_catalog::is_live(row);
                let response = ui.add_enabled(live, egui::Button::new(row.label));
                if !live {
                    response.clone().on_disabled_hover_text("Not in OpenLaser yet");
                }
                if response.clicked() && !crate::menu_catalog::is_hw_row(row) {
                    self.invoke_menu_command(ui.ctx(), &crate::menu_catalog::native_id(row));
                    ui.close();
                }
            }
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let label = if self.stop_overlay_needed() { "Toolbar stop laser" } else { "Stop laser" };
            let stop = ui.add(egui::Button::new(egui::RichText::new("STOP").strong().color(egui::Color32::WHITE)).fill(egui::Color32::from_rgb(190, 24, 36)));
            stop.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
            if stop.clicked() {
                self.stop();
            }
            egui::ScrollArea::horizontal().id_salt("toolbar-scroll").show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (label, id, icon) in [
                        ("New", "file.new", "file-plus"),
                        ("Open", "file.open", "folder-open"),
                        ("Import", "file.import", "file-input"),
                        ("Save", "file.save", "save"),
                        ("Undo", "edit.undo", "undo-2"),
                        ("Redo", "edit.redo", "redo-2"),
                        ("Cut", "edit.cut", "scissors"),
                        ("Copy", "edit.copy", "copy"),
                        ("Paste", "edit.paste", "clipboard-paste"),
                        ("Pan", "view.pan", "hand"),
                        ("Zoom", "view.zoomPage", "maximize"),
                        ("Settings", "edit.settings", "settings"),
                        ("Device Settings", "device.settings", "settings-2"),
                        ("Preview", "preview.open", "eye"),
                    ] {
                        let disabled = self.ui.machine_active && matches!(id, "file.open" | "file.import" | "file.new");
                        let response = ui.add_enabled(!disabled, egui::Button::image(icon_image(icon))).on_hover_text(label);
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, !disabled, label));
                        if response.clicked() {
                            self.invoke_menu_command(ui.ctx(), id);
                        }
                    }
                    ui.separator();
                    ui.label(self.machine_chip());
                });
            });
        });
    }

    fn stop_overlay_needed(&self) -> bool {
        self.is_machine_busy() || self.arm_dialog_open() || self.ui.cut_settings_open || self.services.human.surface_open()
    }

    fn machine_chip(&self) -> String {
        self.session
            .machines()
            .first()
            .map(|m| format!("{}: {:?}", m.name, self.session.status(&m.id).map(|s| &s.state).unwrap_or(&MachineState::Disconnected)))
            .unwrap_or_else(|| "No device: Disconnected".into())
    }

    fn invoke_ui_action(&mut self, id: &str) {
        if (self.is_machine_busy() || self.arm_dialog_open()) && id.starts_with("file.") && id != "file.exit" {
            self.ui.status_message = "Stop the active job before using file actions".to_owned();
            return;
        }
        match id {
            "edit.settings" => self.ui.settings_open = true,
            "device.settings" | "edit.deviceSettings" => self.ui.device_settings_open = true,
            "laser.liveFraming" => self.ui.live_framing_open = true,
            "file.quit" | "file.exit" => (self.services.quit)(),
            "view.zoomPage" | "view.zoomToFit" => self.canvas_zoom = None,
            "view.pan" => self.ui.selected_tool = "Pan",
            "tools.select" => {
                self.ui.selected_tool = "Select";
                self.execute_widget(id, serde_json::Value::Null);
            }
            "tools.rectangle" => {
                self.ui.selected_tool = "Rectangle";
                self.execute_widget(id, serde_json::Value::Null);
            }
            "tools.ellipse" => {
                self.ui.selected_tool = "Ellipse";
                self.execute_widget(id, serde_json::Value::Null);
            }
            "file.open" | "file.import" => {
                let kind = if id == "file.open" { crate::services::DialogKind::OpenProject } else { crate::services::DialogKind::Import };
                let ticket = self.services.files.open(kind);
                self.pending_dialogs.push((ticket, kind, if self.frame_synthetic { Origin::Synthetic } else { Origin::Human }));
            }
            "file.save" | "file.saveAs" => self.ui.status_message = "Project saving requires the format package".to_owned(),
            "window.preview" | "preview.open" => self.ui.preview_open = true,
            _ => self.execute_widget(id, serde_json::Value::Null),
        }
    }

    fn poll_dialogs(&mut self) {
        if self.ui.machine_active {
            return;
        }
        let mut pending = Vec::new();
        for (ticket, kind, origin) in self.pending_dialogs.drain(..) {
            match self.services.files.poll(ticket) {
                None => pending.push((ticket, kind, origin)),
                Some(None) => self.ui.status_message = "File selection cancelled".to_owned(),
                Some(Some((path, _bytes))) => {
                    let name = std::path::Path::new(&path).file_name().and_then(|n| n.to_str()).unwrap_or("file");
                    let id = if kind == crate::DialogKind::Import { "file.import" } else { "file.open" };
                    self.ui.status_message = self
                        .session
                        .execute_as(origin, id, serde_json::json!({"path":path}))
                        .map(|_| format!("Opened {name}"))
                        .unwrap_or_else(|e| e.to_string());
                }
            }
        }
        self.pending_dialogs = pending;
    }

    fn persist_ui_settings(&mut self) {
        let mut settings = self.services.settings.load();
        settings.show_laser = self.dock.contains(&"laser".into());
        settings.dock_layout = crate::services::normalize_dock_layout(&crate::dock::encode(&self.dock));
        self.services.settings.save(&settings);
    }

    fn project(&self) -> Option<&openlaser_doc::Project> {
        #[cfg(test)]
        if let Some(project) = &self.test_project {
            return Some(project);
        }
        self.session.active().map(|doc| doc.project.as_ref())
    }

    fn execute_widget(&mut self, id: &str, params: serde_json::Value) {
        let origin = if self.frame_synthetic { Origin::Synthetic } else { Origin::Human };
        #[cfg(test)]
        self.dispatched.push((origin, id.to_owned(), params.clone()));
        self.ui.status_message = self.session.execute_as(origin, id, params).map(|_| format!("{id} complete")).unwrap_or_else(|e| e.to_string());
    }

    fn cuts_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Cuts / Layers");
        let used = self.project().map(used_layer_indices).unwrap_or_default();
        let air = self.active_machine_is_gantry();
        egui::Grid::new("cuts-table").striped(true).spacing([6.0, 4.0]).show(ui, |ui| {
            for heading in ["#", "Layer", "Mode", "Spd/Pwr", "Output", "Show"] {
                ui.strong(heading);
            }
            if air {
                ui.strong("Air");
            }
            ui.end_row();
            for idx in used {
                let name =
                    self.project().and_then(|p| p.color_layer(idx as u8)).map(|l| l.name.clone()).unwrap_or_else(|| format!("T{}", idx.saturating_sub(29)));
                let (mode, speed, power, _, mut output, mut show) = self.layer_values(idx);
                let key = if idx < 30 { serde_json::json!(idx) } else { serde_json::json!(format!("T{}", idx - 29)) };
                let response = ui
                    .add_sized([28.0, 24.0], egui::Button::new(if idx < 30 { format!("{idx:02}") } else { format!("T{}", idx - 29) }).fill(palette_color(idx)));
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Cut layer {name}")));
                ui.label(&name);
                ui.label(mode);
                ui.label(format!("{speed} / {power}"));
                let out = ui.add_enabled(idx < 30, egui::Checkbox::without_text(&mut output));
                out.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Checkbox, idx < 30, format!("Output {name}")));
                if out.changed() {
                    self.execute_widget("layer.setOutput", serde_json::json!({"layer": key, "on": output}));
                }
                let visible = ui.checkbox(&mut show, "");
                visible.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Checkbox, true, format!("Show {name}")));
                if visible.changed() {
                    self.execute_widget("layer.setShow", serde_json::json!({"layer": key, "on": show}));
                }
                if air {
                    let mut on = self.project().and_then(|p| p.color_layer(idx as u8)).is_some_and(|l| l.air_assist);
                    ui.add_enabled(false, egui::Checkbox::new(&mut on, format!("Air {name}")));
                }
                ui.end_row();
                if response.clicked() {
                    self.ui.active_layer = idx;
                    self.execute_widget("layer.select", serde_json::json!({"layer": key}));
                    let now = ui.input(|input| input.time);
                    if self.last_layer_click.is_some_and(|(previous, at)| previous == idx && now - at <= 0.4) {
                        self.ui.cut_settings_open = true;
                        self.last_layer_click = None;
                    } else {
                        self.last_layer_click = Some((idx, now));
                    }
                }
            }
        });
        ui.separator();
        let (mode, speed, power, frequency, _, _) = self.layer_values(self.ui.active_layer);
        ui.label("Quick edit");
        for (label, value) in [("Mode", mode.to_owned()), ("Speed", speed), ("Power", power), ("Frequency", frequency)] {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.label(value);
            });
        }
        if ui.button("Edit Cut Settings…").clicked() {
            self.ui.cut_settings_open = true;
        }
    }

    fn layer_values(&self, idx: usize) -> (&'static str, String, String, String, bool, bool) {
        let Some(layer) = (idx < 30).then(|| self.project().and_then(|project| project.color_layer(idx as u8))).flatten() else {
            return ("Tool", "—".to_owned(), "—".to_owned(), "—".to_owned(), false, true);
        };
        let Some(op) = layer.ops.first() else {
            return ("—", "—".to_owned(), "—".to_owned(), "—".to_owned(), layer.output, layer.show);
        };
        let mode = match op.mode {
            openlaser_doc::CutMode::Line => "Line",
            openlaser_doc::CutMode::Fill => "Fill",
            openlaser_doc::CutMode::OffsetFill => "Offset Fill",
        };
        (
            mode,
            format!("{} mm/s", op.galvo.speed.get()),
            format!("{} %", op.galvo.power.get()),
            format!("{} kHz", f64::from(op.galvo.frequency.get()) / 1000.0),
            layer.output,
            layer.show,
        )
    }

    pub(crate) fn control_dock(&mut self, method: &str, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        use craft_ui::docking::{Action, Placement, Zone};
        let panel = params.get("panel").and_then(serde_json::Value::as_str).ok_or("panel is required")?.to_owned();
        if !crate::dock::PANELS.contains(&panel.as_str()) {
            return Err("unknown panel".into());
        }
        let action = match method {
            "ui.dock.close" => Action::Close { panel },
            "ui.dock.open" => Action::Open { panel, anchor: Some("cuts".into()) },
            "ui.dock.float" => Action::Float { panel, rect: [80.0, 80.0, 320.0, 420.0] },
            "ui.dock.resize" => {
                let rect = params.get("rect").and_then(serde_json::Value::as_array).filter(|a| a.len() == 4).ok_or("rect must contain x,y,width,height")?;
                let mut values = [0.0; 4];
                for (out, value) in values.iter_mut().zip(rect) {
                    *out = value.as_f64().ok_or("rect must contain numbers")? as f32;
                }
                Action::MoveFloating { panel, rect: values }
            }
            _ => Action::Move {
                panel,
                anchor: params.get("anchor").and_then(serde_json::Value::as_str).ok_or("anchor is required")?.into(),
                placement: Placement::Split(match params.get("zone").and_then(serde_json::Value::as_str).unwrap_or("center") {
                    "left" => Zone::Left,
                    "right" => Zone::Right,
                    "top" => Zone::Top,
                    "bottom" => Zone::Bottom,
                    "center" => Zone::Center,
                    _ => return Err("unknown dock zone".into()),
                }),
            },
        };
        self.dock.apply_with_permissions(action, crate::dock::permissions).map_err(|e| e.to_string())?;
        Ok(serde_json::json!({"layout": self.dock}))
    }

    fn paint_canvas(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("0 mm");
            ui.separator();
            ui.label("100 mm");
        });
        if ui.button("Zoom to fit").clicked() {
            self.canvas_zoom = None;
            self.canvas_center = [0.0; 2];
        }
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let zoom = self.canvas_zoom.get_or_insert(f64::from(rect.width().min(rect.height())) / 240.0);
        if response.hovered() {
            let factor = ui.input(|i| f64::from(i.zoom_delta()) * (f64::from(i.smooth_scroll_delta.y) * 0.002).exp());
            let next = *zoom * factor;
            if next.is_finite() && (0.05..=1000.0).contains(&next) {
                *zoom = next;
            }
        }
        if response.dragged_by(egui::PointerButton::Middle) || (response.dragged() && self.ui.selected_tool == "Pan") {
            let delta = ui.input(|i| i.pointer.delta());
            self.canvas_center[0] -= f64::from(delta.x) / *zoom;
            self.canvas_center[1] += f64::from(delta.y) / *zoom;
        }
        let view = crate::canvas::ViewXform { zoom: *zoom, center: self.canvas_center, rect: rect.shrink(20.0) };
        self.ui.pointer_mm = response.hover_pos().map(|position| view.screen_to_world(position));
        ui.painter().rect_filled(rect, 0.0, egui::Color32::from_gray(38));
        ui.painter().rect_stroke(rect.shrink(16.0), 0.0, egui::Stroke::new(1.0, egui::Color32::from_gray(88)), egui::StrokeKind::Inside);
        ui.painter().line_segment([rect.center() + egui::vec2(-8.0, 0.0), rect.center() + egui::vec2(8.0, 0.0)], egui::Stroke::new(1.0, egui::Color32::GRAY));
        ui.painter().line_segment([rect.center() + egui::vec2(0.0, -8.0), rect.center() + egui::vec2(0.0, 8.0)], egui::Stroke::new(1.0, egui::Color32::GRAY));
        let painter = ui.painter().with_clip_rect(rect);
        let low = view.screen_to_world(rect.left_bottom());
        let high = view.screen_to_world(rect.right_top());
        let base = 10.0_f64.powf((40.0 / view.zoom).log10().floor());
        let ratio = 40.0 / (view.zoom * base);
        let step = base
            * if ratio <= 1.0 {
                1.0
            } else if ratio <= 2.0 {
                2.0
            } else if ratio <= 5.0 {
                5.0
            } else {
                10.0
            };
        let precision = if step < 1.0 { (-step.log10()).ceil().min(6.0) as usize } else { 0 };
        for (axis, (low, high)) in low.into_iter().zip(high).enumerate() {
            let start = (low / step).ceil() as i32;
            let end = (high / step).floor() as i32;
            for n in start..=end {
                let value = f64::from(n) * step;
                let (a, b, label_at) = if axis == 0 {
                    let x = view.world_to_screen([value, 0.0]).x;
                    (egui::pos2(x, rect.top() + 18.0), egui::pos2(x, rect.bottom()), egui::pos2(x + 2.0, rect.top()))
                } else {
                    let y = view.world_to_screen([0.0, value]).y;
                    (egui::pos2(rect.left() + 18.0, y), egui::pos2(rect.right(), y), egui::pos2(rect.left(), y))
                };
                painter.line_segment([a, b], egui::Stroke::new(0.5, egui::Color32::from_gray(65)));
                painter.text(label_at, egui::Align2::LEFT_TOP, format!("{value:.precision$}"), egui::FontId::monospace(10.0), egui::Color32::GRAY);
            }
        }
        let machine = self.session.machine_for(None).or_else(|| self.session.machines().first());
        if let Some(openlaser_machine::MachineProfile { kind: openlaser_machine::Kinematics::Galvo(galvo), .. }) = machine {
            let half = galvo.encoding_field.get() / 2.0;
            for (min, max, color) in
                [([-half, -half], [half, half], egui::Color32::DARK_GREEN), (galvo.work_area.min, galvo.work_area.max, egui::Color32::LIGHT_GRAY)]
            {
                painter.rect_stroke(
                    egui::Rect::from_two_pos(view.world_to_screen(min), view.world_to_screen(max)),
                    0.0,
                    egui::Stroke::new(1.0, color),
                    egui::StrokeKind::Inside,
                );
            }
        }
        if let Some(doc) = self.session.active() {
            let machine = self.session.machines().iter().find(|machine| machine.id == doc.machine);
            let list = openlaser_render::document_list(
                &doc.project,
                machine,
                &openlaser_render::ViewOpts {
                    dark: true,
                    selection: doc.selection.ids.clone(),
                    refusals: Vec::new(),
                    notices: Vec::new(),
                    show_field: true,
                    show_work_area: true,
                },
            );
            self.renderer.paint(&ui.painter().with_clip_rect(rect), &list, &view);
        }
        if self.ui.selected_tool == "Select" {
            if response.clicked()
                && let Some(position) = response.interact_pointer_pos()
            {
                let world = view.screen_to_world(position);
                let mut selected = None;
                if let Some(doc) = self.session.active() {
                    for shape in doc.project.shapes.iter().rev() {
                        let paths = doc.project.world_polylines(shape.id, 0.02).into_iter().map(|line| line.0).collect::<Vec<_>>();
                        if crate::canvas::hit_test(&paths, world, 6.0 / view.zoom).is_some() {
                            selected = Some(shape.id);
                            break;
                        }
                    }
                }
                self.execute_widget(
                    "edit.select",
                    serde_json::json!({"ids":selected.into_iter().collect::<Vec<_>>(),"mode":if ui.input(|i|i.modifiers.shift) {"toggle"}else{"replace"}}),
                );
            }
            if response.drag_stopped()
                && let Some(position) = response.interact_pointer_pos()
            {
                let start = position - response.drag_delta();
                let a = view.screen_to_world(start);
                let b = view.screen_to_world(position);
                self.execute_widget(
                    "edit.select",
                    serde_json::json!({"rectMm":[a,b],"crossing":position.x<start.x,"mode":if ui.input(|i|i.modifiers.shift) {"add"}else{"replace"}}),
                );
            }
        }
    }

    fn active_machine_is_gantry(&self) -> bool {
        self.session.active().is_some_and(|doc| doc.project.workspace.kind == openlaser_doc::DeviceKind::Grbl)
    }

    fn laser_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Laser");
        let key = self.session.active_index().zip(self.session.active().map(|doc| doc.revision));
        if self.plan_cache.as_ref().is_none_or(|(cached, _)| *cached != key) {
            self.plan_cache = Some((key, self.session.build_job(None, &Default::default())));
        }
        let can_plan = self.plan_cache.as_ref().is_some_and(|(_, plan)| plan.is_ok());
        let can_start = self.plan_cache.as_ref().and_then(|(_, plan)| plan.as_ref().ok()).is_some_and(|job| {
            self.session.status(&job.summary.machine).and_then(|status| status.receipt.as_ref()).is_some_and(|receipt| {
                receipt.fingerprint == job.summary.fingerprint
                    && receipt.machine == job.summary.machine
                    && receipt.session == self.session.id()
                    && receipt.full_passes > 0
                    && receipt.covers_job()
                    && receipt.origin == openlaser_types::FrameOrigin::Human
                    && self.services.clock.now().saturating_since(receipt.at) <= openlaser_types::FRAME_RECEIPT_MAX_AGE
            })
        });
        #[cfg(test)]
        let can_plan = can_plan || self.frame_ready;
        let machines = self.session.machines().iter().map(|m| (m.id.clone(), m.name.clone())).collect::<Vec<_>>();
        let mut device = self.session.active().map(|doc| doc.machine.clone()).or_else(|| machines.first().map(|(id, _)| id.clone()));
        let previous = device.clone();
        ui.add_enabled_ui(!self.is_machine_busy(), |ui| {
            egui::ComboBox::from_label("Device")
                .selected_text(machines.iter().find(|(id, _)| Some(id) == device.as_ref()).map(|(_, name)| name.as_str()).unwrap_or("No device"))
                .show_ui(ui, |ui| {
                    for (id, name) in &machines {
                        ui.selectable_value(&mut device, Some(id.clone()), name);
                    }
                });
        });
        if device != previous
            && let Some(id) = device
        {
            self.execute_widget("device.select", serde_json::json!({"device":id}));
        }
        ui.label(self.machine_chip());
        ui.horizontal(|ui| {
            for name in ["Connect", "Frame", "Start"] {
                let enabled = match name {
                    "Connect" => true,
                    "Frame" => can_plan && !self.ui.machine_active,
                    "Start" => can_start && !self.ui.machine_active,
                    _ => false,
                };
                let r = ui.add_enabled(enabled, egui::Button::new(name));
                if name == "Frame" && !can_plan {
                    r.clone().on_disabled_hover_text("Frame is unavailable until the current job can be planned.");
                }
                if name == "Start" {
                    r.clone().on_disabled_hover_text("Frame this job first. A human must approve every marking job.");
                }
                if !self.services.human.frame_is_synthetic(ctx) && self.services.human.is_human_click(ctx, &r) {
                    #[cfg(test)]
                    {
                        self.human_actions += 1;
                    }
                    let result = match name {
                        "Connect" => self
                            .session
                            .machines()
                            .first()
                            .map(|m| m.id.clone())
                            .ok_or(openlaser_engine::EngineError::NoDevice)
                            .and_then(|id| self.session.connect(Origin::Human, &id)),
                        "Frame" => self.session.start_frame(Origin::Human, openlaser_types::FrameMode::Once, None, None).map(|_| ()),
                        _ => self.session.request_arm(Origin::Human).map(|_| ()),
                    };
                    self.ui.status_message = result.map(|_| format!("{name} requested")).unwrap_or_else(|e| e.to_string());
                } else if r.clicked() {
                    self.ui.status_message = format!("{name} requires operator input");
                }
            }
        });
        let stop = ui.button("Stop");
        if stop.clicked() {
            self.stop();
        }
        ui.add_enabled(false, egui::Button::new("Start From: Absolute · galvo uses field centre"))
            .on_disabled_hover_text("Start From is unavailable for galvo machines; the field centre is the origin.");
        for (name, reason) in [
            ("Pause", "Pause is unavailable for galvo machines; stop and frame again."),
            ("Send", "Send is unavailable for galvo machines; jobs require local approval."),
            ("Home", "Home is unavailable for galvo machines; the field centre is fixed."),
            ("Go to Origin", "Go to Origin is unavailable for galvo machines; use framing."),
        ] {
            ui.add_enabled(false, egui::Button::new(name)).on_disabled_hover_text(reason);
        }
        ui.label("Repeat");
        let mut repeats = self.session.active().map(|doc| doc.project.job.repeats).unwrap_or(1);
        let repeat_response = ui.add_enabled(self.session.active().is_some(), egui::DragValue::new(&mut repeats)).labelled_by(ui.label("Repeat count").id);
        if repeat_response.changed() {
            self.execute_widget("laser.repeat", serde_json::json!({"count":repeats}));
        }
        let mut cut_selected = self.session.active().is_some_and(|doc| doc.project.job.cut_selected);
        ui.add_enabled_ui(self.session.active().is_some(), |ui| {
            if ui.checkbox(&mut cut_selected, "Cut Selected Graphics").changed() {
                self.execute_widget("laser.cutSelected", serde_json::json!({"on":cut_selected}));
            }
        });
        match self.plan_cache.as_ref().map(|(_, plan)| plan) {
            None => {}
            Some(Ok(job)) => {
                ui.label(format!("Estimate: {:.1} s", job.summary.estimate.as_secs_f64()));
                ui.label("Frame this job first");
            }
            Some(Err(refused)) if refused.refusals.is_empty() => {
                ui.colored_label(egui::Color32::from_rgb(234, 107, 71), "Job unavailable");
            }
            Some(Err(refused)) => {
                ui.colored_label(egui::Color32::from_rgb(234, 107, 71), format!("{} refusal(s)", refused.refusals.len()));
                for refusal in refused.refusals.iter().take(3) {
                    ui.label(refusal.message.as_str());
                }
            }
        }
        if self.session.active().is_none() {
            ui.label("No current job · Frame this job first");
        }
        let mut requests = self
            .session
            .requests()
            .submissions()
            .into_iter()
            .filter(|r| matches!(r.state, openlaser_engine::SubmissionState::AwaitingHuman))
            .map(|r| (r.submission, format!("Job from {}", r.client), format!("{} · {:.1} s", r.note, r.summary.estimate.as_secs_f64())))
            .collect::<Vec<_>>();
        requests.extend(
            self.session
                .requests()
                .frame_requests()
                .into_iter()
                .filter(|r| matches!(r.state, openlaser_engine::FrameRequestState::AwaitingHuman))
                .map(|r| (r.request, format!("Frame request from {}", r.client), format!("{:?}", r.mode))),
        );
        for (id, title, detail) in requests {
            self.request_card(ui, ctx, id, &title, &detail);
        }
    }

    fn request_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, id: openlaser_engine::RequestId, title: &str, detail: &str) {
        ui.group(|ui| {
            ui.label(title);
            ui.label(detail);
            ui.horizontal(|ui| {
                for (label, approve) in [("Approve framing", true), ("Reject request", false)] {
                    let response = ui.add_enabled(!self.ui.machine_active, egui::Button::new(label));
                    if !self.services.human.frame_is_synthetic(ctx) && self.services.human.is_human_click(ctx, &response) {
                        let result = if approve { self.session.approve_request(Origin::Human, id) } else { self.session.decline_request(Origin::Human, id) };
                        self.ui.status_message = result.map(|_| format!("{label} complete")).unwrap_or_else(|e| e.to_string());
                    }
                }
            });
        });
    }
}

impl eframe::App for OpenLaserApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })) {
            if self.is_machine_busy() || self.arm_dialog_open() {
                self.stop();
            } else {
                self.ui.selected_tool = "Select";
                let _ = self.session.execute_as(
                    if self.services.human.frame_is_synthetic(ctx) { Origin::Synthetic } else { Origin::Human },
                    "edit.selectNone",
                    serde_json::Value::Null,
                );
            }
            raw.events.retain(|e| !matches!(e, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. }));
        }
        if self.services.human.surface_open() {
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

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let text_focus = ctx.egui_wants_keyboard_input();
        let shortcut_ids = ctx.input_mut(|input| crate::shortcuts::take_pressed(input, &crate::shortcuts::table(), text_focus));
        for id in shortcut_ids {
            self.invoke_menu_command(ctx, id);
        }
        self.poll_session();
        self.ui.machine_active = self.is_machine_busy() || self.arm_dialog_open();
        self.poll_dialogs();
        self.guard_close(ctx);
        if self.ui.machine_active {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if let Some(rx) = self.services.automation.take_requests() {
            self.control_rx = Some(rx);
        }
        let requests: Vec<ControlRequest> = self.control_rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        for request in requests {
            let reply_tx = request.reply.clone();
            if request.deadline <= std::time::Instant::now() || !request.dispatch.try_dispatch() {
                let reply = crate::control::error_reply(request.request.id.clone(), "control request expired or was already settled");
                let _ = reply_tx.send(reply);
                continue;
            }
            let outcome = crate::control::handle(self, ctx, request);
            let reply = match outcome {
                crate::control::Outcome::Done(reply) | crate::control::Outcome::AfterInput(reply) => reply,
                crate::control::Outcome::Screenshot { id, max_side, reply } => {
                    self.screenshots.push((id, max_side, reply, std::time::Instant::now()));
                    continue;
                }
            };
            let _ = reply_tx.send(reply);
        }
        let _ = &self.services.quit;
        let images = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(std::sync::Arc::clone(image)) } else { None })
                .collect::<Vec<_>>()
        });
        if let Some(image) = images.last() {
            for (id, side, reply, _) in self.screenshots.drain(..) {
                let result = crate::screenshot::encode(image, side);
                let _ = reply.send(match result {
                    Ok(result) => openlaser_control::wire::Reply { id, ok: true, result: Some(result), error: None },
                    Err(error) => crate::control::error_reply(id, &error),
                });
            }
        }
        self.screenshots.retain(|(id, _, reply, started)| {
            if started.elapsed() > std::time::Duration::from_secs(5) {
                let _ = reply.send(crate::control::error_reply(id.clone(), "screenshot timed out"));
                false
            } else {
                true
            }
        });
        if !self.screenshots.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.persist_ui_settings();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame_synthetic = self.services.human.frame_is_synthetic(ui.ctx());
        if show_in_window_menus(self.services.settings.load().in_window_menus, cfg!(target_os = "macos")) {
            egui::Panel::top("menu_bar").show(ui, |ui| self.menu_bar(ui));
        }
        egui::Panel::top("main_toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::top("numeric_edits").show(ui, |ui| {
            let bounds = self.session.active().and_then(|doc| doc.project.bounds(&doc.selection.ids));
            let values = bounds.map(|b| [b.min[0], b.min[1], b.max[0] - b.min[0], b.max[1] - b.min[1], 100.0, 100.0, 0.0]).unwrap_or([0.0; 7]);
            ui.horizontal_wrapped(|ui| {
                for ((label, key), mut value) in
                    [("XPos", "xMm"), ("YPos", "yMm"), ("Width", "wMm"), ("Height", "hMm"), ("W%", "wMm"), ("H%", "hMm"), ("Rotate °", "rotateDeg")]
                        .into_iter()
                        .zip(values)
                {
                    let name = ui.label(label);
                    let response = ui.add_enabled(bounds.is_some(), egui::DragValue::new(&mut value).speed(0.1)).labelled_by(name.id);
                    if response.changed() {
                        let value = if label == "W%" {
                            values[2] * value / 100.0
                        } else if label == "H%" {
                            values[3] * value / 100.0
                        } else {
                            value
                        };
                        let mut params = serde_json::json!({"anchor":self.ui.anchor,"lockRatio":self.ui.lock_ratio,"coalesce":format!("numeric-{key}")});
                        if let Some(map) = params.as_object_mut() {
                            map.insert(key.into(), serde_json::json!(value));
                        }
                        self.execute_widget("arrange.transform", params);
                    }
                }
                ui.checkbox(&mut self.ui.lock_ratio, "Lock aspect");
                egui::Grid::new("anchor-grid").spacing([1.0, 1.0]).show(ui, |ui| {
                    for n in 0..9 {
                        let response = ui.add(egui::Button::new("·").small().selected(self.ui.anchor == n));
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Anchor {}", n + 1)));
                        if response.clicked() {
                            self.ui.anchor = n;
                        }
                        if n % 3 == 2 {
                            ui.end_row();
                        }
                    }
                });
            });
        });
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(self.ui.pointer_mm.map(|[x, y]| format!("x {x:.2}   y {y:.2} mm")).unwrap_or_else(|| "Cursor outside design".into()));
                ui.separator();
                ui.label(format!("{} shapes", self.session.active().map(|doc| doc.project.shapes.len()).unwrap_or_default()));
                ui.separator();
                ui.label(self.machine_chip());
                ui.separator();
                ui.label(&self.ui.status_message);
                if let Some(info) = self.services.automation.info() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(format!("Agent connected · {} client(s)", info.clients.len()));
                    });
                }
            });
        });
        egui::Panel::bottom("palette").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                ui.spacing_mut().button_padding.x = 1.0;
                let swatch_width = ((ui.available_width() - 31.0) / 32.0).floor();
                for idx in 0..32 {
                    let label = if idx < 30 { format!("{idx:02}") } else { format!("T{}", idx - 29) };
                    let color = palette_color(idx);
                    let button = egui::Button::new(egui::RichText::new(label).color(if idx < 10 { egui::Color32::WHITE } else { egui::Color32::BLACK }))
                        .fill(color)
                        .min_size(egui::vec2(swatch_width, 24.0));
                    let layer_label = if idx < 30 { format!("Layer {idx:02}") } else { format!("T{}", idx - 29) };
                    let response = ui.add_sized([swatch_width, 24.0], button).on_hover_text(&layer_label);
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &layer_label));
                    if response.clicked() {
                        self.ui.active_layer = idx;
                        let command = if self.session.active().is_some_and(|doc| !doc.selection.ids.is_empty()) { "layer.assign" } else { "layer.setCurrent" };
                        self.execute_widget(
                            command,
                            serde_json::json!({"layer": if idx < 30 { serde_json::json!(idx) } else { serde_json::json!(format!("T{}",idx-29)) }}),
                        );
                    }
                }
            });
        });
        egui::Panel::left("tools").default_size(42.0).show(ui, |ui| {
            for (tool, icon) in [
                ("Select", "mouse-pointer-2"),
                ("Line", "spline"),
                ("Rectangle", "rectangle-horizontal"),
                ("Ellipse", "circle"),
                ("Polygon", "pentagon"),
                ("Pen", "pen-tool"),
                ("Text", "type"),
                ("Edit Nodes", "spline"),
                ("Offset", "layers"),
                ("Weld", "link-2"),
                ("Measure", "ruler"),
                ("Pan", "hand"),
            ] {
                let response = ui.add(egui::Button::image(icon_image(icon)).selected(self.ui.selected_tool == tool)).on_hover_text(tool);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tool));
                if response.clicked() {
                    self.ui.selected_tool = tool;
                    self.execute_widget(
                        match tool {
                            "Select" => "tools.select",
                            "Line" => "tools.line",
                            "Rectangle" => "tools.rectangle",
                            "Ellipse" => "tools.ellipse",
                            "Polygon" => "tools.polygon",
                            "Pen" => "tools.pen",
                            "Text" => "tools.text",
                            "Edit Nodes" => "tools.editNodes",
                            "Offset" => "tools.offset",
                            "Weld" => "tools.weld",
                            "Measure" => "tools.measure",
                            _ => "view.pan",
                        },
                        serde_json::Value::Null,
                    );
                }
            }
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let snapshot = self.dock.clone();
            let style = craft_ui::docking::DockStyle::from_ui(ui);
            let output = craft_ui::docking::DockArea::new(egui::Id::new("editor-dock")).show(
                ui,
                &snapshot,
                &style,
                |panel| crate::dock::label(panel),
                crate::dock::permissions,
                |ui, panel| match panel.as_str() {
                    "canvas" => self.paint_canvas(ui),
                    "cuts" => {
                        egui::ScrollArea::vertical().show(ui, |ui| self.cuts_panel(ui));
                    }
                    "laser" => {
                        let ctx = ui.ctx().clone();
                        self.laser_panel(ui, &ctx);
                    }
                    "console" => {
                        ui.heading("Console");
                        for machine in self.session.machines() {
                            if let Some(status) = self.session.status(&machine.id) {
                                for line in &status.trace_tail {
                                    ui.monospace(format!("{line:?}"));
                                }
                            }
                        }
                        ui.label(&self.ui.status_message);
                    }
                    _ => {
                        ui.heading(crate::dock::label(panel));
                    }
                },
            );
            for action in output.actions {
                if let Err(error) = self.dock.apply_with_permissions(action, crate::dock::permissions) {
                    self.ui.status_message = error.to_string();
                }
            }
        });
        if self.ui.cut_settings_open {
            if self.cut_draft.is_none() {
                self.cut_draft =
                    self.project().and_then(|p| p.color_layer(self.ui.active_layer as u8)).and_then(|l| l.ops.first()).map(crate::cut_editor::Draft::from_op);
            }
            let mut apply = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("cut-settings-modal")).show(ui.ctx(), |ui| {
                ui.heading("Cut Settings Editor");
                if let Some(draft) = &mut self.cut_draft {
                    draft.fields(ui);
                } else {
                    ui.label("Select a color layer in use to edit its settings.");
                }
                let button = ui.add_enabled(self.cut_draft.is_some(), egui::Button::new("OK"));
                button.clone().on_disabled_hover_text("Select a color layer in use first.");
                apply = button.clicked();
                cancel = ui.button("Cancel").clicked();
                ui.label(&self.ui.status_message);
            });
            if apply && let Some(draft) = &self.cut_draft {
                let params = draft.params(self.ui.active_layer);
                let origin = if self.frame_synthetic { Origin::Synthetic } else { Origin::Human };
                match self.session.execute_as(origin, "layer.cutSettings.set", params) {
                    Ok(_) => {
                        self.ui.cut_settings_open = false;
                        self.cut_draft = None;
                    }
                    Err(e) => self.ui.status_message = e.to_string(),
                }
            }
            if cancel {
                self.ui.cut_settings_open = false;
                self.cut_draft = None;
            }
        } else {
            self.cut_draft = None;
        }
        if self.ui.settings_open {
            let mut open = true;
            egui::Window::new("Settings").open(&mut open).show(ui.ctx(), |ui| {
                ui.heading("Automation");
                let mut on = self.services.automation.info().is_some();
                let response = ui.checkbox(&mut on, "Allow agent connections");
                if self.services.human.is_human_click(ui.ctx(), &response)
                    && !self.frame_synthetic
                    && let Err(e) = self.services.automation.set(on)
                {
                    self.ui.status_message = e;
                }
                ui.label(&self.ui.status_message);
            });
            self.ui.settings_open = open;
        }
        if self.ui.device_settings_open {
            if self.device_draft.is_none() {
                self.device_draft = self.session.machine_for(None).or_else(|| self.session.machines().first()).cloned();
            }
            let mut open = true;
            let mut save_requested = false;
            egui::Window::new("Device Settings").open(&mut open).show(ui.ctx(), |ui| {
                if let Some(profile) = &mut self.device_draft {
                    ui.label(&profile.name);
                    if let openlaser_machine::Kinematics::Galvo(g) = &mut profile.kind {
                        for (label, values) in [
                            ("Work area min", &mut g.work_area.min),
                            ("Work area max", &mut g.work_area.max),
                            ("Field offset mm", &mut g.cal.offset_mm),
                            ("Axis scale", &mut g.cal.scale),
                            ("Red dot offset mm", &mut g.red_dot.offset_mm),
                            ("Red dot scale", &mut g.red_dot.scale),
                        ] {
                            for (axis, value) in values.iter_mut().enumerate() {
                                ui.horizontal(|ui| {
                                    let label = ui.label(format!("{label} {}", if axis == 0 { "X" } else { "Y" }));
                                    ui.add(egui::DragValue::new(value).speed(0.01)).labelled_by(label.id);
                                });
                            }
                        }
                        ui.horizontal(|ui| {
                            let label = ui.label("Field angle degrees");
                            ui.add(egui::DragValue::new(&mut g.cal.angle_deg)).labelled_by(label.id);
                        });
                        ui.checkbox(&mut g.cal.swap_xy, "Swap XY");
                        for (axis, negate) in g.cal.negate.iter_mut().enumerate() {
                            ui.checkbox(negate, format!("Negate {}", if axis == 0 { "X" } else { "Y" }));
                        }
                        ui.label(format!("Encoding field {} mm · pulse table {:?} ns", g.encoding_field.get(), g.source.pulse_table_ns));
                    }
                }
                let save = ui.add_enabled(self.device_draft.is_some(), egui::Button::new("Save device settings"));
                save_requested = !self.frame_synthetic && self.services.human.is_human_click(ui.ctx(), &save);
                ui.label(&self.ui.status_message);
            });
            if save_requested && let Some(profile) = &self.device_draft {
                match openlaser_machine::validate_profile(profile) {
                    Err(errors) => self.ui.status_message = errors.join("; "),
                    Ok(()) => {
                        if let openlaser_machine::Kinematics::Galvo(g) = &profile.kind {
                            let params = serde_json::json!({"device":profile.id,"workArea":g.work_area,"cal":g.cal,"redDot":g.red_dot});
                            match self.session.execute_as(Origin::Human, "device.settings.set", params) {
                                Ok(_) => open = false,
                                Err(e) => self.ui.status_message = e.to_string(),
                            }
                        }
                    }
                }
            }
            self.ui.device_settings_open = open;
            if !open {
                self.device_draft = None;
            }
        }
        if self.ui.live_framing_open {
            let mut open = true;
            egui::Window::new("Live Framing").open(&mut open).show(ui.ctx(), |ui| {
                ui.label("Red light only");
                let style = self.ui.live_style.get_or_insert(openlaser_types::FrameStyle::Contour);
                ui.horizontal(|ui| {
                    ui.radio_value(style, openlaser_types::FrameStyle::Bounds, "Bounds");
                    ui.radio_value(style, openlaser_types::FrameStyle::Contour, "Contour");
                });
                let source = self.ui.live_source.get_or_insert(openlaser_types::FrameSource::Job);
                ui.horizontal(|ui| {
                    ui.radio_value(source, openlaser_types::FrameSource::Job, "Job");
                    ui.radio_value(source, openlaser_types::FrameSource::ToolLayersOnly, "Tool Layers Only");
                    ui.radio_value(source, openlaser_types::FrameSource::JobAndToolLayers, "Both");
                });
                let start = ui.button("Start live framing");
                if !self.frame_synthetic && self.services.human.is_human_click(ui.ctx(), &start) {
                    self.ui.status_message = self
                        .session
                        .start_frame(Origin::Human, openlaser_types::FrameMode::Loop, self.ui.live_style, self.ui.live_source)
                        .map(|_| "Framing".into())
                        .unwrap_or_else(|e| e.to_string());
                }
                if ui.button("Stop framing").clicked() {
                    self.stop();
                }
            });
            self.ui.live_framing_open = open;
        }
        if self.ui.preview_open {
            egui::Window::new("Preview").open(&mut self.ui.preview_open).show(ui.ctx(), |ui| {
                ui.heading("Preview");
                match self.plan_cache.as_ref().map(|(_, plan)| plan) {
                    Some(Ok(job)) => {
                        ui.label(format!("Estimate {:.1} s · {} packet(s)", job.summary.estimate.as_secs_f64(), job.summary.packets));
                        for row in &job.summary.rows {
                            ui.label(format!(
                                "{} · pass {}/{} · {:.2} mm/s (actual) · {:.1}%",
                                row.layer_name, row.pass, row.of, row.speed_mm_s, row.power_pct
                            ));
                        }
                    }
                    Some(Err(refused)) => {
                        for reason in &refused.refusals {
                            ui.label(&reason.message);
                        }
                    }
                    None => {
                        ui.label("Open a project to preview its plan.");
                    }
                }
            });
        }
        self.draw_arm_dialog(ui.ctx());
        self.ui.machine_active = self.is_machine_busy() || self.arm_dialog_open();
        if self.stop_overlay_needed() {
            egui::Area::new(egui::Id::new("stop_overlay")).order(egui::Order::Tooltip).anchor(egui::Align2::RIGHT_TOP, egui::vec2(-18.0, 62.0)).show(
                ui.ctx(),
                |ui| {
                    if ui.add(egui::Button::new(egui::RichText::new("Stop laser").strong().color(egui::Color32::WHITE)).fill(egui::Color32::RED)).clicked() {
                        self.stop();
                    }
                },
            );
        }
    }
}

fn icon_image(name: &str) -> egui::Image<'static> {
    static ICONS: std::sync::OnceLock<craft_ui::icons::SvgIconSet> = std::sync::OnceLock::new();
    let icons = ICONS.get_or_init(|| {
        craft_ui::icons::SvgIconSet::new(
            crate::icon_data::ICONS,
            craft_ui::icons::SvgStyle { current_color: "#ffffff", stroke_width: None },
            "bytes://openlaser/icons/",
            "settings",
        )
    });
    egui::Image::new(icons.source(name)).fit_to_exact_size(egui::vec2(18.0, 18.0))
}

fn used_layer_indices(project: &openlaser_doc::Project) -> Vec<usize> {
    fn collect(shapes: &[std::sync::Arc<openlaser_doc::Shape>], used: &mut std::collections::BTreeSet<usize>) {
        for shape in shapes {
            if let openlaser_doc::Geom::Group(children) = &shape.geom {
                collect(children, used);
            } else {
                used.insert(match shape.layer {
                    openlaser_types::LayerKey::Color(n) => usize::from(n),
                    openlaser_types::LayerKey::Tool(n) => usize::from(n) + 29,
                });
            }
        }
    }
    let mut used = std::collections::BTreeSet::new();
    collect(&project.shapes, &mut used);
    used.into_iter().collect()
}

fn state_requires_stop(state: &MachineState) -> bool {
    matches!(state, MachineState::Framing | MachineState::Running { .. } | MachineState::Stopping | MachineState::StopNotConfirmed)
}

fn show_in_window_menus(requested: bool, macos: bool) -> bool {
    requested || !macos
}

fn palette_color(idx: usize) -> egui::Color32 {
    let rgb = if idx < 30 { openlaser_doc::PALETTE.get(idx) } else { openlaser_doc::TOOL_COLORS.get(idx.saturating_sub(30)) };
    rgb.map(|rgb| egui::Color32::from_rgb(rgb.0[0], rgb.0[1], rgb.0[2])).unwrap_or(egui::Color32::GRAY)
}

#[cfg(test)]
mod kittests {
    use super::*;
    use crate::services::{ControlInfo, DialogKind, DialogTicket, FileDialogs, SettingsStore, UiSettings};
    use eframe::App;
    use egui_kittest::kittest::{NodeT, Queryable};
    use openlaser_engine::{DeviceAccess, FsAuthority, Origin, SessionConfig};
    use openlaser_types::{Clock, SystemClock};
    use std::sync::Arc;

    struct Files;
    impl FileDialogs for Files {
        fn open(&mut self, _kind: DialogKind) -> DialogTicket {
            DialogTicket(1)
        }
        fn poll(&mut self, _ticket: DialogTicket) -> Option<Option<(String, Vec<u8>)>> {
            None
        }
        fn save(&mut self, _kind: DialogKind, _bytes: Vec<u8>) -> DialogTicket {
            DialogTicket(2)
        }
    }
    struct Settings(UiSettings);
    impl SettingsStore for Settings {
        fn load(&self) -> UiSettings {
            self.0.clone()
        }
        fn save(&mut self, value: &UiSettings) {
            self.0 = value.clone();
        }
    }
    struct Clipboard;
    impl crate::Clipboard for Clipboard {
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

    fn harness_with_size(size: egui::Vec2) -> egui_kittest::Harness<'static, OpenLaserApp> {
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
            human: Box::new(crate::NoHuman),
            files: Box::new(Files),
            settings: Box::new(Settings(UiSettings { in_window_menus: true, ..UiSettings::default() })),
            clipboard: Box::new(Clipboard),
            automation: Box::new(Automation),
            quit: Box::new(|| {}),
        };
        egui_kittest::Harness::builder().with_size(size).with_step_dt(0.05).build_eframe(move |cc| OpenLaserApp::new(cc, session, services))
    }

    fn harness() -> egui_kittest::Harness<'static, OpenLaserApp> {
        harness_with_size(egui::vec2(1440.0, 900.0))
    }

    #[test]
    fn kittest_lightburn_regions_present() {
        let mut harness = harness();
        harness.run();
        for label in ["File", "Edit", "Tools", "Arrange", "Laser Tools", "Window", "Language", "Help", "Cuts / Layers", "Laser", "Stop laser"] {
            assert!(harness.query_all_by_label(label).next().is_some(), "missing accessible region/control: {label}");
        }
    }

    fn active(h: &mut egui_kittest::Harness<'static, OpenLaserApp>, state: MachineState) -> Arc<std::sync::atomic::AtomicBool> {
        let latch = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let clone = Arc::clone(&latch);
        h.state_mut().machine_state = Some(state);
        h.state_mut().stop_probe = Some(Box::new(move || {
            clone.store(true, std::sync::atomic::Ordering::SeqCst);
            true
        }));
        latch
    }

    #[test]
    fn kittest_palette_has_30_plus_tools() {
        for width in [1440.0, 1024.0, 760.0] {
            let mut h = harness_with_size(egui::vec2(width, 900.0));
            h.run();
            for label in (0..30).map(|n| format!("Layer {n:02}")).chain(["T1".to_owned(), "T2".to_owned()]) {
                let rect = h.get_by_label(&label).rect();
                assert!(rect.left() >= 0.0 && rect.right() <= width, "{label} offscreen at {width}: {rect:?}");
            }
        }
        for idx in 0..32 {
            let rgb = if idx < 30 { openlaser_doc::PALETTE[idx] } else { openlaser_doc::TOOL_COLORS[idx - 30] };
            assert_eq!(palette_color(idx), egui::Color32::from_rgb(rgb.0[0], rgb.0[1], rgb.0[2]));
        }
    }

    #[test]
    fn kittest_accessibility_labels() {
        use egui_kittest::kittest::NodeT;
        let mut h = harness();
        h.run();
        let mut interactive = 0;
        for node in h.root().children_recursive() {
            let node = node.accesskit_node();
            if matches!(
                node.role(),
                egui::accesskit::Role::Button
                    | egui::accesskit::Role::CheckBox
                    | egui::accesskit::Role::TextInput
                    | egui::accesskit::Role::NumberInput
                    | egui::accesskit::Role::SpinButton
                    | egui::accesskit::Role::Slider
                    | egui::accesskit::Role::RadioButton
                    | egui::accesskit::Role::Switch
                    | egui::accesskit::Role::MenuItem
                    | egui::accesskit::Role::MenuItemCheckBox
                    | egui::accesskit::Role::MenuItemRadio
                    | egui::accesskit::Role::ComboBox
                    | egui::accesskit::Role::Tab
            ) {
                interactive += 1;
                assert!(node.label().is_some_and(|s| !s.trim().is_empty()), "unlabelled interactive node: {node:?}");
            }
        }
        assert!(interactive > 50, "the full tree was not walked");
    }

    #[test]
    fn kittest_stop_always_visible_and_enabled() {
        use std::sync::atomic::Ordering;
        for (state, expected_active) in [
            (MachineState::Disconnected, false),
            (MachineState::Connecting, false),
            (MachineState::Idle, false),
            (MachineState::Framing, true),
            (MachineState::Running { packets_acked: 0, total: 1 }, true),
            (MachineState::Stopping, true),
            (MachineState::Stopped, false),
            (MachineState::Fault, false),
            (MachineState::StopNotConfirmed, true),
        ] {
            for panel in ["laser", "closed", "floated", "modal"] {
                for ppp in [1.0, 2.0] {
                    let mut h = harness_with_size(egui::vec2(1024.0, 768.0));
                    h.ctx.set_pixels_per_point(ppp);
                    let latch = active(&mut h, state.clone());
                    if panel == "closed" {
                        h.state_mut().control_dock("ui.dock.close", &serde_json::json!({"panel":"laser"})).unwrap();
                    }
                    if panel == "floated" {
                        h.state_mut().control_dock("ui.dock.float", &serde_json::json!({"panel":"laser"})).unwrap();
                    }
                    // Real egui::Modal; active STOP must stay above its backdrop.
                    h.state_mut().ui.cut_settings_open = panel == "modal";
                    h.run();
                    assert_eq!(h.state().ui.machine_active, expected_active);
                    let stop = h.get_by_label("Stop laser");
                    assert!(!stop.accesskit_node().is_disabled());
                    assert!(stop.rect().is_positive());
                    stop.click();
                    h.run();
                    assert!(latch.load(Ordering::SeqCst), "STOP failed in {state:?}/{panel}/{ppp}");
                }
            }
        }
    }

    #[test]
    fn stop_is_available_while_pending_arm() {
        let mut h = harness();
        let latch = active(&mut h, MachineState::Idle);
        h.state_mut().pending_arm = true;
        h.state_mut().ui.cut_settings_open = true;
        h.run();
        assert!(h.state().ui.machine_active);
        h.get_by_label("Stop laser").click();
        h.run();
        assert!(latch.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!h.state().pending_arm);
    }

    #[test]
    fn kittest_air_hidden_for_galvo() {
        let mut h = harness();
        h.state_mut().ui.cut_settings_open = true;
        h.run();
        assert!(h.query_all_by_label("Air Assist").next().is_none());
    }

    #[test]
    fn kittest_no_file_dialog_while_active() {
        let mut h = harness();
        active(&mut h, MachineState::Framing);
        h.run();
        h.state_mut().invoke_ui_action("file.open");
        assert!(h.state().pending_dialogs.is_empty());
    }

    #[test]
    fn kittest_esc_trips_latch_before_widgets() {
        use std::sync::atomic::Ordering;
        let mut h = harness();
        let latch = active(&mut h, MachineState::Framing);
        let ctx = h.ctx.clone();
        let mut text = "keep this text".to_owned();
        let mut input = egui::RawInput::default();
        let mut focused = None;
        let mut output = ctx.run_ui(input.clone(), |ui| {
            let response = ui.add(egui::TextEdit::singleline(&mut text).id_salt("focused"));
            focused = Some(response.id);
            response.request_focus();
        });
        output.textures_delta.clear();
        assert!(ctx.memory(|memory| memory.focused()) == focused && focused.is_some(), "text field was not focused");
        input.events = crate::control::input::events_for("ui.key", &serde_json::json!({"key":"Escape"})).unwrap();
        h.state_mut().raw_input_hook(&ctx, &mut input);
        assert!(latch.load(Ordering::SeqCst), "Esc did not trip stop before widgets");
        let mut output = ctx.run_ui(input, |ui| {
            ui.menu_button("Open menu", |ui| {
                ui.label("menu item");
            });
            ui.add(egui::TextEdit::singleline(&mut text).id_salt("focused"));
        });
        output.textures_delta.clear();
        assert_eq!(text, "keep this text");
    }

    fn used_project() -> Arc<openlaser_doc::Project> {
        let defaults = openlaser_machine::jpt_m7_200_sim().layer_defaults;
        let mut project = openlaser_doc::Project::new(
            openlaser_doc::Workspace { kind: openlaser_doc::DeviceKind::GalvoLmc, size_mm: [200.0, 200.0], origin: openlaser_doc::WorkspaceOrigin::Center },
            &defaults,
        );
        let op = openlaser_doc::CutOp {
            name: "Line".into(),
            enabled: true,
            mode: openlaser_doc::CutMode::Line,
            passes: 1,
            galvo: defaults.galvo.clone(),
            gantry: defaults.gantry.clone(),
            fill: defaults.fill.clone(),
        };
        project.layers.colors.push(openlaser_doc::ColorLayer {
            index: 0,
            name: "C00".into(),
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
        });
        let shape = openlaser_doc::Shape {
            id: openlaser_types::ObjectId(1),
            name: None,
            layer: openlaser_types::LayerKey::Color(0),
            xform: Default::default(),
            geom: openlaser_doc::Geom::Rect {
                w: openlaser_types::Mm::new(10.0).unwrap(),
                h: openlaser_types::Mm::new(10.0).unwrap(),
                corner_radius: openlaser_types::Mm::new(0.0).unwrap(),
            },
            locked: false,
            power_scale: openlaser_types::Percent::new(100.0).unwrap(),
            priority: 0,
        };
        project.shapes.push(Arc::new(shape));
        Arc::new(project)
    }

    #[test]
    fn kittest_double_click_opens_cut_settings() {
        let mut h = harness();
        h.state_mut().test_project = Some(used_project());
        h.run();
        h.get_by_label("Cut layer C00").click();
        h.run();
        assert!(h.state().last_layer_click.is_some());
        h.get_by_label("Cut layer C00").click();
        h.run();
        assert!(h.state().ui.cut_settings_open);
    }

    #[test]
    fn kittest_start_without_receipt_says_frame_first() {
        let mut h = harness();
        h.run();
        assert!(h.get_by_label("Start").accesskit_node().is_disabled());
        assert!(h.query_by_label("Frame this job first. A human must approve every marking job.").is_none());
        h.get_by_label("Start").hover();
        h.run_steps(20);
        assert!(h.query_by_label("Frame this job first. A human must approve every marking job.").is_some());
    }

    #[test]
    fn kittest_laser_panel_galvo_disabled_reasons() {
        for (label, reason) in [
            ("Start From: Absolute · galvo uses field centre", "Start From is unavailable for galvo machines; the field centre is the origin."),
            ("Pause", "Pause is unavailable for galvo machines; stop and frame again."),
            ("Send", "Send is unavailable for galvo machines; jobs require local approval."),
            ("Home", "Home is unavailable for galvo machines; the field centre is fixed."),
            ("Go to Origin", "Go to Origin is unavailable for galvo machines; use framing."),
        ] {
            let mut h = harness();
            h.run();
            assert!(h.get_by_label(label).accesskit_node().is_disabled());
            assert!(h.query_by_label(reason).is_none());
            h.get_by_label(label).hover();
            h.run_steps(20);
            assert!(h.query_by_label(reason).is_some(), "missing hover reason for {label}");
        }
    }

    struct SyntheticInjector;
    impl crate::HumanSurfaces for SyntheticInjector {
        fn inject(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput, events: Vec<egui::Event>) {
            raw.events.extend(events);
        }
        fn frame_is_synthetic(&self, _ctx: &egui::Context) -> bool {
            true
        }
        // Deliberately adversarial: the widget dispatcher must independently enforce provenance.
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
    #[test]
    fn synthetic_click_on_frame_button_does_not_frame() {
        let mut h = harness();
        h.state_mut().frame_ready = true;
        h.state_mut().services.human = Box::new(SyntheticInjector);
        h.run();
        let frame = h.get_by_label("Frame");
        assert!(!frame.accesskit_node().is_disabled());
        let pos = frame.rect().center();
        let events = crate::control::input::events_for("ui.click", &serde_json::json!({"x":pos.x,"y":pos.y})).unwrap();
        h.state_mut().queue_input(events);
        // egui_kittest 0.36 calls logic/ui but not raw_input_hook, so drive that hook explicitly.
        let ctx = h.ctx.clone();
        let mut raw = egui::RawInput::default();
        h.state_mut().raw_input_hook(&ctx, &mut raw);
        assert_eq!(raw.events.len(), 3, "synthetic click was not injected");
        for event in raw.events {
            h.event(event);
            h.run_steps(1);
        }
        assert!(!h.state().ui.machine_active);
        assert_eq!(h.state().human_actions, 0);
        assert_eq!(h.state().ui.status_message, "Frame requires operator input");
        // NoHuman drops synthetic input; a real harness click also fails the provenance fence.
        h.get_by_label("Frame").click();
        h.run();
        assert_eq!(h.state().ui.status_message, "Frame requires operator input");
    }

    #[test]
    fn canvas_draws_grid_rulers_field_and_work_area() {
        let app = harness().into_state();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 600.0)).build_ui_state(|ui, app: &mut OpenLaserApp| app.paint_canvas(ui), app);
        h.run();
        let shapes = &h.output().shapes;
        assert!(
            shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Rect(rect) if rect.stroke.color==egui::Color32::DARK_GREEN)),
            "missing galvo field"
        );
        assert!(shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Rect(rect) if rect.stroke.color==egui::Color32::LIGHT_GRAY)), "missing work area");
        assert!(
            shapes
                .iter()
                .filter(|shape| matches!(&shape.shape,egui::Shape::LineSegment {stroke,..} if stroke.width==0.5 && stroke.color==egui::Color32::from_gray(65)))
                .count()
                > 6,
            "missing grid"
        );
        assert!(
            shapes.iter().filter(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.text().parse::<f64>().is_ok())).count() > 6,
            "missing ruler ticks"
        );
    }

    #[test]
    fn canvas_wheel_zoom_pan_and_fit() {
        let mut h = harness();
        h.run();
        h.get_by_label("Zoom to fit").click();
        h.run();
        let before = h.state().canvas_zoom.unwrap();
        let position = egui::pos2(200.0, 350.0);
        h.hover_at(position);
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            delta: egui::vec2(0.0, 120.0),
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(5);
        assert!(h.state().canvas_zoom.unwrap() > before);
        h.event(egui::Event::PointerButton { pos: position, button: egui::PointerButton::Middle, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        h.event(egui::Event::PointerMoved(position + egui::vec2(60.0, 30.0)));
        h.run_steps(1);
        h.event(egui::Event::PointerButton {
            pos: position + egui::vec2(60.0, 30.0),
            button: egui::PointerButton::Middle,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(1);
        assert_ne!(h.state().canvas_center, [0.0; 2]);
        h.get_by_label("Zoom to fit").click();
        h.run();
        assert_eq!(h.state().canvas_center, [0.0; 2]);
        assert!((h.state().canvas_zoom.unwrap() - before).abs() < 1e-6);
    }

    #[test]
    fn widgets_dispatch_synthetic_origin() {
        let mut h = harness();
        h.run();
        h.get_by_label("Rectangle").click();
        h.run();
        assert!(h.state().dispatched.iter().any(|(origin, id, _)| *origin == Origin::Synthetic && id == "tools.rectangle"));
        h.get_by_label("Copy").click();
        h.run();
        assert!(h.state().dispatched.iter().any(|(origin, id, _)| *origin == Origin::Synthetic && id == "edit.copy"));
    }

    #[test]
    fn submenus_are_nested() {
        let mut h = harness();
        h.run();
        h.get_by_label("File").click();
        h.run();
        assert!(h.query_by_label("Import Prefs…").is_none());
        h.get_by_label("Preferences ⏵").hover();
        h.run_steps(20);
        assert!(h.query_by_label("Import Prefs…").is_some());
    }

    #[test]
    fn in_window_menu_flag_is_respected_on_both_platforms() {
        assert!(!show_in_window_menus(false, true));
        assert!(show_in_window_menus(true, true));
        assert!(show_in_window_menus(false, false));
        assert!(show_in_window_menus(true, false));
    }

    fn control(h: &mut egui_kittest::Harness<'static, OpenLaserApp>, method: &str, params: serde_json::Value) -> crate::control::Outcome {
        let (tx, _) = std::sync::mpsc::channel();
        let ctx = h.ctx.clone();
        crate::control::handle(
            h.state_mut(),
            &ctx,
            ControlRequest {
                conn: 1,
                client: "test".into(),
                request: openlaser_control::wire::Request { id: serde_json::json!(1), method: method.into(), params },
                reply: tx,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(1),
                dispatch: Arc::new(Default::default()),
            },
        )
    }
    fn reply_error(out: crate::control::Outcome) -> Option<String> {
        match out {
            crate::control::Outcome::Done(reply) | crate::control::Outcome::AfterInput(reply) => reply.error,
            _ => None,
        }
    }
    struct ClosedSurface(crate::NoHuman);
    impl crate::HumanSurfaces for ClosedSurface {
        fn inject(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput, events: Vec<egui::Event>) {
            crate::HumanSurfaces::inject(&mut self.0, ctx, raw, events);
        }
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
    #[test]
    fn ordinary_control_fields_and_cut_dialog_are_editable() {
        let mut h = harness();
        h.state_mut().services.human = Box::new(ClosedSurface(crate::NoHuman));
        h.state_mut().test_project = Some(used_project());
        assert!(reply_error(control(&mut h, "ui.set", serde_json::json!({"field":"activeLayer","value":0}))).is_none());
        assert!(reply_error(control(&mut h, "ui.dialog.open", serde_json::json!({"dialog":"cutSettings"}))).is_none());
        assert!(reply_error(control(&mut h, "ui.dialog.set", serde_json::json!({"dialog":"cutSettings","field":"speedMmS","value":1234}))).is_none());
        assert_eq!(h.state().cut_draft.as_ref().unwrap().speed, 1234.0);
        assert!(reply_error(control(&mut h, "ui.dialog.open", serde_json::json!({"dialog":"arm"}))).unwrap_or_default().contains("human-only"));
        assert!(reply_error(control(&mut h, "ui.drag", serde_json::json!({"from":{"x":10,"y":20},"to":{"x":50,"y":70}}))).is_none());
        assert_eq!(h.state().input_steps.len(), 4);
    }

    #[test]
    fn synthetic_request_card_click_never_approves() {
        let mut app = harness().into_state();
        app.services.human = Box::new(SyntheticInjector);
        let mut h = egui_kittest::Harness::builder().build_ui_state(
            |ui, app: &mut OpenLaserApp| {
                let ctx = ui.ctx().clone();
                app.request_card(ui, &ctx, openlaser_engine::RequestId(1), "Job from test", "Ready for human review");
            },
            app,
        );
        h.run();
        h.get_by_label("Approve framing").click();
        h.run();
        assert!(h.state().ui.status_message.is_empty());
        h.get_by_label("Reject request").click();
        h.run();
        assert!(h.state().ui.status_message.is_empty());
    }

    #[test]
    fn every_control_method_has_a_handler() {
        let mut h = harness();
        h.state_mut().services.human = Box::new(ClosedSurface(crate::NoHuman));
        for method in crate::control::ROUTES.iter().chain(openlaser_control::methods::METHODS.iter()) {
            // NoHuman refuses mutations while its surface is open; test the real routing too by
            // querying reads and explicitly testing every mutation with a closed surface double.
            let error = reply_error(control(&mut h, method, serde_json::json!({}))).unwrap_or_default();
            assert!(!error.contains("unknown method") && !error.contains("not available"), "{method}: {error}");
        }
        assert!(reply_error(control(&mut h, "unknown.method", serde_json::json!({}))).is_some());
    }
    #[test]
    fn control_mutations_refused_while_busy_or_human_surface_open() {
        let mut h = harness();
        active(&mut h, MachineState::Framing);
        for method in ["engine.execute", "ui.menu.invoke", "ui.click", "app.quit"] {
            assert_eq!(reply_error(control(&mut h, method, serde_json::json!({"command":"file.new"}))).as_deref(), Some("machine_busy"));
        }
        for method in ["ui.inspect", "ui.elements", "engine.commands", "ui.screenshot", "machine.stop"] {
            let error = reply_error(control(&mut h, method, serde_json::json!({}))).unwrap_or_default();
            assert!(!error.contains("machine_busy") && !error.contains("human_confirmation_open"), "{method}: {error}");
        }
        h.state_mut().machine_state = Some(MachineState::Idle);
        assert_eq!(reply_error(control(&mut h, "engine.execute", serde_json::json!({"command":"file.new"}))).as_deref(), Some("human_confirmation_open"));
    }

    #[test]
    fn kittest_cuts_columns_and_actual_speed() {
        let mut h = harness();
        h.state_mut().test_project = Some(used_project());
        h.run();
        for label in ["#", "Layer", "Mode", "Spd/Pwr", "Output", "Show"] {
            assert!(h.query_all_by_label(label).next().is_some(), "missing {label}");
        }
        let target = h.get_by_label("Cut layer C00");
        assert!(target.rect().width() >= 24.0);
        assert!(target.rect().height() >= 24.0);
        assert!(h.get_by_label("Output C00").accesskit_node().toggled().is_some());
        assert!(h.get_by_label("Show C00").accesskit_node().toggled().is_some());
        assert!(h.query_all_by_label_contains("(actual)").next().is_none(), "requested speed must not be called actual");
        assert!(h.query_by_label("Cut layer C01").is_none(), "unused layer is listed");
    }

    fn seed_document(h: &mut egui_kittest::Harness<'static, OpenLaserApp>) {
        h.state_mut().session.execute_as(Origin::Synthetic, "file.new", serde_json::json!({})).unwrap();
        let fixture = used_project();
        h.state_mut()
            .session
            .edit(openlaser_engine::Label { wording: "Test fixture", value: None }, None, move |project, selection| {
                *project = fixture.as_ref().clone();
                selection.ids = vec![openlaser_types::ObjectId(1)];
                Ok(())
            })
            .unwrap();
    }
    fn set_off_table_pulse(h: &mut egui_kittest::Harness<'static, OpenLaserApp>) {
        h.state_mut().session.execute_as(Origin::Synthetic, "layer.cutSettings.set", serde_json::json!({"layer":0,"galvo":{"qPulseNs":7}})).unwrap();
    }

    #[test]
    #[ignore = "needs I2"]
    fn kittest_cut_settings_ok_is_one_undo_step() {
        let mut h = harness();
        seed_document(&mut h);
        let before = h.state().session.active().unwrap().history.labels().len();
        let previous = h.state().session.active().unwrap().project.color_layer(0).unwrap().ops[0].galvo.speed;
        h.state_mut().ui.cut_settings_open = true;
        h.run();
        h.state_mut().cut_draft.as_mut().unwrap().speed = 1234.0;
        h.get_by_label("OK").click();
        h.run();
        assert!(!h.state().ui.cut_settings_open);
        assert_eq!(h.state().session.active().unwrap().history.labels().len(), before + 1);
        h.state_mut().session.undo().unwrap();
        assert_eq!(h.state().session.active().unwrap().project.color_layer(0).unwrap().ops[0].galvo.speed, previous);
    }

    #[test]
    #[ignore = "needs I2"]
    fn kittest_palette_click_assigns_selection() {
        let mut h = harness();
        seed_document(&mut h);
        h.run();
        h.get_by_label("Layer 05").click();
        h.run();
        assert_eq!(h.state().session.active().unwrap().project.shapes[0].layer, openlaser_types::LayerKey::Color(5));
        h.state_mut().session.undo().unwrap();
        assert_eq!(h.state().session.active().unwrap().project.shapes[0].layer, openlaser_types::LayerKey::Color(0));
    }

    #[test]
    #[ignore = "needs I2"]
    fn kittest_refusal_badge_for_off_table_pulse() {
        let mut h = harness();
        seed_document(&mut h);
        set_off_table_pulse(&mut h);
        h.run();
        assert!(h.query_all_by_label_contains("refusal").next().is_some());
        assert!(h.query_all_by_label_contains("pulse").next().is_some());
    }

    #[test]
    #[ignore = "needs I2"]
    fn kittest_mark_disabled_while_refusals() {
        let mut h = harness();
        seed_document(&mut h);
        set_off_table_pulse(&mut h);
        h.run();
        assert!(matches!(&h.state().plan_cache.as_ref().unwrap().1,Err(refused) if !refused.refusals.is_empty()));
        assert!(h.get_by_label("Start").accesskit_node().is_disabled());
    }

    #[test]
    fn control_refuses_human_only() {
        let mut h = harness();
        let mut ids: Vec<&str> = vec!["device.connect", "device.settings.set", "laser.frame", "laser.start", "laser.arm", "laser.run"];
        ids.extend(openlaser_engine::registry().specs().iter().filter(|s| s.access == openlaser_engine::Access::Human).map(|s| s.id));
        for id in ids {
            for method in ["engine.execute", "ui.menu.invoke"] {
                assert!(
                    reply_error(control(&mut h, method, serde_json::json!({"command":id}))).unwrap_or_default().contains("human-only"),
                    "{method} allowed {id}"
                );
            }
            assert!(
                reply_error(control(&mut h, "engine.execute", serde_json::json!({"command":"command.batch","params":{"steps":[{"command":id}]}})))
                    .unwrap_or_default()
                    .contains("human-only")
            );
        }
    }

    #[test]
    fn kittest_quit_while_marking_stops_first() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let mut h = harness();
        let latch = active(&mut h, MachineState::Running { packets_acked: 0, total: 1 });
        let quit = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&quit);
        let stopped = Arc::clone(&latch);
        h.state_mut().services.quit = Box::new(move || {
            assert!(stopped.load(Ordering::SeqCst));
            observed.store(true, Ordering::SeqCst);
        });
        let ctx = h.ctx.clone();
        let mut raw = egui::RawInput::default();
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
        let mut output = ctx.run_ui(raw, |ui| h.state_mut().guard_close(ui.ctx()));
        output.textures_delta.clear();
        assert!(output.viewport_output[&egui::ViewportId::ROOT].commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(quit.load(Ordering::SeqCst));
        // A stopped state without a confirmed report must never allow deferred close.
        quit.store(false, Ordering::SeqCst);
        h.state_mut().close_authorized = false;
        h.state_mut().close_after_stop = true;
        h.state_mut().stop_confirmed = false;
        h.state_mut().machine_state = Some(MachineState::Stopped);
        let mut raw = egui::RawInput::default();
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
        let mut output = ctx.run_ui(raw, |ui| h.state_mut().guard_close(ui.ctx()));
        output.textures_delta.clear();
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT].commands.contains(&egui::ViewportCommand::CancelClose),
            "another close must stay blocked until the report arrives"
        );
        assert!(!quit.load(Ordering::SeqCst));
    }

    #[test]
    fn kittest_esc_closes_arm_dialog() {
        let mut h = harness();
        h.state_mut().pending_arm = true;
        assert!(h.state().arm_dialog_open());
        let ctx = h.ctx.clone();
        let mut raw =
            egui::RawInput { events: crate::control::input::events_for("ui.key", &serde_json::json!({"key":"Escape"})).unwrap(), ..Default::default() };
        h.state_mut().raw_input_hook(&ctx, &mut raw);
        assert!(!h.state().pending_arm);
        assert!(!h.state().arm_dialog_open());
    }

    #[test]
    fn hidden_window_still_polls_while_active() {
        let mut h = harness();
        active(&mut h, MachineState::Framing);
        h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().minimized = Some(true);
        let before = h.state().polls;
        h.run_steps(3);
        assert!(h.state().polls >= before + 3);
        assert!(h.state().ui.machine_active);
        assert!(h.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay <= std::time::Duration::from_millis(250));
    }
}
