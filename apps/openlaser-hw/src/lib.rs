//! `openlaser-hw` as a library (SPEC §3.21; owned by WP9, red zone): the logic lives here behind a [`Terminal`]
//! trait so tests run it with a scripted terminal (in `tests/`, getting its `HumanConfirm` from `arm::testing`);
//! the real terminal is in `src/tty_arm.rs`, the only file that builds a TtyWitness.
//!
//! WP0 stub: every subcommand is refused. Nothing here opens a device.
#![forbid(unsafe_code)]

pub mod tty_arm;

use openlaser_arm::HumanConfirm;
use openlaser_types::MonoTime;

/// The operator's terminal (real: `tty_arm::TtyTerminal`, crossterm raw mode; tests: scripted).
pub trait Terminal {
    /// stdin and stdout are both TTYs (else NotATty).
    fn is_tty(&self) -> bool;
    /// One line typed by the operator, without the newline; None at end of input.
    fn read_line(&mut self) -> Option<String>;
    fn write(&mut self, text: &str);
    /// Prints the 6-digit `challenge`, reads `ARM <challenge>` and returns the confirmation, or None (wrong text,
    /// end of input, not a TTY). The real implementation is in `src/tty_arm.rs`.
    fn confirm_arm(&mut self, challenge: u64, at: MonoTime) -> Option<HumanConfirm>;
}

/// Runs one invocation; returns the process exit code (0 ok, 1 failure/refusal, 2 usage).
pub fn run(args: &[String], term: &mut dyn Terminal) -> u8 {
    let sub = args.first().map(String::as_str).unwrap_or("");
    term.write(&format!("openlaser-hw: `{sub}` is not implemented yet (WP9)\n"));
    1
}
