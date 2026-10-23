//! The planner (SPEC §3.7; owned by WP3, red zone): document + machine → [`JobPlan`].
//!
//! - **Sections** in burn order: Cuts / Layers order, then sub-layer, then pass (global passes repeat the whole
//!   sub-layer stack); draw order inside. Line sections carry the flattened outlines; Fill sections carry 2-point
//!   hatch segments from `geom::hatch`, one section per hatch pass. Coordinates are machine mm
//!   ([`machine_transform`] for a galvo, [`gantry_transform`] for a gantry).
//! - **Quantised words** ([`GalvoCutQ`], [`GantryCutQ`]) are computed once, here ([`quantise`]); encoders read only
//!   the words.
//! - **Frame** (Bounds or Contour) from every outline on an emitting layer, hidden or not (frame ⊇ burn); Fill
//!   layers frame their outlines, never the hatch. Tool layers T1/T2 go to `tool_frame` only.
//! - **Facts** for everything `safety` must judge; nothing is dropped silently.
//! - **Fingerprint** per S21 ([`fingerprint_of`]), frame style and source excluded.
#![forbid(unsafe_code)]

use std::time::Duration;

mod estimate;
mod fingerprint;
mod frame;
mod planner;
pub mod quantise;
mod transform;

pub use openlaser_doc::{DeviceKind, Project, WorkspaceOrigin};
pub use openlaser_geom::{Affine, Bounds, Polyline};
pub use openlaser_machine::{GalvoMachine, GantryMachine, MachineProfile};
pub use openlaser_types::galvo::PulseDecision;
pub use openlaser_types::{Fingerprint, FrameSource, FrameStyle, Hz, LayerKey, MachineId, MmPerS, ObjectId, Percent, SectionMode};

pub use fingerprint::fingerprint_of;
pub use frame::{HULL_FRAME_UNSUPPORTED, bounds_rect, frame_style_of};
pub use planner::{
    MAX_COORD_MM, MAX_DESIGN_POINTS, MAX_HATCH_LINES, MAX_PASSES_PER_LAYER, MAX_SECTION_POINTS, MAX_SECTIONS, MAX_SUB_LAYERS, MIN_FLATTEN_TOL_MM,
};
pub use transform::{gantry_bed, gantry_transform};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq)]
pub struct PlanOptions {
    /// Cut Selected Graphics.
    pub selected: Option<Vec<ObjectId>>,
    pub repeats: u16,
    pub frame_style: FrameStyle,
    pub frame_source: FrameSource,
    pub flatten_tol_mm: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JobPlan {
    pub machine: MachineId,
    pub device: DeviceKind,
    /// Burn order: Cuts/Layers order, then sub-layer, then pass; draw order inside.
    pub sections: Vec<Section>,
    /// Machine mm; what a Job frame traces (style applied); red-dot NOT applied.
    pub frame: Vec<Polyline>,
    /// T1/T2 shapes, for ToolLayersOnly / JobAndToolLayers.
    pub tool_frame: Vec<Polyline>,
    /// What safety must judge; the planner never drops anything silently.
    pub facts: PlanFacts,
    pub repeats: u16,
    /// All repeats.
    pub estimate: Duration,
    /// S21: blake3 over the canonical encoding of machine, device, sections (layer, pass, quantised words, path f64
    /// bit patterns), repeats, the machine's calibration, transform, red-dot calibration and first-jump/timing
    /// settings. NOT frame, tool_frame, frame style or source: those are framing choices, and the receipt records
    /// them (`covers_job` decides). See [`fingerprint_of`].
    pub fingerprint: Fingerprint,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub layer: LayerKey,
    pub layer_name: String,
    pub sub: u8,
    pub pass: u16,
    pub of: u16,
    pub mode: SectionMode,
    pub cut: CutQ,
    /// Machine mm; Fill/Image: 2-point segments in burn order.
    pub paths: Vec<Polyline>,
    pub objects: Vec<ObjectId>,
    pub estimate: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CutQ {
    Galvo(GalvoCutQ),
    Gantry(GantryCutQ),
}

/// Quantised once, here (PLAN D9). The encoder reads only the words.
#[derive(Debug, Clone, PartialEq)]
pub struct GalvoCutQ {
    pub requested_speed: MmPerS,
    /// May be 0: safety refuses (B7).
    pub speed_units: u16,
    /// Display only.
    pub actual_speed_mm_s: f64,
    pub power: Percent,
    pub mark_current: u16,
    pub frequency: Hz,
    pub q_period: u16,
    /// Exact | Snapped | Refused (safety refuses).
    pub pulse: PulseDecision,
    pub timings: ResolvedTimings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTimings {
    pub laser_on: (u16, u16),
    pub laser_off: (u16, u16),
    pub polygon: (u16, u16),
    pub jump_delay_short: (u16, u16),
    pub jump_delay_long: (u16, u16),
    pub jump_speed_units: u16,
    pub mo_settle_10us: u16,
    pub first_jump_settle_10us: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GantryCutQ {
    pub speed: MmPerS,
    pub power: Percent,
    pub s_value: u32,
    pub air_assist: bool,
    pub dynamic: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlanFacts {
    /// Layer not in the table (import of a forged/foreign file).
    pub orphan_objects: Vec<ObjectId>,
    /// show == false && output == true (B6).
    pub hidden_output_layers: Vec<LayerKey>,
    /// Fill/Image layer with shapes but no segments.
    pub empty_fill_layers: Vec<LayerKey>,
    pub open_fill_shapes: Vec<(ObjectId, LayerKey)>,
    pub non_finite_objects: Vec<ObjectId>,
    /// OffsetFill, timings, wobble, Hull frame.
    pub unsupported: Vec<(LayerKey, String)>,
    pub tool_layer_objects: Vec<ObjectId>,
    /// Project hint vs machine kind.
    pub device_mismatch: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("geometry: {0}")]
    Geom(#[from] openlaser_geom::Error),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

/// Plans a project for a machine. `Err` only for a geometry failure that is not a refusal (a stubbed or broken
/// geometry kernel); everything about the design itself becomes a section, a frame path or a fact.
pub fn plan(project: &Project, machine: &MachineProfile, opts: &PlanOptions) -> Result<JobPlan, Error> {
    planner::plan(project, machine, opts)
}

/// The device kind a machine profile drives.
pub fn device_of(machine: &MachineProfile) -> DeviceKind {
    planner::device_of(machine)
}

/// Design mm → machine mm: field angle, swap, negate, offset. Never scale (scale lives in F).
pub fn machine_transform(m: &GalvoMachine) -> Affine {
    transform::machine_transform(m)
}

/// Galvo time model: mark length / actual speed + jump length / jump speed + jump delays + laser on/off/polygon
/// delays per vertex + MO settle + first-jump settle.
pub fn estimate_galvo(paths: &[Polyline], q: &GalvoCutQ, m: &GalvoMachine) -> Duration {
    estimate::estimate_galvo(paths, q, m)
}

/// Gantry time model: travel at the machine's travel speed, burn moves at the layer speed (no acceleration).
pub fn estimate_gantry(paths: &[Polyline], q: &GantryCutQ, m: &GantryMachine) -> Duration {
    estimate::estimate_gantry(paths, q, m)
}
