//! Tools and the Creation toolbar: Select, Rectangle, Ellipse, Draw Lines (Appendix A). With params, a drawing
//! command creates the shape (the agent's way to draw); the GUI's toolbar arms the tool instead.

use std::sync::Arc;

use openlaser_doc::{Geom, Shape};
use openlaser_geom::boolean::{BooleanOp, Cap, Join};
use openlaser_geom::{Affine, BezPath, Vec2};
use openlaser_types::{LayerKey, Mm, ObjectId, Percent};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Created, always, needs_doc};
use crate::command::{Access, CommandSpec, EngineError, Kind};
use crate::params::{IdsParams, LayerKeySchema, NoParams, finite, positive};
use crate::session::{Label, Session};
use crate::{command, geomx};

/// Largest drawn size per axis (mm).
const MAX_SIZE_MM: f64 = 100_000.0;
/// Most points one `tools.drawLines` call may add.
const MAX_POINTS: usize = 100_000;

const FULL_POWER: Percent = match Percent::new_const(100.0) {
    Some(p) => p,
    None => Percent::MAX_VALID,
};

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command!("tools.select", "Select", menu: ["Tools"], shortcut: None, kind: Kind::View, access: Access::Ui,
            enabled: always, params: NoParams, run: |_s: &mut Session, _p: NoParams| Ok::<_, EngineError>(json!({ "ui": "tools.select" })), doc: "{}"),
        command!("tools.rectangle", "Rectangle", menu: ["Tools"], shortcut: Some("Cmd+R"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: RectangleParams, run: rectangle, doc: "{xMm, yMm, wMm, hMm, cornerRadiusMm?, layer?} (x, y = lower-left corner)"),
        command!("tools.ellipse", "Ellipse", menu: ["Tools"], shortcut: Some("Cmd+E"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: EllipseParams, run: ellipse, doc: "{cxMm, cyMm, wMm, hMm, layer?}"),
        command!("tools.drawLines", "Draw Lines", menu: ["Tools"], shortcut: Some("Cmd+L"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: DrawLinesParams, run: draw_lines, doc: "{pointsMm?:[[x,y],…] | d?:\"M0 0L10 0…\", closed?, layer?}"),
        command!("tools.polygon", "Polygon", menu: ["Tools", "Shape"], shortcut: None, kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: PolygonParams, run: polygon, doc: "{cxMm, cyMm, radiusMm, sides? (3..=64, default 6), rotateDeg?, layer?}"),
        command!("tools.measure", "Measure", menu: ["Tools"], shortcut: Some("Alt+M"), kind: Kind::Query, access: Access::Agent,
            enabled: always, params: MeasureParams, run: measure, doc: "{fromMm:[x,y], toMm:[x,y]} → {distanceMm, dxMm, dyMm, angleDeg (CCW from +X, Y up)}"),
        command!("tools.weld", "Weld Shapes", menu: ["Tools"], shortcut: Some("Cmd+W"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: IdsParams, run: weld, doc: "{ids?} (≥ 2 closed objects, unioned into one path)"),
        command!("tools.booleanUnion", "Boolean Union", menu: ["Tools"], shortcut: Some("Alt+Plus"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: IdsParams, run: |s, p| boolean_two(s, p, BooleanOp::Union), doc: "{ids?} (exactly 2 closed objects)"),
        command!("tools.booleanSubtract", "Boolean Subtract", menu: ["Tools"], shortcut: Some("Alt+Minus"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: IdsParams, run: |s, p| boolean_two(s, p, BooleanOp::Difference), doc: "{ids?} (exactly 2 closed objects; the topmost is subtracted)"),
        command!("tools.booleanIntersect", "Boolean Intersection", menu: ["Tools"], shortcut: Some("Alt+Shift+8"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: IdsParams, run: |s, p| boolean_two(s, p, BooleanOp::Intersection), doc: "{ids?} (exactly 2 closed objects)"),
        command!("tools.offsetShapes", "Offset Shapes…", menu: ["Tools"], shortcut: Some("Alt+O"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: OffsetParams, run: offset_shapes,
            doc: "{ids?, distanceMm (>0, ≤1000), direction?\"outward\"|\"inward\"|\"both\", corner?\"round\"|\"sharp\"|\"bevel\", deleteOriginal?}"),
    ]
}

/// Closed world-space polylines of one top-level target (groups contribute their closed leaves; flattened at the
/// session's curve tolerance). An open ring is a bad-param error naming the object.
fn closed_world_rings(s: &Session, id: &str, oid: openlaser_types::ObjectId) -> Result<Vec<openlaser_geom::Polyline>, EngineError> {
    let tol = s.prefs().curve_tolerance_mm();
    let polys = s.doc()?.project.world_polylines(oid, tol).map_err(geomx::geom_error)?;
    for pl in &polys {
        if !pl.is_closed(openlaser_geom::CLOSURE_TOL_MM) {
            return Err(EngineError::bad(id, "/ids", format!("weld and booleans need closed shapes; object {} is open", oid.0)));
        }
    }
    if polys.is_empty() {
        return Err(EngineError::bad(id, "/ids", format!("object {} has no outline", oid.0)));
    }
    Ok(polys)
}

/// Rings → one closed BezPath (world space; the new shape gets xform identity).
fn rings_to_path(rings: &[openlaser_geom::Polyline]) -> BezPath {
    let mut path = BezPath::new();
    for r in rings {
        for (i, pt) in r.0.iter().enumerate() {
            if i == 0 {
                path.move_to((pt[0], pt[1]));
            } else {
                path.line_to((pt[0], pt[1]));
            }
        }
        path.close_path();
    }
    path
}

/// Replace the target shapes by `rings` as one new path: bottom-most target's layer, topmost target's z-slot,
/// the targets removed, the result selected.
fn commit_paths(s: &mut Session, label: &'static str, ids: &[openlaser_types::ObjectId], rings: Vec<openlaser_geom::Polyline>) -> Result<Created, EngineError> {
    s.edit(Label::new(label), None, |project, sel| {
        let mut low: Option<(usize, LayerKey)> = None;
        let mut top = 0usize;
        for (i, sh) in project.shapes.iter().enumerate() {
            if ids.contains(&sh.id) {
                if low.is_none() {
                    low = Some((i, sh.layer));
                }
                top = i;
            }
        }
        let layer = low.map(|(_, l)| l).unwrap_or(LayerKey::Color(0));
        let removed_above_slot = ids.len() - 1;
        let id = project.alloc_id().map_err(geomx::doc_error)?;
        project.shapes.retain(|sh| !ids.contains(&sh.id));
        let at = top.saturating_sub(removed_above_slot).min(project.shapes.len());
        project.shapes.insert(
            at,
            Arc::new(Shape {
                id,
                name: None,
                layer,
                xform: Affine::IDENTITY,
                geom: Geom::Path(Arc::new(rings_to_path(&rings))),
                locked: false,
                power_scale: FULL_POWER,
                priority: 0,
            }),
        );
        sel.ids = vec![id];
        sel.key = Some(id);
        Ok(Created { ids: vec![id] })
    })
}

fn weld(s: &mut Session, p: IdsParams) -> Result<Created, EngineError> {
    const ID: &str = "tools.weld";
    let ids = super::target_ids(s, ID, p.ids)?;
    if ids.len() < 2 {
        return Err(EngineError::bad(ID, "/ids", "weld needs at least two objects"));
    }
    for oid in &ids {
        if s.doc()?.project.shape(*oid).is_some_and(|sh| sh.locked) {
            return Err(EngineError::disabled(ID, "a locked object cannot be welded; unlock it first"));
        }
    }
    let shapes: Vec<Vec<openlaser_geom::Polyline>> = ids.iter().map(|oid| closed_world_rings(s, ID, *oid)).collect::<Result<_, _>>()?;
    let rings = openlaser_geom::boolean::union_all(&shapes).map_err(geomx::geom_error)?;
    if rings.is_empty() {
        return Err(EngineError::bad(ID, "/ids", "weld produced nothing"));
    }
    commit_paths(s, "Weld", &ids, rings)
}

fn boolean_two(s: &mut Session, p: IdsParams, op: BooleanOp) -> Result<Created, EngineError> {
    const ID: &str = "tools.boolean";
    let ids = super::target_ids(s, ID, p.ids)?;
    if ids.len() != 2 {
        return Err(EngineError::bad(ID, "/ids", "booleans need exactly two objects"));
    }
    // Draw order decides the operands: the topmost target is the clip (Subtract removes it from the bottom one).
    let mut order: Vec<(usize, openlaser_types::ObjectId)> =
        s.doc()?.project.shapes.iter().enumerate().filter(|(_, sh)| ids.contains(&sh.id)).map(|(i, sh)| (i, sh.id)).collect();
    order.sort_by_key(|(i, _)| *i);
    for (_, oid) in &order {
        if s.doc()?.project.shape(*oid).is_some_and(|sh| sh.locked) {
            return Err(EngineError::disabled(ID, "a locked object cannot take part in a boolean; unlock it first"));
        }
    }
    let Some((bottom, top)) = order.first().zip(order.last()).map(|(b, t)| (b.1, t.1)) else {
        return Err(EngineError::bad(ID, "/ids", "booleans need exactly two objects"));
    };
    let a = closed_world_rings(s, ID, bottom)?;
    let b = closed_world_rings(s, ID, top)?;
    let rings = openlaser_geom::boolean::boolean(&a, &b, op).map_err(geomx::geom_error)?;
    if rings.is_empty() {
        return Err(EngineError::bad(ID, "/ids", "the shapes do not overlap"));
    }
    commit_paths(s, "Boolean", &[bottom, top], rings)
}

/// `tools.offsetShapes`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OffsetParams {
    /// Object ids. Default: the selection.
    #[serde(default)]
    pub ids: Option<Vec<u64>>,
    /// Offset distance in mm (> 0, ≤ 1000). Outward grows, inward shrinks, open paths get a stroke this wide/2.
    pub distance_mm: f64,
    /// "outward" (default), "inward" or "both".
    #[serde(default)]
    pub direction: Option<String>,
    /// "round" (default), "sharp" (miter, limit 4) or "bevel".
    #[serde(default)]
    pub corner: Option<String>,
    /// Remove the originals. Default false.
    #[serde(default)]
    pub delete_original: Option<bool>,
}

fn offset_shapes(s: &mut Session, p: OffsetParams) -> Result<Value, EngineError> {
    const ID: &str = "tools.offsetShapes";
    let d = positive(ID, "/distanceMm", p.distance_mm, 1000.0)?;
    let direction = p.direction.as_deref().unwrap_or("outward");
    if !matches!(direction, "outward" | "inward" | "both") {
        return Err(EngineError::bad(ID, "/direction", format!("`{direction}` is not outward, inward or both")));
    }
    let join = match p.corner.as_deref().unwrap_or("round") {
        "round" => Join::Round,
        "sharp" => Join::Miter { limit: 4.0 },
        "bevel" => Join::Bevel,
        other => return Err(EngineError::bad(ID, "/corner", format!("`{other}` is not round, sharp or bevel"))),
    };
    let ids = super::target_ids(s, ID, p.ids)?;
    let tol = s.prefs().curve_tolerance_mm();
    let delete = p.delete_original.unwrap_or(false);
    let delete_idcs: std::collections::HashSet<u64> = if delete { ids.iter().map(|i| i.0).collect() } else { std::collections::HashSet::new() };
    // Per target, world-space rings: closed → offset ±d, open → stroke outline at half-width.
    let mut per: Vec<(usize, LayerKey, Vec<openlaser_geom::Polyline>)> = Vec::new();
    let mut vanished: Vec<u64> = Vec::new();
    for oid in &ids {
        let Some(sh) = s.doc()?.project.shapes.iter().enumerate().find(|(_, s)| s.id == *oid).map(|(i, s)| (i, s.clone())) else {
            continue;
        };
        if sh.1.locked {
            return Err(EngineError::disabled(ID, "a locked object cannot be offset; unlock it first"));
        }
        let polys = s.doc()?.project.world_polylines(*oid, tol).map_err(geomx::geom_error)?;
        let closed: Vec<openlaser_geom::Polyline> = polys.iter().filter(|pl| pl.is_closed(openlaser_geom::CLOSURE_TOL_MM)).cloned().collect();
        let mut rings = Vec::new();
        if closed.is_empty() {
            rings.extend(openlaser_geom::boolean::stroke_outline(&polys, d / 2.0, join, Cap::Round, tol).map_err(geomx::geom_error)?);
        } else {
            for sign in match direction {
                "both" => vec![d, -d],
                "inward" => vec![-d],
                _ => vec![d],
            } {
                rings.extend(openlaser_geom::boolean::offset(&closed, sign, join, tol).map_err(geomx::geom_error)?);
            }
        }
        if rings.is_empty() {
            vanished.push(oid.0);
        } else {
            per.push((sh.0, sh.1.layer, rings));
        }
    }
    if per.is_empty() {
        return Err(EngineError::bad(ID, "/ids", "every offset vanished; nothing to create"));
    }
    let created = s.edit(Label::new("Offset Shapes"), None, move |project, sel| {
        let mut fresh = Vec::with_capacity(per.len());
        for (slot, layer, rings) in &per {
            let id = project.alloc_id().map_err(geomx::doc_error)?;
            fresh.push(id);
            let mut shape = Shape {
                id,
                name: None,
                layer: *layer,
                xform: Affine::IDENTITY,
                geom: Geom::Path(Arc::new(rings_to_path(rings))),
                locked: false,
                power_scale: FULL_POWER,
                priority: 0,
            };
            shape.locked = false;
            project.shapes.insert((*slot + 1).min(project.shapes.len()), Arc::new(shape));
        }
        project.shapes.retain(|sh| !delete_idcs.contains(&sh.id.0));
        sel.ids = fresh.clone();
        sel.key = fresh.last().copied();
        Ok(fresh)
    })?;
    Ok(json!({ "ids": created, "vanished": vanished }))
}

/// `tools.rectangle`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RectangleParams {
    /// Lower-left corner x (design mm, Y up).
    pub x_mm: f64,
    /// Lower-left corner y.
    pub y_mm: f64,
    /// Width (> 0).
    pub w_mm: f64,
    /// Height (> 0).
    pub h_mm: f64,
    /// Corner radius (0..=min(w, h)/2). Default 0.
    #[serde(default)]
    pub corner_radius_mm: Option<f64>,
    /// Layer. Default: the current layer.
    #[serde(default)]
    #[schemars(with = "Option<LayerKeySchema>")]
    pub layer: Option<LayerKey>,
}

/// `tools.ellipse`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EllipseParams {
    /// Centre x (design mm).
    pub cx_mm: f64,
    /// Centre y.
    pub cy_mm: f64,
    /// Width (> 0).
    pub w_mm: f64,
    /// Height (> 0).
    pub h_mm: f64,
    /// Layer. Default: the current layer.
    #[serde(default)]
    #[schemars(with = "Option<LayerKeySchema>")]
    pub layer: Option<LayerKey>,
}

/// `tools.drawLines`: exactly one of `pointsMm` and `d`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DrawLinesParams {
    /// Polyline vertices in design mm (≥ 2).
    #[serde(default)]
    pub points_mm: Option<Vec<[f64; 2]>>,
    /// SVG path data in design mm (Y up as given).
    #[serde(default)]
    pub d: Option<String>,
    /// Close the polyline (pointsMm only). Default false.
    #[serde(default)]
    pub closed: Option<bool>,
    /// Layer. Default: the current layer.
    #[serde(default)]
    #[schemars(with = "Option<LayerKeySchema>")]
    pub layer: Option<LayerKey>,
}

fn layer_or_current(s: &Session, layer: Option<LayerKey>) -> Result<LayerKey, EngineError> {
    Ok(match layer {
        Some(l) => l,
        None => s.doc()?.current_layer,
    })
}

/// Adds one shape whose object-space bounding box is centred on `centre` (design mm); selects it.
fn add_centred(s: &mut Session, label: &'static str, layer: LayerKey, geom: Geom, centre: (f64, f64)) -> Result<Created, EngineError> {
    let probe = Shape { id: ObjectId(0), name: None, layer, xform: Affine::IDENTITY, geom, locked: false, power_scale: FULL_POWER, priority: 0 };
    let offset = match geomx::shape_extent(&probe) {
        Some(b) => Vec2::new(centre.0 - (b.min[0] + b.max[0]) / 2.0, centre.1 - (b.min[1] + b.max[1]) / 2.0),
        None => Vec2::new(centre.0, centre.1),
    };
    let geom = probe.geom;
    add_placed(s, label, layer, geom, Affine::translate(offset))
}

/// Adds one shape with the exact transform `xform`; selects it.
fn add_placed(s: &mut Session, label: &'static str, layer: LayerKey, geom: Geom, xform: Affine) -> Result<Created, EngineError> {
    let probe = Shape { id: ObjectId(0), name: None, layer, xform, geom, locked: false, power_scale: FULL_POWER, priority: 0 };
    let ids = s.edit(Label::new(label), None, move |project, sel| {
        let mut shape = probe;
        shape.id = project.alloc_id().map_err(geomx::doc_error)?;
        let id = shape.id;
        project.shapes.push(Arc::new(shape));
        sel.ids = vec![id];
        sel.key = Some(id);
        Ok(vec![id])
    })?;
    Ok(Created { ids })
}

fn mm(id: &str, pointer: &str, v: f64) -> Result<Mm, EngineError> {
    Mm::new(v).map_err(|e| EngineError::bad(id, pointer, e.to_string()))
}

fn rectangle(s: &mut Session, p: RectangleParams) -> Result<Created, EngineError> {
    const ID: &str = "tools.rectangle";
    let x = finite(ID, "/xMm", p.x_mm)?;
    let y = finite(ID, "/yMm", p.y_mm)?;
    let w = positive(ID, "/wMm", p.w_mm, MAX_SIZE_MM)?;
    let h = positive(ID, "/hMm", p.h_mm, MAX_SIZE_MM)?;
    let r = finite(ID, "/cornerRadiusMm", p.corner_radius_mm.unwrap_or(0.0))?;
    if r < 0.0 || r > w.min(h) / 2.0 {
        return Err(EngineError::bad(ID, "/cornerRadiusMm", format!("{r} is outside 0..={}", w.min(h) / 2.0)));
    }
    let layer = layer_or_current(s, p.layer)?;
    let geom = Geom::Rect { w: mm(ID, "/wMm", w)?, h: mm(ID, "/hMm", h)?, corner_radius: mm(ID, "/cornerRadiusMm", r)? };
    add_centred(s, "Rectangle", layer, geom, (x + w / 2.0, y + h / 2.0))
}

fn ellipse(s: &mut Session, p: EllipseParams) -> Result<Created, EngineError> {
    const ID: &str = "tools.ellipse";
    let cx = finite(ID, "/cxMm", p.cx_mm)?;
    let cy = finite(ID, "/cyMm", p.cy_mm)?;
    let w = positive(ID, "/wMm", p.w_mm, MAX_SIZE_MM)?;
    let h = positive(ID, "/hMm", p.h_mm, MAX_SIZE_MM)?;
    let layer = layer_or_current(s, p.layer)?;
    let geom = Geom::Ellipse { rx: mm(ID, "/wMm", w / 2.0)?, ry: mm(ID, "/hMm", h / 2.0)? };
    add_centred(s, "Ellipse", layer, geom, (cx, cy))
}

/// `tools.polygon`: a regular polygon, first vertex straight up unless `rotateDeg` turns it (CCW, Y up).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PolygonParams {
    /// Centre x (design mm).
    pub cx_mm: f64,
    /// Centre y.
    pub cy_mm: f64,
    /// Vertex radius (> 0).
    pub radius_mm: f64,
    /// Vertex count, 3..=64. Default 6.
    #[serde(default)]
    pub sides: Option<u16>,
    /// Rotation about the centre, degrees counter-clockwise. Default 0.
    #[serde(default)]
    pub rotate_deg: Option<f64>,
    /// Layer. Default: the current layer.
    #[serde(default)]
    #[schemars(with = "Option<LayerKeySchema>")]
    pub layer: Option<LayerKey>,
}

fn polygon(s: &mut Session, p: PolygonParams) -> Result<Created, EngineError> {
    const ID: &str = "tools.polygon";
    let cx = finite(ID, "/cxMm", p.cx_mm)?;
    let cy = finite(ID, "/cyMm", p.cy_mm)?;
    let radius = positive(ID, "/radiusMm", p.radius_mm, MAX_SIZE_MM)?;
    let sides = p.sides.unwrap_or(6);
    if !(3..=64).contains(&sides) {
        return Err(EngineError::bad(ID, "/sides", format!("{sides} is outside 3..=64")));
    }
    let rotate = finite(ID, "/rotateDeg", p.rotate_deg.unwrap_or(0.0))?;
    let layer = layer_or_current(s, p.layer)?;
    let geom = Geom::Polygon { sides, radius: mm(ID, "/radiusMm", radius)? };
    // The polygon's own centre (its circumcentre, the model's origin) lands on cx/cy, so the canvas drag from that
    // point and the shape agree. Centring the bounding box instead shifts an odd-sided or rotated polygon.
    add_placed(s, "Polygon", layer, geom, Affine::translate((cx, cy)) * Affine::rotate(rotate.to_radians()))
}

/// `tools.measure`: a pure query — never touches the document or history.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MeasureParams {
    /// Start point, design mm.
    pub from_mm: [f64; 2],
    /// End point, design mm.
    pub to_mm: [f64; 2],
}

fn measure(s: &mut Session, p: MeasureParams) -> Result<Value, EngineError> {
    const ID: &str = "tools.measure";
    let _ = s;
    let fx = finite(ID, "/fromMm/0", p.from_mm[0])?;
    let fy = finite(ID, "/fromMm/1", p.from_mm[1])?;
    let tx = finite(ID, "/toMm/0", p.to_mm[0])?;
    let ty = finite(ID, "/toMm/1", p.to_mm[1])?;
    let (dx, dy) = (tx - fx, ty - fy);
    Ok(json!({ "distanceMm": dx.hypot(dy), "dxMm": dx, "dyMm": dy, "angleDeg": dy.atan2(dx).to_degrees() }))
}

fn draw_lines(s: &mut Session, p: DrawLinesParams) -> Result<Value, EngineError> {
    const ID: &str = "tools.drawLines";
    let path = match (p.points_mm, p.d) {
        (Some(points), None) => {
            if points.len() < 2 {
                return Err(EngineError::bad(ID, "/pointsMm", "a line needs at least 2 points"));
            }
            if points.len() > MAX_POINTS {
                return Err(EngineError::bad(ID, "/pointsMm", format!("at most {MAX_POINTS} points per call")));
            }
            let mut path = BezPath::new();
            for (i, [x, y]) in points.iter().enumerate() {
                let x = finite(ID, &format!("/pointsMm/{i}/0"), *x)?;
                let y = finite(ID, &format!("/pointsMm/{i}/1"), *y)?;
                if i == 0 {
                    path.move_to((x, y));
                } else {
                    path.line_to((x, y));
                }
            }
            if p.closed.unwrap_or(false) {
                path.close_path();
            }
            path
        }
        (None, Some(d)) => {
            if p.closed.is_some() {
                return Err(EngineError::bad(ID, "/closed", "`closed` applies to pointsMm; close SVG path data with Z"));
            }
            if d.len() > 4 << 20 {
                return Err(EngineError::bad(ID, "/d", "path data is larger than 4 MiB"));
            }
            let path = BezPath::from_svg(&d).map_err(|e| EngineError::bad(ID, "/d", format!("not SVG path data: {e}")))?;
            let finite_ok = path.elements().iter().all(geomx::el_finite);
            if !finite_ok {
                return Err(EngineError::bad(ID, "/d", "path data contains a non-finite number"));
            }
            if path.elements().len() > MAX_POINTS {
                return Err(EngineError::bad(ID, "/d", format!("at most {MAX_POINTS} path elements per call")));
            }
            if path.elements().is_empty() {
                return Err(EngineError::bad(ID, "/d", "the path is empty"));
            }
            path
        }
        (Some(_), Some(_)) => return Err(EngineError::bad(ID, "", "give either pointsMm or d, not both")),
        (None, None) => return Err(EngineError::bad(ID, "", "give pointsMm or d")),
    };
    let layer = layer_or_current(s, p.layer)?;
    let ids = s.edit(Label::new("Draw Lines"), None, move |project, sel| {
        let id = project.alloc_id().map_err(geomx::doc_error)?;
        project.shapes.push(Arc::new(Shape {
            id,
            name: None,
            layer,
            xform: Affine::IDENTITY,
            geom: Geom::Path(Arc::new(path)),
            locked: false,
            power_scale: FULL_POWER,
            priority: 0,
        }));
        sel.ids = vec![id];
        sel.key = Some(id);
        Ok(vec![id])
    })?;
    Ok(json!({ "ids": ids }))
}
