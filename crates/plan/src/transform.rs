//! Design mm → machine mm (SPEC §3.0, §3.7).
//!
//! Galvo: field angle, then axis swap, then per-axis negate, then offset; never scale (the per-axis scale lives in
//! the encoding field F, `GalvoMachine::field_f`). Design (0, 0) is the field centre (S13).
//!
//! Gantry: the project's design origin (its `Workspace.origin`) is moved to the machine's origin on its bed, the
//! way OM `gcode._xy` does it for a centre-origin design. Section paths of a gantry plan are therefore machine
//! (G-code) coordinates, and `safety` checks them against [`gantry_bed`].

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

/// Design mm (origin `design`, Y up) → the gantry's machine coordinates. A top-left machine counts Y downwards
/// (OM `gcode._xy`: `h/2 − y` for a centre-origin design).
pub fn gantry_transform(design: WorkspaceOrigin, m: &GantryMachine) -> Affine {
    if design == m.origin {
        return Affine::IDENTITY;
    }
    let [w, h] = m.bed_mm;
    // Where design (0, 0) sits on the bed, in bed coordinates (bottom-left origin, Y up).
    let (dx, dy) = match design {
        WorkspaceOrigin::Center => (w / 2.0, h / 2.0),
        WorkspaceOrigin::BottomLeft => (0.0, 0.0),
        WorkspaceOrigin::TopLeft => (0.0, h),
    };
    match m.origin {
        WorkspaceOrigin::BottomLeft => Affine::new([1.0, 0.0, 0.0, 1.0, dx, dy]),
        WorkspaceOrigin::Center => Affine::new([1.0, 0.0, 0.0, 1.0, dx - w / 2.0, dy - h / 2.0]),
        WorkspaceOrigin::TopLeft => Affine::new([1.0, 0.0, 0.0, -1.0, dx, h - dy]),
    }
}

/// The bed in machine coordinates: `[0, w] × [0, h]`, or centred on (0, 0) for a centre-origin machine.
pub fn gantry_bed(m: &GantryMachine) -> Bounds {
    let [w, h] = m.bed_mm;
    match m.origin {
        WorkspaceOrigin::Center => Bounds { min: [-w / 2.0, -h / 2.0], max: [w / 2.0, h / 2.0] },
        WorkspaceOrigin::BottomLeft | WorkspaceOrigin::TopLeft => Bounds { min: [0.0, 0.0], max: [w, h] },
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

    #[test]
    fn gantry_matches_om_xy_for_a_centre_design() {
        use WorkspaceOrigin::*;
        let p = [12.5, -7.25];
        // OM gcode._xy: centre → (x, y); bottom_left → (x + w/2, y + h/2); top_left → (x + w/2, h/2 − y).
        assert_eq!(apply(gantry_transform(Center, &gantry(Center)), p), p);
        assert_eq!(apply(gantry_transform(Center, &gantry(BottomLeft)), p), [12.5 + 300.0, -7.25 + 200.0]);
        assert_eq!(apply(gantry_transform(Center, &gantry(TopLeft)), p), [12.5 + 300.0, 200.0 - -7.25]);
        // A bottom-left design on a top-left machine flips Y within the bed.
        assert_eq!(apply(gantry_transform(BottomLeft, &gantry(TopLeft)), [10.0, 30.0]), [10.0, 370.0]);
        assert_eq!(apply(gantry_transform(TopLeft, &gantry(BottomLeft)), [10.0, -30.0]), [10.0, 370.0]);
        assert_eq!(apply(gantry_transform(BottomLeft, &gantry(Center)), [0.0, 0.0]), [-300.0, -200.0]);
    }

    #[test]
    fn gantry_bed_rect_follows_origin() {
        assert_eq!(gantry_bed(&gantry(WorkspaceOrigin::Center)), Bounds { min: [-300.0, -200.0], max: [300.0, 200.0] });
        assert_eq!(gantry_bed(&gantry(WorkspaceOrigin::TopLeft)), Bounds { min: [0.0, 0.0], max: [600.0, 400.0] });
    }
}
