//! The planner (SPEC §3.7; owned by WP3, red zone). WP0 stubs.
#![forbid(unsafe_code)]

use std::time::Duration;

pub use openlaser_doc::{DeviceKind, Project};
pub use openlaser_geom::{Affine, Polyline};
pub use openlaser_machine::{GalvoMachine, GantryMachine, MachineProfile};
pub use openlaser_types::galvo::PulseDecision;
pub use openlaser_types::{Fingerprint, FrameSource, FrameStyle, Hz, LayerKey, MachineId, MmPerS, ObjectId, Percent, SectionMode};

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
    /// blake3 over the canonical encoding of machine, device, sections (layer, pass, quantised words, path f64 bit
    /// patterns), frame, tool_frame, repeats, the machine's calibration and transform, frame style/source.
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

pub fn plan(project: &Project, machine: &MachineProfile, opts: &PlanOptions) -> Result<JobPlan, Error> {
    let _ = (project, machine, opts);
    Err(Error::NotImplemented("openlaser_plan::plan"))
}

/// Design mm → machine mm: field angle, swap, negate, offset. Never scale (scale lives in F).
/// WP0 stub: identity.
pub fn machine_transform(m: &GalvoMachine) -> Affine {
    let _ = m;
    Affine::IDENTITY
}

/// Galvo time model: mark length / actual speed + jump length / jump speed + jump delays + laser on/off/polygon
/// delays per vertex + MO settle + first-jump settle. WP0 stub: zero.
pub fn estimate_galvo(paths: &[Polyline], q: &GalvoCutQ, m: &GalvoMachine) -> Duration {
    let _ = (paths, q, m);
    Duration::ZERO
}

/// WP0 stub: zero.
pub fn estimate_gantry(paths: &[Polyline], q: &GantryCutQ, m: &GantryMachine) -> Duration {
    let _ = (paths, q, m);
    Duration::ZERO
}
