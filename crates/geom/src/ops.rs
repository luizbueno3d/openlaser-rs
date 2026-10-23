//! Flattening and path clean-up (OM `geometry.py` port). WP0 stubs return neutral values.

use crate::{BezPath, Bounds, Polyline};

/// One polyline per subpath; ClosePath repeats the first point. Tolerance in mm.
pub fn flatten(path: &BezPath, tol_mm: f64) -> Vec<Polyline> {
    let _ = (path, tol_mm);
    Vec::new()
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

/// Auto-Join (OM `geometry.join_lines` port): chains ends closer than tol.
pub fn join_lines(polys: &[Polyline], tol_mm: f64) -> Vec<Polyline> {
    let _ = (polys, tol_mm);
    Vec::new()
}

/// Close Path: closes open paths whose ends are within tol; returns how many were closed.
pub fn close_paths(polys: &[Polyline], tol_mm: f64) -> (Vec<Polyline>, usize) {
    let _ = (polys, tol_mm);
    (Vec::new(), 0)
}

/// Delete Duplicates: indices of polylines equal (within tol, either direction) to an earlier one.
pub fn duplicates(polys: &[Polyline], tol_mm: f64) -> Vec<usize> {
    let _ = (polys, tol_mm);
    Vec::new()
}
