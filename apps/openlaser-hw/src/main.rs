//! `openlaser-hw`: the human-only hardware CLI. Thin main over the library (SPEC §3.21).
#![forbid(unsafe_code)]

use std::process::ExitCode;

use openlaser_hw::tty_arm::TtyTerminal;

fn main() -> ExitCode {
    // Non-Unicode arguments are usage errors (exit 2), never a panic.
    let mut args = Vec::new();
    for a in std::env::args_os().skip(1) {
        match a.into_string() {
            Ok(s) => args.push(s),
            Err(_) => {
                eprintln!("openlaser-hw: arguments must be valid Unicode");
                return ExitCode::from(openlaser_hw::EXIT_USAGE);
            }
        }
    }
    ExitCode::from(openlaser_hw::run(&args, &mut TtyTerminal::default()))
}
