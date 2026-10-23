//! Display-list builders (SPEC §3.10): visibility, selection, dark mode, marker placement, travel and order.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;
use std::time::Duration;

use openlaser_doc::{ColorLayer, DeviceKind, Geom, Project, Shape, Workspace, WorkspaceOrigin};
use openlaser_geom::{Affine, Polyline};
use openlaser_plan::{CutQ, GantryCutQ, JobPlan, PlanFacts, Section};
use openlaser_render::{
    DisplayList, FRAME_COLOR, Item, MarkerKind, NOTICE_COLOR, PlanShow, REFUSAL_COLOR, Rgba, SELECTION_COLOR, Stroke, TRAVEL_COLOR, ViewOpts, document_list,
    plan_list,
};
use openlaser_types::{Fingerprint, LayerKey, MachineId, Mm, MmPerS, Notice, NoticeCode, ObjectId, Percent, Refusal, RefusalCode, SectionMode};

fn color_layer(index: u8, show: bool) -> ColorLayer {
    let gantry = serde_json::json!({"speed": 100.0, "powerMax": 30.0, "powerMin": 0.0, "constantPower": false});
    let galvo = serde_json::json!({"speed": 1000.0, "power": 30.0, "minPower": 0.0, "frequency": 30000, "qPulse": 200.0, "timings": null, "wobble": null});
    serde_json::from_value(serde_json::json!({
        "index": index, "name": format!("C{index:02}"), "output": true, "show": show,
        "ops": [{"name": "op", "enabled": true, "mode": "line", "passes": 1, "galvo": galvo, "gantry": gantry,
            "fill": {"interval": 0.1, "angle": 0.0, "angleIncrement": 90.0, "bidirectional": true, "crossHatch": false}}],
        "globalPasses": 1,
        "image": {"mode": "threshold", "interval": 0.1, "angle": 0.0, "bidirectional": true, "negative": false, "threshold": 128, "passes": 1, "galvo": galvo, "gantry": gantry},
        "airAssist": false
    }))
    .unwrap()
}

fn rect(id: u64, layer: LayerKey, at: [f64; 2], size: f64) -> Arc<Shape> {
    Arc::new(Shape {
        id: ObjectId(id),
        name: None,
        layer,
        xform: Affine::translate((at[0], at[1])),
        geom: Geom::Rect { w: Mm::new(size).unwrap(), h: Mm::new(size).unwrap(), corner_radius: Mm::new(0.0).unwrap() },
        locked: false,
        power_scale: Percent::new(100.0).unwrap(),
        priority: 0,
    })
}

/// C00 shown with shapes 1 (at 0,0) and 3 (a group holding a rect at 20,0); C01 hidden with shape 2.
fn project() -> Project {
    let profile = openlaser_machine::jpt_m7_200();
    let mut p = Project::new(Workspace { kind: DeviceKind::Grbl, size_mm: [400.0, 400.0], origin: WorkspaceOrigin::Center }, &profile.layer_defaults);
    p.layers.colors = vec![color_layer(0, true), color_layer(1, false)];
    let group = Shape {
        id: ObjectId(3),
        name: None,
        layer: LayerKey::Color(0),
        xform: Affine::translate((20.0, 0.0)),
        geom: Geom::Group(vec![rect(30, LayerKey::Color(0), [0.0, 0.0], 4.0)]),
        locked: false,
        power_scale: Percent::new(100.0).unwrap(),
        priority: 0,
    };
    p.shapes = vec![rect(1, LayerKey::Color(0), [0.0, 0.0], 10.0), rect(2, LayerKey::Color(1), [50.0, 50.0], 10.0), Arc::new(group)];
    p
}

fn opts() -> ViewOpts {
    ViewOpts { dark: false, selection: Vec::new(), refusals: Vec::new(), notices: Vec::new(), show_field: false, show_work_area: false }
}

fn polylines(list: &DisplayList) -> Vec<(Stroke, Rgba, Option<ObjectId>, Vec<Polyline>)> {
    list.items
        .iter()
        .filter_map(|item| match item {
            Item::Polylines { stroke, color, lines, object } => Some((*stroke, *color, *object, lines.to_vec())),
            _ => None,
        })
        .collect()
}

fn markers(list: &DisplayList) -> Vec<(MarkerKind, [f64; 2], Rgba)> {
    list.items
        .iter()
        .filter_map(|item| match item {
            Item::Marker { kind, at, color } => Some((*kind, *at, *color)),
            _ => None,
        })
        .collect()
}

#[test]
fn document_list_skips_hidden_layers_and_expands_groups() {
    let list = document_list(&project(), None, &opts());
    let drawn = polylines(&list);
    let objects: Vec<_> = drawn.iter().map(|(_, _, object, _)| *object).collect();
    assert_eq!(objects, vec![Some(ObjectId(1)), Some(ObjectId(3))], "C01 is hidden; the group draws as its top-level id");
    assert_eq!(drawn[1].3[0].0[0], [20.0, 0.0], "group transform applied to the child");
    assert!(drawn.iter().all(|(stroke, ..)| *stroke == Stroke::Hairline));
    assert_eq!(markers(&list).iter().map(|m| m.0).collect::<Vec<_>>(), vec![MarkerKind::Origin]);
}

#[test]
fn document_list_marks_the_selection_dashed_with_a_box() {
    let list = document_list(&project(), None, &ViewOpts { selection: vec![ObjectId(3)], ..opts() });
    let drawn = polylines(&list);
    assert_eq!(drawn[0].0, Stroke::Hairline);
    assert_eq!(drawn[1].0, Stroke::Dashed, "the selected group is dashed");
    let boxes: Vec<_> =
        list.items.iter().filter_map(|i| if let Item::Rect { stroke, color, rect } = i { Some((*stroke, *color, *rect)) } else { None }).collect();
    assert_eq!(boxes.len(), 1);
    assert_eq!((boxes[0].0, boxes[0].1), (Stroke::Dashed, SELECTION_COLOR));
    assert_eq!((boxes[0].2.min, boxes[0].2.max), ([20.0, 0.0], [24.0, 4.0]));
}

#[test]
fn document_list_dark_mode_keeps_dark_layers_visible() {
    let light = polylines(&document_list(&project(), None, &opts()));
    let dark = polylines(&document_list(&project(), None, &ViewOpts { dark: true, ..opts() }));
    let luma = |c: Rgba| (u32::from(c.0[0]) * 299 + u32::from(c.0[1]) * 587 + u32::from(c.0[2]) * 114) / 1000;
    // C00 is black in LightBurn's palette (and in the WP0 stub): drawn black on light, light on dark.
    assert!(luma(light[0].1) < 80, "{:?}", light[0].1);
    assert!(luma(dark[0].1) > 150, "{:?}", dark[0].1);
}

#[test]
fn document_list_places_markers_where_they_apply() {
    let refusals = vec![
        Refusal::new(RefusalCode::OutOfBed, "at a point").at([7.0, -3.0]),
        Refusal::new(RefusalCode::OrphanObject, "on an object").object(ObjectId(1)),
        Refusal::new(RefusalCode::NothingToMark, "nowhere"),
    ];
    let notices = vec![
        Notice { n: 4, code: NoticeCode::OpenShapeUnhatched, message: "group".into(), object: Some(ObjectId(3)), layer: None },
        Notice { n: 5, code: NoticeCode::ImportTextIgnored, message: "no place".into(), object: None, layer: None },
    ];
    let list = document_list(&project(), None, &ViewOpts { refusals, notices, ..opts() });
    let found = markers(&list);
    assert_eq!(found.len(), 4, "{found:?}");
    assert_eq!(found[0], (MarkerKind::Refusal, [7.0, -3.0], REFUSAL_COLOR));
    assert_eq!(found[1], (MarkerKind::Refusal, [5.0, 5.0], REFUSAL_COLOR), "centre of object 1");
    assert_eq!(found[2], (MarkerKind::Notice(4), [22.0, 2.0], NOTICE_COLOR), "centre of the group");
    assert_eq!(found[3].0, MarkerKind::Origin);
}

#[test]
fn document_list_centres_the_gantry_bed() {
    let mut profile = openlaser_machine::jpt_m7_200();
    profile.kind = openlaser_machine::Kinematics::Gantry(openlaser_machine::GantryMachine {
        bed_mm: [400.0, 300.0],
        origin: WorkspaceOrigin::BottomLeft,
        max_speed: MmPerS::new(250.0).unwrap(),
        travel_speed: MmPerS::new(250.0).unwrap(),
        s_max: 1000,
        tool: openlaser_machine::GantryTool::Laser { dynamic: true },
        air_assist: false,
        frame_before_mark: true,
        home_on_connect: false,
        link: openlaser_machine::GantryLink::ExportOnly,
    });
    let list = document_list(&project(), Some(&profile), &ViewOpts { show_work_area: true, ..opts() });
    let bed = list.items.iter().find_map(|i| if let Item::Rect { rect, .. } = i { Some(*rect) } else { None }).expect("bed outline");
    assert_eq!((bed.min, bed.max), ([-200.0, -150.0], [200.0, 150.0]));
}

fn job() -> JobPlan {
    let cut =
        CutQ::Gantry(GantryCutQ { speed: MmPerS::new(10.0).unwrap(), power: Percent::new(10.0).unwrap(), s_value: 100, air_assist: false, dynamic: true });
    let section = |layer: u8, mode: SectionMode, paths: Vec<Polyline>| Section {
        layer: LayerKey::Color(layer),
        layer_name: format!("C{layer:02}"),
        sub: 0,
        pass: 1,
        of: 1,
        mode,
        cut: cut.clone(),
        paths,
        objects: vec![ObjectId(u64::from(layer) + 1)],
        estimate: Duration::ZERO,
    };
    JobPlan {
        machine: MachineId("m".into()),
        device: DeviceKind::Grbl,
        sections: vec![
            section(0, SectionMode::Line, vec![Polyline(vec![[0.0, 0.0], [1.0, 0.0]]), Polyline(vec![[2.0, 0.0], [3.0, 0.0], [3.0, 1.0]])]),
            section(1, SectionMode::Fill, vec![Polyline(vec![[5.0, 5.0], [6.0, 5.0]]), Polyline(vec![[6.0, 6.0], [5.0, 6.0]])]),
        ],
        frame: vec![Polyline(vec![[-1.0, -1.0], [7.0, -1.0], [7.0, 7.0], [-1.0, 7.0], [-1.0, -1.0]])],
        tool_frame: Vec::new(),
        facts: PlanFacts::default(),
        repeats: 1,
        estimate: Duration::ZERO,
        fingerprint: Fingerprint([0; 32]),
    }
}

#[test]
fn plan_list_travel_runs_from_each_path_end_to_the_next_start() {
    let show = PlanShow { burn: false, travel: true, frame: false, order: false };
    let list = plan_list(&job(), &openlaser_machine::jpt_m7_200(), &show);
    assert_eq!(list.items.len(), 1);
    let Item::Segments { color, segs } = &list.items[0] else { panic!("travel segments") };
    assert_eq!(*color, TRAVEL_COLOR);
    assert_eq!(segs.to_vec(), vec![[[1.0, 0.0], [2.0, 0.0]], [[3.0, 1.0], [5.0, 5.0]], [[6.0, 5.0], [6.0, 6.0]]]);
}

#[test]
fn plan_list_burn_order_and_frame() {
    let m = openlaser_machine::jpt_m7_200();
    let plain = plan_list(&job(), &m, &PlanShow { burn: true, travel: false, frame: true, order: false });
    assert_eq!(plain.items.len(), 3, "one line item, one fill item, the frame");
    assert!(matches!(&plain.items[2], Item::Polylines { stroke: Stroke::Dashed, color, .. } if *color == FRAME_COLOR));
    let ordered = plan_list(&job(), &m, &PlanShow { burn: true, travel: false, frame: false, order: true });
    let colours: Vec<Rgba> = ordered
        .items
        .iter()
        .map(|i| match i {
            Item::Polylines { color, .. } | Item::Segments { color, .. } => *color,
            _ => panic!("unexpected item"),
        })
        .collect();
    assert_eq!(colours.len(), 3, "each line path, then the fill section");
    assert!(colours[0].0[2] > colours[1].0[2] && colours[1].0[0] < colours[2].0[0], "blue → red along the order: {colours:?}");
    let none = plan_list(&job(), &m, &PlanShow { burn: false, travel: false, frame: false, order: true });
    assert!(none.items.is_empty());
}
