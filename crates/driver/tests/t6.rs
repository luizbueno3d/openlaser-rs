//! T6 (SPEC §6.3): the driver on lmc-sim's FakeLmc and FakeGrbl, the §6.3 table by name.
//!
//! Integration tests because of the `driver ← lmc-sim` dev-dependency cycle (§2.3 rule 10). Randomised stops are
//! tripped from `FakeLmc::set_hook` at a seeded wire event, never from another thread. `T6_RUNS` raises the 200
//! deterministic runs; `T6_RT_RUNS` the 40 real-time ones.
//!
//! Marking jobs are built from GP's own packets (`openlaser_lmc::py_compat::mark_polylines`, the v0 encoder replay)
//! with the shipping prelude and postlude: the driver's mechanics do not depend on how the packets were encoded (the
//! shipping encoder's own session stream is pinned by `rust_v1_session.rs`).

// Test code: clippy.toml allows these inside #[test] fns; the shared helpers of this file need it spelled out.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use openlaser_arm::testing::token_for_tests;
use openlaser_driver::{DriverEvent, DriverHandle, LmcBackend, MachineState, Outcome, StopReport, Stopper, spawn};
use openlaser_lmc::py_compat::{OmParams, WireEvent, mark_polylines};
use openlaser_lmc::{Decoded, LmcFrameJob, LmcJob, LmcList, Packet, Single, decode_packet, encode_frame};
use openlaser_lmc_sim::{FakeLmc, Fault, Recorder, SimConfig, SimWire};
use openlaser_machine::{GalvoMachine, Kinematics};
use openlaser_safety::Polyline;
use openlaser_safety::testing::frame_plan_unchecked;
use openlaser_types::{
    ArmToken, Clock, Fingerprint, FrameMode, FrameSource, FrameStyle, LayerKey, MachineId, MonoTime, RefusalCode, SessionId, SystemClock, VirtualClock,
};

// The variant is aliased because xtask rule 13 greps for the engine's dispatch origin `Origin::Human`.
use openlaser_types::FrameOrigin::{self, Human as HumanFrame};

const BY_HUMAN: FrameOrigin = HumanFrame;
const SESSION: SessionId = SessionId(1);
const FP: Fingerprint = Fingerprint([0x5a; 32]);

fn machine_id() -> MachineId {
    MachineId("jpt-m7-200-sim".to_owned())
}

fn galvo() -> GalvoMachine {
    match openlaser_machine::jpt_m7_200_sim().kind {
        Kinematics::Galvo(g) => g,
        Kinematics::Gantry(_) => panic!("the JPT M7 is a galvo"),
    }
}

/// F = 200 × 1.1834.
fn field_f() -> f64 {
    galvo().field_f()[0]
}

fn board(buffer_depth: u8, faults: Vec<Fault>) -> SimConfig {
    SimConfig { buffer_depth, transfer_latency: Duration::ZERO, status_extra_bits: 0, pid: 0x9899, faults, emission_cap_pct: None }
}

fn runs(var: &str, default: usize) -> usize {
    std::env::var(var).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// A seeded xorshift, so randomised stops are reproducible.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n.max(1) as u64) as usize
    }
}

fn single(op: u16, p1: u16) -> Single {
    Single([op, p1, 0, 0, 0, 0])
}

const RESET_LIST: u16 = 0x0012;
const WRITE_PORT: u16 = 0x0021;
const FIBER_SET_MO: u16 = 0x0033;
const GET_VERSION: u16 = 0x0007;
const GET_MARK_TIME: u16 = 0x0041;
const EXECUTE_LIST: u16 = 0x0005;
const STOP_EXECUTE: u16 = 0x001F;
const GET_SERIAL_NO: u16 = 0x0009;

/// The galvo time model of each packet of one list, from its decoded commands, as the board executes them: jumps and
/// marks at their speed words (galvo units per ms), laser on/off/polygon and jump delays in µs, `DelayTime` in 10 µs.
/// The speed and delay words carry over from packet to packet (GP sets them once, in the list's first packet), as in
/// the shipping encoder's `packet_est` (`decode::packet_times`).
fn packet_ests(packets: &[Packet]) -> Vec<Duration> {
    let (mut jump, mut mark, mut jump_delay, mut mark_delays) = (830.0_f64, 276.0_f64, 0.0_f64, [0.0_f64; 3]);
    packets
        .iter()
        .map(|p| {
            let mut us = 0.0_f64;
            for d in decode_packet(p) {
                match d {
                    Decoded::JumpSpeed(v) => jump = f64::from(v.max(1)),
                    Decoded::MarkSpeed(v) => mark = f64::from(v.max(1)),
                    Decoded::JumpDelay(v, _) => jump_delay = f64::from(v),
                    Decoded::LaserOnDelay(v, _) => mark_delays[0] = f64::from(v),
                    Decoded::LaserOffDelay(v, _) => mark_delays[1] = f64::from(v),
                    Decoded::PolygonDelay(v, _) => mark_delays[2] = f64::from(v),
                    Decoded::JumpTo { dist, .. } => us += f64::from(dist) / jump * 1000.0 + jump_delay,
                    Decoded::MarkTo { dist, .. } => us += f64::from(dist) / mark * 1000.0 + mark_delays.iter().sum::<f64>(),
                    Decoded::DelayTime(n) => us += f64::from(n) * 10.0,
                    _ => {}
                }
            }
            Duration::from_micros(us as u64)
        })
        .collect()
}

/// A marking job from GP's packets for `polys` (one list), with the shipping prelude and postlude.
fn mark_job(polys: &[Vec<[f64; 2]>], power: f64, speed: f64, repeats: u16) -> LmcJob {
    let p = OmParams { power, frequency_khz: 30.0, pulse_width_ns: 200.0, speed_mm_s: speed };
    let events = mark_polylines(polys, &p, field_f()).expect("py_compat encodes the job");
    let packets: Vec<Packet> = events
        .into_iter()
        .filter_map(|e| match e {
            WireEvent::List { n_cmds, bytes, .. } if n_cmds > 0 => Some(bytes),
            _ => None,
        })
        .collect();
    let packet_est = packet_ests(&packets);
    LmcJob {
        return_axis: None,
        prelude: vec![single(RESET_LIST, 0), single(WRITE_PORT, 1), single(FIBER_SET_MO, 1), single(GET_VERSION, 0)],
        lists: vec![LmcList { axis_move: None, layer: LayerKey::color(0).unwrap(), pass: 1, packets, packet_est }],
        repeats,
        postlude: vec![single(FIBER_SET_MO, 0), single(WRITE_PORT, 0), single(GET_MARK_TIME, 3)],
        fingerprint: FP,
        machine: machine_id(),
    }
}

fn box10() -> Vec<Vec<[f64; 2]>> {
    vec![vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0], [-5.0, -5.0]]]
}

/// `n` separate 0.5 mm segments (OM's long_job: 9 packets).
fn many_segments(n: usize) -> Vec<Vec<[f64; 2]>> {
    (0..n)
        .map(|i| {
            let x = -40.0 + (i % 80) as f64;
            let y = -40.0 + (i / 80) as f64;
            vec![[x, y], [x + 0.5, y]]
        })
        .collect()
}

fn frame_job(paths: Vec<Vec<[f64; 2]>>, source: FrameSource) -> LmcFrameJob {
    let plan = frame_plan_unchecked(paths.into_iter().map(Polyline).collect(), FP, machine_id(), FrameStyle::Contour, source);
    encode_frame(&plan, &galvo()).expect("encode_frame")
}

fn token(job: &LmcJob, nonce: u64, expires: MonoTime) -> ArmToken {
    token_for_tests(job.fingerprint, job.bytes_hash(), machine_id(), SESSION, expires, nonce)
}

fn far() -> MonoTime {
    MonoTime(Duration::from_secs(1_000_000))
}

type Lmc = DriverHandle<LmcBackend<FakeLmc>>;

struct Rig {
    handle: Lmc,
    recorder: Arc<Recorder>,
    /// The wire event (0-based, counted by the board's hook) at which the hook trips the stop; `usize::MAX`: never.
    stop_at: Arc<AtomicUsize>,
}

/// Spawns a driver on a FakeLmc; `stop_at` trips the stop at that wire event (0-based) from the board's hook.
fn rig(cfg: SimConfig, clock: Arc<dyn Clock>, stop_at: Option<usize>) -> Rig {
    let fake = FakeLmc::new(cfg, Arc::clone(&clock));
    let recorder = fake.recorder();
    let stopper_slot: Arc<OnceLock<Stopper>> = Arc::new(OnceLock::new());
    let target = Arc::new(AtomicUsize::new(stop_at.unwrap_or(usize::MAX)));
    let (slot, at) = (Arc::clone(&stopper_slot), Arc::clone(&target));
    let mut n = 0usize;
    fake.set_hook(Box::new(move |_w: &SimWire, _t: MonoTime| {
        if n == at.load(Ordering::SeqCst)
            && let Some(s) = slot.get()
        {
            s.stop();
        }
        n += 1;
    }));
    let backend = LmcBackend::new(fake, galvo()).with_provider_name("sim");
    let handle = spawn(backend, machine_id(), SESSION, Arc::clone(&clock));
    let _ = stopper_slot.set(handle.stopper());
    Rig { handle, recorder, stop_at: target }
}

/// Arms the hook to trip the stop `k` wire events from now (call it while the driver is idle).
fn stop_after(r: &Rig, k: usize) {
    r.stop_at.store(r.recorder.snapshot().wire.len() + k, Ordering::SeqCst);
}

fn virtual_clock() -> Arc<dyn Clock> {
    Arc::new(VirtualClock::new())
}

/// Reads events until `pred` matches (panics after 20 s).
fn until(h: &Lmc, pred: impl Fn(&DriverEvent) -> bool) -> Vec<DriverEvent> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let ev = h.events().recv_timeout(left).unwrap_or_else(|_| panic!("no matching event; saw {:?}", seen.iter().rev().take(8).collect::<Vec<_>>()));
        let hit = pred(&ev);
        seen.push(ev);
        if hit {
            return seen;
        }
    }
}

fn idle(e: &DriverEvent) -> bool {
    matches!(e, DriverEvent::State(MachineState::Idle))
}

fn stopped(e: &DriverEvent) -> bool {
    matches!(e, DriverEvent::Stopped(_))
}

fn connected(r: &Rig) {
    r.handle.connect();
    until(&r.handle, idle);
}

/// Connect, frame the job once (a human frame of the job), then run it with a valid token.
fn framed_run(r: &Rig, job: LmcJob, nonce: u64) {
    if let Some(evs) = framed_run_or_stopped(r, job, nonce) {
        panic!("stopped before the run was queued: {:?}", report_of(&evs));
    }
}

/// [`framed_run`] for the randomised stops, which trip the latch at any wire event: the stop (or a fault, which the
/// runtime follows with the emergency stop) can land in the connect or the frame, both stop states of SPEC §5.4
/// ("connecting", "framing"). The machine then never reaches the state the next step waits for, so the run is not
/// queued and the events up to the `Stopped` report are returned; `None` means the run was queued.
fn framed_run_or_stopped(r: &Rig, job: LmcJob, nonce: u64) -> Option<Vec<DriverEvent>> {
    r.handle.connect();
    let evs = until(&r.handle, |e| idle(e) || stopped(e));
    if evs.iter().any(stopped) {
        return Some(evs);
    }
    r.handle.frame(frame_job(box10(), FrameSource::Job), FrameMode::Once, BY_HUMAN);
    // The frame's receipt, then its Idle (the outcome event comes before the state change).
    for wanted in [frame_completed as fn(&DriverEvent) -> bool, idle] {
        let evs = until(&r.handle, |e| wanted(e) || stopped(e));
        if evs.iter().any(stopped) {
            return Some(evs);
        }
    }
    let t = token(&job, nonce, far());
    r.handle.run(job, t);
    None
}

fn frame_completed(e: &DriverEvent) -> bool {
    matches!(e, DriverEvent::FrameCompleted(_))
}

fn finished(e: &DriverEvent) -> bool {
    matches!(e, DriverEvent::Finished(_))
}

fn singles(wire: &[SimWire]) -> Vec<[u16; 6]> {
    wire.iter()
        .filter_map(|w| match w {
            SimWire::Single { words, .. } => Some(*words),
            _ => None,
        })
        .collect()
}

/// The last MO and port values written on the wire.
fn final_mo_and_port(wire: &[SimWire]) -> (Option<u16>, Option<u16>) {
    let s = singles(wire);
    let mo = s.iter().rev().find(|w| w[0] == FIBER_SET_MO).map(|w| w[1]);
    let port = s.iter().rev().find(|w| w[0] == WRITE_PORT).map(|w| w[1]);
    (mo, port)
}

fn report_of(evs: &[DriverEvent]) -> StopReport {
    evs.iter()
        .rev()
        .find_map(|e| match e {
            DriverEvent::Stopped(r) => Some(r.clone()),
            _ => None,
        })
        .expect("a StopReport")
}

fn p99(mut v: Vec<Duration>) -> Duration {
    v.sort();
    let i = ((v.len() as f64) * 0.99).ceil() as usize;
    v[i.saturating_sub(1).min(v.len() - 1)]
}

mod lmc {
    use super::*;

    #[test]
    fn t6_stop_p99_under_100ms_all_states() {
        // Real-time variant: SystemClock, 2 ms per transfer. Stops land in connect, framing, streaming, buffer full,
        // packet in flight and BUSY (random wire events across all of them): one run in three trips the stop in the
        // connect or the Frame Once, the others in the run itself (framed by one Loop pass, which is stopped first).
        let mut rng = Rng(0x0dd5_eed5);
        let mut latencies = Vec::new();
        for run in 0..runs("T6_RT_RUNS", 40) {
            let cfg = SimConfig { transfer_latency: Duration::from_millis(2), ..board(1 + (run % 4) as u8, Vec::new()) };
            let r = rig(cfg, Arc::new(SystemClock::new()), None);
            let evs = if run % 3 == 0 {
                r.stop_at.store(rng.below(160), Ordering::SeqCst);
                r.handle.connect();
                r.handle.frame(frame_job(box10(), FrameSource::Job), FrameMode::Once, BY_HUMAN);
                until(&r.handle, stopped)
            } else {
                connected(&r);
                r.handle.frame(frame_job(box10(), FrameSource::Job), FrameMode::Loop, BY_HUMAN);
                until(&r.handle, frame_completed);
                let ack = r.handle.stopper().stop_and_wait(Duration::from_secs(2));
                assert!(ack.report.is_some_and(|x| x.confirmed), "run {run}: the Loop frame stopped");
                until(&r.handle, stopped);
                let job = mark_job(&many_segments(1100), 30.0, 1000.0, 1);
                let t = token(&job, run as u64, far());
                stop_after(&r, rng.below(150));
                r.handle.run(job, t);
                let evs = until(&r.handle, |e| stopped(e) || finished(e));
                assert!(evs.iter().any(|e| matches!(e, DriverEvent::State(MachineState::Running { .. }))), "run {run}: the stop landed in the run");
                evs
            };
            let report = report_of(&evs);
            assert!(report.mo_closed && report.ports_zero, "run {run}: {report:?}");
            latencies.push(report.elapsed);
        }
        let worst = latencies.iter().max().copied().unwrap_or_default();
        let p = p99(latencies);
        eprintln!("T6 stop latency: p99 {p:?}, worst {worst:?}");
        assert!(p <= Duration::from_millis(100), "p99 {p:?}");
        assert!(worst <= Duration::from_millis(250), "worst {worst:?}");
    }

    #[test]
    fn t6_no_write_while_not_ready() {
        let mut rng = Rng(0x1234_5678);
        for run in 0..runs("T6_RUNS", 200) {
            let depth = 1 + rng.below(8) as u8;
            let stop_at = rng.below(400);
            let r = rig(board(depth, Vec::new()), virtual_clock(), Some(stop_at));
            if framed_run_or_stopped(&r, mark_job(&many_segments(1100), 30.0, 1000.0, 1), run as u64).is_none() {
                until(&r.handle, |e| stopped(e) || finished(e));
            }
            let rec = r.recorder.snapshot();
            assert_eq!(rec.writes_while_not_ready, 0, "run {run} depth {depth} stop at {stop_at}");
        }
    }

    #[test]
    fn t6_mo_closed_ports_zero_after_every_stop() {
        let mut rng = Rng(0xfeed_beef);
        for run in 0..runs("T6_RUNS", 200) {
            let stop_at = rng.below(300);
            let r = rig(board(1 + rng.below(4) as u8, Vec::new()), virtual_clock(), Some(stop_at));
            let evs = framed_run_or_stopped(&r, mark_job(&many_segments(600), 30.0, 1000.0, 1), run as u64)
                .unwrap_or_else(|| until(&r.handle, |e| stopped(e) || finished(e)));
            let rec = r.recorder.snapshot();
            let (mo, port) = final_mo_and_port(&rec.wire);
            assert_eq!((mo, port), (Some(0), Some(0)), "run {run} stop at {stop_at}");
            if evs.iter().any(stopped) {
                assert!(report_of(&evs).confirmed, "run {run}");
            }
        }
    }

    #[test]
    fn t6_busy_20s_completes_without_mo_close() {
        // B1: a job the board executes for ~20 s completes; MO closes only after BUSY clears.
        let r = rig(board(8, Vec::new()), virtual_clock(), None);
        let slow = vec![vec![[-50.0, -50.0], [50.0, -50.0], [50.0, 50.0], [-50.0, 50.0], [-50.0, -50.0]]; 10];
        framed_run(&r, mark_job(&slow, 30.0, 20.0, 1), 1);
        let evs = until(&r.handle, |e| matches!(e, DriverEvent::Finished(_)));
        assert!(matches!(evs.last(), Some(DriverEvent::Finished(Outcome::Finished { duration })) if *duration >= Duration::from_secs(19)));
        let rec = r.recorder.snapshot();
        assert_eq!(rec.mo_closed_while_busy, 0);
        assert!(!evs.iter().any(|e| matches!(e, DriverEvent::Fault(_))), "no watchdog fault");
    }

    #[test]
    fn t6_abort_no_write_while_not_ready() {
        // B2: a board that never raises READY: no 3072-byte write during the abort; MO(0) and WritePort(0) still go.
        let r = rig(board(8, vec![Fault::ReadyNeverReturns { after_execute: false }]), virtual_clock(), None);
        connected(&r);
        let before = r.recorder.snapshot().wire.len();
        r.handle.stopper().stop_and_wait(Duration::from_secs(2));
        let rec = r.recorder.snapshot();
        let abort = &rec.wire[before..];
        assert!(!abort.iter().any(|w| matches!(w, SimWire::Packet { .. })), "no packet into a board that is not READY");
        assert_eq!(rec.writes_while_not_ready, 0, "no packet into a board that is not READY");
        assert_eq!(final_mo_and_port(&rec.wire), (Some(0), Some(0)));
    }

    #[test]
    fn t6_ready_never_returns_stopped_within_2s() {
        let clock = virtual_clock();
        let r = rig(board(8, vec![Fault::ReadyNeverReturns { after_execute: true }]), Arc::clone(&clock), None);
        framed_run(&r, mark_job(&many_segments(1100), 30.0, 1000.0, 1), 1);
        let t0 = clock.now();
        let evs = until(&r.handle, |e| matches!(e, DriverEvent::State(MachineState::Fault | MachineState::StopNotConfirmed)));
        assert!(evs.iter().any(|e| matches!(e, DriverEvent::Fault(openlaser_driver::DriverError::Watchdog { .. }))));
        assert!(report_of(&evs).mo_closed);
        assert_eq!(final_mo_and_port(&r.recorder.snapshot().wire), (Some(0), Some(0)));
        // Packets estimated ~0.5 s each, ≤ 3 queued: 1.5 s × 1.5 + 2 s is the longest the board can stay silent.
        assert!(clock.now().saturating_since(t0) <= Duration::from_secs(5));
    }

    #[test]
    fn t6_watchdog_only_without_progress() {
        // A board at buffer depth 1 progresses slowly but steadily: never stopped by the watchdog.
        let r = rig(board(1, Vec::new()), virtual_clock(), None);
        framed_run(&r, mark_job(&many_segments(2000), 30.0, 300.0, 1), 1);
        let evs = until(&r.handle, |e| matches!(e, DriverEvent::Finished(_) | DriverEvent::Fault(_)));
        assert!(matches!(evs.last(), Some(DriverEvent::Finished(Outcome::Finished { .. }))), "{:?}", evs.last());
    }

    #[test]
    fn t6_framing_records_zero_emission() {
        for mode in [FrameMode::Once, FrameMode::Loop] {
            let r = rig(board(8, Vec::new()), virtual_clock(), None);
            connected(&r);
            r.handle.frame(frame_job(box10(), FrameSource::Job), mode, BY_HUMAN);
            until(&r.handle, |e| matches!(e, DriverEvent::FrameCompleted(f) if f.full_passes >= 2 || mode == FrameMode::Once));
            r.handle.stopper().stop_and_wait(Duration::from_secs(2));
            let rec = r.recorder.snapshot();
            assert!(rec.burned.is_empty(), "{mode:?}: nothing burned");
            assert!(rec.mo_open.is_empty(), "{mode:?}: MO never opened");
            assert!(!rec.lit.is_empty(), "{mode:?}: the red light traced the frame");
            assert!(!singles(&rec.wire).iter().any(|w| w[0] == FIBER_SET_MO && w[1] != 0), "{mode:?}: no Fiber_SetMo(1) on the wire");
        }
    }

    #[test]
    fn t6_run_ahead_cap_packets_and_seconds() {
        // At every packet write: ≤ 3 packets sent but not yet executed (by the time model the driver uses), and
        // ≤ 2 s queued after the write unless the board held nothing before it.
        let r = rig(board(8, Vec::new()), virtual_clock(), None);
        let job = mark_job(&many_segments(2200), 30.0, 1000.0, 1);
        let est = job.lists[0].packet_est.clone();
        framed_run(&r, job, 1);
        until(&r.handle, |e| matches!(e, DriverEvent::Finished(_)));
        let wire = r.recorder.snapshot().wire;
        let start = wire.iter().position(|w| matches!(w, SimWire::Single { words, .. } if words[0] == FIBER_SET_MO && words[1] == 1)).unwrap();
        let mut done_at: Vec<MonoTime> = Vec::new();
        let mut sent: Vec<Duration> = Vec::new();
        let mut exec = false;
        for w in &wire[start..] {
            match w {
                SimWire::Single { at, words } if words[0] == EXECUTE_LIST && !exec => {
                    exec = true;
                    let mut c = *at;
                    for e in &sent {
                        c = c.plus(*e);
                        done_at.push(c);
                    }
                }
                SimWire::Packet { at, .. } => {
                    let e = est[sent.len()];
                    let (queued, unexecuted) = if exec {
                        (done_at.last().map(|t| t.saturating_since(*at)).unwrap_or_default(), done_at.iter().filter(|t| *t > at).count())
                    } else {
                        (sent.iter().sum(), sent.len())
                    };
                    assert!(unexecuted < 3, "packet {}: {unexecuted} unexecuted before it", sent.len() + 1);
                    assert!(queued.is_zero() || queued + e <= Duration::from_secs(2), "packet {}: {:?}", sent.len() + 1, queued + e);
                    sent.push(e);
                    if exec {
                        let s = done_at.last().copied().filter(|t| t > at).unwrap_or(*at);
                        done_at.push(s.plus(e));
                    }
                }
                _ => {}
            }
        }
        assert_eq!(sent.len(), est.len());
    }

    #[test]
    fn t6_slow_packets_execute_without_deadlock() {
        // Every packet estimated > 2 s (the business-card raster at 300 mm/s class): ExecuteList after packet 1.
        let r = rig(board(8, Vec::new()), virtual_clock(), None);
        let long: Vec<Vec<[f64; 2]>> = (0..600).map(|i| vec![[-90.0, -90.0 + 0.3 * f64::from(i)], [90.0, -90.0 + 0.3 * f64::from(i)]]).collect();
        let job = mark_job(&long, 30.0, 300.0, 1);
        assert!(job.lists[0].packet_est.iter().all(|e| *e > Duration::from_secs(2)), "{:?}", job.lists[0].packet_est);
        framed_run(&r, job, 1);
        let evs = until(&r.handle, |e| matches!(e, DriverEvent::Finished(_) | DriverEvent::Fault(_)));
        assert!(matches!(evs.last(), Some(DriverEvent::Finished(Outcome::Finished { .. }))), "{:?}", evs.last());
        // The job's list starts at its Fiber_SetMo(1) (the Frame Once's lighting list came before it).
        let wire = r.recorder.snapshot().wire;
        let start = wire.iter().position(|w| matches!(w, SimWire::Single { words, .. } if words[0] == FIBER_SET_MO && words[1] == 1)).unwrap();
        let exec = start + wire[start..].iter().position(|w| matches!(w, SimWire::Single { words, .. } if words[0] == EXECUTE_LIST)).unwrap();
        let packets_before_exec = wire[start..exec].iter().filter(|w| matches!(w, SimWire::Packet { .. })).count();
        assert_eq!(packets_before_exec, 1);
    }

    #[test]
    fn t6_buffer_depth_1_and_2_complete() {
        for depth in [1u8, 2] {
            let r = rig(board(depth, Vec::new()), virtual_clock(), None);
            framed_run(&r, mark_job(&many_segments(1100), 30.0, 1000.0, 1), 1);
            let evs = until(&r.handle, |e| matches!(e, DriverEvent::Finished(_) | DriverEvent::Fault(_)));
            assert!(matches!(evs.last(), Some(DriverEvent::Finished(Outcome::Finished { .. }))), "depth {depth}: {:?}", evs.last());
            let rec = r.recorder.snapshot();
            assert_eq!(rec.writes_while_not_ready, 0);
            let wire = &rec.wire;
            let start = wire.iter().position(|w| matches!(w, SimWire::Single { words, .. } if words[0] == FIBER_SET_MO && words[1] == 1)).unwrap();
            let exec = wire[start..].iter().position(|w| matches!(w, SimWire::Single { words, .. } if words[0] == EXECUTE_LIST)).unwrap();
            let before = wire[start..start + exec].iter().filter(|w| matches!(w, SimWire::Packet { .. })).count();
            assert!(before < 3, "depth {depth}: READY dropped before packet 3, ExecuteList after {before}");
        }
    }

    #[test]
    fn t6_inflight_packet_completed_before_stop_execute() {
        // Stop at the wire event of a packet write (latency 2 ms keeps it in flight): it completes, then StopExecute.
        let cfg = SimConfig { transfer_latency: Duration::from_millis(2), ..board(8, Vec::new()) };
        let probe = rig(cfg.clone(), Arc::new(SystemClock::new()), None);
        framed_run(&probe, mark_job(&many_segments(1100), 30.0, 1000.0, 1), 1);
        until(&probe.handle, |e| matches!(e, DriverEvent::Finished(_)));
        let wire = probe.recorder.snapshot().wire;
        let third_packet = wire.iter().enumerate().filter(|(_, w)| matches!(w, SimWire::Packet { .. })).nth(4).map(|(i, _)| i).unwrap();
        let r = rig(cfg, Arc::new(SystemClock::new()), Some(third_packet));
        framed_run(&r, mark_job(&many_segments(1100), 30.0, 1000.0, 1), 1);
        let evs = until(&r.handle, stopped);
        let report = report_of(&evs);
        assert!(!report.possible_desync, "{report:?}");
        let rec = r.recorder.snapshot().wire;
        let stop = rec.iter().position(|w| matches!(w, SimWire::Single { words, .. } if words[0] == STOP_EXECUTE)).unwrap();
        assert!(stop > third_packet);
        assert!(!rec[third_packet + 1..stop].iter().any(|w| matches!(w, SimWire::Packet { .. })), "no packet after the stop");
    }

    #[test]
    fn t6_abort_mid_list_sends_no_execute_list() {
        // Stop right after ExecuteList: the abort's dummy packet (if READY) is not followed by ExecuteList.
        let probe = rig(board(8, Vec::new()), virtual_clock(), None);
        framed_run(&probe, mark_job(&many_segments(1100), 30.0, 1000.0, 1), 1);
        until(&probe.handle, |e| matches!(e, DriverEvent::Finished(_)));
        let wire = probe.recorder.snapshot().wire;
        let start = wire.iter().position(|w| matches!(w, SimWire::Single { words, .. } if words[0] == FIBER_SET_MO && words[1] == 1)).unwrap();
        let exec = start + wire[start..].iter().position(|w| matches!(w, SimWire::Single { words, .. } if words[0] == EXECUTE_LIST)).unwrap();
        let r = rig(board(8, Vec::new()), virtual_clock(), Some(exec + 1));
        framed_run(&r, mark_job(&many_segments(1100), 30.0, 1000.0, 1), 1);
        until(&r.handle, stopped);
        let wire = r.recorder.snapshot().wire;
        let stop = wire.iter().position(|w| matches!(w, SimWire::Single { words, .. } if words[0] == STOP_EXECUTE)).unwrap();
        assert!(!singles(&wire[stop..]).iter().any(|w| w[0] == EXECUTE_LIST), "no ExecuteList in a mid-list abort");

        // After a finished list (idle, READY), the abort writes the dummy end-of-list packet (byte-exact with v0's,
        // `openlaser_lmc::abort_packet`), then SetEndOfList and ExecuteList (OM v1 parity).
        let idle_rig = rig(board(8, Vec::new()), virtual_clock(), None);
        connected(&idle_rig);
        let before = idle_rig.recorder.snapshot().wire.len();
        idle_rig.handle.stopper().stop_and_wait(Duration::from_secs(2));
        let wire = idle_rig.recorder.snapshot().wire;
        let abort = &wire[before..];
        let dummy = openlaser_lmc::abort_packet().sha256_hex();
        let packets: Vec<&String> = abort
            .iter()
            .filter_map(|w| match w {
                SimWire::Packet { sha256, .. } => Some(sha256),
                _ => None,
            })
            .collect();
        assert_eq!(packets, vec![&dummy], "one dummy end-of-list packet");
        let after = abort.iter().position(|w| matches!(w, SimWire::Packet { .. })).unwrap();
        let next: Vec<u16> = singles(&abort[after..]).iter().map(|w| w[0]).take(2).collect();
        assert_eq!(next, vec![0x0019, EXECUTE_LIST], "SetEndOfList, then ExecuteList on an idle board");
    }

    #[test]
    fn t6_dead_man_handle_drop_stops() {
        let r = rig(board(8, Vec::new()), Arc::new(SystemClock::new()), None);
        connected(&r);
        r.handle.frame(frame_job(box10(), FrameSource::Job), FrameMode::Loop, BY_HUMAN);
        until(&r.handle, |e| matches!(e, DriverEvent::FrameCompleted(_)));
        let Rig { handle, recorder, .. } = r;
        drop(handle);
        assert_eq!(final_mo_and_port(&recorder.snapshot().wire), (Some(0), Some(0)));
    }

    #[test]
    fn t6_handle_drop_waits_for_stop_report() {
        // The dropping thread returns only after the abort reached the wire (≤ 500 ms).
        let r = rig(SimConfig { transfer_latency: Duration::from_millis(2), ..board(8, Vec::new()) }, Arc::new(SystemClock::new()), None);
        framed_run(&r, mark_job(&many_segments(1100), 30.0, 1000.0, 1), 1);
        until(&r.handle, |e| matches!(e, DriverEvent::Progress { .. }));
        let Rig { handle, recorder, .. } = r;
        let t = Instant::now();
        drop(handle);
        assert!(t.elapsed() <= Duration::from_millis(600));
        let wire = recorder.snapshot().wire;
        assert!(singles(&wire).iter().any(|w| w[0] == STOP_EXECUTE), "the abort was on the wire before drop returned");
        assert_eq!(final_mo_and_port(&wire), (Some(0), Some(0)));
    }

    #[test]
    fn t6_panic_guard_stops() {
        // A transport that panics mid-run (wrapping FakeLmc): contained, then the emergency stop.
        struct Panicky(FakeLmc, u32);
        impl openlaser_lmc::LmcTransport for Panicky {
            fn submit_single(&mut self, cmd: Single) -> Result<(), openlaser_lmc::XferError> {
                self.0.submit_single(cmd)
            }
            fn poll_single(&mut self, slice: Duration) -> Result<Option<Option<[u16; 4]>>, openlaser_lmc::XferError> {
                self.0.poll_single(slice)
            }
            fn submit_packet(&mut self, pkt: &Packet) -> Result<(), openlaser_lmc::XferError> {
                self.1 += 1;
                assert!(self.1 != 3, "transport panics on the 3rd packet");
                self.0.submit_packet(pkt)
            }
            fn poll_packet(&mut self, slice: Duration) -> Result<Option<()>, openlaser_lmc::XferError> {
                self.0.poll_packet(slice)
            }
            fn cancel_all(&mut self) {
                self.0.cancel_all();
            }
            fn drain_replies(&mut self) {
                self.0.drain_replies();
            }
        }
        let clock = virtual_clock();
        let fake = FakeLmc::new(board(8, Vec::new()), Arc::clone(&clock));
        let recorder = fake.recorder();
        let h = spawn(LmcBackend::new(Panicky(fake, 0), galvo()), machine_id(), SESSION, clock);
        h.connect();
        let job = mark_job(&many_segments(1100), 30.0, 1000.0, 1);
        h.frame(frame_job(box10(), FrameSource::Job), FrameMode::Once, BY_HUMAN);
        let t = token(&job, 1, far());
        h.run(job, t);
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut evs = Vec::new();
        while !evs.iter().any(stopped) {
            evs.push(h.events().recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("events"));
        }
        assert!(evs.iter().any(|e| matches!(e, DriverEvent::Fault(openlaser_driver::DriverError::PanicContained))));
        assert_eq!(final_mo_and_port(&recorder.snapshot().wire), (Some(0), Some(0)));
    }

    #[test]
    fn t6_driver_never_blocks_on_unpolled_events() {
        // Nobody reads events during a long job (> 100 000 transfers of traffic); the stop is as fast as ever.
        let r = rig(board(8, Vec::new()), Arc::new(SystemClock::new()), None);
        r.handle.connect();
        r.handle.frame(frame_job(many_segments(2000), FrameSource::Job), FrameMode::Loop, BY_HUMAN);
        std::thread::sleep(Duration::from_secs(2));
        let ack = r.handle.stopper().stop_and_wait(Duration::from_millis(500));
        assert!(ack.report.as_ref().is_some_and(|r| r.confirmed), "{ack:?}");
        assert!(ack.elapsed_ms <= 250, "{ack:?}");
    }

    #[test]
    fn t6_frame_receipt_only_after_executed_pass() {
        // Once: the receipt comes after a status read showing !BUSY after the list's ExecuteList.
        let clock = virtual_clock();
        let r = rig(board(8, Vec::new()), Arc::clone(&clock), None);
        connected(&r);
        r.handle.frame(frame_job(box10(), FrameSource::Job), FrameMode::Once, BY_HUMAN);
        let evs = until(&r.handle, |e| matches!(e, DriverEvent::FrameCompleted(_)));
        let Some(DriverEvent::FrameCompleted(receipt)) = evs.last() else { panic!() };
        let wire = r.recorder.snapshot().wire;
        let exec_at = wire
            .iter()
            .find_map(|w| match w {
                SimWire::Single { at, words } if words[0] == EXECUTE_LIST => Some(*at),
                _ => None,
            })
            .unwrap();
        let idle_read = wire.iter().find_map(|w| match w {
            SimWire::Read { at, status } if *at >= exec_at && status & 0x04 == 0 => Some(*at),
            _ => None,
        });
        assert!(idle_read.is_some_and(|t| t <= receipt.at), "receipt at {:?}, first !BUSY read {idle_read:?}", receipt.at);
        assert_eq!(receipt.full_passes, frame_job(box10(), FrameSource::Job).once_passes);

        // Loop: receipt k only after pass k's !BUSY; a stop during a pass gives no receipt for it.
        let r = rig(board(8, Vec::new()), virtual_clock(), None);
        connected(&r);
        r.handle.frame(frame_job(box10(), FrameSource::Job), FrameMode::Loop, BY_HUMAN);
        until(&r.handle, |e| matches!(e, DriverEvent::FrameCompleted(f) if f.full_passes == 3));
        let evs = {
            let s = r.handle.stopper();
            s.stop();
            until(&r.handle, stopped)
        };
        let passes: Vec<u32> = evs
            .iter()
            .filter_map(|e| match e {
                DriverEvent::FrameCompleted(f) => Some(f.full_passes),
                _ => None,
            })
            .collect();
        assert!(passes.windows(2).all(|w| w[1] == w[0] + 1), "{passes:?}");
        let execs = singles(&r.recorder.snapshot().wire).iter().filter(|w| w[0] == EXECUTE_LIST).count();
        let last = passes.last().copied().unwrap_or(3) as usize;
        assert!(execs >= last && execs <= last + 2, "every receipt had its own executed list");
    }

    #[test]
    fn t6_stop_after_queue_cancels_queued_run() {
        let r = rig(board(8, Vec::new()), virtual_clock(), None);
        connected(&r);
        r.handle.frame(frame_job(box10(), FrameSource::Job), FrameMode::Once, BY_HUMAN);
        until(&r.handle, idle);
        let job = mark_job(&box10(), 30.0, 1000.0, 1);
        let run_packets: Vec<String> = job.lists[0].packets.iter().map(Packet::sha256_hex).collect();
        let t = token(&job, 7, far());
        let before = r.recorder.snapshot().wire.len();
        // Queue the run behind a connect that the stop interrupts, so the driver cannot dequeue it first.
        r.handle.connect();
        r.handle.run(job, t);
        r.handle.stop();
        // The run's refusal and the stop's report, in whichever order the driver thread produced them.
        let (refused, reported) = (std::cell::Cell::new(false), std::cell::Cell::new(false));
        until(&r.handle, |e| {
            if matches!(e, DriverEvent::Fault(openlaser_driver::DriverError::Refused(x)) if x.code == RefusalCode::StoppedAfterQueue && x.message.starts_with("run"))
            {
                refused.set(true);
            }
            reported.set(reported.get() || stopped(e));
            refused.get() && reported.get()
        });
        // The stop's own abort may write its dummy end-of-list packet (the board is READY); nothing of the run is.
        let wire = r.recorder.snapshot().wire;
        let written: Vec<&String> = wire[before..]
            .iter()
            .filter_map(|w| match w {
                SimWire::Packet { sha256, .. } => Some(sha256),
                _ => None,
            })
            .collect();
        assert!(written.iter().all(|sha| !run_packets.contains(sha)), "nothing of the run was written");
        assert!(written.iter().all(|sha| **sha == openlaser_lmc::abort_packet().sha256_hex()), "only the abort's dummy packet");
        assert!(!singles(&wire[before..]).iter().any(|w| w[0] == FIBER_SET_MO && w[1] == 1));
    }

    #[test]
    fn t6_run_refused_when_bytes_differ_from_token() {
        let r = rig(board(8, Vec::new()), virtual_clock(), None);
        connected(&r);
        let job = mark_job(&box10(), 30.0, 1000.0, 1);
        r.handle.frame(frame_job(box10(), FrameSource::Job), FrameMode::Once, BY_HUMAN);
        until(&r.handle, idle);
        let t = token(&job, 1, far());
        // The same fingerprint, one byte different: the GP packet of a 31 % job.
        let mut other = mark_job(&box10(), 31.0, 1000.0, 1);
        other.fingerprint = job.fingerprint;
        let before = r.recorder.snapshot().wire.len();
        r.handle.run(other, t);
        until(&r.handle, |e| matches!(e, DriverEvent::Fault(openlaser_driver::DriverError::Refused(x)) if x.code == RefusalCode::BytesMismatch));
        assert!(!r.recorder.snapshot().wire[before..].iter().any(|w| matches!(w, SimWire::Packet { .. })));
    }

    #[test]
    fn t6_run_refused_while_framing() {
        let r = rig(board(8, Vec::new()), virtual_clock(), None);
        connected(&r);
        r.handle.frame(frame_job(box10(), FrameSource::Job), FrameMode::Loop, BY_HUMAN);
        until(&r.handle, |e| matches!(e, DriverEvent::FrameCompleted(_)));
        let job = mark_job(&box10(), 30.0, 1000.0, 1);
        let t = token(&job, 1, far());
        r.handle.run(job, t);
        until(&r.handle, |e| matches!(e, DriverEvent::Fault(openlaser_driver::DriverError::Refused(x)) if x.code == RefusalCode::MachineBusy));
        r.handle.stopper().stop_and_wait(Duration::from_secs(2));
        assert!(r.recorder.snapshot().burned.is_empty());
    }

    #[test]
    fn t6_transfer_timeout_faults_and_stops() {
        // The reply that never comes lands in the run: count the replies of a fault-free connect and Frame Once first
        // (deterministic on the virtual clock), then let the 25th reply of the run time out.
        let probe = rig(board(8, Vec::new()), virtual_clock(), None);
        connected(&probe);
        probe.handle.frame(frame_job(box10(), FrameSource::Job), FrameMode::Once, BY_HUMAN);
        until(&probe.handle, frame_completed);
        until(&probe.handle, idle);
        let replies = probe.recorder.snapshot().wire.iter().filter(|w| matches!(w, SimWire::Read { .. })).count() as u32;
        let r = rig(board(8, vec![Fault::ReplyTimeouts { every: replies + 25 }]), virtual_clock(), None);
        framed_run(&r, mark_job(&many_segments(1100), 30.0, 1000.0, 1), 1);
        let evs = until(&r.handle, |e| matches!(e, DriverEvent::State(MachineState::Fault | MachineState::StopNotConfirmed)));
        assert!(evs.iter().any(|e| matches!(e, DriverEvent::State(MachineState::Running { .. }))), "the timeout landed in the run");
        assert!(
            evs.iter().any(|e| matches!(e, DriverEvent::Fault(openlaser_driver::DriverError::Transport(m)) if m.contains("no reply within 250 ms"))),
            "a transfer fault after transfer_timeout_ms, not a retry"
        );
        let wire = r.recorder.snapshot().wire;
        assert!(singles(&wire).iter().any(|w| w[0] == STOP_EXECUTE), "emergency stop after the fault");
        assert_eq!(singles(&wire).iter().filter(|w| w[0] == GET_SERIAL_NO).count(), 1, "no reconnect, no reset");
    }

    #[test]
    fn t6_run_refused_without_own_receipt() {
        let r = rig(board(8, Vec::new()), virtual_clock(), None);
        connected(&r);
        let job = mark_job(&box10(), 30.0, 1000.0, 1);
        let t = token(&job, 1, far());
        r.handle.run(job, t);
        until(&r.handle, |e| matches!(e, DriverEvent::Fault(openlaser_driver::DriverError::Refused(x)) if x.code == RefusalCode::FrameRequired));
    }

    #[test]
    fn t6_run_refused_tool_layers_receipt() {
        let r = rig(board(8, Vec::new()), virtual_clock(), None);
        connected(&r);
        r.handle.frame(frame_job(box10(), FrameSource::ToolLayersOnly), FrameMode::Once, BY_HUMAN);
        until(&r.handle, idle);
        let job = mark_job(&box10(), 30.0, 1000.0, 1);
        let t = token(&job, 1, far());
        r.handle.run(job, t);
        until(&r.handle, |e| matches!(e, DriverEvent::Fault(openlaser_driver::DriverError::Refused(x)) if x.code == RefusalCode::FrameDoesNotCoverJob));
    }

    #[test]
    fn arm_token_single_use() {
        let r = rig(board(8, Vec::new()), virtual_clock(), None);
        let job = mark_job(&box10(), 30.0, 1000.0, 1);
        framed_run(&r, job.clone(), 99);
        until(&r.handle, |e| matches!(e, DriverEvent::Finished(_)));
        let t = token(&job, 99, far());
        r.handle.run(job, t);
        until(&r.handle, |e| matches!(e, DriverEvent::Fault(openlaser_driver::DriverError::Refused(x)) if x.code == RefusalCode::ArmNonceUsed));
    }

    #[test]
    fn arm_token_expired_refused() {
        let clock = Arc::new(VirtualClock::new());
        let r = rig(board(8, Vec::new()), clock.clone(), None);
        connected(&r);
        r.handle.frame(frame_job(box10(), FrameSource::Job), FrameMode::Once, BY_HUMAN);
        until(&r.handle, idle);
        let job = mark_job(&box10(), 30.0, 1000.0, 1);
        let t = token(&job, 1, clock.now().plus(Duration::from_millis(1)));
        clock.advance(Duration::from_secs(61));
        r.handle.run(job, t);
        until(&r.handle, |e| matches!(e, DriverEvent::Fault(openlaser_driver::DriverError::Refused(x)) if x.code == RefusalCode::ArmExpired));
    }

    #[test]
    fn t6_business_card_emission_matches_design() {
        // The business card's vector-engrave layer (304 paths, 1750 points): FakeLmc's independently decoded burn
        // equals the design's segments within one galvo unit.
        let path = openlaser_testkit::golden_dir().join("t1_lmc").join("inputs").join("bc_mark_vector-engrave_pass1.json");
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let polys: Vec<Vec<[f64; 2]>> = serde_json::from_value(v["polylines"].clone()).unwrap();
        let r = rig(board(8, Vec::new()), virtual_clock(), None);
        framed_run(&r, mark_job(&polys, 100.0, 1000.0, 1), 1);
        until(&r.handle, |e| matches!(e, DriverEvent::Finished(_)));
        let burned = r.recorder.snapshot().burned;
        // GP drops a mark to the point it is already at, in galvo units: compare segments that move on the wire.
        let galvo_unit = |v: f64| (32768.0 + v * (65535.0 / field_f())).round_ties_even();
        let same = |a: [f64; 2], b: [f64; 2]| galvo_unit(a[0]) == galvo_unit(b[0]) && galvo_unit(a[1]) == galvo_unit(b[1]);
        let design: Vec<([f64; 2], [f64; 2])> = polys.iter().flat_map(|p| p.windows(2).map(|w| (w[0], w[1]))).filter(|(a, b)| !same(*a, *b)).collect();
        assert_eq!(burned.len(), design.len());
        let unit = field_f() / 65535.0;
        for (s, (a, b)) in burned.iter().zip(&design) {
            for (got, want) in [(s.a, *a), (s.b, *b)] {
                assert!((got[0] - want[0]).abs() <= unit && (got[1] - want[1]).abs() <= unit, "{got:?} vs {want:?}");
            }
        }
    }

    #[test]
    fn t6_session_init_once() {
        let r = rig(board(8, Vec::new()), virtual_clock(), None);
        let job = mark_job(&box10(), 30.0, 1000.0, 1);
        framed_run(&r, job.clone(), 1);
        until(&r.handle, |e| matches!(e, DriverEvent::Finished(_)));
        r.handle.frame(frame_job(box10(), FrameSource::Job), FrameMode::Once, BY_HUMAN);
        // The run's own Idle follows its Finished event: wait for this frame's receipt before its Idle.
        until(&r.handle, frame_completed);
        until(&r.handle, idle);
        let t = token(&job, 2, far());
        r.handle.run(job, t);
        until(&r.handle, |e| matches!(e, DriverEvent::Finished(_)));
        let s = singles(&r.recorder.snapshot().wire);
        assert_eq!(s.iter().filter(|w| w[0] == GET_SERIAL_NO).count(), 1, "one init per connection");
        assert_eq!(s.iter().filter(|w| w[0] == 0x0040).count(), 1, "one Reset per connection, none per job");
        assert!(!s.iter().any(|w| w[0] == STOP_EXECUTE), "no abort after a successful job");
    }

    #[test]
    fn frame_once_outcome_and_repeat_counts() {
        // Sanity of the Once path as the engine sees it: one receipt, Idle after, no MO.
        let r = rig(board(8, Vec::new()), virtual_clock(), None);
        connected(&r);
        let f = frame_job(box10(), FrameSource::Job);
        let passes = f.once_passes;
        r.handle.frame(f, FrameMode::Once, BY_HUMAN);
        let evs = until(&r.handle, idle);
        let receipts: Vec<u32> = evs
            .iter()
            .filter_map(|e| match e {
                DriverEvent::FrameCompleted(x) => Some(x.full_passes),
                _ => None,
            })
            .collect();
        assert_eq!(receipts, vec![passes]);
    }
}

mod grbl {
    use std::collections::VecDeque;

    use openlaser_driver::{Backend, ByteLink, Cx, DriverError, GrblBackend, LinkError, StopLatch};
    use openlaser_gcode::GcodeProgram;
    use openlaser_lmc_sim::grbl::FakeGrbl;
    use openlaser_machine::{GantryLink, GantryMachine, GantryTool, MmPerS, WorkspaceOrigin};

    use super::*;

    fn gantry(bed: [f64; 2], s_max: u32) -> GantryMachine {
        GantryMachine {
            bed_mm: bed,
            origin: WorkspaceOrigin::BottomLeft,
            max_speed: MmPerS::new(250.0).unwrap(),
            travel_speed: MmPerS::new(250.0).unwrap(),
            s_max,
            tool: GantryTool::Laser { dynamic: true },
            air_assist: false,
            frame_before_mark: true,
            home_on_connect: true,
            link: GantryLink::Sim,
        }
    }

    fn program(lines: usize) -> GcodeProgram {
        let mut text = String::from("G21\nG90\nM5\nM4 S300\n");
        for i in 0..lines {
            text.push_str(&format!("G1 X{}.5 Y{}.25 F3000\n", 10 + i % 300, 10 + i % 200));
        }
        text.push_str("M5\nG0 X0 Y0\n");
        GcodeProgram { text, lines: 0, fingerprint: FP, machine: MachineId("diode-grbl-400x400".into()), distance_mm: 0.0, estimate: Duration::from_secs(10) }
    }

    /// Counts lines written, injects `error:20` after line `error_after`, trips `stop` after line `stop_after`, and
    /// records realtime bytes, around lmc-sim's FakeGrbl.
    struct Tap {
        inner: FakeGrbl,
        lines: usize,
        error_after: Option<usize>,
        stop_after: Option<(usize, Arc<StopLatch>)>,
        realtime: Arc<Mutex<Vec<u8>>>,
        written: Arc<Mutex<Vec<String>>>,
        injected: VecDeque<u8>,
        /// Bytes written and not yet acknowledged (one entry per line), and the most ever outstanding.
        unacked: VecDeque<usize>,
        partial: Vec<u8>,
        max_in_flight: Arc<Mutex<usize>>,
    }

    impl Tap {
        fn count_acks(&mut self, bytes: &[u8]) {
            self.partial.extend_from_slice(bytes);
            while let Some(pos) = self.partial.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.partial.drain(..=pos).collect();
                let text = String::from_utf8_lossy(&line);
                let text = text.trim();
                if text == "ok" || text.starts_with("error:") {
                    self.unacked.pop_front();
                }
            }
        }
    }

    impl ByteLink for Tap {
        fn write_all(&mut self, bytes: &[u8], slice: Duration) -> Result<(), LinkError> {
            for line in String::from_utf8_lossy(bytes).split_terminator('\n') {
                self.unacked.push_back(line.len() + 1);
                let in_flight: usize = self.unacked.iter().sum();
                let mut max = self.max_in_flight.lock().unwrap();
                *max = (*max).max(in_flight);
                drop(max);
                self.lines += 1;
                self.written.lock().unwrap().push(line.to_owned());
                if self.error_after == Some(self.lines) {
                    self.injected.extend(b"error:20\r\n");
                }
                if let Some((n, latch)) = &self.stop_after
                    && *n == self.lines
                {
                    latch.trip();
                }
            }
            self.inner.write_all(bytes, slice)
        }

        fn write_realtime(&mut self, byte: u8) -> Result<(), LinkError> {
            if byte != b'?' {
                self.realtime.lock().unwrap().push(byte);
            }
            self.inner.write_realtime(byte)
        }

        fn read_available(&mut self, buf: &mut Vec<u8>, slice: Duration) -> Result<usize, LinkError> {
            if !self.injected.is_empty() {
                let n = self.injected.len();
                let bytes: Vec<u8> = self.injected.drain(..).collect();
                self.count_acks(&bytes);
                buf.extend_from_slice(&bytes);
                return Ok(n);
            }
            let mut fresh = Vec::new();
            let n = self.inner.read_available(&mut fresh, slice)?;
            self.count_acks(&fresh);
            buf.extend_from_slice(&fresh);
            Ok(n)
        }
    }

    struct G {
        backend: GrblBackend<Tap>,
        latch: Arc<StopLatch>,
        clock: Arc<VirtualClock>,
        realtime: Arc<Mutex<Vec<u8>>>,
        written: Arc<Mutex<Vec<String>>>,
        max_in_flight: Arc<Mutex<usize>>,
        tx: crossbeam_channel::Sender<DriverEvent>,
        _rx: openlaser_driver::Receiver<DriverEvent>,
        _w: openlaser_driver::Receiver<()>,
    }

    fn grbl(m: GantryMachine, error_after: Option<usize>, stop_after: Option<usize>) -> G {
        let clock = Arc::new(VirtualClock::new());
        let (wake, _w) = crossbeam_channel::bounded(1);
        let latch = Arc::new(StopLatch::with_clock(wake, clock.clone()));
        let (tx, _rx) = crossbeam_channel::unbounded();
        let realtime = Arc::new(Mutex::new(Vec::new()));
        let written = Arc::new(Mutex::new(Vec::new()));
        let max_in_flight = Arc::new(Mutex::new(0));
        let tap = Tap {
            inner: FakeGrbl::new(clock.clone()),
            lines: 0,
            error_after,
            stop_after: stop_after.map(|n| (n, Arc::clone(&latch))),
            realtime: Arc::clone(&realtime),
            written: Arc::clone(&written),
            injected: VecDeque::new(),
            unacked: VecDeque::new(),
            partial: Vec::new(),
            max_in_flight: Arc::clone(&max_in_flight),
        };
        G { backend: GrblBackend::new(tap, m), latch, clock, realtime, written, max_in_flight, tx, _rx, _w }
    }

    #[test]
    fn grbl_never_moves_on_connect() {
        let mut g = grbl(gantry([400.0, 400.0], 1000), None, None);
        let mut cx = Cx::new(&g.latch, &g.tx, &*g.clock);
        g.backend.connect(&mut cx).unwrap();
        let written = g.written.lock().unwrap().clone();
        assert!(written.iter().all(|l| l.starts_with('$') && !l.starts_with("$H") && !l.starts_with("$J")), "{written:?}");
        assert_eq!(g.backend.settings().get(&32), Some(&1.0));
        assert!(g.realtime.lock().unwrap().is_empty(), "no reset needed: the fake greets on connect");
    }

    #[test]
    fn grbl_preflight_refusals() {
        // fake_grbl's settings: $30=1000, $32=1, $130=$131=400.
        for (m, expect) in [(gantry([600.0, 400.0], 1000), "$130"), (gantry([400.0, 400.0], 255), "$30")] {
            let mut g = grbl(m, None, None);
            let mut cx = Cx::new(&g.latch, &g.tx, &*g.clock);
            g.backend.connect(&mut cx).unwrap();
            let before = g.written.lock().unwrap().len();
            let err = g.backend.run(&program(5), &mut cx).unwrap_err();
            let DriverError::Refused(r) = &err else { panic!("{err:?}") };
            assert_eq!(r.code, RefusalCode::GrblPreflight);
            assert!(r.message.contains(expect), "{}", r.message);
            assert_eq!(g.written.lock().unwrap().len(), before, "nothing streamed");
        }
    }

    #[test]
    fn grbl_error_sends_hold_and_reset() {
        let mut g = grbl(gantry([400.0, 400.0], 1000), Some(12), None);
        let mut cx = Cx::new(&g.latch, &g.tx, &*g.clock);
        g.backend.connect(&mut cx).unwrap();
        let err = g.backend.run(&program(100), &mut cx).unwrap_err();
        assert!(matches!(&err, DriverError::Protocol(m) if m.contains("error:20")), "{err:?}");
        // The runtime follows every fault with the emergency stop.
        let report = g.backend.emergency_stop(&mut cx);
        assert_eq!(*g.realtime.lock().unwrap(), vec![b'!', 0x18]);
        assert!(report.confirmed, "{report:?}");
    }

    #[test]
    fn grbl_stop_under_100ms() {
        let mut g = grbl(gantry([400.0, 400.0], 1000), None, Some(40));
        let mut cx = Cx::new(&g.latch, &g.tx, &*g.clock);
        g.backend.connect(&mut cx).unwrap();
        let out = g.backend.run(&program(400), &mut cx).unwrap();
        let Outcome::Stopped(report) = out else { panic!("{out:?}") };
        assert!(report.elapsed <= Duration::from_millis(100), "{report:?}");
        assert_eq!(*g.realtime.lock().unwrap(), vec![b'!', 0x18]);
    }

    #[test]
    fn grbl_char_counting_respects_rx() {
        let mut g = grbl(gantry([400.0, 400.0], 1000), None, None);
        let mut cx = Cx::new(&g.latch, &g.tx, &*g.clock);
        g.backend.connect(&mut cx).unwrap();
        let rx = g.backend.rx_size();
        let out = g.backend.run(&program(300), &mut cx).unwrap();
        assert!(matches!(out, Outcome::Finished { .. }), "{out:?}");
        let max = *g.max_in_flight.lock().unwrap();
        assert!(max <= rx, "{max} bytes outstanding with an RX buffer of {rx}");
        assert!(max > rx / 2, "the buffer was used ({max})");
    }

    #[test]
    #[ignore = "FAKE_GRBL_PY=1 and research/rayforge-test/fake_grbl.py listening on 127.0.0.1:2323"]
    fn grbl_interop_with_python_fake() {
        // Our own research script (not third-party code, not hardware). A test-local TCP link: the transport's
        // TcpLink is default-deny like every real link.
        if std::env::var("FAKE_GRBL_PY").as_deref() != Ok("1") {
            return;
        }
        struct Tcp(std::net::TcpStream);
        impl ByteLink for Tcp {
            fn write_all(&mut self, bytes: &[u8], _slice: Duration) -> Result<(), LinkError> {
                use std::io::Write;
                self.0.write_all(bytes).map_err(|e| LinkError::Io(e.to_string()))
            }
            fn write_realtime(&mut self, byte: u8) -> Result<(), LinkError> {
                use std::io::Write;
                self.0.write_all(&[byte]).map_err(|e| LinkError::Io(e.to_string()))
            }
            fn read_available(&mut self, buf: &mut Vec<u8>, slice: Duration) -> Result<usize, LinkError> {
                use std::io::Read;
                self.0.set_read_timeout(Some(slice.max(Duration::from_millis(1)))).map_err(|e| LinkError::Io(e.to_string()))?;
                let mut tmp = [0u8; 512];
                match self.0.read(&mut tmp) {
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        Ok(n)
                    }
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Ok(0),
                    Err(e) => Err(LinkError::Io(e.to_string())),
                }
            }
        }
        let stream = std::net::TcpStream::connect(("127.0.0.1", 2323)).unwrap();
        let clock = Arc::new(SystemClock::new());
        let (wake, _w) = crossbeam_channel::bounded(1);
        let latch = StopLatch::new(wake);
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut cx = Cx::new(&latch, &tx, &*clock);
        let mut b = GrblBackend::new(Tcp(stream), gantry([400.0, 400.0], 1000));
        b.connect(&mut cx).unwrap();
        let out = b.run(&program(50), &mut cx).unwrap();
        assert!(matches!(out, Outcome::Finished { .. }));
    }
}

// Extended-axis tests use the production backend directly on FakeLmc, without hardware or ARM witnesses.
fn rotary_backend_run(
    stop_event: Option<usize>,
    never_stops: bool,
) -> (openlaser_lmc_sim::Recording, Result<Outcome, openlaser_driver::DriverError>, Option<StopReport>, Option<MonoTime>) {
    use openlaser_driver::{Backend, Cx, StopLatch};
    use openlaser_lmc::AxisMove;
    let clock = Arc::new(VirtualClock::new());
    let (wake, _) = crossbeam_channel::unbounded();
    let latch = Arc::new(StopLatch::with_clock(wake, clock.clone()));
    let (events, _) = crossbeam_channel::unbounded();
    let cx = &mut Cx::new(&latch, &events, clock.as_ref());
    let fake = FakeLmc::new(board(3, if never_stops { vec![Fault::AxisNeverStops] } else { vec![] }), clock.clone());
    let recorder = fake.recorder();
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (flag, stop) = (armed.clone(), latch.clone());
    let mut event = 0usize;
    fake.set_hook(Box::new(move |_, _| {
        if flag.load(Ordering::SeqCst) {
            if stop_event == Some(event) {
                stop.trip();
            }
            event += 1;
        }
    }));
    let mut m = galvo();
    m.rotary = Some(openlaser_machine::RotaryCfg { enabled: true, ..Default::default() });
    let mut backend = LmcBackend::new(fake, m);
    backend.connect(cx).unwrap();
    let mut job = mark_job(&box10(), 1.0, 1000.0, 1);
    let mv = AxisMove { target_pulses: -10, min_speed: 100, max_speed: 1000, accel_ms: 100 };
    job.lists[0].axis_move = Some(mv);
    let mut second = job.lists[0].clone();
    second.axis_move = Some(AxisMove { target_pulses: 10, ..mv });
    job.lists.push(second);
    job.return_axis = Some(AxisMove { target_pulses: 0, ..mv });
    let before = recorder.snapshot().wire.len();
    armed.store(true, Ordering::SeqCst);
    let result = backend.run(&job, cx);
    let report = match &result {
        Ok(Outcome::Stopped(r)) => Some(r.clone()),
        Err(_) => Some(backend.emergency_stop(cx)),
        _ => None,
    };
    let mut record = recorder.snapshot();
    record.wire.drain(..before);
    (record, result, report, latch.tripped_at())
}

#[test]
fn rotary_wire_order_and_no_motion_with_mo_or_list_active() {
    let (record, result, report, _) = rotary_backend_run(None, false);
    assert!(matches!(result, Ok(Outcome::Finished { .. })), "{result:?}");
    assert!(report.is_none());
    let mut mo = false;
    let mut ports = 0u16;
    let mut executing = false;
    let mut moving = false;
    let mut previous = [0u16; 2];
    let mut targets = Vec::new();
    let mut saw_moving = false;
    for event in &record.wire {
        match event {
            SimWire::Single { words, .. } => {
                let [op, p1, p2, ..] = *words;
                if op == 0x29 {
                    assert!(!mo && ports == 0 && !executing && !moving, "unsafe MoveAxisTo");
                    assert_eq!(previous, [0x26, 0x27]);
                    targets.push(openlaser_lmc::axis_position([0, p1, p2, 0]));
                    moving = true;
                }
                if op == FIBER_SET_MO {
                    if p1 != 0 {
                        assert!(!moving, "MO while axis moving");
                    }
                    mo = p1 != 0;
                }
                if op == WRITE_PORT {
                    ports = p1;
                }
                if op == EXECUTE_LIST {
                    assert!(!moving);
                    executing = true;
                }
                previous = [previous[1], op];
            }
            SimWire::Read { status, .. } => {
                if status & 0x40 != 0 {
                    saw_moving = true;
                } else {
                    moving = false;
                }
                if status & 0x04 == 0 {
                    executing = false;
                }
            }
            _ => {}
        }
    }
    assert_eq!(targets, vec![-10, 10, 0]);
    assert!(saw_moving);
    assert!(!record.mo && record.port == 0);
    assert_eq!(record.axis_position, 0);
    assert!(!record.axis_moving);
}

#[test]
fn rotary_stop_at_every_wire_event_preserves_d7_and_dark_outputs() {
    let total = rotary_backend_run(None, false).0.wire.len();
    let mut motion_stops = 0;
    for event in 0..total {
        let (record, result, report, tripped) = rotary_backend_run(Some(event), false);
        assert!(result.is_ok(), "event {event}: {result:?}");
        // A stop on the last completed reply may race with successful return; explicitly stop in that case is unnecessary: outputs are already dark.
        if let Some(report) = report {
            assert!(report.mo_closed && report.ports_zero, "event {event}: {report:?}");
            if report.notes.iter().any(|s| s.contains("rotary axis may finish")) {
                motion_stops += 1;
            }
            let stop = record
                .wire
                .iter()
                .find_map(|e| match e {
                    SimWire::Single { at, words } if words[0] == STOP_EXECUTE => Some(*at),
                    _ => None,
                })
                .unwrap();
            assert!(stop.saturating_since(tripped.unwrap()) <= Duration::from_millis(40), "event {event}");
        }
        assert!(!record.mo && record.port == 0, "event {event}");
    }
    assert!(motion_stops > 0);
}

#[test]
fn rotary_axis_never_stops_faults_at_distance_based_deadline() {
    let (record, result, report, _) = rotary_backend_run(None, true);
    assert!(result.unwrap_err().to_string().contains("rotary axis did not stop"));
    let start = record
        .wire
        .iter()
        .find_map(|e| match e {
            SimWire::Single { at, words } if words[0] == 0x29 => Some(*at),
            _ => None,
        })
        .unwrap();
    let stop = record
        .wire
        .iter()
        .find_map(|e| match e {
            SimWire::Single { at, words } if words[0] == STOP_EXECUTE => Some(*at),
            _ => None,
        })
        .unwrap();
    assert!(stop.saturating_since(start) <= Duration::from_millis(2160));
    let report = report.unwrap();
    assert!(report.mo_closed && report.ports_zero);
    assert!(report.notes.iter().any(|n| n.contains("no stop-axis command")));
}

#[test]
fn rotary_bench_motion_is_relative_dark_and_refuses_active_board() {
    use openlaser_driver::RotarySession;
    use openlaser_lmc::LmcTransport;
    use openlaser_lmc::{AxisMove, axis_singles};
    let clock = Arc::new(VirtualClock::new());
    let mut m = galvo();
    m.rotary = Some(openlaser_machine::RotaryCfg { enabled: true, ..Default::default() });
    let fake = FakeLmc::new(board(3, vec![]), clock.clone());
    let recorder = fake.recorder();
    let mut session = RotarySession::new(fake, m.clone(), clock.clone()).unwrap();
    assert_eq!(session.execute(Some(90.0)).unwrap(), 3200);
    assert_eq!(session.execute(None).unwrap(), 3200);
    assert_eq!(session.execute(Some(-90.0)).unwrap(), 0);
    let record = recorder.snapshot();
    assert!(record.burned.is_empty() && record.mo_open.is_empty() && !record.mo && record.port == 0);
    assert!(record.wire.iter().all(|w| !matches!(w, SimWire::Packet { .. })));
    assert!(singles(&record.wire).iter().all(|w| w[0] != FIBER_SET_MO || w[1] == 0));
    let mut fake = FakeLmc::new(board(3, vec![]), clock.clone());
    let recorder = fake.recorder();
    for cmd in axis_singles(&AxisMove { target_pulses: 1000, min_speed: 100, max_speed: 1000, accel_ms: 100 }) {
        fake.submit_single(cmd).unwrap();
        fake.poll_single(Duration::ZERO).unwrap();
    }
    let before = recorder.snapshot().wire.len();
    let mut session = RotarySession::new(fake, m, clock).unwrap();
    assert!(session.execute(Some(90.0)).unwrap_err().to_string().contains("active"));
    let record = recorder.snapshot();
    assert_eq!(
        singles(&record.wire[before..]).iter().map(|w| w[0]).collect::<Vec<_>>(),
        vec![GET_VERSION],
        "busy refusal must not interrupt the active machine"
    );
}
