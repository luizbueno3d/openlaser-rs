//! Dialogs and floating windows (UI §4.10, §4.11, §6, §7.4): the Cut Settings Editor (a Modal, read-only while a
//! machine is active, STOP above it), Settings (with the human-only Allow agent connections switch), Device
//! Settings (Save is human-only), Live Framing, Preview, and the ARM confirmation drawn by `services.human`.

use openlaser_types::LayerKey;

use crate::app::{OpenLaserApp, PlanView};
use crate::i18n::t;

impl OpenLaserApp {
    /// The Offset Shapes dialog (LightBurn's): distance, direction, corner style and delete-original, with a live
    /// ghost on the canvas (view.rs paints it while this is open). OK dispatches exactly one tools.offsetShapes.
    fn offset_dialog(&mut self, ctx: &egui::Context) {
        let Some(o) = self.ui.offset else { return };
        let mut o = o;
        let mut open = true;
        let mut ok = false;
        let mut cancel = false;
        egui::Window::new(t("Offset Shapes", "Deslocar formas"))
            .id(egui::Id::new("offset_shapes_window"))
            .open(&mut open)
            .collapsible(false)
            .auto_sized()
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(t("Offset distance", "Distância do deslocamento"));
                    let r = ui.add(egui::DragValue::new(&mut o.distance).speed(0.1).range(0.001..=1000.0).suffix(" mm"));
                    self.labeled(&r, egui::WidgetType::DragValue, "Offset distance", false);
                });
                ui.horizontal(|ui| {
                    ui.label(t("Direction", "Direção"));
                    egui::ComboBox::from_id_salt("offset-direction")
                        .selected_text(
                            [t("Outward", "Para fora"), t("Inward", "Para dentro"), t("Both", "Ambos")]
                                .get(usize::from(o.direction))
                                .copied()
                                .unwrap_or(t("Outward", "Para fora")),
                        )
                        .show_ui(ui, |ui| {
                            for (i, name) in [t("Outward", "Para fora"), t("Inward", "Para dentro"), t("Both", "Ambos")].iter().enumerate() {
                                if ui.selectable_label(o.direction as usize == i, *name).clicked() {
                                    o.direction = i as u8;
                                }
                            }
                        });
                    ui.label(t("Corner", "Canto"));
                    egui::ComboBox::from_id_salt("offset-corner")
                        .selected_text(
                            [t("Round", "Arredondado"), t("Sharp", "Vivo"), t("Bevel", "Chanfrado")]
                                .get(usize::from(o.corner))
                                .copied()
                                .unwrap_or(t("Round", "Arredondado")),
                        )
                        .show_ui(ui, |ui| {
                            for (i, name) in [t("Round", "Arredondado"), t("Sharp", "Vivo"), t("Bevel", "Chanfrado")].iter().enumerate() {
                                if ui.selectable_label(o.corner as usize == i, *name).clicked() {
                                    o.corner = i as u8;
                                }
                            }
                        });
                });
                let r = ui.checkbox(&mut o.delete_original, t("Delete original shapes", "Excluir formas originais"));
                self.labeled(&r, egui::WidgetType::Checkbox, "Delete original shapes", false);
                ui.horizontal(|ui| {
                    let ok_btn = ui.add(egui::Button::new("OK"));
                    self.labeled(&ok_btn, egui::WidgetType::Button, "OK", false);
                    if ok_btn.clicked() {
                        ok = true;
                    }
                    let cancel_btn = ui.add(egui::Button::new(t("Cancel", "Cancelar")));
                    self.labeled(&cancel_btn, egui::WidgetType::Button, "Cancel", false);
                    if cancel_btn.clicked() {
                        cancel = true;
                    }
                });
            });
        self.ui.offset = Some(o);
        if ok {
            self.execute_widget("tools.offsetShapes", o.params());
        }
        if ok || !open || cancel {
            self.ui.offset = None;
        }
    }

    pub(crate) fn dialogs(&mut self, ctx: &egui::Context) {
        if self.ui.cut_settings_open {
            self.cut_settings_dialog(ctx);
        } else {
            self.cut_draft = None;
        }
        if self.ui.settings_open {
            self.settings_window(ctx);
        }
        self.material_test_dialog(ctx);
        self.offset_dialog(ctx);
        self.trace_dialog(ctx);
        if self.ui.device_settings_open {
            self.device_settings_window(ctx);
        } else {
            self.device_draft = None;
            self.device_draft_source = None;
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
            ui.heading(crate::tf!("Cut Settings Editor: {}", "Editor de configurações de corte: {}", self.layer_name(self.ui.active_layer)));
            if read_only {
                ui.colored_label(
                    crate::panels::STOP_RED,
                    t(
                        "Read-only while a machine is active. Stop laser is above this dialog.",
                        "Somente leitura enquanto uma máquina está ativa. Parar laser fica acima desta janela.",
                    ),
                );
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
                    ui.label(t(
                        "Gantry cut settings arrive with the gantry editor (M4); G-code export uses the layer's gantry block.",
                        "As configurações de corte de gantry chegam com o editor de gantry (M4); a exportação de G-code usa o bloco gantry da camada.",
                    ));
                }
                (None, _) => {
                    ui.label(t(
                        "Pick a colour layer that is in use to edit its settings (tool layers T1/T2 have none).",
                        "Escolha uma camada de cor em uso para editar suas configurações (as camadas de ferramenta T1/T2 não têm).",
                    ));
                }
            }
            let refusals: Vec<String> = self
                .plan()
                .map(|p| p.refusals().iter().filter(|r| r.layer == Some(self.ui.active_layer)).map(crate::i18n::refusal).collect())
                .unwrap_or_default();
            if refusals.is_empty() {
                ui.weak(t("refusals: none", "recusas: nenhuma"));
            } else {
                for line in &refusals {
                    ui.colored_label(crate::panels::STOP_RED, line);
                }
            }
            ui.separator();
            let valid = match &self.cut_draft {
                None => Err(t("Nothing to apply.", "Nada para aplicar.").to_owned()),
                Some(_) if galvo.is_none() => Err(t("Not editable for this device yet.", "Ainda não editável para este dispositivo.").to_owned()),
                Some(_) if read_only => Err(t("Read-only while a machine is active.", "Somente leitura enquanto uma máquina está ativa.").to_owned()),
                Some(draft) => draft.validate(),
            };
            ui.horizontal(|ui| {
                let c = ui.button(t("Cancel", "Cancelar"));
                cancel = c.clicked();
                tracked.push((c, "Cancel".into()));
                let o = ui.add_enabled(valid.is_ok(), egui::Button::new("OK"));
                ok = o.clicked();
                let o = match &valid {
                    Err(reason) => o.on_disabled_hover_text(reason.as_str()),
                    Ok(()) => o.on_hover_text(t("Apply every change as one undo step", "Aplica todas as alterações como uma única etapa de desfazer")),
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
                    self.set_status(t("Cut settings applied", "Configurações de corte aplicadas"));
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
        egui::Window::new(crate::i18n::t("Settings", "Configurações")).id(egui::Id::new("settings_window")).open(&mut open).collapsible(false).show(
            ctx,
            |ui| {
                // Bilingual heading so either reader finds it.
                ui.heading("Language / Idioma");
                let current = crate::i18n::language();
                let mut picked = current;
                let combo = egui::ComboBox::from_id_salt("language").selected_text(current.native_name()).show_ui(ui, |ui| {
                    for language in crate::i18n::Language::ALL {
                        ui.selectable_value(&mut picked, language, language.native_name());
                    }
                });
                combo.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Language"));
                if picked != current {
                    self.set_language(picked);
                }
                ui.separator();
                ui.heading(t("Automation", "Automação"));
                let mut on = self.services.automation.info().is_some();
                let r = ui.checkbox(&mut on, t("Allow agent connections", "Permitir conexões de agentes"));
                self.track(&r, "checkBox", "Allow agent connections", true);
                // Human-only (§4.6): only a real click in an untainted frame changes automation access.
                let human = self.services.human.is_human_click(ctx, &r);
                if human && !self.frame_synthetic && !self.services.human.frame_is_synthetic(ctx) {
                    match self.services.automation.set(on) {
                        Ok(Some(info)) => {
                            self.set_status(crate::tf!("Agents may connect on 127.0.0.1:{}", "Agentes podem se conectar em 127.0.0.1:{}", info.port))
                        }
                        Ok(None) => self.set_status(t("Agent connections are off", "Conexões de agentes desativadas")),
                        Err(e) => self.set_status(crate::tf!("Automation: {e}", "Automação: {e}")),
                    }
                } else if r.clicked() {
                    self.set_status(t(
                        "Allow agent connections needs a person's click in this window.",
                        "Permitir conexões de agentes requer o clique de uma pessoa nesta janela.",
                    ));
                }
                match self.services.automation.info() {
                    Some(info) => ui.label(crate::tf!(
                        "Listening on 127.0.0.1:{} · {} client(s)",
                        "Escutando em 127.0.0.1:{} · {} cliente(s)",
                        info.port,
                        info.clients.len()
                    )),
                    None => ui.weak(t(
                        "Agents cannot connect. Turn this on to let openlaser-cli mcp --bridge auto drive this window.",
                        "Agentes não podem se conectar. Ative para permitir que openlaser-cli mcp --bridge auto controle esta janela.",
                    )),
                };
                ui.separator();
                ui.heading("Laser");
                // Human-only like the automation switch: only a person's real click changes how Start arms.
                let mut settings = self.services.settings.load();
                let mut require_frame = settings.require_frame;
                let r = ui.checkbox(&mut require_frame, t("Require framing before Start", "Exigir enquadramento antes de Iniciar"));
                self.track(&r, "checkBox", "Require framing before Start", true);
                let human = self.services.human.is_human_click(ctx, &r);
                if human && !self.frame_synthetic && !self.services.human.frame_is_synthetic(ctx) {
                    settings.require_frame = require_frame;
                    self.session.set_frame_required(require_frame);
                    self.services.settings.save(&settings);
                }
                let mut skip_arm = settings.skip_arm;
                let r = ui.checkbox(&mut skip_arm, t("Skip the ARM confirmation (Start fires at once)", "Pular a confirmação ARM (Iniciar dispara na hora)"));
                self.track(&r, "checkBox", "Skip the ARM confirmation", true);
                let human = self.services.human.is_human_click(ctx, &r);
                if human && !self.frame_synthetic && !self.services.human.frame_is_synthetic(ctx) {
                    settings.skip_arm = skip_arm;
                    self.services.human.set_skip_arm(skip_arm);
                    self.services.settings.save(&settings);
                }
                ui.weak(if settings.skip_arm {
                    t(
                        "Start fires the laser straight away. STOP and the E-stop still stop it.",
                        "Iniciar dispara o laser imediatamente. PARAR e o botão de emergência continuam parando o laser.",
                    )
                } else {
                    t(
                        "You type ARM and tick the checklist once each time OpenLaser opens; after that, Start fires at once.",
                        "Você digita ARM e marca a lista de verificação uma vez a cada abertura do OpenLaser; depois disso, Iniciar dispara na hora.",
                    )
                });
                ui.separator();
                ui.heading(t("Window", "Janela"));
                let mut menus = self.in_window_menus;
                let r = ui.checkbox(&mut menus, t("Menus inside the window", "Menus dentro da janela"));
                self.track(&r, "checkBox", "Menus inside the window", false);
                if r.changed() {
                    self.in_window_menus = menus;
                    let mut settings = self.services.settings.load();
                    settings.in_window_menus = menus;
                    self.services.settings.save(&settings);
                }
            },
        );
        self.ui.settings_open = open;
    }

    fn device_settings_window(&mut self, ctx: &egui::Context) {
        // The draft is only ever mutated by this window's widgets, driven by a person's input: Device
        // Settings is a human-only surface, so synthetic input is dropped while it is open (§4.6). If the
        // profile changed under the open draft (an agent-side device command, a different device selected),
        // the draft is reloaded so Save can never persist something the person did not type.
        let live = self.active_machine().cloned();
        if live != self.device_draft_source {
            if self.device_draft.is_some() {
                self.set_status(t(
                    "The device profile changed; the Device Settings draft was reloaded. Review it before saving.",
                    "O perfil do dispositivo mudou; o rascunho das Configurações do dispositivo foi recarregado. Revise antes de salvar.",
                ));
            }
            self.device_draft = live;
            self.device_draft_source = self.device_draft.clone();
        }
        let mut open = true;
        let mut save = false;
        let read_only = self.ui.machine_active;
        egui::Window::new(t("Device Settings", "Configurações do dispositivo")).id(egui::Id::new("device_settings_window")).open(&mut open).collapsible(false).show(ctx, |ui| {
            let mut tracked: Vec<(egui::Response, String)> = Vec::new();
            egui::ScrollArea::vertical().id_salt("device-settings-scroll").max_height((ctx.content_rect().height() - 180.0).max(160.0)).show(ui, |ui| {
                match &mut self.device_draft {
                    Some(profile) => {
                        ui.strong(&profile.name);
                        if let openlaser_machine::Kinematics::Galvo(g) = &mut profile.kind {
                            ui.label(crate::tf!(
                                "Field Size {} mm (encoding) × scale → encodable ±{:.2} mm",
                                "Tamanho do campo {} mm (codificação) × escala → codificável ±{:.2} mm",
                                g.encoding_field.get(),
                                g.encoding_field.get() * g.cal.scale[0] / 2.0
                            ));
                            egui::Grid::new("device-fields").num_columns(3).show(ui, |ui| {
                                for (label, pt, values) in [
                                    ("Work Area min (mm)", "Área de trabalho mín. (mm)", &mut g.work_area.min),
                                    ("Work Area max (mm)", "Área de trabalho máx. (mm)", &mut g.work_area.max),
                                    ("Field Offset (mm)", "Deslocamento do campo (mm)", &mut g.cal.offset_mm),
                                    ("Field Scale", "Escala do campo", &mut g.cal.scale),
                                    ("Red Dot Offset (mm)", "Deslocamento do ponto vermelho (mm)", &mut g.red_dot.offset_mm),
                                    ("Red Dot Scale", "Escala do ponto vermelho", &mut g.red_dot.scale),
                                ] {
                                    crate::panels::caption(ui, t(label, pt));
                                    for (axis, value) in values.iter_mut().enumerate() {
                                        let r = ui.add_enabled(!read_only, egui::DragValue::new(value).speed(0.01).max_decimals(4));
                                        let name = format!("{label} {}", if axis == 0 { "X" } else { "Y" });
                                        let n = name.clone();
                                        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, !read_only, n.clone()));
                                        tracked.push((r, name));
                                    }
                                    ui.end_row();
                                }
                                crate::panels::caption(ui, t("Field Angle (°)", "Ângulo do campo (°)"));
                                let r = ui.add_enabled(!read_only, egui::DragValue::new(&mut g.cal.angle_deg).speed(0.1));
                                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, !read_only, "Field Angle (°)"));
                                tracked.push((r, "Field Angle (°)".into()));
                                ui.end_row();
                            });
                            let mut galvo_x = !g.cal.swap_xy;
                            let r = ui.add_enabled(!read_only, egui::Checkbox::new(&mut galvo_x, t("Galvo 1 is X Axis", "Galvo 1 é o eixo X")));
                            g.cal.swap_xy = !galvo_x;
                            tracked.push((r, "Galvo 1 is X Axis".into()));
                            for (axis, negate) in g.cal.negate.iter_mut().enumerate() {
                                let axis = if axis == 0 { "X" } else { "Y" };
                                let label = format!("Reverse {axis} direction");
                                let text = crate::tf!("Reverse {axis} direction", "Inverter direção {axis}");
                                let r = ui.add_enabled(!read_only, egui::Checkbox::new(negate, text));
                                tracked.push((r, label));
                            }
                            egui::CollapsingHeader::new(t("Rotary (extended axis)", "Rotativo (eixo estendido)")).id_salt("device-rotary").show(ui, |ui| {
                                // Merely opening settings does not install a config or change legacy profiles.
                                let mut r = g.rotary.clone().unwrap_or_default();
                                let old = r.clone();
                                ui.add_enabled_ui(!read_only, |ui| {
                                    let response = ui.checkbox(&mut r.enabled, t("Enable rotary", "Ativar rotativo"));
                                    tracked.push((response, "Enable rotary".into()));
                                    ui.horizontal(|ui| {
                                        ui.label(t("Drive", "Acionamento"));
                                        ui.selectable_value(&mut r.kind, openlaser_machine::RotaryKind::Chuck, t("Chuck", "Mandril"));
                                        ui.selectable_value(&mut r.kind, openlaser_machine::RotaryKind::Roller, t("Roller", "Roletes"));
                                        ui.label(t("Wrap axis", "Eixo de giro"));
                                        ui.selectable_value(&mut r.axis, openlaser_machine::RotaryAxis::X, "X");
                                        ui.selectable_value(&mut r.axis, openlaser_machine::RotaryAxis::Y, "Y");
                                    });
                                    egui::Grid::new("rotary-fields").num_columns(2).show(ui, |ui| {
                                        for (label, pt, value) in [
                                            ("Drive diameter (mm)", "Diâmetro de acionamento (mm)", &mut r.drive_diameter_mm),
                                            ("Object diameter (mm, layout)", "Diâmetro do objeto (mm, layout)", &mut r.object_diameter_mm),
                                            ("Split (mm)", "Divisão (mm)", &mut r.split_mm),
                                            ("Overlap (mm)", "Sobreposição (mm)", &mut r.overlap_mm),
                                            ("Maximum turns per job", "Máximo de voltas por trabalho", &mut r.max_turns_per_job),
                                        ] {
                                            ui.label(t(label, pt));
                                            let response = ui.add(egui::DragValue::new(value).speed(0.01));
                                            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, !read_only, label));
                                            tracked.push((response, label.into()));
                                            ui.end_row();
                                        }
                                        ui.label(t("Pulses per revolution", "Pulsos por revolução"));
                                        let response = ui.add(egui::DragValue::new(&mut r.pulses_per_rev));
                                        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, !read_only, "Pulses per revolution"));
                                        tracked.push((response, "Pulses per revolution".into()));
                                        ui.end_row();
                                        for (label, pt, value) in [
                                            ("Minimum speed (pulses/s)", "Velocidade mínima (pulsos/s)", &mut r.min_speed_pps),
                                            ("Maximum speed (pulses/s)", "Velocidade máxima (pulsos/s)", &mut r.max_speed_pps),
                                            ("Acceleration time (ms)", "Tempo de aceleração (ms)", &mut r.accel_ms),
                                        ] {
                                            ui.label(t(label, pt));
                                            let response = ui.add(egui::DragValue::new(value));
                                            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, !read_only, label));
                                            tracked.push((response, label.into()));
                                            ui.end_row();
                                        }
                                    });
                                    for (label, pt, value) in [
                                        ("Reverse rotary", "Inverter rotativo", &mut r.reverse),
                                        ("Return rotary to start", "Retornar rotativo ao início", &mut r.return_to_start),
                                    ] {
                                        let response = ui.checkbox(value, t(label, pt));
                                        tracked.push((response, label.into()));
                                    }
                                });
                                if r != old {
                                    g.rotary = Some(r);
                                }
                                ui.weak(t(
                                    "Drive diameter means object diameter for a chuck, roller diameter for a roller. Protocol unverified (D12).",
                                    "Diâmetro de acionamento é o diâmetro do objeto no mandril e o diâmetro do rolete nos roletes. Protocolo não verificado (D12).",
                                ));
                            });
                            let mut required = self.session.frame_required();
                            let r = ui.add_enabled(false, egui::Checkbox::new(&mut required, t("Require framing before start", "Exigir enquadramento antes de iniciar")));
                            let r = r.on_disabled_hover_text(t("Change it in Settings ▸ Laser.", "Altere em Configurações ▸ Laser."));
                            tracked.push((r, "Require framing before start".into()));
                            ui.weak(crate::tf!(
                                "Laser source: pulse table {:?} ns · {}–{} kHz",
                                "Fonte laser: tabela de pulsos {:?} ns · {}–{} kHz",
                                g.source.pulse_table_ns, g.source.freq_khz[0], g.source.freq_khz[1]
                            ));
                        } else {
                            ui.label(t("Gantry device settings arrive with live GRBL (M4).", "As configurações de dispositivo gantry chegam com o GRBL ao vivo (M4)."));
                        }
                    }
                    None => {
                        ui.label(t("No device profile.", "Nenhum perfil de dispositivo."));
                    }
                }
            });
            ui.separator();
            let r = ui.add_enabled(self.device_draft.is_some() && !read_only, egui::Button::new(t("Save device settings", "Salvar configurações do dispositivo")));
            let human = self.services.human.is_human_click(ctx, &r);
            save = human && !self.frame_synthetic && !self.services.human.frame_is_synthetic(ctx);
            if r.clicked() && !save {
                self.set_status(t(
                    "Saving a machine profile needs a person's click in this window.",
                    "Salvar um perfil de máquina requer o clique de uma pessoa nesta janela.",
                ));
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
                            serde_json::json!({ "device": profile.id.0, "workArea": g.work_area, "cal": g.cal, "redDot": g.red_dot, "rotary": g.rotary })
                        }
                        openlaser_machine::Kinematics::Gantry(_) => serde_json::json!({ "device": profile.id.0 }),
                    };
                    match self.session.execute_as(openlaser_engine::Origin::Human, "device.settings.set", params) {
                        Ok(_) => {
                            open = false;
                            self.set_status(t("Device settings saved", "Configurações do dispositivo salvas"));
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
        egui::Window::new(t("Live Framing", "Enquadramento ao vivo")).id(egui::Id::new("live_framing_window")).open(&mut open).collapsible(false).show(
            ctx,
            |ui| {
                ui.label(t(
                    "Red light only: the galvo traces the job; the laser never fires while framing.",
                    "Somente luz vermelha: o galvo percorre o trabalho; o laser nunca dispara durante o enquadramento.",
                ));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.ui.live_style, openlaser_types::FrameStyle::Bounds, t("Bounds", "Limites"));
                    ui.radio_value(&mut self.ui.live_style, openlaser_types::FrameStyle::Contour, t("Contour", "Contorno"));
                });
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.ui.live_source, openlaser_types::FrameSource::Job, t("Job", "Trabalho"));
                    ui.radio_value(&mut self.ui.live_source, openlaser_types::FrameSource::ToolLayersOnly, t("Tool Layers Only", "Só camadas de ferramenta"));
                    ui.radio_value(
                        &mut self.ui.live_source,
                        openlaser_types::FrameSource::JobAndToolLayers,
                        t("Job and Tool Layers", "Trabalho e camadas de ferramenta"),
                    );
                });
                let gate = crate::panels::frame_gate(self.plan(), self.ui.machine_active);
                let start = ui.add_enabled(gate.is_ok(), egui::Button::new(t("Start Framing", "Iniciar enquadramento")));
                self.track(&start, "button", "Start Framing", true);
                let human = self.services.human.is_human_click(ctx, &start);
                if human && gate.is_ok() && !self.frame_synthetic && !self.services.human.frame_is_synthetic(ctx) {
                    #[cfg(test)]
                    self.seam.human_actions.push("Start Framing");
                    let (style, source) = (self.ui.live_style, self.ui.live_source);
                    let message = match self.session.start_frame(openlaser_engine::Origin::Human, openlaser_types::FrameMode::Loop, Some(style), Some(source)) {
                        Ok(_) => t("Framing", "Enquadrando").to_owned(),
                        Err(e) => e.to_string(),
                    };
                    self.set_status(message);
                } else if start.clicked() {
                    self.set_status(t(
                        "Framing is human-only: it needs a person's click in this window.",
                        "O enquadramento é só para pessoas: requer o clique de uma pessoa nesta janela.",
                    ));
                }
                if let Err(reason) = gate {
                    start.on_disabled_hover_text(reason);
                }
                let stop = ui.button(t("Stop Framing", "Parar enquadramento"));
                self.track(&stop, "button", "Stop Framing", false);
                if stop.clicked() {
                    self.stop();
                }
            },
        );
        self.ui.live_framing_open = open;
    }

    fn preview_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let plan = self.plan().cloned();
        egui::Window::new(t("Preview", "Pré-visualização")).id(egui::Id::new("preview_window")).open(&mut open).show(ctx, |ui| match &plan {
            Some(PlanView::Ready(summary)) => {
                ui.label(crate::tf!(
                    "Estimate {} · {} packet(s) · repeats {}",
                    "Estimativa {} · {} pacote(s) · repetições {}",
                    crate::panels::mmss(summary.estimate),
                    summary.packets,
                    summary.repeats
                ));
                egui::Grid::new("preview-rows").striped(true).show(ui, |ui| {
                    for heading in [
                        t("Layer", "Camada"),
                        t("Pass", "Passada"),
                        t("Mode", "Modo"),
                        t("Speed (actual)", "Velocidade (real)"),
                        t("Power", "Potência"),
                        "kHz",
                        "ns",
                        t("Paths", "Caminhos"),
                    ] {
                        ui.strong(heading);
                    }
                    ui.end_row();
                    for row in &summary.rows {
                        ui.label(&row.layer_name);
                        ui.label(format!("{}/{}", row.pass, row.of));
                        ui.label(crate::i18n::section_mode(row.mode));
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
                    ui.label(crate::i18n::refusal(r));
                }
            }
            None => {
                ui.label(t("Open a project to preview its plan.", "Abra um projeto para pré-visualizar seu plano."));
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
            foot_switch: matches!(&machine.kind, openlaser_machine::Kinematics::Galvo(g) if g.ports.foot_switch.is_some()),
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
    /// The Trace Image dialog: LightBurn-style options with a live ghost on the canvas. Every preview trace
    /// runs on a worker thread (std::thread + channels): at most one job in flight, a newer request replaces
    /// the queued one (latest wins), stale results are dropped, and a "tracing…" hint shows while it works.
    /// Values persist on `ui.trace_defaults` when the dialog closes.
    fn trace_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut t) = self.ui.trace.clone() else { return };
        // Worker channel on first open.
        if t.tx.is_none() {
            let (tx, rx) = OpenLaserApp::spawn_trace_worker();
            t.tx = Some(tx);
            t.rx = Some(std::sync::Arc::new(std::sync::Mutex::new(rx)));
        }
        // Collect a finished result; drop it when it is stale (older settings seq or another image).
        if let Some(rx) = &t.rx {
            let mut got = None;
            while let Some(res) = rx.lock().ok().and_then(|r| r.try_recv().ok()) {
                got = Some(res);
            }
            if let Some(res) = got {
                t.in_flight = false;
                if t.pending.accept(res.seq, res.image_id) {
                    t.preview = Some(res.polys);
                }
            }
        }
        let before = (t.threshold, t.ignore_less_than, t.smoothness, t.optimize, t.invert);
        let mut open = true;
        let mut ok = false;
        let mut cancel = false;
        egui::Window::new(crate::i18n::t("Trace Image", "Vetorizar imagem"))
            .id(egui::Id::new("trace_image_window"))
            .open(&mut open)
            .collapsible(false)
            .auto_sized()
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::t("Threshold", "Limiar"));
                    let r = ui.add(egui::Slider::new(&mut t.threshold, 1.0..=255.0));
                    self.labeled(&r, egui::WidgetType::Slider, "Threshold", false);
                });
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::t("Ignore less than", "Ignorar menores que"));
                    let r = ui.add(egui::DragValue::new(&mut t.ignore_less_than).speed(0.1).range(0.0..=1000.0));
                    self.labeled(&r, egui::WidgetType::DragValue, "Ignore less than", false);
                    ui.label("px²");
                });
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::t("Smoothness", "Suavidade"));
                    let r = ui.add(egui::Slider::new(&mut t.smoothness, 0.0..=1.33));
                    self.labeled(&r, egui::WidgetType::Slider, "Smoothness", false);
                });
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::t("Optimize", "Otimizar"));
                    let r = ui.add(egui::DragValue::new(&mut t.optimize).speed(0.05).range(0.0..=5.0));
                    self.labeled(&r, egui::WidgetType::DragValue, "Optimize", false);
                    ui.label("px");
                });
                let r = ui.checkbox(&mut t.invert, crate::i18n::t("Invert (trace the light side)", "Inverter (vetorizar o lado claro)"));
                self.labeled(&r, egui::WidgetType::Checkbox, "Invert", false);
                let r = ui.checkbox(&mut t.delete_image, crate::i18n::t("Delete image after tracing", "Excluir imagem após vetorizar"));
                self.labeled(&r, egui::WidgetType::Checkbox, "Delete image after tracing", false);
                if t.in_flight {
                    ui.small(crate::i18n::t("tracing…", "vetorizando…"));
                }
                ui.horizontal(|ui| {
                    let r = ui.button("OK");
                    self.labeled(&r, egui::WidgetType::Button, "OK", false);
                    if r.clicked() {
                        ok = true;
                    }
                    let r = ui.button(crate::i18n::t("Cancel", "Cancelar"));
                    self.labeled(&r, egui::WidgetType::Button, "Cancel", false);
                    if r.clicked() {
                        cancel = true;
                    }
                });
            });
        // Debounce a change then send it; the worker drops superseded requests itself.
        if before != (t.threshold, t.ignore_less_than, t.smoothness, t.optimize, t.invert) || t.preview_at.is_none() {
            t.dirty = true;
            t.preview_at = Some(std::time::Instant::now());
        }
        if t.dirty && t.preview_at.is_some_and(|at| at.elapsed() > std::time::Duration::from_millis(150)) {
            let bitmap = self.project().and_then(|p| {
                p.shape(openlaser_types::ObjectId(t.id)).and_then(|sh| if let openlaser_doc::Geom::Bitmap(b) = &sh.geom { Some(b.clone()) } else { None })
            });
            if let (Some(b), Some(tx)) = (bitmap, &t.tx) {
                t.request_seq += 1;
                t.pending.sent(t.request_seq, t.id);
                let _ = tx.send(crate::app::TraceJob {
                    seq: t.request_seq,
                    image_id: t.id,
                    gray: b.gray.clone(),
                    px: b.px,
                    size_mm: b.size_mm,
                    opts: t.geom_opts(),
                    runs: t.runs.clone(),
                });
                t.in_flight = true;
                t.dirty = false;
            }
        }
        if t.in_flight || t.dirty {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if ok {
            let params = t.params();
            // Persist the values for the next open (id/worker fields are per-open).
            let mut saved = t.clone();
            saved.preview = None;
            saved.tx = None;
            saved.rx = None;
            self.ui.trace_defaults = Some(saved);
            self.ui.trace = None;
            self.execute_widget("tools.traceImage", params);
        } else if cancel || !open {
            let mut saved = t.clone();
            saved.preview = None;
            saved.tx = None;
            saved.rx = None;
            self.ui.trace_defaults = Some(saved);
            self.ui.trace = None;
        } else {
            self.ui.trace = Some(t);
        }
    }
}
