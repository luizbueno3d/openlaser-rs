//! The project model (SPEC §3.3; owned by WP1). Pure data.
//!
#![forbid(unsafe_code)]

mod layers;
mod palette;
mod project;

pub use layers::{
    ColorLayer, CutMode, CutOp, CutTable, FillParams, GalvoParams, GalvoTimings, GantryParams, ImageCut, ImageMode, LayerDefaults, ToolLayer, Wobble,
};
pub use palette::{COLOR_LAYERS, PALETTE, TOOL_COLORS};
pub use project::{
    AssetId, BitmapData, DeviceHint, DeviceKind, Geom, JobSettings, MAX_ARC_CUBICS_PER_QUARTER, MAX_NEXT_ID, Project, Selection, Shape, Workspace,
    WorkspaceOrigin, default_layer_name,
};

// Re-exports for callers that name these through the model (SPEC §3.0).
pub use kurbo::{Affine, BezPath};
pub use openlaser_geom::Polyline;
pub use openlaser_types::{Bounds, Deg, FrameSource, FrameStyle, Hz, LayerKey, MachineId, Mm, MmPerS, Ns, ObjectId, Percent, Rgb, Us};

pub const FORMAT_VERSION: u32 = 1;

/// Deserialize JSON with the workspace's float-roundtrip parser.
pub fn deserialize_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|error| error.to_string())
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    /// WP1 addition: `Project::alloc_id` cannot hand out a new id (`next_id` is 0 or would pass `MAX_NEXT_ID`).
    #[error("no object id left to allocate (next id {next_id}; ids run 1..2^53)")]
    IdsExhausted { next_id: u64 },
    /// A pass count of 0 (`CutOp::passes`, `ColorLayer::global_passes` or `ImageCut::passes`): the layer would
    /// plan nothing for its shapes. Refused on load/save and by the planner via [`CutTable::check_passes`].
    #[error("layer {layer} has zero passes at {field}")]
    ZeroPasses { layer: LayerKey, field: String },
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}
