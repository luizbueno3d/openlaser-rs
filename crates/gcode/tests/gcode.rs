//! Shipping G-code encoder (SPEC §3.11, §6.6, §7.3) and the regression tests of the WP2 review.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use openlaser_gcode::{GantryMachine, Header, compile, frame_program};
use openlaser_geom::Polyline;
use openlaser_machine::{GantryLink, GantryTool, WorkspaceOrigin};
use openlaser_plan::{CutQ, GalvoCutQ, GantryCutQ, ResolvedTimings, Section};
use openlaser_safety::testing::{emit_plan_unchecked, frame_plan_unchecked};
use openlaser_safety::{EmitPlan, FramePlan};
use openlaser_types::galvo::PulseDecision;
use openlaser_types::{DeviceKind, Fingerprint, FrameSource, FrameStyle, Hz, LayerKey, MachineId, MmPerS, Percent, SectionMode};

/// A T2 machine row as a shipping `GantryMachine`.
fn gantry(machine_id: &str) -> GantryMachine {
    let index = openlaser_testkit::t2::load().unwrap();
    let row = index.machines.iter().find(|m| m.machine_id == machine_id).unwrap();
    GantryMachine {
        bed_mm: [row.width_mm, row.height_mm],
        origin: match row.origin.as_str() {
            "center" => WorkspaceOrigin::Center,
            "bottom_left" => WorkspaceOrigin::BottomLeft,
            "top_left" => WorkspaceOrigin::TopLeft,
            other => panic!("origin {other}"),
        },
        max_speed: MmPerS::new(row.max_speed_mm_s).unwrap(),
        travel_speed: MmPerS::new(row.travel_speed_mm_s).unwrap(),
        s_max: row.power_scale,
        tool: if row.is_plotter { GantryTool::Pen } else { GantryTool::Laser { dynamic: row.laser_command == "M4" } },
        air_assist: row.supports_air_assist,
        frame_before_mark: true,
        home_on_connect: false,
        link: GantryLink::ExportOnly,
    }
}

fn diode() -> GantryMachine {
    gantry("diode-grbl-400x400")
}

fn header(name: &str) -> Header {
    Header { machine_name: name.to_owned(), generated: "<MASKED>".to_owned() }
}

fn gantry_cut(speed: f64, s_value: u32) -> CutQ {
    let power = Percent::new((f64::from(s_value) / 10.0).min(100.0)).unwrap();
    CutQ::Gantry(GantryCutQ { speed: MmPerS::new(speed).unwrap(), power, s_value, air_assist: false, dynamic: true })
}

/// OM's MOPA defaults as a galvo cut block (1000 mm/s, 100 %): what B11 leaked into G-code.
fn mopa_cut() -> CutQ {
    let timing = (0, 0x8000);
    CutQ::Galvo(GalvoCutQ {
        requested_speed: MmPerS::new(1000.0).unwrap(),
        speed_units: 4225,
        actual_speed_mm_s: 1000.0,
        power: Percent::new(100.0).unwrap(),
        mark_current: 4095,
        frequency: Hz::new(30_000).unwrap(),
        q_period: 33,
        pulse: PulseDecision::Exact(200),
        timings: ResolvedTimings {
            laser_on: timing,
            laser_off: timing,
            polygon: timing,
            jump_delay_short: timing,
            jump_delay_long: timing,
            jump_speed_units: 0,
            mo_settle_10us: 0,
            first_jump_settle_10us: 0,
        },
    })
}

fn section(name: &str, cut: CutQ, paths: Vec<Polyline>) -> Section {
    Section {
        layer: LayerKey::Color(0),
        layer_name: name.to_owned(),
        sub: 0,
        pass: 1,
        of: 1,
        mode: SectionMode::Line,
        cut,
        paths,
        objects: Vec::new(),
        estimate: Duration::ZERO,
    }
}

fn square() -> Polyline {
    Polyline(vec![[-10.0, -10.0], [10.0, -10.0], [10.0, 10.0], [-10.0, 10.0], [-10.0, -10.0]])
}

fn plan(sections: Vec<Section>, repeats: u16) -> EmitPlan {
    emit_plan_unchecked(sections, Fingerprint([1; 32]), MachineId("diode-grbl-400x400".to_owned()), DeviceKind::Grbl, repeats)
}

fn frame(paths: Vec<Polyline>) -> FramePlan {
    frame_plan_unchecked(paths, Fingerprint([2; 32]), MachineId("diode-grbl-400x400".to_owned()), FrameStyle::Bounds, FrameSource::Job)
}

/// Non-comment lines of a program.
fn code_lines(text: &str) -> Vec<&str> {
    text.lines().filter(|l| !l.starts_with(';')).collect()
}

/// GRBL acts on these bytes anywhere in the stream (comments included): soft reset, feed hold, cycle start,
/// status, and the extended-ASCII real-time overrides (feed, rapid, spindle/laser power, coolant).
fn grbl_realtime(b: u8) -> bool {
    matches!(b, 0x18 | b'!' | b'~' | b'?' | 0x84 | 0x85 | 0x87 | 0x90..=0x9e | 0xa0 | 0xa1)
}

const HOSTILE: &str = "Diode\nM3 S1000\nG1 X10 Y10 F600\rM4 S1000\u{2028}G1 X20\u{85}~!?ĚĚĚ\u{18}";

// ---- B11 ---------------------------------------------------------------------------------------------------------

/// SPEC §6.6 B11: MOPA (galvo) values never reach G-code; F and S come from the gantry block only.
#[test]
fn gcode_diode_job_uses_gantry_defaults() {
    let m = diode();
    // A section that carries galvo (MOPA) settings refuses the job — alone or after a valid gantry section.
    for sections in [
        vec![section("mopa", mopa_cut(), vec![square()])],
        vec![section("ok", gantry_cut(20.0, 250), vec![square()]), section("mopa", mopa_cut(), vec![square()])],
    ] {
        let err = compile(&plan(sections, 1), &m, &header("Diode")).expect_err("galvo settings must be refused");
        assert!(err.to_string().contains("galvo") && err.to_string().contains("B11"), "{err}");
    }
    // The gantry block's speed (20 mm/s → F1200) and S (250) are the only F and S words.
    let program = compile(&plan(vec![section("diode", gantry_cut(20.0, 250), vec![square()])], 1), &m, &header("Diode")).unwrap();
    let code = code_lines(&program.text);
    let g1: Vec<_> = code.iter().filter(|l| l.starts_with("G1 ")).collect();
    assert_eq!(g1.len(), 4);
    assert!(g1.iter().all(|l| l.ends_with(" F1200")), "{g1:?}");
    assert_eq!(code.iter().filter(|l| l.starts_with("M4 ")).copied().collect::<Vec<_>>(), vec!["M4 S250"]);
    assert!(!program.text.contains("F60000") && !program.text.contains("S1000"), "MOPA values leaked:\n{}", program.text);
    // OM's centre-origin square on a bottom-left 400 × 400 bed.
    assert!(code.contains(&"G0 X190 Y190") && code.contains(&"G1 X210 Y190 F1200"), "{}", program.text);
}

/// SPEC §7.3: defence in depth, a section above the machine's max speed (or S above s_max) is an error, never capped.
#[test]
fn gcode_compile_never_caps() {
    let m = diode();
    for (speed, s) in [(250.0001, 100), (1000.0, 100), (100.0, 1001)] {
        let result = compile(&plan(vec![section("fast", gantry_cut(speed, s), vec![square()])], 1), &m, &header("Diode"));
        assert!(result.is_err(), "{speed} mm/s, S{s}");
    }
    assert!(compile(&plan(vec![section("edge", gantry_cut(250.0, 1000), vec![square()])], 1), &m, &header("Diode")).is_ok());
}

/// T2 (SPEC §6.2): "shipping refuses the capped cases": OM caps `speed_power_edges` ('fast', 1000 mm/s) and the
/// business card's fiber speeds (1000/300 mm/s) to each machine's maximum; the shipping encoder refuses them.
#[test]
fn t2_shipping_refuses_capped_cases() {
    let index = openlaser_testkit::t2::load().unwrap();
    for row in index.machines.iter().filter(|m| m.output_format == "gcode") {
        let m = gantry(&row.machine_id);
        for speed in [1000.0, 300.0].into_iter().filter(|s| *s > row.max_speed_mm_s) {
            let result = compile(&plan(vec![section("fast", gantry_cut(speed, 122), vec![square()])], 1), &m, &header(&row.name));
            assert!(result.is_err(), "{}: {speed} mm/s must be refused, not capped", row.machine_id);
        }
    }
}

// ---- G-code line injection (review item 2) -----------------------------------------------------------------------

/// A hostile machine name, timestamp or layer name cannot start a G-code line or carry a GRBL real-time byte:
/// the only burn lines are the plan's own.
#[test]
fn gcode_hostile_names_cannot_inject_lines() {
    let m = diode();
    let h = Header { machine_name: HOSTILE.to_owned(), generated: HOSTILE.to_owned() };
    let program = compile(&plan(vec![section(HOSTILE, gantry_cut(20.0, 250), vec![square()])], 1), &m, &h).unwrap();
    for line in code_lines(&program.text) {
        let allowed = matches!(line, "G21" | "G90" | "M5" | "M9" | "G0 X0 Y0" | "M4 S250")
            || line.starts_with("G0 X")
            || (line.starts_with("G1 X") && line.ends_with(" F1200"));
        assert!(allowed, "injected line {line:?} in:\n{}", program.text);
        assert!(!line.contains("S1000") && !line.contains("F600"), "{line:?}");
    }
    assert_eq!(program.text.lines().count(), usize::try_from(program.lines).unwrap());
    assert!(program.text.bytes().all(|b| !grbl_realtime(b) && (b == b'\n' || b >= 0x20)), "real-time or control byte in:\n{:?}", program.text);
}

/// SPEC §7.3: the frame program never turns the laser on (no M3/M4, no S, no G1), even with a hostile machine name.
#[test]
fn gcode_frame_program_has_no_laser_on() {
    let m = diode();
    for name in ["Diode", HOSTILE] {
        let h = Header { machine_name: name.to_owned(), generated: name.to_owned() };
        let framed = frame_program(&frame(vec![square(), Polyline(vec![[0.0, 0.0]])]), &m, &h).unwrap();
        let text = &framed.program().text;
        for line in code_lines(text) {
            assert!(matches!(line, "G21" | "G90" | "M5") || line.starts_with("G0 X"), "frame line {line:?}");
            assert!(!line.contains('S') && !line.contains('F'), "frame line {line:?}");
        }
        assert!(!text.contains("M3") && !text.contains("M4") && !text.contains("\nG1"), "{text}");
        assert!(text.bytes().all(|b| !grbl_realtime(b) && (b == b'\n' || b >= 0x20)), "{text:?}");
        assert_eq!(code_lines(text).iter().filter(|l| l.starts_with("G0 ")).count(), 5 + 1 + 1, "every frame vertex, the dot, the return");
    }
}

// ---- no silent drops, estimate, repeats --------------------------------------------------------------------------

#[test]
fn gcode_refuses_paths_it_cannot_burn() {
    let m = diode();
    for paths in [vec![Polyline(vec![[0.0, 0.0]])], vec![Polyline(Vec::new())], vec![square(), Polyline(vec![[1.0, 1.0]])]] {
        assert!(compile(&plan(vec![section("short", gantry_cut(20.0, 250), paths)], 1), &m, &header("Diode")).is_err());
    }
    assert!(compile(&plan(Vec::new(), 1), &m, &header("Diode")).is_err(), "nothing to export");
    assert!(compile(&plan(vec![section("ok", gantry_cut(20.0, 250), vec![square()])], 0), &m, &header("Diode")).is_err(), "0 repeats");
    assert!(compile(&plan(vec![section("far", gantry_cut(20.0, 250), vec![Polyline(vec![[0.0, 0.0], [201.0, 0.0]])])], 1), &m, &header("Diode")).is_err());
    assert!(compile(&plan(vec![section("nan", gantry_cut(20.0, 250), vec![Polyline(vec![[0.0, 0.0], [f64::NAN, 0.0]])])], 1), &m, &header("Diode")).is_err());
    assert!(frame_program(&frame(vec![Polyline(Vec::new())]), &m, &header("Diode")).is_err());
    let galvo_plan =
        emit_plan_unchecked(vec![section("ok", gantry_cut(20.0, 250), vec![square()])], Fingerprint([1; 32]), MachineId("m".into()), DeviceKind::GalvoLmc, 1);
    assert!(compile(&galvo_plan, &m, &header("Diode")).is_err(), "a galvo plan is not G-code");
}

/// The estimate uses each section's burn speed for burn distance and the travel speed for G0 moves.
#[test]
fn gcode_estimate_uses_burn_speed() {
    let m = diode(); // bottom-left origin, 250 mm/s travel
    let line = Polyline(vec![[0.0, 0.0], [10.0, 0.0]]); // machine (200, 200) → (210, 200)
    let program = compile(&plan(vec![section("slow", gantry_cut(5.0, 100), vec![line])], 1), &m, &header("Diode")).unwrap();
    assert!((program.distance_mm - 10.0).abs() < 1e-9);
    let travel = 200.0f64.hypot(200.0) + 210.0f64.hypot(200.0);
    let expected = 10.0 / 5.0 + travel / 250.0;
    assert!((program.estimate.as_secs_f64() - expected).abs() < 1e-6, "{:?} vs {expected}", program.estimate);
    let framed = frame_program(&frame(vec![square()]), &m, &header("Diode")).unwrap();
    assert!(framed.program().estimate > Duration::ZERO);
}

#[test]
fn gcode_repeats_the_job() {
    let m = diode();
    let once = compile(&plan(vec![section("r", gantry_cut(20.0, 250), vec![square()])], 1), &m, &header("Diode")).unwrap();
    let thrice = compile(&plan(vec![section("r", gantry_cut(20.0, 250), vec![square()])], 3), &m, &header("Diode")).unwrap();
    let burns = |text: &str| text.lines().filter(|l| l.starts_with("M4 ")).count();
    assert_eq!((burns(&once.text), burns(&thrice.text)), (1, 3));
    assert!(thrice.text.contains("; Repeat 3/3"));
    assert!((thrice.distance_mm - 3.0 * once.distance_mm).abs() < 1e-9);
    assert_ne!(once.bytes_hash(), thrice.bytes_hash());
}
