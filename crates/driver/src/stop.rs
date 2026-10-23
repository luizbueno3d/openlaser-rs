//! The stop path: latch, stopper, per-call context.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crossbeam_channel::Sender;
use openlaser_types::{Clock, MonoTime};
use serde::Serialize;

use crate::backend::DriverEvent;

/// Tripped by anyone, from any thread; read by the driver between every transfer. Signal handlers never call
/// `trip()` (a channel send is not async-signal-safe): they only store `true` into `signal_flag()` (signal-hook's
/// `flag::register`), which `is_tripped()` also reads, and the app's watcher thread then calls `trip()`.
/// `generation()` counts trips; every queued command records it at submission (§5.3 "queued-then-stopped").
pub struct StopLatch {
    tripped: AtomicBool,
    signal: Arc<AtomicBool>,
    generation: AtomicU64,
    tripped_at_ns: AtomicU64,
    wake: Sender<()>,
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
        }
    }

    pub fn trip(&self) {
        self.tripped.store(true, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
        let _ = self.tripped_at_ns.load(Ordering::Relaxed);
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
}

/// Placeholder for the driver's stop-report mailbox (WP5).
pub(crate) struct StopMailbox;

/// Cheap, cloneable, Send + Sync handle for the control server's fast path, Esc, signals and the CLI key thread.
#[derive(Clone)]
pub struct Stopper {
    latch: Arc<StopLatch>,
    mailbox: Arc<StopMailbox>,
}

impl Stopper {
    pub(crate) fn new(latch: Arc<StopLatch>) -> Self {
        Stopper { latch, mailbox: Arc::new(StopMailbox) }
    }

    /// Atomic store + wake; never blocks, never does I/O.
    pub fn stop(&self) {
        self.latch.trip();
    }

    /// stop(), then wait ≤ max for the StopReport. WP0 stub: latches and returns at once without a report.
    pub fn stop_and_wait(&self, max: Duration) -> StopAck {
        self.stop();
        let _ = (max, &self.mailbox);
        StopAck { latched: self.latch.is_tripped(), report: None, elapsed_ms: 0 }
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

/// What the driver thread gets each call: stop checks, events and the clock.
pub struct Cx<'a> {
    latch: &'a StopLatch,
    events: &'a Sender<DriverEvent>,
    clock: &'a dyn Clock,
}

impl<'a> Cx<'a> {
    /// WP0 addition: built by the runtime (and by backend tests).
    pub fn new(latch: &'a StopLatch, events: &'a Sender<DriverEvent>, clock: &'a dyn Clock) -> Self {
        Cx { latch, events, clock }
    }
}

impl Cx<'_> {
    /// Err as soon as the latch is tripped.
    pub fn check(&self) -> Result<(), Stopped> {
        if self.latch.is_tripped() { Err(Stopped) } else { Ok(()) }
    }

    /// Sleep ≤ min(d, 20 ms), then check.
    pub fn slice(&self, d: Duration) -> Result<(), Stopped> {
        self.check()?;
        self.clock.sleep(d.min(Duration::from_millis(20)));
        self.check()
    }

    pub fn now(&self) -> MonoTime {
        self.clock.now()
    }

    /// Never blocks: events go through an UNBOUNDED crossbeam channel, so a UI that stops polling (window hidden,
    /// UI thread stalled) can never stall the driver thread or its stop checks. Trace lines beyond 100 000 per
    /// operation are dropped and counted (`TraceLine` "N trace lines dropped").
    pub fn emit(&self, ev: DriverEvent) {
        let _ = self.events.try_send(ev);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopped;
