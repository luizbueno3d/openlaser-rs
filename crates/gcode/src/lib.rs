//! GRBL G-code export (SPEC §3.11).
//!
//! Ported from OpenMopa `mopa_luiz/gcode.py` (`_number`, `_xy`, `_distance`, `compile_gcode`) at commit
//! `6aed8ffa8be5a938d195bf35198cfd168bb07f4e`. MIT © 2026 Luiz Bueno (see ATTRIBUTION.md).
//! [`om::compile_gcode_om`] reproduces OM's bytes (T2); the shipping [`compile`] keeps OM's program layout but
//! diverges on purpose (tests/golden/README.md, "Intentional divergences"):
//! - B11: only the gantry cut block is read; a section carrying galvo settings refuses the whole job;
//! - speed above the machine maximum, S above `s_max`, a non-finite number and a path of fewer than 2 points are
//!   refusals (OM caps the speed silently);
//! - text interpolated into comments (machine name, timestamp, layer name) is reduced to printable ASCII without
//!   GRBL's real-time characters: GRBL ends a line at CR or LF and acts on `!`, `~`, `?`, Ctrl-X and the bytes
//!   0x80–0xFF (feed, rapid and spindle/laser-power overrides) wherever they appear, comments included;
//! - the job is repeated `EmitPlan::repeats` times.
//!
//! Coordinates: `Section.paths` and `FramePlan::paths` for a gantry machine are centre-origin bed millimetres,
//! Y up (the frame OM's `_xy` takes). The encoder maps them to the controller's origin
//! ([`openlaser_machine::WorkspaceOrigin`]) and refuses any point outside the bed.
#![forbid(unsafe_code)]
#![deny(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_possible_wrap)]

use std::time::Duration;

use openlaser_plan::CutQ;
use openlaser_types::{DeviceKind, SectionMode};

pub mod om;

pub use openlaser_machine::GantryMachine;
pub use openlaser_safety::{EmitPlan, FramePlan};
pub use openlaser_types::{Fingerprint, MachineId};

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Longest text interpolated into one comment line (keeps every line short enough for GRBL's 128-byte receive
/// buffer under character-counting streaming).
pub const MAX_COMMENT_CHARS: usize = 60;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("G-code refused: {0}")]
    Refused(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct GcodeProgram {
    pub text: String,
    pub lines: u32,
    pub fingerprint: Fingerprint,
    pub machine: MachineId,
    pub distance_mm: f64,
    pub estimate: Duration,
}

impl GcodeProgram {
    /// blake3 of `text` (S22).
    pub fn bytes_hash(&self) -> Fingerprint {
        Fingerprint(*blake3::hash(self.text.as_bytes()).as_bytes())
    }
}

/// (sealed) A laser-off frame program: built only by `frame_program`, so GRBL's `Backend::Frame` cannot be handed a
/// burning program (the LMC side gets the same guarantee from `LightingList`).
#[derive(Debug, Clone, PartialEq)]
pub struct GcodeFrame {
    program: GcodeProgram,
}

impl GcodeFrame {
    pub fn program(&self) -> &GcodeProgram {
        &self.program
    }
}

/// Tests pass "<MASKED>".
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub machine_name: String,
    pub generated: String,
}

fn refuse(message: impl Into<String>) -> Error {
    Error::Refused(message.into())
}

/// Shipping encoder (B11: only the gantry block is read). Overspeed never reaches here: safety refuses it.
/// `distance_mm` is the burn (G1) distance; `estimate` is burn distance at each section's speed plus travel (G0,
/// including the return to X0 Y0) at the machine's travel speed, over every repeat.
pub fn compile(emit: &EmitPlan, m: &GantryMachine, h: &Header) -> Result<GcodeProgram, Error> {
    check_machine(m)?;
    if emit.device() != DeviceKind::Grbl {
        return Err(refuse(format!("plan is for a {:?} device; G-code export needs a GRBL plan", emit.device())));
    }
    let repeats = emit.repeats();
    if repeats == 0 {
        return Err(refuse("the job has 0 repeats"));
    }
    // Every section is checked before a line is written: one bad section refuses the whole job.
    let mut cuts = Vec::with_capacity(emit.sections().len());
    for (index, section) in emit.sections().iter().enumerate() {
        let what = || format!("section {} (layer {}, pass {}/{})", index + 1, comment_text(&section.layer_name), section.pass, section.of);
        let CutQ::Gantry(cut) = &section.cut else {
            return Err(refuse(format!("{} carries galvo cut settings; G-code takes only a layer's gantry settings (B11)", what())));
        };
        let speed = cut.speed.get();
        if !speed.is_finite() || speed <= 0.0 {
            return Err(refuse(format!("{}: cut speed must be finite and positive", what())));
        }
        if speed > m.max_speed.get() {
            return Err(refuse(format!("{}: speed {speed} mm/s exceeds the machine maximum {} mm/s", what(), m.max_speed.get())));
        }
        if cut.s_value > m.s_max {
            return Err(refuse(format!("{}: power S{} exceeds the machine maximum S{}", what(), cut.s_value, m.s_max)));
        }
        for (path_index, path) in section.paths.iter().enumerate() {
            if path.0.len() < 2 {
                return Err(refuse(format!("{}: path {} has {} point(s); a burn path needs at least 2", what(), path_index + 1, path.0.len())));
            }
        }
        cuts.push(cut);
    }
    let mut lines = header_lines(&h.machine_name, &h.generated);
    let mut burn_mm = 0.0;
    let mut burn_s = 0.0;
    let mut travel_mm = 0.0;
    let mut position = [0.0, 0.0];
    let mut paths = 0usize;
    for repeat in 0..repeats {
        if repeats > 1 {
            lines.push(format!("; Repeat {}/{repeats}", repeat + 1));
        }
        for (section, cut) in emit.sections().iter().zip(&cuts) {
            let speed = cut.speed.get();
            let feed = try_number(speed * 60.0)?;
            let air = cut.air_assist && m.air_assist;
            lines.push(format!("; Layer: {} · {}", comment_text(&section.layer_name), mode_name(section.mode)));
            if air {
                lines.push("M8".to_owned());
            }
            for path in &section.paths {
                let points = path.0.iter().map(|&p| machine_xy(p, m)).collect::<Result<Vec<_>, _>>()?;
                let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
                    return Err(refuse("a burn path has no points"));
                };
                lines.push("M5".to_owned());
                lines.push(format!("G0 X{} Y{}", try_number(first[0])?, try_number(first[1])?));
                travel_mm += distance(position, first);
                match &m.tool {
                    openlaser_machine::GantryTool::Pen => lines.push("M3".to_owned()),
                    openlaser_machine::GantryTool::Laser { .. } => lines.push(format!("{} S{}", if cut.dynamic { "M4" } else { "M3" }, cut.s_value)),
                }
                let mut length = 0.0;
                for pair in points.windows(2) {
                    let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else { continue };
                    lines.push(format!("G1 X{} Y{} F{feed}", try_number(b[0])?, try_number(b[1])?));
                    length += distance(a, b);
                }
                lines.push("M5".to_owned());
                burn_mm += length;
                burn_s += length / speed;
                position = last;
                paths += 1;
            }
            if air {
                lines.push("M9".to_owned());
            }
        }
    }
    if paths == 0 {
        return Err(refuse("no output-enabled geometry for G-code export"));
    }
    travel_mm += distance(position, MACHINE_ORIGIN);
    lines.extend(["M5".to_owned(), "M9".to_owned(), "G0 X0 Y0".to_owned(), "; End OpenLaser job".to_owned(), String::new()]);
    let seconds = burn_s + travel_mm / m.travel_speed.get();
    finish(lines, *emit.fingerprint(), emit.machine().clone(), burn_mm, seconds)
}

/// Laser-off frame around the job (G0 only, no M3/M4/S>0; a test greps the text). A one-point path is a move to
/// that point; an empty path refuses. No caller text enters the frame program (its comments are fixed, so even a
/// naive grep for `M3` stays meaningful), and the text is checked line by line before it is sealed.
pub fn frame_program(frame: &FramePlan, m: &GantryMachine, h: &Header) -> Result<GcodeFrame, Error> {
    check_machine(m)?;
    let _ = h;
    let mut lines: Vec<String> =
        ["; OpenLaser frame (laser off)", "; Design origin: center; units: millimetres", "G21", "G90", "M5"].into_iter().map(str::to_owned).collect();
    let mut travel_mm = 0.0;
    let mut position = [0.0, 0.0];
    for (index, path) in frame.paths().iter().enumerate() {
        if path.0.is_empty() {
            return Err(refuse(format!("frame path {} is empty", index + 1)));
        }
        for &point in &path.0 {
            let p = machine_xy(point, m)?;
            lines.push(format!("G0 X{} Y{}", try_number(p[0])?, try_number(p[1])?));
            travel_mm += distance(position, p);
            position = p;
        }
    }
    if frame.paths().is_empty() {
        return Err(refuse("nothing to frame"));
    }
    travel_mm += distance(position, MACHINE_ORIGIN);
    lines.extend(["M5".to_owned(), "G0 X0 Y0".to_owned(), "; End OpenLaser frame".to_owned(), String::new()]);
    let program = finish(lines, *frame.fingerprint(), frame.machine().clone(), 0.0, travel_mm / m.travel_speed.get())?;
    check_laser_off(&program.text)?;
    Ok(GcodeFrame { program })
}

/// Defence in depth for the sealed frame: every line is a comment, `G21`, `G90`, `M5` or `G0 X… Y…`.
fn check_laser_off(text: &str) -> Result<(), Error> {
    for line in text.lines() {
        let ok = line.starts_with(';')
            || matches!(line, "G21" | "G90" | "M5")
            || line.strip_prefix("G0 X").is_some_and(|rest| rest.split_once(" Y").is_some_and(|(x, y)| is_number(x) && is_number(y)));
        if !ok {
            return Err(refuse(format!("frame program line {line:?} is not a laser-off move")));
        }
    }
    Ok(())
}

fn is_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b'-') && text.parse::<f64>().is_ok_and(f64::is_finite)
}

fn finish(lines: Vec<String>, fingerprint: Fingerprint, machine: MachineId, distance_mm: f64, seconds: f64) -> Result<GcodeProgram, Error> {
    let text = lines.join("\n");
    let line_count = u32::try_from(lines.len().saturating_sub(1)).map_err(|_| refuse("G-code has too many lines"))?;
    let estimate = Duration::try_from_secs_f64(seconds).map_err(|e| refuse(format!("estimate: {e}")))?;
    Ok(GcodeProgram { text, lines: line_count, fingerprint, machine, distance_mm, estimate })
}

fn check_machine(m: &GantryMachine) -> Result<(), Error> {
    let [w, h] = m.bed_mm;
    if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return Err(refuse(format!("machine bed {w} × {h} mm is not a positive, finite size")));
    }
    if m.s_max == 0 {
        return Err(refuse("machine S maximum is 0"));
    }
    Ok(())
}

fn mode_name(mode: SectionMode) -> &'static str {
    match mode {
        SectionMode::Line => "line",
        SectionMode::Fill => "fill",
        SectionMode::Image => "image",
    }
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// Where the closing `G0 X0 Y0` goes: the controller's origin, in machine coordinates.
const MACHINE_ORIGIN: [f64; 2] = [0.0, 0.0];

/// OM `_number`: format!("{:.3}") then trim zeros and dot; "-0" → "0". T2 number_format (100,019 rows).
/// Non-finite input gives Python's own spelling (`nan`, `inf`, `-inf`), never a valid coordinate such as `0`; the
/// shipping encoder never gets there, because it formats through [`try_number`], which refuses non-finite values.
pub fn number(v: f64) -> String {
    if v.is_nan() {
        return "nan".to_owned();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf" } else { "-inf" }.to_owned();
    }
    let mut text = format!("{v:.3}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" || text.is_empty() { "0".to_owned() } else { text }
}

/// [`number`] for the shipping encoder: NaN and infinity are an error.
pub fn try_number(v: f64) -> Result<String, Error> {
    if !v.is_finite() {
        return Err(refuse(format!("a G-code number is not finite ({v})")));
    }
    Ok(number(v))
}

/// Text placed after `; ` in a comment: printable ASCII except GRBL's real-time characters (`!`, `~`, `?`), every
/// other character replaced by `_`, at most [`MAX_COMMENT_CHARS`] characters. A CR, LF or control character can
/// therefore never start a new G-code line, and no byte can trigger a GRBL real-time command.
pub fn comment_text(text: &str) -> String {
    text.chars().take(MAX_COMMENT_CHARS).map(|c| if matches!(c, ' '..='}') && !matches!(c, '!' | '?') { c } else { '_' }).collect()
}

fn header_lines(machine: &str, generated: &str) -> Vec<String> {
    vec![
        "; OpenLaser G-code".to_owned(),
        format!("; Machine: {}", comment_text(machine)),
        format!("; Generated: {}", comment_text(generated)),
        "; Design origin: center; units: millimetres".to_owned(),
        "G21".to_owned(),
        "G90".to_owned(),
        "M5".to_owned(),
    ]
}

/// OM `_xy`: centre-origin bed mm → the controller's coordinates; outside the bed (1e-6 mm tolerance) refuses.
fn machine_xy(point: [f64; 2], m: &GantryMachine) -> Result<[f64; 2], Error> {
    let [x, y] = point;
    if !x.is_finite() || !y.is_finite() {
        return Err(refuse("geometry contains a non-finite coordinate"));
    }
    let [mx, my] = match m.origin {
        openlaser_machine::WorkspaceOrigin::Center => [x, y],
        openlaser_machine::WorkspaceOrigin::BottomLeft => [x + m.bed_mm[0] / 2.0, y + m.bed_mm[1] / 2.0],
        openlaser_machine::WorkspaceOrigin::TopLeft => [x + m.bed_mm[0] / 2.0, m.bed_mm[1] / 2.0 - y],
    };
    let [min_x, max_x, min_y, max_y] = match m.origin {
        openlaser_machine::WorkspaceOrigin::Center => [-m.bed_mm[0] / 2.0, m.bed_mm[0] / 2.0, -m.bed_mm[1] / 2.0, m.bed_mm[1] / 2.0],
        _ => [0.0, m.bed_mm[0], 0.0, m.bed_mm[1]],
    };
    if mx < min_x - 1e-6 || mx > max_x + 1e-6 || my < min_y - 1e-6 || my > max_y + 1e-6 {
        return Err(refuse(format!("geometry point ({x:.3}, {y:.3}) mm is outside the {} × {} mm bed", m.bed_mm[0], m.bed_mm[1])));
    }
    Ok([mx, my])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_normalizes_negative_zero_and_trims() {
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(-0.0004), "0");
        assert_eq!(number(20.125), "20.125");
        assert_eq!(number(20.0), "20");
        assert_eq!(number(0.0625), "0.062");
    }

    #[test]
    fn number_of_non_finite_is_never_a_coordinate() {
        assert_eq!(number(f64::NAN), "nan");
        assert_eq!(number(f64::INFINITY), "inf");
        assert_eq!(number(f64::NEG_INFINITY), "-inf");
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(try_number(v).is_err());
        }
        assert_eq!(try_number(1.5).as_deref(), Ok("1.5"));
    }

    #[test]
    fn comment_text_is_printable_ascii_without_realtime_characters() {
        let hostile = "a\nb\rc\u{18}d!e~f?g\u{2028}h\u{85}iĚj×k\u{7f}l";
        let clean = comment_text(hostile);
        assert_eq!(clean, "a_b_c_d_e_f_g_h_i_j_k_l");
        assert!(clean.bytes().all(|b| (0x20..0x7f).contains(&b)));
        assert_eq!(comment_text(&"x".repeat(500)).len(), MAX_COMMENT_CHARS);
    }

    #[test]
    fn frame_check_rejects_anything_but_rapids() {
        assert!(check_laser_off("; c\nG21\nG90\nM5\nG0 X1 Y-2.5\n").is_ok());
        for bad in ["M3 S1000", "M4", "G1 X1 Y1 F600", "G0 X1 Y1 S10", "G0 X1", "S1000", "M8", "G0 Xnan Y1"] {
            assert!(check_laser_off(bad).is_err(), "{bad}");
        }
    }
}
