//! Importers (SPEC §3.6).
#![forbid(unsafe_code)]

use std::sync::Arc;

use image::GenericImageView;
use openlaser_doc::{Affine, AssetId, BezPath, BitmapData, Geom, PALETTE};
use openlaser_geom::P2;
use openlaser_types::{NoticeCode, ObjectId, Percent};

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

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
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
    let extension = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase());
    match extension.as_deref() {
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
        _ if String::from_utf8_lossy(bytes).trim_start().starts_with("<svg") || String::from_utf8_lossy(bytes).contains("<svg") => Some(Format::Svg),
        _ => None,
    }
}

pub fn import(name: &str, bytes: &[u8], opts: &ImportOptions) -> Result<Imported, Error> {
    validate_options(opts)?;
    match sniff(name, bytes).ok_or_else(|| Error::UnknownFormat(name.to_owned()))? {
        Format::Svg => import_svg(name, bytes, opts),
        Format::Dxf => import_dxf(name, bytes, opts),
        Format::Png | Format::Jpeg | Format::Bmp | Format::Tiff | Format::Webp | Format::Gif => import_raster(name, bytes, opts),
        Format::OmWorkspace => {
            let workspace = om::parse_workspace(bytes)?;
            let mut imported = om::to_imported(&workspace, &om::builtin_layers())?;
            imported.notices.push(Notice {
                n: 1,
                code: NoticeCode::ImportEntitySkipped,
                message: "layer_settings.json was not supplied; OM built-in layers were used".to_owned(),
                object: None,
                layer: None,
            });
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
        && (!x.is_finite() || !y.is_finite())
    {
        return Err(Error::Malformed { format: "import options", message: "centre must be finite".to_owned() });
    }
    Ok(())
}

fn looks_like_workspace(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes).ok().and_then(|value| value.get("objects").cloned()).is_some_and(|objects| objects.is_array())
}

fn import_dxf(name: &str, bytes: &[u8], opts: &ImportOptions) -> Result<Imported> {
    let mut polylines = dxf_om::parse_dxf_om(bytes)?;
    if polylines.is_empty() {
        return Err(Error::Malformed { format: "DXF", message: format!("no usable geometry found in {name}") });
    }
    let unit_scale = match opts.dxf_units {
        DxfUnits::Auto => dxf_om::dxf_units_to_mm(bytes)?,
        DxfUnits::Mm => 1.0,
        DxfUnits::Inch => 25.4,
    };
    transform_polylines(&mut polylines, unit_scale * opts.scale, opts.center_at, false)?;
    paths_to_imported(polylines, opts.layer.unwrap_or(opts.current_layer), Vec::new())
}

fn import_svg(name: &str, bytes: &[u8], opts: &ImportOptions) -> Result<Imported> {
    let source = std::str::from_utf8(bytes).map_err(|e| Error::Malformed { format: "SVG", message: e.to_string() })?;
    if opts.svg_units == SvgUnits::RequirePhysical && !svg_has_physical_size(source) {
        return Err(Error::Unitless(name.to_owned()));
    }
    let dpi = match opts.svg_units {
        SvgUnits::Px72 => 72.0,
        SvgUnits::RequirePhysical | SvgUnits::Px96 => 96.0,
    };
    let parser_opts = usvg::Options { dpi, ..usvg::Options::default() };
    let tree = usvg::Tree::from_data(bytes, &parser_opts).map_err(|e| Error::Malformed { format: "SVG", message: e.to_string() })?;
    let mm_per_px = 25.4 / f64::from(dpi);
    let mut imported = Imported { shapes: Vec::new(), layers: Vec::new(), notices: Vec::new(), bounds: None };
    let mut notice_n = 1u32;
    if source.contains("<text") {
        imported.notices.push(Notice {
            n: notice_n,
            code: NoticeCode::ImportTextIgnored,
            message: "SVG text was ignored; convert text to paths before importing".to_owned(),
            object: None,
            layer: None,
        });
        notice_n += 1;
    }
    collect_svg_nodes(tree.root(), opts, mm_per_px, &mut imported, &mut notice_n)?;
    if imported.shapes.is_empty() {
        return Err(Error::Malformed { format: "SVG", message: format!("no usable paths found in {name}") });
    }
    imported.bounds = shape_bounds(&imported.shapes, opts.flatten_tol_mm);
    recenter_shapes(&mut imported.shapes, imported.bounds, opts.center_at, opts.scale)?;
    imported.bounds = shape_bounds(&imported.shapes, opts.flatten_tol_mm);
    Ok(imported)
}

fn collect_svg_nodes(group: &usvg::Group, opts: &ImportOptions, mm_per_px: f64, imported: &mut Imported, notice_n: &mut u32) -> Result<()> {
    for node in group.children() {
        match node {
            usvg::Node::Group(child) => collect_svg_nodes(child, opts, mm_per_px, imported, notice_n)?,
            usvg::Node::Path(path) => {
                let layer = opts.layer.unwrap_or_else(|| svg_layer(path, opts.current_layer, imported, notice_n));
                let bez = svg_bez_path(path, mm_per_px)?;
                if !bez.is_empty() {
                    imported.shapes.push(shape(layer, Geom::Path(Arc::new(bez)), Affine::IDENTITY)?);
                }
            }
            usvg::Node::Image(_) | usvg::Node::Text(_) => {}
        }
    }
    Ok(())
}

fn svg_bez_path(path: &usvg::Path, scale: f64) -> Result<BezPath> {
    use usvg::tiny_skia_path::PathSegment;
    let t = path.abs_transform();
    let mut bez = BezPath::new();
    for segment in path.data().segments() {
        match segment {
            PathSegment::MoveTo(p) => bez.move_to(svg_point(p, t, scale)),
            PathSegment::LineTo(p) => bez.line_to(svg_point(p, t, scale)),
            PathSegment::QuadTo(p1, p2) => bez.quad_to(svg_point(p1, t, scale), svg_point(p2, t, scale)),
            PathSegment::CubicTo(p1, p2, p3) => bez.curve_to(svg_point(p1, t, scale), svg_point(p2, t, scale), svg_point(p3, t, scale)),
            PathSegment::Close => bez.close_path(),
        }
    }
    if bez.elements().len() > 2_000_000 {
        return Err(Error::TooLarge("SVG contains more than 2,000,000 path elements".to_owned()));
    }
    Ok(bez)
}

fn svg_point(p: usvg::tiny_skia_path::Point, t: usvg::Transform, scale: f64) -> openlaser_geom::Point {
    let x = f64::from(t.sx) * f64::from(p.x) + f64::from(t.kx) * f64::from(p.y) + f64::from(t.tx);
    let y = f64::from(t.ky) * f64::from(p.x) + f64::from(t.sy) * f64::from(p.y) + f64::from(t.ty);
    openlaser_geom::Point::new(x * scale, -y * scale)
}

fn svg_layer(path: &usvg::Path, current: LayerKey, imported: &mut Imported, notice_n: &mut u32) -> LayerKey {
    let paint = path.stroke().map(usvg::Stroke::paint).or_else(|| path.fill().map(usvg::Fill::paint));
    if let Some(usvg::Paint::Color(color)) = paint {
        let rgb = [color.red, color.green, color.blue];
        if let Some(index) = PALETTE.iter().position(|candidate| candidate.0 == rgb)
            && let Ok(index) = u8::try_from(index)
        {
            return LayerKey::Color(index);
        }
    }
    imported.notices.push(Notice {
        n: *notice_n,
        code: NoticeCode::ImportColorUnmatched,
        message: format!("SVG colour did not exactly match the palette; assigned to {current}"),
        object: None,
        layer: Some(current),
    });
    *notice_n = notice_n.saturating_add(1);
    current
}

fn svg_has_physical_size(source: &str) -> bool {
    ["mm", "cm", "in", "pt", "pc"].iter().any(|unit| {
        ["width", "height"].iter().all(|attribute| attribute_value(source, attribute).is_some_and(|value| value.trim().to_ascii_lowercase().ends_with(unit)))
    })
}

fn attribute_value<'a>(source: &'a str, attribute: &str) -> Option<&'a str> {
    let marker = format!("{attribute}=");
    let rest = source.split_once(&marker)?.1.trim_start();
    let quote = rest.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    rest.get(1..)?.split_once(quote).map(|(value, _)| value)
}

fn import_raster(name: &str, bytes: &[u8], opts: &ImportOptions) -> Result<Imported> {
    let image = image::load_from_memory(bytes).map_err(|e| Error::Malformed { format: "image", message: e.to_string() })?;
    let (width, height) = image.dimensions();
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > 16_777_216 {
        return Err(Error::TooLarge(format!("image has {pixels} pixels; maximum is 16777216")));
    }
    let dpi = raster_dpi(bytes).unwrap_or(254.0);
    let size_mm = [f64::from(width) * 25.4 / dpi * opts.scale, f64::from(height) * 25.4 / dpi * opts.scale];
    let gray: Arc<[u8]> = image.to_luma8().into_raw().into();
    let bitmap = BitmapData {
        asset: AssetId(*blake3::hash(bytes).as_bytes()),
        px: [width, height],
        size_mm,
        gray,
        source_ext: name.rsplit_once('.').map_or("image", |(_, ext)| ext).to_ascii_lowercase(),
    };
    let [cx, cy] = opts.center_at.unwrap_or([0.0, 0.0]);
    let xform = Affine::translate((cx - size_mm[0] / 2.0, cy - size_mm[1] / 2.0));
    let layer = opts.layer.unwrap_or(opts.current_layer);
    let mut notices = Vec::new();
    if raster_dpi(bytes).is_none() {
        notices.push(Notice {
            n: 1,
            code: NoticeCode::ImportEntitySkipped,
            message: "Image has no usable DPI metadata; imported at 254 DPI (10 px/mm)".to_owned(),
            object: None,
            layer: Some(layer),
        });
    }
    Ok(Imported {
        shapes: vec![shape(layer, Geom::Bitmap(Arc::new(bitmap)), xform)?],
        layers: Vec::new(),
        notices,
        bounds: Some(Bounds { min: [cx - size_mm[0] / 2.0, cy - size_mm[1] / 2.0], max: [cx + size_mm[0] / 2.0, cy + size_mm[1] / 2.0] }),
    })
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
    }
    None
}

fn paths_to_imported(polylines: Vec<Polyline>, layer: LayerKey, notices: Vec<Notice>) -> Result<Imported> {
    let bounds = polyline_bounds(&polylines);
    let mut shapes = Vec::with_capacity(polylines.len());
    for polyline in polylines {
        let mut path = BezPath::new();
        let Some([first_x, first_y]) = polyline.0.first().copied() else { continue };
        path.move_to((first_x, first_y));
        for [x, y] in polyline.0.into_iter().skip(1) {
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

fn transform_polylines(polylines: &mut [Polyline], scale: f64, center_at: Option<P2>, flip_y: bool) -> Result<()> {
    let bounds = polyline_bounds(polylines).ok_or_else(|| Error::Malformed { format: "geometry", message: "no finite points".to_owned() })?;
    let source_center = [(bounds.min[0] + bounds.max[0]) / 2.0, (bounds.min[1] + bounds.max[1]) / 2.0];
    let target = center_at.unwrap_or(source_center);
    for line in polylines {
        for point in &mut line.0 {
            let [x, y] = *point;
            let y = if flip_y { -y } else { y };
            *point = [(x - source_center[0]) * scale + target[0], (y - source_center[1]) * scale + target[1]];
        }
    }
    Ok(())
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

fn shape_bounds(shapes: &[Shape], tolerance: f64) -> Option<Bounds> {
    let _ = tolerance;
    let mut bounds: Option<Bounds> = None;
    for shape in shapes {
        if let Geom::Path(path) = &shape.geom {
            for element in path.elements() {
                let ps: Vec<openlaser_geom::Point> = match element {
                    openlaser_geom::PathEl::MoveTo(p) | openlaser_geom::PathEl::LineTo(p) => vec![*p],
                    openlaser_geom::PathEl::QuadTo(p1, p2) => vec![*p1, *p2],
                    openlaser_geom::PathEl::CurveTo(p1, p2, p3) => vec![*p1, *p2, *p3],
                    openlaser_geom::PathEl::ClosePath => continue,
                };
                for p in ps {
                    let p = shape.xform * p;
                    let point = [p.x, p.y];
                    bounds = Some(match bounds {
                        None => Bounds { min: point, max: point },
                        Some(old) => {
                            Bounds { min: [old.min[0].min(point[0]), old.min[1].min(point[1])], max: [old.max[0].max(point[0]), old.max[1].max(point[1])] }
                        }
                    });
                }
            }
        }
    }
    bounds
}

fn recenter_shapes(shapes: &mut [Shape], bounds: Option<Bounds>, center_at: Option<P2>, scale: f64) -> Result<()> {
    let Some(bounds) = bounds else { return Ok(()) };
    let source = [(bounds.min[0] + bounds.max[0]) / 2.0, (bounds.min[1] + bounds.max[1]) / 2.0];
    let target = center_at.unwrap_or(source);
    let transform = Affine::translate((target[0], target[1])) * Affine::scale(scale) * Affine::translate((-source[0], -source[1]));
    for shape in shapes {
        shape.xform = transform * shape.xform;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_true_scale_mm() {
        let bytes = include_bytes!("../../../tests/fixtures/square10mm.svg");
        let imported = import("square10mm.svg", bytes, &ImportOptions::default()).expect("valid SVG");
        let bounds = imported.bounds.expect("path bounds");
        assert!((bounds.max[0] - bounds.min[0] - 10.0).abs() < 1e-6, "bounds: {bounds:?}");
        assert!((bounds.max[1] - bounds.min[1] - 10.0).abs() < 1e-6);
    }

    #[test]
    fn svg_unitless_refused_unless_explicit_units() {
        let bytes = include_bytes!("../../../tests/fixtures/unitless.svg");
        assert!(matches!(import("unitless.svg", bytes, &ImportOptions::default()), Err(Error::Unitless(_))));
        let opts = ImportOptions { svg_units: SvgUnits::Px96, ..ImportOptions::default() };
        assert!(import("unitless.svg", bytes, &opts).is_ok());
    }

    #[test]
    fn om_malformed_point_is_a_b9_refusal() {
        let bytes = br#"{"version":1,"objects":[{"name":"bad-shape","layer_id":"x","polylines":[[[0,1,2]]],"x":0,"y":0,"scale":1,"rotation":0}]}"#;
        assert!(matches!(om::parse_workspace(bytes), Err(Error::MalformedPoint { object, .. }) if object == "bad-shape"));
    }

    #[test]
    fn om_fixture_import_preserves_objects_and_layers() {
        let root = openlaser_testkit::fixture("om_small");
        let workspace = std::fs::read(root.join("workspace.json")).expect("fixture workspace");
        let layer_data = std::fs::read(root.join("layer_settings.json")).expect("fixture settings");
        let ws = om::parse_workspace(&workspace).expect("workspace parses");
        let layers = om::parse_layer_settings(&layer_data).expect("settings parse");
        let imported = om::to_imported(&ws, &layers).expect("workspace imports");
        assert_eq!(imported.shapes.len(), 3);
        assert_eq!(imported.layers.len(), 3);
    }

    #[test]
    fn om_business_card_imports_403_objects_and_3057_paths() {
        let root = openlaser_testkit::golden_dir().join("inputs");
        let workspace = std::fs::read(root.join("workspace.json")).expect("business card workspace");
        let layer_data = std::fs::read(root.join("layer_settings.json")).expect("business card settings");
        let ws = om::parse_workspace(&workspace).expect("workspace parses");
        let layers = om::parse_layer_settings(&layer_data).expect("settings parse");
        let imported = om::to_imported(&ws, &layers).expect("business card imports");
        assert_eq!(imported.shapes.len(), 403);
        assert_eq!(ws.objects.iter().flat_map(|object| &object.polylines).map(Vec::len).sum::<usize>(), 3057);
        assert_eq!(imported.layers.len(), 3);
    }

    #[test]
    fn svg_palette_match_uses_exact_rgb_value() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"><path d="M1 1L9 9" stroke="#000000" fill="none"/></svg>"##;
        let imported = import("palette.svg", svg, &ImportOptions::default()).expect("SVG imports");
        assert_eq!(imported.shapes.first().map(|shape| shape.layer), Some(LayerKey::Color(0)));
    }

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config { cases: 256, .. proptest::test_runner::Config::default() })]
        #[test]
        fn prop_importers_never_panic(bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..4096)) {
            let _ = import("random.unknown", &bytes, &ImportOptions::default());
            let _ = dxf_om::parse_dxf_om(&bytes);
            let _ = om::parse_workspace(&bytes);
        }
    }
}
