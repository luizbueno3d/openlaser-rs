# I2 integration notes

I2 work completed in this checkout:

- `camera.list`, `camera.status`, `camera.stop` and calibrated mapping remain agent-readable/usable. `camera.start` and
  every `camera.calibrate.*` command now require `Access::Human`; the engine refuses Headless/Synthetic/Replay origins
  before decoding parameters. The Camera Control panel still routes its Start click through `execute_widget`, which
  applies the UI human-input provenance gate.
- Unparked the three UI kittests, T2 hatch cases and engine `panic_hunt`. The UI palette test used an obsolete
  `tools.rectangle` parameter shape; it now supplies the current `xMm`/`yMm`/`wMm`/`hMm` fields without changing its
  assignment or undo assertions.

## Whole-app simulator acceptance (blocked by the real Human dialog)

`apps/openlaser/tests/whole_app.rs` now contains all seven requested I2 cases. It instantiates the shipping
`OpenLaserApp`, a real `Session`, the real `Human` and `SimProvider`/`FakeLmc`. No token witnesses, emulated app
state, hardware features or production seams were added. The app's real control server binds port zero and uses
`openlaser::control_runtime::fast_stop`; bridge calls run on a worker while the harness drains requests.
Kittest 0.36 omits `raw_input_hook`, so the harness explicitly calls that production hook before each frame.
Human clicks use pointer press/release events. Automation events enter the real `Human::inject`; the wrapper only
counts returned tokens and provides an injection queue, delegating every safety decision to `Human`.

Passing cases in the workspace gate:

- `frame_request_without_approval_sends_no_lighting_packets`: a request card alone produces no packet, light,
  burn, receipt or token on the connected simulator.
- `stop_served_while_human_surface_open`: a mutating bridge request is refused while the real ARM modal is open;
  control-server STOP returns latched/confirmed, cancels PendingArm and leaves MO closed and ports zero.
- `synthetic_input_cannot_arm_end_to_end`: injected Frame and Start (including Start with a valid human receipt),
  checklist clicks, typing and Confirm cannot mint or mark; even a completed human form cannot be confirmed by
  automation. Subsequent genuine human input still arms and finishes on FakeLmc.
- `grouped_shape_lands_where_designed`: commands create two nested document groups with scale, rotation and
  translation. Planned coordinates match the independently calculated `(30 - 2*y, 20 + 2*x)` transformation;
  real human Frame/ARM burns those segments within one galvo unit, rather than at local coordinates.

Outstanding / failing cases (kept enabled, not ignored):

- `kittest_business_card_flow`: imports 403 business-card objects, checks layers and a human receipt, independently
  decodes the exact pending job bytes and checks the ARM display's power/frequency/pulse/speed values. It then
  fails on an off-screen checklist target before it can ARM, compare all burns or exercise second-run Esc.
- `mcp_stop_latches_while_ui_thread_blocked`: the business-card prerequisite hits the same ARM dialog blocker,
  before the queued-read/independent-stop-connection assertions can run.
- `mcp_bridge_end_to_end`: bridge import, command, plan, submit and visible submission card are covered; the
  remaining human framing/ARM/finished-status sequence is not yet green. The current harness stops at an
  off-screen approval target `(1365.9, 3209.0)`; the Laser panel already has a scroll area, so this case needs
  ordinary harness scrolling before it can reach the separate Human modal blocker. This is incomplete test
  interaction, not evidence of another production defect.

### Red-zone blocker — stop and report, no workaround

The business card has 67 numbered notices. `crates/human/src/arm_dialog.rs:131` places the full decoded summary
and every notice in an unbounded modal before the checklist/ARM field/Confirm controls (notice loop at line 241;
field at line 146). There is no scroll area or height bound. On the 1800 x 1400 harness viewport a checklist click
target is already at `(518.6, 1408.0)`, outside the screen. The original symptom was an ARM field that remained
empty after pointer focus and typing; an explicit viewport assertion makes the cause reproducible.

Per the task's restriction on changes inside `crates/human`, implementation stops here. No restricted crate was
changed, no safety assertion was weakened and the viewport was not enlarged to conceal the failure. The Human
owner must make the complete decoded confirmation and controls reachable before these tests can be completed.
I2 remains partial; the new suite intentionally records failures until that blocker is resolved.

## Gate for this blocked hand-in

All commands used Rust 1.99.0, `OPENLASER_NO_HW=1`, no hardware feature, and the explicitly requested
`~/AI/OpenLaser/.target/integ` lane. Free disk was 20 GiB before the build and 19 GiB at the gate.

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings`: passed.
- `cargo test --workspace --no-fail-fast`: failed; 1,038 passed, 4 failed, 4 ignored across 79 test/doc-test
  result groups. New suite: 4 passed, 3 failed, none ignored, with the failures described above. The other failure
  was the existing `lmc::t6_stop_p99_under_100ms_all_states` at `crates/driver/tests/t6.rs:344` (p99/worst 211 ms).
  That workspace run overlapped the initial targeted test run. An isolated recheck with
  `cargo test -p openlaser-driver --test t6 t6_stop_p99_under_100ms_all_states -- --nocapture` also failed
  (0 passed, 1 failed, 34 filtered; p99/worst 152 ms). The cause is not diagnosed; no latency assertion or driver
  code was changed, and `crates/driver` is also excluded from fixes by the task.
- `cargo xtask deps`: passed; 27 crates, 5 platforms, rules 1–17 hold.
- `cargo xtask golden-verify`: passed; all 174 manifest fixtures match.

Still ignored (unchanged): Python GRBL interoperability needs its external fake server; S1/S2 UI performance
benchmarks require Luiz's release benchmark machine; `layout_screenshot` requires an explicit review PNG path.

Commit attempt: `git add` and `git commit` were refused by the filesystem sandbox when creating
`/Users/luizbueno/AI/OpenLaser/openlaser/.git/worktrees/integration/index.lock` (`Operation not permitted`).
The session does not allow permission escalation. Changes remain uncommitted on `integration`; the latest
existing commit is `d0f0e96`. No push, merge, branch switch or restricted-crate edit was performed.

## Fixes found by the whole-app tests (2026-10-26)
- ARM dialog (crates/human/src/arm_dialog.rs): with the business card's 67 notices the checklist, the ARM field and
  Confirm were pushed off-screen, so a person could not arm. The summary now scrolls inside a bounded area; the
  bench power cap stays outside the scroll and always visible.
- Laser panel (crates/ui-egui/src/panels.rs): agent request cards ("Approve framing", submitted jobs) were drawn
  below the notice list, about 2000 px down. They now come first, and the notice list scrolls in 160 px.
- panic_hunt hung for 10 minutes: as the Human origin it called `camera.start`, which opened the real webcam and
  blocked on the macOS permission prompt. The hunt now skips device-opening commands; it runs in ~9 s with no
  contained panics.
- Follow-up from the same finding: opening a camera runs on the UI thread and can stall. Camera Start is now
  disabled while framing or marking, so a stall can never freeze the on-screen STOP mid-job. Open the camera on
  a worker thread (Phase 3).
- `t6_stop_p99_under_100ms_all_states` exceeded the budget (152–211 ms) only while other builds loaded the CPU;
  3/3 runs pass on a quiet machine. It is a real-time simulator test; watch it in CI.
