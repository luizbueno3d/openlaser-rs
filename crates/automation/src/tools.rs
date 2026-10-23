//! The tool table (§4.1) and strict schemas. WP0 stubs: no tools yet.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Headless,
    Bridge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolAccess {
    Read,
    Agent,
    Approval,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hints {
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
}

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    /// Strict: additionalProperties:false everywhere, $refs inlined.
    pub input_schema: fn() -> serde_json::Value,
    pub modes: &'static [Mode],
    pub access: ToolAccess,
    pub hints: Hints,
    /// Listed under --compact (`stop` always is).
    pub compact: bool,
}

/// §4.1, in that order. WP0 stub: empty.
pub fn tools() -> &'static [ToolDef] {
    &[]
}

/// WP0 stub: the schema unchanged.
pub fn strict_schema(s: schemars::Schema) -> serde_json::Value {
    s.to_value()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}
