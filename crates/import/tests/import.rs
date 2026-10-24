//! Importer acceptance tests (SPEC §6.6, §7.3) and the regression tests of the WP2 review.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;

use openlaser_doc::{CutMode, Geom, PALETTE};
use openlaser_geom::PathEl;
use openlaser_import::om::{self, GANTRY_DEFAULT_POWER_PCT, GANTRY_DEFAULT_SPEED_MM_S};
use openlaser_import::{DxfUnits, Error, ImportOptions, Imported, LayerKey, MAX_SVG_DEPTH, MAX_VECTOR_FILE_BYTES, Shape, SvgUnits, dxf_om, import, sniff};
use openlaser_types::NoticeCode;

fn svg_doc(attrs: &str, body: &str) -> Vec<u8> {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" {attrs}>{body}</svg>"#).into_bytes()
}

fn svg_mm(body: &str) -> Vec<u8> {
    svg_doc(r#"width="20mm" height="20mm" viewBox="0 0 20 20""#, body)
}

/// Every placed point of a path shape (control points included).
fn shape_points(shape: &Shape) -> Vec<[f64; 2]> {
    let Geom::Path(path) = &shape.geom else { return Vec::new() };
    let mut out = Vec::new();
    for element in path.elements() {
        let points = match element {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => vec![*p],
            PathEl::QuadTo(a, b) => vec![*a, *b],
            PathEl::CurveTo(a, b, c) => vec![*a, *b, *c],
            PathEl::ClosePath => vec![],
        };
        out.extend(points.into_iter().map(|p| {
            let q = shape.xform * p;
            [q.x, q.y]
        }));
    }
    out
}

fn notice_codes(imported: &Imported) -> Vec<NoticeCode> {
    imported.notices.iter().map(|n| n.code).collect()
}

fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(2 * 1024 * 1024).spawn(f).expect("spawn").join().expect("no stack overflow")
}

// ---- SPEC §7.3 acceptance ----------------------------------------------------------------------------------------

#[test]
fn svg_true_scale_mm() {
    let bytes = std::fs::read(openlaser_testkit::fixture("square10mm.svg")).unwrap();
    let imported = import("square10mm.svg", &bytes, &ImportOptions::default()).expect("valid SVG");
    let bounds = imported.bounds.expect("path bounds");
    assert!((bounds.max[0] - bounds.min[0] - 10.0).abs() < 1e-6, "bounds: {bounds:?}");
    assert!((bounds.max[1] - bounds.min[1] - 10.0).abs() < 1e-6);
    // File coordinates kept (SVG Y down becomes design Y up): x 5..15, y -15..-5.
    assert!((bounds.min[0] - 5.0).abs() < 1e-6 && (bounds.max[1] + 5.0).abs() < 1e-6, "bounds: {bounds:?}");
}

#[test]
fn svg_unitless_refused() {
    let bytes = std::fs::read(openlaser_testkit::fixture("unitless.svg")).unwrap();
    assert!(matches!(import("unitless.svg", &bytes, &ImportOptions::default()), Err(Error::Unitless(_))));
    let px96 = import("unitless.svg", &bytes, &ImportOptions { svg_units: SvgUnits::Px96, ..ImportOptions::default() }).expect("96 px/in");
    let px72 = import("unitless.svg", &bytes, &ImportOptions { svg_units: SvgUnits::Px72, ..ImportOptions::default() }).expect("72 px/in");
    let width = |i: &Imported| i.bounds.map(|b| b.max[0] - b.min[0]).unwrap();
    // The 50-unit square with a 1-unit stroke centred on it: 50 px.
    assert!((width(&px96) - 50.0 * 25.4 / 96.0).abs() < 1e-6);
    assert!((width(&px72) - 50.0 * 25.4 / 72.0).abs() < 1e-6);
}

/// Review item 5: only the root `<svg>`'s own width and height count; a child's `width="…mm"` or a
/// `stroke-width` never makes a unitless file physical.
#[test]
fn svg_unitless_check_reads_the_root_element_only() {
    let child_has_units = svg_doc(r#"viewBox="0 0 100 100""#, r##"<rect x="0" y="0" width="50mm" height="50mm" stroke="#000000"/>"##);
    assert!(matches!(import("child.svg", &child_has_units, &ImportOptions::default()), Err(Error::Unitless(_))));
    let stroke_width_first = svg_doc(r#"stroke-width="2mm" viewBox="0 0 100 100" width="100" height="100""#, r##"<path d="M0 0L9 9" stroke="#000000"/>"##);
    assert!(matches!(import("stroke.svg", &stroke_width_first, &ImportOptions::default()), Err(Error::Unitless(_))));
    let half = svg_doc(r#"width="100mm" height="100""#, r##"<path d="M0 0L9 9" stroke="#000000"/>"##);
    assert!(matches!(import("half.svg", &half, &ImportOptions::default()), Err(Error::Unitless(_))));
    let root_mm = svg_doc(r#"width="10mm" height="1in" viewBox="0 0 10 25.4""#, r##"<rect width="5" height="5" stroke="#000000"/>"##);
    assert!(import("root.svg", &root_mm, &ImportOptions::default()).is_ok());
}

#[test]
fn svg_colour_to_palette_layer() {
    let hex = |rgb: [u8; 3]| format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
    let current = LayerKey::Color(7);
    let opts = ImportOptions { current_layer: current, ..ImportOptions::default() };
    // Exact palette colours land on the first palette entry with that colour (stroke first, else fill).
    for index in [0usize, 1, 5, 29] {
        let colour = PALETTE[index].0;
        let expected = LayerKey::Color(u8::try_from(PALETTE.iter().position(|c| c.0 == colour).unwrap()).unwrap());
        let stroke = svg_mm(&format!(r#"<path d="M1 1L9 9" stroke="{}" fill="none"/>"#, hex(colour)));
        let fill = svg_mm(&format!(r#"<path d="M1 1L9 9L1 9Z" fill="{}"/>"#, hex(colour)));
        for bytes in [stroke, fill] {
            let imported = import("p.svg", &bytes, &opts).unwrap();
            assert_eq!(imported.shapes[0].layer, expected, "palette {index}");
            assert!(!notice_codes(&imported).contains(&NoticeCode::ImportColorUnmatched));
        }
    }
    // A non-black palette colour maps to its own layer (once WP1 lands the documented palette; the WP0 stub is all
    // black, so then the test exercises a non-black colour that is not in the palette).
    if let Some((index, colour)) = PALETTE.iter().enumerate().find(|(_, c)| c.0 != [0, 0, 0]) {
        let first = PALETTE.iter().position(|c| c == colour).unwrap();
        let imported = import("p.svg", &svg_mm(&format!(r#"<path d="M1 1L9 9" stroke="{}"/>"#, hex(colour.0))), &opts).unwrap();
        assert_eq!(imported.shapes[0].layer, LayerKey::Color(u8::try_from(first).unwrap()), "palette {index}");
    }
    // Colours outside the palette go to the current layer, with ONE aggregated notice naming them.
    let off: Vec<[u8; 3]> = [[0x12, 0x34, 0x57], [0xfe, 0x01, 0x02], [0x01, 0xfe, 0x03]].into_iter().filter(|c| !PALETTE.iter().any(|p| p.0 == *c)).collect();
    assert!(!off.is_empty());
    let body: String = off.iter().map(|c| format!(r#"<path d="M1 1L9 9" stroke="{}"/>"#, hex(*c))).collect();
    let imported = import("off.svg", &svg_mm(&body), &opts).unwrap();
    assert!(imported.shapes.iter().all(|s| s.layer == current));
    let notices: Vec<_> = imported.notices.iter().filter(|n| n.code == NoticeCode::ImportColorUnmatched).collect();
    assert_eq!(notices.len(), 1, "{:?}", imported.notices);
    assert!(notices[0].message.starts_with(&format!("{} SVG paths", off.len())), "{}", notices[0].message);
    assert!(off.iter().all(|c| notices[0].message.contains(&hex(*c))));
    assert_eq!(notices[0].layer, Some(current));
    // A forced layer wins over the palette and raises no colour notice.
    let forced = import("off.svg", &svg_mm(&body), &ImportOptions { layer: Some(LayerKey::Color(3)), ..opts.clone() }).unwrap();
    assert!(forced.shapes.iter().all(|s| s.layer == LayerKey::Color(3)));
    assert!(!notice_codes(&forced).contains(&NoticeCode::ImportColorUnmatched));
}

/// B9: a point that is not exactly [x, y] refuses the whole import, naming the object.
#[test]
fn import_malformed_points_refused() {
    for bad in ["[0,1,2]", "[0]", "[]", "5", "null", "{\"x\":1}", "[0,1,[2]]"] {
        let ws = format!(
            r#"{{"version":1,"objects":[{{"name":"good","layer_id":"vector-engrave","polylines":[[[0,0],[1,1]]],"x":0,"y":0,"scale":1,"rotation":0}},
            {{"layer_id":"vector-engrave","polylines":[[[0,0],{bad}]],"x":0,"y":0,"scale":1,"rotation":0,"name":"bad-shape"}}]}}"#
        );
        match om::parse_workspace(ws.as_bytes()) {
            Err(Error::MalformedPoint { object, .. }) => assert_eq!(object, "bad-shape", "{bad}"),
            other => panic!("{bad}: expected a B9 refusal, got {other:?}"),
        }
        assert!(matches!(import("workspace.json", ws.as_bytes(), &ImportOptions::default()), Err(Error::MalformedPoint { .. })), "{bad}");
    }
    // A built workspace (not parsed) is refused by to_imported too.
    let mut ws =
        om::parse_workspace(br#"{"version":1,"objects":[{"name":"o","layer_id":"raster","polylines":[[[0,0],[1,1]]],"x":0,"y":0,"scale":1,"rotation":0}]}"#)
            .unwrap();
    ws.objects[0].polylines[0][1] = vec![1.0, 1.0, 1.0];
    assert!(matches!(om::to_imported(&ws, &om::builtin_layers()), Err(Error::MalformedPoint { object, .. }) if object == "o"));
}

/// SPEC §7.3: 403 objects, 3057 points, layers per Appendix B.
#[test]
fn om_workspace_business_card_imports() {
    let root = openlaser_testkit::golden_dir().join("inputs");
    let workspace = std::fs::read(root.join("workspace.json")).unwrap();
    let settings = std::fs::read(root.join("layer_settings.json")).unwrap();
    let ws = om::parse_workspace(&workspace).expect("workspace parses");
    let layers = om::parse_layer_settings(&settings).expect("settings parse");
    let imported = om::to_imported(&ws, &layers).expect("business card imports");
    assert_eq!(imported.shapes.len(), 403);
    assert!(imported.notices.is_empty(), "{:?}", imported.notices);
    // Every point arrives, one subpath per OM polyline, with the object's name and Appendix B layer.
    let points: usize = imported.shapes.iter().map(|s| shape_points(s).len()).sum();
    assert_eq!(points, 3057);
    let mut per_layer: BTreeMap<LayerKey, usize> = BTreeMap::new();
    for (shape, object) in imported.shapes.iter().zip(&ws.objects) {
        assert_eq!(shape.name.as_deref(), Some(object.name.as_str()));
        assert!(matches!(shape.geom, Geom::Path(_)), "Appendix B: Geom::Path");
        let expected = match object.layer_id.as_str() {
            "vector-engrave" => LayerKey::Color(0),
            "raster" => LayerKey::Color(2),
            other => panic!("unexpected layer {other}"),
        };
        assert_eq!(shape.layer, expected);
        *per_layer.entry(shape.layer).or_default() += 1;
        let first = &object.polylines[0][0];
        assert_eq!(shape_points(shape)[0], [first[0], first[1]], "identity transform keeps OM coordinates");
    }
    assert_eq!(per_layer, BTreeMap::from([(LayerKey::Color(0), 304), (LayerKey::Color(2), 99)]));
    let bounds = imported.bounds.expect("bounds");
    for (got, want) in
        [bounds.min[0], bounds.max[0], bounds.min[1], bounds.max[1]].into_iter().zip([9.393671609715117, 48.580000000000005, -13.412474384625824, 44.35])
    {
        assert!((got - want).abs() < 1e-9, "bounds {bounds:?}");
    }
    // Layers: C00, C01, C02 from the file (T1 frame-only is a tool layer and carries no settings).
    let keys: Vec<LayerKey> = imported.layers.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![LayerKey::Color(0), LayerKey::Color(1), LayerKey::Color(2)]);
    let expect = [
        ("Vector Engrave", CutMode::Line, 1000.0, 30_000u32),
        ("Vector Cut / Deep Mark", CutMode::Line, 300.0, 30_000),
        ("Raster", CutMode::Fill, 300.0, 45_000),
    ];
    for ((key, layer), (name, mode, speed, hz)) in imported.layers.iter().zip(expect) {
        let op = &layer.ops[0];
        assert_eq!((layer.name.as_str(), op.mode, layer.output, layer.show), (name, mode, true, true), "{key}");
        assert_eq!(op.galvo.speed.get(), speed, "{key}");
        assert_eq!(op.galvo.power.get(), 100.0, "{key}");
        assert_eq!(u32::from(op.galvo.frequency), hz, "{key}");
        assert_eq!(op.galvo.q_pulse.map(|q| q.get()), Some(200.0), "{key}");
        assert_eq!(op.passes, 1);
    }
    let raster = &imported.layers[2].1.ops[0].fill;
    assert_eq!(
        (raster.interval.get(), raster.angle.get(), raster.angle_increment.get(), raster.bidirectional, raster.cross_hatch),
        (0.1, 90.0, 0.0, true, false)
    );
}

/// B11: OM's MOPA speed/power go to the galvo block only; gantry blocks never copy them.
#[test]
fn om_import_never_copies_galvo_values_into_gantry_blocks() {
    let root = openlaser_testkit::golden_dir().join("inputs");
    let ws = om::parse_workspace(&std::fs::read(root.join("workspace.json")).unwrap()).unwrap();
    let layers = om::parse_layer_settings(&std::fs::read(root.join("layer_settings.json")).unwrap()).unwrap();
    let imported = om::to_imported(&ws, &layers).unwrap();
    assert_eq!(imported.layers.len(), 3);
    for (key, layer) in &imported.layers {
        for (gantry, galvo_speed, galvo_power) in [
            (&layer.ops[0].gantry, layer.ops[0].galvo.speed.get(), layer.ops[0].galvo.power.get()),
            (&layer.image.gantry, layer.image.galvo.speed.get(), layer.image.galvo.power.get()),
        ] {
            assert_eq!(gantry.speed.get(), GANTRY_DEFAULT_SPEED_MM_S, "{key}");
            assert_eq!(gantry.power_max.get(), GANTRY_DEFAULT_POWER_PCT, "{key}");
            assert_ne!(gantry.speed.get(), galvo_speed, "{key}: gantry speed copied from galvo");
            assert_ne!(gantry.power_max.get(), galvo_power, "{key}: gantry power copied from galvo");
        }
    }
}

#[test]
fn om_fixture_import_preserves_objects_and_layers() {
    let root = openlaser_testkit::fixture("om_small");
    let ws = om::parse_workspace(&std::fs::read(root.join("workspace.json")).unwrap()).unwrap();
    let layers = om::parse_layer_settings(&std::fs::read(root.join("layer_settings.json")).unwrap()).unwrap();
    let imported = om::to_imported(&ws, &layers).unwrap();
    assert_eq!(imported.shapes.len(), 3);
    assert_eq!(imported.layers.len(), 3);
}

#[test]
fn om_workspace_without_layer_settings_uses_builtins_with_a_notice() {
    let bytes = std::fs::read(openlaser_testkit::fixture("om_small").join("workspace.json")).unwrap();
    let imported = import("workspace.json", &bytes, &ImportOptions::default()).unwrap();
    assert_eq!(imported.notices.len(), 1);
    assert!(imported.notices[0].message.contains("layer_settings.json"));
    assert_eq!(imported.notices[0].n, 1);
}

// ---- review items 3, 4, 6: SVG hardening -------------------------------------------------------------------------

/// The depth refusal: `TooLarge` naming the nesting limit (anything else means the file was refused for another
/// reason, or not at all).
fn depth_refusal(name: &'static str, bytes: Vec<u8>) -> Result<String, String> {
    on_small_stack(move || match import(name, &bytes, &ImportOptions::default()) {
        Err(Error::TooLarge(message)) if message.contains(&format!("nested more than {MAX_SVG_DEPTH} deep")) => Ok(message),
        other => Err(format!("{other:?}")),
    })
}

/// Review item 4: 20,000 nested `<g>` (~60 KB) aborted the process (stack overflow). Now refused before roxmltree
/// and usvg, on a 2 MiB thread stack.
#[test]
fn svg_deep_nesting_refused() {
    let deep = svg_mm(&format!("{}{}", "<g>".repeat(20_000), "</g>".repeat(20_000)));
    let result = depth_refusal("deep.svg", deep);
    assert!(result.is_ok(), "{result:?}");
    // At the limit (the root, 254 groups and the path) still imports on the same stack.
    let levels = MAX_SVG_DEPTH - 2;
    let ok = svg_mm(&format!(r##"{}<path d="M1 1L9 9" stroke="#000000"/>{}"##, "<g>".repeat(levels), "</g>".repeat(levels)));
    let result = on_small_stack(move || import("ok.svg", &ok, &ImportOptions::default()).map(|i| i.shapes.len()).map_err(|e| e.to_string()));
    assert_eq!(result, Ok(1));
}

/// Nesting hidden in an internal entity: the root plus 256 levels in the entity is one level too deep. No entity is
/// ever expanded (an internal subset is always refused, before any parser runs); the refusal names the depth.
#[test]
fn svg_entity_nesting_refused() {
    let levels = MAX_SVG_DEPTH;
    let entity = format!("{}{}", "<g>".repeat(levels), "</g>".repeat(levels));
    let doc = format!(r#"<!DOCTYPE svg [<!ENTITY deep "{entity}">]><svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm">&deep;</svg>"#);
    let result = depth_refusal("entity.svg", doc.into_bytes());
    assert!(result.as_ref().is_ok_and(|m| m.contains("DOCTYPE entity")), "{result:?}");
}

/// A `<use>` fan-out ("billion laughs" for SVG) is bounded before usvg expands it; a recursive `<use>` refuses.
#[test]
fn svg_use_expansion_is_bounded() {
    let mut defs = String::from(r##"<path id="l0" d="M0 0L1 1" stroke="#000000"/>"##);
    for level in 1..=8 {
        defs.push_str(&format!(r#"<g id="l{level}">{}</g>"#, format!(r##"<use href="#l{}"/>"##, level - 1).repeat(10)));
    }
    let bomb = svg_mm(&format!(r##"<defs>{defs}</defs><use href="#l8"/>"##));
    let started = std::time::Instant::now();
    let result = on_small_stack(move || import("bomb.svg", &bomb, &ImportOptions::default()).map(|_| ()).map_err(|e| e.to_string()));
    assert!(result.as_ref().is_err_and(|e| e.contains("too large")), "{result:?}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));

    let recursive = svg_mm(r##"<g id="a"><path d="M0 0L1 1" stroke="#000000"/><g><use href="#b"/></g></g><g id="b"><use href="#a"/></g>"##);
    assert!(import("loop.svg", &recursive, &ImportOptions::default()).is_err());
    // An ordinary reuse imports.
    let reuse = svg_mm(r##"<defs><path id="p" d="M0 0L1 1" stroke="#000000"/></defs><use href="#p"/><use href="#p" x="2"/>"##);
    assert_eq!(import("reuse.svg", &reuse, &ImportOptions::default()).unwrap().shapes.len(), 2);
}

/// Markers (usvg clones the marker's content at every vertex, and clips it) are refused, however they are set; an
/// unused marker definition is harmless.
#[test]
fn svg_markers_are_refused() {
    let marker = r##"<defs><marker id="m"><path d="M0 0L1 1" stroke="#000000"/></marker></defs>"##;
    for set in [r##"marker-mid="url(#m)""##, r##"marker-end=" url( #m ) ""##, r##"style="marker:url(#m)""##] {
        let doc = svg_mm(&format!(r##"{marker}<path d="M0 0L5 5L9 1" stroke="#000000" {set}/>"##));
        assert!(matches!(import("arrow.svg", &doc, &ImportOptions::default()), Err(Error::NotImplemented(m)) if m.contains("marker")), "{set}");
    }
    // The marker elements themselves are refused whether anything uses them.
    let unused = svg_mm(&format!(r##"{marker}<path d="M0 0L5 5L9 1" stroke="#000000" marker-mid="none"/>"##));
    assert!(matches!(import("unused.svg", &unused, &ImportOptions::default()), Err(Error::NotImplemented(m)) if m.contains("marker")));
    assert_eq!(
        import("none.svg", &svg_mm(r##"<path d="M0 0L5 5L9 1" stroke="#000000" marker-mid="none"/>"##), &ImportOptions::default()).unwrap().shapes.len(),
        1
    );
}

/// Review item 3: `<image>` hrefs are never resolved, and the import says so. (That no file is read is proven on the
/// resolver itself, `svg_image_href_resolver_never_reads_files`; this test links a real local file rather than
/// `/dev/zero`, so a regression cannot exhaust the machine's memory.)
#[test]
fn svg_images_are_never_loaded() {
    let local = openlaser_testkit::fixture("gray64.png");
    let body = format!(
        r##"<image href="{}" width="5" height="5"/><image href="\\server\share\x.png" width="5" height="5"/><path d="M1 1L9 9" stroke="#000000"/>"##,
        local.display()
    );
    let imported = import("image.svg", &svg_mm(&body), &ImportOptions::default()).expect("imports");
    assert_eq!(imported.shapes.len(), 1);
    assert!(imported.notices.iter().any(|n| n.code == NoticeCode::ImportEntitySkipped && n.message.contains("2 SVG <image>")), "{:?}", imported.notices);
}

/// Review item 6: input byte caps.
#[test]
fn oversized_vector_inputs_are_refused() {
    let mut big = svg_mm("");
    big.resize(MAX_VECTOR_FILE_BYTES + 1, b' ');
    assert!(matches!(import("big.svg", &big, &ImportOptions::default()), Err(Error::TooLarge(_))));
    assert!(matches!(import("big.dxf", &big, &ImportOptions::default()), Err(Error::TooLarge(_))));
    assert!(matches!(dxf_om::parse_dxf_om(&big), Err(Error::TooLarge(_))));
    assert!(matches!(om::parse_workspace(&big), Err(Error::TooLarge(_))));
    assert!(matches!(om::parse_layer_settings(&big), Err(Error::TooLarge(_))));
    assert_ne!(sniff("big.json", &big), Some(openlaser_import::Format::OmWorkspace), "an oversized .json is not even probed");
}

/// Review item 6: coordinates that overflow after unit scaling or a transform are refused, never imported.
#[test]
fn non_finite_after_transforms_is_refused() {
    let parsecs =
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n20\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n1e300\n20\n0\n11\n-1e300\n21\n0\n0\nENDSEC\n0\nEOF\n";
    assert!(import("far.dxf", parsecs.as_bytes(), &ImportOptions::default()).is_err());
    assert!(dxf_om::import_geometry_om("far.dxf", parsecs.as_bytes()).is_err());
    let svg = svg_mm(r##"<path d="M0 0L3e38 0" transform="scale(1e30)" stroke="#000000"/>"##);
    assert!(import("far.svg", &svg, &ImportOptions::default()).is_err());
    let scaled = std::fs::read(openlaser_testkit::fixture("square10mm.svg")).unwrap();
    assert!(import("square10mm.svg", &scaled, &ImportOptions { scale: 1e300, ..ImportOptions::default() }).is_err());
    assert!(import("square10mm.svg", &scaled, &ImportOptions { center_at: Some([f64::MAX, 0.0]), ..ImportOptions::default() }).is_err());
}

// ---- review item 7: DXF placement in non-mm units ----------------------------------------------------------------

fn dxf_line(insunits: u8, x0: f64, x1: f64) -> Vec<u8> {
    format!("0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n{insunits}\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n{x0}\n20\n1\n11\n{x1}\n21\n1\n0\nENDSEC\n0\nEOF\n")
        .into_bytes()
}

fn assert_line(imported: &Imported, expected: [[f64; 2]; 2]) {
    let points = shape_points(&imported.shapes[0]);
    assert_eq!(points.len(), 2);
    for (got, want) in points.iter().zip(expected) {
        assert!((got[0] - want[0]).abs() < 1e-9 && (got[1] - want[1]).abs() < 1e-9, "got {points:?}, want {expected:?}");
    }
}

#[test]
fn dxf_inch_file_keeps_its_place_in_mm() {
    let bytes = dxf_line(1, 10.0, 12.0);
    let imported = import("inch.dxf", &bytes, &ImportOptions::default()).unwrap();
    assert_line(&imported, [[254.0, 25.4], [304.8, 25.4]]);
    let bounds = imported.bounds.unwrap();
    assert!((bounds.min[0] - 254.0).abs() < 1e-9 && (bounds.max[0] - 304.8).abs() < 1e-9);
    // Scale about the drawing's centre (279.4 mm), in mm.
    assert_line(&import("inch.dxf", &bytes, &ImportOptions { scale: 2.0, ..ImportOptions::default() }).unwrap(), [[228.6, 25.4], [330.2, 25.4]]);
    assert_line(&import("inch.dxf", &bytes, &ImportOptions { center_at: Some([0.0, 0.0]), ..ImportOptions::default() }).unwrap(), [[-25.4, 0.0], [25.4, 0.0]]);
    // Forcing inches on a file without $INSUNITS gives the same placement.
    let no_units = "0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n10\n20\n1\n11\n12\n21\n1\n0\nENDSEC\n0\nEOF\n";
    assert_line(
        &import("inch.dxf", no_units.as_bytes(), &ImportOptions { dxf_units: DxfUnits::Inch, ..ImportOptions::default() }).unwrap(),
        [[254.0, 25.4], [304.8, 25.4]],
    );
}

#[test]
fn dxf_cm_file_keeps_its_place_in_mm() {
    let imported = import("cm.dxf", &dxf_line(5, 10.0, 12.0), &ImportOptions::default()).unwrap();
    assert_line(&imported, [[100.0, 10.0], [120.0, 10.0]]);
    assert!(imported.notices.is_empty(), "{:?}", imported.notices);
}

#[test]
fn dxf_notices_for_assumed_units_unsupported_and_degenerate_entities() {
    let text = "0\nSECTION\n2\nENTITIES\n0\nTEXT\n1\nhello\n0\nINSERT\n2\nblock\n0\nLWPOLYLINE\n10\n1\n20\n1\n0\nLINE\n10\n0\n20\n0\n11\n1\n21\n1\n0\nENDSEC\n0\nEOF\n";
    let imported = import("mixed.dxf", text.as_bytes(), &ImportOptions::default()).unwrap();
    assert_eq!(imported.shapes.len(), 1);
    let messages: Vec<&str> = imported.notices.iter().map(|n| n.message.as_str()).collect();
    assert_eq!(messages.len(), 3, "{messages:?}");
    assert!(messages[0].contains("millimetres were assumed"));
    assert!(messages[1].contains("1 DXF entities with fewer than 2 points"));
    assert!(messages[2].contains("INSERT ×1") && messages[2].contains("TEXT ×1"));
    assert_eq!(imported.notices.iter().map(|n| n.n).collect::<Vec<_>>(), vec![1, 2, 3]);
}

// ---- raster -----------------------------------------------------------------------------------------------------

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn png_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    png.extend_from_slice(&body);
    png.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// A PNG whose header claims `width × height` 8-bit grey pixels and whose image data is a few bytes: decoding
/// it would need the full buffer, so only a check on the header can refuse it cheaply.
fn png_header_only(width: u32, height: u32) -> Vec<u8> {
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png_chunk(&mut png, b"IHDR", &ihdr);
    png_chunk(&mut png, b"IDAT", &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]);
    png_chunk(&mut png, b"IEND", &[]);
    png
}

/// Size limits apply to the header, before a pixel is decoded.
#[test]
fn raster_limits_apply_before_decoding() {
    for (w, h) in [(100_000, 100_000), (16_385, 1), (10_000, 10_000)] {
        let result = import("huge.png", &png_header_only(w, h), &ImportOptions::default());
        assert!(matches!(result, Err(Error::TooLarge(_))), "{w}×{h}: {result:?}");
    }
    let gray = std::fs::read(openlaser_testkit::fixture("gray64.png")).unwrap();
    let imported = import("gray64.png", &gray, &ImportOptions::default()).unwrap();
    let bounds = imported.bounds.unwrap();
    assert!((bounds.max[0] - bounds.min[0] - 6.4).abs() < 1e-9, "254 dpi from pHYs: {bounds:?}");
    assert!(imported.notices.is_empty());
}

// ---- SPEC §6.5: prop_importers_never_panic -----------------------------------------------------------------------

mod props {
    use proptest::prelude::*;

    use super::*;

    const NAMES: [&str; 11] =
        ["random.svg", "random.dxf", "workspace.json", "random.png", "random.jpg", "random.bmp", "random.tif", "random.webp", "random.gif", "noext", "x.json"];

    fn dxf_value() -> impl Strategy<Value = String> {
        prop_oneof![
            prop::sample::select(vec![
                "LINE",
                "LWPOLYLINE",
                "POLYLINE",
                "VERTEX",
                "SEQEND",
                "CIRCLE",
                "ARC",
                "ELLIPSE",
                "SPLINE",
                "SECTION",
                "ENTITIES",
                "ENDSEC",
                "$INSUNITS",
                "EOF",
                "TEXT",
                "INSERT"
            ])
            .prop_map(str::to_owned),
            prop::sample::select(vec!["0", "1", "-1", "nan", "inf", "-inf", "1e308", "-1e308", "1e-320", "", "x", "0.5", "3", "20", "99999999999999999999"])
                .prop_map(str::to_owned),
            any::<f64>().prop_map(|v| v.to_string()),
            any::<i32>().prop_map(|v| v.to_string()),
        ]
    }

    fn dxf_doc() -> impl Strategy<Value = Vec<u8>> {
        let code = prop::sample::select(vec!["0", "2", "8", "9", "10", "20", "11", "21", "40", "41", "42", "50", "51", "70", "71", "72", "73", "74"]);
        let eol = prop::sample::select(vec!["\n", "\r\n", "\r", "\u{2028}"]);
        (prop::collection::vec((code, dxf_value()), 0..120), eol).prop_map(|(pairs, eol)| {
            let mut text = String::new();
            for (code, value) in pairs {
                text.push_str(code);
                text.push_str(eol);
                text.push_str(&value);
                text.push_str(eol);
            }
            text.into_bytes()
        })
    }

    fn svg_docs() -> impl Strategy<Value = Vec<u8>> {
        let element = prop::sample::select(vec![
            r##"<path d="M0 0L5 5"/>"##,
            r##"<path d="M1e38 0L-1e38 1e38" stroke="#ff0000"/>"##,
            r##"<g transform="scale(1e30)">"##,
            "</g>",
            "<g>",
            r##"<use href="#a"/>"##,
            r##"<g id="a"><use href="#a"/></g>"##,
            r##"<rect width="10" height="10" fill="#000000"/>"##,
            r##"<image href="missing.png"/>"##,
            "<text>x</text>",
            r##"<marker id="m"><path d="M0 0L1 1" marker-mid="url(#m)"/></marker>"##,
            "<circle r='5'/>",
        ]);
        let size = prop::sample::select(vec![r#"width="10mm" height="10mm""#, r#"width="10" height="10""#, ""]);
        (prop::collection::vec(element, 0..40), size).prop_map(|(parts, size)| svg_doc(size, &parts.concat()))
    }

    fn om_doc() -> impl Strategy<Value = Vec<u8>> {
        let coord = prop_oneof![
            any::<f64>().prop_filter("JSON numbers are finite", |v| v.is_finite()).prop_map(|v| serde_json::json!(v)),
            Just(serde_json::json!("x")),
            Just(serde_json::json!(null))
        ];
        let point = prop::collection::vec(coord, 0..4);
        let polyline = prop::collection::vec(point, 0..6);
        let object = (prop::collection::vec(polyline, 0..3), prop::sample::select(vec!["vector-engrave", "raster", "frame-only", "nope"]), any::<f64>(), any::<f64>())
            .prop_map(|(polylines, layer, scale, rotation)| {
                let finite = |v: f64| if v.is_finite() { v } else { 1.0 };
                serde_json::json!({"name": "o", "layer_id": layer, "polylines": polylines, "x": 0, "y": 0, "scale": finite(scale), "rotation": finite(rotation)})
            });
        prop::collection::vec(object, 0..5).prop_map(|objects| serde_json::to_vec(&serde_json::json!({"version": 1, "objects": objects})).unwrap_or_default())
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        #[test]
        fn prop_importers_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..4096), name in prop::sample::select(NAMES.to_vec())) {
            let _ = import(name, &bytes, &ImportOptions::default());
            let _ = import(name, &bytes, &ImportOptions { svg_units: SvgUnits::Px96, ..ImportOptions::default() });
            let _ = dxf_om::parse_dxf_om(&bytes);
            let _ = dxf_om::import_geometry_om("random.dxf", &bytes);
            let _ = om::parse_workspace(&bytes);
            let _ = om::parse_layer_settings(&bytes);
        }

        #[test]
        fn prop_structured_dxf_never_panics(doc in dxf_doc(), scale in prop::sample::select(vec![1.0, 1e-300, 1e300])) {
            let _ = import("gen.dxf", &doc, &ImportOptions { scale, ..ImportOptions::default() });
            let _ = dxf_om::parse_dxf_om(&doc);
            let _ = dxf_om::import_geometry_om("gen.dxf", &doc);
        }

        #[test]
        fn prop_structured_svg_never_panics(doc in svg_docs()) {
            let _ = import("gen.svg", &doc, &ImportOptions::default());
            let _ = import("gen.svg", &doc, &ImportOptions { svg_units: SvgUnits::Px72, center_at: Some([1.0, 2.0]), ..ImportOptions::default() });
        }

        #[test]
        fn prop_structured_om_never_panics(doc in om_doc()) {
            let _ = import("workspace.json", &doc, &ImportOptions::default());
            if let Ok(ws) = om::parse_workspace(&doc) {
                let _ = om::to_imported(&ws, &om::builtin_layers());
            }
        }
    }
}
