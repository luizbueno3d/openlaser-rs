use openlaser_testkit::t0::{Group, Table, load};
use openlaser_types::galvo::{
    PulseDecision, delay_word, distance_word, frequency_to_period_om, mm_to_galvo_om, nearest_pulse_width_om, power_to_mark_current, speed_to_units,
};
use serde_json::Value;

fn n(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| v.as_i64().map_or(0.0, |i| i as f64))
}
fn w(v: &Value) -> u16 {
    v.as_u64().and_then(|n| u16::try_from(n).ok()).unwrap_or(0)
}
fn rows(t: &Table) -> Vec<(Option<&Group>, &Vec<Value>)> {
    t.groups.iter().flat_map(|g| g.rows.iter().map(move |r| (Some(g), r))).chain(t.rows.iter().map(|r| (None, r))).collect()
}

#[test]
fn t0_numeric_all_reference_rows() {
    let files = [
        "mm_to_galvo.json",
        "power_to_mark_current.json",
        "frequency_to_period.json",
        "speed_to_units.json",
        "delay_word.json",
        "nearest_pulse_width.json",
        "distance_word.json",
    ];
    for file in files {
        let t = load(file).unwrap_or_else(|e| panic!("{file}: {e}"));
        for (group, row) in rows(&t) {
            match t.function.as_str() {
                "mm_to_galvo" => {
                    let f = group.and_then(|g| g.params.get("field_size_mm")).map(n).unwrap_or(0.0);
                    assert_eq!(mm_to_galvo_om(n(&row[0]), f), w(&row[2]), "{file} row {row:?}");
                    assert_eq!(mm_to_galvo_om(n(&row[1]), f), w(&row[3]), "{file} row {row:?}");
                }
                "power_to_mark_current" => assert_eq!(power_to_mark_current(n(&row[0])).ok(), Some(w(&row[1])), "{file} row {row:?}"),
                "frequency_to_period" => assert_eq!(frequency_to_period_om(n(&row[0])).ok(), Some(w(&row[1])), "{file} row {row:?}"),
                "speed_to_units" => {
                    let value = group.and_then(|g| g.params.get("field_size_mm"));
                    let (f, expected) =
                        if value.and_then(Value::as_f64).is_some() { (value.map(n).unwrap_or(0.0), w(&row[1])) } else { (n(&row[1]), w(&row[2])) };
                    assert_eq!(speed_to_units(n(&row[0]), f).ok(), Some(expected), "{file} row {row:?}");
                }
                "delay_word" => {
                    let got = delay_word(n(&row[0])).unwrap_or((u16::MAX, u16::MAX));
                    assert_eq!((got.0, got.1), (w(&row[1]), w(&row[2])), "{file} row {row:?}");
                }
                "distance_word" => assert_eq!(distance_word((w(&row[0]), w(&row[1])), (w(&row[2]), w(&row[3]))), w(&row[4]), "{file} row {row:?}"),
                "nearest_pulse_width" => {
                    let requested = number_or_special(&row[0]);
                    let expected = w(&row[1]);
                    let got = nearest_pulse_width_om(requested, &openlaser_types::galvo::JPT_M7_PULSES_NS);
                    let want = match row[2].as_str().unwrap_or("") {
                        "exact" => PulseDecision::Exact(expected),
                        "snap" => PulseDecision::Snapped { requested, to: expected },
                        _ => PulseDecision::Refused { requested, nearest: expected },
                    };
                    assert_eq!(got, want, "{file} row {row:?}");
                }
                other => panic!("unhandled T0 function {other}"),
            }
        }
        for row in &t.errors {
            let rejected = match t.function.as_str() {
                "delay_word" => delay_word(n(&row[0])).is_err(),
                "power_to_mark_current" => power_to_mark_current(n(&row[0])).is_err(),
                "frequency_to_period" => frequency_to_period_om(n(&row[0])).is_err(),
                "speed_to_units" => speed_to_units(n(&row[0]), n(&row[1])).is_err(),
                _ => false,
            };
            assert!(rejected, "{file} must reject error row {row:?}");
        }
        if t.function == "nearest_pulse_width" {
            for row in &t.specials {
                let request = number_or_special(&row[0]);
                let got = nearest_pulse_width_om(request, &openlaser_types::galvo::JPT_M7_PULSES_NS);
                let nearest = w(&row[1]);
                let decision = row[2].as_str().unwrap_or("");
                assert!(
                    matches!((decision,got),("snap",PulseDecision::Snapped{to,..}) if to==nearest)
                        || matches!((decision,got),("refuse",PulseDecision::Refused{nearest:n,..}) if n==nearest),
                    "{file} special row {row:?}"
                );
            }
        }
    }
}

fn number_or_special(v: &Value) -> f64 {
    match v.as_str() {
        Some("inf") => f64::INFINITY,
        Some("-inf") => f64::NEG_INFINITY,
        Some("nan") => f64::NAN,
        _ => n(v),
    }
}
