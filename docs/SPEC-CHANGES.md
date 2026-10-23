# SPEC changes made by WP0

Minimal fixes and fill-ins made while pasting the §3 contracts of `docs/SPEC.md` **revision 2** into the crates,
one line each: what, then why. Work packages code against the crates as they are; the integrator folds these into
SPEC revision 3.

## Contract fixes (would not compile, or leave a named item undefined)

- `types::UnitError::Malformed { what, text }` added: `LayerKey::from_str` and `Fingerprint::from_hex` must return `UnitError`, and no listed variant describes unparseable text.
- Every f64 unit newtype and `Hz` gained `MAX_VALID` and `const fn new_const(v) -> Option<Self>`: `machine::jpt_m7_200()` returns a plain value, so its literals are checked at compile time (`lit!`) with no `unwrap`/`panic!`.
- `lmc` keeps `sha2` as a normal dependency: `Packet`'s manual Debug shows a sha256 prefix (golden packet file names), which §2.2's lmc row (serde, blake3, thiserror) cannot provide.
- `lmc_sim::SimWire` defined (`Single { at, words }`, `Packet { at, sha256 }`, `Read { at, status }`): `Recording.wire` and `FakeLmc::set_hook` name it, §3.14 never defines it.
- `lmc_sim::WireHook` alias, and `FakeLmc::set_hook(&self, hook: WireHook)`: the same type as §3.14's `Box<dyn FnMut(&SimWire, MonoTime) + Send>`, spelled through an alias for clippy `type_complexity`.
- `driver::StopLatch::new(wake)` and `driver::Cx::new(latch, events, clock)` added: §3.13 gives neither a constructor; the runtime and backend tests need them.
- `driver::DriverHandle` keeps a private `Control<B>` enum; `spawn` starts no thread in the stub (requests are dropped, the stopper still latches).
- `engine::StopRegistry::register(machine, stopper)` and `StopRegistry: Default` added: the device hub needs a way to register each driver's stopper.
- `ui_egui::{DialogKind, DialogTicket(u64), UiSettings { in_window_menus }, UiState}` defined as placeholders: §3.19 names them without defining them (WP8 extends them).
- `ui_egui::control` is a directory (`control/mod.rs` + `control/input.rs` with `events_for(method, params) -> Result<Vec<egui::Event>, String>`): §2.3 rule 16 names `src/control/input.rs` as the only builder of synthetic events.
- `apps/openlaser-hw`: `Terminal { is_tty, read_line, write, confirm_arm(challenge: u64, at: MonoTime) -> Option<HumanConfirm> }`, `run(args, term) -> u8`, real terminal `tty_arm::TtyTerminal`: §3.21 leaves the trait's shape open (`confirm_arm(...)`).
- `gcode::om::{OmMachine, OmLayer, OmObject, OmRefusal { kind, message }}` fields copied from `tests/golden/t2_gcode/index.json`: §3.11 says "mirror the fields" without listing them (WP2 may refine; it owns `gcode`).
- `testkit::t0::Group { params, rows }` defined: §3.22 names `Group` without fields; the fixture keeps rows inside `groups`.
- `machine::Kinematics` carries `#[allow(clippy::large_enum_variant)]`: keeps the contract's unboxed variants under `-D warnings`.
- `Fingerprint`-returning stubs (`LmcJob::bytes_hash`, `EncodedJob::bytes_hash` for `LmcFrameJob`) return all zeros: §3.0 gives no neutral value for a hash; `ArmToken::check` refuses every token anyway.
- `GcodeProgram::bytes_hash` is implemented (blake3 of `text`), not stubbed: §3.11 defines it exactly and the driver's `EncodedJob` impls forward to it.
- `SimProvider::emission_cap_pct` returns `Ok(SimConfig.emission_cap_pct)` and `record_emission` `Ok(())`: §3.13 defines the source, and the simulator has no bench session to record.
- `ui_egui::NoHuman` fails closed like the stubs (`frame_is_synthetic` true, `surface_open` true, injected input dropped): §1.3 rule 7 covers test doubles; WP8 owns it and may relax it for its kittests.

## Build and dependency fixes

- eframe's features are declared once in `[workspace.dependencies]` (`wgpu`, `accesskit`, `persistence`, `wayland`, `x11`) instead of the app adding `persistence` (and the Linux backends winit needs): one feature set lets `-p openlaser-ui-egui` / `-p openlaser-human` reuse the gui lane's artifacts and build on Linux.
- `Cargo.lock` pins tokio at 1.53.1 (`cargo update -p tokio --precise 1.53.1`): §2.4's lock note; a fresh resolve picked 1.54.1.
- Acceptance "one kurbo": epaint 0.36.2 depends unconditionally on vello_cpu → peniko → kurbo 0.13.1, so the GUI closure holds two kurbos (PhotoCraft's lock too). The geometry stack (`geom`, `doc`, `import`/usvg, `render`) has exactly one, 0.11.3; check it with `cargo tree -d -e normal -p openlaser-engine`.
- `xtask deps` rule 10: a dev-dependency on a workspace crate is allowed when it is listed, `testkit`, acyclic, or one of two cycle exceptions used only from `tests/`: `driver → lmc-sim` (§6.3) and `types → arm` (`types/tests/arm_*.rs` need `arm::testing::token_for_tests`; `ArmToken::issue` is disallowed in tests). `automation`, `openlaser-cli` and `ui-egui` may not dev-depend on `arm` or `human` either.
- `xtask deps` rule 13 also lets `crates/engine` name `Origin::Human`: the engine defines the enum and matches it in the policy table.
- `xtask deps` rule 16 treats a line with `matches!(`, `if let`, `=>` or `..` as a pattern, not a construction.
- `xtask deps` checks the external crates of each workspace crate against §2.2 as well as the internal edges, so a work package cannot add a dependency without the integrator.

## Implemented beyond §3.0's list (trivial, spec-dictated; owners keep or replace them)

Unit validation, `Hz::from_khz`, `LayerKey` parse/display/serde, `Fingerprint` hex/serde/Debug, `DeviceKind`, `serde_secs`/`serde_ms`, `MonoTime` serde, the clocks, `RefusalCode::as_str`, `FrameReceipt::covers_job`, the `Polyline` helpers `len`/`is_empty`/`length_mm`/`bounds`/`transformed`/`all_finite` and `bounds_of`, `Project::alloc_id`/`shape`/`color_layer(_mut)`, `default_layer_name`, `Single::bytes`, `StatusWord`, `status_from_reply`, the `Box` forwarding impls, `StopLatch`/`Stopper`/`Cx` (stops must always latch), `StopRegistry::stop_all`, `History` queries, `Token::to_hex` and its redacted Debug.
