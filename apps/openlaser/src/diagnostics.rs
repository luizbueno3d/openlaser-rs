//! Persistent diagnostics (Alexandre's unexpected closes, 2026-11): a log file, a session marker and, on Windows, a
//! supervisor process, so every end of the GUI leaves a trace that tells a Rust panic, a native crash, an
//! out-of-memory abort, a kill and an intentional close apart.
//!
//! Where: `<data dir>/OpenLaser/logs/` (`%LOCALAPPDATA%\OpenLaser\logs` on Windows), or `OPENLASER_LOG_DIR`.
//! - `openlaser.log`: timestamped records (UTC) of the `log` facade, the child's stderr (panic text, Rust's
//!   "memory allocation of N bytes failed", "has overflowed its stack") and the supervisor's exit record. Rotated to
//!   `openlaser.old.log` at start once it is over [`ROTATE_BYTES`].
//! - `session.txt`: the marker. `state=running` while a GUI runs; `state=ended` with the reason after a clean end;
//!   `state=died` with the exit status when the supervisor saw the child die. A start that finds `state=running`
//!   reports that the previous run did not end cleanly (killed, power loss, or a crash with no supervisor).
//!
//! Why a supervisor and no exception filter: the exit status of the child already names the failure
//! (0xC0000005 access violation, 0xC0000409 fail-fast/abort, 0xC00000FD stack overflow), Windows Error Reporting
//! names the faulting DLL (Event 1000, read by the diagnostics script), and the child's stderr carries Rust's own
//! last words. That needs no `unsafe` code (AGENTS.md: `unsafe_code = "forbid"`), and it also catches the failures an
//! in-process filter cannot (`__fastfail`, which Rust's abort and the OOM handler use, bypasses every filter).
#![forbid(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Overrides the log folder (tests, or a person who wants the log elsewhere).
pub const LOG_DIR_VAR: &str = "OPENLASER_LOG_DIR";
/// Set in the child the supervisor starts; a supervised child never supervises again.
pub const SUPERVISED_VAR: &str = "OPENLASER_SUPERVISED";
/// `1` turns the supervisor off (Windows) or on (other systems, where it is off by default).
pub const SUPERVISOR_VAR: &str = "OPENLASER_SUPERVISOR";
pub const LOG_FILE: &str = "openlaser.log";
pub const OLD_LOG_FILE: &str = "openlaser.old.log";
pub const SESSION_FILE: &str = "session.txt";
/// The log is rotated at start once it is bigger than this.
pub const ROTATE_BYTES: u64 = 4 * 1024 * 1024;
/// One process writes at most this much; a runaway warning loop cannot fill the disk.
pub const SESSION_CAP_BYTES: u64 = 32 * 1024 * 1024;
/// How often the heartbeat records memory and the UI its frame rate and textures (often enough to see a leak's slope
/// before it kills a 32-bit process: Alexandre's died 92 s after start).
pub const HEARTBEAT: Duration = Duration::from_secs(20);
/// The `log` target of every record that says why the GUI is closing; the last one becomes the exit reason.
pub const EXIT_TARGET: &str = "openlaser::exit";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The log folder: `OPENLASER_LOG_DIR`, else `<data dir>/OpenLaser/logs`.
pub fn log_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(LOG_DIR_VAR).filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    craft_settings::paths::data_dir("OpenLaser").map(|d| d.join("logs"))
}

/// `2026-11-03T14:05:09.123Z`: UTC, so it needs no time-zone data (the diagnostics script prints the PC's offset).
pub fn utc_timestamp(at: SystemTime) -> String {
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil_from_days(i64::try_from(days).unwrap_or(0));
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3600, rem % 3600 / 60, rem % 60, since.subsec_millis())
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian (H. Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, u32::try_from(month).unwrap_or(1), u32::try_from(day).unwrap_or(1))
}

// ---- the logger ------------------------------------------------------------------------------------------------------

struct Sink {
    file: File,
    written: u64,
    capped: bool,
    /// The last record's text and how often it repeated since, so a per-frame warning is one line, not thousands.
    last: String,
    repeats: u64,
}

/// A `log::Log` that appends to `openlaser.log`. OpenLaser's own targets at Info and above, everything else (wgpu,
/// eframe, winit) at Warn and above.
pub struct FileLogger {
    path: PathBuf,
    sink: Mutex<Sink>,
    /// The last few [`EXIT_TARGET`] records, oldest first.
    exit_reason: Mutex<Vec<String>>,
}

impl FileLogger {
    /// Opens (creates) the log for appending; `rotate` moves an oversized log aside first.
    pub fn open(dir: &Path, rotate: bool) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(LOG_FILE);
        if rotate && std::fs::metadata(&path).map(|m| m.len() > ROTATE_BYTES).unwrap_or(false) {
            let _ = std::fs::rename(&path, dir.join(OLD_LOG_FILE));
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self { path, sink: Mutex::new(Sink { file, written: 0, capped: false, last: String::new(), repeats: 0 }), exit_reason: Mutex::new(Vec::new()) })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The last records logged under [`EXIT_TARGET`] (at most 3), joined by ` → `: why the GUI was closing.
    pub fn exit_reason(&self) -> Option<String> {
        lock(&self.exit_reason).filter(|r| !r.is_empty()).map(|r| r.join(" → "))
    }

    /// Writes one line (or a block) with the timestamp, the level and the thread.
    pub fn write_record(&self, level: log::Level, target: &str, message: &str) {
        let Some(mut sink) = lock(&self.sink) else { return };
        let key = format!("{level} {target}: {message}");
        if key == sink.last {
            sink.repeats = sink.repeats.saturating_add(1);
            if !sink.repeats.is_multiple_of(1000) {
                return;
            }
        }
        let mut text = String::new();
        let now = utc_timestamp(SystemTime::now());
        if sink.repeats > 0 && key != sink.last {
            text.push_str(&format!("{now} (the previous record repeated {} more times)\n", sink.repeats));
            sink.repeats = 0;
        }
        let thread = std::thread::current();
        text.push_str(&format!("{now} {level:<5} [{}] {key}\n", thread.name().unwrap_or("?")));
        sink.last = key;
        let len = u64::try_from(text.len()).unwrap_or(u64::MAX);
        if sink.capped {
            return;
        }
        if sink.written.saturating_add(len) > SESSION_CAP_BYTES {
            sink.capped = true;
            text = format!("{now} WARN  log cap of {} MiB reached: nothing more is written by this process\n", SESSION_CAP_BYTES >> 20);
        }
        // One write per record: an append-mode write is atomic, so the supervisor's and the child's lines never mix.
        if sink.file.write_all(text.as_bytes()).is_ok() {
            sink.written = sink.written.saturating_add(len);
        }
    }
}

/// The logger's lock, poison or not; gives up after 200 ms instead of deadlocking a panic hook that fired while the
/// same thread was inside the logger.
fn lock<T>(mutex: &Mutex<T>) -> Option<std::sync::MutexGuard<'_, T>> {
    let deadline = Instant::now() + Duration::from_millis(200);
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Some(guard),
            Err(std::sync::TryLockError::Poisoned(p)) => return Some(p.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(1)),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        }
    }
}

/// OpenLaser's own crates log at Info; the graphics and windowing stack only from Warn.
pub fn wanted(level: log::Level, target: &str) -> bool {
    if target.starts_with("openlaser") { level <= log::Level::Info } else { level <= log::Level::Warn }
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        wanted(metadata.level(), metadata.target())
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let message = record.args().to_string();
        if record.target() == EXIT_TARGET
            && let Some(mut reason) = lock(&self.exit_reason)
        {
            if reason.len() >= 3 {
                reason.remove(0);
            }
            reason.push(message.clone());
        }
        self.write_record(record.level(), record.target(), &message);
    }

    fn flush(&self) {
        if let Some(mut sink) = lock(&self.sink) {
            let _ = sink.file.flush();
        }
    }
}

static LOGGER: OnceLock<FileLogger> = OnceLock::new();

/// Installs the file logger as the process's `log` logger. `None` when the folder cannot be written (the app runs
/// on without a log) or a logger is already installed.
pub fn install(dir: &Path, rotate: bool) -> Option<&'static FileLogger> {
    if let Some(installed) = LOGGER.get() {
        return Some(installed);
    }
    let logger = FileLogger::open(dir, rotate).ok()?;
    let logger = LOGGER.get_or_init(|| logger);
    log::set_logger(logger).ok()?;
    log::set_max_level(log::LevelFilter::Info);
    Some(logger)
}

/// The installed file logger, if any.
pub fn logger() -> Option<&'static FileLogger> {
    LOGGER.get()
}

/// A short description of the OS, CPU and build for the first record of a run.
pub fn build_line() -> String {
    format!(
        "OpenLaser {VERSION} ({} {}, {} build), pid {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) { "debug" } else { "release" },
        std::process::id()
    )
}

// ---- the session marker ----------------------------------------------------------------------------------------------

/// What the previous run left in `session.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviousRun {
    /// No marker (first start) or it ended cleanly.
    Clean,
    /// `state=running`: it never wrote its end. Killed (Task Manager, `taskkill /F`), power loss, or a crash with no
    /// supervisor.
    Unclean { pid: String, version: String, started: String },
    /// `state=died`: the supervisor saw it die and wrote the status.
    Died { pid: String, version: String, started: String, exit: String },
}

fn field<'a>(text: &'a str, key: &str) -> &'a str {
    text.lines().find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('='))).unwrap_or("?")
}

/// Reads the previous marker.
pub fn previous_run(dir: &Path) -> PreviousRun {
    let Ok(text) = std::fs::read_to_string(dir.join(SESSION_FILE)) else { return PreviousRun::Clean };
    let (pid, version, started) = (field(&text, "pid").to_owned(), field(&text, "version").to_owned(), field(&text, "started").to_owned());
    match field(&text, "state") {
        "running" => PreviousRun::Unclean { pid, version, started },
        "died" => PreviousRun::Died { pid, version, started, exit: field(&text, "exit").to_owned() },
        _ => PreviousRun::Clean,
    }
}

/// The text a start logs about the previous run, if it did not end cleanly.
pub fn describe_previous(previous: &PreviousRun) -> Option<String> {
    match previous {
        PreviousRun::Clean => None,
        PreviousRun::Unclean { pid, version, started } => Some(format!(
            "THE PREVIOUS RUN DID NOT END CLEANLY: OpenLaser {version}, pid {pid}, started {started}, wrote no end record \
             (killed by Task Manager or taskkill /F, power loss, or a crash the supervisor did not see). Its last records are above."
        )),
        PreviousRun::Died { pid, version, started, exit } => {
            Some(format!("THE PREVIOUS RUN DIED: OpenLaser {version}, pid {pid}, started {started}: {exit}. Its last records are above."))
        }
    }
}

/// Writes `state=running` for this process. Returns what the previous run left.
pub fn begin_session(dir: &Path) -> PreviousRun {
    let previous = previous_run(dir);
    let text = format!(
        "state=running\npid={}\nversion={VERSION}\narch={}\nstarted={}\n",
        std::process::id(),
        std::env::consts::ARCH,
        utc_timestamp(SystemTime::now())
    );
    let _ = std::fs::write(dir.join(SESSION_FILE), text);
    previous
}

/// Rewrites the marker as `state=<state>`, keeping its pid/version/started, with `key=value` added. Only when the
/// marker belongs to `pid` (a later start may already own it).
fn finish(dir: &Path, pid: u32, state: &str, key: &str, value: &str) {
    let path = dir.join(SESSION_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    if field(&text, "pid") != pid.to_string() || field(&text, "state") != "running" {
        return;
    }
    let mut out = format!("state={state}\n");
    for line in text.lines().filter(|l| !l.starts_with("state=")) {
        out.push_str(line);
        out.push('\n');
    }
    let value = value.replace(['\r', '\n'], " ");
    out.push_str(&format!("{key}={value}\nended={}\n", utc_timestamp(SystemTime::now())));
    let _ = std::fs::write(path, out);
}

/// This process ends cleanly, for `reason`.
pub fn end_session(dir: &Path, reason: &str) {
    finish(dir, std::process::id(), "ended", "reason", reason);
}

/// The supervisor saw child `pid` die with this description.
pub fn mark_died(dir: &Path, pid: u32, exit: &str) {
    finish(dir, pid, "died", "exit", exit);
}

// ---- exit statuses ---------------------------------------------------------------------------------------------------

/// How a child's exit status reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitDescription {
    /// A normal end: 0 (window closed, File > Exit, agent quit) or 143 (the signal watcher, after stopping).
    pub clean: bool,
    pub text: String,
}

/// Names an exit status. `code` is the process exit code (on Windows the NTSTATUS of a crash); `signal` the unix
/// signal that ended it.
pub fn describe_exit(code: Option<i32>, signal: Option<i32>) -> ExitDescription {
    let dirty = |text: String| ExitDescription { clean: false, text };
    let Some(code) = code else {
        return match signal {
            Some(9) => dirty("killed by signal 9 (SIGKILL: kill -9, or the system out of memory)".to_owned()),
            Some(6) => dirty("aborted (signal 6, SIGABRT: a Rust abort, e.g. out of memory or a panic during a panic)".to_owned()),
            Some(11) => dirty("crashed (signal 11, SIGSEGV: a native crash, e.g. in the graphics driver)".to_owned()),
            Some(s) => dirty(format!("ended by signal {s}")),
            None => dirty("ended with no exit status".to_owned()),
        };
    };
    let hex = code.cast_unsigned();
    let crash = |what: &str| dirty(format!("exit code 0x{hex:08X}: {what}"));
    match hex {
        0 => ExitDescription { clean: true, text: "exit code 0: a normal close".to_owned() },
        143 => ExitDescription {
            clean: true,
            text: "exit code 143: the signal watcher stopped the machines and exited (Ctrl-C, console closed, logoff)".to_owned(),
        },
        1 => dirty("exit code 1: a start failure (see the records above), or killed by taskkill /F or Task Manager".to_owned()),
        101 => dirty("exit code 101: a Rust panic ended the program (its text is above)".to_owned()),
        0xC000_0005 => crash("ACCESS VIOLATION, a native crash (often inside the graphics driver; Windows Event Log 'Application Error' names the DLL)"),
        0xC000_0409 => {
            crash("FAIL-FAST: a Rust abort (out of memory: 'memory allocation of N bytes failed' above; a panic during a panic) or a native stack-buffer check")
        }
        0xC000_00FD => crash("STACK OVERFLOW"),
        0xC000_0017 | 0xC000_012D => crash("OUT OF MEMORY (no memory or no page file left)"),
        0xC000_001D => crash("ILLEGAL INSTRUCTION (a CPU feature this PC lacks)"),
        0xC000_0374 => crash("HEAP CORRUPTION (native code)"),
        0xC000_0135 | 0xC000_007B => crash("a DLL is missing or is for the wrong CPU"),
        0x8000_0003 => crash("BREAKPOINT (a native assertion)"),
        0x4001_0004 | 0xC000_013A => crash("ended by Ctrl-C/close outside the signal watcher, or by logoff/shutdown"),
        _ if hex >= 0xC000_0000 => crash("a native crash (NTSTATUS)"),
        _ => dirty(format!("exit code {code}")),
    }
}

// ---- the supervisor ----------------------------------------------------------------------------------------------------

/// Whether this process should supervise a child instead of opening the window: Windows by default (no console, so
/// nothing else would record how the GUI ended), elsewhere only with `OPENLASER_SUPERVISOR=1`. Never in the child.
pub fn supervisor_wanted(supervised: Option<&std::ffi::OsStr>, setting: Option<&std::ffi::OsStr>) -> bool {
    if supervised.is_some() {
        return false;
    }
    match setting.and_then(|s| s.to_str()) {
        Some("0") => false,
        Some("1") => true,
        _ => cfg!(windows),
    }
}

/// What the supervisor saw.
#[derive(Debug)]
pub struct Supervised {
    pub pid: u32,
    pub exit: ExitDescription,
    /// The exit code to pass on (1 for a crash or a signal).
    pub code: i32,
}

/// Starts `child` with its stderr appended to the log, waits for it, and records how it ended (log and marker).
pub fn supervise(mut child: std::process::Command, dir: &Path) -> std::io::Result<Supervised> {
    let stderr = OpenOptions::new().create(true).append(true).open(dir.join(LOG_FILE))?;
    child.env(SUPERVISED_VAR, "1").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(stderr);
    let mut process = child.spawn()?;
    let pid = process.id();
    log::info!(target: "openlaser::supervisor", "started the window process, pid {pid}");
    let status = process.wait()?;
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&status);
    #[cfg(not(unix))]
    let signal = None;
    let exit = describe_exit(status.code(), signal);
    if exit.clean {
        log::info!(target: "openlaser::supervisor", "the window process (pid {pid}) ended: {}", exit.text);
    } else {
        log::error!(target: "openlaser::supervisor", "THE WINDOW PROCESS (pid {pid}) DIED: {}", exit.text);
        mark_died(dir, pid, &exit.text);
    }
    let code = if exit.clean { status.code().unwrap_or(0) } else { 1 };
    Ok(Supervised { pid, exit, code })
}

// ---- the heartbeat -----------------------------------------------------------------------------------------------------

/// Starts a thread that records this process's memory every [`HEARTBEAT`]; on a 32-bit build the address space is
/// 2 GB, and the graphics driver's memory counts too. The gaps between beats also date a silent death.
pub fn start_heartbeat() {
    let started = Instant::now();
    let _ = std::thread::Builder::new().name("openlaser-heartbeat".into()).spawn(move || {
        loop {
            std::thread::sleep(HEARTBEAT);
            let memory = memory_report().unwrap_or_else(|| "memory: not available".to_owned());
            log::info!(target: "openlaser::heartbeat", "up {} s; {memory}", started.elapsed().as_secs());
        }
    });
}

/// This process's virtual size, private bytes, working set, handles and threads (Windows: `wmic`; unix: `ps`).
pub fn memory_report() -> Option<String> {
    let pid = std::process::id().to_string();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("wmic")
            .args(["process", "where", &format!("ProcessId={pid}"), "get", "VirtualSize,PrivatePageCount,WorkingSetSize,HandleCount,ThreadCount", "/value"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let mb = |key: &str| field(&text.replace('\r', ""), key).trim().parse::<u64>().map(|b| format!("{} MB", b >> 20)).unwrap_or_else(|_| "?".to_owned());
        let n = |key: &str| field(&text.replace('\r', ""), key).trim().to_owned();
        Some(format!(
            "memory: virtual {}, private {}, working set {}, handles {}, threads {}",
            mb("VirtualSize"),
            mb("PrivatePageCount"),
            mb("WorkingSetSize"),
            n("HandleCount"),
            n("ThreadCount")
        ))
    }
    #[cfg(not(windows))]
    {
        let out = std::process::Command::new("ps").args(["-o", "vsz=,rss=", "-p", &pid]).stdin(std::process::Stdio::null()).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let mut parts = text.split_whitespace().map(|v| v.parse::<u64>().ok());
        let (vsz, rss) = (parts.next().flatten()?, parts.next().flatten()?);
        Some(format!("memory: virtual {} MB, resident {} MB", vsz >> 10, rss >> 10))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_utc_iso() {
        assert_eq!(utc_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        // 2026-11-03T14:05:09.123Z
        let t = UNIX_EPOCH + Duration::from_millis(1_793_714_709_123);
        assert_eq!(utc_timestamp(t), "2026-11-03T14:05:09.123Z");
        // A leap day.
        assert_eq!(utc_timestamp(UNIX_EPOCH + Duration::from_secs(951_782_400)), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn the_filter_keeps_openlaser_info_and_only_warnings_of_others() {
        assert!(wanted(log::Level::Info, "openlaser_engine::session"));
        assert!(wanted(log::Level::Info, EXIT_TARGET));
        assert!(!wanted(log::Level::Debug, "openlaser_engine"));
        assert!(!wanted(log::Level::Info, "wgpu_core::device"));
        assert!(wanted(log::Level::Warn, "wgpu_hal::gles"));
        assert!(wanted(log::Level::Error, "eframe"));
    }

    #[test]
    fn records_are_appended_with_time_level_and_repeats_folded() {
        let dir = tempfile::tempdir().unwrap();
        let logger = FileLogger::open(dir.path(), true).unwrap();
        logger.write_record(log::Level::Info, "openlaser::ops", "laser.frame ok");
        for _ in 0..5 {
            logger.write_record(log::Level::Warn, "wgpu_hal", "same warning");
        }
        logger.write_record(log::Level::Error, "openlaser::panic", "boom");
        let text = std::fs::read_to_string(dir.path().join(LOG_FILE)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "{text}");
        assert!(lines[0].contains("INFO ") && lines[0].ends_with("openlaser::ops: laser.frame ok"), "{text}");
        assert!(lines[1].ends_with("WARN wgpu_hal: same warning"), "{text}");
        assert!(lines[2].contains("repeated 4 more times"), "{text}");
        assert!(lines[3].ends_with("ERROR openlaser::panic: boom"), "{text}");
        assert!(lines[0].starts_with("20") && lines[0].as_bytes()[23] == b'Z', "{text}");
    }

    #[test]
    fn an_oversized_log_is_rotated_only_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let big = vec![b'x'; usize::try_from(ROTATE_BYTES).unwrap() + 1];
        std::fs::write(dir.path().join(LOG_FILE), &big).unwrap();
        drop(FileLogger::open(dir.path(), false).unwrap());
        assert!(!dir.path().join(OLD_LOG_FILE).exists());
        drop(FileLogger::open(dir.path(), true).unwrap());
        assert_eq!(std::fs::metadata(dir.path().join(OLD_LOG_FILE)).unwrap().len(), ROTATE_BYTES + 1);
        assert_eq!(std::fs::metadata(dir.path().join(LOG_FILE)).unwrap().len(), 0);
    }

    #[test]
    fn exit_records_become_the_exit_reason() {
        let dir = tempfile::tempdir().unwrap();
        let logger = FileLogger::open(dir.path(), false).unwrap();
        assert_eq!(logger.exit_reason(), None);
        log::Log::log(&logger, &log::Record::builder().level(log::Level::Info).target(EXIT_TARGET).args(format_args!("File > Exit")).build());
        log::Log::log(&logger, &log::Record::builder().level(log::Level::Info).target("openlaser::ops").args(format_args!("other")).build());
        assert_eq!(logger.exit_reason().as_deref(), Some("File > Exit"));
        for text in ["a", "b", "c"] {
            log::Log::log(&logger, &log::Record::builder().level(log::Level::Info).target(EXIT_TARGET).args(format_args!("{text}")).build());
        }
        assert_eq!(logger.exit_reason().as_deref(), Some("a → b → c"));
    }

    #[test]
    fn the_marker_tells_a_clean_end_from_an_unclean_one_and_a_death() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(begin_session(dir.path()), PreviousRun::Clean);
        // This run never wrote its end: the next start sees it.
        let previous = begin_session(dir.path());
        let PreviousRun::Unclean { pid, version, .. } = &previous else { panic!("{previous:?}") };
        assert_eq!((pid.as_str(), version.as_str()), (std::process::id().to_string().as_str(), VERSION));
        assert!(describe_previous(&previous).unwrap().contains("DID NOT END CLEANLY"));
        end_session(dir.path(), "window closed");
        assert_eq!(begin_session(dir.path()), PreviousRun::Clean);
        let text = std::fs::read_to_string(dir.path().join(SESSION_FILE)).unwrap();
        assert!(text.starts_with("state=running\n"), "{text}");
        mark_died(dir.path(), std::process::id(), "exit code 0xC0000005: ACCESS VIOLATION");
        let previous = begin_session(dir.path());
        assert!(matches!(&previous, PreviousRun::Died { exit, .. } if exit.contains("ACCESS VIOLATION")), "{previous:?}");
        // Another pid's marker is never overwritten.
        mark_died(dir.path(), std::process::id().wrapping_add(1), "x");
        assert!(matches!(previous_run(dir.path()), PreviousRun::Unclean { .. }));
    }

    #[test]
    fn exit_statuses_are_named() {
        assert!(describe_exit(Some(0), None).clean);
        assert!(describe_exit(Some(143), None).clean);
        let av = describe_exit(Some(0xC000_0005_u32.cast_signed()), None);
        assert!(!av.clean && av.text.contains("0xC0000005") && av.text.contains("ACCESS VIOLATION"), "{av:?}");
        assert!(describe_exit(Some(0xC000_0409_u32.cast_signed()), None).text.contains("out of memory"));
        assert!(describe_exit(Some(0xC000_00FD_u32.cast_signed()), None).text.contains("STACK OVERFLOW"));
        assert!(describe_exit(Some(101), None).text.contains("panic"));
        assert!(describe_exit(Some(1), None).text.contains("taskkill"));
        assert!(describe_exit(None, Some(9)).text.contains("SIGKILL"));
        assert!(!describe_exit(None, Some(6)).clean);
    }

    #[test]
    fn the_supervisor_runs_on_windows_by_default_never_in_its_child() {
        use std::ffi::OsStr;
        assert_eq!(supervisor_wanted(None, None), cfg!(windows));
        assert!(supervisor_wanted(None, Some(OsStr::new("1"))));
        assert!(!supervisor_wanted(None, Some(OsStr::new("0"))));
        assert!(!supervisor_wanted(Some(OsStr::new("1")), Some(OsStr::new("1"))));
    }

    fn shell(script: &str) -> std::process::Command {
        #[cfg(windows)]
        {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", script]);
            c
        }
        #[cfg(not(windows))]
        {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", script]);
            c
        }
    }

    #[test]
    fn the_supervisor_records_the_childs_stderr_and_a_death() {
        let dir = tempfile::tempdir().unwrap();
        // The child "dies" with 101 after Rust-like last words on stderr; its marker says running.
        std::fs::write(dir.path().join(SESSION_FILE), "state=running\npid=0\nversion=x\nstarted=y\n").unwrap();
        let seen = supervise(shell("echo memory allocation of 4096 bytes failed 1>&2 && exit 101"), dir.path()).unwrap();
        assert!(!seen.exit.clean && seen.code == 1, "{seen:?}");
        let log = std::fs::read_to_string(dir.path().join(LOG_FILE)).unwrap();
        assert!(log.contains("memory allocation of 4096 bytes failed"), "{log}");
        let clean = supervise(shell("exit 0"), dir.path()).unwrap();
        assert!(clean.exit.clean && clean.code == 0, "{clean:?}");
    }

    #[test]
    fn memory_is_reported_on_this_os() {
        let report = memory_report();
        if let Some(text) = report {
            assert!(text.starts_with("memory: virtual "), "{text}");
        }
    }
}
