//! T3 (SPEC §6.2): `hatch_om` against the 46 OM `raster.hatch_polylines` cases in `tests/golden/t3_hatch/`.
//! Segment count and order exact; coordinates within 1e-9 mm; refusal cases refuse.
//!
//! An integration test because `openlaser-testkit` depends on `openlaser-types` (SPEC §2.3 rule 10).

use openlaser_geom::{Error, HatchParams, Polyline, hatch, hatch_om};
use openlaser_testkit::t3::{self, Expected};

const TOL: f64 = 1e-9;

fn polys(case: &t3::Case) -> Vec<Polyline> {
    case.polylines.iter().map(|p| Polyline(p.clone())).collect()
}

#[test]
fn t3_hatch() {
    let cases = t3::load().unwrap();
    assert_eq!(cases.len(), 46, "T3 has 46 cases");
    let mut checked_segments = 0usize;
    for case in &cases {
        let p = &case.params;
        let got = hatch_om(&polys(case), p.angle_deg, p.pitch_mm, p.passes, p.angle_step_deg, p.closure_tol_mm);
        match &case.expected {
            Expected::Error { kind, message } => {
                assert_eq!(kind, "ValueError", "{}", case.name);
                assert!(message.contains("pitch"), "{}", case.name);
                assert!(matches!(got, Err(Error::BadInterval(_))), "{}: expected a refusal, got {got:?}", case.name);
            }
            Expected::Output(want) => {
                let got = got.unwrap_or_else(|e| panic!("{}: {e}", case.name));
                assert_eq!(want.segment_count, want.segments.len(), "{}: fixture is self-consistent", case.name);
                assert_eq!(got.segments.len(), want.segments.len(), "{}: segment count", case.name);
                assert_eq!(got.closed_input_count, want.closed_input_count, "{}: closed inputs", case.name);
                assert_eq!(got.skipped_open_count, want.skipped_open_count, "{}: skipped open inputs", case.name);
                assert_eq!(got.angles_deg, want.angles_deg, "{}: angles", case.name);
                for (i, (g, w)) in got.segments.iter().zip(&want.segments).enumerate() {
                    for k in 0..2 {
                        for c in 0..2 {
                            let d = (g[k][c] - w[k][c]).abs();
                            assert!(d <= TOL, "{}: segment {i} point {k} coord {c}: {} vs {} (|d| = {d:e})", case.name, g[k][c], w[k][c]);
                        }
                    }
                }
                checked_segments += got.segments.len();
            }
        }
    }
    assert!(checked_segments > 8000, "checked {checked_segments} segments");
}

#[test]
fn t3_business_card_counts() {
    let cases = t3::load().unwrap();
    let get = |name: &str| cases.iter().find(|c| c.name == name).unwrap();

    let saved = get("business_card_raster_saved_settings");
    let p = &saved.params;
    let r = hatch_om(&polys(saved), p.angle_deg, p.pitch_mm, p.passes, p.angle_step_deg, p.closure_tol_mm).unwrap();
    assert_eq!((r.segments.len(), r.closed_input_count, r.skipped_open_count), (969, 32, 67));

    let cross = get("business_card_raster_crosshatch");
    let p = &cross.params;
    let r = hatch_om(&polys(cross), p.angle_deg, p.pitch_mm, p.passes, p.angle_step_deg, p.closure_tol_mm).unwrap();
    assert_eq!(r.segments.len(), 1637);

    // LightBurn Fill with the same settings, bidirectional, reports the 67 open inputs by index.
    let fill = HatchParams {
        interval_mm: 0.1,
        angle_deg: 0.0,
        passes: 1,
        angle_increment_deg: 0.0,
        bidirectional: true,
        cross_hatch: true,
        closure_tol_mm: 0.05,
        max_lines: 1_000_000,
    };
    let f = hatch(&polys(cross), &fill).unwrap();
    assert_eq!(f.passes.concat(), r.segments, "cross-hatch = OM's 0°/90° two-pass hatch");
    assert_eq!((f.closed_used, f.open_inputs.len()), (32, 67));
}
