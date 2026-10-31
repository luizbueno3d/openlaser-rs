//! In-process tests of `openlaser-hw` (SPEC §6.1: binaries are tested through `src/lib.rs` with injected stdio and
//! terminal; nothing here spawns the binary or opens a device).
//!
//! The scripted terminal gets its `HumanConfirm` from `openlaser_arm::testing` (SPEC §3.21). The scripted rig stands
//! in for the engine session so the CLI's own decisions (TTY gate, Enter only after an executed pass, stop keys,
//! `--allow-emission`, the bench cap, the challenge) are checked on their own; the tests at the end drive the real
//! engine (planner, encoder, driver) with the simulator, each with its own temporary profile folder.
// Test code: clippy.toml allows these inside `#[test]` fns only; the scripted terminal and rig below are helpers.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use openlaser_arm::HumanConfirm;
use openlaser_arm::testing::{confirm_for_tests, token_for_tests};
use openlaser_engine::{
    ArmToken, EmissionSummary, Fingerprint, FrameMode, FrameReceipt, FrameSource, FrameStyle, MachineId, MonoTime, SessionId, StopAck, StopReport,
};
use openlaser_hw::args::JobSource;
use openlaser_hw::builtin::Builtin;
use openlaser_hw::flow::{KeyStops, start_marking, watch_marking};
use openlaser_hw::rig::{ArmView, EngineRigs, Rig, RigEvent, RigFactory, RigSpec};
use openlaser_hw::stop::{CountingStop, StopTarget};
use openlaser_hw::trace::{TraceWriter, from_t1};
use openlaser_hw::tty_arm::arm_line_matches;
use openlaser_hw::{EXIT_FAIL, EXIT_OK, EXIT_USAGE, Env, HwError, KeyPoll, KeySession, StopFn, StopKey, Terminal, default_sim_config, run_with};
use openlaser_types::{Bounds, Clock, FrameOrigin, Notice, NoticeCode, PassRow, SectionMode, VirtualClock};
use serde_json::Value;

// ---- the scripted terminal -----------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    /// Nothing for this many key polls.
    Wait(u32),
    Enter,
    Stop(StopKey),
    Close,
    /// Pressed just as the key session ends: delivered by `Drop`, after the main loop stopped polling (a stop
    /// racing the acceptance of the frame).
    StopWhenDropped(StopKey),
}

/// How the operator answers the challenge: the typed line, from the challenge printed.
type Typing = fn(u64) -> String;

struct Scripted {
    tty: bool,
    lines: VecDeque<String>,
    out: Arc<Mutex<String>>,
    keys: Arc<Mutex<VecDeque<Key>>>,
    typing: Option<Typing>,
    /// How long the operator takes to type the line, on this clock.
    typing_takes: Option<(Arc<VirtualClock>, Duration)>,
    checklist_len: usize,
    challenges: Vec<u64>,
    /// (when the challenge was printed, when the confirmation was stamped).
    stamps: Vec<(MonoTime, MonoTime)>,
}

impl Scripted {
    fn new(tty: bool, keys: &[Key]) -> Self {
        Scripted {
            tty,
            lines: VecDeque::new(),
            out: Arc::new(Mutex::new(String::new())),
            keys: Arc::new(Mutex::new(keys.iter().copied().collect())),
            typing: None,
            typing_takes: None,
            checklist_len: 4,
            challenges: Vec::new(),
            stamps: Vec::new(),
        }
    }

    fn output(&self) -> String {
        self.out.lock().unwrap().clone()
    }
}

impl Terminal for Scripted {
    fn is_tty(&self) -> bool {
        self.tty
    }

    fn read_line(&mut self) -> Option<String> {
        self.lines.pop_front()
    }

    fn write(&mut self, text: &str) {
        self.out.lock().unwrap().push_str(text);
    }

    /// Emulates `TtyTerminal::confirm_arm`: a line matching the challenge yields a confirmation (here from
    /// `arm::testing`, as a GUI-kind confirmation of "ARM" with every item ticked) stamped once the line is in,
    /// anything else None.
    fn confirm_arm(&mut self, challenge: u64, now: &dyn Fn() -> MonoTime) -> Option<HumanConfirm> {
        self.challenges.push(challenge);
        let asked = now();
        let typed = (self.typing?)(challenge);
        if let Some((clock, took)) = &self.typing_takes {
            clock.advance(*took);
        }
        self.write(&format!("> {typed}\n"));
        let at = now();
        self.stamps.push((asked, at));
        arm_line_matches(&typed, challenge).then(|| confirm_for_tests("ARM", vec![true; self.checklist_len], at))
    }

    fn keys(&mut self, stop: StopFn) -> Result<Box<dyn KeySession>, String> {
        if !self.tty {
            return Err("not a terminal".into());
        }
        Ok(Box::new(ScriptedKeys { keys: Arc::clone(&self.keys), stop }))
    }
}

struct ScriptedKeys {
    keys: Arc<Mutex<VecDeque<Key>>>,
    stop: StopFn,
}

impl KeySession for ScriptedKeys {
    fn poll(&mut self) -> KeyPoll {
        let mut keys = self.keys.lock().unwrap();
        match keys.pop_front() {
            None => KeyPoll::Nothing,
            Some(Key::Wait(n)) => {
                if n > 1 {
                    keys.push_front(Key::Wait(n - 1));
                }
                KeyPoll::Nothing
            }
            Some(Key::Enter) => KeyPoll::Enter,
            Some(Key::Stop(k)) => {
                drop(keys);
                (self.stop)(k);
                KeyPoll::Nothing
            }
            Some(Key::Close) => KeyPoll::Closed,
            Some(k @ Key::StopWhenDropped(_)) => {
                keys.push_front(k);
                KeyPoll::Nothing
            }
        }
    }
}

impl Drop for ScriptedKeys {
    fn drop(&mut self) {
        let pending = {
            let mut keys = self.keys.lock().unwrap();
            match keys.front() {
                Some(Key::StopWhenDropped(k)) => {
                    let k = *k;
                    keys.pop_front();
                    Some(k)
                }
                _ => None,
            }
        };
        if let Some(k) = pending {
            (self.stop)(k);
        }
    }
}

// ---- the scripted rig ------------------------------------------------------------------------------------------

const FP: Fingerprint = Fingerprint([0x42; 32]);

fn row(power_pct: f64) -> PassRow {
    PassRow {
        layer: openlaser_types::LayerKey::Color(0),
        layer_name: "C00".into(),
        sub: 0,
        pass: 1,
        of: 1,
        mode: SectionMode::Line,
        power_pct,
        frequency_khz: Some(20_000.0 / 667.0),
        pulse_ns: Some(200),
        speed_mm_s: 996.8,
        galvo: None,
        paths: 1,
        mark_mm: 40.0,
        bounds: Bounds { min: [-5.0, -5.0], max: [5.0, 5.0] },
        estimate: Duration::from_millis(62),
    }
}

fn summary(power_pct: f64) -> EmissionSummary {
    EmissionSummary {
        rotary: None,
        fingerprint: FP,
        bytes_hash: Fingerprint([0x43; 32]),
        machine: MachineId("jpt-m7-200-sim".into()),
        rows: vec![row(power_pct)],
        notices: Vec::new(),
        repeats: 1,
        estimate: Duration::from_millis(62),
        crash_bound: Duration::from_millis(62),
        packets: 1,
        work_area: Bounds { min: [-100.0, -100.0], max: [100.0, 100.0] },
        negate: [false, false],
        swap_xy: false,
    }
}

fn receipt(passes: u32, source: FrameSource) -> FrameReceipt {
    FrameReceipt {
        fingerprint: FP,
        machine: MachineId("jpt-m7-200-sim".into()),
        session: SessionId(1),
        full_passes: passes,
        at: MonoTime::default(),
        style: FrameStyle::Bounds,
        source,
        origin: FrameOrigin::Human,
    }
}

fn confirmed() -> StopReport {
    StopReport { confirmed: true, elapsed: Duration::from_millis(40), possible_desync: false, mo_closed: true, ports_zero: true, notes: Vec::new() }
}

#[derive(Default)]
struct Script {
    /// One batch per poll, in order; then nothing.
    batches: VecDeque<Vec<RigEvent>>,
    /// Delivered before the next batch (stops land here).
    urgent: VecDeque<RigEvent>,
    calls: Vec<String>,
    power_pct: f64,
    cap: Option<f64>,
    polls: u32,
    opened: u32,
    /// What `stop_and_wait` answers instead of one confirmed report (a driver that never reported, say).
    stop_acks: Option<Vec<(MachineId, StopAck)>>,
    /// A stop that lands while `run_armed` is queueing the job (the key thread racing the main thread).
    stop_during_run_armed: Option<Arc<dyn StopTarget>>,
    /// `stop_now` trips nothing the rig ever reports (a wedged transfer, a dead driver thread): no Stopped event.
    silent_stop_now: bool,
    /// The checklist `request_arm` returns (default: four items).
    checklist: Option<Vec<String>>,
    /// Edits every summary the rig hands out (document text an agent wrote, say).
    forge: Option<fn(&mut EmissionSummary)>,
    /// The rig's clock (`fake_env` shares it with `Env`).
    clock: Option<Arc<VirtualClock>>,
}

type Shared = Arc<Mutex<Script>>;

struct FakeStop(Shared);

impl StopTarget for FakeStop {
    fn stop_now(&self) {
        let mut s = self.0.lock().unwrap();
        s.calls.push("stop_now".into());
        if !s.silent_stop_now {
            s.urgent.push_back(RigEvent::Stopped(confirmed()));
        }
    }

    fn stop_and_wait(&self, _max: Duration) -> Vec<(MachineId, StopAck)> {
        let mut s = self.0.lock().unwrap();
        s.calls.push("stop_and_wait".into());
        if let Some(acks) = s.stop_acks.clone() {
            return acks;
        }
        s.urgent.push_back(RigEvent::Stopped(confirmed()));
        vec![(MachineId("jpt-m7-200-sim".into()), StopAck { latched: true, report: Some(confirmed()), elapsed_ms: 40 })]
    }
}

struct FakeRig {
    s: Shared,
    machine: MachineId,
}

impl FakeRig {
    fn call(&self, c: &str) {
        self.s.lock().unwrap().calls.push(c.to_owned());
    }

    fn summary_now(&self) -> EmissionSummary {
        let s = self.s.lock().unwrap();
        let mut out = summary(s.power_pct);
        if let Some(forge) = s.forge {
            forge(&mut out);
        }
        out
    }
}

impl Rig for FakeRig {
    fn machine(&self) -> &MachineId {
        &self.machine
    }

    fn session(&self) -> SessionId {
        SessionId(1)
    }

    fn now(&self) -> MonoTime {
        self.s.lock().unwrap().clock.as_ref().map_or_else(MonoTime::default, |c| c.now())
    }

    fn load(
        &mut self,
        job: &JobSource,
        _layers: Option<&std::path::Path>,
        _repeats: Option<u16>,
        power: Option<f64>,
        _extent: Option<f64>,
    ) -> Result<(), HwError> {
        self.call(&format!("load {job:?} power={power:?}"));
        Ok(())
    }

    fn summary(&mut self) -> Result<EmissionSummary, HwError> {
        self.call("summary");
        Ok(self.summary_now())
    }

    fn connect(&mut self) -> Result<(), HwError> {
        self.call("connect");
        Ok(())
    }

    fn start_frame(&mut self, mode: FrameMode, _style: Option<FrameStyle>, _source: Option<FrameSource>) -> Result<Fingerprint, HwError> {
        self.call(&format!("start_frame {mode:?}"));
        Ok(FP)
    }

    fn poll(&mut self) -> Vec<RigEvent> {
        let mut s = self.s.lock().unwrap();
        s.polls += 1;
        if s.polls > 5_000 {
            return vec![RigEvent::Fault("test guard: the flow never ended".into())];
        }
        let mut out: Vec<RigEvent> = s.urgent.drain(..).collect();
        if out.is_empty() {
            out = s.batches.pop_front().unwrap_or_default();
        }
        out
    }

    fn stopper(&self) -> Arc<dyn StopTarget> {
        Arc::new(FakeStop(Arc::clone(&self.s)))
    }

    fn request_arm(&mut self) -> Result<ArmView, HwError> {
        self.call("request_arm");
        let checklist = self.s.lock().unwrap().checklist.clone().unwrap_or_else(|| vec!["E-stop in reach".into(); 4]);
        Ok(ArmView { summary: self.summary_now(), receipt: receipt(1, FrameSource::Job), checklist })
    }

    fn emission_cap_pct(&self) -> Result<Option<f64>, HwError> {
        Ok(self.s.lock().unwrap().cap)
    }

    fn run_armed(&mut self, token: ArmToken) -> Result<(), HwError> {
        self.call(&format!("run_armed {}", token.fingerprint().short()));
        let racing = self.s.lock().unwrap().stop_during_run_armed.clone();
        if let Some(stop) = racing {
            stop.stop_now();
        }
        self.s
            .lock()
            .unwrap()
            .batches
            .extend([vec![RigEvent::Progress { packets_acked: 1, total: 1, eta_ms: 0 }], vec![RigEvent::Finished(Duration::from_millis(62))]]);
        Ok(())
    }

    fn probe(&mut self) -> Result<Vec<String>, HwError> {
        Ok(vec!["serial: TEST".into()])
    }

    fn disconnect(&mut self) {
        self.call("disconnect");
    }
}

struct FakeRigs(Shared);

impl RigFactory for FakeRigs {
    fn open(&mut self, _spec: &RigSpec) -> Result<Box<dyn Rig>, HwError> {
        self.0.lock().unwrap().opened += 1;
        Ok(Box::new(FakeRig { s: Arc::clone(&self.0), machine: MachineId("jpt-m7-200-sim".into()) }))
    }
}

fn fake_env(mut script: Script) -> (Env, Shared) {
    let clock = Arc::new(VirtualClock::new());
    script.clock = Some(Arc::clone(&clock));
    let shared = Arc::new(Mutex::new(script));
    let env = Env {
        profiles_dir: None,
        clock,
        poll: Duration::from_millis(20),
        signals: false,
        exit: Arc::new(|_| {}),
        rigs: Box::new(FakeRigs(Arc::clone(&shared))),
    };
    (env, shared)
}

fn argv(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_owned).collect()
}

fn calls(s: &Shared) -> Vec<String> {
    s.lock().unwrap().calls.clone()
}

fn has(calls: &[String], prefix: &str) -> bool {
    calls.iter().any(|c| c.starts_with(prefix))
}

/// A frame that completes one executed pass on the third poll.
fn one_pass_on_poll_3(power_pct: f64, cap: Option<f64>) -> Script {
    Script { batches: VecDeque::from([vec![], vec![], vec![RigEvent::FrameCompleted(receipt(1, FrameSource::Job))]]), power_pct, cap, ..Script::default() }
}

// ---- TTY gate ----------------------------------------------------------------------------------------------------

#[test]
fn cli_arm_refused_without_tty() {
    let (mut env, s) = fake_env(one_pass_on_poll_3(1.0, None));
    let mut term = Scripted::new(false, &[Key::Enter]);
    term.typing = Some(|c| format!("ARM {c:06}"));
    let code = run_with(&argv("run --builtin dot --allow-emission --sim"), &mut term, &mut env);
    assert_eq!(code, EXIT_FAIL);
    assert!(term.output().contains("not_a_tty"), "{}", term.output());
    assert_eq!(s.lock().unwrap().opened, 0, "nothing is opened without a TTY");
    assert!(term.challenges.is_empty(), "no challenge is ever issued without a TTY");
    // `frame` too.
    let code = run_with(&argv("frame --builtin box10 --sim"), &mut term, &mut env);
    assert_eq!(code, EXIT_FAIL);
    assert_eq!(s.lock().unwrap().opened, 0);
}

#[test]
fn cli_bench_start_requires_tty() {
    let (mut env, _) = fake_env(Script::default());
    for sub in ["bench start", "bench status", "bench end"] {
        let mut term = Scripted::new(false, &[]);
        assert_eq!(run_with(&argv(sub), &mut term, &mut env), EXIT_FAIL, "{sub}");
        assert!(term.output().contains("not_a_tty"), "{sub}: {}", term.output());
    }
}

#[test]
fn cli_profile_set_requires_tty() {
    let dir = tempfile::tempdir().unwrap();
    let (mut env, _) = fake_env(Script::default());
    env.profiles_dir = Some(dir.path().to_path_buf());
    for sub in ["profile set cal.negate.x true", "profile import-om calibration.json", "profile show"] {
        let mut term = Scripted::new(false, &[]);
        term.lines.push_back("SAVE jpt-m7-200".into());
        assert_eq!(run_with(&argv(sub), &mut term, &mut env), EXIT_FAIL, "{sub}");
        assert!(term.output().contains("not_a_tty"), "{sub}: {}", term.output());
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "nothing was written");
}

#[test]
fn cli_profile_set_refused_while_its_file_is_skipped() {
    // The saved profile no longer loads: the diff would compare the built-in, and SAVE would overwrite the file with
    // the built-in plus the change. Refused; the file is left as it is.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("jpt-m7-200.json");
    std::fs::write(&file, b"{ \"id\": \"jpt-m7-200\", ").unwrap();
    let (mut env, _) = fake_env(Script::default());
    env.profiles_dir = Some(dir.path().to_path_buf());
    for sub in ["profile set cal.negate.x true", "profile import-om calibration.json", "profile import-ezcad markcfg7"] {
        let mut term = Scripted::new(true, &[]);
        term.lines.push_back("SAVE jpt-m7-200".into());
        assert_eq!(run_with(&argv(sub), &mut term, &mut env), EXIT_FAIL, "{sub}");
        let out = term.output();
        assert!(out.contains("machine profile files were skipped") && out.contains("nothing was written"), "{sub}: {out}");
        assert!(!out.contains("Type `SAVE"), "{sub}: never asks to save: {out}");
    }
    assert_eq!(std::fs::read(&file).unwrap(), b"{ \"id\": \"jpt-m7-200\", ");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

// ---- hardware is a build feature ---------------------------------------------------------------------------------

#[cfg(not(feature = "hw"))]
#[test]
fn cli_usb_requires_hw_feature() {
    let clock = Arc::new(VirtualClock::new());
    let mut env = Env {
        profiles_dir: None,
        clock: clock.clone(),
        poll: Duration::from_millis(20),
        signals: false,
        exit: Arc::new(|_| {}),
        rigs: Box::new(EngineRigs::new(None, default_sim_config(), clock)),
    };
    for sub in [
        "run --builtin dot --allow-emission",
        "run card.olp --machine jpt-m7-200 --allow-emission",
        "frame --builtin box10",
        "frame --builtin box10 --machine jpt-m7-200-sim",
        "status",
        "detect",
        "bench start",
    ] {
        let mut term = Scripted::new(true, &[]);
        let code = run_with(&argv(sub), &mut term, &mut env);
        let out = term.output();
        assert_eq!(code, EXIT_FAIL, "{sub}: {out}");
        assert!(out.contains("hardware_disabled"), "{sub}: {out}");
        assert!(!out.contains("Framing"), "{sub}: nothing started");
    }
}

// ---- the frame-then-ARM flow ------------------------------------------------------------------------------------

#[test]
fn cli_enter_before_full_frame_pass_refused() {
    // Enter on the first key poll (no receipt yet), then again after the pass the board executed on poll 3.
    let (mut env, s) = fake_env(one_pass_on_poll_3(1.0, None));
    let mut term = Scripted::new(true, &[Key::Enter, Key::Wait(3), Key::Enter]);
    let code = run_with(&argv("run --builtin dot --sim"), &mut term, &mut env);
    let out = term.output();
    assert!(out.contains("Enter ignored (frame_required)"), "{out}");
    assert_eq!(out.matches("Enter ignored").count(), 1, "{out}");
    assert!(out.contains("Frame pass 1 executed by the board."), "{out}");
    assert!(out.contains("Frame accepted after 1 executed pass(es)"), "{out}");
    // Without --allow-emission the run ends here.
    assert_eq!(code, EXIT_FAIL);
    let c = calls(&s);
    assert!(has(&c, "start_frame Loop"), "{c:?}");
    assert!(!has(&c, "request_arm"), "{c:?}");
}

#[test]
fn cli_enter_refused_for_receipts_that_do_not_clear_the_job() {
    for (r, code) in [(receipt(0, FrameSource::Job), "frame_required"), (receipt(3, FrameSource::ToolLayersOnly), "frame_does_not_cover_job")] {
        let script = Script { batches: VecDeque::from([vec![RigEvent::FrameCompleted(r)]]), power_pct: 1.0, ..Script::default() };
        let (mut env, s) = fake_env(script);
        let mut term = Scripted::new(true, &[Key::Wait(2), Key::Enter, Key::Wait(2), Key::Stop(StopKey::Esc)]);
        assert_eq!(run_with(&argv("run --builtin dot --sim --allow-emission"), &mut term, &mut env), EXIT_FAIL);
        let out = term.output();
        assert!(out.contains(&format!("Enter ignored ({code})")), "{out}");
        assert!(!has(&calls(&s), "request_arm"));
    }
}

#[test]
fn cli_run_without_allow_emission_stops_after_frame() {
    let (mut env, s) = fake_env(one_pass_on_poll_3(1.0, None));
    let mut term = Scripted::new(true, &[Key::Wait(4), Key::Enter]);
    term.typing = Some(|c| format!("ARM {c:06}"));
    let code = run_with(&argv("run --builtin dot --sim"), &mut term, &mut env);
    let out = term.output();
    assert_eq!(code, EXIT_FAIL);
    assert!(out.contains("emission_not_allowed"), "{out}");
    let c = calls(&s);
    let frame = c.iter().position(|x| x.starts_with("start_frame")).unwrap();
    let stop = c.iter().position(|x| x == "stop_and_wait").unwrap();
    assert!(stop > frame, "framing is stopped: {c:?}");
    assert!(!has(&c, "request_arm") && !has(&c, "run_armed"), "{c:?}");
    assert!(term.challenges.is_empty());
}

#[test]
fn unconfirmed_frame_stop_never_reaches_arm() {
    // Stopping the accepted frame must end in a confirmed StopReport from every machine; a wait that ran out, a latch
    // that did not trip, an unconfirmed report or no machine at all ends the run before the ARM step.
    let m = || MachineId("jpt-m7-200-sim".into());
    let unconfirmed = StopReport { confirmed: false, ..confirmed() };
    for acks in [
        vec![(m(), StopAck { latched: true, report: None, elapsed_ms: 500 })],
        vec![(m(), StopAck { latched: false, report: None, elapsed_ms: 0 })],
        vec![(m(), StopAck { latched: true, report: Some(unconfirmed), elapsed_ms: 60 })],
        vec![],
    ] {
        let mut script = one_pass_on_poll_3(1.0, None);
        script.stop_acks = Some(acks.clone());
        let (mut env, s) = fake_env(script);
        let mut term = Scripted::new(true, &[Key::Wait(4), Key::Enter]);
        term.typing = Some(|c| format!("ARM {c:06}"));
        let code = run_with(&argv("run --builtin dot --sim --allow-emission"), &mut term, &mut env);
        let out = term.output();
        assert_eq!(code, EXIT_FAIL, "{acks:?}: {out}");
        assert!(out.contains("the frame stop was not confirmed within 500 ms"), "{acks:?}: {out}");
        assert!(!has(&calls(&s), "request_arm"), "{acks:?}: {:?}", calls(&s));
        assert!(term.challenges.is_empty());
    }
}

#[test]
fn frame_stop_the_engine_does_not_report_refuses_to_arm() {
    // The driver confirmed the stop, but the engine never reports it (or reports it unconfirmed): the wait runs out
    // or the report says no, and the run ends before the ARM step instead of going on.
    let confirmed_ack = vec![(MachineId("jpt-m7-200-sim".into()), StopAck { latched: true, report: Some(confirmed()), elapsed_ms: 40 })];
    let unconfirmed = StopReport { confirmed: false, ..confirmed() };
    let fc = || RigEvent::FrameCompleted(receipt(1, FrameSource::Job));
    for (batches, says) in [
        (VecDeque::from([vec![], vec![], vec![fc()]]), "did not report the frame stop within 1000 ms"),
        (VecDeque::from([vec![], vec![], vec![fc()], vec![], vec![RigEvent::Stopped(unconfirmed)]]), "the frame stop was not confirmed"),
    ] {
        let script = Script { batches, power_pct: 1.0, stop_acks: Some(confirmed_ack.clone()), ..Script::default() };
        let (mut env, s) = fake_env(script);
        let mut term = Scripted::new(true, &[Key::Wait(4), Key::Enter]);
        term.typing = Some(|c| format!("ARM {c:06}"));
        assert_eq!(run_with(&argv("run --builtin dot --sim --allow-emission"), &mut term, &mut env), EXIT_FAIL);
        let out = term.output();
        assert!(out.contains(says), "{out}");
        assert!(!has(&calls(&s), "request_arm"), "{:?}", calls(&s));
    }
}

#[test]
fn stop_racing_the_frame_acceptance_marks_nothing() {
    // Enter accepts the frame and Esc follows at once; the key thread handles the Esc while it winds down, after the
    // main loop took the Enter. The run ends before the ARM step instead of counting the Esc as the frame stop.
    let (mut env, s) = fake_env(one_pass_on_poll_3(1.0, None));
    let mut term = Scripted::new(true, &[Key::Wait(4), Key::Enter, Key::StopWhenDropped(StopKey::Esc)]);
    term.typing = Some(|c| format!("ARM {c:06}"));
    assert_eq!(run_with(&argv("run --builtin dot --sim --allow-emission"), &mut term, &mut env), EXIT_FAIL);
    let out = term.output();
    assert!(out.contains("a stop was asked for while the frame was being accepted"), "{out}");
    assert!(!has(&calls(&s), "request_arm"), "{:?}", calls(&s));
    assert!(term.challenges.is_empty());
}

fn armed_token() -> ArmToken {
    token_for_tests(FP, Fingerprint([0x43; 32]), MachineId("jpt-m7-200-sim".into()), SessionId(1), MonoTime(Duration::from_secs(60)), 7)
}

fn fake_rig(s: &Shared) -> Box<dyn Rig> {
    FakeRigs(Arc::clone(s)).open(&RigSpec { purpose: openlaser_hw::rig::Purpose::Run, device: Default::default(), extent: None }).unwrap()
}

#[test]
fn stop_after_the_frame_was_accepted_marks_nothing() {
    // Space between the accepted frame and queueing the run: the driver would clear its latch for a command queued
    // after the trip (SPEC §5.3), so the CLI refuses to queue it at all.
    let s: Shared = Arc::new(Mutex::new(Script::default()));
    let mut rig = fake_rig(&s);
    let stops = CountingStop::new(rig.stopper());
    let before = stops.requests();
    stops.stop_now();
    let err = start_marking(rig.as_mut(), &stops, before, armed_token()).unwrap_err();
    assert!(err.to_string().contains("nothing was marked"), "{err}");
    assert!(!has(&calls(&s), "run_armed"), "{:?}", calls(&s));
}

#[test]
fn stop_racing_the_queued_run_trips_again_after_it() {
    // The key thread trips the latch while the main thread is queueing the run: the CLI trips it again once the run
    // exists, so the stop cancels it instead of being cleared for it.
    let s: Shared = Arc::new(Mutex::new(Script::default()));
    let mut rig = fake_rig(&s);
    let stops = Arc::new(CountingStop::new(rig.stopper()));
    s.lock().unwrap().stop_during_run_armed = Some(Arc::clone(&stops) as Arc<dyn StopTarget>);
    let before = stops.requests();
    start_marking(rig.as_mut(), &stops, before, armed_token()).unwrap();
    let c = calls(&s);
    let queued = c.iter().position(|x| x.starts_with("run_armed")).unwrap();
    let after: Vec<&String> = c.iter().skip(queued + 1).filter(|x| *x == "stop_now").collect();
    assert_eq!(after.len(), 2, "the racing stop and the re-trip after queueing: {c:?}");
    // Without a stop nothing is tripped.
    let s: Shared = Arc::new(Mutex::new(Script::default()));
    let mut rig = fake_rig(&s);
    let stops = CountingStop::new(rig.stopper());
    start_marking(rig.as_mut(), &stops, stops.requests(), armed_token()).unwrap();
    assert!(!has(&calls(&s), "stop_now"), "{:?}", calls(&s));
}

#[test]
fn enter_read_with_the_first_receipt_is_judged_before_it() {
    // The board's first executed pass and an Enter arrive in the same poll: the Enter was pressed before the CLI saw
    // the pass, so it does not accept the frame.
    let script = Script { batches: VecDeque::from([vec![RigEvent::FrameCompleted(receipt(1, FrameSource::Job))]]), power_pct: 1.0, ..Script::default() };
    let (mut env, s) = fake_env(script);
    let mut term = Scripted::new(true, &[Key::Enter, Key::Wait(2), Key::Stop(StopKey::Esc)]);
    assert_eq!(run_with(&argv("run --builtin dot --sim --allow-emission"), &mut term, &mut env), EXIT_FAIL);
    let out = term.output();
    assert!(out.contains("Enter ignored (frame_required)"), "{out}");
    assert!(!out.contains("Frame accepted"), "{out}");
    assert!(!has(&calls(&s), "request_arm"));
}

#[test]
fn cli_first_emission_power_cap_refused() {
    // The bench session's first emission is capped at 10 %; the job's only row is at 50 %.
    let (mut env, s) = fake_env(one_pass_on_poll_3(50.0, Some(10.0)));
    let mut term = Scripted::new(true, &[Key::Wait(4), Key::Enter]);
    term.typing = Some(|c| format!("ARM {c:06}"));
    let code = run_with(&argv("run --builtin box10 --sim --allow-emission"), &mut term, &mut env);
    let out = term.output();
    assert_eq!(code, EXIT_FAIL);
    assert!(out.contains("bench_power_cap"), "{out}");
    assert!(out.contains("capped at 10 %"), "{out}");
    assert!(term.challenges.is_empty(), "refused before any challenge");
    assert!(!has(&calls(&s), "run_armed"));
}

#[test]
fn cli_space_and_esc_stop() {
    for key in [StopKey::Space, StopKey::Esc, StopKey::CtrlC] {
        let (mut env, s) = fake_env(Script { power_pct: 1.0, ..Script::default() });
        let mut term = Scripted::new(true, &[Key::Wait(3), Key::Stop(key)]);
        let code = run_with(&argv("run --builtin dot --sim --allow-emission"), &mut term, &mut env);
        let out = term.output();
        assert_eq!(code, EXIT_FAIL, "{key}");
        assert!(out.contains("Stopped: confirmed"), "{key}: {out}");
        assert!(!out.contains("STOP NOT CONFIRMED"), "{key}: the report came in time: {out}");
        assert!(out.contains("nothing was marked"), "{key}: {out}");
        let c = calls(&s);
        assert!(has(&c, "stop_now"), "{key} trips the latch at once: {c:?}");
        assert!(!has(&c, "request_arm"), "{c:?}");
    }
    // In `frame --loop`, a stop key is how a loop frame ends: success.
    let (mut env, s) = fake_env(Script { batches: VecDeque::from([vec![RigEvent::FrameCompleted(receipt(1, FrameSource::Job))]]), ..Script::default() });
    let mut term = Scripted::new(true, &[Key::Wait(5), Key::Stop(StopKey::Space)]);
    assert_eq!(run_with(&argv("frame --builtin box10 --loop --sim"), &mut term, &mut env), EXIT_OK, "{}", term.output());
    assert!(has(&calls(&s), "stop_now"));
}

/// What the watchdog's second request gets from a driver that never reported: latched, no StopReport.
fn no_report() -> Vec<(MachineId, StopAck)> {
    vec![(MachineId("jpt-m7-200-sim".into()), StopAck { latched: true, report: None, elapsed_ms: 500 })]
}

#[test]
fn stop_key_without_a_report_says_stop_not_confirmed_within_500_ms() {
    // SPEC §5.4 watchdog: the key trips the latch, but no StopReport ever arrives (a wedged transfer, a dead driver
    // thread). 500 ms later the operator reads STOP NOT CONFIRMED and is sent to the key switch / E-stop, the CLI asks
    // every machine again and prints their answers, and the command ends in an error, both in `frame --loop` (where a
    // reported stop is a success) and in `run`.
    for (cmd, key) in [("frame --builtin box10 --loop --sim", StopKey::Esc), ("run --builtin dot --sim --allow-emission", StopKey::Space)] {
        let script = Script { power_pct: 1.0, silent_stop_now: true, stop_acks: Some(no_report()), ..Script::default() };
        let (mut env, s) = fake_env(script);
        let clock = s.lock().unwrap().clock.clone().unwrap();
        let mut term = Scripted::new(true, &[Key::Wait(3), Key::Stop(key)]);
        let code = run_with(&argv(cmd), &mut term, &mut env);
        let out = term.output();
        assert_eq!(code, EXIT_FAIL, "{cmd}: {out}");
        assert!(out.contains("STOP NOT CONFIRMED: no StopReport within 500 ms of the stop key: use the key switch / E-stop."), "{cmd}: {out}");
        assert!(out.contains("stop jpt-m7-200-sim: latched; no StopReport within 500 ms: use the key switch / E-stop"), "{cmd}: {out}");
        assert!(out.contains("the stop was not confirmed: use the key switch / E-stop"), "{cmd}: {out}");
        let c = calls(&s);
        let pressed = c.iter().position(|x| x == "stop_now").unwrap();
        assert!(c.iter().skip(pressed).any(|x| x == "stop_and_wait"), "asked again: {c:?}");
        assert!(!has(&c, "request_arm"), "{c:?}");
        // The key came on the 4th poll (60 ms); the notice came on the first poll at or after 560 ms, not later.
        let t = clock.now().0;
        assert!(t >= Duration::from_millis(560) && t <= Duration::from_millis(600), "{cmd}: {t:?}");
    }
}

#[test]
fn stop_key_without_a_report_while_marking_says_stop_not_confirmed() {
    let progress = |n| vec![RigEvent::Progress { packets_acked: n, total: 1_000, eta_ms: 60_000 }];
    let script = Script { silent_stop_now: true, stop_acks: Some(no_report()), batches: (1..2_000).map(progress).collect(), ..Script::default() };
    let (env, s) = fake_env(script);
    let mut rig = fake_rig(&s);
    let key_stops = KeyStops::new(rig.stopper(), Arc::clone(&env.clock));
    let mut term = Scripted::new(true, &[Key::Wait(5), Key::Stop(StopKey::Esc)]);
    let mut keys = term.keys(key_stops.stop_fn()).unwrap();
    let err = watch_marking(rig.as_mut(), &mut term, &env, &key_stops, keys.as_mut()).unwrap_err();
    let out = term.output();
    assert!(out.contains("STOP NOT CONFIRMED: no StopReport within 500 ms of the stop key: use the key switch / E-stop."), "{out}");
    assert!(err.to_string().contains("the stop was not confirmed: use the key switch / E-stop"), "{err}");
    assert!(key_stops.pressed_at().is_some());
    let polls = s.lock().unwrap().polls;
    assert!(polls <= 6 + 26, "the watchdog fired 500 ms after the key, not later ({polls} polls)");
    // The driver confirms the stop when asked again: still an error, and it says the engine never reported.
    let script = Script { silent_stop_now: true, batches: (1..2_000).map(progress).collect(), ..Script::default() };
    let (env, s) = fake_env(script);
    let mut rig = fake_rig(&s);
    let key_stops = KeyStops::new(rig.stopper(), Arc::clone(&env.clock));
    let mut term = Scripted::new(true, &[Key::Wait(5), Key::Stop(StopKey::Esc)]);
    let mut keys = term.keys(key_stops.stop_fn()).unwrap();
    let err = watch_marking(rig.as_mut(), &mut term, &env, &key_stops, keys.as_mut()).unwrap_err();
    assert!(err.to_string().contains("the engine did not report the stop within 500 ms"), "{err}");
    assert!(term.output().contains("stop jpt-m7-200-sim: confirmed in 40 ms"), "{}", term.output());
}

#[test]
fn cli_empty_checklist_refused_before_any_challenge() {
    let mut script = one_pass_on_poll_3(1.0, None);
    script.checklist = Some(Vec::new());
    let (mut env, s) = fake_env(script);
    let mut term = Scripted::new(true, &[Key::Wait(4), Key::Enter]);
    term.typing = Some(|c| format!("ARM {c:06}"));
    assert_eq!(run_with(&argv("run --builtin dot --sim --allow-emission"), &mut term, &mut env), EXIT_FAIL);
    let out = term.output();
    assert!(out.contains("checklist_incomplete"), "{out}");
    assert!(term.challenges.is_empty(), "no challenge is issued");
    assert!(has(&calls(&s), "request_arm") && !has(&calls(&s), "run_armed"), "{:?}", calls(&s));
}

#[test]
fn document_text_reaches_the_terminal_as_plain_text() {
    // An agent named the layer so that it prints fake numbers and then conceals (ESC[8m) the real ones and everything
    // after them; a notice and a warning carry more escape sequences, a carriage return and a bidi override. Whatever
    // reaches Terminal::write has no control character but the CLI's own line ends, and the real power is shown.
    let mut script = one_pass_on_poll_3(80.0, None);
    script.forge = Some(|s| {
        s.rows[0].layer_name = "C00   Line  1/1  5.00 % (fake)\u{1b}[8m".into();
        s.notices.push(Notice {
            n: 1,
            code: NoticeCode::OpenShapeUnhatched,
            message: "open shape\u{1b}[2J\u{9b}8m\r\u{202E}%08\u{200B}".into(),
            object: None,
            layer: None,
        });
    });
    script.batches.push_front(vec![RigEvent::Warning("trace\u{1b}]0;title\u{7}\u{1b}[8m".into())]);
    script.checklist = Some(vec!["E-stop in reach\u{1b}[8m".into()]);
    let (mut env, _s) = fake_env(script);
    let mut term = Scripted::new(true, &[Key::Wait(5), Key::Enter]);
    term.typing = Some(|c| format!("ARM {c:06}"));
    let _ = run_with(&argv("run --builtin dot --sim --allow-emission"), &mut term, &mut env);
    let out = term.output();
    let bad: Vec<char> = out.chars().filter(|c| *c != '\n' && (c.is_control() || ['\u{202E}', '\u{200B}'].contains(c))).collect();
    assert!(bad.is_empty(), "{bad:?} reached the terminal:\n{out}");
    assert!(out.contains("80.00 %"), "{out}");
    assert!(out.contains("Notice 1: open shape[2J8m%08"), "{out}");
    assert!(out.contains("warning: trace]0;title[8m"), "{out}");
    assert!(out.contains("Checklist (typing the challenge confirms every item):\n  1. E-stop in reach[8m\n"), "{out}");
}

#[test]
fn arm_confirmation_is_stamped_after_the_line_is_read() {
    // The operator takes 30 s to read the summary and type the challenge: the confirmation carries the time the line
    // came in, not the time the prompt was printed (mint refuses a confirmation older than 10 s).
    let (mut env, s) = fake_env(one_pass_on_poll_3(1.0, None));
    let clock = s.lock().unwrap().clock.clone().unwrap();
    let mut term = Scripted::new(true, &[Key::Wait(4), Key::Enter]);
    term.typing = Some(|c| format!("ARM {c:06}"));
    term.typing_takes = Some((clock, Duration::from_secs(30)));
    let _ = run_with(&argv("run --builtin dot --sim --allow-emission"), &mut term, &mut env);
    assert_eq!(term.stamps.len(), 1, "{}", term.output());
    let (asked, at) = term.stamps[0];
    assert_eq!(at, asked.plus(Duration::from_secs(30)), "stamped when the line was read");
}

#[test]
fn closed_key_reader_stops_the_machine() {
    let (mut env, s) = fake_env(Script { power_pct: 1.0, ..Script::default() });
    let mut term = Scripted::new(true, &[Key::Wait(2), Key::Close]);
    assert_eq!(run_with(&argv("run --builtin dot --sim --allow-emission"), &mut term, &mut env), EXIT_FAIL);
    assert!(has(&calls(&s), "stop_and_wait"));
    assert!(term.output().contains("The key reader closed: stopping."));
}

#[test]
fn frame_once_ends_on_its_receipt() {
    let (mut env, s) =
        fake_env(Script { batches: VecDeque::from([vec![], vec![RigEvent::FrameCompleted(receipt(6, FrameSource::Job))]]), ..Script::default() });
    let mut term = Scripted::new(true, &[]);
    assert_eq!(run_with(&argv("frame --builtin box10 --sim"), &mut term, &mut env), EXIT_OK, "{}", term.output());
    assert!(term.output().contains("Frame done: 6 pass(es) executed."));
    let c = calls(&s);
    assert!(has(&c, "start_frame Once") && has(&c, "disconnect"), "{c:?}");
}

#[test]
fn faults_end_the_flow() {
    let (mut env, _) = fake_env(Script { batches: VecDeque::from([vec![RigEvent::Fault("transport: transfer timed out".into())]]), ..Script::default() });
    let mut term = Scripted::new(true, &[]);
    assert_eq!(run_with(&argv("frame --builtin box10 --loop --sim"), &mut term, &mut env), EXIT_FAIL);
    assert!(term.output().contains("machine fault: transport: transfer timed out"));
}

#[test]
fn cli_wrong_nonce_refused() {
    let (mut env, s) = fake_env(one_pass_on_poll_3(1.0, None));
    let mut term = Scripted::new(true, &[Key::Wait(4), Key::Enter]);
    term.typing = Some(|c| format!("ARM {:06}", (c + 1) % 1_000_000));
    let code = run_with(&argv("run --builtin dot --sim --allow-emission"), &mut term, &mut env);
    let out = term.output();
    assert_eq!(code, EXIT_FAIL, "{out}");
    assert!(out.contains("arm_text_mismatch"), "{out}");
    assert_eq!(term.challenges.len(), 1);
    assert!(!has(&calls(&s), "run_armed"));
}

#[test]
fn cli_correct_challenge_arms_and_marks() {
    let (mut env, s) = fake_env(one_pass_on_poll_3(1.0, None));
    let mut term = Scripted::new(true, &[Key::Wait(4), Key::Enter]);
    term.typing = Some(|c| format!("ARM {c:06}"));
    let code = run_with(&argv("run --builtin dot --sim --allow-emission"), &mut term, &mut env);
    let out = term.output();
    assert_eq!(code, EXIT_OK, "{out}");
    assert!(has(&calls(&s), "run_armed"), "{:?}", calls(&s));
    assert!(out.contains("Finished in"), "{out}");
}

#[test]
fn status_reports_the_probe() {
    let (mut env, _) = fake_env(Script::default());
    let mut term = Scripted::new(false, &[]);
    assert_eq!(run_with(&argv("status --sim"), &mut term, &mut env), EXIT_OK);
    assert!(term.output().contains("serial: TEST"));
}

#[test]
fn usage_errors_exit_2() {
    let (mut env, s) = fake_env(Script::default());
    for bad in ["", "mark card.olp", "run --builtin corners --extent 110 --sim", "run card.olp --force", "frame"] {
        let mut term = Scripted::new(true, &[]);
        assert_eq!(run_with(&argv(bad), &mut term, &mut env), EXIT_USAGE, "`{bad}`");
        assert!(term.output().contains("usage:"), "`{bad}`");
    }
    assert_eq!(s.lock().unwrap().opened, 0);
}

// ---- goldens -----------------------------------------------------------------------------------------------------

fn golden(oracle: &str, scenario: &str) -> Value {
    let p = openlaser_testkit::golden_dir().join("t1_lmc").join(oracle).join(format!("{scenario}.json"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap()
}

#[test]
fn cli_builtin_box10_matches_t1_inputs() {
    for b in [Builtin::Box10, Builtin::Dot, Builtin::Line] {
        let scenario = b.t1_scenario().unwrap();
        for oracle in ["v0", "v1"] {
            let inputs = &golden(oracle, scenario)["inputs"];
            let p = b.pattern(None);
            let polylines: Vec<Vec<[f64; 2]>> = serde_json::from_value(inputs["polylines"].clone()).unwrap();
            assert_eq!(p.polylines, polylines, "{scenario} {oracle}");
            assert_eq!(p.power_pct, inputs["power"].as_f64().unwrap(), "{scenario}");
            assert_eq!(p.frequency_khz, inputs["frequency_khz"].as_f64().unwrap(), "{scenario}");
            assert_eq!(p.pulse_ns, inputs["pulse_width_ns"].as_f64().unwrap(), "{scenario}");
            assert_eq!(p.speed_mm_s, inputs["speed_mm_s"].as_f64().unwrap(), "{scenario}");
            // The engine commands carry the same numbers.
            let cmds = p.commands(b.name(), &MachineId("jpt-m7-200".into()), None);
            let set = &cmds.iter().find(|(id, _)| *id == "layer.cutSettings.set").unwrap().1["galvo"];
            assert_eq!(set["powerPct"].as_f64(), inputs["power"].as_f64());
            assert_eq!(set["frequencyKhz"].as_f64(), inputs["frequency_khz"].as_f64());
            assert_eq!(set["qPulseNs"].as_f64(), inputs["pulse_width_ns"].as_f64());
            assert_eq!(set["speedMmS"].as_f64(), inputs["speed_mm_s"].as_f64());
            let drawn: Vec<Value> = cmds.iter().filter(|(id, _)| *id == "tools.drawLines").map(|(_, p)| p["pointsMm"].clone()).collect();
            assert_eq!(drawn, inputs["polylines"].as_array().unwrap().clone());
        }
    }
    // frame_box10 traces the same box.
    assert_eq!(
        serde_json::from_value::<Vec<Vec<[f64; 2]>>>(golden("v1", "frame_box10")["inputs"]["polylines"].clone()).unwrap(),
        Builtin::Box10.pattern(None).polylines
    );
}

#[test]
fn cli_trace_file_matches_t1_event_format() {
    let dir = tempfile::tempdir().unwrap();
    let mut scenarios = 0;
    let dirs = ["v0", "v1"].map(|o| openlaser_testkit::golden_dir().join("t1_lmc").join(o));
    for entry in dirs.iter().flat_map(|d| std::fs::read_dir(d).unwrap()) {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let g: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let events = g["events"].as_array().unwrap();
        let out = dir.path().join(format!("{}-{}.jsonl", g["meta"]["oracle"].as_str().unwrap_or("x"), g["scenario"].as_str().unwrap()));
        let mut w = TraceWriter::create(&out).unwrap();
        for (i, e) in events.iter().enumerate() {
            let ev = from_t1(e).unwrap_or_else(|| panic!("{}: event {i} {e}", path.display()));
            w.write(&ev, MonoTime(Duration::from_millis(u64::try_from(i).unwrap()))).unwrap();
        }
        let text = std::fs::read_to_string(&out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), events.len());
        for (i, (l, e)) in lines.iter().zip(events).enumerate() {
            let mut v: Value = serde_json::from_str(l).unwrap();
            let at = v.as_object_mut().unwrap().remove("atS").unwrap();
            assert_eq!(at.as_f64().unwrap(), i as f64 / 1000.0);
            assert_eq!(&v, e, "{} line {i}", path.display());
        }
        scenarios += 1;
    }
    assert!(scenarios >= 20, "every v0 and v1 scenario was replayed ({scenarios})");
}

// ---- the engine, the planner, the encoder and the driver on the simulator -----------------------------------------

/// The engine rig on the simulator, with its own empty profile folder: never the user's real one, and never `None`
/// (a machine whose saved profile cannot be read is refused, so `None`, "no profile folder on this system", refuses
/// every device command). Keep the `TempDir` alive as long as the `Env`.
fn engine_env(sim: openlaser_lmc_sim::SimConfig) -> (Env, tempfile::TempDir) {
    let clock: Arc<dyn openlaser_types::Clock> = Arc::new(openlaser_types::SystemClock::new());
    let profiles = tempfile::tempdir().unwrap();
    let dir = profiles.path().join("machines");
    let env = Env {
        profiles_dir: Some(dir.clone()),
        clock: Arc::clone(&clock),
        poll: Duration::from_millis(5),
        signals: false,
        exit: Arc::new(|_| {}),
        rigs: Box::new(EngineRigs::new(Some(dir), sim, clock)),
    };
    (env, profiles)
}

/// Presses Enter every second for a minute: the first one after an executed pass is accepted, the earlier ones are
/// refused (as a person would retry), and the rest are ignored while marking.
fn enter_after_frame() -> Vec<Key> {
    (0..60).flat_map(|_| [Key::Wait(200), Key::Enter]).collect()
}

#[test]
fn cli_run_sim_marks_business_card() {
    let card = openlaser_testkit::golden_dir().join("inputs").join("workspace.json");
    let (mut env, _profiles) = engine_env(default_sim_config());
    let mut term = Scripted::new(true, &enter_after_frame());
    term.typing = Some(|c| format!("ARM {c:06}"));
    let code = run_with(&[String::from("run"), card.to_string_lossy().into_owned(), "--sim".into(), "--allow-emission".into()], &mut term, &mut env);
    let out = term.output();
    assert_eq!(code, EXIT_OK, "{out}");
    assert!(out.contains("Finished in"), "{out}");
    assert_eq!(term.challenges.len(), 1);
}

#[test]
fn cli_first_emission_power_cap_refused_on_the_simulator() {
    let mut sim = default_sim_config();
    sim.emission_cap_pct = Some(10.0);
    let (mut env, _profiles) = engine_env(sim);
    let mut term = Scripted::new(true, &enter_after_frame());
    term.typing = Some(|c| format!("ARM {c:06}"));
    // box10 is the T1 input at 50 %.
    let code = run_with(&argv("run --builtin box10 --sim --allow-emission"), &mut term, &mut env);
    let out = term.output();
    assert_eq!(code, EXIT_FAIL, "{out}");
    assert!(out.contains("bench_power_cap"), "{out}");
    assert!(term.challenges.is_empty());
    // At 5 % it goes through.
    let mut term = Scripted::new(true, &enter_after_frame());
    term.typing = Some(|c| format!("ARM {c:06}"));
    assert_eq!(run_with(&argv("run --builtin box10 --power-pct 5 --sim --allow-emission"), &mut term, &mut env), EXIT_OK, "{}", term.output());
}

#[test]
fn cli_trace_sim_frame_writes_t1_lines() {
    let dir = tempfile::tempdir().unwrap();
    let trace: PathBuf = dir.path().join("frame.jsonl");
    let (mut env, _profiles) = engine_env(default_sim_config());
    let mut term = Scripted::new(true, &[]);
    let args = vec!["frame".into(), "--builtin".into(), "box10".into(), "--sim".into(), "--trace".into(), trace.to_string_lossy().into_owned()];
    assert_eq!(run_with(&args, &mut term, &mut env), EXIT_OK, "{}", term.output());
    let text = std::fs::read_to_string(&trace).unwrap();
    let kinds: Vec<String> = text.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["kind"].as_str().unwrap().to_owned()).collect();
    assert_eq!(kinds.first().map(String::as_str), Some("open"));
    assert_eq!(kinds.last().map(String::as_str), Some("close"));
    assert!(kinds.iter().any(|k| k == "list"), "the lighting list went out");
    assert!(kinds.iter().all(|k| ["open", "single", "read", "list", "close"].contains(&k.as_str())));
}

#[test]
fn cli_frame_extent_reaches_past_the_work_area_on_red_light_only() {
    let (mut env, _profiles) = engine_env(default_sim_config());
    let mut term = Scripted::new(true, &[]);
    assert_eq!(run_with(&argv("frame --builtin corners --extent 110 --sim"), &mut term, &mut env), EXIT_OK, "{}", term.output());
    // 118.34 is +F/2 at scale 1.1834: never encodable.
    let mut term = Scripted::new(true, &[]);
    assert_eq!(run_with(&argv("frame --builtin corners --extent 118.34 --sim"), &mut term, &mut env), EXIT_FAIL);
    assert!(term.output().contains("not_encodable"), "{}", term.output());
}

#[test]
fn cli_skipped_profile_file_refuses_device_commands() {
    // A saved profile that no longer loads would leave the machine on its built-in calibration: refused, not run.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("jpt-m7-200-sim.json"), b"{ \"id\": \"jpt-m7-200-sim\", ").unwrap();
    let (mut env, _profiles) = engine_env(default_sim_config());
    env.rigs = Box::new(EngineRigs::new(Some(dir.path().to_path_buf()), default_sim_config(), Arc::clone(&env.clock)));
    let mut term = Scripted::new(true, &[]);
    assert_eq!(run_with(&argv("frame --builtin box10 --sim"), &mut term, &mut env), EXIT_FAIL);
    let out = term.output();
    assert!(out.contains("machine profile files were skipped"), "{out}");
    assert!(!out.contains("Framing"), "{out}");
}

#[test]
fn rotary_commands_require_tty_and_enabled_profile() {
    for sub in ["rotary move --deg 90 --sim", "rotary pos --sim"] {
        let (mut env, s) = fake_env(Script::default());
        let mut term = Scripted::new(false, &[]);
        assert_eq!(run_with(&argv(sub), &mut term, &mut env), EXIT_FAIL);
        assert!(term.output().contains("not_a_tty"));
        assert_eq!(s.lock().unwrap().opened, 0);
        let (mut env, _profiles) = engine_env(default_sim_config());
        let mut term = Scripted::new(true, &[]);
        assert_eq!(run_with(&argv(sub), &mut term, &mut env), EXIT_FAIL);
        assert!(term.output().contains("rotary.enabled"), "{}", term.output());
    }
}
