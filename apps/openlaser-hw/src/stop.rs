//! Stops from outside the main loop (SPEC §5.4): signals and panics.
//!
//! - **Signals.** SIGINT, SIGTERM and SIGHUP (signal-hook flags on unix; ctrlc's Ctrl-C/Ctrl-Break/console-close on
//!   Windows, §2.6) only store `true` into an `AtomicBool` (async-signal-safe). A watcher thread sees the flag, trips
//!   every latch, waits ≤ 500 ms for the StopReports, prints them, then exits. The driver never runs inside a
//!   signal handler.
//! - **Panics.** A panic hook stops every machine (≤ 500 ms) before the previous hook runs.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use openlaser_engine::{MachineId, StopAck, StopRegistry};

use crate::plain;

/// How long a signal or a panic waits for the StopReports before the process goes.
pub const STOP_WAIT: Duration = Duration::from_millis(500);
/// How often the watcher looks at the signal flags.
const WATCH_EVERY: Duration = Duration::from_millis(5);

/// Every machine the process drives. Implemented by the engine's `StopRegistry`.
pub trait StopTarget: Send + Sync {
    /// Trips every latch; never blocks for a report.
    fn stop_now(&self);
    /// Trips every latch, then waits ≤ `max` for each StopReport.
    fn stop_and_wait(&self, max: Duration) -> Vec<(MachineId, StopAck)>;
}

impl StopTarget for StopRegistry {
    fn stop_now(&self) {
        let _ = self.stop_all(Duration::ZERO);
    }

    fn stop_and_wait(&self, max: Duration) -> Vec<(MachineId, StopAck)> {
        self.stop_all(max)
    }
}

/// Counts every stop asked for through it (the key thread, the signal watcher, the panic hook and the flow), then
/// forwards it. The driver clears its latch for a command queued after a trip (SPEC §5.3), so a stop that lands
/// between the typed `ARM <challenge>` and the run being queued would be lost; the flow compares this count before
/// and after queueing to refuse or re-trip instead (see `flow::start_marking`).
pub struct CountingStop {
    inner: Arc<dyn StopTarget>,
    requests: AtomicU64,
}

impl CountingStop {
    pub fn new(inner: Arc<dyn StopTarget>) -> Self {
        CountingStop { inner, requests: AtomicU64::new(0) }
    }

    /// Stops asked for so far.
    pub fn requests(&self) -> u64 {
        self.requests.load(Ordering::SeqCst)
    }
}

impl StopTarget for CountingStop {
    /// Counts first, then trips: whoever reads an unchanged count knows the trip has not happened yet.
    fn stop_now(&self) {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.inner.stop_now();
    }

    fn stop_and_wait(&self, max: Duration) -> Vec<(MachineId, StopAck)> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.inner.stop_and_wait(max)
    }
}

/// The lines printed for a set of StopAcks (machine ids and driver notes as plain, bounded text).
pub fn report_lines(acks: &[(MachineId, StopAck)]) -> Vec<String> {
    if acks.is_empty() {
        return vec!["stop: no machine was connected".to_owned()];
    }
    acks.iter()
        .map(|(m, a)| (plain::field(&m.0, plain::NAME_MAX_CHARS), a))
        .map(|(m, a)| match &a.report {
            Some(r) => format!(
                "stop {}: {} in {} ms (MO closed: {}, ports at 0: {}{}){}",
                m,
                if r.confirmed { "confirmed" } else { "NOT CONFIRMED: use the key switch / E-stop" },
                r.elapsed.as_millis(),
                r.mo_closed,
                r.ports_zero,
                if r.possible_desync { ", possible desync" } else { "" },
                if r.notes.is_empty() { String::new() } else { format!(" — {}", plain::field(&r.notes.join("; "), plain::MESSAGE_MAX_CHARS)) },
            ),
            None if a.latched => {
                format!("stop {m}: latched; no StopReport within {} ms: use the key switch / E-stop if the laser is still active", a.elapsed_ms)
            }
            None => format!("stop {m}: NOT latched: use the key switch / E-stop"),
        })
        .collect()
}

/// One signal flag: what the handler sets, its name and the exit code it leads to.
pub struct SignalFlag {
    pub flag: Arc<AtomicBool>,
    pub name: &'static str,
    pub exit_code: u8,
}

/// The installed handlers. Dropping it unregisters them (unix).
pub struct Signals {
    pub flags: Vec<SignalFlag>,
    #[cfg(unix)]
    ids: Vec<signal_hook::SigId>,
}

impl Drop for Signals {
    fn drop(&mut self) {
        #[cfg(unix)]
        for id in self.ids.drain(..) {
            let _ = signal_hook::low_level::unregister(id);
        }
    }
}

/// SIGINT (130), SIGTERM (143), SIGHUP (129): each sets its own flag.
#[cfg(unix)]
pub fn install_signals() -> Result<Signals, String> {
    use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM};
    let mut out = Signals { flags: Vec::new(), ids: Vec::new() };
    for (sig, name, exit_code) in [(SIGINT, "SIGINT", 130u8), (SIGTERM, "SIGTERM", 143), (SIGHUP, "SIGHUP", 129)] {
        let flag = Arc::new(AtomicBool::new(false));
        let id = signal_hook::flag::register(sig, Arc::clone(&flag)).map_err(|e| format!("cannot install the {name} handler: {e}"))?;
        out.ids.push(id);
        out.flags.push(SignalFlag { flag, name, exit_code });
    }
    Ok(out)
}

/// Ctrl-C, Ctrl-Break and console close (ctrlc with `termination`). ctrlc allows one handler per process.
#[cfg(windows)]
pub fn install_signals() -> Result<Signals, String> {
    let flag = Arc::new(AtomicBool::new(false));
    let f = Arc::clone(&flag);
    ctrlc::set_handler(move || f.store(true, Ordering::SeqCst)).map_err(|e| format!("cannot install the Ctrl-C handler: {e}"))?;
    Ok(Signals { flags: vec![SignalFlag { flag, name: "Ctrl-C", exit_code: 130 }] })
}

#[cfg(not(any(unix, windows)))]
pub fn install_signals() -> Result<Signals, String> {
    Err("no signal handling on this platform".to_owned())
}

/// The watcher thread. Dropping it stops the thread (it never exits the process unless a flag fired).
pub struct Watcher {
    done: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// What the watcher does with its report lines (the binary: stderr).
pub type ReportSink = Box<dyn FnMut(&str) + Send>;

/// Starts the watcher: on the first flag it trips every latch, waits ≤ 500 ms for the reports, hands every line to
/// `sink`, then calls `exit` with the signal's code.
pub fn watch(flags: Vec<SignalFlag>, target: Arc<dyn StopTarget>, mut sink: ReportSink, exit: Arc<dyn Fn(u8) + Send + Sync>) -> Result<Watcher, String> {
    let done = Arc::new(AtomicBool::new(false));
    let d = Arc::clone(&done);
    let handle = std::thread::Builder::new()
        .name("openlaser-hw-signals".to_owned())
        .spawn(move || {
            while !d.load(Ordering::SeqCst) {
                if let Some(fired) = flags.iter().find(|f| f.flag.load(Ordering::SeqCst)) {
                    sink(&format!("\n{} received: stopping every machine", fired.name));
                    let acks = target.stop_and_wait(STOP_WAIT);
                    for l in report_lines(&acks) {
                        sink(&l);
                    }
                    exit(fired.exit_code);
                    return;
                }
                std::thread::sleep(WATCH_EVERY);
            }
        })
        .map_err(|e| format!("cannot start the signal watcher: {e}"))?;
    Ok(Watcher { done, handle: Some(handle) })
}

/// A panic anywhere stops every machine (≤ 500 ms) before the previous hook prints and the thread unwinds.
pub fn install_panic_hook(target: Arc<dyn StopTarget>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = target.stop_and_wait(STOP_WAIT);
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use openlaser_engine::StopReport;

    use super::*;

    /// Records stops; answers with a report after `delay`, as a driver finishing its abort would.
    struct Recording {
        stops: Mutex<Vec<&'static str>>,
        delay: Duration,
    }

    impl StopTarget for Recording {
        fn stop_now(&self) {
            self.stops.lock().unwrap().push("now");
        }

        fn stop_and_wait(&self, max: Duration) -> Vec<(MachineId, StopAck)> {
            self.stops.lock().unwrap().push("wait");
            std::thread::sleep(self.delay.min(max));
            let report = StopReport { confirmed: true, elapsed: self.delay, possible_desync: false, mo_closed: true, ports_zero: true, notes: Vec::new() };
            vec![(MachineId("jpt-m7-200".into()), StopAck { latched: true, report: Some(report), elapsed_ms: 0 })]
        }
    }

    #[test]
    fn cli_signal_waits_for_stop_report() {
        let target = Arc::new(Recording { stops: Mutex::new(Vec::new()), delay: Duration::from_millis(80) });
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let exited = Arc::new(Mutex::new(Vec::<(u8, usize)>::new()));
        let flag = Arc::new(AtomicBool::new(false));
        let l = Arc::clone(&lines);
        let e = Arc::clone(&exited);
        let l2 = Arc::clone(&lines);
        let _w = watch(
            vec![SignalFlag { flag: Arc::clone(&flag), name: "SIGTERM", exit_code: 143 }],
            target.clone(),
            Box::new(move |s| l.lock().unwrap().push(s.to_owned())),
            Arc::new(move |code| {
                let printed = l2.lock().unwrap().len();
                e.lock().unwrap().push((code, printed));
            }),
        )
        .unwrap();
        flag.store(true, Ordering::SeqCst);
        let t0 = std::time::Instant::now();
        while exited.lock().unwrap().is_empty() && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let exited = exited.lock().unwrap().clone();
        assert_eq!(exited.len(), 1);
        let (code, printed_before_exit) = exited[0];
        assert_eq!(code, 143);
        assert!(t0.elapsed() >= Duration::from_millis(80), "it waited for the report");
        assert_eq!(*target.stops.lock().unwrap(), vec!["wait"]);
        let lines = lines.lock().unwrap().clone();
        assert_eq!(printed_before_exit, lines.len(), "the report is printed before exit");
        assert!(lines.iter().any(|l| l.contains("stop jpt-m7-200: confirmed in 80 ms (MO closed: true, ports at 0: true)")), "{lines:?}");
    }

    #[test]
    fn watcher_drop_never_exits() {
        let target = Arc::new(Recording { stops: Mutex::new(Vec::new()), delay: Duration::ZERO });
        let exited = Arc::new(AtomicBool::new(false));
        let e = Arc::clone(&exited);
        let w = watch(
            vec![SignalFlag { flag: Arc::new(AtomicBool::new(false)), name: "SIGINT", exit_code: 130 }],
            target.clone(),
            Box::new(|_| {}),
            Arc::new(move |_| e.store(true, Ordering::SeqCst)),
        )
        .unwrap();
        drop(w);
        assert!(!exited.load(Ordering::SeqCst));
        assert!(target.stops.lock().unwrap().is_empty());
    }

    #[test]
    fn report_lines_say_when_to_reach_for_the_e_stop() {
        let none = vec![(MachineId("m".into()), StopAck { latched: true, report: None, elapsed_ms: 500 })];
        assert!(report_lines(&none)[0].contains("E-stop"));
        assert_eq!(report_lines(&[]), vec!["stop: no machine was connected".to_owned()]);
    }

    #[test]
    fn report_lines_are_plain_text() {
        let report = StopReport {
            confirmed: false,
            elapsed: Duration::from_millis(500),
            possible_desync: false,
            mo_closed: false,
            ports_zero: false,
            notes: vec!["abort\u{1b}[8m".into(), "\u{9b}2Jok\r".into()],
        };
        let acks = vec![(MachineId("m\u{1b}[2J".into()), StopAck { latched: true, report: Some(report), elapsed_ms: 500 })];
        let line = &report_lines(&acks)[0];
        assert!(!line.chars().any(plain::hidden), "{line:?}");
        assert!(line.starts_with("stop m[2J: NOT CONFIRMED: use the key switch / E-stop"), "{line}");
        assert!(line.ends_with(" — abort[8m; 2Jok"), "{line}");
    }

    /// A real SIGINT to this process sets the flag, and the watcher trips the latch of a real driver stopper
    /// through the engine's StopRegistry.
    #[cfg(unix)]
    #[test]
    fn cli_sigint_trips_latch() {
        use std::sync::Arc;

        use openlaser_driver::LmcBackend;
        use openlaser_lmc_sim::{FakeLmc, VirtualClock};
        use openlaser_machine::{Kinematics, jpt_m7_200_sim};
        use openlaser_types::SessionId;

        let profile = jpt_m7_200_sim();
        let Kinematics::Galvo(galvo) = profile.kind else { panic!("galvo profile") };
        let clock: Arc<dyn openlaser_types::Clock> = Arc::new(VirtualClock::new());
        let fake = FakeLmc::new(crate::default_sim_config(), Arc::clone(&clock));
        let handle =
            openlaser_driver::spawn(LmcBackend::new(Box::new(fake) as Box<dyn openlaser_driver::LmcTransport>, galvo), profile.id.clone(), SessionId(1), clock);
        let registry = Arc::new(StopRegistry::default());
        registry.register(profile.id.clone(), handle.stopper());

        let signals = install_signals().unwrap();
        let exited = Arc::new(Mutex::new(None::<u8>));
        let e = Arc::clone(&exited);
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let l = Arc::clone(&lines);
        let flags = signals.flags.iter().map(|f| SignalFlag { flag: Arc::clone(&f.flag), name: f.name, exit_code: f.exit_code }).collect();
        let _w = watch(flags, registry, Box::new(move |s| l.lock().unwrap().push(s.to_owned())), Arc::new(move |c| *e.lock().unwrap() = Some(c))).unwrap();

        signal_hook::low_level::raise(signal_hook::consts::signal::SIGINT).unwrap();
        let t0 = std::time::Instant::now();
        while exited.lock().unwrap().is_none() && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(*exited.lock().unwrap(), Some(130));
        let lines = lines.lock().unwrap().clone();
        assert!(lines.iter().any(|l| l.contains("SIGINT received")), "{lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("stop jpt-m7-200-sim:") && !l.contains("NOT latched")), "the latch tripped: {lines:?}");
        drop(signals);
        drop(handle);
    }

    #[test]
    fn panic_hook_stops_before_unwinding() {
        let target = Arc::new(Recording { stops: Mutex::new(Vec::new()), delay: Duration::ZERO });
        install_panic_hook(target.clone());
        let r = std::thread::spawn(|| panic!("boom in a test-only thread")).join();
        // Leave the default hook behind for the rest of the binary.
        let _ = std::panic::take_hook();
        assert!(r.is_err());
        assert!(target.stops.lock().unwrap().contains(&"wait"));
    }
}
