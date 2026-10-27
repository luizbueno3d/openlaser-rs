//! Cuts / Layers window, the Cut Settings Editor and the colour palette (Appendix A). Cut-setting edits are
//! undoable (S13); physical sanity is checked here (unit ranges), machine limits at plan time by `safety`.

use std::sync::Arc;

use openlaser_doc::{ColorLayer, Project};
use openlaser_types::{Deg, Hz, LayerKey, Mm, MmPerS, Ns, Percent};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Changed, needs_doc, target_ids};
use crate::command::{Access, CommandSpec, EngineError, Kind};
use crate::inspect::{layer_json, layer_order};
use crate::params::{CutModeP, ImageModeP, LayerKeySchema, LayerParams, name};
use crate::session::{Label, Session};
use crate::{command, geomx};

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command!("layer.select", "Select Layer", menu: [], shortcut: None, kind: Kind::View, access: Access::Agent,
            enabled: needs_doc, params: LayerParams, run: set_current, doc: "{layer}"),
        command!("layer.setCurrent", "Set Current Layer", menu: [], shortcut: None, kind: Kind::View, access: Access::Agent,
            enabled: needs_doc, params: LayerParams, run: set_current, doc: "{layer}"),
        command!("layer.assign", "Assign Layer", menu: [], shortcut: None, kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: AssignParams, run: assign, doc: "{layer, ids?}"),
        command!("layer.cutSettings.set", "Cut Settings", menu: [], shortcut: None, kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: CutSettingsParams, run: cut_settings_set,
            doc: "{layer, subLayer?, name?, output?, show?, mode?, passes?, globalPasses?, airAssist?, galvo?:{speedMmS, powerPct, minPowerPct, frequencyKhz | frequencyHz, qPulseNs}, gantry?:{speedMmS, powerMaxPct, powerMinPct, constantPower}, fill?:{intervalMm | lpi, angleDeg, angleIncrementDeg, bidirectional, crossHatch}, image?:{mode, intervalMm, angleDeg, bidirectional, negative, threshold, passes}}"),
        command!("layer.cutSettings.get", "Get Cut Settings", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: needs_doc, params: LayerParams, run: cut_settings_get, doc: "{layer}"),
        command!("layer.cutSettings.open", "Open Cut Settings Editor", menu: [], shortcut: None, kind: Kind::View, access: Access::Ui,
            enabled: needs_doc, params: LayerParams, run: |_s: &mut Session, p: LayerParams| Ok::<_, EngineError>(json!({ "ui": "layer.cutSettings.open", "layer": p.layer.to_string() })),
            doc: "{layer}"),
        command!("layer.setMode", "Mode", menu: [], shortcut: None, kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: ModeParams, run: set_mode, doc: "{layer, mode:\"line\"|\"fill\"|\"offsetFill\"}"),
        command!("layer.setOutput", "Output", menu: [], shortcut: None, kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: OnParams, run: set_output, doc: "{layer, on}"),
        command!("layer.setShow", "Show", menu: [], shortcut: None, kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: OnParams, run: set_show, doc: "{layer, on}"),
        command!("layer.reorder", "Move Layer", menu: [], shortcut: None, kind: Kind::Edit, access: Access::Agent,
            enabled: needs_doc, params: ReorderParams, run: reorder, doc: "{layer, toIndex}"),
    ]
}

/// The layer must exist in the project (a colour layer of the table, or T1/T2).
fn check_layer(project: &Project, id: &str, key: LayerKey) -> Result<(), EngineError> {
    let ok = match key {
        LayerKey::Color(i) => project.color_layer(i).is_some(),
        LayerKey::Tool(w) => project.layers.tools.iter().any(|t| t.which == w),
    };
    if ok { Ok(()) } else { Err(EngineError::bad(id, "/layer", format!("the project has no layer {key}"))) }
}

fn set_current(s: &mut Session, p: LayerParams) -> Result<Value, EngineError> {
    check_layer(&s.doc()?.project, "layer.setCurrent", p.layer)?;
    s.doc_mut()?.current_layer = p.layer;
    Ok(json!({ "currentLayer": p.layer.to_string() }))
}

/// `layer.assign`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AssignParams {
    /// Target layer: 0..=29, "C05", "T1", "T2".
    #[schemars(with = "LayerKeySchema")]
    pub layer: LayerKey,
    /// Object ids. Default: the selection.
    #[serde(default)]
    pub ids: Option<Vec<u64>>,
}

fn assign(s: &mut Session, p: AssignParams) -> Result<Changed, EngineError> {
    const ID: &str = "layer.assign";
    check_layer(&s.doc()?.project, ID, p.layer)?;
    let ids = target_ids(s, ID, p.ids)?;
    let layer = p.layer;
    s.edit(Label::with("Assign to {name}", layer.to_string()), None, |project, _| {
        let mut n = 0;
        for arc in &mut project.shapes {
            if ids.contains(&arc.id) && arc.layer != layer {
                *arc = Arc::new(geomx::set_layer(arc, layer));
                n += 1;
            }
        }
        Ok(Changed { changed: n })
    })
}

/// Galvo energy block (S13: separate from the gantry block).
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GalvoSet {
    /// Mark speed, mm/s (0 < v ≤ 100000; the machine's max speed is checked at plan time).
    #[serde(default)]
    #[schemars(with = "Option<f64>")]
    pub speed_mm_s: Option<MmPerS>,
    /// Max power, percent (0..=100).
    #[serde(default)]
    #[schemars(with = "Option<f64>")]
    pub power_pct: Option<Percent>,
    /// Min power, percent (0..=100).
    #[serde(default)]
    #[schemars(with = "Option<f64>")]
    pub min_power_pct: Option<Percent>,
    /// Pulse frequency in kHz (a whole number of hertz). Exactly one of frequencyKhz and frequencyHz.
    #[serde(default)]
    pub frequency_khz: Option<f64>,
    /// Pulse frequency in Hz (1..=10000000).
    #[serde(default)]
    pub frequency_hz: Option<u32>,
    /// Q-pulse width in ns (MOPA; snapped to the source's pulse table at plan time).
    #[serde(default)]
    #[schemars(with = "Option<f64>")]
    pub q_pulse_ns: Option<Ns>,
}

/// Gantry energy block.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GantrySet {
    /// Speed, mm/s.
    #[serde(default)]
    #[schemars(with = "Option<f64>")]
    pub speed_mm_s: Option<MmPerS>,
    /// Max power, percent.
    #[serde(default)]
    #[schemars(with = "Option<f64>")]
    pub power_max_pct: Option<Percent>,
    /// Min power, percent.
    #[serde(default)]
    #[schemars(with = "Option<f64>")]
    pub power_min_pct: Option<Percent>,
    /// Constant power (M3) instead of dynamic (M4).
    #[serde(default)]
    pub constant_power: Option<bool>,
}

/// Fill (hatch) geometry, shared by galvo and gantry.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FillSet {
    /// Line interval in mm (> 0). Exactly one of intervalMm and lpi.
    #[serde(default)]
    pub interval_mm: Option<f64>,
    /// Lines per inch (> 0).
    #[serde(default)]
    pub lpi: Option<f64>,
    /// Scan angle, degrees.
    #[serde(default)]
    #[schemars(with = "Option<f64>")]
    pub angle_deg: Option<Deg>,
    /// Angle added on each pass, degrees.
    #[serde(default)]
    #[schemars(with = "Option<f64>")]
    pub angle_increment_deg: Option<Deg>,
    /// Bi-directional fill.
    #[serde(default)]
    pub bidirectional: Option<bool>,
    /// Cross-hatch (a +90° pass after each pass).
    #[serde(default)]
    pub cross_hatch: Option<bool>,
}

/// Image cut settings (S16 subset).
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ImageSet {
    /// Dithering mode.
    #[serde(default)]
    pub mode: Option<ImageModeP>,
    /// Line interval in mm (> 0).
    #[serde(default)]
    pub interval_mm: Option<f64>,
    /// Scan angle, degrees.
    #[serde(default)]
    #[schemars(with = "Option<f64>")]
    pub angle_deg: Option<Deg>,
    /// Bi-directional scanning.
    #[serde(default)]
    pub bidirectional: Option<bool>,
    /// Negative image.
    #[serde(default)]
    pub negative: Option<bool>,
    /// Threshold (0..=255).
    #[serde(default)]
    pub threshold: Option<u8>,
    /// Passes (1..=999).
    #[serde(default)]
    pub passes: Option<u16>,
}

/// `layer.cutSettings.set`: every field optional (a partial update); one undo step.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CutSettingsParams {
    /// Layer: 0..=29, "C05", "T1", "T2". Tool layers take only name and show.
    #[schemars(with = "LayerKeySchema")]
    pub layer: LayerKey,
    /// Sub-layer index (0 = the layer itself). Default 0.
    #[serde(default)]
    pub sub_layer: Option<u8>,
    /// Layer name (≤ 64 characters).
    #[serde(default)]
    pub name: Option<String>,
    /// Output: burn this layer.
    #[serde(default)]
    pub output: Option<bool>,
    /// Show: draw this layer (a hidden layer with Output on is refused at plan time).
    #[serde(default)]
    pub show: Option<bool>,
    /// Mode.
    #[serde(default)]
    pub mode: Option<CutModeP>,
    /// Passes (1..=999).
    #[serde(default)]
    pub passes: Option<u16>,
    /// Global passes: repeats the whole sub-layer stack (galvo; 1..=999).
    #[serde(default)]
    pub global_passes: Option<u16>,
    /// Air assist (gantry only).
    #[serde(default)]
    pub air_assist: Option<bool>,
    /// Galvo energy block.
    #[serde(default)]
    pub galvo: Option<GalvoSet>,
    /// Gantry energy block.
    #[serde(default)]
    pub gantry: Option<GantrySet>,
    /// Fill geometry.
    #[serde(default)]
    pub fill: Option<FillSet>,
    /// Image settings.
    #[serde(default)]
    pub image: Option<ImageSet>,
}

fn passes(id: &str, pointer: &str, v: u16) -> Result<u16, EngineError> {
    if (1..=999).contains(&v) { Ok(v) } else { Err(EngineError::bad(id, pointer, format!("{v} is outside 1..=999"))) }
}

fn interval(id: &str, pointer: &str, mm: Option<f64>, lpi: Option<f64>) -> Result<Option<Mm>, EngineError> {
    let v = match (mm, lpi) {
        (Some(_), Some(_)) => return Err(EngineError::bad(id, pointer, "give intervalMm or lpi, not both")),
        (Some(v), None) => v,
        (None, Some(l)) => {
            if !(l.is_finite() && l > 0.0) {
                return Err(EngineError::bad(id, &format!("{pointer}/lpi"), format!("{l} lines per inch is not > 0")));
            }
            25.4 / l
        }
        (None, None) => return Ok(None),
    };
    if !(v.is_finite() && v > 0.0 && v <= 100.0) {
        return Err(EngineError::bad(id, &format!("{pointer}/intervalMm"), format!("{v} mm is outside 0 (exclusive)..=100")));
    }
    Mm::new(v).map(Some).map_err(|e| EngineError::bad(id, pointer, e.to_string()))
}

/// The one-field wording for the undo label, else the generic one.
fn label_for(p: &CutSettingsParams, key: LayerKey) -> Label {
    let g = p.galvo.as_ref();
    let fields: Vec<&'static str> = [
        (p.name.is_some(), "Set {name} Name"),
        (p.output.is_some(), "Set {name} Output"),
        (p.show.is_some(), "Set {name} Show"),
        (p.mode.is_some(), "Set {name} Mode"),
        (p.passes.is_some(), "Set {name} Passes"),
        (p.global_passes.is_some(), "Set {name} Global Passes"),
        (p.air_assist.is_some(), "Set {name} Air Assist"),
        (g.is_some_and(|g| g.speed_mm_s.is_some()), "Set {name} Speed"),
        (g.is_some_and(|g| g.power_pct.is_some()), "Set {name} Power"),
        (g.is_some_and(|g| g.min_power_pct.is_some()), "Set {name} Min Power"),
        (g.is_some_and(|g| g.frequency_khz.is_some() || g.frequency_hz.is_some()), "Set {name} Frequency"),
        (g.is_some_and(|g| g.q_pulse_ns.is_some()), "Set {name} Q-Pulse Width"),
        (p.gantry.is_some(), "Set {name} Cut Settings"),
        (p.fill.is_some(), "Set {name} Fill"),
        (p.image.is_some(), "Set {name} Image"),
    ]
    .into_iter()
    .filter_map(|(set, w)| set.then_some(w))
    .collect();
    match fields.as_slice() {
        [one] => Label::with(one, key.to_string()),
        _ => Label::with("Set {name} Cut Settings", key.to_string()),
    }
}

/// Validates everything that does not need the project, then applies it to `layer` / `op`.
pub(super) fn apply_color(p: &CutSettingsParams, l: &mut ColorLayer) -> Result<(), EngineError> {
    const ID: &str = "layer.cutSettings.set";
    let sub = usize::from(p.sub_layer.unwrap_or(0));
    if let Some(n) = &p.name {
        l.name = name(ID, "/name", n.trim(), 64)?;
    }
    if let Some(v) = p.output {
        l.output = v;
    }
    if let Some(v) = p.show {
        l.show = v;
    }
    if let Some(v) = p.global_passes {
        l.global_passes = passes(ID, "/globalPasses", v)?;
    }
    if let Some(v) = p.air_assist {
        l.air_assist = v;
    }
    if let Some(img) = &p.image {
        if let Some(m) = img.mode {
            l.image.mode = m.into();
        }
        if let Some(v) = interval(ID, "/image", img.interval_mm, None)? {
            l.image.interval = v;
        }
        if let Some(v) = img.angle_deg {
            l.image.angle = v;
        }
        if let Some(v) = img.bidirectional {
            l.image.bidirectional = v;
        }
        if let Some(v) = img.negative {
            l.image.negative = v;
        }
        if let Some(v) = img.threshold {
            l.image.threshold = v;
        }
        if let Some(v) = img.passes {
            l.image.passes = passes(ID, "/image/passes", v)?;
        }
    }
    let ops_len = l.ops.len();
    let Some(op) = l.ops.get_mut(sub) else {
        return Err(EngineError::bad(ID, "/subLayer", format!("sub-layer {sub} does not exist (the layer has {ops_len})")));
    };
    if let Some(m) = p.mode {
        op.mode = m.into();
    }
    if let Some(v) = p.passes {
        op.passes = passes(ID, "/passes", v)?;
    }
    if let Some(g) = &p.galvo {
        if let Some(v) = g.speed_mm_s {
            op.galvo.speed = v;
        }
        if let Some(v) = g.power_pct {
            op.galvo.power = v;
        }
        if let Some(v) = g.min_power_pct {
            op.galvo.min_power = v;
        }
        op.galvo.frequency = match (g.frequency_khz, g.frequency_hz) {
            (Some(_), Some(_)) => return Err(EngineError::bad(ID, "/galvo", "give frequencyKhz or frequencyHz, not both")),
            (Some(k), None) => Hz::from_khz(k).map_err(|e| EngineError::bad(ID, "/galvo/frequencyKhz", e.to_string()))?,
            (None, Some(h)) => Hz::new(h).map_err(|e| EngineError::bad(ID, "/galvo/frequencyHz", e.to_string()))?,
            (None, None) => op.galvo.frequency,
        };
        if let Some(v) = g.q_pulse_ns {
            op.galvo.q_pulse = Some(v);
        }
    }
    if let Some(g) = &p.gantry {
        if let Some(v) = g.speed_mm_s {
            op.gantry.speed = v;
        }
        if let Some(v) = g.power_max_pct {
            op.gantry.power_max = v;
        }
        if let Some(v) = g.power_min_pct {
            op.gantry.power_min = v;
        }
        if let Some(v) = g.constant_power {
            op.gantry.constant_power = v;
        }
    }
    if let Some(f) = &p.fill {
        if let Some(v) = interval(ID, "/fill", f.interval_mm, f.lpi)? {
            op.fill.interval = v;
        }
        if let Some(v) = f.angle_deg {
            op.fill.angle = v;
        }
        if let Some(v) = f.angle_increment_deg {
            op.fill.angle_increment = v;
        }
        if let Some(v) = f.bidirectional {
            op.fill.bidirectional = v;
        }
        if let Some(v) = f.cross_hatch {
            op.fill.cross_hatch = v;
        }
    }
    Ok(())
}

fn cut_settings_set(s: &mut Session, p: CutSettingsParams) -> Result<Value, EngineError> {
    const ID: &str = "layer.cutSettings.set";
    let key = p.layer;
    check_layer(&s.doc()?.project, ID, key)?;
    if let LayerKey::Tool(_) = key {
        let extra = p.sub_layer.is_some()
            || p.output.is_some()
            || p.mode.is_some()
            || p.passes.is_some()
            || p.global_passes.is_some()
            || p.air_assist.is_some()
            || p.galvo.is_some()
            || p.gantry.is_some()
            || p.fill.is_some()
            || p.image.is_some();
        if extra {
            return Err(EngineError::bad(ID, "/layer", format!("{key} is a tool layer: it never outputs and takes only name and show")));
        }
    }
    let label = label_for(&p, key);
    let coalesce = s.coalesce();
    s.edit(label, coalesce.as_deref(), |project, _| match key {
        LayerKey::Color(i) => {
            let Some(l) = project.color_layer_mut(i) else { return Err(EngineError::bad(ID, "/layer", format!("the project has no layer {key}"))) };
            apply_color(&p, l)?;
            // Defence in depth behind the per-field `passes()` range check: the table the edit leaves must pass
            // the same `check_passes` `format` and the planner refuse (a zero-pass layer plans nothing).
            project.layers.check_passes().map_err(geomx::doc_error)
        }
        LayerKey::Tool(w) => {
            let Some(t) = project.layers.tools.iter_mut().find(|t| t.which == w) else {
                return Err(EngineError::bad(ID, "/layer", format!("the project has no layer {key}")));
            };
            if let Some(n) = &p.name {
                t.name = name(ID, "/name", n.trim(), 64)?;
            }
            if let Some(v) = p.show {
                t.show = v;
            }
            Ok(())
        }
    })?;
    let d = s.doc()?;
    Ok(layer_json(&d.project, key, s.machine_for(None), true))
}

fn cut_settings_get(s: &mut Session, p: LayerParams) -> Result<Value, EngineError> {
    let d = s.doc()?;
    check_layer(&d.project, "layer.cutSettings.get", p.layer)?;
    Ok(layer_json(&d.project, p.layer, s.machine_for(None), true))
}

/// `layer.setMode`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ModeParams {
    /// Colour layer: 0..=29 or "C05".
    #[schemars(with = "LayerKeySchema")]
    pub layer: LayerKey,
    /// "line", "fill" or "offsetFill" (P1: refused at plan time).
    pub mode: CutModeP,
}

/// `layer.setOutput`, `layer.setShow`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OnParams {
    /// Layer: 0..=29, "C05", "T1", "T2".
    #[schemars(with = "LayerKeySchema")]
    pub layer: LayerKey,
    /// On or off.
    pub on: bool,
}

fn partial(layer: LayerKey) -> CutSettingsParams {
    CutSettingsParams {
        layer,
        sub_layer: None,
        name: None,
        output: None,
        show: None,
        mode: None,
        passes: None,
        global_passes: None,
        air_assist: None,
        galvo: None,
        gantry: None,
        fill: None,
        image: None,
    }
}

fn set_mode(s: &mut Session, p: ModeParams) -> Result<Value, EngineError> {
    cut_settings_set(s, CutSettingsParams { mode: Some(p.mode), ..partial(p.layer) })
}

fn set_output(s: &mut Session, p: OnParams) -> Result<Value, EngineError> {
    if p.layer.is_tool() {
        return Err(EngineError::bad("layer.setOutput", "/layer", format!("{} is a tool layer: it never outputs", p.layer)));
    }
    cut_settings_set(s, CutSettingsParams { output: Some(p.on), ..partial(p.layer) })
}

fn set_show(s: &mut Session, p: OnParams) -> Result<Value, EngineError> {
    cut_settings_set(s, CutSettingsParams { show: Some(p.on), ..partial(p.layer) })
}

/// `layer.reorder`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReorderParams {
    /// The layer to move (it must be in use).
    #[schemars(with = "LayerKeySchema")]
    pub layer: LayerKey,
    /// Its new position in the Cuts / Layers list (0 = first to burn).
    pub to_index: usize,
}

fn reorder(s: &mut Session, p: ReorderParams) -> Result<Value, EngineError> {
    const ID: &str = "layer.reorder";
    let mut order = layer_order(&s.doc()?.project);
    let Some(from) = order.iter().position(|k| *k == p.layer) else {
        return Err(EngineError::bad(ID, "/layer", format!("{} is not used in this project", p.layer)));
    };
    if p.to_index >= order.len() {
        return Err(EngineError::bad(ID, "/toIndex", format!("{} is outside 0..{}", p.to_index, order.len())));
    }
    let k = order.remove(from);
    order.insert(p.to_index, k);
    let o = order.clone();
    s.edit(Label::with("Move {name}", p.layer.to_string()), None, move |project, _| {
        project.layers.order = o;
        Ok(())
    })?;
    Ok(json!({ "order": order.iter().map(|k| k.to_string()).collect::<Vec<_>>() }))
}
