//! The only file that may build a GUI confirmation (SPEC §2.3 rule 11): `HumanConfirm::from_gui_gesture` and
//! `GuiGestureWitness::new` are clippy disallowed-methods everywhere else.
//!
//! [`gesture`] checks what egui itself can tell about one click (primary button only, no AccessKit action this frame,
//! focused window, widget visible ≥ 300 ms). The taint record (synthetic input, SPEC §3.20) lives in [`crate::Human`],
//! which is the gate every human-only widget goes through; `gesture` additionally refuses while the secondary taint
//! marker that `Human::inject` writes into the context is live, so it fails closed even when called on its own.
//! A [`HumanGesture`] can be turned into a `HumanConfirm` only inside this crate (`HumanGesture::confirm`).
#![allow(clippy::disallowed_methods)]

use std::time::Duration;

use openlaser_arm::{GuiGestureWitness, HumanConfirm};
use openlaser_types::MonoTime;

/// A human-only widget must have been visible this long before a click on it counts (SPEC §3.20).
pub const MIN_VISIBLE: Duration = Duration::from_millis(300);

/// (no Clone) The witness for one real click.
pub struct HumanGesture {
    _private: (),
}

impl HumanGesture {
    /// Turns the click into the confirmation `openlaser_arm::mint` takes. Crate-private: only the ARM dialog does it.
    pub(crate) fn confirm(self, typed: &str, ticked: Vec<bool>, at: MonoTime) -> HumanConfirm {
        let HumanGesture { _private: () } = self;
        HumanConfirm::from_gui_gesture(GuiGestureWitness::new(), typed, ticked, at)
    }
}

/// Some only if: response.clicked_by(PointerButton::Primary); the frame is untainted; no AccessKit action request
/// this frame; window focused; widget visible ≥ 300 ms. (egui 0.36.2: clicked_by excludes keyboard and AccessKit.)
///
/// "Untainted" here is the context marker `Human::inject` leaves; the authoritative record is `Human`'s own, and
/// `Human::is_human_click` applies both.
pub fn gesture(ctx: &egui::Context, response: &egui::Response, visible_since: MonoTime, now: MonoTime) -> Option<HumanGesture> {
    let tainted = crate::taint::context_marker_live(ctx);
    gesture_checked(ctx, response, visible_since, now, tainted)
}

/// [`gesture`] with the taint decision supplied by the caller (`Human`).
pub(crate) fn gesture_checked(ctx: &egui::Context, response: &egui::Response, visible_since: MonoTime, now: MonoTime, tainted: bool) -> Option<HumanGesture> {
    if tainted {
        return None;
    }
    if !response.clicked_by(egui::PointerButton::Primary) {
        return None;
    }
    if accesskit_action_this_frame(ctx) {
        return None;
    }
    if !ctx.input(|i| i.focused) {
        return None;
    }
    if now.saturating_since(visible_since) < MIN_VISIBLE {
        return None;
    }
    Some(HumanGesture { _private: () })
}

/// Any AccessKit action request in this frame's input (assistive technology, or automation driving it).
pub(crate) fn accesskit_action_this_frame(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::AccessKitActionRequest(_))))
}
