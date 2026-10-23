//! Approval cards for agent requests (SPEC §4.4): a submission or a frame request waits for a person. The card shows
//! the fingerprint, the decoded summary and the client; the agent's note is plain text, labelled "Note from agent",
//! at most 280 characters, and sits **outside** the confirmation block. Approve and Decline are human-only widgets
//! (the gate in `Human`), and a drawn card is a human-only surface: injected input is dropped while it is shown.

use openlaser_types::{EmissionSummary, Fingerprint};

use crate::{Human, format};

/// What one card shows. The engine's `SubmissionView` / `FrameRequestView` map onto it.
#[derive(Debug, Clone, Copy)]
pub struct ApprovalCard<'a> {
    /// Distinguishes several cards in one window (the request id).
    pub id: u64,
    /// "Job from an agent" / "Frame request".
    pub title: &'a str,
    pub client: &'a str,
    pub fingerprint: &'a Fingerprint,
    /// Decoded from the bytes (submissions); None for a frame request.
    pub summary: Option<&'a EmissionSummary>,
    pub note: &'a str,
    pub approve_label: &'a str,
    pub decline_label: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalAction {
    Pending,
    Approve,
    Decline,
}

impl Human {
    /// Draws one approval card; returns Approve or Decline only for a real human click (SPEC §3.20 gate). Call it on
    /// every frame the card is shown.
    pub fn approval_card(&mut self, ui: &mut egui::Ui, card: &ApprovalCard<'_>) -> ApprovalAction {
        let ctx = ui.ctx().clone();
        self.surface_shown(&ctx);
        let mut approve = None;
        let mut decline = None;
        ui.push_id(("openlaser-human::approval", card.id), |ui| {
            ui.group(|ui| {
                ui.strong(card.title);
                ui.label(format!("From {} · job {}", card.client, card.fingerprint.short()));
                if let Some(s) = card.summary {
                    for row in &s.rows {
                        ui.label(format!(
                            "{} {} {} · {} · {} · {} · {}",
                            format::layer(row),
                            format::mode(row.mode),
                            format::pass(row),
                            format::power(row.power_pct),
                            format::frequency(row.frequency_khz),
                            format::pulse(row.pulse_ns),
                            format::speed(row.speed_mm_s),
                        ));
                    }
                    ui.label(format!("Repeats: {} · total {}", s.repeats, format::seconds(s.estimate)));
                }
                ui.horizontal(|ui| {
                    approve = Some(ui.button(card.approve_label));
                    decline = Some(ui.button(card.decline_label));
                });
            });
            let note = format::note(card.note);
            if !note.is_empty() {
                ui.label("Note from agent");
                ui.label(egui::RichText::new(note).italics());
            }
        });
        // Both buttons go through the gate every frame (visibility timers, press provenance).
        let approved = approve.is_some_and(|r| self.gate(&ctx, &r).is_some());
        let declined = decline.is_some_and(|r| self.gate(&ctx, &r).is_some());
        match (approved, declined) {
            (true, false) => ApprovalAction::Approve,
            (false, true) => ApprovalAction::Decline,
            _ => ApprovalAction::Pending,
        }
    }
}
