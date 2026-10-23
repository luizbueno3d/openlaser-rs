//! The only file that may call `ArmToken::issue` (SPEC §2.3 rule 11).
//!
//! WP0 stubs fail closed (§1.3 rule 7): `mint` and `tty_nonce` refuse until WP3 implements them. The `testing`
//! module below is WP0's, real, and must stay in this file (§3.0, §7.12): it is the second caller of `issue`.
#![allow(clippy::disallowed_methods)]

use openlaser_types::{ArmToken, EmissionSummary, FrameReceipt, MonoTime, Refusal, RefusalCode, SessionId};

use crate::HumanConfirm;

/// Mints a single-use token valid 60 s whose `bytes` = `summary.bytes_hash` (S22). Refuses unless ALL hold:
/// summary.fingerprint == receipt.fingerprint (Stale); receipt.machine == summary.machine (MachineMismatch);
/// receipt.session == session (ArmSessionMismatch); receipt.full_passes >= 1 and age <= 10 min
/// (FrameRequired / FrameReceiptExpired); receipt.origin == Human (FrameNotHuman); receipt.covers_job()
/// (FrameDoesNotCoverJob); typed text == "ARM" (GUI, trimmed, case-insensitive) or "ARM <nonce>" (TTY)
/// (ArmTextMismatch). GuiGesture: checklist_len >= 1, ticked.len() == checklist_len and all true
/// (ChecklistIncomplete; an empty checklist is refused, never vacuously true). Tty: the caller printed the checklist
/// before the challenge, and the typed `ARM <nonce>` line is the acknowledgement; `ticked` is ignored. The token's
/// nonce is a fresh getrandom u64, not the TTY challenge.
pub fn mint(
    summary: &EmissionSummary,
    receipt: &FrameReceipt,
    confirm: HumanConfirm,
    checklist_len: usize,
    session: SessionId,
    now: MonoTime,
) -> Result<ArmToken, Refusal> {
    let HumanConfirm { kind, typed, ticked, at } = confirm;
    let _ = (summary, receipt, kind, typed, ticked, at, checklist_len, session, now);
    Err(Refusal::new(RefusalCode::NotImplemented, "not implemented: openlaser_arm::mint"))
}

/// getrandom; 6 decimal digits shown to the operator (the challenge only).
pub fn tty_nonce() -> Result<u64, Refusal> {
    Err(Refusal::new(RefusalCode::NotImplemented, "not implemented: openlaser_arm::tty_nonce"))
}

/// Dev-dependency only (feature `test-witness`; `compile_error!` without debug_assertions). Implemented in full by
/// WP0 (SPEC §3.0); referenced only from tests (§2.3 rule 14).
#[cfg(feature = "test-witness")]
pub mod testing {
    use openlaser_types::{ArmToken, Fingerprint, MachineId, MonoTime, SessionId};

    use crate::{ConfirmKind, HumanConfirm};

    pub fn confirm_for_tests(typed: &str, ticked: Vec<bool>, at: MonoTime) -> HumanConfirm {
        HumanConfirm { kind: ConfirmKind::GuiGesture, typed: typed.to_owned(), ticked, at }
    }

    /// Lets driver/engine tests (WP5, WP6) run before WP3's `mint` lands.
    pub fn token_for_tests(fingerprint: Fingerprint, bytes: Fingerprint, machine: MachineId, session: SessionId, expires: MonoTime, nonce: u64) -> ArmToken {
        ArmToken::issue(fingerprint, bytes, machine, session, expires, nonce)
    }
}

#[cfg(all(test, feature = "test-witness"))]
mod tests {
    use openlaser_types::{Fingerprint, MachineId, MonoTime, RefusalCode, SessionId};

    use super::testing::token_for_tests;

    #[test]
    fn token_for_tests_carries_what_it_was_given() {
        let t = token_for_tests(Fingerprint([1; 32]), Fingerprint([2; 32]), MachineId("m".into()), SessionId(7), MonoTime::default(), 42);
        assert_eq!(t.fingerprint(), &Fingerprint([1; 32]));
        assert_eq!(t.bytes(), &Fingerprint([2; 32]));
        assert_eq!(t.machine(), &MachineId("m".into()));
        assert_eq!(t.nonce(), 42);
        assert!(!format!("{t:?}").contains("42"), "Debug must never show the nonce");
    }

    #[test]
    fn stubs_fail_closed() {
        assert_eq!(super::tty_nonce().unwrap_err().code, RefusalCode::NotImplemented);
    }
}
