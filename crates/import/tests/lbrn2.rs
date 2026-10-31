#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use base64::Engine;
use openlaser_doc::{CutMode, FillParams, GalvoParams, GantryParams, Geom, ImageMode, LayerDefaults};
use openlaser_geom::{Affine, PathEl, Point};
use openlaser_import::{Error, Format, ImportOptions, Imported, import, sniff};
use openlaser_types::{Deg, Hz, LayerKey, Mm, MmPerS, Ns, Percent};
use std::sync::Arc;

fn options() -> ImportOptions {
    ImportOptions {
        layer_defaults: Some(LayerDefaults {
            galvo: GalvoParams {
                speed: MmPerS::new(777.0).unwrap(),
                power: Percent::new(23.0).unwrap(),
                min_power: Percent::new(2.0).unwrap(),
                frequency: Hz::new(123000).unwrap(),
                q_pulse: Some(Ns::new(45.0).unwrap()),
                timings: None,
                wobble: None,
            },
            gantry: GantryParams {
                speed: MmPerS::new(42.0).unwrap(),
                power_max: Percent::new(18.0).unwrap(),
                power_min: Percent::new(1.0).unwrap(),
                constant_power: false,
            },
            fill: FillParams {
                interval: Mm::new(0.2).unwrap(),
                angle: Deg::new(15.0).unwrap(),
                angle_increment: Deg::new(0.0).unwrap(),
                bidirectional: true,
                cross_hatch: false,
            },
        }),
        ..ImportOptions::default()
    }
}
fn parse(s: &str) -> Imported {
    import("test.lbrn2", s.as_bytes(), &options()).unwrap()
}
fn fixture(name: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/lbrn2").join(name)).unwrap()
}
fn layer(p: &Imported, i: u8) -> &openlaser_doc::ColorLayer {
    &p.layers.iter().find(|(k, _)| *k == LayerKey::Color(i)).unwrap().1
}
fn path(p: &Imported, i: usize) -> &openlaser_doc::BezPath {
    let Geom::Path(v) = &p.shapes[i].geom else { panic!("path") };
    v
}

#[test]
fn shapes_exact_sharing_groups_text_and_notices() {
    let p = parse(&fixture("shapes.lbrn2"));
    assert_eq!(p.shapes.len(), 7);
    assert_eq!(p.shapes[0].xform, Affine::new([-1., 0., 0., 1., 20., 30.]));
    assert_eq!(p.shapes[0].layer, LayerKey::Color(2));
    assert!(matches!(p.shapes[0].geom,Geom::Rect { w,h,corner_radius } if (w.get(),h.get(),corner_radius.get()) == (10.,6.,1.)));
    assert!(matches!(p.shapes[1].geom,Geom::Ellipse { rx,ry } if (rx.get(),ry.get()) == (4.,2.)));
    let expected = [
        PathEl::MoveTo(Point::new(0., 0.)),
        PathEl::CurveTo(Point::new(2., 3.), Point::new(6., 3.), Point::new(8., 0.)),
        PathEl::LineTo(Point::new(8., 4.)),
        PathEl::LineTo(Point::new(0., 0.)),
        PathEl::ClosePath,
    ];
    assert_eq!(path(&p, 2).elements(), expected);
    assert_eq!(path(&p, 2), path(&p, 3));
    assert!(matches!(path(&p, 4).elements().last(), Some(PathEl::ClosePath)));
    let group = &p.shapes[5];
    let Geom::Group(g) = &group.geom else { panic!("group") };
    let Geom::Group(g2) = &g[0].geom else { panic!("group") };
    let xf = group.xform * g[0].xform * g2[0].xform;
    assert_eq!(xf * Point::ORIGIN, Point::new(2., 25.));
    assert_eq!(p.shapes[6].xform * Point::ORIGIN, Point::new(5., 7.));
    let messages = p.notices.iter().map(|n| n.message.as_str()).collect::<Vec<_>>().join("\n");
    for name in ["Missing outline", "fonts", "Type=Star (2)", "VirtualArray", "SubLayer"] {
        assert!(messages.contains(name), "{name}: {messages}");
    }
}
#[test]
fn collapsed_controls_and_disconnected_subpaths() {
    let p = parse(
        r#"<LightBurnProject><Shape Type="Path"><VertList>V2 3c0x1c1x1V4 5c0x1c1x1V8 9V10 11</VertList><PrimList>B0 1L2 3</PrimList></Shape></LightBurnProject>"#,
    );
    assert_eq!(
        path(&p, 0).elements(),
        [
            PathEl::MoveTo((2., 3.).into()),
            PathEl::CurveTo((2., 3.).into(), (4., 5.).into(), (4., 5.).into()),
            PathEl::MoveTo((8., 9.).into()),
            PathEl::LineTo((10., 11.).into())
        ]
    );
    let p = parse(r#"<LightBurnProject><Shape Type="Path"><VertList>V2 3c0x1c0y7V4 5c1x1c1y8</VertList><PrimList>B0 1</PrimList></Shape></LightBurnProject>"#);
    assert!(matches!(path(&p,0).elements()[1],PathEl::CurveTo(a,b,_) if a == Point::new(1.,7.) && b == Point::new(1.,8.)));
}
#[test]
fn layers_all_fields_defaults_disabled_modes_and_order() {
    let p = parse(&fixture("settings.lbrn2"));
    assert_eq!(p.layers[0].0, LayerKey::Color(6));
    let l = layer(&p, 2);
    assert_eq!(l.name, "Synthetic");
    assert!(l.output);
    assert_eq!(l.global_passes, 4);
    assert_eq!(l.ops.len(), 2);
    let op = &l.ops[0];
    assert_eq!(op.mode, CutMode::Fill);
    assert_eq!(l.ops[1].mode, CutMode::Line);
    assert_eq!(op.passes, 3);
    assert_eq!(op.galvo.speed.get(), 550.);
    assert_eq!(op.gantry.speed.get(), 550.);
    assert_eq!(op.galvo.power.get(), 65.);
    assert_eq!(op.gantry.power_max.get(), 65.);
    assert_eq!(op.gantry.power_min.get(), 12.);
    assert_eq!(op.galvo.min_power.get(), 12.);
    assert_eq!(op.galvo.frequency.khz(), 25.);
    assert_eq!(op.galvo.q_pulse.unwrap().get(), 60.);
    assert_eq!(op.fill.interval.get(), 0.15);
    assert_eq!(op.fill.angle.get(), 135.);
    assert!(op.fill.cross_hatch);
    assert!(!op.fill.bidirectional);
    assert_eq!(l.image.mode, ImageMode::Stucki);
    assert_eq!(l.image.galvo.speed.get(), 100.);
    assert_eq!(l.image.galvo.power.get(), 20.);
    assert_eq!(l.image.galvo.frequency.khz(), 50.);
    assert_eq!(l.image.galvo.q_pulse.unwrap().get(), 80.);
    assert_eq!(l.image.gantry.power_min.get(), 5.);
    assert_eq!(l.image.interval.get(), 0.05);
    assert_eq!(l.image.angle.get(), 45.);
    assert!(l.image.bidirectional);
    assert_eq!(l.image.passes, 2);
    assert_eq!(layer(&p, 6).ops[0].mode, CutMode::OffsetFill);
    assert_eq!(layer(&p, 6).ops[0].galvo, options().layer_defaults.unwrap().galvo);
    for i in [7, 8, 9] {
        assert!(!layer(&p, i).output);
    }
    assert_eq!(p.notices.iter().filter(|n| n.layer == Some(LayerKey::Color(6)) && n.message.contains("defaults")).count(), 1);
    assert!(p.notices.iter().any(|n| n.message.contains("tabsEnabled") && n.message.contains("maxPower2") && n.message.contains("tabCount")));
}
#[test]
fn dithering_names_and_invalid_setting_ranges() {
    for (name, expected) in [
        ("threshold", ImageMode::Threshold),
        ("jarvis", ImageMode::Jarvis),
        ("stucki", ImageMode::Stucki),
        ("floyd", ImageMode::FloydSteinberg),
        ("dither", ImageMode::FloydSteinberg),
        ("atkinson", ImageMode::Atkinson),
        ("ordered", ImageMode::Ordered),
    ] {
        let p = parse(&format!(
            r#"<LightBurnProject><CutSetting_Img type="Image"><index Value="0"/><ditherMode Value="{name}"/></CutSetting_Img></LightBurnProject>"#
        ));
        assert_eq!(layer(&p, 0).image.mode, expected);
        assert!(layer(&p, 0).output);
    }
    for (key, value) in [
        ("frequency", "0"),
        ("frequency", "25000.5"),
        ("frequency", "10000001"),
        ("QPulseWidth", "0"),
        ("interval", "101"),
        ("interval", "0"),
        ("numPasses", "1000"),
        ("globalRepeat", "0"),
        ("angle", "NaN"),
        ("bidir", "2"),
        ("minPower", "-1"),
        ("speed", "100001"),
    ] {
        let p = parse(&format!(r#"<LightBurnProject><CutSetting type="Cut"><index Value="0"/><{key} Value="{value}"/></CutSetting></LightBurnProject>"#));
        assert!(!layer(&p, 0).output, "{key}");
        assert!(p.notices.iter().any(|n| n.message.contains(key) && n.message.contains("Output OFF")));
    }
}
#[test]
fn bitmap_png_alpha_size_and_placement() {
    let img = image::RgbaImage::from_raw(2, 1, vec![0, 0, 0, 0, 0, 0, 0, 128]).unwrap();
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img).write_to(&mut png, image::ImageFormat::Png).unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let p = parse(&format!(
        r#"<LightBurnProject><Shape Type="Bitmap" CutIndex="3" W="12" H="6" Data="{encoded}"><XForm>1 0 0 1 8 10</XForm></Shape></LightBurnProject>"#
    ));
    let Geom::Bitmap(b) = &p.shapes[0].geom else { panic!("bitmap") };
    assert_eq!(b.px, [2, 1]);
    assert_eq!(b.size_mm, [12., 6.]);
    assert_eq!(b.gray.as_ref(), [255, 127]);
    assert_eq!(p.bounds.unwrap().min, [2., 7.]);
    assert!(!p.notices.iter().any(|n| n.message.contains("DPI")));
}
#[test]
fn sniff_placement_and_mandatory_profile_defaults() {
    let s = fixture("shapes.lbrn2");
    assert_eq!(sniff("x.LBRN2", b""), Some(Format::Lbrn2));
    assert_eq!(sniff("x", s.as_bytes()), Some(Format::Lbrn2));
    assert!(import("x.lbrn2", s.as_bytes(), &ImportOptions::default()).is_err());
    let opts = ImportOptions { center_at: Some([0., 0.]), scale: 2., ..options() };
    let p = import("x.lbrn2", s.as_bytes(), &opts).unwrap();
    let b = p.bounds.unwrap();
    assert!((b.min[0] + b.max[0]).abs() < 1e-8);
    assert!((b.min[1] + b.max[1]).abs() < 1e-8);
    let opts = ImportOptions { layer: Some(LayerKey::Color(9)), ..options() };
    let p = import("x.lbrn2", s.as_bytes(), &opts).unwrap();
    assert!(p.shapes.iter().all(|s| s.layer == LayerKey::Color(9)));
}
#[test]
fn malformed_size_depth_shape_and_geometry_caps() {
    let o = options();
    let huge = vec![b' '; openlaser_import::MAX_VECTOR_FILE_BYTES + 1];
    assert!(matches!(import("x.lbrn2", &huge, &o), Err(Error::TooLarge(_))));
    let depth = format!("<LightBurnProject>{}{}</LightBurnProject>", "<Children>".repeat(65), "</Children>".repeat(65));
    assert!(matches!(import("x.lbrn2", depth.as_bytes(), &o), Err(Error::TooLarge(_))));
    let count = format!("<LightBurnProject>{}</LightBurnProject>", "<Shape Type=\"Unknown\"/>".repeat(openlaser_import::lbrn2::MAX_SHAPES + 1));
    assert!(matches!(import("x.lbrn2", count.as_bytes(), &o), Err(Error::TooLarge(_))));
    for body in [
        "<!DOCTYPE LightBurnProject [<!ENTITY x 'hello'>]><LightBurnProject>&x;</LightBurnProject>",
        "<LightBurnProject><Shape Type='Path'><VertList>V0 0</VertList><PrimList>L0 9</PrimList></Shape></LightBurnProject>",
        "<LightBurnProject><Shape Type='Rect' W='NaN' H='1'/></LightBurnProject>",
    ] {
        assert!(import("x.lbrn2", body.as_bytes(), &o).is_err());
    }
    let verts = format!(
        "<LightBurnProject><Shape Type='Path'><VertList>{}</VertList><PrimList>LineClosed</PrimList></Shape></LightBurnProject>",
        "V0 0".repeat(openlaser_import::MAX_ENTITY_POINTS + 1)
    );
    assert!(matches!(import("x.lbrn2", verts.as_bytes(), &o), Err(Error::TooLarge(_))));
}
#[test]
fn truncated_and_garbage_never_panic() {
    let text = fixture("shapes.lbrn2");
    for i in 0..text.len() {
        let _ = import("x.lbrn2", &text.as_bytes()[..i], &options());
    }
}
proptest::proptest! {
    #[test]
    fn arbitrary_bytes_and_path_tokens_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(),0..4096), tokens in ".{0,256}") {
        let _ = import("x.lbrn2",&bytes,&options());
        let doc = format!("<LightBurnProject><Shape Type='Path'><VertList>{tokens}</VertList><PrimList>B0 1</PrimList></Shape></LightBurnProject>");
        let _ = import("x.lbrn2",doc.as_bytes(),&options());
    }
}

#[test]
#[ignore = "local user sample only; set LBRN2_SAMPLE"]
fn real_sample_counts_layers_and_notices() {
    let Ok(path) = std::env::var("LBRN2_SAMPLE") else {
        eprintln!("LBRN2_SAMPLE unset; skipped");
        return;
    };
    let bytes = std::fs::read(path).unwrap();
    let p = import("sample.lbrn2", &bytes, &options()).unwrap();
    let mut counts = [0; 5];
    let mut stack: Vec<_> = p.shapes.iter().collect();
    while let Some(s) = stack.pop() {
        match &s.geom {
            Geom::Path(_) => counts[0] += 1,
            Geom::Rect { .. } => counts[1] += 1,
            Geom::Ellipse { .. } => counts[2] += 1,
            Geom::Bitmap(_) => counts[3] += 1,
            Geom::Group(g) => {
                counts[4] += 1;
                stack.extend(g.iter().map(Arc::as_ref));
            }
            _ => panic!("unexpected shape"),
        }
    }
    if counts[0] > 0 {
        assert_eq!(counts, [276, 26, 26, 2, 13]);
        let c0 = layer(&p, 0);
        assert_eq!(c0.ops[0].galvo.speed.get(), 550.);
        assert_eq!(c0.ops[0].galvo.frequency.khz(), 25.);
        assert_eq!(c0.global_passes, 4);
        assert_eq!(c0.image.mode, ImageMode::Stucki);
        assert_eq!(c0.image.galvo.speed.get(), 100.);
        let c2 = layer(&p, 2);
        assert_eq!(c2.ops[0].galvo.speed.get(), 900.);
        assert_eq!(c2.ops[0].galvo.frequency.khz(), 100.);
        assert_eq!(c2.ops[0].galvo.power.get(), 60.);
        assert_eq!(c2.image.mode, ImageMode::Stucki);
        assert_eq!(layer(&p, 6).ops[0].mode, CutMode::OffsetFill);
    } else {
        assert_eq!(counts, [0, 1, 0, 0, 0]);
    }
    eprintln!("counts={counts:?}; top-level={}; notices={}", p.shapes.len(), p.notices.len());
    for n in p.notices {
        eprintln!("{}: {}", n.n, n.message);
    }
}

#[test]
fn bitmap_side_limit_and_reused_geometry_total_budget() {
    let img = image::GrayImage::new(openlaser_import::MAX_RASTER_SIDE + 1, 1);
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(img).write_to(&mut png, image::ImageFormat::Png).unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let xml = format!(r#"<LightBurnProject><Shape Type="Bitmap" W="1" H="1" Data="{data}"/></LightBurnProject>"#);
    assert!(matches!(import("x.lbrn2", xml.as_bytes(), &options()), Err(Error::TooLarge(_))));
    let mut xml = format!(
        r#"<LightBurnProject><Shape Type="Path" VertID="v" PrimID="p"><VertList>{}</VertList><PrimList>LineClosed</PrimList></Shape>"#,
        "V0 0".repeat(200)
    );
    xml.push_str(&r#"<Shape Type="Path" VertID="v" PrimID="p"/>"#.repeat(4000));
    xml.push_str("</LightBurnProject>");
    assert!(matches!(import("x.lbrn2", xml.as_bytes(), &options()), Err(Error::TooLarge(_))));
}
#[test]
fn conflicting_image_priority_keeps_the_layer_with_a_notice_and_unknown_children_are_counted() {
    let p = parse(
        r#"<LightBurnProject><CutSetting type="Cut"><index Value="0"/><priority Value="1"/></CutSetting><CutSetting_Img type="Image"><index Value="0"/><priority Value="2"/></CutSetting_Img><Shape Type="Rect" W="1" H="1"><Array/></Shape></LightBurnProject>"#,
    );
    // Priority only orders layers (it never changes what burns): the layer stays on, the notice says which won.
    assert!(layer(&p, 0).output);
    assert!(p.notices.iter().any(|n| n.message.contains("vector/image priorities differ (1, 2)") && n.message.contains("uses the earlier one")));
    assert!(p.notices.iter().any(|n| n.message.contains("Skipped Rect element Array (1)")));
}

#[test]
fn independent_repeat_counts_and_image_cross_hatch_are_not_silently_changed() {
    let p = parse(
        r#"<LightBurnProject><CutSetting type="Cut"><index Value="0"/><globalRepeat Value="4"/></CutSetting><CutSetting_Img type="Image"><index Value="0"/><globalRepeat Value="2"/><crossHatch Value="1"/></CutSetting_Img></LightBurnProject>"#,
    );
    assert!(!layer(&p, 0).output);
    assert_eq!(layer(&p, 0).global_passes, 4);
    assert!(p.notices.iter().any(|n| n.message.contains("global passes (4, 2)")));
    assert!(p.notices.iter().any(|n| n.message.contains("image crossHatch is unsupported")));
}
