//! T0 numeric tables (`tests/golden/t0_numeric/*.json`). Owned by WP4 (SPEC §7.12).

use serde_json::{Map, Value};

use crate::{golden_dir, sha256_hex};

/// One table. The fixture keeps its rows inside `groups` (one group per field size or parameter set).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    pub function: String,
    /// Every other top-level key (`n_rows`, `inputs_f64_sha256`, `formula`, `meta`, …), as in the fixture.
    pub params: serde_json::Map<String, serde_json::Value>,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub errors: Vec<Vec<serde_json::Value>>,
    pub specials: Vec<Vec<serde_json::Value>>,
    pub groups: Vec<Group>,
}

/// One group: its parameters (`field_size_mm`, `inputs_f64_sha256`, …) and its rows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Group {
    pub params: serde_json::Map<String, serde_json::Value>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

/// Loads `t0_numeric/<file>`.
pub fn load(file: &str) -> Result<Table, String> {
    if file.is_empty() || file.contains(['/', '\\']) || !file.ends_with(".json") {
        return Err(format!("invalid T0 fixture name: {file}"));
    }
    let path = golden_dir().join("t0_numeric").join(file);
    let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let root: Value = serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))?;
    let obj = root.as_object().ok_or_else(|| format!("{}: root is not an object", path.display()))?;

    let params = obj.iter().filter(|(k, _)| !matches!(k.as_str(), "rows" | "errors" | "specials" | "groups")).map(|(k, v)| (k.clone(), v.clone())).collect();
    Ok(Table {
        function: string_field(obj, "function", &path)?,
        params,
        columns: string_array(obj, "columns", &path)?,
        rows: row_array(obj.get("rows"), "rows", &path)?,
        errors: row_array(obj.get("errors"), "errors", &path)?,
        specials: row_array(obj.get("specials"), "specials", &path)?,
        groups: groups(obj.get("groups"), &path)?,
    })
}

/// The fixtures' `inputs_f64_sha256`: sha256 over the little-endian IEEE-754 bit patterns of `values`, in order
/// (`generator/common.py` `float_bits_sha256`). Recomputing it proves every float input was parsed exactly.
pub fn f64_bits_sha256(values: impl IntoIterator<Item = f64>) -> String {
    let bytes: Vec<u8> = values.into_iter().flat_map(f64::to_le_bytes).collect();
    sha256_hex(&bytes)
}

fn string_field(obj: &Map<String, Value>, key: &str, path: &std::path::Path) -> Result<String, String> {
    obj.get(key).and_then(Value::as_str).map(str::to_owned).ok_or_else(|| format!("{}: missing string {key}", path.display()))
}

fn string_array(obj: &Map<String, Value>, key: &str, path: &std::path::Path) -> Result<Vec<String>, String> {
    let values = obj.get(key).and_then(Value::as_array).ok_or_else(|| format!("{}: missing array {key}", path.display()))?;
    values.iter().map(|v| v.as_str().map(str::to_owned).ok_or_else(|| format!("{}: {key} contains a non-string", path.display()))).collect()
}

fn row_array(value: Option<&Value>, key: &str, path: &std::path::Path) -> Result<Vec<Vec<Value>>, String> {
    let Some(value) = value else { return Ok(Vec::new()) };
    let rows = value.as_array().ok_or_else(|| format!("{}: {key} is not an array", path.display()))?;
    rows.iter().map(|row| row.as_array().cloned().ok_or_else(|| format!("{}: {key} contains a non-array row", path.display()))).collect()
}

fn groups(value: Option<&Value>, path: &std::path::Path) -> Result<Vec<Group>, String> {
    let Some(value) = value else { return Ok(Vec::new()) };
    let values = value.as_array().ok_or_else(|| format!("{}: groups is not an array", path.display()))?;
    values
        .iter()
        .map(|value| {
            let mut params = value.as_object().cloned().ok_or_else(|| format!("{}: group is not an object", path.display()))?;
            let rows = row_array(params.remove("rows").as_ref(), "groups[].rows", path)?;
            Ok(Group { params, rows })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f64_bits_sha256_hashes_le_bit_patterns() {
        // Python: hashlib.sha256(struct.pack("<d", 1.0) + struct.pack("<d", -0.0)).hexdigest()
        assert_eq!(f64_bits_sha256([1.0, -0.0]), "5e9d905ef08923718da5998eb8ae14dc75a714656224d3753b679523d5a268d9");
        assert_ne!(f64_bits_sha256([0.0]), f64_bits_sha256([-0.0]));
        assert_eq!(f64_bits_sha256([]), sha256_hex(b""));
    }

    #[test]
    fn tables_keep_their_top_level_parameters() {
        let t = load("power_to_mark_current.json").unwrap();
        assert_eq!(t.params.get("n_rows").and_then(Value::as_u64), Some(20_018));
        assert!(t.params.get("inputs_f64_sha256").and_then(Value::as_str).is_some());
        assert!(!t.params.contains_key("rows"));
        assert!(load("../x.json").is_err());
    }
}
