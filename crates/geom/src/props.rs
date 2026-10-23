//! Property tests (SPEC §6.5; proptest, 256 cases per gate run).

use kurbo::{BezPath, CubicBez, ParamCurve, ParamCurveNearest, Point, QuadBez};
use proptest::prelude::*;

use crate::{HatchParams, P2, Polyline, flatten, hatch, hatch_om, inside_even_odd};

/// Distance from `p` to the polyline.
fn dist_to_polyline(poly: &Polyline, p: P2) -> f64 {
    let mut best = f64::INFINITY;
    for w in poly.0.windows(2) {
        let (a, b) = (w[0], w[1]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let len2 = d[0] * d[0] + d[1] * d[1];
        let t = if len2 > 0.0 { (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len2).clamp(0.0, 1.0) } else { 0.0 };
        let q = [a[0] + t * d[0], a[1] + t * d[1]];
        best = best.min((p[0] - q[0]).hypot(p[1] - q[1]));
    }
    best
}

fn pt() -> impl Strategy<Value = Point> {
    (-100.0f64..100.0, -100.0f64..100.0).prop_map(|(x, y)| Point::new(x, y))
}

/// A star polygon (alternating radii), closed, as in T3's random stars.
fn star(cx: f64, cy: f64, n: usize, r_out: f64, r_in: f64, rot: f64) -> Polyline {
    let mut pts: Vec<P2> = (0..2 * n)
        .map(|i| {
            let r = if i % 2 == 0 { r_out } else { r_in };
            let a = rot + std::f64::consts::PI * i as f64 / n as f64;
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect();
    pts.push(pts[0]);
    Polyline(pts)
}

fn stars() -> impl Strategy<Value = Vec<Polyline>> {
    (-50.0f64..50.0, -50.0f64..50.0, 3usize..12, 5.0f64..30.0, 0.2f64..0.9, 0.0f64..6.3, any::<bool>()).prop_map(|(cx, cy, n, r, ratio, rot, hole)| {
        let mut v = vec![star(cx, cy, n, r, r * ratio, rot)];
        if hole {
            // A hole well inside the inner radius, wound the other way.
            let mut h = star(cx, cy, n, r * ratio * 0.5, r * ratio * 0.3, rot + 0.1);
            h.0.reverse();
            v.push(h);
        }
        v
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Every curve point is within the tolerance of the polyline, every vertex is within it of the curve, and the
    /// ends are exact.
    #[test]
    fn prop_flatten_within_tolerance(p0 in pt(), p1 in pt(), p2 in pt(), p3 in pt(), quad in any::<bool>(), tol_exp in -3.0f64..0.0) {
        let tol = 10f64.powf(tol_exp);
        let mut path = BezPath::new();
        path.move_to(p0);
        if quad { path.quad_to(p1, p3); } else { path.curve_to(p1, p2, p3); }
        let out = flatten(&path, tol).unwrap();
        prop_assert_eq!(out.len(), 1);
        let poly = &out[0];
        prop_assert_eq!(poly.0.first().copied(), Some([p0.x, p0.y]));
        prop_assert_eq!(poly.0.last().copied(), Some([p3.x, p3.y]));
        let eval = |t: f64| if quad { QuadBez::new(p0, p1, p3).eval(t) } else { CubicBez::new(p0, p1, p2, p3).eval(t) };
        for i in 0..=1000 {
            let q = eval(i as f64 / 1000.0);
            let d = dist_to_polyline(poly, [q.x, q.y]);
            prop_assert!(d <= tol * 1.0001 + 1e-9, "curve point {i} is {d} from the polyline (tol {tol})");
        }
        for v in &poly.0 {
            let p = Point::new(v[0], v[1]);
            let d = if quad { QuadBez::new(p0, p1, p3).nearest(p, 1e-12).distance_sq } else { CubicBez::new(p0, p1, p2, p3).nearest(p, 1e-12).distance_sq }.sqrt();
            prop_assert!(d <= tol * 1.0001 + 1e-9, "vertex {v:?} is {d} off the curve (tol {tol})");
        }
    }

    /// Every hatch segment's interior is inside the even-odd region of the closed inputs (bidirectional, one-way
    /// and cross-hatch alike).
    #[test]
    fn prop_hatch_inside_even_odd(polys in stars(), angle in -180.0f64..180.0, pitch in 0.2f64..2.0, bidirectional in any::<bool>(), cross_hatch in any::<bool>()) {
        let p = HatchParams { interval_mm: pitch, angle_deg: angle, passes: 2, angle_increment_deg: 37.0, bidirectional, cross_hatch, closure_tol_mm: 0.05, max_lines: 100_000 };
        let r = hatch(&polys, &p).unwrap();
        prop_assert_eq!(r.closed_used, polys.len());
        prop_assert!(!r.passes.concat().is_empty());
        for seg in r.passes.concat() {
            let [a, b] = seg;
            if (b[0] - a[0]).hypot(b[1] - a[1]) < 1e-3 {
                continue;
            }
            for f in [0.25, 0.5, 0.75] {
                let m = [a[0] + f * (b[0] - a[0]), a[1] + f * (b[1] - a[1])];
                prop_assert!(inside_even_odd(&polys, m), "{m:?} on {seg:?} is outside");
            }
        }
    }

    /// LightBurn Fill, bidirectional without cross-hatch, is OM's hatch exactly.
    #[test]
    fn prop_hatch_bidirectional_equals_om(polys in stars(), angle in -180.0f64..180.0, pitch in 0.1f64..2.0, passes in 1u16..4, step in -180.0f64..180.0) {
        let p = HatchParams { interval_mm: pitch, angle_deg: angle, passes, angle_increment_deg: step, bidirectional: true, cross_hatch: false, closure_tol_mm: 0.05, max_lines: 1_000_000 };
        let r = hatch(&polys, &p).unwrap();
        let om = hatch_om(&polys, angle, pitch, u32::from(passes), step, 0.05).unwrap();
        prop_assert_eq!(r.passes.concat(), om.segments);
        prop_assert_eq!(r.angles_deg, om.angles_deg);
        prop_assert_eq!(r.closed_used, om.closed_input_count);
        prop_assert_eq!(r.open_inputs.len(), om.skipped_open_count);
    }
}
