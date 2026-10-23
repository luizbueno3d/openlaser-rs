//! `plan()`: document + machine → `JobPlan` (SPEC §3.7, §7.4).
//!
//! Every shape lands in exactly one place: a burn section, the frame, the tool frame, or a fact that `safety`
//! turns into a refusal or a numbered notice. Nothing is dropped silently, clamped or truncated.

use std::collections::{BTreeSet, HashSet};
use std::time::Duration;

use openlaser_doc::{ColorLayer, CutMode, CutOp, DeviceKind, Geom, Project, Shape, ToolLayer};
use openlaser_geom::{self as geom, Affine, BezPath, CLOSURE_TOL_MM, DEFAULT_FLATTEN_TOL_MM, HatchParams, PathEl, Point, Polyline};
use openlaser_machine::{Kinematics, MachineProfile};
use openlaser_types::{FrameStyle, LayerKey, ObjectId, SectionMode};

use crate::estimate::{estimate_galvo, estimate_gantry};
use crate::fingerprint::fingerprint_of;
use crate::frame::{HULL_FRAME_UNSUPPORTED, styled};
use crate::quantise::{galvo_unencodable, gantry_unencodable, quantise_galvo, quantise_gantry};
use crate::transform::{gantry_transform, machine_transform};
use crate::{CutQ, Error, JobPlan, PlanFacts, PlanOptions, Section};

/// Curve tolerances below this are raised to it: 1 µm is far below a fiber spot, and it bounds the point count.
pub const MIN_FLATTEN_TOL_MM: f64 = 1e-3;
/// A shape with a control point farther than this from the origin (100 m) is not flattened: its anchor points
/// stand in for it, so the plan still carries it and `safety` refuses it (outside the work area or the bed).
pub const MAX_COORD_MM: f64 = 100_000.0;
/// Budget of flattened design points (a pre-flatten upper estimate), against forged or runaway documents.
pub const MAX_DESIGN_POINTS: f64 = 5_000_000.0;
/// Global passes × passes of one sub-layer.
pub const MAX_PASSES_PER_LAYER: u32 = 1000;
/// Layer-passes (LMC lists) in one job.
pub const MAX_SECTIONS: usize = 10_000;
/// Points across every section of a job (each pass holds its own copy of the paths).
pub const MAX_SECTION_POINTS: usize = 20_000_000;
/// Scan lines one Fill pass may need (`HatchParams::max_lines`).
pub const MAX_HATCH_LINES: usize = 200_000;
/// Sub-layers per layer (LightBurn's 1..=11).
pub const MAX_SUB_LAYERS: usize = 11;

/// One drawable leaf of the shape tree (groups expanded, each child on its own layer), in design mm.
struct Leaf {
    id: ObjectId,
    layer: LayerKey,
    polys: Vec<Polyline>,
    bitmap: bool,
}

enum Flat {
    Polys(Vec<Polyline>),
    NonFinite,
    TooDetailed(f64),
}

fn finite_point(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

fn el_points(el: &PathEl) -> Vec<Point> {
    match *el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => vec![p],
        PathEl::QuadTo(a, b) => vec![a, b],
        PathEl::CurveTo(a, b, c) => vec![a, b, c],
        PathEl::ClosePath => Vec::new(),
    }
}

/// An upper estimate of the points `geom::flatten` makes: 1 per line, `1 + 2·√(control length / tol)` per curve.
fn flatten_estimate(bez: &BezPath, tol: f64) -> f64 {
    let mut n = 0.0;
    let mut at = Point::ZERO;
    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => {
                n += 1.0;
                at = p;
            }
            PathEl::QuadTo(a, b) => {
                n += 1.0 + 2.0 * ((at.distance(a) + a.distance(b)) / tol).sqrt();
                at = b;
            }
            PathEl::CurveTo(a, b, c) => {
                n += 1.0 + 2.0 * ((at.distance(a) + a.distance(b) + b.distance(c)) / tol).sqrt();
                at = c;
            }
            PathEl::ClosePath => n += 1.0,
        }
    }
    n
}

/// The on-curve anchor points of each subpath (ClosePath repeats the subpath's first point).
fn anchors(bez: &BezPath) -> Vec<Polyline> {
    let mut out: Vec<Polyline> = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if !cur.is_empty() {
                    out.push(Polyline(std::mem::take(&mut cur)));
                }
                cur.push([p.x, p.y]);
            }
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => cur.push([p.x, p.y]),
            PathEl::ClosePath => {
                if let Some(&first) = cur.first() {
                    cur.push(first);
                }
            }
        }
    }
    if !cur.is_empty() {
        out.push(Polyline(cur));
    }
    out
}

/// Flattens one object-space path placed by `xf` (design mm), within the job's point budget.
fn flatten_placed(mut bez: BezPath, xf: Affine, tol: f64, budget: &mut f64) -> Flat {
    if !xf.is_finite() {
        return Flat::NonFinite;
    }
    bez.apply_affine(xf);
    let pts: Vec<Point> = bez.elements().iter().flat_map(el_points).collect();
    if !pts.iter().all(|p| finite_point(*p)) {
        return Flat::NonFinite;
    }
    if pts.iter().any(|p| p.x.abs() > MAX_COORD_MM || p.y.abs() > MAX_COORD_MM) {
        return Flat::Polys(anchors(&bez));
    }
    let estimate = flatten_estimate(&bez, tol);
    if estimate > *budget {
        return Flat::TooDetailed(estimate);
    }
    *budget -= estimate;
    let polys = geom::flatten(&bez, tol);
    if polys.iter().all(Polyline::all_finite) { Flat::Polys(polys) } else { Flat::NonFinite }
}

struct Ctx<'a> {
    project: &'a Project,
    machine: &'a MachineProfile,
    opts: &'a PlanOptions,
    xf: Affine,
    facts: PlanFacts,
    sections: Vec<Section>,
    section_points: usize,
    out_of_budget: bool,
    frame_outlines: Vec<Polyline>,
    tool_outlines: Vec<Polyline>,
    /// Membership mirrors of `facts.non_finite_objects` / `facts.open_fill_shapes` (no quadratic scans).
    non_finite_seen: HashSet<ObjectId>,
    open_seen: HashSet<(ObjectId, LayerKey)>,
}

impl<'a> Ctx<'a> {
    fn unsupported(&mut self, key: LayerKey, why: String) {
        if !self.facts.unsupported.iter().any(|(k, w)| *k == key && *w == why) {
            self.facts.unsupported.push((key, why));
        }
    }

    fn to_machine(&self, polys: &[Polyline]) -> Vec<Polyline> {
        polys.iter().map(|p| p.transformed(&self.xf)).collect()
    }

    fn selected(&self, id: ObjectId) -> bool {
        self.opts.selected.as_ref().is_none_or(|s| s.contains(&id))
    }

    /// Walks the shape tree in draw order (explicit stack: no recursion depth limit to hit).
    fn leaves(&mut self, tol: f64) -> Vec<Leaf> {
        let mut out = Vec::new();
        let mut budget = MAX_DESIGN_POINTS;
        let project: &'a Project = self.project;
        let mut stack: Vec<(&'a Shape, Affine, bool)> = project.shapes.iter().rev().map(|s| (s.as_ref(), Affine::IDENTITY, self.selected(s.id))).collect();
        while let Some((shape, parent, sel)) = stack.pop() {
            let xf = parent * shape.xform;
            if let Geom::Group(children) = &shape.geom {
                for c in children.iter().rev() {
                    stack.push((c.as_ref(), xf, sel || self.selected(c.id)));
                }
                continue;
            }
            if !sel {
                continue;
            }
            let bitmap = matches!(shape.geom, Geom::Bitmap(_));
            match flatten_placed(shape.geom.to_bez(), xf, tol, &mut budget) {
                Flat::Polys(polys) => {
                    let polys: Vec<Polyline> = polys.into_iter().filter(|p| p.len() >= 2).collect();
                    out.push(Leaf { id: shape.id, layer: shape.layer, polys, bitmap });
                }
                Flat::NonFinite => {
                    if self.non_finite_seen.insert(shape.id) {
                        self.facts.non_finite_objects.push(shape.id);
                    }
                }
                Flat::TooDetailed(n) => self.unsupported(
                    shape.layer,
                    format!(
                        "object {} on {} would flatten to about {n:.0} points, more than this build plans ({MAX_DESIGN_POINTS:.0} per job); simplify it or raise the curve tolerance",
                        shape.id.0, shape.layer
                    ),
                ),
            }
        }
        out
    }

    /// Output order: `layers.order` (first occurrence), then any used layer it misses, in key order.
    fn order(&self, leaves: &[Leaf]) -> Vec<LayerKey> {
        let used: BTreeSet<LayerKey> = leaves.iter().map(|l| l.layer).collect();
        let mut order: Vec<LayerKey> = Vec::new();
        for k in self.project.layers.order.iter().chain(used.iter()) {
            if used.contains(k) && !order.contains(k) {
                order.push(*k);
            }
        }
        order
    }

    fn tool_layer(&self, which: u8) -> Option<&ToolLayer> {
        self.project.layers.tools.iter().find(|t| t.which == which)
    }

    fn plan_tool_layer(&mut self, show: bool, leaves: &[&Leaf]) {
        for l in leaves {
            self.facts.tool_layer_objects.push(l.id);
            if show {
                let m = self.to_machine(&l.polys);
                self.tool_outlines.extend(m);
            }
        }
    }

    fn cut_for(&mut self, key: LayerKey, layer: &ColorLayer, op: &CutOp) -> CutQ {
        match &self.machine.kind {
            Kinematics::Galvo(g) => {
                if op.galvo.timings.is_some() {
                    self.unsupported(key, format!("{key}: per-layer galvo timings are not in this build; clear them to use the machine's timings"));
                }
                if op.galvo.wobble.is_some() {
                    self.unsupported(key, format!("{key}: wobble is not in this build; turn it off"));
                }
                let p = &op.galvo;
                match quantise_galvo(p.speed, p.power, p.frequency, p.q_pulse, g) {
                    Ok(q) => CutQ::Galvo(q),
                    Err(e) => {
                        self.unsupported(key, format!("{key}: {e}"));
                        CutQ::Galvo(galvo_unencodable(p.speed, p.power, p.frequency, p.q_pulse, g))
                    }
                }
            }
            Kinematics::Gantry(g) => {
                let p = &op.gantry;
                match quantise_gantry(p.speed, p.power_max, layer.air_assist, p.constant_power, g) {
                    Ok(q) => CutQ::Gantry(q),
                    Err(e) => {
                        self.unsupported(key, format!("{key}: {e}"));
                        CutQ::Gantry(gantry_unencodable(p.speed, p.power_max))
                    }
                }
            }
        }
    }

    /// Fill: hatch the layer's outlines in design space (the scan angle is the design's), then move the segments
    /// to the machine. One section per hatch pass (cross-hatch passes included).
    fn fill_passes(&mut self, key: LayerKey, op: &CutOp, leaves: &[&Leaf]) -> Result<Option<Vec<Vec<Polyline>>>, Error> {
        let interval = op.fill.interval.get();
        if interval <= 0.0 {
            self.unsupported(key, format!("{key}: Line Interval {interval} mm must be above 0"));
            return Ok(None);
        }
        let mut polys: Vec<Polyline> = Vec::new();
        let mut owner: Vec<ObjectId> = Vec::new();
        for l in leaves {
            for p in &l.polys {
                polys.push(p.clone());
                owner.push(l.id);
            }
        }
        let params = HatchParams {
            interval_mm: interval,
            angle_deg: op.fill.angle.get(),
            passes: op.passes,
            angle_increment_deg: op.fill.angle_increment.get(),
            bidirectional: op.fill.bidirectional,
            cross_hatch: op.fill.cross_hatch,
            closure_tol_mm: CLOSURE_TOL_MM,
            max_lines: MAX_HATCH_LINES,
        };
        let r = match geom::hatch(&polys, &params) {
            Ok(r) => r,
            Err(e @ (geom::Error::BadInterval(_) | geom::Error::TooManyLines(..))) => {
                self.unsupported(key, format!("{key}: Fill cannot be hatched: {e}; raise the Line Interval"));
                return Ok(None);
            }
            Err(e) => return Err(Error::Geom(e)),
        };
        for i in &r.open_inputs {
            if let Some(&id) = owner.get(*i)
                && self.open_seen.insert((id, key))
            {
                self.facts.open_fill_shapes.push((id, key));
            }
        }
        let passes: Vec<Vec<Polyline>> = r.passes.iter().map(|segs| segs.iter().map(|[a, b]| Polyline(vec![*a, *b]).transformed(&self.xf)).collect()).collect();
        if passes.iter().all(Vec::is_empty) && !self.facts.empty_fill_layers.contains(&key) {
            self.facts.empty_fill_layers.push(key);
        }
        Ok(Some(passes))
    }

    /// Admits one more section of `paths` under the job's budget; past it, records one `unsupported` fact.
    fn admit(&mut self, key: LayerKey, pass: u16, paths: &[Polyline]) -> bool {
        if self.out_of_budget {
            return false;
        }
        let pts: usize = paths.iter().map(Polyline::len).sum();
        if self.sections.len() >= MAX_SECTIONS || self.section_points.saturating_add(pts) > MAX_SECTION_POINTS {
            if !self.out_of_budget {
                self.out_of_budget = true;
                self.unsupported(
                    key,
                    format!(
                        "the job needs more than {MAX_SECTIONS} layer passes or {MAX_SECTION_POINTS} points across its passes (reached at {key} pass {pass}); reduce passes or detail"
                    ),
                );
            }
            return false;
        }
        self.section_points += pts;
        true
    }

    fn estimate(&self, paths: &[Polyline], cut: &CutQ) -> Duration {
        match (&self.machine.kind, cut) {
            (Kinematics::Galvo(m), CutQ::Galvo(q)) => estimate_galvo(paths, q, m),
            (Kinematics::Gantry(m), CutQ::Gantry(q)) => estimate_gantry(paths, q, m),
            _ => Duration::ZERO,
        }
    }

    fn plan_color_layer(&mut self, key: LayerKey, layer: &ColorLayer, leaves: &[&Leaf]) -> Result<(), Error> {
        if !layer.output {
            return Ok(());
        }
        let images = leaves.iter().filter(|l| l.bitmap).count();
        if images > 0 {
            self.unsupported(
                key,
                format!("{key} holds {images} image(s): burning images is not in this build; turn the layer's Output off or move the images"),
            );
        }
        let vector: Vec<&Leaf> = leaves.iter().copied().filter(|l| !l.bitmap).collect();
        if layer.ops.is_empty() || layer.ops.len() > MAX_SUB_LAYERS {
            self.unsupported(key, format!("{key} has {} sub-layers; a layer needs 1 to {MAX_SUB_LAYERS}", layer.ops.len()));
            return Ok(());
        }
        let enabled: Vec<(u8, &CutOp)> =
            layer.ops.iter().enumerate().filter(|(_, op)| op.enabled).filter_map(|(i, op)| Some((u8::try_from(i).ok()?, op))).collect();
        if enabled.is_empty() || vector.is_empty() {
            return Ok(());
        }
        // B6: this layer would burn, so it must be visible (it is framed either way).
        if !layer.show && !self.facts.hidden_output_layers.contains(&key) {
            self.facts.hidden_output_layers.push(key);
        }
        // Frame ⊇ burn: every outline of every vector shape on an emitting layer, whatever `show` says (B6).
        for l in &vector {
            let m = self.to_machine(&l.polys);
            self.frame_outlines.extend(m);
        }
        if layer.global_passes == 0 {
            self.unsupported(key, format!("{key}: Global Passes is 0; set at least 1"));
            return Ok(());
        }
        let objects: Vec<ObjectId> = vector.iter().filter(|l| !l.polys.is_empty()).map(|l| l.id).collect();
        let line_paths: Vec<Polyline> = vector.iter().flat_map(|l| self.to_machine(&l.polys)).collect();

        /// Line repeats one set of paths; Fill has one set per hatch pass. Paths are cloned into a section only
        /// after the job's budget admits it.
        enum Passes {
            Repeat(Vec<Polyline>, u16),
            Each(Vec<Vec<Polyline>>),
        }
        impl Passes {
            fn count(&self) -> u32 {
                match self {
                    Passes::Repeat(_, n) => u32::from(*n),
                    Passes::Each(v) => u32::try_from(v.len()).unwrap_or(u32::MAX),
                }
            }
            fn get(&self, i: u32) -> Option<&Vec<Polyline>> {
                match self {
                    Passes::Repeat(p, n) => (i < u32::from(*n)).then_some(p),
                    Passes::Each(v) => v.get(usize::try_from(i).ok()?),
                }
            }
        }
        struct OpPlan {
            sub: u8,
            mode: SectionMode,
            cut: CutQ,
            passes: Passes,
            of: u16,
        }
        let mut ops: Vec<OpPlan> = Vec::new();
        for (sub, op) in enabled {
            if op.passes == 0 {
                self.unsupported(key, format!("{key} sub-layer {}: Passes is 0; set at least 1", sub + 1));
                continue;
            }
            let (mode, passes) = match op.mode {
                CutMode::Line => (SectionMode::Line, Passes::Repeat(line_paths.clone(), op.passes)),
                CutMode::Fill => match self.fill_passes(key, op, &vector)? {
                    Some(p) => (SectionMode::Fill, Passes::Each(p)),
                    None => continue,
                },
                CutMode::OffsetFill => {
                    self.unsupported(key, format!("{key}: Offset Fill is not in this build; use Line or Fill"));
                    continue;
                }
            };
            let total = u32::from(layer.global_passes).saturating_mul(passes.count());
            if total > MAX_PASSES_PER_LAYER {
                self.unsupported(key, format!("{key} sub-layer {}: {total} passes is above the {MAX_PASSES_PER_LAYER} this build plans", sub + 1));
                continue;
            }
            let of = u16::try_from(total).unwrap_or(u16::MAX);
            let cut = self.cut_for(key, layer, op);
            ops.push(OpPlan { sub, mode, cut, passes, of });
        }

        for g in 0..u32::from(layer.global_passes) {
            for op in &ops {
                let n = op.passes.count();
                for p in 0..n {
                    let Some(paths) = op.passes.get(p) else { continue };
                    if paths.is_empty() {
                        continue;
                    }
                    let pass = u16::try_from(g.saturating_mul(n).saturating_add(p).saturating_add(1)).unwrap_or(u16::MAX);
                    if !self.admit(key, pass, paths) {
                        return Ok(());
                    }
                    let estimate = self.estimate(paths, &op.cut);
                    self.sections.push(Section {
                        layer: key,
                        layer_name: layer.name.clone(),
                        sub: op.sub,
                        pass,
                        of: op.of,
                        mode: op.mode,
                        cut: op.cut.clone(),
                        paths: paths.clone(),
                        objects: objects.clone(),
                        estimate,
                    });
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn device_of(m: &MachineProfile) -> DeviceKind {
    match m.kind {
        Kinematics::Galvo(_) => DeviceKind::GalvoLmc,
        Kinematics::Gantry(_) => DeviceKind::Grbl,
    }
}

fn kind_name(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::GalvoLmc => "galvo (LMC)",
        DeviceKind::Grbl => "GRBL gantry",
    }
}

pub(crate) fn plan(project: &Project, machine: &MachineProfile, opts: &PlanOptions) -> Result<JobPlan, Error> {
    let device = device_of(machine);
    let xf = match &machine.kind {
        Kinematics::Galvo(g) => machine_transform(g),
        Kinematics::Gantry(g) => gantry_transform(project.workspace.origin, g),
    };
    let tol = if opts.flatten_tol_mm.is_finite() && opts.flatten_tol_mm > 0.0 { opts.flatten_tol_mm.max(MIN_FLATTEN_TOL_MM) } else { DEFAULT_FLATTEN_TOL_MM };
    let mut cx = Ctx {
        project,
        machine,
        opts,
        xf,
        facts: PlanFacts::default(),
        sections: Vec::new(),
        section_points: 0,
        out_of_budget: false,
        frame_outlines: Vec::new(),
        tool_outlines: Vec::new(),
        non_finite_seen: HashSet::new(),
        open_seen: HashSet::new(),
    };
    if let Some(h) = &project.device_hint
        && h.kind != device
    {
        cx.facts.device_mismatch = Some(format!(
            "this project was made for {} ({}), but {} is a {}; choose a {} machine or start a project for this one",
            h.profile.0,
            kind_name(h.kind),
            machine.id.0,
            kind_name(device),
            kind_name(h.kind)
        ));
    }

    let leaves = cx.leaves(tol);
    for key in cx.order(&leaves) {
        let on: Vec<&Leaf> = leaves.iter().filter(|l| l.layer == key).collect();
        match key {
            LayerKey::Tool(t) => match cx.tool_layer(t).map(|l| l.show) {
                Some(show) => cx.plan_tool_layer(show, &on),
                None => cx.facts.orphan_objects.extend(on.iter().map(|l| l.id)),
            },
            LayerKey::Color(i) => match project.color_layer(i) {
                Some(layer) => cx.plan_color_layer(key, layer, &on)?,
                None => cx.facts.orphan_objects.extend(on.iter().map(|l| l.id)),
            },
        }
    }

    if opts.frame_style == FrameStyle::Hull {
        let key = cx.sections.first().map(|s| s.layer).or_else(|| cx.project.layers.order.first().copied()).unwrap_or(LayerKey::Color(0));
        cx.unsupported(key, HULL_FRAME_UNSUPPORTED.to_owned());
    }

    let frame = styled(std::mem::take(&mut cx.frame_outlines), opts.frame_style);
    let tool_frame = styled(std::mem::take(&mut cx.tool_outlines), opts.frame_style);
    let one = cx.sections.iter().fold(Duration::ZERO, |a, s| a.saturating_add(s.estimate));
    let estimate = one.checked_mul(u32::from(opts.repeats)).unwrap_or(Duration::MAX);
    let fingerprint = fingerprint_of(machine, device, &cx.sections, opts.repeats);
    Ok(JobPlan { machine: machine.id.clone(), device, sections: cx.sections, frame, tool_frame, facts: cx.facts, repeats: opts.repeats, estimate, fingerprint })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_keep_subpaths_and_close() {
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.curve_to((1.0, 1.0), (2.0, 1.0), (3.0, 0.0));
        b.close_path();
        b.move_to((5.0, 5.0));
        b.line_to((6.0, 5.0));
        assert_eq!(anchors(&b), vec![Polyline(vec![[0.0, 0.0], [3.0, 0.0], [0.0, 0.0]]), Polyline(vec![[5.0, 5.0], [6.0, 5.0]])]);
    }

    #[test]
    fn flatten_estimate_counts_lines_and_bounds_curves() {
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.line_to((10.0, 0.0));
        b.line_to((10.0, 10.0));
        assert_eq!(flatten_estimate(&b, 0.01), 3.0);
        let mut c = BezPath::new();
        c.move_to((0.0, 0.0));
        c.curve_to((0.0, 100.0), (100.0, 100.0), (100.0, 0.0));
        // control length 300 mm at 0.01 mm: 1 + 2·√30000 ≈ 347, far above kurbo's real count.
        let n = flatten_estimate(&c, 0.01);
        assert!(n > 340.0 && n < 350.0, "{n}");
    }

    #[test]
    fn huge_or_non_finite_shapes_are_not_flattened() {
        let mut budget = MAX_DESIGN_POINTS;
        let mut g = BezPath::new();
        g.move_to((0.0, 0.0));
        g.curve_to((1e9, 0.0), (1e9, 1e9), (0.0, 1e9));
        match flatten_placed(g.clone(), Affine::IDENTITY, 0.01, &mut budget) {
            Flat::Polys(p) => assert_eq!(p, vec![Polyline(vec![[0.0, 0.0], [0.0, 1e9]])]),
            _ => panic!("a huge shape keeps its anchors"),
        }
        assert_eq!(budget, MAX_DESIGN_POINTS);
        assert!(matches!(flatten_placed(g, Affine::scale(f64::NAN), 0.01, &mut budget), Flat::NonFinite));
        let mut nan = BezPath::new();
        nan.move_to((0.0, f64::NAN));
        nan.line_to((1.0, 1.0));
        assert!(matches!(flatten_placed(nan, Affine::IDENTITY, 0.01, &mut budget), Flat::NonFinite));
        let mut tiny = 10.0;
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.curve_to((0.0, 50.0), (50.0, 50.0), (50.0, 0.0));
        assert!(matches!(flatten_placed(b, Affine::IDENTITY, 0.01, &mut tiny), Flat::TooDetailed(_)));
        assert_eq!(tiny, 10.0, "a refused shape spends no budget");
    }
}
