//! `plan()`: document + machine → `JobPlan` (SPEC §3.7, §7.4).
//!
//! Every shape lands in exactly one place: a burn section, the frame, the tool frame, or a fact that `safety`
//! turns into a refusal or a numbered notice. Nothing is clamped or truncated. The one exception to "lands
//! somewhere": a path with fewer than 2 points (a lone point, an empty path) has nothing to burn or trace, so it
//! burns nothing and is not framed (it can only burn less, never more; PlanFacts has no field for it yet).

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
/// Curve tolerances above this (0.1 mm) are refused (an `unsupported` fact), never used: a coarse tolerance turns
/// curves into chords that burn visibly off the design.
pub const MAX_FLATTEN_TOL_MM: f64 = 0.1;
/// A shape with a control point farther than this from the origin (100 m) is not flattened: its control polygon
/// (every on- and off-curve point, in order) stands in for it. The curve lies inside the convex hull of those
/// points, and the stand-in holds the far point itself, so `safety` always refuses it (outside the work area or
/// the bed); it is never quietly replaced by an in-field chord.
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
    /// `Shape.power_scale` in percent (P1 behaviour: stored, not applied, so anything but 100 is refused).
    power_scale: f64,
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

/// The control polygon of each subpath: every on- and off-curve point in order (ClosePath repeats the subpath's
/// first point). It contains the far control point that made the shape too big to flatten, so the stand-in is
/// never inside the field when the real curve is not.
fn control_polygons(bez: &BezPath) -> Vec<Polyline> {
    let mut out: Vec<Polyline> = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    for el in bez.elements() {
        if let PathEl::MoveTo(_) = el
            && !cur.is_empty()
        {
            out.push(Polyline(std::mem::take(&mut cur)));
        }
        if let PathEl::ClosePath = el {
            if let Some(&first) = cur.first() {
                cur.push(first);
            }
            continue;
        }
        cur.extend(el_points(el).into_iter().map(|p| [p.x, p.y]));
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
        return Flat::Polys(control_polygons(&bez));
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
    /// Membership mirrors of `facts.non_finite_objects` / `facts.open_fill_shapes` / `facts.unsupported` (no
    /// quadratic scans on forged documents).
    non_finite_seen: HashSet<ObjectId>,
    open_seen: HashSet<(ObjectId, LayerKey)>,
    unsupported_seen: HashSet<(LayerKey, String)>,
    /// `opts.selected` as a set.
    selection: Option<HashSet<ObjectId>>,
}

impl<'a> Ctx<'a> {
    fn new(project: &'a Project, machine: &'a MachineProfile, opts: &'a PlanOptions) -> Self {
        let xf = match &machine.kind {
            Kinematics::Galvo(g) => machine_transform(g),
            Kinematics::Gantry(g) => gantry_transform(project.workspace.origin, g),
        };
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
            unsupported_seen: HashSet::new(),
            selection: opts.selected.as_ref().map(|s| s.iter().copied().collect()),
        };
        let device = device_of(machine);
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
        cx
    }

    fn unsupported(&mut self, key: LayerKey, why: String) {
        if self.unsupported_seen.insert((key, why.clone())) {
            self.facts.unsupported.push((key, why));
        }
    }

    /// The layer a job-wide fact is filed under: the first burned layer, else the first in the output order.
    fn job_key(&self) -> LayerKey {
        self.sections.first().map(|s| s.layer).or_else(|| self.project.layers.order.first().copied()).unwrap_or(LayerKey::Color(0))
    }

    fn to_machine(&self, polys: &[Polyline]) -> Vec<Polyline> {
        polys.iter().map(|p| p.transformed(&self.xf)).collect()
    }

    fn selected(&self, id: ObjectId) -> bool {
        self.selection.as_ref().is_none_or(|s| s.contains(&id))
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
                    out.push(Leaf { id: shape.id, layer: shape.layer, polys, bitmap, power_scale: shape.power_scale.get() });
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
        // Per-object power scale is stored but not applied in this build: a scale below 100 % would burn hotter
        // than the document asks, so it is refused, never ignored.
        let scaled = vector.iter().filter(|l| l.power_scale != 100.0).count();
        if scaled > 0 {
            self.unsupported(
                key,
                format!("{key}: {scaled} object(s) have a Power Scale other than 100 %; per-object power scale is not in this build, set it to 100 %"),
            );
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
    let tol = if opts.flatten_tol_mm.is_finite() && opts.flatten_tol_mm > 0.0 { opts.flatten_tol_mm.max(MIN_FLATTEN_TOL_MM) } else { DEFAULT_FLATTEN_TOL_MM };
    let mut cx = Ctx::new(project, machine, opts);
    let leaves = cx.leaves(tol);
    cx.finish(&leaves, tol)
}

impl Ctx<'_> {
    /// Plans the flattened leaves: one layer at a time in output order, then the frames, estimate and fingerprint.
    fn finish(mut self, leaves: &[Leaf], tol: f64) -> Result<JobPlan, Error> {
        let project = self.project;
        for key in self.order(leaves) {
            let on: Vec<&Leaf> = leaves.iter().filter(|l| l.layer == key).collect();
            match key {
                LayerKey::Tool(t) => match self.tool_layer(t).map(|l| l.show) {
                    Some(show) => self.plan_tool_layer(show, &on),
                    None => self.facts.orphan_objects.extend(on.iter().map(|l| l.id)),
                },
                LayerKey::Color(i) => match project.color_layer(i) {
                    Some(layer) => self.plan_color_layer(key, layer, &on)?,
                    None => self.facts.orphan_objects.extend(on.iter().map(|l| l.id)),
                },
            }
        }

        if tol > MAX_FLATTEN_TOL_MM {
            let key = self.job_key();
            self.unsupported(key, format!("the curve tolerance {tol} mm is above the {MAX_FLATTEN_TOL_MM} mm this build plans; lower it"));
        }
        if self.opts.frame_style == FrameStyle::Hull {
            let key = self.job_key();
            self.unsupported(key, HULL_FRAME_UNSUPPORTED.to_owned());
        }

        let machine = self.machine;
        let opts = self.opts;
        let device = device_of(machine);
        let frame = styled(std::mem::take(&mut self.frame_outlines), opts.frame_style);
        let tool_frame = styled(std::mem::take(&mut self.tool_outlines), opts.frame_style);
        let one = self.sections.iter().fold(Duration::ZERO, |a, s| a.saturating_add(s.estimate));
        let estimate = one.checked_mul(u32::from(opts.repeats)).unwrap_or(Duration::MAX);
        let fingerprint = fingerprint_of(machine, device, &self.sections, opts.repeats);
        Ok(JobPlan {
            machine: machine.id.clone(),
            device,
            sections: self.sections,
            frame,
            tool_frame,
            facts: self.facts,
            repeats: opts.repeats,
            estimate,
            fingerprint,
        })
    }
}

#[cfg(test)]
mod tests {
    use openlaser_doc::{ImageCut, ImageMode, Workspace, WorkspaceOrigin, default_layer_name};
    use openlaser_types::FrameSource;

    use super::*;

    const C0: LayerKey = LayerKey::Color(0);
    const C1: LayerKey = LayerKey::Color(1);

    fn color_layer(m: &MachineProfile, i: u8, show: bool) -> ColorLayer {
        let d = &m.layer_defaults;
        ColorLayer {
            index: i,
            name: default_layer_name(LayerKey::Color(i)),
            output: true,
            show,
            ops: vec![CutOp {
                name: String::new(),
                enabled: true,
                mode: CutMode::Line,
                passes: 1,
                galvo: d.galvo.clone(),
                gantry: d.gantry.clone(),
                fill: d.fill.clone(),
            }],
            global_passes: 1,
            image: ImageCut {
                mode: ImageMode::Threshold,
                interval: d.fill.interval,
                angle: d.fill.angle,
                bidirectional: true,
                negative: false,
                threshold: 128,
                passes: 1,
                galvo: d.galvo.clone(),
                gantry: d.gantry.clone(),
            },
            air_assist: false,
        }
    }

    /// C00 shown, C01 hidden; both Output on, one Line pass each.
    fn two_layer_project(m: &MachineProfile) -> Project {
        let mut p = Project::new(Workspace { kind: DeviceKind::GalvoLmc, size_mm: [200.0, 200.0], origin: WorkspaceOrigin::Center }, &m.layer_defaults);
        p.layers.colors = vec![color_layer(m, 0, true), color_layer(m, 1, false)];
        p.layers.order = vec![C0, C1];
        p
    }

    fn opts(style: FrameStyle, tol: f64) -> PlanOptions {
        PlanOptions { selected: None, repeats: 1, frame_style: style, frame_source: FrameSource::Job, flatten_tol_mm: tol }
    }

    fn square(cx: f64) -> Polyline {
        Polyline(vec![[cx - 2.0, -2.0], [cx + 2.0, -2.0], [cx + 2.0, 2.0], [cx - 2.0, 2.0], [cx - 2.0, -2.0]])
    }

    fn leaf(id: u64, layer: LayerKey, cx: f64, power_scale: f64) -> Leaf {
        Leaf { id: ObjectId(id), layer, polys: vec![square(cx)], bitmap: false, power_scale }
    }

    /// B6 regression that runs on WP0 stubs (the leaves are built directly, no `to_bez`/`flatten`): a hidden layer
    /// whose Output is on is recorded AND framed, so frame ⊇ burn holds for the layer nobody can see. Reverting the
    /// "whatever `show` says" framing fails here.
    #[test]
    fn hidden_emitting_layers_are_framed_and_recorded() {
        let m = openlaser_machine::jpt_m7_200_sim();
        let p = two_layer_project(&m);
        let leaves = vec![leaf(1, C0, -20.0, 100.0), leaf(2, C1, 20.0, 100.0)];
        for style in [FrameStyle::Contour, FrameStyle::Bounds] {
            let o = opts(style, 0.01);
            let cx = Ctx::new(&p, &m, &o);
            let xf = cx.xf;
            let plan = cx.finish(&leaves, 0.01).unwrap();
            assert_eq!(plan.facts.hidden_output_layers, vec![C1]);
            let hidden: Vec<&Polyline> = plan.sections.iter().filter(|s| s.layer == C1).flat_map(|s| s.paths.iter()).collect();
            assert_eq!(hidden, vec![&square(20.0).transformed(&xf)], "the hidden layer still plans its burn (safety refuses it)");
            let fb = geom::bounds_of(&plan.frame).unwrap();
            for q in plan.sections.iter().flat_map(|s| s.paths.iter()).flat_map(|p| p.0.iter()) {
                assert!(q[0] >= fb.min[0] && q[0] <= fb.max[0] && q[1] >= fb.min[1] && q[1] <= fb.max[1], "{style:?}: burn point {q:?} outside the frame");
            }
            if style == FrameStyle::Contour {
                assert!(plan.frame.contains(&square(20.0).transformed(&xf)), "the hidden layer's outline is traced");
                assert!(plan.frame.contains(&square(-20.0).transformed(&xf)));
            }
        }
    }

    #[test]
    fn power_scale_other_than_100_is_refused_not_ignored() {
        let m = openlaser_machine::jpt_m7_200_sim();
        let p = two_layer_project(&m);
        let o = opts(FrameStyle::Contour, 0.01);
        let plan = Ctx::new(&p, &m, &o).finish(&[leaf(1, C0, -20.0, 50.0), leaf(2, C0, 0.0, 100.0)], 0.01).unwrap();
        assert!(plan.facts.unsupported.iter().any(|(k, w)| *k == C0 && w.contains("Power Scale")), "{:?}", plan.facts.unsupported);
        let plan = Ctx::new(&p, &m, &o).finish(&[leaf(1, C0, -20.0, 100.0)], 0.01).unwrap();
        assert!(!plan.facts.unsupported.iter().any(|(_, w)| w.contains("Power Scale")), "{:?}", plan.facts.unsupported);
    }

    #[test]
    fn coarse_curve_tolerance_is_refused() {
        let m = openlaser_machine::jpt_m7_200_sim();
        let p = two_layer_project(&m);
        let o = opts(FrameStyle::Contour, 0.5);
        let plan = Ctx::new(&p, &m, &o).finish(&[leaf(1, C0, 0.0, 100.0)], 0.5).unwrap();
        assert!(plan.facts.unsupported.iter().any(|(_, w)| w.contains("curve tolerance")), "{:?}", plan.facts.unsupported);
        let o = opts(FrameStyle::Contour, MAX_FLATTEN_TOL_MM);
        let plan = Ctx::new(&p, &m, &o).finish(&[leaf(1, C0, 0.0, 100.0)], MAX_FLATTEN_TOL_MM).unwrap();
        assert!(!plan.facts.unsupported.iter().any(|(_, w)| w.contains("curve tolerance")), "{:?}", plan.facts.unsupported);
    }

    #[test]
    fn unsupported_facts_are_deduplicated() {
        let m = openlaser_machine::jpt_m7_200_sim();
        let p = two_layer_project(&m);
        let o = opts(FrameStyle::Contour, 0.01);
        let mut cx = Ctx::new(&p, &m, &o);
        for _ in 0..3 {
            cx.unsupported(C0, "x".into());
        }
        cx.unsupported(C1, "x".into());
        assert_eq!(cx.facts.unsupported, vec![(C0, "x".to_owned()), (C1, "x".to_owned())]);
    }

    #[test]
    fn control_polygons_keep_subpaths_controls_and_close() {
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.curve_to((1.0, 1.0), (2.0, 1.0), (3.0, 0.0));
        b.close_path();
        b.move_to((5.0, 5.0));
        b.quad_to((5.5, 6.0), (6.0, 5.0));
        b.line_to((7.0, 5.0));
        assert_eq!(
            control_polygons(&b),
            vec![Polyline(vec![[0.0, 0.0], [1.0, 1.0], [2.0, 1.0], [3.0, 0.0], [0.0, 0.0]]), Polyline(vec![[5.0, 5.0], [5.5, 6.0], [6.0, 5.0], [7.0, 5.0]])]
        );
    }

    /// Review regression: anchors inside the field, one control point 1e9 mm out. The real curve bulges ~750 km
    /// out; the stand-in must carry that far point (so safety refuses it), never the 10 mm chord between anchors.
    #[test]
    fn huge_control_point_with_anchors_inside_is_never_a_chord() {
        let mut budget = MAX_DESIGN_POINTS;
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.curve_to((1e9, 0.0), (1e9, 0.0), (10.0, 0.0));
        match flatten_placed(b, Affine::IDENTITY, 0.01, &mut budget) {
            Flat::Polys(p) => {
                assert_eq!(p, vec![Polyline(vec![[0.0, 0.0], [1e9, 0.0], [1e9, 0.0], [10.0, 0.0]])]);
                assert!(p.iter().flat_map(|l| l.0.iter()).any(|q| q[0].abs() > MAX_COORD_MM), "the far point is kept");
            }
            _ => panic!("a huge shape keeps its control polygon"),
        }
        assert_eq!(budget, MAX_DESIGN_POINTS);
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
            Flat::Polys(p) => assert_eq!(p, vec![Polyline(vec![[0.0, 0.0], [1e9, 0.0], [1e9, 1e9], [0.0, 1e9]])]),
            _ => panic!("a huge shape keeps its control polygon"),
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
