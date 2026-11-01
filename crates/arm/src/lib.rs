//! ARM minting (SPEC §3.9; owned by WP3, red zone).
//!
//! Only a human can ARM (SPEC §1.3 rule 4). The constructors of [`HumanConfirm`] and of the witnesses are clippy
//! `disallowed-methods` outside one file each (§2.3 rule 11), and this crate's normal reverse dependencies are
//! exactly {`openlaser-human`, `openlaser-hw`} (§2.3 rule 4). [`mint`] binds the token to the plan fingerprint, the
//! confirmed bytes (`bytes_hash`, S22), the machine and the session, for 60 s, and only after a human-originated,
//! executed, recent frame of the same job (frame ⊇ burn, §4.6 mechanism 5).
//! `testing` (feature `test-witness`, in `src/mint.rs`) is WP0's.
#![forbid(unsafe_code)]

#[cfg(all(feature = "test-witness", not(debug_assertions)))]
compile_error!("openlaser-arm: the `test-witness` feature is test-only (dev-dependencies) and needs debug assertions");

mod mint;

pub use mint::{ARM_TEXT, CONFIRM_MAX_AGE, TTY_NONCE_RANGE, mint, tty_nonce};
pub use openlaser_types::{ArmToken, EmissionSummary, FrameReceipt, MonoTime, Refusal, SessionId};

/// Proof that a human confirmed. (no Clone) Constructors are clippy disallowed-methods outside one file each.
pub struct HumanConfirm {
    kind: ConfirmKind,
    typed: String,
    ticked: Vec<bool>,
    at: MonoTime,
}

pub enum ConfirmKind {
    GuiGesture,
    Tty {
        nonce: u64,
    },
    /// The foot switch (SPEC M3, design C3.8): a debounced press of the physical pedal, read from the board by the
    /// driver thread, replaces the typed `ARM`. Every checklist box must still be ticked.
    FootSwitch,
}

pub struct GuiGestureWitness(());
pub struct TtyWitness(());
pub struct FootSwitchWitness(());

// No `Default`: that would be a second, unpoliced way to build a witness.
#[allow(clippy::new_without_default)]
impl GuiGestureWitness {
    /// Only crates/human/src/gesture.rs.
    pub fn new() -> Self {
        GuiGestureWitness(())
    }
}

// No `Default`: that would be a second, unpoliced way to build a witness.
#[allow(clippy::new_without_default)]
impl TtyWitness {
    /// Only apps/openlaser-hw/src/tty_arm.rs.
    pub fn new() -> Self {
        TtyWitness(())
    }
}

// No `Default`: that would be a second, unpoliced way to build a witness.
#[allow(clippy::new_without_default)]
impl FootSwitchWitness {
    /// Only crates/human/src/pedal.rs.
    pub fn new() -> Self {
        FootSwitchWitness(())
    }
}

impl HumanConfirm {
    /// Only crates/human/src/pedal.rs. `at` is when the pedal press completed (its debounced edge).
    pub fn from_foot_switch(w: FootSwitchWitness, ticked: Vec<bool>, at: MonoTime) -> Self {
        let FootSwitchWitness(()) = w;
        HumanConfirm { kind: ConfirmKind::FootSwitch, typed: String::new(), ticked, at }
    }

    /// Only crates/human/src/gesture.rs.
    pub fn from_gui_gesture(w: GuiGestureWitness, typed: &str, ticked: Vec<bool>, at: MonoTime) -> Self {
        let GuiGestureWitness(()) = w;
        HumanConfirm { kind: ConfirmKind::GuiGesture, typed: typed.to_owned(), ticked, at }
    }

    /// Only apps/openlaser-hw/src/tty_arm.rs.
    pub fn from_tty(w: TtyWitness, typed: &str, nonce: u64, at: MonoTime) -> Self {
        let TtyWitness(()) = w;
        HumanConfirm { kind: ConfirmKind::Tty { nonce }, typed: typed.to_owned(), ticked: Vec::new(), at }
    }
}

/// Dev-dependency only; `compile_error!` without debug_assertions. Lives in `src/mint.rs` (SPEC §3.0).
#[cfg(feature = "test-witness")]
pub use mint::testing;

#[cfg(test)]
#[cfg(feature = "test-witness")]
mod tests;
