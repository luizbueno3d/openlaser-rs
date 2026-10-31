#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
#[path = "common/image.rs"]
mod fixture;
use fixture::*;
use openlaser_doc::{Geom, ImageMode};
use openlaser_geom::{Affine, Polyline};
use openlaser_machine::{Kinematics, jpt_m7_200_sim};
use openlaser_plan::{CutQ, JobPlan};
use openlaser_types::{Deg, Mm, ObjectId, Percent, SectionMode};
use std::sync::Arc;

fn machine_paths(m: &openlaser_machine::MachineProfile, design: &[[[f64; 2]; 2]]) -> Vec<Polyline> {
    let Kinematics::Galvo(g) = &m.kind else { panic!("galvo") };
    design.iter().map(|p| Polyline(p.to_vec()).transformed(&openlaser_plan::machine_transform(g))).collect()
}
fn paths(p: &JobPlan) -> &[Polyline] {
    &p.sections[0].paths
}

#[test]
fn image_threshold_runs_and_full_rectangle_frame() {
    let mut m = jpt_m7_200_sim();
    if let Kinematics::Galvo(g) = &mut m.kind {
        g.cal.negate = [true, false];
        g.cal.offset_mm = [3.0, 1.0];
    }
    let p = project(&m);
    let a = plan(&p, &m);
    assert!(a.facts.unsupported.is_empty());
    assert_eq!(a.sections.len(), 1);
    assert_eq!(a.sections[0].mode, SectionMode::Image);
    assert_eq!(a.sections[0].objects, vec![ObjectId(1)]);
    assert_eq!(paths(&a), machine_paths(&m, &[[[-2.0, 0.5], [0.0, 0.5]], [[1.0, 0.5], [2.0, 0.5]], [[1.0, -0.5], [2.0, -0.5]]]));
    let Kinematics::Galvo(g) = &m.kind else { panic!("galvo") };
    assert_eq!(
        a.frame,
        vec![Polyline(vec![[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0], [-2.0, -1.0]]).transformed(&openlaser_plan::machine_transform(g))]
    );
}

#[test]
fn image_bidirectional_reverses_order_and_direction_negative_and_rotation() {
    let m = jpt_m7_200_sim();
    let mut p = project(&m);
    bitmap(&mut p).gray = Arc::from([0, 0, 255, 0, 0, 255, 0, 0]);
    layer(&mut p).image.bidirectional = true;
    assert_eq!(
        paths(&plan(&p, &m)),
        machine_paths(&m, &[[[-2.0, 0.5], [0.0, 0.5]], [[1.0, 0.5], [2.0, 0.5]], [[2.0, -0.5], [0.0, -0.5]], [[-1.0, -0.5], [-2.0, -0.5]]])
    );
    let mut p = project(&m);
    layer(&mut p).image.negative = true;
    assert_eq!(paths(&plan(&p, &m)), machine_paths(&m, &[[[0.0, 0.5], [1.0, 0.5]], [[-2.0, -0.5], [1.0, -0.5]]]));
    let mut p = project(&m);
    let xf = Affine::translate((10.0, 20.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2);
    Arc::make_mut(&mut p.shapes[0]).xform = xf;
    let a = plan(&p, &m);
    for segment in paths(&a) {
        assert!((segment.0[0][0] - segment.0[1][0]).abs() < 1e-12);
    }
    let baseline = plan(&project(&m), &m);
    assert_eq!(paths(&a), baseline.sections[0].paths.iter().map(|p| p.transformed(&xf)).collect::<Vec<_>>());
}

#[test]
fn image_box_filter_averages_area_and_clips_right_edge() {
    let m = jpt_m7_200_sim();
    let mut p = project(&m);
    bitmap(&mut p).size_mm = [-1.5, -1.0];
    bitmap(&mut p).px = [3, 1];
    bitmap(&mut p).gray = Arc::from([0, 255, 0]);
    // First cell averages black and white to 128, so it does not burn; final half cell is black.
    assert_eq!(paths(&plan(&p, &m)), machine_paths(&m, &[[[0.25, 0.0], [0.75, 0.0]]]));
    // A source-pixel boundary cuts through the cell: average = (0*0.75 + 255*0.25) / 1 = 63.75.
    bitmap(&mut p).px = [2, 1];
    bitmap(&mut p).gray = Arc::from([0, 255]);
    assert_eq!(paths(&plan(&p, &m)), machine_paths(&m, &[[[-0.75, 0.0], [0.25, 0.0]]]));
}

#[test]
fn image_refusals_are_facts_and_output_off_is_unplanned() {
    let m = jpt_m7_200_sim();
    let mut cases = Vec::new();
    let mut p = project(&m);
    layer(&mut p).image.angle = Deg::new(90.0).unwrap();
    cases.push((p, "Scan Angle"));
    for interval in [0.0, -1.0, 0.009, 2.001] {
        let mut p = project(&m);
        layer(&mut p).image.interval = Mm::new(interval).unwrap();
        cases.push((p, "Interval"));
    }
    let mut p = project(&m);
    bitmap(&mut p).size_mm = [100.0, 100.0];
    layer(&mut p).image.interval = Mm::new(0.01).unwrap();
    cases.push((p, "100000000 cells"));
    let mut p = project(&m);
    Arc::make_mut(&mut p.shapes[0]).power_scale = Percent::new(50.0).unwrap();
    cases.push((p, "Power Scale"));
    let mut p = project(&m);
    bitmap(&mut p).gray = Arc::from([0]);
    cases.push((p, "source dimensions"));
    for (mut p, message) in cases {
        let a = plan(&p, &m);
        assert!(a.facts.unsupported.iter().any(|(_, why)| why.contains(message)), "{message}: {:?}", a.facts);
        layer(&mut p).output = false;
        let a = plan(&p, &m);
        assert!(a.sections.is_empty() && a.frame.is_empty() && a.facts.unsupported.is_empty());
    }
}

#[test]
fn image_empty_passes_order_budget_and_deterministic_fingerprint() {
    let m = jpt_m7_200_sim();
    let mut p = project(&m);
    let a = plan(&p, &m);
    assert_eq!(a, plan(&p, &m));
    bitmap(&mut p).gray = Arc::from([255; 8]);
    let empty = plan(&p, &m);
    assert!(empty.sections.is_empty());
    assert_eq!(empty.facts.empty_fill_layers, vec![openlaser_types::LayerKey::Color(0)]);
    let mut p = project(&m);
    bitmap(&mut p).gray = Arc::from([0; 8]);
    assert_ne!(a.fingerprint, plan(&p, &m).fingerprint);
    let mut p = project(&m);
    layer(&mut p).image.interval = Mm::new(0.5).unwrap();
    assert_ne!(a.fingerprint, plan(&p, &m).fingerprint);
    let mut p = project(&m);
    bitmap(&mut p).gray = Arc::from([100; 8]);
    let threshold = plan(&p, &m);
    layer(&mut p).image.mode = ImageMode::Ordered;
    assert_ne!(threshold.fingerprint, plan(&p, &m).fingerprint);
    let mut p = project(&m);
    let mut vector = (*p.shapes[0]).clone();
    vector.id = ObjectId(2);
    vector.geom = Geom::Rect { w: Mm::new(1.0).unwrap(), h: Mm::new(1.0).unwrap(), corner_radius: Mm::new(0.0).unwrap() };
    p.shapes.push(Arc::new(vector));
    layer(&mut p).image.passes = 2;
    layer(&mut p).global_passes = 2;
    let a = plan(&p, &m);
    assert_eq!(
        a.sections.iter().map(|s| (s.mode, s.pass, s.of)).collect::<Vec<_>>(),
        vec![
            (SectionMode::Line, 1, 2),
            (SectionMode::Image, 1, 4),
            (SectionMode::Image, 2, 4),
            (SectionMode::Line, 2, 2),
            (SectionMode::Image, 3, 4),
            (SectionMode::Image, 4, 4)
        ]
    );
    layer(&mut p).image.passes = 501;
    assert!(plan(&p, &m).facts.unsupported.iter().any(|(_, s)| s.contains("1002 passes")));
    layer(&mut p).image.passes = 0;
    assert!(
        openlaser_plan::plan(
            &p,
            &m,
            &openlaser_plan::PlanOptions {
                selected: None,
                repeats: 1,
                frame_style: openlaser_types::FrameStyle::Contour,
                frame_source: openlaser_types::FrameSource::Job,
                flatten_tol_mm: 0.01
            }
        )
        .is_err()
    );
}

#[test]
fn image_gantry_uses_image_cut_settings() {
    let m = openlaser_machine::om_gantries().remove(1);
    let mut p = project(&m);
    layer(&mut p).image.gantry.power_max = Percent::new(42.0).unwrap();
    let a = plan(&p, &m);
    assert_eq!(a.sections[0].mode, SectionMode::Image);
    let CutQ::Gantry(q) = &a.sections[0].cut else { panic!("gantry") };
    assert_eq!(q.s_value, 420);
    assert_eq!(q.speed, m.layer_defaults.gantry.speed);
}

#[test]
fn image_interval_limits_and_partial_final_row() {
    let m = jpt_m7_200_sim();
    for interval in [0.01, 2.0] {
        let mut p = project(&m);
        layer(&mut p).image.interval = Mm::new(interval).unwrap();
        assert!(plan(&p, &m).facts.unsupported.is_empty());
    }
    let mut p = project(&m);
    // A short final row (1.1 mm at 1 mm: the second row is 0.1 mm high) is scanned at the centre of the part inside
    // the rectangle (y = -0.5), never below it, and never refused.
    bitmap(&mut p).size_mm = [4.0, 1.1];
    let a = plan(&p, &m);
    assert!(a.facts.unsupported.is_empty(), "{:?}", a.facts.unsupported);
    let last = paths(&a).last().unwrap();
    let want = &machine_paths(&m, &[[[1.0, -0.5], [2.0, -0.5]]])[0];
    for (got, want) in last.0.iter().zip(&want.0) {
        assert!((got[0] - want[0]).abs() < 1e-9 && (got[1] - want[1]).abs() < 1e-9, "{last:?} vs {want:?}");
    }
    bitmap(&mut p).gray = Arc::from([255; 8]);
    let white = plan(&p, &m);
    assert!(white.facts.unsupported.is_empty());
    assert_eq!(white.facts.empty_fill_layers, vec![openlaser_types::LayerKey::Color(0)]);
    // An undersized (subnormal) dimension must refuse before chunks_exact(0), never panic.
    bitmap(&mut p).size_mm = [f64::from_bits(1), 2.0];
    layer(&mut p).image.interval = Mm::new(2.0).unwrap();
    assert!(!plan(&p, &m).facts.unsupported.is_empty());
}

#[test]
fn rotary_image_angle_is_a_notice_and_scans_without_rotating_artwork() {
    use openlaser_machine::{RotaryAxis, RotaryCfg};
    let mut m = jpt_m7_200_sim();
    let Kinematics::Galvo(g) = &mut m.kind else { panic!("galvo") };
    g.rotary = Some(RotaryCfg { enabled: true, axis: RotaryAxis::X, split_mm: 1.0, ..Default::default() });
    for angle in [90.0, 45.0, 180.0, 270.0] {
        let mut p = project(&m);
        layer(&mut p).image.angle = Deg::new(angle).unwrap();
        let out = plan(&p, &m);
        assert!(out.facts.unsupported.is_empty(), "{angle}: {:?}", out.facts.unsupported);
        assert_eq!(out.facts.rotary_notices.len(), if angle == 90.0 || angle == 270.0 { 0 } else { 1 });
        assert!(!out.sections.is_empty());
        let radians = angle.to_radians();
        for path in out.sections.iter().flat_map(|s| &s.paths) {
            let [a, b] = path.0.as_slice() else { panic!("segment") };
            let cross = (b[0] - a[0]) * radians.sin() - (b[1] - a[1]) * radians.cos();
            assert!(cross.abs() < 1e-10, "{angle}: {path:?}");
            for point in &path.0 {
                // Inside the 1 × 2 mm image, up to float noise: sin/cos round differently per platform (Windows gave
                // 1.0000000000000002), and safety allows the same kind of noise at the field edge.
                assert!(point[0].abs() <= 0.5 + 1e-12 && point[1].abs() <= 1.0 + 1e-12, "{angle}: {point:?}");
            }
        }
    }
}
