//! Quantisation of cut settings to the words the encoders send (PLAN D9): done once, here, through
//! `openlaser_types::galvo`; the encoders read only the words. `safety` calls the same functions to re-derive every
//! word of a plan and refuses a plan whose words do not match its settings (`Stale`), so the values the rules
//! judge are the values the bytes carry.

use openlaser_machine::{GalvoMachine, GantryMachine, GantryTool};
use openlaser_types::galvo::{self, PulseDecision};
use openlaser_types::{Hz, MmPerS, Ns, Percent};

use crate::{GalvoCutQ, GantryCutQ, ResolvedTimings};

/// A cut setting or machine timing that does not encode to its word. The planner turns it into a fact
/// (`unsupported`), and `safety` refuses the plan.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{what} does not encode: {message}")]
pub struct QuantiseError {
    pub what: &'static str,
    pub message: String,
}

fn qerr(what: &'static str, e: impl std::fmt::Display) -> QuantiseError {
    QuantiseError { what, message: e.to_string() }
}

/// The field size used for mark and jump speed units: galvo 1's F (SPEC §5.2 rule 4; H10 is open when the two
/// axis scales differ).
pub fn speed_field_mm(m: &GalvoMachine) -> f64 {
    m.field_f()[0]
}

/// The requested Q-pulse width a decision was made from: `None` when no pulse was set (or it was not a number).
pub fn pulse_request(d: &PulseDecision) -> Option<f64> {
    match *d {
        PulseDecision::Exact(n) => Some(f64::from(n)),
        PulseDecision::Snapped { requested, .. } | PulseDecision::Refused { requested, .. } => requested.is_finite().then_some(requested),
    }
}

/// The word a pulse decision sends, if it sends one (`Refused` sends nothing: safety refuses the job).
pub fn pulse_word(d: &PulseDecision) -> Option<u16> {
    match *d {
        PulseDecision::Exact(n) => Some(n),
        PulseDecision::Snapped { to, .. } => Some(to),
        PulseDecision::Refused { .. } => None,
    }
}

/// The decision recorded when a layer sets no Q-pulse width: refused (never guessed), `requested` NaN.
pub fn pulse_not_set(table: &[u16]) -> PulseDecision {
    PulseDecision::Refused { requested: f64::NAN, nearest: table.first().copied().unwrap_or(0) }
}

/// Machine-level timing words, the same for every galvo section of a machine.
pub fn resolve_timings(m: &GalvoMachine) -> Result<ResolvedTimings, QuantiseError> {
    let t = &m.timings;
    let laser_on = galvo::delay_word(t.laser_on_us).map_err(|e| qerr("laser-on delay", e))?;
    let laser_off = galvo::delay_word(t.laser_off_us).map_err(|e| qerr("laser-off delay", e))?;
    let polygon = galvo::delay_word(t.polygon_us).map_err(|e| qerr("polygon delay", e))?;
    let jump_delay_short = galvo::delay_word(t.jump_delay_short_us).map_err(|e| qerr("jump delay", e))?;
    // The long delay opens every list's first jump (SPEC §5.2 rule 6); it is the first-jump setting.
    let jump_delay_long = galvo::delay_word(m.first_jump.delay_us).map_err(|e| qerr("first-jump delay", e))?;
    let jump_speed_units = galvo::speed_to_units(m.jump_speed.get(), speed_field_mm(m)).map_err(|e| qerr("jump speed", e))?;
    let mo_settle_10us = ten_us_word(m.mo_settle.get())
        .ok_or_else(|| QuantiseError {
            what: "MO settle", message: format!("{} µs is not a whole, non-negative 16-bit count of 10 µs", m.mo_settle.get())
        })?;
    Ok(ResolvedTimings {
        laser_on,
        laser_off,
        polygon,
        jump_delay_short,
        jump_delay_long,
        jump_speed_units,
        mo_settle_10us,
        first_jump_settle_10us: m.first_jump.settle_10us,
    })
}

/// `trunc(us / 10)` as a 16-bit `DelayTime` count; `None` if negative, non-finite or too large.
fn ten_us_word(us: f64) -> Option<u16> {
    let v = (us / 10.0).trunc();
    // Checked whole and in 0..=65535 just above, so the conversion is exact (never a saturating cast).
    (v.is_finite() && (0.0..=f64::from(u16::MAX)).contains(&v)).then_some(v as u16)
}

/// Quantises one galvo cut setting. `Err` names the first parameter that does not encode.
pub fn quantise_galvo(speed: MmPerS, power: Percent, frequency: Hz, q_pulse: Option<Ns>, m: &GalvoMachine) -> Result<GalvoCutQ, QuantiseError> {
    let f = speed_field_mm(m);
    let speed_units = galvo::speed_to_units(speed.get(), f).map_err(|e| qerr("speed", e))?;
    let actual_speed_mm_s = galvo::units_to_speed(speed_units, f);
    let mark_current = galvo::power_to_mark_current(power.get()).map_err(|e| qerr("power", e))?;
    let q_period = galvo::frequency_to_period(frequency.khz()).map_err(|e| qerr("frequency", e))?;
    let table = &m.source.pulse_table_ns;
    let pulse = match q_pulse {
        Some(ns) => galvo::nearest_pulse_width(ns.get(), table),
        None => pulse_not_set(table),
    };
    let timings = resolve_timings(m)?;
    Ok(GalvoCutQ { requested_speed: speed, speed_units, actual_speed_mm_s, power, mark_current, frequency, q_period, pulse, timings })
}

/// A cut that encodes nothing usable: every word 0 and the pulse refused. Used only when quantisation failed, next
/// to an `unsupported` fact, so the plan still shows the layer and `safety` refuses it.
pub fn galvo_unencodable(speed: MmPerS, power: Percent, frequency: Hz, q_pulse: Option<Ns>, m: &GalvoMachine) -> GalvoCutQ {
    let table = &m.source.pulse_table_ns;
    let pulse = match q_pulse {
        Some(ns) => PulseDecision::Refused { requested: ns.get(), nearest: table.first().copied().unwrap_or(0) },
        None => pulse_not_set(table),
    };
    let zero = (0, 0);
    GalvoCutQ {
        requested_speed: speed,
        speed_units: 0,
        actual_speed_mm_s: 0.0,
        power,
        mark_current: 0,
        frequency,
        q_period: 0,
        pulse,
        timings: ResolvedTimings {
            laser_on: zero,
            laser_off: zero,
            polygon: zero,
            jump_delay_short: zero,
            jump_delay_long: zero,
            jump_speed_units: 0,
            mo_settle_10us: 0,
            first_jump_settle_10us: 0,
        },
    }
}

/// The GRBL spindle value for a power: `round_ties_even(s_max × power / 100)`, in OM `gcode.py`'s operation order
/// (`int(round(machine.power_scale * power / 100.0))`; Python `round` is ties-to-even).
pub fn s_value(power: Percent, s_max: u32) -> Option<u32> {
    let v = (f64::from(s_max) * power.get() / 100.0).round_ties_even();
    if !(v.is_finite() && v >= 0.0 && v <= f64::from(s_max)) {
        return None;
    }
    u32_from_whole(v)
}

fn u32_from_whole(v: f64) -> Option<u32> {
    // Checked whole and in 0..=u32::MAX first, so the conversion is exact (never a saturating cast).
    (v.is_finite() && v.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&v)).then_some(v as u32)
}

/// Quantises one gantry cut setting from the gantry block only (B11): max power gives `S`; `dynamic` (M4) needs a
/// dynamic laser tool and the layer not in constant-power mode (M3); air assist needs the layer and the machine.
pub fn quantise_gantry(speed: MmPerS, power: Percent, layer_air_assist: bool, constant_power: bool, m: &GantryMachine) -> Result<GantryCutQ, QuantiseError> {
    let s =
        s_value(power, m.s_max).ok_or_else(|| QuantiseError { what: "power", message: format!("{} % of S{} is not a spindle value", power.get(), m.s_max) })?;
    let dynamic = matches!(m.tool, GantryTool::Laser { dynamic: true }) && !constant_power;
    Ok(GantryCutQ { speed, power, s_value: s, air_assist: layer_air_assist && m.air_assist, dynamic })
}

/// The gantry cut recorded when quantisation failed, next to an `unsupported` fact (safety refuses the plan).
pub fn gantry_unencodable(speed: MmPerS, power: Percent) -> GantryCutQ {
    GantryCutQ { speed, power, s_value: 0, air_assist: false, dynamic: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_us_word_truncates_and_refuses() {
        assert_eq!(ten_us_word(8000.0), Some(800));
        assert_eq!(ten_us_word(8009.9), Some(800));
        assert_eq!(ten_us_word(0.0), Some(0));
        assert_eq!(ten_us_word(655_350.0), Some(65535));
        assert_eq!(ten_us_word(655_360.0), None);
        assert_eq!(ten_us_word(-10.0), None);
        assert_eq!(ten_us_word(f64::NAN), None);
    }

    #[test]
    fn s_value_rounds_half_even_and_stays_in_range() {
        let p = |v| Percent::new(v).unwrap();
        assert_eq!(s_value(p(30.0), 1000), Some(300));
        assert_eq!(s_value(p(100.0), 1000), Some(1000));
        assert_eq!(s_value(p(0.05), 1000), Some(0)); // 0.5 → 0 (ties even)
        assert_eq!(s_value(p(0.15), 1000), Some(2)); // 1.5 → 2
        assert_eq!(u32_from_whole(4_294_967_295.0), Some(u32::MAX));
        assert_eq!(u32_from_whole(1.5), None);
    }

    #[test]
    fn pulse_helpers() {
        assert_eq!(pulse_request(&PulseDecision::Exact(200)), Some(200.0));
        assert_eq!(pulse_request(&PulseDecision::Snapped { requested: 199.8, to: 200 }), Some(199.8));
        assert_eq!(pulse_request(&PulseDecision::Refused { requested: f64::NAN, nearest: 2 }), None);
        assert_eq!(pulse_word(&PulseDecision::Snapped { requested: 199.8, to: 200 }), Some(200));
        assert_eq!(pulse_word(&PulseDecision::Refused { requested: 170.0, nearest: 150 }), None);
        assert!(matches!(pulse_not_set(&[2, 4]), PulseDecision::Refused { nearest: 2, requested } if requested.is_nan()));
    }
}
