//! Thin process entry point; CLI behavior lives in `lib.rs` for in-process tests.
#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let code = openlaser_cli::run(std::env::args_os().skip(1), &mut std::io::stdout(), &mut std::io::stderr());
    ExitCode::from(code)
}
