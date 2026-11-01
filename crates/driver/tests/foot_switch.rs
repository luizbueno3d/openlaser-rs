//! The foot switch on the driver thread (SPEC M3, design C3.8), on lmc-sim's FakeLmc with a simulated pedal. Real
//! time (the idle input poll waits on the wall clock).

// Test code: clippy.toml allows these inside #[test] fns; the shared helpers of this file need it spelled out.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use openlaser_driver::{DriverEvent, DriverHandle, LmcBackend, MachineState, spawn};
use openlaser_lmc_sim::{FakeLmc, Recorder, SimConfig, SimInputs, SimWire};
use openlaser_machine::{FootSwitchCfg, GalvoMachine, Kinematics};
use openlaser_types::{Clock, MachineId, SessionId, SystemClock};

type Lmc = DriverHandle<LmcBackend<FakeLmc>>;

const READ_PORT: u16 = 0x0025;

fn galvo(foot: Option<FootSwitchCfg>) -> GalvoMachine {
    match openlaser_machine::jpt_m7_200_sim().kind {
        Kinematics::Galvo(mut g) => {
            g.ports.foot_switch = foot;
            g
        }
        Kinematics::Gantry(_) => panic!("the JPT M7 is a galvo"),
    }
}

fn rig(foot: Option<FootSwitchCfg>) -> (Lmc, SimInputs, Arc<Recorder>) {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let cfg = SimConfig { buffer_depth: 3, transfer_latency: Duration::ZERO, status_extra_bits: 0, pid: 0x9899, faults: vec![], emission_cap_pct: None };
    let mut fake = FakeLmc::new(cfg, Arc::clone(&clock));
    let inputs = SimInputs::default();
    fake.set_inputs(inputs.clone());
    let recorder = fake.recorder();
    let handle = spawn(LmcBackend::new(fake, galvo(foot)).with_provider_name("sim"), MachineId("jpt-m7-200-sim".into()), SessionId(1), clock);
    (handle, inputs, recorder)
}

/// Every event that arrives within `d`.
fn events_for(h: &Lmc, d: Duration) -> Vec<DriverEvent> {
    let end = Instant::now() + d;
    let mut out = Vec::new();
    while let Ok(ev) = h.events().recv_timeout(end.saturating_duration_since(Instant::now())) {
        out.push(ev);
    }
    out
}

fn connect(h: &Lmc) {
    h.connect();
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let ev = h.events().recv_timeout(end.saturating_duration_since(Instant::now())).expect("connected");
        if matches!(ev, DriverEvent::State(MachineState::Idle)) {
            return;
        }
    }
}

fn presses(evs: &[DriverEvent]) -> usize {
    evs.iter().filter(|e| matches!(e, DriverEvent::FootSwitch { .. })).count()
}

fn read_ports(r: &Recorder) -> usize {
    r.snapshot().wire.iter().filter(|w| matches!(w, SimWire::Single { words, .. } if words[0] == READ_PORT)).count()
}

/// Pedal down for `hold`, then up, then events for 300 ms.
fn press(h: &Lmc, inputs: &SimInputs, hold: Duration) -> Vec<DriverEvent> {
    inputs.set_bit(15, true);
    let mut evs = events_for(h, hold);
    inputs.set_bit(15, false);
    evs.extend(events_for(h, Duration::from_millis(300)));
    evs
}

#[test]
fn without_a_foot_switch_the_driver_never_reads_the_inputs() {
    let (h, inputs, rec) = rig(None);
    connect(&h);
    let evs = press(&h, &inputs, Duration::from_millis(200));
    assert_eq!(presses(&evs), 0);
    assert_eq!(read_ports(&rec), 0, "no ReadPort on the wire");
}

#[test]
fn a_full_pulse_reports_one_press_and_nothing_is_sent_but_reads() {
    let (h, inputs, rec) = rig(Some(FootSwitchCfg::new(1, 15)));
    connect(&h);
    let lists_before = rec.snapshot().wire.iter().filter(|w| matches!(w, SimWire::Packet { .. })).count();
    events_for(&h, Duration::from_millis(150));
    let evs = press(&h, &inputs, Duration::from_millis(200));
    assert_eq!(presses(&evs), 1, "{evs:?}");
    assert!(evs.iter().any(|e| matches!(e, DriverEvent::Inputs { pedal_down: true, words } if words[1] == 0x8000)));
    assert!(evs.iter().any(|e| matches!(e, DriverEvent::Inputs { pedal_down: false, .. })));
    // The press only reports: no list, no state change, MO closed, ports at 0.
    assert!(!evs.iter().any(|e| matches!(e, DriverEvent::State(_))), "{evs:?}");
    let snap = rec.snapshot();
    assert_eq!(snap.wire.iter().filter(|w| matches!(w, SimWire::Packet { .. })).count(), lists_before);
    assert!(!snap.mo);
    assert_eq!(snap.port, 0);
    assert!(read_ports(&rec) > 5);
}

#[test]
fn a_pedal_held_at_connect_or_through_a_stop_must_be_released_first() {
    let (h, inputs, _) = rig(Some(FootSwitchCfg::new(1, 15)));
    inputs.set_bit(15, true);
    connect(&h);
    events_for(&h, Duration::from_millis(150));
    inputs.set_bit(15, false);
    assert_eq!(presses(&events_for(&h, Duration::from_millis(300))), 0, "the release of a press before connect is not a press");
    // Down, then a stop while it is held: its release does not count either.
    inputs.set_bit(15, true);
    events_for(&h, Duration::from_millis(150));
    h.stop();
    events_for(&h, Duration::from_millis(200));
    inputs.set_bit(15, false);
    assert_eq!(presses(&events_for(&h, Duration::from_millis(300))), 0, "a stop forgets the press in progress");
    // A whole new press counts.
    assert_eq!(presses(&press(&h, &inputs, Duration::from_millis(200))), 1);
}

#[test]
fn a_bounce_shorter_than_the_debounce_is_not_a_press() {
    let (h, inputs, _) = rig(Some(FootSwitchCfg { debounce_ms: 200, ..FootSwitchCfg::new(1, 15) }));
    connect(&h);
    events_for(&h, Duration::from_millis(300));
    assert_eq!(presses(&press(&h, &inputs, Duration::from_millis(60))), 0);
}
