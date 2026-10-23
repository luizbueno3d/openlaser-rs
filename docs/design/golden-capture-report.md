# Golden capture report

Date: 2026-10-23. Scope: the golden-oracle fixtures that the Rust LMC encoder, G-code writer, hatcher and DXF
importer must match. They were generated from the Python prototype (OM) with no hardware attached and no USB
device opened. For formats and usage, see [`tests/golden/README.md`](../../tests/golden/README.md).

## Result

| Tier | Directory | Files | Bytes | Content |
| --- | --- | --- | --- | --- |
| T0 numeric | `tests/golden/t0_numeric/` | 7 | 5,682,900 | 155,054 table rows (+9 error rows, 3 specials) |
| T1 LMC streams | `tests/golden/t1_lmc/` | 132 | 1,159,144 | 2 × 9 scenarios + 2 × 2 slices, 105 distinct 3072-byte packets, 4 shared input files, v0-vs-v1 diff |
| T2 G-code | `tests/golden/t2_gcode/` | 26 | 3,363,150 | 44 cases (24 `.gcode` files, 20 refusals) + 100,019 number-format rows |
| T3 hatch | `tests/golden/t3_hatch/` | 1 | 956,679 | 46 cases |
| T4 import | `tests/golden/t4_import/` | 2 | 296,183 | 30 cases |
| **Fixtures** | | **168** | **11,458,056** | listed with sha256 in `tests/golden/manifest.json` |
| Inputs | `tests/golden/inputs/` | 7 | 987,006 (+ `inputs.json`) | workspace, layer settings, calibration, profile, `square.dxf`, `businessCard.dxf` |

Everything under `tests/golden/` is about 12.4 MB on disk, under the 30 MB budget. Nothing was committed.

## Oracles: two, not one

The brief assumed that OM `HEAD` is the hardware-proven prototype. It no longer is.
- OM's checked-out branch is `fix/safety-b1-b12`, at `5f7aa63`: the B1–B10 and B12 safety patches from PLAN §11.
- `main` is still at `6aed8ff`, the commit the plan calls **oracle-v0** (the bytes proven on the laser).

`py-compat` must reproduce v0 (PLAN §6.1), so the generator captures both:

| Oracle | Commit | Use |
| --- | --- | --- |
| v0 | `6aed8ffa8be5a938d195bf35198cfd168bb07f4e` (`main`) | `py-compat` byte-for-byte target |
| v1 | `5f7aa635ad4bf9ab0bd6592c4e9754909d6dd4bf` (`HEAD`, `fix/safety-b1-b12`) | packet-content target; a *candidate* until HIL-P passes |

Each commit is exported with `git archive` into a temp directory, so the OM repo is only read. Its working copy
stayed clean, and no `~/Library/Application Support/OpenLaser/board.lock` was created. Every fixture records its
OM SHA(s), the generator version (1.0.0), galvoplotter 0.2.0, pyusb 1.3.1 and Python 3.10.0.

## T1 scenarios

Every scenario was run on F = 236.68 mm (200 × 1.1834) with OM's profile. The fake board answers READY and is
never BUSY. Counts are identical for v0 and v1 except where noted.

| Scenario | Events | Singles | List packets | List commands | v1 vs v0 |
| --- | --- | --- | --- | --- | --- |
| `init_abort` | 57 | 27 | 1 | 0 | identical |
| `mark_box10` | 80 | 38 | 2 | 17 | identical |
| `frame_box10` (150 repeats) | 86 | 40 | 4 | 607 | last `WritePort` 0x0100 → 0x0000 |
| `dot_test` | 80 | 38 | 2 | 12 (v1: 14) | v1 adds `JumpDelay(8)`, `JumpTo(32768,32768,0,0)` |
| `line_test` | 80 | 38 | 2 | 14 | identical |
| `long_job` (1100 segments) | 120 | 54 | 10 | 2212 | identical |
| `bc_mark_vector-engrave_pass1` | 105 | 48 | 7 | 1522 | identical |
| `bc_mark_raster_pass1` (969 hatch segments) | 115 | 52 | 9 | 1950 | identical |
| `bc_frame` (403 paths, 6 repeats) | 441 | 182 | 75 | 18808 | last `WritePort` 0x0100 → 0x0000 |

`init.json` (39 events) and `abort.json` (18 events) are slices of `init_abort`. `v0_vs_v1.json` is generated
from the decoded streams. It shows that the v1 patches change only:
- the B3 first jump: `dot_test`, the one scenario that starts at the field centre;
- the B2 final `WritePort`: the two frame scenarios.

That matches PLAN §11: "`dot_test` contains `0x8001`; `mark_box10` SHA unchanged".

## Verification

- **USB isolation.** `generator/usb_guard.py` runs first in every process. It installs a CPython audit hook that
  refuses any `ctypes.dlopen` of a real library, so libusb cannot load. It also replaces
  `usb.core.find`/`show_devices`, every pyusb backend's `get_backend`, `libusb_package`'s finders and
  galvoplotter's `USBConnection` with raisers.
  - Each process self-tests 6 probes, among them `usb.core.find(idVendor=0x9588, idProduct=0x9899)` and
    `ctypes.CDLL("libusb-1.0.dylib")`. Each must raise.
  - At exit, each process asserts that no USB library was loaded and nothing outside the self-test tried.
  - Result: 44 worker processes plus the orchestrator all passed. 0 libraries were loaded and 0 attempts were
    blocked outside the self-test.
  - The T1 board is attached directly as `controller.connection`, so galvoplotter never builds a
    `USBConnection` or a `MockConnection`.
  - Workers run with `HOME`/`TMPDIR` in a temp directory and `OPENMOPA_MOCK=1` (no board lock).
- **Determinism.**
  - Every T1 scenario ran twice per oracle, in separate processes, with byte-identical output.
  - Two later full `generate.py --check` runs regenerated all 168 files byte-identically.
  - T0/T2/T3/T4 came out byte-identical from v0 and v1, so they are stored once.
- **Against the research 05 capture.** The scratch harness's `mark_box10`, `frame_box10`, `dot_test` and
  `long_job` captures decode to the same list commands and the same non-poll single commands as the new v0
  fixtures. (The old `long_job` ran a 20 s BUSY board, hence 555 versus 54 singles, all of them extra
  `GetVersion` polls.) The reference values all hold:
  - `9ca1e12d…59f5` (mark list) and `e479558e…6a18` (abort dummy packet);
  - `dot_test` with no `JumpTo` and `listMarkTo(32782,32768,0,14,0)`;
  - `long_job` with 9 job packets;
  - `mm_to_galvo` (5,5) → 34152, (25,−25) → (39690,25846) at F = 236.68;
  - power 30 % → 1228, 70 % → 2866;
  - 30 kHz → 667;
  - 1000 mm/s → 276 and 3000 mm/s → 830 at F = 236.68;
  - business-card raster: 969 segments, 32 closed, 67 open skipped.
- **OM's own cross-checks inside the generator.**
  - `cli.power_to_mark_current` and `cli.frequency_to_period` equal galvoplotter's wire words on every
    in-range row.
  - The jump and mark speed encodings are identical.
  - The four delay opcodes encode identically.
  - JumpTo and MarkTo share the same distance word.
  - Any mismatch aborts generation.

## Findings

1. **OM `HEAD` is not v0.** See [Oracles](#oracles-two-not-one). Any tool that assumes "OM HEAD = the
   hardware-proven bytes" is now wrong.
2. **v0 leaves the red-light output bit on after Frame Once.**
   - Both the end-of-list `WritePort` and the abort's final `WritePort` are `0x0100` (bit 8, light), followed by
     the USB reset.
   - Research 05 §1.9 says the abort's `WritePort(0x0000)` turns the red-light bit off. That is true after a
     mark, not after a frame.
   - v1's safe abort fixes it.
   - Whether the board keeps the pointer lit after the USB reset is a hardware question (H11).
3. **NaN pulse width passes OM's check.**
   - `evaluate_emission` treats NaN as "snapped to 2 ns" (a warning), because `abs(2 - nan) > 0.5` is false.
   - Python's `json` accepts a `NaN` literal, so an `/api/mark` payload could carry one.
   - It is low risk (the browser UI never sends NaN), but Rust must refuse it.
4. **B11 is not in v1.** G-code export is still live in OM, and `gcode.py` is identical in v0 and v1. The T2
   goldens therefore represent both. Gantry overspeed is capped silently: `speed_power_edges` and the gantry
   `business_card` show `F30000`/`F15000`/`F9000` caps. Shipping Rust refuses those, so those cases are
   `py-compat`-only.
5. **Encoder quirks pinned by T0.**
   - A delay of 0 encodes as `(0, 0x8000)`, but 0.4 encodes as `(0, 0x0000)`.
   - Frequency wraps with `& 0xFFFF` below 0.3052 kHz, and −1 kHz encodes as 45536.
   - Power above about 1600.38 % or below about −0.0122 % raises `struct.error`; −0.01 % still encodes as 0.
   - The first speed that encodes to 1 unit is 3.0518 mm/s at F = 200 and 3.6115 mm/s at F = 236.68.

## Choices made while generating

- **Busy-board and stop-mid-job streams were not captured.** They depend on timing. T6 (PLAN §6.1) writes them
  fresh in Rust against a scripted board. v1's per-packet time estimates are kept as
  `informational.packet_estimates_s`, to seed the B1 budget tests.
- **No orphan object in T2.** `mixed_layers` has none, because v0 drops an orphan object and v1 refuses it (B8),
  and T2 should stay oracle-independent. Orphan handling belongs in T5.
- **Extra scenarios:**
  - `line_test` (the built-in Test Line);
  - T2 `plotter_drawing`, `mixed_layers`, `speed_power_edges` and three refusal scenarios;
  - SVG/STL importer rows, marked `rust_must_match: false`.
- **Large T1 inputs** (long job, business-card layers, frame outlines) are stored once in `t1_lmc/inputs/`,
  with a sha256 of the compact JSON. Every scenario is self-contained for a Rust test.
- **Inputs are copied** into `tests/golden/inputs/`, with sha256s:
  - `workspace.json`, `layer_settings.json` and `calibration.json` are untracked in OM;
  - the DXFs live in `~/Documents/Playground`.
- **Byte-exact files are protected.** `tests/golden/.gitattributes` sets `-text` on fixtures, so git never
  rewrites line endings in `.gcode`, `.hex` or `.dxf`.

## Files

Generator (Python; run with `~/AI/OpenLaser/OpenMopa/.venv/bin/python`):

| File | Role |
| --- | --- |
| `tests/golden/generator/generate.py` | Orchestrator: export the oracles, run the workers, check determinism, merge, write `manifest.json`. `--check` verifies and writes nothing |
| `tests/golden/generator/worker.py` | Runs inside one exported OM tree; one tier, or one T1 scenario, per process |
| `tests/golden/generator/usb_guard.py` | USB isolation (run it alone to see the self-test) |
| `tests/golden/generator/fake_board.py` | Deterministic recording LMC board and virtual clock |
| `tests/golden/generator/common.py` | Deterministic JSON, the hex packet format, metadata constants |
| `tests/golden/generator/decode_t1.py` | Pretty-prints a T1 stream (stdlib only) |

## Next steps for the Rust side

1. **`ol-units`/T0 tests:** load each table and assert every row; verify `inputs_f64_sha256` after parsing.
2. **T1 `py-compat`:** replay each `t1_lmc/v0/*.json` scenario on a recording fake transport that answers
   READY, and compare the event lists exactly. Then compare shipping packets against v1 with a
   divergence-aware command-stream comparator.
3. **Freeze `rust-v1`.** After review, freeze the shipping sequence as `rust-v1` goldens (with the explicit
   first jump, the long jump delay and settle, and one session).
4. **Re-run when OM moves.** Re-run `generate.py` when HIL-P passes and v1 becomes the tagged oracle-v1, or
   whenever OM changes. `--check` will flag any drift.
