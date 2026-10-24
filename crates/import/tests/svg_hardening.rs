//! Regression tests for the WP2 re-review of the SVG importer (B1: stack overflows, B2: unbounded allocation and
//! work, hidden geometry). Every hostile input is imported on a 2 MiB thread (a stack overflow aborts the test
//! binary) and must finish within [`TIME_BOUND`] (a loop or a quadratic blow-up fails the test instead of hanging
//! it). Each refusal is asserted by its variant and message, which come from the guard that runs before usvg: the
//! same input reaching usvg would fail differently (or crash, hang or exhaust memory), so a test passing for the
//! wrong reason is caught. (A counting global allocator would need `unsafe impl GlobalAlloc`, which the workspace's
//! `unsafe_code = "forbid"` rules out for test targets too; the refusals are proven to happen before usvg instead.)
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::mpsc;
use std::time::{Duration, Instant};

use openlaser_import::{
    Error, ImportOptions, Imported, MAX_SVG_ATTRIBUTES, MAX_SVG_DEPTH, MAX_SVG_EMPTY_SEGMENTS, MAX_SVG_NAMESPACES, MAX_SVG_PATH_DATA_BYTES, SvgUnits, import,
};
use openlaser_types::NoticeCode;

const TIME_BOUND: Duration = Duration::from_secs(10);

fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="20mm" height="20mm" viewBox="0 0 20 20">{body}</svg>"#)
}

/// Imports `doc` on a 2 MiB thread, failing the test if it takes longer than [`TIME_BOUND`].
fn run(doc: String) -> Result<Imported, Error> {
    run_with(doc, ImportOptions::default())
}

fn run_with(doc: String, opts: ImportOptions) -> Result<Imported, Error> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let started = Instant::now();
            let result = import("hostile.svg", doc.as_bytes(), &opts);
            let _ = tx.send((result, started.elapsed()));
        })
        .expect("spawn");
    let (result, elapsed) = rx.recv_timeout(TIME_BOUND).expect("the import finished within the time bound on a 2 MiB stack");
    assert!(elapsed < TIME_BOUND, "took {elapsed:?}");
    result
}

#[track_caller]
fn assert_too_large(result: Result<Imported, Error>, message: &str) {
    match result {
        Err(Error::TooLarge(m)) => assert!(m.contains(message), "TooLarge({m:?}) does not mention {message:?}"),
        other => panic!("expected TooLarge mentioning {message:?}, got {other:?}"),
    }
}

#[track_caller]
fn assert_not_implemented(result: Result<Imported, Error>, message: &str) {
    match result {
        Err(Error::NotImplemented(m)) => assert!(m.contains(message), "NotImplemented({m:?}) does not mention {message:?}"),
        other => panic!("expected NotImplemented mentioning {message:?}, got {other:?}"),
    }
}

fn deep(levels: usize) -> String {
    format!("{}{}", "<g>".repeat(levels), "</g>".repeat(levels))
}

fn notice<'a>(imported: &'a Imported, text: &str) -> &'a openlaser_import::Notice {
    imported.notices.iter().find(|n| n.message.contains(text)).unwrap_or_else(|| panic!("no notice mentions {text:?}: {:?}", imported.notices))
}

// ---- B1 (a): the depth pre-scan cannot be skipped -----------------------------------------------------------------

/// `<!DOCTYPE svg [<?x [?>]>` made the old scan read the rest of the file as DOCTYPE; 20,000 nested `<g>` then
/// overflowed roxmltree. Any internal subset is now refused before a parser runs, whatever it holds.
#[test]
fn b1a_doctype_internal_subsets_are_refused_before_parsing() {
    for subset in ["<?x [?>", r#"<?x "?>"#, "<!-- ] -->", r#"<!ENTITY a "]">"#, ""] {
        let doc = format!("<!DOCTYPE svg [{subset}]>{}", svg(&deep(20_000)));
        assert_not_implemented(run(doc), "internal subset");
    }
}

/// A quote inside a processing instruction, a comment or CDATA does not open a string for the scan.
#[test]
fn b1a_quotes_in_markup_cannot_desync_the_depth_scan() {
    for (before, inside) in [(r#"<?x "?>"#, ""), (r#"<!-- " -->"#, ""), ("", r#"<![CDATA[ " ]]>"#), ("", r#"<?x '?>"#), ("", r#"<g a=">'"/>"#)] {
        let doc = format!("{before}{}", svg(&format!("{inside}{}", deep(20_000))));
        assert_too_large(run(doc), "nested more than");
    }
    // A plain DOCTYPE (no internal subset) still imports: it is blanked and roxmltree runs without DTD support.
    let plain = format!(
        r#"<?xml version="1.0"?><!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">{}"#,
        svg(r##"<path d="M1 1L9 9" stroke="#000000"/>"##)
    );
    assert_eq!(run(plain).expect("plain DOCTYPE imports").shapes.len(), 1);
}

// ---- B1 (b): nesting hidden in an entity ---------------------------------------------------------------------------

#[test]
fn b1b_entity_nesting_is_refused_as_too_deep() {
    let doc = format!(r#"<!DOCTYPE svg [<!ENTITY deep "{}">]>{}"#, deep(20_000), svg("&deep;"));
    assert_too_large(run(doc), &format!("nested more than {MAX_SVG_DEPTH} deep"));
}

// ---- B1 (c): reference cycles and chains --------------------------------------------------------------------------

const SHAPE: &str = r##"<rect width="5" height="5" stroke="#000000"/>"##;

#[test]
fn b1c_clip_path_and_mask_cycles_are_refused() {
    let clip3 = format!(
        r##"<defs><clipPath id="a" clip-path="url(#b)">{SHAPE}</clipPath><clipPath id="b" clip-path="url(#c)">{SHAPE}</clipPath><clipPath id="c" clip-path="url(#a)">{SHAPE}</clipPath></defs><path d="M1 1L9 9" stroke="#000000" clip-path="url(#a)"/>"##
    );
    let clip_child = format!(
        r##"<defs><clipPath id="a"><rect width="5" height="5" clip-path="url(#b)"/></clipPath><clipPath id="b"><rect width="5" height="5" clip-path="url(#c)"/></clipPath><clipPath id="c"><rect width="5" height="5" clip-path="url(#a)"/></clipPath></defs><g clip-path="url(#a)">{SHAPE}</g>"##
    );
    let mask3 = format!(
        r##"<defs><mask id="a" mask="url(#b)">{SHAPE}</mask><mask id="b" mask="url(#c)">{SHAPE}</mask><mask id="c" mask="url(#a)">{SHAPE}</mask></defs><g mask="url(#a)">{SHAPE}</g>"##
    );
    let styled = format!(r##"<defs><clipPath id="a">{SHAPE}</clipPath></defs><g style="fill:#000000; Clip-Path : URL( '#a' )">{SHAPE}</g>"##);
    let sheet = format!(r##"<style>.c{{clip-path:url(#a)}}</style><defs><clipPath id="a">{SHAPE}</clipPath></defs><g class="c">{SHAPE}</g>"##);
    for doc in [clip3, clip_child, mask3, styled, sheet] {
        assert_not_implemented(run(svg(&doc)), "clip");
    }
}

/// 2,000-link chains overflowed usvg's recursion for clip-path, mask, pattern fills and filter → feImage → element.
#[test]
fn b1c_long_reference_chains_are_refused() {
    let n = 2_000;
    let chain = |open: &dyn Fn(usize) -> String, close: &str| -> String { (0..n).map(|i| format!("{}{SHAPE}{close}", open(i))).collect::<String>() };
    let clips = chain(&|i| format!(r##"<clipPath id="c{i}" clip-path="url(#c{})">"##, i + 1), "</clipPath>");
    assert_not_implemented(run(svg(&format!(r##"<defs>{clips}</defs><g clip-path="url(#c0)">{SHAPE}</g>"##))), "clip");
    let masks = chain(&|i| format!(r##"<mask id="m{i}" mask="url(#m{})">"##, i + 1), "</mask>");
    assert_not_implemented(run(svg(&format!(r##"<defs>{masks}</defs><g mask="url(#m0)">{SHAPE}</g>"##))), "mask");
    let patterns = (0..n)
        .map(|i| format!(r##"<pattern id="p{i}" width="1" height="1"><rect width="1" height="1" fill="url(#p{})"/></pattern>"##, i + 1))
        .collect::<String>();
    assert_not_implemented(run(svg(&format!(r##"<defs>{patterns}</defs><rect width="5" height="5" fill="url(#p0)"/>"##))), "pattern");
    let filters = (0..n)
        .map(|i| format!(r##"<filter id="f{i}"><feImage href="#e{i}"/></filter><g id="e{i}" filter="url(#f{})">{SHAPE}</g>"##, i + 1))
        .collect::<String>();
    assert_not_implemented(run(svg(&format!(r##"<defs>{filters}</defs><g filter="url(#f0)">{SHAPE}</g>"##))), "feImage");
    let filter_only = format!(r##"<defs><filter id="f"><feGaussianBlur stdDeviation="1"/></filter></defs><g filter="url(#f)">{SHAPE}</g>"##);
    assert_not_implemented(run(svg(&filter_only)), "filter");
}

/// usvg follows gradient `href` chains with a loop check that only stops at the starting gradient: a → b → c → b
/// looped forever. Loops, long chains and non-gradient links are refused; an ordinary chain imports.
#[test]
fn b1c_gradient_href_chains_are_bounded() {
    let stops = r##"<stop offset="0" stop-color="#000000"/><stop offset="1" stop-color="#ffffff"/>"##;
    let looped = svg(&format!(
        r##"<defs><linearGradient id="a" href="#b"/><linearGradient id="b" href="#c"/><linearGradient id="c" xlink:href="#b">{stops}</linearGradient></defs><rect width="5" height="5" fill="url(#a)"/>"##
    ));
    match run(looped) {
        Err(Error::Malformed { message, .. }) => assert!(message.contains("loop"), "{message}"),
        other => panic!("expected a loop refusal, got {other:?}"),
    }
    let long: String = (0..40).map(|i| format!(r##"<linearGradient id="g{i}" href="#g{}"/>"##, i + 1)).collect();
    assert_too_large(
        run(svg(&format!(r##"<defs>{long}<linearGradient id="g40">{stops}</linearGradient></defs><rect width="5" height="5" fill="url(#g0)"/>"##))),
        "href chain",
    );
    let to_rect = svg(r##"<defs><linearGradient id="a" href="#p"/><rect id="p" width="1" height="1"/></defs><rect width="5" height="5" fill="url(#a)"/>"##);
    assert_not_implemented(run(to_rect), "not a gradient");
    let ordinary = svg(&format!(
        r##"<defs><linearGradient id="base">{stops}</linearGradient><linearGradient id="a" xlink:href="#base" x2="1"/></defs><rect width="5" height="5" style="fill:url(#a)"/><rect x="6" width="5" height="5" fill="url('#base') #000000"/>"##
    ));
    let imported = run(ordinary).expect("gradient fills import");
    assert_eq!(imported.shapes.len(), 2);
    assert!(notice(&imported, "a gradient or pattern").code == NoticeCode::ImportColorUnmatched);
}

// ---- B2 (a): entity expansion ---------------------------------------------------------------------------------------

/// "Billion laughs": 101 KB grew to 1.6–7.4 GB. No entity is ever defined now.
#[test]
fn b2a_entity_expansion_bomb_is_refused() {
    let mut entities = String::from(r#"<!ENTITY l0 "laughlaughlaughlaughlaughlaughlaughlaughlaughlaughlaughlaughlaughlaughlaughlaughlaugh">"#);
    for level in 1..=5 {
        entities.push_str(&format!(r#"<!ENTITY l{level} "{}">"#, format!("&l{};", level - 1).repeat(10)));
    }
    let doc = format!("<!DOCTYPE svg [{entities}]>{}", svg(r##"<text>&l5;</text><path d="M1 1L9 9" stroke="#000000"/>"##));
    assert_not_implemented(run(doc), "internal subset");
}

// ---- B2 (b): work counted in vertices before usvg builds the tree ---------------------------------------------------

fn polyline_path(id: &str, vertices: usize) -> String {
    format!(r##"<path id="{id}" d="M0 0{}" stroke="#000000"/>"##, "L1 1".repeat(vertices - 1))
}

#[test]
fn b2b_use_multiplies_vertices_before_usvg() {
    // 150,000 vertices × 20 uses = 3 M points: refused before usvg (usvg itself would build them all first).
    let doc = svg(&format!(r##"<defs>{}</defs>{}"##, polyline_path("p", 150_000), r##"<use href="#p"/>"##.repeat(20)));
    assert_too_large(run(doc), "points once <use> references are expanded");
    // One path over the per-entity limit, and an ordinary reuse that fits.
    assert_too_large(run(svg(&polyline_path("big", 200_001))), "an SVG <path> makes 200001 points");
    let fits = svg(&format!(r##"<defs>{}</defs>{}"##, polyline_path("p", 1_000), r##"<use href="#p"/>"##.repeat(20)));
    assert_eq!(run(fits).expect("20 000 points import").shapes.len(), 20);
}

/// marker-mid × marker content was quadratic (24 KB → 571 MB); an objectBoundingBox pattern was copied per painted
/// path (178 KB → 1.36 GB). Markers and patterns are refused.
#[test]
fn b2b_markers_and_patterns_are_refused() {
    let vertices: String = (0..30_000).map(|i| format!("L{i} {i}")).collect();
    let marker = format!(
        r##"<defs><marker id="m"><g>{}</g></marker></defs><path d="M0 0{vertices}" stroke="#000000" marker-mid="url(#m)"/>"##,
        r##"<path d="M0 0L1 1" stroke="#000000"/>"##.repeat(50)
    );
    assert_not_implemented(run(svg(&marker)), "marker");
    assert_not_implemented(run(svg(r##"<style>path{marker:url(#m)}</style><path d="M0 0L1 1" stroke="#000000"/>"##)), "marker");
    let pattern = format!(
        r##"<defs><pattern id="p" width="1" height="1" patternContentUnits="objectBoundingBox">{}</pattern></defs>{}"##,
        r##"<path d="M0 0L1 1" stroke="#000000"/>"##.repeat(200),
        r##"<rect width="5" height="5" fill="url(#p)"/>"##.repeat(200)
    );
    assert_not_implemented(run(svg(&pattern)), "pattern");
}

/// Arcs become curves by their radius (kurbo, tolerance 0.1): a 30-byte arc became billions of curves.
#[test]
fn b2b_arcs_are_measured_by_the_curves_they_become() {
    let huge = svg(r##"<path transform="scale(1e-27)" d="M0 0A1e27 1e27 0 1 1 1e27 0" stroke="#000000"/>"##);
    assert_too_large(run(huge), "would become more than");
    let circle = svg(r##"<circle transform="scale(1e-30)" r="1e30" stroke="#000000"/>"##);
    assert_too_large(run(circle), "would become more than");
    assert_not_implemented(run(svg(r##"<circle r="50%" stroke="#000000"/>"##)), "radii in %");
    let ordinary = svg(
        r##"<path d="M1 1A4 4 0 1 1 9 9a2 3 30 0 0 1 1" stroke="#000000"/><circle cx="10" cy="10" r="5" stroke="#000000"/><rect x="1" y="1" width="5" height="5" rx="1mm" stroke="#000000"/>"##,
    );
    assert_eq!(run(ordinary).expect("arcs import").shapes.len(), 3);
}

/// A single 20 MB `d` cost about 1 GB; repeated ClosePath recursed once each inside svgtypes (100,000 overflowed).
#[test]
fn b2b_path_data_is_bounded() {
    let mut long = String::from("M0 0L1 1");
    long.push_str(&" ".repeat(MAX_SVG_PATH_DATA_BYTES));
    assert_too_large(run(svg(&format!(r##"<path d="{long}" stroke="#000000"/>"##))), "d attribute");
    let closes = svg(&format!(r##"<path d="M0 0L1 1{}" stroke="#000000"/>"##, "Z".repeat(100_000)));
    assert_too_large(run(closes), "draw nothing");
    let at_limit = svg(&format!(r##"<path d="M0 0L1 1{}" stroke="#000000"/>"##, "Z".repeat(MAX_SVG_EMPTY_SEGMENTS + 1)));
    assert!(run(at_limit).is_ok());
    let points = svg(&format!(r##"<polyline points="{}" stroke="#000000"/>"##, " ".repeat(MAX_SVG_PATH_DATA_BYTES + 1)));
    assert_too_large(run(points), "points attribute");
}

/// usvg copies text, styles, dash arrays and gradient stops into every element `<use>` expands.
#[test]
fn b2b_copies_per_expanded_element_are_bounded() {
    let text = svg(&format!(
        r##"<defs><text id="t">{}</text></defs>{}<path d="M0 0L1 1" stroke="#000000"/>"##,
        "x".repeat(1_000_000),
        r##"<use href="#t"/>"##.repeat(300)
    ));
    assert_too_large(run(text), "bytes of attributes and text");
    let dashes = svg(&format!(r##"<g stroke-dasharray="{}">{}</g>"##, "1 ".repeat(3_000), r##"<path d="M0 0L1 1" stroke="#000000"/>"##.repeat(12_000)));
    assert_too_large(run(dashes), "read or copy");
    let stops: String = (0..1_000).map(|i| format!(r##"<stop offset="{}" stop-color="#000000"/>"##, f64::from(i) / 1_000.0)).collect();
    let gradient = svg(&format!(
        r##"<defs><linearGradient id="g">{stops}</linearGradient></defs><g fill="url(#g)">{}</g>"##,
        r##"<rect width="1" height="1"/>"##.repeat(10_000)
    ));
    assert_too_large(run(gradient), "read or copy");
}

// ---- styles ---------------------------------------------------------------------------------------------------------

/// simplecss matches combinators by recursion and backtracking (`a+a+…` overflowed the stack, `x g g … path` is
/// exponential). Only element, class and id selectors are accepted, and their work is bounded.
#[test]
fn style_sheets_are_bounded() {
    let siblings = svg(&format!("<style>{}{{fill:red}}</style>{}", vec!["a"; 30_000].join("+"), "<a/>".repeat(30_000)));
    assert_not_implemented(run(siblings), "selectors");
    let descendant = svg(&format!(
        r##"<style>x {} path{{fill:red}}</style>{}<path d="M0 0L1 1" stroke="#000000"/>{}"##,
        vec!["g"; 30].join(" "),
        "<g>".repeat(200),
        "</g>".repeat(200)
    ));
    assert_not_implemented(run(descendant), "selectors");
    for rule in ["g>path{fill:red}", "[fill]{fill:red}", "path:first-child{fill:red}", "@import url(x.css);", "p{fill:red}}", r"p{fill:\72 ed}"] {
        assert!(matches!(run(svg(&format!("<style>{rule}</style>{SHAPE}"))), Err(Error::NotImplemented(_) | Error::Malformed { .. })), "{rule}");
    }
    let many: String = (0..2_000).map(|i| format!(".c{i}{{stroke-width:1}}")).collect();
    let crowded = svg(&format!("<style>{many}</style>{}", r##"<path d="M0 0L1 1" stroke="#000000"/>"##.repeat(20_000)));
    assert_too_large(run(crowded), "comparisons");
    let long_class = svg(&format!(r##"<style>.a{{stroke:#000000}}</style><path class="{}" d="M0 0L1 1"/>"##, "a ".repeat(200)));
    assert_too_large(run(long_class), "class or id");
    // An ordinary class sheet imports and is applied (the stroke comes from the sheet).
    let ordinary = svg(r##"<style><![CDATA[ .cut, path.alt { fill: none; stroke: #000000 } /* comment */ ]]></style><path class="cut" d="M1 1L9 9"/>"##);
    let imported = run(ordinary).expect("a class style sheet imports");
    assert_eq!(imported.shapes.len(), 1);
    assert!(!imported.notices.iter().any(|n| n.message.contains("invisible")), "{:?}", imported.notices);
}

#[test]
fn tref_and_element_fe_images_are_refused() {
    assert_not_implemented(run(svg(&format!(r##"<text id="t">hi</text><text><tref href="#t"/></text>{SHAPE}"##))), "tref");
    assert_not_implemented(run(svg(&format!(r##"<defs><filter id="f"><feImage href="#r"/></filter></defs><g id="r">{SHAPE}</g>"##))), "feImage");
}

// ---- hidden geometry is never burned --------------------------------------------------------------------------------

#[test]
fn hidden_svg_geometry_is_skipped_with_a_notice() {
    let visible = r##"<path d="M1 1L9 9" stroke="#000000"/>"##;
    let body = format!(
        r##"{visible}<path d="M1 2L9 9" stroke="#000000" display="none"/><g style="display: none !important">{visible}{visible}</g><path d="M1 3L9 9" stroke="#000000" visibility="hidden"/><g opacity="0">{visible}</g><path d="M1 4L9 9" fill="none" stroke="#000000" stroke-opacity="0"/><path d="M1 5L9 9" fill="none"/><path d="M1 6L9 9L1 9Z" fill="#000000" fill-opacity="0"/>"##
    );
    let imported = run(svg(&body)).expect("imports the visible path");
    assert_eq!(imported.shapes.len(), 1, "only the visible path is imported");
    let display = notice(&imported, "display:none");
    assert!(display.message.starts_with("2 SVG elements"), "{}", display.message);
    let invisible = notice(&imported, "invisible");
    assert!(invisible.message.starts_with("5 SVG paths"), "{}", invisible.message);
    assert!(imported.notices.iter().all(|n| n.code == NoticeCode::ImportEntitySkipped));
    assert_eq!(imported.notices.iter().map(|n| n.n).collect::<Vec<_>>(), (1..=u32::try_from(imported.notices.len()).unwrap()).collect::<Vec<_>>());
    // A sheet that hides says so; a file whose paths are all hidden is refused, naming why.
    let sheet = run(svg(&format!("<style>.h{{display:none}}</style>{visible}<path class=\"h\" d=\"M0 0L1 1\" stroke=\"#000000\"/>"))).expect("imports");
    assert_eq!(sheet.shapes.len(), 1);
    notice(&sheet, "style sheet hides");
    match run(svg(r##"<path d="M1 1L9 9" stroke="#000000" visibility="hidden"/>"##)) {
        Err(Error::Malformed { message, .. }) => assert!(message.contains("invisible"), "{message}"),
        other => panic!("expected a refusal naming the hidden path, got {other:?}"),
    }
}

/// The walk ignores clips, so clipped geometry is refused rather than burned whole: clip paths and masks from the
/// file before usvg runs, and the viewport clip usvg adds for a `<symbol>` or a sized nested `<svg>` after.
#[test]
fn clipped_svg_geometry_is_refused() {
    let symbol = r##"<defs><symbol id="s" viewBox="0 0 10 10"><path d="M0 0L20 20" stroke="#000000"/></symbol></defs><use href="#s" width="10" height="10"/>"##;
    assert_not_implemented(run(svg(symbol)), "viewport");
    let nested = r##"<svg x="0" y="0" width="5" height="5"><path d="M0 0L20 20" stroke="#000000"/></svg>"##;
    assert_not_implemented(run(svg(nested)), "viewport");
    let visible_overflow = r##"<defs><symbol id="s" viewBox="0 0 10 10" overflow="visible"><path d="M0 0L20 20" stroke="#000000"/></symbol></defs><use href="#s" width="10" height="10"/>"##;
    assert_eq!(run(svg(visible_overflow)).expect("an unclipped symbol imports").shapes.len(), 1);
    // A CSS filter function needs no reference but still changes what is shown.
    assert_not_implemented(run(svg(r##"<g filter="blur(2)"><path d="M0 0L20 20" stroke="#000000"/></g>"##)), "filter");
    // usvg does not clip a nested <svg> without both width and height, browsers do: refused before usvg runs.
    for no_size in [
        r##"<svg viewBox="0 0 10 10"><path d="M0 0L20 20" stroke="#000000"/></svg>"##,
        r##"<svg width="10"><path d="M0 0L20 20" stroke="#000000"/></svg>"##,
        r##"<defs><svg id="n" height="10"><path d="M0 0L20 20" stroke="#000000"/></svg></defs><use href="#n" width="5" height="5"/>"##,
    ] {
        assert_not_implemented(run(svg(no_size)), "nested SVG <svg> elements without both width and height");
    }
    let unclipped = r##"<svg width="10" height="10" overflow="visible"><path d="M0 0L20 20" stroke="#000000"/></svg>"##;
    assert_eq!(run(svg(unclipped)).expect("a nested svg with overflow visible is not clipped").shapes.len(), 1);
}

// ---- the guard's own work stays linear ------------------------------------------------------------------------------

/// Many elements sharing one id, each named by many references: `<use>` resolves the way usvg does (the first element
/// with that id), each paint id is looked up once, and a gradient `href` naming a shared id is refused.
#[test]
fn shared_ids_keep_the_guard_linear() {
    let uses = svg(&format!(
        r##"<defs><path id="a" d="M0 0L1 1" stroke="#000000"/>{}</defs>{}"##,
        r#"<g id="a"/>"#.repeat(100_000),
        r##"<use href="#a"/>"##.repeat(20_000)
    ));
    assert_eq!(run(uses).expect("imports").shapes.len(), 20_000);
    let gradients = svg(&format!(
        r##"<defs>{}</defs>{}"##,
        r##"<linearGradient id="g" href="#g"/>"##.repeat(50_000),
        r##"<rect width="1" height="1" fill="url(#g)"/>"##.repeat(50_000)
    ));
    assert_not_implemented(run(gradients), "several elements share");
}

/// An ordinary large drawing (class styles, a shared userSpaceOnUse gradient as Inkscape, Illustrator and Corel write
/// them, 50,000 paths) imports within the time bound.
#[test]
fn large_ordinary_svg_imports_quickly() {
    let paths: String =
        (0..50_000).map(|i| format!(r#"<path class="{}" d="M{} 1L{} 9"/>"#, if i % 2 == 0 { "cut" } else { "fill" }, i % 20, (i + 3) % 20)).collect();
    let doc = svg(&format!(
        r##"<style>.cut{{fill:none;stroke:#000000;stroke-width:0.1}} .fill{{fill:url(#g)}}</style><defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x2="20"><stop offset="0" stop-color="#000000"/><stop offset="1" stop-color="#ffffff"/></linearGradient></defs>{paths}"##
    ));
    let started = Instant::now();
    let imported = run(doc).expect("imports");
    assert_eq!(imported.shapes.len(), 50_000);
    eprintln!("50,000 paths in {:?}", started.elapsed());
}

// ---- the second re-review (B1–B9 below are its numbering) ---------------------------------------------------------------

/// B9: usvg ignores a clip-path or mask that is not a `url(…)` (CSS shapes, gradients, images), so the clipped-away
/// geometry would be burned. Any value but `none` is refused, in attributes, `style` and sheets.
#[test]
fn b9_non_url_clips_and_masks_are_refused() {
    let line = r##"<path d="M0 0L20 20" stroke="#000000""##;
    for doc in [
        format!(r#"{line} clip-path="circle(1px)"/>"#),
        format!(r#"{line} style="clip-path: inset(5px)"/>"#),
        format!(r#"<style>.c{{clip-path:polygon(0 0, 10 0, 10 10)}}</style>{line} class="c"/>"#),
        format!(r#"{line} mask="linear-gradient(red, blue)"/>"#),
        format!(r#"<g style="MASK: Linear-Gradient(red, blue)">{line}/></g>"#),
        format!(r#"<g style="-webkit-mask-image: linear-gradient(red, blue)">{line}/></g>"#),
        format!(r#"<style>g{{-webkit-clip-path:inset(1px)}}</style><g>{line}/></g>"#),
    ] {
        assert_not_implemented(run(svg(&doc)), "clip");
    }
    let none = format!(r#"<g clip-path="none" style="mask: None; clip-rule: evenodd">{line}/></g>"#);
    assert_eq!(run(svg(&none)).expect("clip-path none imports").shapes.len(), 1);
}

/// B1: usvg outlines every stroke to measure it; a 1e16-wide stroke on 200 curves took 2.18 GB. The widest
/// stroke-width anywhere (attribute, style, sheet, inherited), in user units, is bounded; relative units are refused.
#[test]
fn b1_huge_stroke_widths_are_refused() {
    let curves = format!(r##"<path d="M0 0{}" stroke="#000000"/>"##, "C10 0 0 10 10 10".repeat(200));
    for holder in [r#"<g stroke-width="1e16">"#, r#"<g style="stroke-width: 1E16px">"#, r#"<g stroke-width="11in">"#, r#"<g stroke-width="1001">"#] {
        assert_too_large(run(svg(&format!("{holder}{curves}</g>"))), "stroke-width of");
    }
    assert_too_large(run(svg(&format!("<style>path{{stroke-width:2e3}}</style>{curves}"))), "stroke-width of");
    for relative in ["5%", "1em", "100ex"] {
        assert_not_implemented(run(svg(&format!(r#"<g stroke-width="{relative}">{curves}</g>"#))), "stroke widths in %");
    }
    let ordinary = format!(r#"<g stroke-width="0.1mm">{curves}</g><g stroke-width="1000">{curves}</g><g stroke-width="inherit">{curves}</g>"#);
    assert_eq!(run(svg(&ordinary)).expect("ordinary stroke widths import").shapes.len(), 3);
}

/// B2: usvg walks every child node of every element `<use>` clones, comments and text included.
#[test]
fn b2_child_nodes_amplified_by_use_are_bounded() {
    for filler in ["<!---->", "<?p?>", "<g/> "] {
        let doc = svg(&format!(
            r##"<defs><g id="c">{}</g></defs>{}<path d="M0 0L1 1" stroke="#000000"/>"##,
            filler.repeat(100_000),
            r##"<use href="#c"/>"##.repeat(2_000)
        ));
        let result = run(doc);
        if filler == "<g/> " {
            // Elements are refused by the element bound first.
            assert_too_large(result, "elements once <use> references are expanded");
        } else {
            assert_too_large(result, "XML nodes");
        }
    }
}

/// B3: usvg looks up every attribute of every element `<use>` clones; empty values cost nothing in bytes.
#[test]
fn b3_attributes_amplified_by_use_are_bounded() {
    let attributes: String = (0..1_000).map(|i| format!(r#" a{i}="""#)).collect();
    let doc = svg(&format!(r##"<defs><g id="a"{attributes}/></defs>{}<path d="M0 0L1 1" stroke="#000000"/>"##, r##"<use href="#a"/>"##.repeat(100_000)));
    assert_too_large(run(doc), "bytes of attributes and text");
}

/// B4: roxmltree compares each attribute of a start tag with every one before it, before the guard sees the tree.
#[test]
fn b4_attribute_counts_are_bounded_before_parsing() {
    let attributes = |n: usize| -> String { (0..n).map(|i| format!(r#" a{i}="""#)).collect() };
    let one = svg(&format!(r##"<g{}/><path d="M0 0L1 1" stroke="#000000"/>"##, attributes(MAX_SVG_ATTRIBUTES + 1)));
    assert_too_large(run(one), &format!("has {} attributes", MAX_SVG_ATTRIBUTES + 1));
    let many = svg(&format!(r##"{}<path d="M0 0L1 1" stroke="#000000"/>"##, format!("<g{}/>", attributes(MAX_SVG_ATTRIBUTES)).repeat(300)));
    assert_too_large(run(many), "duplicates");
    // `=` inside a quoted value is not an attribute.
    let quoted = svg(&format!(r##"<g data-x="{}"/><path d="M0 0L1 1" stroke="#000000"/>"##, "=".repeat(10_000)));
    assert_eq!(run(quoted).expect("imports").shapes.len(), 1);
}

/// B5: roxmltree resolves namespaces with linear scans per element.
#[test]
fn b5_namespace_declarations_are_bounded() {
    let declarations: String = (0..MAX_SVG_NAMESPACES).map(|i| format!(r#" xmlns:n{i}="urn:n{i}""#)).collect();
    let root = format!(r##"<svg xmlns="http://www.w3.org/2000/svg"{declarations} width="1mm" height="1mm"><path d="M0 0L1 1" stroke="#000000"/></svg>"##);
    assert_too_large(run(root), "XML namespaces");
    let children = svg(&format!(r##"{}<path d="M0 0L1 1" stroke="#000000"/>"##, r#"<g xmlns:z = "urn:z"/>"#.repeat(100_000)));
    assert_too_large(run(children), "XML namespaces");
}

/// B6: usvg's recursive-pattern pass is quadratic even for patterns nothing uses: every <pattern> is refused.
#[test]
fn b6_unused_patterns_are_refused() {
    let rects = r##"<rect width="1" height="1" fill="#000000"/>"##.repeat(50);
    let patterns: String = (0..2_000).map(|i| format!(r#"<pattern id="p{i}" width="1" height="1">{rects}</pattern>"#)).collect();
    let doc = svg(&format!(r##"<defs>{patterns}</defs><path d="M0 0L1 1" stroke="#000000"/>"##));
    assert_not_implemented(run(doc), "<pattern> elements");
    assert_not_implemented(run(svg(r##"<defs><pattern id="p"/></defs><path d="M0 0L1 1" stroke="#000000"/>"##)), "<pattern> elements");
}

/// B7: usvg copies an objectBoundingBox gradient for every path it paints and deduplicates the copies with a linear
/// scan each (quadratic); a userSpaceOnUse gradient is shared.
#[test]
fn b7_gradient_copies_are_bounded() {
    let stops = r##"<stop offset="0" stop-color="#000000"/><stop offset="1" stop-color="#ffffff"/>"##;
    let rects = r#"<rect width="1" height="1"/>"#.repeat(25_000);
    let object_units = svg(&format!(r##"<defs><linearGradient id="g">{stops}</linearGradient></defs><g fill="url(#g)">{rects}</g>"##));
    assert_too_large(run(object_units), "gradients");
    let inherited = svg(&format!(
        r##"<defs><linearGradient id="b" gradientUnits="objectBoundingBox">{stops}</linearGradient><linearGradient id="g" href="#b"/></defs><g style="fill:url(#g)">{rects}</g>"##
    ));
    assert_too_large(run(inherited), "gradients");
    let distinct: String = (0..25_000)
        .map(|i| {
            format!(r##"<linearGradient id="g{i}" gradientUnits="userSpaceOnUse">{stops}</linearGradient><rect width="1" height="1" fill="url(#g{i})"/>"##)
        })
        .collect();
    assert_too_large(run(svg(&distinct)), "gradients");
    // A shared userSpaceOnUse gradient, and an objectBoundingBox one on a few paths of a large drawing, import.
    let shared = svg(&format!(r##"<defs><linearGradient id="g" gradientUnits="userSpaceOnUse">{stops}</linearGradient></defs><g fill="url(#g)">{rects}</g>"##));
    assert_eq!(run(shared).expect("a shared gradient imports").shapes.len(), 25_000);
    let few = svg(&format!(
        r##"<defs><linearGradient id="g">{stops}</linearGradient></defs><g fill="url(#g)">{}</g><g stroke="#000000">{rects}</g>"##,
        r#"<rect width="1" height="1"/>"#.repeat(10)
    ));
    assert_eq!(run(few).expect("a few gradient paths import").shapes.len(), 25_010);
}

/// B8: simplecss tests every part of a compound selector (`.a.a.a…`) against every element.
#[test]
fn b8_css_selector_parts_are_bounded() {
    let doc = svg(&format!(r#"<style>{}{{fill:red}}</style>{}"#, ".a".repeat(2_000), r#"<path class="a" d="M0 0L1 1"/>"#.repeat(20_000)));
    assert_too_large(run(doc), "selector parts");
}

/// simplecss computes a line and column from the start of the style text at every token it rejects (each
/// declaration's end, each number without a unit): long `style` attributes and sheets cost quadratic time.
#[test]
fn style_text_parsing_is_bounded() {
    let styles = svg(&format!(r#"<path style="{}" stroke="black" d="M0 0L1 1"/>"#, "fill:red;".repeat(1_800)).repeat(50));
    assert_too_large(run(styles), "rescan");
    let dashes = svg(&format!(r#"<path style="stroke-dasharray:{}" stroke="black" d="M0 0L1 1"/>"#, "1 ".repeat(30_000)));
    assert_too_large(run(dashes), "rescan");
    let sheet = svg(&format!(r#"<style>a{{stroke-dasharray:{}}}</style><path stroke="black" d="M0 0L1 1"/>"#, "1 ".repeat(30_000)));
    assert_too_large(run(sheet), "rescan");
    let used = svg(&format!(
        r##"<defs><path id="p" style="{}" stroke="black" d="M0 0L1 1"/></defs>{}"##,
        "fill:red;".repeat(400),
        r##"<use href="#p"/>"##.repeat(2_000)
    ));
    assert_too_large(run(used), "rescan");
}

/// usvg looks inherited properties up by scanning the attributes of every ancestor, for every element: deep nesting
/// with many attributes (or many style declarations) per level is quadratic.
#[test]
fn ancestor_attribute_scans_are_bounded() {
    let names = ["opacity", "x", "y", "width", "height", "rx", "ry", "cx", "cy", "r", "x1", "y1", "x2", "y2", "offset"];
    let mut attributes = String::new();
    for prefix in ["", "s:", "l:", "xml:"] {
        for name in names {
            attributes.push_str(&format!(r#" {prefix}{name}="1""#));
        }
    }
    let doc = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:s="http://www.w3.org/2000/svg" xmlns:l="http://www.w3.org/1999/xlink" width="1mm" height="1mm">{}{}{}</svg>"##,
        format!("<g{attributes}>").repeat(250),
        r#"<path d="M0 0L1 1" stroke="red"/>"#.repeat(50_000),
        "</g>".repeat(250)
    );
    assert_too_large(run(doc), "attribute comparisons");
    let declarations: String = ["fill:red", "stroke:red", "opacity:1", "fill-opacity:1", "stroke-opacity:1", "stroke-linecap:butt"].join(";");
    let sheet = svg(&format!(
        "<style>*{{{}}}</style>{}{}{}",
        [declarations.as_str(); 10].join(";"),
        "<g>".repeat(250),
        r#"<path d="M0 0L1 1"/>"#.repeat(50_000),
        "</g>".repeat(250)
    ));
    assert_too_large(run(sheet), "attribute comparisons");
}

/// A DOCTYPE internal subset is refused with advice that works (re-export; deleting the subset leaves the references
/// undefined), and an undefined entity reference says what it is.
#[test]
fn entity_declarations_and_references_are_explained() {
    match run(format!(r#"<!DOCTYPE svg [<!ENTITY ns_extend "http://ns.adobe.com/Extensibility/1.0/">]>{}"#, svg(""))) {
        Err(Error::NotImplemented(m)) => assert!(m.contains("export the file again") && !m.contains("delete"), "{m}"),
        other => panic!("expected the internal subset refusal, got {other:?}"),
    }
    let dangling = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="&ns_extend;" width="1mm" height="1mm"><path d="M0 0L1 1" stroke="#000000"/></svg>"##;
    match run(dangling.to_owned()) {
        Err(Error::Malformed { message, .. }) => assert!(message.contains("&ns_extend; (line 1") && message.contains("export the file again"), "{message}"),
        other => panic!("expected the undefined entity refusal, got {other:?}"),
    }
}

/// What usvg drops without a word gets a numbered notice: foreignObject content, failed conditions outside <switch>,
/// degenerate transforms and coordinates beyond f32. A viewBox beyond f32 (usvg ignores it: wrong scale) is refused.
#[test]
fn silently_dropped_svg_content_gets_notices() {
    let visible = r##"<path d="M1 1L9 9" stroke="#000000"/>"##;
    let body = format!(
        r##"{visible}
        <foreignObject width="10" height="10"><div xmlns="http://www.w3.org/1999/xhtml">hi</div></foreignObject>
        <path requiredFeatures="http://www.w3.org/TR/SVG11/feature#Extensibility" d="M1 2L9 9" stroke="#000000"/>
        <g systemLanguage="fr"><path d="M1 3L9 9" stroke="#000000"/></g>
        <path requiredExtensions="http://example.org/x" d="M1 4L9 9" stroke="#000000"/>
        <switch><path systemLanguage="fr" d="M1 5L9 9" stroke="#000000"/><path systemLanguage="en-GB" d="M1 6L9 9" stroke="#000000"/></switch>
        <path transform="scale(0)" d="M1 7L9 9" stroke="#000000"/>
        <g style="transform: scale(0, 1)"><path d="M1 8L9 9" stroke="#000000"/></g>
        <g style="transform: rotate(30) scale(0, 1)"><path d="M1 9L9 9" stroke="#000000"/></g>
        <path transform="translate(1e39)" d="M2 1L9 9" stroke="#000000"/>
        <path d="M2 2L1e39 9" stroke="#000000"/>
        <rect width="1e39" height="1" stroke="#000000"/>
        <path d="M2 3L9 9" stroke="#000000" transform="translate(1 2) scale(1e-30)"/>"##
    );
    let imported = run(svg(&body)).expect("imports the visible paths");
    // A singular matrix whose scale factors are not zero (rotate, then flatten) is kept by usvg, as a line.
    assert_eq!(imported.shapes.len(), 3, "the first path, the switch's English path and the flattened path: {:?}", imported.notices);
    assert!(notice(&imported, "<foreignObject>").message.starts_with("1 SVG"));
    assert!(notice(&imported, "outside <switch>").message.starts_with("3 SVG elements"));
    // scale(0), scale(0, 1) in a style, translate(1e39) and scale(1e-30) (its square underflows f32, as in tiny-skia).
    assert!(notice(&imported, "scales to zero").message.starts_with("4 SVG elements"));
    assert!(notice(&imported, "beyond ±3.4e38").message.starts_with("2 SVG elements"));
    assert_eq!(imported.notices.iter().map(|n| n.n).collect::<Vec<_>>(), (1..=u32::try_from(imported.notices.len()).unwrap()).collect::<Vec<_>>());
    let sheet = run(svg(&format!("<style>.z{{transform:scale(0)}}</style>{visible}<path class=\"z\" d=\"M0 0L1 1\" stroke=\"#000000\"/>"))).expect("imports");
    assert_eq!(sheet.shapes.len(), 1);
    notice(&sheet, "style sheet sets a transform");
    let view_box = r##"<svg xmlns="http://www.w3.org/2000/svg" width="1mm" height="1mm" viewBox="0 0 1e39 1e39"><path d="M0 0L1 1" stroke="#000000"/></svg>"##;
    assert_too_large(run(view_box.to_owned()), "viewBox");
}

/// Typical exports from the editors laser users have still import: Inkscape 1.x (namespaces, metadata, a gradient
/// chain in userSpaceOnUse units), Illustrator 2023 (CSS classes; SVG 1.1 Save As with a generator comment),
/// CorelDRAW (CDATA class sheet, `clip-rule` on the root) and a W3C sample with a plain DOCTYPE.
#[test]
fn real_world_svgs_still_import() {
    let inkscape = r##"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<!-- Created with Inkscape (http://www.inkscape.org/) -->
<svg width="100mm" height="50mm" viewBox="0 0 100 50" version="1.1" id="svg5" inkscape:version="1.3 (0e150ed6c4, 2023-07-21)" sodipodi:docname="drawing.svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns="http://www.w3.org/2000/svg" xmlns:svg="http://www.w3.org/2000/svg" xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:cc="http://creativecommons.org/ns#" xmlns:dc="http://purl.org/dc/elements/1.1/">
  <sodipodi:namedview id="namedview7" pagecolor="#ffffff" bordercolor="#000000" borderopacity="0.25" inkscape:showpageshadow="2" inkscape:pageopacity="0.0" inkscape:pagecheckerboard="0" inkscape:deskcolor="#d1d1d1" inkscape:document-units="mm" inkscape:zoom="1.4" inkscape:cx="200" inkscape:cy="100" inkscape:window-width="1920" inkscape:window-height="1011" inkscape:window-x="0" inkscape:window-y="0" inkscape:window-maximized="1" inkscape:current-layer="layer1" />
  <defs id="defs2">
    <linearGradient id="linearGradient1" inkscape:collect="always"><stop style="stop-color:#000000;stop-opacity:1;" offset="0" id="stop1" /><stop style="stop-color:#000000;stop-opacity:0;" offset="1" id="stop2" /></linearGradient>
    <linearGradient inkscape:collect="always" xlink:href="#linearGradient1" id="linearGradient3" x1="10" y1="10" x2="40" y2="10" gradientUnits="userSpaceOnUse" />
  </defs>
  <metadata id="metadata1"><rdf:RDF><cc:Work rdf:about=""><dc:format>image/svg+xml</dc:format></cc:Work></rdf:RDF></metadata>
  <g inkscape:label="Layer 1" inkscape:groupmode="layer" id="layer1" transform="translate(-10,-20)">
    <rect style="fill:none;stroke:#000000;stroke-width:0.264583;stroke-linecap:round;stroke-linejoin:round" id="rect1" width="40" height="20" x="20" y="30" ry="2" />
    <path style="fill:url(#linearGradient3);fill-opacity:1;stroke:#ff0000;stroke-width:0.264583px;stroke-linecap:butt;stroke-linejoin:miter;stroke-opacity:1" d="m 20,60 c 10,-5 20,5 30,0 z" id="path1" sodipodi:nodetypes="ccc" />
    <circle style="fill:none;stroke:#0000ff;stroke-width:0.5" id="path2" cx="80" cy="45" r="8" />
  </g>
</svg>
"##;
    let illustrator_css = r#"<?xml version="1.0" encoding="UTF-8"?>
<svg id="Layer_1" data-name="Layer 1" xmlns="http://www.w3.org/2000/svg" width="72mm" height="36mm" viewBox="0 0 204.09 102.05">
  <defs>
    <style>
      .cls-1 {
        fill: none;
        stroke: #000;
        stroke-miterlimit: 10;
      }

      .cls-2 {
        fill: #ed1c24;
      }
    </style>
  </defs>
  <rect class="cls-1" x="10.5" y="10.5" width="80" height="40"/>
  <circle class="cls-2" cx="150" cy="50" r="30"/>
  <polyline class="cls-1" points="10 90 50 70 90 90"/>
</svg>
"#;
    let illustrator_save_as = r#"<?xml version="1.0" encoding="utf-8"?>
<!-- Generator: Adobe Illustrator 27.0.0, SVG Export Plug-In . SVG Version: 6.00 Build 0)  -->
<svg version="1.1" id="Layer_1" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" x="0px" y="0px"
	 viewBox="0 0 200 100" style="enable-background:new 0 0 200 100;" xml:space="preserve">
<style type="text/css">
	.st0{fill:none;stroke:#000000;stroke-miterlimit:10;}
	.st1{fill:#FF0000;}
</style>
<rect x="10" y="10" class="st0" width="80" height="40"/>
<path class="st1" d="M150,20c16.6,0,30,13.4,30,30s-13.4,30-30,30s-30-13.4-30-30S133.4,20,150,20z"/>
</svg>
"#;
    let corel = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- Creator: CorelDRAW 2021 (64-Bit) -->
<svg xmlns="http://www.w3.org/2000/svg" xml:space="preserve" width="50mm" height="30mm" version="1.1" style="shape-rendering:geometricPrecision; text-rendering:geometricPrecision; image-rendering:optimizeQuality; fill-rule:evenodd; clip-rule:evenodd"
viewBox="0 0 5000 3000"
 xmlns:xlink="http://www.w3.org/1999/xlink"
 xmlns:xodm="http://www.corel.com/coreldraw/odm/2003">
 <defs>
  <style type="text/css">
   <![CDATA[
    .str0 {stroke:black;stroke-width:7.62;stroke-miterlimit:22.9256}
    .fil0 {fill:none}
    .fil1 {fill:#2B2A29}
   ]]>
  </style>
 </defs>
 <g id="Layer_x0020_1">
  <metadata id="CorelCorpID_0Corel-Layer"/>
  <rect class="fil0 str0" x="100" y="100" width="2000" height="1000"/>
  <path class="fil1" d="M3000 500l1000 0 0 1000 -1000 0z"/>
 </g>
</svg>
"#;
    let w3c = r#"<?xml version="1.0" standalone="no"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">
<svg width="12cm" height="4cm" viewBox="0 0 1200 400" xmlns="http://www.w3.org/2000/svg" version="1.1">
  <desc>Example rect01 - rectangle with sharp corners</desc>
  <rect x="1" y="1" width="1198" height="398" fill="none" stroke="blue" stroke-width="2"/>
  <rect x="400" y="100" width="400" height="200" fill="yellow" stroke="navy" stroke-width="10"/>
</svg>
"#;
    for (name, doc, units, shapes) in [
        ("inkscape", inkscape, SvgUnits::RequirePhysical, 3),
        ("illustrator css", illustrator_css, SvgUnits::RequirePhysical, 3),
        ("illustrator save as", illustrator_save_as, SvgUnits::Px72, 2),
        ("corel", corel, SvgUnits::RequirePhysical, 2),
        ("w3c", w3c, SvgUnits::RequirePhysical, 2),
    ] {
        let imported = run_with(doc.to_owned(), ImportOptions { svg_units: units, ..ImportOptions::default() }).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(imported.shapes.len(), shapes, "{name}: {:?}", imported.notices);
        assert!(imported.notices.iter().all(|n| n.code != NoticeCode::ImportEntitySkipped), "{name}: {:?}", imported.notices);
    }
}
