//! Repository tasks (SPEC §6.1, §7.1). Run with `cargo xtask <task>`.
//!
//! - `deps`: the dependency rules of SPEC §2.2/§2.3 from `cargo metadata`, plus source greps (rules 9, 11, 12).
//! - `golden-verify`: sha256 and size of every fixture listed in `tests/golden/manifest.json`.
//! - `parity`: the LightBurn menu parity report (`docs/parity-checklist.md`), once WP8 fills the catalogue.
//! - `ci`: the whole gate, including the Windows clippy check (§2.6).
//!
//! xtask never enables the `hw` feature or `--all-features`, and never touches a device.
#![forbid(unsafe_code)]

mod ci;
mod deps;
mod files;
mod golden;
mod parity;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <deps | golden-verify | parity | ci [--no-windows]>";

fn main() -> ExitCode {
    let mut args = Vec::new();
    for a in std::env::args_os().skip(1) {
        match a.into_string() {
            Ok(s) => args.push(s),
            Err(_) => {
                eprintln!("xtask: arguments must be valid Unicode\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let Some((task, rest)) = args.split_first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let root = repo_root();
    let result = match task.as_str() {
        "deps" => deps::run(&root),
        "golden-verify" => golden::run(&root),
        "parity" => parity::run(&root),
        "ci" => ci::run(&root, rest),
        _ => {
            eprintln!("xtask: unknown task `{task}`\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask {task}: FAILED\n{e}");
            ExitCode::FAILURE
        }
    }
}

/// `<repo>`: the parent of this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().map(PathBuf::from).unwrap_or(manifest)
}

/// The cargo that runs us (`cargo xtask` sets `CARGO`), else `cargo` from PATH.
pub fn cargo() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}
