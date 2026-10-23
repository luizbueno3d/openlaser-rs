//! The only file that may build a GUI confirmation (SPEC §2.3 rule 11): `HumanConfirm::from_gui_gesture` and
//! `GuiGestureWitness::new` are clippy disallowed-methods everywhere else.
//!
//! WP0 stub: `gesture` never returns a witness (fail closed).
#![allow(clippy::disallowed_methods)]

use openlaser_types::MonoTime;

/// (no Clone) The witness for one real click.
pub struct HumanGesture {
    _private: (),
}

/// Some only if: response.clicked_by(PointerButton::Primary); the frame is untainted; no AccessKit action request
/// this frame; window focused; widget visible ≥ 300 ms. (egui 0.36.2: clicked_by excludes keyboard and AccessKit.)
pub fn gesture(ctx: &egui::Context, response: &egui::Response, visible_since: MonoTime, now: MonoTime) -> Option<HumanGesture> {
    let _ = (ctx, response, visible_since, now);
    None
}
