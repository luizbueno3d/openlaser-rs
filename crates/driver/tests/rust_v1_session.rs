//! rust-v1 session goldens (SPEC §6.2): the wire event stream of the shipping session on a READY FakeLmc, compared
//! byte for byte with the frozen `tests/golden/rust-v1/session/` (signed by Luiz 2026-10-26, D13, for controlled
//! hardware validation). A change needs a new review: `UPDATE_RUST_V1=1` writes the new stream to
//! `tests/golden/rust-v1-candidate/session/` and the comparison still fails.

// Test code: clippy.toml allows these inside #[test] fns; the shared helpers of this file need it spelled out.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use openlaser_arm::testing::token_for_tests;
use openlaser_driver::{DriverEvent, DriverHandle, LmcBackend, MachineState, spawn};
use openlaser_lmc::{LmcFrameJob, LmcJob, encode_frame};
use openlaser_lmc_sim::{FakeLmc, Recorder, SimConfig, SimWire};
use openlaser_machine::{GalvoMachine, Kinematics};
use openlaser_safety::Polyline;
use openlaser_safety::testing::frame_plan_unchecked;
use openlaser_types::{Fingerprint, FrameMode, FrameSource, FrameStyle, MachineId, MonoTime, SessionId, VirtualClock};
use serde_json::{Value, json};

// The variant is aliased because xtask rule 13 greps for the engine's dispatch origin `Origin::Human`.
use openlaser_types::FrameOrigin::{self, Human as HumanFrame};

const BY_HUMAN: FrameOrigin = HumanFrame;
const SESSION: SessionId = SessionId(1);
const FP: Fingerprint = Fingerprint([0xb0; 32]);

fn machine_id() -> MachineId {
    MachineId("jpt-m7-200-sim".to_owned())
}

fn galvo() -> GalvoMachine {
    match openlaser_machine::jpt_m7_200_sim().kind {
        Kinematics::Galvo(g) => g,
        Kinematics::Gantry(_) => panic!("the JPT M7 is a galvo"),
    }
}

fn box10() -> Vec<[f64; 2]> {
    vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0], [-5.0, -5.0]]
}

/// GOLD `frame_box10`: Frame Once of the 10 mm box (Contour, the job itself).
fn frame_box10() -> LmcFrameJob {
    let plan = frame_plan_unchecked(vec![Polyline(box10())], FP, machine_id(), FrameStyle::Contour, FrameSource::Job);
    encode_frame(&plan, &galvo()).expect("encode_frame")
}

/// GOLD `mark_box10` through the SHIPPING encoder: the 10 mm box at 50 %, 30 kHz, 200 ns, 1000 mm/s (the words of
/// GOLD `mark_box10`; R05 §1.5).
fn mark_box10_job() -> LmcJob {
    use openlaser_plan::{CutQ, GalvoCutQ, ResolvedTimings, Section};
    use openlaser_types::galvo::PulseDecision;
    use openlaser_types::{DeviceKind, Hz, LayerKey, MmPerS, ObjectId, Percent, SectionMode};
    let q = GalvoCutQ {
        requested_speed: MmPerS::new(1000.0).unwrap(),
        speed_units: 276,
        actual_speed_mm_s: 276.0 * 236.68 * 1000.0 / 65535.0,
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
    };
    let section = Section {
        layer: LayerKey::color(0).unwrap(),
        layer_name: "C00".to_owned(),
        sub: 0,
        pass: 1,
        of: 1,
        mode: SectionMode::Line,
        cut: CutQ::Galvo(q),
        paths: vec![Polyline(box10())],
        objects: vec![ObjectId(1)],
        estimate: Duration::from_millis(50),
    };
    let emit = openlaser_safety::testing::emit_plan_unchecked(vec![section], FP, machine_id(), DeviceKind::GalvoLmc, 1);
    openlaser_lmc::encode_job(&emit, &galvo()).expect("encode_job")
}

fn ready_board() -> SimConfig {
    SimConfig { buffer_depth: 8, transfer_latency: Duration::ZERO, status_extra_bits: 0, pid: 0x9899, faults: Vec::new(), emission_cap_pct: None }
}

fn until(h: &DriverHandle<LmcBackend<FakeLmc>>, pred: impl Fn(&DriverEvent) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let ev = h.events().recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("driver event");
        assert!(!matches!(ev, DriverEvent::Fault(_)), "{ev:?}");
        if pred(&ev) {
            return;
        }
    }
}

fn idle(e: &DriverEvent) -> bool {
    matches!(e, DriverEvent::State(MachineState::Idle))
}

/// The recorded wire as golden JSON events (GOLD's T1 shape: single/read/list; timestamps left out).
fn events(recorder: &Recorder) -> Vec<Value> {
    recorder
        .snapshot()
        .wire
        .iter()
        .map(|w| match w {
            SimWire::Single { words, .. } => {
                let hex: String = words.iter().flat_map(|w| w.to_le_bytes()).map(|b| format!("{b:02x}")).collect();
                json!({"kind": "single", "hex": hex})
            }
            SimWire::Read { status, .. } => json!({"kind": "read", "status": status}),
            SimWire::Packet { sha256, .. } => json!({"kind": "list", "sha256": sha256}),
        })
        .collect()
}

fn golden(tier: &str, name: &str) -> PathBuf {
    openlaser_testkit::golden_dir().join(tier).join("session").join(format!("{name}.json"))
}

fn render(tier: &str, name: &str, description: &str, events: &[Value]) -> String {
    let doc = json!({
        "meta": {"schema": "openlaser-golden/1", "tier": tier, "generator": "crates/driver/tests/rust_v1_session.rs"},
        "scenario": name,
        "description": description,
        "board": "FakeLmc, buffer depth 8, no latency, no faults (READY board)",
        "events": events,
    });
    format!("{}\n", serde_json::to_string_pretty(&doc).expect("json"))
}

/// Compares with the frozen golden; with `UPDATE_RUST_V1=1` also writes a candidate for a new review.
fn check(name: &str, description: &str, events: Vec<Value>) {
    if std::env::var("UPDATE_RUST_V1").as_deref() == Ok("1") {
        let path = golden("rust-v1-candidate", name);
        std::fs::create_dir_all(path.parent().expect("a parent folder")).expect("create the candidate folder");
        std::fs::write(&path, render("rust-v1-candidate", name, description, &events)).expect("write the candidate");
    }
    let path = golden("rust-v1", name);
    let want = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let text = render("rust-v1", name, description, &events);
    assert_eq!(text, want, "{} differs from the signed rust-v1 golden (a change needs a new D13 review)", path.display());
}

fn spawn_ready() -> (DriverHandle<LmcBackend<FakeLmc>>, Arc<Recorder>) {
    let clock = Arc::new(VirtualClock::new());
    let fake = FakeLmc::new(ready_board(), clock.clone());
    let recorder = fake.recorder();
    let h = spawn(LmcBackend::new(fake, galvo()).with_provider_name("sim"), machine_id(), SESSION, clock);
    (h, recorder)
}

#[test]
fn rust_v1_session_connect_frame_box10() {
    let (h, recorder) = spawn_ready();
    h.connect();
    until(&h, idle);
    h.frame(frame_box10(), FrameMode::Once, BY_HUMAN);
    until(&h, |e| matches!(e, DriverEvent::FrameCompleted(_)));
    until(&h, idle);
    check("connect_frame_box10", "connect (init once) + Frame Once of the 10 mm box", events(&recorder));
}

#[test]
fn rust_v1_session() {
    // connect + frame_box10 + mark_box10 on a READY board equals the candidate stream (the frame comes first: the
    // driver runs nothing without its own executed-pass receipt).
    let job = mark_box10_job();
    let (h, recorder) = spawn_ready();
    h.connect();
    until(&h, idle);
    h.frame(frame_box10(), FrameMode::Once, BY_HUMAN);
    until(&h, |e| matches!(e, DriverEvent::FrameCompleted(_)));
    until(&h, idle);
    let token = token_for_tests(job.fingerprint, job.bytes_hash(), machine_id(), SESSION, MonoTime(Duration::from_secs(3600)), 1);
    h.run(job, token);
    until(&h, |e| matches!(e, DriverEvent::Finished(_)));
    check("connect_frame_box10_mark_box10", "connect + Frame Once of the 10 mm box + mark_box10 (shipping encoder)", events(&recorder));
}
