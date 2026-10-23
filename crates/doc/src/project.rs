//! The project, its shapes and settings.

use std::collections::BTreeMap;
use std::sync::Arc;

use kurbo::{Affine, BezPath, Ellipse, Point, Rect, RoundedRect, Shape as KurboShape};
use openlaser_geom::{DEFAULT_FLATTEN_TOL_MM, Polyline, bounds_of, flatten};
use openlaser_types::{Bounds, FrameSource, FrameStyle, LayerKey, MachineId, Mm, ObjectId, Percent};
use serde::{Deserialize, Serialize};

use crate::COLOR_LAYERS;
use crate::layers::{ColorLayer, CutMode, CutOp, CutTable, ImageCut, ImageMode, LayerDefaults, ToolLayer};

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub name: String,
    /// Draw order: index 0 first (bottom).
    pub shapes: Vec<Arc<Shape>>,
    pub layers: CutTable,
    pub workspace: Workspace,
    pub device_hint: Option<DeviceHint>,
    pub job: JobSettings,
    pub notes: String,
    /// Per project: ids are deterministic.
    pub next_id: u64,
    /// Unknown foreign keys, kept for round trips.
    pub extras: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub id: ObjectId,
    pub name: Option<String>,
    pub layer: LayerKey,
    /// Object space → design mm.
    pub xform: Affine,
    pub geom: Geom,
    pub locked: bool,
    /// P1 behaviour; stored now.
    pub power_scale: Percent,
    /// P1 behaviour; stored now.
    pub priority: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Geom {
    /// Object-space mm.
    Path(Arc<BezPath>),
    Rect {
        w: Mm,
        h: Mm,
        corner_radius: Mm,
    },
    Ellipse {
        rx: Mm,
        ry: Mm,
    },
    Polygon {
        sides: u16,
        radius: Mm,
    },
    Bitmap(Arc<BitmapData>),
    Group(Vec<Arc<Shape>>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct BitmapData {
    pub asset: AssetId,
    pub px: [u32; 2],
    pub size_mm: [f64; 2],
    pub gray: Arc<[u8]>,
    pub source_ext: String,
}

/// blake3 of the original file bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssetId(pub [u8; 32]);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub kind: DeviceKind,
    pub size_mm: [f64; 2],
    pub origin: WorkspaceOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceOrigin {
    Center,
    BottomLeft,
    TopLeft,
}

/// Defined in types (§3.1) so driver can name it without a doc edge.
pub use openlaser_types::DeviceKind;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceHint {
    pub kind: DeviceKind,
    pub profile: MachineId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSettings {
    /// 1..=999.
    pub repeats: u16,
    pub cut_selected: bool,
    pub frame_style: FrameStyle,
    pub frame_source: FrameSource,
}

/// key = last selected (align reference).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub ids: Vec<ObjectId>,
    pub key: Option<ObjectId>,
}

impl Project {
    /// 30 colour layers + T1/T2, next_id = 1.
    pub fn new(workspace: Workspace, defaults: &LayerDefaults) -> Self {
        let tool = |which: u8| ToolLayer { which, name: format!("T{which}"), show: true };
        let colors = (0..COLOR_LAYERS).filter_map(|index| u8::try_from(index).ok()).map(|index| color_layer(index, defaults)).collect();
        Project {
            name: String::new(),
            shapes: Vec::new(),
            layers: CutTable { colors, tools: [tool(1), tool(2)], order: Vec::new() },
            workspace,
            device_hint: None,
            job: JobSettings { repeats: 1, cut_selected: false, frame_style: FrameStyle::Contour, frame_source: FrameSource::Job },
            notes: String::new(),
            next_id: 1,
            extras: BTreeMap::new(),
        }
    }

    pub fn alloc_id(&mut self) -> ObjectId {
        let id = ObjectId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    pub fn shape(&self, id: ObjectId) -> Option<&Arc<Shape>> {
        find_shape(&self.shapes, id)
    }

    pub fn color_layer(&self, index: u8) -> Option<&ColorLayer> {
        self.layers.colors.get(usize::from(index))
    }

    pub fn color_layer_mut(&mut self, index: u8) -> Option<&mut ColorLayer> {
        self.layers.colors.get_mut(usize::from(index))
    }

    pub fn used_layers(&self) -> Vec<LayerKey> {
        let mut used = Vec::new();
        for shape in &self.shapes {
            collect_layers(shape, &mut used);
        }
        used
    }

    /// Flattened world polylines (design mm) of one shape, groups expanded; bitmaps give their outline rectangle.
    pub fn world_polylines(&self, id: ObjectId, tol_mm: f64) -> Result<Vec<Polyline>, openlaser_geom::Error> {
        let Some(shape) = find_shape(&self.shapes, id) else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        append_world_polylines(shape, Affine::IDENTITY, tol_mm, &mut out)?;
        Ok(out)
    }

    pub fn bounds(&self, ids: &[ObjectId]) -> Result<Option<Bounds>, openlaser_geom::Error> {
        let polys: Vec<Polyline> =
            ids.iter().map(|id| self.world_polylines(*id, DEFAULT_FLATTEN_TOL_MM)).collect::<Result<Vec<_>, _>>()?.into_iter().flatten().collect();
        Ok(bounds_of(&polys))
    }

    /// Removes unused layer entries, then appends newly used layers in draw traversal order. Groups do not use
    /// their own layer; only non-group shapes contribute. Call after every edit.
    pub fn normalize(&mut self) {
        let used = self.used_layers();
        self.layers.order.retain(|key| used.contains(key));
        for key in used {
            if !self.layers.order.contains(&key) {
                self.layers.order.push(key);
            }
        }
    }
}

impl Geom {
    /// Parametric shapes stay parametric until Convert to Path.
    pub fn to_bez(&self) -> BezPath {
        match self {
            Geom::Path(path) => path.as_ref().clone(),
            Geom::Rect { w, h, corner_radius } => {
                let (hw, hh) = (w.get() / 2.0, h.get() / 2.0);
                let rect = Rect::new(-hw, -hh, hw, hh).abs();
                let radius = corner_radius.get().max(0.0).min(rect.width().abs() / 2.0).min(rect.height().abs() / 2.0);
                RoundedRect::from_rect(rect, radius).to_path(curve_tolerance(rect.width().abs().max(rect.height().abs())))
            }
            Geom::Ellipse { rx, ry } => {
                let rx = rx.get().abs();
                let ry = ry.get().abs();
                Ellipse::new(Point::ORIGIN, (rx, ry), 0.0).to_path(curve_tolerance(rx.max(ry)))
            }
            Geom::Polygon { sides, radius } => polygon_path(*sides, radius.get()),
            Geom::Bitmap(bitmap) => {
                let [w, h] = bitmap.size_mm;
                let rect = Rect::new(-w / 2.0, -h / 2.0, w / 2.0, h / 2.0).abs();
                rect.to_path(curve_tolerance(rect.width().abs().max(rect.height().abs())))
            }
            Geom::Group(_) => BezPath::new(),
        }
    }
}

fn curve_tolerance(size: f64) -> f64 {
    DEFAULT_FLATTEN_TOL_MM.max(if size.is_finite() { size * 1e-9 } else { DEFAULT_FLATTEN_TOL_MM })
}

/// "C00".."C29", "T1", "T2".
pub fn default_layer_name(k: LayerKey) -> String {
    k.to_string()
}

fn color_layer(index: u8, defaults: &LayerDefaults) -> ColorLayer {
    let op = CutOp {
        name: default_layer_name(LayerKey::Color(index)),
        enabled: true,
        mode: CutMode::Line,
        passes: 1,
        galvo: defaults.galvo.clone(),
        gantry: defaults.gantry.clone(),
        fill: defaults.fill.clone(),
    };
    ColorLayer {
        index,
        name: default_layer_name(LayerKey::Color(index)),
        output: true,
        show: true,
        ops: vec![op],
        global_passes: 1,
        image: ImageCut {
            mode: ImageMode::Threshold,
            interval: defaults.fill.interval,
            angle: defaults.fill.angle,
            bidirectional: defaults.fill.bidirectional,
            negative: false,
            threshold: 128,
            passes: 1,
            galvo: defaults.galvo.clone(),
            gantry: defaults.gantry.clone(),
        },
        air_assist: false,
    }
}

fn collect_layers(shape: &Shape, out: &mut Vec<LayerKey>) {
    if let Geom::Group(children) = &shape.geom {
        for child in children {
            collect_layers(child, out);
        }
    } else if !out.contains(&shape.layer) {
        out.push(shape.layer);
    }
}

fn find_shape(shapes: &[Arc<Shape>], id: ObjectId) -> Option<&Arc<Shape>> {
    for shape in shapes {
        if shape.id == id {
            return Some(shape);
        }
        if let Geom::Group(children) = &shape.geom
            && let Some(found) = find_shape(children, id)
        {
            return Some(found);
        }
    }
    None
}

fn append_world_polylines(shape: &Shape, parent: Affine, tol_mm: f64, out: &mut Vec<Polyline>) -> Result<(), openlaser_geom::Error> {
    let xform = parent * shape.xform;
    match &shape.geom {
        Geom::Group(children) => {
            for child in children {
                append_world_polylines(child, xform, tol_mm, out)?;
            }
        }
        geom => out.extend(flatten(&geom.to_bez(), tol_mm)?.into_iter().map(|poly| poly.transformed(&xform))),
    }
    Ok(())
}

fn polygon_path(sides: u16, radius: f64) -> BezPath {
    let mut path = BezPath::new();
    if sides < 3 || !radius.is_finite() {
        return path;
    }
    let radius = radius.abs();
    for i in 0..sides {
        let angle = std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * f64::from(i) / f64::from(sides);
        let point = Point::new(radius * angle.cos(), radius * angle.sin());
        if i == 0 {
            path.move_to(point);
        } else {
            path.line_to(point);
        }
    }
    path.close_path();
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_types::{Deg, Hz, MmPerS, Ns};

    fn defaults() -> LayerDefaults {
        LayerDefaults {
            galvo: crate::GalvoParams {
                speed: MmPerS::new(1000.0).unwrap(),
                power: Percent::new(30.0).unwrap(),
                min_power: Percent::new(0.0).unwrap(),
                frequency: Hz::new(30_000).unwrap(),
                q_pulse: Some(Ns::new(200.0).unwrap()),
                timings: None,
                wobble: None,
            },
            gantry: crate::GantryParams {
                speed: MmPerS::new(100.0).unwrap(),
                power_max: Percent::new(20.0).unwrap(),
                power_min: Percent::new(0.0).unwrap(),
                constant_power: false,
            },
            fill: crate::FillParams {
                interval: Mm::new(0.1).unwrap(),
                angle: Deg::new(0.0).unwrap(),
                angle_increment: Deg::new(90.0).unwrap(),
                bidirectional: true,
                cross_hatch: false,
            },
        }
    }

    fn workspace() -> Workspace {
        Workspace { kind: DeviceKind::GalvoLmc, size_mm: [200.0, 200.0], origin: WorkspaceOrigin::Center }
    }

    #[test]
    fn new_has_all_layers_and_independent_defaults() {
        let mut project = Project::new(workspace(), &defaults());
        assert_eq!(project.layers.colors.len(), 30);
        assert_eq!(project.layers.tools[0].name, "T1");
        assert_eq!(project.next_id, 1);
        project.layers.colors[0].ops[0].galvo.power = Percent::new(70.0).unwrap();
        assert_eq!(project.layers.colors[1].ops[0].galvo.power.get(), 30.0);
    }

    #[test]
    fn world_polylines_expand_groups_and_normalize_layers() {
        let mut project = Project::new(workspace(), &defaults());
        let child = Arc::new(Shape {
            id: ObjectId(2),
            name: None,
            layer: LayerKey::Color(2),
            xform: Affine::translate((5.0, 0.0)),
            geom: Geom::Rect { w: Mm::new(2.0).unwrap(), h: Mm::new(4.0).unwrap(), corner_radius: Mm::new(0.0).unwrap() },
            locked: false,
            power_scale: Percent::new(100.0).unwrap(),
            priority: 0,
        });
        project.shapes.push(Arc::new(Shape {
            id: ObjectId(1),
            name: None,
            layer: LayerKey::Tool(1),
            xform: Affine::translate((10.0, 20.0)),
            geom: Geom::Group(vec![child]),
            locked: false,
            power_scale: Percent::new(100.0).unwrap(),
            priority: 0,
        }));
        project.normalize();
        assert_eq!(project.layers.order, vec![LayerKey::Color(2)]);
        let bounds = project.bounds(&[ObjectId(1)]).unwrap().unwrap();
        assert_eq!(bounds.min, [14.0, 18.0]);
        assert_eq!(bounds.max, [16.0, 22.0]);
        assert!(!project.world_polylines(ObjectId(2), 0.01).unwrap().is_empty());
        assert!(project.shape(ObjectId(2)).is_some());
    }

    #[test]
    fn enormous_ellipse_path_has_bounded_element_count() {
        let ellipse = Geom::Ellipse { rx: Mm::new(1e200).unwrap(), ry: Mm::new(1e200).unwrap() };
        let path = ellipse.to_bez();
        assert!(path.elements().len() <= 32, "unexpected {} ellipse path elements", path.elements().len());
    }

    #[test]
    fn group_layer_is_not_counted_as_used() {
        let mut project = Project::new(workspace(), &defaults());
        let child = Arc::new(Shape {
            id: ObjectId(2),
            name: None,
            layer: LayerKey::Color(4),
            xform: Affine::IDENTITY,
            geom: Geom::Rect { w: Mm::new(1.0).unwrap(), h: Mm::new(1.0).unwrap(), corner_radius: Mm::new(0.0).unwrap() },
            locked: false,
            power_scale: Percent::new(100.0).unwrap(),
            priority: 0,
        });
        project.shapes.push(Arc::new(Shape {
            id: ObjectId(1),
            name: None,
            layer: LayerKey::Color(9),
            xform: Affine::IDENTITY,
            geom: Geom::Group(vec![child]),
            locked: false,
            power_scale: Percent::new(100.0).unwrap(),
            priority: 0,
        }));
        assert_eq!(project.used_layers(), vec![LayerKey::Color(4)]);
    }
}
