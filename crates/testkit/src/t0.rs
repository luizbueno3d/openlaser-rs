//! T0 numeric tables (`tests/golden/t0_numeric/*.json`). Owned by WP4 (SPEC §7.12). WP0 stub.

/// One table. The fixture keeps its rows inside `groups` (one group per field size or parameter set).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    pub function: String,
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
    Err(format!("not implemented: openlaser_testkit::t0::load({file})"))
}
