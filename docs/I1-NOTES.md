# I1 notes: MOPA core path on the simulator

Integration step I1 (SPEC §7.11) on branch `integration`: main + wp3-safety, wp4-lmc, wp5-driver, wp6-engine,
wp9-human and the current wp1-model and wp2-import. Everything below ran with `OPENLASER_NO_HW=1`, no `hw` feature,
and nothing opened a USB device: every machine is lmc-sim's FakeLmc (virtual clock, or the system clock with 2 ms per
transfer for the real-time stop budget).

## 1. The failures I1 started with, and their root causes

| Failure | Root cause | Fix |
|---|---|---|
| `rust_v1_session` | `openlaser-plan` was not a dev-dependency of the driver, so the test could not build the shipping `mark_box10` `EmitPlan` | `crates/driver/Cargo.toml` dev-dependency; the job is built in `crates/driver/tests/rust_v1_session.rs` (`mark_box10_job`) |
| `rust_v1_session_connect_frame_box10` | candidate file never written | written with `UPDATE_RUST_V1=1` after review (§3) |
| abort without the dummy packet (`StopReport` note "the protocol crate provides none") | `openlaser_lmc::Packet` had no constructor the driver could use | `openlaser_lmc::abort_packet()` (`crates/lmc/src/packet.rs`), byte-exact with the v0 abort golden (sha256 `e479558e…6a18`, test `abort_packet_is_the_v0_dummy_end_of_list_packet`); the session's abort packet is no longer optional (`crates/driver/src/lmc_session.rs`, `Session::new`) |
| `t6_stop_p99_under_100ms_all_states` (p99 195-229 ms) | FakeLmc's `poll_single`/`poll_packet` slept the whole 20 ms slice before looking at the pending transfer, so every abort transfer cost 20 ms even with 2 ms latency (10 transfers ≈ 200 ms). A real `wait_next_complete(≤ 20 ms)` returns when the transfer completes | `crates/lmc-sim/src/fake.rs`: polls wait only until the transfer completes (or the whole slice when nothing is coming); test `polls_return_when_the_transfer_completes_not_after_a_whole_slice`. Now p99 ≈ 30 ms, worst ≈ 33 ms |
| `t6_slow_packets_execute_without_deadlock` (Watchdog, waited 35 960 ms) | test bug: its per-packet estimator reset the speed words at every packet, but GP sets MarkSpeed once, in the list's first packet; packets 2–5 were estimated at 276 units/ms instead of 83, 3.3× too short, so the watchdog (est × 1.5 + 2 s, SPEC §5.3) rightly faulted a board that was still marking. The shipping encoder's `packet_est` (`decode::packet_times`) already carries the model across packets | the test's estimator carries the words across the list (`packet_ests`); and the "ExecuteList after packet 1" assertion now looks at the job's list, not the Frame Once's lighting list that came before it |
| `t6_no_write_while_not_ready`, `t6_mo_closed_ports_zero_after_every_stop` ("no matching event") | test bug: the randomised stop can land in the connect or the frame (both stop states of SPEC §5.4), which then never reach the state `framed_run` waited for | `framed_run_or_stopped`: a stop or fault before the run returns its events instead of waiting forever |
| `t6_transfer_timeout_faults_and_stops` | test bug: `ReplyTimeouts { every: 40 }` landed in the 2 s Frame Once (≈ 200 status polls), before the run | a fault-free probe counts the replies of connect + frame; the 25th reply of the run times out; the test also checks the message ("no reply within 250 ms") and that the fault was in the run |
| `t6_session_init_once` | test bug: after `Finished` it waited for `State(Idle)` and caught the run's own Idle (the runtime emits the outcome, then the state), queued the next run while the frame was still going, and got `MachineBusy` | wait for the frame's receipt, then its Idle (as `framed_run` does) |
| 4 × openlaser-hw `cli_*` ("machine profile directory is unavailable") | `engine_env` passed `None` as the profile folder, which `format::machines::load_all` reports as a skipped profile, and the rig refuses device commands while a profile is skipped (fail-closed, correct) | `engine_env` gives every test its own temporary profile folder (never the user's real one) |

Contract resolved at I1 (WP2 follow-up "pin WP3 to this convention"): gantry `Section.paths` and frames were
controller coordinates in `plan` (WP3) but centre-origin bed mm, Y up, in `gcode` (WP2), so every gantry job was moved
by half the bed a second time; a 10 mm square at the centre of OM's 600 × 400 mm bottom-left CO2 bed was refused as
off the bed. `plan::gantry_transform` now plans centre-origin bed mm (the machine's origin plays no part; `gcode`
maps to it), `plan::gantry_bed` is the centred bed, `plan::gantry_home` places the controller's origin for the
estimate, safety's T5 gantry tests use the convention, and the engine's gantry summary and decoded burns are in the
same frame (`crates/engine/src/jobs.rs`, `bed_centre`). Test: `gantry::gantry_job_lands_where_it_was_designed`
(`crates/engine/tests/engine.rs`; fails with the old transform) and
`gantry_plans_centre_origin_bed_mm_whatever_the_controller` (`crates/plan/src/transform.rs`).

`xtask deps` rule 13 (WP5 report): its grep matched `Origin::Human` inside `FrameOrigin::Human`, which forced the
driver tests to alias the import; it now matches the path on its own (`names_path`, unit-tested).

Code changes behind T6 test changes: none of the driver's behaviour changed for them (the T6 expectations already
matched SPEC §5.3/§5.4 except where noted). `t6_stop_p99_under_100ms_all_states` now also stops runs
(streaming, buffer full, packet in flight, BUSY): one run in three stops in the connect or the Frame Once, the others
are framed by one Loop pass (stopped), then stopped at a random wire event of the run. Before, the run was queued
while the frame was still going and refused `MachineBusy`, so no streaming stop was ever measured.

Found by the mutation checks (§4): `t6_abort_no_write_while_not_ready` did not catch a dummy packet sent into a
not-READY board, because FakeLmc refused such a write before recording it on the wire. FakeLmc now records the
attempted OUT transfer first (it goes on the wire whatever the board's state), and the test also asserts
`writes_while_not_ready == 0`. And `t6_stop_after_queue_cancels_queued_run` was flaky once the abort writes its dummy
packet: it asserted "no packet at all"; it now asserts that none of the run's packets (and no `Fiber_SetMo(1)`) went
out, only the abort's dummy packet.

## 2. Tests un-ignored (all pass)

- driver: the whole T6 table (`crates/driver/tests/t6.rs`, 34 tests incl. GRBL), `rust_v1_session` ×2,
  `runtime::tests::{run_finishes_with_a_valid_token_and_own_receipt, arm_token_single_use}`.
  `grbl_interop_with_python_fake` stays ignored (needs the Python fake on 127.0.0.1:2323).
- plan: `plan_business_card_matches_om` and the other 11 (`crates/plan/tests/plan.rs`).
- safety: the 9 T5 tests incl. `safety_outside_work_area_refused` (B10, 100.5 mm at scale 1.1834).
- engine: `e2e_mopa_business_card_sim` and the 11 other emission tests, `undo_roundtrip_edit_commands_needing_geometry_and_import`,
  `tests::field_wider_than_rated_is_refused`. `panic_hunt` stays ignored (I2).
- human: `human_click_after_300ms_mints_token_with_real_mint`.
- openlaser-hw: `cli_run_sim_marks_business_card`, `cli_first_emission_power_cap_refused_on_the_simulator`,
  `cli_trace_sim_frame_writes_t1_lines`, `cli_frame_extent_reaches_past_the_work_area_on_red_light_only`,
  `cli_skipped_profile_file_refuses_device_commands`, `arm_confirmation_is_stamped_after_the_line_is_read`,
  `cli_wrong_nonce_refused`, `cli_correct_challenge_arms_and_marks`.
- Not un-ignored (paths I1 must not edit): `crates/gcode/tests/t2.rs::t2_hatched_cases` (WP2) and WP7's
  `job_simulate_business_card` (WP7 is not merged at I1). See the requests in §6.

## 3. rust-v1 session candidates

`tests/golden/rust-v1-candidate/session/connect_frame_box10.json` and `connect_frame_box10_mark_box10.json`, checked
before writing: connect = the R05 §1.4 init verbatim, once; Frame Once = `ResetList · GetVersion · 3 × (packet ·
SetEndOfList)` with `ExecuteList` after the 3rd packet (GP parity), status polls until !BUSY, then `Fiber_SetMo(0) ·
WritePort(0)`; mark_box10 = `ResetList · WritePort(0x0001) · Fiber_SetMo(1) · GetVersion · packet · SetEndOfList ·
ExecuteList`, polls until !BUSY, `Fiber_SetMo(0) · WritePort(0) · GetMarkTime(3)`, no abort, no reset. The packet
sha256s are the lmc candidates' (`frame_box10` aae28ef4…, ffed1f4b…, 0fa54ddf…; `mark_box10` 69aa7150…).

## 4. Mutation checks (SPEC §6.6)

Each fix was reverted locally (one at a time, never committed), its named test run, and the tree restored with
`git checkout` (script kept out of the repo). "FAILED" is the expected result.

| Bug | Mutation (the prototype's behaviour put back) | Test(s) | Result |
|---|---|---|---|
| B1 3 s wait cap | `run_list` stops waiting for !BUSY after 3 s and runs the postlude (MO close) | `t6_busy_20s_completes_without_mo_close` | FAILED ✓ |
| B2 abort into a not-READY board | the abort's dummy packet goes out without waiting for READY | `t6_abort_no_write_while_not_ready` | first run: **passed (not caught)**; after the FakeLmc/test fix of §1: FAILED ✓ |
| B3 first jump elided | the encoder skips the first jump when the start is the assumed position (centre) and drops the settle | `t1_dot_test_has_explicit_jump_and_settle` | FAILED ✓ |
| B4 confirm shows Frame/requested values | (a) the ARM dialog shows a nominal (rounded) power; (b) `emission_summary` takes the row power from the plan's requested power instead of the decoded MarkCurrent | (a) `kittest_confirm_shows_decoded_layer_params`; (b) `summary_rows_and_words_come_from_one_decoded_block`, `prop_summary_matches_plan` | FAILED ✓ (both). Note: the engine suite alone does not catch (b) |
| B5 ARM always true / sticky | (a) the driver skips its nonce check; (b) `run_armed` skips the revision check; (c) openlaser-hw skips the TTY gate; (d) `openlaser-engine` gains a normal dependency on `openlaser-arm` | (a) `arm_token_single_use` (runtime unit test and T6); (b) `arm_token_invalid_after_edit`; (c) `cli_arm_refused_without_tty`; (d) `cargo xtask deps` rule 4 | FAILED ✓ (all four) |
| B6 hidden + Output burns unframed | (a) safety skips the hidden-output refusal; (b) the planner frames shown layers only | (a) `safety_hidden_output_refused`; (b) `prop_frame_covers_burn` | FAILED ✓ (both) |
| B7 speed < 3.61 mm/s → 0 units | (a) safety skips the zero-units refusal; (b) `speed_to_units` rounds instead of truncating | (a) `safety_speed_zero_units_refused`; (b) `prop_speed_units_stable_over_field` | FAILED ✓ (both) |
| B8 orphan objects dropped | safety skips the orphan refusal | `safety_orphan_object_refused` | FAILED ✓ |
| B9 3-coordinate points crash | the OM workspace parser ignores its malformed-point flag | `import_malformed_points_refused` | FAILED ✓ |
| B10 field bound = galvo reach | safety's galvo area is ±F/2 (±118.34 mm) instead of the rated work area | `safety_outside_work_area_refused` | FAILED ✓ |

The I1 fixes and follow-ups were checked the same way (each test fails with its fix reverted):
`abort_packet_is_the_v0_dummy_end_of_list_packet` and the idle-abort half of `t6_abort_mid_list_sends_no_execute_list`
(dummy packet), `polls_return_when_the_transfer_completes_not_after_a_whole_slice` (FakeLmc latency),
`plan_refuses_an_unusable_curve_tolerance`, `ten_us_word_rounds_up_and_refuses`,
`safety_negative_jump_and_polygon_delays_refused`, `safety_speed_above_the_word_cap_refused`,
`summary_refuses_a_list_spliced_from_another_job` (N-c), `in_work_area_admits_exactly_the_encoder_words_of_each_axis`
(N-e), `duplicate_points_are_elided_and_dist_follows_the_last_sent_point` (N-f).

## 5. Follow-ups folded (integration/followups.md)

WP3: curve tolerance → `unsupported` fact; MO settle rounded up; negative jump/polygon delays refused; speed (and jump
speed) above the 65535-unit word refused. WP4: N-a, N-b (tests), N-c (summary re-encodes each section), N-e (exact
work-area words), N-f (`Error::SubUnitSection`), N-h (ATTRIBUTION), N-g (SPEC-CHANGES).

Not done here, blocked on `crates/types/src/refusal.rs` (WP1): the air-assist notice and the `validate_frame`
notices need new `NoticeCode` variants (requests in §6). The TTY `at` stamp (WP3 mint note) is covered by
`arm_confirmation_is_stamped_after_the_line_is_read`, now running; the zero-hash refusal no longer blocks arming since
WP4's real `bytes_hash` landed (`e2e_mopa_business_card_sim` arms and marks).

## 6. Requests for package owners (files I1 must not edit)

1. **WP1** (`crates/types/src/refusal.rs`): add `NoticeCode::AirAssistUnavailable` ("air_assist_unavailable") and
   `NoticeCode::NotFramed` ("not_framed"). Then I1/I2 adds `PlanFacts::air_assist_unavailable: Vec<LayerKey>` (the
   planner records a gantry layer whose air-assist request the machine cannot honour) with a safety notice, and
   `FramePlan::notices()` filled by `validate_frame` for orphan, too-detailed and over-budget objects left out of the
   frame (WP3 follow-ups).
2. **WP1**: the importer's ASSUMED_DEFAULT notice still reuses `ImportEntitySkipped` (WP2 follow-up); a distinct code.
3. **WP2** (`crates/gcode/tests/t2.rs`): un-ignore `t2_hatched_cases` (its reason, WP1's `hatch_om`, is merged; it
   passes with `--ignored` on `integration`).
4. **WP7**: `job_simulate_business_card` (`crates/automation/src/backend.rs`, ignored "needs I1") can be un-ignored when
   WP7 rebases on `integration`; `openlaser_lmc_sim::simulate_job` and the engine path it needs are merged.
5. **WP9 / WP5**: `openlaser-hw --trace` on the real USB device asks the driver to re-export
   `openlaser_lmc::{Single, Packet}` so the transport can be wrapped; not needed for the simulator path (left for I2).
6. **WP1**: N-d (`PassRow.paths` counts mark runs, not planned paths: 304 paths vs 64 runs on the business card) is a
   rename or doc fix in `crates/types` (WP1's file).
