//! T4 (SPEC §6.2): OM importer parity. DXF rows must match exactly (polyline and point counts, coordinates — here
//! bit for bit, stricter than the 1e-9 rule); SVG and STL rows are informational (the shipping importer reads SVG
//! at true scale and has no STL); refusal rows must refuse.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use openlaser_import::{ImportOptions, Polyline, SvgUnits, dxf_om, import};
use openlaser_testkit::t4;

fn point_count(lines: &[Polyline]) -> usize {
    lines.iter().map(Polyline::len).sum()
}

#[test]
fn t4() {
    let cases = t4::load().expect("T4 fixtures load");
    assert_eq!(cases.len(), 30, "T4 has 30 cases");
    let (mut dxf_rows, mut refusals, mut informational) = (0usize, 0usize, 0usize);
    for case in &cases {
        let bytes = t4::source_bytes(case).expect("T4 source");
        if let Some(expected_sha) = case.source.get("sha256").and_then(serde_json::Value::as_str) {
            assert_eq!(openlaser_testkit::sha256_hex(&bytes), expected_sha, "{}: source file changed", case.name);
        }
        if case.error.is_some() {
            assert!(case.output.is_none(), "{}: a refusal row has no output", case.name);
            let refused = match case.kind.as_str() {
                "dxf" => dxf_om::import_geometry_om(&case.filename, &bytes).is_err(),
                _ => dxf_om::import_geometry_om(&case.filename, &bytes).is_err() && import(&case.filename, &bytes, &ImportOptions::default()).is_err(),
            };
            assert!(refused, "T4 {} must refuse", case.name);
            refusals += 1;
            continue;
        }
        let output = case.output.as_ref().expect("output");
        match case.kind.as_str() {
            "dxf" => {
                assert!(case.rust_must_match, "{}: DXF rows are parity rows", case.name);
                let actual = match case.function.as_str() {
                    "parse_dxf" => dxf_om::parse_dxf_om(&bytes).expect("DXF parses"),
                    "import_geometry" => dxf_om::import_geometry_om(&case.filename, &bytes).expect("DXF geometry imports"),
                    other => panic!("{}: unexpected T4 function {other}", case.name),
                };
                let expected = output.get("polylines").expect("expected polylines");
                assert_eq!(serde_json::to_value(&actual).expect("serialisable"), *expected, "T4 {}", case.name);
                if let Some(count) = output.get("polyline_count").and_then(serde_json::Value::as_u64) {
                    assert_eq!(actual.len() as u64, count, "T4 {} polyline count", case.name);
                }
                if let Some(count) = output.get("point_count").and_then(serde_json::Value::as_u64) {
                    assert_eq!(point_count(&actual) as u64, count, "T4 {} point count", case.name);
                }
                dxf_rows += 1;
            }
            "svg" => {
                assert!(!case.rust_must_match);
                let options = ImportOptions { svg_units: SvgUnits::Px96, ..ImportOptions::default() };
                let imported = import(&case.filename, &bytes, &options).unwrap_or_else(|e| panic!("T4 SVG {} imports: {e}", case.name));
                assert!(!imported.shapes.is_empty(), "T4 SVG {}", case.name);
                informational += 1;
            }
            "stl" => {
                assert!(!case.rust_must_match);
                assert!(import(&case.filename, &bytes, &ImportOptions::default()).is_err(), "STL is not imported: {}", case.name);
                informational += 1;
            }
            other => panic!("{}: unexpected T4 kind {other}", case.name),
        }
    }
    assert_eq!((dxf_rows, refusals, informational), (24, 2, 4), "T4 coverage");
}

#[test]
fn t4_business_card_dxf_counts() {
    let case = t4::load().unwrap().into_iter().find(|case| case.name == "businessCard.dxf.import_geometry").expect("business card");
    let bytes = t4::source_bytes(&case).unwrap();
    let lines = dxf_om::import_geometry_om(&case.filename, &bytes).unwrap();
    assert_eq!((lines.len(), point_count(&lines)), (403, 3057));
}
