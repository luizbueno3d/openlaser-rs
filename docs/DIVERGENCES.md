# Intentional divergences: shipping Rust vs the Python oracle

> **Status: D1–D11 SIGNED by Luiz on 2026-10-26, for controlled hardware validation only** (decision D13, PLAN D13,
> SPEC S15, §9 item 11). This approves the protocol changes in §3 so the bench sessions of `HARDWARE.md` can run. It
> does **not** declare the implementation hardware-safe: that is decided by the HIL results, and the open risks are
> listed in `HARDWARE.md` § Remaining safety risks. The `rust-v1` candidates are frozen in `tests/golden/rust-v1/`.

Written by WP0 from SPEC §5.6 (revision 2) and `tests/golden/README.md`, completed by the integrator at I1 from the
rust-v1 listings (frozen 2026-10-26 from the candidates): `tests/golden/rust-v1/lmc/*.txt` (every packet of every T1 scenario, all 256
slots, with a decoded listing) and `tests/golden/rust-v1/session/*.json` (the driver's wire stream on a
READY FakeLmc).

`py-compat` (test-only, SPEC §5.5) reproduces oracle **v0** (`6aed8ff`) exactly. Shipping packets are compared with
oracle **v1** (`5f7aa63`) minus the lines below (`crates/lmc/tests/t1_shipping_vs_v1.rs`: a divergence-aware
comparator over decoded commands; every other command, every `dist` word and the packet framing must be equal, and
v1's own stream is rejected, so the divergences are required, not merely allowed).

## 1. What the bytes look like (rust-v1 candidate vs oracle v1)

Command counts are the real commands before the explicit `EndOfList`. Every list keeps GP's packet framing: 256
commands per packet; the last packet ends with `EndOfList` and `0x8002` padding.

| Scenario | v1: packets / commands | rust-v1: packets / commands | First rust-v1 packet | What differs (all of it) |
|---|---|---|---|---|
| `mark_box10` | 1 / 17 | 1 / 18 | `69aa7150…` | D1: `JumpDelay(8) · JumpTo(31384,31384,0,1957)` becomes `JumpDelay(200) · JumpTo(31384,31384,0,1957) · DelayTime(100)` |
| `dot_test` | 1 / 14 | 1 / 15 | `bdd788bf…` | D1: `JumpDelay(200) · JumpTo(32768,32768,0,0) · DelayTime(100)` before `MarkTo(32782,32768,0,14)` (v0 elided this jump: B3) |
| `line_test` | 1 / 14 | 1 / 15 | `40c7d260…` | D1 |
| `long_job` | 9 / 2212 | 9 / 2214 | `39fe116b…` | D1, and `JumpDelay(8)` set again before the second jump |
| `bc_mark_vector-engrave_pass1` | 6 / 1522 | 6 / 1524 | `7088b032…` | D1, `JumpDelay(8)` again before the second jump |
| `bc_mark_raster_pass1` | 8 / 1950 | 8 / 1952 | `65ae13e8…` | D1, `JumpDelay(8)` again before the second jump |
| `frame_box10` (Frame Once) | 3 / 607 | 3 / 610 | `aae28ef4…` | D3: `JumpSpeed(830)` before the first (dark) jump; D1 on the lighting list: `JumpDelay(200) · JumpTo(start) · DelayTime(100)`, then `JumpDelay(8)` again |
| `bc_frame` (Frame Once) | 74 / 18808 | 74 / 18811 | `4bbb1fd8…` | D3, and D1 as in `frame_box10` |
| `frame_box10_loop_pass`, `bc_frame_loop_pass` | not captured (GP appended Loop passes to one endless list) | 1 / 14; 13 / 3141 | `fdd7fd78…`; `4bbb1fd8…` | D8: one complete one-pass list per Loop pass |

Every list's first `JumpTo` has its `dist` computed from (0x8000, 0x8000) (D2), which is what v1's packets already
show (each OM layer-pass ran on a fresh GP controller): `mark_box10` 1957, `dot_test` 0.

## 2. What the session looks like (rust-v1 session candidates vs the oracle streams)

The oracle streams (`t1_lmc/v0|v1/*.json`) are one job per process: open, init, job, abort, close. The shipping
session (`tests/golden/rust-v1/session/connect_frame_box10_mark_box10.json`) is one connection:

```
connect     GetSerialNo GetVersion Reset WriteCorTable … EnableZ WriteAnalogPort1(0x07FF) EnableZ   (R05 §1.4, once)
            50 ms pause
Frame Once  ResetList · GetVersion · packet · SetEndOfList · GetVersion · packet · SetEndOfList · GetVersion · packet
            · SetEndOfList · ExecuteList (after the 3rd packet, GP parity) · GetVersion … until !BUSY
            · Fiber_SetMo(0) · WritePort(0)                       (the frame receipt only after !BUSY: S20)
mark_box10  ResetList · WritePort(0x0001) · Fiber_SetMo(1) · GetVersion · packet · SetEndOfList · ExecuteList
            (its last packet) · GetVersion … until !BUSY · Fiber_SetMo(0) · WritePort(0) · GetMarkTime(3)
            (no abort, no Reset, no close)
```

against v1, per job: `open · init · ResetList · WritePort · Fiber_SetMo · GetVersion · packet · SetEndOfList ·
ExecuteList · GetVersion · Fiber_SetMo · WritePort · GetMarkTime · GetVersion · StopExecute · Fiber_SetMo · ResetList
· GetVersion · dummy packet · SetEndOfList · ExecuteList · Fiber_SetMo · WritePort · close`.

The abort runs only on a stop or a fault: `StopExecute · Fiber_SetMo(0) · ResetList`, then **only if READY within
50 ms** the dummy end-of-list packet (`openlaser_lmc::abort_packet()`, 256 × `0x8002`, sha256 `e479558e…6a18`,
byte-exact with v0's) `· SetEndOfList`, and `ExecuteList` **only if no list was executing**, then `Fiber_SetMo(0) ·
WritePort(0)`, stale replies drained, and a `GetVersion` that must show !BUSY for the `StopReport` to confirm. On the
simulator (real-time variant, 2 ms per transfer) the whole stop takes p99 ≈ 30 ms, worst ≈ 33 ms, from connect,
framing, streaming, a full buffer, a packet in flight and BUSY (`t6_stop_p99_under_100ms_all_states`).

## 3. Byte- and sequence-level divergences

| # | Divergence | v0 / v1 behaviour | Where it shows | Bug / source |
|---|---|---|---|---|
| D1 | Every list starts its motion with an explicit `JumpTo` using the **long** jump delay (200) and a `DelayTime(100)` settle (1 ms) before the first `MarkTo` or light-on; the short delay (8) is set again before the next jump | v0 elides the first jump to the assumed start; v1 jumps with the short delay (8) and no settle | every list (§1) | B3, PLAN §3.3 |
| D2 | Every list's first-jump `dist` is computed from (0x8000, 0x8000), so packet bytes never depend on session state | each OM layer-pass ran on a fresh GP controller, so v1's packets already match | every list (`mark_box10` 1957, `dot_test` 0) | SPEC §5.2 rule 6, rev 2 |
| D3 | Lighting lists set `JumpSpeed(light)` before their first (dark) jump | GP leaves the board's previous jump speed | `frame_box10`, `bc_frame` | SPEC §5.2, rev 2 |
| D4 | One session per connection: init (`GetSerialNo` … `EnableZ`, incl. `Reset`) once per connection; no per-job abort, close or USB reset | per job (v0, v1) | session candidates (§2) | PLAN §3.3 |
| D5 | MO close and port-off only after BUSY clears; no wait ever proceeds on a deadline (a progress-aware watchdog stops a stuck board instead); abort only on stop or fault | v0: 3 s wait cap, MO closed at +3 s, StopExecute at +6 s | T6 `t6_busy_20s_completes_without_mo_close` | B1 |
| D6 | The abort's dummy end-of-list packet goes out only if READY within 50 ms; its `ExecuteList` only if no list is executing (OM v1 `_safe_abort`); always MO(0) and **WritePort(0)** at the end | v0 sent it unconditionally and left the red-light bit set (`WritePort(0x0100)` after a frame) | `frame_box10`, `bc_frame` final `WritePort`; T6 `t6_abort_no_write_while_not_ready`, `t6_abort_mid_list_sends_no_execute_list` | B2, rev 2 |
| D7 | An in-flight packet is completed before a stop (`cancel_all` only on overrun, flagged `possible_desync`) | — | T6 `t6_inflight_packet_completed_before_stop_execute`; stop bound: see Sign-off, D7 | PLAN §3.5 |
| D8 | Frame Loop sends one complete list per pass and counts a pass only after the board reports !BUSY | GP's FrameLoop appended to one endless list (never captured) | `*_loop_pass` candidates; T6 `t6_frame_receipt_only_after_executed_pass` | S20, rev 2 |
| D9 | `ExecuteList` may come before the 3rd packet when the run-ahead window fills or READY drops; packet bytes unchanged, only the single-command order differs | GP waits for the 3rd packet | T6 `t6_slow_packets_execute_without_deadlock`, `t6_buffer_depth_1_and_2_complete`; ordering and underrun: see Sign-off, D9 | SPEC §5.3, rev 2 |
| D10 | Zero delays keep the `0x8000` sign word (`delay_word(0) = (0, 0x8000)`); the parameter cache resets at every list start, so every list carries the full parameter block | same as GP (kept on purpose until H9) | every list | GOLD item 9 |
| D11 | (I1) The MO settle is rounded **up** to 10 µs (`ceil`), so the board never waits less than the profile says | OM truncated (`int()`): up to 10 µs shorter | only a settle that is not a multiple of 10 µs; the built-in 8000 µs is `DelayTime(800)` either way | WP3 follow-up |

## 4. Refusals instead of silent behaviour (no byte change in the captured scenarios)

| # | Divergence | Python behaviour | Bug / source |
|---|---|---|---|
| R1 | Work area = the lens's rated field, ±100 mm for Luiz's 200 mm lens, intersected with the strictly encodable square; refused, never clamped | v0: ±118.34 (galvo reach) | B10 |
| R2 | +F/2 exactly is refused | v0 and v1 clamp it to 65535 (T0 shows the clamp) | SPEC §5.2 rule 2 |
| R3 | Speed encoded once to whole units; 0 units refused (below 3.6115 mm/s at F = 236.68) | v0 encodes 0 | B7 |
| R4 | NaN and infinite parameters refused | OM snaps a NaN pulse width to 2 ns | — |
| R5 | Frequencies that would wrap a 16-bit period are refused | GP wraps (`& 0xFFFF`) | — |
| R6 | Hidden + Output layers refused | v0 burned them unframed | B6 |
| R7 | Orphan objects (layer not in the table) refused | v0 dropped them | B8 |
| R8 | An empty Fill layer refused | OM `raster_skip` | — |
| R9 | Gantry overspeed refused | `gcode.py` caps it silently (T2 `speed_power_edges`, gantry `business_card`: `py-compat` matches the capped output, shipping refuses) | — |
| R10 | Open shapes in a Fill layer become numbered notices | skipped silently (bytes identical) | — |
| R11 | (I1) A curve tolerance that is not a number from 0.001 to 0.1 mm is refused | OM has no such setting | WP3 follow-up |
| R12 | (I1) A mark speed, or the profile's jump speed, above what the 65535-unit speed word carries (F × 1000 mm/s) is refused | `speed_to_units` caps at 65535 | WP3 follow-up |
| R13 | (I1) A negative jump delay or polygon delay is refused (zero keeps 0x8000, D10) | sent as `(magnitude, 0x8000)` | WP3 follow-up |
| R14 | (I1) The ARM confirmation is refused unless each list's bytes are exactly what its plan section encodes to (a list spliced in from another job with the same words is refused) | the confirm showed the Frame values (B4) | WP4 N-c, S22 |
| R15 | (I1) A layer-pass whose every path is shorter than one galvo unit is refused by name ("nothing would be marked") | GP would open MO and mark nothing | WP4 N-f |

## 5. Import, profile and confirmation

| # | Divergence | Python behaviour | Bug / source |
|---|---|---|---|
| I1 | SVG imported at true scale; unitless SVG refused unless 96 or 72 px per inch is chosen | OM's SVG import (T4 SVG rows are informational) | SPEC §3.6, GOLD item 11 |
| I2 | No STL import | OM imports STL (T4 STL rows are informational) | GOLD item 11 |
| I3 | DXF SPLINE evaluated (the shipping importer); `dxf_om::parse_dxf_om` keeps OM's control/fit points for T4 | control/fit points | SPEC §3.6 |
| I4 | Points that are not (x, y) refused | v0 crashed | B9 |
| I5 | Mirror flags live in the machine profile (`cal.negate`, human-only) | browser localStorage | Appendix B |
| I6 | The repeat count and each layer's decoded values are shown at confirmation; the ARM token binds the bytes (`bytes_hash`) | the confirm showed Frame values | B4, S22 |
| I7 | G-code uses per-machine-kind layer defaults | MOPA defaults leaked into G-code (OM v1 did not fix it) | B11 |

## 6. The prototype's bugs: where each fix lives and its regression test

Each B1–B10 test was mutation-checked at I1 (fix reverted locally, test fails, fix restored): `docs/I1-NOTES.md` §4.

| Bug | Fix | Test |
|---|---|---|
| B1 3 s wait cap | wait for !BUSY with a progress-aware watchdog, never a deadline (D5) | `t6_busy_20s_completes_without_mo_close` |
| B2 abort into a not-READY board | dummy packet only if READY within 50 ms (D6) | `t6_abort_no_write_while_not_ready` |
| B3 first jump elided | explicit first jump, long delay, settle (D1) | `t1_dot_test_has_explicit_jump_and_settle` |
| B4 confirm shows Frame values | the ARM dialog shows only what the bytes decode to (I6, R14) | `kittest_confirm_shows_decoded_layer_params`, `summary_rows_and_words_come_from_one_decoded_block` |
| B5 ARM always true / sticky | single-use nonce, token bound to fingerprint and bytes, TTY gate, `arm` linked only by human surfaces | `arm_token_single_use`, `arm_token_invalid_after_edit`, `cli_arm_refused_without_tty`, `xtask deps` rule 4 |
| B6 hidden + Output burns unframed | refused; hidden emitting layers are still framed (R6) | `safety_hidden_output_refused`, `prop_frame_covers_burn` |
| B7 speed < 3.61 mm/s → 0 units | refused (R3) | `safety_speed_zero_units_refused`, `prop_speed_units_stable_over_field` |
| B8 orphan objects dropped | refused (R7) | `safety_orphan_object_refused` |
| B9 3-coordinate points crash | refused at import (I4) | `import_malformed_points_refused` |
| B10 field bound = galvo reach | rated work area (R1) | `safety_outside_work_area_refused` (100.5 mm at scale 1.1834) |
| B11 MOPA defaults leak into G-code | per-machine-kind defaults (I7) | `gcode_diode_job_uses_gantry_defaults` (§6.6 mutation-checks B1–B10) |

## Sign-off

- [x] **D1–D11 (§3) approved by Luiz Bueno, 2026-10-26**, in his words "as the proposed protocol changes for
      hardware validation … not for declaring the implementation hardware-safe".
      - Oracle commits: v0 `6aed8ffa8be5a938d195bf35198cfd168bb07f4e`, v1 `5f7aa635ad4bf9ab0bd6592c4e9754909d6dd4bf`.
      - rust-v1 candidate commit: `7128a6f21cee83beec99fa0eade89cfd2b7cfc8e` (the last commit touching
        `tests/golden/rust-v1-candidate/`). Frozen into `tests/golden/rust-v1/` with every packet, hex word and sha256
        unchanged; only the first header line of each `lmc/*.txt` and `meta.tier` of each `session/*.json` changed.
        Pinned by `crates/lmc/tests/rust_v1.rs` and `crates/driver/tests/rust_v1_session.rs` (byte for byte) and by
        `cargo xtask golden-verify` (sha256 and size in `tests/golden/manifest.json`; an unlisted file fails).
      - Conditions attached to the approval: hardware access stays manual (Luiz at the machine, `openlaser-hw` behind
        the TTY ARM; no AI agent may build the `hw` feature, open the board or activate the laser), and D7/D9 were
        verified before any physical test (below).
- [ ] §4–§6 (refusals, import/profile/confirmation, bug-fix map) are not part of this signature. None of them changes
      a byte in the frozen scenarios; they stay documented behaviour, reviewed with the HIL results.

### D7 and D9: verification before physical testing (2026-10-26)

**D7, bounded stop latency.** Every wait of the driver thread is a ≤ 20 ms slice that checks the stop latch
(`MAX_SLICE`; the real USB wire's `poll_single`/`poll_packet` cap each wait at 20 ms and `submit_*` never block).
From the stop request:
- **StopExecute** is written after ≤ 20 ms to see the stop + ≤ 20 ms in-flight grace. On the real wire add
  ≤ 2 × 20 ms `CANCEL_COLLECT` when the packet had to be cancelled. So the worst case is ≤ 80 ms; it is ≤ 40 ms in
  the simulator.
- **MO(0)** follows StopExecute after ≤ 60 ms (the 50 ms abort transfer cap, polled in 20 ms slices), even when the
  board does not answer.
- **The whole sequence** is ≤ 40 ms + the 400 ms `EMERGENCY_BUDGET`.

Pinned by:
- `stop_at_every_wire_event_leaves_mo_closed_and_ports_zero`: StopExecute ≤ 40 ms after a stop at every wire event of
  a job; the whole stop < 100 ms on a responsive board.
- `stop_execute_and_mo_close_are_bounded_on_the_worst_path`: a stalled in-flight packet plus a board that goes mute.
- `a_wedged_board_still_gets_mo_close_and_ports_zero_within_the_watchdog`.
- T6 `t6_stop_p99_under_100ms_all_states` (real time, p99 ≤ 100 ms, worst ≤ 250 ms).

The 250 ms acceptance in HARDWARE.md is measured on the bench, not assumed.

**D9, no premature execution and no buffer overrun.**
- `ExecuteList` is sent once per list. It follows the prelude's `Fiber_SetMo(1)`, every packet written so far having
  completed its transfer, and a `SetEndOfList` with no other write in between. So the board is never told to run a
  partial or unterminated list. Pinned by `execute_list_only_follows_a_completed_and_terminated_packet` over buffer
  depths 1/2/3/8, slow packets and 1- and 2-packet lists.
- No byte is written unless the most recent status read showed READY (`writes_while_not_ready == 0` in every session
  and T6 test).
- The run-ahead window caps what the board holds at 3 packets and 2 s
  (`run_ahead_window_caps_packets_and_seconds`, `t6_run_ahead_cap_packets_and_seconds`).

**D9, underrun.** Software cannot rule out an underrun. If the host is descheduled longer than the board's queue
(≤ 2 s, often only a few hundred ms), the board runs dry mid-list:
- `a_host_stall_that_starves_the_board_is_survived_or_stops_the_board` shows the host side is safe. A 1 s stall is
  survived with no write while not READY, no second `ExecuteList` and MO(0) only after BUSY cleared. A 5 s stall
  trips the watchdog, and the emergency stop follows.
- What the real board does while dry (holds with the laser off and resumes on the next packet, as the simulator
  assumes) is **unverified**: `HARDWARE.md` item H-UR, a red-light-only bench test.
