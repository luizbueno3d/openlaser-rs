//! Process lifecycle and argument parsing, testable in process without a window or a device (AGENTS.md §5).
//!
//! SPEC §3.21: before `run_native` the app takes `session.stop_registry()`, installs a panic hook that stops
//! every machine (≤ 500 ms) before the default hook runs, and a signal watcher (signal-hook flags on unix, ctrlc
//! on Windows) that trips every latch, waits ≤ 500 ms for the reports, then exits 143.
#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use openlaser_engine::{MachineId, StopAck, StopRegistry};

pub mod control_runtime;
pub mod native_files;
#[cfg(target_os = "macos")]
pub mod native_menu;

/// How long a panic, a signal or the control fast path waits for the stop reports (SPEC §5.4).
pub const STOP_BUDGET: Duration = Duration::from_millis(500);
/// Exit status of a usage error (SPEC §3.21).
pub const EXIT_USAGE: u8 = 2;
/// Exit status after a terminating signal (128 + SIGTERM, on every OS).
pub const EXIT_SIGNAL: i32 = 143;

pub const USAGE: &str = "usage: openlaser [--control <port>] [--control-token-file <path>] [--automation-read-root <dir>] \
                         [--automation-write-root <dir>] [--in-window-menus] [files…]";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub control: Option<u16>,
    pub token_file: Option<PathBuf>,
    pub read_roots: Vec<PathBuf>,
    pub write_roots: Vec<PathBuf>,
    pub in_window_menus: bool,
    pub files: Vec<PathBuf>,
}

/// Arguments exclude argv[0]. Non-Unicode arguments and unknown flags are refused before any window opens.
pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let arg = arg.into_string().map_err(|_| "non-Unicode argument".to_owned())?;
        match arg.as_str() {
            "--in-window-menus" => parsed.in_window_menus = true,
            "--control" | "--control-token-file" | "--automation-read-root" | "--automation-write-root" => {
                let value = args.next().ok_or_else(|| format!("{arg} requires a value"))?.into_string().map_err(|_| "non-Unicode argument".to_owned())?;
                match arg.as_str() {
                    "--control" => {
                        let port = value.parse::<u16>().map_err(|_| "--control requires a port between 1 and 65535".to_owned())?;
                        if port == 0 {
                            return Err("--control requires a port between 1 and 65535".to_owned());
                        }
                        parsed.control = Some(port);
                    }
                    "--control-token-file" => parsed.token_file = Some(PathBuf::from(value)),
                    "--automation-read-root" => parsed.read_roots.push(PathBuf::from(value)),
                    _ => parsed.write_roots.push(PathBuf::from(value)),
                }
            }
            "--" => {
                for rest in args.by_ref() {
                    parsed.files.push(PathBuf::from(rest.into_string().map_err(|_| "non-Unicode argument".to_owned())?));
                }
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option `{arg}`")),
            _ => parsed.files.push(PathBuf::from(arg)),
        }
    }
    if parsed.read_roots.len() > 1 || parsed.write_roots.len() > 1 {
        return Err("only one read root and one write root are supported".into());
    }
    Ok(parsed)
}

/// `main`'s first step: the parsed arguments, or the exit status after the usage message was written to `stderr`.
pub fn preflight(args: impl IntoIterator<Item = OsString>, stderr: &mut dyn std::io::Write) -> Result<Args, u8> {
    parse_args(args).map_err(|message| {
        let _ = writeln!(stderr, "openlaser: {message}\n{USAGE}");
        EXIT_USAGE
    })
}

/// Latches every registered machine at once, then waits at most `budget` in total for the reports (a helper thread
/// bounds the aggregate wait even if the registry waits per machine). Never blocks longer than `budget`.
pub fn stop_all_bounded(registry: &Arc<StopRegistry>, budget: Duration) -> Vec<(MachineId, StopAck)> {
    let latched = registry.stop_all(Duration::ZERO);
    let (tx, rx) = std::sync::mpsc::channel();
    let waiting = Arc::clone(registry);
    let spawned = std::thread::Builder::new().name("openlaser-stop-reports".into()).spawn(move || {
        let _ = tx.send(waiting.stop_all(budget));
    });
    if spawned.is_err() {
        return latched;
    }
    rx.recv_timeout(budget).unwrap_or(latched)
}

/// The stop action the panic hook and the signal watcher run: every machine, bounded by the budget.
pub fn registry_stop(registry: Arc<StopRegistry>) -> impl Fn(Duration) + Send + Sync + 'static {
    move |budget| {
        let _ = stop_all_bounded(&registry, budget);
    }
}

/// Installs a panic hook that runs `stop(STOP_BUDGET)` first and then the previously installed (default) hook.
pub fn install_panic_hook(stop: impl Fn(Duration) + Send + Sync + 'static) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        stop(STOP_BUDGET);
        default(info);
    }));
}

/// The signals the watcher handles on unix (Windows: ctrlc with `termination`: Ctrl-C, Ctrl-Break, console close).
#[cfg(unix)]
pub const STOP_SIGNALS: [std::ffi::c_int; 3] = [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM, signal_hook::consts::SIGHUP];

/// Keeps the watcher thread for the life of the window; dropping it ends the thread without stopping anything.
pub struct SignalWatcher {
    shutdown: Arc<AtomicBool>,
}

impl Drop for SignalWatcher {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

fn watch(flag: &AtomicBool, shutdown: &AtomicBool, stop: impl FnOnce(Duration), exit: impl FnOnce(i32)) {
    while !flag.load(Ordering::SeqCst) {
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    stop(STOP_BUDGET);
    exit(EXIT_SIGNAL);
}

fn spawn_watcher(
    flag: Arc<AtomicBool>,
    stop: impl FnOnce(Duration) + Send + 'static,
    exit: impl FnOnce(i32) + Send + 'static,
) -> Result<SignalWatcher, String> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let done = Arc::clone(&shutdown);
    std::thread::Builder::new().name("openlaser-signal".into()).spawn(move || watch(&flag, &done, stop, exit)).map_err(|e| e.to_string())?;
    Ok(SignalWatcher { shutdown })
}

#[cfg(unix)]
fn install_for(
    signals: &[std::ffi::c_int],
    stop: impl FnOnce(Duration) + Send + 'static,
    exit: impl FnOnce(i32) + Send + 'static,
) -> Result<SignalWatcher, String> {
    let flag = Arc::new(AtomicBool::new(false));
    for signal in signals {
        // A flag store is async-signal-safe; the watcher thread does the stop (driver::StopLatch docs).
        signal_hook::flag::register(*signal, Arc::clone(&flag)).map_err(|e| e.to_string())?;
    }
    spawn_watcher(flag, stop, exit)
}

/// SIGINT/SIGTERM/SIGHUP (unix) or Ctrl-C/Ctrl-Break/console close (Windows) → `stop(STOP_BUDGET)` → `exit(143)`.
/// The app passes [`registry_stop`] and `std::process::exit`.
pub fn install_signal_watcher(stop: impl FnOnce(Duration) + Send + 'static, exit: impl FnOnce(i32) + Send + 'static) -> Result<SignalWatcher, String> {
    #[cfg(unix)]
    {
        install_for(&STOP_SIGNALS, stop, exit)
    }
    #[cfg(windows)]
    {
        let flag = Arc::new(AtomicBool::new(false));
        let handler = Arc::clone(&flag);
        ctrlc::set_handler(move || handler.store(true, Ordering::SeqCst)).map_err(|e| e.to_string())?;
        spawn_watcher(flag, stop, exit)
    }
    #[cfg(not(any(unix, windows)))]
    {
        spawn_watcher(Arc::new(AtomicBool::new(false)), stop, exit)
    }
}

/// Small executor for native async file sheets. Runs on the dialog worker, never the UI thread.
pub fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    struct ThreadWake(std::thread::Thread);
    impl std::task::Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(value) => return value,
            std::task::Poll::Pending => std::thread::park(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::mpsc;

    #[test]
    fn arguments_are_retained_and_invalid_ports_refused() {
        let args = parse_args(
            [
                "--control",
                "1234",
                "--in-window-menus",
                "--control-token-file",
                "token",
                "--automation-read-root",
                "read",
                "--automation-write-root",
                "write",
                "drawing.olp",
                "--",
                "--odd-name.svg",
            ]
            .map(OsString::from),
        )
        .unwrap();
        assert_eq!(args.control, Some(1234));
        assert!(args.in_window_menus);
        assert_eq!(args.token_file, Some(PathBuf::from("token")));
        assert_eq!(args.read_roots, [PathBuf::from("read")]);
        assert_eq!(args.write_roots, [PathBuf::from("write")]);
        assert_eq!(args.files, [PathBuf::from("drawing.olp"), PathBuf::from("--odd-name.svg")]);
        for args in [vec!["--control", "0"], vec!["--control", "65536"], vec!["--control", "x"], vec!["--control"], vec!["--bad"]] {
            assert!(parse_args(args.into_iter().map(OsString::from)).is_err());
        }
    }

    fn non_unicode() -> OsString {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(vec![b'a', 0xff])
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            OsString::from_wide(&[0x61, 0xd800])
        }
    }

    #[test]
    fn non_unicode_argument_is_a_usage_error_with_exit_2() {
        for args in [vec![non_unicode()], vec![OsString::from("--control-token-file"), non_unicode()], vec![OsString::from("--"), non_unicode()]] {
            let mut stderr = Vec::new();
            assert_eq!(preflight(args, &mut stderr), Err(2));
            let text = String::from_utf8(stderr).unwrap();
            assert!(text.contains("non-Unicode argument") && text.contains("usage: openlaser"), "{text}");
        }
        let mut stderr = Vec::new();
        assert_eq!(preflight([OsString::from("--control"), OsString::from("0")], &mut stderr), Err(EXIT_USAGE));
        assert!(preflight([OsString::from("a.svg")], &mut Vec::new()).is_ok());
    }

    /// The panic hook is process-global; this lock keeps the one test that swaps it from racing itself.
    static HOOK: Mutex<()> = Mutex::new(());

    #[test]
    fn installed_panic_hook_stops_every_machine_before_the_default_hook() {
        let _guard = HOOK.lock().unwrap_or_else(|e| e.into_inner());
        let me = std::thread::current().id();
        let events: Arc<Mutex<Vec<(&'static str, std::thread::ThreadId)>>> = Arc::default();
        let previous = std::panic::take_hook();
        let recorder = Arc::clone(&events);
        // Stands in for the default hook the app chains to.
        std::panic::set_hook(Box::new(move |_| recorder.lock().unwrap().push(("default", std::thread::current().id()))));
        let recorder = Arc::clone(&events);
        install_panic_hook(move |budget| {
            assert_eq!(budget, STOP_BUDGET);
            recorder.lock().unwrap().push(("stop", std::thread::current().id()));
        });
        let result = std::panic::catch_unwind(|| std::panic::panic_any("simulated UI panic"));
        let _ = std::panic::take_hook();
        std::panic::set_hook(previous);
        assert!(result.is_err());
        let mine: Vec<&str> = events.lock().unwrap().iter().filter(|(_, thread)| *thread == me).map(|(what, _)| *what).collect();
        assert_eq!(mine, ["stop", "default"]);
    }

    #[test]
    fn registry_stop_is_bounded_and_returns() {
        let registry = Arc::new(StopRegistry::default());
        let started = std::time::Instant::now();
        registry_stop(Arc::clone(&registry))(STOP_BUDGET);
        assert!(stop_all_bounded(&registry, STOP_BUDGET).is_empty());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[derive(Debug, PartialEq)]
    enum Step {
        Stop(Duration),
        Exit(i32),
    }

    #[cfg(unix)]
    #[test]
    fn a_real_signal_stops_then_exits_143() {
        let (tx, rx) = mpsc::channel();
        let exit_tx = tx.clone();
        // SIGUSR2 stands in for SIGINT/SIGTERM/SIGHUP: raising those in the test process would also swallow the
        // harness's own termination signals once registered.
        let _watcher = install_for(
            &[signal_hook::consts::SIGUSR2],
            move |budget| tx.send(Step::Stop(budget)).unwrap(),
            move |code| exit_tx.send(Step::Exit(code)).unwrap(),
        )
        .unwrap();
        signal_hook::low_level::raise(signal_hook::consts::SIGUSR2).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Step::Stop(STOP_BUDGET));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Step::Exit(143));
        assert_eq!(STOP_SIGNALS, [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM, signal_hook::consts::SIGHUP]);
    }

    #[test]
    fn watcher_stops_before_exit_and_shutdown_never_stops() {
        let (tx, rx) = mpsc::channel();
        let exit_tx = tx.clone();
        let flag = Arc::new(AtomicBool::new(false));
        let _watcher =
            spawn_watcher(Arc::clone(&flag), move |budget| tx.send(Step::Stop(budget)).unwrap(), move |code| exit_tx.send(Step::Exit(code)).unwrap()).unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err(), "nothing happens before a signal");
        flag.store(true, Ordering::SeqCst);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Step::Stop(STOP_BUDGET));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Step::Exit(143));

        let (tx, rx) = mpsc::channel::<Step>();
        let exit_tx = tx.clone();
        let watcher = spawn_watcher(
            Arc::new(AtomicBool::new(false)),
            move |budget| tx.send(Step::Stop(budget)).unwrap(),
            move |code| exit_tx.send(Step::Exit(code)).unwrap(),
        )
        .unwrap();
        drop(watcher);
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err(), "dropping the watcher must not stop or exit");
    }

    #[test]
    fn executor_handles_wakeup() {
        struct Yield(bool);
        impl std::future::Future for Yield {
            type Output = u32;
            fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<u32> {
                if self.0 {
                    std::task::Poll::Ready(42)
                } else {
                    self.0 = true;
                    cx.waker().wake_by_ref();
                    std::task::Poll::Pending
                }
            }
        }
        assert_eq!(block_on(Yield(false)), 42);
    }
}
