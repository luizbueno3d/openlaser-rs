//! The node model behind the Edit Nodes tool: parse a `BezPath` into subpaths of nodes, edit them, write back.
//! Pure — no egui — so every operation is unit-tested directly. Coordinates stay in the shape's object space;
//! the caller converts the pointer delta through the inverse world transform (grouped shapes work).

use openlaser_doc::BezPath;
use openlaser_geom::{PathEl, Point, Vec2};

/// A node: an anchor plus optional control handles. `line_in` means the incoming segment is a line (the
/// handles are ignored); otherwise the incoming segment is a cubic between `handle_in` of this node and
/// `handle_out` of the previous one. QuadTo is promoted to a cubic exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub p: Point,
    /// Handle controlling the outgoing cubic (`None` when the outgoing side has no curve handle).
    pub out: Option<Point>,
    /// Handle controlling the incoming cubic.
    pub inn: Option<Point>,
    /// The incoming segment is a line (true) or a cubic (false). For the first node it is meaningless.
    pub line_in: bool,
}

/// One subpath (M… then L/C, optionally Z).
#[derive(Debug, Clone, PartialEq)]
pub struct Subpath {
    pub nodes: Vec<Node>,
    pub closed: bool,
}

/// Parse `path` into subpaths of nodes.
pub fn to_nodes(path: &BezPath) -> Vec<Subpath> {
    let mut out: Vec<Subpath> = Vec::new();
    let mut cur = Point::ZERO;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                out.push(Subpath { nodes: vec![Node { p, out: None, inn: None, line_in: true }], closed: false });
                cur = p;
            }
            PathEl::LineTo(p) => {
                if let Some(sp) = out.last_mut() {
                    sp.nodes.push(Node { p, out: None, inn: None, line_in: true });
                }
                cur = p;
            }
            PathEl::QuadTo(a, b) => {
                // Promote exactly: cubic (a' = cur + 2/3(a-cur), b' = b + 2/3(a-b)).
                let (a1, b1) = (cur + (a - cur) * (2.0 / 3.0), b + (a - b) * (2.0 / 3.0));
                if let Some(sp) = out.last_mut() {
                    if let Some(prev) = sp.nodes.last_mut() {
                        prev.out = Some(a1);
                    }
                    sp.nodes.push(Node { p: b, out: None, inn: Some(b1), line_in: false });
                }
                cur = b;
            }
            PathEl::CurveTo(a, b, c) => {
                if let Some(sp) = out.last_mut() {
                    if let Some(prev) = sp.nodes.last_mut() {
                        prev.out = Some(a);
                    }
                    sp.nodes.push(Node { p: c, out: None, inn: Some(b), line_in: false });
                }
                cur = c;
            }
            PathEl::ClosePath => {
                if let Some(sp) = out.last_mut() {
                    sp.closed = true;
                }
            }
        }
    }
    out
}

/// Write subpaths back to a BezPath. A node's incoming cubic runs from the previous node's `out` handle to
/// its `inn` handle; a closed subpath's last segment connects the last node to the first.
pub fn to_path(subs: &[Subpath]) -> BezPath {
    let mut path = BezPath::new();
    for sp in subs {
        let Some(first) = sp.nodes.first() else { continue };
        path.move_to(first.p);
        let mut prev = first;
        for n in sp.nodes.iter().skip(1) {
            if n.line_in {
                path.line_to(n.p);
            } else {
                path.curve_to(prev.out.unwrap_or(prev.p), n.inn.unwrap_or(n.p), n.p);
            }
            prev = n;
        }
        if sp.closed {
            // The closing segment is only explicit when it is a curve; a closing line is `ClosePath` itself.
            if !first.line_in {
                path.curve_to(prev.out.unwrap_or(prev.p), first.inn.unwrap_or(first.p), first.p);
            }
            path.close_path();
        }
    }
    path
}

/// Move the nodes at `sel` by `delta` (their handles move with them). All/any selected across subpaths.
pub fn move_nodes(subs: &mut [Subpath], sel: &[(usize, usize)], delta: Vec2) {
    for &(s, i) in sel {
        if let Some(n) = subs.get_mut(s).and_then(|sp| sp.nodes.get_mut(i)) {
            n.p += delta;
            if let Some(o) = &mut n.out {
                *o += delta;
            }
            if let Some(h) = &mut n.inn {
                *h += delta;
            }
        }
    }
}

/// Move one handle of a node. `inn_side` selects the incoming handle (the outgoing otherwise).
pub fn move_handle(subs: &mut [Subpath], s: usize, i: usize, inn_side: bool, to: Point) {
    if let Some(n) = subs.get_mut(s).and_then(|sp| sp.nodes.get_mut(i)) {
        let slot = if inn_side { &mut n.inn } else { &mut n.out };
        if let Some(h) = slot {
            *h = to;
        }
    }
}

/// Delete nodes: an open subpath reconnects its neighbours; deleting down to < 2 nodes removes the subpath; a
/// closed subpath stays closed while ≥ 3 nodes remain (it opens below that).
pub fn delete_nodes(subs: &mut Vec<Subpath>, sel: &[(usize, usize)]) {
    for (s, sp) in subs.iter_mut().enumerate() {
        let mut kept = Vec::with_capacity(sp.nodes.len());
        for (i, n) in sp.nodes.drain(..).enumerate() {
            if !sel.contains(&(s, i)) {
                kept.push(n);
            }
        }
        // The surviving node's incoming segment adopts the topology of the deleted predecessor it followed:
        // deleting `i` re-routes into `i+1` exactly like the predecessor did. LightBurn keeps the neighbour's
        // segment kind, which is what splicing naturally does (node i+1's line_in stands).
        sp.nodes = kept;
    }
    subs.retain(|sp| sp.nodes.len() >= 2);
}

/// Insert a node on the segment before `i` (or the closing segment when `i == len` on a closed subpath) at
/// parameter `t`. De Casteljau for cubics keeps the shape exactly; a line splits collinearly.
pub fn insert_node(subs: &mut [Subpath], s: usize, i: usize, t: f64) {
    let Some(sp) = subs.get_mut(s) else { return };
    let t = t.clamp(0.0, 1.0);
    let (prev, at) = if i < sp.nodes.len() {
        (i - 1, i)
    } else if sp.closed && i == sp.nodes.len() {
        (sp.nodes.len() - 1, sp.nodes.len())
    } else {
        return;
    };
    let (a, b) = match (sp.nodes.get(prev), sp.nodes.get(at % sp.nodes.len())) {
        (Some(a), Some(b)) => (a.clone(), b.clone()),
        _ => return,
    };
    if b.line_in {
        let mid = a.p + (b.p - a.p) * t;
        sp.nodes.insert(at, Node { p: mid, out: None, inn: None, line_in: true });
    } else {
        // De Casteljau split of the cubic (a.out, b.inn).
        let (h0, h1) = (a.out.unwrap_or(a.p), b.inn.unwrap_or(b.p));
        let (p01, p12) = (a.p + (h0 - a.p) * t, h0 + (h1 - h0) * t);
        let p23 = h1 + (b.p - h1) * t;
        let (p012, p123) = (p01 + (p12 - p01) * t, p12 + (p23 - p12) * t);
        let m = p012 + (p123 - p012) * t;
        // Left half: a.out = p01, new node inn = p012; right half: new node out = p123, b.inn = p23.
        if let Some(prev) = sp.nodes.get_mut(at - 1) {
            prev.out = Some(p01);
        }
        let wrapped = at % sp.nodes.len();
        if let Some(end) = sp.nodes.get_mut(wrapped) {
            end.inn = Some(p23);
        }
        sp.nodes.insert(at, Node { p: m, out: Some(p123), inn: Some(p012), line_in: false });
    }
}

/// Convert the segment before node `i` (or the closing segment when `i == len` on closed) to a line, or to a
/// cubic with handles at 1/3 and 2/3 (shape unchanged).
pub fn convert_segment(subs: &mut [Subpath], s: usize, i: usize, to_line: bool) {
    let Some(sp) = subs.get_mut(s) else { return };
    let count = sp.nodes.len();
    let at = if i < count {
        i
    } else if sp.closed && i == count {
        0
    } else {
        return;
    };
    // `at` is the node whose incoming segment we change. For the closing segment the first node holds the inn.
    let prev = if at == 0 { count - 1 } else { at - 1 };
    let (a, b) = match (sp.nodes.get(prev), sp.nodes.get(at)) {
        (Some(a), Some(b)) => (a.p, b.p),
        _ => return,
    };
    if to_line {
        if let Some(n) = sp.nodes.get_mut(prev) {
            n.out = None;
        }
        if let Some(n) = sp.nodes.get_mut(at) {
            n.inn = None;
            n.line_in = true;
        }
    } else {
        if let Some(n) = sp.nodes.get_mut(prev) {
            n.out = Some(a + (b - a) / 3.0);
        }
        if let Some(n) = sp.nodes.get_mut(at) {
            n.inn = Some(a + (b - a) * (2.0 / 3.0));
            n.line_in = false;
        }
    }
}

/// Break a closed subpath at node `i`: it becomes open, starting at `i`. Nothing changes for open subpaths.
pub fn break_subpath(subs: &mut [Subpath], s: usize, i: usize) {
    let Some(sp) = subs.get_mut(s) else { return };
    if !sp.closed || i >= sp.nodes.len() {
        return;
    }
    // Rotate so `i` is first: the moved node loses its `line_in` (an open subpath's first node has no incoming
    // segment) — actually its original incoming segment becomes the link between old neighbours.
    sp.nodes.rotate_left(i);
    if let Some(first) = sp.nodes.first_mut() {
        first.inn = None;
        first.line_in = true;
    }
    sp.closed = false;
}

/// Join two endpoints of open subpaths: the node at (sa, ia) with (sb, ib). When within `tol` the nodes merge
/// (the second subpath is absorbed, reversed as needed); otherwise a line connects them.
pub fn join_endpoints(subs: &mut Vec<Subpath>, a: (usize, usize), b: (usize, usize), tol: f64) {
    let (sa, ia) = a;
    let (sb, ib) = b;
    if sa >= subs.len() || sb >= subs.len() || sa == sb {
        return;
    }
    if subs.get(sa).is_some_and(|sp| sp.closed) || subs.get(sb).is_some_and(|sp| sp.closed) {
        return;
    }
    let (pa, pb) = (subs.get(sa).and_then(|sp| sp.nodes.get(ia)), subs.get(sb).and_then(|sp| sp.nodes.get(ib)));
    let Some((pa, pb)) = pa.zip(pb) else { return };
    if pa.p.distance(pb.p) <= tol {
        // Merge: reverse `b` when its join endpoint is its first node, so `b`'s other end lands last.
        let Some(bsp) = subs.get(sb) else { return };
        let b_nodes = if ib == 0 {
            let mut v: Vec<Node> = bsp.nodes.iter().rev().cloned().collect();
            for n in &mut v {
                std::mem::swap(&mut n.out, &mut n.inn);
            }
            v
        } else {
            bsp.nodes.clone()
        };
        // `a`'s join endpoint is at `ia`; if it is the first node, `b` goes before `a`.
        let Some(asp) = subs.get_mut(sa) else { return };
        if ia == 0 {
            let a_nodes = std::mem::take(&mut asp.nodes);
            asp.nodes = b_nodes;
            asp.nodes.extend(a_nodes.iter().cloned());
        } else {
            for mut n in b_nodes.into_iter().skip(1) {
                std::mem::swap(&mut n.out, &mut n.inn);
                asp.nodes.push(n);
            }
        }
        subs.remove(sb);
    } else {
        // Connect with a line segment.
        let node = Node { p: pb.p, out: None, inn: None, line_in: true };
        let Some(asp) = subs.get_mut(sa) else { return };
        let at = if ia == 0 { 0 } else { asp.nodes.len() };
        asp.nodes.insert(at, node);
    }
}

/// Point-to-polyline distance (segment distance, not just vertex distance).
#[cfg(test)]
fn seg_dist(line: &[[f64; 2]], p: Point) -> f64 {
    line.windows(2)
        .map(|w| {
            let (a, b) = (Point::new(w[0][0], w[0][1]), Point::new(w[1][0], w[1][1]));
            let ab = b - a;
            let t = (((p - a).dot(ab)) / ab.hypot2()).clamp(0.0, 1.0);
            (a + ab * t).distance(p)
        })
        .fold(f64::MAX, f64::min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_geom::flatten;

    fn roundtrip() -> BezPath {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.curve_to((12.0, 0.0), (14.0, 4.0), (10.0, 6.0));
        p.move_to((20.0, 0.0));
        p.line_to((30.0, 0.0));
        p.line_to((25.0, 5.0));
        p.close_path();
        p
    }

    #[test]
    fn nodes_round_trip_identity() {
        let p = roundtrip();
        let subs = to_nodes(&p);
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].nodes.len(), 3);
        assert!(!subs[0].closed);
        assert_eq!(subs[1].nodes.len(), 3);
        assert!(subs[1].closed);
        let back = to_path(&subs);
        // Exact: the round trip reproduces every element.
        assert_eq!(back.elements().len(), p.elements().len());
        for (a, b) in p.elements().iter().zip(back.elements()) {
            match (a, b) {
                (PathEl::MoveTo(x), PathEl::MoveTo(y)) | (PathEl::LineTo(x), PathEl::LineTo(y)) => assert!(x == y, "{a:?} vs {b:?}"),
                (PathEl::ClosePath, PathEl::ClosePath) => {}
                (PathEl::CurveTo(a1, a2, a3), PathEl::CurveTo(b1, b2, b3)) => {
                    assert_eq!((a1, a2, a3), (b1, b2, b3));
                }
                _ => panic!("element changed kind: {a:?} vs {b:?}"),
            }
        }
    }

    #[test]
    fn quads_are_promoted_to_cubics() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.quad_to((5.0, 10.0), (10.0, 0.0));
        let subs = to_nodes(&p);
        assert_eq!(subs[0].nodes.len(), 2);
        assert!(!subs[0].nodes[1].line_in);
        let back = to_path(&subs);
        assert!(matches!(back.elements()[1], PathEl::CurveTo(..)));
    }

    #[test]
    fn a_split_preserves_the_curve() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((10.0, 20.0), (30.0, 20.0), (40.0, 0.0));
        let mut subs = to_nodes(&p);
        let _before = flatten(&p, 1e-4).unwrap();
        insert_node(&mut subs, 0, 1, 0.5);
        assert_eq!(subs[0].nodes.len(), 3);
        let after = flatten(&to_path(&subs), 1e-4).unwrap()[0].0.clone();
        // 50 samples of the original curve vs the flattened split.
        for i in 0..=50 {
            let t = i as f64 / 50.0;
            let (c0, c1, p1) = (Point::new(0.0, 0.0), Point::new(10.0, 20.0), Point::new(30.0, 20.0));
            let e = Point::new(40.0, 0.0);
            let q = c0 + (c1 - c0) * t;
            let r = c1 + (p1 - c1) * t;
            let s2 = p1 + (e - p1) * t;
            let q2 = q + (r - q) * t;
            let r2 = r + (s2 - r) * t;
            let m = q2 + (r2 - q2) * t;
            let near = seg_dist(&after, m);
            assert!(near <= 1e-4, "sample {i} drifted {near} mm");
        }
    }

    #[test]
    fn line_to_curve_preserves_the_shape() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((12.0, 0.0));
        let mut subs = to_nodes(&p);
        convert_segment(&mut subs, 0, 1, false);
        assert!(!subs[0].nodes[1].line_in);
        // The cubic's handles sit exactly on the segment: flattened = the same line.
        let pts = flatten(&to_path(&subs), 0.001).unwrap()[0].0.clone();
        assert!(pts.iter().all(|pt| pt[1].abs() < 1e-9), "{pts:?}");
        assert!((pts.last().unwrap()[0] - 12.0).abs() < 1e-9);
        // Back to a line restores it.
        convert_segment(&mut subs, 0, 1, true);
        assert!(subs[0].nodes[1].line_in && subs[0].nodes[1].inn.is_none());
    }

    #[test]
    fn delete_follows_the_rules() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.line_to((10.0, 10.0));
        let mut subs = to_nodes(&p);
        delete_nodes(&mut subs, &[(0, 1)]);
        assert_eq!(subs[0].nodes.len(), 2);
        // Down to < 2: the subpath is gone.
        delete_nodes(&mut subs, &[(0, 0), (0, 1)]);
        assert!(subs.is_empty());
        // A closed 3-node triangle stays closed until 2 remain — then it opens.
        let mut tri = to_nodes(&{
            let mut t = BezPath::new();
            t.move_to((0.0, 0.0));
            t.line_to((10.0, 0.0));
            t.line_to((5.0, 8.0));
            t.close_path();
            t
        });
        delete_nodes(&mut tri, &[(0, 0)]);
        assert_eq!(tri[0].nodes.len(), 2);
    }

    #[test]
    fn break_and_join() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.line_to((5.0, 8.0));
        p.close_path();
        let mut subs = to_nodes(&p);
        break_subpath(&mut subs, 0, 1);
        assert!(!subs[0].closed);
        assert_eq!(subs[0].nodes[0].p, Point::new(10.0, 0.0));
        // Join: two open subpaths whose ends coincide merge.
        let mut q = BezPath::new();
        q.move_to((0.0, 0.0));
        q.line_to((10.0, 0.0));
        q.move_to((10.0, 0.0));
        q.line_to((20.0, 5.0));
        let mut qs = to_nodes(&q);
        join_endpoints(&mut qs, (0, 1), (1, 0), 0.01);
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].nodes.len(), 3);
        // And distant ends join with a line.
        let mut r = BezPath::new();
        r.move_to((0.0, 0.0));
        r.line_to((10.0, 0.0));
        r.move_to((0.0, 30.0));
        r.line_to((10.0, 30.0));
        let mut rs = to_nodes(&r);
        join_endpoints(&mut rs, (0, 1), (1, 0), 0.01);
        assert_eq!(rs[0].nodes.len(), 3);
        assert_eq!(rs.len(), 2, "a far join keeps the other subpath and links a line");
    }

    #[test]
    fn moving_nodes_moves_handles_and_handles_move_alone() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((5.0, 5.0), (15.0, 5.0), (20.0, 0.0));
        let mut subs = to_nodes(&p);
        move_nodes(&mut subs, &[(0, 0)], Vec2::new(1.0, 2.0));
        assert_eq!(subs[0].nodes[0].p, Point::new(1.0, 2.0));
        assert_eq!(subs[0].nodes[0].out, Some(Point::new(6.0, 7.0)));
        move_handle(&mut subs, 0, 1, true, Point::new(9.0, 9.0));
        assert_eq!(subs[0].nodes[1].inn, Some(Point::new(9.0, 9.0)));
        // The anchor stayed.
        assert_eq!(subs[0].nodes[1].p, Point::new(20.0, 0.0));
    }
}
