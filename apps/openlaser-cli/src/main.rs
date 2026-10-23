//! `openlaser-cli`: the agent CLI and MCP server (SPEC §3.21; owned by WP7).
//!
//! WP0 stub: hardware subcommands are redirected (exit 2); everything else is not implemented yet (exit 1).
#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let first = std::env::args_os().nth(1);
    match first.as_ref().and_then(|a| a.to_str()) {
        Some("frame" | "mark" | "run-hw") => {
            eprintln!("openlaser-cli: hardware commands live in openlaser-hw and are run by a person at a terminal");
            ExitCode::from(2)
        }
        _ => {
            eprintln!("openlaser-cli {}: WP0 scaffold; subcommands arrive with WP7.", env!("CARGO_PKG_VERSION"));
            ExitCode::from(1)
        }
    }
}
