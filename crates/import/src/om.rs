//! OM workspace and layer-settings import (Appendix B).
//!
//! Mapping and built-in values are ported from OpenMopa `mopa_luiz/layers.py` and
//! `mopa_luiz/importers.py`, commit `6aed8ffa8be5a938d195bf35198cfd168bb07f4e` (MIT).

use std::sync::Arc;

use openlaser_doc::{ColorLayer, CutMode, Geom, Shape};
use openlaser_geom::{Affine, BezPath, Point, Polyline};
use openlaser_types::{LayerKey, ObjectId, Percent};
use serde::Deserialize;

use crate::{Error, Imported};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OmWorkspace {
    pub version: u32,
    pub objects: Vec<OmObject>,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OmObject {
    pub name: String,
    pub layer_id: String,
    pub polylines: Vec<Vec<Vec<f64>>>,
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    pub rotation: f64,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OmLayerSpec {
    pub layer_id: String,
    pub name: String,
    pub operation: String,
    pub power_percent: f64,
    pub frequency_khz: f64,
    pub pulse_width_ns: f64,
    pub speed_mm_s: f64,
    pub passes: u32,
    pub visible: bool,
    pub output: bool,
    pub color: String,
    pub air_assist: bool,
    pub raster_pitch_mm: f64,
    pub raster_angle_deg: f64,
    pub raster_angle_step_deg: f64,
}

pub fn parse_workspace(bytes: &[u8]) -> Result<OmWorkspace, Error> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| malformed(e.to_string()))?;
    if let Some(objects) = value.get("objects").and_then(serde_json::Value::as_array) {
        for object in objects {
            let name = object.get("name").and_then(serde_json::Value::as_str).unwrap_or("<unnamed>");
            if let Some(lines) = object.get("polylines").and_then(serde_json::Value::as_array) {
                for line in lines {
                    if let Some(points) = line.as_array() {
                        for point in points {
                            if point.as_array().is_none_or(|coords| coords.len() != 2) {
                                return Err(Error::MalformedPoint { object: name.to_owned(), message: "each point must contain exactly [x, y]".to_owned() });
                            }
                        }
                    }
                }
            }
        }
    }
    serde_json::from_value(value).map_err(|e| malformed(e.to_string()))
}

pub fn parse_layer_settings(bytes: &[u8]) -> Result<Vec<OmLayerSpec>, Error> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| malformed(e.to_string()))?;
    let layers = value.get("layers").cloned().unwrap_or(value);
    serde_json::from_value(layers).map_err(|e| malformed(e.to_string()))
}

/// Four default layers from the OM MOPA profile (R05 §4.1).
pub fn builtin_layers() -> Vec<OmLayerSpec> {
    [
        ("vector-engrave", "Vector Engrave", "vector_engrave", 100.0, 30.0, 200.0, 1000.0),
        ("vector-cut", "Vector Cut / Deep Mark", "vector_cut", 100.0, 30.0, 200.0, 300.0),
        ("raster", "Raster", "raster_engrave", 100.0, 30.0, 200.0, 1000.0),
        ("frame-only", "Frame Only", "frame_only", 0.0, 30.0, 200.0, 1000.0),
    ]
    .into_iter()
    .map(|(layer_id, name, operation, power_percent, frequency_khz, pulse_width_ns, speed_mm_s)| OmLayerSpec {
        layer_id: layer_id.to_owned(),
        name: name.to_owned(),
        operation: operation.to_owned(),
        power_percent,
        frequency_khz,
        pulse_width_ns,
        speed_mm_s,
        passes: 1,
        visible: true,
        output: true,
        color: String::new(),
        air_assist: false,
        raster_pitch_mm: 0.1,
        raster_angle_deg: 0.0,
        raster_angle_step_deg: 90.0,
    })
    .collect()
}

/// Unknown object layer references refuse the entire import; objects and points are never silently dropped.
pub fn to_imported(ws: &OmWorkspace, layers: &[OmLayerSpec]) -> Result<Imported, Error> {
    let mut keys = std::collections::BTreeMap::new();
    let mut imported_layers = Vec::new();
    let mut next_color = 3u8;
    for spec in layers {
        let key = match spec.layer_id.as_str() {
            "vector-engrave" | "vector_engrave" => LayerKey::Color(0),
            "vector-cut" | "vector_cut" => LayerKey::Color(1),
            "raster" | "raster_engrave" => LayerKey::Color(2),
            "frame-only" | "frame_only" => LayerKey::Tool(1),
            _ => {
                if next_color >= 30 {
                    return Err(Error::TooLarge("OM workspace contains more than 30 colour layers".to_owned()));
                }
                let key = LayerKey::Color(next_color);
                next_color = next_color.saturating_add(1);
                key
            }
        };
        keys.insert(spec.layer_id.as_str(), key);
        if let LayerKey::Color(index) = key {
            imported_layers.push((key, layer_from_spec(spec, index)?));
        }
    }
    let known: std::collections::BTreeSet<&str> = layers.iter().map(|layer| layer.layer_id.as_str()).collect();
    let missing: Vec<&str> = ws.objects.iter().filter(|object| !known.contains(object.layer_id.as_str())).map(|object| object.name.as_str()).collect();
    if !missing.is_empty() {
        return Err(malformed(format!("objects reference missing layer settings: {}", missing.join(", "))));
    }

    let mut shapes = Vec::new();
    let mut all_lines = Vec::new();
    for object in &ws.objects {
        let Some(layer) = keys.get(object.layer_id.as_str()).copied() else { continue };
        if !object.x.is_finite() || !object.y.is_finite() || !object.scale.is_finite() || object.scale <= 0.0 || !object.rotation.is_finite() {
            return Err(malformed(format!("object {} has invalid transform", object.name)));
        }
        let transform = Affine::translate((object.x, object.y)) * Affine::rotate(object.rotation.to_radians()) * Affine::scale(object.scale);
        let mut children = Vec::new();
        for raw_line in &object.polylines {
            if raw_line.len() < 2 {
                continue;
            }
            let mut path = BezPath::new();
            let mut line = Vec::with_capacity(raw_line.len());
            for (i, raw_point) in raw_line.iter().enumerate() {
                if raw_point.len() != 2 {
                    return Err(Error::MalformedPoint { object: object.name.clone(), message: "each point must contain exactly [x, y]".to_owned() });
                }
                let Some((&x, &y)) = raw_point.first().zip(raw_point.get(1)) else {
                    return Err(Error::MalformedPoint { object: object.name.clone(), message: "each point must contain exactly [x, y]".to_owned() });
                };
                if !x.is_finite() || !y.is_finite() {
                    return Err(Error::MalformedPoint { object: object.name.clone(), message: "coordinates must be finite".to_owned() });
                }
                let p = Point::new(x, y);
                if i == 0 {
                    path.move_to(p);
                } else {
                    path.line_to(p);
                }
                let world = transform * p;
                line.push([world.x, world.y]);
            }
            all_lines.push(Polyline(line));
            let power_scale = Percent::new(100.0).map_err(|e| malformed(e.to_string()))?;
            children.push(Arc::new(Shape {
                id: ObjectId(0),
                name: None,
                layer,
                xform: Affine::IDENTITY,
                geom: Geom::Path(Arc::new(path)),
                locked: false,
                power_scale,
                priority: 0,
            }));
        }
        if !children.is_empty() {
            let power_scale = Percent::new(100.0).map_err(|e| malformed(e.to_string()))?;
            shapes.push(Shape {
                id: ObjectId(0),
                name: Some(object.name.clone()),
                layer,
                xform: transform,
                geom: Geom::Group(children),
                locked: false,
                power_scale,
                priority: 0,
            });
        }
    }
    let bounds = openlaser_geom::bounds_of(&all_lines);
    Ok(Imported { shapes, layers: imported_layers, notices: Vec::new(), bounds })
}

fn layer_from_spec(spec: &OmLayerSpec, index: u8) -> Result<ColorLayer, Error> {
    let frequency = spec.frequency_khz * 1000.0;
    if !frequency.is_finite() || frequency.fract() != 0.0 || frequency < 1.0 || frequency > 10_000_000.0 {
        return Err(malformed(format!("layer {} frequency must be a valid whole number of hertz", spec.layer_id)));
    }
    let frequency_hz = format!("{frequency:.0}").parse::<u32>().map_err(|e| malformed(e.to_string()))?;
    let mode = if spec.operation.contains("raster") { CutMode::Fill } else { CutMode::Line };
    let value = serde_json::json!({
        "index": index, "name": spec.name, "output": spec.output && spec.operation != "disabled", "show": spec.visible,
        "ops": [{"name": spec.name, "enabled": spec.operation != "disabled", "mode": mode, "passes": spec.passes,
            "galvo": {"speed": spec.speed_mm_s, "power": spec.power_percent, "minPower": 0.0, "frequency": frequency_hz, "qPulse": spec.pulse_width_ns, "timings": null, "wobble": null},
            "gantry": {"speed": spec.speed_mm_s, "powerMax": spec.power_percent, "powerMin": 0.0, "constantPower": false},
            "fill": {"interval": spec.raster_pitch_mm, "angle": spec.raster_angle_deg, "angleIncrement": spec.raster_angle_step_deg, "bidirectional": true, "crossHatch": false}}],
        "globalPasses": 1, "image": {"mode":"threshold", "interval":spec.raster_pitch_mm, "angle":spec.raster_angle_deg,"bidirectional":true,"negative":false,"threshold":128,"passes":spec.passes,
            "galvo":{"speed":spec.speed_mm_s,"power":spec.power_percent,"minPower":0.0,"frequency":frequency_hz,"qPulse":spec.pulse_width_ns,"timings":null,"wobble":null},
            "gantry":{"speed":spec.speed_mm_s,"powerMax":spec.power_percent,"powerMin":0.0,"constantPower":false}}, "airAssist": spec.air_assist
    });
    serde_json::from_value(value).map_err(|e| malformed(format!("invalid layer {}: {e}", spec.layer_id)))
}

fn malformed(message: String) -> Error {
    Error::Malformed { format: "OM workspace", message }
}
