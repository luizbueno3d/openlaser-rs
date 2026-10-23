//! The registry (ENG §2.2, with typed params). WP0 stubs: the registry is empty and `command!` is not usable yet.

use std::collections::HashMap;
use std::sync::OnceLock;

use openlaser_types::{ObjectId, Refusal};
use serde::Serialize;

use crate::session::{JobRefused, Session};

pub type RunFn = fn(&mut Session, serde_json::Value) -> Result<serde_json::Value, EngineError>;

#[derive(Clone, Copy)]
pub struct CommandSpec {
    /// "layer.cutSettings.set" (camelCase segments, S2).
    pub id: &'static str,
    /// English; also the i18n key.
    pub label: &'static str,
    /// LightBurn menu path; [] = not in menus.
    pub menu: &'static [&'static str],
    /// "Cmd+Shift+H"; Cmd = ⌘ on macOS, Ctrl elsewhere.
    pub shortcut: Option<&'static str>,
    pub kind: Kind,
    pub access: Access,
    /// One-line params summary for command_list.
    pub doc: &'static str,
    /// From the typed params struct (doc comments become descriptions).
    pub schema: fn() -> schemars::Schema,
    pub enabled: fn(&Session) -> Result<(), Disabled>,
    pub run: RunFn,
    pub checked: Option<fn(&Session) -> bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Edit,
    View,
    Query,
    Io,
    Settings,
    Device(DeviceClass),
    Request,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Stop,
    Connect,
    Light,
    Emit,
}

/// Who may run it; checked in dispatch before params are decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Any origin.
    Agent,
    /// Any origin; for Synthetic/Headless/Replay every path param resolves inside the granted roots.
    Roots,
    /// Needs a window: Human or Synthetic; Headless → Disabled "needs the OpenLaser window (bridge mode)".
    Ui,
    /// Origin::Human only; anything else → Refused(HumanOnly). No menu path, no shortcut (S18).
    Human,
    /// Any origin; never disabled, never refused, never journaled.
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Human,
    Synthetic,
    Headless,
    Replay,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Disabled {
    pub reason: String,
    pub needs_document: bool,
}

fn suggest(suggestions: &[String]) -> String {
    if suggestions.is_empty() { String::new() } else { format!("; did you mean {}?", suggestions.join(", ")) }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{id}`{}", suggest(.suggestions))]
    UnknownCommand { id: String, suggestions: Vec<String> },
    #[error("`{id}`: {pointer}: {message}")]
    BadParams { id: String, pointer: String, message: String },
    #[error("`{id}` is disabled: {reason}")]
    Disabled { id: String, reason: String, needs_document: bool },
    /// Policy and per-action safety.
    #[error("refused: {}", .0.message)]
    Refused(Refusal),
    #[error("job refused ({} refusals)", .0.refusals.len())]
    JobRefused(Box<JobRefused>),
    #[error("no document open")]
    NoDocument,
    #[error("no object {0:?}")]
    NoShape(ObjectId),
    #[error("this session has no device access")]
    NoDevice,
    #[error("needs bridge mode: turn on Settings › Automation › Allow agent connections in OpenLaser, then `openlaser-cli mcp --bridge auto`")]
    NeedsBridge,
    #[error("cancelled")]
    Cancelled,
    #[error("file: {0}")]
    Io(String),
    #[error("`{0}` failed with an internal error; the project is unchanged")]
    Internal(String),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

pub struct Registry {
    specs: Vec<CommandSpec>,
    by_id: HashMap<&'static str, usize>,
}

impl Registry {
    /// Exact, then snake_case alias (`flip_h` → `flipH`). WP0 stub: exact only.
    pub fn find(&self, id: &str) -> Option<&CommandSpec> {
        self.by_id.get(id).and_then(|&i| self.specs.get(i))
    }

    /// WP0 stub: none.
    pub fn suggestions(&self, id: &str) -> Vec<&'static str> {
        let _ = id;
        Vec::new()
    }

    pub fn specs(&self) -> &[CommandSpec] {
        &self.specs
    }
}

/// OnceLock; duplicate ids fail `registry_ids_unique`. WP0 stub: empty.
pub fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Registry { specs: Vec::new(), by_id: HashMap::new() })
}

/// `command!("arrange.flipH", "Flip Horizontal", menu: ["Arrange"], shortcut: Some("Cmd+Shift+H"),
///   kind: Kind::Edit, access: Access::Agent, enabled: needs_selection, params: IdsParams, run: arrange::flip_h)`
/// expands to a CommandSpec whose `run` decodes `IdsParams` strictly, calls the typed fn and serialises the result.
///
/// WP0 stub: any use is a compile error until WP6 implements it (ENG §2.2).
#[macro_export]
macro_rules! command {
    ($($tokens:tt)*) => {
        compile_error!("openlaser_engine::command! is a WP0 stub; WP6 implements it (ENG §2.2)")
    };
}

/// Normalises snake_case keys to camelCase (recursively), strips the universal keys `coalesce` and `dryRun`,
/// then decodes with deny_unknown_fields. Errors name the JSON pointer and the accepted keys.
pub fn decode<P: serde::de::DeserializeOwned>(id: &str, params: serde_json::Value) -> Result<P, EngineError> {
    let _ = (id, params);
    Err(EngineError::NotImplemented("openlaser_engine::decode"))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRow {
    pub id: String,
    pub label: String,
    pub menu: Vec<String>,
    pub shortcut: Option<String>,
    pub default_shortcut: Option<String>,
    pub kind: String,
    pub agent: AgentClass,
    pub params: String,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub needs_document: bool,
    pub schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentClass {
    Allowed,
    RootsOnly,
    Ui,
    HumanOnly,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListFilter {
    pub filter: Option<String>,
    pub enabled_only: bool,
    pub agent: Option<AgentClass>,
    pub schema: bool,
}
