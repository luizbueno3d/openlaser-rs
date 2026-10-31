#![allow(dead_code, clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use openlaser_doc::{AssetId, BitmapData, ColorLayer, Geom, Project, Shape, Workspace, WorkspaceOrigin};
use openlaser_geom::Affine;
use openlaser_machine::MachineProfile;
use openlaser_plan::{JobPlan, PlanOptions};
use openlaser_types::{FrameSource, FrameStyle, LayerKey, Mm, ObjectId, Percent};
use std::sync::Arc;

pub fn project(m: &MachineProfile) -> Project {
    let mut p = Project::new(Workspace { kind: openlaser_plan::device_of(m), size_mm: [200.0, 200.0], origin: WorkspaceOrigin::Center }, &m.layer_defaults);
    p.shapes.push(Arc::new(Shape {
        id: ObjectId(1),
        name: None,
        layer: LayerKey::Color(0),
        xform: Affine::IDENTITY,
        geom: Geom::Bitmap(Arc::new(BitmapData {
            asset: AssetId([0; 32]),
            px: [4, 2],
            size_mm: [4.0, 2.0],
            gray: Arc::from([0, 0, 255, 0, 255, 255, 255, 0]),
            source_ext: "png".into(),
        })),
        locked: false,
        power_scale: Percent::new(100.0).unwrap(),
        priority: 0,
    }));
    p.next_id = 2;
    layer(&mut p).image.interval = Mm::new(1.0).unwrap();
    layer(&mut p).image.bidirectional = false;
    p
}
pub fn layer(p: &mut Project) -> &mut ColorLayer {
    p.color_layer_mut(0).unwrap()
}
pub fn bitmap(p: &mut Project) -> &mut BitmapData {
    let shape = Arc::make_mut(&mut p.shapes[0]);
    let Geom::Bitmap(b) = &mut shape.geom else { panic!("bitmap fixture") };
    Arc::make_mut(b)
}
pub fn plan(p: &Project, m: &MachineProfile) -> JobPlan {
    openlaser_plan::plan(
        p,
        m,
        &PlanOptions { selected: None, repeats: 1, frame_style: FrameStyle::Contour, frame_source: FrameSource::Job, flatten_tol_mm: 0.01 },
    )
    .unwrap()
}
