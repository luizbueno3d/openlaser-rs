//! Human-only surfaces and the input-provenance gate (SPEC §3.20, §4.6 mechanism 3; owned by WP9, red zone).
//!
//! [`Human`] implements `openlaser_ui_egui::HumanSurfaces`, the only way automation input enters egui and the gate
//! every human-only widget acts through:
//!
//! - **Taint.** `inject` taints the pass that carries the synthetic events, the next two passes, and every pass until
//!   1 s after the last injected event. A press that started with automation input taints its whole press-release
//!   sequence, so "press synthetic, release human" never counts.
//! - **Gate.** A human-only widget acts only on a primary-button click (never keyboard or AccessKit), in an untainted
//!   pass, with no AccessKit action request in the pass, with the window focused, after the widget has been visible
//!   for ≥ 300 ms. Callers must run `is_human_click` for a human-only widget on **every** frame it is drawn (that is
//!   how its visibility timer runs); never short-circuit it behind `response.clicked()`.
//! - **Surfaces.** While a human-only surface (the ARM dialog, an approval card, anything marked with
//!   [`Human::surface_shown`]) is open, injected events are dropped and `surface_open` is true, so the control
//!   dispatcher refuses mutating methods.
//! - **ARM dialog.** Built only from `ArmModel` (the summary decoded from the bytes); typed `ARM` plus the profile's
//!   checklist; Confirm mints through `openlaser_arm::mint` and returns the token once.
#![forbid(unsafe_code)]

mod approval;
mod arm_dialog;
pub mod format;
mod gesture;
mod pedal;
mod taint;

use std::cell::Cell;
use std::sync::Arc;

pub use approval::{ApprovalAction, ApprovalCard};
pub use arm_dialog::{AUTOMATION_IGNORED, CANCEL_LABEL, CONFIRM_LABEL, TITLE as ARM_DIALOG_TITLE};
pub use gesture::{HumanGesture, MIN_VISIBLE, gesture};
use openlaser_types::{ArmToken, Clock, EmissionSummary, FrameReceipt, MonoTime, Refusal, SessionId};
use openlaser_ui_egui::{ArmModel, ArmOutcome, HumanSurfaces, PedalPress};
pub use taint::{TAINT_HOLD, TAINT_PASSES};

use crate::arm_dialog::ArmDialogState;
use crate::taint::{Surfaces, Taint, Visibility};

/// `openlaser_arm::mint`'s signature; tests substitute a recorder (only under `cfg(test)`).
#[cfg(test)]
pub(crate) type MintFn = fn(&EmissionSummary, &FrameReceipt, openlaser_arm::HumanConfirm, usize, SessionId, MonoTime) -> Result<ArmToken, Refusal>;

/// The taint record lives in this struct (keyed by `ctx.cumulative_pass_nr()` plus the time of the last injected
/// event), NOT in egui context data: any code holding the `Context` can clear `ctx.data_mut(..)`, while `Human` is
/// reachable only through `&mut dyn HumanSurfaces`.
pub struct Human {
    clock: Arc<dyn Clock>,
    pub(crate) taint: Taint,
    pub(crate) visibility: Visibility,
    /// A `Cell` so that `frame_is_synthetic(&self)` can notice a surface closing too.
    surfaces: Cell<Surfaces>,
    pub(crate) arm: ArmDialogState,
    /// The last real click on Start and when it happened (`start_click`); used once, by the ARM that follows it.
    pub(crate) start: Option<(HumanGesture, MonoTime)>,
    /// The last foot-switch press (`foot_switch`); used once, by the ARM that follows it.
    pub(crate) pedal: Option<PedalPress>,
    /// What the person typed and ticked the last time they armed in this session (ARM once per session).
    pub(crate) remembered: Option<arm_dialog::Remembered>,
    /// Settings ▸ Laser: no ARM confirmation at all.
    pub(crate) skip_arm: bool,
    #[cfg(test)]
    pub(crate) test_mint: Option<MintFn>,
}

impl Human {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Human {
            clock,
            taint: Taint::default(),
            visibility: Visibility::default(),
            surfaces: Cell::new(Surfaces::default()),
            arm: ArmDialogState::default(),
            start: None,
            pedal: None,
            remembered: None,
            skip_arm: false,
            #[cfg(test)]
            test_mint: None,
        }
    }

    pub(crate) fn now(&self) -> MonoTime {
        self.clock.now()
    }

    /// Marks a human-only surface (Live Framing, a device dialog) as drawn in this pass. While any surface is open,
    /// injected input is dropped and `surface_open` is true. Call it on every frame the surface is shown.
    pub fn surface_shown(&mut self, ctx: &egui::Context) {
        let pass = ctx.cumulative_pass_nr();
        self.refresh(pass);
        self.mark_shown(pass);
    }

    /// Forgets the ARM dialog's typed text and ticks at once. The app calls it whenever it voids the PendingArm (any
    /// revision bump, any stop): `ArmModel` carries no revision, so this is what makes even a new PendingArm that is
    /// identical to the voided one, drawn in the very next pass, a new opening.
    pub fn close_arm_dialog(&mut self) {
        self.arm.forget();
    }

    pub(crate) fn mark_shown(&self, pass: u64) {
        let mut s = self.surfaces.get();
        s.shown(pass);
        self.surfaces.set(s);
    }

    pub(crate) fn close_surfaces(&self) {
        let mut s = self.surfaces.get();
        s.close();
        self.surfaces.set(s);
    }

    /// Notices surfaces that were not drawn in the previous pass. Returns true when they just closed.
    fn notice_closed(&self, pass: u64) -> bool {
        let mut s = self.surfaces.get();
        let closed = s.refresh(pass);
        self.surfaces.set(s);
        closed
    }

    /// Synthetic events handed to `inject` so far (diagnostics for `ui.inspect`).
    pub fn injected_events(&self) -> u64 {
        self.taint.injected()
    }

    /// Synthetic events dropped because a human-only surface was open.
    pub fn dropped_events(&self) -> u64 {
        self.taint.dropped()
    }

    /// Closes surfaces that were not drawn in the previous pass; a closed ARM dialog forgets its typed text and ticks.
    pub(crate) fn refresh(&mut self, pass: u64) {
        if self.notice_closed(pass) {
            self.arm.forget();
        }
    }

    fn tainted(&self, ctx: &egui::Context, pass: u64, now: MonoTime) -> bool {
        self.taint.live(pass, now) || gesture::accesskit_action_this_frame(ctx) || taint::context_marker_live(ctx)
    }

    /// The gate (SPEC §3.20). Runs every frame for every human-only widget it is asked about: it advances the
    /// widget's visibility timer and the press provenance, and returns a witness only for a real click.
    pub(crate) fn gate(&mut self, ctx: &egui::Context, response: &egui::Response) -> Option<HumanGesture> {
        let pass = ctx.cumulative_pass_nr();
        let now = self.now();
        self.refresh(pass);
        let tainted = self.tainted(ctx, pass, now);
        let (pressed, down) = ctx.input(|i| (i.pointer.any_pressed(), i.pointer.any_down()));
        self.taint.observe_pointer(tainted, pressed, down);
        let visible_since = self.visibility.visible_since(response.id, pass, now);
        if self.taint.press_tainted() {
            return None;
        }
        gesture::gesture_checked(ctx, response, visible_since, now, tainted)
    }

    pub(crate) fn mint(
        &self,
        summary: &EmissionSummary,
        receipt: &FrameReceipt,
        confirm: openlaser_arm::HumanConfirm,
        checklist_len: usize,
        session: SessionId,
        now: MonoTime,
    ) -> Result<ArmToken, Refusal> {
        #[cfg(test)]
        if let Some(f) = self.test_mint {
            return f(summary, receipt, confirm, checklist_len, session, now);
        }
        openlaser_arm::mint(summary, receipt, confirm, checklist_len, session, now)
    }
}

impl HumanSurfaces for Human {
    fn inject(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput, events: Vec<egui::Event>) {
        let pass = ctx.cumulative_pass_nr();
        let now = self.now();
        self.refresh(pass);
        if events.is_empty() {
            return;
        }
        self.taint.record(ctx, pass, now, &events);
        if self.surfaces.get().open() {
            // The freeze: nothing an agent sends reaches a human-only surface (or anything else) while one is open.
            self.taint.note_dropped(events.len());
            return;
        }
        raw.events.extend(events);
    }

    fn frame_is_synthetic(&self, ctx: &egui::Context) -> bool {
        let pass = ctx.cumulative_pass_nr();
        // A closed ARM dialog's text is also forgotten when it is next drawn (`ArmDialogState::continues`).
        self.notice_closed(pass);
        self.tainted(ctx, pass, self.now())
    }

    fn is_human_click(&mut self, ctx: &egui::Context, response: &egui::Response) -> bool {
        self.gate(ctx, response).is_some()
    }

    fn start_click(&mut self, ctx: &egui::Context, response: &egui::Response) -> bool {
        match self.gate(ctx, response) {
            Some(g) => {
                self.start = Some((g, self.now()));
                true
            }
            None => false,
        }
    }

    fn set_skip_arm(&mut self, skip: bool) {
        self.skip_arm = skip;
    }

    fn foot_switch(&mut self, press: PedalPress) {
        self.pedal = Some(press);
    }

    fn arm_dialog(&mut self, ctx: &egui::Context, model: &ArmModel) -> ArmOutcome {
        self.show_arm_dialog(ctx, model)
    }

    fn surface_open(&self) -> bool {
        self.surfaces.get().open()
    }
}

#[cfg(test)]
mod tests;
