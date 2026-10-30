//! `openlaser-hw` as a library (SPEC §3.21, §5.4, §8; owned by WP9, red zone): the human-only hardware CLI.
//!
//! The logic lives here behind two seams so tests run it in process (SPEC §6.1):
//! - [`Terminal`]: the operator's terminal. The real one is `tty_arm::TtyTerminal`, the only code that builds a TTY
//!   witness (after `IsTerminal` on stdin and stdout); the scripted one lives in `tests/` and gets its
//!   `HumanConfirm` from `openlaser_arm::testing`.
//! - [`rig::Rig`]: the machine session (engine `Session` + device provider). `rig::EngineRig` is the real one;
//!   tests may substitute a scripted rig through [`Env::rigs`].
//!
//! Safety shape (AGENTS.md §1, SPEC §1.3): nothing that moves the galvo, lights the red dot or writes a profile runs
//! unless stdin and stdout are TTYs (`NotATty`); real USB needs a build with feature `hw` (else `HardwareDisabled`;
//! `--sim` drives FakeLmc), and the transport additionally needs `OPENLASER_HW_PRESENT=1`, a bench session and the
//! board lock (S17, S23). `run` frames in a loop, accepts Enter only after a pass the board has executed (S20),
//! stops framing, stops there without `--allow-emission`, refuses rows above the bench session's first-emission cap,
//! prints the decoded summary and the checklist, and arms only on `ARM <6-digit challenge>` typed at the TTY. Esc,
//! Space and Ctrl-C (key thread) and SIGINT/SIGTERM/SIGHUP (signal-hook; ctrlc on Windows) trip every latch.
#![forbid(unsafe_code)]

pub mod args;
pub mod bench;
pub mod builtin;
pub mod flow;
pub mod plain;
pub mod profile;
pub mod rig;
pub mod stop;
pub mod summary;
pub mod trace;
pub mod tty_arm;
mod usb_trace;

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use openlaser_arm::HumanConfirm;
use openlaser_lmc_sim::SimConfig;
use openlaser_types::{Clock, MonoTime, Refusal, RefusalCode, SystemClock};

use crate::args::{Cmd, USAGE};
use crate::rig::RigFactory;

/// Process exit codes (SPEC §3.21): 0 ok, 1 failure or refusal, 2 usage.
pub const EXIT_OK: u8 = 0;
pub const EXIT_FAIL: u8 = 1;
pub const EXIT_USAGE: u8 = 2;

/// A key that stops the machine at once (SPEC §5.4: Esc, Space, Ctrl-C in raw mode). `InputLost` is the key
/// reader failing or the input closing: the machine is stopped too (fail safe).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopKey {
    Esc,
    Space,
    CtrlC,
    InputLost,
}

impl fmt::Display for StopKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            StopKey::Esc => "Esc",
            StopKey::Space => "Space",
            StopKey::CtrlC => "Ctrl-C",
            StopKey::InputLost => "input lost",
        })
    }
}

/// Called on the key thread for every stop key; it must only trip latches (never block, never do I/O).
pub type StopFn = Arc<dyn Fn(StopKey) + Send + Sync>;

/// What the main loop sees from the key reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPoll {
    Nothing,
    Enter,
    /// The key reader ended (EOF, error, script exhausted). The flow stops the machine.
    Closed,
}

/// Raw-mode key reading while a machine is active. Dropping it restores the terminal.
pub trait KeySession {
    /// Never blocks. Stop keys have already been handled by the time they would show here.
    fn poll(&mut self) -> KeyPoll;
}

/// The operator's terminal (real: `tty_arm::TtyTerminal`, crossterm raw mode; tests: scripted).
pub trait Terminal {
    /// stdin and stdout are both TTYs (else NotATty).
    fn is_tty(&self) -> bool;
    /// One line typed by the operator, without the newline; None at end of input.
    fn read_line(&mut self) -> Option<String>;
    fn write(&mut self, text: &str);
    /// Prints the 6-digit `challenge`, reads `ARM <challenge>` and returns the confirmation, or None (wrong text,
    /// end of input, not a TTY). The confirmation is stamped with `now()` read after the line was read, so it is as
    /// young as the typing (`mint` refuses one older than 10 s however long the operator took to read the summary).
    /// The real implementation is in `src/tty_arm.rs`.
    fn confirm_arm(&mut self, challenge: u64, now: &dyn Fn() -> MonoTime) -> Option<HumanConfirm>;
    /// Starts reading single keys while a machine frames or marks: Esc, Space and Ctrl-C call `stop` at once (on
    /// the key thread, never waiting for the main loop); Enter is reported by [`KeySession::poll`].
    fn keys(&mut self, stop: StopFn) -> Result<Box<dyn KeySession>, String>;
}

/// Why a command did not complete.
#[derive(Debug, Clone, PartialEq)]
pub enum HwError {
    /// Bad command line (exit 2).
    Usage(String),
    /// A safety or policy refusal the operator can act on.
    Refused(Refusal),
    /// The job does not pass the safety rules: every refusal.
    Refusals(Vec<Refusal>),
    /// Anything else (engine, driver, file).
    Failed(String),
}

impl HwError {
    pub fn exit_code(&self) -> u8 {
        match self {
            HwError::Usage(_) => EXIT_USAGE,
            _ => EXIT_FAIL,
        }
    }

    pub fn refused(code: RefusalCode, message: impl Into<String>) -> Self {
        HwError::Refused(Refusal::new(code, message))
    }

    /// The refusal code, when there is exactly one.
    pub fn code(&self) -> Option<RefusalCode> {
        match self {
            HwError::Refused(r) => Some(r.code),
            _ => None,
        }
    }
}

impl fmt::Display for HwError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HwError::Usage(m) => write!(f, "usage: {m}"),
            HwError::Refused(r) => write!(f, "refused ({}): {}", r.code.as_str(), r.message),
            HwError::Refusals(rs) => {
                write!(f, "refused: the job does not pass {} safety rule(s)", rs.len())?;
                for r in rs {
                    write!(f, "\n  {}: {}", r.code.as_str(), r.message)?;
                    if let Some(at) = r.at {
                        write!(f, " at ({:.3}, {:.3}) mm", at[0], at[1])?;
                    }
                }
                Ok(())
            }
            HwError::Failed(m) => write!(f, "failed: {m}"),
        }
    }
}

impl From<Refusal> for HwError {
    fn from(r: Refusal) -> Self {
        HwError::Refused(r)
    }
}

/// Everything platform-, device- or test-specific the commands need.
pub struct Env {
    /// Machine profile files (`format::machines::profiles_dir()`); the only place `profile` writes (S24).
    pub profiles_dir: Option<PathBuf>,
    pub clock: Arc<dyn Clock>,
    /// How long the main loop waits between engine polls.
    pub poll: Duration,
    /// Install the real signal handlers and the panic hook (the binary does; in-process tests do not).
    pub signals: bool,
    /// What the signal watcher calls after the StopReport (the binary: `std::process::exit`).
    pub exit: Arc<dyn Fn(u8) + Send + Sync>,
    /// Builds the machine session for `frame`, `run` and `status`.
    pub rigs: Box<dyn RigFactory>,
}

/// The default simulator board for `--sim`: an idle, READY board that queues 3 packets, 2 ms per transfer.
pub fn default_sim_config() -> SimConfig {
    SimConfig { buffer_depth: 3, transfer_latency: Duration::from_millis(2), status_extra_bits: 0, pid: 0x9899, faults: Vec::new(), emission_cap_pct: None }
}

impl Env {
    /// The binary's environment: per-user profile files, the system clock, real signals, the engine rig.
    pub fn real() -> Self {
        let profiles_dir = openlaser_format::machines::profiles_dir();
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let rigs = rig::EngineRigs::new(profiles_dir.clone(), default_sim_config(), Arc::clone(&clock));
        Env {
            profiles_dir,
            clock,
            poll: Duration::from_millis(20),
            signals: true,
            exit: Arc::new(|code| {
                // A signal can arrive while the key thread holds the terminal in raw mode.
                let _ = crossterm::terminal::disable_raw_mode();
                std::process::exit(i32::from(code))
            }),
            rigs: Box::new(rigs),
        }
    }
}

/// Runs one invocation; returns the process exit code (0 ok, 1 failure/refusal, 2 usage).
pub fn run(args: &[String], term: &mut dyn Terminal) -> u8 {
    let mut env = Env::real();
    run_with(args, term, &mut env)
}

/// [`run`] with an explicit environment (tests).
pub fn run_with(args: &[String], term: &mut dyn Terminal, env: &mut Env) -> u8 {
    // Everything written from here on reaches the terminal as plain text (see `plain`).
    let mut term = PlainTerminal(term);
    let cmd = match args::parse(args) {
        Ok(c) => c,
        Err(e) => {
            term.write(&format!("openlaser-hw: {e}\n\n{USAGE}"));
            return e.exit_code();
        }
    };
    match execute(cmd, &mut term, env) {
        Ok(()) => EXIT_OK,
        Err(e) => {
            term.write(&format!("openlaser-hw: {e}\n"));
            e.exit_code()
        }
    }
}

fn execute(cmd: Cmd, term: &mut dyn Terminal, env: &mut Env) -> Result<(), HwError> {
    match cmd {
        Cmd::Help => {
            term.write(USAGE);
            Ok(())
        }
        Cmd::Detect => rig::detect(term),
        Cmd::Status(o) => flow::status(&o, term, env),
        Cmd::Bench(b) => bench::run(b, term),
        Cmd::Profile(p) => profile::run(&p, term, env),
        Cmd::Frame(o) => flow::frame(&o, term, env),
        Cmd::Run(o) => flow::run(&o, term, env),
    }
}

/// The terminal every command writes to: [`plain::block`] drops control and invisible characters from all of it
/// (the CLI's own `\n` stays), so no string that came from a document, the engine or a file can move the cursor,
/// conceal text or reorder a line on the operator's terminal.
struct PlainTerminal<'a>(&'a mut dyn Terminal);

impl Terminal for PlainTerminal<'_> {
    fn is_tty(&self) -> bool {
        self.0.is_tty()
    }

    fn read_line(&mut self) -> Option<String> {
        self.0.read_line()
    }

    fn write(&mut self, text: &str) {
        self.0.write(&plain::block(text));
    }

    fn confirm_arm(&mut self, challenge: u64, now: &dyn Fn() -> MonoTime) -> Option<HumanConfirm> {
        self.0.confirm_arm(challenge, now)
    }

    fn keys(&mut self, stop: StopFn) -> Result<Box<dyn KeySession>, String> {
        self.0.keys(stop)
    }
}

/// Refuses unless stdin and stdout are TTYs (SPEC §3.21; checked before anything else the command does).
pub fn require_tty(term: &dyn Terminal, what: &str) -> Result<(), HwError> {
    if term.is_tty() {
        Ok(())
    } else {
        Err(HwError::refused(
            RefusalCode::NotATty,
            format!("`openlaser-hw {what}` runs only with stdin and stdout on a terminal, operated by a person at the machine"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder(String);

    impl Terminal for Recorder {
        fn is_tty(&self) -> bool {
            false
        }

        fn read_line(&mut self) -> Option<String> {
            None
        }

        fn write(&mut self, text: &str) {
            self.0.push_str(text);
        }

        fn confirm_arm(&mut self, _challenge: u64, _now: &dyn Fn() -> MonoTime) -> Option<HumanConfirm> {
            None
        }

        fn keys(&mut self, _stop: StopFn) -> Result<Box<dyn KeySession>, String> {
            Err("no keys".to_owned())
        }
    }

    #[test]
    fn everything_a_command_writes_is_plain() {
        let mut rec = Recorder::default();
        PlainTerminal(&mut rec).write("layer \u{1b}[8m\u{9b}2Jx\r\u{202E}y\n");
        assert_eq!(rec.0, "layer [8m2Jxy\n");
    }
}
