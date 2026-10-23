//! `cargo xtask ci`: the whole gate (SPEC §6.1, §2.6 item 1), with `OPENLASER_NO_HW=1` and `OPENLASER_HW_PRESENT`
//! removed for every step (S23). Never `--all-features`, never the `hw` feature, never `--release`.
//!
//! Run it in the gui lane (`CARGO_TARGET_DIR=~/AI/OpenLaser/.target/gui`): it builds the whole workspace. The
//! Windows cross-check needs ≥ 8 GB free and the `x86_64-pc-windows-msvc` target; `--no-windows` skips it.

use std::path::Path;
use std::process::Command;

use crate::cargo;

const WINDOWS_TARGET: &str = "x86_64-pc-windows-msvc";
const MIN_RUSTC: (u32, u32) = (1, 99);

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let mut windows = !cfg!(windows);
    for a in args {
        match a.as_str() {
            "--no-windows" => windows = false,
            other => return Err(format!("unknown ci flag `{other}` (only --no-windows)")),
        }
    }

    check_rustc(root)?;
    step(root, "fmt", &["fmt", "--all", "--", "--check"])?;
    step(root, "clippy", &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])?;
    step(root, "test", &["test", "--workspace"])?;
    println!("==> deps");
    crate::deps::run(root)?;
    println!("==> golden-verify");
    crate::golden::run(root)?;
    if windows {
        if !windows_target_installed(root) {
            return Err(format!("the {WINDOWS_TARGET} target is not installed: rustup target add {WINDOWS_TARGET}"));
        }
        step(root, "clippy (Windows)", &["clippy", "--workspace", "--all-targets", "--target", WINDOWS_TARGET, "--", "-D", "warnings"])?;
    } else {
        println!("==> clippy (Windows): skipped");
    }
    println!("ci: all gates green");
    Ok(())
}

fn command(root: &Path, program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new(program);
    c.current_dir(root).env("OPENLASER_NO_HW", "1").env_remove("OPENLASER_HW_PRESENT");
    c
}

fn step(root: &Path, name: &str, args: &[&str]) -> Result<(), String> {
    println!("==> {name}: cargo {}", args.join(" "));
    let status = command(root, cargo()).args(args).status().map_err(|e| format!("{name}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{name} failed ({status})")) }
}

/// The gate needs rustc ≥ 1.99 (the `stable` toolchain, §2.5).
fn check_rustc(root: &Path) -> Result<(), String> {
    let out = command(root, "rustc").arg("--version").output().map_err(|e| format!("rustc --version: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let version = text.split_whitespace().nth(1).unwrap_or_default();
    let mut parts = version.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    println!("==> {}", text.trim());
    if (major, minor) >= MIN_RUSTC { Ok(()) } else { Err(format!("rustc {version} is older than {}.{}", MIN_RUSTC.0, MIN_RUSTC.1)) }
}

fn windows_target_installed(root: &Path) -> bool {
    match command(root, "rustup").args(["target", "list", "--installed"]).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim() == WINDOWS_TARGET),
        // No rustup (e.g. a distro toolchain): let cargo report a missing target itself.
        Err(_) => true,
    }
}
