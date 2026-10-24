//! `.olp` read/write (SPEC §3.5, S14) and machine profile files (S24); owned by WP1. WP0 stubs.
#![forbid(unsafe_code)]

pub mod machines;

use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use openlaser_doc::{AssetId, BitmapData, Geom, Shape};
use openlaser_geom::{Affine, BezPath, PathEl, Point};
use openlaser_types::{LayerKey, Mm, Percent};
use serde::{Deserialize, Serialize};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

pub use openlaser_doc::Project;

pub const FORMAT: &str = "openlaser-project";
pub const FORMAT_VERSION: u32 = 1;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not an OpenLaser project: {0}")]
    Corrupt(String),
    #[error("project format {found} is newer than this build ({supported})")]
    TooNew { found: u32, supported: u32 },
    #[error("non-finite number at {path}")]
    NonFinite { path: String },
    /// Entries, decompressed bytes, group depth, points, zip-slip names.
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

/// zip: manifest.json {format, formatVersion, generator, project: ProjectM, paths: {pN: "M0 0L10 0…Z"}} +
/// assets/<blake3>.gray8 (raw row-major grayscale pixels). AssetId remains the hash of the original source bytes,
/// which cannot be recomputed or verified after conversion because those bytes are not stored; load verifies the
/// referenced gray8 payload against its asset filename instead. The 64 MiB archive cap applies to decoded
/// grayscale bytes rather than the original encoded image. ProjectM is a separate serde model converted
/// with exhaustive struct literals both ways, so a new doc field fails to compile until it is saved.
pub fn save(project: &Project) -> Result<Vec<u8>, Error> {
    let mut paths = std::collections::BTreeMap::new();
    let mut assets = std::collections::BTreeMap::new();
    let mut next_path = 0usize;
    let model = ProjectM::from_project(project, &mut paths, &mut assets, &mut next_path, 0)?;
    let manifest = Manifest { format: FORMAT.to_owned(), format_version: FORMAT_VERSION, generator: "OpenLaser".to_owned(), project: model, paths };
    // Save refuses exactly what load refuses, so a saved file always loads.
    validate_manifest(&manifest)?;
    let json = serde_json::to_vec(&manifest).map_err(|error| Error::Corrupt(error.to_string()))?;
    check_json_finite(&serde_json::from_slice(&json).map_err(|error| Error::Corrupt(error.to_string()))?, "manifest")?;
    if json.len() > MAX_DECOMPRESSED {
        return Err(Error::LimitExceeded("manifest exceeds 64 MiB".to_owned()));
    }
    let archive_size = assets.values().try_fold(json.len(), |size, asset| size.checked_add(asset.len()));
    if archive_size.is_none_or(|size| size > MAX_DECOMPRESSED) || assets.len().saturating_add(1) > MAX_ENTRIES {
        return Err(Error::LimitExceeded("project assets exceed archive limits".to_owned()));
    }
    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer.start_file("manifest.json", SimpleFileOptions::default()).map_err(|error| Error::Corrupt(error.to_string()))?;
    writer.write_all(&json)?;
    for (name, bytes) in &assets {
        writer.start_file(name, SimpleFileOptions::default()).map_err(|error| Error::Corrupt(error.to_string()))?;
        writer.write_all(bytes)?;
    }
    writer.finish().map(|cursor| cursor.into_inner()).map_err(|error| Error::Corrupt(error.to_string()))
}

pub fn load(bytes: &[u8]) -> Result<Project, Error> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor).map_err(|error| Error::Corrupt(error.to_string()))?;
    if archive.len() > MAX_ENTRIES {
        return Err(Error::LimitExceeded("too many zip entries".to_owned()));
    }
    // Duplicate names are found on the raw central directory, before the reader keeps only one entry per name;
    // local headers that disagree with the central directory and overlapping entries are refused too.
    archive.validate_header_consistency().map_err(|error| Error::Corrupt(format!("inconsistent zip archive: {error}")))?;
    let mut manifest_bytes = Vec::new();
    let mut assets = std::collections::BTreeMap::new();
    let mut total = 0usize;
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|error| Error::Corrupt(error.to_string()))?;
        let name = file.name().map_err(|error| Error::Corrupt(error.to_string()))?.into_owned();
        if name.starts_with('/') || name.split('/').any(|part| part == "..") || name.contains('\\') {
            return Err(Error::LimitExceeded(format!("unsafe zip entry name: {name}")));
        }
        let size = usize::try_from(file.size()).map_err(|_| Error::LimitExceeded("zip entry is too large".to_owned()))?;
        total = total.checked_add(size).ok_or_else(|| Error::LimitExceeded("decompressed size overflow".to_owned()))?;
        check_decompressed_size(total)?;
        if name == "manifest.json" {
            manifest_bytes = read_entry(file, size)?;
        } else if name.starts_with("assets/") {
            let asset_bytes = read_entry(file, size)?;
            if assets.insert(name.clone(), asset_bytes).is_some() {
                return Err(Error::Corrupt(format!("duplicate asset entry {name}")));
            }
        } else {
            // Nothing outside the manifest and its assets is kept, so nothing else may be present.
            return Err(Error::Corrupt(format!("unexpected zip entry {name}")));
        }
    }
    if manifest_bytes.is_empty() {
        return Err(Error::Corrupt("missing manifest.json".to_owned()));
    }
    let value: serde_json::Value = serde_json::from_slice(&manifest_bytes).map_err(|error| Error::Corrupt(error.to_string()))?;
    check_json_finite(&value, "manifest")?;
    let found = value
        .get("formatVersion")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| Error::Corrupt("manifest formatVersion is missing or invalid".to_owned()))?;
    if found > FORMAT_VERSION {
        return Err(Error::TooNew { found, supported: FORMAT_VERSION });
    }
    let manifest: Manifest = serde_json::from_value(value).map_err(|error| Error::Corrupt(error.to_string()))?;
    if manifest.format != FORMAT {
        return Err(Error::Corrupt(format!("unexpected format `{}`", manifest.format)));
    }
    if manifest.format_version > FORMAT_VERSION {
        return Err(Error::TooNew { found: manifest.format_version, supported: FORMAT_VERSION });
    }
    if manifest.format_version != FORMAT_VERSION {
        return Err(Error::Corrupt(format!("unsupported format version {}", manifest.format_version)));
    }
    validate_manifest(&manifest)?;
    let mut referenced = std::collections::BTreeSet::new();
    collect_assets(&manifest.project.shapes, &mut referenced);
    if let Some(name) = assets.keys().find(|name| !referenced.contains(name.as_str())) {
        return Err(Error::Corrupt(format!("unreferenced zip entry {name}")));
    }
    manifest.project.into_project(&manifest.paths, &assets, 0)
}

/// Reads one entry, refusing more bytes than its header declared (the declared sizes are what the limits checked).
fn read_entry(file: impl Read, size: usize) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::with_capacity(size);
    let limit = u64::try_from(size).map_err(|_| Error::LimitExceeded("zip entry is too large".to_owned()))?.saturating_add(1);
    file.take(limit).read_to_end(&mut bytes)?;
    if bytes.len() > size {
        return Err(Error::Corrupt("zip entry is larger than its declared size".to_owned()));
    }
    Ok(bytes)
}

fn collect_assets<'a>(shapes: &'a [ShapeM], out: &mut std::collections::BTreeSet<&'a str>) {
    for shape in shapes {
        match &shape.geom {
            GeomM::Bitmap { gray_asset, .. } => {
                out.insert(gray_asset.as_str());
            }
            GeomM::Group { shapes } => collect_assets(shapes, out),
            _ => {}
        }
    }
}

/// craft-atomic.
pub fn save_to(path: &Path, project: &Project) -> Result<(), Error> {
    let bytes = save(project)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    craft_atomic::atomic_write(path, &bytes)?;
    Ok(())
}

pub fn load_from(path: &Path) -> Result<Project, Error> {
    load(&std::fs::read(path)?)
}

const MAX_DECOMPRESSED: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
/// Path points (M/L 1, Q 2, C 3) plus closes (Z 1) in one project.
const MAX_POINTS: usize = 1_000_000;
const MAX_GROUP_DEPTH: usize = 32;

/// The largest magnitude save and load accept for any design length or coordinate (mm) and any transform
/// coefficient: 1e9 (1000 km). Larger values are never a real design and overflow geometry maths downstream.
pub const MAX_MAGNITUDE: f64 = 1e9;

fn check_decompressed_size(total: usize) -> Result<(), Error> {
    if total > MAX_DECOMPRESSED { Err(Error::LimitExceeded("decompressed project exceeds 64 MiB".to_owned())) } else { Ok(()) }
}

fn bitmap_pixel_count(px: [u32; 2]) -> Result<usize, Error> {
    usize::try_from(px[0])
        .ok()
        .and_then(|width| usize::try_from(px[1]).ok().and_then(|height| width.checked_mul(height)))
        .ok_or_else(|| Error::LimitExceeded("bitmap pixel dimensions overflow".to_owned()))
}

/// Refuses a value above [`MAX_MAGNITUDE`] (NaN included, which serde and the finite checks refuse earlier).
fn check_magnitude(value: f64, what: &str) -> Result<(), Error> {
    if value.abs() <= MAX_MAGNITUDE { Ok(()) } else { Err(Error::LimitExceeded(format!("{what} = {value} is above the 1e9 limit"))) }
}

/// Everything save and load both refuse.
fn validate_manifest(manifest: &Manifest) -> Result<(), Error> {
    check_paths(&manifest.paths)?;
    check_shape_depth(&manifest.project.shapes, 0)?;
    if manifest.project.workspace.size_mm.iter().any(|v| !v.is_finite()) {
        return Err(Error::NonFinite { path: "project.workspace.sizeMm".to_owned() });
    }
    validate_project(&manifest.project, &manifest.paths)
}

fn validate_project(project: &ProjectM, paths: &std::collections::BTreeMap<String, String>) -> Result<(), Error> {
    let corrupt = |message: &str| Error::Corrupt(message.to_owned());
    if project.layers.colors.len() != openlaser_doc::COLOR_LAYERS {
        return Err(corrupt("project must have exactly 30 colour layers"));
    }
    for (position, layer) in project.layers.colors.iter().enumerate() {
        if usize::from(layer.index) != position {
            return Err(corrupt("colour layer index must equal its position"));
        }
        if !(1..=11).contains(&layer.ops.len()) {
            return Err(corrupt("colour layer must have 1..=11 operations"));
        }
    }
    if project.layers.tools[0].which != 1 || project.layers.tools[1].which != 2 {
        return Err(corrupt("tool layers must be T1 and T2 in order"));
    }
    if !(1..=999).contains(&project.job.repeats) {
        return Err(corrupt("job repeats must be in 1..=999"));
    }
    if project.workspace.size_mm.iter().any(|size| *size <= 0.0) {
        return Err(corrupt("workspace dimensions must be positive"));
    }
    for size in project.workspace.size_mm {
        check_magnitude(size, "project.workspace.sizeMm")?;
    }
    let mut order = std::collections::BTreeSet::new();
    for key in &project.layers.order {
        let valid = match key {
            LayerKey::Color(index) => *index < 30,
            LayerKey::Tool(which) => (1..=2).contains(which),
        };
        if !valid || !order.insert(*key) {
            return Err(corrupt("layer order contains an invalid or duplicate entry"));
        }
    }
    let mut seen = ShapeScan::default();
    scan_shapes(&project.shapes, paths, &mut seen)?;
    // The order is the output order: the planner walks it, so it must list every used layer and nothing else.
    if let Some(key) = seen.layers.difference(&order).next() {
        return Err(Error::Corrupt(format!("layer order is missing used layer {key}")));
    }
    if let Some(key) = order.difference(&seen.layers).next() {
        return Err(Error::Corrupt(format!("layer order lists unused layer {key}")));
    }
    if let Some(key) = paths.keys().find(|key| !seen.paths.contains(key.as_str())) {
        return Err(Error::Corrupt(format!("path {key} is not used by any shape")));
    }
    if project.next_id == 0 || project.next_id <= seen.max_id {
        return Err(corrupt("nextId must be greater than every shape id"));
    }
    if project.next_id > openlaser_doc::MAX_NEXT_ID {
        return Err(corrupt("nextId is above 2^53"));
    }
    Ok(())
}

#[derive(Default)]
struct ShapeScan<'a> {
    ids: std::collections::BTreeSet<u64>,
    max_id: u64,
    layers: std::collections::BTreeSet<LayerKey>,
    paths: std::collections::BTreeSet<&'a str>,
}

fn scan_shapes<'a>(shapes: &'a [ShapeM], paths: &std::collections::BTreeMap<String, String>, seen: &mut ShapeScan<'a>) -> Result<(), Error> {
    for shape in shapes {
        if shape.id.0 == 0 || !seen.ids.insert(shape.id.0) {
            return Err(Error::Corrupt("shape ids must be unique and nonzero".to_owned()));
        }
        seen.max_id = seen.max_id.max(shape.id.0);
        let at = |what: &str| format!("shape[{}].{what}", shape.id.0);
        if shape.xform.iter().any(|value| !value.is_finite()) {
            return Err(Error::NonFinite { path: at("xform") });
        }
        for value in shape.xform {
            check_magnitude(value, &at("xform"))?;
        }
        match &shape.geom {
            GeomM::Path { path } => {
                if !paths.contains_key(path) {
                    return Err(Error::Corrupt(format!("missing path {path}")));
                }
                seen.paths.insert(path.as_str());
            }
            GeomM::Rect { w, h, corner_radius } => {
                for (value, what) in [(w.get(), "w"), (h.get(), "h"), (corner_radius.get(), "cornerRadius")] {
                    check_magnitude(value, &at(what))?;
                }
            }
            GeomM::Ellipse { rx, ry } => {
                check_magnitude(rx.get(), &at("rx"))?;
                check_magnitude(ry.get(), &at("ry"))?;
            }
            GeomM::Polygon { sides, radius } => {
                if *sides < 3 {
                    return Err(Error::Corrupt(format!("{} must be at least 3", at("sides"))));
                }
                check_magnitude(radius.get(), &at("radius"))?;
            }
            GeomM::Bitmap { px, size_mm, .. } => {
                if px.contains(&0) {
                    return Err(Error::Corrupt(format!("{} must be at least 1 × 1", at("px"))));
                }
                bitmap_pixel_count(*px)?;
                if size_mm.iter().any(|size| !size.is_finite()) {
                    return Err(Error::NonFinite { path: at("sizeMm") });
                }
                if size_mm.iter().any(|size| *size <= 0.0) {
                    return Err(Error::Corrupt(format!("{} must be positive", at("sizeMm"))));
                }
                for size in size_mm {
                    check_magnitude(*size, &at("sizeMm"))?;
                }
            }
            GeomM::Group { shapes } => scan_shapes(shapes, paths, seen)?,
        }
        if !matches!(shape.geom, GeomM::Group { .. }) {
            seen.layers.insert(shape.layer);
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format: String,
    format_version: u32,
    generator: String,
    project: ProjectM,
    paths: std::collections::BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectM {
    name: String,
    shapes: Vec<ShapeM>,
    layers: openlaser_doc::CutTable,
    workspace: openlaser_doc::Workspace,
    device_hint: Option<openlaser_doc::DeviceHint>,
    job: openlaser_doc::JobSettings,
    notes: String,
    next_id: u64,
    extras: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShapeM {
    id: openlaser_types::ObjectId,
    name: Option<String>,
    layer: LayerKey,
    xform: [f64; 6],
    geom: GeomM,
    locked: bool,
    power_scale: Percent,
    priority: i32,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
enum GeomM {
    Path { path: String },
    Rect { w: Mm, h: Mm, corner_radius: Mm },
    Ellipse { rx: Mm, ry: Mm },
    Polygon { sides: u16, radius: Mm },
    Bitmap { asset: [u8; 32], px: [u32; 2], size_mm: [f64; 2], gray_asset: String, source_ext: String },
    Group { shapes: Vec<ShapeM> },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "cmd", content = "points")]
enum PathCommand {
    Move([f64; 2]),
    Line([f64; 2]),
    Quad([[f64; 2]; 2]),
    Curve([[f64; 2]; 3]),
    Close,
}

impl ProjectM {
    fn from_project(
        project: &Project,
        paths: &mut std::collections::BTreeMap<String, String>,
        assets: &mut std::collections::BTreeMap<String, Vec<u8>>,
        next: &mut usize,
        depth: usize,
    ) -> Result<Self, Error> {
        Ok(Self {
            name: project.name.clone(),
            shapes: project.shapes.iter().map(|shape| ShapeM::from_shape(shape, paths, assets, next, depth)).collect::<Result<_, _>>()?,
            layers: project.layers.clone(),
            workspace: project.workspace.clone(),
            device_hint: project.device_hint.clone(),
            job: project.job.clone(),
            notes: project.notes.clone(),
            next_id: project.next_id,
            extras: project.extras.clone(),
        })
    }
    fn into_project(
        self,
        paths: &std::collections::BTreeMap<String, String>,
        assets: &std::collections::BTreeMap<String, Vec<u8>>,
        depth: usize,
    ) -> Result<Project, Error> {
        if depth > MAX_GROUP_DEPTH {
            return Err(Error::LimitExceeded("group nesting exceeds 32".to_owned()));
        }
        Ok(Project {
            name: self.name,
            shapes: self.shapes.into_iter().map(|shape| shape.into_shape(paths, assets, depth)).collect::<Result<_, _>>()?,
            layers: self.layers,
            workspace: self.workspace,
            device_hint: self.device_hint,
            job: self.job,
            notes: self.notes,
            next_id: self.next_id,
            extras: self.extras,
        })
    }
}

impl ShapeM {
    fn from_shape(
        shape: &Shape,
        paths: &mut std::collections::BTreeMap<String, String>,
        assets: &mut std::collections::BTreeMap<String, Vec<u8>>,
        next: &mut usize,
        depth: usize,
    ) -> Result<Self, Error> {
        if depth > MAX_GROUP_DEPTH {
            return Err(Error::LimitExceeded("group nesting exceeds 32".to_owned()));
        }
        if shape.xform.as_coeffs().iter().any(|value| !value.is_finite()) {
            return Err(Error::NonFinite { path: format!("shape[{}].xform", shape.id.0) });
        }
        let geom = match &shape.geom {
            Geom::Path(path) => {
                if path.elements().iter().map(PathCommand::from_el).flat_map(|cmd| cmd.coordinates()).any(|value| !value.is_finite()) {
                    return Err(Error::NonFinite { path: format!("shape[{}].path", shape.id.0) });
                }
                let key = format!("p{}", *next);
                *next = next.checked_add(1).ok_or_else(|| Error::LimitExceeded("path id overflow".to_owned()))?;
                paths.insert(key.clone(), encode_path(path));
                GeomM::Path { path: key }
            }
            Geom::Rect { w, h, corner_radius } => GeomM::Rect { w: *w, h: *h, corner_radius: *corner_radius },
            Geom::Ellipse { rx, ry } => GeomM::Ellipse { rx: *rx, ry: *ry },
            Geom::Polygon { sides, radius } => GeomM::Polygon { sides: *sides, radius: *radius },
            Geom::Bitmap(bitmap) => {
                let expected = bitmap_pixel_count(bitmap.px)?;
                if bitmap.size_mm.iter().any(|size| !size.is_finite()) {
                    return Err(Error::NonFinite { path: format!("shape[{}].bitmap.sizeMm", shape.id.0) });
                }
                if expected != bitmap.gray.len() {
                    return Err(Error::Corrupt("bitmap pixel dimensions do not match grayscale bytes".to_owned()));
                }
                let digest = blake3::hash(&bitmap.gray);
                let name = format!("assets/{}.gray8", digest.to_hex());
                assets.entry(name.clone()).or_insert_with(|| bitmap.gray.to_vec());
                GeomM::Bitmap { asset: bitmap.asset.0, px: bitmap.px, size_mm: bitmap.size_mm, gray_asset: name, source_ext: bitmap.source_ext.clone() }
            }
            Geom::Group(shapes) => {
                GeomM::Group { shapes: shapes.iter().map(|item| ShapeM::from_shape(item, paths, assets, next, depth + 1)).collect::<Result<_, _>>()? }
            }
        };
        Ok(Self {
            id: shape.id,
            name: shape.name.clone(),
            layer: shape.layer,
            xform: shape.xform.as_coeffs(),
            geom,
            locked: shape.locked,
            power_scale: shape.power_scale,
            priority: shape.priority,
        })
    }
    fn into_shape(
        self,
        paths: &std::collections::BTreeMap<String, String>,
        assets: &std::collections::BTreeMap<String, Vec<u8>>,
        depth: usize,
    ) -> Result<Arc<Shape>, Error> {
        if depth > MAX_GROUP_DEPTH {
            return Err(Error::LimitExceeded("group nesting exceeds 32".to_owned()));
        }
        let geom = match self.geom {
            GeomM::Path { path } => {
                let commands = paths.get(&path).ok_or_else(|| Error::Corrupt(format!("missing path {path}")))?;
                let mut bez = BezPath::new();
                for command in decode_path(commands)? {
                    bez.push(command.to_el());
                }
                Geom::Path(Arc::new(bez))
            }
            GeomM::Rect { w, h, corner_radius } => Geom::Rect { w, h, corner_radius },
            GeomM::Ellipse { rx, ry } => Geom::Ellipse { rx, ry },
            GeomM::Polygon { sides, radius } => Geom::Polygon { sides, radius },
            GeomM::Bitmap { asset, px, size_mm, gray_asset, source_ext } => {
                let expected = bitmap_pixel_count(px)?;
                let gray = assets.get(&gray_asset).ok_or_else(|| Error::Corrupt(format!("missing bitmap asset {gray_asset}")))?;
                if expected != gray.len() {
                    return Err(Error::Corrupt("bitmap pixel dimensions do not match grayscale bytes".to_owned()));
                }
                let expected_name = format!("assets/{}.gray8", blake3::hash(gray).to_hex());
                if gray_asset != expected_name {
                    return Err(Error::Corrupt(format!("bitmap asset hash does not match {gray_asset}")));
                }
                Geom::Bitmap(Arc::new(BitmapData { asset: AssetId(asset), px, size_mm, gray: Arc::from(gray.as_slice()), source_ext }))
            }
            GeomM::Group { shapes } => Geom::Group(
                shapes.into_iter().map(|item| item.into_shape(paths, assets, depth + 1).map(|shape| (*shape).clone().into())).collect::<Result<_, _>>()?,
            ),
        };
        Ok(Arc::new(Shape {
            id: self.id,
            name: self.name,
            layer: self.layer,
            xform: Affine::new(self.xform),
            geom,
            locked: self.locked,
            power_scale: self.power_scale,
            priority: self.priority,
        }))
    }
}

impl PathCommand {
    fn from_el(el: &PathEl) -> Self {
        let p = |point: &Point| [point.x, point.y];
        match el {
            PathEl::MoveTo(a) => Self::Move(p(a)),
            PathEl::LineTo(a) => Self::Line(p(a)),
            PathEl::QuadTo(a, b) => Self::Quad([p(a), p(b)]),
            PathEl::CurveTo(a, b, c) => Self::Curve([p(a), p(b), p(c)]),
            PathEl::ClosePath => Self::Close,
        }
    }
    fn to_el(&self) -> PathEl {
        let p = |point: [f64; 2]| Point::new(point[0], point[1]);
        match self {
            Self::Move(a) => PathEl::MoveTo(p(*a)),
            Self::Line(a) => PathEl::LineTo(p(*a)),
            Self::Quad([a, b]) => PathEl::QuadTo(p(*a), p(*b)),
            Self::Curve([a, b, c]) => PathEl::CurveTo(p(*a), p(*b), p(*c)),
            Self::Close => PathEl::ClosePath,
        }
    }

    fn coordinates(&self) -> Vec<f64> {
        match self {
            Self::Move(p) | Self::Line(p) => p.to_vec(),
            Self::Quad(points) => points.iter().flat_map(|p| p.iter().copied()).collect(),
            Self::Curve(points) => points.iter().flat_map(|p| p.iter().copied()).collect(),
            Self::Close => Vec::new(),
        }
    }
}

fn check_paths(paths: &std::collections::BTreeMap<String, String>) -> Result<(), Error> {
    // Weigh every command before building anything: each point of M/L/Q/C and each Z counts.
    let mut weight = 0usize;
    for path in paths.values() {
        for token in path.split_whitespace() {
            weight += match token {
                "M" | "L" | "Z" => 1,
                "Q" => 2,
                "C" => 3,
                _ => 0,
            };
            if weight > MAX_POINTS {
                return Err(Error::LimitExceeded("project paths exceed 1 M points and closes".to_owned()));
            }
        }
    }
    for (key, path) in paths {
        for value in decode_path(path)?.iter().flat_map(PathCommand::coordinates) {
            if !value.is_finite() {
                return Err(Error::NonFinite { path: format!("paths.{key}") });
            }
            check_magnitude(value, &format!("paths.{key}"))?;
        }
    }
    Ok(())
}

fn encode_path(path: &BezPath) -> String {
    let mut out = String::new();
    for element in path.elements() {
        let (command, points): (&str, Vec<Point>) = match element {
            PathEl::MoveTo(a) => ("M", vec![*a]),
            PathEl::LineTo(a) => ("L", vec![*a]),
            PathEl::QuadTo(a, b) => ("Q", vec![*a, *b]),
            PathEl::CurveTo(a, b, c) => ("C", vec![*a, *b, *c]),
            PathEl::ClosePath => {
                out.push_str("Z ");
                continue;
            }
        };
        out.push_str(command);
        for point in points {
            out.push(' ');
            out.push_str(&point.x.to_string());
            out.push(' ');
            out.push_str(&point.y.to_string());
        }
        out.push(' ');
    }
    out.trim_end().to_owned()
}

fn decode_path(path: &str) -> Result<Vec<PathCommand>, Error> {
    let mut tokens = path.split_whitespace();
    let mut commands = Vec::new();
    while let Some(command) = tokens.next() {
        let mut read_point = || -> Result<[f64; 2], Error> {
            let x = tokens
                .next()
                .ok_or_else(|| Error::Corrupt("truncated compact path".to_owned()))?
                .parse::<f64>()
                .map_err(|error| Error::Corrupt(error.to_string()))?;
            let y = tokens
                .next()
                .ok_or_else(|| Error::Corrupt("truncated compact path".to_owned()))?
                .parse::<f64>()
                .map_err(|error| Error::Corrupt(error.to_string()))?;
            Ok([x, y])
        };
        let parsed = match command {
            "M" => PathCommand::Move(read_point()?),
            "L" => PathCommand::Line(read_point()?),
            "Q" => PathCommand::Quad([read_point()?, read_point()?]),
            "C" => PathCommand::Curve([read_point()?, read_point()?, read_point()?]),
            "Z" => PathCommand::Close,
            _ => return Err(Error::Corrupt(format!("unknown compact path command `{command}`"))),
        };
        // kurbo requires a path to begin with MoveTo (`BezPath::push` debug-asserts it).
        if commands.is_empty() && !matches!(parsed, PathCommand::Move(_)) {
            return Err(Error::Corrupt("compact path must begin with M".to_owned()));
        }
        commands.push(parsed);
    }
    Ok(commands)
}

fn check_shape_depth(shapes: &[ShapeM], depth: usize) -> Result<(), Error> {
    if depth > MAX_GROUP_DEPTH {
        return Err(Error::LimitExceeded("group nesting exceeds 32".to_owned()));
    }
    for shape in shapes {
        if let GeomM::Group { shapes } = &shape.geom {
            check_shape_depth(shapes, depth + 1)?;
        }
    }
    Ok(())
}

fn check_json_finite(value: &serde_json::Value, path: &str) -> Result<(), Error> {
    match value {
        serde_json::Value::Number(number) if number.as_f64().is_some_and(|v| !v.is_finite()) => Err(Error::NonFinite { path: path.to_owned() }),
        serde_json::Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                check_json_finite(item, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        serde_json::Value::Object(items) => {
            for (key, item) in items {
                check_json_finite(item, &format!("{path}.{key}"))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_doc::{DeviceKind, Workspace, WorkspaceOrigin};
    use openlaser_machine::jpt_m7_200_sim;
    use openlaser_types::{FrameSource, FrameStyle, ObjectId};

    fn project() -> Project {
        let profile = jpt_m7_200_sim();
        let mut project =
            Project::new(Workspace { kind: DeviceKind::GalvoLmc, size_mm: [200.0, 200.0], origin: WorkspaceOrigin::Center }, &profile.layer_defaults);
        project.name = "roundtrip".to_owned();
        project.shapes.push(Arc::new(Shape {
            id: ObjectId(7),
            name: Some("triangle".to_owned()),
            layer: LayerKey::Color(2),
            xform: Affine::translate((1.0, 2.0)),
            geom: Geom::Path(Arc::new({
                let mut path = BezPath::new();
                path.move_to((0.0, 0.0));
                path.line_to((10.0, 0.0));
                path.line_to((5.0, 8.0));
                path.close_path();
                path
            })),
            locked: false,
            power_scale: Percent::new(100.0).unwrap(),
            priority: 0,
        }));
        project.next_id = 8;
        project.job.frame_style = FrameStyle::Bounds;
        project.job.frame_source = FrameSource::Job;
        project.normalize();
        project
    }

    fn corrupt_manifest(mut edit: impl FnMut(&mut serde_json::Value)) -> Result<Project, Error> {
        let bytes = save(&project())?;
        let mut archive = ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|error| Error::Corrupt(error.to_string()))?;
        let mut manifest: serde_json::Value = serde_json::from_reader(archive.by_name("manifest.json").map_err(|error| Error::Corrupt(error.to_string()))?)
            .map_err(|error| Error::Corrupt(error.to_string()))?;
        edit(&mut manifest);
        let mut json = Vec::new();
        serde_json::to_writer(&mut json, &manifest).map_err(|error| Error::Corrupt(error.to_string()))?;
        let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer.start_file("manifest.json", SimpleFileOptions::default()).map_err(|error| Error::Corrupt(error.to_string()))?;
        writer.write_all(&json).map_err(Error::Io)?;
        let bytes = writer.finish().map_err(|error| Error::Corrupt(error.to_string()))?.into_inner();
        load(&bytes)
    }

    #[track_caller]
    fn assert_corrupt(edit: impl FnMut(&mut serde_json::Value)) {
        let result = corrupt_manifest(edit).map(|_| ());
        assert!(matches!(result, Err(Error::Corrupt(_))), "{result:?}");
    }

    #[track_caller]
    fn assert_limit(edit: impl FnMut(&mut serde_json::Value)) {
        let result = corrupt_manifest(edit).map(|_| ());
        assert!(matches!(result, Err(Error::LimitExceeded(_))), "{result:?}");
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            writer.start_file(*name, SimpleFileOptions::default()).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn manifest_json(project: &Project) -> Vec<u8> {
        let bytes = save(project).unwrap();
        let mut archive = ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut json = Vec::new();
        archive.by_name("manifest.json").unwrap().read_to_end(&mut json).unwrap();
        json
    }

    fn with_geom(geom: Geom) -> Project {
        let mut project = project();
        let mut shape = (*project.shapes[0]).clone();
        shape.geom = geom;
        project.shapes[0] = Arc::new(shape);
        project
    }

    fn mm(v: f64) -> Mm {
        Mm::new(v).unwrap()
    }

    #[test]
    fn olp_roundtrip_equal() {
        let original = project();
        let bytes = save(&original).unwrap();
        assert_eq!(load(&bytes).unwrap(), original);
    }

    #[test]
    fn olp_bitmap_asset_roundtrip() {
        let mut original = project();
        let pixels: Arc<[u8]> = Arc::from([0_u8, 1, 127, 255]);
        let mut shape = (*original.shapes[0]).clone();
        shape.geom = Geom::Bitmap(Arc::new(BitmapData {
            asset: AssetId([0x5a; 32]),
            px: [2, 2],
            size_mm: [10.0, 10.0],
            gray: pixels.clone(),
            source_ext: "png".to_owned(),
        }));
        original.shapes[0] = Arc::new(shape);

        let bytes = save(&original).unwrap();
        let mut archive = ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let mut names = Vec::new();
        for index in 0..archive.len() {
            names.push(archive.by_index(index).unwrap().name().unwrap().into_owned());
        }
        let asset_name = names.iter().find(|name| name.starts_with("assets/")).unwrap();
        assert_eq!(asset_name, &format!("assets/{}.gray8", blake3::hash(&pixels).to_hex()));
        let mut stored = Vec::new();
        archive.by_name(asset_name).unwrap().read_to_end(&mut stored).unwrap();
        assert_eq!(stored, pixels.as_ref());
        assert_eq!(load(&bytes).unwrap(), original);
    }

    mod strategies {
        use super::*;
        use openlaser_doc::{ColorLayer, CutMode, CutOp, DeviceHint, FillParams, GalvoParams, GalvoTimings, GantryParams, ImageMode, Wobble};
        use openlaser_types::{Deg, Hz, MachineId, MmPerS, Ns, Us};
        use proptest::prelude::*;

        fn coord() -> impl Strategy<Value = f64> {
            -1000.0f64..1000.0
        }

        fn point() -> impl Strategy<Value = Point> {
            (coord(), coord()).prop_map(|(x, y)| Point::new(x, y))
        }

        fn path() -> impl Strategy<Value = BezPath> {
            let el = prop_oneof![
                point().prop_map(PathEl::MoveTo),
                point().prop_map(PathEl::LineTo),
                (point(), point()).prop_map(|(a, b)| PathEl::QuadTo(a, b)),
                (point(), point(), point()).prop_map(|(a, b, c)| PathEl::CurveTo(a, b, c)),
                Just(PathEl::ClosePath),
            ];
            (point(), proptest::collection::vec(el, 0..10)).prop_map(|(start, rest)| {
                let mut path = BezPath::new();
                path.move_to(start);
                for el in rest {
                    path.push(el);
                }
                path
            })
        }

        fn bitmap() -> impl Strategy<Value = Geom> {
            (1u32..5, 1u32..5, 0.1f64..300.0, 0.1f64..300.0, any::<[u8; 32]>(), prop_oneof![Just("png"), Just("jpg"), Just("bmp")], any::<u64>()).prop_map(
                |(w, h, sw, sh, asset, ext, seed)| {
                    let gray: Vec<u8> = (0..w * h).map(|i| seed.rotate_left(i).to_le_bytes()[0]).collect();
                    Geom::Bitmap(Arc::new(BitmapData {
                        asset: AssetId(asset),
                        px: [w, h],
                        size_mm: [sw, sh],
                        gray: Arc::from(gray),
                        source_ext: ext.to_owned(),
                    }))
                },
            )
        }

        fn leaf_geom() -> impl Strategy<Value = Geom> {
            prop_oneof![
                path().prop_map(|p| Geom::Path(Arc::new(p))),
                (-500.0f64..500.0, -500.0f64..500.0, -5.0f64..50.0).prop_map(|(w, h, r)| Geom::Rect { w: mm(w), h: mm(h), corner_radius: mm(r) }),
                (-500.0f64..500.0, -500.0f64..500.0).prop_map(|(rx, ry)| Geom::Ellipse { rx: mm(rx), ry: mm(ry) }),
                (3u16..200, -500.0f64..500.0).prop_map(|(sides, radius)| Geom::Polygon { sides, radius: mm(radius) }),
                bitmap(),
            ]
        }

        fn layer() -> impl Strategy<Value = LayerKey> {
            prop_oneof![4 => (0u8..30).prop_map(LayerKey::Color), 1 => (1u8..=2).prop_map(LayerKey::Tool)]
        }

        fn xform() -> impl Strategy<Value = Affine> {
            (-10.0f64..10.0, -10.0f64..10.0, -10.0f64..10.0, -10.0f64..10.0, -1e4f64..1e4, -1e4f64..1e4)
                .prop_map(|(a, b, c, d, e, f)| Affine::new([a, b, c, d, e, f]))
        }

        fn attrs() -> impl Strategy<Value = (Affine, LayerKey, Option<String>, bool, f64, i32)> {
            (xform(), layer(), proptest::option::of("[a-z ]{0,8}"), any::<bool>(), 0.0f64..=100.0, any::<i32>())
        }

        /// A shape tree with every id 0 (ids are assigned by `number`).
        fn shape() -> impl Strategy<Value = Shape> {
            let leaf = (attrs(), leaf_geom()).prop_map(|((xform, layer, name, locked, power, priority), geom)| Shape {
                id: ObjectId(0),
                name,
                layer,
                xform,
                geom,
                locked,
                power_scale: Percent::new(power).unwrap(),
                priority,
            });
            leaf.prop_recursive(3, 24, 4, |inner| {
                (attrs(), proptest::collection::vec(inner, 0..4)).prop_map(|((xform, layer, name, locked, power, priority), children)| Shape {
                    id: ObjectId(0),
                    name,
                    layer,
                    xform,
                    geom: Geom::Group(children.into_iter().map(Arc::new).collect()),
                    locked,
                    power_scale: Percent::new(power).unwrap(),
                    priority,
                })
            })
        }

        /// Gives the tree project-allocated ids, parents before children.
        pub fn number(shape: &Shape, project: &mut Project) -> Arc<Shape> {
            let mut out = shape.clone();
            out.id = project.alloc_id().unwrap();
            if let Geom::Group(children) = &shape.geom {
                out.geom = Geom::Group(children.iter().map(|child| number(child, project)).collect());
            }
            Arc::new(out)
        }

        fn galvo() -> impl Strategy<Value = GalvoParams> {
            let timings =
                (-1000.0f64..1000.0, -1000.0f64..1000.0, 0.0f64..1000.0, 0.0f64..1000.0, 1.0f64..10_000.0, 0.0f64..500.0, 0.0f64..5000.0, 0.0f64..50.0)
                    .prop_map(|(on, off, end, polygon, jump, min, max, dist)| GalvoTimings {
                        laser_on_tc: Us::new(on).unwrap(),
                        laser_off_tc: Us::new(off).unwrap(),
                        end_tc: Us::new(end).unwrap(),
                        polygon_tc: Us::new(polygon).unwrap(),
                        jump_speed: MmPerS::new(jump).unwrap(),
                        min_jump_delay: Us::new(min).unwrap(),
                        max_jump_delay: Us::new(max).unwrap(),
                        jump_distance_limit: Mm::new(dist).unwrap(),
                    });
            let wobble = (0.0f64..2.0, 0.0f64..2.0).prop_map(|(step, size)| Wobble { step: Mm::new(step).unwrap(), size: Mm::new(size).unwrap() });
            (
                1.0f64..10_000.0,
                0.0f64..=100.0,
                0.0f64..=100.0,
                1u32..=4_000_000,
                proptest::option::of(1.0f64..500.0),
                proptest::option::of(timings),
                proptest::option::of(wobble),
            )
                .prop_map(|(speed, power, min_power, hz, q, timings, wobble)| GalvoParams {
                    speed: MmPerS::new(speed).unwrap(),
                    power: Percent::new(power).unwrap(),
                    min_power: Percent::new(min_power).unwrap(),
                    frequency: Hz::new(hz).unwrap(),
                    q_pulse: q.map(|q| Ns::new(q).unwrap()),
                    timings,
                    wobble,
                })
        }

        fn gantry() -> impl Strategy<Value = GantryParams> {
            (1.0f64..1000.0, 0.0f64..=100.0, 0.0f64..=100.0, any::<bool>()).prop_map(|(speed, max, min, constant_power)| GantryParams {
                speed: MmPerS::new(speed).unwrap(),
                power_max: Percent::new(max).unwrap(),
                power_min: Percent::new(min).unwrap(),
                constant_power,
            })
        }

        fn fill() -> impl Strategy<Value = FillParams> {
            (0.001f64..5.0, -360.0f64..360.0, -180.0f64..180.0, any::<bool>(), any::<bool>()).prop_map(|(interval, angle, inc, bidirectional, cross_hatch)| {
                FillParams {
                    interval: Mm::new(interval).unwrap(),
                    angle: Deg::new(angle).unwrap(),
                    angle_increment: Deg::new(inc).unwrap(),
                    bidirectional,
                    cross_hatch,
                }
            })
        }

        fn op() -> impl Strategy<Value = CutOp> {
            (
                "[A-Za-z0-9 ]{0,10}",
                any::<bool>(),
                prop_oneof![Just(CutMode::Line), Just(CutMode::Fill), Just(CutMode::OffsetFill)],
                1u16..100,
                galvo(),
                gantry(),
                fill(),
            )
                .prop_map(|(name, enabled, mode, passes, galvo, gantry, fill)| CutOp { name, enabled, mode, passes, galvo, gantry, fill })
        }

        /// New settings for one colour layer (its index is kept).
        pub fn layer_settings() -> impl Strategy<Value = (u8, ColorLayer)> {
            (
                0u8..30,
                "[A-Za-z0-9 ]{0,10}",
                any::<bool>(),
                any::<bool>(),
                proptest::collection::vec(op(), 1..=3),
                1u16..50,
                (
                    prop_oneof![Just(ImageMode::Threshold), Just(ImageMode::Jarvis)],
                    0.01f64..1.0,
                    -90.0f64..90.0,
                    any::<bool>(),
                    any::<bool>(),
                    any::<u8>(),
                    1u16..9,
                    galvo(),
                    gantry(),
                ),
                any::<bool>(),
            )
                .prop_map(
                    |(
                        index,
                        name,
                        output,
                        show,
                        ops,
                        global_passes,
                        (mode, interval, angle, bidirectional, negative, threshold, passes, galvo, gantry),
                        air_assist,
                    )| {
                        let image = openlaser_doc::ImageCut {
                            mode,
                            interval: Mm::new(interval).unwrap(),
                            angle: Deg::new(angle).unwrap(),
                            bidirectional,
                            negative,
                            threshold,
                            passes,
                            galvo,
                            gantry,
                        };
                        (index, ColorLayer { index, name, output, show, ops, global_passes, image, air_assist })
                    },
                )
        }

        /// Shapes, colour-layer settings, tool-layer names, device hint, job settings and notes.
        pub type Parts = (Vec<Shape>, Vec<(u8, ColorLayer)>, [String; 2], Option<DeviceHint>, (u16, bool, FrameStyle, FrameSource), String);

        pub fn project_parts() -> impl Strategy<Value = Parts> {
            (
                proptest::collection::vec(shape(), 0..6),
                proptest::collection::vec(layer_settings(), 0..4),
                ["[A-Za-z0-9 ]{0,6}", "[A-Za-z0-9 ]{0,6}"],
                proptest::option::of(
                    ("[a-z0-9-]{1,12}", any::<bool>())
                        .prop_map(|(id, galvo)| DeviceHint { kind: if galvo { DeviceKind::GalvoLmc } else { DeviceKind::Grbl }, profile: MachineId(id) }),
                ),
                (
                    1u16..=999,
                    any::<bool>(),
                    prop_oneof![Just(FrameStyle::Bounds), Just(FrameStyle::Contour), Just(FrameStyle::Hull)],
                    prop_oneof![Just(FrameSource::Job), Just(FrameSource::ToolLayersOnly), Just(FrameSource::JobAndToolLayers)],
                ),
                ".{0,20}",
            )
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

        /// Groups, curves, parametric shapes, bitmaps, layer settings and job settings all survive save and load.
        #[test]
        fn olp_roundtrip_equal_for_random_projects((shapes, layers, tool_names, device_hint, (repeats, cut_selected, frame_style, frame_source), notes) in strategies::project_parts()) {
            let mut original = project();
            original.shapes.clear();
            original.next_id = 1;
            for shape in &shapes {
                let numbered = strategies::number(shape, &mut original);
                original.shapes.push(numbered);
            }
            for (index, layer) in layers {
                original.layers.colors[usize::from(index)] = layer;
            }
            for (tool, name) in original.layers.tools.iter_mut().zip(tool_names) {
                tool.name = name;
            }
            original.device_hint = device_hint;
            original.job = openlaser_doc::JobSettings { repeats, cut_selected, frame_style, frame_source };
            original.notes = notes;
            original.extras.insert("foreign".to_owned(), serde_json::json!({"a": [1, 2.5, "x"], "b": null}));
            original.normalize();
            let bytes = save(&original).map_err(|error| proptest::test_runner::TestCaseError::fail(error.to_string()))?;
            let restored = load(&bytes).map_err(|error| proptest::test_runner::TestCaseError::fail(error.to_string()))?;
            proptest::prop_assert_eq!(restored, original);
        }
    }

    #[test]
    fn olp_nonfinite_refused() {
        let mut invalid = project();
        let mut shape = (*invalid.shapes[0]).clone();
        shape.xform = Affine::translate((f64::NAN, 0.0));
        invalid.shapes[0] = Arc::new(shape);
        assert!(matches!(save(&invalid), Err(Error::NonFinite { .. })));
    }

    #[test]
    fn olp_too_new() {
        let mut archive = ZipWriter::new(std::io::Cursor::new(Vec::new()));
        archive.start_file("manifest.json", SimpleFileOptions::default()).unwrap();
        archive.write_all(br#"{"format":"openlaser-project","formatVersion":2}"#).unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        assert!(matches!(load(&bytes), Err(Error::TooNew { found: 2, supported: 1 })));
    }

    #[test]
    fn olp_zip_slip_refused() {
        let mut archive = ZipWriter::new(std::io::Cursor::new(Vec::new()));
        archive.start_file("../manifest.json", SimpleFileOptions::default()).unwrap();
        archive.write_all(b"{}").unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        assert!(matches!(load(&bytes), Err(Error::LimitExceeded(_))));
    }

    #[test]
    fn olp_load_rejects_invalid_project_invariants() {
        assert_corrupt(|m| m["project"]["layers"]["colors"] = serde_json::json!([]));
        assert_corrupt(|m| m["project"]["layers"]["colors"][3]["index"] = serde_json::json!(9));
        assert_corrupt(|m| m["project"]["layers"]["colors"][0]["ops"] = serde_json::json!([]));
        assert_corrupt(|m| {
            let ops = m["project"]["layers"]["colors"][0]["ops"].as_array_mut().unwrap();
            let op = ops[0].clone();
            ops.resize(12, op);
        });
        assert_corrupt(|m| m["project"]["layers"]["tools"][0]["which"] = serde_json::json!(2));
        assert_corrupt(|m| m["project"]["job"]["repeats"] = serde_json::json!(0));
        assert_corrupt(|m| m["project"]["job"]["repeats"] = serde_json::json!(65535));
        assert_corrupt(|m| {
            let duplicate = m["project"]["shapes"][0].clone();
            m["project"]["shapes"].as_array_mut().unwrap().push(duplicate);
        });
        assert_corrupt(|m| {
            let child = m["project"]["shapes"][0].clone();
            let mut second = child.clone();
            second["id"] = child["id"].clone();
            let mut group = child.clone();
            group["id"] = serde_json::json!(1);
            group["geom"] = serde_json::json!({"type":"group", "shapes":[child, second]});
            m["project"]["shapes"] = serde_json::json!([group]);
            m["project"]["nextId"] = serde_json::json!(8);
        });
        assert_corrupt(|m| m["project"]["nextId"] = serde_json::json!(0));
        assert_corrupt(|m| m["project"]["nextId"] = serde_json::json!(7));
        assert_corrupt(|m| m["project"]["workspace"]["sizeMm"] = serde_json::json!([-5.0, 0.0]));
        assert_corrupt(|m| m["project"]["layers"]["order"] = serde_json::json!([2, 2]));
        assert_corrupt(|m| m["project"]["layers"]["order"] = serde_json::json!([30]));
    }

    #[test]
    fn olp_save_refuses_nonfinite_bitmap_dimensions() {
        let mut invalid = project();
        let mut shape = (*invalid.shapes[0]).clone();
        shape.geom = Geom::Bitmap(Arc::new(BitmapData {
            asset: AssetId([1; 32]),
            px: [1, 1],
            size_mm: [f64::INFINITY, 1.0],
            gray: Arc::from([0_u8]),
            source_ext: "png".to_owned(),
        }));
        invalid.shapes[0] = Arc::new(shape);
        assert!(matches!(save(&invalid), Err(Error::NonFinite { .. })));
    }

    #[test]
    fn olp_loader_rejects_nonfinite_bitmap_dimensions() {
        let bytes = save(&project()).unwrap();
        let mut archive = ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut manifest = String::new();
        archive.by_name("manifest.json").unwrap().read_to_string(&mut manifest).unwrap();
        let json = manifest.replace("\"sizeMm\":[200.0,200.0]", "\"sizeMm\":[1e999,200.0]");
        assert_ne!(json, manifest);
        let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer.start_file("manifest.json", SimpleFileOptions::default()).unwrap();
        writer.write_all(json.as_bytes()).unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        assert!(matches!(load(&bytes), Err(Error::NonFinite { .. }) | Err(Error::Corrupt(_))));
    }

    #[test]
    fn olp_entry_and_point_limits_are_enforced() {
        assert!(check_decompressed_size(MAX_DECOMPRESSED).is_ok());
        assert!(matches!(check_decompressed_size(MAX_DECOMPRESSED + 1), Err(Error::LimitExceeded(_))));
        let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for i in 0..MAX_ENTRIES + 1 {
            writer.start_file(format!("extra/{i}"), SimpleFileOptions::default()).unwrap();
        }
        let bytes = writer.finish().unwrap().into_inner();
        assert!(matches!(load(&bytes), Err(Error::LimitExceeded(_))));
        let many_points = std::iter::repeat_n("L 0 0", MAX_POINTS + 1).collect::<Vec<_>>().join(" ");
        let paths = std::collections::BTreeMap::from([("p0".to_owned(), format!("M 0 0 {many_points}"))]);
        assert!(matches!(check_paths(&paths), Err(Error::LimitExceeded(_))));
    }

    #[test]
    fn olp_group_depth_limit_is_enforced() {
        let mut nested = ShapeM {
            id: ObjectId(1),
            name: None,
            layer: LayerKey::Color(0),
            xform: Affine::IDENTITY.as_coeffs(),
            geom: GeomM::Group { shapes: Vec::new() },
            locked: false,
            power_scale: Percent::new(100.0).unwrap(),
            priority: 0,
        };
        for _ in 0..=MAX_GROUP_DEPTH {
            nested = ShapeM {
                id: ObjectId(1),
                name: None,
                layer: LayerKey::Color(0),
                xform: Affine::IDENTITY.as_coeffs(),
                geom: GeomM::Group { shapes: vec![nested] },
                locked: false,
                power_scale: Percent::new(100.0).unwrap(),
                priority: 0,
            };
        }
        assert!(matches!(check_shape_depth(&[nested], 0), Err(Error::LimitExceeded(_))));
    }

    #[test]
    fn olp_refuses_absurd_magnitudes() {
        // A shape whose numbers are above 1e9 is refused on load (it would overflow geometry maths), and on save.
        assert_limit(|m| m["project"]["shapes"][0]["geom"] = serde_json::json!({"type": "ellipse", "rx": 1e200, "ry": 1.0}));
        assert_limit(|m| m["project"]["shapes"][0]["geom"] = serde_json::json!({"type": "ellipse", "rx": f64::MAX, "ry": f64::MIN_POSITIVE}));
        assert_limit(|m| m["project"]["shapes"][0]["geom"] = serde_json::json!({"type": "rect", "w": 2e9, "h": 1.0, "corner_radius": 0.0}));
        assert_limit(|m| m["project"]["shapes"][0]["geom"] = serde_json::json!({"type": "rect", "w": 1.0, "h": -2e9, "corner_radius": 0.0}));
        assert_limit(|m| m["project"]["shapes"][0]["geom"] = serde_json::json!({"type": "rect", "w": 1.0, "h": 1.0, "corner_radius": 2e9}));
        assert_limit(|m| m["project"]["shapes"][0]["geom"] = serde_json::json!({"type": "polygon", "sides": 5, "radius": -2e9}));
        assert_limit(|m| m["project"]["shapes"][0]["xform"] = serde_json::json!([1e300, 0.0, 0.0, 1.0, 0.0, 0.0]));
        assert_limit(|m| m["project"]["shapes"][0]["xform"] = serde_json::json!([1.0, 0.0, 0.0, 1.0, 0.0, -2e9]));
        assert_limit(|m| m["paths"]["p0"] = serde_json::json!("M 0 0 L 1e10 0"));
        assert_limit(|m| m["project"]["workspace"]["sizeMm"] = serde_json::json!([2e9, 200.0]));
        assert_limit(|m| {
            m["project"]["shapes"][0]["geom"] = serde_json::json!({"type": "bitmap", "asset": ([0_u8; 32]), "px": [1, 1], "size_mm": [2e9, 1.0], "gray_asset": "assets/x.gray8", "source_ext": "png"});
        });
        // The limit itself is accepted.
        let edge = corrupt_manifest(|m| {
            m["project"]["shapes"][0]["geom"] = serde_json::json!({"type": "ellipse", "rx": 1e9, "ry": -1e9});
            m["project"]["shapes"][0]["xform"] = serde_json::json!([1e9, 0.0, 0.0, 1e9, -1e9, 1e9]);
            m["paths"] = serde_json::json!({});
        });
        assert!(edge.is_ok(), "{:?}", edge.map(|_| ()));
        assert!(matches!(save(&with_geom(Geom::Ellipse { rx: mm(1e10), ry: mm(1.0) })), Err(Error::LimitExceeded(_))));
        let mut far = project();
        let mut shape = (*far.shapes[0]).clone();
        shape.xform = Affine::scale(1e300);
        far.shapes[0] = Arc::new(shape);
        assert!(matches!(save(&far), Err(Error::LimitExceeded(_))));
    }

    #[test]
    fn olp_counts_closes_against_the_point_limit() {
        // ~62 KB compressed, 31 M `Z` decompressed: every Z costs as much as a point.
        let closes = " Z".repeat(MAX_POINTS);
        assert_limit(|m| m["paths"]["p0"] = serde_json::json!(format!("M 0 0 L 1 0{closes}")));
        let at_limit = std::collections::BTreeMap::from([("p0".to_owned(), format!("M 0 0{}", " Z".repeat(MAX_POINTS - 1)))]);
        assert!(check_paths(&at_limit).is_ok());
        let over = std::collections::BTreeMap::from([("p0".to_owned(), format!("M 0 0{}", " Z".repeat(MAX_POINTS)))]);
        assert!(matches!(check_paths(&over), Err(Error::LimitExceeded(_))));
    }

    #[test]
    fn olp_quads_and_curves_weigh_their_points() {
        let weigh = |segment: &str, n: usize| check_paths(&std::collections::BTreeMap::from([("p0".to_owned(), format!("M 0 0{}", segment.repeat(n)))]));
        // M + 2 per Q: 499 999 quads fit, 500 000 (1 000 001 points) do not.
        assert!(weigh(" Q 1 1 2 0", (MAX_POINTS - 1) / 2).is_ok());
        assert!(matches!(weigh(" Q 1 1 2 0", MAX_POINTS / 2), Err(Error::LimitExceeded(_))));
        // M + 3 per C: 333 333 curves are exactly 1 000 000 points, one more is over.
        assert!(weigh(" C 1 1 2 1 3 0", MAX_POINTS / 3).is_ok());
        assert!(matches!(weigh(" C 1 1 2 1 3 0", MAX_POINTS / 3 + 1), Err(Error::LimitExceeded(_))));
    }

    #[test]
    fn olp_malformed_path_is_corrupt() {
        for bad in ["M 0", "M 0 0 X 1 2", "M a b", "M 0 0 Q 1 2 3", "L 1 2", "Z", "M 0 0 C 1 1 2 2"] {
            let result = corrupt_manifest(|m| m["paths"]["p0"] = serde_json::json!(bad)).map(|_| ());
            assert!(matches!(result, Err(Error::Corrupt(_))), "{bad}: {result:?}");
        }
    }

    #[test]
    fn olp_next_id_must_stay_within_2_pow_53() {
        assert_corrupt(|m| m["project"]["nextId"] = serde_json::json!(u64::MAX));
        assert_corrupt(|m| m["project"]["nextId"] = serde_json::json!(openlaser_doc::MAX_NEXT_ID + 1));
        let loaded = corrupt_manifest(|m| m["project"]["nextId"] = serde_json::json!(openlaser_doc::MAX_NEXT_ID));
        assert_eq!(loaded.map(|p| p.next_id).ok(), Some(openlaser_doc::MAX_NEXT_ID));
        let mut far = project();
        far.next_id = u64::MAX;
        assert!(matches!(save(&far), Err(Error::Corrupt(_))));
    }

    #[test]
    fn olp_layer_order_must_list_exactly_the_used_layers() {
        assert_corrupt(|m| m["project"]["layers"]["order"] = serde_json::json!([]));
        assert_corrupt(|m| m["project"]["layers"]["order"] = serde_json::json!([2, 5]));
        assert_corrupt(|m| m["project"]["layers"]["order"] = serde_json::json!(["T1", 2]));
        // Any order of the used layers is the user's output order and round-trips.
        let mut two = project();
        let mut shape = (*two.shapes[0]).clone();
        shape.id = ObjectId(8);
        shape.layer = LayerKey::Tool(1);
        two.shapes.push(Arc::new(shape));
        two.next_id = 9;
        two.layers.order = vec![LayerKey::Tool(1), LayerKey::Color(2)];
        assert_eq!(load(&save(&two).unwrap()).unwrap(), two);
        // Save refuses a project whose order is stale, rather than writing a file load would refuse.
        two.layers.order = vec![LayerKey::Color(2)];
        assert!(matches!(save(&two), Err(Error::Corrupt(_))));
    }

    #[test]
    fn olp_refuses_degenerate_polygons_and_bitmaps() {
        for sides in [0, 1, 2] {
            assert_corrupt(|m| m["project"]["shapes"][0]["geom"] = serde_json::json!({"type": "polygon", "sides": sides, "radius": 5.0}));
        }
        let bitmap = |px: [u32; 2], size: [f64; 2]| serde_json::json!({"type": "bitmap", "asset": ([0_u8; 32]), "px": px, "size_mm": size, "gray_asset": "assets/x.gray8", "source_ext": "png"});
        assert_corrupt(|m| m["project"]["shapes"][0]["geom"] = bitmap([0, 0], [10.0, 10.0]));
        assert_corrupt(|m| m["project"]["shapes"][0]["geom"] = bitmap([0, 4], [10.0, 10.0]));
        assert_corrupt(|m| m["project"]["shapes"][0]["geom"] = bitmap([2, 2], [0.0, 10.0]));
        assert_corrupt(|m| m["project"]["shapes"][0]["geom"] = bitmap([2, 2], [10.0, -1.0]));
        let image = |px: [u32; 2], size_mm: [f64; 2], gray: &[u8]| {
            with_geom(Geom::Bitmap(Arc::new(BitmapData { asset: AssetId([1; 32]), px, size_mm, gray: Arc::from(gray), source_ext: "png".to_owned() })))
        };
        assert!(matches!(save(&image([0, 0], [10.0, 10.0], &[])), Err(Error::Corrupt(_))));
        assert!(matches!(save(&image([1, 1], [0.0, 10.0], &[7])), Err(Error::Corrupt(_))));
        assert!(matches!(save(&with_geom(Geom::Polygon { sides: 0, radius: mm(5.0) })), Err(Error::Corrupt(_))));
        assert!(load(&save(&image([1, 1], [1e-3, 10.0], &[7])).unwrap()).is_ok());
    }

    #[test]
    fn olp_rejects_single_field_invariant_breaks() {
        // Each of these used to load.
        assert_corrupt(|m| m["project"]["layers"]["tools"][1]["which"] = serde_json::json!(1));
        assert_corrupt(|m| m["project"]["workspace"]["sizeMm"] = serde_json::json!([0.0, 200.0]));
        assert_corrupt(|m| m["project"]["workspace"]["sizeMm"] = serde_json::json!([200.0, 0.0]));
        assert_corrupt(|m| m["project"]["shapes"][0]["id"] = serde_json::json!(0));
    }

    #[test]
    fn olp_refuses_duplicate_zip_entries() {
        // Two manifests: the zip reader would keep one of them silently.
        let first = manifest_json(&project());
        let mut other = project();
        other.name = "the other one".to_owned();
        let second = manifest_json(&other);
        let mut bytes = zip_of(&[("manifest.json", &first), ("manifest.jsoX", &second)]);
        let (from, to) = (b"manifest.jsoX".as_slice(), b"manifest.json".as_slice());
        let mut patched = 0;
        for i in 0..bytes.len() - from.len() {
            if &bytes[i..i + from.len()] == from {
                bytes[i..i + to.len()].copy_from_slice(to);
                patched += 1;
            }
        }
        assert_eq!(patched, 2, "local header and central directory");
        let result = load(&bytes).map(|_| ());
        assert!(matches!(result, Err(Error::Corrupt(_))), "{result:?}");
    }

    #[test]
    fn olp_refuses_unexpected_and_unreferenced_content() {
        let json = manifest_json(&project());
        assert!(load(&zip_of(&[("manifest.json", &json)])).is_ok());
        let result = load(&zip_of(&[("manifest.json", &json), ("notes.txt", b"hello")])).map(|_| ());
        assert!(matches!(result, Err(Error::Corrupt(_))), "{result:?}");
        let gray = [1_u8, 2, 3];
        let name = format!("assets/{}.gray8", blake3::hash(&gray).to_hex());
        let result = load(&zip_of(&[("manifest.json", &json), (&name, &gray)])).map(|_| ());
        assert!(matches!(result, Err(Error::Corrupt(_))), "{result:?}");
        assert_corrupt(|m| m["paths"]["p9"] = serde_json::json!("M 0 0 L 1 1"));
    }
}
