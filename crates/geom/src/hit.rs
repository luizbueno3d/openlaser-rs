//! R-tree over polyline segments for canvas picking (world mm).

use std::collections::{BTreeMap, BTreeSet};

use openlaser_types::ids::ObjectId;
use rstar::primitives::{GeomWithData, Line};
use rstar::{AABB, RTree};

use crate::{Bounds, P2, Polyline};

/// Coordinates beyond this magnitude (mm) are not indexed: the R-tree's area arithmetic would overflow. No
/// machine field or bed comes near it.
pub const HIT_MAX_COORD_MM: f64 = 1e12;

type Seg = GeomWithData<Line<P2>, ObjectId>;

/// R-tree over polyline segments for canvas picking (world mm).
///
/// Every polyline contributes its segments; a one-point polyline contributes a zero-length segment so it can
/// still be picked. Segments with a non-finite or out-of-range ([`HIT_MAX_COORD_MM`]) end are not indexed.
#[derive(Debug, Default)]
pub struct HitIndex {
    tree: RTree<Seg>,
    /// Indexed segments per object (for "fully enclosed" selection).
    per_object: BTreeMap<ObjectId, usize>,
}

fn usable(p: &P2) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= HIT_MAX_COORD_MM)
}

fn rect_envelope(r: &Bounds) -> Option<AABB<P2>> {
    if !usable(&r.min) || !usable(&r.max) {
        return None;
    }
    Some(AABB::from_corners(r.min, r.max))
}

/// Liang–Barsky: does segment a→b touch the closed rectangle?
fn segment_touches_rect(a: P2, b: P2, min: P2, max: P2) -> bool {
    let d = [b[0] - a[0], b[1] - a[1]];
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-d[0], a[0] - min[0]), (d[0], max[0] - a[0]), (-d[1], a[1] - min[1]), (d[1], max[1] - a[1])] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

impl HitIndex {
    pub fn build<'a>(items: impl IntoIterator<Item = (ObjectId, &'a [Polyline])>) -> Self {
        let mut segs: Vec<Seg> = Vec::new();
        let mut per_object: BTreeMap<ObjectId, usize> = BTreeMap::new();
        for (id, polys) in items {
            for poly in polys {
                let pts = &poly.0;
                let mut push = |a: P2, b: P2| {
                    if usable(&a) && usable(&b) {
                        segs.push(GeomWithData::new(Line::new(a, b), id));
                        *per_object.entry(id).or_insert(0) += 1;
                    }
                };
                match pts.as_slice() {
                    [] => {}
                    [p] => push(*p, *p),
                    _ => {
                        for w in pts.windows(2) {
                            if let (Some(&a), Some(&b)) = (w.first(), w.get(1)) {
                                push(a, b);
                            }
                        }
                    }
                }
            }
        }
        HitIndex { tree: RTree::bulk_load(segs), per_object }
    }

    /// Nearest segment within tol (ties: the lower id). None for a non-finite point or tolerance.
    pub fn pick(&self, at: P2, tol_mm: f64) -> Option<ObjectId> {
        if !usable(&at) || !(tol_mm.is_finite() && tol_mm >= 0.0) {
            return None;
        }
        let tol2 = tol_mm * tol_mm;
        let mut best: Option<(f64, ObjectId)> = None;
        for (seg, d2) in self.tree.nearest_neighbor_iter_with_distance_2(at) {
            if d2 > tol2 {
                break;
            }
            match best {
                None => best = Some((d2, seg.data)),
                Some((bd, bid)) => {
                    if d2 > bd {
                        break;
                    }
                    if seg.data < bid {
                        best = Some((bd, seg.data));
                    }
                }
            }
        }
        best.map(|(_, id)| id)
    }

    /// crossing=false: fully enclosed. Ascending ids, each once.
    ///
    /// `crossing = true` selects every object with any segment touching the rectangle (LightBurn's
    /// right-to-left drag); `crossing = false` selects objects whose every indexed segment lies inside it.
    pub fn in_rect(&self, r: Bounds, crossing: bool) -> Vec<ObjectId> {
        let Some(env) = rect_envelope(&r) else { return Vec::new() };
        let (min, max) = (env.lower(), env.upper());
        if crossing {
            let ids: BTreeSet<ObjectId> =
                self.tree.locate_in_envelope_intersecting(env).filter(|s| segment_touches_rect(s.geom().from, s.geom().to, min, max)).map(|s| s.data).collect();
            ids.into_iter().collect()
        } else {
            let mut inside: BTreeMap<ObjectId, usize> = BTreeMap::new();
            for s in self.tree.locate_in_envelope(env) {
                *inside.entry(s.data).or_insert(0) += 1;
            }
            inside.into_iter().filter(|(id, n)| self.per_object.get(id) == Some(n)).map(|(id, _)| id).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(cx: f64, cy: f64, h: f64) -> Polyline {
        Polyline(vec![[cx - h, cy - h], [cx + h, cy - h], [cx + h, cy + h], [cx - h, cy + h], [cx - h, cy - h]])
    }

    #[test]
    fn pick_nearest_within_tolerance() {
        let a = [sq(0.0, 0.0, 5.0)];
        let b = [sq(20.0, 0.0, 5.0)];
        let dot = [Polyline(vec![[40.0, 40.0]])];
        let idx = HitIndex::build([(ObjectId(1), &a[..]), (ObjectId(2), &b[..]), (ObjectId(3), &dot[..])]);
        assert_eq!(idx.pick([5.1, 0.0], 0.2), Some(ObjectId(1)));
        assert_eq!(idx.pick([15.05, 0.0], 0.2), Some(ObjectId(2)));
        assert_eq!(idx.pick([10.0, 0.0], 0.2), None);
        assert_eq!(idx.pick([40.0, 40.1], 0.2), Some(ObjectId(3)));
        assert_eq!(idx.pick([f64::NAN, 0.0], 0.2), None);
    }

    #[test]
    fn rect_selection_crossing_and_enclosed() {
        let a = [sq(0.0, 0.0, 5.0)];
        let b = [sq(20.0, 0.0, 5.0)];
        let diag = [Polyline(vec![[-10.0, 20.0], [10.0, 40.0]])];
        let bad = [Polyline(vec![[f64::NAN, 0.0], [1.0, 1.0]])];
        let idx = HitIndex::build([(ObjectId(1), &a[..]), (ObjectId(2), &b[..]), (ObjectId(3), &diag[..]), (ObjectId(4), &bad[..])]);
        let r = Bounds { min: [-6.0, -6.0], max: [16.0, 6.0] };
        assert_eq!(idx.in_rect(r, false), vec![ObjectId(1)]);
        assert_eq!(idx.in_rect(r, true), vec![ObjectId(1), ObjectId(2)]);
        // The diagonal's envelope overlaps this rectangle, the segment does not.
        let corner = Bounds { min: [5.0, 20.0], max: [10.0, 24.0] };
        assert!(idx.in_rect(corner, true).is_empty());
        let nan = Bounds { min: [f64::NAN, 0.0], max: [1.0, 1.0] };
        assert!(idx.in_rect(nan, true).is_empty());
    }
}
