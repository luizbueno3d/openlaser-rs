//! `make_plan()` acceptance (SPEC §7.4): the business card against the OM oracle, section order, facts, frames, the
//! fingerprint (S21) and the properties of §6.5.
//!
//! Projects are built as literals (30 colour layers from the JPT M7 defaults), not through `Project::new`, so the
//! layer table is under the test's control. The tests that need real geometry (WP1's `Geom::to_bez`, `flatten`,
//! `hatch`), the OM importer (WP2) or the galvo encoders (WP4) run since I1 merged those packages.

// An integration-test crate is test code; clippy's in-test allowances only reach `#[test]` functions, not helpers.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::sync::Arc;

use openlaser_doc::{
    BezPath, ColorLayer, CutMode, CutOp, CutTable, DeviceHint, DeviceKind, Geom, ImageCut, ImageMode, JobSettings, LayerDefaults, Project, Shape, ToolLayer,
    Workspace, WorkspaceOrigin, default_layer_name,
};
use openlaser_geom::{Affine, Polyline};
use openlaser_machine::{GalvoMachine, Kinematics, MachineProfile, jpt_m7_200_sim};
use openlaser_plan::{CutQ, HULL_FRAME_UNSUPPORTED, JobPlan, PlanOptions, Section, plan as make_plan};
use openlaser_types::{Deg, FrameSource, FrameStyle, LayerKey, MachineId, Mm, MmPerS, ObjectId, Percent, SectionMode};
use proptest::prelude::*;

// ---- builders --------------------------------------------------------------------------------------------------

fn machine() -> MachineProfile {
    jpt_m7_200_sim()
}

fn galvo(m: &MachineProfile) -> &GalvoMachine {
    match &m.kind {
        Kinematics::Galvo(g) => g,
        Kinematics::Gantry(_) => panic!("expected a galvo profile"),
    }
}

fn op(d: &LayerDefaults, mode: CutMode, passes: u16) -> CutOp {
    CutOp { name: String::new(), enabled: true, mode, passes, galvo: d.galvo.clone(), gantry: d.gantry.clone(), fill: d.fill.clone() }
}

fn table(d: &LayerDefaults) -> CutTable {
    let colors = (0u8..30)
        .map(|i| ColorLayer {
            index: i,
            name: default_layer_name(LayerKey::Color(i)),
            output: true,
            show: true,
            ops: vec![op(d, CutMode::Line, 1)],
            global_passes: 1,
            image: ImageCut {
                mode: ImageMode::Threshold,
                interval: d.fill.interval,
                angle: d.fill.angle,
                bidirectional: true,
                negative: false,
                threshold: 128,
                passes: 1,
                galvo: d.galvo.clone(),
                gantry: d.gantry.clone(),
            },
            air_assist: false,
        })
        .collect();
    let tool = |which: u8| ToolLayer { which, name: format!("T{which}"), show: true };
    CutTable { colors, tools: [tool(1), tool(2)], order: Vec::new() }
}

fn project(m: &MachineProfile, shapes: Vec<Shape>) -> Project {
    let mut order: Vec<LayerKey> = Vec::new();
    for s in &shapes {
        if !order.contains(&s.layer) {
            order.push(s.layer);
        }
    }
    let mut layers = table(&m.layer_defaults);
    layers.order = order;
    Project {
        name: "test".into(),
        shapes: shapes.into_iter().map(Arc::new).collect(),
        layers,
        workspace: Workspace { kind: DeviceKind::GalvoLmc, size_mm: [200.0, 200.0], origin: WorkspaceOrigin::Center },
        device_hint: None,
        job: JobSettings { repeats: 1, cut_selected: false, frame_style: FrameStyle::Contour, frame_source: FrameSource::Job },
        notes: String::new(),
        next_id: 1000,
        extras: BTreeMap::new(),
    }
}

fn path_shape(id: u64, layer: LayerKey, pts: &[[f64; 2]], closed: bool) -> Shape {
    let mut b = BezPath::new();
    if let Some((first, rest)) = pts.split_first() {
        b.move_to((first[0], first[1]));
        for p in rest {
            b.line_to((p[0], p[1]));
        }
        if closed {
            b.close_path();
        }
    }
    Shape {
        id: ObjectId(id),
        name: None,
        layer,
        xform: Affine::IDENTITY,
        geom: Geom::Path(Arc::new(b)),
        locked: false,
        power_scale: Percent::new(100.0).unwrap(),
        priority: 0,
    }
}

fn rect(id: u64, layer: LayerKey, cx: f64, cy: f64, w: f64, h: f64) -> Shape {
    let (x0, y0, x1, y1) = (cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0);
    path_shape(id, layer, &[[x0, y0], [x1, y0], [x1, y1], [x0, y1]], true)
}

fn opts(style: FrameStyle) -> PlanOptions {
    PlanOptions { selected: None, repeats: 1, frame_style: style, frame_source: FrameSource::Job, flatten_tol_mm: 0.01 }
}

const C0: LayerKey = LayerKey::Color(0);
const C1: LayerKey = LayerKey::Color(1);
const C2: LayerKey = LayerKey::Color(2);

fn layer_mut(p: &mut Project, i: u8) -> &mut ColorLayer {
    p.color_layer_mut(i).unwrap()
}

fn pass_ids(plan: &JobPlan) -> Vec<(LayerKey, u8, u16, u16)> {
    plan.sections.iter().map(|s| (s.layer, s.sub, s.pass, s.of)).collect()
}

/// Even-odd point-in-region over closed polylines (independent of `geom::inside_even_odd`).
fn inside(closed: &[Polyline], p: [f64; 2]) -> bool {
    let mut odd = false;
    for poly in closed {
        for w in poly.0.windows(2) {
            let (a, b) = (w[0], w[1]);
            if (a[1] > p[1]) != (b[1] > p[1]) {
                let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if p[0] < x {
                    odd = !odd;
                }
            }
        }
    }
    odd
}

fn close_enough(a: &[[f64; 2]], b: &[[f64; 2]], tol: f64) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| (p[0] - q[0]).abs() <= tol && (p[1] - q[1]).abs() <= tol)
}

// ---- the business card (OM oracle) -------------------------------------------------------------------------------

/// `rel` is `/`-separated; it is joined component by component (no `/` literals in paths, SPEC §2.6).
fn golden_json(rel: &str) -> serde_json::Value {
    let path = rel.split('/').fold(openlaser_testkit::golden_dir(), |p, c| p.join(c));
    let bytes = std::fs::read(path).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn polylines_of(v: &serde_json::Value) -> Vec<Vec<[f64; 2]>> {
    v.as_array().unwrap().iter().map(|poly| poly.as_array().unwrap().iter().map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()]).collect()).collect()
}

#[test]
fn plan_business_card_matches_om() {
    let golden = openlaser_testkit::golden_dir();
    let ws = openlaser_import::om::parse_workspace(&std::fs::read(golden.join("inputs").join("workspace.json")).unwrap()).unwrap();
    let specs = openlaser_import::om::parse_layer_settings(&std::fs::read(golden.join("inputs").join("layer_settings.json")).unwrap()).unwrap();
    let imported = openlaser_import::om::to_imported(&ws, &specs).unwrap();
    let m = machine();
    let mut p = project(&m, Vec::new());
    for (k, l) in imported.layers {
        if let LayerKey::Color(i) = k {
            p.layers.colors[usize::from(i)] = l;
        }
    }
    for (n, mut s) in (1u64..).zip(imported.shapes) {
        s.id = ObjectId(n);
        p.shapes.push(Arc::new(s));
    }
    // OM's layer order (Appendix B): vector-engrave C00, vector-cut C01, raster C02.
    p.layers.order = vec![C0, C1, C2];
    let plan = make_plan(&p, &m, &opts(FrameStyle::Contour)).unwrap();
    assert!(plan.facts.orphan_objects.is_empty() && plan.facts.unsupported.is_empty() && plan.facts.hidden_output_layers.is_empty(), "{:?}", plan.facts);

    // Vector Engrave: one pass, exactly the polylines OM marked (T1 bc_mark_vector-engrave_pass1 inputs).
    let vector: Vec<&Section> = plan.sections.iter().filter(|s| s.layer == C0).collect();
    assert_eq!(vector.len(), 1);
    assert_eq!(vector[0].mode, SectionMode::Line);
    let om_vector = polylines_of(&golden_json("t1_lmc/inputs/bc_mark_vector-engrave_pass1.json")["polylines"]);
    assert_eq!((vector[0].paths.len(), vector[0].paths.iter().map(Polyline::len).sum::<usize>()), (304, 1750));
    for (ours, om) in vector[0].paths.iter().zip(&om_vector) {
        assert!(close_enough(&ours.0, om, 1e-9), "vector path differs from OM");
    }
    let CutQ::Galvo(q) = &vector[0].cut else { panic!("galvo cut") };
    assert_eq!((q.mark_current, q.q_period, q.speed_units, q.timings.jump_speed_units), (4095, 667, 276, 830));

    // Raster: one Fill pass, exactly T3 business_card_raster_saved_settings (969 segments, 32 closed, 67 open).
    let raster: Vec<&Section> = plan.sections.iter().filter(|s| s.layer == C2).collect();
    assert_eq!(raster.len(), 1);
    assert_eq!(raster[0].mode, SectionMode::Fill);
    let t3 = golden_json("t3_hatch/hatch.json");
    let case = t3["cases"].as_array().unwrap().iter().find(|c| c["name"] == "business_card_raster_saved_settings").unwrap();
    let segs = polylines_of(&case["output"]["segments"]);
    assert_eq!(raster[0].paths.len(), 969);
    for (ours, om) in raster[0].paths.iter().zip(&segs) {
        assert!(close_enough(&ours.0, om, 1e-9), "hatch segment differs from T3");
    }
    assert_eq!(plan.facts.open_fill_shapes.len(), 67);
    assert!(plan.facts.empty_fill_layers.is_empty());

    // Frame: every outline OM framed (T1 bc_frame inputs: 403 paths, 3057 points), the raster's outlines, not its hatch.
    let om_frame = polylines_of(&golden_json("t1_lmc/inputs/bc_frame.json")["polylines"]);
    assert_eq!((plan.frame.len(), plan.frame.iter().map(Polyline::len).sum::<usize>()), (403, 3057));
    for (ours, om) in plan.frame.iter().zip(&om_frame) {
        assert!(close_enough(&ours.0, om, 1e-9), "frame path differs from OM");
    }
    // Nothing in it would be refused by a fact; `safety`'s tests judge the rest.
    assert!(plan.facts.non_finite_objects.is_empty() && plan.facts.device_mismatch.is_none() && plan.facts.tool_layer_objects.is_empty());
}

// ---- sections, facts, frames ----------------------------------------------------------------------------------

#[test]
fn plan_orders_layers_sub_layers_passes_and_global_passes() {
    let m = machine();
    let d = m.layer_defaults.clone();
    let mut p = project(&m, vec![rect(1, C1, 0.0, 0.0, 10.0, 10.0), rect(2, C0, 20.0, 0.0, 5.0, 5.0)]);
    p.layers.order = vec![C1, C0];
    {
        let l = layer_mut(&mut p, 1);
        l.ops = vec![op(&d, CutMode::Line, 2), op(&d, CutMode::Line, 1)];
        l.global_passes = 2;
    }
    let plan = make_plan(&p, &m, &opts(FrameStyle::Contour)).unwrap();
    assert_eq!(pass_ids(&plan), vec![(C1, 0, 1, 4), (C1, 0, 2, 4), (C1, 1, 1, 2), (C1, 0, 3, 4), (C1, 0, 4, 4), (C1, 1, 2, 2), (C0, 0, 1, 1)]);
    assert!(plan.sections.iter().all(|s| s.estimate > std::time::Duration::ZERO));
    let one: std::time::Duration = plan.sections.iter().map(|s| s.estimate).sum();
    assert_eq!(plan.estimate, one);
    // A disabled sub-layer burns nothing and a layer with Output off is neither burned nor framed.
    layer_mut(&mut p, 1).ops[1].enabled = false;
    layer_mut(&mut p, 0).output = false;
    let plan = make_plan(&p, &m, &opts(FrameStyle::Contour)).unwrap();
    assert_eq!(pass_ids(&plan), vec![(C1, 0, 1, 4), (C1, 0, 2, 4), (C1, 0, 3, 4), (C1, 0, 4, 4)]);
    assert_eq!(plan.frame.len(), 1);
}

/// A `passes`, `globalPasses` or `image.passes` of 0 would plan no sections for the layer's shapes, and nothing is
/// ever dropped silently: `plan` refuses it through `CutTable::check_passes` (`doc::Error::ZeroPasses`), the same
/// check `format` applies on load/save. `layer.cutSettings.set` refuses the same values at edit time.
#[test]
fn plan_refuses_a_zero_pass_layer() {
    let m = machine();
    let refuses = |p: &Project, field: &str| match make_plan(p, &m, &opts(FrameStyle::Contour)) {
        Err(openlaser_plan::Error::Doc(openlaser_doc::Error::ZeroPasses { layer, field: got })) => {
            assert_eq!((layer, got.as_str()), (C2, field), "the refusal names the layer and the field");
        }
        other => panic!("{field}: a zero-pass layer must be refused, got {other:?}"),
    };
    let mut p = project(&m, vec![rect(1, C2, 0.0, 0.0, 10.0, 10.0)]);
    layer_mut(&mut p, 2).ops[0].passes = 0;
    refuses(&p, "ops[0].passes");
    let mut p = project(&m, vec![rect(1, C2, 0.0, 0.0, 10.0, 10.0)]);
    layer_mut(&mut p, 2).global_passes = 0;
    refuses(&p, "globalPasses");
    let mut p = project(&m, vec![rect(1, C2, 0.0, 0.0, 10.0, 10.0)]);
    layer_mut(&mut p, 2).image.passes = 0;
    refuses(&p, "image.passes");
}

#[test]
fn plan_fill_hatches_closed_shapes_and_notes_open_ones() {
    let m = machine();
    let d = m.layer_defaults.clone();
    let mut p = project(&m, vec![rect(1, C2, 0.0, 0.0, 10.0, 10.0), path_shape(2, C2, &[[20.0, 0.0], [30.0, 5.0]], false)]);
    {
        let l = layer_mut(&mut p, 2);
        l.ops = vec![op(&d, CutMode::Fill, 1)];
        l.ops[0].fill.interval = Mm::new(1.0).unwrap();
        l.ops[0].fill.cross_hatch = true;
    }
    let plan = make_plan(&p, &m, &opts(FrameStyle::Contour)).unwrap();
    // Cross-hatch: a +90° pass after the pass; each a section of 2-point segments inside the square.
    assert_eq!(pass_ids(&plan), vec![(C2, 0, 1, 2), (C2, 0, 2, 2)]);
    for s in &plan.sections {
        assert_eq!(s.mode, SectionMode::Fill);
        assert!(!s.paths.is_empty() && s.paths.iter().all(|seg| seg.len() == 2));
        assert!(s.paths.iter().flat_map(|seg| seg.0.iter()).all(|q| q[0].abs() <= 5.0 + 1e-9 && q[1].abs() <= 5.0 + 1e-9));
    }
    assert_eq!(plan.facts.open_fill_shapes, vec![(ObjectId(2), C2)]);
    // The frame traces both outlines (placement), never the hatch.
    assert_eq!(plan.frame.len(), 2);
    // Only open shapes on a Fill layer: nothing to hatch is a fact (refused by safety), never silence.
    let mut p2 = project(&m, vec![path_shape(3, C2, &[[0.0, 0.0], [10.0, 0.0]], false)]);
    layer_mut(&mut p2, 2).ops = vec![op(&d, CutMode::Fill, 1)];
    let plan2 = make_plan(&p2, &m, &opts(FrameStyle::Contour)).unwrap();
    assert!(plan2.sections.is_empty());
    assert_eq!(plan2.facts.empty_fill_layers, vec![C2]);
}

#[test]
fn plan_records_facts_never_drops() {
    // Runs on WP0 stubs too: facts depend on the layer table, not on geometry.
    let m = machine();
    let d = m.layer_defaults.clone();
    let mut p = project(
        &m,
        vec![
            rect(1, C0, 0.0, 0.0, 10.0, 10.0),
            rect(2, C1, 0.0, 0.0, 10.0, 10.0),
            rect(3, LayerKey::Tool(1), 0.0, 0.0, 30.0, 30.0),
            rect(4, LayerKey::Color(29), 0.0, 0.0, 5.0, 5.0),
            rect(5, C2, 0.0, 0.0, 5.0, 5.0),
        ],
    );
    // A forged table: C29 does not exist.
    p.layers.colors.truncate(29);
    layer_mut(&mut p, 1).show = false;
    {
        let l = layer_mut(&mut p, 2);
        l.ops = vec![op(&d, CutMode::OffsetFill, 1)];
        l.ops[0].galvo.wobble = Some(openlaser_doc::Wobble { step: Mm::new(0.1).unwrap(), size: Mm::new(0.2).unwrap() });
    }
    p.device_hint = Some(DeviceHint { kind: DeviceKind::Grbl, profile: MachineId("diode-grbl-400x400".into()) });
    let plan = make_plan(&p, &m, &opts(FrameStyle::Hull)).unwrap();
    assert_eq!(plan.facts.orphan_objects, vec![ObjectId(4)]);
    assert_eq!(plan.facts.hidden_output_layers, vec![C1]);
    assert_eq!(plan.facts.tool_layer_objects, vec![ObjectId(3)]);
    assert!(plan.facts.unsupported.iter().any(|(k, w)| *k == C2 && w.contains("Offset Fill")), "{:?}", plan.facts.unsupported);
    assert!(plan.facts.unsupported.iter().any(|(_, w)| w == HULL_FRAME_UNSUPPORTED));
    assert!(plan.facts.device_mismatch.as_deref().is_some_and(|w| w.contains("diode-grbl-400x400")));
    assert!(plan.frame.is_empty() && plan.tool_frame.is_empty(), "Hull traces nothing");
    assert_eq!((plan.machine.clone(), plan.device, plan.repeats), (m.id.clone(), DeviceKind::GalvoLmc, 1));
}

/// B6: a hidden layer with Output on is recorded (safety refuses it) and still framed, Contour and Bounds alike.
#[test]
fn plan_frames_hidden_output_layers() {
    let m = machine();
    let mut p = project(&m, vec![rect(1, C0, 0.0, 0.0, 10.0, 10.0), rect(2, C1, 30.0, 0.0, 10.0, 10.0)]);
    layer_mut(&mut p, 1).show = false;
    for style in [FrameStyle::Contour, FrameStyle::Bounds] {
        let plan = make_plan(&p, &m, &opts(style)).unwrap();
        assert_eq!(plan.facts.hidden_output_layers, vec![C1]);
        let hidden: Vec<&Polyline> = plan.sections.iter().filter(|s| s.layer == C1).flat_map(|s| s.paths.iter()).collect();
        assert_eq!(hidden.len(), 1, "the hidden layer plans its burn");
        let fb = openlaser_geom::bounds_of(&plan.frame).unwrap();
        for q in hidden.iter().flat_map(|h| h.0.iter()) {
            assert!(q[0] >= fb.min[0] && q[0] <= fb.max[0] && q[1] >= fb.min[1] && q[1] <= fb.max[1], "{style:?}: {q:?} is outside the frame");
        }
        if style == FrameStyle::Contour {
            assert!(hidden.iter().all(|h| plan.frame.contains(h)), "the hidden layer's outline is traced");
        }
    }
}

/// A curve tolerance this build cannot use (not a number, zero, negative or below 1 µm) is refused, never replaced by
/// the default or raised to the minimum: the plan records an `unsupported` fact, which `safety` refuses.
#[test]
fn plan_refuses_an_unusable_curve_tolerance() {
    let m = machine();
    let p = project(&m, vec![rect(1, C0, 0.0, 0.0, 10.0, 10.0)]);
    let tolerance_facts = |tol: f64| {
        let plan = make_plan(&p, &m, &PlanOptions { flatten_tol_mm: tol, ..opts(FrameStyle::Contour) }).unwrap();
        plan.facts.unsupported.iter().filter(|(_, why)| why.contains("curve tolerance")).count()
    };
    for tol in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -0.01, 1e-6, 0.000_999] {
        assert_eq!(tolerance_facts(tol), 1, "tolerance {tol} mm");
    }
    for tol in [0.001, 0.01, 0.1] {
        assert_eq!(tolerance_facts(tol), 0, "tolerance {tol} mm is usable");
    }
    assert_eq!(tolerance_facts(0.2), 1, "above the 0.1 mm this build plans");
}

#[test]
fn plan_selected_only_and_tool_frame() {
    let m = machine();
    let mut p = project(&m, vec![rect(1, C0, 0.0, 0.0, 10.0, 10.0), rect(2, C0, 30.0, 0.0, 10.0, 10.0), rect(3, LayerKey::Tool(1), 0.0, 0.0, 80.0, 80.0)]);
    p.layers.order = vec![C0];
    let mut o = opts(FrameStyle::Bounds);
    o.selected = Some(vec![ObjectId(2), ObjectId(3)]);
    let plan = make_plan(&p, &m, &o).unwrap();
    assert_eq!(plan.sections.len(), 1);
    assert_eq!(plan.sections[0].objects, vec![ObjectId(2)]);
    assert_eq!(plan.frame, vec![openlaser_plan::bounds_rect(&openlaser_geom::Bounds { min: [25.0, -5.0], max: [35.0, 5.0] })]);
    assert_eq!(plan.tool_frame, vec![openlaser_plan::bounds_rect(&openlaser_geom::Bounds { min: [-40.0, -40.0], max: [40.0, 40.0] })]);
    assert_eq!(openlaser_plan::frame_style_of(&plan.frame), FrameStyle::Bounds);
}

#[test]
fn plan_applies_the_machine_transform_after_hatching() {
    let mut m = machine();
    if let Kinematics::Galvo(g) = &mut m.kind {
        g.cal.negate = [true, false];
        g.cal.offset_mm = [1.0, 0.0];
    }
    let p = project(&m, vec![path_shape(1, C0, &[[10.0, 5.0], [20.0, 5.0]], false)]);
    let plan = make_plan(&p, &m, &opts(FrameStyle::Contour)).unwrap();
    assert_eq!(plan.sections[0].paths, vec![Polyline(vec![[-9.0, 5.0], [-19.0, 5.0]])]);
    assert_eq!(plan.frame, plan.sections[0].paths);
}

#[test]
fn fingerprint_changes_with_power() {
    let m = machine();
    let p = project(&m, vec![rect(1, C0, 0.0, 0.0, 10.0, 10.0)]);
    let a = make_plan(&p, &m, &opts(FrameStyle::Contour)).unwrap();
    let mut p2 = p.clone();
    layer_mut(&mut p2, 0).ops[0].galvo.power = Percent::new(31.0).unwrap();
    let b = make_plan(&p2, &m, &opts(FrameStyle::Contour)).unwrap();
    assert_ne!(a.fingerprint, b.fingerprint);
    // Speed, frequency, a moved shape and the repeat count change it too; a renamed layer does not.
    let mut p3 = p.clone();
    layer_mut(&mut p3, 0).ops[0].galvo.speed = MmPerS::new(1500.0).unwrap();
    assert_ne!(a.fingerprint, make_plan(&p3, &m, &opts(FrameStyle::Contour)).unwrap().fingerprint);
    let p4 = project(&m, vec![rect(1, C0, 0.001, 0.0, 10.0, 10.0)]);
    assert_ne!(a.fingerprint, make_plan(&p4, &m, &opts(FrameStyle::Contour)).unwrap().fingerprint);
    let mut o = opts(FrameStyle::Contour);
    o.repeats = 2;
    assert_ne!(a.fingerprint, make_plan(&p, &m, &o).unwrap().fingerprint);
    let mut p5 = p.clone();
    layer_mut(&mut p5, 0).name = "Engrave".into();
    assert_eq!(a.fingerprint, make_plan(&p5, &m, &opts(FrameStyle::Contour)).unwrap().fingerprint);
}

#[test]
fn estimate_galvo_counts_marks_jumps_and_delays() {
    let m = machine();
    let g = galvo(&m);
    let p = project(&m, vec![path_shape(1, C0, &[[10.0, 0.0], [20.0, 0.0]], false)]);
    let plan = make_plan(&p, &m, &opts(FrameStyle::Contour)).unwrap();
    let CutQ::Galvo(q) = &plan.sections[0].cut else { panic!("galvo cut") };
    let f = g.field_f()[0];
    let mark = f64::from(q.speed_units) * f * 1000.0 / 65535.0;
    let jump = f64::from(q.timings.jump_speed_units) * f * 1000.0 / 65535.0;
    // MO settle 8 ms + first jump 10 mm + long delay 200 µs + settle 1 ms + 10 mm mark + on/off/polygon 300 µs.
    let want = 8e-3 + 10.0 / jump + 200e-6 + 1e-3 + 10.0 / mark + 300e-6;
    let got = plan.sections[0].estimate.as_secs_f64();
    assert!((got - want).abs() < 1e-9, "{got} vs {want}");
    assert_eq!(openlaser_plan::estimate_galvo(&plan.sections[0].paths, q, g), plan.sections[0].estimate);
}

// ---- properties ----------------------------------------------------------------------------------------------

/// A random design: rectangles and triangles on a Line layer (C00) and a Fill layer (C01).
fn design() -> impl Strategy<Value = (Vec<Shape>, bool, u16, f64)> {
    let shape = (0u8..2, -80.0f64..80.0, -80.0f64..80.0, 1.0f64..15.0, 1.0f64..15.0, any::<bool>());
    (prop::collection::vec(shape, 1..6), any::<bool>(), 1u16..3, 0.3f64..1.5).prop_map(|(raw, cross, passes, interval)| {
        let shapes = (1u64..)
            .zip(raw)
            .map(|(id, (l, cx, cy, w, h, tri))| {
                let layer = LayerKey::Color(l);
                if tri {
                    path_shape(id, layer, &[[cx - w / 2.0, cy - h / 2.0], [cx + w / 2.0, cy - h / 2.0], [cx, cy + h / 2.0]], true)
                } else {
                    rect(id, layer, cx, cy, w, h)
                }
            })
            .collect();
        (shapes, cross, passes, interval)
    })
}

fn design_project(m: &MachineProfile, shapes: Vec<Shape>, cross: bool, passes: u16, interval: f64) -> Project {
    let d = m.layer_defaults.clone();
    let mut p = project(m, shapes);
    let l = layer_mut(&mut p, 1);
    l.ops = vec![op(&d, CutMode::Fill, passes)];
    l.ops[0].fill.interval = Mm::new(interval).unwrap();
    l.ops[0].fill.cross_hatch = cross;
    l.ops[0].fill.angle = Deg::new(30.0).unwrap();
    p
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// PLAN §3.6 layer 3: every Line point lies on a traced contour, every Fill segment within the even-odd region
    /// of the traced closed contours; with Bounds, everything lies inside the rectangle (B6). `hide` hides C00
    /// and/or C01 with Output still on: a hidden emitting layer is recorded and must be framed all the same.
    #[test]
    fn prop_frame_covers_burn((shapes, cross, passes, interval) in design(), bounds in any::<bool>(), hide in any::<(bool, bool)>()) {
        let m = machine();
        let mut p = design_project(&m, shapes, cross, passes, interval);
        layer_mut(&mut p, 0).show = !hide.0;
        layer_mut(&mut p, 1).show = !hide.1;
        let style = if bounds { FrameStyle::Bounds } else { FrameStyle::Contour };
        let plan = make_plan(&p, &m, &opts(style)).unwrap();
        for (i, hidden) in [(0u8, hide.0), (1u8, hide.1)] {
            let key = LayerKey::Color(i);
            let burns = plan.sections.iter().any(|s| s.layer == key);
            let fact = plan.facts.hidden_output_layers.contains(&key);
            prop_assert!(!(hidden && burns) || fact, "a hidden layer that burns is a fact");
            prop_assert!(hidden || !fact, "a shown layer is never reported hidden");
        }
        let fb = openlaser_geom::bounds_of(&plan.frame);
        prop_assert!(plan.sections.is_empty() || fb.is_some(), "a job that burns has a frame");
        for s in &plan.sections {
            for path in &s.paths {
                for q in &path.0 {
                    let b = fb.unwrap();
                    prop_assert!(q[0] >= b.min[0] - 1e-9 && q[0] <= b.max[0] + 1e-9 && q[1] >= b.min[1] - 1e-9 && q[1] <= b.max[1] + 1e-9);
                }
                if style == FrameStyle::Contour {
                    match s.mode {
                        SectionMode::Line => prop_assert!(plan.frame.contains(path), "a Line path is not traced"),
                        _ => {
                            let mid = [(path.0[0][0] + path.0[1][0]) / 2.0, (path.0[0][1] + path.0[1][1]) / 2.0];
                            // Fill is even-odd per layer (S13): judge against this layer's outlines, i.e. the traced
                            // contours that are not some Line layer's paths.
                            let line_paths: Vec<&Polyline> = plan.sections.iter().filter(|x| x.mode == SectionMode::Line).flat_map(|x| x.paths.iter()).collect();
                            let closed: Vec<Polyline> = plan
                                .frame
                                .iter()
                                .filter(|f| f.is_closed(openlaser_geom::CLOSURE_TOL_MM) && !line_paths.contains(f))
                                .cloned()
                                .collect();
                            prop_assert!(inside(&closed, mid), "a Fill segment lies outside the traced contours");
                        }
                    }
                }
            }
        }
    }

    /// SPEC §5.2 rule 4: the speed is encoded once, from the requested value, to whole units (trunc); the displayed
    /// actual speed is never re-encoded. F ∈ [100, 300].
    #[test]
    fn prop_speed_units_stable_over_field(f in 100.0f64..300.0, v in 0.5f64..6000.0) {
        let mut m = machine();
        let g = match &mut m.kind { Kinematics::Galvo(g) => g, Kinematics::Gantry(_) => panic!("galvo") };
        g.cal.scale = [f / 200.0, f / 200.0];
        let ff = g.field_f()[0];
        let d = &m.layer_defaults.galvo;
        let q = openlaser_plan::quantise::quantise_galvo(MmPerS::new(v).unwrap(), d.power, d.frequency, d.q_pulse, galvo(&m)).unwrap();
        let want = (v * (65535.0 / ff) / 1000.0).trunc().min(65535.0);
        prop_assert_eq!(f64::from(q.speed_units), want);
        prop_assert_eq!(q.actual_speed_mm_s.to_bits(), (f64::from(q.speed_units) * ff * 1000.0 / 65535.0).to_bits());
        // Quantising the same request again gives the same word (stable); the actual speed never exceeds it.
        let again = openlaser_plan::quantise::quantise_galvo(MmPerS::new(v).unwrap(), d.power, d.frequency, d.q_pulse, galvo(&m)).unwrap();
        prop_assert_eq!(again.speed_units, q.speed_units);
        prop_assert!(q.actual_speed_mm_s <= v * (1.0 + 1e-12));
    }

    #[test]
    fn prop_plan_deterministic((shapes, cross, passes, interval) in design()) {
        let m = machine();
        let p = design_project(&m, shapes, cross, passes, interval);
        let a = make_plan(&p, &m, &opts(FrameStyle::Contour)).unwrap();
        let b = make_plan(&p.clone(), &m.clone(), &opts(FrameStyle::Contour)).unwrap();
        prop_assert_eq!(&a, &b);
        prop_assert_eq!(a.fingerprint, openlaser_plan::fingerprint_of(&m, a.device, &a.sections, a.repeats));
    }

    /// S21: frame style and source are framing choices, not part of the emission identity.
    #[test]
    fn prop_fingerprint_ignores_frame_style((shapes, cross, passes, interval) in design()) {
        let m = machine();
        let p = design_project(&m, shapes, cross, passes, interval);
        let fps: Vec<_> = [FrameStyle::Bounds, FrameStyle::Contour]
            .into_iter()
            .flat_map(|st| [FrameSource::Job, FrameSource::ToolLayersOnly, FrameSource::JobAndToolLayers].map(move |src| (st, src)))
            .map(|(st, src)| {
                let mut o = opts(st);
                o.frame_source = src;
                make_plan(&p, &m, &o).unwrap().fingerprint
            })
            .collect();
        prop_assert!(fps.windows(2).all(|w| w[0] == w[1]));
    }
}

#[test]
fn rotary_fingerprint_frame_and_scan_angle_notice() {
    use openlaser_machine::RotaryCfg;
    let mut m = machine();
    let Kinematics::Galvo(g) = &mut m.kind else { panic!("galvo") };
    g.rotary = Some(RotaryCfg { enabled: true, ..Default::default() });
    let mut p = project(&m, vec![rect(1, C0, 0.0, 0.0, 10.0, 30.0)]);
    layer_mut(&mut p, 0).ops[0].mode = CutMode::Fill;
    layer_mut(&mut p, 0).ops[0].fill.angle = Deg::new(90.0).unwrap();
    let plan = make_plan(&p, &m, &opts(FrameStyle::Contour)).unwrap();
    assert!(plan.facts.unsupported.is_empty(), "{:?}", plan.facts.unsupported);
    assert_eq!(plan.facts.rotary_notices.len(), 1);
    assert_eq!(plan.sections.len(), 3);
    assert!(plan.frame.iter().all(|p| p.0.first() == p.0.last()), "clipped fill outlines must close along strip edges");
    let bounds = openlaser_geom::bounds_of(&plan.frame).unwrap();
    for pt in plan.sections.iter().flat_map(|s| &s.paths).flat_map(|p| &p.0) {
        assert!(pt[0] >= bounds.min[0] && pt[0] <= bounds.max[0] && pt[1] >= bounds.min[1] && pt[1] <= bounds.max[1]);
    }
    assert!(bounds.min[1] >= -5.0 && bounds.max[1] <= 5.0);
    let fp = plan.fingerprint;
    let mut altered = plan.sections.clone();
    altered[0].rotary.as_mut().unwrap().target_pulses += 1;
    assert_ne!(fp, openlaser_plan::fingerprint_of(&m, plan.device, &altered, 1));
    let Kinematics::Galvo(g) = &mut m.kind else { panic!("galvo") };
    g.rotary.as_mut().unwrap().min_speed_pps += 1;
    assert_ne!(fp, openlaser_plan::fingerprint_of(&m, plan.device, &plan.sections, 1));
    layer_mut(&mut p, 0).ops[0].fill.angle = Deg::new(180.0).unwrap();
    assert!(make_plan(&p, &m, &opts(FrameStyle::Contour)).unwrap().facts.rotary_notices.is_empty());
}
