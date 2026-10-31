//! Material Test dialog: design only, dispatched through the same command as CLI/MCP.
use serde_json::json;

use crate::app::OpenLaserApp;
use crate::i18n::t;

/// Wire key, English title (also the automation label), Portuguese title.
const PARAMS: [(&str, &str, &str); 6] = [
    ("powerPct", "Power (%)", "Potência (%)"),
    ("speedMmS", "Speed (mm/s)", "Velocidade (mm/s)"),
    ("frequencyKhz", "Frequency (kHz)", "Frequência (kHz)"),
    ("qPulseNs", "Q-pulse (ns)", "Pulso Q (ns)"),
    ("intervalMm", "Interval (mm)", "Intervalo (mm)"),
    ("passes", "Passes", "Passadas"),
];

#[derive(Debug, Clone)]
pub(crate) struct AxisUi {
    param: usize,
    min: f64,
    max: f64,
    pub count: u8,
}

#[derive(Debug, Clone)]
pub(crate) struct MaterialTestUi {
    pub rows: AxisUi,
    pub cols: AxisUi,
    w: f64,
    h: f64,
    gap: f64,
    center: [f64; 2],
    line: bool,
    base: [f64; 6],
    galvo: bool,
    labels: bool,
    label_power: f64,
    label_speed: f64,
}

impl MaterialTestUi {
    pub fn new(m: &openlaser_machine::MachineProfile) -> Self {
        let d = &m.layer_defaults;
        let galvo = matches!(m.kind, openlaser_machine::Kinematics::Galvo(_));
        let power = if galvo { d.galvo.power.get() } else { d.gantry.power_max.get() };
        let speed = if galvo { d.galvo.speed.get() } else { d.gantry.speed.get() };
        let frequency = f64::from(d.galvo.frequency.get()) / 1000.0;
        let center = match &m.kind {
            openlaser_machine::Kinematics::Gantry(g) => match g.origin {
                openlaser_doc::WorkspaceOrigin::Center => [0.0, 0.0],
                openlaser_doc::WorkspaceOrigin::BottomLeft => [g.bed_mm[0] / 2.0, g.bed_mm[1] / 2.0],
                openlaser_doc::WorkspaceOrigin::TopLeft => [g.bed_mm[0] / 2.0, -g.bed_mm[1] / 2.0],
            },
            _ => [0.0, 0.0],
        };
        Self {
            rows: AxisUi { param: 0, min: power, max: power, count: 5 },
            cols: AxisUi { param: if galvo { 2 } else { 1 }, min: if galvo { frequency } else { speed }, max: if galvo { frequency } else { speed }, count: 5 },
            w: 5.0,
            h: 5.0,
            gap: 2.0,
            center,
            line: false,
            base: [power, speed, frequency, d.galvo.q_pulse.map_or(1.0, |v| v.get()), d.fill.interval.get(), 1.0],
            galvo,
            labels: true,
            label_power: power,
            label_speed: speed,
        }
    }

    fn params(&self) -> serde_json::Value {
        let axis = |a: &AxisUi| json!({"param": PARAMS.get(a.param).map_or("powerPct", |p| p.0), "min": a.min, "max": a.max, "count": a.count});
        let mut base = serde_json::Map::new();
        for ((key, _, _), v) in PARAMS.iter().zip(self.base) {
            if self.galvo || !matches!(*key, "frequencyKhz" | "qPulseNs") {
                base.insert((*key).into(), json!(v));
            }
        }
        json!({"rows": axis(&self.rows), "cols": axis(&self.cols), "cellWMm": self.w, "cellHMm": self.h,
            "gapMm": self.gap, "centerMm": self.center, "mode": if self.line {"line"} else {"fill"}, "base": base,
            "labels": self.labels, "labelPowerPct": self.label_power, "labelSpeedMmS": self.label_speed})
    }
}

impl OpenLaserApp {
    pub(crate) fn material_test_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.ui.material_test.clone() else { return };
        let mut open = true;
        let mut create = false;
        let mut cancel = false;
        let mut tracked = Vec::new();
        egui::Window::new(t("Material Test", "Teste de material"))
            .id(egui::Id::new("material_test_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                for (name, pt, axis) in [("Rows", "Linhas", &mut d.rows), ("Columns", "Colunas", &mut d.cols)] {
                    ui.horizontal(|ui| {
                        ui.label(t(name, pt));
                        egui::ComboBox::from_id_salt(("material-axis", name))
                            .selected_text(PARAMS.get(axis.param).map_or(t("Power (%)", "Potência (%)"), |p| t(p.1, p.2)))
                            .show_ui(ui, |ui| {
                                for (i, (_, title, pt)) in PARAMS.iter().enumerate() {
                                    if !d.galvo && matches!(i, 2 | 3) {
                                        continue;
                                    }
                                    ui.selectable_value(&mut axis.param, i, t(title, pt));
                                }
                            });
                        ui.label(t("Min", "Mín."));
                        tracked.push((ui.add(egui::DragValue::new(&mut axis.min).speed(0.1)), format!("{name} min")));
                        ui.label(t("Max", "Máx."));
                        tracked.push((ui.add(egui::DragValue::new(&mut axis.max).speed(0.1)), format!("{name} max")));
                        ui.label(t("Count", "Quantidade"));
                        tracked.push((ui.add(egui::DragValue::new(&mut axis.count).range(1..=30)), format!("{name} count")));
                    });
                }
                ui.weak(t(
                    "Values run from min to max; descending ranges are allowed.",
                    "Os valores vão do mínimo ao máximo; faixas decrescentes são permitidas.",
                ));
                ui.horizontal(|ui| {
                    for (label, pt, v) in
                        [("Cell width", "Largura da célula", &mut d.w), ("Cell height", "Altura da célula", &mut d.h), ("Gap", "Espaço", &mut d.gap)]
                    {
                        ui.label(t(label, pt));
                        tracked.push((ui.add(egui::DragValue::new(v).speed(0.1).suffix(" mm")), label.into()));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(t("Center X / Y", "Centro X / Y"));
                    for (i, v) in d.center.iter_mut().enumerate() {
                        tracked.push((ui.add(egui::DragValue::new(v).speed(0.1).suffix(" mm")), format!("Center {}", if i == 0 { "X" } else { "Y" })));
                    }
                    ui.selectable_value(&mut d.line, false, t("Fill", "Preenchimento"));
                    ui.selectable_value(&mut d.line, true, t("Line", "Linha"));
                });
                egui::CollapsingHeader::new(t("Base settings", "Configurações base")).id_salt("material-base-settings").show(ui, |ui| {
                    egui::Grid::new("material-base").show(ui, |ui| {
                        for (i, ((_, label, pt), v)) in PARAMS.iter().zip(d.base.iter_mut()).enumerate() {
                            if !d.galvo && matches!(i, 2 | 3) {
                                continue;
                            }
                            ui.label(t(label, pt));
                            tracked.push((ui.add(egui::DragValue::new(v).speed(0.1)), format!("Base {label}")));
                            ui.end_row();
                        }
                    });
                });
                tracked.push((ui.checkbox(&mut d.labels, t("Labels", "Rótulos")), "Labels".into()));
                if d.labels {
                    ui.horizontal(|ui| {
                        ui.label(t("Label power", "Potência do rótulo"));
                        tracked.push((ui.add(egui::DragValue::new(&mut d.label_power).suffix(" %")), "Label power".into()));
                        ui.label(t("Label speed", "Velocidade do rótulo"));
                        tracked.push((ui.add(egui::DragValue::new(&mut d.label_speed).suffix(" mm/s")), "Label speed".into()));
                    });
                }
                let cells = usize::from(d.rows.count) * usize::from(d.cols.count);
                let max = if d.labels { 29 } else { 30 };
                ui.label(crate::tf!(
                    "{cells} cells (max {max} {})",
                    "{cells} células (máx. {max} {})",
                    if d.labels { t("with labels", "com rótulos") } else { t("without labels", "sem rótulos") }
                ));
                if d.rows.param == d.cols.param {
                    ui.colored_label(
                        crate::panels::STOP_RED,
                        t("Choose different parameters for rows and columns.", "Escolha parâmetros diferentes para linhas e colunas."),
                    );
                }
                ui.horizontal(|ui| {
                    let r = ui.add_enabled(!self.safety_active() && cells <= max && d.rows.param != d.cols.param, egui::Button::new(t("Create", "Criar")));
                    create = r.clicked();
                    tracked.push((r, "Create".into()));
                    let r = ui.button(t("Cancel", "Cancelar"));
                    cancel = r.clicked();
                    tracked.push((r, "Cancel".into()));
                });
            });
        for (r, label) in tracked {
            let kind = match label.as_str() {
                "Create" | "Cancel" => egui::WidgetType::Button,
                "Labels" => egui::WidgetType::Checkbox,
                _ => egui::WidgetType::DragValue,
            };
            self.labeled(&r, kind, &label, false);
        }
        let done = create && self.execute_widget("laserTools.materialTest", d.params());
        self.ui.material_test = if done || cancel || !open { None } else { Some(d) };
    }
}
