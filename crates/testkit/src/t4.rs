//! T4 importer fixture loader.

use std::fs;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Case {
    pub name: String,
    pub kind: String,
    pub function: String,
    pub filename: String,
    pub source: serde_json::Value,
    pub rust_must_match: bool,
    pub output: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct FileCases {
    cases: Vec<Case>,
}

pub fn load() -> Result<Vec<Case>, String> {
    let dir = crate::golden_dir().join("t4_import");
    let mut cases = Vec::new();
    for name in ["import.json", "businessCard_dxf.json"] {
        let path = dir.join(name);
        let bytes = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let file: FileCases = serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))?;
        cases.extend(file.cases);
    }
    Ok(cases)
}

pub fn source_bytes(case: &Case) -> Result<Vec<u8>, String> {
    if let Some(text) = case.source.get("inline_text").and_then(serde_json::Value::as_str) {
        return Ok(text.as_bytes().to_vec());
    }
    let Some(file) = case.source.get("file").and_then(serde_json::Value::as_str) else {
        return Err(format!("{} has no byte source", case.name));
    };
    let path = crate::golden_dir().join(file);
    fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))
}
