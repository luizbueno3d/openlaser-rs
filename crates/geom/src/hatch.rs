//! Hatch: the exact OM `raster.hatch_polylines` port (T3) and LightBurn Fill.
//!
//! Ported from the OpenMopa prototype (OM) `mopa_luiz/raster.py` @ 6aed8ff (identical at 5f7aa63), MIT licence.
//! Python's operation order is kept exactly: the rotation formulas, `y_start = floor(min_y / pitch) * pitch +
//! pitch * 0.0173`, `y += pitch` accumulation, the half-open `[lo, hi)` crossing test, the `x1 - x0 > 1e-6` pair
//! filter and the boustrophedon flip (T3: segment count and order exact, coordinates within 1e-9 mm).
//!
//! Divergences from OM, all refusals where OM would loop forever or return garbage: non-finite inputs are
//! refused (`NonFinite`), a non-finite pitch is refused like a non-positive one (`BadInterval`), and a hatch
//! needing more scan lines than the limit is refused (`TooManyLines`) instead of running unbounded.

use std::cmp::Ordering;

use crate::{Error, P2, Polyline};

/// Scan lines `hatch_om` will run over all passes before refusing with `TooManyLines`. A 200 mm field at a
/// 0.001 mm interval is 200 000 lines per pass.
pub const HATCH_OM_MAX_LINES: usize = 2_000_000;

/// OM's scan offset as a fraction of the pitch: the scan never hits a vertex row exactly.
const SCAN_OFFSET_FRACTION: f64 = 0.0173;

/// Pairs closer than this (mm, along the scan line) are dropped (OM `x1 - x0 > 1e-6`).
const MIN_PAIR_MM: f64 = 1e-6;

/// Exact port of OM `raster.hatch_polylines` (T3: segment count and order exact, coordinates within 1e-9).
pub fn hatch_om(polys: &[Polyline], angle_deg: f64, pitch_mm: f64, passes: u32, angle_step_deg: f64, closure_tol_mm: f64) -> Result<OmHatch, Error> {
    if !(pitch_mm.is_finite() && pitch_mm > 0.0) {
        return Err(Error::BadInterval(pitch_mm));
    }
    if passes < 1 {
        return Ok(OmHatch { segments: Vec::new(), closed_input_count: 0, skipped_open_count: 0, angles_deg: Vec::new() });
    }
    if !angle_deg.is_finite() || !angle_step_deg.is_finite() || polys.iter().any(|p| !p.all_finite()) {
        return Err(Error::NonFinite);
    }
    let (closed, open) = split_closed(polys, closure_tol_mm);
    if closed.is_empty() {
        return Ok(OmHatch { segments: Vec::new(), closed_input_count: 0, skipped_open_count: open.len(), angles_deg: Vec::new() });
    }
    let angles: Vec<f64> = (0..passes).map(|i| angle_deg + f64::from(i) * angle_step_deg).collect();
    let mut budget = Budget::new(HATCH_OM_MAX_LINES);
    let mut segments = Vec::new();
    for &angle in &angles {
        segments.extend(hatch_one_pass(&closed, angle, pitch_mm, true, &mut budget)?);
    }
    Ok(OmHatch { segments, closed_input_count: closed.len(), skipped_open_count: open.len(), angles_deg: angles })
}

#[derive(Debug, Clone, PartialEq)]
pub struct OmHatch {
    pub segments: Vec<[P2; 2]>,
    pub closed_input_count: usize,
    pub skipped_open_count: usize,
    pub angles_deg: Vec<f64>,
}

/// LightBurn Fill. Pass i uses angle + i × angle_increment; `bidirectional` = OM boustrophedon (identical
/// segments to `hatch_om` for that pass); one-way flips every other row back; `cross_hatch` adds a +90° pass
/// after each pass. Even-odd over all closed inputs; open inputs are reported, never dropped silently.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HatchParams {
    pub interval_mm: f64,
    pub angle_deg: f64,
    pub passes: u16,
    pub angle_increment_deg: f64,
    pub bidirectional: bool,
    pub cross_hatch: bool,
    pub closure_tol_mm: f64,
    pub max_lines: usize,
}

/// `passes[k]` is scanned at `angles_deg[k]`. With `cross_hatch`, entry `2i` is pass i and `2i + 1` its +90°
/// companion. No closed input gives no passes (as OM). `open_inputs` are the indices of inputs that are not
/// closed within `closure_tol_mm` (they are not filled; the caller reports them).
#[derive(Debug, Clone, PartialEq)]
pub struct HatchResult {
    pub passes: Vec<Vec<[P2; 2]>>,
    pub angles_deg: Vec<f64>,
    pub closed_used: usize,
    pub open_inputs: Vec<usize>,
}

/// LightBurn Fill (see [`HatchParams`]). `max_lines` bounds the scan lines over all passes
/// (`TooManyLines(needed, max_lines)`).
pub fn hatch(polys: &[Polyline], p: &HatchParams) -> Result<HatchResult, Error> {
    if !(p.interval_mm.is_finite() && p.interval_mm > 0.0) {
        return Err(Error::BadInterval(p.interval_mm));
    }
    if !p.angle_deg.is_finite() || !p.angle_increment_deg.is_finite() || polys.iter().any(|q| !q.all_finite()) {
        return Err(Error::NonFinite);
    }
    let (closed, open_inputs) = split_closed(polys, p.closure_tol_mm);
    let mut out = HatchResult { passes: Vec::new(), angles_deg: Vec::new(), closed_used: closed.len(), open_inputs };
    if closed.is_empty() || p.passes == 0 {
        return Ok(out);
    }
    let mut budget = Budget::new(p.max_lines);
    for i in 0..p.passes {
        let angle = p.angle_deg + f64::from(i) * p.angle_increment_deg;
        out.passes.push(hatch_one_pass(&closed, angle, p.interval_mm, p.bidirectional, &mut budget)?);
        out.angles_deg.push(angle);
        if p.cross_hatch {
            let cross = angle + 90.0;
            out.passes.push(hatch_one_pass(&closed, cross, p.interval_mm, p.bidirectional, &mut budget)?);
            out.angles_deg.push(cross);
        }
    }
    Ok(out)
}

/// OM `is_closed` split: (closed polylines, indices of the others).
fn split_closed(polys: &[Polyline], closure_tol_mm: f64) -> (Vec<&[P2]>, Vec<usize>) {
    let mut closed = Vec::new();
    let mut open = Vec::new();
    for (i, p) in polys.iter().enumerate() {
        if p.is_closed(closure_tol_mm) {
            closed.push(p.0.as_slice());
        } else {
            open.push(i);
        }
    }
    (closed, open)
}

/// Scan lines left before `TooManyLines`.
struct Budget {
    used: usize,
    max: usize,
}

impl Budget {
    fn new(max: usize) -> Self {
        Budget { used: 0, max }
    }

    /// Reserves `n` lines up front (an estimate; refuses before doing any work).
    fn check_estimate(&self, n: f64) -> Result<(), Error> {
        let left = self.max.saturating_sub(self.used);
        if !n.is_finite() || n > left as f64 {
            // Saturating float→int cast: an infinite estimate reports usize::MAX.
            let needed = self.used.saturating_add(if n.is_finite() && n > 0.0 { n as usize } else { usize::MAX });
            return Err(Error::TooManyLines(needed, self.max));
        }
        Ok(())
    }

    /// Counts one scan line.
    fn take(&mut self) -> Result<(), Error> {
        self.used = self.used.saturating_add(1);
        if self.used > self.max { Err(Error::TooManyLines(self.used, self.max)) } else { Ok(()) }
    }
}

/// OM `_hatch_one_pass`; `boustrophedon = false` keeps every row left to right (LightBurn one-way fill).
fn hatch_one_pass(closed: &[&[P2]], angle_deg: f64, pitch: f64, boustrophedon: bool, budget: &mut Budget) -> Result<Vec<[P2; 2]>, Error> {
    let rotate_in = (-angle_deg).to_radians();
    let (cin, sin_in) = (rotate_in.cos(), rotate_in.sin());
    let rotate_out = angle_deg.to_radians();
    let (cout, sout) = (rotate_out.cos(), rotate_out.sin());

    let rotated: Vec<Vec<P2>> = closed.iter().map(|poly| poly.iter().map(|&[x, y]| [x * cin - y * sin_in, x * sin_in + y * cin]).collect()).collect();

    let mut ys = rotated.iter().flat_map(|p| p.iter().map(|q| q[1]));
    let Some(first_y) = ys.next() else { return Ok(Vec::new()) };
    let (min_y, max_y) = ys.fold((first_y, first_y), |(lo, hi), y| (lo.min(y), hi.max(y)));

    let offset = pitch * SCAN_OFFSET_FRACTION;
    let y_start = (min_y / pitch).floor() * pitch + offset;
    let y_end = max_y + pitch;
    if !y_start.is_finite() || !y_end.is_finite() {
        return Err(Error::NonFinite);
    }
    budget.check_estimate(((y_end - y_start) / pitch).ceil())?;

    let mut segments = Vec::new();
    let mut xs: Vec<f64> = Vec::new();
    let mut pairs: Vec<(f64, f64)> = Vec::new();
    let mut flip = false;
    let mut y = y_start;
    while y < y_end {
        budget.take()?;
        xs.clear();
        for poly in &rotated {
            for w in poly.windows(2) {
                let (Some(&[ax, ay]), Some(&[bx, by])) = (w.first(), w.get(1)) else { continue };
                if ay == by {
                    continue;
                }
                let (lo, hi) = if ay < by { (ay, by) } else { (by, ay) };
                // Half-open interval keeps shared vertices from being counted twice.
                if y < lo || y >= hi {
                    continue;
                }
                let t = (y - ay) / (by - ay);
                xs.push(ax + t * (bx - ax));
            }
        }
        // Stable, like Python's list.sort(); every value is finite here.
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        pairs.clear();
        let (pairs_exact, _) = xs.as_chunks::<2>();
        for &[x0, x1] in pairs_exact {
            if x1 - x0 > MIN_PAIR_MM {
                pairs.push((x0, x1));
            }
        }
        if flip {
            pairs.reverse();
        }
        for &(a, b) in &pairs {
            let (x0, x1) = if flip { (b, a) } else { (a, b) };
            let p0 = [x0 * cout - y * sout, x0 * sout + y * cout];
            let p1 = [x1 * cout - y * sout, x1 * sout + y * cout];
            segments.push([p0, p1]);
        }
        if boustrophedon {
            flip = !flip;
        }
        let next = y + pitch;
        if next <= y {
            // The pitch vanished against y's magnitude: OM would loop forever.
            return Err(Error::TooManyLines(usize::MAX, budget.max));
        }
        y = next;
    }
    Ok(segments)
}

/// Even-odd point-in-region test over closed polylines (used by the frame ⊇ burn property test).
///
/// Every polyline is treated as a ring (a closing edge is added when its ends differ). Crossings use the same
/// half-open `[lo, hi)` rule as the hatch scan, so a point on a hatch line between a pair is inside. Non-finite
/// points are never inside.
pub fn inside_even_odd(closed: &[Polyline], p: P2) -> bool {
    let [px, py] = p;
    if !px.is_finite() || !py.is_finite() {
        return false;
    }
    let mut inside = false;
    for poly in closed {
        let pts = &poly.0;
        let (Some(&first), Some(&last)) = (pts.first(), pts.last()) else { continue };
        let closing = if first != last { Some([last, first]) } else { None };
        let edges = pts.windows(2).filter_map(|w| Some([*w.first()?, *w.get(1)?])).chain(closing);
        for [[ax, ay], [bx, by]] in edges {
            if ay == by {
                continue;
            }
            let (lo, hi) = if ay < by { (ay, by) } else { (by, ay) };
            if py < lo || py >= hi {
                continue;
            }
            let x = ax + (py - ay) / (by - ay) * (bx - ax);
            if x > px {
                inside = !inside;
            }
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(s: f64) -> Polyline {
        let h = s / 2.0;
        Polyline(vec![[-h, -h], [h, -h], [h, h], [-h, h], [-h, -h]])
    }

    #[test]
    fn om_square_matches_the_t3_first_rows() {
        let r = hatch_om(&[square(10.0)], 0.0, 1.0, 1, 90.0, 0.05).unwrap();
        assert_eq!(r.segments.len(), 10);
        assert_eq!(r.segments[0], [[-5.0, -4.9827], [5.0, -4.9827]]);
        assert_eq!(r.segments[1], [[5.0, -3.9827000000000004], [-5.0, -3.9827000000000004]]);
        assert_eq!(r.angles_deg, vec![0.0]);
        assert_eq!((r.closed_input_count, r.skipped_open_count), (1, 0));
    }

    #[test]
    fn om_refusals_and_empty_cases() {
        assert_eq!(hatch_om(&[square(10.0)], 0.0, 0.0, 1, 90.0, 0.05), Err(Error::BadInterval(0.0)));
        assert!(matches!(hatch_om(&[square(10.0)], 0.0, f64::NAN, 1, 90.0, 0.05), Err(Error::BadInterval(_))));
        let r = hatch_om(&[square(10.0)], 0.0, 1.0, 0, 90.0, 0.05).unwrap();
        assert_eq!((r.segments.len(), r.closed_input_count, r.skipped_open_count), (0, 0, 0));
        let open = Polyline(vec![[0.0, 0.0], [5.0, 0.0]]);
        let r = hatch_om(&[open], 0.0, 1.0, 1, 90.0, 0.05).unwrap();
        assert_eq!((r.segments.len(), r.closed_input_count, r.skipped_open_count), (0, 0, 1));
        let nan = Polyline(vec![[0.0, 0.0], [f64::NAN, 0.0], [0.0, 1.0], [0.0, 0.0]]);
        assert_eq!(hatch_om(&[nan], 0.0, 1.0, 1, 90.0, 0.05), Err(Error::NonFinite));
    }

    #[test]
    fn om_line_budget_refuses_instead_of_looping() {
        assert!(matches!(hatch_om(&[square(10.0)], 0.0, 1e-9, 1, 90.0, 0.05), Err(Error::TooManyLines(_, HATCH_OM_MAX_LINES))));
        let far = Polyline(vec![[1e17, 0.0], [1e17, 1.0], [1e17 + 1e3, 1.0], [1e17, 0.0]]);
        assert!(matches!(hatch_om(&[far], 90.0, 0.5, 1, 90.0, 0.05), Err(Error::TooManyLines(..))));
    }

    fn params(passes: u16, bidirectional: bool, cross_hatch: bool) -> HatchParams {
        HatchParams { interval_mm: 1.0, angle_deg: 0.0, passes, angle_increment_deg: 45.0, bidirectional, cross_hatch, closure_tol_mm: 0.05, max_lines: 10_000 }
    }

    #[test]
    fn fill_bidirectional_equals_om() {
        let polys = [square(10.0), Polyline(vec![[0.0, 0.0], [1.0, 0.0]])];
        let r = hatch(&polys, &params(3, true, false)).unwrap();
        let om = hatch_om(&polys, 0.0, 1.0, 3, 45.0, 0.05).unwrap();
        assert_eq!(r.passes.concat(), om.segments);
        assert_eq!(r.angles_deg, om.angles_deg);
        assert_eq!((r.closed_used, r.open_inputs.clone()), (1, vec![1]));
    }

    #[test]
    fn fill_one_way_and_cross_hatch() {
        let r = hatch(&[square(10.0)], &params(1, false, true)).unwrap();
        assert_eq!(r.angles_deg, vec![0.0, 90.0]);
        assert_eq!(r.passes.len(), 2);
        // One-way: every row left to right.
        assert!(r.passes[0].iter().all(|s| s[0][0] < s[1][0]));
        assert_eq!(r.passes[0].len(), 10);
        assert_eq!(r.passes[1].len(), 10);
    }

    #[test]
    fn fill_limits() {
        let mut p = params(1, true, false);
        p.max_lines = 5;
        assert!(matches!(hatch(&[square(10.0)], &p), Err(Error::TooManyLines(_, 5))));
        p.interval_mm = -1.0;
        assert_eq!(hatch(&[square(10.0)], &p), Err(Error::BadInterval(-1.0)));
        let r = hatch(&[square(10.0)], &params(0, true, false)).unwrap();
        assert!(r.passes.is_empty());
    }

    #[test]
    fn even_odd_handles_holes() {
        let donut = [square(10.0), square(4.0)];
        assert!(inside_even_odd(&donut, [4.0, 0.0]));
        assert!(!inside_even_odd(&donut, [0.0, 0.0]));
        assert!(!inside_even_odd(&donut, [6.0, 0.0]));
        assert!(!inside_even_odd(&donut, [f64::NAN, 0.0]));
        // An unclosed ring still gets its closing edge.
        let tri = [Polyline(vec![[0.0, 0.0], [4.0, 0.0], [0.0, 4.0]])];
        assert!(inside_even_odd(&tri, [1.0, 1.0]));
        assert!(!inside_even_odd(&tri, [3.0, 3.0]));
    }
}
