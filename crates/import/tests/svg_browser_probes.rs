//! Browser-alignment probes (the WP2 review rendered ~240 SVGs in Chrome as the reference): every accepted input
//! must land its geometry where Chrome draws it, in millimetres, and everything the parser would place or hide
//! differently is refused with a clear message. SVG is y-down, imported as y-up (a rect at y=1..5 lands at
//! y = -5..-1).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use openlaser_import::{Error, ImportOptions, Imported, SvgUnits, import};

fn svg_doc(attrs: &str, body: &str) -> Vec<u8> {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" {attrs}>{body}</svg>"#).into_bytes()
}

/// A 100 mm × 100 mm canvas in user units.
fn svg_mm(body: &str) -> Vec<u8> {
    svg_doc(r#"width="100mm" height="100mm" viewBox="0 0 100 100""#, body)
}

fn run(name: &'static str, bytes: Vec<u8>) -> Result<Imported, Error> {
    import(name, &bytes, &ImportOptions::default())
}

/// Whether the import refused with a message mentioning `needle`.
fn refused(result: &Result<Imported, Error>, needle: &str) -> bool {
    matches!(result, Err(e) if e.to_string().contains(needle))
}

fn bounds_of(imported: &Imported) -> ([f64; 2], [f64; 2]) {
    (imported.bounds.expect("bounds").min, imported.bounds.expect("bounds").max)
}

/// The review finding: usvg composes a `<use>` element's `transform` into the clone's absolute transform a second
/// time, so a shape expected at 40–45 mm landed at 70–75 mm. The importer expands `<use>` itself, once.
#[test]
fn use_transform_is_applied_exactly_once() {
    let doc = svg_mm(r##"<defs><rect id="r" x="10" y="10" width="5" height="5" stroke="#000000"/></defs><use href="#r" transform="translate(30 0)"/>"##);
    let imported = run("use.svg", doc).expect("a <use> clone imports");
    assert_eq!(imported.shapes.len(), 1);
    let (min, max) = bounds_of(&imported);
    // Chrome: x 10+30..15+30, y -(10+5)..-10.
    for (got, want) in [min[0], max[0], min[1], max[1]].into_iter().zip([40.0, 45.0, -15.0, -10.0]) {
        assert!((got - want).abs() < 1e-4, "got {min:?}..{max:?}");
    }
    // A second transform on an ancestor applies once more — not the use transform twice.
    let nested = svg_mm(
        r##"<defs><rect id="r" x="10" y="10" width="5" height="5" stroke="#000000"/></defs><g transform="translate(0 50)"><use href="#r" transform="translate(30 0)"/></g>"##,
    );
    let (min, max) = bounds_of(&run("use2.svg", nested).unwrap());
    for (got, want) in [min[0], max[0], min[1], max[1]].into_iter().zip([40.0, 45.0, -65.0, -60.0]) {
        assert!((got - want).abs() < 1e-4, "got {min:?}..{max:?}");
    }
}

/// `x`/`y` on a `<use>` are a `translate` appended to its transform list (applied first): x=5 then translate(30 0)
/// moves the clone 35 right.
#[test]
fn use_x_y_and_transform_compose_like_a_browser() {
    let doc =
        svg_mm(r##"<defs><rect id="r" x="10" y="10" width="5" height="5" stroke="#000000"/></defs><use href="#r" transform="translate(30 0)" x="5" y="0"/>"##);
    let (min, max) = bounds_of(&run("use_xy.svg", doc).unwrap());
    for (got, want) in [min[0], max[0], min[1], max[1]].into_iter().zip([45.0, 50.0, -15.0, -10.0]) {
        assert!((got - want).abs() < 1e-4, "got {min:?}..{max:?}");
    }
    // Inkscape-style: xlink:href plus x/y="0".
    let inkscape = svg_mm(
        r##"<defs><rect id="r2" x="10" y="10" width="5" height="5" stroke="#000000"/></defs><use xlink:href="#r2" transform="translate(70.2,31.4)" x="0" y="0" width="100%" height="100%"/>"##,
    );
    let (min, max) = bounds_of(&run("inkscape.svg", inkscape).unwrap());
    for (got, want) in [min[0], max[0], min[1], max[1]].into_iter().zip([80.2, 85.2, -46.4, -41.4]) {
        assert!((got - want).abs() < 1e-4, "got {min:?}..{max:?}");
    }
    // A use of a group inherits the group's own transform; the use transform lands outside it.
    let group = svg_mm(
        r##"<defs><g id="g" transform="translate(0 5)"><rect x="10" y="10" width="5" height="5" stroke="#000000"/></g></defs><use href="#g" transform="translate(30 0)"/>"##,
    );
    let (min, max) = bounds_of(&run("use_g.svg", group).unwrap());
    for (got, want) in [min[0], max[0], min[1], max[1]].into_iter().zip([40.0, 45.0, -20.0, -15.0]) {
        assert!((got - want).abs() < 1e-4, "got {min:?}..{max:?}");
    }
    // A use inside defs and a chained use both stay in place.
    let chained = svg_mm(
        r##"<defs><use id="u" href="#r" transform="translate(30 0)"/><rect id="r" x="10" y="10" width="5" height="5" stroke="#000000"/></defs><use href="#u"/>"##,
    );
    let (min, max) = bounds_of(&run("use_u.svg", chained).unwrap());
    for (got, want) in [min[0], max[0], min[1], max[1]].into_iter().zip([40.0, 45.0, -15.0, -10.0]) {
        assert!((got - want).abs() < 1e-4, "got {min:?}..{max:?}");
    }
}

/// `<use>` edge cases a browser resolves differently: external references, conflicting hrefs and clones that are not
/// plain shapes are refused; a use whose children are fallback content is fine (they are dropped).
#[test]
fn use_edge_cases_refuse_clearly() {
    for (body, expect) in [
        (r##"<use href="other.svg#x"/><path d="M0 0L1 1" stroke="#000000"/>"##, "external reference"),
        (r##"<use href="#a" xlink:href="#b"/><path id="a" d="M0 0L1 1" stroke="#000000"/><path id="b" d="M0 0L1 1" stroke="#000000"/>"##, "conflicting"),
        (r##"<defs><clipPath id="c"><rect width="5" height="5"/></clipPath></defs><use href="#c"/>"##, "clip"),
        (r##"<defs><marker id="m"><path d="M0 0L1 1"/></marker></defs><use href="#m"/>"##, "marker"),
        (r##"<use href="#missing"/><path d="M0 0L1 1" stroke="#000000"/>"##, ""),
    ] {
        let result = run("edge.svg", svg_mm(body));
        if expect.is_empty() {
            assert_eq!(result.expect("a missing target imports the rest").shapes.len(), 1);
        } else {
            assert!(refused(&result, expect), "{expect}: {result:?}");
        }
    }
    // Children of <use> are fallback content a browser never draws.
    let fallback =
        svg_mm(r##"<defs><rect id="r" width="5" height="5" stroke="#000000"/></defs><use href="#r"><rect x="50" y="50" width="5" height="5"/></use>"##);
    assert_eq!(run("fallback.svg", fallback).unwrap().shapes.len(), 1);
}

/// `100MM` and friends: browsers treat unit suffixes case-insensitively and accept more of them; the parser would
/// drop or mis-scale the value, so it is refused with the unit named.
#[test]
fn unit_spellings_browsers_accept_are_refused() {
    for (attrs, body, unit) in [
        (r#"width="100MM" height="100mm" viewBox="0 0 100 100""#, r##"<rect width="5" height="5" stroke="#000000"/>"##, "MM"),
        (r#"width="100mm" height="100mm" viewBox="0 0 100 100""#, r##"<rect x="5Cm" width="5" height="5" stroke="#000000"/>"##, "Cm"),
        (r#"width="100mm" height="100mm" viewBox="0 0 100 100""#, r##"<rect x="1Q" width="5" height="5" stroke="#000000"/>"##, "Q"),
        (r#"width="100mm" height="100mm" viewBox="0 0 100 100""#, r##"<rect width="5" height="5" stroke="#000000" style="stroke-width:0.1MM"/>"##, "MM"),
    ] {
        let result = run("unit.svg", svg_doc(attrs, body));
        assert!(matches!(&result, Err(Error::Malformed { message, .. }) if message.contains(&format!("\"{unit}\""))), "{unit}: {result:?}");
    }
    // A trailing space on a single length: the parser drops the whole value where a browser trims it.
    let trailing = svg_doc(r#"width="100mm " height="100mm" viewBox="0 0 100 100""#, r##"<rect width="5" height="5" stroke="#000000"/>"##);
    assert!(matches!(run("ws.svg", trailing), Err(Error::Malformed { message, .. }) if message.contains("whitespace")));
    // And the ordinary spellings import at the same scale Chrome draws them: x="10mm" and width="25.4mm" are CSS
    // lengths (96 px/in), 10 mm = 37.8 user units and 25.4 mm = 96.
    let doc = svg_doc(r#"width="100mm" height="100mm" viewBox="0 0 100 100""#, r##"<rect x="10mm" y="10" width="25.4mm" height="10" stroke="#000000"/>"##);
    let (min, max) = bounds_of(&run("units.svg", doc).unwrap());
    for (got, want) in [min[0], max[0], min[1], max[1]].into_iter().zip([37.7953, 133.7953, -20.0, -10.0]) {
        assert!((got - want).abs() < 1e-3, "got {min:?}..{max:?}");
    }
}

/// Missing units use the caller's px dpi like a CSS px (Px96 here), and a viewBox edge a browser and the parser
/// disagree about is refused.
#[test]
fn unitless_and_viewbox_edges() {
    let px = ImportOptions { svg_units: SvgUnits::Px96, ..ImportOptions::default() };
    let doc = svg_doc(r#"viewBox="0 0 100 100""#, r##"<rect x="10" y="10" width="96" height="96" stroke="#000000"/>"##);
    let (min, _) = bounds_of(&import("px.svg", &doc, &px).unwrap());
    assert!((min[0] - 10.0 * 25.4 / 96.0).abs() < 1e-4, "{min:?}");
    for view_box in ["0 0 0 100", "0 0 100 -5"] {
        let doc = svg_doc(&format!(r#"width="100mm" height="100mm" viewBox="{view_box}""#), r##"<rect width="5" height="5" stroke="#000000"/>"##);
        let result = run("vb.svg", doc);
        assert!(refused(&result, "viewBox"), "{view_box}: {result:?}");
    }
    let junk = svg_doc(r#"width="100mm" height="100mm" viewBox="0 0 100 100 mm""#, r##"<rect width="5" height="5" stroke="#000000"/>"##);
    assert!(refused(&run("vb2.svg", junk), "viewBox"));
    // An incomplete points list renders its prefix in the parser but nothing in a browser.
    let points = svg_mm(r##"<polyline points="0,0 10,10 junk" stroke="#000000"/>"##);
    assert!(matches!(run("points.svg", points), Err(Error::Malformed { message, .. }) if message.contains("points")));
}

/// CSS transforms in styles are a different grammar than the transform attribute and are refused; the attribute form
/// still lands where Chrome puts it. `!important` may hide (never show) geometry.
#[test]
fn css_transforms_and_important() {
    let attribute = svg_mm(r##"<rect x="10" y="10" width="5" height="5" transform="translate(30 0)" stroke="#000000"/>"##);
    let (min, _) = bounds_of(&run("attr.svg", attribute).unwrap());
    assert!((min[0] - 40.0).abs() < 1e-4, "{min:?}");
    for style in
        ["transform: translateX(5px)", "transform: translate(5px,0)", "transform-origin: 0 0", "width: 5", "animation: a 1s", "offset-path: path('M0 0')"]
    {
        let doc = svg_mm(&format!(r##"<rect x="10" y="10" width="5" height="5" style="{style}" stroke="#000000"/>"##));
        assert!(matches!(run("css.svg", doc), Err(Error::Malformed { .. })), "{style}");
    }
    // Hidden stays hidden with !important, in an attribute or a sheet.
    for (name, body) in [
        (
            "attr",
            r##"<rect width="5" height="5" style="display:none !important" stroke="#000000"/><rect x="10" y="10" width="5" height="5" stroke="#000000"/>"##,
        ),
        (
            "sheet",
            r##"<style>.h{display:none !important}</style><rect class="h" width="5" height="5" stroke="#000000"/><rect x="10" y="10" width="5" height="5" stroke="#000000"/>"##,
        ),
        ("vis", r##"<rect width="5" height="5" visibility="hidden" stroke="#000000"/><rect x="10" y="10" width="5" height="5" stroke="#000000"/>"##),
    ] {
        let imported = run(name, svg_mm(body)).expect("imports the visible rect");
        assert_eq!(imported.shapes.len(), 1);
        let (min, _) = bounds_of(&imported);
        assert!((min[0] - 10.0).abs() < 1e-4, "{name}: {min:?}");
    }
}

/// `media="print"` (and any non-screen condition), disabled or titled sheets and an external `xml-stylesheet` are
/// refused: a browser would not apply (or would load) what the parser would.
#[test]
fn style_sheets_refuse_non_screen_and_external() {
    for attrs in [r#"media="print""#, r#"media="projection""#, r#"disabled="true""#, r#"title="alt""#, r#"type="TEXT/CSS""#] {
        let doc = svg_mm(&format!(
            r##"<style {attrs}>.h{{display:none}}</style><rect class="h" width="5" height="5" stroke="#000000"/><rect x="10" y="10" width="5" height="5" stroke="#000000"/>"##
        ));
        assert!(matches!(run("sheet.svg", doc), Err(Error::NotImplemented(_))), "{attrs}");
    }
    // media="screen" and the default still apply: the hidden rect stays hidden.
    let doc = svg_mm(
        r##"<style media="screen">.h{display:none}</style><rect class="h" width="5" height="5" stroke="#000000"/><rect x="10" y="10" width="5" height="5" stroke="#000000"/>"##,
    );
    assert_eq!(run("sheet2.svg", doc).unwrap().shapes.len(), 1);
    let pi =
        format!(r#"<?xml-stylesheet type="text/css" href="x.css"?>{}"#, String::from_utf8_lossy(&svg_mm(r##"<rect width="5" height="5" stroke="#000000"/>"##)));
    assert!(matches!(run("pi.svg", pi.into_bytes()), Err(Error::NotImplemented(m)) if m.contains("xml-stylesheet")));
}

/// `<switch>`, animation and scripting: refused, never silently dropped or rendered.
#[test]
fn switch_animation_and_script_refused() {
    for (body, expect) in [
        (r##"<switch><path d="M0 0L1 1" stroke="#000000"/></switch><path d="M1 1L9 9" stroke="#000000"/>"##, "switch"),
        (r##"<path d="M0 0L1 1" stroke="#000000"><animate attributeName="x" to="9"/></path>"##, "animation"),
        (r##"<animateTransform attributeName="transform" type="translate" to="9 0"/><path d="M0 0L1 1" stroke="#000000"/>"##, "animation"),
        (r##"<set attributeName="x" to="9"/><path d="M0 0L1 1" stroke="#000000"/>"##, "animation"),
        (r##"<script>alert(1)</script><path d="M0 0L1 1" stroke="#000000"/>"##, "script"),
        (r##"<link rel="stylesheet" href="x.css"/><path d="M0 0L1 1" stroke="#000000"/>"##, "link"),
    ] {
        assert!(matches!(run("hostile.svg", svg_mm(body)), Err(Error::NotImplemented(m)) if m.contains(expect)), "{expect}");
    }
}

/// An unknown element that hides SVG children is refused (usvg would drop the subtree a browser renders); an unknown
/// element holding only editor metadata imports.
#[test]
fn unknown_elements_cannot_hide_svg_content() {
    let hidden = svg_mm(r##"<foo><rect width="5" height="5" stroke="#000000"/></foo><path d="M1 1L9 9" stroke="#000000"/>"##);
    assert!(matches!(run("hidden.svg", hidden), Err(Error::Malformed { message, .. }) if message.contains("<foo>")));
    let meta = svg_mm(
        r##"<sodipodi:namedview xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd" pagecolor="#fff"/><path d="M1 1L9 9" stroke="#000000"/>"##,
    );
    assert_eq!(run("meta.svg", meta).unwrap().shapes.len(), 1);
    let font = svg_mm(r##"<defs><font><font-face font-family="x"/></font></defs><path d="M1 1L9 9" stroke="#000000"/>"##);
    assert!(matches!(run("font.svg", font), Err(Error::Malformed { message, .. }) if message.contains("<font>")));
}
