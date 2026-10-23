//! GRBL G-code export (SPEC §3.11).
#![forbid(unsafe_code)]
#![deny(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_possible_wrap)]

use std::time::Duration;

use openlaser_plan::CutQ;

pub mod om;

pub use openlaser_machine::GantryMachine;
pub use openlaser_safety::{EmitPlan, FramePlan};
pub use openlaser_types::{Fingerprint, MachineId};

pub type Result<T, E = Error> = std::result::Result<T, E>;

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

/// Shipping encoder (B11: only the gantry block is read). Overspeed never reaches here: safety refuses it.
pub fn compile(emit: &EmitPlan, m: &GantryMachine, h: &Header) -> Result<GcodeProgram, Error> {
    let mut lines = header_lines(&h.machine_name, &h.generated);
    let mut distance = 0.0;
    for section in emit.sections() {
        let CutQ::Gantry(cut) = &section.cut else {
            return Err(Error::Refused("galvo cut settings cannot be exported as gantry G-code".to_owned()));
        };
        if cut.speed.get() > m.max_speed.get() {
            return Err(Error::Refused(format!("section speed {} mm/s exceeds machine maximum {} mm/s", cut.speed.get(), m.max_speed.get())));
        }
        if cut.s_value > m.s_max {
            return Err(Error::Refused(format!("section power {} exceeds machine S maximum {}", cut.s_value, m.s_max)));
        }
        if !cut.speed.get().is_finite() || cut.speed.get() <= 0.0 {
            return Err(Error::Refused("cut speed must be finite and positive".to_owned()));
        }
        lines.push(format!("; Layer: {} · {:?}", section.layer_name, section.mode));
        if cut.air_assist && m.air_assist {
            lines.push("M8".to_owned());
        }
        for path in &section.paths {
            if path.0.len() < 2 {
                continue;
            }
            let points = path.0.iter().map(|&p| machine_xy(p, m)).collect::<Result<Vec<_>, _>>()?;
            let Some(first) = points.first().copied() else { continue };
            lines.push("M5".to_owned());
            lines.push(format!("G0 X{} Y{}", number(first[0]), number(first[1])));
            if matches!(&m.tool, openlaser_machine::GantryTool::Pen) {
                lines.push("M3".to_owned());
            } else {
                lines.push(format!("{} S{}", if cut.dynamic { "M4" } else { "M3" }, cut.s_value));
            }
            for pair in points.windows(2) {
                let Some((a, b)) = pair.first().zip(pair.get(1)) else { continue };
                lines.push(format!("G1 X{} Y{} F{}", number(b[0]), number(b[1]), number(cut.speed.get() * 60.0)));
                distance += (b[0] - a[0]).hypot(b[1] - a[1]);
            }
            lines.push("M5".to_owned());
        }
        if cut.air_assist && m.air_assist {
            lines.push("M9".to_owned());
        }
    }
    if !distance.is_finite() || distance <= 0.0 {
        return Err(Error::Refused("no output-enabled geometry for G-code export".to_owned()));
    }
    lines.extend(["M5".to_owned(), "M9".to_owned(), "G0 X0 Y0".to_owned(), "; End OpenLaser job".to_owned(), String::new()]);
    let text = lines.join("\n");
    let line_count = u32::try_from(lines.len().saturating_sub(1)).map_err(|_| Error::Refused("G-code has too many lines".to_owned()))?;
    let speed = m.travel_speed.get();
    let seconds = distance / speed;
    let estimate = Duration::try_from_secs_f64(seconds).map_err(|e| Error::Refused(e.to_string()))?;
    Ok(GcodeProgram { text, lines: line_count, fingerprint: *emit.fingerprint(), machine: emit.machine().clone(), distance_mm: distance, estimate })
}

/// Laser-off frame around the job (G0 only, no M3/M4/S>0; a test greps the text).
pub fn frame_program(frame: &FramePlan, m: &GantryMachine, h: &Header) -> Result<GcodeFrame, Error> {
    let mut lines = header_lines(&h.machine_name, &h.generated);
    for path in frame.paths() {
        if path.0.len() < 2 {
            continue;
        }
        let points = path.0.iter().map(|&p| machine_xy(p, m)).collect::<Result<Vec<_>, _>>()?;
        let Some(first) = points.first().copied() else { continue };
        lines.push(format!("G0 X{} Y{}", number(first[0]), number(first[1])));
        for point in points.iter().skip(1) {
            lines.push(format!("G0 X{} Y{}", number(point[0]), number(point[1])));
        }
    }
    lines.extend(["M5".to_owned(), "G0 X0 Y0".to_owned(), "; End OpenLaser frame".to_owned(), String::new()]);
    let text = lines.join("\n");
    let line_count = u32::try_from(lines.len().saturating_sub(1)).map_err(|_| Error::Refused("G-code has too many lines".to_owned()))?;
    let program = GcodeProgram {
        text,
        lines: line_count,
        fingerprint: *frame.fingerprint(),
        machine: frame.machine().clone(),
        distance_mm: 0.0,
        estimate: Duration::ZERO,
    };
    Ok(GcodeFrame { program })
}

/// OM `_number`: format!("{:.3}") then trim zeros and dot; "-0" → "0". T2 number_format (100,019 rows).
pub fn number(v: f64) -> String {
    if !v.is_finite() {
        return "0".to_owned();
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

fn header_lines(machine: &str, generated: &str) -> Vec<String> {
    vec![
        "; OpenLaser G-code".to_owned(),
        format!("; Machine: {machine}"),
        format!("; Generated: {generated}"),
        "; Design origin: center; units: millimetres".to_owned(),
        "G21".to_owned(),
        "G90".to_owned(),
        "M5".to_owned(),
    ]
}

fn machine_xy(point: [f64; 2], m: &GantryMachine) -> Result<[f64; 2], Error> {
    let [x, y] = point;
    if !x.is_finite() || !y.is_finite() {
        return Err(Error::Refused("geometry contains a non-finite coordinate".to_owned()));
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
        return Err(Error::Refused(format!("geometry point ({x}, {y}) lies outside the gantry bed")));
    }
    Ok([mx, my])
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_plan::{CutQ, GantryCutQ, Section};
    use openlaser_types::{DeviceKind, FrameSource, FrameStyle, LayerKey, MmPerS, Percent, SectionMode};

    #[test]
    fn t2_number_format_matches_all_oracle_rows() {
        let table = openlaser_testkit::t2::load_numbers().expect("T2 number fixture");
        for (input, expected) in table.rows {
            assert_eq!(number(input), expected, "input {input:?}");
        }
    }

    #[test]
    fn number_normalizes_negative_zero_and_trims() {
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(20.125), "20.125");
        assert_eq!(number(20.0), "20");
    }

    fn gantry() -> GantryMachine {
        GantryMachine {
            bed_mm: [300.0, 200.0],
            origin: openlaser_machine::WorkspaceOrigin::Center,
            max_speed: MmPerS::new(1000.0).expect("speed"),
            travel_speed: MmPerS::new(2000.0).expect("travel speed"),
            s_max: 1000,
            tool: openlaser_machine::GantryTool::Laser { dynamic: true },
            air_assist: true,
            frame_before_mark: true,
            home_on_connect: false,
            link: openlaser_machine::GantryLink::ExportOnly,
        }
    }

    #[test]
    fn gcode_compile_never_caps_overspeed() {
        let cut = GantryCutQ {
            speed: MmPerS::new(1200.0).expect("requested speed"),
            power: Percent::new(50.0).expect("power"),
            s_value: 500,
            air_assist: false,
            dynamic: true,
        };
        let section = Section {
            layer: LayerKey::Color(0),
            layer_name: "diode".to_owned(),
            sub: 0,
            pass: 1,
            of: 1,
            mode: SectionMode::Line,
            cut: CutQ::Gantry(cut),
            paths: vec![openlaser_geom::Polyline(vec![[0.0, 0.0], [1.0, 0.0]])],
            objects: Vec::new(),
            estimate: Duration::ZERO,
        };
        let emit = openlaser_safety::testing::emit_plan_unchecked(
            vec![section],
            openlaser_types::Fingerprint([1; 32]),
            MachineId("diode".to_owned()),
            DeviceKind::Grbl,
            1,
        );
        assert!(compile(&emit, &gantry(), &Header { machine_name: "Diode".to_owned(), generated: "<MASKED>".to_owned() }).is_err());
    }

    #[test]
    #[ignore = "needs I1 built-in gantry profiles and WP3 planning defaults"]
    fn gcode_diode_job_uses_gantry_defaults() {
        let profile = openlaser_machine::om_gantries().into_iter().find(|profile| profile.id.0 == "diode-grbl-400x400").expect("built-in diode profile");
        let openlaser_machine::Kinematics::Gantry(gantry) = profile.kind else { panic!("diode profile must be a gantry") };
        assert!(gantry.max_speed.get() > 0.0);
        assert!(profile.layer_defaults.gantry.speed.get() <= gantry.max_speed.get());
    }

    #[test]
    fn gcode_frame_program_has_no_laser_on() {
        let frame = openlaser_safety::testing::frame_plan_unchecked(
            vec![openlaser_geom::Polyline(vec![[-1.0, -1.0], [1.0, 1.0]])],
            openlaser_types::Fingerprint([2; 32]),
            MachineId("diode".to_owned()),
            FrameStyle::Bounds,
            FrameSource::Job,
        );
        let program = frame_program(&frame, &gantry(), &Header { machine_name: "Diode".to_owned(), generated: "<MASKED>".to_owned() }).expect("frame program");
        assert!(!program.program().text.contains("M3"));
        assert!(!program.program().text.contains("M4"));
        assert!(!program.program().text.contains(" S1"));
    }
}
