//! The ARM confirmation (SPEC §3.20, normative). Built only from `ArmModel.summary` (decoded from the bytes that will
//! be sent) and a thumbnail of `ArmModel.thumbnail` (decoded emitted segments, never the document). The checklist
//! comes from the profile. The `ARM` field accepts text only in untainted frames; any tainted frame while the dialog
//! is open clears the field and every tick and shows [`AUTOMATION_IGNORED`]. Confirm is a human-only widget; on a
//! human click it builds the `HumanConfirm` (gesture.rs) and calls `openlaser_arm::mint`, and returns `Armed(token)`
//! once. Typed text never persists between openings: a new model (any revision bump builds a new `PendingArm`), a
//! frame without the dialog (any stop voids `PendingArm`, so the app stops drawing it), Cancel and Armed all forget
//! it. Over a bench session's first emission (S23) the dialog shows the cap and refuses rows above it before minting.

use std::time::Duration;

use openlaser_types::{Fingerprint, FrameReceipt, MachineId, MonoTime, Refusal, RefusalCode, SessionId};
use openlaser_ui_egui::{ArmModel, ArmOutcome};

use crate::Human;
use crate::format;

pub const TITLE: &str = "Confirm laser emission";
pub const CONFIRM_LABEL: &str = "ARM and start";
pub const CANCEL_LABEL: &str = "Cancel";
/// SPEC §3.20 wording.
pub const AUTOMATION_IGNORED: &str = "Automation input reached this dialog and was ignored.";
const FIELD_HINT: &str = "Type ARM";
/// An empty checklist can never be confirmed (`mint` refuses it too, SPEC §3.9).
pub const NO_CHECKLIST: &str = "This machine profile has no checklist, so nothing can be armed. Add one in Device Settings.";
const THUMB_SIDE: f32 = 160.0;

/// What identifies one opening: a different `PendingArm` is a different dialog. Everything the person confirms by
/// ticking and typing is in it: the bytes (whose decoded summary is shown), the frame receipt, the checklist items
/// themselves (not only how many there are) and the bench cap.
#[derive(Debug, Clone, PartialEq)]
struct ModelKey {
    fingerprint: Fingerprint,
    bytes: Fingerprint,
    machine: MachineId,
    session: SessionId,
    receipt: FrameReceipt,
    rows: usize,
    checklist: Vec<String>,
    /// Compared bit for bit, so a NaN cap is still one key.
    cap_bits: Option<u64>,
}

impl ModelKey {
    fn of(m: &ArmModel) -> Self {
        ModelKey {
            fingerprint: m.summary.fingerprint,
            bytes: m.summary.bytes_hash,
            machine: m.summary.machine.clone(),
            session: m.session,
            receipt: m.receipt.clone(),
            rows: m.summary.rows.len(),
            checklist: m.checklist.clone(),
            cap_bits: m.emission_cap_pct.map(f64::to_bits),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ArmDialogState {
    key: Option<ModelKey>,
    last_pass: Option<u64>,
    typed: String,
    ticked: Vec<bool>,
    automation_seen: bool,
    refusal: Option<String>,
}

impl ArmDialogState {
    /// Forgets everything typed or ticked (the dialog closed).
    pub(crate) fn forget(&mut self) {
        *self = ArmDialogState::default();
    }

    fn open(&mut self, key: ModelKey, checklist_len: usize) {
        *self = ArmDialogState { key: Some(key), ticked: vec![false; checklist_len], ..ArmDialogState::default() };
    }

    fn continues(&self, key: &ModelKey, pass: u64, checklist_len: usize) -> bool {
        self.key.as_ref() == Some(key) && self.ticked.len() == checklist_len && self.last_pass.is_some_and(|p| p.saturating_add(1) >= pass)
    }

    fn clear_input(&mut self) {
        self.typed.clear();
        self.ticked.iter_mut().for_each(|t| *t = false);
    }

    fn typed_ok(&self) -> bool {
        self.typed.trim().eq_ignore_ascii_case("ARM")
    }

    fn all_ticked(&self) -> bool {
        !self.ticked.is_empty() && self.ticked.iter().all(|t| *t)
    }
}

#[cfg(test)]
impl ArmDialogState {
    pub(crate) fn ticked_for_tests(&self) -> Vec<bool> {
        self.ticked.clone()
    }
}

enum Clicked {
    Nothing,
    Cancel,
}

impl Human {
    pub(crate) fn show_arm_dialog(&mut self, ctx: &egui::Context, model: &ArmModel) -> ArmOutcome {
        let pass = ctx.cumulative_pass_nr();
        let now = self.now();
        self.refresh(pass);
        let key = ModelKey::of(model);
        if !self.arm.continues(&key, pass, model.checklist.len()) {
            self.arm.open(key, model.checklist.len());
        }
        self.arm.last_pass = Some(pass);
        self.mark_shown(pass);

        let tainted = self.tainted(ctx, pass, now);
        if tainted {
            self.arm.clear_input();
            self.arm.automation_seen = true;
        }
        let violations = format::cap_violations(&model.summary.rows, model.emission_cap_pct);
        let ready = !tainted && self.arm.typed_ok() && self.arm.all_ticked() && violations.is_empty();

        let mut clicked = Clicked::Nothing;
        let mut confirm: Option<egui::Response> = None;
        let state = &mut self.arm;
        egui::Modal::new(egui::Id::new("openlaser-human::arm-dialog")).show(ctx, |ui| {
            ui.set_max_width(980.0);
            ui.heading(TITLE);
            draw_summary(ui, model);
            draw_cap(ui, model.emission_cap_pct, &violations);
            ui.separator();
            ui.label("Before you arm:");
            if model.checklist.is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, NO_CHECKLIST);
            }
            for (i, item) in model.checklist.iter().enumerate() {
                if let Some(t) = state.ticked.get_mut(i) {
                    ui.checkbox(t, item.as_str());
                }
            }
            ui.add(egui::TextEdit::singleline(&mut state.typed).hint_text(FIELD_HINT).desired_width(120.0));
            if state.automation_seen {
                ui.colored_label(ui.visuals().warn_fg_color, AUTOMATION_IGNORED);
            }
            if let Some(r) = &state.refusal {
                ui.colored_label(ui.visuals().error_fg_color, r.as_str());
            }
            ui.horizontal(|ui| {
                confirm = Some(ui.add_enabled(ready, egui::Button::new(CONFIRM_LABEL)));
                if ui.button(CANCEL_LABEL).clicked() {
                    clicked = Clicked::Cancel;
                }
            });
        });
        if tainted {
            // Anything a tainted frame put into the field or the ticks (an AccessKit SetValue, say) is undone.
            self.arm.clear_input();
        }
        if let Clicked::Cancel = clicked {
            self.arm.forget();
            self.close_surfaces();
            return ArmOutcome::Cancelled;
        }
        // The gate runs every frame for the Confirm button (its visibility timer and the press provenance).
        let Some(confirm) = confirm else { return ArmOutcome::Open };
        let Some(gesture) = self.gate(ctx, &confirm) else { return ArmOutcome::Open };
        if !violations.is_empty() {
            let r = Refusal::new(RefusalCode::BenchPowerCap, violations.join("; "));
            self.arm.refusal = Some(refusal_text(&r));
            return ArmOutcome::Open;
        }
        if !(self.arm.typed_ok() && self.arm.all_ticked()) {
            return ArmOutcome::Open;
        }
        let at = latest(now, model.now);
        let typed = self.arm.typed.trim().to_owned();
        let ticked = self.arm.ticked.clone();
        let confirmation = gesture.confirm(&typed, ticked, at);
        match self.mint(&model.summary, &model.receipt, confirmation, model.checklist.len(), model.session, at) {
            Ok(token) => {
                self.arm.forget();
                self.close_surfaces();
                ArmOutcome::Armed(token)
            }
            Err(r) => {
                self.arm.typed.clear();
                self.arm.refusal = Some(refusal_text(&r));
                ArmOutcome::Open
            }
        }
    }
}

/// The later of the two clocks' readings: a stale `ArmModel.now` must never make a receipt look younger.
fn latest(a: MonoTime, b: MonoTime) -> MonoTime {
    if a >= b { a } else { b }
}

fn refusal_text(r: &Refusal) -> String {
    format!("Refused ({}): {}", r.code.as_str(), r.message)
}

fn draw_summary(ui: &mut egui::Ui, model: &ArmModel) {
    let s = &model.summary;
    ui.label(format!("Machine: {}", s.machine.0));
    ui.label(format!("Job {}", s.fingerprint.short()));
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(THUMB_SIDE, THUMB_SIDE), egui::Sense::hover());
        ui.painter().rect_stroke(rect, 2.0, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
        openlaser_ui_egui::canvas::paint_fit(ui.painter(), &model.thumbnail, rect.shrink(4.0));
        ui.vertical(|ui| {
            egui::Grid::new("openlaser-human::arm-rows").striped(true).show(ui, |ui| {
                for h in ["Layer", "Mode", "Pass", "Power", "Frequency", "Pulse", "Speed (actual)", "Paths", "Bounds", "Time"] {
                    ui.strong(h);
                }
                ui.end_row();
                for row in &s.rows {
                    ui.label(format::layer(row));
                    ui.label(format::mode(row.mode));
                    ui.label(format::pass(row));
                    ui.label(format::power(row.power_pct));
                    ui.label(format::frequency(row.frequency_khz));
                    ui.label(format::pulse(row.pulse_ns));
                    ui.label(format::speed(row.speed_mm_s));
                    ui.label(row.paths.to_string());
                    ui.label(format::bounds(&row.bounds));
                    ui.label(format::seconds(row.estimate));
                    ui.end_row();
                }
            });
        });
    });
    ui.label(format!("Repeats: {}", s.repeats));
    for n in &s.notices {
        ui.label(format!("Notice {}: {}", n.n, n.message));
    }
    ui.label(format!("Total time: {}", format::seconds(s.estimate)));
    ui.label(format!("Emission still possible after a host crash: up to {}", format::seconds(s.crash_bound)));
    if s.crash_bound > Duration::ZERO {
        ui.small("The E-stop and the key switch stop everything else.");
    }
}

fn draw_cap(ui: &mut egui::Ui, cap: Option<f64>, violations: &[String]) {
    let Some(cap) = cap else { return };
    ui.separator();
    ui.colored_label(ui.visuals().warn_fg_color, format!("First emission of this bench session: power is capped at {cap} %."));
    for v in violations {
        ui.colored_label(ui.visuals().error_fg_color, v.as_str());
    }
}
