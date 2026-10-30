//! Opt-in local visual verification; no customer artwork is embedded in the repository.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use openlaser_render::*;
use std::sync::Arc;

#[test]
#[ignore = "local sample only; set LBRN2_SAMPLE"]
fn real_lbrn2_preview() {
    let Ok(path) = std::env::var("LBRN2_SAMPLE") else {
        eprintln!("LBRN2_SAMPLE unset; skipped");
        return;
    };
    let bytes = std::fs::read(path).unwrap();
    let profile = openlaser_machine::jpt_m7_200_sim();
    let options = openlaser_import::ImportOptions { layer_defaults: Some(profile.layer_defaults.clone()), ..Default::default() };
    let imported = openlaser_import::import("sample.lbrn2", &bytes, &options).unwrap();
    if imported.shapes.len() > 1 {
        let color = |i| &imported.layers.iter().find(|(k, _)| *k == openlaser_types::LayerKey::Color(i)).unwrap().1;
        assert_eq!(color(0).ops[0].galvo.speed.get(), 550.0);
        assert_eq!(color(2).ops[0].galvo.frequency.khz(), 100.0);
        assert_eq!(color(6).ops[0].galvo.frequency, profile.layer_defaults.galvo.frequency);
        for i in [0, 2] {
            assert_eq!(color(i).image.mode, openlaser_doc::ImageMode::Stucki);
            assert_eq!(color(i).image.galvo.frequency, profile.layer_defaults.galvo.frequency);
            assert_eq!(color(i).image.galvo.q_pulse, profile.layer_defaults.galvo.q_pulse);
            assert!(!color(i).output);
        }
    }
    let text = std::str::from_utf8(&bytes).unwrap();
    // The thumbnail is an attribute on a single empty element. Extract without another XML dependency.
    let source = text.split("<Thumbnail").nth(1).unwrap().split("Source=\"").nth(1).unwrap().split('"').next().unwrap();
    let out = std::path::Path::new("/private/tmp/claude-501/-Users-luizbueno-AI-ClaudeCode/ba10990c-59a4-470b-ab69-9ee5e8b41d86/scratchpad/lbrn2");
    std::fs::create_dir_all(out).unwrap();
    std::fs::write(out.join("thumb.png"), openlaser_import::lbrn2::decode_png(source).unwrap()).unwrap();
    let mut project = Project::new(
        openlaser_doc::Workspace { kind: openlaser_types::DeviceKind::GalvoLmc, size_mm: [200., 200.], origin: openlaser_doc::WorkspaceOrigin::Center },
        &profile.layer_defaults,
    );
    for (key, layer) in imported.layers {
        if let openlaser_types::LayerKey::Color(i) = key {
            *project.color_layer_mut(i).unwrap() = layer;
        }
    }
    project.shapes = imported.shapes.into_iter().map(Arc::new).collect();
    let opts = ViewOpts { dark: false, selection: vec![], refusals: vec![], notices: vec![], show_field: false, show_work_area: false };
    let list = document_list(&project, None, &opts);
    let b = fit(&list, 0.0).unwrap();
    let side = (b.max[0] - b.min[0]).max(b.max[1] - b.min[1]);
    let center = [(b.max[0] + b.min[0]) / 2.0, (b.max[1] + b.min[1]) / 2.0];
    let world = Bounds { min: [center[0] - side / 2.0, center[1] - side / 2.0], max: [center[0] + side / 2.0, center[1] + side / 2.0] };
    eprintln!("full preview bounds: {world:?}");
    let render = |list: &DisplayList, world: Bounds, name: &str| {
        std::fs::write(out.join(name), render_png(list, &RenderView { world, max_side: 2048, background: Rgba([255, 255, 255, 255]) }).unwrap()).unwrap();
    };
    render(&list, world, "ours.png");
    // Main artwork detail (outlying attribution text otherwise dominates this sample's thumbnail).
    render(&list, Bounds { min: [-300., -200.], max: [550., 350.] }, "ours-detail.png");
    // Render each attribution outline at a readable scale to settle Y direction and mirroring.
    fn find_text(shapes: &[Arc<openlaser_doc::Shape>], parent: openlaser_geom::Affine, out: &mut Vec<Arc<openlaser_doc::Shape>>) {
        for s in shapes {
            let xf = parent * s.xform;
            if let openlaser_doc::Geom::Group(children) = &s.geom {
                find_text(children, xf, out);
            } else if s.name.as_deref() == Some("Created by Olena Panasovska") {
                let mut shape = s.as_ref().clone();
                shape.xform = xf;
                out.push(Arc::new(shape));
            }
        }
    }
    let mut text = Vec::new();
    find_text(&project.shapes, openlaser_geom::Affine::IDENTITY, &mut text);
    for (i, shape) in text.into_iter().enumerate() {
        project.shapes = vec![shape];
        let mut list = document_list(&project, None, &opts);
        list.items.retain(|item| !matches!(item, Item::Marker { .. }));
        render(&list, fit(&list, 0.05).unwrap(), &format!("ours-text-{i}.png"));
    }
}
