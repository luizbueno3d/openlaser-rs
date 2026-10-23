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
/// assets/<blake3>.gray8 (raw row-major grayscale pixels). ProjectM is a separate serde model converted
/// with exhaustive struct literals both ways, so a new doc field fails to compile until it is saved.
pub fn save(project: &Project) -> Result<Vec<u8>, Error> {
    let mut paths = std::collections::BTreeMap::new();
    let mut assets = std::collections::BTreeMap::new();
    let mut next_path = 0usize;
    let model = ProjectM::from_project(project, &mut paths, &mut assets, &mut next_path, 0)?;
    if project.workspace.size_mm.iter().any(|v| !v.is_finite()) {
        return Err(Error::NonFinite { path: "project.workspace.sizeMm".to_owned() });
    }
    check_paths(&paths)?;
    let manifest = Manifest { format: FORMAT.to_owned(), format_version: FORMAT_VERSION, generator: "OpenLaser".to_owned(), project: model, paths };
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
    let mut manifest_bytes = Vec::new();
    let mut assets = std::collections::BTreeMap::new();
    let mut total = 0usize;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|error| Error::Corrupt(error.to_string()))?;
        let name = file.name().map_err(|error| Error::Corrupt(error.to_string()))?.into_owned();
        if name.starts_with('/') || name.split('/').any(|part| part == "..") || name.contains('\\') {
            return Err(Error::LimitExceeded(format!("unsafe zip entry name: {name}")));
        }
        let size = usize::try_from(file.size()).map_err(|_| Error::LimitExceeded("zip entry is too large".to_owned()))?;
        total = total.checked_add(size).ok_or_else(|| Error::LimitExceeded("decompressed size overflow".to_owned()))?;
        if total > MAX_DECOMPRESSED {
            return Err(Error::LimitExceeded("decompressed project exceeds 64 MiB".to_owned()));
        }
        if name == "manifest.json" {
            file.read_to_end(&mut manifest_bytes)?;
        } else if name.starts_with("assets/") {
            let mut asset_bytes = Vec::with_capacity(size);
            file.read_to_end(&mut asset_bytes)?;
            if assets.insert(name.clone(), asset_bytes).is_some() {
                return Err(Error::Corrupt(format!("duplicate asset entry {name}")));
            }
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
    check_paths(&manifest.paths)?;
    check_shape_depth(&manifest.project.shapes, 0)?;
    if manifest.project.workspace.size_mm.iter().any(|v| !v.is_finite()) {
        return Err(Error::NonFinite { path: "project.workspace.sizeMm".to_owned() });
    }
    manifest.project.into_project(&manifest.paths, &assets, 0)
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
const MAX_POINTS: usize = 1_000_000;
const MAX_GROUP_DEPTH: usize = 32;

fn bitmap_pixel_count(px: [u32; 2]) -> Result<usize, Error> {
    usize::try_from(px[0])
        .ok()
        .and_then(|width| usize::try_from(px[1]).ok().and_then(|height| width.checked_mul(height)))
        .ok_or_else(|| Error::LimitExceeded("bitmap pixel dimensions overflow".to_owned()))
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
    let points = paths
        .values()
        .try_fold(0usize, |count, path| {
            let commands = decode_path(path).ok()?;
            let added = commands.iter().filter(|command| !matches!(command, PathCommand::Close)).count();
            count.checked_add(added)
        })
        .ok_or_else(|| Error::LimitExceeded("point count overflow".to_owned()))?;
    if points > MAX_POINTS {
        return Err(Error::LimitExceeded("project exceeds 1 M points".to_owned()));
    }
    for (key, path) in paths {
        let commands = decode_path(path)?;
        if commands.iter().flat_map(PathCommand::coordinates).any(|value| !value.is_finite()) {
            return Err(Error::NonFinite { path: format!("paths.{key}") });
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
        project.job.frame_style = FrameStyle::Bounds;
        project.job.frame_source = FrameSource::Job;
        project.normalize();
        project
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

    proptest::proptest! {
        #[test]
        fn olp_roundtrip_equal_for_finite_transforms(x in -10_000.0f64..10_000.0, y in -10_000.0f64..10_000.0) {
            let mut original = project();
            let mut shape = (*original.shapes[0]).clone();
            shape.xform = Affine::translate((x, y));
            original.shapes[0] = Arc::new(shape);
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
}
