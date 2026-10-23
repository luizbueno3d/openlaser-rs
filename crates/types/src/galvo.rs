//! The T0 encoders (SPEC §3.1 galvo.rs; owned by WP4). Python operation order; tests read
//! `tests/golden/t0_numeric`.
//!
//! WP0 stubs: every `Result` function returns `EncodeError::NotImplemented`, the plain-value functions return
//! neutral values, and `nearest_pulse_width` refuses.

use serde::{Deserialize, Serialize};

pub const CENTER: u16 = 0x8000;
pub const JPT_M7_PULSES_NS: [u16; 20] = [2, 4, 6, 9, 13, 20, 30, 45, 55, 60, 80, 100, 150, 200, 250, 300, 350, 400, 450, 500];

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EncodeError {
    #[error("{what} is not finite")]
    NonFinite { what: &'static str },
    #[error("{what} = {value} does not fit a 16-bit word")]
    OutOfRange { what: &'static str, value: f64 },
    /// WP0 stub marker (SPEC §3.0); removed at I3.
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

/// `(32768.0 + v * (65535.0 / f)).round_ties_even()`, then `u16::try_from`. Never clamps: +F/2 gives 65536 → Err.
pub fn mm_to_galvo(v_mm: f64, field_mm: f64) -> Result<u16, EncodeError> {
    let _ = (v_mm, field_mm);
    Err(EncodeError::NotImplemented("openlaser_types::galvo::mm_to_galvo"))
}

/// OM/GP behaviour: the same, clamped to 0..=65535. T0 and py-compat only.
pub fn mm_to_galvo_om(v_mm: f64, field_mm: f64) -> u16 {
    let _ = (v_mm, field_mm);
    0
}

/// `(p * 4095.0 / 100.0).round_ties_even()`; Err outside 0..=65535 (30 % → 1228, 70 % → 2866).
pub fn power_to_mark_current(pct: f64) -> Result<u16, EncodeError> {
    let _ = pct;
    Err(EncodeError::NotImplemented("openlaser_types::galvo::power_to_mark_current"))
}

/// Shipping: `(20000.0 / khz).round_ties_even()`; Err if non-finite, <= 0, or > 65535. Never wraps.
pub fn frequency_to_period(khz: f64) -> Result<u16, EncodeError> {
    let _ = khz;
    Err(EncodeError::NotImplemented("openlaser_types::galvo::frequency_to_period"))
}

/// GP: `round_ties_even(20000 / khz) & 0xFFFF` including the negative wrap; Err only for khz == 0. T0/py-compat.
pub fn frequency_to_period_om(khz: f64) -> Result<u16, EncodeError> {
    let _ = khz;
    Err(EncodeError::NotImplemented("openlaser_types::galvo::frequency_to_period_om"))
}

/// `let gpm = 65535.0 / f; min(trunc(v * gpm / 1000.0), 65535)`; Err for negative or non-finite v.
/// 0 is a legal word here; `safety` refuses a mark speed of 0 units (B7).
pub fn speed_to_units(mm_s: f64, field_mm: f64) -> Result<u16, EncodeError> {
    let _ = (mm_s, field_mm);
    Err(EncodeError::NotImplemented("openlaser_types::galvo::speed_to_units"))
}

/// Display only: `units * f * 1000 / 65535`.
pub fn units_to_speed(units: u16, field_mm: f64) -> f64 {
    let _ = (units, field_mm);
    0.0
}

/// `(trunc(|d|), if d > 0 { 0x0000 } else { 0x8000 })`; Err if trunc(|d|) > 65535. 0 → (0, 0x8000); 0.4 → (0, 0).
pub fn delay_word(d: f64) -> Result<(u16, u16), EncodeError> {
    let _ = d;
    Err(EncodeError::NotImplemented("openlaser_types::galvo::delay_word"))
}

/// `min(trunc(hypot(dx, dy)), 65535)` on integer galvo units.
pub fn distance_word(last: (u16, u16), to: (u16, u16)) -> u16 {
    let _ = (last, to);
    0
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PulseDecision {
    Exact(u16),
    Snapped { requested: f64, to: u16 },
    Refused { requested: f64, nearest: u16 },
}

/// Ascending scan, first minimum wins ties; |Δ| <= 0.5 → Snapped; NaN or ±inf → Refused (divergence: OM snaps NaN).
pub fn nearest_pulse_width(ns: f64, table: &[u16]) -> PulseDecision {
    let _ = table;
    PulseDecision::Refused { requested: ns, nearest: 0 }
}

/// OM decision including the NaN quirk. T0 only.
pub fn nearest_pulse_width_om(ns: f64, table: &[u16]) -> PulseDecision {
    let _ = table;
    PulseDecision::Refused { requested: ns, nearest: 0 }
}
