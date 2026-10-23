//! `ArmToken::check` (SPEC §3.1 arm.rs, §7.4 `arm_token_check_<case>`; WP3, red zone).
//!
//! Tokens come from `openlaser_arm::testing::token_for_tests`: `ArmToken::issue` is clippy-disallowed outside
//! `crates/arm/src/mint.rs`, tests included (§2.3 rule 11). The types → arm dev-dependency cycle is used only from
//! this `tests/` file (§2.3 rule 10).

use std::time::Duration;

use openlaser_arm::testing::token_for_tests;
use openlaser_types::{ARM_TOKEN_TTL, ArmToken, Fingerprint, MachineId, MonoTime, RefusalCode, SessionId};

const FP: Fingerprint = Fingerprint([0xA1; 32]);
const BYTES: Fingerprint = Fingerprint([0xB2; 32]);
const SESSION: SessionId = SessionId(3);

fn machine() -> MachineId {
    MachineId("jpt-m7-200-sim".into())
}

fn minted_at() -> MonoTime {
    MonoTime(Duration::from_secs(1000))
}

fn token() -> ArmToken {
    token_for_tests(FP, BYTES, machine(), SESSION, minted_at().plus(ARM_TOKEN_TTL), 0x5EED)
}

/// The refusal code (None for Ok), after checking the message is not empty.
fn code(r: Result<(), openlaser_types::Refusal>) -> Option<RefusalCode> {
    let e = r.err()?;
    assert!(!e.message.is_empty());
    Some(e.code)
}

#[test]
fn arm_token_check_ok() {
    assert_eq!(token().check(&FP, &BYTES, &machine(), SESSION, minted_at()), Ok(()));
    assert_eq!(token().check(&FP, &BYTES, &machine(), SESSION, minted_at().plus(Duration::from_secs(59))), Ok(()));
}

#[test]
fn arm_token_check_stale() {
    assert_eq!(code(token().check(&Fingerprint([0xA2; 32]), &BYTES, &machine(), SESSION, minted_at())), Some(RefusalCode::Stale));
}

#[test]
fn arm_token_check_bytes_mismatch() {
    // S22: the job handed to the driver must be the bytes the human confirmed, to the last bit.
    let mut other = BYTES.0;
    other[31] ^= 1;
    assert_eq!(code(token().check(&FP, &Fingerprint(other), &machine(), SESSION, minted_at())), Some(RefusalCode::BytesMismatch));
}

#[test]
fn arm_token_check_machine_mismatch() {
    assert_eq!(code(token().check(&FP, &BYTES, &MachineId("jpt-m7-200".into()), SESSION, minted_at())), Some(RefusalCode::MachineMismatch));
}

#[test]
fn arm_token_check_session_mismatch() {
    assert_eq!(code(token().check(&FP, &BYTES, &machine(), SessionId(4), minted_at())), Some(RefusalCode::ArmSessionMismatch));
}

#[test]
fn arm_token_check_expired() {
    let t = token();
    let expires = minted_at().plus(ARM_TOKEN_TTL);
    assert_eq!(t.check(&FP, &BYTES, &machine(), SESSION, MonoTime(expires.0 - Duration::from_nanos(1))), Ok(()));
    assert_eq!(code(t.check(&FP, &BYTES, &machine(), SESSION, expires)), Some(RefusalCode::ArmExpired));
    assert_eq!(code(t.check(&FP, &BYTES, &machine(), SESSION, expires.plus(Duration::from_secs(3600)))), Some(RefusalCode::ArmExpired));
}

#[test]
fn arm_token_check_order() {
    // Everything wrong at once: the fingerprint is reported first, then bytes, machine, session, expiry.
    let late = minted_at().plus(Duration::from_secs(3600));
    let other = MachineId("other".into());
    let t = token();
    assert_eq!(code(t.check(&Fingerprint([0; 32]), &Fingerprint([0; 32]), &other, SessionId(9), late)), Some(RefusalCode::Stale));
    assert_eq!(code(t.check(&FP, &Fingerprint([0; 32]), &other, SessionId(9), late)), Some(RefusalCode::BytesMismatch));
    assert_eq!(code(t.check(&FP, &BYTES, &other, SessionId(9), late)), Some(RefusalCode::MachineMismatch));
    assert_eq!(code(t.check(&FP, &BYTES, &machine(), SessionId(9), late)), Some(RefusalCode::ArmSessionMismatch));
}

/// Fail closed (S22): an all-zero bytes hash, on the token or recomputed by the driver, matches nothing, even
/// when both sides are zero (a stubbed or broken hasher).
#[test]
fn arm_token_check_zero_bytes() {
    let zero = Fingerprint([0; 32]);
    assert_eq!(code(token().check(&FP, &zero, &machine(), SESSION, minted_at())), Some(RefusalCode::BytesMismatch));
    let zero_token = token_for_tests(FP, zero, machine(), SESSION, minted_at().plus(ARM_TOKEN_TTL), 1);
    assert_eq!(code(zero_token.check(&FP, &zero, &machine(), SESSION, minted_at())), Some(RefusalCode::BytesMismatch));
    assert_eq!(code(zero_token.check(&FP, &BYTES, &machine(), SESSION, minted_at())), Some(RefusalCode::BytesMismatch));
}

#[test]
fn arm_token_check_zero_fingerprint() {
    let zero = Fingerprint([0; 32]);
    let zero_token = token_for_tests(zero, BYTES, machine(), SESSION, minted_at().plus(ARM_TOKEN_TTL), 1);
    assert_eq!(code(zero_token.check(&zero, &BYTES, &machine(), SESSION, minted_at())), Some(RefusalCode::Stale));
    assert_eq!(code(token().check(&zero, &BYTES, &machine(), SESSION, minted_at())), Some(RefusalCode::Stale));
}

#[test]
fn arm_token_debug_never_shows_the_nonce() {
    let s = format!("{:?}", token());
    assert!(!s.contains("24301") && !s.to_lowercase().contains("5eed"), "{s}");
    assert!(s.contains(&FP.short()));
}
