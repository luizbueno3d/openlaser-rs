//! Profile data types.

use openlaser_doc::{LayerDefaults, WorkspaceOrigin};
use openlaser_types::{MachineId, Mm, MmPerS, Us};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineProfile {
    pub id: MachineId,
    pub name: String,
    pub kind: Kinematics,
    pub limits: JobLimits,
    pub layer_defaults: LayerDefaults,
    pub checklist: Vec<String>,
}

// A handful of profiles live per session; boxing the galvo variant would only complicate the contract.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Kinematics {
    Galvo(GalvoMachine),
    Gantry(GantryMachine),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GalvoMachine {
    /// vid 0x9588, pid 0x9899, optional serial; pid 0x9980 is refused (clone board).
    pub board: LmcBoardFilter,
    /// Usb | Sim.
    pub link: GalvoLink,
    /// Nominal FIELDSIZE (200). F = encoding_field × cal.scale, ONLY for unit conversion.
    pub encoding_field: Mm,
    /// The lens's rated field: ±100 mm. Shrinks freely; grows only within h27_cleared.
    pub work_area: FieldRect,
    pub h27_cleared: Option<FieldRect>,
    pub cal: FieldCalibration,
    /// Applied to lighting lists only; identity until H26.
    pub red_dot: RedDotCal,
    pub source: LaserSource,
    pub timings: GalvoTimingDefaults,
    /// Long jump delay 200 µs + settle 1 ms until H3/H19.
    pub first_jump: FirstJump,
    /// 3000.
    pub jump_speed: MmPerS,
    /// 3000 (red-light frame speed).
    pub light_speed: MmPerS,
    /// 6000; above it is refused.
    pub max_speed: MmPerS,
    /// 8000 µs → DelayTime(800).
    pub mo_settle: Us,
    /// Laser bit 0 (0x0001), red light bit 8 (0x0100).
    pub ports: PortMap,
    /// ≤ 3 packets and ≤ 2 s estimated emission, never < 1 packet; watchdog factors.
    pub run_ahead: RunAhead,
    /// 250: total budget of one USB transfer (sliced ≤ 20 ms); then Fault + stop (H13).
    pub transfer_timeout_ms: u32,
    /// false until H12: if the device is unconfigured, refuse (UsbNotConfigured) instead of calling
    /// set_configuration(1); Luiz flips it at a TTY if H12 says so.
    pub usb_set_configuration: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LmcBoardFilter {
    pub vid: u16,
    pub pid: u16,
    pub serial: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GalvoLink {
    Usb,
    Sim,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldRect {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldCalibration {
    /// Per galvo axis; OM's uniform 1.1834 imports as [1.1834, 1.1834]; 0.5..=2.0.
    pub scale: [f64; 2],
    /// Field Offset X/Y.
    pub offset_mm: [f64; 2],
    /// Field Angle.
    pub angle_deg: f64,
    /// "Galvo 1 is X axis" off.
    pub swap_xy: bool,
    /// "Reverse direction" per axis (the prototype's mirror flags live here now).
    pub negate: [bool; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedDotCal {
    pub offset_mm: [f64; 2],
    pub scale: [f64; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaserSource {
    pub kind: SourceKind,
    pub pulse_table_ns: Vec<u16>,
    pub freq_khz: [f64; 2],
    pub max_power_pct: f64,
    pub q_pulse_enabled: bool,
    pub mo_only_when_running: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    MopaFiberJpt,
}

/// GP defaults: 100, 100, 100, 8, 200.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GalvoTimingDefaults {
    pub laser_on_us: f64,
    pub laser_off_us: f64,
    pub polygon_us: f64,
    pub jump_delay_short_us: f64,
    pub jump_delay_long_us: f64,
}

/// 200 µs, 100 (= 1 ms).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirstJump {
    pub delay_us: f64,
    pub settle_10us: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortMap {
    pub laser_bit: u8,
    pub red_bit: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunAhead {
    /// 3.
    pub max_packets: u8,
    /// 2.0.
    pub max_emission_s: f64,
    /// 1.5 (OM v1 5f7aa63); refined after H1/H9.
    pub watchdog_factor: f64,
    /// 2.0.
    pub watchdog_slack_s: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GantryMachine {
    pub bed_mm: [f64; 2],
    pub origin: WorkspaceOrigin,
    pub max_speed: MmPerS,
    pub travel_speed: MmPerS,
    pub s_max: u32,
    /// Laser { dynamic: bool /* M4 */ } | Pen.
    pub tool: GantryTool,
    pub air_assist: bool,
    pub frame_before_mark: bool,
    /// Default false.
    pub home_on_connect: bool,
    pub link: GantryLink,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum GantryTool {
    Laser { dynamic: bool },
    Pen,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum GantryLink {
    ExportOnly,
    Serial { port: String, baud: u32 },
    Tcp { host: String, port: u16 },
    Sim,
}

/// 3000 / 2000 / 20000.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobLimits {
    pub max_paths: u32,
    pub max_points_per_path: u32,
    pub max_points_per_pass: u32,
}

impl GalvoMachine {
    /// encoding_field × scale, per axis (236.68 for Luiz).
    pub fn field_f(&self) -> [f64; 2] {
        let f = self.encoding_field.get();
        [f * self.cal.scale[0], f * self.cal.scale[1]]
    }

    /// work_area ∩ the strictly encodable square (|v| < F/2 on each axis).
    /// WP0 stub: an empty rectangle at the origin (accepts nothing) until WP1 implements it.
    pub fn effective_work_area(&self) -> FieldRect {
        FieldRect { min: [0.0, 0.0], max: [0.0, 0.0] }
    }
}

/// Collects every problem. Galvo: scale in 0.5..=2.0; work_area ⊆ h27_cleared, or ⊆ ±encoding_field/2 when
/// h27_cleared is None (the rated field); effective_work_area non-empty; freq range within 1..=4000 kHz and lo < hi;
/// pulse table non-empty and ascending; max_power_pct in (0, 100]; run_ahead.max_packets ≥ 1, max_emission_s > 0,
/// watchdog_factor ≥ 1; transfer_timeout_ms in 20..=2000; checklist non-empty (mint needs ≥ 1 tick, §3.9).
/// Gantry: bed and speeds positive, s_max ≥ 1. Every save and every load runs it; a failing profile is not used.
///
/// WP0 stub: refuses every profile (fail closed) until WP1 implements the checks.
pub fn validate_profile(p: &MachineProfile) -> Result<(), Vec<String>> {
    Err(vec![format!("not implemented: openlaser_machine::validate_profile({})", p.id.0)])
}

/// S24. Writes are human-only by construction: only the GUI and openlaser-hw construct an implementation.
pub trait ProfileStore: Send + Sync {
    /// validate_profile first; atomic.
    fn save(&self, p: &MachineProfile) -> Result<(), String>;
    fn delete(&self, id: &MachineId) -> Result<(), String>;
}
