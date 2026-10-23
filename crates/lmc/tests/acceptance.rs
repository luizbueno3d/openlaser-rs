//! Encoder refusals (SPEC §5.2), the strict emission summary (S22, §3.12), disassembly and the WP4 properties
//! (§7.5 acceptance: `prop_*`, `bytes_hash_changes_with_any_byte`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::time::Duration;

use common::{FP, PAD, box10, check_list_framing, encoded, field, frame, galvo_q, machine, machine_id, packetize, plans, section, section_with, setup, words};
use openlaser_geom::Polyline;
use openlaser_lmc::{
    Decoded, EmissionSummary, Error, GalvoMachine, JobPlan, LmcJob, Packet, Single, decode_packet, disassemble, disassemble_frame, emission_summary,
    emitted_segments, encode_frame, encode_job, op,
};
use openlaser_machine::FieldRect;
use openlaser_plan::{CutQ, GalvoCutQ};
use openlaser_types::galvo::{CENTER, JPT_M7_PULSES_NS, PulseDecision, distance_word, mm_to_galvo, units_to_speed};
use openlaser_types::{DeviceKind, Fingerprint, GalvoWords, LayerKey, MachineId, MmPerS};
use proptest::prelude::*;

fn encode(paths: Vec<Polyline>, m: &GalvoMachine) -> Result<LmcJob, Error> {
    let (emit, _) = plans(vec![section(paths)], 1);
    encode_job(&emit, m)
}
fn encode_q(q: GalvoCutQ, paths: Vec<Polyline>) -> Result<LmcJob, Error> {
    let (emit, _) = plans(vec![section_with(q, paths)], 1);
    encode_job(&emit, &machine())
}
fn decoded(job: &LmcJob) -> Vec<Decoded> {
    job.lists.iter().flat_map(|l| l.packets.iter().flat_map(decode_packet)).collect()
}
fn marks(job: &LmcJob) -> Vec<(u16, u16, u16)> {
    decoded(job)
        .into_iter()
        .filter_map(|d| match d {
            Decoded::MarkTo { x, y, dist } => Some((x, y, dist)),
            _ => None,
        })
        .collect()
}
fn not_encodable<T>(r: Result<T, Error>) -> bool {
    matches!(r, Err(Error::NotEncodable { .. }))
}
fn mismatch(r: Result<EmissionSummary, Error>) -> bool {
    matches!(r, Err(Error::SummaryMismatch))
}
/// The job with list 0's commands edited, re-framed exactly as the encoder frames them (so only the edit differs).
fn edit_list(job: &LmcJob, edit: impl FnOnce(&mut Vec<[u16; 6]>)) -> LmcJob {
    let mut job = job.clone();
    let mut commands = words(&job.lists[0].packets);
    edit(&mut commands);
    job.lists[0].packets = packetize(commands);
    job.lists[0].packet_est = vec![Duration::from_millis(1); job.lists[0].packets.len()];
    job
}
fn first(commands: &[[u16; 6]], opcode: u16) -> usize {
    commands.iter().position(|w| w[0] == opcode).unwrap()
}
fn quantize(p: [f64; 2]) -> (u16, u16) {
    (mm_to_galvo(p[0], field()).unwrap(), mm_to_galvo(p[1], field()).unwrap())
}
fn mm(w: u16) -> f64 {
    (f64::from(w) - 32768.0) * field() / 65535.0
}

// ---- B1: the rated work area (SPEC §5.2 rules 2 and 3) ------------------------------------------------------------

#[test]
fn points_outside_the_rated_work_area_are_refused() {
    let m = machine();
    let half = m.field_f()[0] / 2.0;
    let outside = [[110.0, 0.0], [100.0001, 0.0], [-100.0001, 0.0], [0.0, 100.0001], [0.0, -100.0001], [half, 0.0], [-half, 0.0], [0.0, half], [0.0, -half]];
    for p in outside.into_iter().chain([[f64::NAN, 0.0], [0.0, f64::INFINITY]]) {
        for path in [vec![p, [0.0, 0.0]], vec![[0.0, 0.0], p], vec![[1.0, 1.0], [0.0, 0.0], p]] {
            assert!(not_encodable(encode(vec![box10(), Polyline(path.clone())], &m)), "job {path:?}");
            assert!(not_encodable(encode_frame(&frame(vec![box10(), Polyline(path.clone())]), &m)), "frame {path:?}");
        }
    }
    for p in [[99.999, 0.0], [-99.999, 0.0], [0.0, 99.999], [100.0, 100.0], [-100.0, -100.0], [100.0, -100.0]] {
        let path = Polyline(vec![[0.0, 0.0], p]);
        assert!(encode(vec![path.clone()], &m).is_ok(), "job {p:?}");
        assert!(encode_frame(&frame(vec![path]), &m).is_ok(), "frame {p:?}");
    }
}

/// The encodable square |v| < F/2 holds even where a (malformed) work area is wider: -F/2 has a word (0) but is
/// refused, and so is everything outside the square.
#[test]
fn the_encodable_square_holds_inside_a_wider_work_area() {
    let mut m = machine();
    m.work_area = FieldRect { min: [-200.0, -200.0], max: [200.0, 200.0] };
    let f = m.field_f()[0];
    let half = f / 2.0;
    assert_eq!(mm_to_galvo(-half, f), Ok(0));
    // Just inside +F/2 the word would round to 65536: refused too, never clamped.
    for p in [[-half, 0.0], [half, 0.0], [0.0, -half], [0.0, half], [118.5, 0.0], [half.next_down(), 0.0]] {
        assert!(not_encodable(encode(vec![Polyline(vec![[0.0, 0.0], p])], &m)), "{p:?}");
    }
    for p in [[118.0, 0.0], [-118.0, 0.0], [118.3, 0.0], [-118.3, 0.0]] {
        assert!(encode(vec![Polyline(vec![[0.0, 0.0], p])], &m).is_ok(), "{p:?}");
    }
    // A NaN in the work area or the field refuses every point.
    let mut nan_area = machine();
    nan_area.work_area.min = [f64::NAN, -100.0];
    assert!(not_encodable(encode(vec![box10()], &nan_area)));
    let mut nan_field = machine();
    nan_field.cal.scale = [f64::NAN, 1.1834];
    assert!(not_encodable(encode(vec![box10()], &nan_field)));
}

#[test]
fn red_light_points_are_bounded_before_and_after_the_red_dot_correction() {
    let line = |x: f64| frame(vec![Polyline(vec![[x, 0.0], [0.0, 0.0]])]);
    let mut m = machine();
    m.red_dot.offset_mm = [2.0, 0.0];
    assert!(not_encodable(encode_frame(&line(99.0), &m)), "99 + 2 leaves the work area");
    m.red_dot.offset_mm = [-2.0, 0.0];
    assert!(not_encodable(encode_frame(&line(101.0), &m)), "the frame point itself is outside");
    assert!(encode_frame(&line(99.0), &m).is_ok());
    m.red_dot.offset_mm = [0.0, 0.0];
    m.red_dot.scale = [1.2, 1.0];
    assert!(not_encodable(encode_frame(&line(90.0), &m)), "90 × 1.2 leaves the work area");
    assert!(encode_frame(&line(80.0), &m).is_ok());
}

// ---- B2: zero speed units (SPEC §5.2 rule 4, B7) ------------------------------------------------------------------

#[test]
fn zero_mark_speed_units_are_refused() {
    let mut q = galvo_q();
    q.speed_units = 0;
    assert!(matches!(encode_q(q.clone(), vec![box10()]), Err(Error::ZeroSpeed)));
    q.speed_units = 1;
    assert!(encode_q(q, vec![box10()]).is_ok());
}

#[test]
fn zero_jump_speed_units_are_refused() {
    let mut q = galvo_q();
    q.timings.jump_speed_units = 0;
    assert!(matches!(encode_q(q.clone(), vec![box10()]), Err(Error::ZeroSpeed)));
    q.timings.jump_speed_units = 1;
    assert!(encode_q(q, vec![box10()]).is_ok());
}

/// 3.6 mm/s is 0 units at F = 236.68 (the first unit is 3.6115 mm/s).
#[test]
fn a_lighting_speed_below_one_unit_is_refused() {
    let mut m = machine();
    m.light_speed = MmPerS::new(3.6).unwrap();
    assert!(matches!(encode_frame(&frame(vec![box10()]), &m), Err(Error::ZeroSpeed)));
    m.light_speed = MmPerS::new(3.62).unwrap();
    assert!(encode_frame(&frame(vec![box10()]), &m).is_ok());
}

// ---- the encoder sends only words the board can take as they are -------------------------------------------------

#[test]
fn words_the_board_cannot_take_are_refused() {
    let mut q = galvo_q();
    q.mark_current = 4096;
    assert!(matches!(encode_q(q, vec![box10()]), Err(Error::Encode(_))));
    let mut q = galvo_q();
    q.q_period = 0;
    assert!(matches!(encode_q(q, vec![box10()]), Err(Error::Encode(_))));
    let mut q = galvo_q();
    q.pulse = PulseDecision::Refused { requested: 170.0, nearest: 150 };
    assert!(matches!(encode_q(q, vec![box10()]), Err(Error::InvalidPulse)));
    let mut q = galvo_q();
    q.pulse = PulseDecision::Exact(201);
    assert!(matches!(encode_q(q, vec![box10()]), Err(Error::InvalidPulse)));
    let mut q = galvo_q();
    q.pulse = PulseDecision::Snapped { requested: 199.8, to: 200 };
    assert!(encode_q(q, vec![box10()]).is_ok());
    let (emit, _) = plans(vec![section(vec![box10()])], 0);
    assert!(matches!(encode_job(&emit, &machine()), Err(Error::Empty)));
    let emit = openlaser_safety::testing::emit_plan_unchecked(vec![section(vec![box10()])], FP, machine_id(), DeviceKind::Grbl, 1);
    assert!(matches!(encode_job(&emit, &machine()), Err(Error::NotGalvo)));
}

// ---- N2, N4, N5 ---------------------------------------------------------------------------------------------------

#[test]
fn duplicate_points_are_elided_and_dist_follows_the_last_sent_point() {
    let job = encoded(vec![Polyline(vec![[-5.0, -5.0], [-5.0, -5.0], [5.0, -5.0], [5.0001, -5.0], [5.0, 5.0]]), Polyline(vec![[5.0, 5.0], [-5.0, 5.0]])]);
    assert_eq!(marks(&job), [(34152, 31384, 2768), (34152, 34152, 2768), (31384, 34152, 2768)]);
    // The second path starts where the first ended: GP sends no jump between them.
    assert_eq!(decoded(&job).iter().filter(|d| matches!(d, Decoded::JumpTo { .. })).count(), 1);
    // A section whose every mark is below one galvo unit is refused, never sent with MO open and nothing to mark.
    assert!(matches!(encode(vec![Polyline(vec![[0.0, 0.0], [0.001, 0.0]])], &machine()), Err(Error::Empty)));
}

#[test]
fn short_paths_are_refused_not_dropped() {
    let m = machine();
    for short in [Polyline(vec![]), Polyline(vec![[1.0, 1.0]])] {
        assert!(matches!(encode(vec![box10(), short.clone()], &m), Err(Error::ShortPath)));
        assert!(matches!(encode(vec![short.clone(), box10()], &m), Err(Error::ShortPath)));
        assert!(matches!(encode_frame(&frame(vec![box10(), short]), &m), Err(Error::ShortPath)));
    }
    assert!(matches!(encode(vec![], &m), Err(Error::Empty)));
    assert!(matches!(encode_frame(&frame(vec![]), &m), Err(Error::Empty)));
}

#[test]
fn lighting_refuses_a_red_bit_that_is_the_laser_bit() {
    let mut m = machine();
    m.ports.red_bit = m.ports.laser_bit;
    assert!(matches!(encode_frame(&frame(vec![box10()]), &m), Err(Error::UnsafeLighting)));
    m.ports.red_bit = 16;
    assert!(matches!(encode_frame(&frame(vec![box10()]), &m), Err(Error::InvalidPort(16))));
}

#[test]
fn box10_has_explicit_jump_and_settle() {
    let (m, emit, _) = setup();
    let job = encode_job(&emit, &m).unwrap();
    let decoded = decoded(&job);
    assert_eq!(decoded.get(11), Some(&Decoded::JumpDelay(200, 0)));
    assert_eq!(decoded.get(12), Some(&Decoded::JumpTo { x: 31384, y: 31384, dist: 1957 }));
    assert_eq!(decoded.get(13), Some(&Decoded::DelayTime(100)));
    assert_eq!(marks(&job).len(), 4);
}

#[test]
fn frame_nonfinite_geometry_returns_error_without_panicking() {
    let m = machine();
    for value in [f64::NAN, f64::INFINITY, f64::MAX] {
        assert!(encode_frame(&frame(vec![Polyline(vec![[value, 0.0], [0.0, 0.0]])]), &m).is_err());
    }
}

// ---- N10: disassembly ---------------------------------------------------------------------------------------------

#[test]
fn disassembly_shows_every_single_and_the_end_of_list() {
    let (m, emit, _) = setup();
    let job = encode_job(&emit, &m).unwrap();
    let listing = disassemble(&job, &m);
    let singles: Vec<&str> = listing.lines.iter().filter(|l| l.packet.is_none()).map(|l| l.text.as_str()).collect();
    assert_eq!(singles, ["ResetList", "WritePort(1)", "Fiber_SetMo(1)", "GetVersion", "Fiber_SetMo(0)", "WritePort(0)", "GetMarkTime(3)"]);
    let texts: Vec<&str> = listing.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts.first(), Some(&"ResetList"));
    assert_eq!(texts.last(), Some(&"GetMarkTime(3)"));
    let packet_lines: Vec<_> = listing.lines.iter().filter(|l| l.packet.is_some()).collect();
    assert_eq!(packet_lines.len(), decoded(&job).len() + 1);
    assert_eq!(packet_lines.last().map(|l| l.text.as_str()), Some("EndOfList"));
    assert_eq!(listing.lines[4].text, "ReadyMark", "the prelude precedes the list");
    assert!(texts.iter().any(|t| t.starts_with("MarkTo x=34152 y=31384 (4.998, -4.998 mm) dist=2768")), "{texts:?}");
    assert!(listing.lines.iter().all(|l| l.list == Some(0)));
    // A stray command hidden behind the EndOfList is listed, not skipped.
    let mut hidden = job.clone();
    let mut slots = hidden.lists[0].packets[0].words();
    let eol = slots.iter().position(|w| w[0] == op::LIST_END_OF_LIST).unwrap();
    slots[eol + 3] = [op::LIST_MARK_TO, 40000, 40000, 0, 0, 0];
    hidden.lists[0].packets[0] = Packet::from_words(&slots).unwrap();
    assert!(disassemble(&hidden, &m).lines.iter().any(|l| l.text.starts_with("MarkTo x=40000")));
    // Frames list both lists and never a single.
    let fj = encode_frame(&frame(vec![box10()]), &m).unwrap();
    let fl = disassemble_frame(&fj, &m);
    assert!(fl.lines.iter().all(|l| l.packet.is_some()));
    assert_eq!(fl.lines.iter().filter(|l| l.text == "EndOfList").count(), 2);
}

// ---- B4: the emission summary is decoded strictly from the bytes --------------------------------------------------

#[test]
fn summary_rows_and_words_come_from_one_decoded_block() {
    let (m, emit, plan) = setup();
    let job = encode_job(&emit, &m).unwrap();
    let s = emission_summary(&job, &m, &plan, &[]).unwrap();
    assert_eq!(s.bytes_hash, job.bytes_hash());
    let row = &s.rows[0];
    let g = row.galvo.unwrap();
    assert_eq!(g, GalvoWords { mark_current: 2048, q_period: 667, pulse_ns: 200, speed_units: 276, jump_speed_units: 830 });
    assert_eq!(row.power_pct, f64::from(g.mark_current) * 100.0 / 4095.0);
    assert_eq!(row.frequency_khz, Some(20000.0 / f64::from(g.q_period)));
    assert_eq!(row.pulse_ns, Some(g.pulse_ns));
    assert_eq!(row.speed_mm_s, units_to_speed(g.speed_units, field()));
    assert_eq!(row.paths, 1);
    assert!((row.mark_mm - 4.0 * mm(34152) * 2.0).abs() < 1e-9, "{}", row.mark_mm);
    assert_eq!(row.bounds.min, [mm(31384), mm(31384)]);
    assert_eq!(row.bounds.max, [mm(34152), mm(34152)]);
    assert_eq!((row.layer, row.pass, row.layer_name.as_str()), (LayerKey::Color(0), 1, "C00"));
}

/// The review's case: MarkCurrent 2048, then 4095, then 41 in one list. One row cannot describe it: refused.
#[test]
fn summary_refuses_a_list_with_more_than_one_value_for_a_parameter() {
    let (m, emit, plan) = setup();
    let job = encode_job(&emit, &m).unwrap();
    let bad = edit_list(&job, |c| {
        let mark = first(c, op::LIST_MARK_TO);
        c.insert(mark + 1, [op::LIST_MARK_CURRENT, 4095, 0, 0, 0, 0]);
        c.insert(mark + 3, [op::LIST_MARK_CURRENT, 41, 0, 0, 0, 0]);
    });
    assert!(mismatch(emission_summary(&bad, &m, &plan, &[])));
    // The lenient preview shows each mark at its own current: the 100 % segment is visible there.
    let powers: Vec<f64> = emitted_segments(&bad, &m).iter().map(|s| s.power_pct).collect();
    assert_eq!(powers, [2048.0 * 100.0 / 4095.0, 100.0, 41.0 * 100.0 / 4095.0, 41.0 * 100.0 / 4095.0]);
    let block = [
        [op::LIST_MARK_CURRENT, 1, 0, 0, 0, 0],
        [op::LIST_MARK_SPEED, 100, 0, 0, 0, 0],
        [op::LIST_Q_SWITCH_PERIOD, 444, 0, 0, 0, 0],
        [op::LIST_FIBER_YLPM_PULSE_WIDTH, 100, 0, 0, 0, 0],
        [op::LIST_JUMP_SPEED, 100, 0, 0, 0, 0],
        [op::LIST_LASER_ON_DELAY, 5, 0, 0, 0, 0],
        [op::LIST_LASER_OFF_DELAY, 5, 0, 0, 0, 0],
        [op::LIST_POLYGON_DELAY, 5, 0, 0, 0, 0],
    ];
    for extra in block {
        // A second, different value after the first mark.
        let after = edit_list(&job, |c| c.insert(first(c, op::LIST_MARK_TO) + 1, extra));
        assert!(mismatch(emission_summary(&after, &m, &plan, &[])), "{extra:?} after a mark");
        // A repeat of the same command inside the block, even with the same value.
        let same = edit_list(&job, |c| {
            let at = first(c, extra[0]);
            let copy = c[at];
            c.insert(at + 1, copy);
        });
        assert!(mismatch(emission_summary(&same, &m, &plan, &[])), "{extra:?} twice in the block");
    }
}

#[test]
fn summary_refuses_a_mark_before_its_parameter_block() {
    let (m, emit, plan) = setup();
    let job = encode_job(&emit, &m).unwrap();
    for opcode in [op::LIST_MARK_CURRENT, op::LIST_MARK_SPEED, op::LIST_Q_SWITCH_PERIOD, op::LIST_FIBER_YLPM_PULSE_WIDTH, op::LIST_LASER_ON_DELAY] {
        let late = edit_list(&job, |c| {
            let w = c.remove(first(c, opcode));
            let mark = first(c, op::LIST_MARK_TO);
            c.insert(mark + 1, w);
        });
        assert!(mismatch(emission_summary(&late, &m, &plan, &[])), "{opcode:#06x} after the first mark");
    }
    // The first motion must be the explicit first jump: the same move as a MarkTo would burn from an unknown point.
    let no_jump = edit_list(&job, |c| {
        let j = first(c, op::LIST_JUMP_TO);
        c[j][0] = op::LIST_MARK_TO;
    });
    assert!(mismatch(emission_summary(&no_jump, &m, &plan, &[])));
    // A list without any mark.
    let dark = edit_list(&job, |c| c.retain(|w| w[0] != op::LIST_MARK_TO));
    assert!(mismatch(emission_summary(&dark, &m, &plan, &[])));
}

#[test]
fn summary_refuses_bytes_the_board_would_read_differently() {
    let (m, emit, plan) = setup();
    let job = encode_job(&emit, &m).unwrap();
    let after_mark = |w: [u16; 6]| edit_list(&job, |c| c.insert(first(c, op::LIST_MARK_TO) + 1, w));
    for w in [
        [op::LIST_LASER_ON_POINT, 100, 0, 0, 0, 0],
        [op::LIST_FIBER_OPEN_MO, 1, 0, 0, 0, 0],
        [op::LIST_WRITE_PORT, 0x0100, 0, 0, 0, 0],
        [op::LIST_MARK_POWER_RATIO, 50, 0, 0, 0, 0],
        [op::LIST_DIRECT_LASER_SWITCH, 1, 0, 0, 0, 0],
        [op::LIST_READY_MARK, 1, 0, 0, 0, 0],
        [op::LIST_DELAY_TIME, 1, 1, 0, 0, 0],
    ] {
        assert!(mismatch(emission_summary(&after_mark(w), &m, &plan, &[])), "{w:?}");
    }
    type Edit = fn(&mut Vec<[u16; 6]>);
    let edits: [(&str, Edit); 4] = [
        ("laser and red bits", |c| {
            let p = first(c, op::LIST_WRITE_PORT);
            c[p][1] = 0x0101;
        }),
        ("an unused word set", |c| {
            let p = first(c, op::LIST_MARK_CURRENT);
            c[p][2] = 1;
        }),
        ("a wrong dist word", |c| {
            let p = first(c, op::LIST_MARK_TO);
            c[p][4] += 1;
        }),
        ("a nonzero z word", |c| {
            let p = first(c, op::LIST_MARK_TO);
            c[p][3] = 1;
        }),
    ];
    for (what, edit) in edits {
        assert!(mismatch(emission_summary(&edit_list(&job, edit), &m, &plan, &[])), "{what}");
    }
    // A MarkTo hidden behind the EndOfList of the last packet.
    let mut hidden = job.clone();
    let mut slots = hidden.lists[0].packets[0].words();
    let eol = slots.iter().position(|w| w[0] == op::LIST_END_OF_LIST).unwrap();
    slots[eol + 1] = [op::LIST_MARK_TO, 40000, 40000, 0, 0, 0];
    hidden.lists[0].packets[0] = Packet::from_words(&slots).unwrap();
    assert!(mismatch(emission_summary(&hidden, &m, &plan, &[])));
    // The same commands framed with a short first packet.
    let commands = words(&job.lists[0].packets);
    let mut split = job.clone();
    split.lists[0].packets = vec![Packet::from_words(&commands[..5]).unwrap()];
    split.lists[0].packets.extend(packetize(commands[5..].to_vec()));
    split.lists[0].packet_est = vec![Duration::from_millis(1); split.lists[0].packets.len()];
    assert!(mismatch(emission_summary(&split, &m, &plan, &[])));
    // A point outside the work area: encoded for a machine with a wider (unvalidated) area, summarised for this one.
    let mut wide = machine();
    wide.work_area = FieldRect { min: [-110.0, -110.0], max: [110.0, 110.0] };
    let (emit105, plan105) = plans(vec![section(vec![Polyline(vec![[0.0, 0.0], [105.0, 0.0]])])], 1);
    let far = encode_job(&emit105, &wide).unwrap();
    assert!(emission_summary(&far, &wide, &plan105, &[]).is_ok());
    assert!(mismatch(emission_summary(&far, &m, &plan105, &[])));
}

#[test]
fn summary_refuses_a_job_that_is_not_this_plans() {
    let (m, emit, plan) = setup();
    let job = encode_job(&emit, &m).unwrap();
    assert!(emission_summary(&job, &m, &plan, &[]).is_ok());
    let mut other_plans: Vec<(&str, JobPlan)> = Vec::new();
    let mut p = plan.clone();
    p.fingerprint = Fingerprint([9; 32]);
    other_plans.push(("fingerprint", p));
    let mut p = plan.clone();
    p.machine = MachineId("jpt-m7-200".into());
    other_plans.push(("machine", p));
    let mut p = plan.clone();
    p.device = DeviceKind::Grbl;
    other_plans.push(("device", p));
    let mut p = plan.clone();
    p.repeats = 2;
    other_plans.push(("repeats", p));
    let mut p = plan.clone();
    p.sections[0].layer = LayerKey::Color(1);
    other_plans.push(("layer", p));
    let mut p = plan.clone();
    p.sections[0].pass = 2;
    other_plans.push(("pass", p));
    let mut p = plan.clone();
    p.sections.push(p.sections[0].clone());
    other_plans.push(("section count", p));
    let edits: [fn(&mut GalvoCutQ); 5] = [
        |q| q.mark_current = 2049,
        |q| q.q_period = 666,
        |q| q.pulse = PulseDecision::Exact(150),
        |q| q.speed_units = 277,
        |q| q.timings.jump_speed_units = 831,
    ];
    for edit in edits {
        let mut p = plan.clone();
        if let CutQ::Galvo(q) = &mut p.sections[0].cut {
            edit(q);
        }
        other_plans.push(("planned words", p));
    }
    for (what, p) in other_plans {
        assert!(mismatch(emission_summary(&job, &m, &p, &[])), "{what}");
    }
    let mut other_jobs: Vec<(&str, LmcJob)> = Vec::new();
    let mut j = job.clone();
    j.prelude[1] = Single([op::WRITE_PORT, 0x0101, 0, 0, 0, 0]);
    other_jobs.push(("prelude", j));
    let mut j = job.clone();
    j.postlude.remove(0);
    other_jobs.push(("postlude", j));
    let mut j = job.clone();
    j.lists[0].packet_est.push(Duration::ZERO);
    other_jobs.push(("packet_est", j));
    let mut j = job.clone();
    j.repeats = 0;
    other_jobs.push(("zero repeats", j));
    for (what, j) in other_jobs {
        assert!(mismatch(emission_summary(&j, &m, &plan, &[])), "{what}");
    }
}

/// 1100 separate 0.5 mm marks (long_job): many packets.
fn long_job(estimate: Duration, repeats: u16) -> (LmcJob, JobPlan) {
    let paths = (0..1100u32)
        .map(|i| {
            let x = -50.0 + f64::from(i % 100);
            let y = -50.0 + f64::from(i / 100) * 5.0;
            Polyline(vec![[x, y], [x + 0.5, y]])
        })
        .collect();
    let mut s = section(paths);
    s.estimate = estimate;
    let (emit, plan) = plans(vec![s], repeats);
    (encode_job(&emit, &machine()).unwrap(), plan)
}

#[test]
fn crash_bound_is_the_largest_run_ahead_window_and_never_zero() {
    let m = machine();
    // A planner estimate of 0 (WP3's estimate_galvo is still a stub).
    let (job, plan) = long_job(Duration::ZERO, 2);
    let est = job.lists[0].packet_est.clone();
    assert!(est.len() > 3, "{} packets", est.len());
    assert!(est.iter().all(|e| !e.is_zero()), "every packet carries its own byte-derived time");
    let window = |n: usize| est.windows(n).map(|w| w.iter().sum::<Duration>()).max().unwrap();
    let s = emission_summary(&job, &m, &plan, &[]).unwrap();
    assert_eq!(s.crash_bound, window(3));
    assert!(s.crash_bound < est.iter().sum::<Duration>());
    assert_eq!(s.rows[0].estimate, est.iter().sum::<Duration>());
    assert_eq!(s.estimate, est.iter().sum::<Duration>() * 2, "all repeats");
    let mut m1 = m.clone();
    m1.run_ahead.max_packets = 1;
    assert_eq!(emission_summary(&job, &m1, &plan, &[]).unwrap().crash_bound, window(1));
    m1.run_ahead.max_packets = 0;
    assert_eq!(emission_summary(&job, &m1, &plan, &[]).unwrap().crash_bound, window(1), "never < 1 packet");
    // `packet_est` is not part of the bytes: zeroed estimates still give the bytes' own time.
    let mut tampered = job.clone();
    tampered.lists[0].packet_est = vec![Duration::ZERO; est.len()];
    assert_eq!(emission_summary(&tampered, &m, &plan, &[]).unwrap().crash_bound, window(3));
    // A planner estimate above the bytes' time wins.
    let (slow, slow_plan) = long_job(Duration::from_secs(1000), 1);
    let share = Duration::from_secs(1000) / u32::try_from(slow.lists[0].packets.len()).unwrap();
    assert!(slow.lists[0].packet_est.iter().all(|e| *e == share));
    assert_eq!(emission_summary(&slow, &m, &slow_plan, &[]).unwrap().crash_bound, share * 3);
    // The one-packet box: the bound is that packet, and it is not zero.
    let (m, emit, plan) = setup();
    let small = encode_job(&emit, &m).unwrap();
    let s = emission_summary(&small, &m, &plan, &[]).unwrap();
    assert_eq!(s.crash_bound, small.lists[0].packet_est[0]);
    assert!(!s.crash_bound.is_zero());
}

// ---- packets ------------------------------------------------------------------------------------------------------

#[test]
fn packets_hold_at_most_256_commands_padded_with_end_of_list() {
    for count in [0usize, 1, 17, 255, 256] {
        let words = vec![[op::LIST_DELAY_TIME, 1, 0, 0, 0, 0]; count];
        let p = Packet::from_words(&words).unwrap();
        assert_eq!(p.bytes().len(), 3072);
        assert_eq!(p.real_commands(), count);
        assert!(p.words().iter().skip(count).all(|w| *w == PAD));
        assert_eq!(Packet::try_from_bytes(p.bytes()).unwrap(), p);
    }
    assert!(Packet::from_words(&vec![[op::LIST_DELAY_TIME, 1, 0, 0, 0, 0]; 257]).is_err());
    assert!(Packet::try_from_bytes(&[0u8; 3071]).is_err());
}

// ---- bytes_hash ---------------------------------------------------------------------------------------------------

#[test]
fn bytes_hash_changes_with_any_byte() {
    let (m, emit, _) = setup();
    let job = encode_job(&emit, &m).unwrap();
    assert_eq!(job, encode_job(&emit, &m).unwrap());
    let hash = job.bytes_hash();
    for i in 0..3072 {
        let mut changed = job.clone();
        let mut bytes = *changed.lists[0].packets[0].bytes();
        bytes[i] ^= 1;
        changed.lists[0].packets[0] = Packet::try_from_bytes(&bytes).unwrap();
        assert_ne!(hash, changed.bytes_hash(), "packet byte {i}");
    }
    for prelude in [true, false] {
        let count = if prelude { job.prelude.len() } else { job.postlude.len() };
        for i in 0..count {
            for byte in 0..12 {
                let mut changed = job.clone();
                let singles = if prelude { &mut changed.prelude } else { &mut changed.postlude };
                singles[i].0[byte / 2] ^= 1u16 << ((byte % 2) * 8);
                assert_ne!(hash, changed.bytes_hash(), "single byte {i}:{byte}");
            }
        }
    }
    let mut changed = job.clone();
    changed.repeats += 1;
    assert_ne!(hash, changed.bytes_hash());
    // Documented as not hashed: labels and estimates (checked against the plan and the token instead).
    let mut labels = job.clone();
    labels.lists[0].pass = 9;
    labels.lists[0].packet_est = vec![Duration::from_secs(9)];
    labels.fingerprint = Fingerprint([1; 32]);
    assert_eq!(hash, labels.bytes_hash());
}

// ---- properties (SPEC §6.5) ---------------------------------------------------------------------------------------

fn arb_point() -> impl Strategy<Value = [f64; 2]> {
    (-99.0f64..99.0, -99.0f64..99.0).prop_map(|(x, y)| [x, y])
}
fn arb_path(max: usize) -> impl Strategy<Value = Vec<[f64; 2]>> {
    prop::collection::vec(arb_point(), 2..=max)
}
/// The MarkTo targets and dist words GP would send for `path`, after the start point.
fn expected_marks(path: &[[f64; 2]]) -> Vec<(u16, u16, u16)> {
    let mut last = quantize(path[0]);
    let mut out = Vec::new();
    for p in &path[1..] {
        let to = quantize(*p);
        if to != last {
            out.push((to.0, to.1, distance_word(last, to)));
            last = to;
        }
    }
    out
}

proptest! {
    #[test]
    fn prop_packets_3072_padded(path in arb_path(700)) {
        let m = machine();
        let job = encode(vec![Polyline(path.clone())], &m).unwrap();
        for list in &job.lists {
            prop_assert!(check_list_framing(&list.packets).is_ok(), "{:?}", check_list_framing(&list.packets));
            prop_assert_eq!(list.packet_est.len(), list.packets.len());
            prop_assert!(list.packets.iter().all(|p| p.bytes().len() == 3072));
        }
        let fj = encode_frame(&frame(vec![Polyline(path)]), &m).unwrap();
        prop_assert!(check_list_framing(&fj.once).is_ok(), "{:?}", check_list_framing(&fj.once));
        prop_assert!(check_list_framing(&fj.pass).is_ok(), "{:?}", check_list_framing(&fj.pass));
        let last = fj.once.last().unwrap();
        prop_assert!(last.words().iter().skip(last.real_commands()).all(|w| *w == PAD));
    }

    #[test]
    fn prop_lighting_never_marks(paths in prop::collection::vec(arb_path(20), 1..5)) {
        let m = machine();
        let red = 1u16 << m.ports.red_bit;
        let job = encode_frame(&frame(paths.into_iter().map(Polyline).collect()), &m).unwrap();
        for list in [&job.once, &job.pass] {
            for w in words(list) {
                prop_assert!(
                    matches!(w[0], op::LIST_READY_MARK | op::LIST_WRITE_PORT | op::LIST_JUMP_SPEED | op::LIST_JUMP_DELAY | op::LIST_JUMP_TO | op::LIST_DELAY_TIME),
                    "lighting list holds {:?}", w
                );
                if w[0] == op::LIST_WRITE_PORT {
                    prop_assert!(w[1] == 0 || w[1] == red, "WritePort({:#06x})", w[1]);
                }
            }
        }
    }

    #[test]
    fn prop_encode_disassemble_roundtrip(path in arb_path(30)) {
        let m = machine();
        let want = expected_marks(&path);
        prop_assume!(!want.is_empty());
        let job = encode(vec![Polyline(path.clone())], &m).unwrap();
        prop_assert_eq!(marks(&job), want.clone());
        let start = quantize(path[0]);
        let first_jump = decoded(&job).into_iter().find(|d| matches!(d, Decoded::JumpTo { .. }));
        prop_assert_eq!(first_jump, Some(Decoded::JumpTo { x: start.0, y: start.1, dist: distance_word((CENTER, CENTER), start) }));
        // Every point lands within half a galvo unit of the millimetres asked for.
        let half_unit = field() / 65535.0 / 2.0 + 1e-12;
        for p in &path {
            let (x, y) = quantize(*p);
            prop_assert!((mm(x) - p[0]).abs() <= half_unit && (mm(y) - p[1]).abs() <= half_unit);
        }
        // The listing shows the same marks, in order.
        let listed: Vec<String> =
            disassemble(&job, &m).lines.into_iter().filter(|l| l.words[0] == op::LIST_MARK_TO && l.packet.is_some()).map(|l| l.text).collect();
        prop_assert_eq!(listed.len(), want.len());
        for (text, (x, y, dist)) in listed.iter().zip(&want) {
            prop_assert!(text.starts_with(&format!("MarkTo x={x} y={y} (")) && text.ends_with(&format!(" mm) dist={dist}")), "{}", text);
        }
    }

    #[test]
    fn prop_summary_matches_plan(
        path in arb_path(30),
        mark_current in 0u16..=4095,
        q_period in 5u16..=20000,
        pulse in prop::sample::select(JPT_M7_PULSES_NS.to_vec()),
        speed_units in 1u16..=2000,
        jump_speed_units in 1u16..=2000,
        repeats in 1u16..=5,
        estimate_ms in 0u64..10_000,
    ) {
        let want = expected_marks(&path);
        prop_assume!(!want.is_empty());
        let mut q = galvo_q();
        q.mark_current = mark_current;
        q.q_period = q_period;
        q.pulse = PulseDecision::Exact(pulse);
        q.speed_units = speed_units;
        q.timings.jump_speed_units = jump_speed_units;
        let mut s = section_with(q, vec![Polyline(path.clone())]);
        s.estimate = Duration::from_millis(estimate_ms);
        let (emit, plan) = plans(vec![s], repeats);
        let m = machine();
        let job = encode_job(&emit, &m).unwrap();
        let summary = emission_summary(&job, &m, &plan, &[]).unwrap();
        prop_assert_eq!(summary.fingerprint, plan.fingerprint);
        prop_assert_eq!(summary.bytes_hash, job.bytes_hash());
        prop_assert_eq!(summary.repeats, repeats);
        prop_assert_eq!(summary.packets, u32::try_from(job.lists[0].packets.len()).unwrap());
        prop_assert!(!summary.crash_bound.is_zero());
        prop_assert!(summary.estimate >= plan.estimate);
        let row = &summary.rows[0];
        prop_assert_eq!(row.galvo, Some(GalvoWords { mark_current, q_period, pulse_ns: pulse, speed_units, jump_speed_units }));
        prop_assert_eq!(row.power_pct, f64::from(mark_current) * 100.0 / 4095.0);
        prop_assert_eq!(row.frequency_khz, Some(20000.0 / f64::from(q_period)));
        prop_assert_eq!(row.pulse_ns, Some(pulse));
        prop_assert_eq!(row.speed_mm_s, units_to_speed(speed_units, field()));
        prop_assert_eq!(row.paths, 1);
        // Mark length and bounds from the quantised points (what the galvo is told), not from the request.
        let mut last = quantize(path[0]);
        let (mut len, mut min, mut max) = (0.0, [f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for (x, y, _) in &want {
            let a = [mm(last.0), mm(last.1)];
            let b = [mm(*x), mm(*y)];
            len += (b[0] - a[0]).hypot(b[1] - a[1]);
            for p in [a, b] {
                for axis in 0..2 {
                    min[axis] = min[axis].min(p[axis]);
                    max[axis] = max[axis].max(p[axis]);
                }
            }
            last = (*x, *y);
        }
        prop_assert!((row.mark_mm - len).abs() < 1e-9, "{} vs {}", row.mark_mm, len);
        prop_assert_eq!(row.bounds.min, min);
        prop_assert_eq!(row.bounds.max, max);
    }

    #[test]
    fn prop_encode_is_session_independent(a in arb_path(30), b in arb_path(30)) {
        prop_assume!(!expected_marks(&a).is_empty() && !expected_marks(&b).is_empty());
        let m = machine();
        let ja = encode(vec![Polyline(a.clone())], &m).unwrap();
        let jb = encode(vec![Polyline(b.clone())], &m).unwrap();
        prop_assert_eq!(&encode(vec![Polyline(a.clone())], &m).unwrap(), &ja, "encoding b in between changed a");
        // A list's bytes do not depend on the lists before it (SPEC §5.2 rule 6).
        let (emit, _) = plans(vec![section(vec![Polyline(b)]), section(vec![Polyline(a.clone())])], 1);
        let both = encode_job(&emit, &m).unwrap();
        prop_assert_eq!(&both.lists[0].packets, &jb.lists[0].packets);
        prop_assert_eq!(&both.lists[1].packets, &ja.lists[0].packets);
        // Every list's first jump is measured from the centre.
        let start = quantize(a[0]);
        let first_jump = both.lists[1].packets.iter().flat_map(decode_packet).find(|d| matches!(d, Decoded::JumpTo { .. }));
        prop_assert_eq!(first_jump, Some(Decoded::JumpTo { x: start.0, y: start.1, dist: distance_word((CENTER, CENTER), start) }));
    }
}
