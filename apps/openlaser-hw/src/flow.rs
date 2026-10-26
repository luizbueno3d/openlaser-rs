//! The device flows (SPEC §3.21): `status`, `frame` and `run`.
//!
//! `run`: connect (board lock) → print the decoded summary → frame in a loop → Enter is accepted only after ≥ 1
//! fully EXECUTED pass (S20) → stop framing → without `--allow-emission` stop here (`EmissionNotAllowed`) → print the
//! summary of the PendingArm, the profile checklist and a 6-digit challenge → `ARM <challenge>` → mark. The first
//! emission of a bench session refuses any row above the cap (`BenchPowerCap`, S23) before the challenge. Esc,
//! Space and Ctrl-C stop at once from the key thread; a stop, a fault or the key reader closing ends the flow with
//! nothing (more) marked. A stop key whose StopReport has not arrived 500 ms later prints "STOP NOT CONFIRMED: use
//! the key switch / E-stop" and asks every machine again (SPEC §5.4 watchdog), so the operator never waits in silence.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use openlaser_engine::{ArmToken, FrameMode, FrameReceipt};
use openlaser_types::{Clock, MonoTime, RefusalCode};

use crate::args::{FrameOpts, RunOpts, StatusOpts};
use crate::rig::{Purpose, Rig, RigEvent, RigSpec};
use crate::stop::{self, CountingStop, STOP_WAIT, StopTarget};
use crate::{Env, HwError, KeyPoll, KeySession, StopFn, StopKey, Terminal, plain, require_tty, summary};

/// Signal handlers, the watcher and the panic hook, held for the length of a device command.
struct Guards {
    _signals: Option<stop::Signals>,
    _watcher: Option<stop::Watcher>,
}

fn guards(env: &Env, target: &Arc<dyn StopTarget>) -> Result<Guards, HwError> {
    if !env.signals {
        return Ok(Guards { _signals: None, _watcher: None });
    }
    stop::install_panic_hook(Arc::clone(target));
    let signals = stop::install_signals().map_err(HwError::Failed)?;
    let flags = signals.flags.iter().map(|f| stop::SignalFlag { flag: Arc::clone(&f.flag), name: f.name, exit_code: f.exit_code }).collect();
    // `\r\n`: a signal can land while the key thread holds the terminal in raw mode, where `\n` alone does not
    // return the cursor.
    let sink: stop::ReportSink = Box::new(|line| eprint!("{}\r\n", plain::block(line).replace('\n', "\r\n")));
    let watcher = stop::watch(flags, Arc::clone(target), sink, Arc::clone(&env.exit)).map_err(HwError::Failed)?;
    Ok(Guards { _signals: Some(signals), _watcher: Some(watcher) })
}

/// The stop keys of one frame or mark phase. Each one trips every latch at once, on the key thread; the first one is
/// also timed, so the main loop knows when its StopReport is late (SPEC §5.4 watchdog, [`STOP_WAIT`]).
pub struct KeyStops {
    target: Arc<dyn StopTarget>,
    clock: Arc<dyn Clock>,
    first: Mutex<Option<MonoTime>>,
}

impl KeyStops {
    pub fn new(target: Arc<dyn StopTarget>, clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(KeyStops { target, clock, first: Mutex::new(None) })
    }

    /// What the key thread calls for Esc, Space, Ctrl-C and a lost input. The time is read before the trip, so the
    /// watchdog never starts late; the trip itself never waits.
    pub fn stop_fn(self: &Arc<Self>) -> StopFn {
        let me = Arc::clone(self);
        Arc::new(move |_key: StopKey| {
            let at = me.clock.now();
            me.target.stop_now();
            let mut first = me.first.lock().unwrap_or_else(PoisonError::into_inner);
            if first.is_none() {
                *first = Some(at);
            }
        })
    }

    /// When the first stop key of this phase was pressed.
    pub fn pressed_at(&self) -> Option<MonoTime> {
        *self.first.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A stop key was pressed [`STOP_WAIT`] ago or more (and the caller has not seen its StopReport).
    fn overdue(&self, now: MonoTime) -> bool {
        self.pressed_at().is_some_and(|at| now >= at.plus(STOP_WAIT))
    }

    /// SPEC §5.4 watchdog: no StopReport within 500 ms of the stop key. Says so at once, asks every machine again
    /// (≤ 500 ms), prints each answer; the phase ends in an error either way.
    fn not_reported(&self, term: &mut dyn Terminal) -> HwError {
        term.write(&format!(
            "STOP NOT CONFIRMED: no StopReport within {} ms of the stop key: use the key switch / E-stop.\nAsking every machine to stop again:\n",
            STOP_WAIT.as_millis()
        ));
        let acks = self.target.stop_and_wait(STOP_WAIT);
        print_stop(term, &acks);
        if stop_confirmed(&acks) {
            HwError::Failed(format!("the engine did not report the stop within {} ms; every machine confirmed it when asked again", STOP_WAIT.as_millis()))
        } else {
            HwError::Failed("the stop was not confirmed: use the key switch / E-stop".to_owned())
        }
    }
}

fn print_stop(term: &mut dyn Terminal, acks: &[(openlaser_engine::MachineId, openlaser_engine::StopAck)]) {
    for l in stop::report_lines(acks) {
        term.write(&format!("{l}\n"));
    }
}

fn print_report(term: &mut dyn Terminal, r: &openlaser_engine::StopReport) {
    term.write(&format!(
        "Stopped: {} in {} ms (MO closed: {}, ports at 0: {}{}).\n",
        if r.confirmed { "confirmed" } else { "NOT CONFIRMED: use the key switch / E-stop" },
        r.elapsed.as_millis(),
        r.mo_closed,
        r.ports_zero,
        if r.possible_desync { ", possible desync" } else { "" }
    ));
}

/// `openlaser-hw status`: GetSerialNo + GetVersion, no init (H12).
pub fn status(o: &StatusOpts, term: &mut dyn Terminal, env: &mut Env) -> Result<(), HwError> {
    let mut rig = env.rigs.open(&RigSpec { purpose: Purpose::Status, device: o.device.clone(), extent: None })?;
    let lines = rig.probe()?;
    for l in lines {
        term.write(&format!("{}\n", plain::field(&l, plain::MESSAGE_MAX_CHARS)));
    }
    Ok(())
}

/// How a monitored phase ended.
enum Phase {
    /// `run`'s frame: a receipt that clears the job, accepted by Enter.
    Accepted(FrameReceipt),
    /// Frame Once completed.
    Completed(u32),
    Stopped,
}

/// `openlaser-hw frame`: red light only; never marks.
pub fn frame(o: &FrameOpts, term: &mut dyn Terminal, env: &mut Env) -> Result<(), HwError> {
    require_tty(term, "frame")?;
    let mut rig = env.rigs.open(&RigSpec { purpose: Purpose::Frame, device: o.device.clone(), extent: o.extent })?;
    let target = rig.stopper();
    let _guards = guards(env, &target)?;
    rig.load(&o.job, None, None, None, o.extent)?;
    rig.connect()?;
    let mode = if o.looping { FrameMode::Loop } else { FrameMode::Once };
    let r = frame_phase(rig.as_mut(), term, env, &target, mode, o, false);
    rig.disconnect();
    match r? {
        Phase::Completed(passes) => term.write(&format!("Frame done: {passes} pass(es) executed.\n")),
        Phase::Stopped if o.looping => {}
        Phase::Stopped => return Err(HwError::Failed("the frame was stopped before it completed".to_owned())),
        Phase::Accepted(_) => {}
    }
    Ok(())
}

fn frame_phase(
    rig: &mut dyn Rig,
    term: &mut dyn Terminal,
    env: &Env,
    target: &Arc<dyn StopTarget>,
    mode: FrameMode,
    o: &FrameOpts,
    accept_on_enter: bool,
) -> Result<Phase, HwError> {
    let key_stops = KeyStops::new(Arc::clone(target), Arc::clone(&env.clock));
    let mut keys = term.keys(key_stops.stop_fn()).map_err(|e| HwError::Failed(format!("cannot read keys: {e}")))?;
    let fp = rig.start_frame(mode, o.style, o.source)?;
    term.write(&format!(
        "Framing job {} with the red light ({}). Esc, Space or Ctrl-C stops.{}\n",
        fp.short(),
        if mode == FrameMode::Loop { "loop" } else { "once" },
        if accept_on_enter { " Press Enter once the frame looks right." } else { "" }
    ));
    let mut accepted: Option<FrameReceipt> = None;
    let mut announced_passes = 0u32;
    loop {
        // Keys first: an Enter is judged against the passes seen before it was read, never against a receipt that
        // arrives in the same iteration (S20: the pass must have executed before the person accepts it). After a stop
        // key, Enter accepts nothing.
        match keys.poll() {
            KeyPoll::Enter if accept_on_enter && key_stops.pressed_at().is_none() => match accepted.as_ref().map(|r| summary::receipt_accepts(r, &fp)) {
                Some(Ok(())) => {
                    if let Some(r) = accepted.take() {
                        return Ok(Phase::Accepted(r));
                    }
                }
                Some(Err(refusal)) => term.write(&format!("Enter ignored ({}): {}\n", refusal.code.as_str(), refusal.message)),
                None => term.write(&format!(
                    "Enter ignored ({}): no frame pass has been executed by the board yet; wait for one full pass.\n",
                    RefusalCode::FrameRequired.as_str()
                )),
            },
            KeyPoll::Enter | KeyPoll::Nothing => {}
            KeyPoll::Closed => {
                term.write("The key reader closed: stopping.\n");
                let acks = target.stop_and_wait(STOP_WAIT);
                print_stop(term, &acks);
                return Ok(Phase::Stopped);
            }
        }
        for ev in rig.poll() {
            match ev {
                RigEvent::FrameCompleted(r) => {
                    if r.full_passes > announced_passes {
                        announced_passes = r.full_passes;
                        term.write(&format!("Frame pass {} executed by the board.\n", r.full_passes));
                    }
                    if mode == FrameMode::Once && !accept_on_enter {
                        return Ok(Phase::Completed(r.full_passes));
                    }
                    accepted = Some(r);
                }
                RigEvent::Stopped(rep) => {
                    print_report(term, &rep);
                    return Ok(Phase::Stopped);
                }
                RigEvent::Fault(e) => {
                    // The driver already stops on a fault; stopping again is idempotent and costs nothing.
                    let acks = target.stop_and_wait(STOP_WAIT);
                    print_stop(term, &acks);
                    return Err(HwError::Failed(format!("machine fault: {}", plain::field(&e, plain::MESSAGE_MAX_CHARS))));
                }
                RigEvent::Warning(w) => term.write(&format!("warning: {}\n", plain::field(&w, plain::MESSAGE_MAX_CHARS))),
                RigEvent::Progress { .. } | RigEvent::Finished(_) | RigEvent::ArmVoided(_) => {}
            }
        }
        if key_stops.overdue(env.clock.now()) {
            return Err(key_stops.not_reported(term));
        }
        env.clock.sleep(env.poll);
    }
}

/// Watches a marking job until it finishes (its duration) or stops or faults (an error). `keys` reads the stop keys
/// through `key_stops.stop_fn()`; a stop key whose StopReport is 500 ms late ends it with "STOP NOT CONFIRMED".
pub fn watch_marking(rig: &mut dyn Rig, term: &mut dyn Terminal, env: &Env, key_stops: &KeyStops, keys: &mut dyn KeySession) -> Result<Duration, HwError> {
    let target = &key_stops.target;
    let stopped = || HwError::Failed("the job was stopped before it finished".to_owned());
    let mut last_report = 0u32;
    loop {
        for ev in rig.poll() {
            match ev {
                RigEvent::Progress { packets_acked, total, eta_ms } => {
                    if packets_acked >= last_report.saturating_add(8) || packets_acked == total {
                        last_report = packets_acked;
                        term.write(&format!("Marking: packet {packets_acked}/{total}, about {:.1} s left.\n", Duration::from_millis(eta_ms).as_secs_f64()));
                    }
                }
                RigEvent::Finished(d) => return Ok(d),
                RigEvent::Stopped(rep) => {
                    print_report(term, &rep);
                    return Err(stopped());
                }
                RigEvent::Fault(e) => {
                    let acks = target.stop_and_wait(STOP_WAIT);
                    print_stop(term, &acks);
                    return Err(HwError::Failed(format!("machine fault: {}; the job was stopped", plain::field(&e, plain::MESSAGE_MAX_CHARS))));
                }
                RigEvent::ArmVoided(reason) => term.write(&format!("ARM voided: {}\n", plain::field(&reason, plain::MESSAGE_MAX_CHARS))),
                RigEvent::Warning(w) => term.write(&format!("warning: {}\n", plain::field(&w, plain::MESSAGE_MAX_CHARS))),
                RigEvent::FrameCompleted(_) => {}
            }
        }
        if keys.poll() == KeyPoll::Closed {
            term.write("The key reader closed: stopping.\n");
            let acks = target.stop_and_wait(STOP_WAIT);
            print_stop(term, &acks);
            return Err(stopped());
        }
        if key_stops.overdue(env.clock.now()) {
            return Err(key_stops.not_reported(term));
        }
        env.clock.sleep(env.poll);
    }
}

/// Every machine latched and reported a confirmed stop. No machine, no report within the wait, or an unconfirmed
/// report is not a stop (AGENTS.md §1 rule 3: never proceed after a wait times out).
pub fn stop_confirmed(acks: &[(openlaser_engine::MachineId, openlaser_engine::StopAck)]) -> bool {
    !acks.is_empty() && acks.iter().all(|(_, a)| a.latched && a.report.as_ref().is_some_and(|r| r.confirmed))
}

/// Stops the accepted frame and folds its stop into the engine before anything else happens: a stop the engine
/// saw only after `request_arm` would void the PendingArm. Nothing is armed unless every machine confirmed the stop.
fn stop_framing(rig: &mut dyn Rig, term: &mut dyn Terminal, env: &Env, target: &Arc<dyn StopTarget>) -> Result<(), HwError> {
    let acks = target.stop_and_wait(STOP_WAIT);
    if !stop_confirmed(&acks) {
        print_stop(term, &acks);
        return Err(HwError::Failed(format!("the frame stop was not confirmed within {} ms; nothing was armed or marked", STOP_WAIT.as_millis())));
    }
    // The engine must see the stop too, before `request_arm` (a Stopped it saw later would void the PendingArm). A
    // wait that runs out refuses like any other (AGENTS.md §1 rule 3), even though the driver confirmed the stop.
    let wait = STOP_WAIT.saturating_mul(2);
    let deadline = env.clock.now().plus(wait);
    loop {
        for ev in rig.poll() {
            match ev {
                RigEvent::Stopped(r) if r.confirmed => return Ok(()),
                RigEvent::Stopped(r) => {
                    print_report(term, &r);
                    return Err(HwError::Failed("the frame stop was not confirmed; nothing was armed or marked".to_owned()));
                }
                RigEvent::Fault(e) => {
                    return Err(HwError::Failed(format!("machine fault while stopping the frame: {}", plain::field(&e, plain::MESSAGE_MAX_CHARS))));
                }
                _ => {}
            }
        }
        if env.clock.now() >= deadline {
            return Err(HwError::Failed(format!("the engine did not report the frame stop within {} ms; nothing was armed or marked", wait.as_millis())));
        }
        env.clock.sleep(env.poll);
    }
}

/// Queues the armed job, unless a stop was asked for since `stops_before` (counted by `stops`). The driver clears its
/// latch for a command queued after a trip (SPEC §5.3 generations), so a stop that landed before the run was queued
/// would otherwise be lost: one seen before queueing refuses; one that raced the queueing trips the latch again, after
/// the run exists, which cancels it (`StoppedAfterQueue`) or stops it.
pub fn start_marking(rig: &mut dyn Rig, stops: &CountingStop, stops_before: u64, token: ArmToken) -> Result<(), HwError> {
    if stops.requests() != stops_before {
        drop(token);
        return Err(HwError::Failed("a stop was asked for after the frame was accepted; nothing was marked".to_owned()));
    }
    rig.run_armed(token)?;
    if stops.requests() != stops_before {
        stops.stop_now();
    }
    Ok(())
}

/// `openlaser-hw run`.
pub fn run(o: &RunOpts, term: &mut dyn Terminal, env: &mut Env) -> Result<(), HwError> {
    require_tty(term, "run")?;
    let mut rig = env.rigs.open(&RigSpec { purpose: Purpose::Run, device: o.device.clone(), extent: None })?;
    // Every stop goes through the counter: keys, signals, the panic hook and the flow itself.
    let stops = Arc::new(CountingStop::new(rig.stopper()));
    let target: Arc<dyn StopTarget> = Arc::clone(&stops) as Arc<dyn StopTarget>;
    let _guards = guards(env, &target)?;
    let r = run_inner(rig.as_mut(), o, term, env, &stops);
    rig.disconnect();
    r
}

fn run_inner(rig: &mut dyn Rig, o: &RunOpts, term: &mut dyn Terminal, env: &Env, stops: &Arc<CountingStop>) -> Result<(), HwError> {
    let target_arc: Arc<dyn StopTarget> = Arc::clone(stops) as Arc<dyn StopTarget>;
    let target = &target_arc;
    rig.load(&o.job, o.layers.as_deref(), o.repeats, o.power_pct, None)?;
    let planned = rig.summary()?;
    term.write(&summary::text(&planned));
    rig.connect()?;

    // Frame in a loop until Enter after an executed pass.
    let frame_opts = FrameOpts { job: o.job.clone(), looping: true, style: None, source: None, extent: None, device: o.device.clone() };
    // Any stop while framing ends the run (frame_phase returns Stopped), so on the way to ARM the only stop expected
    // from here is stop_framing's own: counting from before the frame, a key or a signal that raced the acceptance of
    // the frame (pressed just after Enter, handled while the key thread wound down) is still seen.
    let stops_before_frame = stops.requests();
    let receipt = match frame_phase(rig, term, env, target, FrameMode::Loop, &frame_opts, true)? {
        Phase::Accepted(r) => r,
        _ => return Err(HwError::Failed("stopped while framing; nothing was marked".to_owned())),
    };
    stop_framing(rig, term, env, target)?;
    // From here on any stop request, from any thread, means nothing is marked.
    let stops_before_arm = stops_before_frame.saturating_add(1);
    if stops.requests() != stops_before_arm {
        return Err(HwError::Failed("a stop was asked for while the frame was being accepted; nothing was armed or marked".to_owned()));
    }
    term.write(&format!("Frame accepted after {} executed pass(es); framing stopped.\n", receipt.full_passes));
    if !o.allow_emission {
        return Err(HwError::refused(
            RefusalCode::EmissionNotAllowed,
            "framing done and nothing was marked: run again with --allow-emission to mark (red light only without it)",
        ));
    }

    // ARM: the engine's PendingArm (its job is the one that will be sent, S22).
    let arm = rig.request_arm()?;
    term.write("\nAbout to mark. Read every line:\n");
    term.write(&summary::text(&arm.summary));
    summary::check_cap(&arm.summary, rig.emission_cap_pct()?)?;
    if arm.checklist.is_empty() {
        return Err(HwError::refused(RefusalCode::ChecklistIncomplete, "the machine profile has no checklist; nothing can be armed"));
    }
    term.write("Checklist (typing the challenge confirms every item):\n");
    for (i, item) in arm.checklist.iter().enumerate() {
        term.write(&format!("  {}. {}\n", i + 1, plain::field(item, plain::MESSAGE_MAX_CHARS)));
    }
    let challenge = openlaser_arm::tty_nonce()?;
    // Stamped by the terminal once the line is read (the operator may take a while to read the summary).
    let Some(confirm) = term.confirm_arm(challenge, &|| rig.now()) else {
        return Err(HwError::refused(RefusalCode::ArmTextMismatch, "the typed text was not `ARM` and the challenge; nothing was armed"));
    };
    let token = openlaser_arm::mint(&arm.summary, &arm.receipt, confirm, arm.checklist.len(), rig.session(), rig.now())?;

    // Keys first, so a stop works from the first packet.
    let key_stops = KeyStops::new(Arc::clone(target), Arc::clone(&env.clock));
    let mut keys = term.keys(key_stops.stop_fn()).map_err(|e| HwError::Failed(format!("cannot read keys: {e}")))?;
    start_marking(rig, stops, stops_before_arm, token)?;
    term.write("Marking. Esc, Space or Ctrl-C stops.\n");
    let d = watch_marking(rig, term, env, &key_stops, keys.as_mut())?;
    term.write(&format!("Finished in {:.1} s.\n", d.as_secs_f64()));
    Ok(())
}
