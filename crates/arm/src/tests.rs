//! `mint` acceptance (SPEC §7.4): one `mint_refuses_<code>` per refusal, the checklist rules, the bytes binding and
//! the 60 s token. TTY confirmations are built with a struct literal (in-crate), never through the witnesses.

use std::time::Duration;

use openlaser_types::{
    ARM_TOKEN_TTL, Bounds, EmissionSummary, FRAME_RECEIPT_MAX_AGE, Fingerprint, FrameOrigin, FrameReceipt, FrameSource, FrameStyle, MachineId, MonoTime,
    RefusalCode, SessionId,
};

use crate::testing::confirm_for_tests;
use crate::{ConfirmKind, HumanConfirm, mint};

const FP: Fingerprint = Fingerprint([0x11; 32]);
const BYTES: Fingerprint = Fingerprint([0x22; 32]);
const SESSION: SessionId = SessionId(7);
const CHECKLIST: usize = 4;

fn t(s: f64) -> MonoTime {
    MonoTime(Duration::from_secs_f64(s))
}

fn machine() -> MachineId {
    MachineId("jpt-m7-200-sim".into())
}

fn summary() -> EmissionSummary {
    EmissionSummary {
        fingerprint: FP,
        bytes_hash: BYTES,
        machine: machine(),
        rows: Vec::new(),
        notices: Vec::new(),
        repeats: 1,
        estimate: Duration::from_secs(3),
        crash_bound: Duration::from_secs(2),
        packets: 2,
        work_area: Bounds { min: [-100.0, -100.0], max: [100.0, 100.0] },
        negate: [false, false],
        swap_xy: false,
    }
}

/// Framed at t = 100 s, one executed pass, by a human, of the job.
fn receipt() -> FrameReceipt {
    FrameReceipt {
        fingerprint: FP,
        machine: machine(),
        session: SESSION,
        full_passes: 1,
        at: t(100.0),
        style: FrameStyle::Contour,
        source: FrameSource::Job,
        origin: FrameOrigin::Human,
    }
}

fn now() -> MonoTime {
    t(130.0)
}

fn gui(typed: &str) -> HumanConfirm {
    confirm_for_tests(typed, vec![true; CHECKLIST], now())
}

fn tty(typed: &str, nonce: u64) -> HumanConfirm {
    HumanConfirm { kind: ConfirmKind::Tty { nonce }, typed: typed.to_owned(), ticked: Vec::new(), at: now() }
}

/// The refusal code `mint` returns (None if it minted), after checking the message is not empty.
fn refused(r: FrameReceipt, c: HumanConfirm, checklist: usize, at: MonoTime) -> Option<RefusalCode> {
    let e = mint(&summary(), &r, c, checklist, SESSION, at).err()?;
    assert!(!e.message.is_empty());
    Some(e.code)
}

#[test]
fn mint_ok_issues_60s_token() {
    let token = mint(&summary(), &receipt(), gui("ARM"), CHECKLIST, SESSION, now()).unwrap();
    assert_eq!(token.fingerprint(), &FP);
    assert_eq!(token.machine(), &machine());
    assert_eq!(token.session(), SESSION);
    assert_eq!(token.expires(), now().plus(ARM_TOKEN_TTL));
    assert_eq!(ARM_TOKEN_TTL, Duration::from_secs(60));
    assert!(token.check(&FP, &BYTES, &machine(), SESSION, now()).is_ok());
    let last = now().plus(ARM_TOKEN_TTL - Duration::from_nanos(1));
    assert!(token.check(&FP, &BYTES, &machine(), SESSION, last).is_ok());
    let expired = token.check(&FP, &BYTES, &machine(), SESSION, now().plus(ARM_TOKEN_TTL)).unwrap_err();
    assert_eq!(expired.code, RefusalCode::ArmExpired);
}

#[test]
fn mint_token_carries_bytes_hash() {
    let token = mint(&summary(), &receipt(), gui("ARM"), CHECKLIST, SESSION, now()).unwrap();
    assert_eq!(token.bytes(), &BYTES);
    let other = Fingerprint([0x23; 32]);
    assert_eq!(token.check(&FP, &other, &machine(), SESSION, now()).unwrap_err().code, RefusalCode::BytesMismatch);
}

#[test]
fn mint_nonces_are_fresh_and_never_shown() {
    let a = mint(&summary(), &receipt(), gui("ARM"), CHECKLIST, SESSION, now()).unwrap();
    let b = mint(&summary(), &receipt(), gui("ARM"), CHECKLIST, SESSION, now()).unwrap();
    assert_ne!(a.nonce(), b.nonce(), "each mint draws a fresh 64-bit nonce");
    assert!(!format!("{a:?}").contains(&a.nonce().to_string()));
}

#[test]
fn mint_refuses_stale() {
    let mut r = receipt();
    r.fingerprint = Fingerprint([0x12; 32]);
    assert_eq!(refused(r, gui("ARM"), CHECKLIST, now()), Some(RefusalCode::Stale));
}

#[test]
fn mint_refuses_machine_mismatch() {
    let mut r = receipt();
    r.machine = MachineId("jpt-m7-200".into());
    assert_eq!(refused(r, gui("ARM"), CHECKLIST, now()), Some(RefusalCode::MachineMismatch));
}

#[test]
fn mint_refuses_arm_session_mismatch() {
    let mut r = receipt();
    r.session = SessionId(8);
    assert_eq!(refused(r, gui("ARM"), CHECKLIST, now()), Some(RefusalCode::ArmSessionMismatch));
}

#[test]
fn mint_refuses_frame_required() {
    let mut r = receipt();
    r.full_passes = 0;
    assert_eq!(refused(r, gui("ARM"), CHECKLIST, now()), Some(RefusalCode::FrameRequired));
}

#[test]
fn mint_refuses_frame_receipt_expired() {
    // Exactly 10 min old is still good; one nanosecond more is not.
    let edge = receipt().at.plus(FRAME_RECEIPT_MAX_AGE);
    let ok = mint(&summary(), &receipt(), confirm_for_tests("ARM", vec![true; CHECKLIST], edge), CHECKLIST, SESSION, edge);
    assert!(ok.is_ok());
    let late = edge.plus(Duration::from_nanos(1));
    assert_eq!(refused(receipt(), confirm_for_tests("ARM", vec![true; CHECKLIST], late), CHECKLIST, late), Some(RefusalCode::FrameReceiptExpired));
    // A receipt stamped in the future is refused too.
    assert_eq!(refused(receipt(), gui("ARM"), CHECKLIST, t(99.0)), Some(RefusalCode::FrameReceiptExpired));
}

#[test]
fn mint_refuses_frame_not_human() {
    let mut r = receipt();
    r.origin = FrameOrigin::Agent;
    assert_eq!(refused(r, gui("ARM"), CHECKLIST, now()), Some(RefusalCode::FrameNotHuman));
}

#[test]
fn mint_refuses_frame_does_not_cover_job() {
    let mut r = receipt();
    r.source = FrameSource::ToolLayersOnly;
    assert_eq!(refused(r, gui("ARM"), CHECKLIST, now()), Some(RefusalCode::FrameDoesNotCoverJob));
    // Framing the job together with the tool layers covers it; Bounds covers it as well as Contour (S21).
    let mut r = receipt();
    r.source = FrameSource::JobAndToolLayers;
    r.style = FrameStyle::Bounds;
    assert!(mint(&summary(), &r, gui("ARM"), CHECKLIST, SESSION, now()).is_ok());
}

#[test]
fn mint_refuses_arm_text_mismatch() {
    for typed in ["", "AR", "ARMED", "ARM!", "arm 123", "A R M", "yes"] {
        assert_eq!(refused(receipt(), gui(typed), CHECKLIST, now()), Some(RefusalCode::ArmTextMismatch), "{typed:?}");
    }
    for typed in ["ARM", "arm", " Arm \n"] {
        assert!(mint(&summary(), &receipt(), gui(typed), CHECKLIST, SESSION, now()).is_ok(), "{typed:?}");
    }
}

#[test]
fn mint_refuses_checklist_incomplete() {
    let at = now();
    for ticked in [vec![true, false, true, true], vec![true; 3], vec![true; 5], vec![false; 4], Vec::new()] {
        let c = confirm_for_tests("ARM", ticked.clone(), at);
        assert_eq!(refused(receipt(), c, CHECKLIST, at), Some(RefusalCode::ChecklistIncomplete), "{ticked:?}");
    }
}

#[test]
fn mint_refuses_empty_gui_checklist() {
    // An empty checklist is refused, never vacuously true.
    assert_eq!(refused(receipt(), confirm_for_tests("ARM", Vec::new(), now()), 0, now()), Some(RefusalCode::ChecklistIncomplete));
}

#[test]
fn mint_tty_needs_arm_and_the_challenge() {
    let token = mint(&summary(), &receipt(), tty("ARM 482913", 482_913), CHECKLIST, SESSION, now()).unwrap();
    assert_ne!(token.nonce(), 482_913, "the token's nonce is fresh, not the TTY challenge (1 in 2^64 to collide)");
    assert_eq!(token.bytes(), &BYTES);
    for typed in ["ARM", "ARM 482914", "482913", "ARM 482913 482913"] {
        assert_eq!(refused(receipt(), tty(typed, 482_913), CHECKLIST, now()), Some(RefusalCode::ArmTextMismatch), "{typed:?}");
    }
    // `ticked` is ignored at a TTY, but an empty checklist is still refused.
    assert_eq!(refused(receipt(), tty("ARM 482913", 482_913), 0, now()), Some(RefusalCode::ChecklistIncomplete));
    // The frame rules hold at a TTY too.
    let mut r = receipt();
    r.origin = FrameOrigin::Agent;
    assert_eq!(refused(r, tty("ARM 482913", 482_913), CHECKLIST, now()), Some(RefusalCode::FrameNotHuman));
}

#[test]
fn mint_refuses_confirmation_from_the_future() {
    let c = confirm_for_tests("ARM", vec![true; CHECKLIST], now().plus(Duration::from_secs(1)));
    assert_eq!(refused(receipt(), c, CHECKLIST, now()), Some(RefusalCode::ArmExpired));
}

#[test]
fn mint_checks_in_the_documented_order() {
    // Several problems at once: the first in §3.9's order wins.
    let mut r = receipt();
    r.fingerprint = Fingerprint([0x99; 32]);
    r.session = SessionId(1);
    r.full_passes = 0;
    assert_eq!(refused(r, gui("nope"), 0, now()), Some(RefusalCode::Stale));
    let mut r = receipt();
    r.full_passes = 0;
    r.origin = FrameOrigin::Agent;
    assert_eq!(refused(r, gui("nope"), CHECKLIST, now()), Some(RefusalCode::FrameRequired));
}
