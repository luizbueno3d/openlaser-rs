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
pub use project::{AssetId, BitmapData, DeviceHint, DeviceKind, Geom, JobSettings, Project, Selection, Shape, Workspace, WorkspaceOrigin, default_layer_name};

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
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}
