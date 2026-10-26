//! `camera.*`: webcam enumeration, capture control and camera↔bed calibration. Enumeration/status/stop
//! are agent-safe; starting capture and changing calibration are human-only physical setup actions.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::always;
use crate::command;
use crate::command::{Access, CommandSpec, Disabled, EngineError, Kind};
use crate::params::{NoParams, finite};
use crate::session::Session;

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command!("camera.list", "Cameras", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: always, params: NoParams, run: list, doc: "{}"),
        command!("camera.status", "Camera Status", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: always, params: NoParams, run: status, doc: "{}"),
        command!("camera.start", "Start Camera", menu: [], shortcut: None, kind: Kind::View, access: Access::Human,
            enabled: always, params: StartParams, run: start, doc: "{index?} (view state; never touches laser state)"),
        command!("camera.stop", "Stop Camera", menu: [], shortcut: None, kind: Kind::View, access: Access::Agent,
            enabled: always, params: NoParams, run: stop, doc: "{} (idempotent)"),
        command!("camera.calibrate.addPair", "Add Calibration Point", menu: [], shortcut: None, kind: Kind::View, access: Access::Human,
            enabled: always, params: AddPairParams, run: add_pair, doc: "{camX, camY, bedX, bedY} (px ↔ mm)"),
        command!("camera.calibrate.removeLast", "Remove Last Calibration Point", menu: [], shortcut: None, kind: Kind::View, access: Access::Human,
            enabled: always, params: NoParams, run: remove_last, doc: "{}"),
        command!("camera.calibrate.fit", "Fit Camera Calibration", menu: [], shortcut: None, kind: Kind::View, access: Access::Human,
            enabled: always, params: NoParams, run: fit, doc: "{} (≥3 pairs; persists through the machine-local store)"),
        command!("camera.calibrate.clear", "Clear Camera Calibration", menu: [], shortcut: None, kind: Kind::View, access: Access::Human,
            enabled: always, params: NoParams, run: clear, doc: "{} (pairs and the fitted calibration)"),
        command!("camera.map.bedToPx", "Map Bed to Camera Pixels", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: needs_calibration, params: MapParams, run: bed_to_px, doc: "{x, y} mm → {camX, camY} px"),
        command!("camera.map.pxToBed", "Map Camera Pixels to Bed", menu: [], shortcut: None, kind: Kind::Query, access: Access::Agent,
            enabled: needs_calibration, params: MapParams, run: px_to_bed, doc: "{x, y} px → {bedX, bedY} mm"),
    ]
}

/// The `camera.map.*` commands are disabled until a calibration exists (persisted or fitted this session).
fn needs_calibration(s: &Session) -> Result<(), Disabled> {
    if s.camera_calibration().is_some() { Ok(()) } else { Err(Disabled::new("the camera is not calibrated")) }
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

/// `camera.calibrate.addPair`: `camX`/`camY` in image pixels, `bedX`/`bedY` in bed millimetres.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AddPairParams {
    /// The clicked image point (pixels).
    pub cam_x: f64,
    pub cam_y: f64,
    /// The same physical point measured on the bed (millimetres).
    pub bed_x: f64,
    pub bed_y: f64,
}

fn add_pair(s: &mut Session, p: AddPairParams) -> Result<Value, EngineError> {
    const ID: &str = "camera.calibrate.addPair";
    let pair = openlaser_camera::Pair {
        cam: [finite(ID, "/camX", p.cam_x)?, finite(ID, "/camY", p.cam_y)?],
        bed: [finite(ID, "/bedX", p.bed_x)?, finite(ID, "/bedY", p.bed_y)?],
    };
    s.camera_add_pair(pair)
}

fn remove_last(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    s.camera_remove_last_pair()
}

fn fit(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    s.camera_fit()
}

fn clear(s: &mut Session, _: NoParams) -> Result<Value, EngineError> {
    s.camera_clear_calibration()
}

/// `camera.map.*`: `{x, y}` in the input's own units (mm for `bedToPx`, px for `pxToBed`).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MapParams {
    pub x: f64,
    pub y: f64,
}

fn bed_to_px(s: &mut Session, p: MapParams) -> Result<Value, EngineError> {
    const ID: &str = "camera.map.bedToPx";
    s.camera_bed_to_px(finite(ID, "/x", p.x)?, finite(ID, "/y", p.y)?)
}

fn px_to_bed(s: &mut Session, p: MapParams) -> Result<Value, EngineError> {
    const ID: &str = "camera.map.pxToBed";
    s.camera_px_to_bed(finite(ID, "/x", p.x)?, finite(ID, "/y", p.y)?)
}
