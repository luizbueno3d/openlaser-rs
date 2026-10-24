//! Edit menu and selection: Undo, Redo, Select All, select, Delete, Duplicate, Close Path, Auto-Join, Delete
//! Duplicates, Settings dialogs (Appendix A).

use std::collections::BTreeMap;
use std::sync::Arc;

use openlaser_doc::{Geom, Shape};
use openlaser_geom::{BezPath, HitIndex, Polyline};
use openlaser_types::{Bounds, LayerKey, ObjectId, Percent};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Changed, Created, always, needs_doc, target_ids};
use crate::command::{Access, CommandSpec, Disabled, EngineError, Kind};
use crate::params::{IdsParams, NoParams, RectMm, SelectModeP, finite, positive};
use crate::session::{Label, Session};
use crate::{command, geomx};

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command!("edit.undo", "Undo", menu: ["Edit"], shortcut: Some("Cmd+Z"), kind: Kind::Edit, access: Access::Agent,
            enabled: can_undo, params: NoParams, run: undo, doc: "{}"),
        command!("edit.redo", "Redo", menu: ["Edit"], shortcut: Some("Cmd+Shift+Z"), kind: Kind::Edit, access: Access::Agent,
            enabled: can_redo, params: NoParams, run: redo, doc: "{}"),
        command!("edit.selectAll", "Select All", menu: ["Edit"], shortcut: Some("Cmd+A"), kind: Kind::View, access: Access::Agent,
            enabled: needs_doc, params: NoParams, run: select_all, doc: "{}"),
        command!("edit.select", "Select", menu: [], shortcut: None, kind: Kind::View, access: Access::Agent,
            enabled: needs_doc, params: SelectParams, run: select, doc: "{ids?, rectMm?:{min:[x,y],max:[x,y]}, crossing?, mode?:\"replace\"|\"add\"|\"remove\"|\"toggle\"}"),
        command!("edit.selectNone", "Select None", menu: [], shortcut: Some("Escape"), kind: Kind::View, access: Access::Agent,
            enabled: needs_doc, params: NoParams, run: select_none, doc: "{}"),
        command!("edit.delete", "Delete", menu: ["Edit"], shortcut: Some("Backspace"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: IdsParams, run: delete, doc: "{ids?}"),
        command!("edit.duplicate", "Duplicate", menu: ["Edit"], shortcut: Some("Cmd+D"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: IdsParams, run: duplicate, doc: "{ids?}"),
        command!("edit.closePath", "Close Path", menu: ["Edit"], shortcut: None, kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: ToleranceParams, run: close_path, doc: "{ids?, toleranceMm?: 0.5}"),
        command!("edit.autoJoin", "Auto-Join Selected Shapes", menu: ["Edit"], shortcut: Some("Alt+J"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: ToleranceParams, run: auto_join, doc: "{ids?, toleranceMm?: 0.05}"),
        command!("edit.deleteDuplicates", "Delete Duplicates", menu: ["Edit"], shortcut: Some("Alt+D"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: IdsParams, run: delete_duplicates, doc: "{ids?} (default: the selection, else every object)"),
        command!("edit.settings", "Settings…", menu: ["OpenLaser"], shortcut: Some("Cmd+,"), kind: Kind::View, access: Access::Ui,
            enabled: always, params: NoParams, run: |_s: &mut Session, _p: NoParams| Ok::<_, EngineError>(json!({ "ui": "edit.settings" })), doc: "{}"),
        command!("edit.deviceSettings", "Device Settings…", menu: ["Edit"], shortcut: None, kind: Kind::View, access: Access::Ui,
            enabled: always, params: NoParams, run: |_s: &mut Session, _p: NoParams| Ok::<_, EngineError>(json!({ "ui": "edit.deviceSettings" })), doc: "{}"),
    ]
}

fn can_undo(s: &Session) -> Result<(), Disabled> {
    needs_doc(s)?;
    if s.active().is_some_and(|d| d.history.can_undo()) { Ok(()) } else { Err(Disabled::new("nothing to undo")) }
}

fn can_redo(s: &Session) -> Result<(), Disabled> {
    needs_doc(s)?;
    if s.active().is_some_and(|d| d.history.can_redo()) { Ok(()) } else { Err(Disabled::new("nothing to redo")) }
}

fn history_json(s: &Session) -> Value {
    s.active().map_or(Value::Null, |d| json!({ "history": d.history.labels(), "canUndo": d.history.can_undo(), "canRedo": d.history.can_redo() }))
}

fn undo(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    s.undo()?;
    Ok(history_json(s))
}

fn redo(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    s.redo()?;
    Ok(history_json(s))
}

/// Through `Session::set_selection_of`: under Cut Selected Graphics a new selection is a new job (revision bump).
fn set_selection(s: &mut Session, ids: Vec<ObjectId>) -> Result<Value, EngineError> {
    let index = s.doc_index()?;
    let mut selection = s.doc()?.selection.clone();
    selection.key = ids.last().copied();
    selection.ids = ids;
    s.set_selection_of(index, selection)?;
    Ok(json!({ "selection": s.doc()?.selection.ids }))
}

fn select_all(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    let ids = s.doc()?.project.shapes.iter().map(|sh| sh.id).collect();
    set_selection(s, ids)
}

fn select_none(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    set_selection(s, Vec::new())
}

/// `edit.select`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SelectParams {
    /// Object ids to select.
    #[serde(default)]
    pub ids: Option<Vec<u64>>,
    /// Marquee in design mm (Y up).
    #[serde(default)]
    pub rect_mm: Option<RectMm>,
    /// Marquee: also pick objects that cross it (default false: fully enclosed only).
    #[serde(default)]
    pub crossing: Option<bool>,
    /// How the picked objects combine with the selection. Default "replace".
    #[serde(default)]
    pub mode: Option<SelectModeP>,
}

fn select(s: &mut Session, p: SelectParams) -> Result<Value, EngineError> {
    const ID: &str = "edit.select";
    if p.ids.is_none() && p.rect_mm.is_none() {
        return Err(EngineError::bad(ID, "", "give `ids`, `rectMm` or both"));
    }
    let mut picked: Vec<ObjectId> = match p.ids {
        Some(list) => target_ids(s, ID, Some(list)).or_else(|e| if matches!(e, EngineError::Disabled { .. }) { Ok(Vec::new()) } else { Err(e) })?,
        None => Vec::new(),
    };
    if let Some(r) = p.rect_mm {
        let (x0, y0) = (finite(ID, "/rectMm/min/0", r.min[0])?, finite(ID, "/rectMm/min/1", r.min[1])?);
        let (x1, y1) = (finite(ID, "/rectMm/max/0", r.max[0])?, finite(ID, "/rectMm/max/1", r.max[1])?);
        if x1 < x0 || y1 < y0 {
            return Err(EngineError::bad(ID, "/rectMm", "max must not be below min"));
        }
        let tol = s.prefs().curve_tolerance_mm();
        let project = &s.doc()?.project;
        let mut polys: Vec<(ObjectId, Vec<Polyline>)> = Vec::with_capacity(project.shapes.len());
        for sh in &project.shapes {
            polys.push((sh.id, project.world_polylines(sh.id, tol).map_err(geomx::geom_error)?));
        }
        let index = HitIndex::build(polys.iter().map(|(id, p)| (*id, p.as_slice())));
        for id in index.in_rect(Bounds { min: [x0, y0], max: [x1, y1] }, p.crossing.unwrap_or(false)) {
            if !picked.contains(&id) {
                picked.push(id);
            }
        }
    }
    let current = s.doc()?.selection.ids.clone();
    let next: Vec<ObjectId> = match p.mode.unwrap_or_default() {
        SelectModeP::Replace => picked,
        SelectModeP::Add => current.iter().copied().chain(picked.into_iter().filter(|i| !current.contains(i))).collect(),
        SelectModeP::Remove => current.into_iter().filter(|i| !picked.contains(i)).collect(),
        SelectModeP::Toggle => {
            let mut v: Vec<ObjectId> = current.iter().copied().filter(|i| !picked.contains(i)).collect();
            v.extend(picked.into_iter().filter(|i| !current.contains(i)));
            v
        }
    };
    set_selection(s, next)
}

fn delete(s: &mut Session, p: IdsParams) -> Result<Changed, EngineError> {
    let ids = target_ids(s, "edit.delete", p.ids)?;
    s.edit(Label::new("Delete"), None, |project, sel| {
        let before = project.shapes.len();
        project.shapes.retain(|sh| !ids.contains(&sh.id));
        sel.ids.retain(|i| !ids.contains(i));
        Ok(Changed { changed: before - project.shapes.len() })
    })
}

fn duplicate(s: &mut Session, p: IdsParams) -> Result<Created, EngineError> {
    let ids = target_ids(s, "edit.duplicate", p.ids)?;
    let ids = s.edit(Label::new("Duplicate"), None, |project, sel| {
        let originals: Vec<Arc<Shape>> = project.shapes.iter().filter(|sh| ids.contains(&sh.id)).cloned().collect();
        let mut fresh = Vec::with_capacity(originals.len());
        for o in originals {
            let copy = geomx::duplicate(project, &o)?;
            fresh.push(copy.id);
            project.shapes.push(Arc::new(copy));
        }
        sel.ids = fresh.clone();
        sel.key = fresh.last().copied();
        Ok(fresh)
    })?;
    Ok(Created { ids })
}

/// `edit.closePath`, `edit.autoJoin`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ToleranceParams {
    /// Object ids. Default: the selection.
    #[serde(default)]
    pub ids: Option<Vec<u64>>,
    /// Largest gap to close or join, in mm (> 0, ≤ 100).
    #[serde(default)]
    pub tolerance_mm: Option<f64>,
}

fn close_in(shape: &Shape, parent: openlaser_geom::Affine, tol: f64, closed: &mut usize) -> Shape {
    let xf = parent * shape.xform;
    let mut s = shape.clone();
    match &shape.geom {
        Geom::Path(path) => {
            let (q, n) = geomx::close_subpaths(path, xf, tol);
            if n > 0 {
                *closed += n;
                s.geom = Geom::Path(Arc::new(q));
            }
        }
        Geom::Group(children) => {
            s.geom = Geom::Group(children.iter().map(|c| Arc::new(close_in(c, xf, tol, closed))).collect());
        }
        _ => {}
    }
    s
}

fn close_path(s: &mut Session, p: ToleranceParams) -> Result<Value, EngineError> {
    const ID: &str = "edit.closePath";
    let tol = positive(ID, "/toleranceMm", p.tolerance_mm.unwrap_or(0.5), 100.0)?;
    let ids = target_ids(s, ID, p.ids)?;
    let closed = s.edit(Label::new("Close Path"), None, |project, _| {
        let mut closed = 0;
        for arc in &mut project.shapes {
            if ids.contains(&arc.id) && !arc.locked {
                let next = close_in(arc, openlaser_geom::Affine::IDENTITY, tol, &mut closed);
                *arc = Arc::new(next);
            }
        }
        Ok(closed)
    })?;
    Ok(json!({ "closed": closed }))
}

fn polyline_path(polys: &[Polyline]) -> BezPath {
    let mut path = BezPath::new();
    for pl in polys {
        let closed = pl.is_closed(1e-9);
        for (i, pt) in pl.0.iter().enumerate() {
            if i == 0 {
                path.move_to((pt[0], pt[1]));
            } else {
                path.line_to((pt[0], pt[1]));
            }
        }
        if closed {
            path.close_path();
        }
    }
    path
}

const FULL_POWER: Percent = match Percent::new_const(100.0) {
    Some(p) => p,
    None => Percent::MAX_VALID,
};

fn auto_join(s: &mut Session, p: ToleranceParams) -> Result<Value, EngineError> {
    const ID: &str = "edit.autoJoin";
    let tol = positive(ID, "/toleranceMm", p.tolerance_mm.unwrap_or(0.05), 100.0)?;
    let ids = target_ids(s, ID, p.ids)?;
    let flat_tol = s.prefs().curve_tolerance_mm();
    let (created, joined) = s.edit(Label::new("Auto-Join"), None, |project, sel| {
        // Per layer: the unlocked path objects' polylines, joined end to end.
        let mut per_layer: BTreeMap<LayerKey, (Vec<ObjectId>, Vec<Polyline>)> = BTreeMap::new();
        for sh in project.shapes.iter().filter(|sh| ids.contains(&sh.id) && !sh.locked && matches!(sh.geom, Geom::Path(_) | Geom::Group(_))) {
            let entry = per_layer.entry(sh.layer).or_default();
            entry.0.push(sh.id);
            entry.1.extend(project.world_polylines(sh.id, flat_tol).map_err(geomx::geom_error)?);
        }
        let mut created = Vec::new();
        let mut joined = 0usize;
        for (layer, (owners, polys)) in per_layer {
            let out = openlaser_geom::join_lines(&polys, tol);
            if out.is_empty() || out.len() >= polys.len() {
                continue;
            }
            joined += polys.len() - out.len();
            project.shapes.retain(|sh| !owners.contains(&sh.id));
            let id = project.alloc_id().map_err(geomx::doc_error)?;
            project.shapes.push(Arc::new(Shape {
                id,
                name: None,
                layer,
                xform: openlaser_geom::Affine::IDENTITY,
                geom: Geom::Path(Arc::new(polyline_path(&out))),
                locked: false,
                power_scale: FULL_POWER,
                priority: 0,
            }));
            created.push(id);
        }
        if !created.is_empty() {
            sel.ids = created.clone();
            sel.key = created.last().copied();
        }
        Ok((created, joined))
    })?;
    Ok(json!({ "ids": created, "joined": joined }))
}

fn delete_duplicates(s: &mut Session, p: IdsParams) -> Result<Changed, EngineError> {
    const ID: &str = "edit.deleteDuplicates";
    let ids: Vec<ObjectId> = match p.ids {
        Some(list) => target_ids(s, ID, Some(list))?,
        None => {
            let d = s.doc()?;
            if d.selection.ids.is_empty() { d.project.shapes.iter().map(|sh| sh.id).collect() } else { d.selection.ids.clone() }
        }
    };
    let tol = s.prefs().curve_tolerance_mm();
    s.edit(Label::new("Delete Duplicates"), None, |project, _| {
        let mut owners = Vec::new();
        let mut polys = Vec::new();
        for id in &ids {
            for pl in project.world_polylines(*id, tol).map_err(geomx::geom_error)? {
                owners.push(*id);
                polys.push(pl);
            }
        }
        let dups = openlaser_geom::duplicates(&polys, openlaser_geom::CLOSURE_TOL_MM);
        let doomed: Vec<ObjectId> = ids
            .iter()
            .copied()
            .filter(|id| {
                let mine: Vec<usize> = owners.iter().enumerate().filter(|(_, o)| *o == id).map(|(i, _)| i).collect();
                !mine.is_empty() && mine.iter().all(|i| dups.contains(i))
            })
            .collect();
        let before = project.shapes.len();
        project.shapes.retain(|sh| !doomed.contains(&sh.id) || sh.locked);
        Ok(Changed { changed: before - project.shapes.len() })
    })
}
