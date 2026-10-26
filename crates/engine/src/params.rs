//! Strict params decoding (SPEC §3.16 `decode`, ENG §2.2): snake_case keys are normalised to camelCase, the
//! universal keys `coalesce` and `dryRun` are stripped, and the rest is decoded with `deny_unknown_fields`.
//!
//! Errors name the JSON pointer of the offending value. The decoder walks the `serde_json::Value` itself (a small
//! path-tracking `Deserializer`), so the pointer is exact without an extra dependency.

use std::fmt;

use schemars::JsonSchema;
use serde::de::{self, DeserializeOwned, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess, SeqAccess, VariantAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::command::EngineError;

/// Keys every command accepts; stripped before the strict decode.
pub const UNIVERSAL_KEYS: [&str; 2] = ["coalesce", "dryRun"];

/// Deeper params than this are refused before anything walks them (adversarial "deep arrays").
pub const MAX_PARAMS_DEPTH: usize = 64;

/// Normalises snake_case keys to camelCase (recursively), strips the universal keys `coalesce` and `dryRun`,
/// then decodes with deny_unknown_fields. Errors name the JSON pointer and the accepted keys.
pub fn decode<P: DeserializeOwned>(id: &str, params: Value) -> Result<P, EngineError> {
    let obj = match params {
        Value::Null => Map::new(),
        Value::Object(m) => m,
        other => return Err(bad(id, "", format!("params must be a JSON object or null, not {}", type_name(&other)))),
    };
    let mut obj = normalise_object(id, obj, "", 0)?;
    for k in UNIVERSAL_KEYS {
        obj.remove(k);
    }
    let value = Value::Object(obj);
    P::deserialize(PathDe { value: &value, path: String::new() }).map_err(|e| bad(id, e.pointer.as_deref().unwrap_or(""), e.message))
}

pub(crate) fn bad(id: &str, pointer: &str, message: impl Into<String>) -> EngineError {
    EngineError::BadParams { id: id.to_owned(), pointer: pointer.to_owned(), message: message.into() }
}

pub(crate) fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// `speed_mm_s` → `speedMmS`, `flip_h` → `flipH`, `rotate_cw_90` → `rotateCw90`. Leading underscores are kept;
/// a string without `_` is returned unchanged.
pub fn snake_to_camel(s: &str) -> String {
    if !s.contains('_') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut seen_other = false;
    let mut upper = false;
    for c in s.chars() {
        if c == '_' {
            if seen_other {
                upper = true;
            } else {
                out.push(c);
            }
            continue;
        }
        seen_other = true;
        if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// A command id with each `.`-separated segment converted from snake_case (`layer.cut_settings.set`).
pub fn normalise_id(id: &str) -> String {
    id.split('.').map(snake_to_camel).collect::<Vec<_>>().join(".")
}

pub(crate) fn pointer_child(path: &str, key: &str) -> String {
    let escaped = key.replace('~', "~0").replace('/', "~1");
    format!("{path}/{escaped}")
}

fn normalise_object(id: &str, obj: Map<String, Value>, path: &str, depth: usize) -> Result<Map<String, Value>, EngineError> {
    if depth > MAX_PARAMS_DEPTH {
        return Err(bad(id, path, format!("params nest deeper than {MAX_PARAMS_DEPTH} levels")));
    }
    let mut out = Map::new();
    for (k, v) in obj {
        let key = snake_to_camel(&k);
        let child = pointer_child(path, &key);
        if out.contains_key(&key) {
            return Err(bad(id, &child, format!("`{k}` is given twice (as snake_case and camelCase)")));
        }
        let v = normalise_value(id, v, &child, depth + 1)?;
        out.insert(key, v);
    }
    Ok(out)
}

fn normalise_value(id: &str, v: Value, path: &str, depth: usize) -> Result<Value, EngineError> {
    if depth > MAX_PARAMS_DEPTH {
        return Err(bad(id, path, format!("params nest deeper than {MAX_PARAMS_DEPTH} levels")));
    }
    Ok(match v {
        Value::Object(m) => Value::Object(normalise_object(id, m, path, depth)?),
        Value::Array(a) => {
            let mut out = Vec::with_capacity(a.len());
            for (i, item) in a.into_iter().enumerate() {
                out.push(normalise_value(id, item, &format!("{path}/{i}"), depth + 1)?);
            }
            Value::Array(out)
        }
        other => other,
    })
}

// ---- the path-tracking deserializer --------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct DeError {
    pointer: Option<String>,
    message: String,
}

impl DeError {
    /// The innermost location wins: an error already placed keeps its pointer.
    fn at(mut self, path: &str) -> Self {
        if self.pointer.is_none() {
            self.pointer = Some(path.to_owned());
        }
        self
    }
}

impl fmt::Display for DeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DeError {}

impl de::Error for DeError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        DeError { pointer: None, message: msg.to_string() }
    }
}

struct PathDe<'a> {
    value: &'a Value,
    path: String,
}

impl<'de> Deserializer<'de> for PathDe<'_> {
    type Error = DeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        let path = self.path;
        let r: Result<V::Value, DeError> = match self.value {
            Value::Null => visitor.visit_unit(),
            Value::Bool(b) => visitor.visit_bool(*b),
            Value::Number(n) => {
                if let Some(u) = n.as_u64() {
                    visitor.visit_u64(u)
                } else if let Some(i) = n.as_i64() {
                    visitor.visit_i64(i)
                } else if let Some(f) = n.as_f64() {
                    visitor.visit_f64(f)
                } else {
                    Err(de::Error::custom("number is not representable"))
                }
            }
            Value::String(s) => visitor.visit_str(s),
            Value::Array(a) => {
                let mut seq = Seq { iter: a.iter(), index: 0, path: &path };
                let v = visitor.visit_seq(&mut seq)?;
                if seq.iter.len() == 0 { Ok(v) } else { Err(de::Error::custom(format!("expected {} elements, got {}", seq.index, a.len()))) }
            }
            Value::Object(m) => {
                let mut map = MapA { iter: m.iter(), current: None, path: &path };
                let v = visitor.visit_map(&mut map)?;
                match map.iter.next() {
                    None => Ok(v),
                    Some((k, _)) => Err(de::Error::custom(format!("unexpected key `{k}`"))),
                }
            }
        };
        r.map_err(|e| e.at(&path))
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.value {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _name: &'static str, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(self, _name: &'static str, _variants: &'static [&'static str], visitor: V) -> Result<V::Value, DeError> {
        let path = self.path;
        let r: Result<V::Value, DeError> = match self.value {
            Value::String(s) => visitor.visit_enum(s.as_str().into_deserializer()),
            Value::Object(m) if m.len() == 1 => match m.iter().next() {
                Some((k, v)) => visitor.visit_enum(EnumDe { variant: k, value: v, path: pointer_child(&path, k) }),
                None => Err(de::Error::custom("expected an enum variant")),
            },
            other => Err(de::Error::custom(format!("expected a string (enum variant), not {}", type_name(other)))),
        };
        r.map_err(|e| e.at(&path))
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_unit()
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf unit unit_struct seq tuple
        tuple_struct map struct identifier
    }
}

struct Seq<'a, 'p> {
    iter: std::slice::Iter<'a, Value>,
    index: usize,
    path: &'p str,
}

impl<'de> SeqAccess<'de> for Seq<'_, '_> {
    type Error = DeError;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, DeError> {
        let Some(v) = self.iter.next() else { return Ok(None) };
        let p = format!("{}/{}", self.path, self.index);
        self.index += 1;
        seed.deserialize(PathDe { value: v, path: p.clone() }).map(Some).map_err(|e| e.at(&p))
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.iter.len())
    }
}

struct MapA<'a, 'p> {
    iter: serde_json::map::Iter<'a>,
    current: Option<(&'a String, &'a Value)>,
    path: &'p str,
}

impl<'de> MapAccess<'de> for MapA<'_, '_> {
    type Error = DeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, DeError> {
        let Some((k, v)) = self.iter.next() else { return Ok(None) };
        self.current = Some((k, v));
        let p = pointer_child(self.path, k);
        seed.deserialize(k.as_str().into_deserializer()).map(Some).map_err(|e: DeError| e.at(&p))
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, DeError> {
        let Some((k, v)) = self.current.take() else { return Err(de::Error::custom("a value was requested before its key")) };
        let p = pointer_child(self.path, k);
        seed.deserialize(PathDe { value: v, path: p.clone() }).map_err(|e| e.at(&p))
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.iter.len())
    }
}

struct EnumDe<'a> {
    variant: &'a str,
    value: &'a Value,
    path: String,
}

impl<'de, 'a> EnumAccess<'de> for EnumDe<'a> {
    type Error = DeError;
    type Variant = VariantDe<'a>;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, VariantDe<'a>), DeError> {
        let v = seed.deserialize(self.variant.into_deserializer())?;
        Ok((v, VariantDe { value: self.value, path: self.path }))
    }
}

struct VariantDe<'a> {
    value: &'a Value,
    path: String,
}

impl<'de> VariantAccess<'de> for VariantDe<'_> {
    type Error = DeError;

    fn unit_variant(self) -> Result<(), DeError> {
        match self.value {
            Value::Null => Ok(()),
            other => Err(DeError { pointer: Some(self.path), message: format!("expected no value, got {}", type_name(other)) }),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, DeError> {
        seed.deserialize(PathDe { value: self.value, path: self.path })
    }

    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, DeError> {
        PathDe { value: self.value, path: self.path }.deserialize_seq(visitor)
    }

    fn struct_variant<V: Visitor<'de>>(self, _fields: &'static [&'static str], visitor: V) -> Result<V::Value, DeError> {
        PathDe { value: self.value, path: self.path }.deserialize_map(visitor)
    }
}

// ---- shared param types ----------------------------------------------------------------------------------------

/// `{}`: the command takes no params.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct NoParams {}

/// `{ids?}`: object ids; default: the selection.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct IdsParams {
    /// Object ids. Default: the current selection.
    #[serde(default)]
    pub ids: Option<Vec<u64>>,
}

/// JSON Schema of a layer key: 0..=29, or "C05", "05", "T1", "T2" (case-insensitive).
pub struct LayerKeySchema;

impl JsonSchema for LayerKeySchema {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "LayerKey".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "A LightBurn layer: colour 0..=29, or \"C05\", \"05\", \"T1\", \"T2\" (case-insensitive).",
            "oneOf": [
                { "type": "integer", "minimum": 0, "maximum": 29 },
                { "type": "string", "pattern": "^([CcTt]?[0-9]{1,2})$" }
            ]
        })
    }
}

/// `{layer}`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LayerParams {
    /// Layer: 0..=29, "C05", "05", "T1" or "T2".
    #[schemars(with = "LayerKeySchema")]
    pub layer: openlaser_types::LayerKey,
}

/// `{device?}`: a machine profile id; default the active document's machine.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DeviceParams {
    /// Machine profile id (`device.list`). Default: the active document's machine.
    #[serde(default)]
    pub device: Option<String>,
}

/// `{machine?}` for the Stop commands.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MachineParams {
    /// Machine profile id. Default: every connected machine.
    #[serde(default)]
    pub machine: Option<String>,
}

/// `{path}`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PathParams {
    /// File path. For agents it must resolve inside the granted automation roots.
    pub path: String,
}

/// Frame outline style (S21: never part of the fingerprint).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum FrameStyleP {
    Bounds,
    Contour,
}

impl From<FrameStyleP> for openlaser_types::FrameStyle {
    fn from(v: FrameStyleP) -> Self {
        match v {
            FrameStyleP::Bounds => openlaser_types::FrameStyle::Bounds,
            FrameStyleP::Contour => openlaser_types::FrameStyle::Contour,
        }
    }
}

/// What a frame traces (S8): the job, the tool layers T1/T2 only (never counts toward Start), or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum FrameSourceP {
    Job,
    #[serde(alias = "tool_layers_only")]
    ToolLayersOnly,
    #[serde(alias = "job_and_tool_layers")]
    JobAndToolLayers,
}

impl From<FrameSourceP> for openlaser_types::FrameSource {
    fn from(v: FrameSourceP) -> Self {
        match v {
            FrameSourceP::Job => openlaser_types::FrameSource::Job,
            FrameSourceP::ToolLayersOnly => openlaser_types::FrameSource::ToolLayersOnly,
            FrameSourceP::JobAndToolLayers => openlaser_types::FrameSource::JobAndToolLayers,
        }
    }
}

/// Frame once, or loop until stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum FrameModeP {
    Once,
    Loop,
}

impl From<FrameModeP> for openlaser_types::FrameMode {
    fn from(v: FrameModeP) -> Self {
        match v {
            FrameModeP::Once => openlaser_types::FrameMode::Once,
            FrameModeP::Loop => openlaser_types::FrameMode::Loop,
        }
    }
}

/// A layer's cut mode (Offset Fill is P1: the planner refuses it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CutModeP {
    Line,
    Fill,
    #[serde(alias = "offset_fill")]
    OffsetFill,
}

impl From<CutModeP> for openlaser_doc::CutMode {
    fn from(v: CutModeP) -> Self {
        match v {
            CutModeP::Line => openlaser_doc::CutMode::Line,
            CutModeP::Fill => openlaser_doc::CutMode::Fill,
            CutModeP::OffsetFill => openlaser_doc::CutMode::OffsetFill,
        }
    }
}

/// Image dithering (S16 subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ImageModeP {
    Threshold,
    Jarvis,
}

impl From<ImageModeP> for openlaser_doc::ImageMode {
    fn from(v: ImageModeP) -> Self {
        match v {
            ImageModeP::Threshold => openlaser_doc::ImageMode::Threshold,
            ImageModeP::Jarvis => openlaser_doc::ImageMode::Jarvis,
        }
    }
}

/// The reference point of a transform on the selection's bounding box (Y up: top = larger y).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AnchorP {
    #[default]
    Center,
    TopLeft,
    Top,
    TopRight,
    Left,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

/// How `edit.select` combines with the current selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SelectModeP {
    #[default]
    Replace,
    Add,
    Remove,
    Toggle,
}

/// SVG units: physical (refuse unitless files), or 96 / 72 px per inch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SvgUnitsP {
    #[default]
    Physical,
    Px96,
    Px72,
}

/// `command_list` agent class filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AgentClassP {
    Allowed,
    RootsOnly,
    Ui,
    HumanOnly,
    Stop,
}

/// A rectangle in design mm, Y up.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RectMm {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

/// A finite number, or BadParams at `pointer`.
pub(crate) fn finite(id: &str, pointer: &str, v: f64) -> Result<f64, EngineError> {
    if v.is_finite() { Ok(v) } else { Err(bad(id, pointer, format!("{v} is not a finite number"))) }
}

/// A finite number > 0 and ≤ `max`, or BadParams at `pointer`.
pub(crate) fn positive(id: &str, pointer: &str, v: f64, max: f64) -> Result<f64, EngineError> {
    if v.is_finite() && v > 0.0 && v <= max { Ok(v) } else { Err(bad(id, pointer, format!("{v} is outside 0 (exclusive)..={max}"))) }
}

/// A short single-line name (layer, project, profile): no control characters, at most `max` characters.
pub(crate) fn name(id: &str, pointer: &str, v: &str, max: usize) -> Result<String, EngineError> {
    if v.chars().count() > max {
        return Err(bad(id, pointer, format!("names are at most {max} characters")));
    }
    if v.chars().any(char::is_control) {
        return Err(bad(id, pointer, "names cannot contain control characters"));
    }
    Ok(v.to_owned())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    struct Inner {
        speed_mm_s: f64,
        #[serde(default)]
        power_pct: Option<openlaser_types::Percent>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    struct Outer {
        layer: openlaser_types::LayerKey,
        #[serde(default)]
        galvo: Option<Inner>,
        #[serde(default)]
        list: Vec<u8>,
        #[serde(default)]
        mode: Option<Mode>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "camelCase")]
    enum Mode {
        Line,
        OffsetFill,
    }

    fn err(v: Value) -> (String, String) {
        match decode::<Outer>("t.cmd", v) {
            Err(EngineError::BadParams { pointer, message, .. }) => (pointer, message),
            other => panic!("expected BadParams, got {other:?}"),
        }
    }

    #[test]
    fn snake_keys_normalise_and_universal_keys_are_stripped() {
        let p: Outer = decode("t.cmd", json!({"layer": "C05", "galvo": {"speed_mm_s": 10, "power_pct": 30}, "coalesce": "k", "dry_run": true})).unwrap();
        assert_eq!(p.layer, openlaser_types::LayerKey::Color(5));
        assert_eq!(p.galvo.unwrap().speed_mm_s, 10.0);
    }

    #[test]
    fn null_is_an_empty_object_and_non_objects_are_refused() {
        assert_eq!(err(json!([1])).0, "");
        assert!(err(Value::Null).1.contains("layer"), "missing field names the field");
    }

    #[test]
    fn pointers_name_the_offending_value() {
        assert_eq!(err(json!({"layer": 3, "galvo": {"speedMmS": 1, "powerPct": 150}})).0, "/galvo/powerPct");
        assert_eq!(err(json!({"layer": 3, "galvo": {"speedMmS": "x"}})).0, "/galvo/speedMmS");
        assert_eq!(err(json!({"layer": 3, "list": [1, 2, 300]})).0, "/list/2");
        assert_eq!(err(json!({"layer": 31})).0, "/layer");
        let (p, m) = err(json!({"layer": 3, "bogus": 1}));
        assert_eq!(p, "/bogus");
        assert!(m.contains("expected one of") && m.contains("galvo"), "{m}");
        assert_eq!(err(json!({"layer": 3, "galvo": {}})).0, "/galvo");
        assert_eq!(err(json!({"layer": 3, "mode": "weird"})).0, "/mode");
    }

    #[test]
    fn enums_accept_camel_case_and_snake_aliases_resolve_through_keys_only() {
        let p: Outer = decode("t.cmd", json!({"layer": 0, "mode": "offsetFill"})).unwrap();
        assert_eq!(p.mode, Some(Mode::OffsetFill));
    }

    #[test]
    fn duplicate_keys_after_normalising_are_refused() {
        let (p, _) = err(json!({"layer": 1, "galvo": {"speedMmS": 1, "speed_mm_s": 2}}));
        assert_eq!(p, "/galvo/speedMmS");
    }

    #[test]
    fn deep_params_are_refused_without_recursion_blowup() {
        let mut v = json!(1);
        for _ in 0..200 {
            v = json!([v]);
        }
        let (_, m) = err(json!({"layer": 1, "list": v}));
        assert!(m.contains("nest deeper"), "{m}");
    }

    #[test]
    fn snake_to_camel_cases() {
        assert_eq!(snake_to_camel("flip_h"), "flipH");
        assert_eq!(snake_to_camel("rotate_cw_90"), "rotateCw90");
        assert_eq!(snake_to_camel("speedMmS"), "speedMmS");
        assert_eq!(snake_to_camel("_x"), "_x");
        assert_eq!(normalise_id("layer.cut_settings.set"), "layer.cutSettings.set");
        assert_eq!(pointer_child("", "a/b~c"), "/a~1b~0c");
    }
}
