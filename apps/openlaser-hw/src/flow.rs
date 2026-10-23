//! The device flows (SPEC §3.21): `status`, `frame` and `run`.
//!
//! `run`: connect (board lock) → print the decoded summary → frame in a loop → Enter is accepted only after ≥ 1
//! fully EXECUTED pass (S20) → stop framing → without `--allow-emission` stop here (`EmissionNotAllowed`) → print the
//! summary of the PendingArm, the profile checklist and a 6-digit challenge → `ARM <challenge>` → mark. The first
//! emission of a bench session refuses any row above the cap (`BenchPowerCap`, S23) before the challenge. Esc,
//! Space and Ctrl-C stop at once from the key thread; a stop, a fault or the key reader closing ends the flow with
//! nothing (more) marked.

use std::sync::Arc;
use std::time::Duration;

use openlaser_engine::{FrameMode, FrameReceipt};
use openlaser_types::RefusalCode;

use crate::args::{FrameOpts, RunOpts, StatusOpts};
use crate::rig::{Purpose, Rig, RigEvent, RigSpec};
use crate::stop::{self, STOP_WAIT, StopTarget};
use crate::{Env, HwError, KeyPoll, KeySession, StopFn, StopKey, Terminal, require_tty, summary};

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
    let watcher = stop::watch(flags, Arc::clone(target), Box::new(|line| eprintln!("{line}")), Arc::clone(&env.exit)).map_err(HwError::Failed)?;
    Ok(Guards { _signals: Some(signals), _watcher: Some(watcher) })
}

fn stop_fn(target: &Arc<dyn StopTarget>) -> StopFn {
    let t = Arc::clone(target);
    Arc::new(move |_key: StopKey| t.stop_now())
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
        term.write(&format!("{l}\n"));
    }
    Ok(())
}

/// How a monitored phase ended.
enum Phase {
    /// `run`'s frame: a receipt that clears the job, accepted by Enter.
    Accepted(FrameReceipt),
    /// Frame Once completed.
    Completed(u32),
    Finished(Duration),
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
        Phase::Accepted(_) | Phase::Finished(_) => {}
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
    let mut keys = term.keys(stop_fn(target)).map_err(|e| HwError::Failed(format!("cannot read keys: {e}")))?;
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
                RigEvent::Fault(e) => return Err(HwError::Failed(format!("machine fault: {e}"))),
                RigEvent::Progress { .. } | RigEvent::Finished(_) | RigEvent::ArmVoided(_) => {}
            }
        }
        match keys.poll() {
            KeyPoll::Enter if accept_on_enter => match accepted.as_ref().map(|r| summary::receipt_accepts(r, &fp)) {
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
        env.clock.sleep(env.poll);
    }
}

/// Watches a marking job until it finishes, stops or faults.
fn mark_phase(rig: &mut dyn Rig, term: &mut dyn Terminal, env: &Env, target: &Arc<dyn StopTarget>, keys: &mut dyn KeySession) -> Result<Phase, HwError> {
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
                RigEvent::Finished(d) => return Ok(Phase::Finished(d)),
                RigEvent::Stopped(rep) => {
                    print_report(term, &rep);
                    return Ok(Phase::Stopped);
                }
                RigEvent::Fault(e) => return Err(HwError::Failed(format!("machine fault: {e}; the job was stopped"))),
                RigEvent::ArmVoided(reason) => term.write(&format!("ARM voided: {reason}\n")),
                RigEvent::FrameCompleted(_) => {}
            }
        }
        if keys.poll() == KeyPoll::Closed {
            term.write("The key reader closed: stopping.\n");
            let acks = target.stop_and_wait(STOP_WAIT);
            print_stop(term, &acks);
            return Ok(Phase::Stopped);
        }
        env.clock.sleep(env.poll);
    }
}

/// Stops the accepted frame and folds its stop into the engine before anything else happens: a stop the engine
/// saw only after `request_arm` would void the PendingArm.
fn stop_framing(rig: &mut dyn Rig, term: &mut dyn Terminal, env: &Env, target: &Arc<dyn StopTarget>) -> Result<(), HwError> {
    let acks = target.stop_and_wait(STOP_WAIT);
    if acks.iter().any(|(_, a)| a.report.as_ref().is_some_and(|r| !r.confirmed)) {
        print_stop(term, &acks);
        return Err(HwError::Failed("the frame stop was not confirmed; nothing was marked".to_owned()));
    }
    let deadline = env.clock.now().plus(STOP_WAIT.saturating_mul(2));
    loop {
        for ev in rig.poll() {
            match ev {
                RigEvent::Stopped(_) => return Ok(()),
                RigEvent::Fault(e) => return Err(HwError::Failed(format!("machine fault while stopping the frame: {e}"))),
                _ => {}
            }
        }
        if env.clock.now() >= deadline {
            return Ok(());
        }
        env.clock.sleep(env.poll);
    }
}

/// `openlaser-hw run`.
pub fn run(o: &RunOpts, term: &mut dyn Terminal, env: &mut Env) -> Result<(), HwError> {
    require_tty(term, "run")?;
    let mut rig = env.rigs.open(&RigSpec { purpose: Purpose::Run, device: o.device.clone(), extent: None })?;
    let target = rig.stopper();
    let _guards = guards(env, &target)?;
    let r = run_inner(rig.as_mut(), o, term, env, &target);
    rig.disconnect();
    r
}

fn run_inner(rig: &mut dyn Rig, o: &RunOpts, term: &mut dyn Terminal, env: &Env, target: &Arc<dyn StopTarget>) -> Result<(), HwError> {
    rig.load(&o.job, o.layers.as_deref(), o.repeats, o.power_pct, None)?;
    let planned = rig.summary()?;
    term.write(&summary::text(&planned));
    rig.connect()?;

    // Frame in a loop until Enter after an executed pass.
    let frame_opts = FrameOpts { job: o.job.clone(), looping: true, style: None, source: None, extent: None, device: o.device.clone() };
    let receipt = match frame_phase(rig, term, env, target, FrameMode::Loop, &frame_opts, true)? {
        Phase::Accepted(r) => r,
        _ => return Err(HwError::Failed("stopped while framing; nothing was marked".to_owned())),
    };
    stop_framing(rig, term, env, target)?;
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
        term.write(&format!("  {}. {item}\n", i + 1));
    }
    let challenge = openlaser_arm::tty_nonce()?;
    let Some(confirm) = term.confirm_arm(challenge, rig.now()) else {
        return Err(HwError::refused(RefusalCode::ArmTextMismatch, "the typed text was not `ARM` and the challenge; nothing was armed"));
    };
    let token = openlaser_arm::mint(&arm.summary, &arm.receipt, confirm, arm.checklist.len(), rig.session(), rig.now())?;

    // Keys first, so a stop works from the first packet.
    let mut keys = term.keys(stop_fn(target)).map_err(|e| HwError::Failed(format!("cannot read keys: {e}")))?;
    rig.run_armed(token)?;
    term.write("Marking. Esc, Space or Ctrl-C stops.\n");
    match mark_phase(rig, term, env, target, keys.as_mut())? {
        Phase::Finished(d) => {
            term.write(&format!("Finished in {:.1} s.\n", d.as_secs_f64()));
            Ok(())
        }
        _ => Err(HwError::Failed("the job was stopped before it finished".to_owned())),
    }
}
