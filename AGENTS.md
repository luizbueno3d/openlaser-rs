# AGENTS.md: rules for AI agents and contributors

OpenLaser is an open-source, native laser design and machine-control app written in **Rust only**.
It aims for LightBurn-equivalent workflow, layout and file compatibility, built Craft-family style
(PhotoCraft, VectorCraft, PdfCraft): native egui/eframe, everything is a command, and every feature
is drivable by agents through `openlaser-cli` and its MCP server. No Tauri, Electron or webview shells.

Its first and most important machine is a **JPT M7 MOPA fiber laser on a JCZ/BJJCZ LMC galvo board**
(USB `0x9588:0x9899`). The build spec is [`docs/SPEC.md`](docs/SPEC.md). The research, design decisions
and history behind it live in [`docs/research/`](docs/research/README.md) — read `PLAN.md` and
`design/redteam-review.md` there before architectural work; they explain the safety posture, why the
LMC encoder must match golden bytes, and why prior art (Rayforge, balor, MeerK40t) is read-only.

## 1. Laser safety (outranks everything)

This software controls a Class 4 laser that can blind, burn and start fires.

1. **Agents never emit.** No AI agent, script, test or CI job may make the laser fire, and none may
   open the real laser USB device. Do not run any command that talks to real hardware. That includes:
   - `openlaser-hw`, or any binary built with `--features hw`;
   - anything built with the `hw` feature (agents never build it: Luiz builds `hw` binaries into his own
     target directory, `~/AI/OpenLaser/.target/hw`, never an agent lane);
   - setting `OPENLASER_HW_PRESENT` (hardware is default-deny, SPEC S23: every USB or serial open needs
     `OPENLASER_HW_PRESENT=1`, a `hw` build and a bench session; agents never set it, and always run with
     `OPENLASER_NO_HW=1`);
   - galvoplotter's own tests;
   - the Python prototype's live mode.

   Hardware sessions are run by a human at the machine.

   A Claude Code hook in this repo (`.claude/hooks/no_hw.py`, wired in `.claude/settings.json`) refuses any Bash
   command that builds with `hw`/`--all-features`, sets `OPENLASER_HW_PRESENT`, uses `.target/hw`, or runs
   `openlaser-hw` without `--sim`. Hardware sessions follow [`HARDWARE.md`](HARDWARE.md).
2. **Agents may operate everything else.** Design, import, layers, cut settings, planning, preview,
   estimate, G-code export, and submitting a job *for human approval* are all allowed. Arming a job
   requires a human confirmation in the GUI (or a physical input) that no agent, API, script or MCP
   call can forge. Never add a code path that arms remotely.
3. **Refuse, never warn and continue.** Do not silently clamp out-of-field points, truncate jobs, or
   proceed after a wait times out. Refuse with an error the user can act on.
4. **Bytes are the truth.** The LMC encoder must match the golden streams in `tests/golden/`
   byte for byte, except for the intentional divergences listed in `tests/golden/README.md` and
   `docs/DIVERGENCES.md`.
   The confirmation the human sees is decoded from the exact bytes that will be sent.
5. **The software stop is not the primary safety device.** The E-stop and key switch are.

## 2. Never crash

Non-test code never panics: no `unwrap()`, `expect()`, `panic!`, `todo!`, `unimplemented!` or
`unreachable!`, and no unchecked indexing on input-derived numbers. Return errors.
`unsafe_code = "forbid"` workspace-wide.

## 3. Golden rules

- **Everything is a command.** User-visible behaviour is an engine command (id, label, menu path,
  shortcut, params, `enabled`, `run`) with tests. The UI, CLI and MCP all dispatch by id.
- **Clean-room.** LightBurn was studied for behaviour, layout and file formats only. Never copy its
  code, icons, images or text. Third-party assets need a permissive licence and an `ATTRIBUTION.md`
  row. GPL sources (balor, LaserGRBL, K40 Whisperer, …) are read for behaviour only, never copied.
  galvoplotter and MeerK40t are MIT: ported code keeps their notice.
- **Tests are the gate.** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and
  `cargo test` must pass. Safety fixes get regression tests that fail without the fix.
- **Disk is tight** (~25 GB free). Use the shared lanes of §5 across worktrees; do not build
  release binaries unless asked.

## 4. Toolchain

Rust 1.99 (the `stable` toolchain, pinned by `rust-toolchain.toml`) lives in `~/.cargo/bin` (prepend it to `PATH`).
Edition 2024, workspace `rust-version` 1.95. Licence `MIT OR Apache-2.0`.

## 5. Build lanes and the gate (SPEC §6.1, §6.7)

Two shared target directories ("lanes"); never `cargo clean` in one, never `--release` unless asked, no
`cargo doc`. Check `df -h /System/Volumes/Data` before a big build: below 8 GB free, stop and report.

- **core lane** `~/AI/OpenLaser/.target/core`: WP1–WP7. It never builds the GUI stack: use `-p <crate>` for every
  member except `openlaser-ui-egui`, `openlaser-human` and `openlaser` (the app); never `--workspace` there.
- **gui lane** `~/AI/OpenLaser/.target/gui`: WP8, WP9 and the integrator (`--workspace` is fine here).
- Luiz's `~/AI/OpenLaser/.target/hw` lane is his alone; agents never build or run anything there.

The gate, before every hand-in (the integrator runs the workspace form, or `cargo xtask ci`, in the gui lane):

```sh
export PATH="$HOME/.cargo/bin:$PATH"
rustc --version                                             # must be >= 1.99
export CARGO_TARGET_DIR="$HOME/AI/OpenLaser/.target/core"   # or .target/gui
export OPENLASER_NO_HW=1
unset OPENLASER_HW_PRESENT
cargo fmt --all -- --check
cargo clippy -p <your crates> --all-targets -- -D warnings
cargo test -p <your crates>
cargo xtask deps                                            # SPEC §2.2 edges and §2.3 rules 1–17
cargo xtask golden-verify                                   # sha256 of every fixture in tests/golden/manifest.json
```

Never `--features hw`, never `--all-features` (it would enable `hw`), never `cargo update`. Work packages never
change `Cargo.lock` or add a dependency that is not in `[workspace.dependencies]` (ask the integrator).

**Binaries are tested in process**: through their `src/lib.rs` entry points with injected stdio and terminal. At most
one smoke test per binary spawns `CARGO_BIN_EXE_*`, because the uplifted `target/debug/<bin>` is shared by every
worktree in a lane and another worktree's build can replace it between build and spawn.

## 6. Windows is required (SPEC §2.6, S19)

The whole workspace builds and its tests pass on Windows 10/11 as well as macOS (Linux keeps compiling). No
`#[cfg(unix)]`-only features: paths come from `craft-settings` app dirs and `Path::join` (never `/` literals);
signals are `signal-hook` on unix and `ctrlc` on windows, both tripping the same latch; no crate may need a C
toolchain for the Windows target (no `ring`/`openssl`; blake3 `pure`, flate2 `rust_backend`); text exports use
`\n` on every OS and goldens are compared as bytes (`.gitattributes` keeps `tests/golden/**` binary-safe). The
integrator checks it from the Mac in the gui lane, once per integration step, with ≥ 8 GB free:
`cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings`. CI
(`.github/workflows/ci.yml`) runs fmt, clippy, tests and `xtask deps` on macOS, Windows and Linux.

## 7. Project history

The complete planning corpus — six research reports, three design proposals and their judging, the
red-team review, the Rayforge evaluation evidence and `PLAN.md` — is preserved in
[`docs/research/`](docs/research/README.md). It descends from `OpenMopa`, the Python prototype
(`~/AI/OpenLaser/OpenMopa`; its B1–B12 safety-bug fixes live on `fix/safety-b1-b12`). Where the corpus
and the code disagree, the code and `docs/SPEC-CHANGES.md` win.
