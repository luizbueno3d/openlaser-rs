//! Arrange menu, Numeric Edits and arrow keys: flips, quarter turns, transform, nudge, rotate, page centre
//! (Appendix A). Every transform is one `edit`; locked objects never move.

use openlaser_doc::WorkspaceOrigin;
use openlaser_geom::{Affine, Vec2};
use openlaser_types::Bounds;
use schemars::JsonSchema;
use serde::Deserialize;

use super::{Changed, needs_doc, target_ids};
use crate::command::{Access, CommandSpec, EngineError, Kind};
use crate::params::{AnchorP, IdsParams, finite, positive};
use crate::session::{Label, Session};
use crate::{command, geomx};

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command!("arrange.flipH", "Flip Horizontal", menu: ["Arrange"], shortcut: Some("Cmd+Shift+H"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: IdsParams, run: flip_h, doc: "{ids?}"),
        command!("arrange.flipV", "Flip Vertical", menu: ["Arrange"], shortcut: Some("Cmd+Shift+V"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: IdsParams, run: flip_v, doc: "{ids?}"),
        command!("arrange.rotateCw90", "Rotate 90° Clockwise", menu: ["Arrange"], shortcut: Some("."), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: IdsParams, run: rotate_cw90, doc: "{ids?}"),
        command!("arrange.rotateCcw90", "Rotate 90° Counter-Clockwise", menu: ["Arrange"], shortcut: Some(","), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: IdsParams, run: rotate_ccw90, doc: "{ids?}"),
        command!("arrange.transform", "Numeric Edits", menu: [], shortcut: None, kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: TransformParams, run: transform,
            doc: "{ids?, xMm?, yMm?, wMm?, hMm?, scalePct?, rotateDeg?, anchor?, lockRatio?}"),
        command!("arrange.nudge", "Nudge", menu: [], shortcut: None, kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: NudgeParams, run: nudge, doc: "{ids?, dxMm, dyMm} (consecutive nudges are one undo step)"),
        command!("arrange.rotateBy", "Rotate", menu: [], shortcut: None, kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: RotateParams, run: rotate_by, doc: "{ids?, deg, anchor?} (counter-clockwise, Y up)"),
        command!("arrange.moveSelection.pageCenter", "Move Selected Objects to Page Center", menu: ["Arrange", "Move Selected Objects"],
            shortcut: Some("P"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: IdsParams, run: page_center, doc: "{ids?}"),
    ]
}

fn anchor_point(b: &Bounds, a: AnchorP) -> (f64, f64) {
    let (x0, y0, x1, y1) = (b.min[0], b.min[1], b.max[0], b.max[1]);
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    match a {
        AnchorP::Center => (cx, cy),
        AnchorP::TopLeft => (x0, y1),
        AnchorP::Top => (cx, y1),
        AnchorP::TopRight => (x1, y1),
        AnchorP::Left => (x0, cy),
        AnchorP::Right => (x1, cy),
        AnchorP::BottomLeft => (x0, y0),
        AnchorP::Bottom => (cx, y0),
        AnchorP::BottomRight => (x1, y0),
    }
}

fn about(m: Affine, at: (f64, f64)) -> Affine {
    Affine::translate((at.0, at.1)) * m * Affine::translate((-at.0, -at.1))
}

/// Resolves the targets and their bounds, builds the transform from the bounds and applies it as one edit.
fn transform_with(
    s: &mut Session,
    id: &str,
    label: &'static str,
    ids: Option<Vec<u64>>,
    coalesce: Option<String>,
    f: impl FnOnce(&Bounds) -> Result<Affine, EngineError>,
) -> Result<Changed, EngineError> {
    let targets = target_ids(s, id, ids)?;
    let tol = s.prefs().curve_tolerance_mm();
    let b = geomx::bounds(&s.doc()?.project, &targets, tol).ok_or_else(|| EngineError::bad(id, "/ids", "the selection has no measurable geometry"))?;
    let m = f(&b)?;
    if !m.as_coeffs().iter().all(|c| c.is_finite()) {
        return Err(EngineError::bad(id, "", "the transform is not finite"));
    }
    let moved = s.edit(Label::new(label), coalesce.as_deref(), |project, _| Ok(geomx::apply(project, &targets, m)))?;
    if moved == 0 {
        return Err(EngineError::disabled(id, "every selected object is locked"));
    }
    Ok(Changed { changed: moved })
}

fn flip_h(s: &mut Session, p: IdsParams) -> Result<Changed, EngineError> {
    let c = s.coalesce();
    transform_with(s, "arrange.flipH", "Flip Horizontal", p.ids, c, |b| Ok(about(Affine::scale_non_uniform(-1.0, 1.0), anchor_point(b, AnchorP::Center))))
}

fn flip_v(s: &mut Session, p: IdsParams) -> Result<Changed, EngineError> {
    let c = s.coalesce();
    transform_with(s, "arrange.flipV", "Flip Vertical", p.ids, c, |b| Ok(about(Affine::scale_non_uniform(1.0, -1.0), anchor_point(b, AnchorP::Center))))
}

fn rotate_cw90(s: &mut Session, p: IdsParams) -> Result<Changed, EngineError> {
    let c = s.coalesce();
    transform_with(s, "arrange.rotateCw90", "Rotate 90° CW", p.ids, c, |b| {
        Ok(about(Affine::rotate(-std::f64::consts::FRAC_PI_2), anchor_point(b, AnchorP::Center)))
    })
}

fn rotate_ccw90(s: &mut Session, p: IdsParams) -> Result<Changed, EngineError> {
    let c = s.coalesce();
    transform_with(s, "arrange.rotateCcw90", "Rotate 90° CCW", p.ids, c, |b| {
        Ok(about(Affine::rotate(std::f64::consts::FRAC_PI_2), anchor_point(b, AnchorP::Center)))
    })
}

/// `arrange.transform` (the Numeric Edits toolbar and the selection handles).
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TransformParams {
    /// Object ids. Default: the selection.
    #[serde(default)]
    pub ids: Option<Vec<u64>>,
    /// New x of the anchor (design mm).
    #[serde(default)]
    pub x_mm: Option<f64>,
    /// New y of the anchor.
    #[serde(default)]
    pub y_mm: Option<f64>,
    /// New width of the selection's bounding box (> 0).
    #[serde(default)]
    pub w_mm: Option<f64>,
    /// New height (> 0).
    #[serde(default)]
    pub h_mm: Option<f64>,
    /// Uniform scale in percent (> 0), applied after w/h.
    #[serde(default)]
    pub scale_pct: Option<f64>,
    /// Rotation about the anchor, degrees counter-clockwise (Y up).
    #[serde(default)]
    pub rotate_deg: Option<f64>,
    /// Reference point on the bounding box. Default "center".
    #[serde(default)]
    pub anchor: Option<AnchorP>,
    /// Keep the aspect ratio when only one of wMm/hMm is given. Default false.
    #[serde(default)]
    pub lock_ratio: Option<bool>,
}

fn transform(s: &mut Session, p: TransformParams) -> Result<Changed, EngineError> {
    const ID: &str = "arrange.transform";
    if p.x_mm.is_none() && p.y_mm.is_none() && p.w_mm.is_none() && p.h_mm.is_none() && p.scale_pct.is_none() && p.rotate_deg.is_none() {
        return Err(EngineError::bad(ID, "", "nothing to change: give xMm, yMm, wMm, hMm, scalePct or rotateDeg"));
    }
    let x = p.x_mm.map(|v| finite(ID, "/xMm", v)).transpose()?;
    let y = p.y_mm.map(|v| finite(ID, "/yMm", v)).transpose()?;
    let w = p.w_mm.map(|v| positive(ID, "/wMm", v, 100_000.0)).transpose()?;
    let h = p.h_mm.map(|v| positive(ID, "/hMm", v, 100_000.0)).transpose()?;
    let scale = p.scale_pct.map(|v| positive(ID, "/scalePct", v, 1.0e6)).transpose()?;
    let rot = p.rotate_deg.map(|v| finite(ID, "/rotateDeg", v)).transpose()?;
    let anchor = p.anchor.unwrap_or_default();
    let lock = p.lock_ratio.unwrap_or(false);
    let c = s.coalesce();
    transform_with(s, ID, "Transform", p.ids, c, |b| {
        let (bw, bh) = (b.max[0] - b.min[0], b.max[1] - b.min[1]);
        let mut sx = 1.0;
        let mut sy = 1.0;
        if let Some(w) = w {
            if bw <= 0.0 {
                return Err(EngineError::bad(ID, "/wMm", "the selection has no width to scale"));
            }
            sx = w / bw;
        }
        if let Some(h) = h {
            if bh <= 0.0 {
                return Err(EngineError::bad(ID, "/hMm", "the selection has no height to scale"));
            }
            sy = h / bh;
        }
        if lock {
            match (w, h) {
                (Some(_), None) => sy = sx,
                (None, Some(_)) => sx = sy,
                _ => {}
            }
        }
        if let Some(k) = scale {
            sx *= k / 100.0;
            sy *= k / 100.0;
        }
        let a = anchor_point(b, anchor);
        let to = (x.unwrap_or(a.0), y.unwrap_or(a.1));
        let r = rot.map_or(Affine::IDENTITY, |d| Affine::rotate(d.to_radians()));
        Ok(Affine::translate((to.0, to.1)) * r * Affine::scale_non_uniform(sx, sy) * Affine::translate((-a.0, -a.1)))
    })
}

/// `arrange.nudge`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct NudgeParams {
    /// Object ids. Default: the selection.
    #[serde(default)]
    pub ids: Option<Vec<u64>>,
    /// Move right by this many mm (negative: left).
    pub dx_mm: f64,
    /// Move up by this many mm (negative: down).
    pub dy_mm: f64,
}

fn nudge(s: &mut Session, p: NudgeParams) -> Result<Changed, EngineError> {
    const ID: &str = "arrange.nudge";
    let dx = finite(ID, "/dxMm", p.dx_mm)?;
    let dy = finite(ID, "/dyMm", p.dy_mm)?;
    let c = s.coalesce().or_else(|| Some(ID.to_owned()));
    transform_with(s, ID, "Move", p.ids, c, |_| Ok(Affine::translate(Vec2::new(dx, dy))))
}

/// `arrange.rotateBy`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RotateParams {
    /// Object ids. Default: the selection.
    #[serde(default)]
    pub ids: Option<Vec<u64>>,
    /// Degrees, counter-clockwise (Y up).
    pub deg: f64,
    /// Pivot on the bounding box. Default "center".
    #[serde(default)]
    pub anchor: Option<AnchorP>,
}

fn rotate_by(s: &mut Session, p: RotateParams) -> Result<Changed, EngineError> {
    const ID: &str = "arrange.rotateBy";
    let deg = finite(ID, "/deg", p.deg)?;
    let anchor = p.anchor.unwrap_or_default();
    let c = s.coalesce();
    transform_with(s, ID, "Rotate", p.ids, c, |b| Ok(about(Affine::rotate(deg.to_radians()), anchor_point(b, anchor))))
}

fn page_center(s: &mut Session, p: IdsParams) -> Result<Changed, EngineError> {
    const ID: &str = "arrange.moveSelection.pageCenter";
    let ws = s.doc()?.project.workspace.clone();
    let [w, h] = ws.size_mm;
    let page = match ws.origin {
        WorkspaceOrigin::Center => (0.0, 0.0),
        WorkspaceOrigin::BottomLeft => (w / 2.0, h / 2.0),
        WorkspaceOrigin::TopLeft => (w / 2.0, -h / 2.0),
    };
    let c = s.coalesce();
    transform_with(s, ID, "Move to Page Center", p.ids, c, |b| {
        let (cx, cy) = anchor_point(b, AnchorP::Center);
        Ok(Affine::translate((page.0 - cx, page.1 - cy)))
    })
}
