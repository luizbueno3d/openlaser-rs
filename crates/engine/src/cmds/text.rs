//! LightBurn-style Edit Text (Appendix A): `tools.text` creates a text shape, `text.edit` re-lays out a selected
//! text, `text.fonts` lists the discovered system fonts, and `edit.convertToPath` bakes parametric/text geometry.
//! The laid-out outline is cached inside `Geom::Text`, so the file keeps its geometry without the font.

use std::sync::Arc;

use openlaser_doc::{Geom, Shape, TextGeom};
use openlaser_geom::Affine;
use openlaser_types::{LayerKey, ObjectId, Percent, TextAlign};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::{Created, always, needs_doc};

/// 100% power for created shapes (tools.rs's constant).
const FULL_POWER: Percent = match Percent::new_const(100.0) {
    Some(p) => p,
    None => Percent::MAX_VALID,
};
use crate::command::{Access, CommandSpec, EngineError, Kind};
use crate::params::{IdsParams, LayerKeySchema, NoParams};
use crate::session::{Label, Session};
use crate::{command, geomx};

/// `tools.text` parameters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TextParams {
    /// Baseline anchor point (the alignment point), world mm.
    pub x_mm: f64,
    pub y_mm: f64,
    /// The text; `\n` splits lines.
    pub text: String,
    /// Font family; default: the first available of the usual system names, else the first indexed family.
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
    /// Cap height in mm (default 10).
    #[serde(default)]
    pub height_mm: Option<f64>,
    /// Extra advance per character in mm (default 0).
    #[serde(default)]
    pub letter_spacing_mm: Option<f64>,
    /// Line-height multiplier (default 1.0).
    #[serde(default)]
    pub line_spacing: Option<f64>,
    /// "left" (default), "center" or "right".
    #[serde(default)]
    pub align: Option<String>,
    /// Layer. Default: C00.
    #[serde(default)]
    #[schemars(with = "Option<LayerKeySchema>")]
    pub layer: Option<LayerKey>,
}

/// `text.edit` parameters: every field optional except `id`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TextEditParams {
    pub id: u64,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub height_mm: Option<f64>,
    #[serde(default)]
    pub letter_spacing_mm: Option<f64>,
    #[serde(default)]
    pub line_spacing: Option<f64>,
    #[serde(default)]
    pub align: Option<String>,
    /// Consecutive edits sharing a key merge into one history entry.
    #[serde(default)]
    pub coalesce: Option<String>,
}

fn align_of(id: &str, value: Option<&str>) -> Result<TextAlign, EngineError> {
    match value.unwrap_or("left").to_ascii_lowercase().as_str() {
        "left" => Ok(TextAlign::Left),
        "center" => Ok(TextAlign::Center),
        "right" => Ok(TextAlign::Right),
        other => Err(EngineError::bad(id, "/align", format!("`{other}` is not left, center or right"))),
    }
}

/// Resolve the font: explicit family, else the LightBurn defaults, else the first indexed family.
fn resolve_face<'a>(s: &'a Session, id: &str, family: Option<&str>, style: Option<&str>) -> Result<openlaser_text::FontRef<'a>, EngineError> {
    let lib = s.fonts();
    if let Some(fam) = family {
        return lib.face(fam, style).ok_or_else(|| EngineError::disabled(id, format!("the font '{fam}' is not installed")));
    }
    for cand in ["Arial", "Helvetica", "Liberation Sans", "DejaVu Sans", "Hack"] {
        if let Some(f) = lib.face(cand, style) {
            return Ok(f);
        }
    }
    if let Some((family, styles)) = lib.families().into_iter().next() {
        return lib
            .face(&family, style.or_else(|| styles.first().map(|s| s.as_str())))
            .ok_or_else(|| EngineError::disabled(id, "no fonts were found on this system"));
    }
    Err(EngineError::disabled(id, "no fonts were found on this system"))
}

/// Lay out a text: shared by `tools.text` and `text.edit` so create and edit see identical geometry.
#[allow(clippy::too_many_arguments)] // the signature mirrors TextGeom field-for-field
fn lay_out(
    s: &Session,
    id: &str,
    text: &str,
    family: Option<&str>,
    style: Option<&str>,
    height: f64,
    letter: f64,
    line: f64,
    align: TextAlign,
) -> Result<(openlaser_doc::BezPath, String, String), EngineError> {
    let font = resolve_face(s, id, family, style)?;
    let (fam, sty) = s
        .fonts()
        .entries()
        .iter()
        .filter(|e| family.is_none_or(|f| e.family.eq_ignore_ascii_case(f)))
        .find(|e| style.is_none_or(|s| e.style.eq_ignore_ascii_case(s)) || style.is_none())
        .map(|e| (e.family.clone(), e.style.clone()))
        .unwrap_or_else(|| ("unknown".to_owned(), "Regular".to_owned()));
    let path = openlaser_text::layout(&font, text, height, letter, line, align).map_err(|e| EngineError::bad(id, "/text", e.to_string()))?;
    Ok((path, fam, sty))
}

/// Builds the same cached text geometry for tools.text and project generators, without editing a document.
pub(super) fn text_shape(s: &Session, id: &str, p: TextParams) -> Result<Shape, EngineError> {
    if !p.x_mm.is_finite() || !p.y_mm.is_finite() {
        return Err(EngineError::bad(id, "/xMm", "position must be finite"));
    }
    if p.text.is_empty() {
        return Err(EngineError::bad(id, "/text", "the text is empty"));
    }
    let height = p.height_mm.unwrap_or(10.0);
    let letter = p.letter_spacing_mm.unwrap_or(0.0);
    let line = p.line_spacing.unwrap_or(1.0);
    let align = align_of(id, p.align.as_deref())?;
    let (outline, family, style) = lay_out(s, id, &p.text, p.family.as_deref(), p.style.as_deref(), height, letter, line, align)?;
    let layer: LayerKey = p.layer.unwrap_or(LayerKey::Color(0));
    let offset = Affine::translate(openlaser_geom::Vec2::new(p.x_mm, p.y_mm));
    Ok(Shape {
        id: ObjectId(0),
        name: None,
        layer,
        xform: offset,
        geom: Geom::Text(Arc::new(TextGeom {
            text: p.text.clone(),
            family,
            style,
            height_mm: height,
            letter_spacing_mm: letter,
            line_spacing: line,
            align,
            outline: Arc::new(outline),
        })),
        locked: false,
        power_scale: FULL_POWER,
        priority: 0,
    })
}

fn tools_text(s: &mut Session, p: TextParams) -> Result<Created, EngineError> {
    let mut shape = text_shape(s, "tools.text", p)?;
    s.edit(Label::new("Text"), None, move |project, sel| {
        let id = project.alloc_id().map_err(geomx::doc_error)?;
        shape.id = id;
        project.shapes.push(Arc::new(shape));
        sel.ids = vec![id];
        sel.key = Some(id);
        Ok(Created { ids: vec![id] })
    })
}

fn text_edit(s: &mut Session, p: TextEditParams) -> Result<Created, EngineError> {
    const ID: &str = "text.edit";
    let oid = ObjectId(p.id);
    let doc = s.doc()?;
    let Some(shape) = doc.project.shape(oid) else {
        return Err(EngineError::bad(ID, "/id", format!("object {} does not exist", oid.0)));
    };
    let Geom::Text(text) = &shape.geom else {
        return Err(EngineError::bad(ID, "/id", format!("object {} is not a text shape", oid.0)));
    };
    if shape.locked {
        return Err(EngineError::disabled(ID, "the text is locked; unlock it first"));
    }
    let text = text.clone();
    let content = p.text.unwrap_or_else(|| text.text.clone());
    if content.is_empty() {
        return Err(EngineError::bad(ID, "/text", "the text is empty"));
    }
    let family = p.family.unwrap_or_else(|| text.family.clone());
    let style = p.style.unwrap_or_else(|| text.style.clone());
    let height = p.height_mm.unwrap_or(text.height_mm);
    let letter = p.letter_spacing_mm.unwrap_or(text.letter_spacing_mm);
    let line = p.line_spacing.unwrap_or(text.line_spacing);
    let align = align_of(ID, p.align.as_deref()).unwrap_or_else(|_| if p.align.is_none() { text.align } else { TextAlign::Left });
    let align = if p.align.is_some() { align } else { text.align };
    let (outline, family, style) = lay_out(s, ID, &content, Some(&family), Some(&style), height, letter, line, align)?;
    let p_coalesce = p.coalesce.clone();
    s.edit(Label::new("Edit Text"), p_coalesce.as_deref(), move |project, sel| {
        let Some(shape) = project.shapes.iter_mut().find(|sh| sh.id == oid) else {
            return Err(EngineError::bad(ID, "/id", "the text vanished mid-edit"));
        };
        let shape = Arc::make_mut(shape);
        shape.geom = Geom::Text(Arc::new(TextGeom {
            text: content,
            family,
            style,
            height_mm: height,
            letter_spacing_mm: letter,
            line_spacing: line,
            align,
            outline: Arc::new(outline),
        }));
        sel.ids = vec![oid];
        sel.key = Some(oid);
        Ok(Created { ids: vec![oid] })
    })
}

fn text_fonts(s: &mut Session, _p: NoParams) -> Result<serde_json::Value, EngineError> {
    let families: Vec<serde_json::Value> = s.fonts().families().iter().map(|(family, styles)| json!({ "family": family, "styles": styles })).collect();
    Ok(json!({ "fonts": families }))
}

/// `edit.convertToPath` {ids?}: Text and the parametric shapes become Geom::Path of their own to_bez, xform kept.
fn convert_to_path(s: &mut Session, p: IdsParams) -> Result<Created, EngineError> {
    const ID: &str = "edit.convertToPath";
    let ids = super::target_ids(s, ID, p.ids)?;
    let mut converted = Vec::new();
    for oid in &ids {
        let Some(shape) = s.doc()?.project.shape(*oid) else {
            return Err(EngineError::bad(ID, "/ids", format!("object {} does not exist", oid.0)));
        };
        if shape.locked {
            return Err(EngineError::disabled(ID, "a locked object cannot be converted; unlock it first"));
        }
        match &shape.geom {
            Geom::Path(_) | Geom::Group(_) => {} // already a path or a group (groups keep children)
            _ => converted.push(*oid),
        }
    }
    s.edit(Label::new("Convert to Path"), None, move |project, sel| {
        for oid in &converted {
            if let Some(shape) = project.shapes.iter_mut().find(|sh| sh.id == *oid) {
                let shape = Arc::make_mut(shape);
                shape.geom = Geom::Path(Arc::new(shape.geom.to_bez()));
            }
        }
        if !converted.is_empty() {
            sel.ids = converted.clone();
            sel.key = converted.last().copied();
        }
        Ok(Created { ids: converted.clone() })
    })
}

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        command!("tools.text", "Edit Text", menu: ["Tools"], shortcut: Some("Cmd+T"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: TextParams, run: tools_text,
            doc: "{xMm, yMm, text, family?, style?, heightMm?(10), letterSpacingMm?(0), lineSpacing?(1), align?\"left\"|\"center\"|\"right\", layer?}"),
        command!("text.edit", "Edit Text…", menu: [], shortcut: Some("Cmd+E"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: TextEditParams, run: text_edit, doc: "{id, text?, family?, style?, heightMm?, letterSpacingMm?, lineSpacing?, align?}"),
        command!("text.fonts", "List Fonts", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: always, params: NoParams, run: text_fonts, doc: "{fonts:[{family, styles}]}"),
        command!("edit.convertToPath", "Convert to Path", menu: ["Arrange"], shortcut: Some("Cmd+Shift+C"), kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: IdsParams, run: convert_to_path, doc: "{ids?} (text and parametric shapes become Geom::Path, xform kept)"),
    ]
}
