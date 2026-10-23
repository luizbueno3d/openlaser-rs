//! T2 G-code fixture loader.

use std::collections::BTreeMap;
use std::fs;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Machine {
    pub machine_id: String,
    pub name: String,
    pub kind: String,
    pub transport: String,
    pub width_mm: f64,
    pub height_mm: f64,
    pub origin: String,
    pub tool: String,
    pub output_format: String,
    pub max_speed_mm_s: f64,
    pub travel_speed_mm_s: f64,
    pub power_scale: u32,
    pub laser_command: String,
    pub tool_on_command: String,
    pub tool_off_command: String,
    pub supports_air_assist: bool,
    pub uses_frequency: bool,
    pub uses_pulse_width: bool,
    pub direct_control: bool,
    pub notes: String,
    pub is_laser: bool,
    pub is_plotter: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Layer {
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Object {
    pub layer_id: String,
    pub polylines: Vec<Vec<[f64; 2]>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Scenario {
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub objects: Vec<Object>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Refusal {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Case {
    pub machine_id: String,
    pub scenario: String,
    pub ok: bool,
    pub file: Option<String>,
    pub sha256: Option<String>,
    pub path_count: Option<u32>,
    pub point_count: Option<u32>,
    pub estimated_distance_mm: Option<f64>,
    pub error: Option<Refusal>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Index {
    pub machines: Vec<Machine>,
    pub scenarios: BTreeMap<String, Scenario>,
    pub cases: Vec<Case>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NumberTable {
    pub inputs_f64_sha256: String,
    pub rows: Vec<(f64, String)>,
}

pub fn load() -> Result<Index, String> {
    let path = crate::golden_dir().join("t2_gcode").join("index.json");
    let bytes = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))
}

pub fn load_numbers() -> Result<NumberTable, String> {
    let path = crate::golden_dir().join("t2_gcode").join("number_format.json");
    let bytes = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))
}

pub fn program(file: &str) -> Result<Vec<u8>, String> {
    let path = crate::golden_dir().join("t2_gcode").join(file);
    fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))
}
