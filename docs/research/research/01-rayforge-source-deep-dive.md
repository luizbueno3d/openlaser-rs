# 01 — Rayforge source-level deep dive (input for OpenLaser planning)

Date: 2026-10-22 · Author: research subagent for Luiz · Status: research only. Nothing was installed or run, and no code from the cloned repos was executed. No USB or serial hardware was touched.

## 0. Method and provenance

| Repo | Commit read | Date | Where it was cloned |
|---|---|---|---|
| `barebaric/rayforge` (main) | `a120f7235c3e` ("Merge PR #623 fix/wcs-marker-and-jog-limits") | 2026-10-23 03:56 +0200 | `scratchpad/rayforge-src` |
| `barebaric/rayforge` branch `feat/galvo-driver` (PR #288) | `e85c5e64e70e` | 2026-06-14 | fetched into the same clone (read with `git show`) |
| `barebaric/raygeo` (Rust/PyO3 core used by Rayforge) | `eab6fa8cfc8f` | 2026-10-23 01:34 +0200 | `scratchpad/raygeo-src` |
| `barebaric/raydriver` (Rust/PyO3 GRBL driver used by Rayforge) | `0b7b9a184db6` | 2026-10-17 | `scratchpad/raydriver-src` |

`scratchpad` = `/private/tmp/claude-501/-Users-luizbueno--claude-spares-spare-HuLL3w/ba10990c-59a4-470b-ab69-9ee5e8b41d86/scratchpad`.

Paths without a prefix are relative to the Rayforge repo root. A `raygeo:` or `raydriver:` prefix means that repo. GitHub data (issues, PRs, releases, contributors, branches) came from `gh api` on 2026-10-22. Published docs came from `website/docs/**` in the repo, which is the source of https://rayforge.org, plus spot checks of the live site. Anything I could not confirm is marked **UNVERIFIED**.

---

## TL;DR (what matters for OpenLaser)

1. **Rayforge is already turning into "a Python/GTK shell around a Rust core".**
   - Geometry, the Ops IR, toolpath assembly, every post-processor (optimize/TSP, tabs, overscan, multipass, lead-in/out, clip, smooth), the pipeline runtime (rayon) and the G-code encoder now live in **raygeo**. raygeo has about 106k lines of Rust: about 64k of core and about 42k of PyO3 bindings. It is MIT-licensed.
   - The GRBL protocol stack lives in **raydriver** (about 6k lines of Rust: tokio plus tokio-serial, MIT).
   - Rayforge itself is about 266k lines of Python, 77k of them GTK UI.
   - The Rust work happened between May and Oct 2026 (CHANGELOG.md lines 1366, 1092-1099 and 211-217).
2. **Rayforge has no galvo, BJJCZ, LMC or EZCAD support in main. The one attempt is unusable.**
   - Issue #261 "Galvo support" and issue #308 "Support for fiber lasers?" were both closed by the stale bot as *not planned*.
   - The maintainer says he has never owned a galvo.
   - PR #288 "Add Galvo/EzCad2 driver support" is a "semi vibe coded" driver of about 2,979 lines that was never hardware-tested. The stale bot closed it unmerged on 2026-07-23. The branch `feat/galvo-driver` still exists.
   - Reading PR #288 shows it would not work. It never flushes the list buffer, so nothing is sent. It silently drops commands past 256. It clamps out-of-field points. Its mm-to-galvo scale is off by 2×. It reports progress before sending. It is also bit-rotted against today's APIs (§2.6).
   - The LightBurn device importer explicitly maps `"EZCAD"` and `"Galvo"` to `None` (`rayforge/machine/device/lightburn_importer.py:26-35`).
3. **The driver contract is G-code/gantry-shaped, but it has a working escape hatch for binary protocols.**
   - A driver can set `uses_gcode = False`. Its own `OpsEncoder` then runs as a `PythonEncoder` on a raygeo rayon thread (`rayforge/pipeline/intent_builder.py:1254-1279`). The Ruida driver uses this route.
   - Gaps for a galvo:
     - No centered-field origin.
     - No galvo delays, wobble, lens or field correction.
     - No USB-bulk transport.
     - No galvo time model.
     - Out-of-bounds checks are advisory: the user can press "Proceed".
     - The documented addon hook for registering drivers (`register_machines`) is **dead code**: it is never invoked.
     - Encoder binary payloads (`driver_data`) are **dropped** by the pipeline.
4. **Fork: no. Contribution: possible, as a side track. Rust app: yes, and borrow selectively.**
   - Rayforge is effectively a one-person project: knipknap made 4,447 of about 4,780 commits (≈93%). It shipped 101 releases in about 15 months, recently a beta every 2-5 days. raygeo shipped 123 releases in 5 months.
   - A fork would rot in weeks.
   - The healthiest options:
     - **(a)** Build OpenLaser in Rust as planned, using Rayforge, raygeo and raydriver as an MIT reference implementation and a source of vendorable code.
     - **(b) Optional:** contribute an LMC/galvo addon upstream, where Luiz's real hardware is the scarce resource the maintainer said he lacks.
5. **macOS packaging:**
   - PyInstaller `.app` (`Rayforge.spec`), **ad-hoc signed only and not notarized**. Users are told to run `xattr -dr com.apple.quarantine`.
   - Release zips/DMGs are 154 MB (arm64 zip) to 325 MB (universal zip). The Linux AppImage is 452 MB.
   - The macOS build pins different dependencies from the dev environment: Python 3.12, NumPy 1.26.4 and SciPy 1.11.4 for Monterey compatibility.

---

## 1. Architecture and module map

### 1.1 Top-level layout (Python lines, excluding tests)

| Module | LOC | Role |
|---|---:|---|
| `rayforge/ui_gtk/` | 77,279 | GTK4/libadwaita UI. 2D canvas is `ui_gtk/canvas2d` (cairo). 3D sim is `ui_gtk/sim3d` (PyOpenGL shaders). Machine settings, wizards and dialogs. |
| `rayforge/builtin_addons/` | 51,940 | 10 built-in addons. Biggest: sketcher 39.5k, laser 5.2k, post 2.2k, print-and-cut 1.3k, cnc 1.3k. |
| `rayforge/machine/` | 27,778 | `driver/` (drivers), `transport/` (I/O), `models/` (Machine, LaserHead, dialects, rotary, zones), `device/` (profile packages, LightBurn `.lbdev` import), `discovery/` (serial and mDNS), `sanity/` (pre-flight checks), `cmd.py` (job send, frame, hold, cancel), `job_monitor.py`. |
| `rayforge/core/` | 15,704 | Document model: `doc.py`, `layer.py`, `workpiece.py`, `step.py`, `stock.py`, `material*.py`, `recipe*.py`, `hooks.py` (pluggy specs), `undo/`, `varset/` (declarative settings → auto UI), `ai/`. |
| `rayforge/image/` | 12,620 | Importers, renderers and exporters per format. Also `dither.py` and `tracing.py`. |
| `rayforge/doceditor/` | 9,011 | `DocEditor` controller and undoable command objects (`*_cmd.py`). |
| `rayforge/pipeline/` | 6,420 | Thin Python orchestration over the raygeo intent pipeline: `pipeline.py`, `intent_builder.py`, `intent_controller.py`, `artifact/`, `encoder/`, `transformer/`, `view/`. |
| `rayforge/shared/` | 6,027 | `tasker/` (asyncio thread plus an optional multiprocessing pool), units, G-code editor widget, OAuth, markdown. |
| `rayforge/camera/` | 5,774 | OpenCV capture, lens calibration and homography alignment. |
| `rayforge/addon_mgr/` | 2,452 | Addon discovery, install via a git registry, license gating. |
| `rayforge/simulator/` | 2,085 | `op_player.py`, `machine_state.py`, `scene3d/` (calls the Rust `compile_scene_3d`). |
| `rayforge/license/` | 839 | Gumroad and Patreon license validation for paid addons. |

Totals: 1,165 `.py` files and 266,481 lines in `rayforge/`, plus 375 test files and 110,690 lines in `tests/`.

### 1.2 The Rust layer (separate repos, owned by the same maintainer)

**raygeo**: `raygeo:Cargo.toml`, `raygeo:src/lib.rs:1-80`, `raygeo:AGENTS.md`.
- 393 `.rs` files, 105,801 lines.
- Lines by layer: `geo` 21.3k, `ops` 31.3k, `cnc` 2.8k, `pipeline` 1.3k, `image` 2.6k, `mesh` 1.3k, `svg` 2.0k, `python` (PyO3 bindings) 41.6k.
- Layering is strict: `geo → ops → cnc`, with `pipeline` as a generic rayon runtime.
- Dependencies: clipper2, earcut, rstar, spade, nalgebra, glam, rayon, swash and fontdb (text), svgtypes and roxmltree (SVG), zstd, mimalloc.
- `crate-type = ["cdylib","lib"]`. The `python` feature is the default but optional.
- **No crates.io release.** `crate raygeo does not exist` on crates.io. Cargo.toml has `version = "0.0.0"`, and the real version is stamped from git tags. It ships only as PyPI wheels (latest 1.67.1).

**raydriver**: `raydriver:README.md`, `raydriver:Cargo.toml`.
- 6,208 lines of Rust: tokio, tokio-serial, async-trait.
- A full port of Rayforge's GRBL serial stack: character-counting flow control, stall detection, deadlock recovery, cancel and safety-shutdown, settings, WCS and probing.
- Includes a Python **GRBL 1.1h firmware emulator** for tests (`raydriver:python/raydriver/emulator.py`).
- Not on crates.io. PyPI 0.5.1. MIT.

The Rayforge repo itself has **no Rust source**. The `.rustfmt.toml` (`max_width = 80`) and `.gitignore` "`# Rust ops crate / target/`" are leftovers from when raygeo lived in-tree ("Migrate raygeo from local source to PyPI package", CHANGELOG.md:1227). See §6.

### 1.3 Processes and threads

- **GTK main thread.** UI only.
- **One asyncio event loop on a daemon thread.** `TaskManager.__init__` (`rayforge/shared/tasker/manager.py:55-58`). All driver I/O coroutines run here. Hold and cancel are scheduled through `task_manager.add_coroutine(...)` (`rayforge/machine/cmd.py:509-523`).
- **raygeo rayon worker threads.** Pipeline compute, aggregation, the machine transform and encoding (`rayforge/pipeline/pipeline.py:97`, `RaygeoPipeline(budget_bytes=...)`, with a default cache budget of 2 GiB at `pipeline.py:67`).
- **`run_thread` helper threads.** Intent rebuild, view rendering and 3D scene prep (`intent_controller.py:447`, `view/view_manager.py:653`, `ui_gtk/sim3d/scene_presenter.py:606,1028,1143`).
- **Multiprocessing pool.** Still present (`shared/tasker/pool.py`) and created lazily on the first `run_process` call (`manager.py:62`). I found **no remaining `run_process` callers**, so it is vestigial since 1.9.0 replaced the multiprocessing DAG: "Replaced multiprocessing pipeline with raygeo intent orchestration…" (CHANGELOG.md:1095-1098).
  - The docs still describe a 3D "subprocess" (`website/docs/developer/pipeline.md` diagram). The code compiles the scene in-process (`rayforge/simulator/scene3d/scene_compiler_runner.py:20-27`).
- **Snap only.** An `LD_PRELOAD` C shim (`snap/semwraplib.c`) renames POSIX semaphore and shared-memory names to satisfy strict confinement (§6).

### 1.4 From design to machine output

```
Doc (layers → Workflow(steps) + WorkPieces; stock; machine)
  │  IntentBuilder.build()                         rayforge/pipeline/intent_builder.py
  ▼  flat list of raygeo NodeRequest with stable keys + SHA-1 version tokens
  ├─ "workpiece:{wp}:{step}"  StageSpec.Compute  (Part = vector geometry or image,
  │                            assembler spec, per-workpiece transformer specs)
  ├─ "step:{step}"            StageSpec.Aggregate (concat + per-step transformers,
  │                            MachineParams{feed, rapid, acceleration})  :1160-1166
  ├─ "job"                    Aggregate with JobStart/LayerStart/…/JobEnd markers
  ├─ "job:machinexform"       MachineTransformSpec: linearize curves if !supports_curves,
  │                            world→machine 4×4, WCS offsets, rotary mapping, Z-flip  :1304-1359
  └─ "job:encode"             EncodeSpec: native Rust GcodeSpec (GRBL dialect) OR
                               PythonEncoder(driver.create_encoder(machine).encode)    :1238-1302,1417-1455
  ▼  raygeo Pipeline.run_intent on rayon; cache keyed by node key + version_token
ArtifactStore (refcounted): WorkPieceArtifact, StepOpsArtifact, JobArtifact(ops, text, op maps)
  ▼  Pipeline rebuilds EncodedOutput(text, MachineCodeOpMap) from the job handle  pipeline.py:458-467
MachineCmd._execute_monitored_job → driver.run(encoded, doc, ops, on_command_done)   machine/cmd.py:89-210
```

- **Version tokens** make incremental rebuilds cheap. A pure move of a workpiece does not invalidate its compute node unless the step is position-sensitive (`intent_builder.py:1-34`, docstring).
- **G-code is encoded in Rust.** `GcodeEncoder.encode()` just calls `ops.to_gcode(dialect_spec, context)` (`rayforge/pipeline/encoder/gcode.py`). The dialect templates and context are converted to plain dicts in `rayforge/pipeline/encoder/rust_helpers.py`.
- **Binary payloads are lost on the pipeline path.** `EncodedOutput` has a `driver_data` dict (`pipeline/encoder/base.py:129-142`), but the PythonEncoder bridge returns only text and op maps (`intent_builder.py:1440-1453`). The job is rebuilt from text only (`pipeline.py:458-467`). Any binary-protocol driver must therefore:
  - emit a *text transcript* that it re-compiles at `run()` time (the Ruida driver does this: "The encoded text IS the GlueScript transcript", `rpa_adapter.py:1144-1158`), or
  - re-encode from the `ops` argument passed to `run()`.

### 1.5 The Ops IR (raygeo)

- `Ops { commands: Arc<Vec<OpNode>>, last_move_to, time_cache }` (`raygeo:src/ops/container/mod.rs:31-35`).
- `OpNode { category, state: Option<Box<State>>, extra_axes: Option<Arc<Vec<(Axis,f64)>>> }` (`raygeo:src/ops/types.rs`). Each node can carry a full state snapshot plus rotary/extra axes.
- **Moving** commands: `MoveTo`, `LineTo`, `ArcTo{center,cw}`, `BezierTo`, `QuadraticBezierTo`, `ScanLine{power_values: Arc<Vec<u8>>}`. A raster row is one move with a per-pixel u8 power array.
- **State** commands: `SetPower(f64)`, `SetFeedRate(i32 mm/min)`, `SetRapidRate(i32)`, `Dwell`, `SetHead`, **`SetFrequency(i32)`**, **`SetPulseWidth(f64)`**, `SetSpindleRpm`, `SetCoolant`, `SetAirAssist`, `SetHeadCoolant`, `SetPowerMode(Dynamic=M4 | Constant=M3)`, `Custom(str)` (`raygeo:src/ops/enums.rs:9-67`, `raygeo:src/ops/state.rs`).
- **Markers**: `JobStart/End`, `LayerStart/End(uid)`, `WorkpieceStart/End(uid)`, `OpsSectionStart/End{VectorOutline|RasterFill, raster_mode}`, `StateBlockStart/End`.
- `RasterMode` = `VariablePower | ConstantPower | DepthMap | Dither`.
- **Op ↔ machine-code map.** `MachineCodeOpMap` stores int32 spans (op → line range) and the reverse map (line → op) as zero-copy numpy views (`pipeline/encoder/base.py:9-127`). It drives progress highlighting and the G-code view. The docstring reports 1.6 M ops at 13 MB instead of 196 MB.
- **Time estimate.** `Ops::estimate_time(feed, rapid, acceleration)` (`raygeo:src/ops/container/time.rs`) uses a gantry trapezoid with feed, rapid and acceleration. The same estimate is added to "machine hours" after a job (`machine/cmd.py:203-208`); hours are estimated, not measured.

---

## 2. The driver and transport API

### 2.1 What a driver must implement (`rayforge/machine/driver/driver.py:260-916`)

**Class flags** (`driver.py:279-307`):
- `label`, `subtitle`
- `supports_settings`, `reports_granular_progress`
- `uses_gcode` (default True)
- `maturity`: STABLE, UNTESTED, EXPERIMENTAL or KNOWN_BUGGY, with a UI warning text
- `supports_probing`
- `native_overscan`: skip Rayforge's OverscanTransformer
- `supports_unit_detection`, `supports_multi_depth_raster`
- `DISCOVERY`: serial or mDNS recognizers
- `z_homing_modes`

**Abstract members.** Every driver must provide all of these, even when the concept does not exist on its hardware:
- `machine_space_wcs` and `machine_space_wcs_display_name`
- `precheck` and `_setup_implementation`
- `get_setup_vars` (a VarSet that auto-generates the connection form)
- `create_encoder(machine) -> OpsEncoder`
- `get_setting_vars`
- `_connect_implementation` (must auto-reconnect)
- `run(encoded, doc, ops, on_command_done)` and `run_raw(str)`
- `set_hold(bool)` and `cancel(emergency=False)`
- `home(axes)` and `move_to(x, y, z=None, speed=None)`
- `select_tool`, `read_settings`, `write_setting`, `clear_alarm`
- `set_power(head, pct)` and `set_focus_power(head, pct)`
- `jog(speed, **deltas)`
- `set_wcs_offset` and `read_wcs_offsets`
- `run_probe_cycle(axis, max_travel, feed)`

**Optional overrides:**
- `frame(corners, speed, doc, repeat_count, corner_pause_s, power_fraction, on_command_done)`. The default builds a well-formed frame `Ops` and runs it like a job (`driver.py:675-721`, `build_frame_ops` at `:215-257`).
- `can_home`, `can_jog`, `supported_wcs`
- `resource_uri`: used to stop two machines opening one port (`connect()`, `:616-637`)
- `supports_pwm` and `get_pwm_params`
- `detect_unit_system`, `update_settings`, `refresh_dialect`
- `read_parser_state`, `select_wcs`

**Signals** (blinker): `state_changed`, `command_status_changed`, `connection_status_changed`, `settings_read`, `job_finished`, `probe_status_changed`, `wcs_updated`, `config_changed`.

**State model:**
- `DeviceState{status, error, machine_pos, work_pos, wco, feed_rate, spindle_speed, buffer_available, buffer_rx_available}`.
- `DeviceStatus` is a copy of GRBL's states: IDLE, RUN, HOLD, JOG, ALARM, DOOR, CHECK, HOME, SLEEP, … (`driver.py:102-169`).
- `DeviceError{code, title, description}`.

**Docs drift.** The published driver guide (https://rayforge.org/docs/developer/driver, source `website/docs/developer/driver.md`) still shows `run(machine_code, op_map, doc, …)` and Ops methods `set_cut_speed`, `set_travel_speed`, `enable_air_assist`. The code uses `run(encoded, doc, ops, …)`, and raygeo's methods are `set_feed_rate`, `set_rapid_rate` and `set_air_assist(mode)` (`raygeo:python/raygeo/ops/__init__.pyi:685-773`).

### 2.2 Job streaming, progress, status, alarms

- **Send path.** `MachineCmd._execute_monitored_job` (`machine/cmd.py:89-210`):
  - Estimates time and creates a `JobMonitor`.
  - If `reports_granular_progress` is set, it passes `on_command_done=monitor.update_progress`.
  - Then awaits `driver.run(...)` on the asyncio thread.
- **GRBL serial** (`grbl/grbl_serial.py`, class at :68, run at :395) is a thin shell over `raydriver.grbl.GrblSession`:
  - Character-counting RX buffer accounting.
  - Status polling every 0.5 s (`raydriver:src/grbl/session.rs:103,126`, `?` written at :513).
  - Stall timeout = estimate × 3, clamped to 5-120 s (`session.rs:90-119,1386-1391`).
  - Optional deadlock recovery with `G4 P0.01` (`:1656-1690`).
  - Alarms and errors are parsed into `DeviceError` (`raydriver:src/grbl/errors.rs`).
- **Ruida.** No granular progress: `reports_granular_progress = False`. `run()` fires `on_command_done` for **every op before sending** (`ruidarpa/rpa_adapter.py:1131-1136`), then blocks in `backend.run_job` on an executor. Progress is simulated.

### 2.3 Jogging, framing, pause/resume/stop

**Jog.** `jog(speed_mm_min, **{axis: mm})`. GRBL uses `$J=` templates from the dialect. Ruida converts mm/min to mm/s; a TODO notes a unit mismatch (`rpa_adapter.py:1313-1316`).

**Frame.** The default traces the bounding-box corners `frame_repeat_count` times. The default repeat count is 20 (`machine/models/laser.py:115`). Power is `frame_power_percent`, default 0, and is forced to M3 constant mode (`driver.py:240-245`). Ruida overrides it with beam-off absolute moves (`rpa_adapter.py:1191-1216`). No driver has a continuous "frame until stopped" mode or red-light pointer framing.

**Pause/resume.**
- GRBL sends realtime `!` and `~` (`raydriver:src/grbl/session.rs:1839-1880`).
- Ruida calls `backend.pause()` / `resume()` through `run_in_executor` (`rpa_adapter.py:1218-1232`).

**Stop.**
- GRBL `cancel()` (`raydriver:src/grbl/session.rs:1757-1800`) runs these steps in order:
  1. Sets the cancelled flag.
  2. Wakes parked senders.
  3. Sends **Ctrl-X soft reset**.
  4. **Hard-aborts the streaming task before resetting flow control**, "so a sender parked waiting for buffer space can never wake up later and resume a job that was cancelled long before (issue #428)".
  5. Drains queued commands.
  6. After a delay, sends the dialect's safety-off commands, plus the emergency command if `emergency`.
- This is the **same failure class we hit on the LMC board**: a spooler blocked on a full buffer delayed the stop. Rayforge fixed it the same way, by aborting the waiting sender.
- Ruida `cancel()` calls `backend.stop_job` through an executor (`rpa_adapter.py:1234-1238`).
- The UI dispatches cancel as an asyncio task (`machine/cmd.py:518-523`).
- **Actual stop latency: UNVERIFIED.** The repo has no measurements and I ran nothing.

### 2.4 Built-in drivers (`rayforge/machine/driver/__init__.py:18-38`)

| Driver class (UI label) | File | Wire / transport | Maturity flag | Notes |
|---|---|---|---|---|
| `GrblSerialDriver` ("GRBL (Serial)") | `grbl/grbl_serial.py` | USB serial via **Rust raydriver** (tokio-serial) | STABLE (default) | Settings, probing, unit detection, serial discovery. Became the default in 1.13.0-beta1 (CHANGELOG.md:211-217). Alias `GrblSerialNextDriver` (`__init__.py:42-45`). |
| `GrblTelnetDriver` ("GRBL (Telnet)") | `grbl/grbl_telnet.py` | raw TCP via raydriver | STABLE | Subclass of serial. For grblHAL and ESP3D. |
| `GrblSerialSimpleDriver` ("GRBL (Serial Simple)") | `grbl/grbl_serial_simple.py` | Python `SerialTransport` ping-pong | EXPERIMENTAL | Fallback with no character counting. |
| `GrblNetworkDriver` ("GRBL (Network)") | `grbl/grbl_network.py` | `HttpTransport` file upload + `WebSocketTransport` | STABLE | No granular progress. |
| `MarlinSerialDriver` | `marlin/marlin_serial.py` | `SerialTransport` | EXPERIMENTAL | |
| `SmoothieDriver` | `smoothie/smoothie_driver.py` | `TelnetTransport` | STABLE | |
| `OctoPrintDriver` | `octoprint/octoprint_driver.py` | HTTP API | UNTESTED | |
| `RuidaRPAAdapter` ("Ruida") | `ruidarpa/rpa_adapter.py` (+ `rpa_encoder.py`, `rpa_direct_driver.py`) | USB, UDP or TCP via the external **ruida-pa** library (PyPI 0.22.0, MIT, by StevenIsaacs), or RPyC to a TUI | **KNOWN_BUGGY** (`rpa_adapter.py:217`) | `uses_gcode=False`, `native_overscan=True`, no multi-depth raster. See §2.5. |
| `NoDeviceDriver` ("No driver") | `dummy.py` | none | — | Default fallback for unknown class names (`get_driver_cls`, `__init__.py:48-49`). |

**Transports** (`rayforge/machine/transport/__init__.py`):
- `Transport` ABC: `connect`, `disconnect`, `send(bytes)`, `purge`, plus `received` and `status_changed` signals (`transport.py:31-83`).
- Implementations: `SerialTransport`, `GrblSerialTransport`, `TelnetTransport`, `UdpTransport`, `UdpServerTransport`, `HttpTransport`, `WebSocketTransport`, and `SerialServerTransport` (non-Windows only).
- Scanners: `serial_scan.py`, `mdns_scan.py`.
- **There is no USB-bulk (libusb) transport, and discovery only knows serial and mDNS** (`machine/discovery/`).

### 2.5 Is "ruidarpa" a real Ruida driver?

**Yes, but it is immature.**
- It is a genuine non-G-code driver. The encoder drives the ruida-pa GlueScript API (layer attribute blocks, job framing and bounding-box math). The driver compiles the script to Ruida's binary protocol at run time (`ruidarpa/rpa_encoder.py:1-10`, `rpa_adapter.py:1144-1172`).
- It was merged via PR #404 (closing issue #14 on 2026-09-13) and is maintained by an outside contributor, StevenIsaacs, under its own rules (`rayforge/machine/driver/ruidarpa/AGENTS.md`).
- Its README (`ruidarpa/README.md`, "Limitations") says:
  - It was tested only on an RDC6442S in a Monport MP570 CO₂ on Fedora.
  - Rotary is not supported.
  - Depth-map engrave is untested.
  - Acceleration over-burn compensation is rudimentary.
  - "many yet to be characterized Ruida commands".
- The AGENTS file mentions an older `driver/ruida` prototype that has since been removed.
- It is the best **template for a list/binary-protocol driver** in Rayforge (layer parameter blocks, native overscan, text transcript as the cached artifact).

### 2.6 Any galvo, BJJCZ, LMC or EZCAD support? (code, issues, PRs, branches, discussions)

**Code on main.** A case-insensitive whole-word search for galvo|bjjcz|ezcad|lmc|jcz|balor|galvoplotter|mopa|qswitch across rayforge, raygeo and raydriver found only:
- `lightburn_importer.py:30,33`, which map `"EZCAD": None` and `"Galvo": None`
- the matching tests (`tests/machine/device/test_lbdev_importer.py:112,247`).

The live device catalogue (https://rayforge.org/resources/devices) lists no galvo, fiber, EZCAD, JCZ or LMC device. Its 35 profile packages are in `rayforge/resources/devices/`.

**GitHub search** (`gh api search/issues`, terms galvo, ezcad, bjjcz, fiber, jcz, lmc, mopa, balor, galvoplotter):
- **#261 "Galvo support"** (issue, by BoboTiG, AtomStack M4, 2026-05-27). The maintainer replied: "Having never owned a galvo laser myself I am not sure what would be needed… someone who has such a device would need to step in." After a user linked meerk40t/galvoplotter, he wrote that he had "semi vibe coded" a driver in PR 288. The reporter noted the M4 actually uses BSL, not EzCad2. Closed by the stale bot 2026-07-22 (*not_planned*).
- **#308 "Support for fiber lasers?"** (issue, 2026-07-13). The maintainer: "Galvo is not yet supported at all (see #261)… such machines require controlling parameters that Grbl based machines just don't have." Closed by the stale bot 2026-08-21.
- **#288 "Add Galvo/EzCad2 driver support"** (PR by knipknap, 2026-06-14). +2,979 / −1 lines, 21 files, one commit, **unmerged**, no reviews, closed by the stale bot 2026-07-23. Branch `feat/galvo-driver` @ `e85c5e64` still exists. Files: `rayforge/machine/driver/galvo/{galvo_consts,galvo_protocol,galvo_controller,galvo_encoder,galvo_driver,galvo_usb_connection,galvo_mock_connection}.py` plus 106 mock-only tests. USB uses pyusb with VID:PID 0x9588:0x9899 (JCZ/LMC) and 0x1A86:0x5512 (CH341 clones).
- **#108** (K40 board M2, unrelated) and **#423** (pointer-laser offset PR, mentions fiber) are not galvo work.
- **Discussions:** GraphQL discussion search for galvo, fiber and ezcad returned 0. Discord was not searched (not accessible).

**What reading PR #288 shows.** Line numbers are on `feat/galvo-driver`.
- **It sends nothing.** `GalvoDriver.run()` writes every 12-byte command with `_list_write` (`galvo_driver.py:284`) but never calls `_end_list()` or executes the list. The packet is only flushed in `enter_idle_mode()`, which `run()` never calls. It then emits `job_finished` immediately (`:286`).
- **It silently truncates.** `_list_write` returns without a word once the 3,072-byte (256-command) list is full (`galvo_controller.py:258`). This is the exact anti-pattern our prototype forbids.
- **It clamps out-of-field coordinates** instead of refusing them (`galvo_encoder.py:96`, `galvo_controller.py:322-345`, `galvo_driver.py:320-321`).
- **The scale is wrong.** `(mm/200)*32768 + 32768` maps ±200 mm onto the 16-bit range, which is 163.84 units/mm. A 200 mm field needs 327.68, so output is half size. With Rayforge's corner-origin machine coordinates (0-200 mm), the whole job would land in one quadrant and clamp at the far edge. Speed uses a different hard-coded `galvos_per_mm=500`. This is arithmetic from the code; **hardware behavior is UNVERIFIED**.
- **Progress is fake.** `on_command_done` fires for every op before any I/O (`galvo_driver.py:255-263`).
- **The jump delay is broken.** `set_jump_delay` checks and stores `self._delay_on`, which is a copy-paste bug (`galvo_controller.py:400-401`).
- **USB I/O blocks the asyncio thread.** Calls are synchronous pyusb with a 100 ms timeout.
- **It is bit-rotted against main.** It uses `CommandType.SET_CUT_SPEED`, `ENABLE_AIR_ASSIST` and `SET_LASER` (now `SET_FEED_RATE`, `SET_AIR_ASSIST` and `SET_HEAD`), `PWMCapability` (removed; `core/capability.py` now has only `MachineCapability`), a dict-style `MachineCodeOpMap` (now a numpy/bytearray map), `encoded.driver_data["binary"]` (now dropped by the pipeline, §1.4), and `cancel(self)` and `move_to(self, x, y)` without the current signature parameters.
- **What it does get right:** the general shape (consts, list/single command split, `Fiber_SetMo`, port bits for the laser pin) is a usable *checklist* of LMC commands.

**Verdict:** there is no working galvo support anywhere in the Rayforge ecosystem as of 2026-10-22.

---

## 3. Gantry assumptions versus galvo needs, and the cost of an LMC driver in Rayforge

### 3.1 Where Rayforge assumes a gantry / G-code machine

| Area | Rayforge today (evidence) | What an LMC galvo needs | Gap |
|---|---|---|---|
| Coordinate frame | `Origin` = TOP_LEFT, BOTTOM_LEFT, TOP_RIGHT or BOTTOM_RIGHT only (`machine/models/machine.py:58-62`; `coordspace.py:30-36`). Axis extents in mm. | Field centered at (0x8000, 0x8000), symmetric ±field/2, per-lens orientation and flip | No CENTER origin. The encoder would have to offset by field/2. |
| Units and scale | mm, `unit_system`, `gcode_precision` | mm → 16-bit galvo units (65536/field), plus **field scale correction** (ours: ×1.1834 → 236.68 mm effective) and a lens distortion table (`.cor`) | No field-scale or lens-correction concept anywhere |
| WCS / homing / probing / Z | `Driver` makes `home`, `set_wcs_offset`, `read_wcs_offsets`, `run_probe_cycle`, `select_tool` and settings **abstract** (`driver.py:754-916`). G54-G59 WCS per layer. | None of these exist. A Z lift table is optional and not driven by LMC. | A galvo driver must stub about 10 methods. Harmless but noisy. |
| Motion and time model | `MachineParams{default_feed_rate, default_rapid_rate, acceleration}` (`intent_builder.py:1160-1166`). Trapezoidal `estimate_time` (raygeo `time.rs`). | Mark speed, jump speed, and delays (laser on/off, polygon, jump, mark-end). No acceleration model. | Time estimates and machine hours would be wrong for a galvo |
| Overscan / bidir offset | `OverscanTransformer` and `BidirScanOffsetTransformer`. The driver can skip overscan with `native_overscan` (`driver.py:288-290`). | Not needed. Galvo raster uses jump/mark with delays. | OK: set `native_overscan = True` |
| Power semantics | `SetPower` 0..1, scaled to `max_power` (default 1000 for GRBL S). `PowerMode` M3/M4 (`raygeo:src/ops/state.rs`). | Fiber current %, Q-switch **frequency (kHz, up to ~4 MHz on MOPA)**, **MOPA pulse width (ns)** | Partly present: `SetFrequency(i32)` and `SetPulseWidth(f64)` exist. Per-step `frequency` and `pulse_width` are emitted by `LaserStep` (`laser_essentials/steps/laser_step.py:53-54,170-173`). But head defaults are **CO₂-PWM semantics**: `pwm_frequency` 500 Hz (max 5,000), `pulse_width` 50 **µs** (5-500), labeled "Pulse width in microseconds" (`machine/models/laser.py:123-127`; `driver.py:182-203`). MOPA needs ns and kHz ranges. |
| Laser types | `LaserType` = DIODE, CO2, FIBER (`laser.py:9-30`). FIBER is used only for wavelength (1064 nm) and "supports_pwm". | Fiber Q-switched vs MOPA, red-light pointer port, `Fiber_SetMo` | FIBER exists as a label only |
| Raster | `ScanLine{power_values: u8[]}` per row, with modes POWER_MODULATION, CONSTANT_POWER, DITHER and MULTI_PASS (`raster_step.py:60-82`). 13 dither algorithms (`image/dither.py:21-34`). Scan angle and cross-hatch. | Per-pixel power is impractical in LMC lists. Raster becomes threshold or dither mark/jump segments, or bitmap "dot" modes. Hatch fill at angles with multiple passes. | The encoder must split ScanLines into mark runs. DITHER and CONSTANT_POWER modes map well. |
| Hatch / wobble | Offset Fill step and cross-hatch raster. **No wobble** anywhere (`grep wobble|hatch` in rayforge and raygeo). | Hatch sets with angle increments, loop/bidir; wobble (circle/ellipse/figure-8) for thick cuts and welds | Missing (wobble) or partial (hatch) |
| Delays | Not modeled in Ops or State | laser-on, laser-off, polygon, jump and mark-end delays per layer; `SetDelayMode` | Missing. Would need new `StateCmd`s in raygeo (Rust, upstream) or `Custom(str)` hacks or encoder-side per-step settings. |
| Framing | One-shot trace × N repeats, beam power configurable, default 0 (`driver.py:215-257,675-721`; `laser.py:112-116`) | **Red-light** pointer loop until stopped, never the laser | Override `frame()`. Must hard-guarantee the laser is off. |
| Safety / bounds | `ExtentCheck2D` is ERROR and `WorkareaCheck2D` is WARNING (`machine/sanity/checks/*.py`). `SanityCheckDialog` **always offers "Proceed"**, even with errors (`ui_gtk/shared/sanity_check_dialog.py:32-43`). If the check itself fails or has no artifact, the job **silently proceeds** (`ui_gtk/mainwindow.py:2354-2395`). | Our rules: refuse out-of-field (no clamping), refuse >3000 paths / >20000 points (no silent truncation), validate the whole job before the first layer burns, never burn Frame-Only/output-off geometry | Must be enforced in the galvo encoder or driver: raise, so the job fails. Do not rely on the UI. |
| Transport / platform | Serial, TCP, UDP, HTTP, WebSocket only. Snap plugs include `serial-port` but **not `raw-usb`** (`snap/snapcraft.yaml:12-23`). Flatpak has `--device=all`. The PyInstaller bundle has no libusb (`scripts/mac/Brewfile`). | libusb bulk endpoints. macOS needs a bundled libusb. Windows needs WinUSB via Zadig, which takes the board from EZCAD. Linux needs a udev rule. | New transport plus packaging work on all three OSes |
| Driver registration from an addon | Docs show `@hookimpl register_machines(machine_manager)` → `machine_manager.register(...)` (`website/docs/developer/addon-overview.md`). In code the hookspec exists (`core/hooks.py:244-253`), but it is **never invoked**: it is absent from `core/registration.py:21-125`, and the only invoked hooks are `rayforge_init`, `register_material_libraries`, `register_model_libraries`, `main_window_ready`, `on_unload`, the registry hooks and the job hooks. `MachineManager` has no `register()`. A module-level `register_driver(cls)` does exist (`machine/driver/__init__.py:52-54`), and the UI iterates the same mutable `drivers` list (`ui_gtk/machine/general_preferences_page.py:78`). | — | An addon would have to call `register_driver()` from `rayforge_init`. Load order relative to machine loading is **UNVERIFIED**. A machine saved with an unknown driver class silently becomes `NoDeviceDriver` (`__init__.py:48-49`). |
| Addon Python dependencies | The manifest only supports `depends` (Rayforge version) and `requires` (other addons) (`addon-manifest.md:105-120`) | pyusb plus a native libusb | No way to ship pip deps into frozen, snap or flatpak builds. A core change (as PR #288 did in `requirements.txt` and `win_setup.sh`) or vendoring is needed. |

### 3.2 How an LMC driver would plug in (concrete)

**Plug-in points:**
1. **`class LmcGalvoDriver(Driver)`** with:
   - `uses_gcode = False`, `native_overscan = True`, `supports_multi_depth_raster = False`
   - `reports_granular_progress = True`, mapping list-packet acknowledgements to op indices
   - `can_home() → False`; `can_jog()` → red-light move
   - `resource_uri = "usb://0x9588:0x9899/<serial>"`
   - an overridden `frame()` that uses only the red-light port bit
   - an async USB spooler (bulk writes off the event loop), status polling, and abort-on-stop that cancels any wait on list-buffer space (our stop fix)
2. **`class LmcEncoder(OpsEncoder)`** returning `EncodedOutput(text=<deterministic LMC transcript>, op_map=…)`. It runs as a `PythonEncoder` on a rayon thread (`intent_builder.py:1254-1279,1417-1455`). Because `driver_data` is dropped (§1.4), `run()` either re-parses the transcript or re-encodes from `ops`. The transcript route matches Ruida and keeps the G-code view and progress mapping working. The encoder:
   - linearizes `ArcTo` (only Béziers are linearized upstream when `supports_curves=False`; set `supports_arcs=False` on the machine as well)
   - converts `ScanLine`s into mark runs
   - applies field offset, flip, scale correction and lens correction
   - **raises** on any out-of-field point or over-limit path/point counts
3. **A machine and device profile** (`rayforge/resources/devices/<name>/device.yaml`, like `generic-ruida-rpa/device.yaml`): driver class name, `axis_extents` = field, `origin`, heads with `laser_type: fiber`.
4. **Per-step galvo parameters.**
   - Frequency and pulse width already flow through `LaserStep` → `ops.set_frequency/set_pulse_width`.
   - Delays, wobble and hatch sets need either:
     - a new step type in an addon, emitting `Custom("lmc:delays …")` state ops that the encoder interprets (works today but hacky), or
     - new typed `StateCmd`s in raygeo. That is Rust and upstream-only, and since `Ops` is a raygeo type it cannot be extended from Python.
5. **USB transport and packaging:** pyusb plus libusb dylib bundled in `Rayforge.spec` and `scripts/mac/mac_build.sh`, `raw-usb` plug in snap, Windows WinUSB.

**Effort estimate.** This is my judgment, not measured.

- **"Works on Luiz's JPT M7" addon that reuses galvoplotter 0.2.0** (MIT, the same library our prototype drives; dormant since 2023-10-27): about 1.5-3k lines of Python plus tests. About **1-2 weeks** with hardware.
  - It still needs two **upstream** touches: a working driver-registration path, and pyusb/libusb in the bundles. Without them it only runs from a source/pixi checkout, not the signed app.
- **A proper, upstreamable galvo feature set**:
  - delays, wobble and hatch as typed ops (raygeo PR)
  - a CENTER origin
  - a galvo time model (raygeo)
  - hard-refuse semantics
  - lens correction upload
  - an emulator in the style of raydriver for tests

  That is about **4-8 weeks**, and it depends on the maintainer accepting Rust changes in raygeo, where he is effectively the only committer (698 of 720 commits).

The Rayforge code is clean enough that this is *feasible*. The blocker is product focus: Rayforge's center of gravity is GRBL and Ruida gantries plus a growing CNC/drag-knife scope (`rayforge-addon-cnc`, open PR #397).

---

## 4. Feature inventory (verified in code)

| Feature | Present? | Evidence |
|---|---|---|
| **Import formats** | SVG and **SVGZ**, DXF, PDF, AI, EPS/PS, HPGL (`.plt .hpgl .hpg .hgl`), PNG, JPEG, BMP, GIF, TIFF, WebP, **Ruida `.rd`**, **LightBurn projects `.lbrn` / `.lbrn2`** (Rect, Ellipse, Path, Text, Group, Bitmap and CutSettings) | `rayforge/image/*/importer.py` (e.g. `svg/importer.py:42`, `lightburn/importer.py:440-442,288-333,497-520`, `ruida/importer.py:32`, `hpgl/importer.py:36-38`, `postscript/importer.py:117-143`). Open request: CorelDRAW (#575), G-code geometry (#549). |
| **Export** | SVG, DXF (geometry), sketch export. G-code via the encoder. Project `.ryp`. | `image/svg/exporter.py`, `image/dxf/exporter.py`, `builtin_addons/rayforge-addon-sketcher/sketcher/image/exporter.py` |
| **LightBurn device import** | `.lbdev` → `DeviceProfile` (Serial→GRBL, Network→GRBL network, Ruida→Ruida; EZCAD and Galvo → none) | `machine/device/lightburn_importer.py` |
| **Text / fonts** | Rendered in Rust (raygeo `geo/shape/text`, swash + fontdb). Templates; open PRs for QR/barcodes (#556), CSV merge (#555), job counter (#554) | CHANGELOG.md:1175; `raygeo:Cargo.toml` |
| **Image modes and dithering** | Engrave depth modes POWER_MODULATION, CONSTANT_POWER, DITHER, MULTI_PASS. Dither: Floyd-Steinberg, Atkinson, Stucki, Jarvis, Sierra, Sierra-2row, Sierra-lite, Burkes, Bayer 2/4/8, newsprint, halftone. Non-destructive image edits (invert, tone, sharpen). | `image/dither.py:21-34`; `laser_essentials/steps/raster_step.py:60-82`; `builtin_addons/rayforge-addon-image-editing` |
| **Tracing** | vtracer (Rust lib via Python binding) + raygeo binary preprocessing + hull detection | `image/tracing.py:1-41` |
| **Operations (steps)** | Contour (with cut side/kerf offset), Engrave (raster), Offset Fill, Shrink Wrap, Wavefront (adaptive), Frame Outline, Material Test Grid, Command step (automation addon) | `builtin_addons/rayforge-addon-laser/laser_essentials/steps/*.py`; `website/docs/features/operations/` |
| **Post-processors** | Optimize (KD-tree nearest neighbor + 2-opt TSP, `raygeo:src/ops/transform/optimize.rs:1-4`), Tabs (manual and auto), Overscan, Bidirectional scan offset (+ per-speed table), Multipass with step-down, Lead-in/out, Merge lines, Smooth, Crop-to-stock, Perforation | `builtin_addons/rayforge-addon-post/post_processors/transformers/*.py` (thin wrappers over raygeo `ops/transform/*.rs`) |
| **Kerf** | Contour `offset_mm` from head `kerf_mm` and cut side | `laser_essentials/steps/contour_step.py:234-318` |
| **Material library and recipes** | 60 material YAMLs. Recipe auto-match by material, thickness, machine and head with specificity scoring | `builtin_addons/rayforge-addon-materials/` (60 yaml); `core/recipe*.py`; README "Materials & Presets" |
| **Material test** | Power × speed grid generator. Interval/focus tests in open PR #539. | `laser_essentials/steps/material_test.py`; `raygeo ops/assembly/material_test_grid` |
| **Camera** | OpenCV capture (local and network streams), lens calibration (`cv2.calibrateCamera` with ChArUco, ArUco/AprilTag grid and dot-grid targets), fisheye correction, **alignment = homography from ≥4 image↔world point pairs** (`cv2.findHomography`), "outside view" | `camera/calibration/target.py:19-26`, `calibrator.py:233`, `controller.py:304-330,635-664`, `camera/models/camera.py:582-627` |
| **Print & Cut** | Registration-mark alignment wizard (addon) | `builtin_addons/rayforge-addon-print-and-cut` |
| **Rotary** | 4th axis or axis-replacement, per-layer rotary modules, cylinder unrolling in the machine transform | `machine/models/rotary_module.py`; `intent_builder.py:1349-1400` |
| **Sketcher / parametric CAD** | Constraint-based 2D sketcher, 21 constraint types, SciPy `least_squares` solver, booleans, arrays | `builtin_addons/rayforge-addon-sketcher/sketcher/core/solver.py:1-60,284` (39.5k lines) |
| **3D / simulation** | Animated 3D toolpath playback (PyOpenGL; scene compiled in Rust `compile_scene_3d`). Material "fold"/burn simulation by wavelength absorption. | `simulator/`, `ui_gtk/sim3d/`, `raygeo:src/ops/material/burn.rs`, CHANGELOG.md:1099 |
| **Stock, no-go zones, WCS** | Stock items with material. No-go zones checked pre-flight. G54-G59 per layer. | `core/stock.py`, `machine/models/zone.py`, `machine/sanity/checks/nogo_zones_2d.py` |
| **Nesting** | Only as the **paid** external addon "Deepnest" (BSL-1.1, Gumroad) | `rayforge-registry/registry.yaml`; `website/docs/addons/deepnest.md` |
| **Smart Stock (CV stock detection)** | **Paid** external addon (BSL-1.1) | registry.yaml; `website/docs/addons/smart-stock.md` |
| **Projector mode** | External MIT addon | registry.yaml; `website/docs/addons/projector-mode.md` |
| **AI workpiece generator** | Text → SVG via OpenAI-compatible providers (addon) | `builtin_addons/rayforge-addon-ai-workpiece/`, `core/ai/openai_provider.py` |
| **Scripting** | `--script` (Python, before addons load) and `--uiscript` (after window), headless `--exit` | `rayforge/script.py`, `uiscript.py`, `website/docs/reference/cli.md` |
| **Addon system** | pluggy hooks (API v24, `core/hooks.py:6-8`). Manifests `rayforge-addon.yaml` with `worker`/`frontend` entry points. Git-based registry (`config.py:69-72`). Gumroad/Patreon license gating for paid addons (`rayforge/license/`, `addon_mgr/addon_manager.py:34,129,667`). | `website/docs/developer/addon-*.md` |
| **G-code dialects and macros** | GRBL, Smoothieware, Marlin, LinuxCNC, Mach4, custom (template editor). Job/layer/workpiece hook macros with variables. | `machine/models/dialect*`, `pipeline/encoder/rust_helpers.py:141-182` |
| **Machine hours / maintenance** | Counters (estimated hours) | `machine/models/machine_hours.py`, `machine/cmd.py:203-208` |
| **Telemetry** | Opt-in Umami usage tracking behind a consent dialog | `rayforge/usage.py`; `ui_gtk/mainwindow.py:585-618` |
| **i18n** | en, pt, es, de, fr, uk, zh_CN, hi | `rayforge/locale/` |

---

## 5. macOS specifics

**Packaging: PyInstaller.**
- Spec: `Rayforge.spec`. Bundle id `org.rayforge.rayforge`, `LSMinimumSystemVersion 12.0`, `NSCameraUsageDescription`. That key was added after #510, where the app crashed on camera enumeration. Liquid Glass `Assets.car` icon.
- Built by `scripts/mac/mac_build.sh` with Homebrew deps (`scripts/mac/Brewfile`: gtk4, libadwaita, gobject-introspection, librsvg, libvips, openslide, mupdf, cairo, pango, harfbuzz). Brew dylibs are copied in and fixed with `install_name_tool` (`mac_build.sh:286-337`).

**Dependency divergence.**
- The mac bundle is built with **Python 3.12** and pinned **NumPy 1.26.4 / SciPy 1.11.4** (`mac_build.sh:10-11,107-151`) to keep Monterey working (#322).
- Newer SciPy needed macOS 13.3's Accelerate LAPACK. The startup error was swallowed and the app exited with code 0, misreporting "another instance running".
- Dev/CI (pixi) uses Python 3.15 and NumPy 2.5.4 (`pixi.toml`). The shipped mac binary therefore runs a different dependency set from the one the tests ran on.

**Signing.**
- **Ad-hoc only** (`codesign --force --deep --sign -`) in both `mac_build.sh:579-585` and CI (`.github/workflows/build-macos-universal.yml:240-262`, where `spctl --assess … || true` tolerates failure).
- **No Developer ID, no notarization.** A grep for notarytool, altool, stapler, "Developer ID" and APPLE_ID across `.github`, `scripts` and the spec found nothing.
- The install page tells users to run `xattr -dr com.apple.quarantine /Applications/Rayforge.app` (`website/src/components/InstallGuide.js:1303-1311`).

**CI.** Separate `macos-26` (arm64) and `macos-26-intel` builds, merged with `lipo` into a universal app and DMG, each with a smoke test (`--version`, `--exit`).

**Sizes** (GitHub release assets):

| Asset | 1.13.0-beta4 | 1.12.0 |
|---|---|---|
| macOS arm64 zip | 154 MB | 154 MB |
| macOS intel zip | 190 MB | 189 MB |
| macOS universal zip | 325 MB | 324 MB |
| macOS universal DMG | 266 MB | 268 MB |
| Windows installer | 248 MB | 248 MB |
| Linux AppImage | 452 MB | — |
| Debian .deb (PPA, uses system libs) | 12 MB | 12 MB |

- The docs give installed disk as "< 1 GB" and say complex projects "can easily consume 10 GB of RAM or more" (`website/docs/getting-started/installation.mdx:19-29`).
- 1.12.0 download counts: Windows exe 1,597; macOS DMG 244; mac zips 253; deb 179.

**macOS issues history.** All of these are closed, and **no macOS-specific issue is open** as of 2026-10-22 (open list checked):
- #510 camera crash (fixed by the plist key).
- #450, #604, #616: closing a dialog in full screen left a blank window (fixed in 1.13.0-beta3).
- #322 Monterey.
- #149 POSIX shared-memory name too long ("Errno 63") in the old multiprocessing pipeline.
- #148 GL shader validation fallback on macOS.
- #224 avoid importing GTK in worker processes.

**What a galvo would add on macOS:** a bundled libusb dylib. It is not in the Brewfile today (PR #288 added only pyusb to requirements). It also needs the ad-hoc signing to cover it. **UNVERIFIED** whether ad-hoc-signed libusb in a quarantined app can claim the device without extra steps.

---

## 6. Rust and C in the repo

- **Rust in the Rayforge repo: none.**
  - `.rustfmt.toml` (`max_width = 80`) and `.gitignore` "`# Rust ops crate`/`target/`" are remnants of the in-tree raygeo era.
  - `pixi.toml` still lists `rust = "==1.99.0"` and `patchelf` as app dependencies. That is presumably for source builds of Rust-backed Python packages and the AppImage; **UNVERIFIED** why it is needed.
  - The Rust lives in the separately versioned **raygeo** and **raydriver** (PyPI wheels for macOS x86_64 and arm64, manylinux x86_64 and aarch64, win_amd64).
  - `AGENTS.md` tells contributors to fix root causes in raygeo instead of working around them (`docs/agents/raygeo.md`).
- **C: one file, `snap/semwraplib.c`.**
  - An `LD_PRELOAD` shim that intercepts `sem_open`, `sem_unlink`, `shm_open` and `shm_unlink` and prefixes names with `snap.<name>.`, so Python multiprocessing semaphores and shared memory work under snap strict confinement.
  - Built in `snap/snapcraft.yaml:44-49` and preloaded at `:28`.
- **Native code pulled in through Python packages:**
  - GTK4/libadwaita/cairo/pango (C)
  - libvips and pyvips (images), OpenCV (camera), PyMuPDF/poppler (PDF)
  - SciPy and NumPy (sketcher solver)
  - vtracer (Rust), trimesh, ezdxf
  - PyOpenGL-accelerate (Cython)

---

## 7. Code health, governance, license

- **License:** **MIT confirmed** (`LICENSE`; `pyproject.toml` `license = {text = "MIT"}`; GitHub `license.spdx_id = MIT`). raygeo and raydriver are MIT too. ruida-pa is MIT (PyPI `license_expression`). galvoplotter is MIT. The *paid* addons Deepnest and Smart Stock are **BSL-1.1** (registry.yaml).
- **Size:** about 266k lines of Python in `rayforge/`, about 111k lines of tests in `tests/`, plus addon tests. raygeo adds 106k lines of Rust and raydriver 6k.
- **Tests** (I counted test functions; I did not run them):
  - 5,677 in `tests/` and 2,252 in addon tests; 93 files are UI tests (`-m ui`, run under xvfb).
  - raygeo: 4,099 Python-side tests. Its rule is that **Rust code is never unit-tested in Rust**; "All tests are Python-based" (`raygeo:AGENTS.md`). The same rule applies to raydriver.
  - raydriver: 159 tests against a GRBL firmware emulator.
- **CI** (`.github/workflows/`):
  - lint (flake8, ruff, pyflakes, pyright, appstream), backend, stress and UI tests on ubuntu
  - Windows exe built with MSYS2 + PyInstaller, smoke-tested, and published to winget
  - macOS universal build
  - AppImage, PPA .deb, snap, PyPI wheel
  - website deploy
  - an **"LLM cost estimate"** bot on PRs
  - a **stale bot** that closes issues and PRs after 30 + 7 days of inactivity, which is how galvo #261 and PR #288 died
  - dependabot for pixi
- **Contributors** (`gh api repos/barebaric/rayforge/contributors`): **knipknap 4,447 commits** (Samuel Abels / Barebaric). Next: switch87 64, heathkh 40, pgilfernandez 38, atkaper 27, MausRundung 27, fllmn 21, StevenIsaacs 17 (Ruida); about 23 humans in total.
  - raygeo: knipknap 698 of about 720.
  - raydriver: knipknap 46 of 46.
  - **Bus factor = 1.**
  - Recent external contribution is real but small: 32 of 167 merged PRs since 2026-07-22 came from non-maintainers, mainly switch87, atkaper, medoix and StevenIsaacs.
- **Cadence:** first release 0.13 on 2025-07-10. **101 GitHub releases** in about 15 months. In Oct 2026: 1.12.0 (10-02), 1.13.0-beta1 through beta4 (10-16 to 10-21). Last 3 months: about 1,125 commits. raygeo: **123 releases since 2026-05-15**, three in the past week (1.66.0, 1.67.0, 1.67.1). Addon API version 24, with `MINIMUM_API_VERSION 16` (`core/hooks.py:6-8`). It has been bumped "for incompatible raygeo changes" (CHANGELOG.md:1202).
- **Development style:** AI-assisted.
  - `AGENTS.md` files in all three repos; `RELEASE_MANIFEST.md` describes an AI "Release Agent".
  - The maintainer called the galvo driver "semi vibe coded".
  - Many issues carry an `rca-done` label.
  - `CONTRIBUTING.md` accepts AI code but rejects "raw, unreviewed AI output".
- **Governance / business:**
  - Single maintainer with Patreon, Gumroad paid addons, and AliExpress affiliate links on the devices page (`website/src/pages/resources/devices.js`).
  - The registry is a GitHub YAML with an allow-list (`barebaric/rayforge-registry`).
  - No foundation, no documented governance.
- **Popularity:** 301 stars and 63 forks (GitHub API, 2026-10-22). Discord exists.
- **Docs drift:** the developer docs lag the code (driver guide `run()` signature and Ops names, §2.1; the 3D "subprocess" in the pipeline docs; the `register_machines` example that cannot work, §3.1).

---

## 8. What to copy, and what to avoid

### 8.1 Copy into OpenLaser

1. **A compact, typed Ops IR with structural markers.** Use Job, Layer, Workpiece and Section markers plus state commands, rather than going straight to G-code (`raygeo:src/ops/types.rs`, `enums.rs`). Add galvo-native state commands (delays, wobble, mark/jump speed, Q-freq in Hz, pulse width **in ns**), which Rayforge lacks.
2. **An Op ↔ machine-code bidirectional map** as compact int32 spans. It drives progress highlighting, "start from here" and the code view cheaply (`pipeline/encoder/base.py:9-127`).
3. **An incremental intent graph with stable keys and content-hash version tokens**: workpiece×step → step → job → machine-transform → encode (`intent_builder.py:1-34`). Position-insensitive caching makes drag and move instant. In Rust this maps to a salsa- or rayon-style DAG.
4. **Separate world → machine transform and encoder stages.** Machine quirks (origin, flips, WCS, rotary, our field scale-correction 1.1834, lens correction) belong in a dedicated transform node, not in each encoder (`intent_builder.py:1304-1359`).
5. **Capability flags on drivers** (`uses_gcode`, `native_overscan`, `reports_granular_progress`, `supports_*`, `maturity` with UI warnings) so the pipeline adapts (`driver.py:279-307`). For OpenLaser: make galvo-versus-gantry a first-class *kinematics* capability, not a pile of stubs.
6. **Stop semantics from raydriver.** On cancel:
   - set the flag
   - wake parked senders
   - send the controller's immediate abort
   - **hard-abort the streaming task before resetting flow state**
   - drain queues
   - delay, then send safety-off commands

   This is `raydriver:src/grbl/session.rs:1757-1830`, and it is the same lesson as our 0.18 s LMC stop fix.
7. **Firmware emulators for tests.** raydriver's GRBL 1.1h emulator models the RX buffer, planner, realtime bytes and alarms. Build an **LMC list-buffer emulator** so stop/abort, full-buffer and USB-timeout behavior gets CI coverage without hardware.
8. **Declarative device profiles** (`resources/devices/<id>/device.yaml`, 35 shipped) and the **LightBurn `.lbrn/.lbrn2` and `.lbdev` importers**. Importing users' LightBurn files and devices is a migration feature in its own right.
9. **Material library and recipes with specificity-scored auto-matching**, plus the material test grid.
10. **Camera pipeline:** ChArUco/ArUco/dot-grid lens calibration, then ≥4-point homography alignment. Simple and proven. In Rust: `opencv` crate or a pure-Rust homography.
11. **VarSet-style declarative settings** that auto-generate driver and step UIs (`core/varset/`).
12. **Opt-in telemetry with an explicit consent dialog** (`usage.py`).
13. **Reusable MIT Rust code (vendor or pin, do not depend blindly):**
    - raygeo `ops/transform/optimize.rs` (KD-tree + 2-opt), `tabs.rs`, `clip.rs`, `lead_in_out.rs`, `merge_lines.rs`, raster assembly, text (swash/fontdb), and clipper2-based offsets
    - raydriver for GRBL
    - vtracer (a Rust crate) for tracing

### 8.2 Avoid (Rayforge's limits and mistakes)

1. **Advisory-only safety.** "Proceed" is available even with ERROR issues, and a failed check proceeds silently (`sanity_check_dialog.py:32-43`, `mainwindow.py:2354-2395`). OpenLaser keeps hard refusals: out-of-field, path/point limits, whole-job validation before the first burn, and output-off/frame-only geometry never burned.
2. **Simulated progress and clamping in non-G-code drivers** (Ruida `run()` and PR #288). Report only acknowledged progress, and never clamp coordinates.
3. **A driver ABC shaped around GRBL.** Every driver must implement WCS, probe, home, tool and settings methods. Use capability traits instead: `Streams`, `Jogs`, `Homes`, `HasWcs`, `GalvoField`, `RedLight` and so on.
4. **Lossy encoder output.** Binary payloads cannot pass through the cached pipeline (`driver_data` dropped). Make the encoded artifact a typed enum: `GcodeText | LmcLists(Vec<[u16;6]>) | RuidaBytes`, plus a human-readable listing.
5. **A Python + GTK + PyInstaller footprint.** 154-325 MB mac bundles and a 452 MB AppImage. The mac build uses different NumPy/SciPy/Python than CI tests. Ad-hoc signing only; no notarization; manual `xattr`. GTK-on-macOS quirks (full-screen dialogs, camera TCC). Snap needs C shims. RAM of "10 GB or more" on complex projects; a 2 GiB pipeline cache default. A single Rust binary with native signing and notarization should be an explicit OpenLaser goal.
6. **Two languages across a PyO3 boundary.** Dict-serialized contexts (`rust_helpers.py`), GIL-bound Python encoders on rayon threads, and API churn that forces addon API bumps. OpenLaser avoids this boundary by being Rust end-to-end.
7. **Rust that is only tested through Python.** raygeo's and raydriver's rule bans Rust tests, and CI never builds raygeo without the `python` feature. raygeo also installs an **unconditional `#[global_allocator]` (mimalloc)** in `lib.rs` (≈ line 140), which conflicts with any consumer that sets its own allocator. If OpenLaser vendors raygeo code, strip the allocator, add Rust tests, and pin a commit. **UNVERIFIED** whether raygeo compiles with `--no-default-features`; I did not run cargo.
8. **Dead extension points and docs drift** (`register_machines`). OpenLaser's plugin and driver registry should be tested end-to-end in CI.
9. **Stale-bot closure of hardware-specific requests.** Galvo requests died because no maintainer had the hardware. OpenLaser's niche, galvo/MOPA first with real-hardware validation, is exactly what Rayforge lacks.

---

## 9. Decision input: Rust app, Rayforge fork, or Rayforge contribution?

| Option | Pros | Cons | Verdict |
|---|---|---|---|
| **Fork Rayforge** | Inherit a huge feature set immediately (§4) | 4.4k-commit single-maintainer firehose: about 1,100 commits per quarter, raygeo releasing several times a week. Python/GTK packaging weight. You would own a 266k-line Python plus 106k-line Rust codebase you didn't write, with drift that forces constant rebases. Contradicts the "Rust app" goal. | **Not recommended** |
| **Contribute an LMC/galvo addon to Rayforge** | The maintainer explicitly asked for someone with hardware to step in (#261). Most of the CAD, camera, material and import stack comes for free. A galvoplotter-based addon reuses our prototype knowledge. Low cost (1-2 weeks for a working version). | Needs upstream changes: driver registration, libusb/pyusb bundling, eventually raygeo typed galvo ops and time model. The Python/GTK limits remain. Subject to the stale bot and the maintainer's priorities. Safety model is advisory unless we enforce it in the driver. | **Optional side track.** Good for goodwill and quick user value. Do not make OpenLaser depend on it. |
| **New Rust app (OpenLaser)** | Matches the goal. Galvo/MOPA-first design (field-centered coordinates, delays, wobble, hatch, lens correction, hard safety) instead of retrofitting a gantry ABC. A single signed and notarized binary is achievable. Can *borrow* MIT Rust code (raygeo, raydriver, vtracer) and copy Rayforge's proven architecture (§8.1). | Large scope to reach LightBurn/Rayforge parity: sketcher, camera, importers, 3D sim. Needs a disciplined MVP. | **Recommended**, with Rayforge, raygeo and raydriver as the reference implementation and the vendor/borrow source |

Suggested posture:
- Build OpenLaser in Rust.
- Start with the **Ops IR + LMC encoder + spooler + emulator + safety core**, porting our prototype's proven LMC behavior.
- Use **raygeo modules as vendored, attributed, pinned code** where they save months (optimize/TSP, tabs, offsets, raster, text), adding Rust tests as they come in.
- Revisit an upstream Rayforge galvo addon once OpenLaser's LMC core is stable. It could even share protocol knowledge or an emulator spec.

---

## 10. Open questions / UNVERIFIED items

1. Does raygeo build and run as a pure Rust crate (`--no-default-features`, no PyO3/numpy)? CI never tests it, and I did not run cargo (rule: no execution of cloned code). Its Rust API has no stability promise (crate version 0.0.0, no crates.io).
2. Would the maintainer accept upstream galvo work in core (typed delay/wobble ops in raygeo, a CENTER origin, a working driver-registration hook, libusb in the bundles)? There is no stated policy. The PR #288 history suggests interest but no hardware.
3. Can an addon-registered driver (via `register_driver()` in `rayforge_init`) be used by saved machines at startup, given lazy `machine_mgr` creation and `get_driver_cls` falling back to `NoDeviceDriver`? This needs a runtime test.
4. Actual stop/pause latency of Rayforge drivers (GRBL via raydriver, Ruida via ruida-pa). Nothing is measured in the repo. Our LMC benchmark is 0.18 s.
5. Behavior of ad-hoc-signed bundles with a vendored libusb on macOS 26 (Gatekeeper/TCC) for USB-bulk devices. Also the Windows WinUSB/Zadig conflict with EZCAD, which is a general galvo issue.
6. Galvo field mapping convention for the JPT M7 / 200 mm lens (units per mm, axis flips, correction table upload), to be confirmed against our prototype and galvoplotter source before reusing anything from PR #288.
7. Whether Rayforge's raster ScanLine model, with per-pixel u8 power, can be encoded efficiently for LMC without per-segment power changes, or whether galvo raster should stay a separate assembler.
8. Why `pixi.toml` pins `rust==1.99.0` for the app environment.
9. Discord (#driver-development) may hold galvo discussion that GitHub search cannot see. Not accessed.

---

## Appendix A — key file index (Rayforge @ a120f72)

- Driver ABC: `rayforge/machine/driver/driver.py` (Driver :260; flags :279-307; run :647; frame :675; set_hold :724; cancel :731; jog :844; build_frame_ops :215)
- Driver registry: `rayforge/machine/driver/__init__.py` (drivers :36-38; get_driver_cls :48; register_driver :52)
- Encoders: `rayforge/pipeline/encoder/{base.py,gcode.py,rust_helpers.py}`
- Pipeline facade and intents: `rayforge/pipeline/pipeline.py`, `intent_builder.py`, `intent_controller.py`
- Job send / hold / cancel: `rayforge/machine/cmd.py` (:89-210, :463, :509-523)
- GRBL (Rust): `rayforge/machine/driver/grbl/grbl_serial.py`; `raydriver:src/grbl/session.rs`
- Ruida: `rayforge/machine/driver/ruidarpa/{rpa_adapter.py,rpa_encoder.py,rpa_direct_driver.py,README.md,AGENTS.md}`
- Machine and laser models: `rayforge/machine/models/{machine.py,laser.py,coordspace.py}`
- Sanity checks: `rayforge/machine/sanity/checks/*.py`, `rayforge/ui_gtk/shared/sanity_check_dialog.py`
- Hooks and registration: `rayforge/core/hooks.py`, `rayforge/core/registration.py`
- LightBurn importers: `rayforge/image/lightburn/importer.py`, `rayforge/machine/device/lightburn_importer.py`
- Camera: `rayforge/camera/{controller.py,calibration/*,models/camera.py}`
- Packaging: `Rayforge.spec`, `scripts/mac/{mac_build.sh,mac_setup.sh,Brewfile}`, `.github/workflows/build-macos-universal.yml`, `snap/{snapcraft.yaml,semwraplib.c}`
- Galvo PR (unmerged): branch `feat/galvo-driver` @ e85c5e6, `rayforge/machine/driver/galvo/*.py`
- Ops IR: `raygeo:src/ops/{types.rs,enums.rs,state.rs,container/mod.rs,container/time.rs,convert/gcode.rs,transform/*.rs}`

## Appendix B — URLs

- https://github.com/barebaric/rayforge · https://rayforge.org · https://rayforge.org/docs/developer/driver · https://rayforge.org/resources/devices
- https://github.com/barebaric/rayforge/issues/261 · /pull/288 · /issues/308 · /issues/14 · /pull/404 · /pull/322 · /issues/510
- https://github.com/barebaric/raygeo · https://pypi.org/project/raygeo/ · https://github.com/barebaric/raydriver · https://pypi.org/project/raydriver/
- https://github.com/barebaric/rayforge-registry (registry.yaml: Deepnest and Smart Stock are BSL-1.1 paid)
- https://github.com/StevenIsaacs/ruida-pa · https://pypi.org/project/ruida-pa/
- https://github.com/meerk40t/galvoplotter (MIT, last push 2023-10-27)
