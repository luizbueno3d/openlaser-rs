//! The opaque ARM token (SPEC §3.1 arm.rs; owned by WP3, red zone).
//!
//! Only `crates/arm/src/mint.rs` may call [`ArmToken::issue`] (clippy `disallowed-methods`, SPEC §2.3 rule 11).
//! The driver calls [`ArmToken::check`] with the fingerprint and the `bytes_hash` it RECOMPUTES from the job it was
//! handed (S22), then spends the nonce (single use is the driver's record, `ArmNonceUsed`).

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

    /// Refusals, checked in this order: Stale (fingerprint), BytesMismatch, MachineMismatch, ArmSessionMismatch,
    /// ArmExpired (valid while `now < expires`: a token minted at t is good until just before t + 60 s).
    pub fn check(&self, fp: &Fingerprint, bytes: &Fingerprint, machine: &MachineId, session: SessionId, now: MonoTime) -> Result<(), Refusal> {
        if self.fingerprint != *fp {
            return Err(Refusal::new(
                RefusalCode::Stale,
                format!("this ARM was for job {}, but the job to run is {}; frame and arm the job again", self.fingerprint.short(), fp.short()),
            ));
        }
        if self.bytes != *bytes {
            return Err(Refusal::new(
                RefusalCode::BytesMismatch,
                format!("the job's bytes ({}) are not the bytes the operator confirmed ({}); nothing was sent, arm again", bytes.short(), self.bytes.short()),
            ));
        }
        if self.machine != *machine {
            return Err(Refusal::new(
                RefusalCode::MachineMismatch,
                format!("this ARM was for machine {}, not {}; arm again on this machine", self.machine.0, machine.0),
            ));
        }
        if self.session != session {
            return Err(Refusal::new(
                RefusalCode::ArmSessionMismatch,
                format!("this ARM was for connection session {}, not {}; arm again", self.session.0, session.0),
            ));
        }
        if now >= self.expires {
            let late = now.saturating_since(self.expires);
            return Err(Refusal::new(
                RefusalCode::ArmExpired,
                format!("this ARM expired {:.1} s ago (it is valid for {} s); arm again", late.as_secs_f64(), ARM_TOKEN_TTL.as_secs()),
            ));
        }
        Ok(())
    }

    /// When the token stops being valid.
    pub fn expires(&self) -> MonoTime {
        self.expires
    }

    /// The connection session the token was minted for.
    pub fn session(&self) -> SessionId {
        self.session
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
