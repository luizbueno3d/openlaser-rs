use super::*;
use openlaser_types::VirtualClock;

fn config() -> SimConfig {
    SimConfig { buffer_depth: 3, transfer_latency: Duration::ZERO, status_extra_bits: 0x0200, pid: 0x9899, faults: vec![], emission_cap_pct: None }
}
fn single(b: &mut FakeLmc, opcode: u16, a: u16) {
    b.submit_single(Single([opcode, a, 0, 0, 0, 0])).unwrap_or_else(|e| panic!("{e}"));
    assert!(b.poll_single(b.cfg.transfer_latency).unwrap_or_else(|e| panic!("{e}")).is_some());
}
fn packet(words: &[[u16; 6]]) -> Packet {
    Packet::from_words(words).unwrap_or_else(|e| panic!("{e}"))
}
fn submit(b: &mut FakeLmc, p: &Packet) {
    b.submit_packet(p).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(b.poll_packet(b.cfg.transfer_latency), Ok(Some(())));
}
fn status(b: &mut FakeLmc) -> openlaser_lmc::StatusWord {
    b.submit_single(Single([op::GET_VERSION, 0, 0, 0, 0, 0])).unwrap_or_else(|e| panic!("{e}"));
    let reply = b.poll_single(b.cfg.transfer_latency).unwrap_or_else(|e| panic!("{e}"));
    openlaser_lmc::StatusWord(reply.unwrap_or(None).unwrap_or([0; 4])[3])
}
fn golden(name: &str) -> Vec<Packet> {
    let s = openlaser_testkit::t1::load("v1", name).unwrap_or_else(|e| panic!("{e}"));
    let [a, b] = s.segments["job"];
    s.events[a..b]
        .iter()
        .filter_map(|e| {
            if let openlaser_testkit::t1::Event::List { sha256, .. } = e {
                Some(Packet::try_from_bytes(&openlaser_testkit::t1::packet(sha256).unwrap_or_else(|e| panic!("{e}"))).unwrap_or_else(|e| panic!("{e}")))
            } else {
                None
            }
        })
        .collect()
}
#[test]
fn fake_records_mark_box10_emission() {
    let clock = Arc::new(VirtualClock::new());
    let mut b = FakeLmc::new(config(), clock.clone());
    single(&mut b, op::FIBER_SET_MO, 1);
    single(&mut b, op::WRITE_PORT, 1);
    for p in golden("mark_box10") {
        submit(&mut b, &p);
    }
    assert!(b.recorder().snapshot().burned.is_empty(), "queued bytes cannot emit");
    single(&mut b, op::EXECUTE_LIST, 0);
    assert!(status(&mut b).busy());
    clock.advance(Duration::from_millis(200));
    assert!(!status(&mut b).busy());
    single(&mut b, op::FIBER_SET_MO, 0);
    let r = b.recorder().snapshot();
    assert_eq!(r.burned.len(), 4);
    for (segment, (a, to)) in
        r.burned.iter().zip([([-5.0, -5.0], [5.0, -5.0]), ([5.0, -5.0], [5.0, 5.0]), ([5.0, 5.0], [-5.0, 5.0]), ([-5.0, 5.0], [-5.0, -5.0])])
    {
        for axis in 0..2 {
            assert!((segment.a[axis] - a[axis]).abs() < 236.68 / 65535.0);
            assert!((segment.b[axis] - to[axis]).abs() < 236.68 / 65535.0);
        }
        assert!((segment.power_pct - 2048.0 * 100.0 / 4095.0).abs() < 1e-10);
    }
    assert_eq!(r.mo_closed_while_busy, 0);
    assert_eq!(r.mo_open.len(), 1);
}
/// Frame emission through the shipping encoder. The `#[cfg(test)]` repeats what `fake.rs` already says for this whole
/// file; `xtask deps` (SPEC §2.3 rule 14) looks for it in the file that names `safety::testing`.
#[cfg(test)]
mod frame_emission {
    use super::*;

    fn sim_machine() -> openlaser_machine::GalvoMachine {
        let openlaser_machine::Kinematics::Galvo(m) = openlaser_machine::jpt_m7_200_sim().kind else { panic!("the sim profile is a galvo") };
        m
    }
    /// Runs one list as the session does (ResetList, GetVersion, packets with SetEndOfList, ExecuteList after the first),
    /// then waits until the board is no longer BUSY.
    fn run_list(b: &mut FakeLmc, clock: &VirtualClock, packets: &[Packet]) {
        single(b, op::RESET_LIST, 0);
        for (i, p) in packets.iter().enumerate() {
            while !status(b).ready() {
                clock.advance(Duration::from_millis(5));
            }
            submit(b, p);
            single(b, op::SET_END_OF_LIST, 0);
            if i == 0 {
                single(b, op::EXECUTE_LIST, 0);
            }
        }
        for _ in 0..100_000 {
            if !status(b).busy() {
                return;
            }
            clock.advance(Duration::from_millis(5));
        }
        panic!("the list never finished");
    }

    /// The shipping frame lists (`encode_frame` on the T1 frame inputs) burn nothing, even on a board whose MO is open and
    /// whose laser bit is set when the frame starts (what a frame after a crashed job would meet), and they never open MO.
    #[test]
    fn fake_frame_records_zero_emission() {
        let m = sim_machine();
        for name in ["frame_box10", "bc_frame"] {
            let s = openlaser_testkit::t1::load("v1", name).unwrap_or_else(|e| panic!("{e}"));
            let paths = openlaser_testkit::t1::polylines(&s).unwrap_or_else(|e| panic!("{e}"));
            let plan = openlaser_safety::testing::frame_plan_unchecked(
                paths.into_iter().map(openlaser_safety::Polyline).collect(),
                openlaser_types::Fingerprint([1; 32]),
                openlaser_types::MachineId("jpt-m7-200-sim".into()),
                openlaser_types::FrameStyle::Contour,
                openlaser_types::FrameSource::Job,
            );
            let job = openlaser_lmc::encode_frame(&plan, &m).unwrap_or_else(|e| panic!("{e}"));
            for mo_already_open in [true, false] {
                let clock = Arc::new(VirtualClock::new());
                let mut b = FakeLmc::new(config(), clock.clone());
                b.set_field_mm(m.field_f());
                if mo_already_open {
                    single(&mut b, op::FIBER_SET_MO, 1);
                    single(&mut b, op::WRITE_PORT, 1);
                }
                let opened_at = b.recorder().snapshot().mo_open_since;
                run_list(&mut b, &clock, &job.once);
                run_list(&mut b, &clock, &job.pass);
                let r = b.recorder().snapshot();
                assert!(r.burned.is_empty(), "{name}: a frame burned {:?}", r.burned.first());
                assert!(!r.lit.is_empty(), "{name}: the red light traced nothing");
                assert_eq!(r.port & 1, 0, "{name}: the frame left the laser bit set");
                assert!(r.mo_open.is_empty(), "{name}: the frame closed or opened MO");
                assert_eq!((r.mo, r.mo_open_since), (mo_already_open, opened_at), "{name}: the frame changed MO");
                assert_eq!(r.writes_while_not_ready, 0);
            }
        }
    }
}

#[test]
fn fake_counts_writes_while_not_ready() {
    let mut b = FakeLmc::new(config(), Arc::new(VirtualClock::new()));
    let p = packet(&[]);
    b.submit_packet(&p).unwrap_or_else(|e| panic!("{e}"));
    assert!(b.submit_packet(&p).is_err());
    assert_eq!(b.recorder().snapshot().writes_while_not_ready, 1);
}
#[test]
fn buffer_depth_ready_busy_and_progress_200_runs() {
    for seed in 0u32..200 {
        let clock = Arc::new(VirtualClock::new());
        let mut cfg = config();
        cfg.buffer_depth = u8::try_from(seed % 8 + 1).unwrap_or(1);
        let depth = cfg.buffer_depth;
        let mut b = FakeLmc::new(cfg, clock.clone());
        let full = packet(&vec![[op::LIST_DELAY_TIME, 10, 0, 0, 0, 0]; 256]);
        for _ in 0..depth {
            assert!(status(&mut b).ready());
            submit(&mut b, &full);
        }
        assert!(!status(&mut b).ready());
        assert!(!status(&mut b).busy());
        single(&mut b, op::EXECUTE_LIST, 0);
        assert!(status(&mut b).busy());
        assert!(!status(&mut b).ready());
        clock.advance(Duration::from_millis(26));
        assert!(status(&mut b).ready());
        submit(&mut b, &packet(&[]));
        clock.advance(Duration::from_secs(2));
        assert!(!status(&mut b).busy());
        assert_eq!(b.recorder().snapshot().writes_while_not_ready, 0);
    }
}
#[test]
fn galvo_time_model_keeps_mo_open_for_twenty_seconds() {
    let clock = Arc::new(VirtualClock::new());
    let mut b = FakeLmc::new(config(), clock.clone());
    single(&mut b, op::FIBER_SET_MO, 1);
    submit(&mut b, &packet(&[[op::LIST_WRITE_PORT, 1, 0, 0, 0, 0], [op::LIST_MARK_SPEED, 1, 0, 0, 0, 0], [op::LIST_MARK_TO, 52768, 32768, 0, 20000, 0]]));
    single(&mut b, op::EXECUTE_LIST, 0);
    clock.advance(Duration::from_secs(19));
    assert!(status(&mut b).busy());
    assert_eq!(b.recorder().snapshot().burned.len(), 1);
    clock.advance(Duration::from_secs(1));
    assert!(!status(&mut b).busy());
    assert_eq!(b.recorder().snapshot().burned.len(), 1);
    single(&mut b, op::FIBER_SET_MO, 0);
    let r = b.recorder().snapshot();
    assert_eq!(r.mo_closed_while_busy, 0);
    assert_eq!(r.mo_open[0].1.0, Duration::from_secs(20));
}
#[test]
fn early_mo_close_is_observable_and_stop_cancels_unexecuted_motion() {
    let clock = Arc::new(VirtualClock::new());
    let mut b = FakeLmc::new(config(), clock.clone());
    single(&mut b, op::FIBER_SET_MO, 1);
    submit(&mut b, &packet(&[[op::LIST_DELAY_TIME, 60000, 0, 0, 0, 0], [op::LIST_MARK_TO, 33000, 32768, 0, 232, 0]]));
    single(&mut b, op::EXECUTE_LIST, 0);
    single(&mut b, op::FIBER_SET_MO, 0);
    assert_eq!(b.recorder().snapshot().mo_closed_while_busy, 1);
    single(&mut b, op::STOP_EXECUTE, 0);
    clock.advance(Duration::from_secs(1));
    assert!(!status(&mut b).busy());
    assert!(b.recorder().snapshot().burned.is_empty());
}
#[test]
fn all_faults_have_distinct_observable_effects() {
    let p = packet(&[]);
    for after_execute in [false, true] {
        let mut cfg = config();
        cfg.faults = vec![Fault::ReadyNeverReturns { after_execute }];
        let mut b = FakeLmc::new(cfg, Arc::new(VirtualClock::new()));
        assert_eq!(status(&mut b).ready(), after_execute);
        single(&mut b, op::EXECUTE_LIST, 0);
        single(&mut b, op::STOP_EXECUTE, 0);
        single(&mut b, op::RESET_LIST, 0);
        assert!(!status(&mut b).ready());
        single(&mut b, op::FIBER_SET_MO, 0);
        single(&mut b, op::WRITE_PORT, 0);
    }
    let mut cfg = config();
    cfg.faults = vec![Fault::BusyStuck];
    let mut b = FakeLmc::new(cfg, Arc::new(VirtualClock::new()));
    assert!(!status(&mut b).busy());
    submit(&mut b, &p);
    single(&mut b, op::EXECUTE_LIST, 0);
    assert!(status(&mut b).busy());
    single(&mut b, op::STOP_EXECUTE, 0);
    assert!(!status(&mut b).busy());
    let mut cfg = config();
    cfg.faults = vec![Fault::StallWrites { after_packets: 1 }];
    let mut b = FakeLmc::new(cfg, Arc::new(VirtualClock::new()));
    submit(&mut b, &p);
    assert_eq!(b.submit_packet(&p), Err(XferError::Stall));
    let mut cfg = config();
    cfg.faults = vec![Fault::DisconnectMidPacket { packet: 1 }];
    let mut b = FakeLmc::new(cfg, Arc::new(VirtualClock::new()));
    assert!(b.submit_packet(&p).is_ok());
    assert_eq!(b.poll_packet(Duration::ZERO), Err(XferError::Disconnected));
    assert!(b.queue.is_empty());
    assert_eq!(b.submit_single(Single([0; 6])), Err(XferError::Disconnected));
    let mut cfg = config();
    cfg.faults = vec![Fault::ReplyTimeouts { every: 2 }];
    let mut b = FakeLmc::new(cfg, Arc::new(VirtualClock::new()));
    single(&mut b, op::GET_VERSION, 0);
    assert!(b.submit_single(Single([op::GET_VERSION, 0, 0, 0, 0, 0])).is_ok());
    assert_eq!(b.poll_single(Duration::from_secs(2)), Ok(None));
    b.drain_replies();
    single(&mut b, op::GET_VERSION, 0);
}
#[test]
fn transfer_latency_hook_and_partial_cancel_model() {
    for partial in [false, true] {
        let clock = Arc::new(VirtualClock::new());
        let mut cfg = config();
        cfg.transfer_latency = Duration::from_millis(20);
        if partial {
            cfg.faults.push(Fault::PartialPacketAfterCancel);
        }
        let mut b = FakeLmc::new(cfg, clock.clone());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let copy = seen.clone();
        b.set_hook(Box::new(move |e, t| copy.lock().unwrap_or_else(|e| e.into_inner()).push((e.clone(), t))));
        let p = packet(&vec![[op::LIST_DELAY_TIME, 100, 0, 0, 0, 0]; 256]);
        assert!(b.submit_packet(&p).is_ok());
        assert_eq!(b.poll_packet(Duration::from_millis(5)), Ok(None));
        assert!(b.queue.is_empty());
        b.cancel_all();
        assert_eq!(b.queue.front().map(|p| p.len()), if partial { Some(128) } else { None });
        assert!(b.recorder().snapshot().burned.is_empty());
        assert_eq!(seen.lock().unwrap_or_else(|e| e.into_inner()).len(), 1);
    }
}

#[test]
fn interrupted_motion_records_only_the_travelled_emission() {
    let clock = Arc::new(VirtualClock::new());
    let mut b = FakeLmc::new(config(), clock.clone());
    single(&mut b, op::FIBER_SET_MO, 1);
    submit(&mut b, &packet(&[[op::LIST_WRITE_PORT, 1, 0, 0, 0, 0], [op::LIST_MARK_SPEED, 1, 0, 0, 0, 0], [op::LIST_MARK_TO, 52768, 32768, 0, 20000, 0]]));
    single(&mut b, op::EXECUTE_LIST, 0);
    clock.advance(Duration::from_secs(5));
    single(&mut b, op::STOP_EXECUTE, 0);
    single(&mut b, op::FIBER_SET_MO, 0);
    let before = b.recorder().snapshot();
    assert_eq!(before.burned.len(), 1);
    assert!((before.burned[0].b[0] - 5000.0 * 236.68 / 65535.0).abs() < 1e-9);
    clock.advance(Duration::from_secs(60));
    assert!(!status(&mut b).busy());
    assert_eq!(b.recorder().snapshot().burned, before.burned);
    assert_eq!(before.mo_closed_while_busy, 0);
}

#[test]
fn offline_simulation_executes_lists_and_repeats_and_reports_malformed_jobs() {
    let openlaser_machine::Kinematics::Galvo(m) = openlaser_machine::jpt_m7_200_sim().kind else {
        return;
    };
    let mut job = LmcJob {
        prelude: vec![Single([op::RESET_LIST, 0, 0, 0, 0, 0]), Single([op::FIBER_SET_MO, 1, 0, 0, 0, 0])],
        lists: vec![openlaser_lmc::LmcList {
            layer: openlaser_types::LayerKey::Color(0),
            pass: 1,
            packets: golden("mark_box10"),
            packet_est: vec![Duration::from_secs(1)],
        }],
        repeats: 2,
        postlude: vec![Single([op::FIBER_SET_MO, 0, 0, 0, 0, 0]), Single([op::WRITE_PORT, 0, 0, 0, 0, 0])],
        fingerprint: openlaser_types::Fingerprint([1; 32]),
        machine: openlaser_types::MachineId("sim".into()),
    };
    let r = simulate_job(&job, &m);
    assert!(r.error.is_none());
    assert_eq!(r.burned.len(), 8);
    assert_eq!(r.mo_open.len(), 2);
    assert_eq!(r.mo_closed_while_busy, 0);
    assert_eq!(r.writes_while_not_ready, 0);
    job.lists[0].packets = vec![packet(&vec![[op::LIST_DELAY_TIME, 1, 0, 0, 0, 0]; 256])];
    let r = simulate_job(&job, &m);
    assert!(r.error.is_some());
    assert!(r.wire.is_empty());
    assert!(r.burned.is_empty());
}

#[test]
fn independent_emission_matches_all_marking_scenario_paths() {
    for name in ["mark_box10", "dot_test", "line_test", "long_job", "bc_mark_vector-engrave_pass1", "bc_mark_raster_pass1"] {
        let clock = Arc::new(VirtualClock::new());
        let mut b = FakeLmc::new(config(), clock.clone());
        single(&mut b, op::FIBER_SET_MO, 1);
        for (i, p) in golden(name).iter().enumerate() {
            while !status(&mut b).ready() {
                clock.advance(Duration::from_millis(5));
            }
            submit(&mut b, p);
            if i == 0 {
                single(&mut b, op::EXECUTE_LIST, 0);
            }
        }
        clock.advance(Duration::from_secs(1000));
        assert!(!status(&mut b).busy());
        single(&mut b, op::FIBER_SET_MO, 0);
        let scenario = openlaser_testkit::t1::load("v1", name).unwrap_or_else(|e| panic!("{e}"));
        let paths = openlaser_testkit::t1::polylines(&scenario).unwrap_or_else(|e| panic!("{e}"));
        let quantize = |p: [f64; 2]| p.map(|v| openlaser_types::galvo::mm_to_galvo(v, scenario.field_size_mm).unwrap_or_else(|e| panic!("{e}")));
        let expected: Vec<_> = paths.iter().flat_map(|p| p.windows(2)).filter(|p| quantize(p[0]) != quantize(p[1])).collect();
        let r = b.recorder().snapshot();
        assert_eq!(r.burned.len(), expected.len(), "{name}");
        for (actual, points) in r.burned.iter().zip(expected) {
            for (actual_point, expected_point) in [actual.a, actual.b].iter().zip(points.iter()) {
                for (a, b) in actual_point.iter().zip(expected_point) {
                    assert!((a - b).abs() < scenario.field_size_mm / 65535.0, "{name} endpoint");
                }
            }
        }
        assert_eq!(r.mo_closed_while_busy, 0);
        assert_eq!(r.writes_while_not_ready, 0);
    }
}

#[test]
fn status_extra_bits_never_claim_ready_or_busy() {
    let mut cfg = config();
    cfg.status_extra_bits = 0xFFFF;
    cfg.faults = vec![Fault::ReadyNeverReturns { after_execute: false }];
    let mut b = FakeLmc::new(cfg, Arc::new(VirtualClock::new()));
    let s = status(&mut b);
    assert!(!s.ready() && !s.busy(), "{:#06x}", s.0);
    assert_eq!(s.0 & 0x0200, 0x0200, "unknown bits are kept");
}

#[test]
fn laser_on_point_is_recorded_as_emission() {
    let clock = Arc::new(VirtualClock::new());
    let mut b = FakeLmc::new(config(), clock.clone());
    b.set_field_mm([200.0, 200.0]);
    single(&mut b, op::FIBER_SET_MO, 1);
    submit(
        &mut b,
        &packet(&[
            [op::LIST_WRITE_PORT, 1, 0, 0, 0, 0],
            [op::LIST_MARK_CURRENT, 4095, 0, 0, 0, 0],
            [op::LIST_JUMP_TO, 32768 + 3277, 32768, 0, 3277, 0],
            [op::LIST_LASER_ON_POINT, 100, 0, 0, 0, 0],
        ]),
    );
    single(&mut b, op::EXECUTE_LIST, 0);
    clock.advance(Duration::from_secs(1));
    assert!(!status(&mut b).busy());
    let r = b.recorder().snapshot();
    assert_eq!(r.burned.len(), 1);
    let at = r.burned[0];
    assert_eq!(at.a, at.b);
    assert!((at.a[0] - 3277.0 * 200.0 / 65535.0).abs() < 1e-9, "{at:?}: reported in the field set by set_field_mm");
    assert_eq!(at.power_pct, 100.0);
}

#[test]
fn an_mo_left_open_and_the_final_port_are_recorded() {
    let clock = Arc::new(VirtualClock::new());
    let mut b = FakeLmc::new(config(), clock.clone());
    let r = b.recorder().snapshot();
    assert_eq!((r.mo, r.mo_open_since, r.port), (false, None, 0));
    clock.advance(Duration::from_secs(3));
    single(&mut b, op::FIBER_SET_MO, 1);
    single(&mut b, op::WRITE_PORT, 0x0101);
    clock.advance(Duration::from_secs(4));
    let r = b.recorder().snapshot();
    assert!(r.mo_open.is_empty(), "the interval has not closed");
    assert_eq!(r.mo_open_since, Some(MonoTime(Duration::from_secs(3))));
    assert_eq!((r.mo, r.port), (true, 0x0101));
    single(&mut b, op::FIBER_SET_MO, 0);
    let r = b.recorder().snapshot();
    assert_eq!(r.mo_open, [(MonoTime(Duration::from_secs(3)), MonoTime(Duration::from_secs(7)))]);
    assert_eq!((r.mo, r.mo_open_since), (false, None));
    single(&mut b, op::RESET, 0);
    assert_eq!(b.recorder().snapshot().port, 0);
}

#[test]
fn polls_return_when_the_transfer_completes_not_after_a_whole_slice() {
    // A USB transfer completes when the board answers; a poll with a 20 ms slice waits only the 2 ms latency (the
    // real-time T6 stop budget counts every abort transfer). A reply that never comes still costs the whole slice.
    let clock = Arc::new(VirtualClock::new());
    let mut cfg = config();
    cfg.transfer_latency = Duration::from_millis(2);
    cfg.faults.push(Fault::ReplyTimeouts { every: 2 });
    let mut b = FakeLmc::new(cfg, clock.clone());
    let slice = Duration::from_millis(20);
    b.submit_single(Single([op::GET_VERSION, 0, 0, 0, 0, 0])).unwrap_or_else(|e| panic!("{e}"));
    let t0 = clock.now();
    assert!(b.poll_single(slice).unwrap_or_else(|e| panic!("{e}")).is_some());
    assert_eq!(clock.now().saturating_since(t0), Duration::from_millis(2));
    b.submit_packet(&packet(&[])).unwrap_or_else(|e| panic!("{e}"));
    let t1 = clock.now();
    assert_eq!(b.poll_packet(slice), Ok(Some(())));
    assert_eq!(clock.now().saturating_since(t1), Duration::from_millis(2));
    // The second reply never comes (ReplyTimeouts every 2): the poll waits the slice and reports nothing.
    b.submit_single(Single([op::GET_VERSION, 0, 0, 0, 0, 0])).unwrap_or_else(|e| panic!("{e}"));
    let t2 = clock.now();
    assert_eq!(b.poll_single(slice), Ok(None));
    assert_eq!(clock.now().saturating_since(t2), slice);
}
