//! Adversarial command params (owned by WP6, SPEC §7.12; ENG §2.13): PhotoCraft's four (null, empty object, wrong
//! top-level types), plus NaN/inf as strings and numbers' nearest JSON forms, 1e308, negative and zero speeds, 0 Hz,
//! 1e9 kHz, unknown keys, wrong types, huge arrays, deep nesting, hostile paths and strings.
//!
//! Every value is valid JSON (JSON has no NaN): non-finite numbers appear as the strings serde would see from a
//! sloppy client. The engine must answer each with an error or a result, never a panic and never a hang.

use serde_json::{Value, json};

/// Nesting depth of the deep-array and deep-object cases (well past the engine's 64-level limit).
pub const DEEP: usize = 200;
/// Length of the huge-array case.
pub const HUGE: usize = 20_000;

fn deep_array(depth: usize) -> Value {
    let mut v = json!(0);
    for _ in 0..depth {
        v = Value::Array(vec![v]);
    }
    v
}

fn deep_object(depth: usize) -> Value {
    let mut v = json!({});
    for _ in 0..depth {
        v = json!({ "galvo": v });
    }
    v
}

/// The adversarial params, each a candidate `params` value for any command.
pub fn params() -> Vec<Value> {
    let nasty_numbers =
        [json!("NaN"), json!("inf"), json!("-inf"), json!(1e308), json!(-1e308), json!(-1), json!(0), json!(u64::MAX), json!(i64::MIN), json!(1e-320)];
    let mut v = vec![
        // PhotoCraft's four.
        Value::Null,
        json!({}),
        json!([]),
        json!("text"),
        json!(42),
        json!(true),
        // Unknown and duplicate (snake/camel) keys, universal keys of the wrong type.
        json!({ "bogus": 1 }),
        json!({ "force": true }),
        json!({ "token": "0000" }),
        json!({ "arm": true, "confirm": true, "skipFrame": true, "override": true }),
        json!({ "coalesce": 7 }),
        json!({ "dryRun": "yes" }),
        json!({ "dryRun": true, "dry_run": true }),
        json!({ "speedMmS": 1, "speed_mm_s": 2 }),
        // Ids, layers, indices.
        json!({ "ids": "all" }),
        json!({ "ids": [1e308] }),
        json!({ "ids": [-1, 0, u64::MAX] }),
        json!({ "ids": (0..HUGE as u64).collect::<Vec<_>>() }),
        json!({ "layer": 31 }),
        json!({ "layer": -1 }),
        json!({ "layer": "C99" }),
        json!({ "layer": "T3" }),
        json!({ "layer": "" }),
        json!({ "layer": 1e308 }),
        json!({ "index": u64::MAX }),
        json!({ "toIndex": u64::MAX, "layer": 0 }),
        json!({ "count": 0 }),
        json!({ "count": 100000 }),
        json!({ "subLayer": 255, "layer": 0 }),
        // Cut settings at and past every edge.
        json!({ "layer": 0, "galvo": { "speedMmS": 0 } }),
        json!({ "layer": 0, "galvo": { "speedMmS": -5 } }),
        json!({ "layer": 0, "galvo": { "speedMmS": "NaN" } }),
        json!({ "layer": 0, "galvo": { "powerPct": 150 } }),
        json!({ "layer": 0, "galvo": { "frequencyHz": 0 } }),
        json!({ "layer": 0, "galvo": { "frequencyKhz": 1e9 } }),
        json!({ "layer": 0, "galvo": { "frequencyKhz": 30.0001 } }),
        json!({ "layer": 0, "galvo": { "frequencyKhz": 30, "frequencyHz": 30000 } }),
        json!({ "layer": 0, "galvo": { "qPulseNs": "inf" } }),
        json!({ "layer": 0, "fill": { "intervalMm": 0 } }),
        json!({ "layer": 0, "fill": { "lpi": 0 } }),
        json!({ "layer": 0, "fill": { "intervalMm": 0.1, "lpi": 254 } }),
        json!({ "layer": "T1", "galvo": { "powerPct": 10 } }),
        json!({ "layer": 0, "mode": "laser" }),
        json!({ "layer": 0, "name": "a\u{0}b" }),
        json!({ "layer": 0, "name": "x".repeat(10_000) }),
        // Geometry.
        json!({ "xMm": "inf", "yMm": 0, "wMm": 10, "hMm": 10 }),
        json!({ "xMm": 0, "yMm": 0, "wMm": 0, "hMm": -1 }),
        json!({ "xMm": 0, "yMm": 0, "wMm": 1e308, "hMm": 1e308 }),
        json!({ "cxMm": 0, "cyMm": 0, "wMm": 10, "hMm": 10, "cornerRadiusMm": 99 }),
        json!({ "pointsMm": [[1, 2, 3]] }),
        json!({ "pointsMm": [[0, 0]] }),
        json!({ "pointsMm": [[0, 0], [1e308, -1e308]] }),
        json!({ "pointsMm": (0..HUGE).map(|i| [i as f64, 0.0]).collect::<Vec<_>>() }),
        json!({ "d": "M 0 0 L nan nan" }),
        json!({ "d": "M 0 0 C" }),
        json!({ "d": "Z Z Z" }),
        json!({ "d": "M0 0", "pointsMm": [[0, 0], [1, 1]] }),
        json!({ "dxMm": 1e308, "dyMm": 1e308 }),
        json!({ "deg": "NaN" }),
        json!({ "scalePct": 0 }),
        json!({ "scalePct": 1e300, "rotateDeg": 1e300 }),
        json!({ "rectMm": { "min": [10, 10], "max": [0, 0] } }),
        json!({ "toleranceMm": -1 }),
        json!({ "toleranceMm": 1e308 }),
        // Files and machines.
        json!({ "path": "" }),
        json!({ "path": "../../../../etc/passwd" }),
        json!({ "path": "/dev/zero" }),
        json!({ "path": "nul\u{0}byte.olp" }),
        json!({ "path": "C:\\Windows\\System32\\config\\SAM" }),
        json!({ "path": "x".repeat(70_000) }),
        json!({ "machine": "no-such-machine" }),
        json!({ "device": "../../machines/evil" }),
        json!({ "id": "UPPER CASE", "name": "x" }),
        json!({ "axis": 3, "requestedMm": 100, "measuredMm": 0 }),
        json!({ "fieldWidthMm": 1e9, "maxPowerPct": 1000 }),
        json!({ "checklist": [] }),
        json!({ "key": "machine.maxPowerPct", "value": 100 }),
        json!({ "key": "automation.allowAgents", "value": true }),
        json!({ "key": "shortcuts.laser.start", "value": "Cmd+Enter" }),
        json!({ "key": "output.curveToleranceMm", "value": "NaN" }),
        json!({ "note": "n".repeat(10_000) }),
        json!({ "submission": u64::MAX }),
        json!({ "mode": "forever" }),
        json!({ "style": "hull" }),
        json!({ "source": "agent" }),
        json!({ "loop": "yes" }),
        // Depth and size.
        json!({ "list": deep_array(DEEP) }),
        deep_object(DEEP),
        json!({ "ids": deep_array(DEEP) }),
        Value::Array((0..HUGE).map(|_| json!({})).collect()),
    ];
    for n in nasty_numbers {
        v.push(json!({ "layer": 0, "galvo": { "powerPct": n.clone(), "speedMmS": n.clone() } }));
        v.push(json!({ "xMm": n.clone(), "yMm": n.clone(), "wMm": n.clone(), "hMm": n.clone() }));
        v.push(json!({ "count": n.clone(), "index": n.clone(), "axis": n }));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_are_many_and_serialisable() {
        let p = params();
        assert!(p.len() > 80, "{}", p.len());
        for v in &p {
            assert!(serde_json::to_string(v).is_ok());
        }
    }
}
