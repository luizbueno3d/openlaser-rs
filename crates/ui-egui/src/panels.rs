//! The regions of LightBurn's layout (UI §4.1): menu bar, main toolbar with the machine chip and STOP, numeric edits,
//! the left tool palette, the dock (Cuts / Layers above Laser / Library / Move / Console), the Arrange toolbar and
//! colour palette, the status bar, and the STOP overlay.

use openlaser_engine::{EmissionSummary, MachineState};
use openlaser_types::{FrameOrigin, FrameReceipt, LayerKey, MonoTime, SessionId};

use crate::app::{OpenLaserApp, PlanView, state_requires_stop};
use crate::menu_catalog::{self, CATALOG, MENUS};

/// STOP's colours are fixed and theme-independent (UI §9.1): white on saturated red.
pub(crate) const STOP_RED: egui::Color32 = egui::Color32::from_rgb(196, 18, 28);

pub(crate) const NOT_YET: &str = "Not in OpenLaser yet";
pub(crate) const FILES_WHILE_ACTIVE: &str = "Stop the active frame or job before opening, importing or saving files.";

/// Lucide icons (ISC, `assets/icons`, ATTRIBUTION.md), recoloured white and tinted by egui.
pub(crate) fn icon_image(name: &str) -> egui::Image<'static> {
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

/// Text painted without an AccessKit node, for captions whose input carries the same label (no duplicate labels).
pub(crate) fn caption(ui: &mut egui::Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), egui::TextStyle::Body.resolve(ui.style()), ui.visuals().text_color());
    let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    ui.painter().galley(rect.min, galley, ui.visuals().text_color());
}

/// Palette slot 0..=31 → layer (30 = T1, 31 = T2).
pub(crate) fn slot_key(slot: usize) -> Option<LayerKey> {
    match slot {
        0..=29 => u8::try_from(slot).ok().map(LayerKey::Color),
        30 => Some(LayerKey::Tool(1)),
        31 => Some(LayerKey::Tool(2)),
        _ => None,
    }
}

/// The swatch text: "00".."29", "T1", "T2".
pub(crate) fn swatch_text(key: LayerKey) -> String {
    match key {
        LayerKey::Color(i) => format!("{i:02}"),
        LayerKey::Tool(t) => format!("T{t}"),
    }
}

/// The swatch's AccessKit label (UI §4.12): "Layer 05", "T1".
pub(crate) fn swatch_label(key: LayerKey) -> String {
    match key {
        LayerKey::Color(i) => format!("Layer {i:02}"),
        LayerKey::Tool(t) => format!("T{t}"),
    }
}

/// Display colour from `openlaser_doc::PALETTE` / `TOOL_COLORS` (interoperability values, provenance in doc).
pub(crate) fn palette_color(key: LayerKey) -> egui::Color32 {
    let rgb = match key {
        LayerKey::Color(i) => openlaser_doc::PALETTE.get(usize::from(i)),
        LayerKey::Tool(t) => openlaser_doc::TOOL_COLORS.get(usize::from(t.saturating_sub(1))),
    };
    rgb.map(|rgb| egui::Color32::from_rgb(rgb.0[0], rgb.0[1], rgb.0[2])).unwrap_or(egui::Color32::GRAY)
}

/// Black or white, whichever reads on `fill`.
pub(crate) fn text_on(fill: egui::Color32) -> egui::Color32 {
    let luminance = 0.299 * f32::from(fill.r()) + 0.587 * f32::from(fill.g()) + 0.114 * f32::from(fill.b());
    if luminance > 140.0 { egui::Color32::BLACK } else { egui::Color32::WHITE }
}

/// The layer key as command params expect it ("C05", "T1").
pub(crate) fn layer_param(key: LayerKey) -> serde_json::Value {
    serde_json::Value::String(key.to_string())
}

/// Layers that have shapes, in output order (`layers.order` first, then any other used layer by key).
pub(crate) fn used_layers(project: &openlaser_doc::Project) -> Vec<LayerKey> {
    fn collect(shapes: &[std::sync::Arc<openlaser_doc::Shape>], used: &mut std::collections::BTreeSet<LayerKey>, depth: usize) {
        for shape in shapes {
            match &shape.geom {
                openlaser_doc::Geom::Group(children) if depth < 64 => collect(children, used, depth + 1),
                _ => {
                    used.insert(shape.layer);
                }
            }
        }
    }
    let mut used = std::collections::BTreeSet::new();
    collect(&project.shapes, &mut used, 0);
    let mut ordered: Vec<LayerKey> = project.layers.order.iter().copied().filter(|k| used.contains(k)).collect();
    ordered.extend(used.into_iter().filter(|k| !project.layers.order.contains(k)));
    ordered.dedup();
    ordered
}

/// Is a command usable from a toolbar or menu now: a live catalogue row, or a registered engine command.
pub(crate) fn command_live(id: &str) -> bool {
    match menu_catalog::find(id) {
        Some(row) => menu_catalog::is_live(row),
        None => openlaser_engine::registry().find(id).is_some(),
    }
}

/// Start (human-only) needs a plan without refusals and a matching human FrameReceipt ≤ 10 min old (UI §4.9).
pub(crate) fn start_gate(plan: Option<&PlanView>, receipt: Option<&FrameReceipt>, session: SessionId, now: MonoTime, active: bool) -> Result<(), String> {
    let summary = ready_summary(plan)?;
    if active {
        return Err("The machine is busy. Stop it, or wait for the frame or job to finish.".into());
    }
    let Some(receipt) = receipt.filter(|r| {
        r.fingerprint == summary.fingerprint && r.machine == summary.machine && r.session == session && r.origin == FrameOrigin::Human && r.covers_job()
    }) else {
        return Err("Frame this job first. A human must approve every marking job.".into());
    };
    if receipt.full_passes == 0 {
        return Err("Frame this job first: no full frame pass has been executed yet.".into());
    }
    if now.saturating_since(receipt.at) > openlaser_types::FRAME_RECEIPT_MAX_AGE {
        return Err("The frame is older than 10 minutes. Frame this job again.".into());
    }
    Ok(())
}

/// Frame (human-only) needs a plan without refusals and an idle machine.
pub(crate) fn frame_gate(plan: Option<&PlanView>, active: bool) -> Result<(), String> {
    ready_summary(plan)?;
    if active {
        return Err("The machine is busy. Stop it first.".into());
    }
    Ok(())
}

fn ready_summary(plan: Option<&PlanView>) -> Result<&EmissionSummary, String> {
    match plan {
        None => Err("No job: open, import or draw a design first.".into()),
        Some(PlanView::Refused(refused)) if refused.refusals.is_empty() => Err(refused.error.clone().unwrap_or_else(|| "The job cannot be planned.".into())),
        Some(PlanView::Refused(refused)) => Err(format!("{} refusal(s) block this job. Fix them first (see the list below).", refused.refusals.len())),
        Some(PlanView::Ready(summary)) => Ok(summary),
    }
}

/// The machine state chip (UI §4.3 / §4.9).
pub(crate) fn state_text(state: Option<&MachineState>, armed: bool) -> (String, egui::Color32) {
    if armed {
        return ("Armed".into(), egui::Color32::from_rgb(230, 140, 30));
    }
    match state {
        None | Some(MachineState::Disconnected) => ("Disconnected".into(), egui::Color32::GRAY),
        Some(MachineState::Connecting) => ("Connecting…".into(), egui::Color32::GRAY),
        Some(MachineState::Idle) => ("Ready".into(), egui::Color32::from_rgb(60, 170, 90)),
        Some(MachineState::Framing) => ("Framing".into(), egui::Color32::from_rgb(230, 80, 60)),
        Some(MachineState::Running { packets_acked, total }) => {
            let pct = if *total == 0 { 0 } else { u64::from(*packets_acked) * 100 / u64::from(*total) };
            (format!("Marking {pct}%"), egui::Color32::from_rgb(230, 60, 40))
        }
        Some(MachineState::Stopping) => ("Stopping".into(), egui::Color32::from_rgb(230, 140, 30)),
        Some(MachineState::Stopped) => ("Stopped".into(), egui::Color32::GRAY),
        Some(MachineState::Fault) => ("Fault".into(), egui::Color32::from_rgb(230, 60, 40)),
        Some(MachineState::StopNotConfirmed) => ("STOP NOT CONFIRMED".into(), STOP_RED),
    }
}

const TOOLBAR_GROUPS: &[&[(&str, &str, &str)]] = &[
    &[("New", "file.new", "file-plus"), ("Open", "file.open", "folder-open"), ("Import", "file.import", "file-input"), ("Save", "file.save", "save")],
    &[("Undo", "edit.undo", "undo-2"), ("Redo", "edit.redo", "redo-2")],
    &[("Cut", "edit.cut", "scissors"), ("Copy", "edit.copy", "copy"), ("Paste", "edit.paste", "clipboard-paste"), ("Delete", "edit.delete", "trash-2")],
    &[
        ("Pan / Drag View", "view.pan", "hand"),
        ("Zoom In", "view.zoomIn", "zoom-in"),
        ("Zoom Out", "view.zoomOut", "zoom-out"),
        ("Zoom to Page", "view.zoomPage", "maximize"),
        ("Frame Selection", "view.zoomSelection", "scan"),
    ],
    &[("Settings", "edit.settings", "settings"), ("Device Settings", "edit.deviceSettings", "settings-2")],
    &[("Trace Image", "tools.traceImage", "wand-sparkles"), ("Preview", "preview.open", "eye"), ("Camera", "laserTools.cameras", "camera")],
];

const LEFT_TOOLS: &[(&str, &str, &str)] = &[
    ("Select", "tools.select", "mouse-pointer-2"),
    ("Draw Lines", "tools.drawLines", "pen-tool"),
    ("Rectangle", "tools.rectangle", "rectangle-horizontal"),
    ("Ellipse", "tools.ellipse", "circle"),
    ("Polygon", "tools.polygon", "pentagon"),
    ("Edit Text", "tools.text", "type"),
    ("Edit Nodes", "tools.editNodes", "spline"),
    ("Measure", "tools.measure", "ruler"),
    ("Offset Shapes", "tools.offsetShapes", "layers"),
    ("Weld Shapes", "tools.weld", "link-2"),
];

const ARRANGE_TOOLS: &[(&str, &str, &str)] = &[
    ("Group", "arrange.group", "link-2"),
    ("Ungroup", "arrange.ungroup", "unlink"),
    ("Flip Horizontal", "arrange.flipH", "flip-horizontal-2"),
    ("Flip Vertical", "arrange.flipV", "flip-vertical-2"),
    ("Rotate 90° Clockwise", "arrange.rotateCw90", "rotate-cw"),
    ("Rotate 90° Counter-Clockwise", "arrange.rotateCcw90", "rotate-ccw"),
    ("Page Center", "arrange.moveSelection.pageCenter", "move"),
];

const BUTTON: egui::Vec2 = egui::vec2(28.0, 26.0);

impl OpenLaserApp {
    // ---- menu bar (in-window) ---------------------------------------------------------------------------------------

    pub(crate) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            for menu in MENUS {
                let response = ui.menu_button(*menu, |ui| self.menu_branch(ui, &[*menu])).response;
                self.track(&response, "menu", *menu, false);
            }
        });
    }

    /// One menu level: rows of `path`, with a nested submenu per next path segment, in catalogue order.
    pub(crate) fn menu_branch(&mut self, ui: &mut egui::Ui, path: &[&'static str]) {
        let mut seen: Vec<&str> = Vec::new();
        for row in CATALOG.iter().filter(|row| row.path.starts_with(path)) {
            if let Some(next) = row.path.get(path.len()) {
                if !seen.contains(next) {
                    seen.push(next);
                    let mut nested = path.to_vec();
                    nested.push(next);
                    ui.menu_button(*next, |ui| self.menu_branch(ui, &nested));
                }
            } else if menu_catalog::is_separator(row) {
                ui.separator();
            } else {
                let live = menu_catalog::is_live(row);
                let blocked = self.ui.machine_active && crate::app::opens_files(row.id);
                let mut button = egui::Button::new(row.label);
                if let Some(shortcut) = row.shortcut {
                    button = button.shortcut_text(crate::shortcuts::pretty(shortcut));
                }
                let response = ui.add_enabled(live && !blocked, button);
                // The reason names the predicate that actually failed: `!live` before `blocked` (a row that
                // is not in OpenLaser yet is not "blocked by an active machine").
                let response = if menu_catalog::is_hw_row(row) {
                    response.on_disabled_hover_text("Hardware actions are human-only and live in the Laser panel.")
                } else if !live {
                    response.on_disabled_hover_text(NOT_YET)
                } else {
                    response.on_disabled_hover_text(FILES_WHILE_ACTIVE)
                };
                if response.clicked() {
                    self.invoke_menu_command(ui.ctx(), &menu_catalog::native_id(row));
                    ui.close();
                }
            }
        }
    }

    // ---- main toolbar ------------------------------------------------------------------------------------------------

    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui) {
        let overlay = self.stop_overlay_needed(ui.ctx());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The machine group is never part of the overflow (UI §7.1): STOP and the state chip come first.
            let stop =
                ui.add(egui::Button::new(egui::RichText::new("STOP").strong().color(egui::Color32::WHITE)).fill(STOP_RED).min_size(egui::vec2(64.0, BUTTON.y)));
            self.labeled(&stop, egui::WidgetType::Button, if overlay { "Stop laser (toolbar)" } else { "Stop laser" }, false);
            if stop.on_hover_text("Stop laser (Esc while active). The E-stop always works.").clicked() {
                self.stop();
            }
            let (text, color) = self.chip();
            let chip = ui.add(egui::Label::new(egui::RichText::new(text).color(color).strong()).truncate());
            self.track(&chip, "status", "Machine state", false);
            ui.separator();
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| self.toolbar_groups(ui));
        });
    }

    fn toolbar_groups(&mut self, ui: &mut egui::Ui) {
        let spacing = ui.spacing().item_spacing.x;
        let natural: Vec<f32> = TOOLBAR_GROUPS.iter().map(|g| g.len() as f32 * (BUTTON.x + spacing) + 8.0).collect();
        let fit = craft_ui::layout::fit_toolbar(&natural, ui.available_width().max(0.0), spacing, 30.0);
        let (shown, overflow): (Vec<usize>, Vec<usize>) = match fit {
            Some(fit) => (fit.shown.iter().map(|(i, _)| *i).collect(), fit.overflow),
            // Too narrow even for the overflow control: every group goes into the overflow menu.
            None => (Vec::new(), (0..TOOLBAR_GROUPS.len()).collect()),
        };
        for (n, index) in shown.iter().enumerate() {
            if n > 0 {
                ui.separator();
            }
            for (label, id, icon) in TOOLBAR_GROUPS.get(*index).copied().unwrap_or_default() {
                self.toolbar_button(ui, label, id, icon);
            }
        }
        if !overflow.is_empty() {
            let response = ui
                .menu_button("»", |ui| {
                    for index in &overflow {
                        for (label, id, _) in TOOLBAR_GROUPS.get(*index).copied().unwrap_or_default() {
                            let (enabled, reason) = self.toolbar_state(id);
                            let r = ui.add_enabled(enabled, egui::Button::new(*label));
                            let r = match reason {
                                Some(reason) => r.on_disabled_hover_text(reason),
                                None => r,
                            };
                            if r.clicked() {
                                self.invoke_menu_command(ui.ctx(), id);
                                ui.close();
                            }
                        }
                        ui.separator();
                    }
                })
                .response;
            self.labeled(&response, egui::WidgetType::Button, "More toolbar buttons", false);
        }
    }

    fn toolbar_state(&self, id: &str) -> (bool, Option<&'static str>) {
        if !command_live(id) {
            (false, Some(NOT_YET))
        } else if self.ui.machine_active && crate::app::opens_files(id) {
            (false, Some(FILES_WHILE_ACTIVE))
        } else {
            (true, None)
        }
    }

    fn toolbar_button(&mut self, ui: &mut egui::Ui, label: &str, id: &str, icon: &str) {
        let (enabled, reason) = self.toolbar_state(id);
        let selected = id == "view.pan" && self.ui.selected_tool == "Pan";
        let response = ui.add_enabled(enabled, egui::Button::image(icon_image(icon)).selected(selected).min_size(BUTTON));
        self.labeled(&response, egui::WidgetType::Button, label, false);
        let response = match reason {
            Some(reason) => response.on_disabled_hover_text(format!("{label}: {reason}")),
            None => response.on_hover_text(label),
        };
        if response.clicked() {
            self.invoke_menu_command(ui.ctx(), id);
        }
    }

    /// "JPT M7 · Simulator: Ready".
    pub(crate) fn chip(&self) -> (String, egui::Color32) {
        let name = self.active_machine().map(|m| m.name.clone()).unwrap_or_else(|| "No device".into());
        let states = self.machine_states();
        let state = self
            .active_machine()
            .and_then(|m| states.iter().find(|(id, _)| *id == m.id))
            .or_else(|| states.iter().find(|(_, s)| state_requires_stop(s)))
            .map(|(_, s)| s.clone());
        let (text, color) = state_text(state.as_ref(), self.arm_dialog_open());
        (format!("{name}: {text}"), color)
    }

    // ---- numeric edits ------------------------------------------------------------------------------------------------

    pub(crate) fn numeric_edits(&mut self, ui: &mut egui::Ui) {
        let selection = self.selection();
        let bounds = self.project().and_then(|p| p.bounds(&selection));
        let enabled = bounds.is_some() && !self.ui.machine_active;
        let reason = if self.ui.machine_active { "Edits are locked while a machine is active." } else { "Select shapes to edit their position and size." };
        ui.horizontal(|ui| {
            // The 9-point anchor: which point of the selection bounds XPos/YPos name.
            egui::Grid::new("anchor-grid").spacing([2.0, 2.0]).min_col_width(8.0).min_row_height(8.0).show(ui, |ui| {
                for n in 0..9u8 {
                    let name = ["top-left", "top", "top-right", "left", "centre", "right", "bottom-left", "bottom", "bottom-right"]
                        .get(usize::from(n))
                        .copied()
                        .unwrap_or("centre");
                    // A painted dot per anchor point (one AccessKit radio button each), LightBurn's 9-dot selector.
                    let (rect, r) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::click());
                    let selected = self.ui.anchor == n;
                    let color = if selected {
                        ui.visuals().selection.bg_fill
                    } else if r.hovered() {
                        ui.visuals().text_color()
                    } else {
                        ui.visuals().weak_text_color()
                    };
                    ui.painter().circle(rect.center(), if selected { 4.0 } else { 2.5 }, color, egui::Stroke::NONE);
                    self.labeled(&r, egui::WidgetType::RadioButton, &format!("Anchor {name}"), false);
                    if r.on_hover_text(format!("XPos/YPos name the {name} of the selection")).clicked() {
                        self.ui.anchor = n;
                    }
                    if n % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
            let (col, row) = (f64::from(self.ui.anchor % 3), f64::from(self.ui.anchor / 3));
            let values = bounds.map(|b| {
                let (w, h) = (b.max[0] - b.min[0], b.max[1] - b.min[1]);
                [b.min[0] + w * col / 2.0, b.max[1] - h * row / 2.0, w, h, 100.0, 100.0, 0.0]
            });
            let fields: [(&str, &str, &str); 7] = [
                ("XPos", "x", "mm"),
                ("YPos", "y", "mm"),
                ("Width", "w", "mm"),
                ("Height", "h", "mm"),
                ("W%", "wPct", "%"),
                ("H%", "hPct", "%"),
                ("Rotate", "rotate", "°"),
            ];
            for (n, (label, key, unit)) in fields.iter().enumerate() {
                if n == 4 {
                    let icon = if self.ui.lock_ratio { "lock" } else { "lock-open" };
                    let r = ui.add(egui::Button::image(icon_image(icon)).selected(self.ui.lock_ratio));
                    self.labeled(&r, egui::WidgetType::Checkbox, "Lock aspect ratio", false);
                    if r.on_hover_text("Lock the aspect ratio").clicked() {
                        self.ui.lock_ratio = !self.ui.lock_ratio;
                    }
                }
                caption(ui, label);
                let mut value = values.and_then(|v| v.get(n).copied()).unwrap_or(0.0);
                let r = ui.add_enabled(enabled, egui::DragValue::new(&mut value).speed(0.1).max_decimals(3).suffix(*unit));
                self.labeled(&r, egui::WidgetType::DragValue, label, false);
                let r = r.on_disabled_hover_text(reason);
                if r.changed() && value.is_finite() {
                    let mut params = serde_json::json!({ "anchor": self.ui.anchor, "lockRatio": self.ui.lock_ratio, "coalesce": format!("numeric-{key}") });
                    let (key, value) = match *key {
                        "wPct" => ("w", values.and_then(|v| v.get(2).copied()).unwrap_or(0.0) * value / 100.0),
                        "hPct" => ("h", values.and_then(|v| v.get(3).copied()).unwrap_or(0.0) * value / 100.0),
                        other => (other, value),
                    };
                    if let Some(map) = params.as_object_mut() {
                        map.insert(key.to_owned(), serde_json::json!(value));
                    }
                    self.execute_widget("arrange.transform", params);
                }
            }
        });
    }

    // ---- left tool palette ------------------------------------------------------------------------------------------

    pub(crate) fn tool_palette(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            for (n, (label, id, icon)) in LEFT_TOOLS.iter().enumerate() {
                if n == 8 {
                    ui.separator();
                }
                let live = *id == "tools.select" || command_live(id);
                let r = ui.add_enabled(live, egui::Button::image(icon_image(icon)).selected(self.ui.selected_tool == *label).min_size(BUTTON));
                self.labeled(&r, egui::WidgetType::Button, label, false);
                let r = if live { r.on_hover_text(*label) } else { r.on_disabled_hover_text(format!("{label}: {NOT_YET}")) };
                if r.clicked() {
                    if *id == "tools.select" {
                        self.invoke_menu_command(ui.ctx(), id);
                    } else {
                        self.ui.selected_tool = label;
                        self.frame_synthetic = self.services.human.frame_is_synthetic(ui.ctx());
                        self.execute_widget(id, serde_json::Value::Null);
                    }
                }
            }
        });
    }

    // ---- Arrange toolbar and colour palette ------------------------------------------------------------------------

    pub(crate) fn arrange_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (label, id, icon) in ARRANGE_TOOLS {
                let live = command_live(id);
                let r = ui.add_enabled(live && !self.ui.machine_active, egui::Button::image(icon_image(icon)).min_size(BUTTON));
                self.labeled(&r, egui::WidgetType::Button, label, false);
                let r = if live {
                    r.on_hover_text(*label).on_disabled_hover_text("Edits are locked while a machine is active.")
                } else {
                    r.on_disabled_hover_text(format!("{label}: {NOT_YET}"))
                };
                if r.clicked() {
                    self.execute_widget(id, serde_json::Value::Null);
                }
            }
        });
    }

    /// 30 colour swatches 00–29 plus T1/T2, sized to fit the window (at 760 pt too).
    pub(crate) fn palette_bar(&mut self, ui: &mut egui::Ui) {
        let gap = 1.0;
        let width = ((ui.available_width() - gap * 31.0) / 32.0).floor().clamp(12.0, 44.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for slot in 0..32 {
                let Some(key) = slot_key(slot) else { continue };
                let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 22.0), egui::Sense::click());
                let fill = palette_color(key);
                let selected = self.ui.active_layer == key;
                ui.painter().rect_filled(rect, 2.0, fill);
                if selected {
                    ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(2.0, ui.visuals().selection.stroke.color), egui::StrokeKind::Inside);
                }
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, swatch_text(key), egui::FontId::proportional(10.0), text_on(fill));
                let label = swatch_label(key);
                self.labeled(&response, egui::WidgetType::Button, &label, false);
                let hover = format!("{label} · {}", self.layer_summary(key));
                let response = response.on_hover_text(hover);
                if response.clicked() {
                    self.palette_click(key);
                }
            }
        });
    }

    /// With shapes selected: move them onto the layer (`layer.assign`); otherwise make it current (`layer.setCurrent`).
    pub(crate) fn palette_click(&mut self, key: LayerKey) {
        self.ui.active_layer = key;
        let selection = self.selection();
        if selection.is_empty() {
            self.execute_widget("layer.setCurrent", serde_json::json!({ "layer": layer_param(key) }));
        } else {
            self.execute_widget("layer.assign", serde_json::json!({ "layer": layer_param(key), "ids": selection }));
        }
    }

    // ---- status bar ----------------------------------------------------------------------------------------------------

    pub(crate) fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(self.ui.pointer_mm.map(|[x, y]| format!("X {x:.2}  Y {y:.2} mm")).unwrap_or_else(|| "X —  Y — mm".into()));
            ui.separator();
            let selection = self.selection();
            let summary = match self.project().and_then(|p| p.bounds(&selection)) {
                Some(b) if !selection.is_empty() => format!("{} selected · {:.2} × {:.2} mm", selection.len(), b.max[0] - b.min[0], b.max[1] - b.min[1]),
                _ => format!("{} shapes", self.project().map(|p| p.shapes.len()).unwrap_or(0)),
            };
            ui.label(summary);
            ui.separator();
            let (chip, color) = self.chip();
            ui.colored_label(color, chip);
            if let Some(summary) = self.plan().and_then(PlanView::summary) {
                ui.separator();
                ui.label(format!("est {}", mmss(summary.estimate)));
            }
            let refusals = self.plan().map(|p| p.refusals().len()).unwrap_or(0);
            if refusals > 0 {
                ui.separator();
                let r = ui.add(egui::Button::new(egui::RichText::new(format!("{refusals} refusal(s)")).color(egui::Color32::WHITE)).fill(STOP_RED).small());
                self.labeled(&r, egui::WidgetType::Button, "Refusals", false);
                if r.on_hover_text("Show the refusals in the Laser panel").clicked() {
                    let _ = self.show_panel(&serde_json::json!({ "panel": "laser", "visible": true }));
                }
            }
            ui.separator();
            ui.label(&self.ui.status_message);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let agent = match self.services.automation.info() {
                    None => "agent: none".to_owned(),
                    Some(info) if info.clients.is_empty() => format!("agent: listening on {}", info.port),
                    Some(info) => format!("agent: connected ({})", info.clients.len()),
                };
                ui.label(agent);
            });
        });
    }

    // ---- dock ------------------------------------------------------------------------------------------------------------

    pub(crate) fn dock_area(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.dock.clone();
        let style = craft_ui::docking::DockStyle::from_ui(ui);
        let blocked: Vec<egui::Rect> = self.stop_overlay_rect.into_iter().collect();
        let output = craft_ui::docking::DockArea::new(egui::Id::new("editor-dock")).blocked_rects(&blocked).show(
            ui,
            &snapshot,
            &style,
            |panel| crate::dock::label(panel),
            crate::dock::permissions,
            |ui, panel| match panel.as_str() {
                "canvas" => self.paint_canvas(ui),
                "cuts" => {
                    egui::ScrollArea::vertical().id_salt("cuts-scroll").show(ui, |ui| self.cuts_panel(ui));
                }
                "laser" => {
                    egui::ScrollArea::vertical().id_salt("laser-scroll").show(ui, |ui| self.laser_panel(ui));
                }
                "console" => self.console_panel(ui),
                "library" => {
                    ui.label("Material Library: presets by material and thickness arrive in P1 (CAT §5.12).");
                }
                "move" => {
                    ui.label(if self.active_is_galvo() {
                        "Move (jog) is for gantry machines. On this galvo, use Live Framing to see where the job lands."
                    } else {
                        "Jogging a gantry arrives with live GRBL streaming (M4); G-code export works today."
                    });
                }
                _ => {
                    ui.label(crate::dock::label(panel));
                }
            },
        );
        for action in output.actions {
            if let Err(error) = self.dock.apply_with_permissions(action, crate::dock::permissions) {
                self.set_status(error.to_string());
            }
        }
    }

    // ---- Cuts / Layers --------------------------------------------------------------------------------------------------

    pub(crate) fn cuts_panel(&mut self, ui: &mut egui::Ui) {
        let used = self.project().map(used_layers).unwrap_or_default();
        let gantry = !self.active_is_galvo();
        let mut columns = vec!["#", "Layer", "Mode", "Spd/Pwr", "Output", "Show"];
        if gantry {
            columns.push("Air");
        }
        egui::Grid::new("cuts-table").striped(true).num_columns(columns.len()).spacing([8.0, 4.0]).show(ui, |ui| {
            for heading in &columns {
                ui.strong(*heading);
            }
            ui.end_row();
            for key in &used {
                self.cuts_row(ui, *key, gantry);
                ui.end_row();
            }
        });
        if used.is_empty() {
            ui.weak("No layers in use. Draw or import shapes, or pick a colour in the palette below.");
        }
        ui.separator();
        self.quick_editor(ui);
    }

    fn cuts_row(&mut self, ui: &mut egui::Ui, key: LayerKey, gantry: bool) {
        let name = self.layer_name(key);
        let fill = palette_color(key);
        let number = ui.add(
            egui::Button::new(egui::RichText::new(swatch_text(key)).color(text_on(fill)).strong())
                .fill(fill)
                .min_size(egui::vec2(30.0, 24.0))
                .selected(self.ui.active_layer == key),
        );
        self.labeled(&number, egui::WidgetType::Button, &format!("Cut layer {name}"), false);
        let refusals: Vec<String> = self
            .plan()
            .map(|p| p.refusals().iter().filter(|r| r.layer == Some(key)).map(|r| format!("{}: {}", r.code.as_str(), r.message)).collect())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(&name);
            if !refusals.is_empty() {
                let badge = ui.add(egui::Button::new(egui::RichText::new(format!("⚠ {}", refusals.len())).color(egui::Color32::WHITE)).fill(STOP_RED).small());
                self.labeled(&badge, egui::WidgetType::Button, &format!("Refusals {name}"), false);
                badge.on_hover_text(refusals.join("\n"));
            }
        });
        let (mode, spd_pwr, detail) = self.layer_columns(key);
        ui.label(mode);
        ui.label(spd_pwr).on_hover_text(detail);
        let (mut output, mut show, mut air) = self.layer_flags(key);
        let tool = key.is_tool();
        let out = ui.add_enabled(!tool && !self.ui.machine_active, egui::Checkbox::without_text(&mut output));
        self.labeled(&out, egui::WidgetType::Checkbox, &format!("Output {name}"), false);
        let out = out.on_disabled_hover_text(if tool { "Tool layers (T1/T2) never output." } else { "Locked while a machine is active." });
        if out.changed() {
            self.execute_widget("layer.setOutput", serde_json::json!({ "layer": layer_param(key), "on": output }));
        }
        let vis = ui.add_enabled(!self.ui.machine_active, egui::Checkbox::without_text(&mut show));
        self.labeled(&vis, egui::WidgetType::Checkbox, &format!("Show {name}"), false);
        if vis.changed() {
            self.execute_widget("layer.setShow", serde_json::json!({ "layer": layer_param(key), "on": show }));
        }
        if gantry {
            let a = ui.add_enabled(!tool && !self.ui.machine_active, egui::Checkbox::without_text(&mut air));
            self.labeled(&a, egui::WidgetType::Checkbox, &format!("Air {name}"), false);
            if a.changed() {
                self.execute_widget("layer.setAir", serde_json::json!({ "layer": layer_param(key), "on": air }));
            }
        }
        if number.clicked() {
            self.ui.active_layer = key;
            self.execute_widget("layer.select", serde_json::json!({ "layer": layer_param(key) }));
            let now = ui.input(|i| i.time);
            let double = number.double_clicked() || self.last_layer_click.is_some_and(|(k, at)| k == key && now - at <= 0.4);
            if double && !tool {
                self.ui.cut_settings_open = true;
                self.cut_draft = None;
                self.last_layer_click = None;
            } else {
                self.last_layer_click = Some((key, now));
            }
        }
    }

    pub(crate) fn layer_name(&self, key: LayerKey) -> String {
        let project = self.project();
        match key {
            LayerKey::Color(i) => project.and_then(|p| p.color_layer(i)).map(|l| l.name.clone()).unwrap_or_else(|| key.to_string()),
            LayerKey::Tool(t) => {
                project.and_then(|p| p.layers.tools.get(usize::from(t.saturating_sub(1)))).map(|l| l.name.clone()).unwrap_or_else(|| key.to_string())
            }
        }
    }

    fn layer_flags(&self, key: LayerKey) -> (bool, bool, bool) {
        let project = self.project();
        match key {
            LayerKey::Color(i) => project.and_then(|p| p.color_layer(i)).map(|l| (l.output, l.show, l.air_assist)).unwrap_or((false, true, false)),
            LayerKey::Tool(t) => (false, project.and_then(|p| p.layers.tools.get(usize::from(t.saturating_sub(1)))).is_none_or(|l| l.show), false),
        }
    }

    /// The actual (decoded) speed of a layer's first pass from the plan, if the job plans (PLAN D9).
    pub(crate) fn actual_speed(&self, key: LayerKey) -> Option<f64> {
        self.plan().and_then(PlanView::summary).and_then(|s| s.rows.iter().find(|r| r.layer == key)).map(|r| r.speed_mm_s)
    }

    /// (Mode, Spd/Pwr cell, hover detail). The speed is the ACTUAL encoded one, labelled so, when the job plans;
    /// otherwise the requested speed, never called actual.
    fn layer_columns(&self, key: LayerKey) -> (String, String, String) {
        let LayerKey::Color(i) = key else {
            return ("Tool".into(), "—".into(), "Tool layers trace and guide; they never burn.".into());
        };
        let Some(op) = self.project().and_then(|p| p.color_layer(i)).and_then(|l| l.ops.first()) else {
            return ("—".into(), "—".into(), String::new());
        };
        let mode = match op.mode {
            openlaser_doc::CutMode::Line => "Line",
            openlaser_doc::CutMode::Fill => "Fill",
            openlaser_doc::CutMode::OffsetFill => "Offset Fill",
        };
        let gantry = !self.active_is_galvo();
        let (requested, power) = if gantry { (op.gantry.speed.get(), op.gantry.power_max.get()) } else { (op.galvo.speed.get(), op.galvo.power.get()) };
        let khz = f64::from(op.galvo.frequency.get()) / 1000.0;
        let ns = op.galvo.q_pulse.map(|n| format!("{} ns", n.get())).unwrap_or_else(|| "—".into());
        match self.actual_speed(key) {
            Some(actual) => (
                mode.into(),
                format!("{actual:.1} / {power:.0}"),
                format!("Speed {actual:.2} mm/s (actual, decoded from the bytes) · requested {requested} mm/s · power {power} % · {khz} kHz · {ns}"),
            ),
            None => (
                mode.into(),
                format!("{requested} / {power:.0}"),
                format!("Speed {requested} mm/s (requested; the actual encoded speed shows once the job plans) · power {power} % · {khz} kHz · {ns}"),
            ),
        }
    }

    pub(crate) fn layer_summary(&self, key: LayerKey) -> String {
        let (mode, spd_pwr, _) = self.layer_columns(key);
        format!("{} · {mode} · {spd_pwr}", self.layer_name(key))
    }

    fn quick_editor(&mut self, ui: &mut egui::Ui) {
        let key = self.ui.active_layer;
        let name = self.layer_name(key);
        ui.strong(format!("Layer {name}"));
        let LayerKey::Color(i) = key else {
            ui.label("Tool layer: never output. Live Framing can trace it (Tool Layers Only).");
            return;
        };
        let Some(op) = self.project().and_then(|p| p.color_layer(i)).and_then(|l| l.ops.first()).cloned() else {
            ui.weak("This layer has no cut settings yet.");
            return;
        };
        let speed = match self.actual_speed(key) {
            Some(actual) => format!("Speed: {} mm/s requested · {actual:.2} mm/s (actual)", op.galvo.speed.get()),
            None => format!("Speed: {} mm/s requested", op.galvo.speed.get()),
        };
        ui.label(speed);
        ui.label(format!(
            "Power {} % · Frequency {} kHz · Q-Pulse {} · Passes {}",
            op.galvo.power.get(),
            f64::from(op.galvo.frequency.get()) / 1000.0,
            op.galvo.q_pulse.map(|n| format!("{} ns", n.get())).unwrap_or_else(|| "—".into()),
            op.passes
        ));
        if op.mode != openlaser_doc::CutMode::Line {
            ui.label(format!(
                "Interval {} mm · Angle {}°{}",
                op.fill.interval.get(),
                op.fill.angle.get(),
                if op.fill.cross_hatch { " · Cross-Hatch" } else { "" }
            ));
        }
        let edit = ui.add(egui::Button::new("Edit Cut Settings…"));
        self.track(&edit, "button", "Edit Cut Settings…", false);
        if edit.clicked() {
            self.ui.cut_settings_open = true;
            self.cut_draft = None;
        }
    }

    // ---- Laser panel ------------------------------------------------------------------------------------------------------

    pub(crate) fn laser_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let active = self.ui.machine_active;
        let galvo = self.active_is_galvo();
        let busy = self.is_machine_busy();

        // 1. Device row.
        ui.horizontal(|ui| {
            let devices = ui.button("Devices…");
            self.track(&devices, "button", "Devices…", false);
            if devices.on_hover_text("Device Settings for the selected machine").clicked() {
                self.ui.device_settings_open = true;
            }
            let machines: Vec<(openlaser_types::MachineId, String)> = self.session.machines().iter().map(|m| (m.id.clone(), m.name.clone())).collect();
            let current = self.active_machine().map(|m| m.id.clone());
            let mut chosen = current.clone();
            let combo = ui
                .add_enabled_ui(!busy, |ui| {
                    egui::ComboBox::from_label("Device")
                        .selected_text(machines.iter().find(|(id, _)| Some(id) == current.as_ref()).map(|(_, n)| n.as_str()).unwrap_or("No device"))
                        .show_ui(ui, |ui| {
                            for (id, name) in &machines {
                                ui.selectable_value(&mut chosen, Some(id.clone()), name);
                            }
                        })
                        .response
                })
                .inner;
            self.track(&combo, "comboBox", "Device", false);
            combo.on_disabled_hover_text("Stop the machine before switching devices.");
            if chosen != current
                && let Some(id) = chosen
            {
                self.execute_widget("device.select", serde_json::json!({ "device": id.0 }));
            }
        });
        ui.horizontal(|ui| {
            let (text, color) = self.chip();
            ui.colored_label(color, text);
            self.human_button(ui, &ctx, "Connect", if busy { Err("Stop the machine first.".to_owned()) } else { Ok(()) }, |app| {
                let id = app.active_machine().map(|m| m.id.clone()).ok_or(openlaser_engine::EngineError::NoDevice)?;
                app.session.connect(openlaser_engine::Origin::Human, &id)
            });
        });

        // 2. Job buttons: Pause, Stop, Start; Frame, Send, Home, Go to Origin.
        let plan = self.plan().cloned();
        let receipt = plan.as_ref().and_then(PlanView::summary).and_then(|s| self.receipt_for(&s.machine));
        let start = start_gate(plan.as_ref(), receipt.as_ref(), self.session.id(), self.services.clock.now(), active);
        let frame = frame_gate(plan.as_ref(), active);
        ui.horizontal(|ui| {
            self.disabled_button(ui, "Pause", if galvo { "Pause is not available on galvo machines in this build (H15). Use Stop." } else { NOT_YET });
            let stop = ui.add(egui::Button::new("Stop").min_size(egui::vec2(56.0, 24.0)));
            self.track(&stop, "button", "Stop", false);
            if stop.on_hover_text("Stop laser: latches every machine at once").clicked() {
                self.stop();
            }
            self.human_button(ui, &ctx, "Start", start.clone(), |app| app.session.request_arm(openlaser_engine::Origin::Human).map(|_| ()));
        });
        ui.horizontal(|ui| {
            let (style, source) = (self.ui.live_style, self.ui.live_source);
            self.human_button(ui, &ctx, "Frame", frame, move |app| {
                app.ui.live_framing_open = true;
                app.session.start_frame(openlaser_engine::Origin::Human, openlaser_types::FrameMode::Once, Some(style), Some(source)).map(|_| ())
            });
            self.disabled_button(ui, "Send", "Not available for this device: Send uploads to DSP controllers only.");
            self.disabled_button(ui, "Home", if galvo { "A galvo has no homing: its origin is the field centre." } else { NOT_YET });
            self.disabled_button(ui, "Go to Origin", if galvo { "A galvo has no head to move: its origin is the field centre." } else { NOT_YET });
        });

        // 3. Start From and Job Origin (galvos always use absolute coordinates, CTL-03).
        ui.horizontal(|ui| {
            caption(ui, "Start From");
            let r = ui.add_enabled(false, egui::Button::new("Absolute Coords"));
            self.labeled(&r, egui::WidgetType::ComboBox, "Start From", false);
            r.on_disabled_hover_text(if galvo { "Galvos always use absolute coordinates: the field centre is the origin." } else { NOT_YET });
            let r = ui.add_enabled(false, egui::Button::new("⋮⋮⋮"));
            self.labeled(&r, egui::WidgetType::Button, "Job Origin", false);
            r.on_disabled_hover_text("Job Origin applies to User Origin and Current Position only.");
        });

        // 4. Options.
        let has_doc = self.project().is_some();
        let mut cut_selected = self.project().is_some_and(|p| p.job.cut_selected);
        let r = ui.add_enabled(has_doc && !active, egui::Checkbox::new(&mut cut_selected, "Cut Selected Graphics"));
        self.track(&r, "checkBox", "Cut Selected Graphics", false);
        if r.changed() {
            self.execute_widget("laser.cutSelected", serde_json::json!({ "on": cut_selected }));
        }
        let mut use_origin = false;
        let r = ui.add_enabled(false, egui::Checkbox::new(&mut use_origin, "Use Selection Origin"));
        self.track(&r, "checkBox", "Use Selection Origin", false);
        r.on_disabled_hover_text(if galvo { "Not used on galvos: the field centre is the origin." } else { NOT_YET });
        ui.horizontal(|ui| {
            caption(ui, "Repeat");
            let mut repeats = self.project().map(|p| p.job.repeats).unwrap_or(1);
            let r = ui.add_enabled(has_doc && !active, egui::DragValue::new(&mut repeats).range(1..=999));
            self.labeled(&r, egui::WidgetType::DragValue, "Repeat count", false);
            if r.changed() {
                self.execute_widget("laser.repeat", serde_json::json!({ "count": repeats }));
            }
        });

        // 5. Estimate; 6. refusals and notices.
        ui.separator();
        match &plan {
            None => {
                ui.weak("No current job. Open, import or draw a design.");
            }
            Some(PlanView::Ready(summary)) => {
                ui.label(format!(
                    "Estimate {} · {} pass row(s) · {} packet(s) · crash bound {:.1} s",
                    mmss(summary.estimate),
                    summary.rows.len(),
                    summary.packets,
                    summary.crash_bound.as_secs_f64()
                ));
                if receipt.is_none() {
                    ui.label("Frame this job first.");
                }
            }
            Some(PlanView::Refused(refused)) => {
                ui.colored_label(STOP_RED, format!("{} refusal(s): this job will not run", refused.refusals.len()));
                for r in &refused.refusals {
                    ui.label(format!("• {}: {}", r.code.as_str(), r.message));
                }
                if let Some(error) = &refused.error {
                    ui.label(error);
                }
            }
        }
        if let Some(plan) = &plan {
            for notice in plan.notices() {
                ui.label(format!("{}. {}", notice.n, notice.message));
            }
        }
        if let Err(reason) = &start {
            ui.weak(format!("Start: {reason}"));
        }
        ui.separator();
        ui.weak("STOP works while this window is focused; the E-stop always works.");
        self.request_cards(ui, &ctx);
    }

    fn disabled_button(&mut self, ui: &mut egui::Ui, label: &str, reason: &str) {
        let r = ui.add_enabled(false, egui::Button::new(label));
        self.track(&r, "button", label, false);
        r.on_disabled_hover_text(reason);
    }

    /// A human-only widget (Connect, Frame, Start): enabled when `gate` allows it; acts only on a real click in an
    /// untainted frame (`services.human.is_human_click`, asked every frame so it can time the widget's visibility).
    fn human_button(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        label: &'static str,
        gate: Result<(), String>,
        action: impl FnOnce(&mut Self) -> Result<(), openlaser_engine::EngineError>,
    ) {
        let r = ui.add_enabled(gate.is_ok(), egui::Button::new(label).min_size(egui::vec2(56.0, 24.0)));
        self.track(&r, "button", label, true);
        let human = self.services.human.is_human_click(ctx, &r);
        let synthetic = self.frame_synthetic || self.services.human.frame_is_synthetic(ctx);
        if human && !synthetic && gate.is_ok() {
            #[cfg(test)]
            self.seam.human_actions.push(label);
            let message = match action(self) {
                Ok(()) => format!("{label}: requested"),
                Err(e) => e.to_string(),
            };
            self.set_status(message);
        } else if r.clicked() {
            self.set_status(format!("{label} is human-only: it needs a person's click in this window (automation input is ignored)."));
        }
        if let Err(reason) = gate {
            r.on_disabled_hover_text(reason);
        }
    }

    fn request_cards(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut cards: Vec<(openlaser_engine::RequestId, String, String)> = self
            .session
            .requests()
            .submissions()
            .into_iter()
            .filter(|r| r.state == openlaser_engine::SubmissionState::AwaitingHuman)
            .map(|r| (r.submission, format!("Job from {}", r.client), format!("Note from agent: {} · est {}", r.note, mmss(r.summary.estimate))))
            .collect();
        cards.extend(
            self.session
                .requests()
                .frame_requests()
                .into_iter()
                .filter(|r| r.state == openlaser_engine::FrameRequestState::AwaitingHuman)
                .map(|r| (r.request, format!("Frame request from {}", r.client), format!("{:?} framing", r.mode))),
        );
        for (id, title, detail) in cards {
            self.request_card(ui, ctx, id, &title, &detail);
        }
    }

    pub(crate) fn request_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, id: openlaser_engine::RequestId, title: &str, detail: &str) {
        ui.group(|ui| {
            ui.strong(title);
            ui.label(detail);
            ui.horizontal(|ui| {
                for (label, approve) in [("Approve framing", true), ("Reject request", false)] {
                    let r = ui.add_enabled(!self.ui.machine_active, egui::Button::new(label));
                    self.track(&r, "button", label, true);
                    let human = self.services.human.is_human_click(ctx, &r);
                    if human && !self.frame_synthetic && !self.services.human.frame_is_synthetic(ctx) {
                        #[cfg(test)]
                        self.seam.human_actions.push(label);
                        let result = if approve {
                            self.session.approve_request(openlaser_engine::Origin::Human, id)
                        } else {
                            self.session.decline_request(openlaser_engine::Origin::Human, id)
                        };
                        let message = match result {
                            Ok(()) => format!("{label}: done"),
                            Err(e) => e.to_string(),
                        };
                        self.set_status(message);
                    } else if r.clicked() {
                        self.set_status(format!("{label} is human-only: it needs a person's click in this window."));
                    }
                }
            });
        });
    }

    // ---- Console ---------------------------------------------------------------------------------------------------------

    fn console_panel(&mut self, ui: &mut egui::Ui) {
        ui.strong("Log");
        egui::ScrollArea::vertical().id_salt("console-log").max_height(ui.available_height() * 0.5).stick_to_bottom(true).show(ui, |ui| {
            for line in &self.log {
                ui.monospace(line);
            }
        });
        ui.separator();
        ui.strong("USB trace (decoded)");
        let lines: Vec<String> = self
            .session
            .machines()
            .iter()
            .filter_map(|m| self.session.status(&m.id))
            .flat_map(|s| s.trace_tail.iter().map(|t| format!("{t:?}")).collect::<Vec<_>>())
            .collect();
        egui::ScrollArea::vertical().id_salt("console-trace").stick_to_bottom(true).show(ui, |ui| {
            if lines.is_empty() {
                ui.weak("No device traffic.");
            }
            for line in lines {
                ui.monospace(line);
            }
        });
    }

    // ---- STOP overlay ----------------------------------------------------------------------------------------------------

    /// While a machine is active, a human-only surface is open, or any modal is up, STOP floats above everything.
    pub(crate) fn stop_overlay_needed(&self, ctx: &egui::Context) -> bool {
        self.safety_active() || self.human_surface_open() || self.ui.cut_settings_open || ctx.memory(|m| m.top_modal_layer().is_some())
    }

    /// STOP at `Order::Tooltip`, drawn last, above every Modal (UI §7.1); never disabled.
    pub(crate) fn stop_overlay(&mut self, ctx: &egui::Context) {
        if !self.stop_overlay_needed(ctx) {
            self.stop_overlay_rect = None;
            return;
        }
        let area = egui::Area::new(egui::Id::new("stop_overlay")).order(egui::Order::Tooltip).anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 72.0)).show(
            ctx,
            |ui| {
                let button = egui::Button::new(egui::RichText::new("Stop laser").strong().size(16.0).color(egui::Color32::WHITE))
                    .fill(STOP_RED)
                    .min_size(egui::vec2(120.0, 40.0));
                let r = ui.add(button);
                self.labeled(&r, egui::WidgetType::Button, "Stop laser", false);
                if r.clicked() {
                    self.stop();
                }
            },
        );
        self.stop_overlay_rect = Some(area.response.rect);
    }
}

pub(crate) fn mmss(d: std::time::Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> EmissionSummary {
        EmissionSummary {
            fingerprint: openlaser_types::Fingerprint([7; 32]),
            bytes_hash: openlaser_types::Fingerprint([8; 32]),
            machine: openlaser_types::MachineId("jpt-m7-200-sim".into()),
            rows: Vec::new(),
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

    pub(crate) fn receipt(summary: &EmissionSummary, at: MonoTime) -> FrameReceipt {
        FrameReceipt {
            fingerprint: summary.fingerprint,
            machine: summary.machine.clone(),
            session: SessionId(1),
            full_passes: 1,
            at,
            style: openlaser_types::FrameStyle::Contour,
            source: openlaser_types::FrameSource::Job,
            origin: FrameOrigin::Human,
        }
    }

    #[test]
    fn start_needs_a_fresh_human_receipt_of_the_same_plan() {
        let s = summary();
        let plan = PlanView::Ready(Box::new(s.clone()));
        let now = MonoTime(std::time::Duration::from_secs(1000));
        let good = receipt(&s, MonoTime(std::time::Duration::from_secs(990)));
        assert_eq!(start_gate(Some(&plan), Some(&good), SessionId(1), now, false), Ok(()));
        let frame_first = Err("Frame this job first. A human must approve every marking job.".to_owned());
        assert_eq!(start_gate(Some(&plan), None, SessionId(1), now, false), frame_first);
        let other = FrameReceipt { fingerprint: openlaser_types::Fingerprint([9; 32]), ..good.clone() };
        assert_eq!(start_gate(Some(&plan), Some(&other), SessionId(1), now, false), frame_first);
        assert_eq!(start_gate(Some(&plan), Some(&good), SessionId(2), now, false), frame_first);
        let agent = FrameReceipt { origin: FrameOrigin::Agent, ..good.clone() };
        assert_eq!(start_gate(Some(&plan), Some(&agent), SessionId(1), now, false), frame_first);
        let tools = FrameReceipt { source: openlaser_types::FrameSource::ToolLayersOnly, ..good.clone() };
        assert_eq!(start_gate(Some(&plan), Some(&tools), SessionId(1), now, false), frame_first);
        let unexecuted = FrameReceipt { full_passes: 0, ..good.clone() };
        assert!(start_gate(Some(&plan), Some(&unexecuted), SessionId(1), now, false).unwrap_err().contains("no full frame pass"));
        let old = FrameReceipt { at: MonoTime(std::time::Duration::from_secs(300)), ..good.clone() };
        assert!(start_gate(Some(&plan), Some(&old), SessionId(1), now, false).unwrap_err().contains("older than 10 minutes"));
        assert!(start_gate(Some(&plan), Some(&good), SessionId(1), now, true).unwrap_err().contains("busy"));
        assert!(start_gate(None, Some(&good), SessionId(1), now, false).is_err());
        let refused = PlanView::Refused(Box::new(openlaser_engine::JobRefused {
            fingerprint: None,
            refusals: vec![openlaser_types::Refusal::new(openlaser_types::RefusalCode::PulseNotInTable, "pulse 7 ns is not in the pulse table")],
            notices: Vec::new(),
            error: None,
        }));
        assert!(start_gate(Some(&refused), Some(&good), SessionId(1), now, false).unwrap_err().contains("1 refusal"));
        assert!(frame_gate(Some(&refused), false).is_err());
        assert_eq!(frame_gate(Some(&plan), false), Ok(()));
        assert!(frame_gate(Some(&plan), true).is_err());
    }

    #[test]
    fn slots_labels_and_contrast() {
        assert_eq!(slot_key(0), Some(LayerKey::Color(0)));
        assert_eq!(slot_key(29), Some(LayerKey::Color(29)));
        assert_eq!(slot_key(30), Some(LayerKey::Tool(1)));
        assert_eq!(slot_key(31), Some(LayerKey::Tool(2)));
        assert_eq!(slot_key(32), None);
        assert_eq!(swatch_label(LayerKey::Color(5)), "Layer 05");
        assert_eq!(swatch_label(LayerKey::Tool(2)), "T2");
        assert_eq!(layer_param(LayerKey::Color(5)), serde_json::json!("C05"));
        assert_eq!(text_on(egui::Color32::BLACK), egui::Color32::WHITE);
        assert_eq!(text_on(egui::Color32::from_rgb(255, 255, 0)), egui::Color32::BLACK);
        for slot in 0..32 {
            let key = slot_key(slot).unwrap();
            let rgb = if slot < 30 { openlaser_doc::PALETTE[slot] } else { openlaser_doc::TOOL_COLORS[slot - 30] };
            assert_eq!(palette_color(key), egui::Color32::from_rgb(rgb.0[0], rgb.0[1], rgb.0[2]), "slot {slot}");
        }
    }
}
