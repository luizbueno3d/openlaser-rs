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
}

impl OpenLaserApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, session: Session, services: Services) -> Self {
        Self { session, ui: UiState { selected_tool: "Select", active_panel: "Laser", ..UiState::default() }, services, control_rx: None }
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
        self.ui.machine_active = false;
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
                            self.ui.status_message = format!("{} is not available yet", row.label);
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
                if ui.button(label).clicked() {
                    if id == "window.preview" {
                        self.ui.preview_open = true;
                    } else {
                        self.ui.status_message = format!("{} is not available yet", label);
                    }
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

    fn cuts_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Cuts / Layers");
        ui.horizontal(|ui| {
            ui.strong("#");
            ui.strong("Layer");
            ui.strong("Mode");
            ui.strong("Speed");
            ui.strong("Power");
            ui.strong("O");
            ui.strong("S");
        });
        for idx in 0..32 {
            let (name, color) =
                if idx < 30 { (format!("{idx:02} C{idx:02}"), palette_color(idx)) } else { (format!("T{} Tool layer", idx - 29), egui::Color32::GRAY) };
            let row = ui.horizontal(|ui| {
                let response = ui.selectable_label(self.ui.active_layer == idx, "");
                let rect = response.rect;
                ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(14.0, 14.0)), 2.0, color);
                ui.label(name);
                ui.label(if idx < 30 { "Line" } else { "Tool" });
                ui.label("1000 mm/s");
                ui.label("30 %");
                ui.label("✓");
                ui.label("✓");
                response
            });
            if row.inner.clicked() {
                self.ui.active_layer = idx;
            }
            if row.inner.double_clicked() {
                self.ui.cut_settings_open = true;
            }
        }
        ui.separator();
        ui.label("Quick edit");
        ui.horizontal(|ui| {
            ui.label("Mode");
            ui.label("Line");
        });
        ui.horizontal(|ui| {
            ui.label("Speed");
            ui.label("1000 mm/s (actual)");
        });
        ui.horizontal(|ui| {
            ui.label("Power");
            ui.label("30 %");
        });
        ui.horizontal(|ui| {
            ui.label("Frequency");
            ui.label("30 kHz");
        });
        ui.horizontal(|ui| {
            ui.label("Q-Pulse");
            ui.label("200 ns");
        });
        if ui.button("Edit Cut Settings…").clicked() {
            self.ui.cut_settings_open = true;
        }
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
                let r = ui.button(name);
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
        ui.label("Start From: Absolute (not available for galvo)");
        ui.checkbox(&mut false, "Cut Selected Graphics");
        ui.label("Frame this job first");
        ui.label("No current job");
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
        let _ = self.session.poll();
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
            for tool in ["Select", "Line", "Rectangle", "Ellipse", "Polygon", "Text", "Pan", "Measure"] {
                let response = ui.selectable_label(self.ui.selected_tool == tool, tool);
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
                ui.label("Air Assist is available for gantry machines only.");
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
