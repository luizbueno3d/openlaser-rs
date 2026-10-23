//! Display lists and deterministic PNG rendering (SPEC §3.10; owned by WP2). WP0 stubs.
#![forbid(unsafe_code)]

use std::sync::Arc;

pub use openlaser_doc::Project;
pub use openlaser_geom::{P2, Polyline};
pub use openlaser_machine::MachineProfile;
pub use openlaser_plan::JobPlan;
pub use openlaser_types::{Bounds, EmittedSegment, Notice, ObjectId, Refusal};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("render too large: {0}")]
    TooLarge(String),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba(pub [u8; 4]);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stroke {
    Hairline,
    Dashed,
    Width(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    Refusal,
    Notice(u32),
    Origin,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Polylines {
        stroke: Stroke,
        color: Rgba,
        lines: Arc<[Polyline]>,
        object: Option<ObjectId>,
    },
    /// Hatch, emission, travel.
    Segments {
        color: Rgba,
        segs: Arc<[[P2; 2]]>,
    },
    /// Field, work area, selection box.
    Rect {
        stroke: Stroke,
        color: Rgba,
        rect: Bounds,
    },
    Marker {
        kind: MarkerKind,
        at: P2,
        color: Rgba,
    },
    Image {
        rect: Bounds,
        gray: Arc<[u8]>,
        px: [u32; 2],
    },
}

/// World mm, Y up.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DisplayList {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewOpts {
    pub dark: bool,
    pub selection: Vec<ObjectId>,
    pub refusals: Vec<Refusal>,
    pub notices: Vec<Notice>,
    pub show_field: bool,
    pub show_work_area: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanShow {
    pub burn: bool,
    pub travel: bool,
    pub frame: bool,
    pub order: bool,
}

pub fn document_list(p: &Project, m: Option<&MachineProfile>, o: &ViewOpts) -> DisplayList {
    let _ = (p, m, o);
    DisplayList::default()
}

pub fn plan_list(plan: &JobPlan, m: &MachineProfile, show: &PlanShow) -> DisplayList {
    let _ = (plan, m, show);
    DisplayList::default()
}

pub fn emission_list(burned: &[EmittedSegment], light: &[[P2; 2]], m: &MachineProfile) -> DisplayList {
    let _ = (burned, light, m);
    DisplayList::default()
}

pub fn fit(list: &DisplayList, margin_frac: f64) -> Option<Bounds> {
    let _ = (list, margin_frac);
    None
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderView {
    pub world: Bounds,
    pub max_side: u32,
    pub background: Rgba,
}

/// Deterministic CPU raster (same input → same bytes). max_side <= 2048; output <= 5 MiB or Err.
pub fn render_png(list: &DisplayList, view: &RenderView) -> Result<Vec<u8>, Error> {
    let _ = (list, view);
    Err(Error::NotImplemented("openlaser_render::render_png"))
}
