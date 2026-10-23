//! Small geometry helpers the commands need on top of `doc`/`geom`: selection bounds, applying a transform to
//! objects, duplicating with fresh ids, closing near-closed subpaths.

use std::sync::Arc;

use openlaser_doc::{Geom, Project, Shape};
use openlaser_geom::{Affine, BezPath, PathEl, Point, Polyline};
use openlaser_types::{Bounds, ObjectId};

fn extend(acc: &mut Option<Bounds>, p: Point) {
    if !(p.x.is_finite() && p.y.is_finite()) {
        return;
    }
    *acc = Some(match *acc {
        None => Bounds { min: [p.x, p.y], max: [p.x, p.y] },
        Some(b) => Bounds { min: [b.min[0].min(p.x), b.min[1].min(p.y)], max: [b.max[0].max(p.x), b.max[1].max(p.y)] },
    });
}

fn el_points(el: &PathEl) -> Vec<Point> {
    match *el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => vec![p],
        PathEl::QuadTo(a, b) => vec![a, b],
        PathEl::CurveTo(a, b, c) => vec![a, b, c],
        PathEl::ClosePath => Vec::new(),
    }
}

/// Object-space extent of a parametric shape (centred on its origin), used when its path is not available.
fn parametric_corners(g: &Geom) -> Vec<Point> {
    let (hx, hy) = match g {
        Geom::Rect { w, h, .. } => (w.get() / 2.0, h.get() / 2.0),
        Geom::Ellipse { rx, ry } => (rx.get(), ry.get()),
        Geom::Polygon { radius, .. } => (radius.get(), radius.get()),
        Geom::Bitmap(b) => (b.size_mm[0] / 2.0, b.size_mm[1] / 2.0),
        Geom::Path(_) | Geom::Group(_) => return Vec::new(),
    };
    vec![Point::new(-hx, -hy), Point::new(hx, -hy), Point::new(hx, hy), Point::new(-hx, hy)]
}

fn control_bounds(shape: &Shape, parent: Affine, acc: &mut Option<Bounds>) {
    let xf = parent * shape.xform;
    match &shape.geom {
        Geom::Group(children) => {
            for c in children {
                control_bounds(c, xf, acc);
            }
        }
        Geom::Path(p) => {
            for el in p.elements() {
                for pt in el_points(el) {
                    extend(acc, xf * pt);
                }
            }
        }
        other => {
            let bez = other.to_bez();
            let pts: Vec<Point> = bez.elements().iter().flat_map(el_points).collect();
            let pts = if pts.is_empty() { parametric_corners(other) } else { pts };
            for pt in pts {
                extend(acc, xf * pt);
            }
        }
    }
}

/// The extent of one shape under its own transform, from its path or parametric size (no model call).
pub(crate) fn shape_extent(shape: &Shape) -> Option<Bounds> {
    let mut acc = None;
    control_bounds(shape, Affine::IDENTITY, &mut acc);
    acc
}

/// Every coordinate of a path element is finite.
pub(crate) fn el_finite(el: &PathEl) -> bool {
    el_points(el).iter().all(|p| p.x.is_finite() && p.y.is_finite())
}

/// Design-mm bounds of objects: the model's exact bounds, else their flattened polylines, else their control
/// points (a conservative hull for curves).
pub(crate) fn bounds(project: &Project, ids: &[ObjectId], tol_mm: f64) -> Option<Bounds> {
    if let Some(b) = project.bounds(ids) {
        return Some(b);
    }
    let polys: Vec<Polyline> = ids.iter().flat_map(|id| project.world_polylines(*id, tol_mm)).collect();
    if let Some(b) = openlaser_geom::bounds_of(&polys) {
        return Some(b);
    }
    let mut acc = None;
    for id in ids {
        if let Some(s) = project.shape(*id) {
            control_bounds(s, Affine::IDENTITY, &mut acc);
        }
    }
    acc
}

/// Pre-multiplies `m` onto the transform of every unlocked object in `ids`; returns how many moved.
pub(crate) fn apply(project: &mut Project, ids: &[ObjectId], m: Affine) -> usize {
    let mut n = 0;
    for arc in &mut project.shapes {
        if ids.contains(&arc.id) && !arc.locked {
            let mut s = (**arc).clone();
            s.xform = m * s.xform;
            *arc = Arc::new(s);
            n += 1;
        }
    }
    n
}

/// A copy of `shape` (groups deep) with fresh ids from `project`.
pub(crate) fn duplicate(project: &mut Project, shape: &Shape) -> Shape {
    let mut s = shape.clone();
    s.id = project.alloc_id();
    if let Geom::Group(children) = &shape.geom {
        let copies: Vec<Arc<Shape>> = children.iter().map(|c| Arc::new(duplicate(project, c))).collect();
        s.geom = Geom::Group(copies);
    }
    s
}

/// Gives `shape` (groups deep) fresh ids from `project` (imported shapes arrive with id 0).
pub(crate) fn assign_ids(project: &mut Project, shape: Shape) -> Shape {
    let mut s = shape;
    s.id = project.alloc_id();
    if let Geom::Group(children) = &s.geom {
        let fresh: Vec<Arc<Shape>> = children.iter().map(|c| Arc::new(assign_ids(project, (**c).clone()))).collect();
        s.geom = Geom::Group(fresh);
    }
    s
}

/// Sets the layer of `shape` and of every child.
pub(crate) fn set_layer(shape: &Shape, layer: openlaser_types::LayerKey) -> Shape {
    let mut s = shape.clone();
    s.layer = layer;
    if let Geom::Group(children) = &shape.geom {
        s.geom = Geom::Group(children.iter().map(|c| Arc::new(set_layer(c, layer))).collect());
    }
    s
}

/// Closes every open subpath whose ends lie within `tol_mm` of each other in design mm. Returns the new path and
/// how many subpaths it closed.
pub(crate) fn close_subpaths(path: &BezPath, xform: Affine, tol_mm: f64) -> (BezPath, usize) {
    let els = path.elements();
    let mut out: Vec<PathEl> = Vec::with_capacity(els.len() + 4);
    let mut closed = 0;
    let mut start: Option<Point> = None;
    let mut last: Option<Point> = None;
    let mut drawn = 0usize;
    let finish = |out: &mut Vec<PathEl>, start: Option<Point>, last: Option<Point>, drawn: usize, closed: &mut usize| {
        if let (Some(a), Some(b)) = (start, last)
            && drawn > 0
            && (xform * a).distance(xform * b) <= tol_mm
        {
            out.push(PathEl::ClosePath);
            *closed += 1;
        }
    };
    for el in els {
        match *el {
            PathEl::MoveTo(p) => {
                finish(&mut out, start, last, drawn, &mut closed);
                start = Some(p);
                last = Some(p);
                drawn = 0;
                out.push(*el);
            }
            PathEl::ClosePath => {
                // Already closed: nothing to finish.
                start = None;
                last = None;
                drawn = 0;
                out.push(*el);
            }
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => {
                last = Some(p);
                drawn += 1;
                out.push(*el);
            }
        }
    }
    finish(&mut out, start, last, drawn, &mut closed);
    (BezPath::from_vec(out), closed)
}

/// Number of subpaths and whether all are closed (by ClosePath), for `doc_inspect` items without flattening.
pub(crate) fn path_stats(project: &Project, id: ObjectId, tol_mm: f64) -> (usize, bool) {
    let polys = project.world_polylines(id, tol_mm);
    if !polys.is_empty() {
        let closed = polys.iter().all(|p| p.is_closed(openlaser_geom::CLOSURE_TOL_MM));
        return (polys.len(), closed);
    }
    let Some(s) = project.shape(id) else { return (0, false) };
    match &s.geom {
        Geom::Path(p) => {
            let subpaths = p.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(_))).count();
            let closes = p.elements().iter().filter(|e| matches!(e, PathEl::ClosePath)).count();
            (subpaths, subpaths > 0 && closes >= subpaths)
        }
        Geom::Rect { .. } | Geom::Ellipse { .. } | Geom::Polygon { .. } | Geom::Bitmap(_) => (1, true),
        Geom::Group(c) => (c.len(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(pts: &[(f64, f64)], close: bool) -> BezPath {
        let mut p = BezPath::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 {
                p.move_to((*x, *y));
            } else {
                p.line_to((*x, *y));
            }
        }
        if close {
            p.close_path();
        }
        p
    }

    #[test]
    fn close_subpaths_closes_only_near_ends() {
        let mut p = path(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 0.3)], false);
        p.extend(path(&[(20.0, 0.0), (30.0, 0.0)], false).elements().iter().copied());
        p.extend(path(&[(40.0, 0.0), (50.0, 0.0), (40.0, 0.0)], true).elements().iter().copied());
        let (q, n) = close_subpaths(&p, Affine::IDENTITY, 0.5);
        assert_eq!(n, 1);
        assert_eq!(q.elements().iter().filter(|e| matches!(e, PathEl::ClosePath)).count(), 2);
        let (_, n) = close_subpaths(&p, Affine::scale(10.0), 0.5);
        assert_eq!(n, 0, "tolerance is in design mm, after the transform");
    }

    #[test]
    fn control_bounds_follow_transforms() {
        let s = Shape {
            id: ObjectId(1),
            name: None,
            layer: openlaser_types::LayerKey::Color(0),
            xform: Affine::translate((5.0, 5.0)),
            geom: Geom::Path(Arc::new(path(&[(0.0, 0.0), (10.0, 2.0)], false))),
            locked: false,
            power_scale: openlaser_types::Percent::new(100.0).unwrap(),
            priority: 0,
        };
        let mut acc = None;
        control_bounds(&s, Affine::IDENTITY, &mut acc);
        assert_eq!(acc, Some(Bounds { min: [5.0, 5.0], max: [15.0, 7.0] }));
    }
}
