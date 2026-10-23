//! Built-in test patterns (SPEC §3.21 `--builtin`, §8.3). `dot`, `line` and `box10` are exactly the T1 `dot_test`,
//! `line_test` and `mark_box10` inputs (`tests/golden/t1_lmc/v1/*.json`), so a hardware trace diffs against the
//! rust-v1 candidate; `square50` is HIL-2's 50 mm square; `f-shape` the asymmetric H4 "F" (axis orientation); `corners`
//! the H27 corner marks at ±extent (default 100 mm, the rated field edge).
//!
//! A pattern becomes a project through ordinary engine commands (`file.new`, `tools.drawLines`,
//! `layer.cutSettings.set`), the same ids an agent or the GUI would use; nothing here builds geometry by hand for the
//! planner.

use openlaser_types::MachineId;
use serde_json::{Value, json};

/// The corner marks' default distance from the centre: the JPT M7 lens's rated ±100 mm field.
pub const CORNERS_DEFAULT_EXTENT_MM: f64 = 100.0;
/// Length of each arm of a corner mark.
const CORNER_ARM_MM: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Dot,
    Line,
    Box10,
    Square50,
    FShape,
    Corners,
}

/// One pattern: open polylines in design mm (field centre at the origin, Y up) on layer C00, Line mode, one pass.
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub polylines: Vec<Vec<[f64; 2]>>,
    pub power_pct: f64,
    pub frequency_khz: f64,
    pub pulse_ns: f64,
    pub speed_mm_s: f64,
}

impl Builtin {
    pub const NAMES: &'static str = "dot, line, box10, square50, f-shape, corners";

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "dot" => Builtin::Dot,
            "line" => Builtin::Line,
            "box10" => Builtin::Box10,
            "square50" => Builtin::Square50,
            "f-shape" => Builtin::FShape,
            "corners" => Builtin::Corners,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Builtin::Dot => "dot",
            Builtin::Line => "line",
            Builtin::Box10 => "box10",
            Builtin::Square50 => "square50",
            Builtin::FShape => "f-shape",
            Builtin::Corners => "corners",
        }
    }

    /// The T1 scenario whose `inputs` this pattern reproduces exactly.
    pub fn t1_scenario(self) -> Option<&'static str> {
        match self {
            Builtin::Dot => Some("dot_test"),
            Builtin::Line => Some("line_test"),
            Builtin::Box10 => Some("mark_box10"),
            _ => None,
        }
    }

    /// The pattern. `extent` applies to `corners` only (the parser refuses it elsewhere).
    pub fn pattern(self, extent: Option<f64>) -> Pattern {
        // T1 parameters (OM's Test Dot / Test Line at 1 %; mark_box10 at 50 %), all at 30 kHz, 200 ns, 1000 mm/s.
        let t1 = |polylines: Vec<Vec<[f64; 2]>>, power_pct: f64| Pattern { polylines, power_pct, frequency_khz: 30.0, pulse_ns: 200.0, speed_mm_s: 1000.0 };
        // The bring-up patterns that have no Python twin start at the first-emission cap (S23), 10 %.
        let bring_up = |polylines: Vec<Vec<[f64; 2]>>| Pattern { polylines, power_pct: 10.0, frequency_khz: 30.0, pulse_ns: 200.0, speed_mm_s: 1000.0 };
        match self {
            Builtin::Dot => t1(vec![vec![[0.0, 0.0], [0.05, 0.0]]], 1.0),
            Builtin::Line => t1(vec![vec![[-2.5, 0.0], [2.5, 0.0]]], 1.0),
            Builtin::Box10 => t1(vec![square(5.0)], 50.0),
            Builtin::Square50 => bring_up(vec![square(25.0)]),
            // Stem on the left, top bar to the right, shorter middle bar: mirrored or rotated, it no longer reads "F".
            Builtin::FShape => bring_up(vec![vec![[-10.0, -15.0], [-10.0, 15.0], [10.0, 15.0]], vec![[-10.0, 0.0], [5.0, 0.0]]]),
            Builtin::Corners => {
                let e = extent.unwrap_or(CORNERS_DEFAULT_EXTENT_MM);
                let a = CORNER_ARM_MM.min(e);
                let corner = |sx: f64, sy: f64| vec![[sx * (e - a), sy * e], [sx * e, sy * e], [sx * e, sy * (e - a)]];
                bring_up(vec![corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)])
            }
        }
    }
}

/// A closed square of half-side `h` as T1 writes it: five points, the last equal to the first.
fn square(h: f64) -> Vec<[f64; 2]> {
    vec![[-h, -h], [h, -h], [h, h], [-h, h], [-h, -h]]
}

impl Pattern {
    /// The engine commands that build this pattern as a new project on `machine` (S2 ids and camelCase params).
    /// `power_override` is `run --power-pct`.
    pub fn commands(&self, name: &str, machine: &MachineId, power_override: Option<f64>) -> Vec<(&'static str, Value)> {
        let mut out = vec![("file.new", json!({ "name": name, "machine": machine.0 }))];
        for poly in &self.polylines {
            out.push(("tools.drawLines", json!({ "pointsMm": poly, "closed": false, "layer": 0 })));
        }
        out.push((
            "layer.cutSettings.set",
            json!({
                "layer": 0,
                "mode": "line",
                "output": true,
                "show": true,
                "passes": 1,
                "galvo": {
                    "speedMmS": self.speed_mm_s,
                    "powerPct": power_override.unwrap_or(self.power_pct),
                    "frequencyKhz": self.frequency_khz,
                    "qPulseNs": self.pulse_ns,
                },
            }),
        ));
        out
    }

    /// Largest |coordinate| of any point.
    pub fn reach_mm(&self) -> f64 {
        self.polylines.iter().flatten().flat_map(|p| [p[0].abs(), p[1].abs()]).fold(0.0, f64::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_round_trips() {
        for b in [Builtin::Dot, Builtin::Line, Builtin::Box10, Builtin::Square50, Builtin::FShape, Builtin::Corners] {
            assert_eq!(Builtin::parse(b.name()), Some(b));
            assert!(Builtin::NAMES.contains(b.name()));
        }
        assert_eq!(Builtin::parse("triangle"), None);
    }

    #[test]
    fn corners_follow_the_extent() {
        let p = Builtin::Corners.pattern(Some(110.0));
        assert_eq!(p.polylines.len(), 4);
        assert_eq!(p.reach_mm(), 110.0);
        assert_eq!(Builtin::Corners.pattern(None).reach_mm(), CORNERS_DEFAULT_EXTENT_MM);
        assert!(p.polylines.iter().all(|l| l.len() == 3));
    }

    #[test]
    fn f_shape_is_asymmetric_in_both_axes() {
        let p = Builtin::FShape.pattern(None);
        let mirrored_x: Vec<Vec<[f64; 2]>> = p.polylines.iter().map(|l| l.iter().map(|q| [-q[0], q[1]]).collect()).collect();
        let mirrored_y: Vec<Vec<[f64; 2]>> = p.polylines.iter().map(|l| l.iter().map(|q| [q[0], -q[1]]).collect()).collect();
        assert_ne!(mirrored_x, p.polylines);
        assert_ne!(mirrored_y, p.polylines);
    }

    #[test]
    fn commands_carry_the_pattern_and_the_override() {
        let p = Builtin::Dot.pattern(None);
        let cmds = p.commands("dot", &MachineId("jpt-m7-200-sim".into()), Some(0.5));
        let ids: Vec<&str> = cmds.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, ["file.new", "tools.drawLines", "layer.cutSettings.set"]);
        assert_eq!(cmds[1].1["pointsMm"], json!([[0.0, 0.0], [0.05, 0.0]]));
        assert_eq!(cmds[2].1["galvo"]["powerPct"], json!(0.5));
        assert_eq!(cmds[2].1["galvo"]["frequencyKhz"], json!(30.0));
    }
}
