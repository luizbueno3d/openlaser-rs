//! The only file that may build a foot-switch confirmation (SPEC §2.3 rule 11, SPEC M3, design C3.8):
//! `HumanConfirm::from_foot_switch` and `FootSwitchWitness::new` are clippy disallowed-methods everywhere else.
//!
//! A [`PedalPress`] is built only by the engine, from its driver thread's debounced read of the board's input port:
//! no software on the computer can press the pedal. The confirmation is stamped with the press time (or, when framing
//! is off and the engine's stand-in receipt is at most 2 s newer than the press, with that receipt's time), so
//! `mint`'s 10 s age and "after the frame" rules apply to the press itself; every checklist box must still be ticked.
#![allow(clippy::disallowed_methods)]

use openlaser_arm::{FootSwitchWitness, HumanConfirm};
use openlaser_ui_egui::PedalPress;

/// The press as the confirmation `openlaser_arm::mint` takes. Crate-private: only the ARM paths do it.
pub(crate) fn confirm(press: &PedalPress, ticked: Vec<bool>, at: openlaser_types::MonoTime) -> HumanConfirm {
    // Never earlier than the press itself.
    let at = if at >= press.at() { at } else { press.at() };
    HumanConfirm::from_foot_switch(FootSwitchWitness::new(), ticked, at)
}
