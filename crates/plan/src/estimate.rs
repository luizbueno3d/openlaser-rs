//! Time models (SPEC §3.7). Display and planning only: the confirmation's estimate is decoded from the bytes by
//! `lmc`, and the driver's run-ahead uses `lmc`'s per-packet estimates.

use std::time::Duration;

use openlaser_geom::{P2, Polyline};
use openlaser_machine::{GalvoMachine, GantryMachine};
use openlaser_types::galvo;

use crate::quantise::speed_field_mm;
use crate::{GalvoCutQ, GantryCutQ};

/// f64 seconds → `Duration`, saturating: negative or NaN → 0, too large or infinite → `Duration::MAX`.
pub(crate) fn secs(s: f64) -> Duration {
    if s.is_nan() || s <= 0.0 {
        return Duration::ZERO;
    }
    Duration::try_from_secs_f64(s).unwrap_or(Duration::MAX)
}

fn dist(a: P2, b: P2) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// Time to cover `d` mm at `v` mm/s; a zero (or non-finite) speed over a non-zero distance never ends.
fn travel(d: f64, v: f64) -> f64 {
    if d == 0.0 {
        0.0
    } else if v > 0.0 && v.is_finite() {
        d / v
    } else {
        f64::INFINITY
    }
}

/// A delay word's duration: the value counts microseconds (`JumpDelay(200)` = 200 µs, SPEC §5.2).
fn us(w: (u16, u16)) -> f64 {
    f64::from(w.0) * 1e-6
}

/// A `DelayTime` count: 10 µs units (`DelayTime(800)` = 8 ms MO settle, `DelayTime(100)` = 1 ms first-jump settle).
fn ten_us(n: u16) -> f64 {
    f64::from(n) * 10e-6
}

/// Galvo time model of one list (one layer-pass): MO settle; the first jump from the field centre at the jump
/// speed with the long jump delay and the first-jump settle; every later jump at the jump speed with the short
/// jump delay; every mark segment at the ACTUAL mark speed (from the quantised units) plus the laser-on, laser-off
/// and polygon delays per vertex. Speeds come from the words, never from the requested values.
pub fn estimate_galvo(paths: &[Polyline], q: &GalvoCutQ, m: &GalvoMachine) -> Duration {
    let f = speed_field_mm(m);
    let mark_v = galvo::units_to_speed(q.speed_units, f);
    let jump_v = galvo::units_to_speed(q.timings.jump_speed_units, f);
    let t = &q.timings;
    let per_vertex = us(t.laser_on) + us(t.laser_off) + us(t.polygon);
    let mut s = ten_us(t.mo_settle_10us);
    // Every list's first jump starts from (0x8000, 0x8000), the field centre (SPEC §5.2 rule 6).
    let mut at: P2 = [0.0, 0.0];
    let mut first = true;
    for path in paths {
        let Some(&start) = path.0.first() else { continue };
        s += travel(dist(at, start), jump_v);
        s += if first { us(t.jump_delay_long) + ten_us(t.first_jump_settle_10us) } else { us(t.jump_delay_short) };
        first = false;
        for w in path.0.windows(2) {
            if let [a, b] = w {
                s += travel(dist(*a, *b), mark_v) + per_vertex;
            }
        }
        at = path.0.last().copied().unwrap_or(start);
    }
    secs(s)
}

/// Gantry time model: travel moves at the machine's travel speed from the machine origin, burn moves at the
/// layer speed. No acceleration (a lower bound, shown as an estimate).
pub fn estimate_gantry(paths: &[Polyline], q: &GantryCutQ, m: &GantryMachine) -> Duration {
    let mut s = 0.0;
    // Paths are centre-origin bed mm; the first travel starts at the controller's origin.
    let mut at: P2 = crate::transform::gantry_home(m);
    for path in paths {
        let Some(&start) = path.0.first() else { continue };
        s += travel(dist(at, start), m.travel_speed.get());
        s += travel(path.length_mm(), q.speed.get());
        at = path.0.last().copied().unwrap_or(start);
    }
    secs(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secs_saturates_and_never_panics() {
        assert_eq!(secs(f64::NAN), Duration::ZERO);
        assert_eq!(secs(-1.0), Duration::ZERO);
        assert_eq!(secs(f64::INFINITY), Duration::MAX);
        assert_eq!(secs(1e300), Duration::MAX);
        assert_eq!(secs(1.5), Duration::from_millis(1500));
    }

    #[test]
    fn travel_handles_zero_speed() {
        assert_eq!(travel(0.0, 0.0), 0.0);
        assert_eq!(travel(10.0, 0.0), f64::INFINITY);
        assert_eq!(travel(10.0, 5.0), 2.0);
    }
}
