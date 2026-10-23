//! The stop path: latch, stopper, the stop-report mailbox and the per-call context (SPEC §3.13, §5.3, §5.4).

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use openlaser_types::{Clock, MonoTime};
use serde::Serialize;

use crate::backend::{DriverEvent, TraceLine};

/// Trace lines beyond this many per operation are dropped and counted (§3.13 `Cx::emit`).
pub const TRACE_LINES_PER_OPERATION: u64 = 100_000;

/// The longest single wait of the driver thread (§5.4 "every wait is a ≤ 20 ms slice calling `cx.check()`").
pub const MAX_SLICE: Duration = Duration::from_millis(20);

/// Tripped by anyone, from any thread; read by the driver between every transfer. Signal handlers never call
/// `trip()` (a channel send is not async-signal-safe): they only store `true` into `signal_flag()` (signal-hook's
/// `flag::register`), which `is_tripped()` also reads, and the app's watcher thread then calls `trip()`.
/// `generation()` counts trips; every queued command records it at submission (§5.3 "queued-then-stopped").
pub struct StopLatch {
    tripped: AtomicBool,
    signal: Arc<AtomicBool>,
    generation: AtomicU64,
    /// Clock time of the first trip not yet cleared, in ns + 1 (0 = unknown). Only set when the latch has a clock.
    tripped_at_ns: AtomicU64,
    wake: Sender<()>,
    clock: Option<Arc<dyn Clock>>,
}

impl StopLatch {
    /// WP0 addition: the runtime builds the latch with the sender that wakes the driver thread.
    pub fn new(wake: Sender<()>) -> Self {
        StopLatch {
            tripped: AtomicBool::new(false),
            signal: Arc::new(AtomicBool::new(false)),
            generation: AtomicU64::new(0),
            tripped_at_ns: AtomicU64::new(0),
            wake,
            clock: None,
        }
    }

    /// Like [`StopLatch::new`], and stamps every trip with `clock.now()` so the `StopReport` can measure the whole
    /// stop from the moment it was requested (`spawn` uses this with the driver's clock).
    pub fn with_clock(wake: Sender<()>, clock: Arc<dyn Clock>) -> Self {
        StopLatch { clock: Some(clock), ..StopLatch::new(wake) }
    }

    /// Atomic stores and a non-blocking wake; never blocks, never does I/O. The generation is bumped BEFORE the flag
    /// is set, so [`StopLatch::clear_if_generation`] can never erase a trip it did not see (see there).
    pub fn trip(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.tripped.store(true, Ordering::SeqCst);
        if let Some(clock) = &self.clock {
            let ns = u64::try_from(clock.now().0.as_nanos()).unwrap_or(u64::MAX - 1).saturating_add(1);
            let _ = self.tripped_at_ns.compare_exchange(0, ns, Ordering::SeqCst, Ordering::SeqCst);
        }
        let _ = self.wake.try_send(());
    }

    pub fn is_tripped(&self) -> bool {
        self.tripped.load(Ordering::SeqCst) || self.signal.load(Ordering::SeqCst)
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn signal_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.signal)
    }

    /// True when a signal handler stored into [`StopLatch::signal_flag`]. That flag is never cleared: the process
    /// is going down, so every later command is refused.
    pub fn signal_raised(&self) -> bool {
        self.signal.load(Ordering::SeqCst)
    }

    /// When the current trip was requested, if the latch has a clock and a trip is pending.
    pub fn tripped_at(&self) -> Option<MonoTime> {
        match self.tripped_at_ns.load(Ordering::SeqCst) {
            0 => None,
            ns => Some(MonoTime(Duration::from_nanos(ns - 1))),
        }
    }

    /// The driver thread clears the latch for a command queued at generation `g`, and only if no trip happened
    /// since (§5.3). Returns false, leaving the latch tripped, when a trip raced in.
    ///
    /// `trip()` bumps the generation before it sets the flag, and this clears the flag before it reads the
    /// generation again (all `SeqCst`). So a racing trip either shows up in that read (the flag is set again here)
    /// or stores its flag after this clear (the flag stays set): a trip is never lost.
    pub fn clear_if_generation(&self, g: u64) -> bool {
        if self.signal_raised() || self.generation() != g {
            return false;
        }
        self.tripped.store(false, Ordering::SeqCst);
        self.tripped_at_ns.store(0, Ordering::SeqCst);
        if self.generation() != g {
            self.tripped.store(true, Ordering::SeqCst);
            return false;
        }
        true
    }
}

/// Where the driver thread posts each `StopReport`, tagged with the latch generation it answers, so
/// [`Stopper::stop_and_wait`] waits for the report of ITS stop. A poisoned lock is recovered.
#[derive(Default)]
pub(crate) struct StopMailbox {
    last: Mutex<Option<(u64, StopReport)>>,
    posted: Condvar,
}

impl StopMailbox {
    pub(crate) fn post(&self, generation: u64, report: StopReport) {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        *last = Some((generation, report));
        self.posted.notify_all();
    }

    /// The first report whose generation is ≥ `generation`, waiting at most `max` (real time).
    pub(crate) fn wait_for(&self, generation: u64, max: Duration) -> Option<StopReport> {
        let deadline = Instant::now().checked_add(max);
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some((g, report)) = last.as_ref()
                && *g >= generation
            {
                return Some(report.clone());
            }
            let now = Instant::now();
            let left = match deadline {
                Some(d) if d > now => d - now,
                Some(_) => return None,
                None => max,
            };
            let (guard, _) = self.posted.wait_timeout(last, left).unwrap_or_else(|e| e.into_inner());
            last = guard;
        }
    }
}

/// Cheap, cloneable, Send + Sync handle for the control server's fast path, Esc, signals and the CLI key thread.
#[derive(Clone)]
pub struct Stopper {
    latch: Arc<StopLatch>,
    mailbox: Arc<StopMailbox>,
}

impl Stopper {
    pub(crate) fn with_mailbox(latch: Arc<StopLatch>, mailbox: Arc<StopMailbox>) -> Self {
        Stopper { latch, mailbox }
    }

    /// Atomic store + wake; never blocks, never does I/O.
    pub fn stop(&self) {
        self.latch.trip();
    }

    /// stop(), then wait ≤ max for the StopReport of this stop (the driver posts one after its abort sequence, or
    /// at once when it is idle). `report: None` means the driver did not confirm in time: the caller shows
    /// "STOP NOT CONFIRMED: use the key switch / E-stop" (§5.4).
    pub fn stop_and_wait(&self, max: Duration) -> StopAck {
        let started = Instant::now();
        self.stop();
        let generation = self.latch.generation();
        let report = self.mailbox.wait_for(generation, max);
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        StopAck { latched: true, report, elapsed_ms }
    }

    /// The latch this stopper trips (apps hand its `signal_flag()` to signal-hook).
    pub fn latch(&self) -> &Arc<StopLatch> {
        &self.latch
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopAck {
    pub latched: bool,
    pub report: Option<StopReport>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopReport {
    pub confirmed: bool,
    #[serde(rename = "elapsedMs", with = "openlaser_types::serde_ms")]
    pub elapsed: Duration,
    pub possible_desync: bool,
    pub mo_closed: bool,
    pub ports_zero: bool,
    pub notes: Vec<String>,
}

impl StopReport {
    /// A stop that could not be confirmed by the machine (nothing connected, or the stop sequence itself failed).
    pub fn unconfirmed(note: impl Into<String>) -> Self {
        StopReport { confirmed: false, elapsed: Duration::ZERO, possible_desync: false, mo_closed: false, ports_zero: false, notes: vec![note.into()] }
    }
}

/// What the runtime lets an operation do through its `Cx`: record the receipts the backend issues, and answer the
/// commands that arrive while it is busy (`MachineBusy`), so nobody waits for a Loop frame to end to learn that.
pub(crate) trait CxHooks {
    fn on_event(&self, ev: &DriverEvent);
    fn pump(&self);
}

/// What the driver thread gets each call: stop checks, events and the clock.
pub struct Cx<'a> {
    latch: &'a StopLatch,
    events: &'a Sender<DriverEvent>,
    clock: &'a dyn Clock,
    hooks: Option<&'a dyn CxHooks>,
    traces: Cell<u64>,
    dropped: Cell<u64>,
}

impl<'a> Cx<'a> {
    /// WP0 addition: built by the runtime (and by backend tests).
    pub fn new(latch: &'a StopLatch, events: &'a Sender<DriverEvent>, clock: &'a dyn Clock) -> Self {
        Cx { latch, events, clock, hooks: None, traces: Cell::new(0), dropped: Cell::new(0) }
    }

    pub(crate) fn with_hooks(latch: &'a StopLatch, events: &'a Sender<DriverEvent>, clock: &'a dyn Clock, hooks: &'a dyn CxHooks) -> Self {
        Cx { hooks: Some(hooks), ..Cx::new(latch, events, clock) }
    }
}

impl Cx<'_> {
    /// Err as soon as the latch is tripped.
    pub fn check(&self) -> Result<(), Stopped> {
        if let Some(h) = self.hooks {
            h.pump();
        }
        if self.latch.is_tripped() { Err(Stopped) } else { Ok(()) }
    }

    /// Sleep ≤ min(d, 20 ms), then check.
    pub fn slice(&self, d: Duration) -> Result<(), Stopped> {
        self.check()?;
        self.clock.sleep(d.min(MAX_SLICE));
        self.check()
    }

    /// Sleep ≤ min(d, 20 ms) whatever the latch says: the emergency sequence runs while the latch is tripped.
    pub fn pause(&self, d: Duration) {
        self.clock.sleep(d.min(MAX_SLICE));
    }

    pub fn now(&self) -> MonoTime {
        self.clock.now()
    }

    /// When the stop being handled was requested: the latch's stamp if it has one, else now.
    pub fn stop_requested_at(&self) -> MonoTime {
        self.latch.tripped_at().unwrap_or_else(|| self.clock.now())
    }

    /// Never blocks: events go through an UNBOUNDED crossbeam channel, so a UI that stops polling (window hidden,
    /// UI thread stalled) can never stall the driver thread or its stop checks. Trace lines beyond 100 000 per
    /// operation are dropped and counted (`TraceLine` "N trace lines dropped").
    pub fn emit(&self, ev: DriverEvent) {
        if matches!(ev, DriverEvent::Trace(_)) {
            let n = self.traces.get().saturating_add(1);
            self.traces.set(n);
            if n > TRACE_LINES_PER_OPERATION {
                self.dropped.set(self.dropped.get().saturating_add(1));
                return;
            }
        }
        if let Some(h) = self.hooks {
            h.on_event(&ev);
        }
        let _ = self.events.try_send(ev);
    }

    /// A trace line for the Console panel (`out`: host → device).
    pub fn trace(&self, out: bool, text: impl Into<String>) {
        self.emit(DriverEvent::Trace(TraceLine { at: self.now(), out, text: text.into() }));
    }

    /// Reports the trace lines dropped in this operation, if any, and starts counting again.
    pub fn flush_dropped_traces(&self) {
        let dropped = self.dropped.replace(0);
        self.traces.set(0);
        if dropped > 0 {
            let line = TraceLine { at: self.now(), out: false, text: format!("{dropped} trace lines dropped") };
            let _ = self.events.try_send(DriverEvent::Trace(line));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopped;

#[cfg(test)]
mod tests {
    use openlaser_types::VirtualClock;

    use super::*;

    fn latch() -> (StopLatch, crossbeam_channel::Receiver<()>) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        (StopLatch::new(tx), rx)
    }

    #[test]
    fn trip_bumps_generation_and_wakes() {
        let (l, wake) = latch();
        assert!(!l.is_tripped());
        l.trip();
        l.trip();
        assert!(l.is_tripped());
        assert_eq!(l.generation(), 2);
        assert!(wake.try_recv().is_ok(), "a trip wakes the driver");
    }

    #[test]
    fn clear_only_for_the_generation_seen() {
        let (l, _w) = latch();
        l.trip();
        assert!(!l.clear_if_generation(0), "a trip after queueing at generation 0 cancels the command");
        assert!(l.is_tripped());
        assert!(l.clear_if_generation(1));
        assert!(!l.is_tripped());
    }

    #[test]
    fn signal_flag_trips_and_is_never_cleared() {
        let (l, _w) = latch();
        l.signal_flag().store(true, Ordering::SeqCst);
        assert!(l.is_tripped());
        assert!(!l.clear_if_generation(0));
        assert!(l.is_tripped());
    }

    #[test]
    fn trip_is_stamped_with_the_clock() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let clock = Arc::new(VirtualClock::new());
        clock.advance(Duration::from_millis(7));
        let l = StopLatch::with_clock(tx, clock.clone());
        assert_eq!(l.tripped_at(), None);
        l.trip();
        clock.advance(Duration::from_millis(5));
        l.trip();
        assert_eq!(l.tripped_at(), Some(MonoTime(Duration::from_millis(7))), "the first pending trip counts");
        assert!(l.clear_if_generation(2));
        assert_eq!(l.tripped_at(), None);
    }

    #[test]
    fn stop_and_wait_returns_the_report_of_this_stop() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let latch = Arc::new(StopLatch::new(tx));
        let mailbox = Arc::new(StopMailbox::default());
        let stopper = Stopper::with_mailbox(Arc::clone(&latch), Arc::clone(&mailbox));
        mailbox.post(0, StopReport::unconfirmed("an older stop"));
        let poster = std::thread::spawn(move || {
            while latch.generation() == 0 {
                std::thread::yield_now();
            }
            mailbox.post(latch.generation(), StopReport { confirmed: true, ..StopReport::unconfirmed("this stop") });
        });
        let ack = stopper.stop_and_wait(Duration::from_secs(5));
        poster.join().unwrap();
        assert!(ack.latched);
        let report = ack.report.unwrap();
        assert!(report.confirmed);
        assert_eq!(report.notes, vec!["this stop".to_owned()]);
    }

    #[test]
    fn stop_and_wait_times_out_without_a_report() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let stopper = Stopper::with_mailbox(Arc::new(StopLatch::new(tx)), Arc::default());
        let ack = stopper.stop_and_wait(Duration::from_millis(30));
        assert!(ack.latched);
        assert_eq!(ack.report, None);
        assert!(ack.elapsed_ms >= 25);
    }

    #[test]
    fn trace_lines_are_capped_and_counted() {
        let (l, _w) = latch();
        let (tx, rx) = crossbeam_channel::unbounded();
        let clock = VirtualClock::new();
        let cx = Cx::new(&l, &tx, &clock);
        for _ in 0..(TRACE_LINES_PER_OPERATION + 5) {
            cx.trace(true, "x");
        }
        cx.emit(DriverEvent::State(crate::MachineState::Idle));
        cx.flush_dropped_traces();
        let all: Vec<DriverEvent> = rx.try_iter().collect();
        assert_eq!(all.len(), usize::try_from(TRACE_LINES_PER_OPERATION).unwrap() + 2);
        let last = all.last().unwrap();
        assert!(matches!(last, DriverEvent::Trace(t) if t.text == "5 trace lines dropped"));
    }

    #[test]
    fn slice_never_sleeps_more_than_20ms() {
        let (l, _w) = latch();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let clock = VirtualClock::new();
        let cx = Cx::new(&l, &tx, &clock);
        cx.slice(Duration::from_secs(3)).unwrap();
        assert_eq!(clock.now(), MonoTime(MAX_SLICE));
        l.trip();
        assert_eq!(cx.slice(Duration::from_millis(1)), Err(Stopped));
    }
}
