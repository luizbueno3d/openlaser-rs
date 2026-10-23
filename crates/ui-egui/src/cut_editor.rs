//! Editable cut-settings draft. One validated engine command commits the entire dialog.
#[derive(Clone)]
pub struct Draft {
    pub mode: String,
    pub speed: f64,
    pub power: f64,
    pub frequency: f64,
    pub pulse: f64,
    pub passes: u16,
    pub interval: f64,
    pub angle: f64,
    pub cross_hatch: bool,
}
impl Draft {
    pub fn from_op(op: &openlaser_doc::CutOp) -> Self {
        Self {
            mode: match op.mode {
                openlaser_doc::CutMode::Line => "line",
                openlaser_doc::CutMode::Fill => "fill",
                openlaser_doc::CutMode::OffsetFill => "offsetFill",
            }
            .into(),
            speed: op.galvo.speed.get(),
            power: op.galvo.power.get(),
            frequency: f64::from(op.galvo.frequency.get()) / 1000.0,
            pulse: op.galvo.q_pulse.map(|v| v.get()).unwrap_or(0.0),
            passes: op.passes,
            interval: op.fill.interval.get(),
            angle: op.fill.angle.get(),
            cross_hatch: op.fill.cross_hatch,
        }
    }
    pub fn fields(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::from_label("Cut mode").selected_text(&self.mode).show_ui(ui, |ui| {
            for mode in ["line", "fill"] {
                ui.selectable_value(&mut self.mode, mode.into(), mode);
            }
        });
        for (label, value) in [
            ("Speed · mm/s", &mut self.speed),
            ("Power · %", &mut self.power),
            ("Frequency · kHz", &mut self.frequency),
            ("Q-Pulse Width · ns", &mut self.pulse),
            ("Line Interval · mm", &mut self.interval),
            ("Scan Angle · °", &mut self.angle),
        ] {
            ui.horizontal(|ui| {
                let label = ui.label(label);
                ui.add(egui::DragValue::new(value).speed(0.1)).labelled_by(label.id);
            });
        }
        ui.horizontal(|ui| {
            let label = ui.label("Passes");
            ui.add(egui::DragValue::new(&mut self.passes)).labelled_by(label.id);
        });
        ui.checkbox(&mut self.cross_hatch, "Cross-hatch");
    }
    pub fn params(&self, layer: usize) -> serde_json::Value {
        serde_json::json!({"layer":layer,"mode":self.mode,"passes":self.passes,"galvo":{"speedMmS":self.speed,"powerPct":self.power,"frequencyKhz":self.frequency,"qPulseNs":if self.pulse == 0.0 {None} else {Some(self.pulse)}},"fill":{"intervalMm":self.interval,"angleDeg":self.angle,"crossHatch":self.cross_hatch}})
    }
}
