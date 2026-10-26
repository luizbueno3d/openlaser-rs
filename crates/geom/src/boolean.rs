//! Boolean overlay, weld (union-all), offset and stroke-outline operations on polylines, over `i_overlay`.
//!
//! Conventions: operand polylines are closed (first == last point within 1e-9); before any combination each
//! operand is normalised by itself with EvenOdd (an "O" whose two contours share a winding keeps its hole), then
//! operands combine under NonZero. Output is closed rings (outer + holes) with the first point repeated, ready
//! for `Geom::Path`. Everything is deterministic single-threaded (integer math by default); non-finite input is
//! refused before i_overlay sees it and the result is checked finite.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::overlay::FloatOverlay;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::stroke::offset::StrokeOffset;
use i_overlay::mesh::float::style::{LineCap, LineJoin, OutlineStyle, StrokeStyle};
use i_overlay::mesh::math::MathMode;

use crate::{Error, MAX_FLATTEN_POINTS, Polyline};

type Path = Vec<[f64; 2]>;
type Shape = Vec<Path>;
type Shapes = Vec<Shape>;

/// Boolean operations on two operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooleanOp {
    Union,
    Difference,
    Intersection,
    Xor,
}

impl BooleanOp {
    fn rule(self) -> OverlayRule {
        match self {
            BooleanOp::Union => OverlayRule::Union,
            BooleanOp::Difference => OverlayRule::Difference,
            BooleanOp::Intersection => OverlayRule::Intersect,
            BooleanOp::Xor => OverlayRule::Xor,
        }
    }
}

/// Line-join style for offsets and strokes. `Miter` carries the limit (max miter-length / half-width ratio).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Join {
    Round,
    Miter { limit: f64 },
    Bevel,
}

/// Line-cap style for stroke outlines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    Butt,
    Round,
    Square,
}

/// Every input point is finite and the count stays under the shared point budget.
fn check_input(polys: &[Polyline], open: bool) -> Result<Vec<Path>, Error> {
    let mut out = Vec::with_capacity(polys.len());
    let mut total = 0usize;
    for pl in polys {
        total = total.saturating_add(pl.0.len());
        if total > MAX_FLATTEN_POINTS {
            return Err(Error::TooManyPoints(total));
        }
        let mut path: Path = pl.0.clone();
        for p in &path {
            if !p[0].is_finite() || !p[1].is_finite() {
                return Err(Error::NonFinite);
            }
        }
        if !open {
            if path.len() < 3 {
                return Err(Error::TooFewPoints);
            }
            // i_overlay auto-closes; drop our repeated closing point.
            if let (Some(first), Some(last)) = (path.first(), path.last())
                && (first[0] - last[0]).abs() <= 1e-9
                && (first[1] - last[1]).abs() <= 1e-9
            {
                path.pop();
            }
        }
        out.push(path);
    }
    Ok(out)
}

/// Every output point is finite, the point budget holds, and each ring repeats its first point.
fn from_shapes(shapes: Shapes) -> Result<Vec<Polyline>, Error> {
    let mut out = Vec::new();
    let mut total = 0usize;
    for shape in shapes {
        for ring in shape {
            total = total.saturating_add(ring.len());
            if total > MAX_FLATTEN_POINTS {
                return Err(Error::TooManyPoints(total));
            }
            if ring.len() < 3 {
                continue;
            }
            let mut path: Path = ring;
            for p in &path {
                if !p[0].is_finite() || !p[1].is_finite() {
                    return Err(Error::NonFinite);
                }
            }
            path.push(path.first().copied().unwrap_or([0.0, 0.0]));
            out.push(Polyline(path));
        }
    }
    Ok(out)
}

/// Runs one i_overlay call so that a panic inside the third-party library becomes an error instead of taking the
/// app down (AGENTS §2), the same guard the SVG importer puts around its parsers.
fn guarded(f: impl FnOnce() -> Shapes) -> Result<Shapes, Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|_| Error::KernelFailed)
}

/// EvenOdd-normalise one operand shape on its own.
fn normalise(shape: Shape) -> Result<Shapes, Error> {
    guarded(|| shape.simplify_shape(FillRule::EvenOdd))
}

/// `boolean(subject, clip, op)` on closed operands; output is closed rings (each shape's contours, holes included).
pub fn boolean(subject: &[Polyline], clip: &[Polyline], op: BooleanOp) -> Result<Vec<Polyline>, Error> {
    let subj: Vec<Shape> = normalise(check_input(subject, false)?)?;
    let clip: Vec<Shape> = normalise(check_input(clip, false)?)?;
    let shapes = guarded(|| FloatOverlay::with_subj_and_clip(&subj, &clip).overlay(op.rule(), FillRule::NonZero))?;
    from_shapes(shapes)
}

/// Union every shape together (weld): each normalised EvenOdd, then all unioned at once.
pub fn union_all(shapes: &[Vec<Polyline>]) -> Result<Vec<Polyline>, Error> {
    let mut all: Vec<Shape> = Vec::with_capacity(shapes.len());
    for s in shapes {
        let paths = check_input(s, false)?;
        all.extend(normalise(paths)?);
    }
    let shapes = guarded(|| FloatOverlay::with_subj_and_clip(&all, &[] as &[Shape]).overlay(OverlayRule::Union, FillRule::NonZero))?;
    from_shapes(shapes)
}

fn join_style(join: Join, tol: f64, distance: f64) -> LineJoin<f64> {
    match join {
        Join::Bevel => LineJoin::Bevel,
        Join::Miter { limit } => {
            // Miter length = half-width / sin(θ/2); the limit caps the ratio → the minimum turn angle.
            let angle = (2.0 * (1.0 / limit.max(1.0)).asin()).clamp(0.01 * std::f64::consts::PI, 0.99 * std::f64::consts::PI);
            LineJoin::Miter(angle)
        }
        // Round's parameter is L/R: chord length over radius. Sagitta ≈ L²/(8R) < tol → L/R = sqrt(8·tol/R).
        Join::Round => LineJoin::Round((8.0 * tol / distance.abs().max(1e-9)).sqrt().clamp(0.01, std::f64::consts::PI / 2.0)),
    }
}

/// Offset a closed shape: `distance_mm` > 0 grows, < 0 shrinks (EvenOdd-normalised first). An empty result is
/// returned as `Ok(vec![])` — the caller decides whether that is an error (inward shrinks may legitimately vanish).
pub fn offset(shape: &[Polyline], distance_mm: f64, join: Join, tol_mm: f64) -> Result<Vec<Polyline>, Error> {
    if !distance_mm.is_finite() {
        return Err(Error::NonFinite);
    }
    let paths = check_input(shape, false)?;
    let all: Vec<Shape> = normalise(paths)?;
    // outer_offset grows the outer rings outward, inner_offset grows holes inward (both positive distances).
    let style = OutlineStyle::new(distance_mm.abs()).line_join(join_style(join, tol_mm, distance_mm));
    let style = if distance_mm >= 0.0 { style.inner_offset(-distance_mm) } else { style.outer_offset(-distance_mm.abs()).inner_offset(distance_mm.abs()) };
    let shapes = guarded(|| all.outline(&style))?;
    from_shapes(shapes)
}

/// The outline of stroking open polylines at `half_width`, with `join` at the corners and `cap` at the ends.
pub fn stroke_outline(open: &[Polyline], half_width: f64, join: Join, cap: Cap, tol_mm: f64) -> Result<Vec<Polyline>, Error> {
    if !half_width.is_finite() || half_width <= 0.0 {
        return Err(Error::NonFinite);
    }
    let paths = check_input(open, true)?;
    let angle = (8.0 * tol_mm / half_width.max(1e-9)).sqrt().clamp(0.01, std::f64::consts::PI / 2.0);
    let cap = match cap {
        Cap::Butt => LineCap::Butt,
        Cap::Round => LineCap::Round(angle),
        Cap::Square => LineCap::Square,
    };
    let style = StrokeStyle {
        width: half_width * 2.0,
        start_cap: cap.clone(),
        end_cap: cap,
        join: join_style(join, tol_mm, half_width),
        miter_min_turn: std::f64::consts::PI / 36.0,
        math: MathMode::Integer,
    };
    let shapes = guarded(|| paths.stroke(style, false))?;
    from_shapes(shapes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pl(pts: &[(f64, f64)]) -> Polyline {
        let mut v: Vec<[f64; 2]> = pts.iter().map(|(x, y)| [*x, *y]).collect();
        if let Some(f) = v.first().copied() {
            v.push(f);
        }
        Polyline(v)
    }

    fn square(x: f64, y: f64, s: f64) -> Polyline {
        pl(&[(x, y), (x + s, y), (x + s, y + s), (x, y + s)])
    }

    fn area(p: &Polyline) -> f64 {
        let mut a = 0.0;
        for w in p.0.windows(2) {
            a += w[0][0] * w[1][1] - w[1][0] * w[0][1];
        }
        a / 2.0
    }

    /// Total area (outers minus holes) of a ring set.
    fn area_of(rings: &[Polyline]) -> f64 {
        let mut acc = std::collections::BTreeMap::<i64, f64>::new();
        for p in rings {
            let a = area(p).abs();
            let parent = (p.0[0][0] * 1024.0) as i64;
            acc.insert(parent, a);
        }
        // EvenOdd area: sum outers − sum holes. Use winding signs instead: outer rings are CCW.
        rings.iter().map(area).sum()
    }

    #[test]
    fn union_of_overlapping_squares_is_one_ring() {
        let out = boolean(&[square(0.0, 0.0, 10.0)], &[square(5.0, 5.0, 10.0)], BooleanOp::Union).unwrap();
        assert_eq!(out.len(), 1, "{out:?}");
        assert!((area(&out[0]).abs() - 175.0).abs() < 1e-6, "{:?}", area(&out[0]));
    }

    #[test]
    fn difference_makes_a_hole() {
        let out = boolean(&[square(0.0, 0.0, 10.0)], &[square(3.0, 3.0, 4.0)], BooleanOp::Difference).unwrap();
        assert_eq!(out.len(), 2, "one outer + one hole: {out:?}");
        assert!((area_of(&out) - 84.0).abs() < 1e-6, "{:?}", area_of(&out));
    }

    #[test]
    fn intersection_is_the_overlap() {
        let out = boolean(&[square(0.0, 0.0, 10.0)], &[square(5.0, 5.0, 10.0)], BooleanOp::Intersection).unwrap();
        assert_eq!(out.len(), 1);
        assert!((area(&out[0]).abs() - 25.0).abs() < 1e-6);
    }

    #[test]
    fn an_o_with_same_winding_contours_keeps_its_hole() {
        // Outer ring and inner ring wound the SAME way: EvenOdd normalisation keeps the hole.
        let outer = pl(&[(-10.0, -10.0), (10.0, -10.0), (10.0, 10.0), (-10.0, 10.0)]);
        let inner = pl(&[(-4.0, -4.0), (4.0, -4.0), (4.0, 4.0), (-4.0, 4.0)]);
        let out = boolean(&[outer, inner], &[square(50.0, 50.0, 1.0)], BooleanOp::Union).unwrap();
        assert_eq!(out.len(), 3, "O ring + hole + clip square: {out:?}");
        let o_area = area_of(&out) - 1.0; // minus the separate 1×1 clip square
        assert!((o_area - (400.0 - 64.0)).abs() < 1e-4, "{o_area}");
    }

    #[test]
    fn offset_grows_and_shrinks() {
        let miter = Join::Miter { limit: 4.0 };
        let out = offset(&[square(0.0, 0.0, 10.0)], 1.0, miter, 0.01).unwrap();
        assert_eq!(out.len(), 1);
        assert!((area(&out[0]).abs() - 144.0).abs() < 0.1, "{:?}", area(&out[0]));
        let out = offset(&[square(0.0, 0.0, 10.0)], 1.0, Join::Round, 0.01).unwrap();
        let want = 144.0 - 4.0 + std::f64::consts::PI; // 12×12 minus squared corners plus arcs: 140 + π
        assert!((area(&out[0]).abs() - want).abs() / want < 0.001, "{:?} vs {want}", area(&out[0]));
        // Shrinking past the middle empties the shape.
        let out = offset(&[square(0.0, 0.0, 10.0)], -6.0, miter, 0.01).unwrap();
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn stroke_outlines_an_open_path() {
        let line = Polyline(vec![[0.0, 0.0], [10.0, 0.0]]);
        let out = stroke_outline(&[line], 1.0, Join::Bevel, Cap::Round, 0.01).unwrap();
        assert_eq!(out.len(), 1);
        let want = 20.0 + std::f64::consts::PI;
        assert!((area(&out[0]).abs() - want).abs() / want < 0.01, "{:?} vs {want}", area(&out[0]));
    }

    #[test]
    fn non_finite_input_is_refused() {
        let bad = Polyline(vec![[0.0, 0.0], [f64::NAN, 0.0], [1.0, 1.0]]);
        assert!(boolean(&[bad], &[square(0.0, 0.0, 1.0)], BooleanOp::Union).is_err());
    }

    #[test]
    fn results_are_deterministic() {
        let run = || boolean(&[square(0.0, 0.0, 10.0)], &[square(5.0, 5.0, 10.0)], BooleanOp::Union).unwrap();
        assert_eq!(run(), run());
    }
}
