//! Importers (SPEC §3.6; owned by WP2). WP0 stubs.
#![forbid(unsafe_code)]

pub mod dxf_om;
pub mod om;

pub use openlaser_doc::{ColorLayer, Shape};
pub use openlaser_geom::Polyline;
pub use openlaser_types::{Bounds, LayerKey, Notice};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Svg,
    Dxf,
    Png,
    Jpeg,
    Bmp,
    Tiff,
    Webp,
    Gif,
    OmWorkspace,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportOptions {
    /// Force a layer; else colour → palette match, else the current layer.
    pub layer: Option<LayerKey>,
    pub current_layer: LayerKey,
    /// Design mm; None = keep file coordinates (SVG/DXF) or centre on (0,0) (images).
    pub center_at: Option<[f64; 2]>,
    /// 1.0.
    pub scale: f64,
    pub svg_units: SvgUnits,
    pub dxf_units: DxfUnits,
    pub flatten_tol_mm: f64,
}

/// Default RequirePhysical: refuse unitless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgUnits {
    RequirePhysical,
    Px96,
    Px72,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DxfUnits {
    Auto,
    Mm,
    Inch,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// Ids are ObjectId(0): the engine assigns ids.
    pub shapes: Vec<Shape>,
    /// Layer settings the file carries (OM workspace); applied by the engine.
    pub layers: Vec<(LayerKey, ColorLayer)>,
    pub notices: Vec<Notice>,
    pub bounds: Option<Bounds>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown file type: {0}")]
    UnknownFormat(String),
    #[error("{0} has no physical units; choose 96 or 72 px per inch")]
    Unitless(String),
    #[error("malformed {format}: {message}")]
    Malformed { format: &'static str, message: String },
    /// B9.
    #[error("malformed point in {object}: {message}")]
    MalformedPoint { object: String, message: String },
    #[error("too large: {0}")]
    TooLarge(String),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

pub fn sniff(name: &str, bytes: &[u8]) -> Option<Format> {
    let _ = (name, bytes);
    None
}

pub fn import(name: &str, bytes: &[u8], opts: &ImportOptions) -> Result<Imported, Error> {
    let _ = (name, bytes, opts);
    Err(Error::NotImplemented("openlaser_import::import"))
}
