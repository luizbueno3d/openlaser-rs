//! Text for the human surfaces. Every number shown comes from an `EmissionSummary` (decoded from the bytes that
//! will be sent, SPEC §1.3 rule 3); nothing here reads the document.

use std::time::Duration;

use openlaser_types::{Bounds, PassRow, SectionMode};

/// Agent notes are plain text, at most this many characters (SPEC §4.4).
pub const NOTE_MAX_CHARS: usize = 280;

pub fn mode(m: SectionMode) -> &'static str {
    match m {
        SectionMode::Line => "Line",
        SectionMode::Fill => "Fill",
        SectionMode::Image => "Image",
    }
}

pub fn layer(row: &PassRow) -> String {
    let key = row.layer.to_string();
    if row.layer_name.is_empty() || row.layer_name == key { key } else { format!("{key} · {}", row.layer_name) }
}

pub fn pass(row: &PassRow) -> String {
    format!("{}/{}", row.pass, row.of)
}

pub fn power(pct: f64) -> String {
    format!("{pct:.2} %")
}

pub fn frequency(khz: Option<f64>) -> String {
    match khz {
        Some(k) => format!("{k:.2} kHz"),
        None => "—".to_owned(),
    }
}

pub fn pulse(ns: Option<u16>) -> String {
    match ns {
        Some(n) => format!("{n} ns"),
        None => "—".to_owned(),
    }
}

pub fn speed(mm_s: f64) -> String {
    format!("{mm_s:.1} mm/s")
}

pub fn bounds(b: &Bounds) -> String {
    format!("({:.2}, {:.2}) to ({:.2}, {:.2}) mm", b.min[0], b.min[1], b.max[0], b.max[1])
}

pub fn seconds(d: Duration) -> String {
    format!("{:.1} s", d.as_secs_f64())
}

/// Plain text, control characters dropped, at most [`NOTE_MAX_CHARS`] characters.
pub fn note(text: &str) -> String {
    text.chars().filter(|c| !c.is_control() || *c == ' ').take(NOTE_MAX_CHARS).collect()
}

/// The rows above the S23 first-emission cap, as "C00 at 50.00 % is above the 10 % cap".
pub fn cap_violations(rows: &[PassRow], cap_pct: Option<f64>) -> Vec<String> {
    let Some(cap) = cap_pct else { return Vec::new() };
    rows.iter()
        .filter(|r| !within_cap(r.power_pct, cap))
        .map(|r| format!("{} pass {} at {} is above the {cap} % cap of this bench session's first emission", layer(r), pass(r), power(r.power_pct)))
        .collect()
}

/// A non-finite power or cap is never within the cap.
fn within_cap(power_pct: f64, cap: f64) -> bool {
    power_pct.is_finite() && cap.is_finite() && power_pct <= cap
}

#[cfg(test)]
mod tests {
    use openlaser_types::{Fingerprint, LayerKey};

    use super::*;

    fn row(power_pct: f64) -> PassRow {
        PassRow {
            layer: LayerKey::Color(0),
            layer_name: "C00".into(),
            sub: 0,
            pass: 1,
            of: 1,
            mode: SectionMode::Line,
            power_pct,
            frequency_khz: Some(20_000.0 / 667.0),
            pulse_ns: Some(200),
            speed_mm_s: 996.8,
            galvo: None,
            paths: 1,
            mark_mm: 40.0,
            bounds: Bounds { min: [-5.0, -5.0], max: [5.0, 5.0] },
            estimate: Duration::from_millis(62),
        }
    }

    #[test]
    fn numbers_render_from_the_decoded_values() {
        let r = row(29.987);
        assert_eq!(power(r.power_pct), "29.99 %");
        assert_eq!(frequency(r.frequency_khz), "29.99 kHz");
        assert_eq!(pulse(r.pulse_ns), "200 ns");
        assert_eq!(speed(r.speed_mm_s), "996.8 mm/s");
        assert_eq!(layer(&r), "C00");
        assert_eq!(bounds(&r.bounds), "(-5.00, -5.00) to (5.00, 5.00) mm");
        let _ = Fingerprint([0; 32]);
    }

    #[test]
    fn cap_refuses_rows_above_it_and_non_finite_power() {
        assert!(cap_violations(&[row(10.0), row(9.9)], Some(10.0)).is_empty());
        assert_eq!(cap_violations(&[row(10.01), row(5.0)], Some(10.0)).len(), 1);
        assert_eq!(cap_violations(&[row(f64::NAN)], Some(10.0)).len(), 1);
        assert!(cap_violations(&[row(100.0)], None).is_empty());
    }

    #[test]
    fn notes_are_plain_and_bounded() {
        let long: String = "x".repeat(400);
        assert_eq!(note(&long).chars().count(), NOTE_MAX_CHARS);
        assert_eq!(note("a\u{1b}[31mb\nc"), "a[31mbc");
    }
}
