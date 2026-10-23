//! The OM prototype's `workspace.json` and `layer_settings.json` (Appendix B). WP0 stubs.

use serde::Deserialize;

use crate::{Error, Imported};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OmWorkspace {
    pub version: u32,
    pub objects: Vec<OmObject>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OmObject {
    pub name: String,
    pub layer_id: String,
    pub polylines: Vec<Vec<Vec<f64>>>,
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    pub rotation: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OmLayerSpec {
    pub layer_id: String,
    pub name: String,
    pub operation: String,
    pub power_percent: f64,
    pub frequency_khz: f64,
    pub pulse_width_ns: f64,
    pub speed_mm_s: f64,
    pub passes: u32,
    pub visible: bool,
    pub output: bool,
    pub color: String,
    pub air_assist: bool,
    pub raster_pitch_mm: f64,
    pub raster_angle_deg: f64,
    pub raster_angle_step_deg: f64,
}

/// Points with != 2 coords → MalformedPoint (B9).
pub fn parse_workspace(bytes: &[u8]) -> Result<OmWorkspace, Error> {
    let _ = bytes;
    Err(Error::NotImplemented("openlaser_import::om::parse_workspace"))
}

pub fn parse_layer_settings(bytes: &[u8]) -> Result<Vec<OmLayerSpec>, Error> {
    let _ = bytes;
    Err(Error::NotImplemented("openlaser_import::om::parse_layer_settings"))
}

/// OM's four built-in layers (R05 §4.1), used when no `layer_settings.json` sits next to the workspace
/// (a notice says so). WP0 stub: none.
pub fn builtin_layers() -> Vec<OmLayerSpec> {
    Vec::new()
}

/// Mapping in Appendix B. An object whose `layer_id` is not in the layer settings refuses the whole import,
/// naming the objects (B8: never dropped, never guessed).
pub fn to_imported(ws: &OmWorkspace, layers: &[OmLayerSpec]) -> Result<Imported, Error> {
    let _ = (ws, layers);
    Err(Error::NotImplemented("openlaser_import::om::to_imported"))
}
