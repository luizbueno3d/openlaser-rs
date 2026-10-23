//! Flattening and path clean-up: Auto-Join (OM `geometry.join_lines` port), Close Path, Delete Duplicates.
//!
//! `join_lines` is ported from the OpenMopa prototype (OM) `mopa_luiz/geometry.py` @ 6aed8ff (identical at
//! 5f7aa63), MIT licence. The operation order (endpoint clustering, chain walking, closing rule) is kept so
//! results match the prototype exactly.

use std::collections::{HashMap, VecDeque};

use kurbo::{BezPath, PathEl, Point};

use crate::{Bounds, DEFAULT_FLATTEN_TOL_MM, P2, Polyline};

/// The smallest flattening tolerance honoured (mm). Smaller requests use this value.
pub const MIN_FLATTEN_TOL_MM: f64 = 1e-6;

/// Curves whose control polygon is longer than this many tolerances are not subdivided adaptively; they are
/// split into [`MAX_CURVE_SEGMENTS`] equal steps in `t` instead (the adaptive split would need more than 2^15
/// segments). At the default 0.01 mm tolerance that is a curve 10 km long: never a real design.
const MAX_LENGTH_OVER_TOL: f64 = 1e9;

/// Line segments one oversized curve is split into (see [`MAX_LENGTH_OVER_TOL`]).
pub const MAX_CURVE_SEGMENTS: usize = 1 << 16;
/// Maximum number of points emitted by one flatten operation.
pub const MAX_FLATTEN_POINTS: usize = 1_000_000;

/// Backstop for the adaptive split (2^20 segments); `MAX_LENGTH_OVER_TOL` keeps real curves far below it.
const MAX_SPLIT_DEPTH: u32 = 20;

/// One polyline per subpath; ClosePath repeats the first point. Tolerance in mm.
///
/// - Lines are copied exactly; quadratic and cubic segments are split adaptively with a guaranteed bound: every
///   point of the curve is within `tol_mm` of the polyline and every vertex lies on the curve.
/// - A tolerance that is not finite or not positive means [`DEFAULT_FLATTEN_TOL_MM`]; below
///   [`MIN_FLATTEN_TOL_MM`] it is raised to that floor.
/// - A subpath that is only a `MoveTo` (nothing drawn) yields no polyline; `M p Z` yields `[p, p]`. A drawing
///   element after `ClosePath` without a `MoveTo` starts a new subpath at the closed subpath's start (SVG).
/// - Non-finite coordinates are copied through (never dropped), so `Polyline::all_finite` can refuse them
///   downstream.
pub fn flatten(path: &BezPath, tol_mm: f64) -> Result<Vec<Polyline>, crate::Error> {
    let tol = effective_tolerance(tol_mm);
    let mut out = Vec::new();
    let mut current: Vec<P2> = Vec::new();
    let mut point_count = 0usize;
    // The current point and the start of the current subpath.
    let mut last: Option<Point> = None;
    let mut start: Option<Point> = None;

    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                finish(&mut current, &mut out);
                push_point(&mut current, [p.x, p.y], &mut point_count)?;
                last = Some(p);
                start = Some(p);
            }
            PathEl::LineTo(p) => {
                begin_segment(&mut current, last, start, p, &mut point_count)?;
                push_point(&mut current, [p.x, p.y], &mut point_count)?;
                last = Some(p);
            }
            PathEl::QuadTo(p1, p2) => {
                let p0 = begin_segment(&mut current, last, start, p2, &mut point_count)?;
                flatten_curve(&[p0, p1, p2], tol, &mut current, &mut point_count)?;
                last = Some(p2);
            }
            PathEl::CurveTo(p1, p2, p3) => {
                let p0 = begin_segment(&mut current, last, start, p3, &mut point_count)?;
                flatten_curve(&[p0, p1, p2, p3], tol, &mut current, &mut point_count)?;
                last = Some(p3);
            }
            PathEl::ClosePath => {
                if let (Some(&first), Some(&end)) = (current.first(), current.last())
                    && (first != end || current.len() == 1)
                {
                    push_point(&mut current, first, &mut point_count)?;
                }
                finish(&mut current, &mut out);
                last = start;
            }
        }
    }
    finish(&mut current, &mut out);
    Ok(out)
}

fn push_point(current: &mut Vec<P2>, point: P2, point_count: &mut usize) -> Result<(), crate::Error> {
    *point_count = point_count.checked_add(1).ok_or(crate::Error::TooManyPoints(MAX_FLATTEN_POINTS))?;
    if *point_count > MAX_FLATTEN_POINTS {
        return Err(crate::Error::TooManyPoints(MAX_FLATTEN_POINTS));
    }
    current.push(point);
    Ok(())
}

fn finish(current: &mut Vec<P2>, out: &mut Vec<Polyline>) {
    if current.len() >= 2 {
        out.push(Polyline(std::mem::take(current)));
    } else {
        current.clear();
    }
}

/// Makes sure `current` holds the segment's start point and returns it. A segment with no current point
/// (a path that does not begin with `MoveTo`) starts at its own end point.
fn begin_segment(current: &mut Vec<P2>, last: Option<Point>, start: Option<Point>, end: Point, point_count: &mut usize) -> Result<Point, crate::Error> {
    let p0 = last.or(start).unwrap_or(end);
    if current.is_empty() {
        push_point(current, [p0.x, p0.y], point_count)?;
    }
    Ok(p0)
}

fn effective_tolerance(tol_mm: f64) -> f64 {
    if tol_mm.is_finite() && tol_mm > 0.0 { tol_mm.max(MIN_FLATTEN_TOL_MM) } else { DEFAULT_FLATTEN_TOL_MM }
}

/// Appends the flattened points of one curve (excluding its start point, which `current` already holds).
///
/// Adaptive de Casteljau halving with a guaranteed bound: a piece is emitted as its chord once every control
/// point lies within `tol` of that chord (as a segment). The curve lies inside its control points' convex hull,
/// and distance to a segment is convex, so the whole piece is then within `tol` of the chord. kurbo's own
/// flattener only estimates the error (measured up to 2x over), so it is not used.
fn flatten_curve(ctrl: &[Point], tol: f64, current: &mut Vec<P2>, point_count: &mut usize) -> Result<(), crate::Error> {
    let end = ctrl.last().copied().unwrap_or(Point::ORIGIN);
    if !ctrl.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
        // Copy the end point through: downstream refuses non-finite geometry; it is never dropped here.
        return push_point(current, [end.x, end.y], point_count);
    }
    let polygon_len: f64 = ctrl.windows(2).filter_map(|w| Some(w.first()?.distance(*w.get(1)?))).sum();
    if !polygon_len.is_finite() || polygon_len / tol > MAX_LENGTH_OVER_TOL {
        return uniform_steps(ctrl, current, point_count);
    }
    let mut stack: Vec<(Vec<Point>, u32)> = vec![(ctrl.to_vec(), 0)];
    while let Some((piece, depth)) = stack.pop() {
        let (Some(&a), Some(&b)) = (piece.first(), piece.last()) else { continue };
        let inner = piece.get(1..piece.len().saturating_sub(1)).unwrap_or(&[]);
        if depth >= MAX_SPLIT_DEPTH || inner.iter().all(|&p| dist_to_segment(p, a, b) <= tol) {
            push_point(current, [b.x, b.y], point_count)?;
            continue;
        }
        let (left, right) = split_half(&piece);
        // Right first: the stack is LIFO, so the left half is emitted first.
        stack.push((right, depth + 1));
        stack.push((left, depth + 1));
    }
    Ok(())
}

fn dist_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let d = b - a;
    let len2 = d.hypot2();
    if len2 > 0.0 {
        let t = ((p - a).dot(d) / len2).clamp(0.0, 1.0);
        p.distance(a + d * t)
    } else {
        p.distance(a)
    }
}

/// De Casteljau split at t = 0.5 of a Bézier of any degree.
fn split_half(ctrl: &[Point]) -> (Vec<Point>, Vec<Point>) {
    let mut row: Vec<Point> = ctrl.to_vec();
    let mut left = Vec::with_capacity(ctrl.len());
    let mut right = Vec::with_capacity(ctrl.len());
    while let (Some(&first), Some(&last)) = (row.first(), row.last()) {
        left.push(first);
        right.push(last);
        row = row.windows(2).filter_map(|w| Some(w.first()?.midpoint(*w.get(1)?))).collect();
    }
    right.reverse();
    (left, right)
}

/// De Casteljau evaluation at [`MAX_CURVE_SEGMENTS`] equal steps in t (oversized curves only).
fn uniform_steps(ctrl: &[Point], current: &mut Vec<P2>, point_count: &mut usize) -> Result<(), crate::Error> {
    if point_count.saturating_add(MAX_CURVE_SEGMENTS) > MAX_FLATTEN_POINTS {
        return Err(crate::Error::TooManyPoints(MAX_FLATTEN_POINTS));
    }
    let n = MAX_CURVE_SEGMENTS as f64;
    let mut scratch: Vec<Point> = Vec::with_capacity(ctrl.len());
    for i in 1..=MAX_CURVE_SEGMENTS {
        let t = i as f64 / n;
        scratch.clear();
        scratch.extend_from_slice(ctrl);
        for k in (1..scratch.len()).rev() {
            for j in 0..k {
                if let (Some(&a), Some(&b)) = (scratch.get(j), scratch.get(j + 1))
                    && let Some(s) = scratch.get_mut(j)
                {
                    *s = a.lerp(b, t);
                }
            }
        }
        if let Some(p) = scratch.first() {
            push_point(current, [p.x, p.y], point_count)?;
        }
    }
    Ok(())
}

pub fn bounds_of(polys: &[Polyline]) -> Option<Bounds> {
    let mut it = polys.iter().flat_map(|p| p.0.iter());
    let first = it.next()?;
    let mut b = Bounds { min: *first, max: *first };
    for p in it {
        b.min = [b.min[0].min(p[0]), b.min[1].min(p[1])];
        b.max = [b.max[0].max(p[0]), b.max[1].max(p[1])];
    }
    Some(b)
}

// ---- Auto-Join: OM geometry.join_lines ---------------------------------------------------------------------

/// What [`join_lines_report`] did (OM's `JoinResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct JoinReport {
    pub polylines: Vec<Polyline>,
    pub input_segments: usize,
    pub output_paths: usize,
    pub joins_performed: usize,
    pub closed_loops: usize,
}

/// Auto-Join (OM `geometry.join_lines` port): chains ends closer than tol.
pub fn join_lines(polys: &[Polyline], tol_mm: f64) -> Vec<Polyline> {
    join_lines_report(polys, tol_mm).polylines
}

/// OM `_NodeMap`: clusters endpoints into node ids within the tolerance.
struct NodeMap {
    tolerance: f64,
    buckets: HashMap<(i64, i64), Vec<usize>>,
    centroids: Vec<P2>,
}

/// Python `round(v)` (ties to even) as a bucket key. The float→int cast saturates (NaN gives 0): far-away
/// points only share a bucket, and the distance test still decides.
fn bucket_key(v: f64) -> i64 {
    v.round_ties_even() as i64
}

impl NodeMap {
    fn new(tolerance: f64) -> Self {
        NodeMap { tolerance: tolerance.max(1e-9), buckets: HashMap::new(), centroids: Vec::new() }
    }

    fn node_id(&mut self, p: P2) -> usize {
        let bx = bucket_key(p[0] / self.tolerance);
        let by = bucket_key(p[1] / self.tolerance);
        for dx in [-1i64, 0, 1] {
            for dy in [-1i64, 0, 1] {
                let Some(ids) = self.buckets.get(&(bx.saturating_add(dx), by.saturating_add(dy))) else { continue };
                for &nid in ids {
                    if let Some(c) = self.centroids.get(nid)
                        && (c[0] - p[0]).hypot(c[1] - p[1]) <= self.tolerance
                    {
                        return nid;
                    }
                }
            }
        }
        let nid = self.centroids.len();
        self.centroids.push(p);
        self.buckets.entry((bx, by)).or_default().push(nid);
        nid
    }
}

fn mark(consumed: &mut [bool], i: usize) {
    if let Some(c) = consumed.get_mut(i) {
        *c = true;
    }
}

/// [`join_lines`] with OM's counters. Inputs with fewer than 2 points are dropped, as in OM (they draw
/// nothing). A tolerance that is negative or not finite joins nothing (OM raises for a negative one): the
/// inputs with at least 2 points come back unchanged.
pub fn join_lines_report(polys: &[Polyline], tol_mm: f64) -> JoinReport {
    let segments: Vec<&[P2]> = polys.iter().filter(|p| p.len() >= 2).map(|p| p.0.as_slice()).collect();
    if !(tol_mm.is_finite() && tol_mm >= 0.0) {
        let polylines: Vec<Polyline> = segments.iter().map(|s| Polyline(s.to_vec())).collect();
        let n = polylines.len();
        return JoinReport { polylines, input_segments: n, output_paths: n, joins_performed: 0, closed_loops: 0 };
    }

    let mut nodes = NodeMap::new(tol_mm);
    let mut seg_endpoints: Vec<(usize, usize)> = Vec::with_capacity(segments.len());
    let mut incidence: HashMap<usize, Vec<usize>> = HashMap::new();
    for (index, seg) in segments.iter().enumerate() {
        let (Some(&first), Some(&last)) = (seg.first(), seg.last()) else { continue };
        let a = nodes.node_id(first);
        let b = nodes.node_id(last);
        seg_endpoints.push((a, b));
        incidence.entry(a).or_default().push(index);
        if a != b {
            incidence.entry(b).or_default().push(index);
        }
    }

    let mut consumed = vec![false; segments.len()];
    let mut output: Vec<Polyline> = Vec::new();
    let mut joins = 0usize;
    let mut closed = 0usize;

    // OM `pick_neighbor`: none at a branching node (degree > 2), else the first unconsumed incident segment.
    let pick_neighbor = |node: usize, consumed: &[bool]| -> Option<usize> {
        let inc = incidence.get(&node)?;
        if inc.len() > 2 {
            return None;
        }
        inc.iter().copied().find(|&s| consumed.get(s) == Some(&false))
    };

    for seed in 0..segments.len() {
        if consumed.get(seed) != Some(&false) {
            continue;
        }
        mark(&mut consumed, seed);
        let (Some(seed_seg), Some(&(mut head_node, mut tail_node))) = (segments.get(seed), seg_endpoints.get(seed)) else { continue };
        let mut chain: VecDeque<P2> = seed_seg.iter().copied().collect();

        // Extend forward off `tail_node`.
        while head_node != tail_node {
            let Some(nxt) = pick_neighbor(tail_node, &consumed) else { break };
            mark(&mut consumed, nxt);
            let (Some(seg), Some(&(na, nb))) = (segments.get(nxt), seg_endpoints.get(nxt)) else { break };
            let mut seg: Vec<P2> = seg.to_vec();
            let new_tail = if na == tail_node {
                nb
            } else {
                seg.reverse();
                na
            };
            chain.extend(seg.iter().skip(1));
            tail_node = new_tail;
            joins += 1;
        }

        // Extend backward off `head_node`.
        while head_node != tail_node {
            let Some(prev) = pick_neighbor(head_node, &consumed) else { break };
            mark(&mut consumed, prev);
            let (Some(seg), Some(&(na, nb))) = (segments.get(prev), seg_endpoints.get(prev)) else { break };
            let mut seg: Vec<P2> = seg.to_vec();
            let new_head = if nb == head_node {
                na
            } else {
                seg.reverse();
                nb
            };
            // chain[:0] = seg[:-1]
            let keep = seg.len().saturating_sub(1);
            for p in seg.iter().take(keep).rev() {
                chain.push_front(*p);
            }
            head_node = new_head;
            joins += 1;
        }

        if head_node == tail_node && chain.len() >= 3 {
            if let (Some(&first), Some(&last)) = (chain.front(), chain.back())
                && first != last
            {
                chain.push_back(first);
            }
            closed += 1;
        }
        output.push(Polyline(chain.into_iter().collect()));
    }

    let output_paths = output.len();
    JoinReport { polylines: output, input_segments: segments.len(), output_paths, joins_performed: joins, closed_loops: closed }
}

// ---- Close Path and Delete Duplicates -------------------------------------------------------------------------

/// Close Path: closes open paths whose ends are within tol; returns how many were closed.
///
/// A path with at least 3 points whose ends differ by `0 < d <= tol` gets its first point appended (an exact
/// closing segment; no vertex moves). Already-closed paths, two-point paths and paths whose ends are farther
/// apart come back unchanged.
pub fn close_paths(polys: &[Polyline], tol_mm: f64) -> (Vec<Polyline>, usize) {
    let mut closed = 0usize;
    let out = polys
        .iter()
        .map(|p| {
            let mut q = p.clone();
            if let (Some(&first), Some(&last)) = (p.0.first(), p.0.last())
                && p.len() >= 3
                && first != last
                && (first[0] - last[0]).hypot(first[1] - last[1]) <= tol_mm
            {
                q.0.push(first);
                closed += 1;
            }
            q
        })
        .collect();
    (out, closed)
}

/// Grid cell of a point. The float→int casts saturate: far-away points only share a cell.
fn cell(p: P2, size: f64) -> (i64, i64) {
    ((p[0] / size).floor() as i64, (p[1] / size).floor() as i64)
}

fn same_within(a: &[P2], b: &[P2], tol: f64, reversed: bool) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let close = |p: &P2, q: &P2| (p[0] - q[0]).hypot(p[1] - q[1]) <= tol;
    if reversed { a.iter().zip(b.iter().rev()).all(|(p, q)| close(p, q)) } else { a.iter().zip(b.iter()).all(|(p, q)| close(p, q)) }
}

/// Delete Duplicates: indices of polylines equal (within tol, either direction) to an earlier one.
///
/// Equal means the same number of points, each pair within `tol_mm`, compared forwards or with one of them
/// reversed. Empty polylines and polylines with non-finite points are never duplicates. Indices ascend.
pub fn duplicates(polys: &[Polyline], tol_mm: f64) -> Vec<usize> {
    let tol = if tol_mm.is_finite() && tol_mm >= 0.0 { tol_mm } else { 0.0 };
    let size = tol.max(1e-9);
    // Cell of an end point → earlier polylines with an end point in it.
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let mut dups = Vec::new();
    for (i, p) in polys.iter().enumerate() {
        let (Some(&first), Some(&last)) = (p.0.first(), p.0.last()) else { continue };
        if !p.all_finite() {
            continue;
        }
        let (cx, cy) = cell(first, size);
        let mut found = false;
        'search: for dx in [-1i64, 0, 1] {
            for dy in [-1i64, 0, 1] {
                let Some(cands) = grid.get(&(cx.saturating_add(dx), cy.saturating_add(dy))) else { continue };
                for &j in cands {
                    if let Some(q) = polys.get(j)
                        && (same_within(&p.0, &q.0, tol, false) || same_within(&p.0, &q.0, tol, true))
                    {
                        found = true;
                        break 'search;
                    }
                }
            }
        }
        if found {
            dups.push(i);
        }
        let a = cell(first, size);
        let b = cell(last, size);
        grid.entry(a).or_default().push(i);
        if b != a {
            grid.entry(b).or_default().push(i);
        }
    }
    dups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pl(pts: &[[f64; 2]]) -> Polyline {
        Polyline(pts.to_vec())
    }

    #[test]
    fn flatten_lines_and_close() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.line_to((10.0, 10.0));
        p.close_path();
        p.move_to((20.0, 20.0));
        p.line_to((30.0, 20.0));
        let out = flatten(&p, 0.01).unwrap();
        assert_eq!(out, vec![pl(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 0.0]]), pl(&[[20.0, 20.0], [30.0, 20.0]])]);
    }

    #[test]
    fn flatten_close_does_not_double_the_start() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.line_to((0.0, 0.0));
        p.close_path();
        p.move_to((3.0, 3.0));
        p.close_path();
        assert_eq!(flatten(&p, 0.01).unwrap(), vec![pl(&[[0.0, 0.0], [10.0, 0.0], [0.0, 0.0]]), pl(&[[3.0, 3.0], [3.0, 3.0]])]);
    }

    #[test]
    fn flatten_drawing_after_close_starts_at_subpath_start() {
        let mut p = BezPath::new();
        p.move_to((1.0, 1.0));
        p.line_to((2.0, 1.0));
        p.close_path();
        p.line_to((1.0, 5.0));
        let out = flatten(&p, 0.01).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[1], pl(&[[1.0, 1.0], [1.0, 5.0]]));
    }

    #[test]
    fn flatten_bare_moves_draw_nothing_and_curves_end_exactly() {
        let mut p = BezPath::new();
        p.move_to((5.0, 5.0));
        p.move_to((0.0, 0.0));
        p.curve_to((0.0, 10.0), (10.0, 10.0), (10.0, 0.0));
        let out = flatten(&p, 0.01).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.first(), Some(&[0.0, 0.0]));
        assert_eq!(out[0].0.last(), Some(&[10.0, 0.0]));
        assert!(out[0].len() > 10);
    }

    #[test]
    fn flatten_huge_and_nonfinite_curves_terminate() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((0.0, 1e300), (1e300, 1e300), (1e300, 0.0));
        p.move_to((0.0, 0.0));
        p.quad_to((f64::NAN, 1.0), (2.0, 0.0));
        let out = flatten(&p, 1e-300).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].len(), MAX_CURVE_SEGMENTS + 1);
        assert_eq!(out[0].0.last(), Some(&[1e300, 0.0]));
        assert_eq!(out[1].0, vec![[0.0, 0.0], [2.0, 0.0]]);
    }

    #[test]
    fn flatten_refuses_output_over_point_budget() {
        let mut path = BezPath::new();
        for _ in 0..=MAX_FLATTEN_POINTS {
            path.move_to((0.0, 0.0));
        }
        assert!(matches!(flatten(&path, 0.01), Err(crate::Error::TooManyPoints(MAX_FLATTEN_POINTS))));
    }

    #[test]
    fn join_chains_and_closes_loops() {
        let polys = vec![pl(&[[0.0, 0.0], [1.0, 0.0]]), pl(&[[1.0, 0.0], [1.0, 1.0]]), pl(&[[0.0, 0.0], [1.0, 1.0]])];
        let r = join_lines_report(&polys, 0.05);
        assert_eq!(r.output_paths, 1);
        assert_eq!(r.joins_performed, 2);
        assert_eq!(r.closed_loops, 1);
        assert_eq!(r.polylines[0].0, vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]]);
    }

    #[test]
    fn join_stops_at_branching_nodes() {
        // A T-junction: three segments meet at (0, 0).
        let polys = vec![pl(&[[0.0, 0.0], [1.0, 0.0]]), pl(&[[0.0, 0.0], [-1.0, 0.0]]), pl(&[[0.0, 0.0], [0.0, 1.0]])];
        let r = join_lines_report(&polys, 0.05);
        assert_eq!(r.output_paths, 3);
        assert_eq!(r.joins_performed, 0);
    }

    #[test]
    fn join_within_tolerance_and_reversed() {
        let polys = vec![pl(&[[0.0, 0.0], [1.0, 0.0]]), pl(&[[2.0, 0.0], [1.03, 0.0]])];
        assert_eq!(join_lines(&polys, 0.05), vec![pl(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]])]);
        // Backward extension prepends the reversed neighbour without its last point.
        let polys = vec![pl(&[[1.0, 0.0], [2.0, 0.0]]), pl(&[[0.0, 0.0], [1.0, 0.0]])];
        assert_eq!(join_lines(&polys, 0.05), vec![pl(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]])]);
        assert_eq!(join_lines(&polys, -1.0).len(), 2);
        assert_eq!(join_lines(&polys, f64::NAN).len(), 2);
    }

    #[test]
    fn close_paths_counts_only_closed_ones() {
        let polys = vec![
            pl(&[[0.0, 0.0], [5.0, 0.0], [5.0, 5.0], [0.0, 0.04]]),
            pl(&[[0.0, 0.0], [5.0, 0.0], [5.0, 5.0], [0.0, 0.0]]),
            pl(&[[0.0, 0.0], [5.0, 0.0], [5.0, 5.0], [0.0, 1.0]]),
            pl(&[[0.0, 0.0], [0.0, 0.01]]),
        ];
        let (out, n) = close_paths(&polys, 0.05);
        assert_eq!(n, 1);
        assert_eq!(out[0].0.last(), Some(&[0.0, 0.0]));
        assert_eq!(out[0].len(), 5);
        assert_eq!(&out[1..], &polys[1..]);
    }

    #[test]
    fn duplicates_forward_and_reversed() {
        let a = pl(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]);
        let b = pl(&[[1.0, 1.0], [1.0, 0.0], [0.0, 0.001]]);
        let c = pl(&[[0.0, 0.0], [1.0, 0.0], [1.0, 2.0]]);
        let d = pl(&[[0.0, 0.0005], [1.0, 0.0], [1.0, 1.0]]);
        let nan = pl(&[[f64::NAN, 0.0], [1.0, 0.0]]);
        assert_eq!(duplicates(&[a.clone(), b, c, d, Polyline::default(), nan.clone(), nan, a], 0.01), vec![1, 3, 7]);
    }
}
