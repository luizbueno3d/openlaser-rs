//! Font discovery and layout, always against the committed Hack fixture — never the host's fonts.

use std::path::PathBuf;

use kurbo::PathEl;
use openlaser_text::{FontLibrary, TextError, layout};
use skrifa::MetadataProvider;

fn fixture_lib() -> FontLibrary {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fonts");
    FontLibrary::with_dirs(vec![dir])
}

fn bounds(path: &kurbo::BezPath) -> [f64; 4] {
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for el in path.elements() {
        let pts: &[kurbo::Point] = match el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => std::slice::from_ref(p),
            PathEl::CurveTo(.., p) => std::slice::from_ref(p),
            PathEl::QuadTo(.., p) => std::slice::from_ref(p),
            PathEl::ClosePath => &[],
        };
        for p in pts {
            lo[0] = lo[0].min(p.x);
            lo[1] = lo[1].min(p.y);
            hi[0] = hi[0].max(p.x);
            hi[1] = hi[1].max(p.y);
        }
    }
    [lo[0], lo[1], hi[0], hi[1]]
}

#[test]
fn discovery_indexes_hack_regular() {
    let lib = fixture_lib();
    assert_eq!(lib.entries().len(), 1, "{:?}", lib.entries());
    let e = &lib.entries()[0];
    assert_eq!((e.family.as_str(), e.style.as_str()), ("Hack", "Regular"));
    assert!(!e.italic && e.weight == 400);
    assert!(lib.families() == vec![("Hack".to_owned(), vec!["Regular".to_owned()])]);
    assert!(lib.face("Hack", None).is_some());
}

#[test]
fn the_cap_height_is_the_height_mm() {
    let lib = fixture_lib();
    let font = lib.face("Hack", None).unwrap();
    // 'H' alone: its outline's top should sit at exactly the cap height.
    let path = layout(&font, "H", 10.0, 0.0, 1.0, openlaser_types::TextAlign::Left).unwrap();
    let b = bounds(&path);
    assert!((b[3] - 10.0).abs() < 0.01, "{b:?}");
    assert!(b[1].abs() < 0.01, "the baseline lands on y = 0: {b:?}");
}

#[test]
fn advance_matches_the_shaped_positions() {
    let lib = fixture_lib();
    let font = lib.face("Hack", None).unwrap();
    // Hack is monospaced: "AV" is exactly twice one advance, and letter spacing widens it.
    let one = layout(&font, "A", 10.0, 0.0, 1.0, openlaser_types::TextAlign::Left).unwrap();
    let two = layout(&font, "AV", 10.0, 0.0, 1.0, openlaser_types::TextAlign::Left).unwrap();
    let b1 = bounds(&one);
    let b2 = bounds(&two);
    // The 'V' starts where 'A' ended; the right edge roughly doubles the advance.
    assert!(b2[2] > b1[2], "{b1:?} vs {b2:?}");
    let spaced = layout(&font, "AV", 10.0, 2.0, 1.0, openlaser_types::TextAlign::Left).unwrap();
    let b3 = bounds(&spaced);
    assert!(b3[2] > b2[2] + 1.5, "letter spacing widens the line: {b2:?} vs {b3:?}");
}

#[test]
fn multi_line_and_alignment() {
    let lib = fixture_lib();
    let font = lib.face("Hack", None).unwrap();
    let p = layout(&font, "A\nA", 10.0, 0.0, 1.0, openlaser_types::TextAlign::Left).unwrap();
    let b = bounds(&p);
    let metrics = font.metrics(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default());
    let step = (metrics.ascent - metrics.descent + metrics.leading) as f64 * (10.0 / metrics.cap_height.unwrap() as f64);
    assert!((b[1] + step).abs() < 0.02, "the second line sits one line-height down: {b:?} vs {step}");
    // Centered: the line is symmetric about x = 0.
    let p = layout(&font, "AA", 10.0, 0.0, 1.0, openlaser_types::TextAlign::Center).unwrap();
    let b = bounds(&p);
    assert!((b[0] + b[2]).abs() < 0.02, "{b:?}");
    // Right: the right edge lands within a sidebearing of x = 0 (the advance end is 0, ink stays inside).
    let p = layout(&font, "AA", 10.0, 0.0, 1.0, openlaser_types::TextAlign::Right).unwrap();
    let b = bounds(&p);
    assert!(b[2].abs() < 0.5 && b[2] < 0.0, "{b:?}");
}

#[test]
fn a_missing_glyph_is_refused_with_the_char() {
    let lib = fixture_lib();
    let font = lib.face("Hack", None).unwrap();
    let e = layout(&font, "中", 10.0, 0.0, 1.0, openlaser_types::TextAlign::Left).unwrap_err();
    match e {
        TextError::MissingGlyph { family, chars } => {
            assert_eq!(family, "Hack");
            assert_eq!(chars, "中");
        }
        other => panic!("{other}"),
    }
    assert!(layout(&font, "A", 0.0, 0.0, 1.0, openlaser_types::TextAlign::Left).is_err(), "height 0 is refused");
    assert!(layout(&font, "A", f64::NAN, 0.0, 1.0, openlaser_types::TextAlign::Left).is_err());
}

#[test]
fn hostile_files_are_skipped_not_crashing() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fonts");
    let hostile = std::env::temp_dir().join(format!("openlaser-text-hostile-{}", std::process::id()));
    std::fs::create_dir_all(&hostile).unwrap();
    // A truncated font and pure garbage: both skipped without a panic.
    let bytes = std::fs::read(dir.join("Hack-Regular.ttf")).unwrap();
    std::fs::write(hostile.join("truncated.ttf"), &bytes[..32.min(bytes.len())]).unwrap();
    std::fs::write(hostile.join("garbage.otf"), [0xDE, 0xAD, 0xBE, 0xEF, 1, 2, 3, 4]).unwrap();
    let lib = FontLibrary::with_dirs(vec![hostile.clone()]);
    assert!(lib.entries().is_empty(), "{:?}", lib.entries());
    std::fs::remove_dir_all(&hostile).ok();
}

#[test]
fn layout_is_deterministic() {
    let lib = fixture_lib();
    let font = lib.face("Hack", None).unwrap();
    let run = || layout(&font, "Hello\nWorld", 12.5, 0.5, 1.2, openlaser_types::TextAlign::Left).unwrap();
    assert_eq!(format!("{:?}", run()), format!("{:?}", run()));
}
