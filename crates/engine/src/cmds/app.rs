//! Session, document, discovery, preferences, help, and the shell-only view commands (Appendix A). The view and
//! window commands are `Ui`: the engine authorises them and the GUI shell acts on the `{"ui": id}` it gets back.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{always, needs_doc};
use crate::command::{AgentClass, CommandSpec, EngineError, Kind, ListFilter, registry};
use crate::params::{AgentClassP, NoParams};
use crate::prefs::PrefEffect;
use crate::session::Session;
use crate::{Access, command};

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command!("document.inspect", "Inspect Document", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: needs_doc, params: InspectParams, run: inspect, doc: "{index?, objects?:\"summary\"|\"all\"}"),
        command!("document.select", "Select Document", menu: [], shortcut: None, kind: Kind::View, access: Access::Agent,
            enabled: always, params: IndexParams, run: select_doc, doc: "{index}"),
        command!("document.close", "Close Document", menu: [], shortcut: None, kind: Kind::View, access: Access::Agent,
            enabled: needs_doc, params: CloseParams, run: close_doc, doc: "{index?}"),
        command!("session.list", "Session", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: always, params: NoParams, run: |s: &mut Session, _p: NoParams| Ok::<_, EngineError>(s.session_list_json()), doc: "{}"),
        command!("command.list", "List Commands", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: always, params: ListParams, run: list, doc: "{filter?, enabledOnly?, agent?, schema?}"),
        command!("command.schema", "Command Schema", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: always, params: SchemaParams, run: |s: &mut Session, p: SchemaParams| s.command_schema(&p.id), doc: "{id}"),
        command!("help.about", "About OpenLaser", menu: ["OpenLaser"], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: always, params: NoParams, run: about, doc: "{}"),
        command!("prefs.get", "Get Preference", menu: [], shortcut: None, kind: Kind::Settings, access: Access::Agent,
            enabled: always, params: PrefGet, run: prefs_get, doc: "{key?}"),
        command!("prefs.set", "Set Preference", menu: [], shortcut: None, kind: Kind::Settings, access: Access::Agent,
            enabled: always, params: PrefSet, run: prefs_set, doc: "{key, value} (refuses machine.*, safety.*, automation.*, humanOnly.*)"),
        command!("view.zoomIn", "Zoom In", menu: ["Window"], shortcut: Some("Cmd+="), kind: Kind::View, access: Access::Ui,
            enabled: always, params: NoParams, run: |_s: &mut Session, _p: NoParams| Ok::<_, EngineError>(json!({ "ui": "view.zoomIn" })), doc: "{}"),
        command!("view.zoomOut", "Zoom Out", menu: ["Window"], shortcut: Some("Cmd+-"), kind: Kind::View, access: Access::Ui,
            enabled: always, params: NoParams, run: |_s: &mut Session, _p: NoParams| Ok::<_, EngineError>(json!({ "ui": "view.zoomOut" })), doc: "{}"),
        command!("view.zoomPage", "Zoom to Page", menu: ["Window"], shortcut: Some("Cmd+0"), kind: Kind::View, access: Access::Ui,
            enabled: always, params: NoParams, run: |_s: &mut Session, _p: NoParams| Ok::<_, EngineError>(json!({ "ui": "view.zoomPage" })), doc: "{}"),
        command!("view.zoomSelection", "Zoom to Selection", menu: ["Window"], shortcut: Some("Cmd+Shift+A"), kind: Kind::View, access: Access::Ui,
            enabled: always, params: NoParams, run: |_s: &mut Session, _p: NoParams| Ok::<_, EngineError>(json!({ "ui": "view.zoomSelection" })), doc: "{}"),
        command!("view.pan", "Pan", menu: [], shortcut: Some("Space"), kind: Kind::View, access: Access::Ui,
            enabled: always, params: NoParams, run: |_s: &mut Session, _p: NoParams| Ok::<_, EngineError>(json!({ "ui": "view.pan" })), doc: "{}"),
        command!("view.cameraBackground", "Camera Background", menu: ["Window"], shortcut: None, kind: Kind::View, access: Access::Ui,
            enabled: always, params: NoParams, run: |_s: &mut Session, _p: NoParams| Ok::<_, EngineError>(json!({ "ui": "view.cameraBackground" })), doc: "{} (toggle; the shell draws the calibrated feed under the design)"),
        command!("window.show", "Show Window", menu: [], shortcut: None, kind: Kind::View, access: Access::Ui,
            enabled: always, params: WindowParams, run: window_show, doc: "{panel, visible}"),
        command!("preview.open", "Preview", menu: ["Window"], shortcut: Some("Alt+P"), kind: Kind::View, access: Access::Ui,
            enabled: always, params: NoParams, run: |_s: &mut Session, _p: NoParams| Ok::<_, EngineError>(json!({ "ui": "preview.open" })), doc: "{}"),
    ]
}

/// `document.inspect`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct InspectParams {
    /// Document index (`session.list`). Default: the active document.
    #[serde(default)]
    pub index: Option<usize>,
    /// "summary" (default: at most 200 objects) or "all".
    #[serde(default)]
    pub objects: Option<ObjectsP>,
}

/// How many objects `document.inspect` lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ObjectsP {
    Summary,
    All,
}

fn inspect(s: &mut Session, p: InspectParams) -> Result<Value, EngineError> {
    s.inspect(p.index, p.objects == Some(ObjectsP::All))
}

/// `document.select`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct IndexParams {
    /// Document index (`session.list`).
    pub index: usize,
}

fn select_doc(s: &mut Session, p: IndexParams) -> Result<Value, EngineError> {
    s.select_doc(p.index)?;
    Ok(s.session_list_json())
}

/// `document.close`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CloseParams {
    /// Document index. Default: the active document.
    #[serde(default)]
    pub index: Option<usize>,
}

fn close_doc(s: &mut Session, p: CloseParams) -> Result<Value, EngineError> {
    s.close_doc(p.index)?;
    Ok(s.session_list_json())
}

/// `command.list`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ListParams {
    /// Substring of the id or label (case-insensitive).
    #[serde(default)]
    pub filter: Option<String>,
    /// Only commands enabled now for this caller.
    #[serde(default)]
    pub enabled_only: Option<bool>,
    /// Only this agent class.
    #[serde(default)]
    pub agent: Option<AgentClassP>,
    /// Include each command's JSON Schema.
    #[serde(default)]
    pub schema: Option<bool>,
}

fn list(s: &mut Session, p: ListParams) -> Result<Value, EngineError> {
    let agent = p.agent.map(|a| match a {
        AgentClassP::Allowed => AgentClass::Allowed,
        AgentClassP::RootsOnly => AgentClass::RootsOnly,
        AgentClassP::Ui => AgentClass::Ui,
        AgentClassP::HumanOnly => AgentClass::HumanOnly,
        AgentClassP::Stop => AgentClass::Stop,
    });
    let f = ListFilter { filter: p.filter, enabled_only: p.enabled_only.unwrap_or(false), agent, schema: p.schema.unwrap_or(false) };
    let rows = s.command_list(s.effective_origin(), &f);
    serde_json::to_value(rows).map_err(|_| EngineError::Internal("command.list".to_owned()))
}

/// `command.schema`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SchemaParams {
    /// Command id (camelCase, or its snake_case alias).
    pub id: String,
}

fn about(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    Ok(json!({
        "name": "OpenLaser",
        "version": env!("CARGO_PKG_VERSION"),
        "session": s.id().0,
        "commands": registry().specs().len(),
        "licence": "MIT OR Apache-2.0",
    }))
}

/// `prefs.get`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PrefGet {
    /// Preference key. Default: every preference.
    #[serde(default)]
    pub key: Option<String>,
}

fn prefs_get(s: &mut Session, p: PrefGet) -> Result<Value, EngineError> {
    Ok(match p.key {
        Some(k) => json!({ "key": k, "value": s.prefs().get(&k) }),
        None => json!(s.prefs().all()),
    })
}

/// `prefs.set`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PrefSet {
    /// Preference key, e.g. "output.curveToleranceMm" or "shortcuts.edit.undo".
    pub key: String,
    /// The new value (JSON).
    pub value: Value,
}

fn prefs_set(s: &mut Session, p: PrefSet) -> Result<Value, EngineError> {
    match s.prefs.set(&p.key, p.value.clone())? {
        PrefEffect::Planner => s.bump_docs(None, "a planner preference changed"),
        PrefEffect::History => {
            let max = s.prefs.max_states();
            for d in &mut s.docs {
                d.history.set_max(max);
            }
        }
        PrefEffect::None => {}
    }
    Ok(json!({ "key": p.key, "value": p.value }))
}

/// `window.show`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WindowParams {
    /// Panel id: "cuts", "laser", "move", "console", "library", "camera", "preview", "shapeProperties", "undoHistory".
    pub panel: String,
    /// Show or hide it.
    pub visible: bool,
}

fn window_show(_s: &mut Session, p: WindowParams) -> Result<Value, EngineError> {
    if p.panel.is_empty() || p.panel.len() > 64 || !p.panel.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(EngineError::bad("window.show", "/panel", "panel ids are short alphanumeric names"));
    }
    Ok(json!({ "ui": "window.show", "panel": p.panel, "visible": p.visible }))
}
