//! T2 parity port of OM `gcode.compile_gcode`, including its silent speed capping (py-compat only).
//! The `Om*` types mirror the fields in tests/golden/t2_gcode/index.json. WP0 stub.

/// A machine row of `index.json` `machines`.
#[derive(Debug, Clone, PartialEq)]
pub struct OmMachine {
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

/// A layer of a scenario (OM `layer_settings.json` fields).
#[derive(Debug, Clone, PartialEq)]
pub struct OmLayer {
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

/// An object of a scenario.
#[derive(Debug, Clone, PartialEq)]
pub struct OmObject {
    pub layer_id: String,
    pub polylines: Vec<Vec<[f64; 2]>>,
}

/// OM's refusal (`index.json` case `error`: a Python exception type and message).
#[derive(Debug, Clone, PartialEq)]
pub struct OmRefusal {
    pub kind: String,
    pub message: String,
}

pub fn compile_gcode_om(machine: &OmMachine, layers: &[OmLayer], objects: &[OmObject], generated: &str) -> Result<String, OmRefusal> {
    let _ = (machine, layers, objects, generated);
    Err(OmRefusal { kind: "NotImplemented".to_owned(), message: "not implemented: openlaser_gcode::om::compile_gcode_om".to_owned() })
}
