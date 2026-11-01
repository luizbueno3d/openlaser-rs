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

# SPEC changes from WP3, WP4, WP5, WP6, WP9 and integration step I1

Contract changes visible on `integration` at I1 (from `git log main..integration` and a diff of every public item
against WP0's stubs), one line each: what, then why. Items a package added beyond §3 that nothing else names are
summarised per crate. Fold into SPEC revision 3.

## WP3: `plan`, `safety`, `arm`, `types::arm`

- `PlanFacts::far_objects: Vec<(ObjectId, LayerKey, f64)>` (§3.7): a shape with a point beyond `MAX_COORD_MM` (100 m) is planned not at all and refused by name, never replaced by stand-in geometry (red-zone review blocker).
- `plan` exports its limits (`MIN_FLATTEN_TOL_MM`, `MAX_FLATTEN_TOL_MM`, `MAX_COORD_MM`, `MAX_DESIGN_POINTS`, `MAX_PASSES_PER_LAYER`, `MAX_SECTIONS`, `MAX_SECTION_POINTS`, `MAX_HATCH_LINES`, `MAX_SUB_LAYERS`): forged documents are bounded, and the UI and tests name the same numbers.
- `plan::quantise` (`quantise_galvo`, `quantise_gantry`, `resolve_timings`, `s_value`, `pulse_request`, `pulse_word`, `pulse_not_set`, `speed_field_mm`, `galvo_unencodable`, `gantry_unencodable`, `QuantiseError`): quantisation happens once, here; `safety` re-derives every word with the same functions and refuses a plan whose words do not match its settings (`Stale`).
- `plan::{fingerprint_of, device_of, frame_style_of, bounds_rect, HULL_FRAME_UNSUPPORTED, gantry_bed, gantry_transform}` and re-exports `Bounds`, `WorkspaceOrigin`: `safety` re-derives the fingerprint (S21) and the frame style a `FramePlan` reports (`validate_frame` reads the style back from the paths: WP3 asked for a `JobPlan.frame_style` field instead; open).
- `safety::{FIELD_EDGE_TOLERANCE_MM (1e-6 mm), MAX_REPEATS (999), SPEED_NOTICE_REL (1 %)}`; `validate` also refuses a plan whose fingerprint or words do not re-derive (`Stale`) and a frame that does not cover the burn (`FrameDoesNotCoverJob`, defence in depth for B6).
- `arm::{ARM_TEXT, TTY_NONCE_RANGE (100000..=999999), CONFIRM_MAX_AGE (10 s)}`; `mint` refuses an all-zero fingerprint or bytes hash, and a confirmation stamped before the frame receipt or older than 10 s; `ArmToken::check` refuses all-zero hashes on either side (S22 binding can never be disabled by a stub hasher).
- `types::ArmToken::{expires, session}` accessors (the driver and engine compare them).

## WP4: `lmc`, `lmc-sim`, `types::galvo` (review note N-g)

- `lmc::Error` gained `ZeroSpeed`, `ShortPath`, `UnsafeLighting`, `InvalidPulse`, `InvalidPort(u8)`, `TooManyCommands`, `SummaryMismatch`, `Packet(&'static str)`: every encoder and summary refusal is named.
- `types::galvo::PulseDecision::word()`; `frequency_to_period` refuses a period of 0.
- `Packet::{from_words, try_from_bytes, sha256_hex}`; `LightingList` and `MarkingList` exported (the type-level split of §5.2 rule 9); `op` holds the whole GP opcode table; `py_compat::frame_once_repeats`.
- `lmc_sim::Recording` gained `error`, `mo_open_since`, `mo`, `port`; `FakeLmc::set_field_mm` is public; `FakeGrbl::{buffer_usage, position, laser_on, set_speedup, inject_alarm, inject_error_next}`.
- The T0 test lives in `crates/lmc/tests/t0_numeric.rs` (not `crates/types/tests/`): the `_om` variants it drives are WP4's.

## WP5: `driver`, `transport`

- `DriverError::{refused(code, message), is_refusal()}`: refusals and faults are told apart (a refusal never faults the machine).
- Constants `DEAD_MAN_JOIN` (500 ms), `MAX_SLICE` (20 ms), `TRACE_LINES_PER_OPERATION` (100 000), `DEFAULT_RX_SIZE` (128).
- `LmcBackend::with_provider_name`, `GrblBackend::{with_provider_name, rx_size, settings, version}`: `DeviceInfo.provider` and the GRBL preflight are observable.
- `StopLatch::{with_clock, signal_raised, tripped_at, clear_if_generation}`, `Stopper::latch`, `StopReport::unconfirmed`, `Cx::{pause, stop_requested_at, trace, flush_dropped_traces}`: the stop is timed from the trip, and a trip racing a dequeue is never lost.
- `DriverHandle::{stop, connect, disconnect}` and `AnyDriver::{stopper, events, capabilities, stop, connect, disconnect}`.
- Each abort transfer waits ≤ 50 ms for its reply (a wedged board still gets MO(0) and WritePort(0) inside the 500 ms stop watchdog); a malformed `LmcJob` is refused (`NotEncodable`), not a fault.
- `transport`: `APP_DIR`, `data_dir`, `hardware_disabled(_by)`, `hardware_refusal`, `bench::{BENCH_FILE, BENCH_MAX_AGE_S, FIRST_EMISSION_CAP_PCT, BenchSession, BenchStore}`, `BoardLock::{acquire_in, path}`, `LMC_VID`/`LMC_PID`/`CLONE_PID`, `UsbLmc::open`, `SerialLink::open`, `TcpLink::connect`: the bench session and the lock are testable on a temporary folder.

## WP6: `engine`

- The device hub on `Session`: `connect`, `disconnect`, `start_frame`, `request_arm`, `pending_arm`, `cancel_arm`, `run_armed`, `stop_all`, `stop_machine`, `stop_registry`, `status`, `poll`, `acknowledge_fault`, `token_expiry`; `PendingArm` (summary, receipt, checklist, decoded burn, revision, expiry, the encoded `job`); `CachedStatus`; `Event`; `DeviceAccess` moved to `devices.rs`, `FsAuthority`/`FileAccess` to `files.rs` (re-exported).
- Job pipeline types `JobOptions`, `Encoded`, `BuiltJob`, `JobRefused`, `LiveCheck`, and `Session::{build_job, live_check, live_error}`.
- `StopRegistry::{unregister, stop_one, trips, machines}`: `run_armed` refuses (`StoppedAfterQueue`) when any stop came through since Start (fast path, signal watcher, panic hook).
- Faults and unconfirmed stops are sticky until `laser.clearFault`; frame, Start and `run_armed` refuse meanwhile.
- Requests (§4.4): `submit_job`, `request_frame`, `approve_request`, `decline_request`, `withdraw_request`, `requests()`; approving frames or arms only the fingerprint the agent asked about.
- `params` (`decode` moved there from `command.rs`, `IdsParams`, `LayerKeySchema`, `NoParams`, `MAX_PARAMS_DEPTH`, `UNIVERSAL_KEYS`, `normalise_id`, `snake_to_camel`), `prefs` (`FORBIDDEN_PREFIXES`), and the limits `STOP_WAIT`, `PENDING_ARM_TTL`, `SUBMISSION_TTL`, `FRAME_REQUEST_TTL`, `MAX_NOTE_CHARS`, `MAX_BATCH_STEPS`, `MAX_JOURNAL`, `MAX_EVENTS`, `MAX_READ_BYTES`, `INSPECT_OBJECTS`, `GENERATOR`.

## WP9: `human`, `apps/openlaser-hw`

- `human`: `close_arm_dialog`, `surface_shown`, `injected_events`, `dropped_events`; `ApprovalCard`/`ApprovalAction`; `format` module (plain, bounded text for every outside string); constants `MIN_VISIBLE` (300 ms), `TAINT_PASSES`, `TAINT_HOLD`, the dialog labels.
- `openlaser-hw` (§3.21): `Terminal::confirm_arm(challenge, now: &dyn Fn() -> MonoTime)` (was `at: MonoTime`): the confirmation is stamped after the line is read, so `mint`'s 10 s age limit measures the typing, not the reading; `Terminal::keys` (raw-mode stop keys); `run_with(args, term, env)` with an injected `Env` (profile folder, clock, rig factory); exit codes `EXIT_OK`/`EXIT_FAIL`/`EXIT_USAGE`; `HwError`.

## I1 (integrator)

- `lmc::abort_packet()`: the abort's dummy end-of-list packet, byte-exact with the v0 abort golden; the driver's session now always has it (§5.3 "dummy EOL packet").
- `lmc::Error::SubUnitSection { layer, pass }` (WP4 N-f): an all-sub-unit layer-pass is refused by name instead of "empty job"; the engine maps it to `NothingToMark` on that layer.
- `emission_summary` also re-encodes each section and refuses a list whose bytes differ (WP4 N-c), and its work-area check admits exactly the encoder's words (N-e, no galvo unit of slack).
- `plan`: a curve tolerance that is not a number in `MIN_FLATTEN_TOL_MM..=MAX_FLATTEN_TOL_MM` is an `unsupported` fact (refused), never replaced or raised; the MO settle word is `ceil(µs / 10)` (was `trunc`).
- `safety`: refuses negative jump and polygon delays (`NotEncodable`), and a mark speed or jump speed above the 65535-unit speed word (`SpeedAboveMax` / `NotEncodable`).
- `lmc-sim::FakeLmc`: polls return when the transfer completes (`transfer_latency`), never later than the slice; a packet written while not READY is recorded on the wire before it is refused.
- Gantry plans (§3.7): `Section.paths` and frame paths are centre-origin bed mm, Y up, whatever the controller's origin (the convention `gcode` already took); `gantry_transform` no longer maps to the controller, `gantry_bed` is the centred bed, new `plan::gantry_home` (the controller's origin in plan coordinates); the engine's gantry `EmissionSummary.work_area` and decoded burns use the same frame.
- `driver` dev-depends on `openlaser-plan` (`rust_v1_session` builds the shipping `mark_box10` plan); `Cargo.lock` gained only that internal edge.
- `xtask deps` rule 13 matches `Origin::Human` as a path of its own, no longer inside `FrameOrigin::Human`.

## I2 (integrator)

- `geom` gains `boolean` (i_overlay 9.1.0, default-features off — integer/single-threaded): `boolean`, `union_all`, `offset`, `stroke_outline` on closed/open polylines; operands are EvenOdd-normalised, combined NonZero; `geom::Error::TooFewPoints` is new.
- `engine`: `tools.weld`, `tools.booleanUnion`/`booleanSubtract`/`booleanIntersect`, `tools.offsetShapes`, `tools.polygon`, `tools.measure`, `arrange.group`/`ungroup`, `edit.copy`/`cut`/`paste`/`pasteInPlace`; the Session holds a per-session clipboard.

- `doc`: `Project::{alloc_id, world_polylines, bounds}` return `Result` (`IdsExhausted`, `BadTolerance`, `NonFinite`, `TooManyPoints`); `geom::flatten` returns `Result` and `flatten_with_budget`/`check_tolerance`/`PointBudget` bound it; `format::MAX_MAGNITUDE` = 1e9; save validates like load (`normalize()` first); load refuses a layer order that differs from the used layers; `CutTable::check_passes` and `Error::ZeroPasses` refuse `passes = 0` on load and save; `Project::world_transform(id)` resolves the composed ancestor transform of one shape; `Shape::xform` is parent-space.
- World queries are ancestor-aware: `world_polylines`/`bounds`/`world_transform` compose every ancestor group's transform (outermost first). Callers holding a bare shape must compose `world_transform` (or walk ancestors); `Shape::xform` alone is not world space.
- `plan`: `Error::Doc(#[from] openlaser_doc::Error)`; `plan()` calls `check_passes` before planning, so a zero-pass layer is refused even if a forged project bypassed load-time validation.
- `engine`: `geomx::{bounds, duplicate, assign_ids, path_stats}` and every caller propagate the new `Result`s (`geom_error`/`doc_error` map to `RefusalCode`s); `document.inspect` propagates rather than undercounts; `cut_settings_set` re-checks `check_passes` as table-level defense on top of the per-field `passes()` range.
- `ui-egui`: `OpenLaserApp::drop` runs the stop path while its session is still live — `terminate:` (Dock Quit, logout, Cmd-Q on an uncustomized menu) never delivers `CloseRequested`, so the close guard could not see it; `main` stops the registry once more after `run_native` returns. `human_surface_open` includes Device Settings while open; the dispatcher refuses requests carrying both `command` and `id`.
- `apps/openlaser`: `discovery::default_path()` is `Result` — enabling control refuses and the server shuts down rather than listen undiscoverably.

- I2: **text**. New `openlaser-text` crate: `FontLibrary` indexes system font dirs (+ `OPENLASER_FONT_DIRS`, bounded, hostile files skipped) and `layout()` shapes each line with harfrust and draws skrifa glyph outlines into a Y-up `BezPath` (cap-height sizing, per-line L/C/R alignment, named missing-glyph errors). `types::TextAlign`; `doc::Geom::Text` caches the text parameters plus the laid-out outline, so planning, rendering, booleans and files opened without the font share one geometry; `.olp` serialises both through the path codec. Engine: `tools.text`, `text.edit`, `text.fonts`, `edit.convertToPath`; the Session owns the FontLibrary (`set_font_dirs` injects the test fixture). UI: the Edit Text tool's inline canvas editor commits `tools.text`/`text.edit`, the text options bar (Font/Style/Height/spacing/Align) drives `text.edit` coalesced, and double-clicking a text reopens the editor.

- I2: **camera phase 6a**. New `openlaser-camera` crate (nokhwa `=0.10.11`, `default-features = false` + `input-native` + `camera-sync-impl`: AVFoundation/MSMF/v4l, no `decoding` so no C toolchain; `camera-sync-impl` makes `nokhwa::Camera` `Send` for the pump thread). `list_devices`, `Capture::open/start/stop/latest_frame` (latest-wins pump, RGBA `Frame` via `decode_image::<RgbAFormat>` — YUYV/NV12/GRAY/raw; an MJPEG-only camera refuses `Error::UnsupportedFormat`), `SyntheticSource` behind feature `synthetic`. `engine` gained the `camera` internal edge and owns the one `Capture` (`Session::camera_*` + `EngineError::Camera`, code `camera`): `camera.list`, `camera.status`, `camera.start {index?}`, `camera.stop` — all `Access::Agent` `Query`/`View` commands (view state, never laser state). `ui-egui`: `camera` joins `dock::PANELS`, `laserTools.cameras` is a shell row opening the Camera Control panel (device dropdown, Start/Stop, `TextureHandle` feed repainting ~33 ms while streaming, lazy enumeration). CI's Linux job installs `libclang-dev` (v4l2-sys-mit's bindgen; Linux only — no C toolchain on Windows).

- I2: **camera phase 6b** (guided placement). `camera` gained `serde` for `calibrate`: `Pair` (cam px ↔ bed mm), `fit` (exact affine at 3 pairs; Hartley-normalised DLT homography at ≥4 — smallest eigenvector of AᵀA by inverse iteration over a safe-Rust Gaussian solve; a spanning 4-subset with no 3 collinear is required on each side, so collinear triples inside a larger set are fine but all-on-a-line is refused), `Calibration::{bed_to_px, px_to_bed, rms_error_px, kind}` (the inverse is the analytic inverse of the fit), `StoredCalibration` and the `CalibrationStore` trait (ProfileStore-style). `format` gained the `camera` internal edge for `camera_cal::{calibration_path, DirCalibrationStore}` — `<data_dir>/camera-calibration.json`, atomic, corrupt files load as `None`; `openlaser` and `openlaser-hw` inject it into `SessionConfig.camera_store` (None keeps calibration session-local, e.g. automation). Engine: `camera.calibrate.addPair/removeLast/fit/clear` and `camera.map.bedToPx/pxToBed` (all `Access::Agent` view state; `camera.status` reports a `calibration` block; `map.*` is disabled until calibrated); `view.cameraBackground` is a `Ui` command the shell toggles (persisted `UiSettings.camera_background`, Window ▸ View Style row shows ✓). `ui-egui`: the Camera Control panel's Calibrate mode (click the feed → enter bed mm → Add; numbered pair dots, Undo/Clear/Fit, persisted RMS shown) and the canvas underlay (tessellated textured mesh through `px_to_bed`, ~60 % alpha under the grid/design, only while streaming+calibrated — no stale frame or map).

- M3 / H11: **foot switch.** `openlaser-hw ports [--watch]` reads `ReadPort` (0x0025) only and prints which input
  bit moves and its rest level. `machine::PortMap.foot_switch: Option<FootSwitchCfg>` (`ports.footSwitch` = word
  0..=2, bit 0..=15, `activeLow`, `fullPulse` default true, `debounceMs` 10..=1000 default 30). It is unset by
  default and not serialised while unset, and it is set only through `openlaser-hw profile set footSwitch.*`.
  - Driver: `Backend::{foot_switch, read_inputs}` (LMC: `ReadPort`, `Capabilities.inputs = true`). While connected,
    idle and unfaulted, and only when the profile sets a pedal, the driver thread reads the inputs every 15 ms.
    `PedalDetector` turns the reads into presses: released → pressed (→ released with full pulse), each level held
    for the debounce time. Any state change resets it. New events `DriverEvent::Inputs { words, pedal_down }` (on
    change) and `DriverEvent::FootSwitch { at }`.
  - Engine: `CachedStatus::{inputs, pedal_down, pedal_blocked}`. A stop or a fault blocks the pedal until a human
    `laser.frame` or `laser.start`. The engine emits the gated `Event::FootSwitch(PedalPress)` and never forwards
    the raw press. Only the engine builds a `PedalPress`; the `test-pedal` feature (test-only, rule 10) adds
    `engine::testing::pedal_press_for_tests`.
  - ARM: new `ConfirmKind::FootSwitch` with `FootSwitchWitness` and `HumanConfirm::from_foot_switch`, both
    disallowed-methods outside `crates/human/src/pedal.rs` (rule 11). `mint` ignores the text for it but requires
    every tick, and keeps all frame, age and bytes rules.
  - GUI: `HumanSurfaces::foot_switch` and `ArmModel.foot_switch`. A press is Start through the Start button's own
    gate (`start_gate_now`), and it arms at once only with a standing confirmation (≤ 2 s old). Inside an open
    dialog, with every item ticked, a press made after the dialog opened (≤ 30 s, design C3.8) replaces typing ARM.
    The Laser panel shows the pedal line.
