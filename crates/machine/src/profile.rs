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
    #[serde(default)]
    pub rotary: Option<RotaryCfg>,
}

/// Extended-axis settings. Protocol units remain unverified until the rotary bench session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RotaryCfg {
    pub enabled: bool,
    pub kind: RotaryKind,
    pub pulses_per_rev: u32,
    pub drive_diameter_mm: f64,
    pub object_diameter_mm: f64,
    pub axis: RotaryAxis,
    pub split_mm: f64,
    pub overlap_mm: f64,
    pub reverse: bool,
    pub min_speed_pps: u16,
    pub max_speed_pps: u16,
    pub accel_ms: u16,
    pub return_to_start: bool,
    pub max_turns_per_job: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RotaryKind {
    #[default]
    Chuck,
    Roller,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RotaryAxis {
    X,
    #[default]
    Y,
}

impl RotaryAxis {
    pub fn coordinate(self, point: [f64; 2]) -> f64 {
        match self {
            Self::X => point[0],
            Self::Y => point[1],
        }
    }
    pub fn translate(self, point: &mut [f64; 2], delta: f64) {
        match self {
            Self::X => point[0] += delta,
            Self::Y => point[1] += delta,
        }
    }
    pub fn index(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
        }
    }
}

impl Default for RotaryCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: RotaryKind::Chuck,
            pulses_per_rev: 12800,
            drive_diameter_mm: 40.0,
            object_diameter_mm: 40.0,
            axis: RotaryAxis::Y,
            split_mm: 10.0,
            overlap_mm: 0.0,
            reverse: false,
            min_speed_pps: 100,
            max_speed_pps: 1000,
            accel_ms: 100,
            return_to_start: true,
            max_turns_per_job: 2.0,
        }
    }
}

impl RotaryCfg {
    pub fn circumference(&self) -> f64 {
        std::f64::consts::PI * self.drive_diameter_mm
    }

    pub fn validate(&self, work: FieldRect) -> Result<(), String> {
        let diameter = |d: f64| d.is_finite() && d > 0.0 && d <= 500.0;
        if !(1..=1_000_000).contains(&self.pulses_per_rev)
            || !diameter(self.drive_diameter_mm)
            || !diameter(self.object_diameter_mm)
            || !self.split_mm.is_finite()
            || self.split_mm < self.circumference() / f64::from(self.pulses_per_rev)
            || self.split_mm > self.axis.coordinate(work.max) - self.axis.coordinate(work.min)
            || !self.overlap_mm.is_finite()
            || self.overlap_mm < 0.0
            || self.overlap_mm >= self.split_mm / 2.0
            || self.min_speed_pps == 0
            || self.max_speed_pps < self.min_speed_pps
            || !(1..=10000).contains(&self.accel_ms)
            || !self.max_turns_per_job.is_finite()
            || self.max_turns_per_job <= 0.0
            || self.max_turns_per_job > 20.0
        {
            return Err("invalid rotary configuration: check pulses/rev (1..=1000000), diameters (0..=500 mm), split (one pulse..field extent), overlap (0..split/2), ordered positive speeds, acceleration (1..=10000 ms), and max turns (0..=20)".into());
        }
        Ok(())
    }
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
    pub fn effective_work_area(&self) -> FieldRect {
        let f = self.field_f();
        let enc_min = [(-f[0] / 2.0).next_up(), (-f[1] / 2.0).next_up()];
        let enc_max = [(f[0] / 2.0).next_down(), (f[1] / 2.0).next_down()];
        FieldRect {
            min: [self.work_area.min[0].max(enc_min[0]), self.work_area.min[1].max(enc_min[1])],
            max: [self.work_area.max[0].min(enc_max[0]), self.work_area.max[1].min(enc_max[1])],
        }
    }
}

/// Collects every problem. Galvo: scale in 0.5..=2.0; work_area ⊆ h27_cleared, or ⊆ ±encoding_field/2 when
/// h27_cleared is None (the rated field); effective_work_area non-empty; freq range within 1..=4000 kHz and lo < hi;
/// pulse table non-empty and ascending; max_power_pct in (0, 100]; run_ahead.max_packets ≥ 1, max_emission_s > 0,
/// watchdog_factor ≥ 1; transfer_timeout_ms in 20..=2000; checklist non-empty (mint needs ≥ 1 tick, §3.9).
/// Gantry: bed and speeds positive, s_max ≥ 1. Every save and every load runs it; a failing profile is not used.
///
pub fn validate_profile(p: &MachineProfile) -> Result<(), Vec<String>> {
    let mut problems = Vec::new();
    if p.id.0.is_empty() {
        problems.push("machine id must not be empty".to_owned());
    }
    if p.name.trim().is_empty() {
        problems.push("machine name must not be empty".to_owned());
    }
    if p.checklist.is_empty() || p.checklist.iter().any(|item| item.trim().is_empty()) {
        problems.push("job checklist must contain at least one non-empty item".to_owned());
    }
    if p.limits.max_paths == 0 || p.limits.max_points_per_path == 0 || p.limits.max_points_per_pass == 0 {
        problems.push("job limits must all be at least 1".to_owned());
    }
    match &p.kind {
        Kinematics::Galvo(g) => validate_galvo(g, &mut problems),
        Kinematics::Gantry(g) => validate_gantry(g, &mut problems),
    }
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

fn finite(values: impl IntoIterator<Item = f64>) -> bool {
    values.into_iter().all(f64::is_finite)
}

fn rect_valid(rect: FieldRect) -> bool {
    finite(rect.min.into_iter().chain(rect.max)) && rect.min[0] < rect.max[0] && rect.min[1] < rect.max[1]
}

fn rect_inside(inner: FieldRect, outer: FieldRect) -> bool {
    inner.min[0] >= outer.min[0] && inner.min[1] >= outer.min[1] && inner.max[0] <= outer.max[0] && inner.max[1] <= outer.max[1]
}

fn validate_galvo(g: &GalvoMachine, problems: &mut Vec<String>) {
    if let Some(r) = &g.rotary
        && let Err(e) = r.validate(g.work_area)
    {
        problems.push(e);
    }
    if !g.cal.scale.iter().all(|v| v.is_finite() && (0.5..=2.0).contains(v)) {
        problems.push("galvo calibration scale must be finite and within 0.5..=2.0 on both axes".to_owned());
    }
    if !finite(g.cal.offset_mm.into_iter().chain([g.cal.angle_deg])) {
        problems.push("galvo calibration offsets and angle must be finite".to_owned());
    }
    if !finite(g.red_dot.offset_mm.into_iter().chain(g.red_dot.scale)) || g.red_dot.scale.iter().any(|v| *v <= 0.0) {
        problems.push("red-dot offsets must be finite and scales must be positive".to_owned());
    }
    if !rect_valid(g.work_area) {
        problems.push("galvo work area must be a finite non-empty rectangle".to_owned());
    }
    let nominal_half = g.encoding_field.get() / 2.0;
    let nominal = FieldRect { min: [-nominal_half, -nominal_half], max: [nominal_half, nominal_half] };
    let allowed = g.h27_cleared.unwrap_or(nominal);
    if !rect_valid(allowed) {
        problems.push("H27-cleared field must be a finite non-empty rectangle".to_owned());
    } else if !rect_inside(g.work_area, allowed) {
        problems.push("work area exceeds the rated field or H27-cleared field".to_owned());
    }
    if !rect_valid(g.effective_work_area()) {
        problems.push("effective work area is empty after intersecting the encodable field".to_owned());
    }
    let [lo, hi] = g.source.freq_khz;
    if !finite([lo, hi]) || !(1.0..=4000.0).contains(&lo) || !(1.0..=4000.0).contains(&hi) || lo >= hi {
        problems.push("laser frequency range must be ordered within 1..=4000 kHz".to_owned());
    }
    if g.source.pulse_table_ns.is_empty() || g.source.pulse_table_ns.contains(&0) || g.source.pulse_table_ns.windows(2).any(|w| w.first() >= w.get(1)) {
        problems.push("pulse-width table must be non-empty, positive and strictly ascending".to_owned());
    }
    if !g.source.max_power_pct.is_finite() || !(0.0 < g.source.max_power_pct && g.source.max_power_pct <= 100.0) {
        problems.push("maximum source power must be within (0, 100] percent".to_owned());
    }
    if !finite([
        g.timings.laser_on_us,
        g.timings.laser_off_us,
        g.timings.polygon_us,
        g.timings.jump_delay_short_us,
        g.timings.jump_delay_long_us,
        g.first_jump.delay_us,
        g.run_ahead.max_emission_s,
        g.run_ahead.watchdog_factor,
        g.run_ahead.watchdog_slack_s,
    ]) {
        problems.push("galvo timings and run-ahead values must be finite".to_owned());
    }
    if g.run_ahead.max_packets < 1 || g.run_ahead.max_emission_s <= 0.0 || g.run_ahead.watchdog_factor < 1.0 || g.run_ahead.watchdog_slack_s < 0.0 {
        problems.push("run-ahead requires at least one packet, positive emission time, watchdog factor >= 1 and non-negative slack".to_owned());
    }
    if !(20..=2000).contains(&g.transfer_timeout_ms) {
        problems.push("USB transfer timeout must be within 20..=2000 ms".to_owned());
    }
    if g.ports.laser_bit >= 16 || g.ports.red_bit >= 16 || g.ports.laser_bit == g.ports.red_bit {
        problems.push("laser and red-light ports must be distinct bits within 0..=15".to_owned());
    }
}

fn validate_gantry(g: &GantryMachine, problems: &mut Vec<String>) {
    if !finite(g.bed_mm) || g.bed_mm.iter().any(|v| *v <= 0.0) {
        problems.push("gantry bed dimensions must be finite and positive".to_owned());
    }
    if g.s_max < 1 {
        problems.push("gantry S maximum must be at least 1".to_owned());
    }
}

/// S24. Writes are human-only by construction: only the GUI and openlaser-hw construct an implementation.
pub trait ProfileStore: Send + Sync {
    /// validate_profile first; atomic.
    fn save(&self, p: &MachineProfile) -> Result<(), String>;
    fn delete(&self, id: &MachineId) -> Result<(), String>;
}
