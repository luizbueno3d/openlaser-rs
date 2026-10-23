//! The opaque ARM token (SPEC §3.1 arm.rs; owned by WP3, red zone).
//!
//! Only `crates/arm/src/mint.rs` may call [`ArmToken::issue`] (clippy `disallowed-methods`, SPEC §2.3 rule 11).
//! WP0 stub: [`ArmToken::check`] refuses every token (fail closed, §1.3 rule 7) until WP3 implements it.

use std::fmt;
use std::time::Duration;

use crate::ids::{Fingerprint, MachineId, SessionId};
use crate::refusal::{Refusal, RefusalCode};
use crate::time::MonoTime;

pub const ARM_TOKEN_TTL: Duration = Duration::from_secs(60);

/// The only key to emission. (no Clone) Private fields. Consumed by value by `DriverHandle::run`.
/// `bytes` = `EmissionSummary.bytes_hash` of the summary the human confirmed (S22). `nonce` is a fresh 64-bit
/// getrandom value for single use, independent of the 6-digit TTY challenge.
pub struct ArmToken {
    fingerprint: Fingerprint,
    bytes: Fingerprint,
    machine: MachineId,
    session: SessionId,
    expires: MonoTime,
    nonce: u64,
}

impl ArmToken {
    /// clippy `disallowed-methods` everywhere except crates/arm/src/mint.rs.
    #[doc(hidden)]
    pub fn issue(fingerprint: Fingerprint, bytes: Fingerprint, machine: MachineId, session: SessionId, expires: MonoTime, nonce: u64) -> Self {
        ArmToken { fingerprint, bytes, machine, session, expires, nonce }
    }

    /// Refusals: Stale (fingerprint), BytesMismatch, MachineMismatch, ArmSessionMismatch, ArmExpired.
    pub fn check(&self, fp: &Fingerprint, bytes: &Fingerprint, machine: &MachineId, session: SessionId, now: MonoTime) -> Result<(), Refusal> {
        let _ = (fp, bytes, machine, session, now, &self.session, self.expires);
        Err(Refusal::new(RefusalCode::NotImplemented, "not implemented: openlaser_types::arm::ArmToken::check"))
    }

    pub fn nonce(&self) -> u64 {
        self.nonce
    }

    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    pub fn bytes(&self) -> &Fingerprint {
        &self.bytes
    }

    pub fn machine(&self) -> &MachineId {
        &self.machine
    }
}

/// Fingerprint short + expiry only, never the nonce.
impl fmt::Debug for ArmToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArmToken").field("fingerprint", &self.fingerprint.short()).field("expires", &self.expires).finish_non_exhaustive()
    }
}
