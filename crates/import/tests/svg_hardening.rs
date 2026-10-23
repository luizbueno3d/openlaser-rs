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

use openlaser_import::{Error, ImportOptions, Imported, MAX_SVG_DEPTH, MAX_SVG_EMPTY_SEGMENTS, MAX_SVG_PATH_DATA_BYTES, import};
use openlaser_types::NoticeCode;

const TIME_BOUND: Duration = Duration::from_secs(10);

fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="20mm" height="20mm" viewBox="0 0 20 20">{body}</svg>"#)
}

/// Imports `doc` on a 2 MiB thread, failing the test if it takes longer than [`TIME_BOUND`].
fn run(doc: String) -> Result<Imported, Error> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let started = Instant::now();
            let result = import("hostile.svg", doc.as_bytes(), &ImportOptions::default());
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
    let to_pattern =
        svg(r##"<defs><linearGradient id="a" href="#p"/><pattern id="p" width="1" height="1"/></defs><rect width="5" height="5" fill="url(#a)"/>"##);
    assert_not_implemented(run(to_pattern), "not a gradient");
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
    let crowded = svg(&format!("<style>{many}</style>{}", r##"<path d="M0 0L1 1" stroke="#000000"/>"##.repeat(10_000)));
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
    let no_size = r##"<svg viewBox="0 0 10 10"><path d="M0 0L20 20" stroke="#000000"/></svg>"##;
    assert_eq!(run(svg(no_size)).expect("a nested svg without a size is not clipped").shapes.len(), 1);
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

/// An ordinary large drawing (class styles, a shared gradient, 50,000 paths) imports within the time bound.
#[test]
fn large_ordinary_svg_imports_quickly() {
    let paths: String =
        (0..50_000).map(|i| format!(r#"<path class="{}" d="M{} 1L{} 9"/>"#, if i % 2 == 0 { "cut" } else { "fill" }, i % 20, (i + 3) % 20)).collect();
    let doc = svg(&format!(
        r##"<style>.cut{{fill:none;stroke:#000000;stroke-width:0.1}} .fill{{fill:url(#g)}}</style><defs><linearGradient id="g"><stop offset="0" stop-color="#000000"/><stop offset="1" stop-color="#ffffff"/></linearGradient></defs>{paths}"##
    ));
    let started = Instant::now();
    let imported = run(doc).expect("imports");
    assert_eq!(imported.shapes.len(), 50_000);
    eprintln!("50,000 paths in {:?}", started.elapsed());
}
