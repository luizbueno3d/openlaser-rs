//! File menu: New, Open, Import, Save, Save As, Save GCode, Quit (Appendix A).

use openlaser_doc::{ColorLayer, Project};
use openlaser_import::{DxfUnits, Format, ImportOptions, Imported, SvgUnits};
use openlaser_types::{LayerKey, MachineId, Notice, NoticeCode, ObjectId};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{always, needs_doc};
use crate::command::{Access, CommandSpec, Disabled, EngineError, Kind};
use crate::files::{extension, file_name, sibling};
use crate::jobs::{Encoded, JobOptions};
use crate::params::{LayerKeySchema, NoParams, PathParams, SvgUnitsP, finite, name, positive};
use crate::session::{Label, Session};
use crate::{command, geomx};

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command!("file.new", "New", menu: ["File"], shortcut: Some("Cmd+N"), kind: Kind::Edit, access: Access::Agent,
            enabled: always, params: NewParams, run: new, doc: "{name?, machine?}"),
        command!("file.open", "Open…", menu: ["File"], shortcut: Some("Cmd+O"), kind: Kind::Io, access: Access::Roots,
            enabled: always, params: PathParams, run: open, doc: "{path} (.olp, or a design file opened as a new project)"),
        command!("file.import", "Import…", menu: ["File"], shortcut: Some("Cmd+I"), kind: Kind::Edit, access: Access::Roots,
            enabled: needs_doc, params: ImportParams, run: import, doc: "{path, layer?, centerMm?:[x,y], scale?, svgUnits?:\"physical\"|\"px96\"|\"px72\"}"),
        command!("file.save", "Save", menu: ["File"], shortcut: Some("Cmd+S"), kind: Kind::Io, access: Access::Roots,
            enabled: has_path, params: NoParams, run: save, doc: "{}"),
        command!("file.saveAs", "Save As…", menu: ["File"], shortcut: Some("Cmd+Shift+S"), kind: Kind::Io, access: Access::Roots,
            enabled: needs_doc, params: PathParams, run: save_as, doc: "{path} (.olp)"),
        command!("file.saveMachineFile", "Save GCode…", menu: ["File"], shortcut: Some("Alt+Shift+L"), kind: Kind::Io, access: Access::Roots,
            enabled: needs_doc, params: MachineFileParams, run: save_machine_file, doc: "{path, selectedOnly?} (GRBL profiles)"),
        command!("file.exit", "Quit OpenLaser", menu: ["OpenLaser"], shortcut: Some("Cmd+Q"), kind: Kind::View, access: Access::Ui,
            enabled: always, params: NoParams, run: exit, doc: "{} (stops every machine first if one is active)"),
    ]
}

fn has_path(s: &Session) -> Result<(), Disabled> {
    needs_doc(s)?;
    if s.active().is_some_and(|d| d.path.is_some()) { Ok(()) } else { Err(Disabled::new("this document was never saved; use Save As")) }
}

/// `file.new`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct NewParams {
    /// Project name. Default "Untitled".
    #[serde(default)]
    pub name: Option<String>,
    /// Machine profile id (`device.list`). Default: the session's first machine.
    #[serde(default)]
    pub machine: Option<String>,
}

fn new(s: &mut Session, p: NewParams) -> Result<Value, EngineError> {
    const ID: &str = "file.new";
    let machine = match p.machine {
        Some(m) => MachineId(m),
        None => s.default_machine().ok_or_else(|| EngineError::disabled(ID, "this session has no machine profiles"))?,
    };
    let profile = s.profile(&machine).cloned().ok_or_else(|| EngineError::bad(ID, "/machine", format!("no machine profile `{}`", machine.0)))?;
    let title = match p.name {
        Some(n) => name(ID, "/name", n.trim(), 256)?,
        None => "Untitled".to_owned(),
    };
    let project = Session::new_project(&profile, &title);
    let index = s.open_project(project, None, Some(machine.clone()))?;
    Ok(json!({ "index": index, "name": title, "machine": machine.0 }))
}

/// `file.import`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ImportParams {
    /// File path (SVG, DXF, PNG/JPEG/BMP/TIFF/WebP/GIF, or an OpenMopa `workspace.json`).
    pub path: String,
    /// Put every imported object on this layer. Default: colour → palette match, else the current layer.
    #[serde(default)]
    #[schemars(with = "Option<LayerKeySchema>")]
    pub layer: Option<LayerKey>,
    /// Centre the import here (design mm). Default: file coordinates (SVG/DXF), the origin (images).
    #[serde(default)]
    pub center_mm: Option<[f64; 2]>,
    /// Scale factor (> 0). Default 1.
    #[serde(default)]
    pub scale: Option<f64>,
    /// SVG units. Default "physical": an SVG without physical units is refused.
    #[serde(default)]
    pub svg_units: Option<SvgUnitsP>,
}

/// What an import needs besides the bytes.
pub(crate) struct ImportArgs {
    pub layer: Option<LayerKey>,
    pub center: Option<[f64; 2]>,
    pub scale: f64,
    pub svg_units: SvgUnits,
}

fn import_error(e: openlaser_import::Error) -> EngineError {
    EngineError::Io(format!("import: {e}"))
}

fn is_om_workspace(name: &str, bytes: &[u8]) -> bool {
    openlaser_import::sniff(name, bytes) == Some(Format::OmWorkspace) || name.eq_ignore_ascii_case("workspace.json")
}

/// Reads a design file into `Imported` (shapes with id 0). An OpenMopa workspace takes `layer_settings.json` from
/// its folder, else OM's four built-in layers with a notice (Appendix B).
pub(crate) fn import_bytes(s: &Session, path: &str, bytes: &[u8], args: &ImportArgs, current_layer: LayerKey) -> Result<Imported, EngineError> {
    let fname = file_name(path);
    let mut imported = if is_om_workspace(&fname, bytes) {
        let ws = openlaser_import::om::parse_workspace(bytes).map_err(import_error)?;
        let settings_path = sibling(path, "layer_settings.json");
        let (layers, missing) = match s.read_file(&settings_path) {
            Ok(b) => (openlaser_import::om::parse_layer_settings(&b).map_err(import_error)?, None),
            Err(e) => (openlaser_import::om::builtin_layers(), Some(e)),
        };
        let mut imp = openlaser_import::om::to_imported(&ws, &layers).map_err(import_error)?;
        if let Some(e) = missing {
            imp.notices.push(Notice {
                n: 0,
                code: NoticeCode::ImportEntitySkipped,
                message: format!("no usable layer_settings.json next to {fname} ({e}); OpenMopa's four built-in layers were used"),
                object: None,
                layer: None,
            });
        }
        imp
    } else {
        let opts = ImportOptions {
            layer: args.layer,
            current_layer,
            center_at: args.center,
            scale: args.scale,
            svg_units: args.svg_units,
            dxf_units: DxfUnits::Auto,
            flatten_tol_mm: s.prefs().curve_tolerance_mm(),
        };
        openlaser_import::import(&fname, bytes, &opts).map_err(import_error)?
    };
    if let Some(layer) = args.layer {
        imported.shapes = imported.shapes.iter().map(|sh| geomx::set_layer(sh, layer)).collect();
    }
    for (i, n) in imported.notices.iter_mut().enumerate() {
        n.n = u32::try_from(i + 1).unwrap_or(u32::MAX);
    }
    Ok(imported)
}

/// Adds imported shapes (fresh ids) and the layer settings a file carries; returns the new top-level ids.
pub(crate) fn apply_import(project: &mut Project, imported: Imported) -> Result<Vec<ObjectId>, EngineError> {
    for (key, layer) in imported.layers {
        match key {
            LayerKey::Color(i) => {
                let Some(slot) = project.color_layer_mut(i) else {
                    return Err(EngineError::Io(format!("the project has no colour layer {key} for the imported settings")));
                };
                *slot = ColorLayer { index: i, ..layer };
            }
            LayerKey::Tool(w) => {
                if let Some(t) = project.layers.tools.iter_mut().find(|t| t.which == w) {
                    t.name = layer.name.clone();
                    t.show = layer.show;
                }
            }
        }
    }
    let mut ids = Vec::with_capacity(imported.shapes.len());
    for shape in imported.shapes {
        let s = geomx::assign_ids(project, shape);
        ids.push(s.id);
        project.shapes.push(std::sync::Arc::new(s));
    }
    Ok(ids)
}

fn import_args(id: &str, center: Option<[f64; 2]>, scale: Option<f64>, units: Option<SvgUnitsP>, layer: Option<LayerKey>) -> Result<ImportArgs, EngineError> {
    let center = match center {
        Some([x, y]) => Some([finite(id, "/centerMm/0", x)?, finite(id, "/centerMm/1", y)?]),
        None => None,
    };
    let scale = positive(id, "/scale", scale.unwrap_or(1.0), 1.0e6)?;
    let svg_units = match units.unwrap_or_default() {
        SvgUnitsP::Physical => SvgUnits::RequirePhysical,
        SvgUnitsP::Px96 => SvgUnits::Px96,
        SvgUnitsP::Px72 => SvgUnits::Px72,
    };
    Ok(ImportArgs { layer, center, scale, svg_units })
}

fn import(s: &mut Session, p: ImportParams) -> Result<Value, EngineError> {
    const ID: &str = "file.import";
    let args = import_args(ID, p.center_mm, p.scale, p.svg_units, p.layer)?;
    let bytes = s.read_file(&p.path)?;
    let current = s.doc()?.current_layer;
    let imported = import_bytes(s, &p.path, &bytes, &args, current)?;
    let notices = imported.notices.clone();
    let bounds = imported.bounds;
    let ids = s.edit(Label::with("Import {name}", file_name(&p.path)), None, move |project, sel| {
        let ids = apply_import(project, imported)?;
        sel.ids = ids.clone();
        sel.key = ids.last().copied();
        Ok(ids)
    })?;
    Ok(json!({ "objects": ids, "bounds": bounds, "notices": notices }))
}

fn open(s: &mut Session, p: PathParams) -> Result<Value, EngineError> {
    const ID: &str = "file.open";
    let bytes = s.read_file(&p.path)?;
    let is_olp = extension(&p.path) == "olp";
    let (project, ids_len, notices) = if is_olp {
        let project = openlaser_format::load(&bytes).map_err(|e| EngineError::Io(format!("{}: {e}", p.path)))?;
        let n = project.shapes.len();
        (project, n, Vec::new())
    } else {
        let machine = s.default_machine().ok_or_else(|| EngineError::disabled(ID, "this session has no machine profiles"))?;
        let profile = s.profile(&machine).cloned().ok_or_else(|| EngineError::disabled(ID, "this session has no machine profiles"))?;
        let stem = std::path::Path::new(&p.path).file_stem().map_or_else(|| "Untitled".to_owned(), |n| n.to_string_lossy().into_owned());
        let mut project = Session::new_project(&profile, &stem);
        let args = import_args(ID, None, None, None, None)?;
        let imported = import_bytes(s, &p.path, &bytes, &args, LayerKey::Color(0))?;
        let notices = imported.notices.clone();
        let ids = apply_import(&mut project, imported)?;
        project.normalize();
        (project, ids.len(), notices)
    };
    let name = project.name.clone();
    let layers = crate::inspect::layer_order(&project).len();
    let index = s.open_project(project, is_olp.then(|| p.path.clone()), None)?;
    Ok(json!({ "index": index, "name": name, "layers": layers, "objects": ids_len, "notices": notices }))
}

fn write_project(s: &mut Session, id: &str, path: String) -> Result<Value, EngineError> {
    if extension(&path) != "olp" {
        return Err(EngineError::bad(id, "/path", "projects are saved as .olp; use file.saveMachineFile for G-code"));
    }
    let bytes = openlaser_format::save(&s.doc()?.project).map_err(|e| EngineError::Io(format!("{path}: {e}")))?;
    s.write_file(&path, &bytes)?;
    let d = s.doc_mut()?;
    d.path = Some(path.clone());
    d.saved_revision = d.revision;
    Ok(json!({ "path": path, "warnings": [] }))
}

fn save(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    let path = s.doc()?.path.clone().ok_or_else(|| EngineError::disabled("file.save", "this document was never saved; use Save As"))?;
    write_project(s, "file.save", path)
}

fn save_as(s: &mut Session, p: PathParams) -> Result<Value, EngineError> {
    write_project(s, "file.saveAs", p.path)
}

/// `file.saveMachineFile`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MachineFileParams {
    /// Where to write the G-code.
    pub path: String,
    /// Only the selected objects (Cut Selected Graphics). Default: the project setting.
    #[serde(default)]
    pub selected_only: Option<bool>,
}

fn save_machine_file(s: &mut Session, p: MachineFileParams) -> Result<Value, EngineError> {
    const ID: &str = "file.saveMachineFile";
    let built = s.build_job(None, &JobOptions { machine: None, selected_only: p.selected_only }).map_err(|r| EngineError::JobRefused(Box::new(r)))?;
    let Encoded::Gcode { job, .. } = &built.encoded else {
        return Err(EngineError::disabled(ID, "Save GCode is for GRBL machines; galvo jobs run from the Laser panel"));
    };
    s.write_file(&p.path, job.text.as_bytes())?;
    Ok(json!({ "path": p.path, "lines": job.lines, "fingerprint": job.fingerprint.hex() }))
}

fn exit(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    let active = s.busy_machine().is_some();
    if active {
        s.stop_all();
    }
    Ok(json!({ "ui": "file.exit", "quit": true, "stopped": active }))
}
