//! T1 LMC scenarios (`tests/golden/t1_lmc/{v0,v1}/*.json`, `packets/*.hex`). Owned by WP4 (SPEC §7.12).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::{golden_dir, hex_decode, sha256_hex};

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Open,
    Single { op: String, hex: String },
    Read { status: u16 },
    List { sha256: String, n_cmds: u16 },
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    pub scenario: String,
    pub inputs: serde_json::Value,
    pub field_size_mm: f64,
    pub events: Vec<Event>,
    /// Named event ranges, e.g. `"init": [0, 39]`.
    pub segments: BTreeMap<String, [usize; 2]>,
}

/// Loads `t1_lmc/<oracle>/<scenario>.json` (`oracle` is `v0` or `v1`).
pub fn load(oracle: &str, scenario: &str) -> Result<Scenario, String> {
    if !matches!(oracle, "v0" | "v1") {
        return Err(format!("invalid T1 oracle: {oracle}"));
    }
    if scenario.is_empty() || scenario.contains(['/', '\\']) {
        return Err(format!("invalid T1 scenario: {scenario}"));
    }
    let path = golden_dir().join("t1_lmc").join(oracle).join(format!("{scenario}.json"));
    let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let root: Value = serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))?;
    let obj = root.as_object().ok_or_else(|| format!("{}: root is not an object", path.display()))?;
    let scenario_name = obj.get("scenario").and_then(Value::as_str).unwrap_or(scenario).to_owned();
    let inputs = obj.get("inputs").cloned().unwrap_or(Value::Null);
    let field_size_mm = obj.get("field_size_mm").and_then(Value::as_f64).unwrap_or(0.0);
    let events = parse_events(obj.get("events"), &path)?;
    let segments = parse_segments(obj.get("segments"), &path)?;
    Ok(Scenario { scenario: scenario_name, inputs, field_size_mm, events, segments })
}

/// The 3072 bytes of `t1_lmc/packets/<sha256>.hex`.
pub fn packet(sha256: &str) -> Result<Vec<u8>, String> {
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("invalid packet sha256: {sha256}"));
    }
    let path = golden_dir().join("t1_lmc").join("packets").join(format!("{sha256}.hex"));
    let text = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let bytes = hex_decode(&text)?;
    if bytes.len() != 3072 {
        return Err(format!("{}: expected 3072 bytes, got {}", path.display(), bytes.len()));
    }
    let actual = sha256_hex(&bytes);
    if actual != sha256.to_ascii_lowercase() {
        return Err(format!("{}: sha256 is {actual}, expected {sha256}", path.display()));
    }
    Ok(bytes)
}

fn parse_events(value: Option<&Value>, path: &std::path::Path) -> Result<Vec<Event>, String> {
    let values = value.and_then(Value::as_array).ok_or_else(|| format!("{}: missing events", path.display()))?;
    values
        .iter()
        .map(|value| {
            let obj = value.as_object().ok_or_else(|| format!("{}: event is not an object", path.display()))?;
            let kind = obj.get("kind").and_then(Value::as_str).ok_or_else(|| format!("{}: event has no kind", path.display()))?;
            match kind {
                "open" => Ok(Event::Open),
                "close" => Ok(Event::Close),
                "single" => Ok(Event::Single { op: required_string(obj, "op", path)?, hex: required_string(obj, "hex", path)? }),
                "read" => Ok(Event::Read { status: required_u16(obj, "status", path)? }),
                "list" => Ok(Event::List { sha256: required_string(obj, "sha256", path)?, n_cmds: required_u16(obj, "n_cmds", path)? }),
                _ => Err(format!("{}: unknown event kind {kind}", path.display())),
            }
        })
        .collect()
}

fn parse_segments(value: Option<&Value>, path: &std::path::Path) -> Result<BTreeMap<String, [usize; 2]>, String> {
    let Some(obj) = value.and_then(Value::as_object) else { return Ok(BTreeMap::new()) };
    obj.iter()
        .map(|(name, value)| {
            let pair = value.as_array().filter(|p| p.len() == 2).ok_or_else(|| format!("{}: bad segment {name}", path.display()))?;
            let start =
                pair.first().and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| format!("{}: bad segment start", path.display()))?;
            let end =
                pair.get(1).and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| format!("{}: bad segment end", path.display()))?;
            Ok((name.clone(), [start, end]))
        })
        .collect()
}

fn required_string(obj: &serde_json::Map<String, Value>, key: &str, path: &std::path::Path) -> Result<String, String> {
    obj.get(key).and_then(Value::as_str).map(str::to_owned).ok_or_else(|| format!("{}: missing string {key}", path.display()))
}

fn required_u16(obj: &serde_json::Map<String, Value>, key: &str, path: &std::path::Path) -> Result<u16, String> {
    let value = obj.get(key).and_then(Value::as_u64).ok_or_else(|| format!("{}: missing integer {key}", path.display()))?;
    u16::try_from(value).map_err(|_| format!("{}: {key} does not fit u16", path.display()))
}
