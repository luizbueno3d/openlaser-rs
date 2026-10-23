//! T0 (SPEC §6.2): every row of the seven `tests/golden/t0_numeric` tables, through the SHIPPING encoders.
//!
//! Shipping must equal the table wherever OM neither clamps, wraps nor snaps NaN, and must be `Err` where OM does
//! (the `_om` variants are checked against every row too). `n_rows` and `inputs_f64_sha256` are recomputed per table
//! or group, and every cell is read strictly: a cell that is not the expected kind of value fails the test.
//!
//! SPEC §6.2 names `crates/types/tests/t0_numeric.rs`; it lives in `openlaser-lmc` because `openlaser-types` has no
//! `openlaser-testkit` dev-dependency (its manifest is outside WP4's scope).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use openlaser_testkit::t0::{Table, f64_bits_sha256, load};
use openlaser_types::galvo::{
    JPT_M7_PULSES_NS, PulseDecision, delay_word, distance_word, frequency_to_period, frequency_to_period_om, mm_to_galvo, mm_to_galvo_om, nearest_pulse_width,
    nearest_pulse_width_om, power_to_mark_current, speed_to_units,
};
use serde_json::{Map, Value};

/// README "T0": 7 tables, 155,054 rows.
const TOTAL_ROWS: usize = 155_054;

fn num(v: &Value, ctx: &str) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("{ctx}: {v} is not a number"))
}
fn word(v: &Value, ctx: &str) -> u16 {
    v.as_u64().and_then(|n| u16::try_from(n).ok()).unwrap_or_else(|| panic!("{ctx}: {v} is not a 16-bit word"))
}
fn text<'a>(v: &'a Value, ctx: &str) -> &'a str {
    v.as_str().unwrap_or_else(|| panic!("{ctx}: {v} is not a string"))
}
fn special(v: &Value, ctx: &str) -> f64 {
    match text(v, ctx) {
        "inf" => f64::INFINITY,
        "-inf" => f64::NEG_INFINITY,
        "nan" => f64::NAN,
        other => panic!("{ctx}: unknown special {other}"),
    }
}
fn param<'a>(map: &'a Map<String, Value>, key: &str, ctx: &str) -> &'a Value {
    map.get(key).unwrap_or_else(|| panic!("{ctx}: missing {key}"))
}
fn cells<'a>(row: &'a [Value], n: usize, ctx: &str) -> &'a [Value] {
    assert_eq!(row.len(), n, "{ctx}: row {row:?} has {} cells, expected {n}", row.len());
    row
}

fn table(file: &str, function: &str) -> Table {
    let t = load(file).unwrap_or_else(|e| panic!("{file}: {e}"));
    assert_eq!(t.function, function, "{file}");
    t
}

/// `n_rows` must count the rows, and `inputs_f64_sha256` must be the hash of the float inputs as parsed.
fn check_header(params: &Map<String, Value>, rows: &[Vec<Value>], inputs: impl Fn(&[Value]) -> Vec<f64>, ctx: &str) {
    assert_eq!(param(params, "n_rows", ctx).as_u64(), u64::try_from(rows.len()).ok(), "{ctx}: n_rows");
    let expected = text(param(params, "inputs_f64_sha256", ctx), ctx);
    let got = f64_bits_sha256(rows.iter().flat_map(|r| inputs(r)));
    assert_eq!(got, expected, "{ctx}: inputs_f64_sha256 (a float input was not parsed exactly)");
}

fn rows_in(t: &Table) -> usize {
    t.rows.len() + t.groups.iter().map(|g| g.rows.len()).sum::<usize>()
}

#[test]
fn t0_tables_hold_every_reference_row() {
    let files = [
        ("mm_to_galvo.json", "mm_to_galvo"),
        ("power_to_mark_current.json", "power_to_mark_current"),
        ("frequency_to_period.json", "frequency_to_period"),
        ("speed_to_units.json", "speed_to_units"),
        ("delay_word.json", "delay_word"),
        ("nearest_pulse_width.json", "nearest_pulse_width"),
        ("distance_word.json", "distance_word"),
    ];
    let total: usize = files.iter().map(|(file, function)| rows_in(&table(file, function))).sum();
    assert_eq!(total, TOTAL_ROWS);
}

#[test]
fn t0_mm_to_galvo() {
    let t = table("mm_to_galvo.json", "mm_to_galvo");
    assert!(t.rows.is_empty() && t.errors.is_empty() && t.specials.is_empty());
    assert_eq!(t.groups.len(), 2);
    let (mut sent, mut refused) = (0usize, 0usize);
    for (gi, g) in t.groups.iter().enumerate() {
        let ctx = format!("mm_to_galvo group {gi}");
        check_header(&g.params, &g.rows, |r| vec![num(&r[0], &ctx), num(&r[1], &ctx)], &ctx);
        let f = num(param(&g.params, "field_size_mm", &ctx), &ctx);
        for row in &g.rows {
            let r = cells(row, 4, &ctx);
            for axis in 0..2 {
                let v = num(&r[axis], &ctx);
                let want = word(&r[2 + axis], &ctx);
                assert_eq!(mm_to_galvo_om(v, f), want, "{ctx} om {row:?}");
                // OM's value before its clamp, in its operation order.
                let raw = (32768.0 + v * (65535.0 / f)).round_ties_even();
                if (0.0..=65535.0).contains(&raw) {
                    assert_eq!(raw, f64::from(want), "{ctx}: the oracle did not clamp {row:?}");
                    assert_eq!(mm_to_galvo(v, f), Ok(want), "{ctx} shipping {row:?}");
                    sent += 1;
                } else {
                    assert_eq!(want, if raw < 0.0 { 0 } else { 65535 }, "{ctx}: the oracle clamps {row:?}");
                    assert!(mm_to_galvo(v, f).is_err(), "{ctx}: shipping must refuse what OM clamps {row:?}");
                    refused += 1;
                }
            }
        }
    }
    assert!(sent > 0 && refused > 0, "{sent} sent, {refused} refused");
}

#[test]
fn t0_power_to_mark_current() {
    let t = table("power_to_mark_current.json", "power_to_mark_current");
    let ctx = "power_to_mark_current";
    assert!(t.groups.is_empty());
    check_header(&t.params, &t.rows, |r| vec![num(&r[0], ctx)], ctx);
    for row in &t.rows {
        let r = cells(row, 2, ctx);
        assert_eq!(power_to_mark_current(num(&r[0], ctx)), Ok(word(&r[1], ctx)), "{ctx} {row:?}");
    }
    assert!(!t.errors.is_empty());
    for row in &t.errors {
        let r = cells(row, 2, ctx);
        assert!(power_to_mark_current(num(&r[0], ctx)).is_err(), "{ctx} error row {row:?}");
    }
}

#[test]
fn t0_frequency_to_period() {
    let t = table("frequency_to_period.json", "frequency_to_period");
    let ctx = "frequency_to_period";
    assert!(t.groups.is_empty());
    check_header(&t.params, &t.rows, |r| vec![num(&r[0], ctx)], ctx);
    let (mut sent, mut refused) = (0usize, 0usize);
    for row in &t.rows {
        let r = cells(row, 2, ctx);
        let f = num(&r[0], ctx);
        let want = word(&r[1], ctx);
        assert_eq!(frequency_to_period_om(f), Ok(want), "{ctx} om {row:?}");
        // GP's value before `& 0xFFFF`.
        let raw = (20000.0 / f).round_ties_even();
        if f > 0.0 && (1.0..=65535.0).contains(&raw) {
            assert_eq!(raw, f64::from(want), "{ctx}: the oracle did not wrap {row:?}");
            assert_eq!(frequency_to_period(f), Ok(want), "{ctx} shipping {row:?}");
            sent += 1;
        } else {
            assert_eq!(raw.rem_euclid(65536.0), f64::from(want), "{ctx}: the oracle wraps {row:?}");
            assert!(frequency_to_period(f).is_err(), "{ctx}: shipping must refuse what GP wraps {row:?}");
            refused += 1;
        }
    }
    assert!(sent > 0 && refused > 0, "{sent} sent, {refused} refused");
    assert!(!t.errors.is_empty());
    for row in &t.errors {
        let f = num(&cells(row, 2, ctx)[0], ctx);
        assert!(frequency_to_period(f).is_err() && frequency_to_period_om(f).is_err(), "{ctx} error row {row:?}");
    }
}

#[test]
fn t0_speed_to_units() {
    let t = table("speed_to_units.json", "speed_to_units");
    assert!(t.rows.is_empty());
    assert_eq!(t.groups.len(), 3);
    for (gi, g) in t.groups.iter().enumerate() {
        let ctx = format!("speed_to_units group {gi}");
        // The sweep group carries the field per row (its own `columns`); the others one field for the group.
        let fixed = g.params.get("field_size_mm").and_then(Value::as_f64);
        let width = if fixed.is_some() { 2 } else { 3 };
        if fixed.is_none() {
            assert_eq!(param(&g.params, "columns", &ctx).as_array().map(Vec::len), Some(3), "{ctx}");
        }
        check_header(&g.params, &g.rows, |r| r.iter().take(width - 1).map(|v| num(v, &ctx)).collect(), &ctx);
        for row in &g.rows {
            let r = cells(row, width, &ctx);
            let f = fixed.unwrap_or_else(|| num(&r[1], &ctx));
            assert_eq!(speed_to_units(num(&r[0], &ctx), f), Ok(word(&r[width - 1], &ctx)), "{ctx} {row:?}");
        }
    }
    assert!(!t.errors.is_empty());
    for row in &t.errors {
        let r = cells(row, 3, "speed_to_units errors");
        assert!(speed_to_units(num(&r[0], "speed"), num(&r[1], "field")).is_err(), "error row {row:?}");
    }
}

#[test]
fn t0_delay_word() {
    let t = table("delay_word.json", "delay_word");
    let ctx = "delay_word";
    assert!(t.groups.is_empty());
    check_header(&t.params, &t.rows, |r| vec![num(&r[0], ctx)], ctx);
    for row in &t.rows {
        let r = cells(row, 3, ctx);
        assert_eq!(delay_word(num(&r[0], ctx)), Ok((word(&r[1], ctx), word(&r[2], ctx))), "{ctx} {row:?}");
    }
    assert!(!t.errors.is_empty());
    for row in &t.errors {
        assert!(delay_word(num(&cells(row, 2, ctx)[0], ctx)).is_err(), "{ctx} error row {row:?}");
    }
}

#[test]
fn t0_nearest_pulse_width() {
    let t = table("nearest_pulse_width.json", "nearest_pulse_width");
    let ctx = "nearest_pulse_width";
    assert!(t.groups.is_empty() && t.errors.is_empty());
    let table_ns: Vec<u16> = param(&t.params, "table_ns", ctx).as_array().unwrap().iter().map(|v| word(v, ctx)).collect();
    assert_eq!(table_ns, JPT_M7_PULSES_NS);
    check_header(&t.params, &t.rows, |r| vec![num(&r[0], ctx)], ctx);
    let mut seen = [0usize; 3];
    for row in &t.rows {
        let r = cells(row, 3, ctx);
        let v = num(&r[0], ctx);
        let nearest = word(&r[1], ctx);
        let want = match text(&r[2], ctx) {
            "exact" => PulseDecision::Exact(nearest),
            "snap" => PulseDecision::Snapped { requested: v, to: nearest },
            "refuse" => PulseDecision::Refused { requested: v, nearest },
            other => panic!("{ctx}: unknown decision {other}"),
        };
        seen[match want {
            PulseDecision::Exact(_) => 0,
            PulseDecision::Snapped { .. } => 1,
            PulseDecision::Refused { .. } => 2,
        }] += 1;
        assert_eq!(nearest_pulse_width(v, &JPT_M7_PULSES_NS), want, "{ctx} shipping {row:?}");
        assert_eq!(nearest_pulse_width_om(v, &JPT_M7_PULSES_NS), want, "{ctx} om {row:?}");
    }
    assert!(seen.iter().all(|n| *n > 0), "{seen:?}");
    // Non-finite inputs: OM through the `_om` variant (it snaps NaN to 2 ns); shipping refuses all of them.
    assert_eq!(t.specials.len(), 3);
    for row in &t.specials {
        let r = cells(row, 3, ctx);
        let v = special(&r[0], ctx);
        let nearest = word(&r[1], ctx);
        let om = nearest_pulse_width_om(v, &JPT_M7_PULSES_NS);
        match text(&r[2], ctx) {
            "snap" => assert!(matches!(om, PulseDecision::Snapped { to, .. } if to == nearest), "{ctx} om special {row:?}: {om:?}"),
            "refuse" => assert!(matches!(om, PulseDecision::Refused { nearest: n, .. } if n == nearest), "{ctx} om special {row:?}: {om:?}"),
            other => panic!("{ctx}: unexpected special decision {other}"),
        }
        let shipping = nearest_pulse_width(v, &JPT_M7_PULSES_NS);
        assert!(matches!(shipping, PulseDecision::Refused { nearest: n, .. } if n == nearest), "{ctx} shipping special {row:?}: {shipping:?}");
    }
}

#[test]
fn t0_distance_word() {
    let t = table("distance_word.json", "distance_word");
    let ctx = "distance_word";
    assert!(t.groups.is_empty() && t.errors.is_empty());
    // Integer inputs: no float hash, but the row count must hold.
    assert_eq!(param(&t.params, "n_rows", ctx).as_u64(), u64::try_from(t.rows.len()).ok());
    for row in &t.rows {
        let r = cells(row, 5, ctx);
        let w: Vec<u16> = r.iter().map(|v| word(v, ctx)).collect();
        assert_eq!(distance_word((w[0], w[1]), (w[2], w[3])), w[4], "{ctx} {row:?}");
    }
}
