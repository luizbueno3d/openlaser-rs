//! `openlaser`: the GUI (SPEC §3.21; owned by WP8).
//!
//! WP0 stub: prints what is coming and exits. No device can be opened: the default build has no `hw` feature.
#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let hw = if cfg!(feature = "hw") { "with" } else { "without" };
    eprintln!("openlaser {}: WP0 scaffold ({hw} the hw feature). The GUI arrives with WP8.", env!("CARGO_PKG_VERSION"));
    ExitCode::from(1)
}
