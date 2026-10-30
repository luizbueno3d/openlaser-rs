# P3c rotary implementation report

Implemented on `phase3-rotary`, without switching branches, committing or pushing.
Hardware was disabled throughout (`OPENLASER_NO_HW=1`); no hardware feature was built and no real device was opened.
No dependencies or lockfile changes. `tests/golden/**` and `.claude/` are untouched. No files were deleted.

## Behaviour and decisions to review

- `GalvoMachine.rotary` defaults to `None`. All fourteen rotary fields are available in the human-only profile CLI and Device Settings, validated on load/save. Device Settings retains its synthetic-input and human-save guards; the engine's settings command remains `Access::Human`. No engine, agent CLI or MCP axis-move command was added.
- Split in machine space, using job minimum as the strip origin. Interior slabs are half-open for parallel boundary segments; the final outer edge is included. Crossing segments share their endpoint, without duplicating positive length outside overlap. Overlap extends each slab on both sides. A short final strip keeps the full slice's centre (not the remaining artwork's centre).
- Each target is rounded independently from its absolute strip centre, using Rust `round()` (half away from zero), then optionally negated. Chuck and roller pulse conversion both use drive diameter; object diameter is layout metadata.
- Sections retain their original layer/sub-layer/pass order within each strip. Planning refuses excess geometry work, section/point budgets, strips including overlap outside the centre-line field, and excessive rotation. The rotation limit counts physical pulse travel: initial positioning, transitions between strips/repeats, and the optional final return. Safety independently checks the total, target limits, profile enablement and monotonic order.
- Framing traces the union of clipped outlines translated to the centre line. Closed fill/image outlines gain strip-edge closing segments. Bounds style encloses that union. Frames do not turn the axis. Receipts continue to bind the complete fingerprint, including rotary steps and enabled rotary profile settings; non-rotary canonical bytes are unchanged.
- Misaligned Fill/Image scan angles are notices. Rotary images at nonzero angles re-hatch the same box-filtered/dithered dark cell bands in image space, without rotating the artwork. The pre-existing non-rotary image-angle refusal is unchanged.
- Axis singles precede every rotary list after the preceding list's !BUSY and MO/ports-off postlude. The final return executes once after all repeats. Singles and the return are included in the ARM byte hash and checked by summary decoding. Live motion refuses while MO, ports, a packet, a list or another axis move is active.
- Motion timeout is `abs(target-current)/min_speed * 1.5 + 2 s`, starting when MoveAxisTo is submitted. Poll spacing is 10 ms; transfers and waits check the stop latch in at most 20 ms slices. Status reads share the remaining motion deadline. A timeout faults into the unchanged emergency command sequence.
- **Coordinate-origin restriction / disagreement to review:** the requested “current position = 0” is not a known controller zeroing command. MoveAxisTo is absolute, and silently rebasing after ARM would change the approved bytes. Jobs therefore require GetAxisPos to report controller zero before the first strip and refuse otherwise. Human bench moves are relative to the reported controller position, with exact absolute targets on the wire. Supporting an arbitrary current physical origin would need an explicit pre-ARM origin capture/re-encode design; this implementation does not invent an unsigned zeroing opcode.
- **Residual risk:** no stop-axis opcode is known. A StopReport can confirm MO closed, ports zero and !BUSY while the motor still finishes its move; the report states that possibility when motion was in progress. The stop path adds no commands.

## New regression coverage

Twenty new tests were added. All twenty passed in the final full workspace run.

- Machine: legacy and partial-profile defaults; every invalid rotary range, including disabled configurations.
- Plan: exact 30 mm / 10 mm strips; overlap length; diagonal crossings; half-open boundary ownership; chuck and roller targets; reverse; 1,000 absolute-rounded targets; strip/layer order; X wrapping; excess travel/width; centred closed frames; fingerprint mutations; Fill and Image angle notices and image direction.
- Safety: forged rotary on a disabled machine; target bounds; nonmonotonic targets; total travel with individually valid targets; field bounds; invalid configuration.
- LMC: exact bytes at zero, ±1 and ±0x7fffffff; unrepresentable i32 minimum refused by move validation; ordered list targets; return choice; decoded rotary summary; hash and summary rejection for target/speed/acceleration/return mutations.
- Driver/FakeLmc: two-strip wire order, dark/idle motion assertions, moving-bit polling, optional return, axis-never-stops timeout; stop injection at every wire event with StopExecute at most 40 ms after the latch; relative bench movement, position query and refusal on an active board without an emergency interrupt of that board.
- Human-only access: agents cannot change rotary settings; rotary CLI refuses without a TTY and without an enabled profile; all profile field setters exercised.

## Gate

Final full gate (Rust 1.99.0, integration lane, `CARGO_INCREMENTAL=0`, hardware disabled):

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Pass |
| macOS `cargo clippy --workspace --all-targets -- -D warnings` | Pass |
| Windows `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings` | Pass |
| `cargo test --workspace --no-fail-fast` | **1,096 passed, 1 failed, 4 ignored**, across 81 test/doc targets; exit 101 |
| `cargo xtask deps` | Pass: 27 workspace crates, 5 platforms, all 17 dependency rules |
| `cargo xtask golden-verify` | Pass: all 188 fixtures match the manifest |
| `git diff --check` | Pass |

Final free disk was 21 GiB (above the task's 4 GiB stop threshold). No expected values or golden fixtures were changed.
The sole failure was `registry_tests::commands_snapshot` in `crates/engine/tests/engine.rs`: the required Device Settings additions introduce `rotary`, plus the `workArea`, `cal` and `redDot` payloads the GUI already sends. Those existing GUI payloads previously disagreed with its command's accepted fields. The original snapshot is unchanged pending explicit permission under the task's prohibition on expected-value edits; `.snap.new` is the reviewable generated result.

## Unverified

D12 is **UNSIGNED, proposed**. Pulse/s and acceleration units, sign-magnitude position replies, moving-bit timing, direction, physical calibration/registration, motor behaviour on StopExecute, optical stop latency, overlap appearance and the acceleration/timeout model are all unverified on the actual board. No homing is implemented. `hw` feature compilation was intentionally not attempted. Windows was cross-checked with Clippy; its tests were not executed on a Windows host.

`HARDWARE.md` section 6 contains R0–R3 for a human session after HIL-2. D1–D11's signature is not extended to D12.

## Files changed

- `docs/P3C-ROTARY-REPORT.md` (this report)
- `ATTRIBUTION.md`
- `HARDWARE.md`
- `apps/openlaser-hw/src/args.rs`
- `apps/openlaser-hw/src/flow.rs`
- `apps/openlaser-hw/src/lib.rs`
- `apps/openlaser-hw/src/profile.rs`
- `apps/openlaser-hw/src/rig.rs`
- `apps/openlaser-hw/src/summary.rs`
- `apps/openlaser-hw/tests/openlaser_hw.rs`
- `crates/arm/src/tests.rs`
- `crates/driver/src/lib.rs`
- `crates/driver/src/lmc_backend.rs`
- `crates/driver/src/lmc_session.rs`
- `crates/driver/src/rotary.rs`
- `crates/driver/tests/rust_v1_session.rs`
- `crates/driver/tests/t6.rs`
- `crates/engine/src/cmds/device.rs`
- `crates/engine/src/devices.rs`
- `crates/engine/src/jobs.rs`
- `crates/engine/src/tests.rs`
- `crates/gcode/tests/gcode.rs`
- `crates/human/src/arm_dialog.rs`
- `crates/human/src/tests/mod.rs`
- `crates/lmc-sim/src/fake.rs`
- `crates/lmc-sim/src/fake_tests.rs`
- `crates/lmc/src/axis.rs`
- `crates/lmc/src/decode.rs`
- `crates/lmc/src/encode.rs`
- `crates/lmc/src/lib.rs`
- `crates/lmc/tests/acceptance.rs`
- `crates/lmc/tests/common/mod.rs`
- `crates/machine/src/builtin.rs`
- `crates/machine/src/lib.rs`
- `crates/machine/src/profile.rs`
- `crates/plan/src/fingerprint.rs`
- `crates/plan/src/image.rs`
- `crates/plan/src/lib.rs`
- `crates/plan/src/planner.rs`
- `crates/plan/src/rotary.rs`
- `crates/plan/tests/image.rs`
- `crates/plan/tests/plan.rs`
- `crates/render/tests/render.rs`
- `crates/safety/src/checks.rs`
- `crates/safety/src/lib.rs`
- `crates/safety/tests/t5_rules.rs`
- `crates/types/src/emission.rs`
- `crates/types/src/lib.rs`
- `crates/types/src/refusal.rs`
- `crates/ui-egui/src/dialogs.rs`
- `crates/ui-egui/src/panels.rs`
- `crates/ui-egui/src/tests/app_kittests.rs`
- `docs/DIVERGENCES.md`

Generated review artifact (not accepted): `crates/engine/tests/snapshots/engine__registry_tests__commands.snap.new`.
