//! The prototype's (OM) `workspace.json` and `layer_settings.json` (SPEC §3.6, Appendix B).
//!
//! Ported from OpenMopa `mopa_luiz/layers.py` (`LayerSpec`, `default_layers`, `layer_from_dict`) and the
//! workspace format its UI saves, at commit `6aed8ffa8be5a938d195bf35198cfd168bb07f4e`.
//! MIT © 2026 Luiz Bueno (see ATTRIBUTION.md).
//!
//! B11: OM's values are MOPA galvo values. They map to the galvo block only; the gantry blocks get
//! [`GANTRY_DEFAULT_SPEED_MM_S`] / [`GANTRY_DEFAULT_POWER_PCT`], never a copy of the galvo numbers.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use openlaser_doc::{ColorLayer, CutMode, Geom, Shape};
use openlaser_geom::{Affine, BezPath, Point, Polyline};
use openlaser_types::{LayerKey, NoticeCode, ObjectId, Percent};
use serde::Deserialize;
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};

use crate::{Error, Imported, Notices, check_placed, check_vector_size};

/// Most objects an OM workspace may hold (OM's own cap).
pub const MAX_OM_OBJECTS: usize = 3000;
/// Most points an OM workspace may hold (OM's own cap).
pub const MAX_OM_POINTS: usize = 500_000;
/// Gantry speed written into imported layers (B11). The importer cannot see the machine; this is the conservative
/// value of the built-in profiles' `layer_defaults.gantry`, and the engine applies the bound machine's own defaults.
pub const GANTRY_DEFAULT_SPEED_MM_S: f64 = 100.0;
/// Gantry power (max) written into imported layers (B11); see [`GANTRY_DEFAULT_SPEED_MM_S`].
pub const GANTRY_DEFAULT_POWER_PCT: f64 = 30.0;

const OPERATIONS: [&str; 5] = ["vector_engrave", "vector_cut", "raster_engrave", "frame_only", "disabled"];

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

/// Points with != 2 coordinates → `MalformedPoint` (B9). Streams the JSON: more than [`MAX_OM_OBJECTS`] objects
/// or [`MAX_OM_POINTS`] points is refused (`TooLarge`) before they are all allocated.
pub fn parse_workspace(bytes: &[u8]) -> Result<OmWorkspace, Error> {
    let bytes = check_vector_size("OM workspace", bytes)?;
    let state = SeedState::default();
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let parsed = WorkspaceSeed(&state).deserialize(&mut de).and_then(|ws| de.end().map(|()| ws));
    if let Some(error) = state.error.take() {
        return Err(error);
    }
    parsed.map_err(|e| malformed(e.to_string()))
}

#[derive(Default)]
struct SeedState {
    objects: Cell<usize>,
    points: Cell<usize>,
    /// A structured refusal (B9, caps) found mid-stream; serde only carries a message.
    error: RefCell<Option<Error>>,
}

impl SeedState {
    fn fail<E: serde::de::Error>(&self, error: Error) -> E {
        let message = error.to_string();
        if self.error.borrow().is_none() {
            *self.error.borrow_mut() = Some(error);
        }
        E::custom(message)
    }
}

struct WorkspaceSeed<'s>(&'s SeedState);

impl<'de> DeserializeSeed<'de> for WorkspaceSeed<'_> {
    type Value = OmWorkspace;
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<OmWorkspace, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for WorkspaceSeed<'_> {
    type Value = OmWorkspace;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an OM workspace object")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<OmWorkspace, A::Error> {
        let mut version = None;
        let mut objects = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "version" => set_once(&mut version, "version", map.next_value::<u32>()?)?,
                "objects" => set_once(&mut objects, "objects", map.next_value_seed(ObjectsSeed(self.0))?)?,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(OmWorkspace { version: required(version, "version")?, objects: required(objects, "objects")? })
    }
}

fn set_once<T, E: serde::de::Error>(slot: &mut Option<T>, name: &'static str, value: T) -> Result<(), E> {
    if slot.is_some() {
        return Err(E::duplicate_field(name));
    }
    *slot = Some(value);
    Ok(())
}

fn required<T, E: serde::de::Error>(slot: Option<T>, name: &'static str) -> Result<T, E> {
    slot.ok_or_else(|| E::missing_field(name))
}

struct ObjectsSeed<'s>(&'s SeedState);

impl<'de> DeserializeSeed<'de> for ObjectsSeed<'_> {
    type Value = Vec<OmObject>;
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Vec<OmObject>, D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for ObjectsSeed<'_> {
    type Value = Vec<OmObject>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of objects")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<OmObject>, A::Error> {
        let mut objects = Vec::new();
        loop {
            let count = self.0.objects.get() + 1;
            if count > MAX_OM_OBJECTS {
                // Only refuse when another object really follows.
                if seq.next_element::<IgnoredAny>()?.is_none() {
                    break;
                }
                return Err(self.0.fail(Error::TooLarge(format!("OM workspace has more than {MAX_OM_OBJECTS} objects"))));
            }
            let Some(object) = seq.next_element_seed(ObjectSeed(self.0))? else { break };
            self.0.objects.set(count);
            objects.push(object);
        }
        Ok(objects)
    }
}

struct ObjectSeed<'s>(&'s SeedState);

impl<'de> DeserializeSeed<'de> for ObjectSeed<'_> {
    type Value = OmObject;
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<OmObject, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for ObjectSeed<'_> {
    type Value = OmObject;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an OM object")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<OmObject, A::Error> {
        let (mut name, mut layer_id, mut polylines) = (None, None, None);
        let (mut x, mut y, mut scale, mut rotation) = (None, None, None, None);
        let malformed_point = Cell::new(false);
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "name" => set_once(&mut name, "name", map.next_value::<String>()?)?,
                "layer_id" => set_once(&mut layer_id, "layer_id", map.next_value::<String>()?)?,
                "polylines" => set_once(&mut polylines, "polylines", map.next_value_seed(PolylinesSeed { state: self.0, malformed: &malformed_point })?)?,
                "x" => set_once(&mut x, "x", map.next_value::<f64>()?)?,
                "y" => set_once(&mut y, "y", map.next_value::<f64>()?)?,
                "scale" => set_once(&mut scale, "scale", map.next_value::<f64>()?)?,
                "rotation" => set_once(&mut rotation, "rotation", map.next_value::<f64>()?)?,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        if malformed_point.get() {
            let object = name.unwrap_or_else(|| "<unnamed>".to_owned());
            return Err(self.0.fail(Error::MalformedPoint { object, message: "each point must contain exactly [x, y]".to_owned() }));
        }
        Ok(OmObject {
            name: required(name, "name")?,
            layer_id: required(layer_id, "layer_id")?,
            polylines: required(polylines, "polylines")?,
            x: required(x, "x")?,
            y: required(y, "y")?,
            scale: required(scale, "scale")?,
            rotation: required(rotation, "rotation")?,
        })
    }
}

struct PolylinesSeed<'s> {
    state: &'s SeedState,
    malformed: &'s Cell<bool>,
}

impl<'de> DeserializeSeed<'de> for PolylinesSeed<'_> {
    type Value = Vec<Vec<Vec<f64>>>;
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for PolylinesSeed<'_> {
    type Value = Vec<Vec<Vec<f64>>>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of polylines")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut polylines = Vec::new();
        while let Some(polyline) = seq.next_element_seed(PolylineSeed { state: self.state, malformed: self.malformed })? {
            polylines.push(polyline);
        }
        Ok(polylines)
    }
}

struct PolylineSeed<'s> {
    state: &'s SeedState,
    malformed: &'s Cell<bool>,
}

impl<'de> DeserializeSeed<'de> for PolylineSeed<'_> {
    type Value = Vec<Vec<f64>>;
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for PolylineSeed<'_> {
    type Value = Vec<Vec<f64>>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of points")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut points = Vec::new();
        loop {
            let count = self.state.points.get() + 1;
            if count > MAX_OM_POINTS {
                if seq.next_element::<IgnoredAny>()?.is_none() {
                    break;
                }
                return Err(self.state.fail(Error::TooLarge(format!("OM workspace has more than {MAX_OM_POINTS} points"))));
            }
            let Some(point) = seq.next_element_seed(PointSeed)? else { break };
            self.state.points.set(count);
            match point {
                Some(xy) => points.push(xy.to_vec()),
                None => self.malformed.set(true),
            }
        }
        Ok(points)
    }
}

/// One point: `Some([x, y])`, or `None` for anything that is not exactly two numbers in an array (B9; the object is
/// named once its map is read).
struct PointSeed;

impl<'de> DeserializeSeed<'de> for PointSeed {
    type Value = Option<[f64; 2]>;
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for PointSeed {
    type Value = Option<[f64; 2]>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a point [x, y]")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let Some(x) = seq.next_element::<f64>()? else { return Ok(None) };
        let Some(y) = seq.next_element::<f64>()? else { return Ok(None) };
        if seq.next_element::<IgnoredAny>()?.is_some() {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            return Ok(None);
        }
        Ok(Some([x, y]))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(None)
    }
    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
}

/// OM `layer_from_dict` over `{"layers": [...]}` or a bare array: missing fields take OM's defaults
/// (power 1 %, 30 kHz, 200 ns, 1000 mm/s, …), `id` stands in for `layer_id`, and an unknown `operation` is refused.
pub fn parse_layer_settings(bytes: &[u8]) -> Result<Vec<OmLayerSpec>, Error> {
    let bytes = check_vector_size("OM layer settings", bytes)?;
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| layer_error(e.to_string()))?;
    let layers = match value.get("layers") {
        Some(layers) => layers,
        None => &value,
    };
    let layers = layers.as_array().ok_or_else(|| layer_error("expected a \"layers\" array".to_owned()))?;
    layers.iter().enumerate().map(|(index, layer)| layer_from_dict(index, layer)).collect()
}

fn layer_from_dict(index: usize, layer: &serde_json::Value) -> Result<OmLayerSpec, Error> {
    let payload = layer.as_object().ok_or_else(|| layer_error(format!("layer {index} is not an object")))?;
    let operation = py_str(payload.get("operation"), "vector_engrave", index, "operation")?;
    if !OPERATIONS.contains(&operation.as_str()) {
        return Err(layer_error(format!("unknown layer operation: {operation}")));
    }
    let layer_id = [payload.get("layer_id"), payload.get("id")]
        .into_iter()
        .flatten()
        .find(|value| py_truthy(value))
        .map(|value| py_str(Some(value), "layer", index, "layer_id"))
        .transpose()?
        .unwrap_or_else(|| "layer".to_owned());
    let float = |key: &str, default: f64| py_float(payload.get(key), default, index, key);
    let boolean = |key: &str, default: bool| payload.get(key).map_or(default, py_truthy);
    Ok(OmLayerSpec {
        layer_id,
        name: py_str(payload.get("name"), "Layer", index, "name")?,
        operation,
        power_percent: float("power_percent", 1.0)?,
        frequency_khz: float("frequency_khz", 30.0)?,
        pulse_width_ns: float("pulse_width_ns", 200.0)?,
        speed_mm_s: float("speed_mm_s", 1000.0)?,
        passes: py_passes(payload.get("passes"), index)?,
        visible: boolean("visible", true),
        output: boolean("output", true),
        color: py_str(payload.get("color"), "#26d0a8", index, "color")?,
        air_assist: boolean("air_assist", false),
        raster_pitch_mm: float("raster_pitch_mm", 0.1)?,
        raster_angle_deg: float("raster_angle_deg", 0.0)?,
        raster_angle_step_deg: float("raster_angle_step_deg", 90.0)?,
    })
}

/// Python `str(value)` for the JSON types OM writes (strings; numbers by their JSON text).
fn py_str(value: Option<&serde_json::Value>, default: &str, index: usize, key: &str) -> Result<String, Error> {
    match value {
        None => Ok(default.to_owned()),
        Some(serde_json::Value::String(text)) => Ok(text.clone()),
        Some(serde_json::Value::Number(number)) => Ok(number.to_string()),
        Some(_) => Err(layer_error(format!("layer {index}: {key} must be a string"))),
    }
}

/// Python `float(value)`: a number, or a string holding one.
fn py_float(value: Option<&serde_json::Value>, default: f64, index: usize, key: &str) -> Result<f64, Error> {
    match value {
        None => Ok(default),
        Some(serde_json::Value::Number(number)) => number.as_f64().ok_or_else(|| layer_error(format!("layer {index}: {key} is not a number"))),
        Some(serde_json::Value::String(text)) => text.trim().parse::<f64>().map_err(|_| layer_error(format!("layer {index}: {key} {text:?} is not a number"))),
        Some(_) => Err(layer_error(format!("layer {index}: {key} must be a number"))),
    }
}

/// Python `int(value)` for passes: integers as given, floats truncated; it must fit a non-negative u32.
fn py_passes(value: Option<&serde_json::Value>, index: usize) -> Result<u32, Error> {
    let out_of_range = || layer_error(format!("layer {index}: passes must be a whole number from 0 to {}", u32::MAX));
    let truncated = match value {
        None => return Ok(1),
        Some(serde_json::Value::Number(number)) => match number.as_u64() {
            Some(whole) => return u32::try_from(whole).map_err(|_| out_of_range()),
            None => number.as_f64().ok_or_else(out_of_range)?.trunc(),
        },
        Some(serde_json::Value::String(text)) => text.trim().parse::<u32>().map_err(|_| out_of_range())?.into(),
        Some(_) => return Err(out_of_range()),
    };
    if !(0.0..=f64::from(u32::MAX)).contains(&truncated) {
        return Err(out_of_range());
    }
    // In range and whole, so the cast is exact.
    Ok(truncated as u32)
}

/// Python truthiness of a JSON value.
fn py_truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_f64().is_some_and(|v| v != 0.0),
        serde_json::Value::String(s) => !s.is_empty(),
        serde_json::Value::Array(a) => !a.is_empty(),
        serde_json::Value::Object(o) => !o.is_empty(),
    }
}

/// OM's four built-in layers (`layers.default_layers`, R05 §4.1).
pub fn builtin_layers() -> Vec<OmLayerSpec> {
    [
        ("vector-engrave", "Vector Engrave", "vector_engrave", 100.0, 1000.0, "#26d0a8"),
        ("vector-cut", "Vector Cut / Deep Mark", "vector_cut", 100.0, 300.0, "#e2b84b"),
        ("raster", "Raster", "raster_engrave", 100.0, 1000.0, "#7aa2ff"),
        ("frame-only", "Frame Only", "frame_only", 0.0, 1000.0, "#93a09c"),
    ]
    .into_iter()
    .map(|(layer_id, name, operation, power_percent, speed_mm_s, color)| OmLayerSpec {
        layer_id: layer_id.to_owned(),
        name: name.to_owned(),
        operation: operation.to_owned(),
        power_percent,
        frequency_khz: 30.0,
        pulse_width_ns: 200.0,
        speed_mm_s,
        passes: 1,
        visible: true,
        output: true,
        color: color.to_owned(),
        air_assist: false,
        raster_pitch_mm: 0.1,
        raster_angle_deg: 0.0,
        raster_angle_step_deg: 90.0,
    })
    .collect()
}

/// Mapping in Appendix B. An object whose `layer_id` is not in the layer settings refuses the whole import,
/// naming the objects (B8: never dropped, never guessed). Each object becomes one `Geom::Path` shape (one subpath
/// per polyline) with `xform` = translate(x, y) · rotate(rotation° CCW) · scale(scale). Polylines of fewer than
/// two points, and objects left with none, are skipped with a notice.
pub fn to_imported(ws: &OmWorkspace, layers: &[OmLayerSpec]) -> Result<Imported, Error> {
    if ws.objects.len() > MAX_OM_OBJECTS {
        return Err(Error::TooLarge(format!("OM workspace has {} objects; the limit is {MAX_OM_OBJECTS}", ws.objects.len())));
    }
    let total_points: usize = ws.objects.iter().flat_map(|object| &object.polylines).map(Vec::len).sum();
    if total_points > MAX_OM_POINTS {
        return Err(Error::TooLarge(format!("OM workspace has {total_points} points; the limit is {MAX_OM_POINTS}")));
    }
    let mut keys = BTreeMap::new();
    let mut used = BTreeSet::new();
    let mut imported_layers = Vec::new();
    let mut next_color = 3u8;
    for spec in layers {
        let key = match spec.layer_id.as_str() {
            "vector-engrave" | "vector_engrave" => LayerKey::Color(0),
            "vector-cut" | "vector_cut" => LayerKey::Color(1),
            "raster" | "raster_engrave" => LayerKey::Color(2),
            "frame-only" | "frame_only" => LayerKey::Tool(1),
            _ => {
                if next_color >= LayerKey::COLORS {
                    return Err(Error::TooLarge(format!("OM layer settings need more than {} colour layers", LayerKey::COLORS)));
                }
                let key = LayerKey::Color(next_color);
                next_color += 1;
                key
            }
        };
        if !used.insert(key) || keys.insert(spec.layer_id.as_str(), key).is_some() {
            return Err(malformed(format!("layer settings define {} ({key}) more than once", spec.layer_id)));
        }
        if let LayerKey::Color(index) = key {
            imported_layers.push((key, layer_from_spec(spec, index)?));
        }
    }
    let missing: Vec<&str> = ws.objects.iter().filter(|object| !keys.contains_key(object.layer_id.as_str())).map(|object| object.name.as_str()).collect();
    if !missing.is_empty() {
        return Err(malformed(format!("objects reference missing layer settings: {}", missing.join(", "))));
    }

    let mut shapes = Vec::new();
    let mut world_lines = Vec::new();
    let mut short_polylines = 0usize;
    let mut empty_objects = Vec::new();
    for object in &ws.objects {
        let Some(layer) = keys.get(object.layer_id.as_str()).copied() else {
            return Err(malformed(format!("objects reference missing layer settings: {}", object.name)));
        };
        if !object.x.is_finite() || !object.y.is_finite() || !object.scale.is_finite() || object.scale <= 0.0 || !object.rotation.is_finite() {
            return Err(malformed(format!("object {} has an invalid transform", object.name)));
        }
        let transform = Affine::translate((object.x, object.y)) * Affine::rotate(object.rotation.to_radians()) * Affine::scale(object.scale);
        let mut path = BezPath::new();
        for raw_line in &object.polylines {
            let mut points = Vec::with_capacity(raw_line.len());
            for raw_point in raw_line {
                let (Some(&x), Some(&y), 2) = (raw_point.first(), raw_point.get(1), raw_point.len()) else {
                    return Err(Error::MalformedPoint { object: object.name.clone(), message: "each point must contain exactly [x, y]".to_owned() });
                };
                if !x.is_finite() || !y.is_finite() {
                    return Err(Error::MalformedPoint { object: object.name.clone(), message: "coordinates must be finite".to_owned() });
                }
                points.push(Point::new(x, y));
            }
            if points.len() < 2 {
                short_polylines += 1;
                continue;
            }
            let mut world = Vec::with_capacity(points.len());
            for (i, p) in points.iter().enumerate() {
                if i == 0 {
                    path.move_to(*p);
                } else {
                    path.line_to(*p);
                }
                let w = transform * *p;
                check_placed(w.x, w.y, &format!("object {}", object.name))?;
                world.push([w.x, w.y]);
            }
            world_lines.push(Polyline(world));
        }
        if path.elements().is_empty() {
            empty_objects.push(object.name.as_str());
            continue;
        }
        let power_scale = Percent::new(100.0).map_err(|e| malformed(e.to_string()))?;
        shapes.push(Shape {
            id: ObjectId(0),
            name: Some(object.name.clone()),
            layer,
            xform: transform,
            geom: Geom::Path(Arc::new(path)),
            locked: false,
            power_scale,
            priority: 0,
        });
    }
    let mut notices = Notices::default();
    if short_polylines > 0 {
        notices.push(NoticeCode::ImportEntitySkipped, format!("{short_polylines} OM polylines with fewer than 2 points were skipped"), None);
    }
    if !empty_objects.is_empty() {
        let names = empty_objects.iter().take(8).copied().collect::<Vec<_>>().join(", ");
        let more = if empty_objects.len() > 8 { format!(" and {} more", empty_objects.len() - 8) } else { String::new() };
        notices.push(
            NoticeCode::ImportEntitySkipped,
            format!("{} OM objects have no drawable polyline and were skipped: {names}{more}", empty_objects.len()),
            None,
        );
    }
    let bounds = openlaser_geom::bounds_of(&world_lines);
    Ok(Imported { shapes, layers: imported_layers, notices: notices.0, bounds })
}

fn layer_from_spec(spec: &OmLayerSpec, index: u8) -> Result<ColorLayer, Error> {
    let frequency = spec.frequency_khz * 1000.0;
    let rounded = frequency.round();
    // kHz × 1000 must be a whole number of hertz; 0.1 kHz steps (16.1 kHz = 16100.000000000002) are whole.
    if !frequency.is_finite() || (frequency - rounded).abs() > 1e-6 || !(1.0..=10_000_000.0).contains(&rounded) {
        return Err(malformed(format!("layer {} frequency {} kHz is not a whole number of hertz in 1 Hz..10 MHz", spec.layer_id, spec.frequency_khz)));
    }
    // A whole number in 1..=10_000_000, so the cast is exact.
    let frequency_hz = rounded as u32;
    let mode = if spec.operation == "raster_engrave" { CutMode::Fill } else { CutMode::Line };
    let passes = spec.passes.max(1);
    let gantry = serde_json::json!({"speed": GANTRY_DEFAULT_SPEED_MM_S, "powerMax": GANTRY_DEFAULT_POWER_PCT, "powerMin": 0.0, "constantPower": false});
    let galvo = serde_json::json!({"speed": spec.speed_mm_s, "power": spec.power_percent, "minPower": 0.0, "frequency": frequency_hz,
        "qPulse": spec.pulse_width_ns, "timings": null, "wobble": null});
    let value = serde_json::json!({
        "index": index, "name": spec.name, "output": spec.output && spec.operation != "disabled", "show": spec.visible,
        "ops": [{"name": spec.name, "enabled": spec.operation != "disabled", "mode": mode, "passes": passes,
            "galvo": galvo, "gantry": gantry,
            "fill": {"interval": spec.raster_pitch_mm, "angle": spec.raster_angle_deg, "angleIncrement": spec.raster_angle_step_deg,
                "bidirectional": true, "crossHatch": false}}],
        "globalPasses": 1,
        "image": {"mode": "threshold", "interval": spec.raster_pitch_mm, "angle": spec.raster_angle_deg, "bidirectional": true, "negative": false,
            "threshold": 128, "passes": passes, "galvo": galvo, "gantry": gantry},
        "airAssist": spec.air_assist
    });
    serde_json::from_value(value).map_err(|e| malformed(format!("invalid layer {}: {e}", spec.layer_id)))
}

fn malformed(message: String) -> Error {
    Error::Malformed { format: "OM workspace", message }
}

fn layer_error(message: String) -> Error {
    Error::Malformed { format: "OM layer settings", message }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequency_accepts_tenth_khz_steps() {
        let mut spec = builtin_layers().remove(0);
        for khz in [16.1, 32.2, 45.0, 0.001, 4000.0] {
            spec.frequency_khz = khz;
            let layer = layer_from_spec(&spec, 0).unwrap_or_else(|e| panic!("{khz} kHz: {e}"));
            assert_eq!(u32::from(layer.ops[0].galvo.frequency), (khz * 1000.0_f64).round() as u32);
        }
        for khz in [16.0001, 0.0005, f64::NAN, 1e9] {
            spec.frequency_khz = khz;
            assert!(layer_from_spec(&spec, 0).is_err(), "{khz} kHz must be refused");
        }
    }

    #[test]
    fn layer_settings_fill_om_defaults_and_accept_id() {
        let settings = parse_layer_settings(br#"[{"id": "custom", "operation": "vector_cut"}, {"layer_id": "", "id": "x"}]"#).expect("parses");
        let first = &settings[0];
        assert_eq!(first.layer_id, "custom");
        assert_eq!(first.name, "Layer");
        assert_eq!((first.power_percent, first.frequency_khz, first.pulse_width_ns, first.speed_mm_s), (1.0, 30.0, 200.0, 1000.0));
        assert_eq!((first.passes, first.visible, first.output, first.air_assist), (1, true, true, false));
        assert_eq!((first.raster_pitch_mm, first.raster_angle_deg, first.raster_angle_step_deg), (0.1, 0.0, 90.0));
        assert_eq!(first.color, "#26d0a8");
        assert_eq!(settings[1].layer_id, "x", "an empty layer_id falls through to id, like Python's `or`");
        assert_eq!(settings[1].operation, "vector_engrave");
        assert!(parse_layer_settings(br#"[{"operation": "laser_everything"}]"#).is_err());
    }

    #[test]
    fn workspace_object_and_point_caps_refuse() {
        let object = r#"{"name":"o","layer_id":"vector-engrave","polylines":[[[0,0],[1,1]]],"x":0,"y":0,"scale":1,"rotation":0}"#;
        let at_cap = format!(r#"{{"version":1,"objects":[{}]}}"#, vec![object; MAX_OM_OBJECTS].join(","));
        assert_eq!(parse_workspace(at_cap.as_bytes()).expect("3000 objects are fine").objects.len(), MAX_OM_OBJECTS);
        let over = format!(r#"{{"version":1,"objects":[{}]}}"#, vec![object; MAX_OM_OBJECTS + 1].join(","));
        assert!(matches!(parse_workspace(over.as_bytes()), Err(Error::TooLarge(_))));

        let points = vec!["[0,0]"; MAX_OM_POINTS + 1].join(",");
        let many =
            format!(r#"{{"version":1,"objects":[{{"name":"big","layer_id":"vector-engrave","polylines":[[{points}]],"x":0,"y":0,"scale":1,"rotation":0}}]}}"#);
        assert!(matches!(parse_workspace(many.as_bytes()), Err(Error::TooLarge(_))));
    }

    #[test]
    fn short_polylines_and_empty_objects_are_reported() {
        let ws = parse_workspace(
            br#"{"version":1,"objects":[{"name":"dot","layer_id":"vector-engrave","polylines":[[[0,0]]],"x":0,"y":0,"scale":1,"rotation":0},
            {"name":"line","layer_id":"vector-engrave","polylines":[[[0,0],[1,0]],[]],"x":0,"y":0,"scale":1,"rotation":0}]}"#,
        )
        .expect("parses");
        let imported = to_imported(&ws, &builtin_layers()).expect("imports");
        assert_eq!(imported.shapes.len(), 1);
        assert_eq!(imported.notices.len(), 2, "{:?}", imported.notices);
        assert!(imported.notices.iter().all(|n| n.code == NoticeCode::ImportEntitySkipped));
        assert!(imported.notices[1].message.contains("dot"));
    }

    #[test]
    fn non_finite_after_transform_is_refused() {
        let ws = parse_workspace(
            br#"{"version":1,"objects":[{"name":"far","layer_id":"vector-engrave","polylines":[[[1e300,0],[1,0]]],"x":0,"y":0,"scale":1e10,"rotation":0}]}"#,
        )
        .expect("parses");
        assert!(to_imported(&ws, &builtin_layers()).is_err());
    }
}
