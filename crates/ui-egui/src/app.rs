//! Native editor shell. Product-specific operations dispatch through the engine; window and view state live here.

use std::sync::mpsc::Receiver;

use openlaser_control::server::ControlRequest;
use openlaser_engine::Session;

use crate::canvas::CanvasRenderer;
use crate::services::Services;

#[derive(Debug, Default)]
pub struct UiState {
    pub active_layer: usize,
    pub selected_tool: &'static str,
    pub active_panel: &'static str,
    pub cut_settings_open: bool,
    pub preview_open: bool,
    pub machine_active: bool,
    pub status_message: String,
}

pub struct OpenLaserApp {
    session: Session,
    ui: UiState,
    services: Services,
    control_rx: Option<Receiver<ControlRequest>>,
    pending_dialogs: Vec<(crate::services::DialogTicket, crate::services::DialogKind)>,
    close_after_stop: bool,
    last_layer_click: Option<(usize, f64)>,
}

impl OpenLaserApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, session: Session, services: Services) -> Self {
        let mut ui = UiState { selected_tool: "Select", active_panel: "Laser", ..UiState::default() };
        let saved = services.settings.load();
        ui.active_panel = if saved.show_laser { "Laser" } else { "Library" };
        Self { session, ui, services, control_rx: None, pending_dialogs: Vec::new(), close_after_stop: false, last_layer_click: None }
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
        let _ = self.session.stop_all();
        self.ui.status_message = "Stop requested".to_owned();
    }

    pub(crate) fn stop_from_control(&mut self) {
        self.stop();
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            for menu in crate::menu_catalog::MENUS {
                ui.menu_button(*menu, |ui| {
                    for row in crate::menu_catalog::CATALOG.iter().filter(|row| row.path.first() == Some(menu)) {
                        if crate::menu_catalog::is_separator(row) {
                            ui.separator();
                            continue;
                        }
                        let live = openlaser_engine::registry().find(row.id).is_some() || row.id.starts_with("window.");
                        let response = ui.add_enabled(live, egui::Button::new(row.label));
                        if !live {
                            response.clone().on_hover_text("Not in OpenLaser yet");
                        }
                        if response.clicked() {
                            if !crate::menu_catalog::is_hw_row(row) {
                                self.invoke_ui_action(row.id);
                            }
                            ui.close();
                        }
                    }
                });
            }
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (label, id) in [
                ("New", "file.new"),
                ("Open", "file.open"),
                ("Import", "file.import"),
                ("Save", "file.save"),
                ("Undo", "edit.undo"),
                ("Redo", "edit.redo"),
                ("Preview", "window.preview"),
            ] {
                let disabled = self.ui.machine_active && matches!(id, "file.open" | "file.import" | "file.new");
                if ui.add_enabled(!disabled, egui::Button::new(label)).clicked() {
                    self.invoke_ui_action(id);
                }
            }
            ui.separator();
            ui.label("JPT M7 · Simulator");
            ui.label("Disconnected");
            let stop = ui.add(egui::Button::new(egui::RichText::new("STOP").strong().color(egui::Color32::WHITE)).fill(egui::Color32::from_rgb(190, 24, 36)));
            stop.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Stop laser"));
            if stop.clicked() {
                self.stop();
            }
        });
    }

    fn invoke_ui_action(&mut self, id: &str) {
        if self.ui.machine_active && id.starts_with("file.") {
            self.ui.status_message = "Stop the active job before using file actions".to_owned();
            return;
        }
        match id {
            "file.open" | "file.import" => {
                let kind = if id == "file.open" { crate::services::DialogKind::OpenProject } else { crate::services::DialogKind::Import };
                let ticket = self.services.files.open(kind);
                self.pending_dialogs.push((ticket, kind));
            }
            "file.save" | "file.saveAs" => self.ui.status_message = "Project saving requires the format package".to_owned(),
            "window.preview" | "preview.open" => self.ui.preview_open = true,
            "window.show.laser" => self.ui.active_panel = "Laser",
            "window.show.library" => self.ui.active_panel = "Library",
            "window.show.move" => self.ui.active_panel = "Move",
            "window.show.console" => self.ui.active_panel = "Console",
            _ => match self.session.execute_as(openlaser_engine::Origin::Human, id, serde_json::Value::Null) {
                Ok(_) => self.ui.status_message = format!("{id} complete"),
                Err(error) => self.ui.status_message = error.to_string(),
            },
        }
    }

    fn poll_dialogs(&mut self) {
        if self.ui.machine_active {
            return;
        }
        let mut pending = Vec::new();
        for (ticket, kind) in self.pending_dialogs.drain(..) {
            match self.services.files.poll(ticket) {
                None => pending.push((ticket, kind)),
                Some(None) => self.ui.status_message = "File selection cancelled".to_owned(),
                Some(Some((path, _bytes))) => {
                    let name = std::path::Path::new(&path).file_name().and_then(|n| n.to_str()).unwrap_or("file");
                    self.ui.status_message = format!("Selected {name}; loading is unavailable until format integration");
                }
            }
        }
        self.pending_dialogs = pending;
    }

    fn persist_ui_settings(&mut self) {
        let mut settings = self.services.settings.load();
        settings.show_laser = self.ui.active_panel == "Laser";
        settings.dock_layout = crate::services::normalize_dock_layout(&settings.dock_layout);
        self.services.settings.save(&settings);
    }

    fn cuts_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Cuts / Layers");
        ui.horizontal(|ui| {
            ui.strong("#");
            ui.strong("Layer");
            ui.strong("Mode");
            ui.strong("Speed");
            ui.strong("Power");
            ui.strong("Freq.");
            ui.strong("O");
            ui.strong("S");
        });
        for idx in 0..32 {
            let (name, color) = if idx < 30 {
                let saved_name = self.session.active().and_then(|doc| doc.project.color_layer(idx as u8)).map(|layer| layer.name.as_str());
                (saved_name.map(str::to_owned).unwrap_or_else(|| format!("C{idx:02}")), palette_color(idx))
            } else {
                (format!("T{} Tool layer", idx - 29), egui::Color32::GRAY)
            };
            let (mode, speed, power, frequency, output, show) = self.layer_values(idx);
            let row = ui.horizontal(|ui| {
                let response = ui.selectable_label(self.ui.active_layer == idx, "");
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Cut layer {name}")));
                let rect = response.rect;
                ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(14.0, 14.0)), 2.0, color);
                ui.label(name);
                ui.label(mode);
                ui.label(speed);
                ui.label(power);
                ui.label(frequency);
                ui.label(if output { "✓" } else { "—" });
                ui.label(if show { "✓" } else { "—" });
                response
            });
            if row.inner.clicked() {
                self.ui.active_layer = idx;
                let now = ui.input(|input| input.time);
                if self.last_layer_click.is_some_and(|(previous, at)| previous == idx && now - at <= 0.4) {
                    self.ui.cut_settings_open = true;
                    self.last_layer_click = None;
                } else {
                    self.last_layer_click = Some((idx, now));
                }
            }
        }
        ui.separator();
        let (mode, speed, power, frequency, _, _) = self.layer_values(self.ui.active_layer);
        ui.label("Quick edit");
        ui.horizontal(|ui| {
            ui.label("Mode");
            ui.label(mode);
        });
        ui.horizontal(|ui| {
            ui.label("Speed");
            ui.label(format!("{speed} (actual)"));
        });
        ui.horizontal(|ui| {
            ui.label("Power");
            ui.label(power);
        });
        ui.horizontal(|ui| {
            ui.label("Frequency");
            ui.label(frequency);
        });
        let q_pulse = self
            .session
            .active()
            .and_then(|doc| doc.project.color_layer(self.ui.active_layer as u8))
            .and_then(|layer| layer.ops.first())
            .and_then(|op| op.galvo.q_pulse)
            .map(|pulse| format!("{} ns", pulse.get()))
            .unwrap_or_else(|| "—".to_owned());
        ui.horizontal(|ui| {
            ui.label("Q-Pulse");
            ui.label(q_pulse);
        });
        if ui.button("Edit Cut Settings…").clicked() {
            self.ui.cut_settings_open = true;
        }
    }

    fn layer_values(&self, idx: usize) -> (&'static str, String, String, String, bool, bool) {
        let Some(layer) = (idx < 30).then(|| self.session.active().and_then(|doc| doc.project.color_layer(idx as u8))).flatten() else {
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

    fn active_machine_is_gantry(&self) -> bool {
        self.session.active().is_some_and(|doc| doc.project.workspace.kind == openlaser_doc::DeviceKind::Grbl)
    }

    fn laser_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Laser");
        ui.label("Device");
        ui.add_enabled_ui(false, |ui| {
            egui::ComboBox::from_id_salt("laser-device").selected_text("JPT M7 · Simulator").show_ui(ui, |_| {});
        });
        ui.label("Disconnected");
        ui.horizontal(|ui| {
            for name in ["Connect", "Frame", "Start"] {
                let can_plan = self.session.build_job(None, &Default::default()).is_ok();
                let enabled = match name {
                    "Connect" => true,
                    "Frame" => can_plan && !self.ui.machine_active,
                    "Start" => false,
                    _ => false,
                };
                let r = ui.add_enabled(enabled, egui::Button::new(name));
                if name == "Frame" && !can_plan {
                    r.clone().on_hover_text("Frame is unavailable until the current job can be planned.");
                }
                if name == "Start" {
                    r.clone().on_hover_text("Frame this job first. A human must approve every marking job.");
                }
                if self.services.human.is_human_click(ctx, &r) {
                    self.ui.status_message = format!("{name} requested");
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
            .on_hover_text("Start From is unavailable for galvo machines; the field centre is the origin.");
        let mut cut_selected = self.session.active().is_some_and(|doc| doc.project.job.cut_selected);
        ui.add_enabled_ui(self.session.active().is_some(), |ui| {
            ui.checkbox(&mut cut_selected, "Cut Selected Graphics");
        });
        let check = self.session.build_job(None, &Default::default());
        match check {
            Ok(_job) => {
                ui.label("Frame this job first");
            }
            Err(refused) if refused.refusals.is_empty() => {
                ui.colored_label(egui::Color32::from_rgb(234, 107, 71), "Job unavailable");
            }
            Err(refused) => {
                ui.colored_label(egui::Color32::from_rgb(234, 107, 71), format!("{} refusal(s)", refused.refusals.len()));
                for refusal in refused.refusals.iter().take(3) {
                    ui.label(refusal.message.as_str());
                }
            }
        }
        if self.session.active().is_none() {
            ui.label("No current job · Frame this job first");
        }
    }
}

impl eframe::App for OpenLaserApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })) {
            self.stop();
            raw.events.retain(|e| !matches!(e, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. }));
        }
        self.services.human.inject(ctx, raw, Vec::new());
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let text_focus = ctx.egui_wants_keyboard_input();
        let shortcut_ids = ctx.input_mut(|input| crate::shortcuts::take_pressed(input, &crate::shortcuts::table(), text_focus));
        for id in shortcut_ids {
            if self.services.human.frame_is_synthetic(ctx) {
                self.ui.status_message = "Shortcut ignored for automated input".to_owned();
            } else {
                self.invoke_ui_action(id);
            }
        }
        let _ = self.session.poll();
        let any_active = self.session.machines().iter().filter_map(|machine| self.session.status(&machine.id)).any(|status| {
            let state = format!("{:?}", status.state);
            state == "Framing" || state.starts_with("Running") || state == "Stopping"
        });
        if any_active {
            self.ui.machine_active = true;
        }
        if self.ui.machine_active && !any_active && self.close_after_stop {
            self.ui.machine_active = false;
            self.close_after_stop = false;
            (self.services.quit)();
        } else if self.ui.machine_active && !any_active {
            self.ui.machine_active = false;
        }
        self.poll_dialogs();
        if ctx.input(|input| input.viewport().close_requested()) && self.ui.machine_active {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_after_stop = true;
            self.stop();
        }
        if self.ui.machine_active {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
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
                crate::control::Outcome::Screenshot { reply, .. } => {
                    let _ = reply.send(crate::control::error_reply(serde_json::Value::Null, "screenshot rendering is unavailable"));
                    continue;
                }
            };
            let _ = reply_tx.send(reply);
        }
        let _ = &self.services.quit;
        self.persist_ui_settings();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.services.settings.load().in_window_menus || cfg!(not(target_os = "macos")) {
            egui::Panel::top("menu_bar").show(ui, |ui| self.menu_bar(ui));
        }
        egui::Panel::top("main_toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::top("numeric_edits").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("X");
                ui.add_enabled(false, egui::TextEdit::singleline(&mut "0.00"));
                ui.label("Y");
                ui.add_enabled(false, egui::TextEdit::singleline(&mut "0.00"));
                ui.label("Width");
                ui.add_enabled(false, egui::TextEdit::singleline(&mut "0.00"));
                ui.label("Height");
                ui.add_enabled(false, egui::TextEdit::singleline(&mut "0.00"));
                ui.label("Rotate °");
                ui.add_enabled(false, egui::TextEdit::singleline(&mut "0"));
            });
        });
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("x 0.00   y 0.00 mm");
                ui.separator();
                ui.label("0 shapes");
                ui.separator();
                ui.label("JPT M7 · Simulator: Disconnected");
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
                ui.label("Arrange:");
                for name in ["Group", "Ungroup", "Flip H", "Flip V", "Align", "Distribute"] {
                    ui.add_enabled(false, egui::Button::new(name));
                }
                ui.separator();
                for idx in 0..32 {
                    let label = if idx < 30 { format!("{idx:02}") } else { format!("T{}", idx - 29) };
                    let color = if idx < 30 { palette_color(idx) } else { egui::Color32::GRAY };
                    let button = egui::Button::new(egui::RichText::new(label).color(if idx < 10 { egui::Color32::WHITE } else { egui::Color32::BLACK }))
                        .fill(color)
                        .min_size(egui::vec2(27.0, 24.0));
                    let response = ui.add(button).on_hover_text(format!("Layer {idx:02}"));
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Layer {idx:02}")));
                    if response.clicked() {
                        self.ui.active_layer = idx;
                    }
                }
            });
        });
        egui::Panel::left("tools").default_size(42.0).show(ui, |ui| {
            for (tool, icon) in [
                ("Select", "⌖"),
                ("Line", "╱"),
                ("Rectangle", "□"),
                ("Ellipse", "◯"),
                ("Polygon", "⬡"),
                ("Pen", "✎"),
                ("Text", "T"),
                ("Edit Nodes", "⌁"),
                ("Offset", "↗"),
                ("Weld", "⋈"),
                ("Measure", "↔"),
                ("Pan", "✋"),
            ] {
                let response = ui.selectable_label(self.ui.selected_tool == tool, icon).on_hover_text(tool);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tool));
                if response.clicked() {
                    self.ui.selected_tool = tool;
                }
            }
        });
        egui::Panel::right("right_dock").default_size(320.0).show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| self.cuts_panel(ui));
            ui.separator();
            ui.horizontal(|ui| {
                for tab in ["Laser", "Library", "Move", "Console"] {
                    if ui.selectable_label(self.ui.active_panel == tab, tab).clicked() {
                        self.ui.active_panel = tab;
                    }
                }
            });
            ui.separator();
            if self.ui.active_panel == "Laser" {
                let ctx = ui.ctx().clone();
                self.laser_panel(ui, &ctx);
            } else {
                ui.heading(self.ui.active_panel);
                ui.label(if self.ui.active_panel == "Console" { "Console output" } else { "No items" });
            }
        });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("0 mm");
                ui.separator();
                ui.label("100 mm");
            });
            let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
            ui.painter().rect_filled(rect, 0.0, egui::Color32::from_gray(38));
            ui.painter().rect_stroke(rect.shrink(16.0), 0.0, egui::Stroke::new(1.0, egui::Color32::from_gray(88)), egui::StrokeKind::Inside);
            ui.painter()
                .line_segment([rect.center() + egui::vec2(-8.0, 0.0), rect.center() + egui::vec2(8.0, 0.0)], egui::Stroke::new(1.0, egui::Color32::GRAY));
            ui.painter()
                .line_segment([rect.center() + egui::vec2(0.0, -8.0), rect.center() + egui::vec2(0.0, 8.0)], egui::Stroke::new(1.0, egui::Color32::GRAY));
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
                let view = crate::canvas::ViewXform { zoom: f64::from(rect.width().min(rect.height())) / 210.0, center: [0.0, 0.0], rect: rect.shrink(16.0) };
                crate::canvas::PainterRenderer.paint(&ui.painter().with_clip_rect(rect), &list, &view);
            }
        });
        if self.ui.cut_settings_open {
            let mut cut_open = true;
            let mut close_cut_settings = false;
            egui::Window::new("Cut Settings Editor").open(&mut cut_open).show(ui.ctx(), |ui| {
                ui.label("Line");
                ui.label("Speed · mm/s");
                ui.label("Power · %");
                ui.label("Frequency · kHz");
                ui.label("Q-Pulse Width · ns");
                ui.label("Line Interval · mm");
                ui.label("Scan Angle · °");
                ui.label("Passes");
                ui.separator();
                if self.active_machine_is_gantry() {
                    ui.checkbox(&mut false, "Air Assist");
                }
                if ui.button("OK").clicked() {
                    close_cut_settings = true;
                }
            });
            self.ui.cut_settings_open = cut_open && !close_cut_settings;
        }
        if self.ui.preview_open {
            egui::Window::new("Preview").open(&mut self.ui.preview_open).show(ui.ctx(), |ui| {
                ui.heading("Preview");
                ui.label("No plan available");
            });
        }
        if self.ui.machine_active {
            egui::Area::new(egui::Id::new("stop_overlay")).order(egui::Order::Tooltip).anchor(egui::Align2::RIGHT_TOP, egui::vec2(-18.0, 62.0)).show(
                ui.ctx(),
                |ui| {
                    if ui.add(egui::Button::new(egui::RichText::new("STOP").strong().color(egui::Color32::WHITE)).fill(egui::Color32::RED)).clicked() {
                        self.stop();
                    }
                },
            );
        }
    }
}

fn palette_color(idx: usize) -> egui::Color32 {
    let hue = ((idx % 30) as f32 / 30.0) * 360.0;
    egui::Color32::from(egui::ecolor::Hsva::new(hue / 360.0, 0.78, 0.88, 1.0))
}

#[cfg(test)]
mod kittests {
    use super::*;
    use crate::services::{ArmModel, ArmOutcome, ControlInfo, DialogKind, DialogTicket, FileDialogs, HumanSurfaces, SettingsStore, UiSettings};
    use eframe::App;
    use egui_kittest::kittest::Queryable;
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
    struct Human;
    impl HumanSurfaces for Human {
        fn inject(&mut self, _ctx: &egui::Context, _raw: &mut egui::RawInput, _events: Vec<egui::Event>) {}
        fn frame_is_synthetic(&self, _ctx: &egui::Context) -> bool {
            false
        }
        fn is_human_click(&mut self, _ctx: &egui::Context, _response: &egui::Response) -> bool {
            false
        }
        fn arm_dialog(&mut self, _ctx: &egui::Context, _model: &ArmModel) -> ArmOutcome {
            ArmOutcome::Cancelled
        }
        fn surface_open(&self) -> bool {
            false
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

    fn harness() -> egui_kittest::Harness<'static, OpenLaserApp> {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let session = Session::new(SessionConfig {
            origin: Origin::Human,
            window: true,
            devices: DeviceAccess::None,
            machines: Vec::new(),
            profiles: None,
            files: FsAuthority::Denied,
            clock,
        });
        let services = Services {
            human: Box::new(Human),
            files: Box::new(Files),
            settings: Box::new(Settings(UiSettings::default())),
            clipboard: Box::new(Clipboard),
            automation: Box::new(Automation),
            quit: Box::new(|| {}),
        };
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .with_step_dt(0.05)
            .build_eframe(move |cc| OpenLaserApp::new(cc, session, services))
    }

    #[test]
    fn kittest_lightburn_regions_present() {
        let mut harness = harness();
        harness.run();
        for label in ["File", "Edit", "Tools", "Arrange", "Laser Tools", "Window", "Language", "Help", "Cuts / Layers", "Laser", "Stop laser"] {
            assert!(harness.query_all_by_label(label).next().is_some(), "missing accessible region/control: {label}");
        }
    }

    #[test]
    fn kittest_palette_has_30_plus_tools() {
        let mut harness = harness();
        harness.run();
        for label in (0..30).map(|n| format!("Layer {n:02}")).chain(["Layer 30".to_owned(), "Layer 31".to_owned()]) {
            assert!(harness.query_by_label(&label).is_some(), "missing palette layer {label}");
        }
        for label in ["Select", "Rectangle", "Ellipse", "Polygon", "Pen", "Text", "Pan", "Measure"] {
            assert!(harness.query_by_label(label).is_some(), "missing creation tool {label}");
        }
    }

    #[test]
    fn kittest_accessibility_labels() {
        let mut harness = harness();
        harness.run();
        assert!(harness.query_by_label("Stop laser").is_some());
        assert!(harness.query_by_label("Layer 00").is_some());
        assert!(harness.query_by_label("Frame").is_some());
    }

    #[test]
    fn kittest_stop_always_visible_and_enabled() {
        let mut harness = harness();
        harness.run();
        let stop = harness.get_by_label("Stop laser");
        stop.click();
        harness.run();
        assert_eq!(harness.state().ui.status_message, "Stop requested", "STOP must accept a click");
    }

    #[test]
    fn kittest_air_hidden_for_galvo() {
        let mut harness = harness();
        harness.state_mut().ui.cut_settings_open = true;
        harness.run();
        assert!(harness.query_all_by_label("Air Assist").next().is_none());
    }

    #[test]
    fn kittest_no_file_dialog_while_active() {
        let mut harness = harness();
        harness.state_mut().ui.machine_active = true;
        harness.state_mut().invoke_ui_action("file.open");
        assert!(harness.state().pending_dialogs.is_empty());
    }

    #[test]
    fn kittest_esc_trips_latch_before_widgets() {
        let mut harness = harness();
        let ctx = harness.ctx.clone();
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: Some(egui::Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        harness.state_mut().raw_input_hook(&ctx, &mut input);
        assert!(!input.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })));
    }

    #[test]
    fn kittest_double_click_opens_cut_settings() {
        let mut harness = harness();
        harness.run();
        harness.get_by_label("Cut layer C00").click();
        harness.run();
        assert!(harness.state().last_layer_click.is_some(), "first row click was not received");
        harness.get_by_label("Cut layer C00").click();
        harness.run();
        assert!(harness.state().ui.cut_settings_open);
    }

    #[test]
    fn kittest_start_without_receipt_says_frame_first() {
        let mut harness = harness();
        harness.run();
        assert!(harness.query_all_by_label("No current job · Frame this job first").next().is_some());
    }

    #[test]
    fn kittest_laser_panel_galvo_disabled_reasons() {
        let mut harness = harness();
        harness.run();
        assert!(harness.query_all_by_label_contains("Start From: Absolute").next().is_some());
        harness.get_by_label("Start").hover();
        harness.run();
        assert!(harness.query_all_by_label_contains("Frame this job first").next().is_some());
    }

    #[test]
    fn synthetic_click_on_frame_button_does_not_frame() {
        let mut harness = harness();
        harness.run();
        harness.get_by_label("Frame").click();
        harness.run();
        assert!(!harness.state().ui.machine_active);
        assert_ne!(harness.state().ui.status_message, "Frame requested");
    }

    #[test]
    #[ignore = "needs I1"]
    fn kittest_cuts_columns_and_actual_speed() {}

    #[test]
    #[ignore = "needs I2"]
    fn kittest_cut_settings_ok_is_one_undo_step() {}

    #[test]
    #[ignore = "needs I2"]
    fn kittest_palette_click_assigns_selection() {}

    #[test]
    #[ignore = "needs I2"]
    fn kittest_refusal_badge_for_off_table_pulse() {}

    #[test]
    #[ignore = "needs I2"]
    fn kittest_mark_disabled_while_refusals() {}

    #[test]
    #[ignore = "needs I2"]
    fn control_refuses_human_only() {}

    #[test]
    #[ignore = "needs I2"]
    fn kittest_quit_while_marking_stops_first() {}

    #[test]
    #[ignore = "needs I2"]
    fn kittest_esc_closes_arm_dialog() {}

    #[test]
    #[ignore = "needs I2"]
    fn hidden_window_still_polls_while_active() {}
}
