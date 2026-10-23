//! T1 (v1), SPEC §6.2: the shipping lists equal OM v1's list commands except the §5.2 rule-6 lines and the lighting
//! lists' first `JumpSpeed`. The comparator works on decoded commands and checks the packet framing of both sides;
//! every other command, including every `dist` word, must be equal.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::{SCENARIOS, check_list_framing, field, fixture, is_frame, machine, oracle_packets, packetize, shipping, shipping_frame, words};
use openlaser_lmc::{Packet, op};
use openlaser_testkit::t1::Scenario;
use openlaser_types::galvo::{delay_word, speed_to_units};

/// v1's commands with only the listed divergences applied: D1 (SPEC §5.2 rule 6) turns the first jump's short delay
/// into the long one and adds the settle after the jump, after which the short delay is set again before the next
/// jump; D3 (§5.6) puts the lighting list's `JumpSpeed(light)` before its first (dark) jump.
fn expected_shipping(oracle: &[Packet], frame: bool) -> Result<Vec<[u16; 6]>, String> {
    let m = machine();
    let (long, _) = delay_word(m.first_jump.delay_us).map_err(|e| e.to_string())?;
    let (short, _) = delay_word(m.timings.jump_delay_short_us).map_err(|e| e.to_string())?;
    let light = speed_to_units(m.light_speed.get(), field()).map_err(|e| e.to_string())?;
    let mut expected = words(oracle);
    let jump = expected.iter().position(|w| w[0] == op::LIST_JUMP_TO).ok_or("v1 has no jump")?;
    let before = jump.checked_sub(1).ok_or("v1's first jump has no delay before it")?;
    if expected[before] != [op::LIST_JUMP_DELAY, short, 0, 0, 0, 0] {
        return Err(format!("v1's first jump is not preceded by JumpDelay({short}): {:?}", expected[before]));
    }
    expected[before][1] = long;
    expected.insert(jump + 1, [op::LIST_DELAY_TIME, m.first_jump.settle_10us, 0, 0, 0, 0]);
    if frame {
        expected.insert(before, [op::LIST_JUMP_SPEED, light, 0, 0, 0, 0]);
    }
    let first = expected.iter().position(|w| w[0] == op::LIST_JUMP_TO).ok_or("no jump")?;
    if let Some(next) = expected.iter().skip(first + 1).position(|w| w[0] == op::LIST_JUMP_TO) {
        expected.insert(first + 1 + next, [op::LIST_JUMP_DELAY, short, 0, 0, 0, 0]);
    }
    Ok(expected)
}

/// v1 + divergences for a scenario, after checking v1's own framing.
fn expected(s: &Scenario) -> Vec<[u16; 6]> {
    let oracle = oracle_packets(s);
    check_list_framing(&oracle).unwrap_or_else(|e| panic!("{} v1 framing: {e}", s.scenario));
    expected_shipping(&oracle, is_frame(s)).unwrap_or_else(|e| panic!("{}: {e}", s.scenario))
}

/// `Ok` only if `actual` is framed correctly and every command equals `expected` (v1 + divergences).
fn compare(actual: &[Packet], expected: &[[u16; 6]]) -> Result<(), String> {
    check_list_framing(actual).map_err(|e| format!("shipping framing: {e}"))?;
    let actual = words(actual);
    if let Some(i) = actual.iter().zip(expected).position(|(a, b)| a != b) {
        return Err(format!("command {i}: shipping {:?}, v1 + divergences {:?}", actual[i], expected[i]));
    }
    if actual.len() != expected.len() {
        return Err(format!("{} commands, v1 + divergences has {}", actual.len(), expected.len()));
    }
    Ok(())
}

#[test]
fn t1_shipping_vs_v1() {
    for name in SCENARIOS {
        let s = fixture("v1", name);
        compare(&shipping(&s), &expected(&s)).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

#[test]
fn t1_dot_test_has_explicit_jump_and_settle() {
    let s = fixture("v1", "dot_test");
    let actual = words(&shipping(&s));
    assert_eq!(actual[11], [op::LIST_JUMP_DELAY, 200, 0, 0, 0, 0]);
    assert_eq!(actual[12], [op::LIST_JUMP_TO, 32768, 32768, 0, 0, 0]);
    assert_eq!(actual[13], [op::LIST_DELAY_TIME, 100, 0, 0, 0, 0]);
    assert_eq!(actual[14], [op::LIST_MARK_TO, 32782, 32768, 0, 14, 0]);
}

/// The comparator is not a tautology: any changed word of a shipping list (dist words included), its padding or its
/// packet boundaries is rejected, and so is v1's own stream (the divergences are required, not merely allowed).
#[test]
fn comparator_rejects_unlisted_word_changes() {
    for name in ["mark_box10", "dot_test", "frame_box10"] {
        let s = fixture("v1", name);
        let packets = shipping(&s);
        let want = expected(&s);
        compare(&packets, &want).unwrap_or_else(|e| panic!("{name}: {e}"));
        for (pi, p) in packets.iter().enumerate() {
            for slot in 0..256 {
                for word in 0..6 {
                    let mut bytes = *p.bytes();
                    bytes[slot * 12 + word * 2] ^= 1;
                    let mut changed = packets.clone();
                    changed[pi] = Packet::try_from_bytes(&bytes).unwrap();
                    assert!(compare(&changed, &want).is_err(), "{name}: packet {pi} slot {slot} word {word} changed and was accepted");
                }
            }
        }
        // The same commands framed differently: a short first packet with an early EndOfList.
        let commands = words(&packets);
        let mut split = vec![Packet::from_words(&commands[..5]).unwrap()];
        split.extend(packetize(commands[5..].to_vec()));
        assert!(compare(&split, &want).is_err(), "{name}: a short non-final packet was accepted");
        // A missing EndOfList (the list fills its last packet exactly) is rejected too.
        let mut full = commands.clone();
        full.resize(256 * commands.len().div_ceil(256), [op::LIST_DELAY_TIME, 0, 0, 0, 0, 0]);
        let unterminated: Vec<Packet> = full.chunks(256).map(|c| Packet::from_words(c).unwrap()).collect();
        assert!(compare(&unterminated, &want).is_err(), "{name}: a list without EndOfList was accepted");
        // v1's own stream lacks the divergences.
        assert!(compare(&oracle_packets(&s), &want).is_err(), "{name}: v1 itself was accepted as shipping");
    }
}

/// Frame Loop's `pass` is one complete list holding exactly the first pass of Frame Once.
#[test]
fn frame_loop_pass_is_one_pass_of_frame_once() {
    for name in ["frame_box10", "bc_frame"] {
        let s = fixture("v1", name);
        let job = shipping_frame(&s);
        check_list_framing(&job.pass).unwrap_or_else(|e| panic!("{name} pass: {e}"));
        check_list_framing(&job.once).unwrap_or_else(|e| panic!("{name} once: {e}"));
        let pass = words(&job.pass);
        let once = words(&job.once);
        assert!(job.once_passes > 1, "{name}");
        assert!(once.starts_with(&pass) && once.len() > pass.len(), "{name}: pass is not the first pass of once");
    }
}
