//! `camera.*`: webcam enumeration and capture control (phase 6a). All four commands are `Access::Agent`
//! view state — they can never touch laser state, so agents may drive them (AGENTS.md §1.2). Calibration
//! and the workspace overlay land in a later phase.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::always;
use crate::command;
use crate::command::{Access, CommandSpec, EngineError, Kind};
use crate::params::NoParams;
use crate::session::Session;

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command!("camera.list", "Cameras", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: always, params: NoParams, run: list, doc: "{}"),
        command!("camera.status", "Camera Status", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: always, params: NoParams, run: status, doc: "{}"),
        command!("camera.start", "Start Camera", menu: [], shortcut: None, kind: Kind::View, access: Access::Agent,
            enabled: always, params: StartParams, run: start, doc: "{index?} (view state; never touches laser state)"),
        command!("camera.stop", "Stop Camera", menu: [], shortcut: None, kind: Kind::View, access: Access::Agent,
            enabled: always, params: NoParams, run: stop, doc: "{} (idempotent)"),
    ]
}

fn list(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    let devices: Vec<Value> = s.camera_devices()?.iter().map(|d| json!({ "index": d.index, "name": d.name })).collect();
    Ok(json!({ "devices": devices }))
}

fn status(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    Ok(s.camera_status_json())
}

/// `camera.start`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct StartParams {
    /// Device index (`camera.list`). Default: the open camera, else the first enumerated one.
    #[serde(default)]
    pub index: Option<u32>,
}

fn start(s: &mut Session, p: StartParams) -> Result<Value, EngineError> {
    s.camera_start(p.index)
}

fn stop(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    s.camera_stop()
}
