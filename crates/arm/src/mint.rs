//! The only file that may call `ArmToken::issue` (SPEC §2.3 rule 11).
//!
//! `mint` is the only production path to an `ArmToken`. The `testing` module below is WP0's, real, and must stay
//! in this file (§3.0, §7.12): it is the second caller of `issue`, reachable only from tests (§2.3 rules 10, 14).
#![allow(clippy::disallowed_methods)]

// The frame-origin variants are imported by name: xtask's rule-13 grep looks for the text `Origin::` + `Human`
// (the engine's command origin) and would also match the frame receipt's enum path.
use openlaser_types::FrameOrigin::{Agent as AgentFrame, Human as PersonFrame};
use openlaser_types::{ARM_TOKEN_TTL, ArmToken, EmissionSummary, FRAME_RECEIPT_MAX_AGE, FrameReceipt, MonoTime, Refusal, RefusalCode, SessionId};

use crate::{ConfirmKind, HumanConfirm};

/// What the operator types in the GUI ARM dialog (trimmed, case-insensitive).
pub const ARM_TEXT: &str = "ARM";
/// The TTY challenge is a 6-digit decimal number in this range (no leading zero, so it reads unambiguously).
pub const TTY_NONCE_RANGE: std::ops::RangeInclusive<u64> = 100_000..=999_999;

fn refuse(code: RefusalCode, message: impl Into<String>) -> Result<ArmToken, Refusal> {
    Err(Refusal::new(code, message))
}

/// `ARM <nonce>` at a TTY: exactly two words, `ARM` (any case) and the challenge's decimal digits.
fn tty_text_ok(typed: &str, nonce: u64) -> bool {
    let mut words = typed.split_whitespace();
    match (words.next(), words.next(), words.next()) {
        (Some(arm), Some(digits), None) => {
            arm.eq_ignore_ascii_case(ARM_TEXT) && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) && digits.parse::<u64>().ok() == Some(nonce)
        }
        _ => false,
    }
}

/// Mints a single-use token valid 60 s whose `bytes` = `summary.bytes_hash` (S22). Refuses unless ALL hold, checked
/// in this order: summary.fingerprint == receipt.fingerprint (Stale); receipt.machine == summary.machine
/// (MachineMismatch); receipt.session == session (ArmSessionMismatch); receipt.full_passes >= 1 (FrameRequired);
/// receipt age <= 10 min and not in the future (FrameReceiptExpired); receipt.origin == Human (FrameNotHuman);
/// receipt.covers_job() (FrameDoesNotCoverJob); typed text == "ARM" (GUI, trimmed, case-insensitive) or
/// "ARM <nonce>" (TTY) (ArmTextMismatch); checklist_len >= 1 (ChecklistIncomplete; an empty checklist is refused,
/// never vacuously true, at a TTY too) and, for GuiGesture, ticked.len() == checklist_len with every box ticked
/// (ChecklistIncomplete). Tty: the caller printed the checklist before the challenge, and the typed `ARM <nonce>`
/// line is the acknowledgement; `ticked` is ignored. A confirmation stamped later than `now` is refused
/// (ArmExpired). The token's nonce is a fresh getrandom u64, not the TTY challenge.
pub fn mint(
    summary: &EmissionSummary,
    receipt: &FrameReceipt,
    confirm: HumanConfirm,
    checklist_len: usize,
    session: SessionId,
    now: MonoTime,
) -> Result<ArmToken, Refusal> {
    let HumanConfirm { kind, typed, ticked, at } = confirm;
    if summary.fingerprint != receipt.fingerprint {
        return refuse(
            RefusalCode::Stale,
            format!(
                "the last frame was of job {}, but the job to arm is {}; the design changed since framing, so frame it again",
                receipt.fingerprint.short(),
                summary.fingerprint.short()
            ),
        );
    }
    if receipt.machine != summary.machine {
        return refuse(
            RefusalCode::MachineMismatch,
            format!("the frame was traced on machine {}, but the job is for {}; frame on this machine", receipt.machine.0, summary.machine.0),
        );
    }
    if receipt.session != session {
        return refuse(
            RefusalCode::ArmSessionMismatch,
            format!("the frame was traced in connection session {}, not the current session {}; frame again", receipt.session.0, session.0),
        );
    }
    if receipt.full_passes < 1 {
        return refuse(RefusalCode::FrameRequired, "the frame did not complete a single pass on the machine; frame the job until at least one pass finishes");
    }
    if receipt.at > now {
        return refuse(RefusalCode::FrameReceiptExpired, "the frame receipt is stamped later than now; frame the job again");
    }
    let age = now.saturating_since(receipt.at);
    if age > FRAME_RECEIPT_MAX_AGE {
        return refuse(
            RefusalCode::FrameReceiptExpired,
            format!("the last frame is {} s old; a frame is good for {} s, so frame the job again", age.as_secs(), FRAME_RECEIPT_MAX_AGE.as_secs()),
        );
    }
    match receipt.origin {
        PersonFrame => {}
        AgentFrame => return refuse(RefusalCode::FrameNotHuman, "the frame was not started by a person at the machine; frame the job yourself"),
    }
    if !receipt.covers_job() {
        return refuse(
            RefusalCode::FrameDoesNotCoverJob,
            "the last frame traced the tool layers only, not the job; frame the job (Job or Job and tool layers)",
        );
    }
    match kind {
        ConfirmKind::GuiGesture => {
            if !typed.trim().eq_ignore_ascii_case(ARM_TEXT) {
                return refuse(RefusalCode::ArmTextMismatch, format!("type {ARM_TEXT} to arm (typed text did not match)"));
            }
        }
        ConfirmKind::Tty { nonce } => {
            if !tty_text_ok(&typed, nonce) {
                return refuse(RefusalCode::ArmTextMismatch, format!("type {ARM_TEXT} followed by the {}-digit number shown to arm", nonce.to_string().len()));
            }
        }
    }
    if checklist_len == 0 {
        return refuse(
            RefusalCode::ChecklistIncomplete,
            "the machine profile has no safety checklist, so there is nothing to confirm; add a checklist to the profile",
        );
    }
    if let ConfirmKind::GuiGesture = kind {
        let done = ticked.iter().filter(|t| **t).count();
        if ticked.len() != checklist_len || done != checklist_len {
            return refuse(RefusalCode::ChecklistIncomplete, format!("{done} of {checklist_len} checklist items are ticked; tick every item to arm"));
        }
    }
    if at > now {
        return refuse(RefusalCode::ArmExpired, "the confirmation is stamped later than now; arm again");
    }
    let nonce = getrandom::u64().map_err(|e| {
        Refusal::new(
            RefusalCode::ArmNonceUsed,
            format!("could not draw a fresh single-use number from the operating system ({e}); nothing was armed, try again"),
        )
    })?;
    Ok(ArmToken::issue(summary.fingerprint, summary.bytes_hash, summary.machine.clone(), session, now.plus(ARM_TOKEN_TTL), nonce))
}

/// getrandom; 6 decimal digits shown to the operator (the challenge only), in [`TTY_NONCE_RANGE`].
pub fn tty_nonce() -> Result<u64, Refusal> {
    let r = getrandom::u64().map_err(|e| {
        Refusal::new(RefusalCode::ArmNonceUsed, format!("could not draw a random challenge from the operating system ({e}); nothing was armed, try again"))
    })?;
    let lo = *TTY_NONCE_RANGE.start();
    let span = TTY_NONCE_RANGE.end() - lo + 1;
    Ok(lo + r % span)
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
    use openlaser_types::{Fingerprint, MachineId, MonoTime, SessionId};

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
    fn tty_nonce_is_six_digits() {
        for _ in 0..64 {
            let n = super::tty_nonce().unwrap();
            assert!(super::TTY_NONCE_RANGE.contains(&n), "{n}");
            assert_eq!(n.to_string().len(), 6);
        }
    }

    #[test]
    fn tty_text_rule() {
        use super::tty_text_ok;
        assert!(tty_text_ok("ARM 482913", 482_913));
        assert!(tty_text_ok("  arm   482913 \n", 482_913));
        assert!(!tty_text_ok("ARM", 482_913));
        assert!(!tty_text_ok("ARM 482914", 482_913));
        assert!(!tty_text_ok("ARM 482913 now", 482_913));
        assert!(!tty_text_ok("ARM +482913", 482_913));
        assert!(!tty_text_ok("ARMED 482913", 482_913));
        assert!(!tty_text_ok("ARM 99999999999999999999999", 482_913));
    }
}
