//! Tools and the Creation toolbar: Select, Rectangle, Ellipse, Draw Lines (Appendix A). With params, a drawing
//! command creates the shape (the agent's way to draw); the GUI's toolbar arms the tool instead.

use std::sync::Arc;

use openlaser_doc::{Geom, Shape};
use openlaser_geom::{Affine, BezPath, Vec2};
use openlaser_types::{LayerKey, Mm, ObjectId, Percent};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Created, always, needs_doc};
use crate::command::{Access, CommandSpec, EngineError, Kind};
use crate::params::{LayerKeySchema, NoParams, finite, positive};
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
    ]
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

/// Adds one shape whose object-space bounding box is centred on `centre` (design mm); selects it. `pre` is an
/// extra object-space transform (e.g. a polygon's rotation) composed under the centring translate.
fn add_centred(s: &mut Session, label: &'static str, layer: LayerKey, geom: Geom, centre: (f64, f64), pre: Option<Affine>) -> Result<Created, EngineError> {
    let probe = Shape { id: ObjectId(0), name: None, layer, xform: Affine::IDENTITY, geom, locked: false, power_scale: FULL_POWER, priority: 0 };
    let offset = match geomx::shape_extent(&probe) {
        Some(b) => Vec2::new(centre.0 - (b.min[0] + b.max[0]) / 2.0, centre.1 - (b.min[1] + b.max[1]) / 2.0),
        None => Vec2::new(centre.0, centre.1),
    };
    let ids = s.edit(Label::new(label), None, move |project, sel| {
        let mut shape = probe;
        shape.id = project.alloc_id().map_err(geomx::doc_error)?;
        shape.xform = Affine::translate(offset) * pre.unwrap_or(Affine::IDENTITY);
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
    add_centred(s, "Rectangle", layer, geom, (x + w / 2.0, y + h / 2.0), None)
}

fn ellipse(s: &mut Session, p: EllipseParams) -> Result<Created, EngineError> {
    const ID: &str = "tools.ellipse";
    let cx = finite(ID, "/cxMm", p.cx_mm)?;
    let cy = finite(ID, "/cyMm", p.cy_mm)?;
    let w = positive(ID, "/wMm", p.w_mm, MAX_SIZE_MM)?;
    let h = positive(ID, "/hMm", p.h_mm, MAX_SIZE_MM)?;
    let layer = layer_or_current(s, p.layer)?;
    let geom = Geom::Ellipse { rx: mm(ID, "/wMm", w / 2.0)?, ry: mm(ID, "/hMm", h / 2.0)? };
    add_centred(s, "Ellipse", layer, geom, (cx, cy), None)
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
    add_centred(s, "Polygon", layer, geom, (cx, cy), if rotate == 0.0 { None } else { Some(Affine::rotate(rotate.to_radians())) })
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
