//! The Cut Settings Editor's draft (UI §6, CAT §6.1–§6.2): the P0 galvo/MOPA fields of one colour layer. Nothing is
//! applied while editing; OK dispatches ONE `layer.cutSettings.set` command, so the whole dialog is one undo step.
//! Values are never clamped: a value the machine cannot run is a refusal from the planner (live in the Laser panel),
//! and a value that is not a number at all keeps OK disabled with the reason.

use openlaser_doc::{ColorLayer, CutMode};
use openlaser_types::LayerKey;

#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub layer: LayerKey,
    pub name: String,
    pub output: bool,
    pub mode: CutMode,
    /// mm/s requested (the actual encoded speed is shown next to it when the job plans).
    pub speed: f64,
    /// Max Power, %.
    pub power: f64,
    /// kHz (stored in the project as Hz).
    pub frequency_khz: f64,
    /// Q-Pulse Width, ns; None = the source's default (no Q-pulse word).
    pub q_pulse_ns: Option<f64>,
    pub passes: u16,
    /// Line Interval, mm (Fill).
    pub interval: f64,
    /// Scan Angle, ° (Fill).
    pub angle: f64,
    /// Angle Increment per pass, ° (Fill).
    pub angle_increment: f64,
    pub bidirectional: bool,
    pub cross_hatch: bool,
}

/// What the editor needs from the machine profile.
pub struct Limits<'a> {
    pub pulse_table_ns: &'a [u16],
    pub freq_khz: [f64; 2],
    pub max_power_pct: f64,
    /// Decoded actual speed of this layer from the plan (PLAN D9), if the job plans.
    pub actual_speed: Option<f64>,
    /// The machine is active: the editor is read-only (STOP stays above it).
    pub read_only: bool,
}

impl Draft {
    pub fn from_layer(layer: &ColorLayer) -> Option<Self> {
        let op = layer.ops.first()?;
        Some(Self {
            layer: LayerKey::Color(layer.index),
            name: layer.name.clone(),
            output: layer.output,
            mode: op.mode,
            speed: op.galvo.speed.get(),
            power: op.galvo.power.get(),
            frequency_khz: f64::from(op.galvo.frequency.get()) / 1000.0,
            q_pulse_ns: op.galvo.q_pulse.map(|v| v.get()),
            passes: op.passes,
            interval: op.fill.interval.get(),
            angle: op.fill.angle.get(),
            angle_increment: op.fill.angle_increment.get(),
            bidirectional: op.fill.bidirectional,
            cross_hatch: op.fill.cross_hatch,
        })
    }

    /// Shape checks only (finite numbers, positive speed/interval, at least one pass). Machine limits are the
    /// planner's job and show up as refusals, never as a silent clamp.
    pub fn validate(&self) -> Result<(), String> {
        let finite = [
            ("Speed", self.speed),
            ("Power", self.power),
            ("Frequency", self.frequency_khz),
            ("Line Interval", self.interval),
            ("Scan Angle", self.angle),
            ("Angle Increment", self.angle_increment),
        ];
        if let Some((name, _)) = finite.iter().find(|(_, v)| !v.is_finite()) {
            return Err(format!("{name} is not a number."));
        }
        if self.speed <= 0.0 {
            return Err("Speed must be above 0 mm/s.".into());
        }
        if self.power < 0.0 {
            return Err("Power cannot be negative.".into());
        }
        if self.frequency_khz <= 0.0 {
            return Err("Frequency must be above 0 kHz.".into());
        }
        if self.passes == 0 {
            return Err("Passes must be at least 1.".into());
        }
        if self.mode != CutMode::Line && self.interval <= 0.0 {
            return Err("Line Interval must be above 0 mm.".into());
        }
        if self.q_pulse_ns.is_some_and(|ns| !ns.is_finite() || ns <= 0.0) {
            return Err("Q-Pulse Width must be above 0 ns.".into());
        }
        Ok(())
    }

    /// `layer.cutSettings.set` params (SPEC §4.3 row shape).
    pub fn params(&self) -> serde_json::Value {
        serde_json::json!({
            "layer": self.layer.to_string(),
            "name": self.name,
            "output": self.output,
            "mode": match self.mode { CutMode::Line => "line", CutMode::Fill => "fill", CutMode::OffsetFill => "offset_fill" },
            "passes": self.passes,
            "galvo": { "speedMmS": self.speed, "powerPct": self.power, "frequencyKhz": self.frequency_khz, "qPulseNs": self.q_pulse_ns },
            "fill": {
                "intervalMm": self.interval,
                "angleDeg": self.angle,
                "angleIncrementDeg": self.angle_increment,
                "bidirectional": self.bidirectional,
                "crossHatch": self.cross_hatch,
            },
        })
    }

    /// Sets one field by its wire name (`ui.dialog.set {dialog: "cutSettings", field, value}`).
    pub fn set_field(&mut self, field: &str, value: &serde_json::Value) -> Result<(), String> {
        let number = || value.as_f64().ok_or_else(|| format!("{field} must be a number"));
        match field {
            "name" => self.name = value.as_str().ok_or("name must be text")?.chars().take(64).collect(),
            "output" => self.output = value.as_bool().ok_or("output must be true or false")?,
            "mode" => {
                self.mode = match value.as_str() {
                    Some("line") => CutMode::Line,
                    Some("fill") => CutMode::Fill,
                    _ => return Err("mode must be line or fill (Offset Fill is P1)".into()),
                }
            }
            "speedMmS" => self.speed = number()?,
            "powerPct" => self.power = number()?,
            "frequencyKhz" => self.frequency_khz = number()?,
            "qPulseNs" => self.q_pulse_ns = if value.is_null() { None } else { Some(number()?) },
            "passes" => self.passes = value.as_u64().and_then(|v| u16::try_from(v).ok()).ok_or("passes must be an integer from 1 to 65535")?,
            "intervalMm" => self.interval = number()?,
            "angleDeg" => self.angle = number()?,
            "angleIncrementDeg" => self.angle_increment = number()?,
            "bidirectional" => self.bidirectional = value.as_bool().ok_or("bidirectional must be true or false")?,
            "crossHatch" => self.cross_hatch = value.as_bool().ok_or("crossHatch must be true or false")?,
            _ => return Err(format!("unknown cut settings field `{field}`")),
        }
        Ok(())
    }

    /// Draws the fields; returns each interactive widget with its AccessKit label (for `ui.elements`).
    pub fn fields(&mut self, ui: &mut egui::Ui, limits: &Limits<'_>) -> Vec<(egui::Response, String)> {
        let mut out = Vec::new();
        let enabled = !limits.read_only;
        let mut add = |ui: &mut egui::Ui, label: &str, response: egui::Response, hint: Option<String>| {
            let text = label.to_owned();
            let on = response.enabled();
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, on, text.clone()));
            let response = match hint {
                Some(hint) => response.on_hover_text(hint),
                None => response,
            };
            let _ = ui;
            out.push((response, label.to_owned()));
        };
        egui::Grid::new("cut-settings-fields").num_columns(3).spacing([10.0, 6.0]).show(ui, |ui| {
            crate::panels::caption(ui, "Layer");
            ui.label(format!("{} ({})", self.name, self.layer));
            ui.end_row();

            crate::panels::caption(ui, "Output");
            let r = ui.add_enabled(enabled, egui::Checkbox::without_text(&mut self.output));
            add(ui, "Output", r, None);
            ui.end_row();

            crate::panels::caption(ui, "Mode");
            let mode_text = match self.mode {
                CutMode::Line => "Line",
                CutMode::Fill => "Fill",
                CutMode::OffsetFill => "Offset Fill",
            };
            let r = ui
                .add_enabled_ui(enabled, |ui| {
                    egui::ComboBox::from_id_salt("cut-mode")
                        .selected_text(mode_text)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.mode, CutMode::Line, "Line");
                            ui.selectable_value(&mut self.mode, CutMode::Fill, "Fill");
                        })
                        .response
                })
                .inner;
            add(ui, "Mode", r, None);
            ui.end_row();

            crate::panels::caption(ui, "Speed (mm/s)");
            let r = ui.add_enabled(enabled, egui::DragValue::new(&mut self.speed).speed(1.0).max_decimals(2));
            add(ui, "Speed (mm/s)", r, None);
            match limits.actual_speed {
                Some(actual) => ui.label(format!("actual {actual:.2} mm/s")),
                None => ui.weak("actual speed shows once the job plans"),
            };
            ui.end_row();

            crate::panels::caption(ui, "Max Power (%)");
            let r = ui.add_enabled(enabled, egui::DragValue::new(&mut self.power).speed(0.5).max_decimals(1));
            add(ui, "Max Power (%)", r, None);
            ui.weak(format!("source max {} %", limits.max_power_pct));
            ui.end_row();

            crate::panels::caption(ui, "Frequency (kHz)");
            let r = ui.add_enabled(enabled, egui::DragValue::new(&mut self.frequency_khz).speed(0.5).max_decimals(3));
            add(ui, "Frequency (kHz)", r, None);
            ui.weak(format!("allowed {} – {} kHz", limits.freq_khz[0], limits.freq_khz[1]));
            ui.end_row();

            crate::panels::caption(ui, "Q-Pulse Width (ns)");
            let current = match self.q_pulse_ns {
                None => "Default".to_owned(),
                Some(ns) if limits.pulse_table_ns.iter().any(|t| (f64::from(*t) - ns).abs() < 1e-9) => format!("{ns}"),
                Some(ns) => format!("{ns} (not in the pulse table)"),
            };
            let r = ui
                .add_enabled_ui(enabled, |ui| {
                    egui::ComboBox::from_id_salt("cut-q-pulse")
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.q_pulse_ns, None, "Default");
                            for ns in limits.pulse_table_ns {
                                ui.selectable_value(&mut self.q_pulse_ns, Some(f64::from(*ns)), format!("{ns} ns"));
                            }
                        })
                        .response
                })
                .inner;
            add(ui, "Q-Pulse Width (ns)", r, Some("The source's pulse table (EZCAD: Q pulse width)".into()));
            ui.weak("MOPA pulse table");
            ui.end_row();

            crate::panels::caption(ui, "Passes");
            let r = ui.add_enabled(enabled, egui::DragValue::new(&mut self.passes).speed(0.1));
            add(ui, "Passes", r, None);
            ui.end_row();

            if self.mode != CutMode::Line {
                crate::panels::caption(ui, "Line Interval (mm)");
                let r = ui.add_enabled(enabled, egui::DragValue::new(&mut self.interval).speed(0.001).max_decimals(4));
                add(ui, "Line Interval (mm)", r, None);
                if self.interval > 0.0 {
                    ui.weak(format!("{:.0} LPI", 25.4 / self.interval));
                } else {
                    ui.weak("—");
                }
                ui.end_row();

                crate::panels::caption(ui, "Scan Angle (°)");
                let r = ui.add_enabled(enabled, egui::DragValue::new(&mut self.angle).speed(1.0));
                add(ui, "Scan Angle (°)", r, None);
                ui.end_row();

                crate::panels::caption(ui, "Angle Increment (°)");
                let r = ui.add_enabled(enabled, egui::DragValue::new(&mut self.angle_increment).speed(1.0));
                add(ui, "Angle Increment (°)", r, Some("Added to the scan angle on each pass".into()));
                ui.end_row();

                crate::panels::caption(ui, "Bi-directional Fill");
                let r = ui.add_enabled(enabled, egui::Checkbox::without_text(&mut self.bidirectional));
                add(ui, "Bi-directional Fill", r, None);
                ui.end_row();

                crate::panels::caption(ui, "Cross-Hatch");
                let r = ui.add_enabled(enabled, egui::Checkbox::without_text(&mut self.cross_hatch));
                add(ui, "Cross-Hatch", r, Some("A second hatch at +90° (EZCAD: hatch 2 at +90°)".into()));
                ui.end_row();
            }
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer() -> ColorLayer {
        let defaults = openlaser_machine::jpt_m7_200_sim().layer_defaults;
        ColorLayer {
            index: 3,
            name: "C03".into(),
            output: true,
            show: true,
            ops: vec![openlaser_doc::CutOp {
                name: "Fill".into(),
                enabled: true,
                mode: CutMode::Fill,
                passes: 2,
                galvo: defaults.galvo.clone(),
                gantry: defaults.gantry.clone(),
                fill: defaults.fill.clone(),
            }],
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

    #[test]
    fn every_p0_field_reaches_the_single_command() {
        let mut draft = Draft::from_layer(&layer()).unwrap();
        for (field, value) in [
            ("speedMmS", serde_json::json!(1234.5)),
            ("powerPct", serde_json::json!(55)),
            ("frequencyKhz", serde_json::json!(60)),
            ("qPulseNs", serde_json::json!(200)),
            ("passes", serde_json::json!(3)),
            ("intervalMm", serde_json::json!(0.05)),
            ("angleDeg", serde_json::json!(45)),
            ("angleIncrementDeg", serde_json::json!(90)),
            ("bidirectional", serde_json::json!(false)),
            ("crossHatch", serde_json::json!(true)),
            ("mode", serde_json::json!("fill")),
        ] {
            draft.set_field(field, &value).unwrap();
        }
        let p = draft.params();
        assert_eq!(p["layer"], "C03");
        assert_eq!(p["mode"], "fill");
        assert_eq!(p["passes"], 3);
        assert_eq!(p["galvo"], serde_json::json!({ "speedMmS": 1234.5, "powerPct": 55.0, "frequencyKhz": 60.0, "qPulseNs": 200.0 }));
        assert_eq!(
            p["fill"],
            serde_json::json!({ "intervalMm": 0.05, "angleDeg": 45.0, "angleIncrementDeg": 90.0, "bidirectional": false, "crossHatch": true })
        );
        assert!(draft.set_field("mode", &serde_json::json!("offset_fill")).is_err(), "Offset Fill is P1");
        assert!(draft.set_field("force", &serde_json::json!(true)).is_err());
    }

    #[test]
    fn non_numbers_keep_ok_disabled_and_nothing_is_clamped() {
        let mut draft = Draft::from_layer(&layer()).unwrap();
        assert_eq!(draft.validate(), Ok(()));
        draft.speed = f64::NAN;
        assert!(draft.validate().unwrap_err().contains("Speed"));
        draft.speed = 0.0;
        assert!(draft.validate().is_err());
        draft.speed = 1e9; // above the machine's max: the planner refuses it; the editor does not clamp it
        assert_eq!(draft.validate(), Ok(()));
        assert_eq!(draft.params()["galvo"]["speedMmS"], 1e9);
        draft.passes = 0;
        assert!(draft.validate().is_err());
    }
}
