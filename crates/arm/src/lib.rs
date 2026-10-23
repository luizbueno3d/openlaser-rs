//! ARM minting (SPEC §3.9; owned by WP3, red zone).
//!
//! Only a human can ARM (SPEC §1.3 rule 4). The constructors of [`HumanConfirm`] and of the witnesses are clippy
//! `disallowed-methods` outside one file each (§2.3 rule 11). WP0 stubs fail closed: `mint` and `tty_nonce` refuse.
//! `testing` (feature `test-witness`, in `src/mint.rs`) is WP0's and real.
#![forbid(unsafe_code)]

#[cfg(all(feature = "test-witness", not(debug_assertions)))]
compile_error!("openlaser-arm: the `test-witness` feature is test-only (dev-dependencies) and needs debug assertions");

mod mint;

pub use mint::{mint, tty_nonce};
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
    Tty { nonce: u64 },
}

pub struct GuiGestureWitness(());
pub struct TtyWitness(());

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

impl HumanConfirm {
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
