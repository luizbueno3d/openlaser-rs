//! Design mm → machine mm (SPEC §3.0, §3.7).
//!
//! Galvo: field angle, then axis swap, then per-axis negate, then offset; never scale (the per-axis scale lives in
//! the encoding field F, `GalvoMachine::field_f`). Design (0, 0) is the field centre (S13).
//!
//! Gantry: section and frame paths are **centre-origin bed millimetres, Y up**, whatever the controller's origin:
//! the project's design origin (its `Workspace.origin`) is placed on the bed and the bed's centre is (0, 0). That is
//! the convention `openlaser_gcode` takes (`gcode/src/lib.rs`, as OM's `gcode._xy`): the G-code encoder maps them to
//! the controller's origin and refuses a point off the bed. `safety` checks them against [`gantry_bed`] (pinned at I1:
//! WP3 had planned controller coordinates, so the encoder moved every job by half the bed again and refused it).

use openlaser_doc::WorkspaceOrigin;
use openlaser_geom::{Affine, Bounds};
use openlaser_machine::{GalvoMachine, GantryMachine};

/// Design mm → machine mm for a galvo: field angle, swap, negate, offset. Never scale (scale lives in F).
///
/// Identity parts are left out, so the default calibration maps every coordinate to itself bit for bit.
pub fn machine_transform(m: &GalvoMachine) -> Affine {
    let c = &m.cal;
    let mut a = Affine::IDENTITY;
    if c.angle_deg != 0.0 {
        a = Affine::rotate(c.angle_deg.to_radians()) * a;
    }
    if c.swap_xy {
        // x' = y, y' = x ("Galvo 1 is X axis" off).
        a = Affine::new([0.0, 1.0, 1.0, 0.0, 0.0, 0.0]) * a;
    }
    if c.negate[0] || c.negate[1] {
        let sx = if c.negate[0] { -1.0 } else { 1.0 };
        let sy = if c.negate[1] { -1.0 } else { 1.0 };
        a = Affine::scale_non_uniform(sx, sy) * a;
    }
    if c.offset_mm != [0.0, 0.0] {
        a = Affine::translate((c.offset_mm[0], c.offset_mm[1])) * a;
    }
    a
}

/// Design mm (origin `design`, Y up) → centre-origin bed mm, Y up (the gantry plan convention, see the module doc).
/// The controller's origin plays no part: `openlaser_gcode` maps these to it.
pub fn gantry_transform(design: WorkspaceOrigin, m: &GantryMachine) -> Affine {
    let [w, h] = m.bed_mm;
    // Where design (0, 0) sits on the bed, relative to the bed's centre (Y up).
    let (dx, dy) = match design {
        WorkspaceOrigin::Center => return Affine::IDENTITY,
        WorkspaceOrigin::BottomLeft => (-w / 2.0, -h / 2.0),
        WorkspaceOrigin::TopLeft => (-w / 2.0, h / 2.0),
    };
    Affine::new([1.0, 0.0, 0.0, 1.0, dx, dy])
}

/// The bed in plan coordinates: `[-w/2, w/2] × [-h/2, h/2]` (centre origin, Y up), whatever the controller's origin.
pub fn gantry_bed(m: &GantryMachine) -> Bounds {
    let [w, h] = m.bed_mm;
    Bounds { min: [-w / 2.0, -h / 2.0], max: [w / 2.0, h / 2.0] }
}

/// The controller's origin (where `G0 X0 Y0` goes, and a job's first travel starts) in plan coordinates.
pub fn gantry_home(m: &GantryMachine) -> [f64; 2] {
    let [w, h] = m.bed_mm;
    match m.origin {
        WorkspaceOrigin::Center => [0.0, 0.0],
        WorkspaceOrigin::BottomLeft => [-w / 2.0, -h / 2.0],
        WorkspaceOrigin::TopLeft => [-w / 2.0, h / 2.0],
    }
}

#[cfg(test)]
mod tests {
    use openlaser_geom::Point;
    use openlaser_machine::{GantryLink, GantryTool, jpt_m7_200_sim};
    use openlaser_types::MmPerS;

    use super::*;

    fn galvo() -> GalvoMachine {
        let openlaser_machine::Kinematics::Galvo(g) = jpt_m7_200_sim().kind else { panic!("the built-in JPT M7 profile is a galvo") };
        g
    }

    fn apply(a: Affine, p: [f64; 2]) -> [f64; 2] {
        let q = a * Point::new(p[0], p[1]);
        [q.x, q.y]
    }

    #[test]
    fn default_calibration_is_identity_bit_for_bit() {
        let a = machine_transform(&galvo());
        assert_eq!(a, Affine::IDENTITY);
        for p in [[0.1, -0.2], [99.999_999_9, -100.0], [-0.0, 3.3]] {
            assert_eq!(apply(a, p).map(f64::to_bits), p.map(|v| (v + 0.0).to_bits()));
        }
    }

    #[test]
    fn order_is_angle_swap_negate_offset() {
        let mut g = galvo();
        g.cal.angle_deg = 90.0;
        g.cal.swap_xy = true;
        g.cal.negate = [true, false];
        g.cal.offset_mm = [1.0, 2.0];
        // (1, 0) → rotate 90° → (0, 1) → swap → (1, 0) → negate x → (−1, 0) → offset → (0, 2).
        let p = apply(machine_transform(&g), [1.0, 0.0]);
        assert!((p[0] - 0.0).abs() < 1e-12 && (p[1] - 2.0).abs() < 1e-12, "{p:?}");
        // Never scale: the calibration scale stays in F.
        g.cal = galvo().cal;
        g.cal.scale = [1.5, 0.7];
        assert_eq!(machine_transform(&g), Affine::IDENTITY);
    }

    #[test]
    fn swap_and_negate_each_axis() {
        let mut g = galvo();
        g.cal.swap_xy = true;
        assert_eq!(apply(machine_transform(&g), [3.0, -4.0]), [-4.0, 3.0]);
        g.cal.swap_xy = false;
        g.cal.negate = [false, true];
        assert_eq!(apply(machine_transform(&g), [3.0, -4.0]), [3.0, 4.0]);
    }

    fn gantry(origin: WorkspaceOrigin) -> GantryMachine {
        GantryMachine {
            bed_mm: [600.0, 400.0],
            origin,
            max_speed: MmPerS::new(500.0).unwrap(),
            travel_speed: MmPerS::new(200.0).unwrap(),
            s_max: 1000,
            tool: GantryTool::Laser { dynamic: true },
            air_assist: true,
            frame_before_mark: true,
            home_on_connect: false,
            link: GantryLink::ExportOnly,
        }
    }

    /// OM `gcode._xy` as `openlaser_gcode` applies it: centre-origin bed mm → the controller's coordinates.
    fn om_xy(p: [f64; 2], m: &GantryMachine) -> [f64; 2] {
        let [w, h] = m.bed_mm;
        match m.origin {
            WorkspaceOrigin::Center => p,
            WorkspaceOrigin::BottomLeft => [p[0] + w / 2.0, p[1] + h / 2.0],
            WorkspaceOrigin::TopLeft => [p[0] + w / 2.0, h / 2.0 - p[1]],
        }
    }

    #[test]
    fn gantry_plans_centre_origin_bed_mm_whatever_the_controller() {
        use WorkspaceOrigin::*;
        let p = [12.5, -7.25];
        for machine in [Center, BottomLeft, TopLeft] {
            let g = gantry(machine);
            // A centre design is already centre-origin bed mm: bit for bit.
            assert_eq!(gantry_transform(Center, &g), Affine::IDENTITY);
            assert_eq!(apply(gantry_transform(BottomLeft, &g), [10.0, 30.0]), [-290.0, -170.0], "{machine:?}");
            assert_eq!(apply(gantry_transform(TopLeft, &g), [10.0, -30.0]), [-290.0, 170.0], "{machine:?}");
            assert_eq!(gantry_bed(&g), Bounds { min: [-300.0, -200.0], max: [300.0, 200.0] }, "{machine:?}");
            // The encoder then lands every design where OM put it (gcode._xy for a centre design) ...
            let q = om_xy(apply(gantry_transform(Center, &g), p), &g);
            let want = match machine {
                Center => p,
                BottomLeft => [12.5 + 300.0, -7.25 + 200.0],
                TopLeft => [12.5 + 300.0, 200.0 - -7.25],
            };
            assert_eq!(q, want, "{machine:?}");
            // ... and the controller's origin is where G0 X0 Y0 goes.
            assert_eq!(om_xy(gantry_home(&g), &g), [0.0, 0.0], "{machine:?}");
        }
        // A design whose origin is the controller's lands on the same controller coordinates (design = G-code).
        for origin in [BottomLeft, TopLeft] {
            let g = gantry(origin);
            // 30 mm up from the bottom-left corner, or 30 mm down from the top-left one (design Y is up).
            let d = if origin == TopLeft { [10.0, -30.0] } else { [10.0, 30.0] };
            assert_eq!(om_xy(apply(gantry_transform(origin, &g), d), &g), [10.0, 30.0], "{origin:?}");
        }
        // A bottom-left design on a top-left controller flips Y within the bed (OM `_xy`).
        assert_eq!(om_xy(apply(gantry_transform(BottomLeft, &gantry(TopLeft)), [10.0, 30.0]), &gantry(TopLeft)), [10.0, 370.0]);
    }
}
