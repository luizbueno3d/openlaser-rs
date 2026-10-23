//! T4 importer fixture loader (`tests/golden/t4_import/`; SPEC §6.2). Owned by WP2.

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
    if let Some(data_url) = case.source.get("data_url").and_then(serde_json::Value::as_str) {
        let (_, encoded) = data_url.split_once(",").ok_or_else(|| format!("{} has malformed data URL", case.name))?;
        return decode_base64(encoded).map_err(|error| format!("{} data URL: {error}", case.name));
    }
    let Some(file) = case.source.get("file").and_then(serde_json::Value::as_str) else {
        return Err(format!("{} has no byte source", case.name));
    };
    let path = crate::golden_dir().join(file);
    fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err("base64 length is not divisible by four".to_owned());
    }
    let mut output = Vec::with_capacity(bytes.len() / 4 * 3);
    for (chunk_index, chunk) in bytes.chunks(4).enumerate() {
        let value = |offset: usize| -> Result<u32, String> {
            let byte = *chunk.get(offset).ok_or_else(|| "incomplete base64 quartet".to_owned())?;
            if byte == b'=' {
                return Ok(0);
            }
            ALPHABET.iter().position(|candidate| *candidate == byte).map(|index| index as u32).ok_or_else(|| "invalid base64 character".to_owned())
        };
        let a = value(0)?;
        let b = value(1)?;
        let c = value(2)?;
        let d = value(3)?;
        let packed = (a << 18) | (b << 12) | (c << 6) | d;
        output.push(((packed >> 16) & 0xff) as u8);
        if chunk.get(2) != Some(&b'=') {
            output.push(((packed >> 8) & 0xff) as u8);
        }
        if chunk.get(3) != Some(&b'=') {
            output.push((packed & 0xff) as u8);
        }
        if (chunk.get(2) == Some(&b'=') && chunk.get(3) != Some(&b'=')) || (chunk_index + 1 != bytes.len() / 4 && chunk.contains(&b'=')) {
            return Err("invalid base64 padding".to_owned());
        }
    }
    Ok(output)
}
