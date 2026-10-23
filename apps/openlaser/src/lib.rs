//! Process lifecycle and argument parsing, testable without a window or a device.
#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

pub mod control_runtime;
pub mod native_files;
#[cfg(target_os = "macos")]
pub mod native_menu;

pub const STOP_BUDGET: Duration = Duration::from_millis(500);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub control: Option<u16>,
    pub token_file: Option<PathBuf>,
    pub read_roots: Vec<PathBuf>,
    pub write_roots: Vec<PathBuf>,
    pub in_window_menus: bool,
    pub files: Vec<PathBuf>,
}

/// Arguments exclude argv[0]. Reject non-Unicode before opening any window.
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
            _ if arg.starts_with('-') => return Err(format!("unknown option `{arg}`")),
            _ => parsed.files.push(PathBuf::from(arg)),
        }
    }
    if parsed.read_roots.len() > 1 || parsed.write_roots.len() > 1 {
        return Err("only one read root and one write root are supported".into());
    }
    Ok(parsed)
}

fn panic_stop_then_default(stop: impl FnOnce(Duration), default: impl FnOnce()) {
    stop(STOP_BUDGET);
    default();
}

pub fn install_panic_hook(registry: Arc<openlaser_engine::StopRegistry>) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        panic_stop_then_default(
            |budget| {
                let _ = registry.stop_all(budget);
            },
            || default(info),
        );
    }));
}

/// Keep the watcher alive for the lifetime of run_native; dropping it wakes no devices.
pub struct SignalWatcher(Arc<AtomicBool>);
impl Drop for SignalWatcher {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn watch_signal(flag: &AtomicBool, shutdown: &AtomicBool, stop: impl FnOnce(Duration), exit: impl FnOnce(i32)) {
    while !flag.load(Ordering::SeqCst) {
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    stop(STOP_BUDGET);
    exit(143);
}

pub fn install_signal_watcher(registry: Arc<openlaser_engine::StopRegistry>) -> Result<SignalWatcher, String> {
    let flag = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM, signal_hook::consts::SIGHUP] {
        signal_hook::flag::register(signal, Arc::clone(&flag)).map_err(|e| e.to_string())?;
    }
    #[cfg(windows)]
    {
        let flag = Arc::clone(&flag);
        ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst)).map_err(|e| e.to_string())?;
    }
    let shutdown = Arc::new(AtomicBool::new(false));
    let done = Arc::clone(&shutdown);
    std::thread::Builder::new()
        .name("openlaser-signal".into())
        .spawn(move || {
            watch_signal(
                &flag,
                &done,
                |budget| {
                    // Latch ALL machines first. The outer deadline bounds the aggregate wait, even if
                    // a dependency waits its budget per machine.
                    let _ = registry.stop_all(Duration::ZERO);
                    let (tx, rx) = std::sync::mpsc::channel();
                    let _ = std::thread::Builder::new().name("openlaser-signal-reports".into()).spawn(move || {
                        let _ = tx.send(registry.stop_all(budget));
                    });
                    let _ = rx.recv_timeout(budget);
                },
                |code| std::process::exit(code),
            );
        })
        .map_err(|e| e.to_string())?;
    Ok(SignalWatcher(shutdown))
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
            ]
            .map(OsString::from),
        )
        .unwrap();
        assert_eq!(args.control, Some(1234));
        assert!(args.in_window_menus);
        assert_eq!(args.token_file, Some(PathBuf::from("token")));
        assert_eq!(args.read_roots, [PathBuf::from("read")]);
        assert_eq!(args.write_roots, [PathBuf::from("write")]);
        assert_eq!(args.files, [PathBuf::from("drawing.olp")]);
        for args in [vec!["--control", "0"], vec!["--control", "65536"], vec!["--control"], vec!["--bad"]] {
            assert!(parse_args(args.into_iter().map(OsString::from)).is_err());
        }
    }
    #[test]
    fn non_unicode_is_usage_error() {
        #[cfg(unix)]
        let arg = {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(vec![0xff])
        };
        #[cfg(windows)]
        let arg = {
            use std::os::windows::ffi::OsStringExt;
            OsString::from_wide(&[0xd800])
        };
        assert_eq!(parse_args([arg]).unwrap_err(), "non-Unicode argument");
    }
    #[test]
    fn panic_stops_before_default_hook() {
        let events = std::cell::RefCell::new(Vec::new());
        panic_stop_then_default(
            |budget| {
                assert_eq!(budget, STOP_BUDGET);
                events.borrow_mut().push("stop");
            },
            || events.borrow_mut().push("default"),
        );
        assert_eq!(*events.borrow(), ["stop", "default"]);
    }
    #[test]
    fn signal_stops_before_exit_143() {
        let flag = AtomicBool::new(true);
        let stopped = AtomicBool::new(false);
        watch_signal(
            &flag,
            &AtomicBool::new(false),
            |budget| {
                assert_eq!(budget, STOP_BUDGET);
                stopped.store(true, Ordering::SeqCst);
            },
            |code| {
                assert!(stopped.load(Ordering::SeqCst));
                assert_eq!(code, 143);
            },
        );
        watch_signal(&AtomicBool::new(false), &AtomicBool::new(true), |_| panic!("stopped on shutdown"), |_| panic!("exit on shutdown"));
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
