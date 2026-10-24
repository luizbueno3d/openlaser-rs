//! Dialogs and floating windows (UI §4.10, §4.11, §6, §7.4): the Cut Settings Editor (a Modal, read-only while a
//! machine is active, STOP above it), Settings (with the human-only Allow agent connections switch), Device
//! Settings (Save is human-only), Live Framing, Preview, and the ARM confirmation drawn by `services.human`.

use openlaser_types::LayerKey;

use crate::app::{OpenLaserApp, PlanView};

impl OpenLaserApp {
    pub(crate) fn dialogs(&mut self, ctx: &egui::Context) {
        if self.ui.cut_settings_open {
            self.cut_settings_dialog(ctx);
        } else {
            self.cut_draft = None;
        }
        if self.ui.settings_open {
            self.settings_window(ctx);
        }
        if self.ui.device_settings_open {
            self.device_settings_window(ctx);
        } else {
            self.device_draft = None;
        }
        if self.ui.live_framing_open {
            self.live_framing_window(ctx);
        }
        if self.ui.preview_open {
            self.preview_window(ctx);
        }
    }

    /// Opens the editor's draft for the active colour layer (None for a tool layer or a layer without settings).
    pub(crate) fn open_cut_draft(&mut self) {
        let LayerKey::Color(i) = self.ui.active_layer else {
            self.cut_draft = None;
            return;
        };
        self.cut_draft = self.project().and_then(|p| p.color_layer(i)).and_then(crate::cut_editor::Draft::from_layer);
    }

    fn cut_settings_dialog(&mut self, ctx: &egui::Context) {
        if self.cut_draft.as_ref().is_none_or(|d| d.layer != self.ui.active_layer) {
            self.open_cut_draft();
        }
        let galvo = self.active_machine().and_then(|m| match &m.kind {
            openlaser_machine::Kinematics::Galvo(g) => Some(g.source.clone()),
            openlaser_machine::Kinematics::Gantry(_) => None,
        });
        let actual = self.actual_speed(self.ui.active_layer);
        let read_only = self.ui.machine_active;
        let mut ok = false;
        let mut cancel = false;
        let mut tracked = Vec::new();
        let modal = egui::Modal::new(egui::Id::new("cut-settings-modal")).show(ctx, |ui| {
            ui.set_min_width(460.0);
            ui.heading(format!("Cut Settings Editor: {}", self.layer_name(self.ui.active_layer)));
            if read_only {
                ui.colored_label(crate::panels::STOP_RED, "Read-only while a machine is active. Stop laser is above this dialog.");
            }
            match (&mut self.cut_draft, &galvo) {
                (Some(draft), Some(source)) => {
                    let limits = crate::cut_editor::Limits {
                        pulse_table_ns: &source.pulse_table_ns,
                        freq_khz: source.freq_khz,
                        max_power_pct: source.max_power_pct,
                        actual_speed: actual,
                        read_only,
                    };
                    tracked = draft.fields(ui, &limits);
                }
                (Some(_), None) => {
                    ui.label("Gantry cut settings arrive with the gantry editor (M4); G-code export uses the layer's gantry block.");
                }
                (None, _) => {
                    ui.label("Pick a colour layer that is in use to edit its settings (tool layers T1/T2 have none).");
                }
            }
            let refusals: Vec<String> = self
                .plan()
                .map(|p| p.refusals().iter().filter(|r| r.layer == Some(self.ui.active_layer)).map(|r| format!("{}: {}", r.code.as_str(), r.message)).collect())
                .unwrap_or_default();
            if refusals.is_empty() {
                ui.weak("refusals: none");
            } else {
                for line in &refusals {
                    ui.colored_label(crate::panels::STOP_RED, line);
                }
            }
            ui.separator();
            let valid = match &self.cut_draft {
                None => Err("Nothing to apply.".to_owned()),
                Some(_) if galvo.is_none() => Err("Not editable for this device yet.".to_owned()),
                Some(_) if read_only => Err("Read-only while a machine is active.".to_owned()),
                Some(draft) => draft.validate(),
            };
            ui.horizontal(|ui| {
                let c = ui.button("Cancel");
                cancel = c.clicked();
                tracked.push((c, "Cancel".into()));
                let o = ui.add_enabled(valid.is_ok(), egui::Button::new("OK"));
                ok = o.clicked();
                let o = match &valid {
                    Err(reason) => o.on_disabled_hover_text(reason.as_str()),
                    Ok(()) => o.on_hover_text("Apply every change as one undo step"),
                };
                tracked.push((o, "OK".into()));
            });
            if let Err(reason) = &valid {
                ui.weak(reason.as_str());
            }
            ui.label(&self.ui.status_message);
        });
        for (response, label) in &tracked {
            self.track(response, "input", label.clone(), false);
        }
        if ok && let Some(draft) = &self.cut_draft {
            let params = draft.params();
            self.frame_synthetic = self.services.human.frame_is_synthetic(ctx);
            let origin = self.origin();
            #[cfg(test)]
            self.seam.dispatched.push((origin, "layer.cutSettings.set".into(), params.clone()));
            match self.session.execute_as(origin, "layer.cutSettings.set", params) {
                Ok(_) => {
                    self.ui.cut_settings_open = false;
                    self.cut_draft = None;
                    self.set_status("Cut settings applied");
                }
                Err(e) => self.set_status(e.to_string()),
            }
        }
        if cancel || (modal.should_close() && !ok) {
            self.ui.cut_settings_open = false;
            self.cut_draft = None;
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        egui::Window::new("Settings").open(&mut open).collapsible(false).show(ctx, |ui| {
            ui.heading("Automation");
            let mut on = self.services.automation.info().is_some();
            let r = ui.checkbox(&mut on, "Allow agent connections");
            self.track(&r, "checkBox", "Allow agent connections", true);
            // Human-only (§4.6): only a real click in an untainted frame changes automation access.
            let human = self.services.human.is_human_click(ctx, &r);
            if human && !self.frame_synthetic && !self.services.human.frame_is_synthetic(ctx) {
                match self.services.automation.set(on) {
                    Ok(Some(info)) => self.set_status(format!("Agents may connect on 127.0.0.1:{}", info.port)),
                    Ok(None) => self.set_status("Agent connections are off"),
                    Err(e) => self.set_status(format!("Automation: {e}")),
                }
            } else if r.clicked() {
                self.set_status("Allow agent connections needs a person's click in this window.");
            }
            match self.services.automation.info() {
                Some(info) => ui.label(format!("Listening on 127.0.0.1:{} · {} client(s)", info.port, info.clients.len())),
                None => ui.weak("Agents cannot connect. Turn this on to let openlaser-cli mcp --bridge auto drive this window."),
            };
            ui.separator();
            ui.heading("Window");
            let mut menus = self.in_window_menus;
            let r = ui.checkbox(&mut menus, "Menus inside the window");
            self.track(&r, "checkBox", "Menus inside the window", false);
            if r.changed() {
                self.in_window_menus = menus;
                let mut settings = self.services.settings.load();
                settings.in_window_menus = menus;
                self.services.settings.save(&settings);
            }
        });
        self.ui.settings_open = open;
    }

    fn device_settings_window(&mut self, ctx: &egui::Context) {
        if self.device_draft.is_none() {
            self.device_draft = self.active_machine().cloned();
        }
        let mut open = true;
        let mut save = false;
        let read_only = self.ui.machine_active;
        egui::Window::new("Device Settings").open(&mut open).collapsible(false).show(ctx, |ui| {
            let mut tracked: Vec<(egui::Response, String)> = Vec::new();
            match &mut self.device_draft {
                Some(profile) => {
                    ui.strong(&profile.name);
                    if let openlaser_machine::Kinematics::Galvo(g) = &mut profile.kind {
                        ui.label(format!(
                            "Field Size {} mm (encoding) × scale → encodable ±{:.2} mm",
                            g.encoding_field.get(),
                            g.encoding_field.get() * g.cal.scale[0] / 2.0
                        ));
                        egui::Grid::new("device-fields").num_columns(3).show(ui, |ui| {
                            for (label, values) in [
                                ("Work Area min (mm)", &mut g.work_area.min),
                                ("Work Area max (mm)", &mut g.work_area.max),
                                ("Field Offset (mm)", &mut g.cal.offset_mm),
                                ("Field Scale", &mut g.cal.scale),
                                ("Red Dot Offset (mm)", &mut g.red_dot.offset_mm),
                                ("Red Dot Scale", &mut g.red_dot.scale),
                            ] {
                                crate::panels::caption(ui, label);
                                for (axis, value) in values.iter_mut().enumerate() {
                                    let r = ui.add_enabled(!read_only, egui::DragValue::new(value).speed(0.01).max_decimals(4));
                                    let name = format!("{label} {}", if axis == 0 { "X" } else { "Y" });
                                    let n = name.clone();
                                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, !read_only, n.clone()));
                                    tracked.push((r, name));
                                }
                                ui.end_row();
                            }
                            crate::panels::caption(ui, "Field Angle (°)");
                            let r = ui.add_enabled(!read_only, egui::DragValue::new(&mut g.cal.angle_deg).speed(0.1));
                            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, !read_only, "Field Angle (°)"));
                            tracked.push((r, "Field Angle (°)".into()));
                            ui.end_row();
                        });
                        let mut galvo_x = !g.cal.swap_xy;
                        let r = ui.add_enabled(!read_only, egui::Checkbox::new(&mut galvo_x, "Galvo 1 is X Axis"));
                        g.cal.swap_xy = !galvo_x;
                        tracked.push((r, "Galvo 1 is X Axis".into()));
                        for (axis, negate) in g.cal.negate.iter_mut().enumerate() {
                            let label = format!("Reverse {} direction", if axis == 0 { "X" } else { "Y" });
                            let r = ui.add_enabled(!read_only, egui::Checkbox::new(negate, label.as_str()));
                            tracked.push((r, label));
                        }
                        let mut always = true;
                        let r = ui.add_enabled(false, egui::Checkbox::new(&mut always, "Require framing before start"));
                        let r = r.on_disabled_hover_text("Always required on galvos (D8).");
                        tracked.push((r, "Require framing before start".into()));
                        ui.weak(format!("Laser source: pulse table {:?} ns · {}–{} kHz", g.source.pulse_table_ns, g.source.freq_khz[0], g.source.freq_khz[1]));
                    } else {
                        ui.label("Gantry device settings arrive with live GRBL (M4).");
                    }
                }
                None => {
                    ui.label("No device profile.");
                }
            }
            ui.separator();
            let r = ui.add_enabled(self.device_draft.is_some() && !read_only, egui::Button::new("Save device settings"));
            let human = self.services.human.is_human_click(ctx, &r);
            save = human && !self.frame_synthetic && !self.services.human.frame_is_synthetic(ctx);
            if r.clicked() && !save {
                self.set_status("Saving a machine profile needs a person's click in this window.");
            }
            tracked.push((r, "Save device settings".into()));
            ui.label(&self.ui.status_message);
            for (response, label) in tracked {
                let human_only = label == "Save device settings";
                self.track(&response, "input", label, human_only);
            }
        });
        if save && let Some(profile) = self.device_draft.clone() {
            match openlaser_machine::validate_profile(&profile) {
                Err(errors) => self.set_status(errors.join("; ")),
                Ok(()) => {
                    let params = match &profile.kind {
                        openlaser_machine::Kinematics::Galvo(g) => {
                            serde_json::json!({ "device": profile.id.0, "workArea": g.work_area, "cal": g.cal, "redDot": g.red_dot })
                        }
                        openlaser_machine::Kinematics::Gantry(_) => serde_json::json!({ "device": profile.id.0 }),
                    };
                    match self.session.execute_as(openlaser_engine::Origin::Human, "device.settings.set", params) {
                        Ok(_) => {
                            open = false;
                            self.set_status("Device settings saved");
                        }
                        Err(e) => self.set_status(e.to_string()),
                    }
                }
            }
        }
        self.ui.device_settings_open = open;
    }

    fn live_framing_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        egui::Window::new("Live Framing").open(&mut open).collapsible(false).show(ctx, |ui| {
            ui.label("Red light only: the galvo traces the job; the laser never fires while framing.");
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.ui.live_style, openlaser_types::FrameStyle::Bounds, "Bounds");
                ui.radio_value(&mut self.ui.live_style, openlaser_types::FrameStyle::Contour, "Contour");
            });
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.ui.live_source, openlaser_types::FrameSource::Job, "Job");
                ui.radio_value(&mut self.ui.live_source, openlaser_types::FrameSource::ToolLayersOnly, "Tool Layers Only");
                ui.radio_value(&mut self.ui.live_source, openlaser_types::FrameSource::JobAndToolLayers, "Job and Tool Layers");
            });
            let gate = crate::panels::frame_gate(self.plan(), self.ui.machine_active);
            let start = ui.add_enabled(gate.is_ok(), egui::Button::new("Start Framing"));
            self.track(&start, "button", "Start Framing", true);
            let human = self.services.human.is_human_click(ctx, &start);
            if human && gate.is_ok() && !self.frame_synthetic && !self.services.human.frame_is_synthetic(ctx) {
                #[cfg(test)]
                self.seam.human_actions.push("Start Framing");
                let (style, source) = (self.ui.live_style, self.ui.live_source);
                let message = match self.session.start_frame(openlaser_engine::Origin::Human, openlaser_types::FrameMode::Loop, Some(style), Some(source)) {
                    Ok(_) => "Framing".to_owned(),
                    Err(e) => e.to_string(),
                };
                self.set_status(message);
            } else if start.clicked() {
                self.set_status("Framing is human-only: it needs a person's click in this window.");
            }
            if let Err(reason) = gate {
                start.on_disabled_hover_text(reason);
            }
            let stop = ui.button("Stop Framing");
            self.track(&stop, "button", "Stop Framing", false);
            if stop.clicked() {
                self.stop();
            }
        });
        self.ui.live_framing_open = open;
    }

    fn preview_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let plan = self.plan().cloned();
        egui::Window::new("Preview").open(&mut open).show(ctx, |ui| match &plan {
            Some(PlanView::Ready(summary)) => {
                ui.label(format!("Estimate {} · {} packet(s) · repeats {}", crate::panels::mmss(summary.estimate), summary.packets, summary.repeats));
                egui::Grid::new("preview-rows").striped(true).show(ui, |ui| {
                    for heading in ["Layer", "Pass", "Mode", "Speed (actual)", "Power", "kHz", "ns", "Paths"] {
                        ui.strong(heading);
                    }
                    ui.end_row();
                    for row in &summary.rows {
                        ui.label(&row.layer_name);
                        ui.label(format!("{}/{}", row.pass, row.of));
                        ui.label(format!("{:?}", row.mode));
                        ui.label(format!("{:.2} mm/s", row.speed_mm_s));
                        ui.label(format!("{:.1} %", row.power_pct));
                        ui.label(row.frequency_khz.map(|k| format!("{k:.2}")).unwrap_or_else(|| "—".into()));
                        ui.label(row.pulse_ns.map(|n| n.to_string()).unwrap_or_else(|| "—".into()));
                        ui.label(row.paths.to_string());
                        ui.end_row();
                    }
                });
            }
            Some(PlanView::Refused(refused)) => {
                for r in &refused.refusals {
                    ui.label(format!("{}: {}", r.code.as_str(), r.message));
                }
            }
            None => {
                ui.label("Open a project to preview its plan.");
            }
        });
        self.ui.preview_open = open;
    }

    /// The ARM confirmation (§3.20) is drawn by `services.human` as a Modal from `PendingArm` only; STOP stays above.
    pub(crate) fn draw_arm_dialog(&mut self, ctx: &egui::Context) {
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
                    self.set_status(e.to_string());
                }
            }
        }
    }
}
