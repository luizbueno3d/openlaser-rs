//! T1 LMC scenarios (`tests/golden/t1_lmc/{v0,v1}/*.json`, `packets/*.hex`). Owned by WP4 (SPEC §7.12). WP0 stub.

use std::collections::BTreeMap;

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
    Err(format!("not implemented: openlaser_testkit::t1::load({oracle}, {scenario})"))
}

/// The 3072 bytes of `t1_lmc/packets/<sha256>.hex`.
pub fn packet(sha256: &str) -> Result<Vec<u8>, String> {
    Err(format!("not implemented: openlaser_testkit::t1::packet({sha256})"))
}
