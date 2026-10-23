//! The T0 encoders (SPEC §3.1 galvo.rs; owned by WP4). Python operation order; tests read
//! `tests/golden/t0_numeric` (every row of the seven tables, `crates/lmc/tests/lmc/t0_numeric.rs`).
//!
//! Rounding follows Python exactly: `round()` is ties-to-even (`f64::round_ties_even`), `int()` truncates. Every
//! shipping encoder refuses a value it cannot represent instead of clamping or wrapping; the `_om` variants keep the
//! prototype's clamping, wrapping and NaN behaviour and exist only for the T0 tables and `py-compat`.
//!
//! Formulas ported from galvoplotter 0.2.0 (`galvo/controller.py`: `_convert_speed`, `_convert_frequency`,
//! `_convert_power`, `list_jump`, `list_*_delay`; meerk40t/galvoplotter, MIT) and the OpenMopa prototype
//! (`mopa_luiz/live.py` `mm_to_galvo`, `mopa_luiz/cli.py` `nearest_pulse_width`; OM @6aed8ff, MIT).

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

/// The largest 16-bit word, as f64.
const WORD_MAX: f64 = 65535.0;

/// The one f64 → u16 narrowing of this module. `w` must be a whole number in 0..=65535; anything else is refused
/// (never clamped, never wrapped). `value` is what the error reports (the caller's input, not `w`).
fn whole_word(w: f64, what: &'static str, value: f64) -> Result<u16, EncodeError> {
    if !w.is_finite() {
        return Err(EncodeError::NonFinite { what });
    }
    if w.fract() != 0.0 || w < 0.0 || w > WORD_MAX {
        return Err(EncodeError::OutOfRange { what, value });
    }
    // Explicit, commented narrowing rule (SPEC §2.5): `w` is finite, whole and inside 0..=65535 (checked above), so
    // the cast is exact; -0.0 becomes 0.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(w as u16)
}

fn finite(v: f64, what: &'static str) -> Result<f64, EncodeError> {
    if v.is_finite() { Ok(v) } else { Err(EncodeError::NonFinite { what }) }
}

/// `(32768.0 + v * (65535.0 / f)).round_ties_even()`, then `u16::try_from`. Never clamps: +F/2 gives 65536 → Err.
pub fn mm_to_galvo(v_mm: f64, field_mm: f64) -> Result<u16, EncodeError> {
    let v = finite(v_mm, "coordinate")?;
    let f = finite(field_mm, "field size")?;
    if f <= 0.0 {
        return Err(EncodeError::OutOfRange { what: "field size", value: f });
    }
    let g = (32768.0 + v * (65535.0 / f)).round_ties_even();
    whole_word(g, "coordinate", v_mm)
}

/// OM/GP behaviour: the same, clamped to 0..=65535. T0 and py-compat only.
///
/// Python raises for NaN (`int(round(nan))`); this plain-value function maps NaN to [`CENTER`] instead. The
/// prototype's `geometry_errors` refuses non-finite points before this is ever reached, as `py_compat` does.
pub fn mm_to_galvo_om(v_mm: f64, field_mm: f64) -> u16 {
    let g = (32768.0 + v_mm * (65535.0 / field_mm)).round_ties_even();
    if g.is_nan() {
        return CENTER;
    }
    whole_word(g.clamp(0.0, WORD_MAX), "coordinate", v_mm).unwrap_or(CENTER)
}

/// `(p * 4095.0 / 100.0).round_ties_even()`; Err outside 0..=65535 (30 % → 1228, 70 % → 2866).
pub fn power_to_mark_current(pct: f64) -> Result<u16, EncodeError> {
    let p = finite(pct, "power")?;
    let w = (p * 4095.0 / 100.0).round_ties_even();
    whole_word(w, "power", pct)
}

/// Shipping: `(20000.0 / khz).round_ties_even()`; Err if non-finite, <= 0, or > 65535. Never wraps.
///
/// A period of 0 (above 40 MHz) is refused as well: the board would divide by it, and the summary's
/// `20000 / q_period` would be infinite.
pub fn frequency_to_period(khz: f64) -> Result<u16, EncodeError> {
    let f = finite(khz, "frequency")?;
    if f <= 0.0 {
        return Err(EncodeError::OutOfRange { what: "frequency", value: khz });
    }
    let p = (20000.0 / f).round_ties_even();
    if p < 1.0 {
        return Err(EncodeError::OutOfRange { what: "frequency", value: khz });
    }
    whole_word(p, "frequency", khz)
}

/// GP: `round_ties_even(20000 / khz) & 0xFFFF` including the negative wrap; Err only for khz == 0. T0/py-compat.
///
/// NaN is refused too (Python raises `ValueError` on `round(nan)`); ±inf gives 0 as in Python.
pub fn frequency_to_period_om(khz: f64) -> Result<u16, EncodeError> {
    if khz == 0.0 {
        return Err(EncodeError::OutOfRange { what: "frequency", value: khz });
    }
    if khz.is_nan() {
        return Err(EncodeError::NonFinite { what: "frequency" });
    }
    let p = (20000.0 / khz).round_ties_even();
    // Python's `& 0xFFFF` on an int is the two's-complement residue mod 2^16. `rem_euclid` of a whole f64 is exact
    // (IEEE fmod), so this is exact for every finite p, including huge ones Python holds as big ints.
    let w = p.rem_euclid(65536.0);
    whole_word(w, "frequency", khz)
}

/// `let gpm = 65535.0 / f; min(trunc(v * gpm / 1000.0), 65535)`; Err for negative or non-finite v.
/// 0 is a legal word here; `safety` refuses a mark speed of 0 units (B7).
pub fn speed_to_units(mm_s: f64, field_mm: f64) -> Result<u16, EncodeError> {
    let v = finite(mm_s, "speed")?;
    let f = finite(field_mm, "field size")?;
    if v < 0.0 {
        return Err(EncodeError::OutOfRange { what: "speed", value: mm_s });
    }
    if f <= 0.0 {
        return Err(EncodeError::OutOfRange { what: "field size", value: f });
    }
    let gpm = 65535.0 / f;
    let w = (v * gpm / 1000.0).trunc().min(WORD_MAX);
    whole_word(w, "speed", mm_s)
}

/// Display only: `units * f * 1000 / 65535`.
pub fn units_to_speed(units: u16, field_mm: f64) -> f64 {
    f64::from(units) * field_mm * 1000.0 / 65535.0
}

/// `(trunc(|d|), if d > 0 { 0x0000 } else { 0x8000 })`; Err if trunc(|d|) > 65535. 0 → (0, 0x8000); 0.4 → (0, 0).
pub fn delay_word(d: f64) -> Result<(u16, u16), EncodeError> {
    let v = finite(d, "delay")?;
    let magnitude = whole_word(v.abs().trunc(), "delay", d)?;
    let sign = if v > 0.0 { 0x0000 } else { 0x8000 };
    Ok((magnitude, sign))
}

/// `min(trunc(hypot(dx, dy)), 65535)` on integer galvo units.
///
/// Computed exactly in integers: the floor of the square root of `dx² + dy²` equals Python's
/// `int(abs(complex(x, y) - complex(lx, ly)))` for every pair of 16-bit points (the sum is below 2^33, and a
/// correctly rounded square root of an integer never crosses the next integer at this size).
pub fn distance_word(last: (u16, u16), to: (u16, u16)) -> u16 {
    let dx = u64::from(last.0.abs_diff(to.0));
    let dy = u64::from(last.1.abs_diff(to.1));
    let d = (dx * dx + dy * dy).isqrt();
    u16::try_from(d).unwrap_or(u16::MAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PulseDecision {
    Exact(u16),
    Snapped { requested: f64, to: u16 },
    Refused { requested: f64, nearest: u16 },
}

impl PulseDecision {
    /// The pulse width word to send: `Some` for `Exact` and `Snapped`, `None` for `Refused`.
    pub fn word(&self) -> Option<u16> {
        match *self {
            PulseDecision::Exact(w) => Some(w),
            PulseDecision::Snapped { to, .. } => Some(to),
            PulseDecision::Refused { .. } => None,
        }
    }
}

/// Python's `min(table, key=lambda t: abs(t - v))`: ascending scan, the first minimum wins ties, and a NaN key never
/// replaces the current best (every comparison with NaN is false). Returns the entry and its distance.
fn nearest_entry(ns: f64, table: &[u16]) -> Option<(u16, f64)> {
    let mut best: Option<(u16, f64)> = None;
    for &t in table {
        let key = (f64::from(t) - ns).abs();
        match best {
            None => best = Some((t, key)),
            Some((_, best_key)) if key < best_key => best = Some((t, key)),
            Some(_) => {}
        }
    }
    best
}

/// Ascending scan, first minimum wins ties; |Δ| <= 0.5 → Snapped; NaN or ±inf → Refused (divergence: OM snaps NaN).
pub fn nearest_pulse_width(ns: f64, table: &[u16]) -> PulseDecision {
    let Some((nearest, distance)) = nearest_entry(ns, table) else {
        return PulseDecision::Refused { requested: ns, nearest: 0 };
    };
    if !ns.is_finite() {
        return PulseDecision::Refused { requested: ns, nearest };
    }
    if ns == f64::from(nearest) {
        PulseDecision::Exact(nearest)
    } else if distance <= 0.5 {
        PulseDecision::Snapped { requested: ns, to: nearest }
    } else {
        PulseDecision::Refused { requested: ns, nearest }
    }
}

/// OM decision including the NaN quirk. T0 only.
///
/// OM refuses when `abs(nearest - v) > 0.5`; for NaN that comparison is false, so NaN "snaps" to the first entry.
pub fn nearest_pulse_width_om(ns: f64, table: &[u16]) -> PulseDecision {
    let Some((nearest, distance)) = nearest_entry(ns, table) else {
        return PulseDecision::Refused { requested: ns, nearest: 0 };
    };
    if ns == f64::from(nearest) {
        PulseDecision::Exact(nearest)
    } else if distance > 0.5 {
        PulseDecision::Refused { requested: ns, nearest }
    } else {
        PulseDecision::Snapped { requested: ns, to: nearest }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: f64 = 200.0 * 1.1834;

    #[test]
    fn spec_reference_values() {
        // SPEC §5.2 rule 1 and R05 §1.7, §1.8.
        assert_eq!(power_to_mark_current(30.0), Ok(1228));
        assert_eq!(power_to_mark_current(70.0), Ok(2866));
        assert_eq!(power_to_mark_current(50.0), Ok(2048));
        assert_eq!(frequency_to_period(30.0), Ok(667));
        assert_eq!(frequency_to_period(45.0), Ok(444));
        assert_eq!(speed_to_units(1000.0, F), Ok(276));
        assert_eq!(speed_to_units(3000.0, F), Ok(830));
        assert_eq!(mm_to_galvo(5.0, F), Ok(34152));
        assert_eq!(mm_to_galvo(-5.0, F), Ok(31384));
        assert_eq!(mm_to_galvo(0.05, F), Ok(32782));
        assert_eq!(mm_to_galvo(25.0, 200.0), Ok(40960));
        assert_eq!(distance_word((0x8000, 0x8000), (31384, 31384)), 1957);
        assert_eq!(distance_word((31384, 31384), (34152, 31384)), 2768);
        assert_eq!(distance_word((0, 0), (0xFFFF, 0xFFFF)), 0xFFFF);
    }

    #[test]
    fn plus_half_field_is_refused_not_clamped() {
        assert!(matches!(mm_to_galvo(F / 2.0, F), Err(EncodeError::OutOfRange { .. })));
        assert_eq!(mm_to_galvo_om(F / 2.0, F), 0xFFFF);
        assert_eq!(mm_to_galvo(-F / 2.0, F), Ok(0));
        assert!(matches!(mm_to_galvo(f64::NAN, F), Err(EncodeError::NonFinite { .. })));
        assert!(matches!(mm_to_galvo(1.0, 0.0), Err(EncodeError::OutOfRange { .. })));
        assert_eq!(mm_to_galvo_om(f64::NAN, F), CENTER);
        assert_eq!(mm_to_galvo_om(-1000.0, F), 0);
    }

    #[test]
    fn delays_keep_the_negative_zero_sign_word() {
        assert_eq!(delay_word(0.0), Ok((0, 0x8000)));
        assert_eq!(delay_word(-0.0), Ok((0, 0x8000)));
        assert_eq!(delay_word(0.4), Ok((0, 0)));
        assert_eq!(delay_word(100.0), Ok((100, 0)));
        assert_eq!(delay_word(-8.9), Ok((8, 0x8000)));
        assert_eq!(delay_word(65535.0), Ok((65535, 0)));
        assert!(delay_word(65536.0).is_err());
        assert!(delay_word(f64::INFINITY).is_err());
    }

    #[test]
    fn frequencies_never_wrap_in_shipping() {
        assert!(frequency_to_period(0.3).is_err());
        assert_eq!(frequency_to_period_om(0.3), Ok(1131));
        assert_eq!(frequency_to_period_om(-1.0), Ok(45536));
        assert!(frequency_to_period(-1.0).is_err());
        assert!(frequency_to_period(0.0).is_err());
        assert!(frequency_to_period_om(0.0).is_err());
        assert!(frequency_to_period(1e6).is_err());
        assert_eq!(frequency_to_period_om(1e6), Ok(0));
        assert_eq!(frequency_to_period(4000.0), Ok(5));
    }

    #[test]
    fn speed_saturates_and_refuses_negative() {
        assert_eq!(speed_to_units(3.6, F), Ok(0));
        assert_eq!(speed_to_units(3.62, F), Ok(1));
        assert_eq!(speed_to_units(1e7, F), Ok(65535));
        assert!(speed_to_units(-1.0, F).is_err());
        assert!(speed_to_units(f64::NAN, F).is_err());
        assert!((units_to_speed(276, F) - 276.0 * F * 1000.0 / 65535.0).abs() < 1e-12);
    }

    #[test]
    fn power_refuses_words_outside_16_bits() {
        assert_eq!(power_to_mark_current(-0.001), Ok(0));
        assert!(power_to_mark_current(-1.0).is_err());
        assert!(power_to_mark_current(1600.4).is_err());
        assert_eq!(power_to_mark_current(101.0), Ok(4136));
    }

    #[test]
    fn pulse_width_decisions() {
        let t = &JPT_M7_PULSES_NS;
        assert_eq!(nearest_pulse_width(200.0, t), PulseDecision::Exact(200));
        assert_eq!(nearest_pulse_width(199.8, t), PulseDecision::Snapped { requested: 199.8, to: 200 });
        assert_eq!(nearest_pulse_width(170.0, t), PulseDecision::Refused { requested: 170.0, nearest: 150 });
        // Ties keep the first (lower) entry.
        assert_eq!(nearest_pulse_width(175.0, t), PulseDecision::Refused { requested: 175.0, nearest: 150 });
        assert!(matches!(nearest_pulse_width(f64::NAN, t), PulseDecision::Refused { nearest: 2, .. }));
        assert!(matches!(nearest_pulse_width_om(f64::NAN, t), PulseDecision::Snapped { to: 2, .. }));
        assert!(matches!(nearest_pulse_width_om(f64::INFINITY, t), PulseDecision::Refused { nearest: 2, .. }));
        assert_eq!(nearest_pulse_width(5.0, &[]), PulseDecision::Refused { requested: 5.0, nearest: 0 });
        assert_eq!(PulseDecision::Snapped { requested: 1.0, to: 2 }.word(), Some(2));
        assert_eq!(PulseDecision::Refused { requested: 1.0, nearest: 2 }.word(), None);
    }

    #[test]
    fn distance_word_matches_float_hypot() {
        for (a, b) in [((0u16, 0u16), (3u16, 4u16)), ((1000, 1000), (1003, 1004)), ((0, 0), (30, 1)), ((5, 7), (5, 7))] {
            let dx = f64::from(b.0) - f64::from(a.0);
            let dy = f64::from(b.1) - f64::from(a.1);
            assert_eq!(f64::from(distance_word(a, b)), dx.hypot(dy).trunc());
        }
    }
}
