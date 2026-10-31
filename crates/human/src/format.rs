//! Text for the human surfaces. Every number shown comes from an `EmissionSummary` (decoded from the bytes that
//! will be sent, SPEC §1.3 rule 3); nothing here reads the document.

use std::time::Duration;

use openlaser_types::{Bounds, PassRow, SectionMode};
use openlaser_ui_egui::tf;

/// Agent notes are plain text, at most this many characters (SPEC §4.4).
pub const NOTE_MAX_CHARS: usize = 280;
/// The longest layer name shown beside its decoded numbers.
pub const NAME_MAX_CHARS: usize = 48;
/// The longest notice, checklist item or machine id shown.
pub const MESSAGE_MAX_CHARS: usize = 300;
/// The longest client name an approval card shows.
pub const CLIENT_MAX_CHARS: usize = 64;

pub fn mode(m: SectionMode) -> &'static str {
    openlaser_ui_egui::i18n::section_mode(m)
}

/// The layer key, then its (plain, bounded) name when it has one of its own.
pub fn layer(row: &PassRow) -> String {
    let key = row.layer.to_string();
    let name = plain(&row.layer_name, NAME_MAX_CHARS);
    if name.is_empty() || name == key { key } else { format!("{key} · {name}") }
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
    tf!("({:.2}, {:.2}) to ({:.2}, {:.2}) mm", "({:.2}, {:.2}) a ({:.2}, {:.2}) mm", b.min[0], b.min[1], b.max[0], b.max[1])
}

pub fn seconds(d: Duration) -> String {
    format!("{:.1} s", d.as_secs_f64())
}

/// Plain text, control characters dropped, at most [`NOTE_MAX_CHARS`] characters.
pub fn note(text: &str) -> String {
    plain(text, NOTE_MAX_CHARS)
}

/// Text that came from a document, an agent or a file, as one line of plain text: control characters (newlines
/// included) and [`invisible_format`] characters dropped, at most `max` characters (the last one `…` when cut).
pub fn plain(text: &str, max: usize) -> String {
    let kept: Vec<char> = text.chars().filter(|c| !c.is_control() && !invisible_format(*c)).collect();
    if kept.len() <= max {
        return kept.into_iter().collect();
    }
    let mut out: String = kept.into_iter().take(max.saturating_sub(1)).collect();
    if max > 0 {
        out.push('…');
    }
    out
}

/// Bidirectional overrides and isolates, zero-width and other invisible characters, the Unicode tag block: they
/// change how the rest of a line reads (or hide text) without being visible themselves, so shown text never carries
/// them.
fn invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'
            | '\u{1160}'
            | '\u{17B4}'
            | '\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{E0000}'..='\u{E007F}'
    )
}

/// The rows above the S23 first-emission cap, as "C00 at 50.00 % is above the 10 % cap".
pub fn cap_violations(rows: &[PassRow], cap_pct: Option<f64>) -> Vec<String> {
    let Some(cap) = cap_pct else { return Vec::new() };
    rows.iter()
        .filter(|r| !within_cap(r.power_pct, cap))
        .map(|r| {
            tf!(
                "{} pass {} at {} is above the {cap} % cap of this bench session's first emission",
                "{} passada {} com {} está acima do limite de {cap} % da primeira emissão desta sessão de bancada",
                layer(r),
                pass(r),
                power(r.power_pct)
            )
        })
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
        // A right-to-left override would make "Approve 05%" read backwards; zero-width characters hide text.
        assert_eq!(note("power \u{202E}%05\u{202C} ok\u{200B}\u{2066}x\u{2069}\u{FEFF}"), "power %05 okx");
        assert_eq!(note("line\u{2028}para\u{2029}end"), "lineparaend");
        assert_eq!(note("Visitenkarte, Ébauche, 名刺 ok"), "Visitenkarte, Ébauche, 名刺 ok");
    }

    #[test]
    fn layer_names_are_plain_and_bounded() {
        let mut r = row(80.0);
        r.layer_name = "C00 5.00 %\u{1b}[8m\nfake\u{202E}\u{200B}".into();
        assert_eq!(layer(&r), "C00 · C00 5.00 %[8mfake");
        r.layer_name = "n".repeat(100);
        assert_eq!(layer(&r).chars().count(), "C00 · ".chars().count() + NAME_MAX_CHARS);
        r.layer_name = "\u{200B}".into();
        assert_eq!(layer(&r), "C00", "a name that is all invisible is no name");
    }
}
