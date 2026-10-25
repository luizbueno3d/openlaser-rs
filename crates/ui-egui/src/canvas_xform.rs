//! Selection-drag math for the canvas (UI §5): handle positions, live scale/rotate previews and the single
//! command each drag commits. Pure `[f64; 2]`/[`Bounds`] math so the kittests and unit tests exercise it
//! without egui.

use openlaser_geom::{Affine, Vec2};
use openlaser_types::Bounds;

/// Smallest side a scale drag can produce (mm); a handle never crosses the opposite side.
pub(crate) const MIN_SIDE: f64 = 0.1;

/// Handle order: the eight points of the bounding box, clockwise from the top-left (world Y up).
/// Corners are even, edge midpoints odd.
pub(crate) const CORNERS: [usize; 4] = [0, 2, 4, 6];

/// Grid position (col, row) of each handle in a 3×3 layout (row 0 = the top, world max-y).
const GRID: [(u8, u8); 8] = [(0, 0), (1, 0), (2, 0), (2, 1), (2, 2), (1, 2), (0, 2), (0, 1)];

/// `AnchorP` serde names for the nine grid positions (index 4 = "center" is a corner-free handle's opposite
/// never used; corners are even, edges odd).
const ANCHORS: [[&str; 3]; 3] = [["topLeft", "top", "topRight"], ["left", "center", "right"], ["bottomLeft", "bottom", "bottomRight"]];

/// The world point of handle `i` on `b`.
pub(crate) fn handle_point(b: &Bounds, i: usize) -> [f64; 2] {
    let Some(&(col, row)) = GRID.get(i) else { return b.min };
    let (w, h) = (b.max[0] - b.min[0], b.max[1] - b.min[1]);
    [b.min[0] + w * f64::from(col) / 2.0, b.max[1] - h * f64::from(row) / 2.0]
}

/// The handle opposite `i` on the box (corners ↔ corners, edges ↔ edges).
pub(crate) fn opposite(i: usize) -> usize {
    (i + 4) % 8
}

/// The `AnchorP` name the engine expects for the handle `i` sits at.
pub(crate) fn anchor_of(i: usize) -> &'static str {
    let Some(&(col, row)) = GRID.get(i) else { return "center" };
    ANCHORS.get(row as usize).and_then(|r| r.get(col as usize)).copied().unwrap_or("center")
}

/// The single axis an edge handle moves (corners move both): T/B → y, R/L → x.
pub(crate) fn edge_axis(i: usize) -> Option<usize> {
    match i {
        1 | 5 => Some(1),
        3 | 7 => Some(0),
        _ => None,
    }
}

/// Which side of the box handle `i` moves on each axis: `Some(false)` = the min side, `Some(true)` = the max
/// side, `None` = the handle does not move that axis (edge midpoints move one axis only).
fn moved_sides(i: usize) -> [Option<bool>; 2] {
    let Some(&(col, row)) = GRID.get(i) else { return [None; 2] };
    [
        match col {
            0 => Some(false),
            2 => Some(true),
            _ => None,
        },
        match row {
            0 => Some(true),
            2 => Some(false),
            _ => None,
        },
    ]
}

/// The bounds after dragging handle `i` of `b` by `delta` (world mm). `lock` keeps the aspect ratio on corner
/// drags; `alt` scales about the centre instead of the opposite handle. No side ever crosses: the dragged
/// size holds at [`MIN_SIDE`]. Returns `b` unchanged for a zero-size box (there is nothing to scale).
pub(crate) fn scale_bounds(b: &Bounds, i: usize, delta: [f64; 2], lock: bool, alt: bool) -> Bounds {
    let sides = moved_sides(i);
    if sides.iter().all(Option::is_none) || (b.max[0] - b.min[0]) <= 0.0 || (b.max[1] - b.min[1]) <= 0.0 {
        return *b;
    }
    let centre = [(b.min[0] + b.max[0]) / 2.0, (b.min[1] + b.max[1]) / 2.0];
    let from = handle_point(b, i);
    let to = [from[0] + delta[0], from[1] + delta[1]];
    let mut out = *b;
    // Per axis: the new half-size (alt) or the new dragged edge, holding MIN_SIDE.
    let mut half: [Option<f64>; 2] = [None; 2];
    for (axis, side) in sides.iter().enumerate() {
        let (Some(t), Some(c)) = (to.get(axis), centre.get(axis)) else { continue };
        match side {
            None => {}
            Some(_) if alt => {
                if let Some(h) = half.get_mut(axis) {
                    *h = Some((t - c).abs().max(MIN_SIDE / 2.0));
                }
            }
            Some(true) => {
                if let (Some(max), Some(min)) = (out.max.get_mut(axis), out.min.get(axis)) {
                    *max = t.max(min + MIN_SIDE);
                }
            }
            Some(false) => {
                if let (Some(min), Some(max)) = (out.min.get_mut(axis), out.max.get(axis)) {
                    *min = t.min(max - MIN_SIDE);
                }
            }
        }
    }
    if lock && sides.iter().all(Option::is_some) {
        // Corner drag: keep the ratio with the stronger of the two factors.
        let (sx, sy) = if alt {
            (half[0].unwrap_or(0.0) / ((b.max[0] - b.min[0]) / 2.0), half[1].unwrap_or(0.0) / ((b.max[1] - b.min[1]) / 2.0))
        } else {
            ((out.max[0] - out.min[0]) / (b.max[0] - b.min[0]), (out.max[1] - out.min[1]) / (b.max[1] - b.min[1]))
        };
        let s = sx.max(sy).max(MIN_SIDE / (b.max[0] - b.min[0]).max(MIN_SIDE));
        if alt {
            for (axis, h) in half.iter_mut().enumerate() {
                if let (Some(max), Some(min)) = (b.max.get(axis), b.min.get(axis)) {
                    *h = Some((max - min) / 2.0 * s);
                }
            }
        } else {
            for (axis, side) in sides.iter().enumerate() {
                let (Some(bmin), Some(bmax)) = (b.min.get(axis), b.max.get(axis)) else { continue };
                let size = (bmax - bmin) * s;
                match side {
                    Some(true) => {
                        if let Some(max) = out.max.get_mut(axis) {
                            *max = bmin + size;
                        }
                    }
                    Some(false) => {
                        if let Some(min) = out.min.get_mut(axis) {
                            *min = bmax - size;
                        }
                    }
                    None => {}
                }
            }
        }
    }
    if alt {
        for (axis, h) in half.iter().enumerate() {
            if let (Some(h), Some(c)) = (h, centre.get(axis))
                && let (Some(min), Some(max)) = (out.min.get_mut(axis), out.max.get_mut(axis))
            {
                *min = c - h;
                *max = c + h;
            }
        }
    }
    out
}

/// Rotation of the segment centre→`to` against centre→`from`, degrees counter-clockwise (world Y up),
/// normalised to (−180, 180]; `snap` rounds to whole multiples of `snap` degrees (0 = off).
pub(crate) fn rotate_deg(centre: [f64; 2], from: [f64; 2], to: [f64; 2], snap: f64) -> f64 {
    let a0 = (from[1] - centre[1]).atan2(from[0] - centre[0]);
    let a1 = (to[1] - centre[1]).atan2(to[0] - centre[0]);
    let mut deg = (a1 - a0).to_degrees();
    while deg <= -180.0 {
        deg += 360.0;
    }
    while deg > 180.0 {
        deg -= 360.0;
    }
    if snap > 0.0 {
        deg = (deg / snap).round() * snap;
    }
    deg
}

/// The affine a move drag commits.
pub(crate) fn move_affine(delta: [f64; 2]) -> Affine {
    Affine::translate(Vec2::new(delta[0], delta[1]))
}

/// The affine `arrange.transform {anchor, wMm, hMm}` commits for a scale from `old` to `new` about the
/// handle-`i` opposite anchor (or the centre with `alt`).
pub(crate) fn scale_affine(old: &Bounds, new: &Bounds, i: usize, alt: bool) -> Affine {
    let pivot = if alt { [(old.min[0] + old.max[0]) / 2.0, (old.min[1] + old.max[1]) / 2.0] } else { handle_point(old, opposite(i)) };
    let sx = if old.max[0] > old.min[0] { (new.max[0] - new.min[0]) / (old.max[0] - old.min[0]) } else { 1.0 };
    let sy = if old.max[1] > old.min[1] { (new.max[1] - new.min[1]) / (old.max[1] - old.min[1]) } else { 1.0 };
    Affine::translate(Vec2::new(pivot[0], pivot[1])) * Affine::scale_non_uniform(sx, sy) * Affine::translate(Vec2::new(-pivot[0], -pivot[1]))
}

/// The affine `arrange.rotateBy {deg, anchor: "center"}` commits.
pub(crate) fn rotate_affine(b: &Bounds, deg: f64) -> Affine {
    let c = [(b.min[0] + b.max[0]) / 2.0, (b.min[1] + b.max[1]) / 2.0];
    Affine::translate(Vec2::new(c[0], c[1])) * Affine::rotate(deg.to_radians()) * Affine::translate(Vec2::new(-c[0], -c[1]))
}

/// The box `affine` produces of `b`, for the ghost (the two dragged corners may not be min/max after a
/// rotation, so all four corners are transformed).
pub(crate) fn bounds_of(b: &Bounds, affine: Affine) -> Bounds {
    let mut out = Bounds { min: [f64::INFINITY; 2], max: [f64::NEG_INFINITY; 2] };
    for (col, row) in GRID.iter().step_by(2).chain([(1, 1)].iter()) {
        let p = affine
            * openlaser_geom::Point::new(b.min[0] + (b.max[0] - b.min[0]) * f64::from(*col) / 2.0, b.max[1] - (b.max[1] - b.min[1]) * f64::from(*row) / 2.0);
        out.min[0] = out.min[0].min(p.x);
        out.max[0] = out.max[0].max(p.x);
        out.min[1] = out.min[1].min(p.y);
        out.max[1] = out.max[1].max(p.y);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x0: f64, y0: f64, x1: f64, y1: f64) -> Bounds {
        Bounds { min: [x0, y0], max: [x1, y1] }
    }

    #[test]
    fn corner_drag_scales_from_the_opposite_corner() {
        // Bottom-right handle (4) dragged +2/-3: the top-left corner (min.x, max.y) stays put.
        let out = scale_bounds(&b(0.0, 0.0, 10.0, 10.0), 4, [2.0, -3.0], false, false);
        assert_eq!(out.min, [0.0, -3.0]);
        assert_eq!(out.max, [12.0, 10.0]);
        // Top-left handle (0) dragged: bottom-right stays put.
        let out = scale_bounds(&b(0.0, 0.0, 10.0, 10.0), 0, [-5.0, 5.0], false, false);
        assert_eq!(out.min, [-5.0, 0.0]);
        assert_eq!(out.max, [10.0, 15.0]);
    }

    #[test]
    fn edge_drag_moves_one_axis_only() {
        let out = scale_bounds(&b(0.0, 0.0, 10.0, 10.0), 3, [4.0, 9.0], false, false);
        assert_eq!(out.max, [14.0, 10.0]);
        assert_eq!(out.min, [0.0, 0.0]);
        let out = scale_bounds(&b(0.0, 0.0, 10.0, 10.0), 5, [4.0, -6.0], false, false);
        assert_eq!(out.min, [0.0, -6.0]);
    }

    #[test]
    fn a_handle_can_never_cross_the_opposite_side() {
        // BR dragged left past the min-x side and up past the max-y side: both axes hold at MIN_SIDE.
        let out = scale_bounds(&b(0.0, 0.0, 10.0, 10.0), 4, [-20.0, 20.0], false, false);
        assert!((out.max[0] - out.min[0] - MIN_SIDE).abs() < 1e-9 && (out.max[1] - out.min[1] - MIN_SIDE).abs() < 1e-9, "{out:?}");
        // Dragging the dragged side further out is free: down stays unclamped.
        let out = scale_bounds(&b(0.0, 0.0, 10.0, 10.0), 4, [-20.0, -20.0], false, false);
        assert_eq!(out.min[1], -20.0);
        let out = scale_bounds(&b(0.0, 0.0, 10.0, 10.0), 0, [50.0, -50.0], false, false);
        assert!((out.max[0] - out.min[0] - MIN_SIDE).abs() < 1e-9);
    }

    #[test]
    fn lock_keeps_the_ratio_and_alt_scales_about_the_centre() {
        let out = scale_bounds(&b(0.0, 0.0, 10.0, 5.0), 4, [10.0, 0.0], true, false);
        assert!(((out.max[0] - out.min[0]) / (out.max[1] - out.min[1]) - 2.0).abs() < 1e-9, "{out:?}");
        // Alt on the BR handle of a square about its centre (5,5): half-extents |to - c| = (15, 5).
        let out = scale_bounds(&b(0.0, 0.0, 10.0, 10.0), 4, [10.0, 10.0], false, true);
        assert_eq!(out.min, [-10.0, 0.0]);
        assert_eq!(out.max, [20.0, 10.0]);
    }

    #[test]
    fn rotate_is_ccw_and_snaps() {
        let c = [0.0, 0.0];
        assert!((rotate_deg(c, [1.0, 0.0], [0.0, 1.0], 0.0) - 90.0).abs() < 1e-9);
        assert!((rotate_deg(c, [1.0, 0.0], [0.0, -1.0], 0.0) + 90.0).abs() < 1e-9);
        assert_eq!(rotate_deg(c, [1.0, 0.0], [0.3, 1.0], 15.0), 75.0);
        // Degenerate press on the centre is 0°, not NaN.
        assert_eq!(rotate_deg(c, [0.0, 0.0], [0.0, 0.0], 0.0), 0.0);
    }

    #[test]
    fn the_scale_affine_matches_the_engine_semantics() {
        // anchor = the opposite handle's fixed point; new bounds land exactly.
        let old = b(1.0, 1.0, 11.0, 6.0);
        let new = scale_bounds(&old, 4, [4.0, -2.0], false, false);
        let m = scale_affine(&old, &new, 4, false);
        let p = m * openlaser_geom::Point::new(1.0, 6.0);
        assert!((p.x - 1.0).abs() < 1e-9 && (p.y - 6.0).abs() < 1e-9, "the top-left anchor stays");
        let p = m * openlaser_geom::Point::new(11.0, 1.0);
        assert!((p.x - new.max[0]).abs() < 1e-9 && (p.y - new.min[1]).abs() < 1e-9);
    }

    #[test]
    fn bounds_of_tracks_the_affine() {
        // A 10×20 box rotated 90° about its own centre (5,10) becomes 20×10 around the same centre.
        let out = bounds_of(&b(0.0, 0.0, 10.0, 20.0), rotate_affine(&b(0.0, 0.0, 10.0, 20.0), 90.0));
        assert!((out.min[0] + 5.0).abs() < 1e-9 && (out.max[0] - 15.0).abs() < 1e-9, "{out:?}");
        assert!((out.min[1] - 5.0).abs() < 1e-9 && (out.max[1] - 15.0).abs() < 1e-9, "{out:?}");
        // The scale affine lands exactly on the new bounds.
        let old = b(0.0, 0.0, 10.0, 10.0);
        let new = scale_bounds(&old, 4, [4.0, -2.0], false, false);
        let out = bounds_of(&old, scale_affine(&old, &new, 4, false));
        for axis in 0..2 {
            assert!((out.min[axis] - new.min[axis]).abs() < 1e-9 && (out.max[axis] - new.max[axis]).abs() < 1e-9, "{out:?}");
        }
    }
}
