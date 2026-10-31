//! Importers (SPEC §3.6): SVG through usvg at true scale, DXF (the OM parser port plus an evaluated-SPLINE shipping
//! variant), LightBurn `.lbrn2`, raster images and the prototype's (OM) `workspace.json` + `layer_settings.json`.
//!
//! Every import is bounded (SPEC §1.3 rule 6, never crash): the input size, the number of output points (per entity
//! and in total), the curve-evaluation work and the raster size are capped, and an SVG is measured by the SVG guard
//! (`svg_guard`) before any parser that could recurse, expand or allocate without a bound sees it. A file over a cap
//! is refused with [`Error::TooLarge`]. Coordinates that are not finite after unit scaling and placement are refused,
//! never passed on. Nothing in a file is dropped silently, and nothing an SVG hides is burned: what is not imported
//! is either a refusal or a numbered [`Notice`].
#![forbid(unsafe_code)]

use std::collections::BTreeSet;
#[cfg(test)]
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use openlaser_doc::{Affine, AssetId, BezPath, BitmapData, Geom, PALETTE};
use openlaser_geom::P2;
use openlaser_types::{NoticeCode, ObjectId, Percent};
use usvg::roxmltree;

pub mod dxf_om;
pub mod lbrn2;
pub mod om;
mod svg_guard;

pub use openlaser_doc::{ColorLayer, Shape};
pub use openlaser_geom::Polyline;
pub use openlaser_types::{Bounds, LayerKey, Notice};

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Largest SVG, DXF, LBRN2 or OM workspace / layer-settings file accepted, in bytes.
pub const MAX_VECTOR_FILE_BYTES: usize = 32 * 1024 * 1024;
/// Largest raster file accepted, in bytes.
pub const MAX_RASTER_FILE_BYTES: usize = 64 * 1024 * 1024;
/// Most output points one import may produce, all shapes together.
pub const MAX_IMPORT_POINTS: usize = 2_000_000;
/// Most points one entity (a DXF entity, an SVG path, an OM object) may produce.
pub const MAX_ENTITY_POINTS: usize = 200_000;
/// Most curve-evaluation steps (spline sampling) one import may spend.
pub const MAX_IMPORT_WORK: u64 = 50_000_000;
/// Deepest SVG element nesting accepted, counted on the raw text before any parser runs and again once `<use>` is
/// expanded (a `<use>` counts two levels). roxmltree, usvg and the tree walk recurse per level; this keeps them far
/// inside a 2 MiB thread stack.
pub const MAX_SVG_DEPTH: usize = 256;
/// Most SVG elements once `<use>` references are expanded (computed before usvg expands anything).
pub const MAX_SVG_ELEMENTS: u64 = 500_000;
/// Most XML nodes (elements, text, comments) roxmltree may build.
pub const MAX_SVG_XML_NODES: u32 = 1_000_000;
/// Largest total size of the CSS in `<style>` elements, and of one `style` attribute, in bytes.
pub const MAX_SVG_STYLE_BYTES: usize = 64 * 1024;
/// Longest SVG `d` or `points` attribute, in bytes.
pub const MAX_SVG_PATH_DATA_BYTES: usize = 4 * 1024 * 1024;
/// Most cubic curves one SVG arc may become (the count grows with the radius; svgtypes' cost is quadratic in it).
pub const MAX_SVG_ARC_CURVES: u32 = 256;
/// Most path segments in a row that draw nothing (a repeated ClosePath): svgtypes recurses once per such segment.
pub const MAX_SVG_EMPTY_SEGMENTS: usize = 256;
/// Most bytes usvg may read or copy (attributes, text, style declarations, dash arrays, gradient stops) once `<use>`
/// references are expanded.
pub const MAX_SVG_EXPANDED_BYTES: u64 = 256 * 1024 * 1024;
/// Largest markup the importer's own `<use>` expansion may write (usvg applies a `<use>` transform twice, so `<use>`
/// elements are expanded into `<g>` clones before usvg ever sees them).
pub const MAX_SVG_EXPANDED_SOURCE_BYTES: usize = 64 * 1024 * 1024;
/// Most style-sheet selector work usvg may do: simple-selector parts (`.a.b` is two) × the attributes each element
/// match reads (one per element plus one per attribute), once `<use>` is expanded.
pub const MAX_SVG_CSS_MATCHES: u64 = 1 << 26;
/// Most attributes one SVG start tag may have (roxmltree checks each against all before it).
pub const MAX_SVG_ATTRIBUTES: usize = 1024;
/// Most attribute pairs roxmltree's duplicate check compares, all start tags together (Σ n·(n−1)/2).
pub const MAX_SVG_ATTRIBUTE_PAIRS: u64 = 1 << 27;
/// Most `xmlns` / `xmlns:*` declarations in one SVG (roxmltree resolves namespaces by linear scans).
pub const MAX_SVG_NAMESPACES: usize = 64;
/// Widest SVG `stroke-width`, in user units (usvg outlines every stroke to measure it; the work grows with the width).
pub const MAX_SVG_STROKE_WIDTH: f64 = 1000.0;
/// Most XML nodes (elements, text, comments, processing instructions) usvg visits once `<use>` is expanded.
pub const MAX_SVG_EXPANDED_NODES: u64 = 2_000_000;
/// Most gradient copies usvg may make: one per painted element for object-bounding-box gradients (or context paints),
/// else one per referenced gradient. usvg deduplicates them with a linear scan each, so the work is quadratic.
pub const MAX_SVG_GRADIENT_COPIES: u64 = 20_000;
/// Most attribute scans usvg's tree may cost: each style declaration applied scans the element's attributes, and each
/// inherited-property lookup scans every ancestor's.
pub const MAX_SVG_ATTRIBUTE_WORK: u64 = 1 << 27;
/// Most text simplecss may rescan: it computes a line and column, from the start of the style text, at every token
/// it rejects (a number without a unit, `#`, the end of each declaration), so long style texts cost quadratic time.
pub const MAX_SVG_STYLE_SCAN: u64 = 1 << 30;
/// Longest `class` or `id` in an SVG with a style sheet (each selector comparison reads it).
pub const MAX_SVG_NAME_BYTES: usize = 256;
/// Longest chain of gradients naming each other with `href`.
pub const MAX_SVG_HREF_CHAIN: usize = 16;
/// Most pixels a raster image may have (16 Mpx).
pub const MAX_RASTER_PIXELS: u64 = 16_777_216;
/// Longest raster side, in pixels.
pub const MAX_RASTER_SIDE: u32 = 16_384;
/// Most bytes the image decoder may allocate.
pub const MAX_RASTER_ALLOC: u64 = 256 * 1024 * 1024;
/// Largest coordinate magnitude accepted after placement, in mm (1000 km): beyond it, downstream bounds arithmetic
/// could overflow.
pub const MAX_COORD_MM: f64 = 1e9;

/// The notice code used where an importer assumed a default (DXF units, raster DPI, OM built-in layers).
/// `openlaser-types` (WP1) has no dedicated code yet; the integrator swaps this one line when it lands.
const ASSUMED_DEFAULT: NoticeCode = NoticeCode::ImportEntitySkipped;

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
    Lbrn2,
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
    /// Required for LightBurn projects: defaults from the target machine profile.
    pub layer_defaults: Option<openlaser_doc::LayerDefaults>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            layer_defaults: None,
            layer: None,
            current_layer: LayerKey::Color(0),
            center_at: None,
            scale: 1.0,
            svg_units: SvgUnits::RequirePhysical,
            dxf_units: DxfUnits::Auto,
            flatten_tol_mm: openlaser_geom::DEFAULT_FLATTEN_TOL_MM,
        }
    }
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
    /// Layer settings the file carries (OM workspace or LightBurn project); applied by the engine.
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
    let extension = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase());
    match extension.as_deref() {
        Some("lbrn2") => Some(Format::Lbrn2),
        Some("svg") => Some(Format::Svg),
        Some("dxf") => Some(Format::Dxf),
        Some("png") => Some(Format::Png),
        Some("jpg" | "jpeg") => Some(Format::Jpeg),
        Some("bmp") => Some(Format::Bmp),
        Some("tif" | "tiff") => Some(Format::Tiff),
        Some("webp") => Some(Format::Webp),
        Some("gif") => Some(Format::Gif),
        Some("json") if looks_like_workspace(bytes) => Some(Format::OmWorkspace),
        _ if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => Some(Format::Png),
        _ if bytes.starts_with(&[0xff, 0xd8, 0xff]) => Some(Format::Jpeg),
        _ if bytes.starts_with(b"BM") => Some(Format::Bmp),
        _ if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") => Some(Format::Gif),
        _ if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") => Some(Format::Tiff),
        _ if bytes.get(0..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") => Some(Format::Webp),
        _ if contains(bytes.get(..bytes.len().min(1 << 20)).unwrap_or(bytes), b"<LightBurnProject") => Some(Format::Lbrn2),
        _ if contains(bytes.get(..bytes.len().min(1 << 20)).unwrap_or(bytes), b"<svg") => Some(Format::Svg),
        _ => None,
    }
}

pub fn import(name: &str, bytes: &[u8], opts: &ImportOptions) -> Result<Imported, Error> {
    validate_options(opts)?;
    match sniff(name, bytes).ok_or_else(|| Error::UnknownFormat(name.to_owned()))? {
        Format::Lbrn2 => lbrn2::import(bytes, opts),
        Format::Svg => import_svg(name, bytes, opts),
        Format::Dxf => import_dxf(name, bytes, opts),
        Format::Png | Format::Jpeg | Format::Bmp | Format::Tiff | Format::Webp | Format::Gif => import_raster(name, bytes, opts),
        Format::OmWorkspace => {
            let workspace = om::parse_workspace(bytes)?;
            let mut imported = om::to_imported(&workspace, &om::builtin_layers())?;
            let mut notices = Notices(std::mem::take(&mut imported.notices));
            notices.push(ASSUMED_DEFAULT, "layer_settings.json was not supplied; OM's four built-in layers were used".to_owned(), None);
            imported.notices = notices.0;
            Ok(imported)
        }
    }
}

fn validate_options(opts: &ImportOptions) -> Result<()> {
    if !opts.scale.is_finite() || opts.scale <= 0.0 {
        return Err(Error::Malformed { format: "import options", message: format!("scale must be finite and positive (got {})", opts.scale) });
    }
    if !opts.flatten_tol_mm.is_finite() || opts.flatten_tol_mm <= 0.0 {
        return Err(Error::Malformed { format: "import options", message: "flatten tolerance must be finite and positive".to_owned() });
    }
    if let Some([x, y]) = opts.center_at
        && (!x.is_finite() || !y.is_finite() || x.abs() > MAX_COORD_MM || y.abs() > MAX_COORD_MM)
    {
        return Err(Error::Malformed { format: "import options", message: format!("centre must be finite and within ±{MAX_COORD_MM} mm") });
    }
    Ok(())
}

/// Work and size limits of one import (see the crate docs).
pub(crate) struct Budget {
    points_left: usize,
    work_left: u64,
}

impl Budget {
    pub(crate) fn new() -> Self {
        Budget { points_left: MAX_IMPORT_POINTS, work_left: MAX_IMPORT_WORK }
    }

    /// Accounts one finished entity of `n` points.
    pub(crate) fn entity(&mut self, n: usize) -> Result<()> {
        check_entity_points(n)?;
        self.points_left = self.points_left.checked_sub(n).ok_or_else(|| Error::TooLarge(format!("the file produces more than {MAX_IMPORT_POINTS} points")))?;
        Ok(())
    }

    /// Accounts `steps` of curve evaluation, before doing them.
    pub(crate) fn work(&mut self, steps: u64) -> Result<()> {
        self.work_left = self
            .work_left
            .checked_sub(steps)
            .ok_or_else(|| Error::TooLarge(format!("evaluating the file's curves would take more than {MAX_IMPORT_WORK} steps")))?;
        Ok(())
    }
}

/// Refuses an entity that has (or is about to have) more than [`MAX_ENTITY_POINTS`] points.
pub(crate) fn check_entity_points(n: usize) -> Result<()> {
    if n > MAX_ENTITY_POINTS {
        return Err(Error::TooLarge(format!("one entity has {n} points; the limit is {MAX_ENTITY_POINTS}")));
    }
    Ok(())
}

pub(crate) fn check_vector_size<'a>(format: &'static str, bytes: &'a [u8]) -> Result<&'a [u8]> {
    if bytes.len() > MAX_VECTOR_FILE_BYTES {
        return Err(Error::TooLarge(format!("{format} file is {} bytes; the limit is {MAX_VECTOR_FILE_BYTES}", bytes.len())));
    }
    Ok(bytes)
}

/// A finite placed coordinate within ±[`MAX_COORD_MM`].
pub(crate) fn check_placed(x: f64, y: f64, what: &str) -> Result<()> {
    if !x.is_finite() || !y.is_finite() {
        return Err(Error::Malformed { format: "geometry", message: format!("{what} has a coordinate that is not finite after unit scaling and placement") });
    }
    if x.abs() > MAX_COORD_MM || y.abs() > MAX_COORD_MM {
        return Err(Error::TooLarge(format!("{what} reaches ({x}, {y}) mm, beyond ±{MAX_COORD_MM} mm")));
    }
    Ok(())
}

/// Numbered notices of one import (n = 1, 2, …).
#[derive(Default)]
pub(crate) struct Notices(pub(crate) Vec<Notice>);

impl Notices {
    pub(crate) fn push(&mut self, code: NoticeCode, message: String, layer: Option<LayerKey>) {
        let n = u32::try_from(self.0.len()).map_or(u32::MAX, |len| len.saturating_add(1));
        self.0.push(Notice { n, code, message, object: None, layer });
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|window| window == needle)
}

fn looks_like_workspace(bytes: &[u8]) -> bool {
    #[derive(serde::Deserialize)]
    struct Probe {
        #[allow(dead_code)]
        objects: Vec<serde::de::IgnoredAny>,
    }
    bytes.len() <= MAX_VECTOR_FILE_BYTES && serde_json::from_slice::<Probe>(bytes).is_ok()
}

// ---- DXF ------------------------------------------------------------------------------------------------------

fn import_dxf(name: &str, bytes: &[u8], opts: &ImportOptions) -> Result<Imported> {
    let text = dxf_om::decode_ignore(check_vector_size("DXF", bytes)?);
    let pairs = dxf_om::dxf_pairs(&text)?;
    let (unit_scale, assumed) = match opts.dxf_units {
        DxfUnits::Auto => dxf_om::dxf_units(&pairs)?,
        DxfUnits::Mm => (1.0, false),
        DxfUnits::Inch => (25.4, false),
    };
    let mut budget = Budget::new();
    let parsed = dxf_om::parse_entities(&pairs, dxf_om::Splines::Evaluate(opts.flatten_tol_mm / unit_scale), &mut budget)?;
    if parsed.polylines.is_empty() {
        return Err(Error::Malformed { format: "DXF", message: format!("no usable geometry found in {name}") });
    }
    let mut polylines = parsed.polylines;
    place_polylines(&mut polylines, unit_scale, opts.scale, opts.center_at)?;
    let layer = opts.layer.unwrap_or(opts.current_layer);
    let mut notices = Notices::default();
    if assumed {
        notices.push(ASSUMED_DEFAULT, format!("{name} gives no known drawing units ($INSUNITS); millimetres were assumed"), Some(layer));
    }
    if parsed.degenerate > 0 {
        notices.push(NoticeCode::ImportEntitySkipped, format!("{} DXF entities with fewer than 2 points were skipped", parsed.degenerate), Some(layer));
    }
    let unsupported = dxf_om::unsupported_entities(&pairs);
    if !unsupported.is_empty() {
        let list = unsupported.iter().map(|(kind, count)| format!("{kind} ×{count}")).collect::<Vec<_>>().join(", ");
        let total: usize = unsupported.values().sum();
        notices.push(NoticeCode::ImportEntitySkipped, format!("{total} DXF entities were not imported (unsupported types: {list})"), Some(layer));
    }
    paths_to_imported(polylines, layer, notices.0)
}

/// File units → mm first (so the centre is in mm too), then `scale` about the drawing's centre, moved to
/// `center_at` when given. With `center_at: None` and `scale == 1` the file's coordinates are kept exactly
/// (in mm).
fn place_polylines(polylines: &mut [Polyline], unit_to_mm: f64, scale: f64, center_at: Option<P2>) -> Result<()> {
    for line in polylines.iter_mut() {
        for point in &mut line.0 {
            let [x, y] = *point;
            *point = [x * unit_to_mm, y * unit_to_mm];
            if !point[0].is_finite() || !point[1].is_finite() {
                return Err(Error::Malformed { format: "DXF", message: format!("point ({x}, {y}) is not finite in millimetres") });
            }
        }
    }
    let bounds = polyline_bounds(polylines).ok_or_else(|| Error::Malformed { format: "geometry", message: "no finite points".to_owned() })?;
    if scale != 1.0 || center_at.is_some() {
        let source = [bounds.min[0] / 2.0 + bounds.max[0] / 2.0, bounds.min[1] / 2.0 + bounds.max[1] / 2.0];
        let target = center_at.unwrap_or(source);
        for line in polylines.iter_mut() {
            for point in &mut line.0 {
                let [x, y] = *point;
                *point = [(x - source[0]) * scale + target[0], (y - source[1]) * scale + target[1]];
            }
        }
    }
    for line in polylines.iter() {
        for &[x, y] in &line.0 {
            check_placed(x, y, "the DXF drawing")?;
        }
    }
    Ok(())
}

// ---- SVG ------------------------------------------------------------------------------------------------------

fn import_svg(name: &str, bytes: &[u8], opts: &ImportOptions) -> Result<Imported> {
    let source = std::str::from_utf8(check_vector_size("SVG", bytes)?).map_err(|e| Error::Malformed { format: "SVG", message: e.to_string() })?;
    let source = svg_guard::prepare(source)?;
    let dpi: f32 = match opts.svg_units {
        SvgUnits::Px72 => 72.0,
        SvgUnits::RequirePhysical | SvgUnits::Px96 => 96.0,
    };
    let doc = parse_svg_xml(source.as_ref())?;
    // `<use>` elements are expanded into `<g>` clones before usvg sees them: usvg applies the use element's
    // `transform` a second time when it computes the clone's absolute transform (a use-translated shape lands twice
    // as far as a browser puts it). The expanded markup gets the same bounded parse and the same guard run.
    match svg_guard::expand_uses(&doc, source.as_ref(), f64::from(dpi))? {
        Some(markup) => {
            let doc = parse_svg_xml(&markup)?;
            import_svg_tree(name, &doc, opts, dpi)
        }
        None => import_svg_tree(name, &doc, opts, dpi),
    }
}

/// Parses an SVG document with the bounded options, converting a panic inside roxmltree into an error (third-party
/// parsers must never take the process down, never-crash).
fn parse_svg_xml(source: &str) -> Result<roxmltree::Document<'_>> {
    let xml_opts = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: MAX_SVG_XML_NODES };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| roxmltree::Document::parse_with_options(source, xml_opts)))
        .map_err(|_| Error::Malformed { format: "SVG", message: "the XML parser panicked on this file".to_owned() })?
        .map_err(|e| match e {
            roxmltree::Error::NodesLimitReached => Error::TooLarge(format!("SVG has more than {MAX_SVG_XML_NODES} XML nodes")),
            // No entity is ever declared (a DOCTYPE internal subset is refused), so any `&name;` is undefined:
            // typically an older Illustrator file whose subset was deleted by hand.
            roxmltree::Error::UnknownEntityReference(entity, pos) => Error::Malformed {
                format: "SVG",
                message: format!(
                    "the entity reference &{entity}; (line {}, column {}) is not defined (entity declarations, which SVGs saved by older Adobe Illustrator \
                     versions rely on, are not supported); export the file again without entities (Illustrator: Save As SVG 1.1 with \"Preserve \
                     Illustrator Editing Capabilities\" off, or File > Export > Export As SVG)",
                    pos.row, pos.col
                ),
            },
            other => Error::Malformed { format: "SVG", message: other.to_string() },
        })
}

fn import_svg_tree(name: &str, doc: &roxmltree::Document<'_>, opts: &ImportOptions, dpi: f32) -> Result<Imported> {
    let facts = svg_guard::check(doc)?;
    if opts.svg_units == SvgUnits::RequirePhysical && !svg_root_has_physical_size(doc.root_element()) {
        return Err(Error::Unitless(name.to_owned()));
    }
    let refused_images = Arc::new(AtomicUsize::new(0));
    let parser_opts = svg_parser_options(dpi, Arc::clone(&refused_images));
    let tree = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| usvg::Tree::from_xmltree(doc, &parser_opts)))
        .map_err(|_| Error::Malformed { format: "SVG", message: "the SVG parser panicked on this file".to_owned() })?
        .map_err(|e| match e {
            usvg::Error::ElementsLimitReached | usvg::Error::ParsingFailed(roxmltree::Error::NodesLimitReached) => {
                Error::TooLarge("SVG nesting or <use> expansion is too large".to_owned())
            }
            other => Error::Malformed { format: "SVG", message: other.to_string() },
        })?;
    let mm_per_px = 25.4 / f64::from(dpi);
    let mut shapes = Vec::new();
    let mut budget = Budget::new();
    let mut unmatched: BTreeSet<String> = BTreeSet::new();
    let mut unmatched_paths = 0usize;
    let (mut hidden_paths, mut dashed_paths) = (0usize, 0usize);
    // Each level carries whether an ancestor group is fully transparent (`opacity="0"`).
    let mut stack = vec![(tree.root().children().iter(), false)];
    while let Some((top, transparent)) = stack.last_mut() {
        let transparent = *transparent;
        let Some(node) = top.next() else {
            stack.pop();
            continue;
        };
        match node {
            usvg::Node::Group(group) => {
                // Clip paths and masks from the file are refused before usvg runs; what remains is the viewport usvg
                // clips a `<symbol>` or nested `<svg>` to. The walk ignores clips, so it would burn what they hide.
                if group.clip_path().is_some() || group.mask().is_some() {
                    return Err(Error::NotImplemented(
                        "SVG content clipped by a <symbol> or nested <svg> viewport; set overflow=\"visible\" on it or expand the symbol before importing",
                    ));
                }
                if !group.filters().is_empty() {
                    return Err(Error::NotImplemented("SVG filter effects; remove them before importing"));
                }
                stack.push((group.children().iter(), transparent || group.opacity().get() <= 0.0));
            }
            usvg::Node::Path(path) => {
                // Geometry the SVG does not show is never burned: hidden, fully transparent, or neither filled nor
                // stroked (usvg marks the last as not visible).
                let paint_shows = path.fill().is_some_and(|f| f.opacity().get() > 0.0) || path.stroke().is_some_and(|s| s.opacity().get() > 0.0);
                if transparent || !path.is_visible() || !paint_shows {
                    hidden_paths += 1;
                    continue;
                }
                let bez = svg_bez_path(path, mm_per_px)?;
                if bez.elements().is_empty() {
                    continue;
                }
                budget.entity(bez_point_count(&bez))?;
                dashed_paths += usize::from(path.stroke().is_some_and(|s| s.dasharray().is_some()));
                let layer = match opts.layer {
                    Some(layer) => layer,
                    None => match svg_palette_layer(path) {
                        Ok(layer) => layer,
                        Err(colour) => {
                            unmatched.insert(colour);
                            unmatched_paths += 1;
                            opts.current_layer
                        }
                    },
                };
                shapes.push(shape(layer, Geom::Path(Arc::new(bez)), Affine::IDENTITY)?);
            }
            usvg::Node::Image(_) | usvg::Node::Text(_) => {}
        }
    }
    let mut notices = Notices::default();
    if facts.display_none > 0 {
        notices.push(
            NoticeCode::ImportEntitySkipped,
            format!("{} SVG elements are hidden with display:none; they and their contents were not imported", facts.display_none),
            None,
        );
    }
    if facts.sheet_hides {
        notices.push(NoticeCode::ImportEntitySkipped, "the SVG style sheet hides elements with display:none; those were not imported".to_owned(), None);
    }
    if hidden_paths > 0 {
        notices.push(
            NoticeCode::ImportEntitySkipped,
            format!("{hidden_paths} SVG paths are invisible (visibility hidden, opacity 0, or neither fill nor stroke); they were not imported"),
            None,
        );
    }
    // What usvg drops without a word, measured by the guard.
    if facts.failed_conditions > 0 {
        notices.push(
            NoticeCode::ImportEntitySkipped,
            format!(
                "{} SVG elements outside <switch> have a requiredFeatures, requiredExtensions or systemLanguage condition that is not met; they and their contents were not imported",
                facts.failed_conditions
            ),
            None,
        );
    }
    if facts.degenerate_transforms > 0 {
        notices.push(
            NoticeCode::ImportEntitySkipped,
            format!(
                "{} SVG elements have a transform that scales to zero or is not finite; they and their contents were not imported",
                facts.degenerate_transforms
            ),
            None,
        );
    }
    if facts.f32_overflows > 0 {
        notices.push(
            NoticeCode::ImportEntitySkipped,
            format!("{} SVG elements have coordinates beyond ±3.4e38 (single precision); geometry that uses them was not imported", facts.f32_overflows),
            None,
        );
    }
    if facts.foreign_objects > 0 {
        notices.push(
            NoticeCode::ImportEntitySkipped,
            format!("{} SVG <foreignObject> elements (HTML or other non-SVG content) were not imported", facts.foreign_objects),
            None,
        );
    }
    if shapes.is_empty() {
        let hidden =
            if notices.0.is_empty() { String::new() } else { format!(" ({})", notices.0.iter().map(|n| n.message.as_str()).collect::<Vec<_>>().join("; ")) };
        return Err(Error::Malformed { format: "SVG", message: format!("no usable paths found in {name}{hidden}") });
    }
    let bounds = shape_bounds(&shapes);
    recenter_shapes(&mut shapes, bounds, opts.center_at, opts.scale);
    let bounds = shape_bounds(&shapes);
    if let Some(b) = bounds {
        check_placed(b.min[0], b.min[1], "the SVG drawing")?;
        check_placed(b.max[0], b.max[1], "the SVG drawing")?;
    }
    if dashed_paths > 0 {
        notices.push(ASSUMED_DEFAULT, format!("{dashed_paths} SVG paths have dashed strokes; they were imported as solid paths"), None);
    }
    if facts.texts > 0 {
        notices.push(NoticeCode::ImportTextIgnored, format!("{} SVG text elements were ignored; convert text to paths before importing", facts.texts), None);
    }
    let images = facts.images.max(refused_images.load(Ordering::Relaxed));
    if images > 0 {
        notices.push(
            NoticeCode::ImportEntitySkipped,
            format!("{images} SVG <image> elements were ignored (embedded and linked images are never loaded; import the image file itself)"),
            None,
        );
    }
    if unmatched_paths > 0 {
        let colours = unmatched.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
        let more = if unmatched.len() > 8 { format!(" and {} more", unmatched.len() - 8) } else { String::new() };
        notices.push(
            NoticeCode::ImportColorUnmatched,
            format!(
                "{unmatched_paths} SVG paths have colours that are not in the layer palette ({colours}{more}); they were assigned to {}",
                opts.current_layer
            ),
            Some(opts.current_layer),
        );
    }
    Ok(Imported { shapes, layers: Vec::new(), notices: notices.0, bounds })
}

/// usvg options for import: never reads a file or decodes an image for `<image>` (both href resolvers return
/// `None`; usvg's defaults would `std::fs::read` any path, including `/dev/zero` or a UNC share). Each refused href
/// is counted for the notice.
pub(crate) fn svg_parser_options(dpi: f32, refused_images: Arc<AtomicUsize>) -> usvg::Options<'static> {
    let data_count = Arc::clone(&refused_images);
    usvg::Options {
        dpi,
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(move |_, _, _| {
                data_count.fetch_add(1, Ordering::Relaxed);
                None
            }),
            resolve_string: Box::new(move |_, _| {
                refused_images.fetch_add(1, Ordering::Relaxed);
                None
            }),
        },
        ..usvg::Options::default()
    }
}

/// Width and height of the root `<svg>` element only (never a child's `width=`, never `stroke-width=`), both in
/// physical units.
pub(crate) fn svg_root_has_physical_size(root: roxmltree::Node<'_, '_>) -> bool {
    [root.attribute("width"), root.attribute("height")].into_iter().all(|value| value.is_some_and(is_physical_length))
}

fn is_physical_length(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    ["mm", "cm", "in", "pt", "pc"]
        .iter()
        .any(|unit| value.strip_suffix(unit).is_some_and(|number| number.parse::<f64>().is_ok_and(|n| n.is_finite() && n > 0.0)))
}

fn svg_bez_path(path: &usvg::Path, scale: f64) -> Result<BezPath> {
    use usvg::tiny_skia_path::PathSegment;
    let t = path.abs_transform();
    let mut bez = BezPath::new();
    let mut started = false;
    for segment in path.data().segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                bez.move_to(svg_point(p, t, scale)?);
                started = true;
            }
            PathSegment::LineTo(p) => {
                if !started {
                    // kurbo's BezPath push debug_asserts the path starts with a MoveTo.
                    return Err(Error::Malformed { format: "SVG", message: "a path does not start with a moveto".to_owned() });
                }
                bez.line_to(svg_point(p, t, scale)?);
            }
            PathSegment::QuadTo(p1, p2) => {
                if !started {
                    return Err(Error::Malformed { format: "SVG", message: "a path does not start with a moveto".to_owned() });
                }
                bez.quad_to(svg_point(p1, t, scale)?, svg_point(p2, t, scale)?);
            }
            PathSegment::CubicTo(p1, p2, p3) => {
                if !started {
                    return Err(Error::Malformed { format: "SVG", message: "a path does not start with a moveto".to_owned() });
                }
                bez.curve_to(svg_point(p1, t, scale)?, svg_point(p2, t, scale)?, svg_point(p3, t, scale)?);
            }
            PathSegment::Close => bez.close_path(),
        }
    }
    Ok(bez)
}

fn svg_point(p: usvg::tiny_skia_path::Point, t: usvg::Transform, scale: f64) -> Result<openlaser_geom::Point> {
    let x = f64::from(t.sx) * f64::from(p.x) + f64::from(t.kx) * f64::from(p.y) + f64::from(t.tx);
    let y = f64::from(t.ky) * f64::from(p.x) + f64::from(t.sy) * f64::from(p.y) + f64::from(t.ty);
    let (x, y) = (x * scale, -y * scale);
    check_placed(x, y, "an SVG path")?;
    Ok(openlaser_geom::Point::new(x, y))
}

fn bez_point_count(bez: &BezPath) -> usize {
    bez.elements()
        .iter()
        .map(|element| match element {
            openlaser_geom::PathEl::MoveTo(_) | openlaser_geom::PathEl::LineTo(_) => 1,
            openlaser_geom::PathEl::QuadTo(..) => 2,
            openlaser_geom::PathEl::CurveTo(..) => 3,
            openlaser_geom::PathEl::ClosePath => 0,
        })
        .sum()
}

/// The palette layer whose colour exactly matches the path's stroke (else fill); `Err` carries the colour
/// (`#rrggbb`, or a description) for the unmatched-colour notice.
fn svg_palette_layer(path: &usvg::Path) -> Result<LayerKey, String> {
    let paint = path.stroke().map(usvg::Stroke::paint).or_else(|| path.fill().map(usvg::Fill::paint));
    match paint {
        Some(usvg::Paint::Color(color)) => {
            let rgb = [color.red, color.green, color.blue];
            PALETTE
                .iter()
                .position(|candidate| candidate.0 == rgb)
                .and_then(|index| u8::try_from(index).ok())
                .map(LayerKey::Color)
                .ok_or_else(|| format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]))
        }
        Some(_) => Err("a gradient or pattern".to_owned()),
        None => Err("no stroke or fill".to_owned()),
    }
}

// ---- raster ---------------------------------------------------------------------------------------------------

fn import_raster(name: &str, bytes: &[u8], opts: &ImportOptions) -> Result<Imported> {
    if bytes.len() > MAX_RASTER_FILE_BYTES {
        return Err(Error::TooLarge(format!("image file is {} bytes; the limit is {MAX_RASTER_FILE_BYTES}", bytes.len())));
    }
    let format = image::guess_format(bytes).map_err(|e| Error::Malformed { format: "image", message: e.to_string() })?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_RASTER_SIDE);
    limits.max_image_height = Some(MAX_RASTER_SIDE);
    limits.max_alloc = Some(MAX_RASTER_ALLOC);
    // The header alone first: the size limits apply before any pixel is decoded.
    let mut header = image::ImageReader::with_format(Cursor::new(bytes), format);
    header.limits(limits.clone());
    let (width, height) = header.into_dimensions().map_err(image_error)?;
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > MAX_RASTER_PIXELS {
        return Err(Error::TooLarge(format!("image has {width} × {height} = {pixels} pixels; the limit is {MAX_RASTER_PIXELS}")));
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let image = reader.decode().map_err(image_error)?;
    let file_dpi = raster_dpi(bytes);
    let dpi = file_dpi.unwrap_or(254.0);
    let size_mm = [f64::from(width) * 25.4 / dpi * opts.scale, f64::from(height) * 25.4 / dpi * opts.scale];
    // Preserve the existing luma conversion for opaque images, including 16-bit input. Alpha is coverage
    // over white, not darkness: a transparent black RGB value must never become a black burn pixel.
    let gray: Arc<[u8]> = if image.color().has_alpha() {
        image
            .to_luma_alpha8()
            .pixels()
            .map(|p| {
                let [luma, alpha] = p.0;
                let a = u32::from(alpha);
                ((u32::from(luma) * a + 255 * (255 - a) + 127) / 255) as u8
            })
            .collect::<Vec<_>>()
            .into()
    } else {
        image.to_luma8().into_raw().into()
    };
    let [cx, cy] = opts.center_at.unwrap_or([0.0, 0.0]);
    let bounds = Bounds { min: [cx - size_mm[0] / 2.0, cy - size_mm[1] / 2.0], max: [cx + size_mm[0] / 2.0, cy + size_mm[1] / 2.0] };
    check_placed(bounds.min[0], bounds.min[1], "the image")?;
    check_placed(bounds.max[0], bounds.max[1], "the image")?;
    let bitmap = BitmapData {
        asset: AssetId(*blake3::hash(bytes).as_bytes()),
        px: [width, height],
        size_mm,
        gray,
        source_ext: name.rsplit_once('.').map_or("image", |(_, ext)| ext).to_ascii_lowercase(),
    };
    let layer = opts.layer.unwrap_or(opts.current_layer);
    let mut notices = Notices::default();
    if file_dpi.is_none() {
        notices.push(ASSUMED_DEFAULT, "Image has no usable DPI metadata; imported at 254 DPI (10 px/mm)".to_owned(), Some(layer));
    }
    let xform = Affine::translate((bounds.min[0], bounds.min[1]));
    Ok(Imported { shapes: vec![shape(layer, Geom::Bitmap(Arc::new(bitmap)), xform)?], layers: Vec::new(), notices: notices.0, bounds: Some(bounds) })
}

fn image_error(e: image::ImageError) -> Error {
    match e {
        image::ImageError::Limits(limit) => Error::TooLarge(format!("image exceeds the decoder limits ({limit}): at most {MAX_RASTER_SIDE} px a side")),
        other => Error::Malformed { format: "image", message: other.to_string() },
    }
}

fn raster_dpi(bytes: &[u8]) -> Option<f64> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let mut offset = 8usize;
        while let Some(header) = bytes.get(offset..offset.saturating_add(8)) {
            let size = usize::try_from(u32::from_be_bytes(header.get(..4)?.try_into().ok()?)).ok()?;
            let kind = header.get(4..8)?;
            let data_start = offset.checked_add(8)?;
            let data_end = data_start.checked_add(size)?;
            let data = bytes.get(data_start..data_end)?;
            if kind == b"pHYs" && data.len() == 9 {
                let x = u32::from_be_bytes(data.get(..4)?.try_into().ok()?);
                if data.get(8) == Some(&1) && x > 0 {
                    return Some(f64::from(x) * 0.0254);
                }
            }
            offset = data_end.checked_add(4)?;
        }
        return None;
    }
    if bytes.starts_with(b"BM") && bytes.len() >= 46 {
        let ppm = i32::from_le_bytes(bytes.get(38..42)?.try_into().ok()?);
        let dpi = f64::from(ppm) * 0.0254;
        return (ppm > 0 && dpi.is_finite()).then_some(dpi);
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        return jpeg_dpi(bytes);
    }
    if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        return tiff_dpi(bytes);
    }
    if bytes.get(0..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        let mut offset = 12usize;
        while let Some(header) = bytes.get(offset..offset.checked_add(8)?) {
            let size = usize::try_from(u32::from_le_bytes(header.get(4..8)?.try_into().ok()?)).ok()?;
            let start = offset.checked_add(8)?;
            let end = start.checked_add(size)?;
            let data = bytes.get(start..end)?;
            if header.get(..4) == Some(b"EXIF") {
                return tiff_dpi(data.strip_prefix(b"Exif\0\0").unwrap_or(data));
            }
            offset = end.checked_add(size & 1)?;
        }
    }
    None
}

fn jpeg_dpi(bytes: &[u8]) -> Option<f64> {
    let mut offset = 2usize;
    while bytes.get(offset) == Some(&0xff) {
        let marker = *bytes.get(offset.checked_add(1)?)?;
        offset = offset.checked_add(2)?;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        let length = usize::from(u16::from_be_bytes(bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?));
        if length < 2 {
            return None;
        }
        let end = offset.checked_add(length)?;
        let segment = bytes.get(offset.checked_add(2)?..end)?;
        if marker == 0xe0 && segment.starts_with(b"JFIF\0") && segment.len() >= 12 {
            let units = *segment.get(7)?;
            let density = u16::from_be_bytes(segment.get(8..10)?.try_into().ok()?);
            let dpi = match units {
                1 => f64::from(density),
                2 => f64::from(density) * 2.54,
                _ => 0.0,
            };
            if dpi.is_finite() && dpi > 0.0 {
                return Some(dpi);
            }
        }
        if marker == 0xe1
            && segment.starts_with(b"Exif\0\0")
            && let Some(dpi) = tiff_dpi(segment.get(6..)?)
        {
            return Some(dpi);
        }
        offset = end;
    }
    None
}

fn tiff_dpi(bytes: &[u8]) -> Option<f64> {
    let little = match bytes.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |offset: usize| -> Option<u16> {
        let s = bytes.get(offset..offset.checked_add(2)?)?;
        Some(if little { u16::from_le_bytes(s.try_into().ok()?) } else { u16::from_be_bytes(s.try_into().ok()?) })
    };
    let u32_at = |offset: usize| -> Option<u32> {
        let s = bytes.get(offset..offset.checked_add(4)?)?;
        Some(if little { u32::from_le_bytes(s.try_into().ok()?) } else { u32::from_be_bytes(s.try_into().ok()?) })
    };
    if u16_at(2)? != 42 {
        return None;
    }
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let count = usize::from(u16_at(ifd)?);
    let mut resolution = None;
    let mut unit = 2u16;
    for index in 0..count {
        let entry = ifd.checked_add(2)?.checked_add(index.checked_mul(12)?)?;
        let tag = u16_at(entry)?;
        if tag == 296 {
            unit = u16_at(entry.checked_add(8)?)?;
        }
        if tag == 282 && u16_at(entry.checked_add(2)?)? == 5 && u32_at(entry.checked_add(4)?)? == 1 {
            let value_offset = usize::try_from(u32_at(entry.checked_add(8)?)?).ok()?;
            let numerator = f64::from(u32_at(value_offset)?);
            let denominator = f64::from(u32_at(value_offset.checked_add(4)?)?);
            if denominator > 0.0 {
                resolution = Some(numerator / denominator);
            }
        }
    }
    let value = resolution?;
    let dpi = match unit {
        2 => value,
        3 => value * 2.54,
        _ => return None,
    };
    (dpi.is_finite() && dpi > 0.0).then_some(dpi)
}

// ---- shared ---------------------------------------------------------------------------------------------------

fn paths_to_imported(polylines: Vec<Polyline>, layer: LayerKey, notices: Vec<Notice>) -> Result<Imported> {
    let bounds = polyline_bounds(&polylines);
    let mut shapes = Vec::with_capacity(polylines.len());
    for polyline in polylines {
        let mut points = polyline.0.into_iter();
        let Some([first_x, first_y]) = points.next() else { continue };
        let mut path = BezPath::new();
        path.move_to((first_x, first_y));
        for [x, y] in points {
            path.line_to((x, y));
        }
        shapes.push(shape(layer, Geom::Path(Arc::new(path)), Affine::IDENTITY)?);
    }
    Ok(Imported { shapes, layers: Vec::new(), notices, bounds })
}

fn shape(layer: LayerKey, geom: Geom, xform: Affine) -> Result<Shape> {
    let power_scale = Percent::new(100.0).map_err(|e| Error::Malformed { format: "shape", message: e.to_string() })?;
    Ok(Shape { id: ObjectId(0), name: None, layer, xform, geom, locked: false, power_scale, priority: 0 })
}

fn polyline_bounds(polylines: &[Polyline]) -> Option<Bounds> {
    let mut bounds: Option<Bounds> = None;
    for line in polylines {
        for &[x, y] in &line.0 {
            if !x.is_finite() || !y.is_finite() {
                return None;
            }
            bounds = Some(match bounds {
                None => Bounds { min: [x, y], max: [x, y] },
                Some(old) => Bounds { min: [old.min[0].min(x), old.min[1].min(y)], max: [old.max[0].max(x), old.max[1].max(y)] },
            });
        }
    }
    bounds
}

/// Bounds of the shapes' path control points (a superset of the curves), after each shape's transform.
fn shape_bounds(shapes: &[Shape]) -> Option<Bounds> {
    let mut bounds: Option<Bounds> = None;
    for shape in shapes {
        let Geom::Path(path) = &shape.geom else { continue };
        for element in path.elements() {
            let points: &[openlaser_geom::Point] = match element {
                openlaser_geom::PathEl::MoveTo(p) | openlaser_geom::PathEl::LineTo(p) => std::slice::from_ref(p),
                openlaser_geom::PathEl::QuadTo(p1, p2) => &[*p1, *p2],
                openlaser_geom::PathEl::CurveTo(p1, p2, p3) => &[*p1, *p2, *p3],
                openlaser_geom::PathEl::ClosePath => &[],
            };
            for p in points {
                let p = shape.xform * *p;
                bounds = Some(match bounds {
                    None => Bounds { min: [p.x, p.y], max: [p.x, p.y] },
                    Some(old) => Bounds { min: [old.min[0].min(p.x), old.min[1].min(p.y)], max: [old.max[0].max(p.x), old.max[1].max(p.y)] },
                });
            }
        }
    }
    bounds
}

/// `scale` about the drawing's centre, moved to `center_at` when given (SVG keeps its coordinates otherwise).
fn recenter_shapes(shapes: &mut [Shape], bounds: Option<Bounds>, center_at: Option<P2>, scale: f64) {
    let Some(bounds) = bounds else { return };
    if scale == 1.0 && center_at.is_none() {
        return;
    }
    let source = [bounds.min[0] / 2.0 + bounds.max[0] / 2.0, bounds.min[1] / 2.0 + bounds.max[1] / 2.0];
    let target = center_at.unwrap_or(source);
    let transform = Affine::translate((target[0], target[1])) * Affine::scale(scale) * Affine::translate((-source[0], -source[1]));
    for shape in shapes {
        shape.xform = transform * shape.xform;
    }
}

/// How many notices of each code an import produced (tests).
#[cfg(test)]
fn notice_counts(imported: &Imported) -> HashMap<NoticeCode, usize> {
    let mut map = HashMap::new();
    for notice in &imported.notices {
        *map.entry(notice.code).or_insert(0) += 1;
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svg(body: &str) -> Vec<u8> {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="20mm" height="20mm" viewBox="0 0 20 20">{body}</svg>"#
        )
        .into_bytes()
    }

    #[test]
    fn svg_image_href_resolver_never_reads_files() {
        // usvg's default resolver would read this file (std::fs::read on any href) and return Some(PNG).
        let png = openlaser_testkit::fixture("gray64.png");
        let png_bytes = std::fs::read(&png).expect("fixture");
        let refused = Arc::new(AtomicUsize::new(0));
        let opts = svg_parser_options(96.0, Arc::clone(&refused));
        assert!((opts.image_href_resolver.resolve_string)(png.to_str().expect("utf-8 path"), &opts).is_none());
        assert!((opts.image_href_resolver.resolve_data)("image/png", Arc::new(png_bytes), &opts).is_none());
        assert_eq!(refused.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn svg_image_elements_are_ignored_with_a_notice() {
        let png = openlaser_testkit::fixture("gray64.png");
        let body = format!(
            r##"<image href="{}" width="5" height="5"/><image xlink:href="data:image/png;base64,iVBORw0KGgo=" width="5" height="5"/><path d="M1 1L9 9" stroke="#000000"/>"##,
            png.display()
        );
        let imported = import("image.svg", &svg(&body), &ImportOptions::default()).expect("SVG with images imports");
        assert_eq!(imported.shapes.len(), 1);
        let notice = imported.notices.iter().find(|n| n.message.contains("<image>")).expect("image notice");
        assert!(notice.message.starts_with("2 SVG <image>"), "{}", notice.message);
        assert_eq!(notice.code, NoticeCode::ImportEntitySkipped);
    }

    #[test]
    fn notices_are_counted_by_code() {
        let body = r##"<path d="M1 1L9 9" stroke="#123456"/><path d="M2 2L8 8" stroke="#654321"/><text x="1" y="1">hi</text>"##;
        let imported = import("colours.svg", &svg(body), &ImportOptions::default()).expect("imports");
        let counts = notice_counts(&imported);
        assert_eq!(counts.get(&NoticeCode::ImportColorUnmatched), Some(&1), "one aggregated notice: {:?}", imported.notices);
        assert_eq!(counts.get(&NoticeCode::ImportTextIgnored), Some(&1));
        let numbers: Vec<u32> = imported.notices.iter().map(|n| n.n).collect();
        assert_eq!(numbers, (1..=u32::try_from(numbers.len()).expect("few")).collect::<Vec<_>>());
    }

    #[test]
    fn raster_dpi_reads_bmp_jpeg_and_tiff_metadata() {
        let mut bmp = vec![0u8; 46];
        bmp[..2].copy_from_slice(b"BM");
        bmp[38..42].copy_from_slice(&10_000i32.to_le_bytes());
        assert!((raster_dpi(&bmp).expect("BMP DPI") - 254.0).abs() < 1e-9);

        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0, 1, 2, 1, 0, 0xfe, 0, 0xfe, 0, 0xff, 0xd9];
        assert!((raster_dpi(&jpeg).expect("JFIF DPI") - 254.0).abs() < 1e-9);

        let tiff = [b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x1a, 1, 5, 0, 1, 0, 0, 0, 26, 0, 0, 0, 0, 0, 0, 0, 100, 0, 0, 0, 1, 0, 0, 0];
        assert!((raster_dpi(&tiff).expect("TIFF DPI") - 100.0).abs() < 1e-9);

        let mut webp = b"RIFF\0\0\0\0WEBPEXIF".to_vec();
        webp.extend_from_slice(&(tiff.len() as u32).to_le_bytes());
        webp.extend_from_slice(&tiff);
        let riff_size = (webp.len() - 8) as u32;
        webp[4..8].copy_from_slice(&riff_size.to_le_bytes());
        assert!((raster_dpi(&webp).expect("WebP EXIF DPI") - 100.0).abs() < 1e-9);
    }
}
