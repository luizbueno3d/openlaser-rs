//! The project, its shapes and settings.

use std::collections::BTreeMap;
use std::sync::Arc;

use kurbo::{Affine, BezPath};
use openlaser_geom::Polyline;
use openlaser_types::{Bounds, FrameSource, FrameStyle, LayerKey, MachineId, Mm, ObjectId, Percent};
use serde::{Deserialize, Serialize};

use crate::layers::{ColorLayer, CutTable, LayerDefaults, ToolLayer};

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
    /// 30 colour layers + T1/T2, next_id = 1. WP0 stub: T1/T2 only; WP1 adds the colour layers from `defaults`.
    pub fn new(workspace: Workspace, defaults: &LayerDefaults) -> Self {
        let _ = defaults;
        let tool = |which: u8| ToolLayer { which, name: format!("T{which}"), show: true };
        Project {
            name: String::new(),
            shapes: Vec::new(),
            layers: CutTable { colors: Vec::new(), tools: [tool(1), tool(2)], order: Vec::new() },
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
        self.shapes.iter().find(|s| s.id == id)
    }

    pub fn color_layer(&self, index: u8) -> Option<&ColorLayer> {
        self.layers.colors.get(usize::from(index))
    }

    pub fn color_layer_mut(&mut self, index: u8) -> Option<&mut ColorLayer> {
        self.layers.colors.get_mut(usize::from(index))
    }

    pub fn used_layers(&self) -> Vec<LayerKey> {
        Vec::new()
    }

    /// Flattened world polylines (design mm) of one shape, groups expanded; bitmaps give their outline rectangle.
    pub fn world_polylines(&self, id: ObjectId, tol_mm: f64) -> Vec<Polyline> {
        let _ = (id, tol_mm);
        Vec::new()
    }

    pub fn bounds(&self, ids: &[ObjectId]) -> Option<Bounds> {
        let _ = ids;
        None
    }

    /// Keeps `layers.order` consistent with the shapes (appends newly used layers). Call after every edit.
    pub fn normalize(&mut self) {}
}

impl Geom {
    /// Parametric shapes stay parametric until Convert to Path. WP0 stub: an empty path.
    pub fn to_bez(&self) -> BezPath {
        BezPath::new()
    }
}

/// "C00".."C29", "T1", "T2".
pub fn default_layer_name(k: LayerKey) -> String {
    k.to_string()
}
