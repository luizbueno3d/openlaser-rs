use std::time::Duration;

use openlaser_geom::Polyline;
use openlaser_lmc::{Decoded, Packet, decode_packet, disassemble, disassemble_frame, emission_summary, encode_frame, encode_job, op};
use openlaser_machine::{Kinematics, jpt_m7_200_sim};
use openlaser_plan::{CutQ, GalvoCutQ, JobPlan, PlanFacts, ResolvedTimings, Section};
use openlaser_safety::EmitPlan;
use openlaser_types::galvo::PulseDecision;
use openlaser_types::{DeviceKind, Fingerprint, FrameSource, FrameStyle, Hz, LayerKey, MachineId, MmPerS, Percent, SectionMode};
use proptest::prelude::*;

fn setup() -> (openlaser_machine::GalvoMachine, EmitPlan, JobPlan) {
    setup_path(Polyline(vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0], [-5.0, -5.0]]))
}

#[allow(clippy::panic)] // Test fixture invariant: the built-in simulator profile is a galvo.
fn setup_path(path: Polyline) -> (openlaser_machine::GalvoMachine, EmitPlan, JobPlan) {
    let profile = jpt_m7_200_sim();
    let Kinematics::Galvo(machine) = profile.kind else { panic!("sim profile must be galvo") };
    let id = profile.id;
    let section = Section {
        layer: LayerKey::Color(0),
        layer_name: "C00".into(),
        sub: 0,
        pass: 1,
        of: 1,
        mode: SectionMode::Line,
        cut: CutQ::Galvo(GalvoCutQ {
            requested_speed: MmPerS::new(1000.0).unwrap_or(MmPerS::MAX_VALID),
            speed_units: 276,
            actual_speed_mm_s: 1000.0,
            power: Percent::new(50.0).unwrap_or(Percent::MAX_VALID),
            mark_current: 2048,
            frequency: Hz::new(30_000).unwrap_or(Hz::MAX_VALID),
            q_period: 667,
            pulse: PulseDecision::Exact(200),
            timings: ResolvedTimings {
                laser_on: (100, 0),
                laser_off: (100, 0),
                polygon: (100, 0),
                jump_delay_short: (8, 0),
                jump_delay_long: (200, 0),
                jump_speed_units: 830,
                mo_settle_10us: 800,
                first_jump_settle_10us: 100,
            },
        }),
        paths: vec![path],
        objects: vec![],
        estimate: Duration::from_secs(2),
    };
    let fp = Fingerprint([7; 32]);
    let emit = openlaser_safety::testing::emit_plan_unchecked(vec![section.clone()], fp, id.clone(), DeviceKind::GalvoLmc, 1);
    let plan = JobPlan {
        machine: id,
        device: DeviceKind::GalvoLmc,
        sections: vec![section],
        frame: vec![],
        tool_frame: vec![],
        facts: PlanFacts::default(),
        repeats: 1,
        estimate: Duration::from_secs(2),
        fingerprint: fp,
    };
    (machine, emit, plan)
}

#[test]
fn box10_has_explicit_jump_and_settle() {
    let (m, emit, _) = setup();
    let job = encode_job(&emit, &m).unwrap_or_else(|e| panic!("{e}"));
    let decoded: Vec<_> = job.lists.iter().flat_map(|l| l.packets.iter().flat_map(decode_packet)).collect();
    assert!(matches!(decoded.get(11), Some(Decoded::JumpDelay(200, 0))));
    assert!(matches!(decoded.get(12), Some(Decoded::JumpTo { x: 31384, y: 31384, dist: 1957 })));
    assert!(matches!(decoded.get(13), Some(Decoded::DelayTime(100))));
    assert_eq!(decoded.iter().filter(|d| matches!(d, Decoded::MarkTo { .. })).count(), 4);
    assert_eq!(disassemble(&job, &m).lines.len(), decoded.len());
}

#[test]
fn prop_packets_3072_padded_fixed_examples() {
    for count in [0, 1, 17, 255, 256] {
        let words = vec![[openlaser_lmc_opcode_ready(), 0, 0, 0, 0, 0]; count];
        let p = Packet::from_words(&words).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(p.bytes().len(), 3072);
        assert_eq!(p.real_commands(), count);
        assert_eq!(p.words().len(), 256);
    }
}
fn openlaser_lmc_opcode_ready() -> u16 {
    op::LIST_READY_MARK
}

#[test]
fn prop_lighting_never_marks_and_summary_reads_encoded_values() {
    let (m, emit, plan) = setup();
    let job = encode_job(&emit, &m).unwrap_or_else(|e| panic!("{e}"));
    let summary = emission_summary(&job, &m, &plan, &[]).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(summary.rows.first().map(|r| r.power_pct), Some(2048.0 * 100.0 / 4095.0));
    assert_eq!(summary.bytes_hash, job.bytes_hash());
    let frame = openlaser_safety::testing::frame_plan_unchecked(
        vec![Polyline(vec![[-5.0, -5.0], [5.0, -5.0]])],
        Fingerprint([8; 32]),
        MachineId("jpt-m7-200-sim".into()),
        FrameStyle::Bounds,
        FrameSource::Job,
    );
    let frame = encode_frame(&frame, &m).unwrap_or_else(|e| panic!("{e}"));
    assert!(disassemble_frame(&frame, &m).lines.iter().all(|l| l.words[0] != op::LIST_MARK_TO));
}

#[test]
fn bytes_hash_changes_with_any_byte() {
    let (m, emit, _) = setup();
    let job = encode_job(&emit, &m).unwrap_or_else(|e| panic!("{e}"));
    let clone = encode_job(&emit, &m).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(job, clone);
    let hash = job.bytes_hash();
    for i in 0..3072 {
        let mut changed = job.clone();
        let mut bytes = *changed.lists[0].packets[0].bytes();
        bytes[i] ^= 1;
        changed.lists[0].packets[0] = Packet::try_from_bytes(&bytes).unwrap_or_else(|e| panic!("{e}"));
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
}

proptest! {
    #[test]
    fn prop_packets_3072_padded(count in 0usize..=256) {
        let words=vec![[op::LIST_READY_MARK,0,0,0,0,0];count];
        let packet=Packet::from_words(&words).unwrap_or_else(|e|panic!("{e}"));
        prop_assert_eq!(packet.bytes().len(),3072);
        prop_assert_eq!(packet.real_commands(),count);
        prop_assert!(packet.words().iter().skip(count).all(|w|w[0]==op::LIST_END_OF_LIST));
    }

    #[test]
    fn prop_lighting_never_marks(x0 in -99.0f64..99.0, y0 in -99.0f64..99.0, x1 in -99.0f64..99.0, y1 in -99.0f64..99.0) {
        let (m,_,_)=setup();
        let frame=openlaser_safety::testing::frame_plan_unchecked(vec![Polyline(vec![[x0,y0],[x1,y1]])],Fingerprint([1;32]),MachineId("jpt-m7-200-sim".into()),FrameStyle::Bounds,FrameSource::Job);
        let job=encode_frame(&frame,&m).unwrap_or_else(|e|panic!("{e}"));
        prop_assert!(disassemble_frame(&job,&m).lines.iter().all(|l|!matches!(l.words[0],op::LIST_MARK_TO|op::LIST_LASER_ON_POINT|op::LIST_FIBER_OPEN_MO)));
    }

    #[test]
    fn prop_encode_disassemble_roundtrip(x in -99.0f64..99.0,y in -99.0f64..99.0) {
        let (m,emit,_)=setup_path(Polyline(vec![[-5.0,-5.0],[x,y]]));
        let job=encode_job(&emit,&m).unwrap_or_else(|e|panic!("{e}"));
        let decoded:Vec<_>=job.lists.iter().flat_map(|l|l.packets.iter().flat_map(decode_packet)).collect();
        prop_assert_eq!(decoded.iter().filter(|d|matches!(d,Decoded::MarkTo{..})).count(),1);
        prop_assert_eq!(disassemble(&job,&m).lines.len(),decoded.len());
    }

    #[test]
    fn prop_summary_matches_plan(_seed in any::<u8>()) {
        let (m,emit,plan)=setup();let job=encode_job(&emit,&m).unwrap_or_else(|e|panic!("{e}"));
        let summary=emission_summary(&job,&m,&plan,&[]).unwrap_or_else(|e|panic!("{e}"));
        prop_assert_eq!(summary.fingerprint,plan.fingerprint);
        prop_assert_eq!(summary.repeats,plan.repeats);
        prop_assert_eq!(summary.rows.len(),plan.sections.len());
        prop_assert_eq!(summary.bytes_hash,job.bytes_hash());
    }

    #[test]
    fn prop_encode_is_session_independent(_seed in any::<u8>()) {
        let (m,emit,_)=setup();
        prop_assert_eq!(encode_job(&emit,&m).unwrap_or_else(|e|panic!("{e}")),encode_job(&emit,&m).unwrap_or_else(|e|panic!("{e}")));
    }
}

#[path = "acceptance/t1.rs"]
mod t1;

#[test]
fn frame_nonfinite_geometry_returns_error_without_panicking() {
    let (m, _, _) = setup();
    for value in [f64::NAN, f64::INFINITY, f64::MAX] {
        let frame = openlaser_safety::testing::frame_plan_unchecked(
            vec![Polyline(vec![[value, 0.0], [0.0, 0.0]])],
            Fingerprint([1; 32]),
            MachineId("sim".into()),
            FrameStyle::Contour,
            FrameSource::Job,
        );
        assert!(encode_frame(&frame, &m).is_err());
    }
}
