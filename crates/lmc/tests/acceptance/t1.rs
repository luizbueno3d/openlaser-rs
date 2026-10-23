#![cfg(test)]
#![allow(clippy::panic, clippy::indexing_slicing)] // Test fixture assertions and fail-fast fixture helpers.

use super::*;
use openlaser_lmc::py_compat::{self, OmParams, WireEvent};
use openlaser_testkit::t1::{self, Event, Scenario};

const SCENARIOS: [&str; 8] =
    ["mark_box10", "dot_test", "line_test", "long_job", "bc_mark_vector-engrave_pass1", "bc_mark_raster_pass1", "frame_box10", "bc_frame"];
fn fixture(oracle: &str, name: &str) -> Scenario {
    t1::load(oracle, name).unwrap_or_else(|e| panic!("{e}"))
}
fn paths(s: &Scenario) -> Vec<Vec<[f64; 2]>> {
    t1::polylines(s).unwrap_or_else(|e| panic!("{e}"))
}
fn params(s: &Scenario) -> OmParams {
    let p = s.inputs.get("layer").unwrap_or(&s.inputs);
    let number = |key: &str| p.get(key).and_then(serde_json::Value::as_f64).unwrap_or_else(|| panic!("missing {key}"));
    OmParams {
        power: number(if p.get("power").is_some() { "power" } else { "power_percent" }),
        frequency_khz: number("frequency_khz"),
        pulse_width_ns: number("pulse_width_ns"),
        speed_mm_s: number("speed_mm_s"),
    }
}
fn assert_events(actual: &[WireEvent], expected: &[Event], name: &str) {
    assert_eq!(actual.len(), expected.len(), "{name} event count");
    for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
        let event = match a {
            WireEvent::Open => Event::Open,
            WireEvent::Close => Event::Close,
            WireEvent::Single { op, hex } => Event::Single { op: op.clone(), hex: hex.clone() },
            WireEvent::Read { status } => Event::Read { status: *status },
            WireEvent::List { sha256, n_cmds, bytes } => {
                assert_eq!(bytes.bytes().as_slice(), t1::packet(sha256).unwrap_or_else(|e| panic!("{e}")), "{name} packet {i}");
                Event::List { sha256: sha256.clone(), n_cmds: *n_cmds }
            }
        };
        assert_eq!(&event, b, "{name} event {i}");
    }
}
#[test]
fn t1_py_compat() {
    for name in SCENARIOS {
        let s = fixture("v0", name);
        let result = if name.contains("frame") {
            py_compat::frame_polylines(&paths(&s), s.field_size_mm)
        } else {
            py_compat::mark_polylines(&paths(&s), &params(&s), s.field_size_mm)
        };
        assert_events(&result.unwrap_or_else(|e| panic!("{name}: {e}")), &s.events, name);
    }
    let all = py_compat::init_abort();
    assert_events(&all, &fixture("v0", "init_abort").events, "init_abort");
    assert_events(&all[..39], &fixture("v0", "init").events, "init");
    assert_events(&all[39..], &fixture("v0", "abort").events, "abort");
}

fn shipping(s: &Scenario) -> (openlaser_machine::GalvoMachine, Vec<Packet>) {
    let (m, _, mut plan) = setup();
    let polys = paths(s);
    if s.scenario.contains("frame") {
        let frame = openlaser_safety::testing::frame_plan_unchecked(
            polys.into_iter().map(Polyline).collect(),
            plan.fingerprint,
            plan.machine,
            FrameStyle::Contour,
            FrameSource::Job,
        );
        let job = encode_frame(&frame, &m).unwrap_or_else(|e| panic!("{e}"));
        return (m, job.once);
    }
    let p = params(s);
    let section = &mut plan.sections[0];
    section.paths = polys.into_iter().map(Polyline).collect();
    if let CutQ::Galvo(q) = &mut section.cut {
        q.mark_current = openlaser_types::galvo::power_to_mark_current(p.power).unwrap_or_else(|e| panic!("{e}"));
        q.q_period = openlaser_types::galvo::frequency_to_period(p.frequency_khz).unwrap_or_else(|e| panic!("{e}"));
        q.speed_units = openlaser_types::galvo::speed_to_units(p.speed_mm_s, s.field_size_mm).unwrap_or_else(|e| panic!("{e}"));
        q.pulse = PulseDecision::Exact(200);
    }
    let emit = openlaser_safety::testing::emit_plan_unchecked(plan.sections, plan.fingerprint, plan.machine, DeviceKind::GalvoLmc, 1);
    let job = encode_job(&emit, &m).unwrap_or_else(|e| panic!("{e}"));
    (m, job.lists.into_iter().flat_map(|l| l.packets).collect())
}
fn words(packets: &[Packet]) -> Vec<[u16; 6]> {
    packets.iter().flat_map(|p| p.words().into_iter().take(p.real_commands())).collect()
}
fn oracle_words(s: &Scenario) -> Vec<[u16; 6]> {
    let [start, end] = s.segments["job"];
    let packets: Vec<_> = s.events[start..end]
        .iter()
        .filter_map(|e| {
            if let Event::List { sha256, .. } = e {
                Some(Packet::try_from_bytes(&t1::packet(sha256).unwrap_or_else(|e| panic!("{e}"))).unwrap_or_else(|e| panic!("{e}")))
            } else {
                None
            }
        })
        .collect();
    words(&packets)
}
/// Exact command comparison after applying only D1/D3 to the *expected* v1 stream.
/// No filtering by opcode: every remaining word, including all distance words, must match.
fn expected_shipping(s: &Scenario) -> Vec<[u16; 6]> {
    let mut expected = oracle_words(s);
    let jump = expected.iter().position(|w| w[0] == op::LIST_JUMP_TO).unwrap_or_else(|| panic!("no jump"));
    assert_eq!(expected[jump - 1], [op::LIST_JUMP_DELAY, 8, 0, 0, 0, 0]);
    expected[jump - 1][1] = 200;
    expected.insert(jump + 1, [op::LIST_DELAY_TIME, 100, 0, 0, 0, 0]);
    if s.scenario.contains("frame") {
        expected.insert(jump - 1, [op::LIST_JUMP_SPEED, 830, 0, 0, 0, 0]);
    }
    // The cache changes from long to short on the next actual jump.
    let first = expected.iter().position(|w| w[0] == op::LIST_JUMP_TO).unwrap_or(0);
    let next = expected.iter().enumerate().skip(first + 1).find(|(_, w)| w[0] == op::LIST_JUMP_TO).map(|(i, _)| i);
    // Lighting's next light move also resets the jump-delay cache, after light-on/speed.
    if let Some(next) = next {
        expected.insert(next, [op::LIST_JUMP_DELAY, 8, 0, 0, 0, 0]);
    }
    expected
}
#[test]
fn t1_shipping_vs_v1() {
    for name in SCENARIOS {
        let s = fixture("v1", name);
        let (_, packets) = shipping(&s);
        let actual = words(&packets);
        let expected = expected_shipping(&s);
        assert_eq!(actual.len(), expected.len(), "{name} commands");
        for (i, (a, b)) in actual.iter().zip(&expected).enumerate() {
            assert_eq!(a, b, "{name} command {i}");
        }
    }
}
#[test]
fn t1_dot_test_has_explicit_jump_and_settle() {
    let s = fixture("v1", "dot_test");
    let (_, packets) = shipping(&s);
    let actual = words(&packets);
    assert_eq!(actual[11], [op::LIST_JUMP_DELAY, 200, 0, 0, 0, 0]);
    assert_eq!(actual[12], [op::LIST_JUMP_TO, 32768, 32768, 0, 0, 0]);
    assert_eq!(actual[13], [op::LIST_DELAY_TIME, 100, 0, 0, 0, 0]);
    assert_eq!(actual[14], [op::LIST_MARK_TO, 32782, 32768, 0, 14, 0]);
}

#[test]
fn rust_v1_candidate_lists() {
    use std::fmt::Write;
    let root = openlaser_testkit::golden_dir().join("rust-v1-candidate").join("lmc");
    let update = std::env::var("UPDATE_RUST_V1").is_ok_and(|v| v == "1");
    if update {
        std::fs::create_dir_all(&root).unwrap_or_else(|e| panic!("{e}"));
    }
    for name in SCENARIOS {
        let s = fixture("v1", name);
        let (m, packets) = shipping(&s);
        let mut outputs = vec![(name.to_owned(), packets)];
        if name.contains("frame") {
            let frame = openlaser_safety::testing::frame_plan_unchecked(
                paths(&s).into_iter().map(Polyline).collect(),
                Fingerprint([7; 32]),
                MachineId("jpt-m7-200-sim".into()),
                FrameStyle::Contour,
                FrameSource::Job,
            );
            let job = encode_frame(&frame, &m).unwrap_or_else(|e| panic!("{e}"));
            outputs.push((format!("{name}_loop_pass"), job.pass));
        }
        for (name, packets) in outputs {
            let mut text = format!(
                "# rust-v1 CANDIDATE (unsigned, SPEC S15 / D13)\n# {name}; millimetre inputs: t1_lmc/v1/{}.json\n# Includes all 256 slots per packet, including explicit EOL and padding.\n",
                s.scenario
            );
            for (i, packet) in packets.iter().enumerate() {
                writeln!(text, "packet {i} sha256={} commands={}", packet.sha256_hex(), packet.real_commands()).unwrap_or_else(|e| panic!("{e}"));
                for slot in packet.bytes().as_chunks::<12>().0 {
                    for byte in slot {
                        write!(text, "{byte:02x}").unwrap_or_else(|e| panic!("{e}"));
                    }
                    text.push('\n');
                }
                text.push_str("listing (slot, opcode, five parameters)\n");
                for (slot, word) in packet.words().iter().take((packet.real_commands() + 1).min(256)).enumerate() {
                    writeln!(text, "{slot:03}: {} {:?}", op::list_name(word[0]), &word[1..]).unwrap_or_else(|e| panic!("{e}"));
                }
            }
            let path = root.join(format!("{name}.txt"));
            if update {
                std::fs::write(&path, &text).unwrap_or_else(|e| panic!("{e}"));
            }
            assert_eq!(
                std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}; regenerate with UPDATE_RUST_V1=1", path.display())),
                text,
                "candidate changed: {}",
                path.display()
            );
        }
    }
}

#[test]
fn comparator_rejects_unlisted_word_changes() {
    let s = fixture("v1", "mark_box10");
    let (_, packets) = shipping(&s);
    let actual = words(&packets);
    let expected = expected_shipping(&s);
    assert_eq!(actual, expected);
    for i in 0..actual.len() {
        for j in 0..6 {
            let mut changed = actual.clone();
            changed[i][j] ^= 1;
            assert_ne!(changed, expected, "word {i}:{j}");
        }
    }
}
