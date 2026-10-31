//! Material tests are new design documents only. Descending axes are intentional: min → max in grid order;
//! output order is independently sorted by energy. Settings use the cut editor's decoder and validator.

use std::sync::Arc;

use openlaser_doc::{ColorLayer, Geom, Project, Shape};
use openlaser_geom::Affine;
use openlaser_machine::Kinematics;
use openlaser_types::{Bounds, LayerKey, Mm, ObjectId, Percent};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{always, layer, text};
use crate::command::{Access, CommandSpec, EngineError, Kind};
use crate::params::{decode, finite, positive};
use crate::session::Session;
use crate::{command, geomx};

const ID: &str = "laserTools.materialTest";

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![command!("laserTools.materialTest", "Material Test…", menu: ["Laser Tools"], shortcut: None,
        kind: Kind::Edit, access: Access::Agent, enabled: always, params: MaterialTestParams, run: generate,
        doc: "{rows:{param,min,max,count}, cols:{param,min,max,count}, cellWMm?, cellHMm?, gapMm?, centerMm?, mode?, base?, labels?, labelPowerPct?, labelSpeedMmS?}; at most 29 cells with labels, 30 without; descending ranges allowed; creates a new project, never burns")]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Param {
    PowerPct,
    SpeedMmS,
    FrequencyKhz,
    QPulseNs,
    IntervalMm,
    Passes,
}

impl Param {
    fn key(self) -> &'static str {
        match self {
            Self::PowerPct => "powerPct",
            Self::SpeedMmS => "speedMmS",
            Self::FrequencyKhz => "frequencyKhz",
            Self::QPulseNs => "qPulseNs",
            Self::IntervalMm => "intervalMm",
            Self::Passes => "passes",
        }
    }

    fn label(self, v: f64) -> String {
        let n = if v != 0.0 && v.abs() < 0.001 { format!("{v:.3e}") } else { format!("{v:.3}").trim_end_matches('0').trim_end_matches('.').to_owned() };
        match self {
            Self::PowerPct => format!("P {n}%"),
            Self::SpeedMmS => format!("S {n} mm/s"),
            Self::FrequencyKhz => format!("F {n} kHz"),
            Self::QPulseNs => format!("Q {n} ns"),
            Self::IntervalMm => format!("I {n} mm"),
            Self::Passes => format!("N {n}"),
        }
    }
}

const PARAMS: [Param; 6] = [Param::PowerPct, Param::SpeedMmS, Param::FrequencyKhz, Param::QPulseNs, Param::IntervalMm, Param::Passes];

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Axis {
    pub param: Param,
    pub min: f64,
    pub max: f64,
    /// 1..=30. One sample uses min; descending ranges are allowed.
    #[schemars(range(min = 1, max = 30))]
    pub count: u8,
}

impl Axis {
    fn values(&self, field: &str) -> Result<Vec<f64>, EngineError> {
        if !(1..=30).contains(&self.count) {
            return Err(EngineError::bad(ID, &format!("/{field}/count"), "count must be in 1..=30"));
        }
        finite(ID, &format!("/{field}/min"), self.min)?;
        finite(ID, &format!("/{field}/max"), self.max)?;
        (0..self.count)
            .map(|i| {
                let v = if i == 0 {
                    self.min
                } else if i == self.count - 1 {
                    self.max
                } else {
                    let t = f64::from(i) / f64::from(self.count - 1);
                    self.min * (1.0 - t) + self.max * t
                };
                finite(ID, &format!("/{field}/{}", self.param.key()), v)
            })
            .collect()
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Base {
    pub speed_mm_s: Option<f64>,
    pub power_pct: Option<f64>,
    pub frequency_khz: Option<f64>,
    pub q_pulse_ns: Option<f64>,
    pub interval_mm: Option<f64>,
    pub passes: Option<f64>,
}

impl Base {
    fn supplied(&self) -> [Option<f64>; 6] {
        [self.power_pct, self.speed_mm_s, self.frequency_khz, self.q_pulse_ns, self.interval_mm, self.passes]
    }
}

#[derive(Debug, Default, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    Fill,
    Line,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MaterialTestParams {
    pub rows: Axis,
    pub cols: Axis,
    pub cell_w_mm: Option<f64>,
    pub cell_h_mm: Option<f64>,
    pub gap_mm: Option<f64>,
    pub center_mm: Option<[f64; 2]>,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub base: Base,
    /// Default true: one extra layer, at most 29 cells.
    pub labels: Option<bool>,
    pub label_power_pct: Option<f64>,
    pub label_speed_mm_s: Option<f64>,
}

/// The cut editor owns every physical range. Decode into its unit types, then apply its shared validation.
/// Remap failures back to the caller's field rather than exposing the intermediate cut-settings JSON.
fn setting(l: &mut ColorLayer, galvo: bool, param: Param, value: f64, field: &str) -> Result<(), EngineError> {
    let result = (|| {
        let mut p = serde_json::Map::new();
        p.insert("layer".into(), json!(l.index));
        let key = param.key();
        match param {
            Param::Passes => {
                let rounded = finite(ID, field, value)?.round().max(1.0);
                let n = format!("{rounded:.0}").parse::<u64>().map_err(|_| EngineError::bad(ID, field, "passes is too large"))?;
                p.insert("passes".into(), json!(n));
            }
            Param::IntervalMm => {
                p.insert("fill".into(), json!({"intervalMm": value}));
            }
            Param::FrequencyKhz | Param::QPulseNs if !galvo => return Err(EngineError::bad(ID, field, format!("{key} is unavailable for a GRBL profile"))),
            _ => {
                let key = if !galvo && param == Param::PowerPct { "powerMaxPct" } else { key };
                p.insert(if galvo { "galvo".into() } else { "gantry".into() }, json!({key: value}));
            }
        }
        let p = decode::<layer::CutSettingsParams>(ID, Value::Object(p))?;
        layer::apply_color(&p, l)
    })();
    result.map_err(|e| EngineError::bad(ID, field, format!("{}: {e}", param.key())))
}

fn value(l: &ColorLayer, galvo: bool, p: Param) -> f64 {
    let Some(op) = l.ops.first() else { return 0.0 };
    match p {
        Param::PowerPct => {
            if galvo {
                op.galvo.power.get()
            } else {
                op.gantry.power_max.get()
            }
        }
        Param::SpeedMmS => {
            if galvo {
                op.galvo.speed.get()
            } else {
                op.gantry.speed.get()
            }
        }
        Param::FrequencyKhz => f64::from(op.galvo.frequency.get()) / 1000.0,
        Param::QPulseNs => op.galvo.q_pulse.map_or(0.0, |v| v.get()),
        Param::IntervalMm => op.fill.interval.get(),
        Param::Passes => f64::from(op.passes),
    }
}

fn add(project: &mut Project, mut shape: Shape) -> Result<(), EngineError> {
    shape.id = project.alloc_id().map_err(geomx::doc_error)?;
    project.shapes.push(Arc::new(shape));
    Ok(())
}

/// Place labels using actual cached ink bounds, including descenders. Column labels fit their cell width.
fn label(s: &Session, project: &mut Project, layer: LayerKey, content: String, at: [f64; 2], align: &str, width: Option<f64>) -> Result<(), EngineError> {
    let mut shape = text::text_shape(
        s,
        ID,
        text::TextParams {
            x_mm: 0.0,
            y_mm: 0.0,
            text: content,
            family: None,
            style: None,
            height_mm: Some(2.0),
            letter_spacing_mm: None,
            line_spacing: None,
            align: None,
            layer: Some(layer),
        },
    )?;
    let b = geomx::shape_extent(&shape).ok_or_else(|| EngineError::bad(ID, "/labels", "label has no outline"))?;
    let scale = width.map_or(1.0, |w| (w / (b.max[0] - b.min[0])).min(1.0));
    let dx = match align {
        "right" => -b.max[0],
        "center" => -(b.min[0] + b.max[0]) / 2.0,
        _ => -b.min[0],
    };
    shape.xform = Affine::translate((at[0], at[1])) * Affine::scale(scale) * Affine::translate((dx, -b.min[1]));
    add(project, shape)
}

fn generate(s: &mut Session, p: MaterialTestParams) -> Result<Value, EngineError> {
    let rv = p.rows.values("rows")?;
    let cv = p.cols.values("cols")?;
    if p.rows.param == p.cols.param {
        return Err(EngineError::bad(ID, "/cols/param", "cols.param must differ from rows.param"));
    }
    let labels = p.labels.unwrap_or(true);
    let cells = usize::from(p.rows.count) * usize::from(p.cols.count);
    if cells > if labels { 29 } else { 30 } {
        return Err(EngineError::bad(
            ID,
            "/rows/count",
            format!("a material test has at most 29 cells with labels (30 without): {} × {} = {cells}; lower the counts", p.cols.count, p.rows.count),
        ));
    }
    let profile =
        s.machine_for(None).or_else(|| s.machines().first()).cloned().ok_or_else(|| EngineError::disabled(ID, "this session has no machine profiles"))?;
    let galvo = matches!(profile.kind, Kinematics::Galvo(_));
    let mut project = Session::new_project(&profile, "Untitled");
    let mut base = project.color_layer(0).cloned().ok_or_else(|| EngineError::bad(ID, "/base", "profile has no layer defaults"))?;
    // Explicit base fields are validated even when an axis later overrides them.
    for (param, supplied) in PARAMS.into_iter().zip(p.base.supplied()) {
        if let Some(v) = supplied {
            setting(&mut base, galvo, param, v, &format!("/base/{}", param.key()))?;
        }
    }
    let w = positive(ID, "/cellWMm", p.cell_w_mm.unwrap_or(5.0), 1.0e6)?;
    let h = positive(ID, "/cellHMm", p.cell_h_mm.unwrap_or(5.0), 1.0e6)?;
    let gap = finite(ID, "/gapMm", p.gap_mm.unwrap_or(2.0))?;
    if gap < 0.0 {
        return Err(EngineError::bad(ID, "/gapMm", "gapMm must be non-negative"));
    }
    let [cx, cy] = p.center_mm.unwrap_or([0.0, 0.0]);
    finite(ID, "/centerMm/0", cx)?;
    finite(ID, "/centerMm/1", cy)?;
    let gw = f64::from(p.cols.count) * w + f64::from(p.cols.count - 1) * gap;
    let gh = f64::from(p.rows.count) * h + f64::from(p.rows.count - 1) * gap;
    let (left, bottom) = (cx - gw / 2.0, cy - gh / 2.0);
    let mut layers = Vec::new();
    let mut ranked = Vec::new();
    for (row, r) in rv.iter().enumerate() {
        for (col, c) in cv.iter().enumerate() {
            let index = u8::try_from(layers.len()).map_err(|_| EngineError::bad(ID, "/rows/count", "too many layers"))?;
            let key = LayerKey::Color(index);
            let mut l = base.clone();
            l.index = index;
            setting(&mut l, galvo, p.rows.param, *r, &format!("/rows/{}", p.rows.param.key()))?;
            setting(&mut l, galvo, p.cols.param, *c, &format!("/cols/{}", p.cols.param.key()))?;
            let r = value(&l, galvo, p.rows.param);
            let c = value(&l, galvo, p.cols.param);
            l.name = format!("{} · {}", p.rows.param.label(r), p.cols.param.label(c));
            let settings = decode::<layer::CutSettingsParams>(
                ID,
                json!({"layer": index, "mode": match p.mode { Mode::Fill => "fill", Mode::Line => "line" }, "fill": {"angleDeg": 0}}),
            )?;
            layer::apply_color(&settings, &mut l)?;
            ranked.push((key, value(&l, galvo, Param::PowerPct), value(&l, galvo, Param::SpeedMmS), value(&l, galvo, Param::QPulseNs)));
            let mut values = serde_json::Map::new();
            for param in PARAMS {
                if galvo || !matches!(param, Param::FrequencyKhz | Param::QPulseNs) {
                    values.insert(param.key().into(), json!(value(&l, galvo, param)));
                }
            }
            layers.push(json!({"layer": key.to_string(), "name": l.name, "values": values}));
            let slot = project.color_layer_mut(index).ok_or_else(|| EngineError::bad(ID, "/rows/count", "no available colour layer"))?;
            *slot = l;
            add(
                &mut project,
                Shape {
                    id: ObjectId(0),
                    name: None,
                    layer: key,
                    xform: Affine::translate((left + col as f64 * (w + gap) + w / 2.0, bottom + row as f64 * (h + gap) + h / 2.0)),
                    geom: Geom::Rect {
                        w: Mm::new(w).map_err(|e| EngineError::bad(ID, "/cellWMm", e.to_string()))?,
                        h: Mm::new(h).map_err(|e| EngineError::bad(ID, "/cellHMm", e.to_string()))?,
                        corner_radius: Mm::new(0.0).map_err(|e| EngineError::bad(ID, "/cellWMm", e.to_string()))?,
                    },
                    locked: false,
                    power_scale: Percent::MAX_VALID,
                    priority: 0,
                },
            )?;
        }
    }
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.2.total_cmp(&b.2)).then(b.3.total_cmp(&a.3)));
    project.layers.order = ranked.iter().map(|v| v.0).collect();
    // Labels use independent profile defaults, never the material cell's base settings.
    let index = u8::try_from(cells).map_err(|_| EngineError::bad(ID, "/labels", "too many layers"))?;
    if labels {
        let key = LayerKey::Color(index);
        let l = project.color_layer_mut(index).ok_or_else(|| EngineError::bad(ID, "/labels", "no layer available for labels"))?;
        l.name = "Labels".into();
        for (param, v, field) in [(Param::PowerPct, p.label_power_pct, "/labelPowerPct"), (Param::SpeedMmS, p.label_speed_mm_s, "/labelSpeedMmS")] {
            if let Some(v) = v {
                setting(l, galvo, param, v, field)?;
            }
        }
        // New-project defaults are Line mode, one pass.
        layers.push(json!({"layer": key.to_string(), "name": l.name, "values": {"powerPct": value(l, galvo, Param::PowerPct), "speedMmS": value(l, galvo, Param::SpeedMmS)}}));
        project.layers.order.push(key);
        for (col, c) in cv.iter().enumerate() {
            let v = project
                .color_layer(u8::try_from(col).map_err(|_| EngineError::bad(ID, "/cols/count", "too many columns"))?)
                .map_or(*c, |l| value(l, galvo, p.cols.param));
            label(s, &mut project, key, p.cols.param.label(v), [left + col as f64 * (w + gap) + w / 2.0, bottom + gh + 2.0], "center", Some(w))?;
        }
        for (row, r) in rv.iter().enumerate() {
            let n = row * usize::from(p.cols.count);
            let v = project.layers.colors.get(n).map_or(*r, |l| value(l, galvo, p.rows.param));
            label(s, &mut project, key, p.rows.param.label(v), [left - 2.0, bottom + row as f64 * (h + gap) + h / 2.0 - 1.0], "right", None)?;
        }
        let settings: Vec<_> = PARAMS
            .into_iter()
            .filter(|v| *v != p.rows.param && *v != p.cols.param && (galvo || !matches!(v, Param::FrequencyKhz | Param::QPulseNs)))
            .map(|v| v.label(value(&base, galvo, v)))
            .collect();
        label(
            s,
            &mut project,
            key,
            format!("{} × {} | {}", p.rows.param.key(), p.cols.param.key(), settings.join("; ")),
            [left, bottom + gh + 6.0],
            "left",
            None,
        )?;
    } else {
        // Supplied label settings still get the cut editor's validation, even with labels off.
        let mut l = base.clone();
        for (param, v, field) in [(Param::PowerPct, p.label_power_pct, "/labelPowerPct"), (Param::SpeedMmS, p.label_speed_mm_s, "/labelSpeedMmS")] {
            if let Some(v) = v {
                setting(&mut l, galvo, param, v, field)?;
            }
        }
    }
    let bounds = project.shapes.iter().filter_map(|s| geomx::shape_extent(s)).fold(Bounds { min: [left, bottom], max: [left + gw, bottom + gh] }, |a, b| {
        Bounds { min: [a.min[0].min(b.min[0]), a.min[1].min(b.min[1])], max: [a.max[0].max(b.max[0]), a.max[1].max(b.max[1])] }
    });
    // Apply the same calibration transform as planning before checking a galvo's physical work area.
    // The enclosing rectangle is conservative for rotated text and grids.
    let fit_bounds = if let Kinematics::Galvo(g) = &profile.kind {
        let xf = openlaser_plan::machine_transform(g);
        let points = [bounds.min, [bounds.min[0], bounds.max[1]], bounds.max, [bounds.max[0], bounds.min[1]]];
        let mut transformed = None::<Bounds>;
        for [x, y] in points {
            let p = xf * openlaser_geom::Point::new(x, y);
            if !p.x.is_finite() || !p.y.is_finite() {
                return Err(EngineError::bad(ID, "/centerMm", "cellWMm/cellHMm/gapMm grid plus labels has a non-finite size after machine calibration"));
            }
            transformed = Some(match transformed {
                Some(b) => Bounds { min: [b.min[0].min(p.x), b.min[1].min(p.y)], max: [b.max[0].max(p.x), b.max[1].max(p.y)] },
                None => Bounds { min: [p.x, p.y], max: [p.x, p.y] },
            });
        }
        transformed.ok_or_else(|| EngineError::bad(ID, "/centerMm", "grid has no bounds"))?
    } else {
        bounds
    };
    let area = match &profile.kind {
        Kinematics::Galvo(g) => g.effective_work_area(),
        // Check in the generated project's design coordinates, before the planner's origin transform.
        Kinematics::Gantry(g) => match project.workspace.origin {
            openlaser_doc::WorkspaceOrigin::Center => {
                openlaser_machine::FieldRect { min: [-g.bed_mm[0] / 2.0, -g.bed_mm[1] / 2.0], max: [g.bed_mm[0] / 2.0, g.bed_mm[1] / 2.0] }
            }
            openlaser_doc::WorkspaceOrigin::BottomLeft => openlaser_machine::FieldRect { min: [0.0, 0.0], max: g.bed_mm },
            openlaser_doc::WorkspaceOrigin::TopLeft => openlaser_machine::FieldRect { min: [0.0, -g.bed_mm[1]], max: [g.bed_mm[0], 0.0] },
        },
    };
    let size = [bounds.max[0] - bounds.min[0], bounds.max[1] - bounds.min[1]];
    if !size.iter().all(|v| v.is_finite())
        || fit_bounds
            .min
            .into_iter()
            .zip(fit_bounds.max)
            .zip(area.min.into_iter().zip(area.max))
            .any(|((lo, hi), (area_lo, area_hi))| !lo.is_finite() || !hi.is_finite() || lo < area_lo || hi > area_hi)
    {
        return Err(EngineError::bad(
            ID,
            "/centerMm",
            format!(
                "cellWMm/cellHMm/gapMm grid plus labels needs {:.3} × {:.3} mm at this centerMm; work area is {:.3} × {:.3} mm; reduce the size or move the center",
                fit_bounds.max[0] - fit_bounds.min[0],
                fit_bounds.max[1] - fit_bounds.min[1],
                area.max[0] - area.min[0],
                area.max[1] - area.min[1]
            ),
        ));
    }
    project.normalize();
    s.open_project(project, None, Some(profile.id))?;
    Ok(json!({"cells": cells, "layers": layers, "sizeMm": size, "labels": labels}))
}
