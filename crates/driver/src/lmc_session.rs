//! The LMC session (SPEC §5.3, §5.4; red zone): flow control, the run-ahead window, the watchdog, executed-pass
//! detection and the emergency stop, written once over the [`Wire`] seam so the same code drives a real
//! `LmcTransport` and the scripted board of the unit tests.
//!
//! The sequencing follows galvoplotter's marking session (meerk40t/galvoplotter@fc35136 `galvo/controller.py`,
//! MIT, © 2023 MeerK40t: prelude, packets with `SetEndOfList` after each, `ExecuteList` after the 3rd packet,
//! wait idle, postlude; the abort order of `abort()`), as research 05 §1.5 and §1.9 record it, with the OpenLaser
//! changes of PLAN §3.5 and SPEC §5.3: no wait ever proceeds on a deadline, the board is never written while not
//! READY, MO closes only after !BUSY, a progress-aware watchdog stops a stuck board, and no transfer is retried.
//! Nothing from galvoplotter's USB layer (resets, retries) is ported.

use std::time::Duration;

use openlaser_lmc::{GalvoMachine, Single, StatusWord, XferError, status_from_reply};
use openlaser_types::MonoTime;

use crate::backend::{DriverError, DriverEvent};
use crate::lmc_ops::{self, EXECUTE_LIST, FIBER_SET_MO, GET_VERSION, RESET_LIST, SET_END_OF_LIST, STOP_EXECUTE, WRITE_PORT};
use crate::stop::{Cx, MAX_SLICE, StopReport, Stopped};

/// How often a waiting session polls the status word (§5.3 "poll ≤ 10 ms").
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(10);
/// GP parity: `ExecuteList` after the 3rd packet at the latest (research 05 §1.5).
pub(crate) const EXECUTE_AFTER_PACKETS: usize = 3;
/// Settle after the init sequence (research 05 §1.4).
pub(crate) const INIT_SETTLE: Duration = Duration::from_millis(50);
/// The in-flight packet gets this long to complete during a stop before `cancel_all` (§5.4).
pub(crate) const IN_FLIGHT_GRACE: Duration = Duration::from_millis(20);
/// The dummy end-of-list packet is written only if READY shows within this (§5.3).
pub(crate) const ABORT_READY_WAIT: Duration = Duration::from_millis(50);
/// The confirming status read waits this long for !BUSY (§5.4 "Confirm ≤ 50 ms").
pub(crate) const CONFIRM_WAIT: Duration = Duration::from_millis(50);
/// The whole emergency sequence stays inside the 500 ms stop watchdog: once this is spent, every remaining transfer
/// (MO close and ports zero are always attempted) gets one slice.
pub(crate) const EMERGENCY_BUDGET: Duration = Duration::from_millis(400);
/// The watchdog budget never exceeds this, whatever the estimates say.
const WATCHDOG_CAP: Duration = Duration::from_secs(24 * 3600);

/// The USB-shaped seam the session drives: `LmcTransport` with the packet type left open.
pub(crate) trait Wire: Send {
    type Pkt;
    fn submit_single(&mut self, cmd: Single) -> Result<(), XferError>;
    fn poll_single(&mut self, slice: Duration) -> Result<Option<Option<[u16; 4]>>, XferError>;
    fn submit_packet(&mut self, p: &Self::Pkt) -> Result<(), XferError>;
    fn poll_packet(&mut self, slice: Duration) -> Result<Option<()>, XferError>;
    fn cancel_all(&mut self);
    fn drain_replies(&mut self);
    /// Short text for the Console trace.
    fn describe_packet(p: &Self::Pkt) -> String;
}

/// The profile numbers the session runs by, checked once.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SessionCfg {
    pub(crate) transfer_timeout: Duration,
    pub(crate) max_packets: usize,
    pub(crate) max_emission: Duration,
    pub(crate) watchdog_factor: f64,
    pub(crate) watchdog_slack: Duration,
}

impl SessionCfg {
    /// Refuses a profile outside `machine::validate_profile`'s ranges instead of guessing.
    pub(crate) fn from_machine(m: &GalvoMachine) -> Result<Self, String> {
        let ra = &m.run_ahead;
        if !(20..=2000).contains(&m.transfer_timeout_ms) {
            return Err(format!("transfer_timeout_ms = {} is outside 20..=2000", m.transfer_timeout_ms));
        }
        if ra.max_packets < 1 {
            return Err("run_ahead.max_packets must be at least 1".to_owned());
        }
        let secs = |what: &str, v: f64, lo: f64, hi: f64| -> Result<Duration, String> {
            if !v.is_finite() || v < lo || v > hi {
                return Err(format!("{what} = {v} is outside {lo}..={hi} s"));
            }
            Duration::try_from_secs_f64(v).map_err(|e| format!("{what} = {v}: {e}"))
        };
        let max_emission = secs("run_ahead.max_emission_s", ra.max_emission_s, 0.001, 600.0)?;
        let watchdog_slack = secs("run_ahead.watchdog_slack_s", ra.watchdog_slack_s, 0.0, 600.0)?;
        if !ra.watchdog_factor.is_finite() || !(1.0..=100.0).contains(&ra.watchdog_factor) {
            return Err(format!("run_ahead.watchdog_factor = {} is outside 1..=100", ra.watchdog_factor));
        }
        Ok(SessionCfg {
            transfer_timeout: Duration::from_millis(u64::from(m.transfer_timeout_ms)),
            max_packets: usize::from(ra.max_packets),
            max_emission,
            watchdog_factor: ra.watchdog_factor,
            watchdog_slack,
        })
    }

    /// Used only to run the emergency sequence for a profile that failed [`SessionCfg::from_machine`].
    pub(crate) fn fallback() -> Self {
        SessionCfg {
            transfer_timeout: Duration::from_millis(250),
            max_packets: 1,
            max_emission: Duration::from_secs(2),
            watchdog_factor: 1.5,
            watchdog_slack: Duration::from_secs(2),
        }
    }
}

/// Why an operation ended early.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Halt {
    /// The latch tripped: the caller runs [`Session::emergency`].
    Stopped,
    /// Transfer error or timeout, watchdog, protocol: sticky fault, then the runtime's emergency stop.
    Fault(DriverError),
}

impl From<Stopped> for Halt {
    fn from(_: Stopped) -> Self {
        Halt::Stopped
    }
}

#[derive(Debug, Clone, Copy)]
enum Mode {
    /// Stop-aware: the latch is checked between slices.
    Live,
    /// Ignores the latch (status reads, disconnect).
    Plain,
    /// The emergency sequence: ignores the latch, shares one overall deadline.
    Abort { deadline: MonoTime },
}

/// One list (one layer-pass, or one frame list) as the session sends it.
pub(crate) struct ListSpec<'a, P> {
    pub(crate) prelude: &'a [Single],
    pub(crate) packets: &'a [P],
    pub(crate) est: &'a [Duration],
    pub(crate) postlude: &'a [Single],
    pub(crate) label: String,
}

/// A whole burn (`LmcJob`) as the session sends it.
pub(crate) struct JobSpec<'a, P> {
    pub(crate) prelude: &'a [Single],
    pub(crate) lists: Vec<(String, &'a [P], &'a [Duration])>,
    pub(crate) postlude: &'a [Single],
    pub(crate) repeats: u16,
}

/// A red-light frame (`LmcFrameJob`) as the session sends it.
pub(crate) struct FrameSpec<'a, P> {
    pub(crate) once: &'a [P],
    pub(crate) once_est: Duration,
    pub(crate) once_passes: u32,
    pub(crate) pass: &'a [P],
    pub(crate) pass_est: Duration,
}

/// Running-state progress across a whole run.
pub(crate) struct Progress {
    pub(crate) acked: u32,
    pub(crate) total: u32,
    /// Estimate of every packet not yet sent.
    pub(crate) unsent: Duration,
}

impl Progress {
    fn packet_done(&mut self, cx: &Cx, est: Duration, queued: Duration) {
        self.acked = self.acked.saturating_add(1);
        self.unsent = self.unsent.saturating_sub(est);
        let eta_ms = u64::try_from(self.unsent.saturating_add(queued).as_millis()).unwrap_or(u64::MAX);
        cx.emit(DriverEvent::Progress { packets_acked: self.acked, total: self.total, eta_ms });
    }
}

/// The run-ahead window (§5.3), from the galvo time model. Before `ExecuteList` nothing executes. At `ExecuteList`
/// the packets already sent are scheduled back to back; a packet sent later starts when the model says the board
/// finished the one before it, or when it is sent if the board had already run dry (never retroactively).
///
/// The next packet may be sent when the model says the board holds nothing ("never < 1 packet"), or when fewer than
/// `max_packets` sent packets are unexecuted and the queued estimate plus the next packet stays within
/// `max_emission`. So a single packet longer than `max_emission` is still sent, alone (the slow-packet case of
/// `t6_slow_packets_execute_without_deadlock`), and the post-crash bound is max(`max_emission`, the longest packet).
/// SPEC §5.3 also floors `queued` at the newest packet's estimate; applied to the window that floor would keep
/// `queued ≤ 0` from ever holding and deadlock slow packets, so it is applied to the watchdog only
/// ([`Window::watched`]), where it means "the board may still hold at least the packet it was sent last".
#[derive(Debug, Default)]
struct Window {
    est: Vec<Duration>,
    /// Model completion time of each sent packet, once `ExecuteList` was sent.
    done_at: Vec<MonoTime>,
    exec_at: Option<MonoTime>,
}

impl Window {
    fn push(&mut self, now: MonoTime, est: Duration) {
        self.est.push(est);
        if self.exec_at.is_some() {
            let start = self.done_at.last().copied().filter(|t| *t > now).unwrap_or(now);
            self.done_at.push(start.plus(est));
        }
    }

    fn execute(&mut self, now: MonoTime) {
        self.exec_at = Some(now);
        self.done_at.clear();
        let mut cursor = now;
        for e in &self.est {
            cursor = cursor.plus(*e);
            self.done_at.push(cursor);
        }
    }

    fn len(&self) -> usize {
        self.est.len()
    }

    fn queued(&self, now: MonoTime) -> Duration {
        match self.exec_at {
            None => self.est.iter().sum(),
            Some(_) => self.done_at.last().map(|t| t.saturating_since(now)).unwrap_or(Duration::ZERO),
        }
    }

    fn unexecuted(&self, now: MonoTime) -> usize {
        match self.exec_at {
            None => self.est.len(),
            Some(_) => self.done_at.iter().filter(|t| **t > now).count(),
        }
    }

    /// The model says the board has run every packet sent so far.
    fn model_finished(&self, now: MonoTime) -> bool {
        self.exec_at.is_some() && self.done_at.last().is_none_or(|t| *t <= now)
    }

    /// What may still be queued, for the watchdog: nothing before `ExecuteList` (a board that never raises READY
    /// is stopped within the slack), afterwards the model's queue but at least the newest packet.
    fn watched(&self, now: MonoTime) -> Duration {
        match self.exec_at {
            None => Duration::ZERO,
            Some(_) => self.queued(now).max(self.est.last().copied().unwrap_or(Duration::ZERO)),
        }
    }

    fn may_send(&self, now: MonoTime, next: Duration, cfg: &SessionCfg) -> bool {
        let queued = self.queued(now);
        if queued.is_zero() {
            return true;
        }
        self.unexecuted(now) < cfg.max_packets && queued.saturating_add(next) <= cfg.max_emission
    }
}

/// §5.3 watchdog: progress is a valid status read whose READY or BUSY bit changed, or a packet transfer completing.
/// Without progress for est(queued) × factor + slack the board is stopped. Failed reads are faults, never progress.
#[derive(Debug)]
struct Watchdog {
    factor: f64,
    slack: Duration,
    last_progress: MonoTime,
    deadline: MonoTime,
    last_bits: Option<u16>,
}

impl Watchdog {
    fn start(now: MonoTime, cfg: &SessionCfg) -> Self {
        Watchdog { factor: cfg.watchdog_factor, slack: cfg.watchdog_slack, last_progress: now, deadline: now.plus(cfg.watchdog_slack), last_bits: None }
    }

    fn budget(&self, queued: Duration) -> Duration {
        let scaled = Duration::try_from_secs_f64(queued.as_secs_f64() * self.factor).unwrap_or(WATCHDOG_CAP);
        scaled.saturating_add(self.slack).min(WATCHDOG_CAP)
    }

    fn progress(&mut self, now: MonoTime, queued: Duration) {
        self.last_progress = now;
        self.deadline = now.plus(self.budget(queued));
    }

    /// `ExecuteList` starts the time model: the queued estimate becomes the budget (never shortens it).
    fn extend(&mut self, now: MonoTime, queued: Duration) {
        let d = now.plus(self.budget(queued));
        if d > self.deadline {
            self.deadline = d;
        }
    }

    fn observe(&mut self, now: MonoTime, s: StatusWord, queued: Duration) {
        let bits = s.0 & (StatusWord::READY | StatusWord::BUSY);
        if self.last_bits.is_some_and(|b| b != bits) {
            self.progress(now, queued);
        }
        self.last_bits = Some(bits);
    }

    fn check(&self, now: MonoTime) -> Result<(), Halt> {
        if now > self.deadline {
            let waited_ms = u64::try_from(now.saturating_since(self.last_progress).as_millis()).unwrap_or(u64::MAX);
            return Err(Halt::Fault(DriverError::Watchdog { waited_ms }));
        }
        Ok(())
    }
}

fn xfer_fault(what: &str, e: &XferError) -> Halt {
    Halt::Fault(DriverError::Transport(format!("{what}: {e}")))
}

fn ms(d: Duration) -> u128 {
    d.as_millis()
}

/// One connection's session state. The wire is owned here; nothing else touches it.
pub(crate) struct Session<W: Wire> {
    pub(crate) wire: W,
    cfg: SessionCfg,
    abort_packet: Option<W::Pkt>,
    packet_in_flight: bool,
    read_abandoned: bool,
    /// `ExecuteList` was sent for the current list and its !BUSY not yet seen (OM v1 `_list_executing`).
    list_executing: bool,
    /// `Fiber_SetMo(1)` was sent and no `Fiber_SetMo(0)` acknowledged since.
    mo_open: bool,
}

impl<W: Wire> Session<W> {
    pub(crate) fn new(wire: W, cfg: SessionCfg, abort_packet: Option<W::Pkt>) -> Self {
        Session { wire, cfg, abort_packet, packet_in_flight: false, read_abandoned: false, list_executing: false, mo_open: false }
    }

    fn budget(&self, mode: Mode, now: MonoTime) -> Duration {
        match mode {
            Mode::Live | Mode::Plain => self.cfg.transfer_timeout,
            Mode::Abort { deadline } => self.cfg.transfer_timeout.min(deadline.saturating_since(now).max(MAX_SLICE)),
        }
    }

    /// One single command: write, then the 8-byte reply, in ≤ 20 ms slices within the transfer budget. Never retried.
    fn single(&mut self, cmd: Single, cx: &Cx, mode: Mode) -> Result<Option<[u16; 4]>, Halt> {
        if matches!(mode, Mode::Live) {
            cx.check()?;
        }
        let op = lmc_ops::opcode(&cmd);
        let name = lmc_ops::single_name(op);
        let [_, p1, ..] = cmd.0;
        cx.trace(true, lmc_ops::describe(&cmd));
        if op == EXECUTE_LIST {
            // Conservative: once ExecuteList may have reached the board, the abort must not send another.
            self.list_executing = true;
        }
        if op == FIBER_SET_MO && p1 != 0 {
            self.mo_open = true;
        }
        self.wire.submit_single(cmd).map_err(|e| xfer_fault(name, &e))?;
        let budget = self.budget(mode, cx.now());
        let start = cx.now();
        let mut polls: u32 = 0;
        loop {
            match self.wire.poll_single(MAX_SLICE) {
                Err(e) => {
                    self.read_abandoned = true;
                    return Err(xfer_fault(name, &e));
                }
                Ok(Some(reply)) => {
                    self.read_abandoned = false;
                    if let Some(r) = reply
                        && op == GET_VERSION
                    {
                        cx.trace(false, format!("status 0x{:04X}", status_from_reply(r).0));
                    }
                    if op == FIBER_SET_MO && p1 == 0 {
                        self.mo_open = false;
                    }
                    return Ok(reply);
                }
                Ok(None) => {
                    polls = polls.saturating_add(1);
                    let waited = cx.now().saturating_since(start).max(MAX_SLICE.saturating_mul(polls));
                    if waited >= budget {
                        self.read_abandoned = true;
                        let msg = format!("{name}: no reply within {} ms (not retried)", ms(budget));
                        return Err(Halt::Fault(DriverError::Transport(msg)));
                    }
                    if matches!(mode, Mode::Live) && cx.check().is_err() {
                        self.read_abandoned = true;
                        return Err(Halt::Stopped);
                    }
                }
            }
        }
    }

    /// One 3072-byte list packet; at most one in flight.
    fn packet(&mut self, p: &W::Pkt, label: &str, cx: &Cx, mode: Mode) -> Result<(), Halt> {
        if matches!(mode, Mode::Live) {
            cx.check()?;
        }
        cx.trace(true, format!("{label} {}", W::describe_packet(p)));
        self.wire.submit_packet(p).map_err(|e| xfer_fault("list packet", &e))?;
        self.packet_in_flight = true;
        let budget = self.budget(mode, cx.now());
        let start = cx.now();
        let mut polls: u32 = 0;
        loop {
            match self.wire.poll_packet(MAX_SLICE) {
                Err(e) => return Err(xfer_fault("list packet", &e)),
                Ok(Some(())) => {
                    self.packet_in_flight = false;
                    return Ok(());
                }
                Ok(None) => {
                    polls = polls.saturating_add(1);
                    let waited = cx.now().saturating_since(start).max(MAX_SLICE.saturating_mul(polls));
                    if waited >= budget {
                        return Err(Halt::Fault(DriverError::Transport(format!("list packet: not completed within {} ms (not retried)", ms(budget)))));
                    }
                    if matches!(mode, Mode::Live) && cx.check().is_err() {
                        return Err(Halt::Stopped);
                    }
                }
            }
        }
    }

    /// GetVersion word 3. A failed read is an error, never status bits.
    fn status_word(&mut self, cx: &Cx, mode: Mode) -> Result<StatusWord, Halt> {
        match self.single(lmc_ops::get_version(), cx, mode)? {
            Some(r) => Ok(status_from_reply(r)),
            None => Err(Halt::Fault(DriverError::Protocol("GetVersion returned no status reply".to_owned()))),
        }
    }

    /// A status read that ignores the latch (`Backend::status`).
    pub(crate) fn read_status(&mut self, cx: &Cx) -> Result<StatusWord, Halt> {
        self.status_word(cx, Mode::Plain)
    }

    /// Connect: the init sequence verbatim (research 05 §1.4), then the 50 ms settle. Returns the first GetVersion
    /// reply (the board version words).
    pub(crate) fn connect(&mut self, init: &[Single], cx: &Cx) -> Result<Option<[u16; 4]>, Halt> {
        if init.is_empty() {
            return Err(Halt::Fault(DriverError::Protocol("the LMC init sequence is empty: refusing to drive an uninitialised board".to_owned())));
        }
        let mut version = None;
        for cmd in init {
            let reply = self.single(*cmd, cx, Mode::Live)?;
            if lmc_ops::opcode(cmd) == GET_VERSION && version.is_none() {
                version = reply;
            }
        }
        let mut left = INIT_SETTLE;
        while !left.is_zero() {
            let s = left.min(MAX_SLICE);
            cx.slice(s)?;
            left = left.saturating_sub(s);
        }
        self.list_executing = false;
        Ok(version)
    }

    fn execute_list(&mut self, cx: &Cx, win: &mut Window, wd: &mut Watchdog) -> Result<(), Halt> {
        self.single(lmc_ops::single(EXECUTE_LIST, 0), cx, Mode::Live)?;
        let now = cx.now();
        win.execute(now);
        wd.extend(now, win.watched(now));
        Ok(())
    }

    /// One list, start to executed (§5.3): prelude · per packet (wait READY and the window; submit; SetEndOfList;
    /// ExecuteList at the first trigger) · wait until executed · postlude.
    ///
    /// "Executed" (S20) = a valid status read shows !BUSY after `ExecuteList`, and either BUSY was seen since, or the
    /// time model says the list's estimate has elapsed: a board that had not raised BUSY yet when first polled is
    /// never taken for finished.
    pub(crate) fn run_list(&mut self, spec: &ListSpec<'_, W::Pkt>, cx: &Cx, mut progress: Option<&mut Progress>) -> Result<(), Halt> {
        let n = spec.packets.len();
        if n == 0 || spec.est.len() != n {
            return Err(Halt::Fault(DriverError::Protocol(format!("{}: {n} packets and {} estimates", spec.label, spec.est.len()))));
        }
        let mut wd = Watchdog::start(cx.now(), &self.cfg);
        let mut win = Window::default();
        self.list_executing = false;
        let mut fresh: Option<StatusWord> = None;

        for cmd in spec.prelude {
            let reply = self.single(*cmd, cx, Mode::Live)?;
            if lmc_ops::opcode(cmd) == GET_VERSION {
                let s = reply.map(status_from_reply).ok_or_else(|| Halt::Fault(DriverError::Protocol("GetVersion returned no status reply".to_owned())))?;
                wd.observe(cx.now(), s, Duration::ZERO);
                fresh = Some(s);
            }
        }

        for (i, (pkt, est)) in spec.packets.iter().zip(spec.est).enumerate() {
            // Never write unless the most recent status had READY set; never proceed on a deadline.
            loop {
                cx.check()?;
                wd.check(cx.now())?;
                let s = match fresh.take() {
                    Some(s) => s,
                    None => {
                        let s = self.status_word(cx, Mode::Live)?;
                        wd.observe(cx.now(), s, win.watched(cx.now()));
                        s
                    }
                };
                if s.ready() && win.may_send(cx.now(), *est, &self.cfg) {
                    break;
                }
                if !s.ready() && !self.list_executing && win.len() > 0 {
                    // The board's buffer is full: it will not take more until it executes.
                    self.execute_list(cx, &mut win, &mut wd)?;
                }
                cx.slice(POLL_INTERVAL)?;
            }
            let label = format!("{} packet {}/{n}", spec.label, i + 1);
            self.packet(pkt, &label, cx, Mode::Live)?;
            let now = cx.now();
            win.push(now, *est);
            wd.progress(now, win.watched(now));
            if let Some(p) = progress.as_deref_mut() {
                p.packet_done(cx, *est, win.queued(now));
            }
            self.single(lmc_ops::single(SET_END_OF_LIST, 0), cx, Mode::Live)?;
            if !self.list_executing {
                let last = i + 1 == n;
                let gp_third = win.len() >= EXECUTE_AFTER_PACKETS;
                let window_full = spec.est.get(i + 1).is_some_and(|next| !win.may_send(cx.now(), *next, &self.cfg));
                if last || gp_third || window_full {
                    self.execute_list(cx, &mut win, &mut wd)?;
                }
            }
        }

        let mut busy_seen = false;
        loop {
            cx.check()?;
            wd.check(cx.now())?;
            let s = self.status_word(cx, Mode::Live)?;
            wd.observe(cx.now(), s, win.watched(cx.now()));
            busy_seen |= s.busy();
            let model_done = win.model_finished(cx.now());
            if !s.busy() && (busy_seen || model_done) {
                break;
            }
            cx.slice(POLL_INTERVAL)?;
        }
        self.list_executing = false;

        for cmd in spec.postlude {
            self.single(*cmd, cx, Mode::Live)?;
        }
        Ok(())
    }

    /// A whole burn: every list of every repeat, the stop checked between lists (§5.3).
    pub(crate) fn run_job(&mut self, job: &JobSpec<'_, W::Pkt>, cx: &Cx, progress: &mut Progress) -> Result<(), Halt> {
        for repeat in 0..job.repeats {
            for (label, packets, est) in &job.lists {
                cx.check()?;
                let label = if job.repeats > 1 { format!("repeat {}/{} {label}", repeat + 1, job.repeats) } else { label.clone() };
                let spec = ListSpec { prelude: job.prelude, packets, est, postlude: job.postlude, label };
                self.run_list(&spec, cx, Some(progress))?;
            }
        }
        Ok(())
    }

    /// A lighting list: ResetList · GetVersion · packets (same flow control) · executed (§5.3 frame).
    fn frame_list(&mut self, packets: &[W::Pkt], total: Duration, label: String, cx: &Cx) -> Result<(), Halt> {
        let n = u32::try_from(packets.len()).unwrap_or(u32::MAX).max(1);
        let est = vec![total / n; packets.len()];
        let prelude = [lmc_ops::single(RESET_LIST, 0), lmc_ops::get_version()];
        self.run_list(&ListSpec { prelude: &prelude, packets, est: &est, postlude: &[], label }, cx, None)
    }

    /// Frame Once: one list, `on_pass(once_passes)` after it executed, then MO(0) · WritePort(0).
    /// Frame Loop: the one-pass list again and again, `on_pass(k)` after pass k executed, until stopped (S20).
    pub(crate) fn frame(&mut self, f: &FrameSpec<'_, W::Pkt>, once: bool, cx: &Cx, on_pass: &mut dyn FnMut(&Cx, u32)) -> Result<u32, Halt> {
        if once {
            self.frame_list(f.once, f.once_est, "frame once".to_owned(), cx)?;
            on_pass(cx, f.once_passes);
            self.single(lmc_ops::single(FIBER_SET_MO, 0), cx, Mode::Live)?;
            self.single(lmc_ops::single(WRITE_PORT, 0), cx, Mode::Live)?;
            return Ok(f.once_passes);
        }
        let mut passes: u32 = 0;
        loop {
            self.frame_list(f.pass, f.pass_est, format!("frame pass {}", passes.saturating_add(1)), cx)?;
            passes = passes.saturating_add(1);
            on_pass(cx, passes);
        }
    }

    /// MO(0) · WritePort(0), ignoring the latch, errors traced (disconnect).
    pub(crate) fn park(&mut self, cx: &Cx) {
        for cmd in [lmc_ops::single(FIBER_SET_MO, 0), lmc_ops::single(WRITE_PORT, 0)] {
            if let Err(Halt::Fault(e)) = self.single(cmd, cx, Mode::Plain) {
                cx.trace(false, format!("{}: {e}", lmc_ops::describe(&cmd)));
            }
        }
    }

    fn abort_single(&mut self, cmd: Single, cx: &Cx, mode: Mode, notes: &mut Vec<String>) -> bool {
        match self.single(cmd, cx, mode) {
            Ok(_) => true,
            Err(Halt::Fault(e)) => {
                notes.push(format!("{}: {e}", lmc_ops::describe(&cmd)));
                false
            }
            Err(Halt::Stopped) => false,
        }
    }

    /// Polls the status (ignoring the latch) until `want` holds or `wait` has passed.
    fn wait_status(&mut self, cx: &Cx, mode: Mode, wait: Duration, want: fn(StatusWord) -> bool, notes: &mut Vec<String>) -> Option<StatusWord> {
        let start = cx.now();
        let mut polls: u32 = 0;
        loop {
            if self.read_abandoned {
                self.wire.drain_replies();
                self.read_abandoned = false;
            }
            match self.status_word(cx, mode) {
                Ok(s) if want(s) => return Some(s),
                Ok(_) => {}
                Err(Halt::Fault(e)) => {
                    notes.push(format!("status read failed: {e}"));
                    return None;
                }
                Err(Halt::Stopped) => return None,
            }
            polls = polls.saturating_add(1);
            if cx.now().saturating_since(start).max(POLL_INTERVAL.saturating_mul(polls)) >= wait {
                return None;
            }
            cx.pause(POLL_INTERVAL);
        }
    }

    /// The in-flight packet gets ≤ 20 ms to complete (it was submitted only when READY).
    fn finish_in_flight(&mut self, cx: &Cx) -> bool {
        let start = cx.now();
        let mut polls: u32 = 0;
        loop {
            match self.wire.poll_packet(MAX_SLICE.min(IN_FLIGHT_GRACE)) {
                Ok(Some(())) => return true,
                Err(_) => return false,
                Ok(None) => {
                    polls = polls.saturating_add(1);
                    if cx.now().saturating_since(start).max(MAX_SLICE.saturating_mul(polls)) >= IN_FLIGHT_GRACE {
                        return false;
                    }
                }
            }
        }
    }

    /// §5.3 emergency stop. Never returns early, ignores the latch, always attempts MO(0) and WritePort(0):
    /// stop submitting → in-flight packet completes (≤ 20 ms) or cancel_all + possible_desync → StopExecute →
    /// Fiber_SetMo(0) → ResetList → ONLY IF READY within 50 ms: dummy EOL packet + SetEndOfList + ExecuteList ONLY IF
    /// no list was executing → Fiber_SetMo(0) → WritePort(0) → drain stale replies → GetVersion !BUSY → StopReport.
    pub(crate) fn emergency(&mut self, cx: &Cx) -> StopReport {
        let requested = cx.stop_requested_at();
        let deadline = cx.now().plus(EMERGENCY_BUDGET);
        let mode = Mode::Abort { deadline };
        let mut notes = Vec::new();
        let mut possible_desync = false;

        if self.packet_in_flight {
            if self.finish_in_flight(cx) {
                notes.push("the in-flight packet completed before the abort".to_owned());
            } else {
                self.wire.cancel_all();
                possible_desync = true;
                notes.push("the in-flight packet did not complete within 20 ms: cancelled; the board may hold part of it (H23)".to_owned());
            }
            self.packet_in_flight = false;
        }
        if self.read_abandoned {
            self.wire.drain_replies();
            self.read_abandoned = false;
        }

        self.abort_single(lmc_ops::single(STOP_EXECUTE, 0), cx, mode, &mut notes);
        let mo_first = self.abort_single(lmc_ops::single(FIBER_SET_MO, 0), cx, mode, &mut notes);
        self.abort_single(lmc_ops::single(RESET_LIST, 0), cx, mode, &mut notes);

        let ready = self.wait_status(cx, mode, ABORT_READY_WAIT, StatusWord::ready, &mut notes).is_some();
        if !ready {
            notes.push("board not READY within 50 ms: no dummy end-of-list packet (never write while not READY)".to_owned());
        } else if let Some(dummy) = self.abort_packet.take() {
            let sent = match self.packet(&dummy, "abort", cx, mode) {
                Ok(()) => true,
                Err(Halt::Fault(e)) => {
                    notes.push(format!("dummy end-of-list packet: {e}"));
                    false
                }
                Err(Halt::Stopped) => false,
            };
            self.abort_packet = Some(dummy);
            if sent {
                self.abort_single(lmc_ops::single(SET_END_OF_LIST, 0), cx, mode, &mut notes);
                if self.list_executing {
                    notes.push("no ExecuteList after the dummy packet: a list was executing (OM v1 rule)".to_owned());
                } else {
                    self.abort_single(lmc_ops::single(EXECUTE_LIST, 0), cx, mode, &mut notes);
                }
            }
        } else {
            notes.push("no dummy end-of-list packet: the protocol crate provides none in this build".to_owned());
        }

        let mo_second = self.abort_single(lmc_ops::single(FIBER_SET_MO, 0), cx, mode, &mut notes);
        let ports_zero = self.abort_single(lmc_ops::single(WRITE_PORT, 0), cx, mode, &mut notes);
        self.wire.drain_replies();
        self.read_abandoned = false;
        let idle = self.wait_status(cx, mode, CONFIRM_WAIT, |s| !s.busy(), &mut notes).is_some();
        if !idle {
            notes.push("the board did not report !BUSY after the stop".to_owned());
        }

        self.list_executing = false;
        let mo_closed = mo_first || mo_second;
        if mo_closed {
            self.mo_open = false;
        }
        StopReport { confirmed: mo_closed && ports_zero && idle, elapsed: cx.now().saturating_since(requested), possible_desync, mo_closed, ports_zero, notes }
    }

    #[cfg(test)]
    pub(crate) fn mo_open(&self) -> bool {
        self.mo_open
    }
}

#[cfg(test)]
pub(crate) mod board {
    //! A scripted LMC board for the session's unit tests: READY from a buffer depth, BUSY from the packets' own
    //! execution times, MO and port state, a wire log with virtual timestamps, faults, and a hook that trips the
    //! latch at an exact wire event. (The T6 suites use `lmc-sim`'s FakeLmc; this one only needs packet ids.)

    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use openlaser_lmc::{Single, XferError};
    use openlaser_types::{Clock, MonoTime, VirtualClock};

    use super::Wire;
    use crate::lmc_ops::{EXECUTE_LIST, FIBER_SET_MO, GET_VERSION, RESET_LIST, STOP_EXECUTE, WRITE_PORT};

    #[derive(Debug, Clone, PartialEq)]
    pub(crate) struct Pkt {
        pub(crate) id: u32,
        /// How long the board takes to execute it.
        pub(crate) exec: Duration,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub(crate) enum Ev {
        Single { at: MonoTime, words: [u16; 6] },
        Packet { at: MonoTime, id: u32 },
        PacketDone { at: MonoTime, id: u32 },
        Read { at: MonoTime, status: u16 },
        Cancel { at: MonoTime },
    }

    impl Ev {
        pub(crate) fn op(&self) -> Option<u16> {
            match self {
                Ev::Single { words, .. } => Some(words[0]),
                _ => None,
            }
        }
    }

    #[derive(Debug, Default, Clone)]
    pub(crate) struct Faults {
        pub(crate) ready_never: bool,
        pub(crate) ready_never_after_execute: bool,
        pub(crate) busy_stuck: bool,
        /// GetVersion replies never come after this many have been answered.
        pub(crate) replies_stop_after: Option<u32>,
        /// Packets never complete (from this 1-based packet on).
        pub(crate) packet_stalls_from: Option<u32>,
        /// The board executes this many times slower than the packets say.
        pub(crate) slowdown: u32,
        pub(crate) buffer_depth: usize,
        /// How long a packet transfer takes to complete.
        pub(crate) packet_latency: Duration,
    }

    #[derive(Default)]
    pub(crate) struct State {
        pub(crate) log: Vec<Ev>,
        pub(crate) faults: Faults,
        loaded: Vec<(u32, Duration)>,
        /// Completion time of each loaded packet once executing.
        done_at: Vec<MonoTime>,
        executing: bool,
        stopping: bool,
        pub(crate) mo: bool,
        pub(crate) port: u16,
        pending_reply: Option<Option<[u16; 4]>>,
        pending_packet: Option<(u32, MonoTime)>,
        pub(crate) writes_while_not_ready: u32,
        pub(crate) mo_closed_while_busy: u32,
        pub(crate) mo_opened_at: Option<MonoTime>,
        pub(crate) burn_windows: Vec<(MonoTime, MonoTime)>,
        replies: u32,
        packets: u32,
        pub(crate) submits: u32,
    }

    impl State {
        fn unexecuted(&self, now: MonoTime) -> usize {
            if !self.executing {
                return self.loaded.len();
            }
            self.done_at.iter().filter(|t| **t > now).count()
        }

        pub(crate) fn ready(&self, now: MonoTime) -> bool {
            if self.faults.ready_never || (self.faults.ready_never_after_execute && self.executing) {
                return false;
            }
            self.unexecuted(now) < self.faults.buffer_depth.max(1)
        }

        pub(crate) fn busy(&self, now: MonoTime) -> bool {
            self.executing && (self.faults.busy_stuck || self.done_at.iter().any(|t| *t > now))
        }

        fn status(&self, now: MonoTime) -> u16 {
            let mut s = 0;
            if self.ready(now) {
                s |= 0x20;
            }
            if self.busy(now) {
                s |= 0x04;
            }
            s
        }

        fn schedule(&mut self, now: MonoTime) {
            let slow = self.faults.slowdown.max(1);
            let mut cursor = self.done_at.last().copied().filter(|t| *t > now).unwrap_or(now);
            while self.done_at.len() < self.loaded.len() {
                let Some((_, exec)) = self.loaded.get(self.done_at.len()) else { break };
                cursor = cursor.plus(exec.saturating_mul(slow));
                self.done_at.push(cursor);
            }
        }
    }

    pub(crate) type Hook = Box<dyn FnMut(&Ev) + Send>;

    pub(crate) struct Board {
        pub(crate) state: Arc<Mutex<State>>,
        pub(crate) clock: Arc<VirtualClock>,
        hook: Option<Hook>,
    }

    impl Board {
        pub(crate) fn new(clock: Arc<VirtualClock>, faults: Faults) -> Self {
            let state = State { faults, ..State::default() };
            Board { state: Arc::new(Mutex::new(state)), clock, hook: None }
        }

        pub(crate) fn with_hook(mut self, hook: Hook) -> Self {
            self.hook = Some(hook);
            self
        }

        fn record(&mut self, ev: Ev) {
            if let Some(h) = self.hook.as_mut() {
                h(&ev);
            }
            self.state.lock().unwrap().log.push(ev);
        }
    }

    impl Wire for Board {
        type Pkt = Pkt;

        fn submit_single(&mut self, cmd: Single) -> Result<(), XferError> {
            let now = self.clock.now();
            self.record(Ev::Single { at: now, words: cmd.0 });
            let mut s = self.state.lock().unwrap();
            s.submits += 1;
            let [op, p1, ..] = cmd.0;
            match op {
                EXECUTE_LIST => {
                    s.executing = true;
                    s.stopping = false;
                    s.done_at.clear();
                    s.schedule(now);
                }
                STOP_EXECUTE => {
                    s.executing = false;
                    s.stopping = true;
                    s.loaded.clear();
                    s.done_at.clear();
                }
                RESET_LIST => {
                    if !s.executing {
                        s.loaded.clear();
                        s.done_at.clear();
                    }
                }
                FIBER_SET_MO => {
                    if p1 == 0 {
                        if s.busy(now) && !s.stopping {
                            s.mo_closed_while_busy += 1;
                        }
                        if let Some(from) = s.mo_opened_at.take() {
                            s.burn_windows.push((from, now));
                        }
                        s.mo = false;
                    } else {
                        s.mo = true;
                        s.mo_opened_at = Some(now);
                    }
                }
                WRITE_PORT => s.port = p1,
                _ => {}
            }
            let answer = op != GET_VERSION || s.faults.replies_stop_after.is_none_or(|n| s.replies < n);
            if op == GET_VERSION {
                s.replies += 1;
            }
            let status = s.status(now);
            s.pending_reply = if answer { Some(Some([0, 0, 0, status])) } else { None };
            Ok(())
        }

        fn poll_single(&mut self, slice: Duration) -> Result<Option<Option<[u16; 4]>>, XferError> {
            let reply = self.state.lock().unwrap().pending_reply.take();
            match reply {
                Some(r) => {
                    if let Some([_, _, _, status]) = r {
                        let at = self.clock.now();
                        self.record(Ev::Read { at, status });
                    }
                    Ok(Some(r))
                }
                None => {
                    self.clock.advance(slice);
                    Ok(None)
                }
            }
        }

        fn submit_packet(&mut self, p: &Pkt) -> Result<(), XferError> {
            let now = self.clock.now();
            self.record(Ev::Packet { at: now, id: p.id });
            let mut s = self.state.lock().unwrap();
            s.submits += 1;
            s.packets += 1;
            if !s.ready(now) {
                s.writes_while_not_ready += 1;
            }
            let stalls = s.faults.packet_stalls_from.is_some_and(|k| s.packets >= k);
            s.loaded.push((p.id, p.exec));
            if s.executing {
                s.schedule(now);
            }
            let latency = s.faults.packet_latency;
            s.pending_packet = if stalls { None } else { Some((p.id, now.plus(latency))) };
            Ok(())
        }

        fn poll_packet(&mut self, slice: Duration) -> Result<Option<()>, XferError> {
            let now = self.clock.now();
            let pending = self.state.lock().unwrap().pending_packet;
            match pending {
                Some((id, done)) if done.saturating_since(now) <= slice => {
                    self.clock.advance(done.saturating_since(now));
                    self.state.lock().unwrap().pending_packet = None;
                    let at = self.clock.now();
                    self.record(Ev::PacketDone { at, id });
                    Ok(Some(()))
                }
                _ => {
                    self.clock.advance(slice);
                    Ok(None)
                }
            }
        }

        fn cancel_all(&mut self) {
            let at = self.clock.now();
            self.record(Ev::Cancel { at });
        }

        fn drain_replies(&mut self) {
            self.state.lock().unwrap().pending_reply = None;
        }

        fn describe_packet(p: &Pkt) -> String {
            format!("#{}", p.id)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use openlaser_types::{Clock, VirtualClock};

    use super::board::{Board, Ev, Faults, Pkt};
    use super::*;
    use crate::lmc_ops::{GET_MARK_TIME, single};
    use crate::stop::StopLatch;

    struct Rig {
        clock: Arc<VirtualClock>,
        latch: Arc<StopLatch>,
        events: crossbeam_channel::Sender<DriverEvent>,
        _rx: crossbeam_channel::Receiver<DriverEvent>,
        _wake: crossbeam_channel::Receiver<()>,
    }

    impl Rig {
        fn new() -> Self {
            let clock = Arc::new(VirtualClock::new());
            let (wake_tx, wake) = crossbeam_channel::bounded(1);
            let latch = Arc::new(StopLatch::with_clock(wake_tx, clock.clone()));
            let (events, _rx) = crossbeam_channel::unbounded();
            Rig { clock, latch, events, _rx, _wake: wake }
        }

        fn cx(&self) -> Cx<'_> {
            Cx::new(&self.latch, &self.events, &*self.clock)
        }

        fn session(&self, faults: Faults) -> Session<Board> {
            let board = Board::new(self.clock.clone(), Faults { buffer_depth: faults.buffer_depth.max(1), ..faults });
            Session::new(board, SessionCfg::from_machine(&machine()).unwrap(), Some(Pkt { id: 9999, exec: Duration::ZERO }))
        }
    }

    fn machine() -> GalvoMachine {
        match openlaser_machine::jpt_m7_200_sim().kind {
            openlaser_machine::Kinematics::Galvo(g) => g,
            openlaser_machine::Kinematics::Gantry(_) => panic!("the JPT M7 is a galvo"),
        }
    }

    fn prelude() -> Vec<Single> {
        vec![single(RESET_LIST, 0), single(WRITE_PORT, 1), single(FIBER_SET_MO, 1), lmc_ops::get_version()]
    }

    fn postlude() -> Vec<Single> {
        vec![single(FIBER_SET_MO, 0), single(WRITE_PORT, 0), single(GET_MARK_TIME, 3)]
    }

    fn packets(n: u32, exec: Duration) -> (Vec<Pkt>, Vec<Duration>) {
        ((1..=n).map(|id| Pkt { id, exec }).collect(), vec![exec; n as usize])
    }

    fn ops(log: &[Ev]) -> Vec<String> {
        log.iter()
            .filter_map(|e| match e {
                Ev::Single { words, .. } => Some(lmc_ops::describe(&Single(*words))),
                Ev::Packet { id, .. } => Some(format!("packet #{id}")),
                Ev::Cancel { .. } => Some("cancel_all".to_owned()),
                _ => None,
            })
            .collect()
    }

    fn run_one_list(rig: &Rig, s: &mut Session<Board>, pk: &[Pkt], est: &[Duration]) -> Result<(), Halt> {
        let (pre, post) = (prelude(), postlude());
        let spec = ListSpec { prelude: &pre, packets: pk, est, postlude: &post, label: "C00 pass 1".to_owned() };
        let mut progress = Progress { acked: 0, total: u32::try_from(pk.len()).unwrap(), unsent: est.iter().sum() };
        s.run_list(&spec, &rig.cx(), Some(&mut progress))
    }

    #[test]
    fn one_packet_list_matches_the_gp_session_order() {
        let rig = Rig::new();
        let mut s = rig.session(Faults { buffer_depth: 8, ..Faults::default() });
        let (pk, est) = packets(1, Duration::from_millis(50));
        run_one_list(&rig, &mut s, &pk, &est).unwrap();
        let log = ops(&s.wire.state.lock().unwrap().log);
        let polls = log.iter().filter(|o| *o == "GetVersion").count();
        assert!(polls >= 2, "a prelude poll and at least one executed poll");
        let mut expect = vec!["ResetList", "WritePort(0x0001)", "Fiber_SetMo(1)", "GetVersion", "packet #1", "SetEndOfList", "ExecuteList"];
        expect.extend(std::iter::repeat_n("GetVersion", polls - 1));
        expect.extend(["Fiber_SetMo", "WritePort", "GetMarkTime(3)"]);
        assert_eq!(log, expect);
        let st = s.wire.state.lock().unwrap();
        assert_eq!((st.writes_while_not_ready, st.mo_closed_while_busy, st.mo, st.port), (0, 0, false, 0));
        assert!(!s.mo_open());
    }

    #[test]
    fn execute_list_after_the_third_packet() {
        let rig = Rig::new();
        let mut s = rig.session(Faults { buffer_depth: 8, ..Faults::default() });
        let (pk, est) = packets(6, Duration::from_millis(100));
        run_one_list(&rig, &mut s, &pk, &est).unwrap();
        let log = ops(&s.wire.state.lock().unwrap().log);
        let exec = log.iter().position(|o| o == "ExecuteList").unwrap();
        let third = log.iter().position(|o| o == "packet #3").unwrap();
        assert_eq!(exec, third + 2, "packet #3 · SetEndOfList · ExecuteList");
        assert_eq!(log.iter().filter(|o| *o == "ExecuteList").count(), 1);
    }

    #[test]
    fn slow_packets_execute_after_the_first_and_finish() {
        // t6_slow_packets_execute_without_deadlock: every packet estimated > 2 s.
        let rig = Rig::new();
        let mut s = rig.session(Faults { buffer_depth: 8, ..Faults::default() });
        let (pk, est) = packets(9, Duration::from_millis(7200));
        run_one_list(&rig, &mut s, &pk, &est).unwrap();
        let log = ops(&s.wire.state.lock().unwrap().log);
        let exec = log.iter().position(|o| o == "ExecuteList").unwrap();
        assert_eq!(log.get(exec - 2).map(String::as_str), Some("packet #1"));
        assert!(rig.clock.now().0 >= Duration::from_millis(9 * 7200), "the whole list ran");
    }

    #[test]
    fn buffer_depth_1_and_2_execute_before_the_third_packet() {
        for depth in [1, 2] {
            let rig = Rig::new();
            let mut s = rig.session(Faults { buffer_depth: depth, ..Faults::default() });
            let (pk, est) = packets(6, Duration::from_millis(100));
            run_one_list(&rig, &mut s, &pk, &est).unwrap();
            let st = s.wire.state.lock().unwrap();
            let log = ops(&st.log);
            let exec = log.iter().position(|o| o == "ExecuteList").unwrap();
            let third = log.iter().position(|o| o == "packet #3").unwrap();
            assert!(exec < third, "depth {depth}: READY dropped before packet 3, so ExecuteList came first");
            assert_eq!(st.writes_while_not_ready, 0, "depth {depth}");
        }
    }

    #[test]
    fn run_ahead_window_caps_packets_and_seconds() {
        // t6_run_ahead_cap_packets_and_seconds: at every packet write, replaying the time model from the wire log,
        // ≤ 3 packets unexecuted and ≤ 2 s queued after the write, unless the board held nothing before it.
        let rig = Rig::new();
        let mut s = rig.session(Faults { buffer_depth: 8, ..Faults::default() });
        let est: Vec<Duration> = (0..40u64).map(|i| Duration::from_millis(150 + (i * 337) % 1400)).collect();
        let pk: Vec<Pkt> = est.iter().enumerate().map(|(i, e)| Pkt { id: u32::try_from(i).unwrap() + 1, exec: *e }).collect();
        run_one_list(&rig, &mut s, &pk, &est).unwrap();
        let st = s.wire.state.lock().unwrap();
        let mut sent: Vec<Duration> = Vec::new();
        let mut done_at: Vec<MonoTime> = Vec::new();
        let mut executing = false;
        let mut writes = 0;
        for ev in &st.log {
            match ev {
                Ev::Single { at, words } if words[0] == EXECUTE_LIST => {
                    executing = true;
                    let mut cursor = *at;
                    for e in &sent {
                        cursor = cursor.plus(*e);
                        done_at.push(cursor);
                    }
                }
                Ev::Packet { at, id } => {
                    writes += 1;
                    let e = est[*id as usize - 1];
                    let (queued_before, unexecuted_before) = if executing {
                        (done_at.last().map(|t| t.saturating_since(*at)).unwrap_or_default(), done_at.iter().filter(|t| **t > *at).count())
                    } else {
                        (sent.iter().sum(), sent.len())
                    };
                    sent.push(e);
                    if executing {
                        let start = done_at.last().copied().filter(|t| t > at).unwrap_or(*at);
                        done_at.push(start.plus(e));
                    }
                    assert!(unexecuted_before < 3, "packet #{id}: {unexecuted_before} unexecuted before it");
                    assert!(queued_before.is_zero() || queued_before + e <= Duration::from_secs(2), "packet #{id}: {:?} queued after it", queued_before + e);
                }
                _ => {}
            }
        }
        assert_eq!(writes, 40);
        assert_eq!(st.writes_while_not_ready, 0);
    }

    #[test]
    fn busy_20s_completes_without_mo_close() {
        // B1: a 20 s list completes; MO closes only after BUSY clears.
        let rig = Rig::new();
        let mut s = rig.session(Faults { buffer_depth: 8, ..Faults::default() });
        let (pk, est) = packets(1, Duration::from_secs(20));
        run_one_list(&rig, &mut s, &pk, &est).unwrap();
        let st = s.wire.state.lock().unwrap();
        assert_eq!(st.mo_closed_while_busy, 0);
        let (open, close) = st.burn_windows[0];
        assert!(close.saturating_since(open) >= Duration::from_secs(20));
    }

    #[test]
    fn ready_never_returns_is_a_fault_within_2s_and_the_abort_writes_nothing() {
        // t6_ready_never_returns_stopped_within_2s + B2 (t6_abort_no_write_while_not_ready).
        let rig = Rig::new();
        let mut s = rig.session(Faults { buffer_depth: 8, ready_never: true, ..Faults::default() });
        let (pk, est) = packets(3, Duration::from_millis(100));
        let t0 = rig.clock.now();
        let err = run_one_list(&rig, &mut s, &pk, &est).unwrap_err();
        assert!(matches!(err, Halt::Fault(DriverError::Watchdog { .. })), "{err:?}");
        assert!(rig.clock.now().saturating_since(t0) <= Duration::from_millis(2100));
        let report = s.emergency(&rig.cx());
        let st = s.wire.state.lock().unwrap();
        assert!(!st.log.iter().any(|e| matches!(e, Ev::Packet { .. })), "no 3072-byte write at all");
        assert_eq!((st.mo, st.port, report.mo_closed, report.ports_zero), (false, 0, true, true));
    }

    #[test]
    fn a_slow_but_progressing_board_is_never_stopped() {
        // t6_watchdog_only_without_progress: the board runs 3× slower than estimated (each packet 2.7 s instead of
        // 0.9 s, inside 0.9 s × 1.5 + 2 s between READY changes), READY toggles as it drains.
        let rig = Rig::new();
        let mut s = rig.session(Faults { buffer_depth: 2, slowdown: 3, ..Faults::default() });
        let (pk, est) = packets(12, Duration::from_millis(900));
        run_one_list(&rig, &mut s, &pk, &est).unwrap();
        assert_eq!(s.wire.state.lock().unwrap().writes_while_not_ready, 0);
    }

    #[test]
    fn busy_stuck_board_faults_after_the_budget() {
        let rig = Rig::new();
        let mut s = rig.session(Faults { buffer_depth: 8, busy_stuck: true, ..Faults::default() });
        let (pk, est) = packets(1, Duration::from_secs(1));
        let err = run_one_list(&rig, &mut s, &pk, &est).unwrap_err();
        assert!(matches!(err, Halt::Fault(DriverError::Watchdog { .. })));
        // 1 s queued × 1.5 + 2 s slack after ExecuteList.
        assert!(rig.clock.now().0 <= Duration::from_millis(3700), "{:?}", rig.clock.now());
        assert!(s.mo_open(), "MO is still open: the runtime's emergency stop closes it");
        let report = s.emergency(&rig.cx());
        assert!(report.mo_closed && report.ports_zero);
    }

    #[test]
    fn reply_timeout_faults_without_retry() {
        // t6_transfer_timeout_faults_and_stops: no retry, no reset.
        let rig = Rig::new();
        let mut s = rig.session(Faults { buffer_depth: 8, replies_stop_after: Some(1), ..Faults::default() });
        let (pk, est) = packets(2, Duration::from_millis(100));
        let t0 = rig.clock.now();
        let err = run_one_list(&rig, &mut s, &pk, &est).unwrap_err();
        let Halt::Fault(DriverError::Transport(msg)) = err else { panic!("{err:?}") };
        assert!(msg.contains("GetVersion") && msg.contains("250 ms"), "{msg}");
        assert!(rig.clock.now().saturating_since(t0) <= Duration::from_millis(400));
        let st = s.wire.state.lock().unwrap();
        let versions = st.log.iter().filter(|e| e.op() == Some(GET_VERSION)).count();
        assert_eq!(versions, 2, "the unanswered GetVersion was not retried");
    }

    #[test]
    fn stop_with_a_packet_in_flight_completes_it_before_stop_execute() {
        // t6_inflight_packet_completed_before_stop_execute.
        let rig = Rig::new();
        let latch = Arc::clone(&rig.latch);
        let faults = Faults { buffer_depth: 8, packet_latency: Duration::from_millis(30), ..Faults::default() };
        let board = Board::new(rig.clock.clone(), faults).with_hook(Box::new(move |ev| {
            if matches!(ev, Ev::Packet { id: 2, .. }) {
                latch.trip();
            }
        }));
        let mut s = Session::new(board, SessionCfg::from_machine(&machine()).unwrap(), Some(Pkt { id: 9999, exec: Duration::ZERO }));
        let (pk, est) = packets(4, Duration::from_millis(100));
        assert_eq!(run_one_list(&rig, &mut s, &pk, &est), Err(Halt::Stopped));
        assert!(s.packet_in_flight, "the stop was seen while packet #2 was still in flight");
        let report = s.emergency(&rig.cx());
        let st = s.wire.state.lock().unwrap();
        let done = st.log.iter().position(|e| matches!(e, Ev::PacketDone { id: 2, .. })).unwrap();
        let stop = st.log.iter().position(|e| e.op() == Some(STOP_EXECUTE)).unwrap();
        assert!(done < stop);
        assert!(!report.possible_desync);
        assert!(report.confirmed, "{report:?}");
    }

    #[test]
    fn a_packet_that_never_completes_is_cancelled_and_flagged() {
        let rig = Rig::new();
        let latch = Arc::clone(&rig.latch);
        let board =
            Board::new(rig.clock.clone(), Faults { buffer_depth: 8, packet_stalls_from: Some(2), ..Faults::default() }).with_hook(Box::new(move |ev| {
                if matches!(ev, Ev::Packet { id: 2, .. }) {
                    latch.trip();
                }
            }));
        let mut s = Session::new(board, SessionCfg::from_machine(&machine()).unwrap(), None);
        let (pk, est) = packets(3, Duration::from_millis(100));
        assert_eq!(run_one_list(&rig, &mut s, &pk, &est), Err(Halt::Stopped));
        let report = s.emergency(&rig.cx());
        assert!(report.possible_desync);
        let log = ops(&s.wire.state.lock().unwrap().log);
        let cancel = log.iter().position(|o| o == "cancel_all").unwrap();
        assert_eq!(log.get(cancel + 1).map(String::as_str), Some("StopExecute"));
    }

    #[test]
    fn abort_after_execute_sends_no_execute_list_but_idle_abort_does() {
        // t6_abort_mid_list_sends_no_execute_list (OM v1 `_list_executing` rule).
        let rig = Rig::new();
        let latch = Arc::clone(&rig.latch);
        let seen = Arc::new(AtomicU32::new(0));
        let seen2 = Arc::clone(&seen);
        let board = Board::new(rig.clock.clone(), Faults { buffer_depth: 8, ..Faults::default() }).with_hook(Box::new(move |ev| {
            if ev.op() == Some(EXECUTE_LIST) && seen2.fetch_add(1, Ordering::SeqCst) == 0 {
                latch.trip();
            }
        }));
        let mut s = Session::new(board, SessionCfg::from_machine(&machine()).unwrap(), Some(Pkt { id: 9999, exec: Duration::ZERO }));
        let (pk, est) = packets(1, Duration::from_secs(5));
        assert_eq!(run_one_list(&rig, &mut s, &pk, &est), Err(Halt::Stopped));
        let _ = s.emergency(&rig.cx());
        let mid = ops(&s.wire.state.lock().unwrap().log);
        let dummy = mid.iter().position(|o| o == "packet #9999").unwrap();
        assert_eq!(mid[dummy + 1], "SetEndOfList");
        assert_eq!(mid[dummy + 2], "Fiber_SetMo", "no ExecuteList after the dummy packet mid-list");

        // A stop while idle (after a finished list): the dummy packet is followed by ExecuteList.
        let rig = Rig::new();
        let mut s = rig.session(Faults { buffer_depth: 8, ..Faults::default() });
        let (pk, est) = packets(1, Duration::from_millis(50));
        run_one_list(&rig, &mut s, &pk, &est).unwrap();
        let _ = s.emergency(&rig.cx());
        let idle = ops(&s.wire.state.lock().unwrap().log);
        let dummy = idle.iter().position(|o| o == "packet #9999").unwrap();
        assert_eq!(&idle[dummy + 1..dummy + 3], ["SetEndOfList", "ExecuteList"]);
    }

    #[test]
    fn emergency_sequence_order() {
        let rig = Rig::new();
        let mut s = rig.session(Faults { buffer_depth: 8, ..Faults::default() });
        let report = s.emergency(&rig.cx());
        let log = ops(&s.wire.state.lock().unwrap().log);
        let expect =
            ["StopExecute", "Fiber_SetMo", "ResetList", "GetVersion", "packet #9999", "SetEndOfList", "ExecuteList", "Fiber_SetMo", "WritePort", "GetVersion"];
        assert_eq!(log, expect);
        assert!(report.confirmed && report.mo_closed && report.ports_zero && !report.possible_desync, "{report:?}");
        assert!(report.elapsed <= Duration::from_millis(100));
    }

    #[test]
    fn stop_at_every_wire_event_leaves_mo_closed_and_ports_zero() {
        // prop_stop_at_random_tick_leaves_mo_closed + t6_no_write_while_not_ready, exhaustively over one job's
        // wire events (seeded, deterministic: the stop is tripped from the wire hook, never from another thread).
        let total = {
            let rig = Rig::new();
            let mut s = rig.session(Faults { buffer_depth: 2, ..Faults::default() });
            let (pk, est) = packets(5, Duration::from_millis(400));
            run_one_list(&rig, &mut s, &pk, &est).unwrap();
            s.wire.state.lock().unwrap().log.len()
        };
        for k in 0..total {
            let rig = Rig::new();
            let latch = Arc::clone(&rig.latch);
            let count = Arc::new(AtomicU32::new(0));
            let board = Board::new(rig.clock.clone(), Faults { buffer_depth: 2, ..Faults::default() }).with_hook(Box::new(move |_| {
                if count.fetch_add(1, Ordering::SeqCst) as usize == k {
                    latch.trip();
                }
            }));
            let mut s = Session::new(board, SessionCfg::from_machine(&machine()).unwrap(), Some(Pkt { id: 9999, exec: Duration::ZERO }));
            let (pk, est) = packets(5, Duration::from_millis(400));
            let res = run_one_list(&rig, &mut s, &pk, &est);
            let report = s.emergency(&rig.cx());
            let st = s.wire.state.lock().unwrap();
            assert!(res.is_ok() || res == Err(Halt::Stopped), "event {k}: {res:?}");
            assert_eq!(st.writes_while_not_ready, 0, "event {k}");
            assert_eq!((st.mo, st.port), (false, 0), "event {k}");
            assert!(report.confirmed, "event {k}: {report:?}");
            assert!(report.elapsed < Duration::from_millis(100), "event {k}: {:?}", report.elapsed);
        }
    }

    #[test]
    fn frame_once_reports_after_the_list_executed() {
        // t6_frame_receipt_only_after_executed_pass (Once) and t6_framing_records_zero_emission.
        let rig = Rig::new();
        let mut s = rig.session(Faults { buffer_depth: 8, ..Faults::default() });
        let (once, _) = packets(2, Duration::from_millis(1000));
        let spec = FrameSpec { once: &once, once_est: Duration::from_secs(2), once_passes: 150, pass: &once[..1], pass_est: Duration::from_millis(20) };
        let state = Arc::clone(&s.wire.state);
        let clock = rig.clock.clone();
        let mut receipts = Vec::new();
        let passes = s
            .frame(&spec, true, &rig.cx(), &mut |_, k| {
                let st = state.lock().unwrap();
                assert!(!st.busy(clock.now()), "a receipt only once the board is idle");
                receipts.push(k);
            })
            .unwrap();
        assert_eq!((passes, receipts), (150, vec![150]));
        let st = s.wire.state.lock().unwrap();
        assert!(st.burn_windows.is_empty() && st.mo_opened_at.is_none(), "MO never opened while framing");
        let log = ops(&st.log);
        assert_eq!(&log[..2], ["ResetList", "GetVersion"]);
        assert_eq!(&log[log.len() - 2..], ["Fiber_SetMo", "WritePort"]);
    }

    #[test]
    fn frame_loop_counts_only_executed_passes() {
        let rig = Rig::new();
        let latch = Arc::clone(&rig.latch);
        let execs = Arc::new(AtomicU32::new(0));
        let execs2 = Arc::clone(&execs);
        let board = Board::new(rig.clock.clone(), Faults { buffer_depth: 8, ..Faults::default() }).with_hook(Box::new(move |ev| {
            if ev.op() == Some(EXECUTE_LIST) && execs2.fetch_add(1, Ordering::SeqCst) == 3 {
                latch.trip(); // stop during pass 4, before its !BUSY
            }
        }));
        let mut s = Session::new(board, SessionCfg::from_machine(&machine()).unwrap(), None);
        let (pass, _) = packets(1, Duration::from_millis(300));
        let spec = FrameSpec { once: &pass, once_est: Duration::from_secs(2), once_passes: 1, pass: &pass, pass_est: Duration::from_millis(300) };
        let mut receipts = Vec::new();
        let res = s.frame(&spec, false, &rig.cx(), &mut |_, k| receipts.push(k));
        assert_eq!(res, Err(Halt::Stopped));
        assert_eq!(receipts, vec![1, 2, 3], "pass 4 was stopped before !BUSY: no receipt");
    }

    #[test]
    fn connect_sends_init_verbatim_then_settles() {
        let rig = Rig::new();
        let mut s = rig.session(Faults { buffer_depth: 8, ..Faults::default() });
        let init = [single(0x0009, 0), lmc_ops::get_version(), single(0x0040, 0)];
        let version = s.connect(&init, &rig.cx()).unwrap();
        assert_eq!(version, Some([0, 0, 0, 0x20]));
        assert_eq!(ops(&s.wire.state.lock().unwrap().log), ["GetSerialNo", "GetVersion", "Reset"]);
        assert!(rig.clock.now().0 >= INIT_SETTLE);
        assert!(matches!(s.connect(&[], &rig.cx()), Err(Halt::Fault(DriverError::Protocol(_)))), "an empty init is refused");
    }

    #[test]
    fn window_rules() {
        let cfg = SessionCfg::from_machine(&machine()).unwrap();
        let mut w = Window::default();
        let t = MonoTime::default();
        let ms = Duration::from_millis;
        assert!(w.may_send(t, Duration::from_secs(60), &cfg), "an empty board always takes one packet");
        w.push(t, ms(500));
        w.push(t, ms(500));
        assert!(w.may_send(t, ms(900), &cfg));
        w.push(t, ms(900));
        assert!(!w.may_send(t, ms(1), &cfg), "3 packets unexecuted");
        assert_eq!(w.watched(t), Duration::ZERO, "before ExecuteList the watchdog budget is the slack alone");
        w.execute(t);
        let later = t.plus(ms(600));
        assert_eq!(w.unexecuted(later), 2);
        assert_eq!(w.queued(later), ms(1300));
        assert!(w.may_send(later, ms(700), &cfg));
        assert!(!w.may_send(later, ms(701), &cfg));
        // The model ran dry at 1.9 s; a packet sent at 5 s starts at 5 s, never retroactively.
        let dry = t.plus(Duration::from_secs(5));
        assert!(w.model_finished(dry));
        w.push(dry, ms(800));
        assert_eq!(w.queued(dry), ms(800));
        assert_eq!(w.unexecuted(dry), 1);
        assert_eq!(w.watched(dry.plus(ms(900))), ms(800), "the watchdog floors at the newest packet");
    }

    #[test]
    fn invalid_profiles_are_refused() {
        let mut m = machine();
        m.transfer_timeout_ms = 5;
        assert!(SessionCfg::from_machine(&m).is_err());
        let mut m = machine();
        m.run_ahead.watchdog_factor = f64::NAN;
        assert!(SessionCfg::from_machine(&m).is_err());
        let mut m = machine();
        m.run_ahead.max_packets = 0;
        assert!(SessionCfg::from_machine(&m).is_err());
    }
}
