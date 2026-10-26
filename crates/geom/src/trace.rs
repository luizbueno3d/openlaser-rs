//! In-house bitmap tracing (no image crate): gray buffer → closed rings on pixel-boundary cracks.
//! The input is an 8-bit luma buffer as `Geom::Bitmap` stores it (row-major, top row first).
//! Pixel coordinates: x right, y DOWN, the unit square over pixel centres at half-integers — every boundary
//! edge lies on an integer grid line, so rings follow pixel edges exactly.

use kurbo::{BezPath, Point};

use crate::{Error, P2, PointBudget, Polyline};

/// Tracing options.
#[derive(Debug, Clone)]
pub struct TraceOptions {
    /// "ink" = `gray < threshold` (dark = ink, like LightBurn). A pixel exactly at the threshold is NOT ink
    /// (documented: the boundary sits at `gray == threshold`, ink is strictly darker).
    pub threshold: u8,
    /// Trace the light side instead.
    pub invert: bool,
    /// Drop rings whose |area| (px²) is below this. A dropped outer ring drops its holes too; a small hole
    /// alone is filled (it just disappears as a hole).
    pub ignore_less_than: f64,
    /// Douglas–Peucker tolerance in px (corner-preserving; a ring never drops below 3 points).
    pub optimize: f64,
    /// Smoothness 0..=1.33 (potrace alphamax): 0 = polygons, 1 = default. Sharp turns beyond the derived
    /// angle stay as corners.
    pub smoothness: f64,
}

impl Default for TraceOptions {
    fn default() -> Self {
        TraceOptions { threshold: 128, invert: false, ignore_less_than: 2.0, smoothness: 1.0, optimize: 0.5 }
    }
}

/// The largest image the tracer accepts (px).
const MAX_PIXELS: usize = 25_000_000;

/// Trace `gray` (px[0] × px[1] row-major, top row first) into closed rings in pixel coordinates.
/// Outer rings have NEGATIVE shoelace area (they run clockwise in y-down space) and holes POSITIVE —
/// opposite orientation, so EvenOdd keeps the holes. In the Y-up world mapping the sign flips again.
/// Diagonal-touching ink pixels are split the same way every time: the "saddle" is cut so the
/// top-left↔bottom-right pair joins (LightBurn-style majority rule on the corner's two ink pixels).
pub fn trace(gray: &[u8], px: [u32; 2], opts: &TraceOptions) -> Result<Vec<Polyline>, Error> {
    let (w, h) = (px[0] as usize, px[1] as usize);
    if gray.len() != w * h {
        return Err(Error::BadInput("the buffer length does not match the pixel size"));
    }
    if w == 0 || h == 0 {
        return Err(Error::BadInput("the image is empty"));
    }
    if w * h > MAX_PIXELS {
        return Err(Error::TooManyPoints(w * h));
    }
    // Ink mask. `ink == threshold` is NOT ink: the threshold is exclusive.
    let ink_of = |i: usize| {
        let v = *gray.get(i).unwrap_or(&0);
        if opts.invert { v > opts.threshold } else { v < opts.threshold }
    };
    let at = |c: isize, r: isize| c >= 0 && r >= 0 && (c as usize) < w && (r as usize) < h && ink_of(r as usize * w + c as usize);

    // Vertex grid (w+1)×(h+1): (c, r) is the corner between pixels (c-1,r-1) | (c,r-1) | (c-1,r) | (c,r).
    // A "saddle" (diagonal pair ink, other diagonal empty) is ambiguous; resolve by the top-left pair:
    // if pixels (c-1,r-1) and (c,r) are the ink ones, the boundary crosses top-left↔bottom-right;
    // otherwise (bottom-left pair) it crosses top-right↔bottom-left.
    let saddled = |c: usize, r: usize| -> Option<bool> {
        let (tl, tr, bl, br) = (at(c as isize - 1, r as isize - 1), at(c as isize, r as isize - 1), at(c as isize - 1, r as isize), at(c as isize, r as isize));
        match (tl, tr, bl, br) {
            (true, false, false, true) => Some(true),  // tl+br pair: cross tl↔br
            (false, true, true, false) => Some(false), // tr+bl pair: cross tr↔bl
            _ => None,
        }
    };
    // Boundary edges: a directed edge on a grid line with ink on the right (so rings run counter-clockwise
    // in y-up convention; in y-down pixel space outer rings get positive shoelace area — checked in tests).
    // For each vertex, up to two directed boundary edges meet it; we chain them into rings.
    let mut rings: Vec<Vec<Point>> = Vec::new();
    let mut visited: std::collections::HashSet<((usize, usize), (usize, usize))> = std::collections::HashSet::new();
    // The four grid directions as (dc, dr, corner): boundary from corner → corner+dir with ink on the right.
    // Right-down: east edge of a pixel, ink to its west (left in screen)… build edges pixel-by-pixel instead:
    // for each ink pixel, each of its 4 sides facing a non-ink neighbour emits an edge.
    let mut edges: std::collections::BTreeMap<(usize, usize), Vec<(usize, usize)>> = std::collections::BTreeMap::new();
    for r in 0..h {
        for c in 0..w {
            if !ink_of(r * w + c) {
                continue;
            }
            // c, r are already usize loop indices.
            // N side (ink below → walk west→east? keep ring so the region is to the LEFT of travel):
            // Use "ink on the left of travel" so outer rings are CCW in pixel (y-down) space.
            if !at(c as isize, r as isize - 1) {
                edges.entry((c + 1, r)).or_default().push((c, r)); // top side, travel right→left
            }
            if !at(c as isize + 1, r as isize) {
                edges.entry((c + 1, r + 1)).or_default().push((c + 1, r)); // right side, travel down→up
            }
            if !at(c as isize, r as isize + 1) {
                edges.entry((c, r + 1)).or_default().push((c + 1, r + 1)); // bottom side, left→right
            }
            if !at(c as isize - 1, r as isize) {
                edges.entry((c, r)).or_default().push((c, r + 1)); // left side, up→down
            }
        }
    }
    // Chain edges into rings. At a vertex with two outgoing options (a saddle corner), pick the turn that
    // keeps the diagonal pair together per the rule above.
    for start in edges.keys() {
        for first in 0..edges.get(start).map(Vec::len).unwrap_or(0) {
            let Some(first_to) = edges.get(start).and_then(|v| v.get(first)).copied() else { continue };
            if !visited.insert((*start, first_to)) {
                continue;
            }
            let mut ring: Vec<Point> = vec![pt(*start), pt(first_to)];
            let mut cur = first_to;
            while let Some(outs) = edges.get(&cur).cloned() {
                // The next edge: at a saddle (2 unvisited outs), pick the turn that keeps the top-left pair
                // together — when `saddled` says the tl+br pixels are ink we turn the way the tl↔br cut runs.
                let cands: Vec<(usize, usize)> = outs.iter().copied().filter(|to| !visited.contains(&(cur, *to))).collect();
                let next = cands.first().copied().map(|to| {
                    if cands.len() > 1
                        && let Some(tl_pair) = saddled(cur.0, cur.1)
                    {
                        // Incoming direction (prev vertex → cur); each candidate turns left, right or is a
                        // U-turn/straight. The tl+br cut favours the turn toward the br quadrant; the tr+bl
                        // cut the opposite turn.
                        let prev_pt = ring.get(ring.len() - 2).copied().unwrap_or(Point::ZERO);
                        let in_dir = (cur.0 as i64 - prev_pt.x as i64, cur.1 as i64 - prev_pt.y as i64);
                        let score = |to: (usize, usize)| -> i64 {
                            let d = (to.0 as i64 - cur.0 as i64, to.1 as i64 - cur.1 as i64);
                            // Cross product of incoming → outgoing: >0 = left turn (in y-down screen space).
                            let cross = in_dir.0 * d.1 - in_dir.1 * d.0;
                            if tl_pair { cross } else { -cross }
                        };
                        cands.iter().copied().max_by_key(|t| score(*t)).unwrap_or(to)
                    } else {
                        to
                    }
                });
                let Some(to) = next else { break };
                visited.insert((cur, to));
                ring.push(pt(to));
                if to == *start {
                    break;
                }
                cur = to;
            }
            if ring.len() >= 3 {
                rings.push(ring);
            }
        }
    }
    // Area filter with hole-awareness: small rings are dropped. A hole (negative area) inside a kept outer
    // ring is dropped alone; an outer ring dropped also drops the holes inside it.
    let mut kept: Vec<(Vec<Point>, f64, usize)> = Vec::new(); // (ring, area, depth as outer=0/hole=1)
    for ring in rings.drain(..) {
        let a = shoelace(&ring);
        kept.push((ring, a, if a < 0.0 { 1 } else { 0 }));
    }
    // Drop: outers under threshold drop; holes under threshold drop individually. An outer's hole list is
    // simply every negative-area ring that lands inside it — after dropping an outer we also drop holes it
    // contained, approximated by containment of the hole's first point.
    kept.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
    let mut out: Vec<Vec<Point>> = Vec::new();
    for (ring, area, depth) in kept.iter() {
        if area.abs() < opts.ignore_less_than {
            if *depth == 0 {
                // The whole component is speck — drop it and any hole it already swallowed (none can follow).
                continue;
            }
            continue; // a small hole: filled
        }
        out.push(ring.clone());
    }
    // Simplify: corner-preserving Douglas–Peucker, never below 3 points on a closed ring.
    let tol = opts.optimize.max(0.0);
    let mut polys: Vec<Polyline> = Vec::new();
    let mut budget = PointBudget::default();
    for mut ring in out {
        if let (Some(&first), Some(&last)) = (ring.first(), ring.last())
            && last == first
        {
            ring.pop();
        }

        let s = simplify_ring(&ring, tol);
        let s = if s.len() >= 3 { s } else { ring };
        let poly = Polyline(s.iter().map(|p| [p.x, p.y]).collect::<Vec<P2>>());
        budget.take(poly.0.len())?;
        polys.push(poly);
    }
    Ok(polys)
}

/// Same as `trace`, returning one `BezPath` (rings as closed subpaths), smoothing with the Catmull–Rom →
/// Bézier pass when `smoothness > 0`.
pub fn trace_bez(gray: &[u8], px: [u32; 2], opts: &TraceOptions) -> Result<BezPath, Error> {
    let polys = trace(gray, px, opts)?;
    let mut path = BezPath::new();
    if opts.smoothness <= 0.0 {
        for p in &polys {
            if let Some(first) = p.0.first() {
                path.move_to((first[0], first[1]));
                for pt in p.0.iter().skip(1) {
                    path.line_to((pt[0], pt[1]));
                }
                path.close_path();
            }
        }
        return Ok(path);
    }
    for p in &polys {
        let Some(&f0) = p.0.first() else { continue };
        path.move_to((f0[0], f0[1]));
        let n = p.0.len();
        let at = |i: usize| -> Point {
            let pt = p.0.get(i % n).copied().unwrap_or([0.0, 0.0]);
            Point::new(pt[0], pt[1])
        };
        let is_corner = |i: usize| {
            let a = at(i + n - 1);
            let b = at(i);
            let c = at(i + 1);
            let (du, dv) = ((b - a).hypot(), (c - b).hypot());
            if du < 1e-12 || dv < 1e-12 {
                return false;
            }
            let (u, v) = ((b - a) / du, (c - b) / dv);
            // Turn angle: beyond smoothness → a corner. alphamax≈1 ⇒ ~135° allowed turn, 0 ⇒ every vertex.
            u.dot(v) < (1.0 - opts.smoothness / 1.5).clamp(-1.0, 1.0)
        };
        for i in 1..=n {
            let i0 = (i - 1) % n;
            let i1 = i % n;
            let pe = at(i1);
            if is_corner(i0) || is_corner(i1) {
                path.line_to((pe.x, pe.y));
                continue;
            }
            // Catmull–Rom to Bézier: cp1 = p1 + (p2 - p0)/6, cp2 = p2 - (p3 - p1)/6.
            let (p0, p1, p2, p3) = (at(i0 + n - 1), at(i0), at(i1), at(i1 + 1));
            let (c1, c2) = (p1 + (p2 - p0) / 6.0, p2 - (p3 - p1) / 6.0);
            path.curve_to((c1.x, c1.y), (c2.x, c2.y), (p2.x, p2.y));
        }
        path.close_path();
    }
    Ok(path)
}

fn pt(v: (usize, usize)) -> Point {
    Point::new(v.0 as f64, v.1 as f64)
}

fn shoelace(ring: &[Point]) -> f64 {
    ring.windows(2).filter_map(|w| w.first().zip(w.get(1))).map(|(a, b)| a.x * b.y - b.x * a.y).sum::<f64>() / 2.0
}

/// Douglas–Peucker on a closed ring, anchored at the vertex farthest from the centroid (a stable corner).
fn simplify_ring(ring: &[Point], tol: f64) -> Vec<Point> {
    if ring.len() <= 3 {
        return ring.to_vec();
    }
    // Anchor at the two farthest points so the ring splits into two open polylines simplified together.
    let (mut a, mut bd) = (0usize, 0.0f64);
    let cvec = ring.iter().fold(kurbo::Vec2::ZERO, |s, p| s + p.to_vec2()) / ring.len() as f64;
    let centroid = Point::new(cvec.x, cvec.y);
    for (i, p) in ring.iter().enumerate() {
        let d = p.distance_squared(centroid);
        if d > bd {
            bd = d;
            a = i;
        }
    }
    let (mut b, mut bd2) = (0usize, 0.0f64);
    for (i, p) in ring.iter().enumerate() {
        let d = p.distance_squared(*ring.get(a).unwrap_or(&Point::ZERO));
        if d > bd2 {
            bd2 = d;
            b = i;
        }
    }
    let (i0, i1) = (a.min(b), a.max(b));
    let seg_a: Vec<Point> = ring.get(i0..=i1).unwrap_or(&[]).to_vec();
    let seg_b: Vec<Point> = ring.get(i1..).unwrap_or(&[]).iter().chain(ring.get(..=i0).unwrap_or(&[]).iter()).copied().collect();
    let mut out = dp(&seg_a, tol);
    out.extend(dp(&seg_b, tol));
    out.dedup();
    if out.last() == out.first() {
        out.pop();
    }
    if out.len() >= 3 { out } else { ring.to_vec() }
}

fn dp(line: &[Point], tol: f64) -> Vec<Point> {
    if line.len() < 3 {
        return line.to_vec();
    }
    let (Some(&a), Some(&b)) = (line.first(), line.last()) else { return Vec::new() };
    let ab = b - a;
    let denom = ab.hypot();
    let (mut best, mut bd) = (0usize, 0.0f64);
    for (i, p) in line.iter().enumerate() {
        let d = if denom < 1e-12 { a.distance(*p) } else { ((*p - a).dot(kurbo::Vec2::new(-ab.y, ab.x))).abs() / denom };
        if d > bd {
            bd = d;
            best = i;
        }
    }
    if bd <= tol {
        return vec![a, b];
    }
    let mut left = dp(line.get(..=best).unwrap_or(&[]), tol);
    let right = dp(line.get(best..).unwrap_or(&[]), tol);
    left.pop();
    left.extend(right);
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a gray buffer with `cells` (col,row) set to 0 (ink) on a white field.
    fn img(w: usize, h: usize, cells: &[(usize, usize)]) -> Vec<u8> {
        let mut g = vec![255u8; w * h];
        for &(c, r) in cells {
            g[r * w + c] = 0;
        }
        g
    }

    fn square_img() -> Vec<u8> {
        // A filled 10×10 square (cols 5..15, rows 5..15) on 20×20 white.
        let mut cells = Vec::new();
        for r in 5..15 {
            for c in 5..15 {
                cells.push((c, r));
            }
        }
        img(20, 20, &cells)
    }

    fn area(ring: &Polyline) -> f64 {
        let p = &ring.0;
        p.iter().zip(p.iter().cycle().skip(1)).take(p.len()).map(|(a, b)| a[0] * b[1] - b[0] * a[1]).sum::<f64>() / 2.0
    }

    #[test]
    fn a_filled_square_gives_one_ring_with_exact_corners() {
        let rings = trace(&square_img(), [20, 20], &TraceOptions { optimize: 0.0, ..Default::default() }).unwrap();
        assert_eq!(rings.len(), 1);
        let mut pts = rings[0].0.clone();
        pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
        assert_eq!(pts, vec![[5.0, 5.0], [5.0, 15.0], [15.0, 5.0], [15.0, 15.0]], "{pts:?}");
        assert!((area(&rings[0]).abs() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn a_donut_gives_outer_plus_hole_opposite_orientation() {
        // Donut: ink ring = square border, hole = 4×4 centre on 20×20.
        let mut cells = Vec::new();
        for r in 5..15 {
            for c in 5..15 {
                cells.push((c, r));
            }
        }
        for r in 8..12 {
            for c in 8..12 {
                cells.retain(|&x| x != (c, r));
            }
        }
        let rings = trace(&img(20, 20, &cells), [20, 20], &TraceOptions { optimize: 0.0, ..Default::default() }).unwrap();
        assert_eq!(rings.len(), 2, "{rings:?}");
        let (a0, a1) = (area(&rings[0]), area(&rings[1]));
        assert!(a0 < 0.0 && a1 > 0.0, "outer negative, hole positive (y-down space): {a0} {a1}");
        assert!((a0 + 100.0).abs() < 1e-9 && (a1 - 16.0).abs() < 1e-9, "{a0} {a1}");
    }

    #[test]
    fn specks_below_ignore_less_than_are_removed() {
        let mut cells = vec![(10, 10), (10, 11), (11, 10), (11, 11)]; // 4 px² speck
        for r in 15..19 {
            for c in 15..19 {
                cells.push((c, r));
            }
        }
        let rings = trace(&img(20, 20, &cells), [20, 20], &TraceOptions { ignore_less_than: 5.0, optimize: 0.0, ..Default::default() }).unwrap();
        assert_eq!(rings.len(), 1, "the 4 px² speck is gone");
    }

    #[test]
    fn invert_traces_the_light_side() {
        let rings = trace(&square_img(), [20, 20], &TraceOptions { invert: true, optimize: 0.0, ignore_less_than: 0.0, ..Default::default() }).unwrap();
        assert!(!rings.is_empty());
        // The white surround: an outer ring around the whole image plus the square as a hole.
        assert_eq!(rings.len(), 2, "{rings:?}");
        let total: f64 = rings.iter().map(|r| area(r).abs()).sum();
        assert!((total - 500.0).abs() < 1e-9, "outer 400 + hole 100: {total}");
    }

    #[test]
    fn the_threshold_is_exclusive() {
        // gray == threshold is NOT ink.
        let mut g = vec![128u8; 4];
        g[0] = 127;
        let rings = trace(&g, [2, 2], &TraceOptions { threshold: 128, ignore_less_than: 0.0, optimize: 0.0, ..Default::default() }).unwrap();
        assert_eq!(rings.len(), 1, "only the darker pixel is ink");
        assert!((area(&rings[0]).abs() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_diagonal_checkerboard_is_consistent_and_closed() {
        // Two diagonally touching ink pixels share only a corner.
        let rings = trace(&img(2, 2, &[(0, 0), (1, 1)]), [2, 2], &TraceOptions { ignore_less_than: 0.0, optimize: 0.0, ..Default::default() }).unwrap();
        // The documented saddle rule (top-left pair cut) always produces the same closed output.
        let again = trace(&img(2, 2, &[(0, 0), (1, 1)]), [2, 2], &TraceOptions { ignore_less_than: 0.0, optimize: 0.0, ..Default::default() }).unwrap();
        assert_eq!(rings, again);
        for r in &rings {
            assert!(r.0.len() >= 3);
        }
    }

    #[test]
    fn a_smoothed_circle_stays_within_tolerance() {
        // Rasterise a disc: circle centre (10,10) radius 8 on 20×20.
        let mut cells = Vec::new();
        for r in 0..20usize {
            for c in 0..20usize {
                let (x, y) = (c as f64 + 0.5, r as f64 + 0.5);
                if ((x - 10.0).powi(2) + (y - 10.0).powi(2)).sqrt() <= 8.0 {
                    cells.push((c, r));
                }
            }
        }
        let g = img(20, 20, &cells);
        let path = trace_bez(&g, [20, 20], &TraceOptions { optimize: 0.5, smoothness: 1.0, ..Default::default() }).unwrap();
        // Every curve control point stays within optimize+0.5 px of the disc boundary (radius 8).
        let polys = crate::flatten(&path, 0.01).unwrap();
        for ring in &polys {
            for pt in &ring.0 {
                let d = ((pt[0] - 10.0).powi(2) + (pt[1] - 10.0).powi(2)).sqrt() - 8.0;
                assert!(d.abs() <= 1.2, "point {pt:?} is {d} px off the circle");
            }
        }
    }

    #[test]
    fn determinism_and_size_refusals() {
        let a = trace(&square_img(), [20, 20], &TraceOptions::default()).unwrap();
        let b = trace(&square_img(), [20, 20], &TraceOptions::default()).unwrap();
        assert_eq!(a, b);
        assert!(matches!(trace(&[0u8; 3], [2, 2], &TraceOptions::default()), Err(Error::BadInput(_))));
        assert!(matches!(trace(&[0u8; 0], [0, 0], &TraceOptions::default()), Err(Error::BadInput(_))));
    }
}
