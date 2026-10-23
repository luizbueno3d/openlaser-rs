# AGENTS.md: rules for AI agents and contributors

OpenLaser is an open-source, native laser design and machine-control app written in **Rust only**.
It aims for LightBurn-equivalent workflow, layout and file compatibility, built Craft-family style
(PhotoCraft, VectorCraft, PdfCraft): native egui/eframe, everything is a command, and every feature
is drivable by agents through `openlaser-cli` and its MCP server. No Tauri, Electron or webview shells.

Its first and most important machine is a **JPT M7 MOPA fiber laser on a JCZ/BJJCZ LMC galvo board**
(USB `0x9588:0x9899`). The build spec is [`docs/SPEC.md`](docs/SPEC.md). The research and plan behind
it are in `~/AI/ClaudeCode/openlaser-rust-plan/` (`PLAN.md`, `research/05-hardware-protocols-and-test-inputs.md`).

## 1. Laser safety (outranks everything)

This software controls a Class 4 laser that can blind, burn and start fires.

1. **Agents never emit.** No AI agent, script, test or CI job may make the laser fire, and none may
   open the real laser USB device. Do not run any command that talks to real hardware. That includes:
   - `openlaser-cli` frame, mark or run against a real device;
   - anything built with the `hw` feature;
   - galvoplotter's own tests;
   - the Python prototype's live mode.

   Hardware sessions are run by a human at the machine.
2. **Agents may operate everything else.** Design, import, layers, cut settings, planning, preview,
   estimate, G-code export, and submitting a job *for human approval* are all allowed. Arming a job
   requires a human confirmation in the GUI (or a physical input) that no agent, API, script or MCP
   call can forge. Never add a code path that arms remotely.
3. **Refuse, never warn and continue.** Do not silently clamp out-of-field points, truncate jobs, or
   proceed after a wait times out. Refuse with an error the user can act on.
4. **Bytes are the truth.** The LMC encoder must match the golden streams in `tests/golden/`
   byte for byte, except for the intentional divergences listed in `tests/golden/README.md`.
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
- **Disk is tight** (~25 GB free). Share one `CARGO_TARGET_DIR` across worktrees; do not build
  release binaries unless asked.

## 4. Toolchain

Rust 1.99 lives in `~/.cargo/bin` (prepend it to `PATH`). Edition 2024. Licence `MIT OR Apache-2.0`.
