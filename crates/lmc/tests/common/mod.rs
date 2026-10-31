//! Shared fixtures for the `openlaser-lmc` integration tests (plans, machines, T1 scenarios, list framing).
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use openlaser_geom::Polyline;
use openlaser_lmc::py_compat::OmParams;
use openlaser_lmc::{GalvoMachine, LmcFrameJob, LmcJob, Packet, encode_frame, encode_job, op};
use openlaser_machine::{Kinematics, jpt_m7_200_sim};
use openlaser_plan::{CutQ, GalvoCutQ, JobPlan, PlanFacts, ResolvedTimings, Section};
use openlaser_safety::{EmitPlan, FramePlan};
use openlaser_testkit::t1::{self, Event, Scenario};
use openlaser_types::galvo::{JPT_M7_PULSES_NS, PulseDecision, frequency_to_period, nearest_pulse_width, power_to_mark_current, speed_to_units};
use openlaser_types::{DeviceKind, Fingerprint, FrameSource, FrameStyle, Hz, LayerKey, MachineId, MmPerS, Percent, SectionMode};

pub const FP: Fingerprint = Fingerprint([7; 32]);
/// An unused slot (and the explicit EndOfList).
pub const PAD: [u16; 6] = [op::LIST_END_OF_LIST, 0, 0, 0, 0, 0];

pub fn machine() -> GalvoMachine {
    let Kinematics::Galvo(m) = jpt_m7_200_sim().kind else { panic!("the sim profile is a galvo") };
    m
}
pub fn machine_id() -> MachineId {
    jpt_m7_200_sim().id
}
/// F = 236.68 mm.
pub fn field() -> f64 {
    machine().field_f()[0]
}

/// mark_box10's words: 50 %, 30 kHz, 200 ns, 1000 mm/s at F = 236.68; GP's timings.
pub fn galvo_q() -> GalvoCutQ {
    GalvoCutQ {
        requested_speed: MmPerS::new(1000.0).unwrap(),
        speed_units: 276,
        actual_speed_mm_s: 1000.0,
        power: Percent::new(50.0).unwrap(),
        mark_current: 2048,
        frequency: Hz::new(30_000).unwrap(),
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
    }
}

pub fn section_with(q: GalvoCutQ, paths: Vec<Polyline>) -> Section {
    Section {
        rotary: None,
        layer: LayerKey::Color(0),
        layer_name: "C00".into(),
        sub: 0,
        pass: 1,
        of: 1,
        mode: SectionMode::Line,
        cut: CutQ::Galvo(q),
        paths,
        objects: vec![],
        estimate: Duration::from_secs(2),
    }
}
pub fn section(paths: Vec<Polyline>) -> Section {
    section_with(galvo_q(), paths)
}

/// The emit plan and the job plan of the same sections (what `safety::validate` would hand over).
pub fn plans(sections: Vec<Section>, repeats: u16) -> (EmitPlan, JobPlan) {
    let estimate = sections.iter().map(|s| s.estimate).sum::<Duration>() * u32::from(repeats);
    let emit = openlaser_safety::testing::emit_plan_unchecked(sections.clone(), FP, machine_id(), DeviceKind::GalvoLmc, repeats);
    let plan = JobPlan {
        machine: machine_id(),
        device: DeviceKind::GalvoLmc,
        sections,
        frame: vec![],
        tool_frame: vec![],
        facts: PlanFacts::default(),
        repeats,
        estimate,
        fingerprint: FP,
    };
    (emit, plan)
}

pub fn box10() -> Polyline {
    Polyline(vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0], [-5.0, -5.0]])
}
pub fn setup() -> (GalvoMachine, EmitPlan, JobPlan) {
    setup_path(box10())
}
pub fn setup_path(path: Polyline) -> (GalvoMachine, EmitPlan, JobPlan) {
    let (emit, plan) = plans(vec![section(vec![path])], 1);
    (machine(), emit, plan)
}
pub fn encoded(paths: Vec<Polyline>) -> LmcJob {
    let (emit, _) = plans(vec![section(paths)], 1);
    encode_job(&emit, &machine()).unwrap()
}

pub fn frame(paths: Vec<Polyline>) -> FramePlan {
    openlaser_safety::testing::frame_plan_unchecked(paths, Fingerprint([8; 32]), machine_id(), FrameStyle::Contour, FrameSource::Job)
}

/// The real commands of a list, packet after packet, without the padding of the last packet (the explicit
/// EndOfList is not included).
pub fn words(packets: &[Packet]) -> Vec<[u16; 6]> {
    packets.iter().flat_map(|p| p.words().into_iter().take(p.real_commands())).collect()
}

/// Re-packetises list commands as the encoder does: 256 per packet, an explicit EndOfList, `0x8002` padding.
pub fn packetize(mut commands: Vec<[u16; 6]>) -> Vec<Packet> {
    commands.push(PAD);
    commands.chunks(256).map(|c| Packet::from_words(c).unwrap()).collect()
}

/// SPEC §5.2 rule 8: every packet is 3072 bytes; every packet but the last holds 256 real commands; the last holds
/// the explicit EndOfList, then only `0x8002` padding.
pub fn check_list_framing(packets: &[Packet]) -> Result<(), String> {
    let Some((last, full)) = packets.split_last() else { return Err("a list has at least one packet".into()) };
    for (i, p) in full.iter().enumerate() {
        if p.bytes().len() != 3072 || p.real_commands() != 256 {
            return Err(format!("packet {i} of {} holds {} commands, not 256", packets.len(), p.real_commands()));
        }
    }
    let words = last.words();
    let n = last.real_commands();
    if n >= 256 {
        return Err("the last packet has no EndOfList".into());
    }
    if let Some(slot) = words.iter().skip(n).position(|w| *w != PAD) {
        return Err(format!("the last packet's slot {} after the EndOfList is not padding", n + slot));
    }
    Ok(())
}

// ---- T1 scenarios ------------------------------------------------------------------------------------------------

pub const SCENARIOS: [&str; 8] =
    ["mark_box10", "dot_test", "line_test", "long_job", "bc_mark_vector-engrave_pass1", "bc_mark_raster_pass1", "frame_box10", "bc_frame"];

pub fn fixture(oracle: &str, name: &str) -> Scenario {
    t1::load(oracle, name).unwrap_or_else(|e| panic!("{e}"))
}
pub fn paths(s: &Scenario) -> Vec<Vec<[f64; 2]>> {
    t1::polylines(s).unwrap_or_else(|e| panic!("{e}"))
}
pub fn params(s: &Scenario) -> OmParams {
    let p = s.inputs.get("layer").unwrap_or(&s.inputs);
    let number = |key: &str| p.get(key).and_then(serde_json::Value::as_f64).unwrap_or_else(|| panic!("missing {key}"));
    OmParams {
        power: number(if p.get("power").is_some() { "power" } else { "power_percent" }),
        frequency_khz: number("frequency_khz"),
        pulse_width_ns: number("pulse_width_ns"),
        speed_mm_s: number("speed_mm_s"),
    }
}
pub fn is_frame(s: &Scenario) -> bool {
    s.scenario.contains("frame")
}

/// The shipping encoder's frame job for a frame scenario.
pub fn shipping_frame(s: &Scenario) -> LmcFrameJob {
    let m = machine();
    assert_eq!(s.field_size_mm, m.field_f()[0], "{}: the scenario's field is the sim profile's F", s.scenario);
    encode_frame(&frame(paths(s).into_iter().map(Polyline).collect()), &m).unwrap_or_else(|e| panic!("{e}"))
}

/// The shipping encoder's job for a marking scenario, quantised exactly as the planner does (no hard-coded words).
pub fn shipping_job(s: &Scenario) -> LmcJob {
    let m = machine();
    assert_eq!(s.field_size_mm, m.field_f()[0], "{}: the scenario's field is the sim profile's F", s.scenario);
    let p = params(s);
    let mut q = galvo_q();
    q.mark_current = power_to_mark_current(p.power).unwrap_or_else(|e| panic!("{e}"));
    q.q_period = frequency_to_period(p.frequency_khz).unwrap_or_else(|e| panic!("{e}"));
    q.speed_units = speed_to_units(p.speed_mm_s, s.field_size_mm).unwrap_or_else(|e| panic!("{e}"));
    q.pulse = nearest_pulse_width(p.pulse_width_ns, &JPT_M7_PULSES_NS);
    let (emit, _) = plans(vec![section_with(q, paths(s).into_iter().map(Polyline).collect())], 1);
    encode_job(&emit, &m).unwrap_or_else(|e| panic!("{e}"))
}

/// The shipping list a scenario compares against v1: the marking list, or the Frame Once list.
pub fn shipping(s: &Scenario) -> Vec<Packet> {
    if is_frame(s) {
        return shipping_frame(s).once;
    }
    let job = shipping_job(s);
    assert_eq!(job.lists.len(), 1);
    job.lists.into_iter().flat_map(|l| l.packets).collect()
}

/// The oracle's job packets (the `job` segment; the abort's dummy packet is outside it).
pub fn oracle_packets(s: &Scenario) -> Vec<Packet> {
    let [start, end] = s.segments["job"];
    s.events[start..end]
        .iter()
        .filter_map(|e| match e {
            Event::List { sha256, .. } => Some(Packet::try_from_bytes(&t1::packet(sha256).unwrap_or_else(|e| panic!("{e}"))).unwrap_or_else(|e| panic!("{e}"))),
            _ => None,
        })
        .collect()
}
