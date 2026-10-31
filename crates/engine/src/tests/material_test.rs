use super::*;

const ID: &str = "laserTools.materialTest";

fn grid() -> Value {
    json!({
        "rows": {"param": "powerPct", "min": 1, "max": 5, "count": 5},
        "cols": {"param": "frequencyKhz", "min": 30, "max": 50, "count": 5},
        "centerMm": [10, -5], "cellWMm": 5, "cellHMm": 4, "gapMm": 2,
        "labelPowerPct": 1, "labelSpeedMmS": 1000
    })
}

fn opts() -> openlaser_plan::PlanOptions {
    openlaser_plan::PlanOptions { selected: None, repeats: 1, frame_style: FrameStyle::Contour, frame_source: FrameSource::Job, flatten_tol_mm: 0.01 }
}

#[test]
fn material_test_grid_values_geometry_order_and_round_trip() {
    let mut s = headless();
    with_hack(&mut s);
    let v = s.execute(ID, grid()).unwrap();
    assert_eq!(v["cells"], 25);
    assert_eq!(v["layers"].as_array().unwrap().len(), 26);
    let p = &s.active().unwrap().project;
    assert_eq!(p.name, "Untitled");
    assert_eq!(p.layers.colors.len(), 30, "the fixed palette remains 30; used layers are in order");
    assert_eq!(p.layers.order.len(), 26);
    assert_eq!(p.layers.order.last(), Some(&LayerKey::Color(25)));
    assert_eq!(p.shapes.len(), 36, "25 rectangles, 5 column labels, 5 row labels and a title");
    for row in 0..5usize {
        for col in 0..5usize {
            let n = row * 5 + col;
            let l = &p.layers.colors[n];
            let op = &l.ops[0];
            assert_eq!(op.galvo.power.get(), (row + 1) as f64);
            assert_eq!(op.galvo.frequency.get(), 30_000 + col as u32 * 5000);
            assert_eq!(op.mode, CutMode::Fill);
            assert_eq!(op.fill.angle.get(), 0.0);
            assert_eq!(op.fill.interval, jpt_m7_200_sim().layer_defaults.fill.interval);
            let shape = &p.shapes[n];
            let b = p.bounds(&[shape.id]).unwrap().unwrap();
            let x = 10.0 - 33.0 / 2.0 + col as f64 * 7.0;
            let y = -5.0 - 28.0 / 2.0 + row as f64 * 6.0;
            assert_eq!(b, openlaser_types::Bounds { min: [x, y], max: [x + 5.0, y + 4.0] });
            assert_eq!(v["layers"][n]["values"]["powerPct"], (row + 1) as f64);
        }
    }
    assert_eq!(p.layers.colors[0].name, "P 1% · F 30 kHz");
    assert_eq!(p.layers.colors[24].name, "P 5% · F 50 kHz");
    let powers: Vec<_> = p
        .layers
        .order
        .iter()
        .take(25)
        .map(|k| match k {
            LayerKey::Color(i) => p.layers.colors[usize::from(*i)].ops[0].galvo.power.get(),
            _ => panic!("colour"),
        })
        .collect();
    assert!(powers.windows(2).all(|v| v[0] >= v[1]));
    assert_eq!(p.layers.colors[25].ops[0].mode, CutMode::Line);
    let bytes = openlaser_format::save(p).unwrap();
    let loaded = openlaser_format::load(&bytes).unwrap();
    assert_eq!(**p, loaded);
    let plan = openlaser_plan::plan(p, &jpt_m7_200_sim(), &opts()).unwrap();
    openlaser_safety::validate(&plan, &jpt_m7_200_sim()).unwrap();
    let label_layer = &p.layers.colors[25];
    assert_eq!(label_layer.ops[0].galvo.power.get(), 1.0);
    let ids: Vec<_> = p.shapes.iter().map(|sh| sh.id).collect();
    let b = p.bounds(&ids).unwrap().unwrap();
    let size = v["sizeMm"].as_array().unwrap();
    // Result bounds may conservatively include text control points beyond the exact curve extrema.
    assert!(size[0].as_f64().unwrap() >= b.max[0] - b.min[0] - 1e-8);
    assert!(size[1].as_f64().unwrap() >= b.max[1] - b.min[1] - 1e-8);
}

#[test]
fn material_test_bad_fields_are_atomic_and_share_cut_validation() {
    let mut s = headless();
    with_doc(&mut s);
    with_hack(&mut s);
    let original = s.active().unwrap().project.clone();
    let cases = [
        ("/rows/count", json!(6), Some(("/cols/count", json!(6)))),
        ("/rows/count", json!(5), Some(("/cols/count", json!(6)))),
        ("/cols/param", json!("powerPct"), None),
        ("/rows/count", json!(0), None),
        ("/rows/min", json!(-1), None),
        ("/rows/max", json!(101), None),
        ("/cellWMm", json!(300), None),
        ("/gapMm", json!(-1), None),
        ("/cellHMm", json!(0), None),
        ("/centerMm/0", json!(100), None),
        ("/base/speedMmS", json!(0), None),
        ("/base/qPulseNs", json!(0), None),
        ("/base/intervalMm", json!(101), None),
        ("/base/passes", json!(1000), None),
        ("/labelPowerPct", json!(101), None),
        ("/labelSpeedMmS", json!(0), None),
    ];
    for (pointer, value, extra) in cases {
        let mut p = grid();
        p["base"] = json!({"speedMmS":1000,"qPulseNs":20,"intervalMm":0.1,"passes":1});
        *p.pointer_mut(pointer).unwrap_or_else(|| panic!("missing test pointer {pointer}")) = value;
        if let Some((field, v)) = extra {
            *p.pointer_mut(field).unwrap() = v;
        }
        let e = s.execute(ID, p).unwrap_err();
        assert_eq!(e.code(), "bad_params", "{pointer}: {e}");
        let field = pointer.rsplit('/').next().unwrap();
        assert!(e.to_string().contains(field) || pointer.starts_with("/rows/") || pointer.starts_with("/cols/"), "{pointer}: {e}");
        assert_eq!(s.active_index(), Some(0));
        assert_eq!(s.active().unwrap().project, original, "{pointer}");
    }
    let mut p = grid();
    p["rows"]["count"] = json!(6);
    p["cols"]["count"] = json!(6);
    assert!(s.execute(ID, p).unwrap_err().to_string().contains("6 × 6 = 36; lower the counts"));
    // The same unit and interval ranges are refused by the ordinary editor.
    for settings in [json!({"galvo": {"powerPct": 101}}), json!({"galvo": {"speedMmS": 0}}), json!({"fill": {"intervalMm": 101}}), json!({"passes": 1000})] {
        let mut settings = settings;
        settings["layer"] = json!(0);
        assert_eq!(s.execute("layer.cutSettings.set", settings).unwrap_err().code(), "bad_params");
    }
}

#[test]
fn material_test_thirty_unlabelled_descending_single_and_rounded_passes() {
    let mut s = headless();
    let mut p = grid();
    p["labels"] = json!(false);
    p["rows"] = json!({"param": "passes", "min": 3.6, "max": 0.1, "count": 6});
    p["cols"] = json!({"param": "powerPct", "min": 5, "max": 1, "count": 5});
    let v = s.execute(ID, p).unwrap();
    assert_eq!(v["cells"], 30);
    assert_eq!(v["labels"], false);
    let doc = s.active().unwrap();
    assert_eq!(doc.project.shapes.len(), 30);
    assert_eq!(doc.project.layers.order.len(), 30);
    assert_eq!(doc.project.layers.colors[0].ops[0].passes, 4);
    assert_eq!(doc.project.layers.colors[29].ops[0].passes, 1);
    assert_eq!(doc.project.layers.colors[0].ops[0].galvo.power.get(), 5.0);
    assert_eq!(doc.project.layers.colors[29].ops[0].galvo.power.get(), 1.0);
    let mut p = grid();
    p["labels"] = json!(false);
    p["rows"]["count"] = json!(1);
    p["cols"]["count"] = json!(1);
    s.execute(ID, p).unwrap();
    let op = &s.active().unwrap().project.layers.colors[0].ops[0];
    assert_eq!(op.galvo.power.get(), 1.0);
    assert_eq!(op.galvo.frequency.get(), 30_000);
}

#[test]
fn material_test_speed_then_pulse_output_order_and_line_mode() {
    let mut s = headless();
    let p = json!({"rows": {"param":"speedMmS", "min":100, "max":300, "count":3},
        "cols": {"param":"qPulseNs", "min":20, "max":100, "count":3}, "mode":"line", "labels":false});
    s.execute(ID, p).unwrap();
    let project = &s.active().unwrap().project;
    assert_eq!(project.layers.order, [2, 1, 0, 5, 4, 3, 8, 7, 6].map(LayerKey::Color));
    assert!(project.layers.colors.iter().take(9).all(|l| l.ops[0].mode == CutMode::Line));
}

#[test]
fn material_test_agent_design_ignores_session_cap_and_never_contacts_device() {
    let provider = Arc::new(TestProvider { cap: Some(1.0), ..TestProvider::default() });
    let mut s = Session::new(config(Origin::Headless, false, DeviceAccess::Provider(provider.clone())));
    with_doc(&mut s);
    with_hack(&mut s);
    s.execute("tools.rectangle", json!({"xMm":0,"yMm":0,"wMm":1,"hMm":1})).unwrap();
    let old = s.active().unwrap().project.clone();
    assert!(s.active().unwrap().dirty());
    let spec = registry().find(ID).unwrap();
    assert_eq!(spec.access, Access::Agent);
    assert_eq!(spec.kind, Kind::Edit);
    s.execute_as(Origin::Synthetic, ID, grid()).unwrap();
    assert_eq!(s.active_index(), Some(1), "file.new semantics preserve the unsaved tab");
    assert!(s.active().unwrap().path.is_none());
    let m = jpt_m7_200_sim();
    let plan = openlaser_plan::plan(&s.active().unwrap().project, &m, &opts()).unwrap();
    openlaser_safety::validate(&plan, &m).unwrap();
    assert_eq!(provider.opens.load(Ordering::SeqCst), 0);
    assert_eq!(provider.records.load(Ordering::SeqCst), 0);
    assert_eq!(s.docs()[0].project, old);
}

#[test]
fn material_test_grbl_uses_current_profile_and_refuses_galvo_fields() {
    let mut cfg = config(Origin::Headless, false, DeviceAccess::None);
    cfg.machines.extend(openlaser_machine::om_gantries());
    let mut s = Session::new(cfg);
    s.execute("file.new", json!({"machine":"co2-grbl-600x400"})).unwrap();
    let mut p = grid();
    p["centerMm"] = json!([100, 100]);
    assert!(s.execute(ID, p.clone()).unwrap_err().to_string().contains("/cols/frequencyKhz"));
    p["cols"] = json!({"param":"speedMmS","min":10,"max":20,"count":5});
    p["base"] = json!({"qPulseNs":20});
    assert!(s.execute(ID, p.clone()).unwrap_err().to_string().contains("/base/qPulseNs"));
    p["base"] = json!({"intervalMm":0.2,"passes":2});
    p.as_object_mut().unwrap().remove("labelPowerPct");
    p.as_object_mut().unwrap().remove("labelSpeedMmS");
    s.execute(ID, p.clone()).unwrap();
    let m = s.machine_for(None).unwrap();
    assert_eq!(m.id.0, "co2-grbl-600x400");
    let project = &s.active().unwrap().project;
    assert_eq!(project.layers.colors[24].ops[0].gantry.power_max.get(), 5.0);
    assert_eq!(project.layers.colors[24].ops[0].gantry.speed.get(), 20.0);
    assert_eq!(project.layers.colors[25].ops[0].gantry.power_max.get(), m.layer_defaults.gantry.power_max.get(), "label defaults are independent of base");
    let plan = openlaser_plan::plan(project, m, &opts()).unwrap();
    openlaser_safety::validate(&plan, m).unwrap();
    p["centerMm"] = json!([0, 0]);
    assert!(s.execute(ID, p).unwrap_err().to_string().contains("work area"), "gantry design origin is respected");
}

#[test]
fn material_test_fit_includes_labels_and_machine_calibration() {
    let mut s = headless();
    with_hack(&mut s);
    let mut p = grid();
    p["rows"]["count"] = json!(1);
    p["cols"]["count"] = json!(1);
    p["centerMm"] = json!([-96.0, 0]);
    p["labels"] = json!(false);
    s.execute(ID, p.clone()).unwrap();
    let index = s.active_index();
    p["labels"] = json!(true);
    assert!(s.execute(ID, p).unwrap_err().to_string().contains("work area"));
    assert_eq!(s.active_index(), index);
    for rotated in [false, true] {
        let mut cfg = config(Origin::Headless, false, DeviceAccess::None);
        let openlaser_machine::Kinematics::Galvo(g) = &mut cfg.machines[0].kind else { panic!("galvo") };
        if rotated {
            g.cal.angle_deg = 45.0;
        } else {
            g.cal.offset_mm = [110.0, 0.0];
        }
        let mut s = Session::new(cfg);
        let mut p = grid();
        p["labels"] = json!(false);
        p["centerMm"] = json!([0, 0]);
        p["rows"]["count"] = json!(1);
        p["cols"]["count"] = json!(1);
        if rotated {
            p["cellWMm"] = json!(150);
            p["cellHMm"] = json!(150);
        }
        let e = s.execute(ID, p).unwrap_err();
        assert_eq!(e.code(), "bad_params");
        assert!(e.to_string().contains("work area"), "{e}");
        assert!(s.active().is_none());
    }
}
