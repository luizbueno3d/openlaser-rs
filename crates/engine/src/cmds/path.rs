//! Node-level path access (Appendix A): `path.get`/`path.set` read and replace a shape's path data in its
//! OBJECT space (the transform is unchanged), so agents and the node editor can read, modify and write `d`.

use std::sync::Arc;

use openlaser_doc::Geom;
use openlaser_types::ObjectId;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Created, needs_doc};
use crate::command::{Access, CommandSpec, EngineError, Kind};
use crate::session::{Label, Session};
use crate::{command, geomx};

/// `path.get` parameters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PathGetParams {
    /// The shape id.
    pub id: u64,
}

/// `path.set` parameters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PathSetParams {
    /// The shape id.
    pub id: u64,
    /// SVG path data in the shape's object space ("M…Z…"). Must start with MoveTo; every number finite.
    pub d: String,
    /// Consecutive sets sharing a key merge into one history entry (arrow-key nudges).
    #[serde(default)]
    pub coalesce: Option<String>,
}

fn path_get(s: &mut Session, p: PathGetParams) -> Result<Value, EngineError> {
    const ID: &str = "path.get";
    let oid = ObjectId(p.id);
    let Some(shape) = s.doc()?.project.shape(oid) else {
        return Err(EngineError::bad(ID, "/id", format!("object {} does not exist", oid.0)));
    };
    let Geom::Path(path) = &shape.geom else {
        return Err(EngineError::bad(ID, "/id", "convert to path first (Arrange > Convert to Path)"));
    };
    Ok(json!({ "d": path.to_svg(), "xform": shape.xform.as_coeffs() }))
}

fn path_set(s: &mut Session, p: PathSetParams) -> Result<Created, EngineError> {
    const ID: &str = "path.set";
    if p.d.len() > 4 << 20 {
        return Err(EngineError::bad(ID, "/d", "path data is larger than 4 MiB"));
    }
    let path = openlaser_doc::BezPath::from_svg(&p.d).map_err(|e| EngineError::bad(ID, "/d", format!("not SVG path data: {e}")))?;
    if !path.elements().iter().all(geomx::el_finite) {
        return Err(EngineError::bad(ID, "/d", "path data contains a non-finite number"));
    }
    if path.elements().len() > super::tools::MAX_POINTS {
        return Err(EngineError::bad(ID, "/d", format!("at most {} path elements", super::tools::MAX_POINTS)));
    }
    if !matches!(path.elements().first(), Some(openlaser_geom::PathEl::MoveTo(_))) {
        return Err(EngineError::bad(ID, "/d", "path data must start with a move"));
    }
    let oid = ObjectId(p.id);
    {
        let Some(shape) = s.doc()?.project.shape(oid) else {
            return Err(EngineError::bad(ID, "/id", format!("object {} does not exist", oid.0)));
        };
        if shape.locked {
            return Err(EngineError::disabled(ID, "the object is locked; unlock it first"));
        }
        if !matches!(shape.geom, Geom::Path(_)) {
            return Err(EngineError::bad(ID, "/id", "convert to path first (Arrange > Convert to Path)"));
        }
    }
    s.edit(Label::new("Edit Nodes"), p.coalesce.as_deref(), move |project, sel| {
        let Some(shape) = project.shapes.iter_mut().find(|sh| sh.id == oid) else {
            return Err(EngineError::bad(ID, "/id", "the shape vanished mid-edit"));
        };
        Arc::make_mut(shape).geom = Geom::Path(Arc::new(path));
        sel.ids = vec![oid];
        sel.key = Some(oid);
        Ok(Created { ids: vec![oid] })
    })
}

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        command!("path.get", "Get Path Data", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: needs_doc, params: PathGetParams, run: path_get, doc: "{id} → {d, xform:[a,b,c,d,e,f]} (object space)"),
        command!("path.set", "Set Path Data", menu: [], shortcut: None, kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: PathSetParams, run: path_set, doc: "{id, d, coalesce?} replaces the object-space path; one undo step"),
    ]
}
