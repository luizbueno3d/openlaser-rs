//! The regions of LightBurn's layout (UI §4.1): menu bar, main toolbar with the machine chip and STOP, numeric edits,
//! the left tool palette, the dock (Cuts / Layers above Laser / Library / Move / Console), the Arrange toolbar and
//! colour palette, the status bar, and the STOP overlay.

use openlaser_engine::{EmissionSummary, MachineState};
use openlaser_types::{FrameOrigin, FrameReceipt, LayerKey, MonoTime, SessionId};

use crate::app::{OpenLaserApp, PlanView, state_requires_stop};
use crate::i18n::t;
use crate::menu_catalog::{self, CATALOG, MENUS};
use crate::tf;

/// STOP's colours are fixed and theme-independent (UI §9.1): white on saturated red.
pub(crate) const STOP_RED: egui::Color32 = egui::Color32::from_rgb(196, 18, 28);
/// The chip and notice of a real machine driven by the simulator (violet: no machine state uses it).
pub(crate) const SIMULATOR_COLOR: egui::Color32 = egui::Color32::from_rgb(150, 90, 220);

pub(crate) fn not_yet() -> &'static str {
    t("Not in OpenLaser yet", "Ainda não disponível no OpenLaser")
}

pub(crate) fn files_while_active() -> &'static str {
    t(
        "Stop the active frame or job before opening, importing or saving files.",
        "Pare o enquadramento ou o trabalho ativo antes de abrir, importar ou salvar arquivos.",
    )
}

/// The Laser panel's notice list scrolls inside this height so it never pushes the rest of the panel away.
const NOTICES_MAX_HEIGHT: f32 = 160.0;

/// Lucide icons (ISC, `assets/icons`, ATTRIBUTION.md), recoloured white and tinted by egui.
/// A toolbar icon, tinted with the theme's text colour. The SVGs are rasterised white, and the tint multiplies,
/// so the icon matches the theme (untinted white icons look disabled on the light theme). A disabled button
/// still fades it.
pub(crate) fn icon_image(name: &str, ui: &egui::Ui) -> egui::Image<'static> {
    icon_image_white(name).tint(ui.visuals().widgets.inactive.fg_stroke.color)
}

fn icon_image_white(name: &str) -> egui::Image<'static> {
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

/// The swatch's visible name in the current language: "Layer 05" / "Camada 05", "T1".
fn swatch_title(key: LayerKey) -> String {
    match key {
        LayerKey::Color(i) => tf!("Layer {i:02}", "Camada {i:02}"),
        LayerKey::Tool(t) => format!("T{t}"),
    }
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
        return Err(t(
            "The machine is busy. Stop it, or wait for the frame or job to finish.",
            "A máquina está ocupada. Pare-a ou espere o enquadramento ou o trabalho terminar.",
        )
        .into());
    }
    let Some(receipt) = receipt.filter(|r| {
        r.fingerprint == summary.fingerprint && r.machine == summary.machine && r.session == session && r.origin == FrameOrigin::Human && r.covers_job()
    }) else {
        return Err(t(
            "Frame this job first. A human must approve every marking job.",
            "Enquadre este trabalho primeiro. Uma pessoa precisa aprovar todo trabalho de marcação.",
        )
        .into());
    };
    if receipt.full_passes == 0 {
        return Err(t(
            "Frame this job first: no full frame pass has been executed yet.",
            "Enquadre este trabalho primeiro: nenhuma passada completa de enquadramento foi feita ainda.",
        )
        .into());
    }
    if now.saturating_since(receipt.at) > openlaser_types::FRAME_RECEIPT_MAX_AGE {
        return Err(
            t("The frame is older than 10 minutes. Frame this job again.", "O enquadramento tem mais de 10 minutos. Enquadre este trabalho de novo.").into()
        );
    }
    Ok(())
}

/// Frame (human-only) needs a plan without refusals and an idle machine.
pub(crate) fn frame_gate(plan: Option<&PlanView>, active: bool) -> Result<(), String> {
    ready_summary(plan)?;
    if active {
        return Err(t("The machine is busy. Stop it first.", "A máquina está ocupada. Pare-a primeiro.").into());
    }
    Ok(())
}

fn ready_summary(plan: Option<&PlanView>) -> Result<&EmissionSummary, String> {
    match plan {
        None => Err(t("No job: open, import or draw a design first.", "Nenhum trabalho: abra, importe ou desenhe um projeto primeiro.").into()),
        Some(PlanView::Refused(refused)) if refused.refusals.is_empty() => {
            Err(refused.error.clone().unwrap_or_else(|| t("The job cannot be planned.", "O trabalho não pode ser planejado.").into()))
        }
        Some(PlanView::Refused(refused)) => Err(tf!(
            "{} refusal(s) block this job. Fix them first (see the list below).",
            "{} recusa(s) bloqueiam este trabalho. Corrija-as primeiro (veja a lista abaixo).",
            refused.refusals.len()
        )),
        Some(PlanView::Ready(summary)) => Ok(summary),
    }
}

/// The machine state chip (UI §4.3 / §4.9).
pub(crate) fn state_text(state: Option<&MachineState>, armed: bool) -> (String, egui::Color32) {
    if armed {
        return (t("Armed", "Armado").into(), egui::Color32::from_rgb(230, 140, 30));
    }
    match state {
        None | Some(MachineState::Disconnected) => (t("Disconnected", "Desconectado").into(), egui::Color32::GRAY),
        Some(MachineState::Connecting) => (t("Connecting…", "Conectando…").into(), egui::Color32::GRAY),
        Some(MachineState::Idle) => (t("Ready", "Pronto").into(), egui::Color32::from_rgb(60, 170, 90)),
        Some(MachineState::Framing) => (t("Framing", "Enquadrando").into(), egui::Color32::from_rgb(230, 80, 60)),
        Some(MachineState::Running { packets_acked, total }) => {
            let pct = if *total == 0 { 0 } else { u64::from(*packets_acked) * 100 / u64::from(*total) };
            (tf!("Marking {pct}%", "Marcando {pct}%"), egui::Color32::from_rgb(230, 60, 40))
        }
        Some(MachineState::Stopping) => (t("Stopping", "Parando").into(), egui::Color32::from_rgb(230, 140, 30)),
        Some(MachineState::Stopped) => (t("Stopped", "Parado").into(), egui::Color32::GRAY),
        Some(MachineState::Fault) => (t("Fault", "Falha").into(), egui::Color32::from_rgb(230, 60, 40)),
        Some(MachineState::StopNotConfirmed) => (t("STOP NOT CONFIRMED", "PARADA NÃO CONFIRMADA").into(), STOP_RED),
    }
}

/// (automation label, Portuguese text, command id, icon). The English label is also the English text.
const TOOLBAR_GROUPS: &[&[(&str, &str, &str, &str)]] = &[
    &[
        ("New", "Novo", "file.new", "file-plus"),
        ("Open", "Abrir", "file.open", "folder-open"),
        ("Import", "Importar", "file.import", "file-input"),
        ("Save", "Salvar", "file.save", "save"),
    ],
    &[("Undo", "Desfazer", "edit.undo", "undo-2"), ("Redo", "Refazer", "edit.redo", "redo-2")],
    &[
        ("Cut", "Recortar", "edit.cut", "scissors"),
        ("Copy", "Copiar", "edit.copy", "copy"),
        ("Paste", "Colar", "edit.paste", "clipboard-paste"),
        ("Delete", "Excluir", "edit.delete", "trash-2"),
    ],
    &[
        ("Pan / Drag View", "Mover / arrastar vista", "view.pan", "hand"),
        ("Zoom In", "Aumentar zoom", "view.zoomIn", "zoom-in"),
        ("Zoom Out", "Diminuir zoom", "view.zoomOut", "zoom-out"),
        ("Zoom to Page", "Zoom na página", "view.zoomPage", "maximize"),
        ("Frame Selection", "Zoom na seleção", "view.zoomSelection", "scan"),
    ],
    &[("Settings", "Configurações", "edit.settings", "settings"), ("Device Settings", "Configurações do dispositivo", "edit.deviceSettings", "settings-2")],
    &[
        ("Trace Image", "Vetorizar imagem", "tools.traceImage", "wand-sparkles"),
        ("Preview", "Pré-visualização", "preview.open", "eye"),
        ("Camera", "Câmera", "laserTools.cameras", "camera"),
    ],
];

const LEFT_TOOLS: &[(&str, &str, &str, &str)] = &[
    ("Select", "Selecionar", "tools.select", "mouse-pointer-2"),
    ("Draw Lines", "Desenhar linhas", "tools.drawLines", "pen-tool"),
    ("Rectangle", "Retângulo", "tools.rectangle", "rectangle-horizontal"),
    ("Ellipse", "Elipse", "tools.ellipse", "circle"),
    ("Polygon", "Polígono", "tools.polygon", "pentagon"),
    ("Edit Text", "Editar texto", "tools.text", "type"),
    ("Edit Nodes", "Editar nós", "tools.editNodes", "spline"),
    ("Measure", "Medir", "tools.measure", "ruler"),
    ("Offset Shapes", "Deslocar formas", "tools.offsetShapes", "layers"),
    ("Weld Shapes", "Soldar formas", "tools.weld", "link-2"),
];

const ARRANGE_TOOLS: &[(&str, &str, &str, &str)] = &[
    ("Group", "Agrupar", "arrange.group", "link-2"),
    ("Ungroup", "Desagrupar", "arrange.ungroup", "unlink"),
    ("Flip Horizontal", "Espelhar na horizontal", "arrange.flipH", "flip-horizontal-2"),
    ("Flip Vertical", "Espelhar na vertical", "arrange.flipV", "flip-vertical-2"),
    ("Rotate 90° Clockwise", "Girar 90° no sentido horário", "arrange.rotateCw90", "rotate-cw"),
    ("Rotate 90° Counter-Clockwise", "Girar 90° no sentido anti-horário", "arrange.rotateCcw90", "rotate-ccw"),
    ("Page Center", "Centro da página", "arrange.moveSelection.pageCenter", "move"),
];

const BUTTON: egui::Vec2 = egui::vec2(28.0, 26.0);

impl OpenLaserApp {
    // ---- menu bar (in-window) ---------------------------------------------------------------------------------------

    pub(crate) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            for menu in MENUS {
                let response = ui.menu_button(crate::i18n::menu(menu), |ui| self.menu_branch(ui, &[*menu])).response;
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
                    ui.menu_button(crate::i18n::menu(next), |ui| self.menu_branch(ui, &nested));
                }
            } else if menu_catalog::is_separator(row) {
                ui.separator();
            } else {
                let live = menu_catalog::is_live(row);
                let blocked = self.ui.machine_active && crate::app::opens_files(row.id);
                let no_image = row.id == "tools.traceImage" && !self.trace_target();
                let checked = (row.id == "view.cameraBackground" && self.ui.camera_background)
                    || (row.id == "prefs.language.en" && crate::i18n::language() == crate::i18n::Language::English)
                    || (row.id == "prefs.language.ptBr" && crate::i18n::language() == crate::i18n::Language::Portuguese);
                let label = crate::i18n::menu(row.label);
                let mut button = egui::Button::new(if checked { format!("✓ {label}") } else { label.to_owned() });
                if let Some(shortcut) = row.shortcut {
                    button = button.shortcut_text(crate::shortcuts::pretty(shortcut));
                }
                let response = ui.add_enabled(live && !blocked && !no_image, button);
                // The reason names the predicate that actually failed: `!live` before `blocked` (a row that
                // is not in OpenLaser yet is not "blocked by an active machine").
                let response = if no_image {
                    response.on_disabled_hover_text(t("Select one image to trace", "Selecione uma imagem para vetorizar"))
                } else if menu_catalog::is_hw_row(row) {
                    response.on_disabled_hover_text(t(
                        "Hardware actions are human-only and live in the Laser panel.",
                        "Ações de hardware só podem ser feitas por uma pessoa e ficam no painel Laser.",
                    ))
                } else if !live {
                    response.on_disabled_hover_text(not_yet())
                } else {
                    response.on_disabled_hover_text(files_while_active())
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
            let stop = ui.add(
                egui::Button::new(egui::RichText::new(t("STOP", "PARAR")).strong().color(egui::Color32::WHITE))
                    .fill(STOP_RED)
                    .min_size(egui::vec2(64.0, BUTTON.y)),
            );
            self.labeled(&stop, egui::WidgetType::Button, if overlay { "Stop laser (toolbar)" } else { "Stop laser" }, false);
            if stop
                .on_hover_text(t(
                    "Stop laser (Esc while active). The E-stop always works.",
                    "Parar o laser (Esc durante a operação). O botão de emergência sempre funciona.",
                ))
                .clicked()
            {
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
            for (label, pt, id, icon) in TOOLBAR_GROUPS.get(*index).copied().unwrap_or_default() {
                self.toolbar_button(ui, label, t(label, pt), id, icon);
            }
        }
        if !overflow.is_empty() {
            let response = ui
                .menu_button("»", |ui| {
                    for index in &overflow {
                        for (label, pt, id, _) in TOOLBAR_GROUPS.get(*index).copied().unwrap_or_default() {
                            let (enabled, reason) = self.toolbar_state(id);
                            let r = ui.add_enabled(enabled, egui::Button::new(t(label, pt)));
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
            (false, Some(not_yet()))
        } else if self.ui.machine_active && crate::app::opens_files(id) {
            (false, Some(files_while_active()))
        } else if id == "tools.traceImage" && !self.trace_target() {
            (false, Some(t("Select one image to trace", "Selecione uma imagem para vetorizar")))
        } else {
            (true, None)
        }
    }

    fn toolbar_button(&mut self, ui: &mut egui::Ui, label: &str, text: &str, id: &str, icon: &str) {
        let (enabled, reason) = self.toolbar_state(id);
        let selected = id == "view.pan" && self.ui.selected_tool == "Pan";
        let response = ui.add_enabled(enabled, egui::Button::image(icon_image(icon, ui)).selected(selected).min_size(BUTTON));
        self.labeled(&response, egui::WidgetType::Button, label, false);
        let response = match reason {
            Some(reason) => response.on_disabled_hover_text(format!("{text}: {reason}")),
            None => response.on_hover_text(text),
        };
        if response.clicked() {
            self.invoke_menu_command(ui.ctx(), id);
        }
    }

    /// "JPT M7 · Simulator: Ready".
    pub(crate) fn chip(&self) -> (String, egui::Color32) {
        let name = self.active_machine().map(|m| m.name.clone()).unwrap_or_else(|| t("No device", "Nenhum dispositivo").into());
        let states = self.machine_states();
        let state = self
            .active_machine()
            .and_then(|m| states.iter().find(|(id, _)| *id == m.id))
            .or_else(|| states.iter().find(|(_, s)| state_requires_stop(s)))
            .map(|(_, s)| s.clone());
        let (text, color) = state_text(state.as_ref(), self.arm_dialog_open());
        if self.simulating_real_machine() {
            // A real machine's profile driven by the simulator: never let it read as the laser (UI §4.3).
            return (tf!("{name} · SIMULATOR, no laser: {text}", "{name} · SIMULADOR, sem laser: {text}"), SIMULATOR_COLOR);
        }
        (format!("{name}: {text}"), color)
    }

    /// The Laser panel's foot-switch line (SPEC M3, UI "IO ports"): where it is read and what it reads now.
    fn foot_switch_row(&mut self, ui: &mut egui::Ui) {
        let Some(m) = self.active_machine() else { return };
        let openlaser_machine::Kinematics::Galvo(g) = &m.kind else { return };
        let status = self.session.status(&m.id);
        let text = match &g.ports.foot_switch {
            None => t(
                "Foot switch: not set. Measure its input with `openlaser-hw ports --watch`, then set it with `openlaser-hw profile set footSwitch.bit <bit>`.",
                "Pedal: não definido. Meça a entrada com `openlaser-hw ports --watch` e defina com `openlaser-hw profile set footSwitch.bit <bit>`.",
            )
            .to_owned(),
            Some(f) => {
                let at = tf!("word {} bit {}", "palavra {} bit {}", f.word, f.bit);
                let state = match status {
                    Some(s) if s.connected && s.pedal_blocked => {
                        t("blocked after a stop: click Frame or Start once", "bloqueado após uma parada: clique em Enquadrar ou Iniciar uma vez").to_owned()
                    }
                    Some(s) if s.connected => match s.pedal_down {
                        Some(true) => t("pressed", "pressionado").to_owned(),
                        Some(false) => t("released", "solto").to_owned(),
                        None => t("reading…", "lendo…").to_owned(),
                    },
                    _ => t("connect to read it", "conecte para ler").to_owned(),
                };
                tf!("Foot switch ({at}): {state}", "Pedal ({at}): {state}")
            }
        };
        let words = status.and_then(|s| s.inputs).map(|w| format!("ReadPort: {:04x} {:04x} {:04x} {:04x}", w[0], w[1], w[2], w[3]));
        let label = ui.label(text);
        let label = match words {
            Some(w) => label.on_hover_text(w),
            None => label,
        };
        self.track(&label, "status", "Foot switch", false);
    }

    /// The active machine's profile names a real link (USB, serial, network), but this window's device provider is
    /// the simulator: nothing it frames or marks reaches a laser.
    pub(crate) fn simulating_real_machine(&self) -> bool {
        use openlaser_machine::{GalvoLink, GantryLink, Kinematics};
        self.session.device_provider_name() == Some("sim")
            && self.active_machine().is_some_and(|m| match &m.kind {
                Kinematics::Galvo(g) => g.link != GalvoLink::Sim,
                Kinematics::Gantry(g) => !matches!(g.link, GantryLink::Sim | GantryLink::ExportOnly),
            })
    }

    // ---- numeric edits ------------------------------------------------------------------------------------------------

    /// LightBurn's text options row, shown while the Edit Text tool is armed or a text shape is selected:
    /// Font/Style/Height/Letter spacing/Line spacing/Align. Edits dispatch `text.edit` (coalesced per field) on
    /// a selected text; otherwise they set the defaults for the next text the tool creates.
    fn text_options(&mut self, ui: &mut egui::Ui) {
        let selected_text = self.selection().first().and_then(|id| {
            self.project().and_then(|p| p.shape(*id).and_then(|sh| if let openlaser_doc::Geom::Text(t) = &sh.geom { Some((*id, t.clone())) } else { None }))
        });
        let armed = self.ui.selected_tool == "Edit Text";
        if !armed && selected_text.is_none() {
            return;
        }
        // Seed the bar from the selected text or defaults.
        if let Some((_, t)) = &selected_text {
            let d = &mut self.ui.text_defaults;
            d.family = t.family.clone();
            d.style = t.style.clone();
            d.height = t.height_mm;
            d.letter_spacing = t.letter_spacing_mm;
            d.line_spacing = t.line_spacing;
            d.align = match t.align {
                openlaser_types::TextAlign::Left => 0,
                openlaser_types::TextAlign::Center => 1,
                openlaser_types::TextAlign::Right => 2,
            };
        }
        let enabled = !self.ui.machine_active;
        let id = selected_text.map(|(id, _)| id.0);
        ui.horizontal(|ui| {
            let fire = |app: &mut Self, key: &str| {
                if let Some(id) = id {
                    app.execute_widget("text.edit", app.ui.text_defaults.edit_params(id, &format!("text-{key}")));
                }
            };
            // Font: a searchable combo of the indexed families; Style: that family's styles.
            let families: Vec<(String, Vec<String>)> = self.session.fonts().families();
            caption(ui, t("Font", "Fonte"));
            let mut family = self.ui.text_defaults.family.clone();
            egui::ComboBox::from_id_salt("text-font").selected_text(if family.is_empty() { t("(default)", "(padrão)") } else { &family }).show_ui(ui, |ui| {
                for (f, _) in &families {
                    if ui.selectable_label(&family == f, f).clicked() {
                        family = f.clone();
                        let styles = families.iter().find(|(ff, _)| ff == &family).map(|(_, s)| s.clone()).unwrap_or_default();
                        self.ui.text_defaults.family = family.clone();
                        self.ui.text_defaults.style = styles.first().cloned().unwrap_or_default();
                        fire(self, "family");
                    }
                }
            });
            caption(ui, t("Style", "Estilo"));
            let mut style = self.ui.text_defaults.style.clone();
            egui::ComboBox::from_id_salt("text-style").selected_text(if style.is_empty() { t("(default)", "(padrão)") } else { &style }).show_ui(ui, |ui| {
                for st in families.iter().find(|(f, _)| f == &family).map(|(_, s)| s.clone()).unwrap_or_default() {
                    if ui.selectable_label(style == st, &st).clicked() {
                        style = st.clone();
                        self.ui.text_defaults.style = st;
                        fire(self, "style");
                    }
                }
            });
            for (label, pt, field, key) in [
                ("Height", "Altura", "height", "heightMm"),
                ("Letter spacing", "Espaço entre letras", "letter_spacing", "letterSpacingMm"),
                ("Line spacing", "Espaço entre linhas", "line_spacing", "lineSpacing"),
            ] {
                caption(ui, t(label, pt));
                let mut value = match field {
                    "height" => self.ui.text_defaults.height,
                    "letter_spacing" => self.ui.text_defaults.letter_spacing,
                    _ => self.ui.text_defaults.line_spacing,
                };
                let r = ui.add_enabled(enabled, egui::DragValue::new(&mut value).speed(0.1).max_decimals(3));
                self.labeled(&r, egui::WidgetType::DragValue, label, false);
                if r.changed() && value.is_finite() {
                    match field {
                        "height" => self.ui.text_defaults.height = value,
                        "letter_spacing" => self.ui.text_defaults.letter_spacing = value,
                        _ => self.ui.text_defaults.line_spacing = value,
                    }
                    fire(self, key);
                }
            }
            for (name, n) in [(t("L", "E"), 0u8), ("C", 1u8), (t("R", "D"), 2u8)] {
                let r = ui.selectable_label(self.ui.text_defaults.align == n, name);
                self.labeled(
                    &r,
                    egui::WidgetType::RadioButton,
                    &format!("Align {}", ["Left", "Center", "Right"].get(usize::from(n)).copied().unwrap_or("Left")),
                    false,
                );
                if r.clicked() {
                    self.ui.text_defaults.align = n;
                    fire(self, "align");
                }
            }
        });
    }

    pub(crate) fn numeric_edits(&mut self, ui: &mut egui::Ui) {
        if self.ui.selected_tool == "Edit Text"
            || self
                .selection()
                .first()
                .and_then(|_id| self.project())
                .is_some_and(|p| self.selection().iter().any(|id| matches!(p.shape(*id).map(|s| &s.geom), Some(openlaser_doc::Geom::Text(_)))))
        {
            self.text_options(ui);
            return;
        }
        let selection = self.selection();
        // An unmeasurable selection (a non-finite transform) gives no bounds: the edits stay disabled.
        let bounds = self.project().and_then(|p| p.bounds(&selection).unwrap_or_default());
        let enabled = bounds.is_some() && !self.ui.machine_active;
        let reason = if self.ui.machine_active {
            t("Edits are locked while a machine is active.", "As edições ficam bloqueadas enquanto uma máquina está ativa.")
        } else {
            t("Select shapes to edit their position and size.", "Selecione formas para editar a posição e o tamanho.")
        };
        ui.horizontal(|ui| {
            // The 9-point anchor: which point of the selection bounds XPos/YPos name.
            egui::Grid::new("anchor-grid").spacing([2.0, 2.0]).min_col_width(8.0).min_row_height(8.0).show(ui, |ui| {
                for n in 0..9u8 {
                    let name = ["top-left", "top", "top-right", "left", "centre", "right", "bottom-left", "bottom", "bottom-right"]
                        .get(usize::from(n))
                        .copied()
                        .unwrap_or("centre");
                    let shown = t(
                        name,
                        [
                            "o canto superior esquerdo",
                            "o topo",
                            "o canto superior direito",
                            "a esquerda",
                            "o centro",
                            "a direita",
                            "o canto inferior esquerdo",
                            "a base",
                            "o canto inferior direito",
                        ]
                        .get(usize::from(n))
                        .copied()
                        .unwrap_or("o centro"),
                    );
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
                    if r.on_hover_text(tf!("XPos/YPos name the {name} of the selection", "PosX/PosY indicam {shown} da seleção")).clicked() {
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
            let fields: [(&str, &str, &str, &str); 7] = [
                ("XPos", "PosX", "xMm", "mm"),
                ("YPos", "PosY", "yMm", "mm"),
                ("Width", "Largura", "wMm", "mm"),
                ("Height", "Altura", "hMm", "mm"),
                ("W%", "L%", "wPct", "%"),
                ("H%", "A%", "hPct", "%"),
                ("Rotate", "Girar", "rotateDeg", "°"),
            ];
            // The 9-dot selector's index → the engine's AnchorP (row-major from the top-left, Y up).
            const ANCHORS: [&str; 9] = ["topLeft", "top", "topRight", "left", "center", "right", "bottomLeft", "bottom", "bottomRight"];
            for (n, (label, pt, key, unit)) in fields.iter().enumerate() {
                if n == 4 {
                    let icon = if self.ui.lock_ratio { "lock" } else { "lock-open" };
                    let r = ui.add(egui::Button::image(icon_image(icon, ui)).selected(self.ui.lock_ratio));
                    self.labeled(&r, egui::WidgetType::Checkbox, "Lock aspect ratio", false);
                    if r.on_hover_text(t("Lock the aspect ratio", "Travar a proporção")).clicked() {
                        self.ui.lock_ratio = !self.ui.lock_ratio;
                    }
                }
                caption(ui, t(label, pt));
                let mut value = values.and_then(|v| v.get(n).copied()).unwrap_or(0.0);
                let r = ui.add_enabled(enabled, egui::DragValue::new(&mut value).speed(0.1).max_decimals(3).suffix(*unit));
                self.labeled(&r, egui::WidgetType::DragValue, label, false);
                let r = r.on_disabled_hover_text(reason);
                if r.changed() && value.is_finite() {
                    let mut params = serde_json::json!({
                        "anchor": ANCHORS.get(usize::from(self.ui.anchor)).copied().unwrap_or("center"),
                        "lockRatio": self.ui.lock_ratio,
                        "coalesce": format!("numeric-{key}"),
                    });
                    let (key, value) = match *key {
                        "wPct" => ("wMm", values.and_then(|v| v.get(2).copied()).unwrap_or(0.0) * value / 100.0),
                        "hPct" => ("hMm", values.and_then(|v| v.get(3).copied()).unwrap_or(0.0) * value / 100.0),
                        other => (other, value),
                    };
                    if let Some(map) = params.as_object_mut() {
                        map.insert(key.to_owned(), serde_json::json!(value));
                    }
                    self.execute_widget("arrange.transform", params);
                }
            }
            // Edit Nodes on a parametric/text shape: LightBurn asks to convert first.
            if self.ui.selected_tool == "Edit Nodes" && self.ui.nodes.is_none() {
                let sel = self.selection();
                let convertible = sel.len() == 1
                    && sel.first().is_some_and(|id| {
                        self.project().is_some_and(|p| {
                            matches!(
                                p.shape(*id).map(|s| &s.geom),
                                Some(
                                    openlaser_doc::Geom::Rect { .. }
                                        | openlaser_doc::Geom::Ellipse { .. }
                                        | openlaser_doc::Geom::Polygon { .. }
                                        | openlaser_doc::Geom::Text(_)
                                )
                            )
                        })
                    });
                if convertible {
                    let r = ui.add(egui::Button::new(t("Convert to Path", "Converter em caminho")));
                    self.labeled(&r, egui::WidgetType::Button, "Convert to Path", false);
                    if r.on_hover_text(t(
                        "Convert the shape to a path so its nodes can be edited",
                        "Converte a forma em caminho para que seus nós possam ser editados",
                    ))
                    .clicked()
                        && let Some(id) = sel.first()
                    {
                        self.execute_widget("edit.convertToPath", serde_json::json!({ "ids": [id.0] }));
                    }
                }
            }
            // The Polygon tool's side count, set before the drag as LightBurn's shape properties do.
            if self.ui.selected_tool == "Polygon" {
                caption(ui, t("Sides", "Lados"));
                let mut sides = f64::from(self.ui.polygon_sides);
                let r = ui.add(egui::DragValue::new(&mut sides).speed(0.1).range(3.0..=64.0).max_decimals(0));
                self.labeled(&r, egui::WidgetType::DragValue, "Sides", false);
                let r = r.on_hover_text(t("The polygon's vertex count (3–64)", "O número de vértices do polígono (3–64)"));
                if r.changed() && (3.0..=64.0).contains(&sides) {
                    self.ui.polygon_sides = sides as u16;
                }
            }
        });
    }

    // ---- left tool palette ------------------------------------------------------------------------------------------

    pub(crate) fn tool_palette(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            for (n, (label, pt, id, icon)) in LEFT_TOOLS.iter().enumerate() {
                if n == 8 {
                    ui.separator();
                }
                let live = *id == "tools.select" || command_live(id);
                let r = ui.add_enabled(live, egui::Button::image(icon_image(icon, ui)).selected(self.ui.selected_tool == *label).min_size(BUTTON));
                self.labeled(&r, egui::WidgetType::Button, label, false);
                let text = t(label, pt);
                let r = if live { r.on_hover_text(text) } else { r.on_disabled_hover_text(format!("{text}: {}", not_yet())) };
                if r.clicked() {
                    // Route every palette row through the one dispatcher: it knows which ids arm a canvas
                    // tool and which execute, marks provenance, and applies the machine-active guard.
                    self.invoke_menu_command(ui.ctx(), id);
                }
            }
        });
    }

    // ---- Arrange toolbar and colour palette ------------------------------------------------------------------------

    pub(crate) fn arrange_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (label, pt, id, icon) in ARRANGE_TOOLS {
                let live = command_live(id);
                let r = ui.add_enabled(live && !self.ui.machine_active, egui::Button::image(icon_image(icon, ui)).min_size(BUTTON));
                self.labeled(&r, egui::WidgetType::Button, label, false);
                let text = t(label, pt);
                let r = if live {
                    r.on_hover_text(text).on_disabled_hover_text(t(
                        "Edits are locked while a machine is active.",
                        "As edições ficam bloqueadas enquanto uma máquina está ativa.",
                    ))
                } else {
                    r.on_disabled_hover_text(format!("{text}: {}", not_yet()))
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
                let hover = format!("{} · {}", swatch_title(key), self.layer_summary(key));
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
            let summary = match self.project().and_then(|p| p.bounds(&selection).unwrap_or_default()) {
                Some(b) if !selection.is_empty() => {
                    tf!("{} selected · {:.2} × {:.2} mm", "{} selecionado(s) · {:.2} × {:.2} mm", selection.len(), b.max[0] - b.min[0], b.max[1] - b.min[1])
                }
                _ => tf!("{} shapes", "{} formas", self.project().map(|p| p.shapes.len()).unwrap_or(0)),
            };
            ui.label(summary);
            ui.separator();
            let (chip, color) = self.chip();
            ui.colored_label(color, chip);
            if let Some(summary) = self.plan().and_then(PlanView::summary) {
                ui.separator();
                ui.label(tf!("est {}", "est. {}", mmss(summary.estimate)));
            }
            let refusals = self.plan().map(|p| p.refusals().len()).unwrap_or(0);
            if refusals > 0 {
                ui.separator();
                let r = ui.add(
                    egui::Button::new(egui::RichText::new(tf!("{refusals} refusal(s)", "{refusals} recusa(s)")).color(egui::Color32::WHITE))
                        .fill(STOP_RED)
                        .small(),
                );
                self.labeled(&r, egui::WidgetType::Button, "Refusals", false);
                if r.on_hover_text(t("Show the refusals in the Laser panel", "Mostrar as recusas no painel Laser")).clicked() {
                    let _ = self.show_panel(&serde_json::json!({ "panel": "laser", "visible": true }));
                }
            }
            ui.separator();
            ui.label(&self.ui.status_message);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let agent = match self.services.automation.info() {
                    None => t("agent: none", "agente: nenhum").to_owned(),
                    Some(info) if info.clients.is_empty() => tf!("agent: listening on {}", "agente: aguardando na porta {}", info.port),
                    Some(info) => tf!("agent: connected ({})", "agente: conectado ({})", info.clients.len()),
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
                "camera" => {
                    egui::ScrollArea::vertical().id_salt("camera-scroll").show(ui, |ui| self.camera_panel(ui));
                }
                "library" => {
                    ui.label(t(
                        "Material Library: presets by material and thickness arrive in P1 (CAT §5.12).",
                        "Biblioteca de materiais: predefinições por material e espessura chegam na P1 (CAT §5.12).",
                    ));
                }
                "move" => {
                    ui.label(if self.active_is_galvo() {
                        t(
                            "Move (jog) is for gantry machines. On this galvo, use Live Framing to see where the job lands.",
                            "Mover (jog) é para máquinas de pórtico. Neste galvo, use o Enquadramento ao vivo para ver onde o trabalho cai.",
                        )
                    } else {
                        t(
                            "Jogging a gantry arrives with live GRBL streaming (M4); G-code export works today.",
                            "O jog de pórtico chega com o streaming GRBL ao vivo (M4); a exportação de G-code já funciona.",
                        )
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

    // ---- Camera Control (phase 6a) ------------------------------------------------------------------------------------

    /// Enumerate cameras into `ui.camera.devices` (the panel's lazy init and its Refresh button share this).
    pub(crate) fn refresh_cameras(&mut self) {
        self.ui.camera.devices = Some(self.session.camera_devices().map_err(|e| e.to_string()));
        if let Some(Ok(devices)) = &self.ui.camera.devices
            && !devices.iter().any(|d| d.index == self.ui.camera.selected)
        {
            self.ui.camera.selected = devices.first().map(|d| d.index).unwrap_or(0);
        }
    }

    /// The Camera Control panel: device dropdown, Start/Stop (engine `camera.*` commands — view state, safe
    /// for agents), the live feed, honest empty/error states and the Calibrate wizard (phase 6b): clicks on
    /// the feed collect `{cam px ↔ bed mm}` pairs for `camera.calibrate.fit`, which persists through the
    /// session's machine-local store.
    pub(crate) fn camera_panel(&mut self, ui: &mut egui::Ui) {
        if self.ui.camera.devices.is_none() {
            self.refresh_cameras();
        }
        let running = self.camera_running();
        let has_devices = matches!(&self.ui.camera.devices, Some(Ok(d)) if !d.is_empty());
        ui.horizontal(|ui| {
            match &self.ui.camera.devices {
                Some(Ok(devices)) if !devices.is_empty() => {
                    let selected =
                        devices.iter().find(|d| d.index == self.ui.camera.selected).or_else(|| devices.first()).map(|d| d.name.clone()).unwrap_or_default();
                    egui::ComboBox::from_id_salt("camera-device").selected_text(selected).show_ui(ui, |ui| {
                        for d in devices {
                            ui.selectable_value(&mut self.ui.camera.selected, d.index, format!("{} ({})", d.name, d.index));
                        }
                    });
                }
                Some(Ok(_)) => {
                    ui.weak(t(
                        "No camera found — check the OS camera permission, then Refresh.",
                        "Nenhuma câmera encontrada — verifique a permissão de câmera do sistema e clique em Atualizar.",
                    ));
                }
                Some(Err(e)) => {
                    ui.weak(tf!("Cannot enumerate cameras: {e}", "Não foi possível listar as câmeras: {e}"));
                }
                None => {}
            }
            let refresh = ui.button(t("Refresh", "Atualizar"));
            self.labeled(&refresh, egui::WidgetType::Button, "Refresh camera list", false);
            if refresh.clicked() {
                self.refresh_cameras();
            }
            // Opening a camera runs on this thread and can stall (a slow device, the OS permission prompt). Never
            // during a frame or a job: a stalled window would freeze the on-screen STOP until it returns.
            let busy = self.safety_active();
            let start = ui.add_enabled(!running && !busy, egui::Button::new(t("Start", "Iniciar"))).on_disabled_hover_text(if busy {
                t("Not while framing or marking: stop the machine first.", "Não durante o enquadramento ou a marcação: pare a máquina primeiro.")
            } else {
                t("The camera is already running.", "A câmera já está ligada.")
            });
            self.labeled(&start, egui::WidgetType::Button, "Start camera", false);
            if start.clicked() {
                // No device list (or none enumerated): let `camera.start` pick the first camera itself.
                let params = if has_devices { serde_json::json!({ "index": self.ui.camera.selected }) } else { serde_json::json!({}) };
                self.execute_widget("camera.start", params);
            }
            let stop = ui.add_enabled(running, egui::Button::new(t("Stop", "Parar")));
            self.labeled(&stop, egui::WidgetType::Button, "Stop camera", false);
            if stop.clicked() {
                self.execute_widget("camera.stop", serde_json::json!({}));
            }
        });

        if let Some(error) = self.session.camera_last_error() {
            ui.colored_label(egui::Color32::from_rgb(230, 80, 60), tf!("Camera: {error}", "Câmera: {error}"));
        }

        if running {
            ui.horizontal(|ui| {
                let cal = ui.selectable_label(self.ui.camera.calibrating, t("Calibrate", "Calibrar"));
                self.labeled(&cal, egui::WidgetType::Button, "Calibrate camera", false);
                if cal.clicked() {
                    self.ui.camera.calibrating = !self.ui.camera.calibrating;
                    self.ui.camera.pending_cam = None;
                }
                if let Some(stored) = self.session.camera_calibration() {
                    ui.weak(tf!(
                        "{} fit · rms {:.2} px · {} pts",
                        "ajuste {} · rms {:.2} px · {} pts",
                        stored.calibration.kind(),
                        stored.calibration.rms_error_px(),
                        stored.pairs.len()
                    ));
                }
            });
            match self.camera_frame() {
                Some(frame) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &frame.rgba);
                    let tex = self.camera_tex.get_or_insert_with(|| ui.ctx().load_texture("camera-feed", image.clone(), egui::TextureOptions::LINEAR));
                    tex.set(image, egui::TextureOptions::LINEAR);
                    // Fit the feed to the panel width (never upscale past 1:1).
                    let natural = egui::vec2(frame.width as f32, frame.height as f32);
                    let scale = (ui.available_width() / natural.x).min(1.0);
                    let sense = if self.ui.camera.calibrating { egui::Sense::click() } else { egui::Sense::hover() };
                    let response = ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), natural * scale)).sense(sense));
                    self.ui.camera.feed_rect = Some(response.rect);
                    if self.ui.camera.calibrating {
                        self.camera_cal_overlay(ui, &response, scale);
                    }
                    // Repaint only while streaming: ~30 fps, matching the pump's own frame rate.
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
                }
                None => {
                    ui.weak(t("Waiting for the first frame…", "Aguardando o primeiro quadro…"));
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
            if self.ui.camera.calibrating {
                self.camera_cal_controls(ui);
            }
        } else {
            if self.camera_tex.is_some() {
                self.camera_tex = None; // free the feed texture once the stream stops
            }
            self.ui.camera.feed_rect = None;
            self.ui.camera.pending_cam = None;
        }
    }

    /// Calibrate mode's feed overlay: the collected pairs as numbered dots, the pending click marker, and
    /// the click that captures a camera-pixel point for its bed-mm coordinates.
    fn camera_cal_overlay(&mut self, ui: &mut egui::Ui, response: &egui::Response, scale: f32) {
        let pairs: Vec<openlaser_engine::CameraPair> = self.session.camera_cal_pairs().to_vec();
        let painter = ui.painter_at(response.rect);
        let to_screen = |cam: [f64; 2]| response.rect.min + egui::vec2(cam[0] as f32 * scale, cam[1] as f32 * scale);
        let accent = egui::Color32::from_rgb(90, 180, 255);
        for (i, pair) in pairs.iter().enumerate() {
            let pos = to_screen(pair.cam);
            painter.circle_filled(pos, 6.5, accent);
            painter.circle_stroke(pos, 6.5, egui::Stroke::new(1.0, egui::Color32::BLACK));
            painter.text(pos, egui::Align2::CENTER_CENTER, format!("{}", i + 1), egui::FontId::proportional(9.0), egui::Color32::BLACK);
        }
        if let Some(cam) = self.ui.camera.pending_cam {
            let pos = to_screen(cam);
            painter.circle_stroke(pos, 7.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 200, 60)));
            painter.line_segment([pos - egui::vec2(10.0, 0.0), pos + egui::vec2(10.0, 0.0)], egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 200, 60)));
            painter.line_segment([pos - egui::vec2(0.0, 10.0), pos + egui::vec2(0.0, 10.0)], egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 200, 60)));
        }
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && scale > 0.0
        {
            let cam = [(pos.x - response.rect.min.x) / scale, (pos.y - response.rect.min.y) / scale];
            if cam.iter().all(|v| v.is_finite()) {
                self.ui.camera.pending_cam = Some([f64::from(cam[0]), f64::from(cam[1])]);
            }
        }
    }

    /// Calibrate mode's controls: the bed-mm inputs for the pending click, Add/Undo/Clear, Fit and the
    /// honest hint — everything dispatches the `camera.calibrate.*` engine commands.
    fn camera_cal_controls(&mut self, ui: &mut egui::Ui) {
        ui.weak(t(
            "Place a marked target at a known bed position, click it in the image, enter its mm coordinates; repeat for at least 4 spread-out points.",
            "Coloque um alvo marcado numa posição conhecida da mesa, clique nele na imagem e digite as coordenadas em mm; repita para pelo menos 4 pontos espalhados.",
        ));
        let pairs = self.session.camera_cal_pairs().len();
        ui.horizontal(|ui| {
            let cam = self.ui.camera.pending_cam;
            ui.label(match cam {
                Some(c) => tf!("image ({:.0}, {:.0}) ↔ bed", "imagem ({:.0}, {:.0}) ↔ mesa", c[0], c[1]),
                None => t("click a target in the image", "clique em um alvo na imagem").to_owned(),
            });
            let x = ui.add(egui::TextEdit::singleline(&mut self.ui.camera.bed_x).desired_width(56.0).hint_text("x mm"));
            self.labeled(&x, egui::WidgetType::TextEdit, "Bed X (mm)", false);
            let y = ui.add(egui::TextEdit::singleline(&mut self.ui.camera.bed_y).desired_width(56.0).hint_text("y mm"));
            self.labeled(&y, egui::WidgetType::TextEdit, "Bed Y (mm)", false);
            let ready = cam.is_some() && parse_bed(&self.ui.camera).is_some();
            let add = ui.add_enabled(ready, egui::Button::new(t("Add point", "Adicionar ponto")));
            self.labeled(&add, egui::WidgetType::Button, "Add calibration point", false);
            if add.clicked()
                && let (Some(c), Some((bx, by))) = (cam, parse_bed(&self.ui.camera))
            {
                let before = self.session.camera_cal_pairs().len();
                self.execute_widget("camera.calibrate.addPair", serde_json::json!({ "camX": c[0], "camY": c[1], "bedX": bx, "bedY": by }));
                if self.session.camera_cal_pairs().len() > before {
                    self.ui.camera.pending_cam = None;
                    self.ui.camera.bed_x.clear();
                    self.ui.camera.bed_y.clear();
                }
            }
        });
        ui.horizontal(|ui| {
            let undo = ui.add_enabled(pairs > 0, egui::Button::new(t("Undo last", "Desfazer último")));
            self.labeled(&undo, egui::WidgetType::Button, "Remove last calibration point", false);
            if undo.clicked() {
                self.execute_widget("camera.calibrate.removeLast", serde_json::json!({}));
            }
            let clear = ui.add_enabled(pairs > 0 || self.session.camera_calibration().is_some(), egui::Button::new(t("Clear", "Limpar")));
            self.labeled(&clear, egui::WidgetType::Button, "Clear camera calibration", false);
            if clear.clicked() {
                self.execute_widget("camera.calibrate.clear", serde_json::json!({}));
            }
            let fit = ui.add_enabled(pairs >= 3, egui::Button::new(t("Fit", "Ajustar")));
            self.labeled(&fit, egui::WidgetType::Button, "Fit camera calibration", false);
            if fit.clicked() {
                self.execute_widget("camera.calibrate.fit", serde_json::json!({}));
            }
            ui.label(tf!("{pairs} point(s)", "{pairs} ponto(s)"));
            let done = ui.button(t("Done", "Concluir"));
            self.labeled(&done, egui::WidgetType::Button, "Finish calibrating", false);
            if done.clicked() {
                self.ui.camera.calibrating = false;
                self.ui.camera.pending_cam = None;
            }
        });
    }

    // ---- Cuts / Layers --------------------------------------------------------------------------------------------------

    pub(crate) fn cuts_panel(&mut self, ui: &mut egui::Ui) {
        let used = self.project().map(used_layers).unwrap_or_default();
        let gantry = !self.active_is_galvo();
        let mut columns = vec!["#", t("Layer", "Camada"), t("Mode", "Modo"), t("Spd/Pwr", "Vel/Pot"), t("Output", "Saída"), t("Show", "Mostrar")];
        if gantry {
            columns.push(t("Air", "Ar"));
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
            ui.weak(t(
                "No layers in use. Draw or import shapes, or pick a colour in the palette below.",
                "Nenhuma camada em uso. Desenhe ou importe formas, ou escolha uma cor na paleta abaixo.",
            ));
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
        let refusals: Vec<String> =
            self.plan().map(|p| p.refusals().iter().filter(|r| r.layer == Some(key)).map(crate::i18n::refusal).collect()).unwrap_or_default();
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
        let out = out.on_disabled_hover_text(if tool {
            t("Tool layers (T1/T2) never output.", "Camadas de ferramenta (T1/T2) nunca têm saída.")
        } else {
            t("Locked while a machine is active.", "Bloqueado enquanto uma máquina está ativa.")
        });
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
            return (
                t("Tool", "Ferramenta").into(),
                "—".into(),
                t("Tool layers trace and guide; they never burn.", "Camadas de ferramenta traçam e guiam; nunca queimam.").into(),
            );
        };
        let Some(op) = self.project().and_then(|p| p.color_layer(i)).and_then(|l| l.ops.first()) else {
            return ("—".into(), "—".into(), String::new());
        };
        let mode = match op.mode {
            openlaser_doc::CutMode::Line => t("Line", "Linha"),
            openlaser_doc::CutMode::Fill => t("Fill", "Preenchimento"),
            openlaser_doc::CutMode::OffsetFill => t("Offset Fill", "Preenchimento deslocado"),
        };
        let gantry = !self.active_is_galvo();
        let (requested, power) = if gantry { (op.gantry.speed.get(), op.gantry.power_max.get()) } else { (op.galvo.speed.get(), op.galvo.power.get()) };
        let khz = f64::from(op.galvo.frequency.get()) / 1000.0;
        let ns = op.galvo.q_pulse.map(|n| format!("{} ns", n.get())).unwrap_or_else(|| "—".into());
        match self.actual_speed(key) {
            Some(actual) => (
                mode.into(),
                format!("{actual:.1} / {power:.0}"),
                tf!(
                    "Speed {actual:.2} mm/s (actual, decoded from the bytes) · requested {requested} mm/s · power {power} % · {khz} kHz · {ns}",
                    "Velocidade {actual:.2} mm/s (real, decodificada dos bytes) · pedida {requested} mm/s · potência {power} % · {khz} kHz · {ns}"
                ),
            ),
            None => (
                mode.into(),
                format!("{requested} / {power:.0}"),
                tf!(
                    "Speed {requested} mm/s (requested; the actual encoded speed shows once the job plans) · power {power} % · {khz} kHz · {ns}",
                    "Velocidade {requested} mm/s (pedida; a velocidade real codificada aparece quando o trabalho for planejado) · potência {power} % · {khz} kHz · {ns}"
                ),
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
        ui.strong(tf!("Layer {name}", "Camada {name}"));
        let LayerKey::Color(i) = key else {
            ui.label(t(
                "Tool layer: never output. Live Framing can trace it (Tool Layers Only).",
                "Camada de ferramenta: nunca tem saída. O Enquadramento ao vivo pode traçá-la (Só camadas de ferramenta).",
            ));
            return;
        };
        let Some(op) = self.project().and_then(|p| p.color_layer(i)).and_then(|l| l.ops.first()).cloned() else {
            ui.weak(t("This layer has no cut settings yet.", "Esta camada ainda não tem configurações de corte."));
            return;
        };
        let speed = match self.actual_speed(key) {
            Some(actual) => {
                tf!("Speed: {} mm/s requested · {actual:.2} mm/s (actual)", "Velocidade: {} mm/s pedida · {actual:.2} mm/s (real)", op.galvo.speed.get())
            }
            None => tf!("Speed: {} mm/s requested", "Velocidade: {} mm/s pedida", op.galvo.speed.get()),
        };
        ui.label(speed);
        ui.label(tf!(
            "Power {} % · Frequency {} kHz · Q-Pulse {} · Passes {}",
            "Potência {} % · Frequência {} kHz · Q-Pulse {} · Passadas {}",
            op.galvo.power.get(),
            f64::from(op.galvo.frequency.get()) / 1000.0,
            op.galvo.q_pulse.map(|n| format!("{} ns", n.get())).unwrap_or_else(|| "—".into()),
            op.passes
        ));
        if op.mode != openlaser_doc::CutMode::Line {
            ui.label(tf!(
                "Interval {} mm · Angle {}°{}",
                "Intervalo {} mm · Ângulo {}°{}",
                op.fill.interval.get(),
                op.fill.angle.get(),
                if op.fill.cross_hatch { t(" · Cross-Hatch", " · Hachura cruzada") } else { "" }
            ));
        }
        let edit = ui.add(egui::Button::new(t("Edit Cut Settings…", "Editar configurações de corte…")));
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
        if self.active_machine().is_some_and(|m| matches!(&m.kind, openlaser_machine::Kinematics::Galvo(g) if g.rotary.as_ref().is_some_and(|r| r.enabled))) {
            ui.strong(t("ROTARY ON", "ROTATIVO LIGADO"));
        }
        let busy = self.is_machine_busy();

        // 1. Device row.
        ui.horizontal(|ui| {
            let devices = ui.button(t("Devices…", "Dispositivos…"));
            self.track(&devices, "button", "Devices…", false);
            if devices.on_hover_text(t("Device Settings for the selected machine", "Configurações do dispositivo da máquina selecionada")).clicked() {
                self.ui.device_settings_open = true;
            }
            let machines: Vec<(openlaser_types::MachineId, String)> = self.session.machines().iter().map(|m| (m.id.clone(), m.name.clone())).collect();
            let current = self.active_machine().map(|m| m.id.clone());
            let mut chosen = current.clone();
            let combo = ui
                .add_enabled_ui(!busy, |ui| {
                    egui::ComboBox::new("laser-device", t("Device", "Dispositivo"))
                        .selected_text(
                            machines
                                .iter()
                                .find(|(id, _)| Some(id) == current.as_ref())
                                .map(|(_, n)| n.as_str())
                                .unwrap_or(t("No device", "Nenhum dispositivo")),
                        )
                        .show_ui(ui, |ui| {
                            for (id, name) in &machines {
                                ui.selectable_value(&mut chosen, Some(id.clone()), name);
                            }
                        })
                        .response
                })
                .inner;
            self.track(&combo, "comboBox", "Device", false);
            combo.on_disabled_hover_text(t("Stop the machine before switching devices.", "Pare a máquina antes de trocar de dispositivo."));
            if chosen != current
                && let Some(id) = chosen
            {
                self.execute_widget("device.select", serde_json::json!({ "device": id.0 }));
            }
        });
        self.foot_switch_row(ui);
        if self.simulating_real_machine() {
            let note = ui.colored_label(
                SIMULATOR_COLOR,
                t(
                    "Simulator: this window cannot reach the laser. Frame, Start and ARM here move nothing. To use the laser, close OpenLaser and open it with “Start OpenLaser with laser”.",
                    "Simulador: esta janela não alcança o laser. Enquadrar, Iniciar e ARMAR aqui não movem nada. Para usar o laser, feche o OpenLaser e abra-o com “Start OpenLaser with laser”.",
                ),
            );
            self.track(&note, "status", "Simulator notice", false);
        }
        ui.horizontal(|ui| {
            let (text, color) = self.chip();
            ui.colored_label(color, text);
            let connected = self.active_machine().is_some_and(|m| self.session.status(&m.id).is_some());
            if connected {
                self.human_button(
                    ui,
                    &ctx,
                    "Disconnect",
                    t("Disconnect", "Desconectar"),
                    if busy { Err(t("Stop the machine first.", "Pare a máquina primeiro.").to_owned()) } else { Ok(()) },
                    |app| {
                        let id = app.active_machine().map(|m| m.id.clone()).ok_or(openlaser_engine::EngineError::NoDevice)?;
                        app.session.disconnect(openlaser_engine::Origin::Human, &id)
                    },
                );
            } else {
                self.human_button(
                    ui,
                    &ctx,
                    "Connect",
                    t("Connect", "Conectar"),
                    if busy { Err(t("Stop the machine first.", "Pare a máquina primeiro.").to_owned()) } else { Ok(()) },
                    |app| {
                        let id = app.active_machine().map(|m| m.id.clone()).ok_or(openlaser_engine::EngineError::NoDevice)?;
                        app.session.connect(openlaser_engine::Origin::Human, &id)
                    },
                );
            }
        });

        // 2. Job buttons: Pause, Stop, Start; Frame, Send, Home, Go to Origin.
        let plan = self.plan().cloned();
        let receipt = plan.as_ref().and_then(PlanView::summary).and_then(|s| self.receipt_for(&s.machine));
        // Framing is optional (Settings): then Start needs only a plannable job and an idle machine. The foot switch
        // goes through the same gate (`start_gate_now`).
        let start = self.start_gate_now();
        let frame = frame_gate(plan.as_ref(), active);
        ui.horizontal(|ui| {
            self.disabled_button(
                ui,
                "Pause",
                t("Pause", "Pausar"),
                if galvo {
                    t(
                        "Pause is not available on galvo machines in this build (H15). Use Stop.",
                        "Pausar não está disponível em máquinas galvo nesta versão (H15). Use Parar.",
                    )
                } else {
                    not_yet()
                },
            );
            let stop = ui.add(egui::Button::new(t("Stop", "Parar")).min_size(egui::vec2(56.0, 24.0)));
            self.track(&stop, "button", "Stop", false);
            if stop.on_hover_text(t("Stop laser: latches every machine at once", "Parar o laser: trava todas as máquinas de uma vez")).clicked() {
                self.stop();
            }
            self.human_button(ui, &ctx, "Start", t("Start", "Iniciar"), start.clone(), |app| {
                app.session.request_arm(openlaser_engine::Origin::Human).map(|_| ())
            });
        });
        ui.horizontal(|ui| {
            let (style, source) = (self.ui.live_style, self.ui.live_source);
            self.human_button(ui, &ctx, "Frame", t("Frame", "Enquadrar"), frame, move |app| {
                app.ui.live_framing_open = true;
                app.session.start_frame(openlaser_engine::Origin::Human, openlaser_types::FrameMode::Once, Some(style), Some(source)).map(|_| ())
            });
            self.disabled_button(
                ui,
                "Send",
                t("Send", "Enviar"),
                t(
                    "Not available for this device: Send uploads to DSP controllers only.",
                    "Indisponível neste dispositivo: Enviar só transfere para controladoras DSP.",
                ),
            );
            self.disabled_button(
                ui,
                "Home",
                t("Home", "Referenciar"),
                if galvo {
                    t("A galvo has no homing: its origin is the field centre.", "Um galvo não tem referenciamento: a origem é o centro do campo.")
                } else {
                    not_yet()
                },
            );
            self.disabled_button(
                ui,
                "Go to Origin",
                t("Go to Origin", "Ir para a origem"),
                if galvo {
                    t("A galvo has no head to move: its origin is the field centre.", "Um galvo não tem cabeçote para mover: a origem é o centro do campo.")
                } else {
                    not_yet()
                },
            );
        });

        // 3. Start From and Job Origin (galvos always use absolute coordinates, CTL-03).
        ui.horizontal(|ui| {
            caption(ui, t("Start From", "Iniciar de"));
            let r = ui.add_enabled(false, egui::Button::new(t("Absolute Coords", "Coord. absolutas")));
            self.labeled(&r, egui::WidgetType::ComboBox, "Start From", false);
            r.on_disabled_hover_text(if galvo {
                t(
                    "Galvos always use absolute coordinates: the field centre is the origin.",
                    "Galvos sempre usam coordenadas absolutas: o centro do campo é a origem.",
                )
            } else {
                not_yet()
            });
            let r = ui.add_enabled(false, egui::Button::new("⋮⋮⋮"));
            self.labeled(&r, egui::WidgetType::Button, "Job Origin", false);
            r.on_disabled_hover_text(t(
                "Job Origin applies to User Origin and Current Position only.",
                "A origem do trabalho só se aplica a Origem do usuário e Posição atual.",
            ));
        });

        // 4. Options.
        let has_doc = self.project().is_some();
        let mut cut_selected = self.project().is_some_and(|p| p.job.cut_selected);
        let r = ui.add_enabled(has_doc && !active, egui::Checkbox::new(&mut cut_selected, t("Cut Selected Graphics", "Cortar gráficos selecionados")));
        self.track(&r, "checkBox", "Cut Selected Graphics", false);
        if r.changed() {
            self.execute_widget("laser.cutSelected", serde_json::json!({ "on": cut_selected }));
        }
        let mut use_origin = false;
        let r = ui.add_enabled(false, egui::Checkbox::new(&mut use_origin, t("Use Selection Origin", "Usar origem da seleção")));
        self.track(&r, "checkBox", "Use Selection Origin", false);
        r.on_disabled_hover_text(if galvo {
            t("Not used on galvos: the field centre is the origin.", "Não usado em galvos: o centro do campo é a origem.")
        } else {
            not_yet()
        });
        ui.horizontal(|ui| {
            caption(ui, t("Repeat", "Repetir"));
            let mut repeats = self.project().map(|p| p.job.repeats).unwrap_or(1);
            let r = ui.add_enabled(has_doc && !active, egui::DragValue::new(&mut repeats).range(1..=999));
            self.labeled(&r, egui::WidgetType::DragValue, "Repeat count", false);
            if r.changed() {
                self.execute_widget("laser.repeat", serde_json::json!({ "count": repeats }));
            }
        });

        // Requests from agents come first, right under the controls: a person must never have to scroll past a
        // long notice list to find "Approve framing" or a submitted job.
        self.request_cards(ui, &ctx);

        // 5. Estimate; 6. refusals and notices.
        ui.separator();
        match &plan {
            None => {
                ui.weak(t("No current job. Open, import or draw a design.", "Nenhum trabalho atual. Abra, importe ou desenhe um projeto."));
            }
            Some(PlanView::Ready(summary)) => {
                ui.label(tf!(
                    "Estimate {} · {} pass row(s) · {} packet(s) · crash bound {:.1} s",
                    "Estimativa {} · {} linha(s) de passada · {} pacote(s) · limite em falha {:.1} s",
                    mmss(summary.estimate),
                    summary.rows.len(),
                    summary.packets,
                    summary.crash_bound.as_secs_f64()
                ));
                if receipt.is_none() {
                    ui.label(t("Frame this job first.", "Enquadre este trabalho primeiro."));
                }
            }
            Some(PlanView::Refused(refused)) => {
                ui.colored_label(STOP_RED, tf!("{} refusal(s): this job will not run", "{} recusa(s): este trabalho não vai rodar", refused.refusals.len()));
                for r in &refused.refusals {
                    ui.label(format!("• {}", crate::i18n::refusal(r)));
                }
                if let Some(error) = &refused.error {
                    ui.label(error);
                }
            }
        }
        if let Some(plan) = &plan {
            let notices = plan.notices();
            if !notices.is_empty() {
                ui.label(tf!("{} notice(s):", "{} aviso(s):", notices.len()));
                // Bounded, so a long list (the business card has 67) never pushes the rest of the panel away.
                egui::ScrollArea::vertical().id_salt("laser-notices").max_height(NOTICES_MAX_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
                    for notice in notices {
                        ui.label(format!("{}. {}", notice.n, notice.message));
                    }
                });
            }
        }
        if let Err(reason) = &start {
            ui.weak(tf!("Start: {reason}", "Iniciar: {reason}"));
        }
        ui.separator();
        ui.weak(t(
            "STOP works while this window is focused; the E-stop always works.",
            "PARAR funciona com esta janela em foco; o botão de emergência sempre funciona.",
        ));
    }

    fn disabled_button(&mut self, ui: &mut egui::Ui, label: &str, text: &str, reason: &str) {
        let r = ui.add_enabled(false, egui::Button::new(text));
        self.track(&r, "button", label, false);
        r.on_disabled_hover_text(reason);
    }

    /// A human-only widget (Connect, Frame, Start): enabled when `gate` allows it; acts only on a real click in an
    /// untainted frame (`services.human.is_human_click`, asked every frame so it can time the widget's visibility).
    /// `label` is the English automation label (also matched below); `text` is what the button shows.
    fn human_button(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        label: &'static str,
        text: &'static str,
        gate: Result<(), String>,
        action: impl FnOnce(&mut Self) -> Result<(), openlaser_engine::EngineError>,
    ) {
        // Start fires the laser: red, so nobody mistakes it for a harmless button (Luiz, 2026-10-31).
        let button = if label == "Start" {
            egui::Button::new(egui::RichText::new(text).strong().color(egui::Color32::WHITE)).fill(egui::Color32::from_rgb(200, 0, 0))
        } else {
            egui::Button::new(text)
        };
        let r = ui.add_enabled(gate.is_ok(), button.min_size(egui::vec2(56.0, 24.0)));
        self.track(&r, "button", label, true);
        // Start keeps its click for the ARM confirmation that follows (ARM once per session, Settings).
        let human = if label == "Start" { self.services.human.start_click(ctx, &r) } else { self.services.human.is_human_click(ctx, &r) };
        let synthetic = self.frame_synthetic || self.services.human.frame_is_synthetic(ctx);
        if human && !synthetic && gate.is_ok() {
            #[cfg(test)]
            self.seam.human_actions.push(label);
            let message = match action(self) {
                Ok(()) => tf!("{label}: requested", "{text}: solicitado"),
                Err(e) => e.to_string(),
            };
            self.set_status(message);
        } else if r.clicked() {
            self.set_status(tf!(
                "{label} is human-only: it needs a person's click in this window (automation input is ignored).",
                "{text} só pode ser feito por uma pessoa: precisa de um clique nesta janela (entradas de automação são ignoradas)."
            ));
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
            .map(|r| {
                (
                    r.submission,
                    tf!("Job from {}", "Trabalho de {}", r.client),
                    tf!("Note from agent: {} · est {}", "Nota do agente: {} · est. {}", r.note, mmss(r.summary.estimate)),
                )
            })
            .collect();
        cards.extend(
            self.session.requests().frame_requests().into_iter().filter(|r| r.state == openlaser_engine::FrameRequestState::AwaitingHuman).map(|r| {
                (r.request, tf!("Frame request from {}", "Pedido de enquadramento de {}", r.client), tf!("{:?} framing", "Enquadramento {:?}", r.mode))
            }),
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
                for (label, text, approve) in
                    [("Approve framing", t("Approve framing", "Aprovar enquadramento"), true), ("Reject request", t("Reject request", "Recusar pedido"), false)]
                {
                    let r = ui.add_enabled(!self.ui.machine_active, egui::Button::new(text));
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
                            Ok(()) => tf!("{label}: done", "{text}: concluído"),
                            Err(e) => e.to_string(),
                        };
                        self.set_status(message);
                    } else if r.clicked() {
                        self.set_status(tf!(
                            "{label} is human-only: it needs a person's click in this window.",
                            "{text} só pode ser feito por uma pessoa: precisa de um clique nesta janela."
                        ));
                    }
                }
            });
        });
    }

    // ---- Console ---------------------------------------------------------------------------------------------------------

    fn console_panel(&mut self, ui: &mut egui::Ui) {
        ui.strong(t("Log", "Registro"));
        egui::ScrollArea::vertical().id_salt("console-log").max_height(ui.available_height() * 0.5).stick_to_bottom(true).show(ui, |ui| {
            for line in &self.log {
                ui.monospace(line);
            }
        });
        ui.separator();
        ui.strong(t("USB trace (decoded)", "Rastreamento USB (decodificado)"));
        let lines: Vec<String> = self
            .session
            .machines()
            .iter()
            .filter_map(|m| self.session.status(&m.id))
            .flat_map(|s| s.trace_tail.iter().map(|t| format!("{t:?}")).collect::<Vec<_>>())
            .collect();
        egui::ScrollArea::vertical().id_salt("console-trace").stick_to_bottom(true).show(ui, |ui| {
            if lines.is_empty() {
                ui.weak(t("No device traffic.", "Sem tráfego do dispositivo."));
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
                let button = egui::Button::new(egui::RichText::new(t("Stop laser", "Parar laser")).strong().size(16.0).color(egui::Color32::WHITE))
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

/// The bed-mm pair in the Calibrate wizard's text fields, once both parse to finite numbers.
fn parse_bed(camera: &crate::app::CameraUi) -> Option<(f64, f64)> {
    let parse = |s: &str| s.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    parse(&camera.bed_x).zip(parse(&camera.bed_y))
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
            rotary: None,
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
