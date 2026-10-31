# Golden-oracle fixtures

These files are the bytes and numbers the Rust OpenLaser must reproduce. They were produced by running
the Python prototype (**OM**, `~/AI/OpenLaser/OpenMopa`) and its driver library **galvoplotter 0.2.0** on a
recording fake board. OM is the behaviour proven on the real JPT M7 MOPA / JCZ LMC galvo. No USB device was
opened to make them, and none can be (see [USB isolation](#usb-isolation)).

`AGENTS.md` §1.4 applies: the LMC encoder must match these streams byte for byte, except for the intentional
divergences listed [below](#intentional-divergences-rust-vs-the-python-oracle).

## Oracles

| Oracle | OM commit | Branch | Status |
| --- | --- | --- | --- |
| **v0** | `6aed8ffa8be5a938d195bf35198cfd168bb07f4e` | `main` | The bytes proven on the laser (research 05 "oracle-v0"). `py-compat` must reproduce these exactly. |
| **v1** | `5f7aa635ad4bf9ab0bd6592c4e9754909d6dd4bf` | `fix/safety-b1-b12` | The patched prototype (B1–B10, B12). A **candidate**: it becomes "oracle-v1" only after the HIL-P hardware session passes (PLAN §11). Packets are compared against v1. |

Both oracles use galvoplotter 0.2.0, pyusb 1.3.1 and CPython 3.10.0, recorded in every fixture's `meta`.
The machine profile is OM's `profiles/openmopa-machine.ini` (FIELDSIZE 200, 1–4000 kHz, JPT M7 pulse table).
The calibration is `scale_correction = 1.1834`, so the **effective field F = 200 × 1.1834 = 236.68 mm**
(f64 `0x1.d95c28f5c28f6p+7`).

T0, T2, T3 and T4 came out byte-identical for v0 and v1, because those modules did not change. They are stored
once with `"oracle": "v0+v1"`. T1 is stored per oracle.

## Layout

```
tests/golden/
  README.md                  this file
  manifest.json              sha256 + size of every fixture file, the inputs, the OM SHAs, totals
  generator/                 the generator (Python, runs in OM's venv) and decode_t1.py
  inputs/                    inputs the generator reads (copied, with sha256s in inputs/inputs.json)
    workspace.json  layer_settings.json  calibration.json  openmopa-machine.ini  dxf/{square,businessCard}.dxf
  t0_numeric/                T0: 7 encoder tables
  t1_lmc/
    v0/<scenario>.json       T1: ordered USB event streams, oracle v0
    v1/<scenario>.json       T1: the same scenarios, oracle v1
    packets/<sha256>.hex     every distinct 3072-byte list packet (shared by v0 and v1)
    inputs/<scenario>.json   large scenario inputs (polylines in mm), shared by v0 and v1
    v0_vs_v1.json            machine-made v0 -> v1 diff of every scenario
  t2_gcode/
    index.json               every (machine, scenario) case: inputs, result or refusal, sha256
    <machine_id>/<scenario>.gcode
    number_format.json       100,019 rows of gcode._number(v)
  t3_hatch/hatch.json        T3: hatch_polylines cases
  t4_import/                 T4: import_geometry / parse_dxf cases (businessCard_dxf.json holds the big one)
```

## Common format

Every JSON fixture starts with a `meta` object:

```json
{"schema": "openlaser-golden/1", "generator": "tests/golden/generator", "generator_version": "1.0.0",
 "galvoplotter_version": "0.2.0", "pyusb_version": "1.3.1", "python_version": "3.10.0",
 "tier": "T1", "oracle": "v0", "om_git_sha": {"v0": "6aed8ffa8be5a938d195bf35198cfd168bb07f4e"}}
```

The `.gcode` and `.hex` files carry no header. Their metadata lives in `t2_gcode/index.json` and `manifest.json`.

- **Floats** are written with Python's shortest round-trip `repr`. Parse them with `serde_json`'s
  `float_roundtrip` feature (or `str::parse::<f64>`). Tables with float inputs carry `inputs_f64_sha256`: the
  sha256 of the little-endian IEEE-754 bit patterns of the float inputs, in row order. A Rust test should
  recompute it to prove it parsed every input exactly. Non-finite inputs never appear as JSON numbers; the few
  that matter are strings (`"inf"`, `"-inf"`, `"nan"`) in a `specials` list.
- **Python errors** are recorded, not hidden. A row the oracle refused goes in `errors`, and a refusal case
  carries `{"error": {"type", "message"}}`. Rust compares the *decision* and the error *category* (its
  messages may differ).
- Nothing is timestamped. Regenerating from the same commits gives the same bytes (`generate.py --check`).

## Tiers

### T0: numeric encoders (`t0_numeric/`), must match exactly

| File | Function (oracle source) | Columns | Rows |
| --- | --- | --- | --- |
| `mm_to_galvo.json` | OM `live.mm_to_galvo`, at F = 200 and F = 236.68 | x_mm, y_mm, gx, gy | 49,698 (incl. 9,798 exact `.5` ties, ±F/2 edges, nextafter neighbours, clamp region) |
| `power_to_mark_current.json` | GP `_convert_power` → `listMarkCurrent` p1 | power %, word | 20,018 (+3 error rows) |
| `frequency_to_period.json` | GP `_convert_frequency(base=20000)` → `listQSwitchPeriod` p1 | kHz, word | 33,052 (incl. 18,047 exact ties, the `& 0xFFFF` wrap below 0.3052 kHz, negatives) |
| `speed_to_units.json` | GP `_convert_speed` → `listMarkSpeed`/`listJumpSpeed` p1 | mm/s, word | 22,932: grids at both fields, exact unit boundaries, plus a 10k random sweep of F in [100, 300] |
| `delay_word.json` | GP `list_laser_on/off/polygon/jump_delay` (checked identical per row) | delay, p1, p2 | 4,015 (+3 error rows) |
| `nearest_pulse_width.json` | OM `cli.nearest_pulse_width` + `safety.evaluate_emission` decision | ns, nearest, exact/snap/refuse | 14,184 + 3 specials |
| `distance_word.json` | GP `list_jump`/`list_mark` p4 (checked identical) | last_x, last_y, x, y, dist | 11,155 |

Each table states its `formula` (the Python operation order) and a `rust_rule`. The rules that bite:
- Python `round()` is **ties-to-even**, so use `f64::round_ties_even`. For example, 30 % encodes to 1228 and
  70 % to 2866. `int()` truncates.
- Keep the operation order: `0x8000 + v * (0xFFFF / F)`, and `(v * gpm) / 1000.0`.
- A delay of 0 (or -0.0) encodes as `(0, 0x8000)`, while 0 < d < 1 encodes as `(0, 0x0000)`.
- `nearest_pulse_width` keeps the **first** (lower) entry on ties. Python maps NaN to 2 ns *and* treats it as a
  valid snap (see Findings).

### T1: LMC USB streams (`t1_lmc/`)

Each scenario ran OM's own entry point (`live.mark_polylines`, `live.frame_polylines`,
`connect_controller`/`stop_controller`) exactly as the app calls it, in a fresh process, with galvoplotter's
connection replaced by a recording board. Every status read on that board answers `(0, 0, 0, 0x0020)`:
READY, never BUSY. So every wait polls exactly once, and the stream does not depend on timing.

**Events**, in wire order:

| `kind` | Fields | Meaning |
| --- | --- | --- |
| `open` | | connect: the device is opened (then `init_laser`) |
| `single` | `op`, `hex` (24 hex digits) | one 12-byte single command, exactly as written |
| `read` | `status` | the 8-byte reply galvoplotter read after it, status word b3 |
| `list` | `sha256`, `n_cmds` | one 3072-byte list packet. Bytes: `packets/<sha256>.hex`. `n_cmds` = slots before the trailing `0x8002` fill (an explicit final `listEndOfList` counts as fill) |
| `close` | | disconnect (galvoplotter does a USB **reset** here) |

**`packets/<sha256>.hex`** has 256 lines of 24 hex digits, one 12-byte slot per line. Strip whitespace and
hex-decode to get the 3072 bytes; their sha256 is the file name. All 105 distinct packets are stored in full.

Each scenario file also holds:
- `inputs`: the polylines in mm (inline, or `polylines_file` + `polylines_compact_sha256`) and the parameters;
- `result`: OM's return value;
- `counts`;
- `segments`: index ranges `[start, end)` for `init`, `job` and `abort`;
- for v1, `informational.packet_estimates_s`: OM v1's per-packet time estimates, input to the B1 wait budget.

| Scenario | What runs | v0 events / packets | v1 vs v0 |
| --- | --- | --- | --- |
| `init_abort` | connect + `init_laser`, then `stop_controller` (abort + close); no job | 57 / 1 | identical |
| `init`, `abort` | slices of `init_abort`: the 19-command init, and StopExecute → close | 39 / 18 | identical |
| `mark_box10` | 10 mm box, 50 %, 30 kHz, 200 ns, 1000 mm/s | 80 / 2 | identical (packet `9ca1e12d…59f5`) |
| `frame_box10` | Frame Once of the same box: 150 repeats, red light | 86 / 4 | final `WritePort` 0x0100 → 0x0000 |
| `dot_test` | built-in Test Dot: 0.05 mm at the centre, 1 % | 80 / 2 | v1 adds `JumpDelay(8)` + `JumpTo(32768,32768,0,0)` (B3) |
| `line_test` | built-in Test Line: 5 mm through the centre, 1 % | 80 / 2 | identical |
| `long_job` | 1100 separate 0.5 mm segments: 9 job packets, ExecuteList after the 3rd | 120 / 10 | identical |
| `bc_mark_vector-engrave_pass1` | business card, Vector Engrave layer: 304 paths, 1750 points, 100 %, 30 kHz, 1000 mm/s | 105 / 7 | identical |
| `bc_mark_raster_pass1` | business card, Raster layer: 969 hatch segments (32 closed shapes, 67 open skipped), 100 %, 45 kHz, 300 mm/s, 90° | 115 / 9 | identical |
| `bc_frame` | business card Frame Once: 403 outlines, 3057 points, 6 repeats | 441 / 75 | final `WritePort` 0x0100 → 0x0000 |

The business card is `inputs/workspace.json` + `inputs/layer_settings.json` at F = 236.68, put through OM's
own `ui.emission_steps` / `layers.framing_jobs`, one hardware job per layer pass, as `/api/mark` does it.
Mirror flags are not part of the workspace (they live in browser localStorage) and were off.

Every scenario ends with the abort, whose 3072-byte dummy end-of-list packet is `e479558e…6a18` (256 × `0x8002`).

**How Rust uses T1.**
1. **`py-compat` (test-only feature)** replays galvoplotter's sequencing: one controller per job, per-job init,
   parameter cache, elided first jump, per-job abort and close. On a fake transport that answers READY, it must
   produce the **v0 event list exactly**: the same kinds, `hex`, `status`, and packet sha256s.
2. **The shipping encoder** must produce the same **packet contents** as v1 for every list, apart from the
   divergences below. The comparator decodes both list-command streams and allows only the listed differences
   (`v0_vs_v1.json` shows the style).
3. The shipping sequence (one session, explicit first jump with settle, no per-job abort) is reviewed once and
   frozen separately as `rust-v1` goldens in `rust-v1/` (signed 2026-10-26 for controlled hardware validation, D13).

### T2: G-code (`t2_gcode/`), byte-exact after masking

`index.json` lists 44 cases: the 4 built-in machines × 11 scenarios. Each case gives the result's `file`, the
`sha256` of the masked text, its counts, and `estimated_distance_mm`, or the refusal. The scenarios are:
`vector`, `multipass`, `air_assist`, `raster_crosshatch`, `plotter_drawing`, `mixed_layers` (skip rules),
`speed_power_edges` (speed capping, S ties), `business_card`, and three refusals (`refuse_out_of_bed`,
`refuse_power`, `refuse_nothing_to_output`). Their layers and objects are inlined under `scenarios`; the
business card references `inputs/`. `jpt-m7-200` refuses every scenario ("does not use G-code output").

Byte rules:
- **Masking.** Line 3 `; Generated: <UTC ISO-8601>` is replaced by `; Generated: <MASKED>`. Rust must replace
  its own line 3 the same way before comparing.
- Lines are joined with `\n` and the file ends with `\n`. It is UTF-8; `·`, `×` and `₂` appear in comments.
- The doubled `M5` between paths is kept.

`number_format.json` holds 100,019 rows of `_number(v) = f"{v:.3f}"` with trailing zeros and the dot stripped
(`-0` becomes `0`). It includes exact binary ties such as 0.0625, which give `0.062`. Check Rust's `{:.3}`
against every row, and use a ties-to-even exact formatter if any row differs.

### T3: hatch (`t3_hatch/hatch.json`)

There are 46 cases:
- OM's `tests/test_raster.py` fixtures;
- closure-tolerance edges (0.04 mm closed, 0.06 mm open);
- refusals (pitch ≤ 0) and `passes = 0`;
- 30 seeded random stars, some with holes;
- the business-card raster layer with the saved settings (969 segments; 32 closed, 67 open skipped) and as a
  0°/90° cross-hatch (1637 segments).

The rule: **segment count and order exact; coordinates within 1e-9 mm**. Keep Python's operation order:
`y += pitch` accumulation, the half-open `[lo, hi)` crossing test, the `x1 - x0 > 1e-6` pair filter, and the
boustrophedon flip.

### T4: importers (`t4_import/`)

There are 30 cases:
- OM's six in-test DXF snippets;
- 7 extra DXF cases run through both `parse_dxf` and `import_geometry`: CIRCLE, ARC, SPLINE, ELLIPSE, bulges,
  `$INSUNITS` cm/m/ft/unknown, CRLF and blank lines;
- `inputs/dxf/square.dxf` (4 polylines, 8 points);
- `inputs/dxf/businessCard.dxf` (403 polylines, 3057 points, in `businessCard_dxf.json`);
- two refusals.

DXF rule: polyline and point counts exact, coordinates within 1e-9. The SVG and STL rows are marked
`rust_must_match: false`. They are informational only: the plan imports SVG at true scale (OM normalises it to
60 mm) and drops STL.

## Regenerating

There is no hardware involved. Run this from the repo root with OM's venv:

```sh
~/AI/OpenLaser/OpenMopa/.venv/bin/python tests/golden/generator/generate.py           # rewrite the fixtures
~/AI/OpenLaser/OpenMopa/.venv/bin/python tests/golden/generator/generate.py --check   # regenerate in a temp dir, compare, write nothing
python3 tests/golden/generator/decode_t1.py tests/golden/t1_lmc/v0/mark_box10.json     # read a stream
```

Options:
- `--v0-rev` (default `6aed8ff`) and `--v1-rev` (default `HEAD`) pick the OM commits;
- `--om-repo` and `--dxf-dir` set the source paths;
- `--refresh-inputs` re-copies `inputs/` from the OM working copy;
- `--tiers t0,t1,...` limits the run.

What the generator does:
- **OM is only read.** Each oracle commit is exported with `git archive` into a temporary directory, so
  uncommitted OM edits never leak in. The workers drop the venv's editable-install finder and assert that
  `mopa_luiz` was imported from the exported tree.
- **Each worker** is a fresh process started with `-s -B` and a sanitised environment:
  - `HOME` and `TMPDIR` point into the temp directory;
  - `OPENMOPA_MOCK=1`, so OM never takes the board lock;
  - `PYTHONHASHSEED=0` and `LC_ALL=C`.
- **Each T1 scenario runs twice per oracle**, and the two runs must be byte-identical.
- **T0/T2/T3/T4 must be identical for v0 and v1.** If they ever differ, the generator stores both copies under
  `<tier>/v0/` and `<tier>/v1/`, and says so in `meta.identical_across_oracles`.
- If you change what a fixture means, bump `GENERATOR_VERSION` in `generator/common.py`.
- To add a T1 scenario, add it to `worker.py:t1_scenarios` (OM entry points only) and regenerate.

### USB isolation

`generator/usb_guard.py` is imported before anything else, in the orchestrator and in every worker. It does
three things:
1. **An audit hook** refuses every `ctypes.dlopen` of a real library, so libusb can never be loaded into the
   process. pyusb reaches libusb-1.0, libusb-0.1 and OpenUSB only through ctypes.
2. **Patches.** It replaces `usb.core.find`, `usb.core.show_devices`, every pyusb backend's `get_backend`,
   `libusb_package`'s finders, and galvoplotter's `USBConnection` (in both `galvo.usb_connection` and
   `galvo.controller`) with functions that raise.
3. **Self-test and final check.**
   - At start-up, each process proves that 6 probes raise, including `usb.core.find(0x9588, 0x9899)` and
     `ctypes.CDLL("libusb-1.0.dylib")`.
   - At exit, it asserts that no USB library was loaded and that nothing outside the self-test tried.

The last run checked 44 worker processes plus the orchestrator: 0 libraries loaded, 0 attempts. The T1 board is
attached with `self.connection = board`, so galvoplotter never constructs a `USBConnection` or a
`MockConnection`. Never run galvoplotter's own `site-packages/test/*.py`: those open real USB.

## Intentional divergences: Rust vs the Python oracle

`py-compat` reproduces v0 exactly. The shipping Rust driver applies the changes below on top. The source is
PLAN §3.3, §3.6, §6.1 and §11; each change gets a line in `DIVERGENCES.md` when that file is written.

**Already in OM v1** (visible in the v1 streams or in v1 behaviour):

| Fix | Change | Where it shows |
| --- | --- | --- |
| B3 | Explicit first `JumpTo` for every list (GP used to elide a jump to the assumed start point `0x8000,0x8000`) | `dot_test` only; other lists keep their bytes |
| B2 | Safe abort: StopExecute → MO(0) → ResetList; the dummy packet only if READY within 50 ms; always MO(0) and **WritePort(0)** at the end (v0 left the red-light bit set: `WritePort(0x0100)` after a frame) | `frame_box10`, `bc_frame` final `WritePort`; a never-READY board is T6 |
| B1 | Progress-aware waits; never proceed on a timeout (v0: 3 s cap, MO closed at +3 s, StopExecute at +6 s) | not visible on a READY, never-BUSY board; T6 |
| B7 | Mark speed that encodes to 0 units is refused (below 3.6115 mm/s at F = 236.68) | T0 `speed_to_units` shows the 0 words; refusal is T5 |
| B10 | Work area = min(effective F, rated FIELDSIZE) = ±100 mm, not the galvo reach ±118.34 mm | refusal (T5) |
| B4, B5, B6, B8, B9, B12 | Confirmation shows each layer's values; single-use ARM nonce; hidden + Output refused; orphan objects refused; points that are not (x, y) refused; board lock | planner/UI, no byte change in these scenarios |

**Rust only** (the `rust-v1` goldens, frozen 2026-10-26 in `rust-v1/`):
1. **First jump:** an explicit `JumpTo` with the **long** jump delay (200, GP's `delay_jump_long`), then
   `DelayTime(100)` (1 ms settle) before the first `MarkTo` or light-on. v1 uses the short delay 8 and no settle.
   The `dist` word is computed as GP does.
2. **One session per connection:**
   - init (`GetSerialNo` … `EnableZ`, including `Reset`) runs once per connection, not once per job or pass;
   - no per-job abort, close or USB reset.
3. **End of job:** MO close and port-off only after BUSY clears. Abort only on stop or fault.
4. **Abort:** the dummy end-of-list packet goes out only if READY (as v1). An in-flight packet is completed
   before a stop.
5. **+F/2:** refused, where v0 and v1 clamp it to 65535 (T0 shows the clamp); work area ±100 mm (as v1).
6. **Speed:** encoded once to whole units; refuse 0 units (as v1).
7. **Refusals instead of silent behaviour:**
   - hidden + Output layers and orphan objects (as v1);
   - an empty Fill layer (OM `raster_skip`);
   - **gantry overspeed** (`gcode.py` caps it). The T2 cases `speed_power_edges` and the gantry
     `business_card` show the capped Python output: `py-compat` matches them, shipping refuses.
   - NaN and infinite parameters (Python snaps a NaN pulse width to 2 ns).
8. Open shapes in a Fill layer become a numbered notice instead of being skipped silently. The bytes are the
   same.
9. **Kept for now:** zero delays keep the `0x8000` sign word (H9). The parameter cache resets at every list
   start, so every list carries the full parameter block, as GP does.
10. **G-code:** per-machine-kind layer defaults (B11, M2). OM v1 did **not** apply B11 (export is still live), so
    `gcode.py` is identical in v0 and v1.
11. **Importers:** SVG at true scale, unitless SVG refused or asked about; no STL.
12. **Not encoded in any golden:** mirror flags checked against the profile; the repeat count shown at confirm.

## Not covered here (yet)

- **T5 safety decisions** (`evaluate_emission`, `validate_layer_settings`, `geometry_errors`, `/api/mark`
  refusal paths).
- **T6 timing:** BUSY boards, a full buffer, stalled writes, stop latency. These are written fresh in Rust.
- **Never captured:**
  - FrameLoop (continuous framing never ends);
  - a stop that lands mid-stream (timing-dependent);
  - pause/resume;
  - `.cor` upload (OM never loads one);
  - GP's CO2 encodings.
- **Hardware unknowns H1–H20** (research 05 §7). These goldens prove we match the Python oracle, not that the
  oracle is right on every point. Examples: raw-ns pulse width (H6), delay units (H9), port bit 0 (H11).

## Findings recorded by these goldens

- **v0 leaves the red-light output bit set after Frame Once.** The end-of-list `WritePort` and the abort's final
  `WritePort` are both `0x0100`, then the USB reset. v1 ends with `WritePort(0x0000)`.
- **Python's pulse-width check accepts NaN as a snap to 2 ns.** `abs(2 - nan) > 0.5` is false, so it raises a
  warning, not an error. Python's `json` accepts a `NaN` literal, so `/api/*` could receive one. Rust must refuse
  it.
- **Delays:** 0 encodes as `(0, 0x8000)`, while 0.4 encodes as `(0, 0x0000)`.
- **Frequency:** below 0.3052 kHz the period wraps with `& 0xFFFF` (0.3 kHz → 1131), and a negative frequency
  encodes as 45536. The profile bounds stop both in OM, and Rust must refuse them in the encoder too.
- **Speed:** the first speed that encodes to 1 unit is 3.0518 mm/s at F = 200 and 3.6115 mm/s at F = 236.68 (1000 × F / 65535).
