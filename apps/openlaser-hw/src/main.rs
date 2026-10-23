//! `openlaser-hw`: the human-only hardware CLI. Thin main over the library (WP9).
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
                return ExitCode::from(2);
            }
        }
    }
    ExitCode::from(openlaser_hw::run(&args, &mut TtyTerminal))
}
