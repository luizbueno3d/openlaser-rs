# Intentional divergences: shipping Rust vs the Python oracle

**Status: DRAFT, unsigned.** Written by WP0 from SPEC §5.6 (revision 2) and `tests/golden/README.md`
("Intentional divergences"). The integrator completes it at I1 with the rust-v1 candidate listings
(`tests/golden/rust-v1-candidate/`). Luiz signs it at D13 (PLAN D13, SPEC S15); only then are the `rust-v1` goldens
frozen as `tests/golden/rust-v1/`. Until then every line below is a proposal he may veto.

`py-compat` (test-only, SPEC §5.5) reproduces oracle **v0** (`6aed8ff`) exactly. Shipping packets are compared with
oracle **v1** (`5f7aa63`) minus the lines below (`crates/lmc/tests/t1_shipping_vs_v1.rs`).

## Byte- and sequence-level (visible in packets or the wire stream)

| # | Divergence | v0 / v1 behaviour | Where it shows | Bug / source |
|---|---|---|---|---|
| D1 | Every list starts its motion with an explicit `JumpTo` using the **long** jump delay (200) and a `DelayTime(100)` settle (1 ms) before the first `MarkTo` or light-on | v0 elides the first jump to the assumed start; v1 jumps with the short delay (8) and no settle | `dot_test`; every list's first jump | B3, PLAN §3.3 |
| D2 | Every list's first-jump `dist` is computed from (0x8000, 0x8000), so packet bytes never depend on session state | each OM layer-pass ran on a fresh GP controller, so v1's packets already match | every list (`mark_box10` 1957, `dot_test` 0) | SPEC §5.2 rule 6, rev 2 |
| D3 | Lighting lists set `JumpSpeed(light)` before their first (dark) jump | GP leaves the board's previous jump speed | `frame_box10`, `bc_frame` | SPEC §5.2, rev 2 |
| D4 | One session per connection: init (`GetSerialNo` … `EnableZ`, incl. `Reset`) once per connection; no per-job abort, close or USB reset | per job (v0, v1) | session streams | PLAN §3.3 |
| D5 | MO close and port-off only after BUSY clears; abort only on stop or fault | v0: 3 s wait cap, MO closed at +3 s, StopExecute at +6 s | T6 (`t6_busy_20s_completes_without_mo_close`) | B1 |
| D6 | The abort's dummy end-of-list packet goes out only if READY within 50 ms; its `ExecuteList` only if no list is executing (OM v1 `_safe_abort`); always MO(0) and **WritePort(0)** at the end | v0 sent it unconditionally and left the red-light bit set (`WritePort(0x0100)`) | `frame_box10`, `bc_frame` final `WritePort`; T6 | B2, rev 2 |
| D7 | An in-flight packet is completed before a stop (`cancel_all` only on overrun, flagged `possible_desync`) | — | T6 (`t6_inflight_packet_completed_before_stop_execute`) | PLAN §3.5 |
| D8 | Frame Loop sends one complete list per pass and counts a pass only after the board reports !BUSY | GP's FrameLoop appended to one endless list (never captured) | T6, rust-v1 session streams | S20, rev 2 |
| D9 | `ExecuteList` may come before the 3rd packet when the run-ahead window fills or READY drops; packet bytes unchanged, only the single-command order differs | GP waits for the 3rd packet | session streams | SPEC §5.3, rev 2 |
| D10 | Zero delays keep the `0x8000` sign word (`delay_word(0) = (0, 0x8000)`); the parameter cache resets at every list start, so every list carries the full parameter block | same as GP (kept on purpose until H9) | every list | GOLD item 9 |

## Refusals instead of silent behaviour (no byte change in the captured scenarios)

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

## Import, profile and confirmation

| # | Divergence | Python behaviour | Bug / source |
|---|---|---|---|
| I1 | SVG imported at true scale; unitless SVG refused unless 96 or 72 px per inch is chosen | OM's SVG import (T4 SVG rows are informational) | SPEC §3.6, GOLD item 11 |
| I2 | No STL import | OM imports STL (T4 STL rows are informational) | GOLD item 11 |
| I3 | DXF SPLINE evaluated (the shipping importer); `dxf_om::parse_dxf_om` keeps OM's control/fit points for T4 | control/fit points | SPEC §3.6 |
| I4 | Points that are not (x, y) refused | v0 crashed | B9 |
| I5 | Mirror flags live in the machine profile (`cal.negate`, human-only) | browser localStorage | Appendix B |
| I6 | The repeat count and each layer's decoded values are shown at confirmation; the ARM token binds the bytes (`bytes_hash`) | the confirm showed Frame values | B4, S22 |
| I7 | G-code uses per-machine-kind layer defaults | MOPA defaults leaked into G-code (OM v1 did not fix it) | B11 |

## Sign-off

- [ ] Luiz has read every row, vetoed or accepted each (SPEC §9 item 11 for D2, D3, D6, D8, D9), and signed:
      name, date, oracle commits (v0 `6aed8ff`, v1 `5f7aa63`), rust-v1 candidate commit.
