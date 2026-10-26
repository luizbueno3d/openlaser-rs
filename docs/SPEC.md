# OpenLaser build specification

*Revision 2, 2026-10-23: revision 1 plus the adversarial review recorded in the [Review log](#review-log) at the
end. This file is the authority for the first OpenLaser build. Where it disagrees with `docs/design/*.md`,
`~/AI/ClaudeCode/openlaser-rust-plan/PLAN.md` or `AGENTS.md`, this file wins and the integrator updates the others
(WP0 updates `AGENTS.md`).*

**Inputs.** PLAN.md §3, §3.6, §5, §6 (cited **PLAN §n**); research 05, the LMC protocol (**R05 §n**); research 04,
crates; the design notes in `docs/design/`: `craft-automation-pattern.md` (**AUT**), `craft-engine-pattern.md`
(**ENG**), `craft-ui-pattern.md` (**UI**), `lightburn-command-catalog.md` (**CAT**), `golden-capture-report.md`;
`tests/golden/README.md` (**GOLD**). Crate versions were checked against the crates.io API on 2026-10-23.

**How to read this as an implementation agent.** Read §0, §1, §2 and §6 in full. Then read the contract of every
crate you own (§3) and of every crate you call, and your work package (§7). Red-zone packages (WP3, WP5, WP9, and
the dispatcher parts of WP7/WP8) also read §4 and §5 in full. `AGENTS.md` §1 (laser safety) outranks this file.

---

## 0. Decisions this spec makes

These close the open questions in the design notes. Each can be reversed later; none may be reversed inside a
work package.

| # | Decision | Closes |
|---|---|---|
| S1 | Crates are `openlaser-<name>` in `crates/<name>`. Binaries: **`openlaser`** (GUI), **`openlaser-cli`** (Craft-style automation CLI and MCP server), **`openlaser-hw`** (human-only hardware CLI, PLAN's `ol`). The PLAN D4 rename is a later global rename | ENG Q2, CAT Q2 |
| S2 | Command ids use **camelCase segments** like PhotoCraft, VectorCraft and FilmCraft (`layer.cutSettings.set`). snake_case aliases resolve (`layer.cut_settings.set`). CAT's snake ids convert segment by segment (`flip_h` → `flipH`). Command params and every JSON result use camelCase keys carrying their unit (`speedMmS`, `powerPct`, `frequencyKhz`, `qPulseNs`, `intervalMm`, `angleDeg`); snake keys are normalised before the strict decode. MCP tool names and tool arguments stay snake_case (`max_side`), as in PhotoCraft | ENG Q1, CAT Q1 |
| S3 | **Two CLIs.** `openlaser-cli` can never link `openlaser-arm`, `openlaser-human`, `openlaser-transport`, `nusb` or `serialport`. `openlaser-hw` links no server crate (rmcp, tokio, hyper, axum, tungstenite) | AUT Q1, UI O2 |
| S4 | **Device I/O by injection.** The engine takes a `DeviceAccess`. Only `openlaser-transport` uses `nusb`/`serialport`. `apps/openlaser` and `apps/openlaser-hw` link it only with their cargo feature **`hw`**; default builds can drive the simulator only. Agents never build with `hw` | AUT Q3 |
| S5 | **ARM minting** lives in `openlaser-arm`; its normal reverse dependencies are exactly {`openlaser-human`, `openlaser-hw`}. `openlaser-ui-egui` hosts the control dispatcher and cannot reach `mint` (it gets tokens through the `HumanSurfaces` trait that `openlaser-human` implements) | AUT Q4 |
| S6 | GUI stack pinned to **egui/eframe/egui_extras/egui_kittest `=0.36.2`**, **craft-ui @ `5c874d51`** (PhotoCraft's exact pin) and **craft-atomic, craft-settings, craft-quit @ `3d37da38`** (PhotoCraft's craft-print pin). Docking is `craft_ui::docking`, not egui_tiles. This overrides PLAN D6. Checked: at `3d37da38` none of atomic/settings/quit depends on craft-ui, so the graph holds one craft-ui; both revisions are in the local cargo git cache | UI U1–U3 |
| S7 | One geometry stack: **kurbo `=0.11.3`, usvg `=0.45.1`, tiny-skia `=0.11.4`**, the versions `egui_extras[svg]` already pulls (all cached locally). Upgrade together with craft-libs | UI O5, PLAN S4 |
| S8 | The prototype's "Frame only" layer becomes LightBurn's **tool layers T1/T2**. Live Framing source "Tool layers only" exists, and a frame of tool layers only never counts toward ARM | UI O4 |
| S9 | Machine profiles, calibration, work area and source limits are **human-only writes**; agents read them (`device.settings.set` is human-only, diverging from CAT's `ok`) | AUT C3.6 |
| S10 | Agent-requested red-light framing (`frame_request`) always needs a human **Approve** click. No "allow for this session" mode in this build | AUT Q2, CAT Q3 |
| S11 | Registry, `Session::edit`, snapshot undo, the origin policy and **atomic** `command_batch {atomic:true}` ship in this build | ENG Q4, Q12 |
| S12 | The control channel is **off by default**; it starts with `--control` or the human-only switch Settings › Automation › Allow agent connections. The discovery file holds the token inline (0600 file in a 0700 folder). A connected agent is always shown in the status bar; no extra pairing prompt | UI O3, AUT Q6, Q7 |
| S13 | Galvo origin = field centre, Y up, mm. Fill is even-odd per layer. Cut-setting edits are undoable. Energy parameters (speed, power, frequency, pulse) live in separate `galvo` and `gantry` blocks; geometry parameters (interval, angle, passes) are shared | ENG Q5, Q6 |
| S14 | Native project file **`.olp`** = zip {`manifest.json` (separate serde model, `formatVersion` 1), `assets/<blake3>.<ext>`}, paths inline as compact path strings | ENG Q9 |
| S15 | Goldens: `py-compat` must reproduce **oracle v0** (`6aed8ff`); shipping packets are compared with **v1** (`5f7aa63`) minus the listed divergences. `rust-v1` goldens are frozen only after Luiz signs `docs/DIVERGENCES.md` (PLAN D13) | golden Q1 |
| S16 | Images are imported and shown in P0. Burning images (Threshold and Jarvis as on/off runs, no per-pixel power, so H21 is not needed) is a **stretch** item of this build | CAT Q4 |
| S17 | Any real-USB open, from the GUI or `openlaser-hw`, needs a valid **bench session** (created at a TTY by `openlaser-hw bench start`, ≤ 2 h old) until Luiz relaxes this | PLAN §3.6 layer 3 |
| S18 | Shortcuts follow CAT §10: Esc is STOP whenever a frame or job is active; ⌘W is Weld (provisional); help is ⌘?. **Human-only commands have no menu item and no shortcut**: they are reached only through human-only widgets (Laser panel, Live Framing, ARM dialog, device dialogs) | CAT Q7 |
| S19 | **Windows is required** (Luiz, 2026-10-23): the whole workspace builds and its tests pass on Windows; see §2.6 | Luiz |
| S20 | **A frame receipt means "the board executed this pass"**, not "the packets were accepted". Frame Once is one list (the OM pass count baked in) and yields one receipt after the board reports !BUSY; Frame Loop sends **one list per pass** and yields a receipt after each pass's !BUSY. A frame stopped before !BUSY yields no receipt for that list | review R1 |
| S21 | **The fingerprint is the emission identity** (machine, device, sections with quantised words and path bits, repeats, calibration and machine transform). Frame style and source are **not** in it: a receipt records them, and `covers_job()` decides. So framing with Bounds or Contour from Live Framing satisfies Start for the same job | review R2 |
| S22 | **The ARM token binds the bytes.** `EmissionSummary.bytes_hash` = blake3 over every single command and packet the run will send; `mint` copies it into the token; the driver recomputes it from the job it was handed and refuses a mismatch. `PendingArm` keeps the exact encoded job whose summary the human saw, and `run_armed` sends that object | review R3 |
| S23 | **Hardware is default-deny**: every USB or serial open and every `detect` needs `OPENLASER_HW_PRESENT=1` in the environment, `OPENLASER_NO_HW` unset, a build with feature `hw` and (S17) a bench session. Luiz builds `hw` binaries into his own target directory (`~/AI/OpenLaser/.target/hw`), never an agent lane. The first emission of each bench session is capped at 10 % power in software (`BenchPowerCap`) | review R4, PLAN §5.2 |
| S24 | **Machine profiles are files written only through an injected `ProfileStore`** (`machine::ProfileStore`, implemented by `format::machines::DirProfileStore` in the per-user app dir `…/OpenLaser/machines/<id>.json`). The GUI and `openlaser-hw` inject it; `openlaser-cli` never does, so no agent path can write a profile even through a policy bug. A saved profile shadows the built-in with the same id | review R5 |

---

## 1. Product

### 1.1 Statement

OpenLaser is LightBurn rebuilt in Rust, the way PhotoCraft rebuilds Photoshop: a native egui application whose
workflow, window layout, menus, terminology, shortcuts and file formats match LightBurn closely enough that a
LightBurn user is at home. It is built Craft-family style: every user-visible action is an engine command with an
id, and the GUI, `openlaser-cli` and its MCP server all dispatch those ids.

Its first and most important machine is Luiz's **JPT M7 MOPA fiber laser on a 200 mm galvo** with a BJJCZ LMC
board (USBLMCV4, `0x9588:0x9899`), driven over USB from a Mac M2 on macOS 26. GRBL gantries (CO₂, diode, pen
plotter) come second: G-code export is in this build; live streaming is coded and tested on a fake only.

**What "exactly LightBurn" means in this build:**

| Aspect | Matched | Checked by |
|---|---|---|
| Workflow | import → colour layers (palette 00–29, tool layers T1/T2) → Cut Settings per layer → Preview → Frame (Live Framing on galvo) → Start → Stop. Galvos always frame first | E2E kittest `kittest_business_card_flow` |
| Layout | macOS menu bar (File, Edit, Tools, Arrange, Laser Tools, Window, Language, Help); main toolbar; numeric-edits bar; left creation tools; canvas with rulers; right dock with Cuts / Layers above a Laser / Library / Move / Console tab group; bottom Arrange toolbar and colour palette; status bar (UI §4.1) | `kittest_lightburn_regions_present`; Luiz's side-by-side check against his LightBurn 1.5.04, never committed |
| Terminology | LightBurn's words: Cuts / Layers, Line, Fill, Offset Fill, Image, Speed, Max Power, Frequency, Q-Pulse Width, Line Interval, Scan Angle, Bi-directional Fill, Cross-Hatch, Angle Increment, Passes, Output, Show, Frame, Start, Stop, Device Settings, Field Size, Red Dot Offset, Galvo 1 is X Axis, Cut Selected Graphics | labels in `menu_catalog.rs` and panels |
| Menus and parity | every LightBurn menu item is in `menu_catalog.rs` in LightBurn's order; unbuilt items render disabled; `cargo xtask parity` writes `docs/parity-checklist.md` and its floor only rises | `parity_floor` test |
| Shortcuts | LightBurn defaults with Ctrl → ⌘, macOS clashes resolved as CAT §10 | shortcut table test |
| Files | read SVG (true scale), DXF, PNG/JPEG/BMP/TIFF/WebP/GIF, the prototype's `workspace.json` + `layer_settings.json`; write `.olp` and G-code. `.lbrn2` read is the next build (P1), write later (P2) | T4, importer tests |

**Never copied:** LightBurn's code, icons, images, screenshots or help text. LightBurn's name appears only in docs
and comparisons, never in the UI or the app name. Palette colour values are interoperability facts and are taken
from LightBurn's public documentation with provenance recorded (UI O6).

**Deliberate divergences from LightBurn** (CAT §13): refuse, never warn or clamp; galvo framing and the job
checklist are mandatory, not preferences; the confirmation shows values decoded from the bytes; no unbounded
"run continuously"; no laser-on framing on galvo; no automatic USB reset; agents can do everything except emit or
touch hardware; galvo and gantry energy blocks are separate; no telemetry, licence keys or update nags.

### 1.2 Non-goals of this build

Not built now (each is in CAT with a tier): text and fonts, node editing, booleans, weld, offset, arrays, nesting,
variable text, barcodes, `.lbrn2` import/export, PDF/AI import, Offset Fill, sub-layer editor UI (the model has
sub-layers), wobble, per-layer timing overrides (model only), grayscale images, trace image, camera, rotary,
cylinder correction, split/repeat marking, Print and Cut, material library, Ruida/DSP, pause/resume on LMC (H15),
`.cor` upload (H5), the foot-switch ARM (M3, H11), door interlocks, `ol-hw-raw` bench binary (H13/H23),
Windows/Linux installers and code signing (Windows *builds and tests* are required, §2.6), notarisation, auto-update, translations beyond English and Portuguese (Brazil) stubs.

**Fallback (Luiz's instruction).** If this build cannot reach the MOPA path, the OpenMopa prototype stays the
working tool and gets LightBurn-like UI changes in a fork. That work is outside this spec.

### 1.3 Safety requirements (normative)

From `AGENTS.md` §1 and PLAN §3.6. Every work package is bound by them.

1. **Agents never emit and never open the real laser USB device.** No test, CI job, agent or script opens USB or
   builds a binary with the `hw` feature. All device tests run on `openlaser-lmc-sim`.
2. **Refuse, never warn and continue.** No clamping, truncating, dropping or "proceed after timeout". Anything in
   the design that will not burn is either a refusal (job blocked) or a numbered notice shown in the confirmation.
3. **Bytes are the truth.** The LMC encoder matches `tests/golden/` byte for byte except the divergences in §5.6.
   The ARM confirmation is rendered only from `EmissionSummary`, decoded from the packets that will be sent.
4. **Only a human can ARM**: typed `ARM` plus checklist in the GUI behind the provenance gate, or `ARM <nonce>` at a
   TTY in `openlaser-hw`. No API, MCP tool, control method, command id, preference, file field or synthetic input
   can mint an `ArmToken`.
5. **Stop always works** from every surface, agents included (through the GUI's control channel; `openlaser-hw`
   has no agent channel by design, because no agent runs during a hardware session, §8.2), and is never disabled,
   hidden, modal-blocked, queued behind another request or behind a permission. A stop tripped after a command was
   queued cancels that command (§5.3). The software stop is still not the primary safety device; the E-stop and key
   switch are.
6. **Never crash.** Non-test code has no `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `unreachable!` or
   unchecked indexing on input-derived values. `unsafe_code = "forbid"` everywhere. `panic = "unwind"` explicitly.
7. **Fail closed.** Every stub, default and test double denies: gates that return `Result` return `Err`, gates that
   return `bool` return the refusing value (`is_human_click` false, `frame_is_synthetic` true), and nothing is
   enabled by a missing setting, file or environment variable (S23).

---

## 2. Workspace

### 2.1 Tree

```
openlaser/
  Cargo.toml  Cargo.lock  rust-toolchain.toml  .cargo/config.toml  clippy.toml  rustfmt.toml  deny.toml
  AGENTS.md  CLAUDE.md  LICENSE-MIT  LICENSE-APACHE  ATTRIBUTION.md
  docs/  SPEC.md  DIVERGENCES.md  design/  parity-checklist.md (generated)
  crates/
    types/  geom/  doc/  machine/  format/  import/  plan/  safety/  arm/  render/  gcode/  lmc/
    driver/  lmc-sim/  transport/  engine/  control/  automation/  ui-egui/  human/  testkit/
  apps/
    openlaser/  openlaser-cli/  openlaser-hw/
  xtask/
  tests/golden/     the Python-oracle fixtures (read-only for work packages)
  tests/fixtures/   small hand-made inputs (SVG, DXF, PNG, OM workspaces)
```

### 2.2 Crates

Every crate: `publish = false`, `#![forbid(unsafe_code)]`, `[lints] workspace = true`, licence
`MIT OR Apache-2.0`, edition 2024. "Internal deps" is the complete allowed list of normal dependencies on workspace
crates; `cargo xtask deps` fails on any edge not listed (dev-dependencies: §2.3 rule 10).

| Crate (`openlaser-…`) | Responsibility | Internal deps (normal) | External deps |
|---|---|---|---|
| `types` | Unit newtypes (`Mm`, `MmPerS`, `Percent`, `Hz`, `Ns`, `Us`, `Deg`); ids; `Fingerprint`; `DeviceKind`; `MonoTime` + `Clock`; refusal and notice codes; `EmissionSummary`; `FrameReceipt`; the opaque `ArmToken` (check only); the `serde_secs`/`serde_ms` helpers; **the T0 numeric encoders** (`galvo.rs`) | — | serde, thiserror |
| `geom` | `Polyline`, kurbo flattening, transforms, bounds, join/close/dedupe (OM `geometry.py` port), hatch (OM `raster.py` port, T3), `HitIndex` (rstar) | types | kurbo, rstar, serde, thiserror |
| `doc` | The project model: shapes, the 30 colour layers + T1/T2, cut settings (galvo/gantry/fill/image), selection, palette. Pure data | types, geom | kurbo, serde, serde_json, thiserror |
| `machine` | Machine profiles (`GalvoMachine`, `GantryMachine`), calibration, JPT M7 built-in, OM gantry profiles, EZCAD `markcfg` import, OM `calibration.json` import, the `ProfileStore` trait (S24). No I/O | types, geom, doc | serde, thiserror |
| `format` | `.olp` read/write (zip + manifest model + migrations), atomic save; `machines` (profile files: `load_all`, `DirProfileStore`); `camera_cal` (`DirCalibrationStore`, the machine-local camera↔bed calibration file) | types, geom, doc, machine, camera | zip, flate2 (backend selection only), serde, serde_json, blake3, craft-atomic, craft-settings (app dirs), thiserror |
| `import` | SVG (usvg, true scale), DXF (OM parser port, T4), raster images, OM `workspace.json` + `layer_settings.json` | types, geom, doc | usvg, image, serde, serde_json, blake3, thiserror |
| `plan` | Document + machine → `JobPlan`: sections per layer/sub-layer/pass, machine transform, hatch, quantised galvo parameters (`GalvoCutQ`), frame paths, plan facts, galvo time model, fingerprint | types, geom, doc, machine | blake3, serde, thiserror |
| `safety` | `validate()`: every hard refusal and notice; sole constructor of `EmitPlan`/`FramePlan` | types, geom, doc, machine, plan | serde, thiserror |
| `text` | System font discovery (`FontLibrary`, bounded directory walk, `OPENLASER_FONT_DIRS`, lazy index) and `layout()`: harfrust shaping (kerning, ligatures) + skrifa glyph outlines into a Y-up `BezPath`; cap-height sizing, line spacing, left/centre/right alignment; missing glyphs are a named error | types, geom | kurbo, skrifa, harfrust, serde, thiserror |
| `arm` | `mint()`: the only code that creates an `ArmToken`; `HumanConfirm` witnesses | types | getrandom |
| `render` | UI-free `DisplayList` (document, plan, emission) and deterministic tiny-skia PNG rendering | types, geom, doc, machine, plan | tiny-skia, thiserror |
| `gcode` | GRBL G-code encoder (T2 parity, shipping refuses overspeed), number formatting, gantry frame program (`GcodeFrame`, laser off by type) | types, geom, machine, plan, safety | serde, blake3, thiserror |
| `lmc` | Pure LMC protocol: opcodes, list builder, `LightingList`/`MarkingList`, packetiser, job and frame encoders, disassembler, `EmissionSummary` from bytes (incl. `bytes_hash`), status parser, `LmcTransport` trait, test-only `py-compat` | types, geom, machine, plan, safety | serde, blake3, thiserror |
| `driver` | Driver runtime: `Backend`, driver thread, `StopLatch`/`Stopper`, watchdog, dead-man rule, panic guard, events; **LMC session** (`LmcBackend`); **GRBL streaming** (`GrblBackend`); `DeviceProvider` (incl. `detect`) and `ByteLink` traits | types, machine, lmc, gcode | crossbeam-channel, serde, thiserror, log |
| `lmc-sim` | `FakeLmc` (board model, virtual clock, faults, emission recorder), offline `simulate_job`, `SimProvider`, `FakeGrbl` (port of `fake_grbl.py`) | types, machine, lmc, driver | crossbeam-channel, thiserror |
| `transport` | **The only crate that touches devices**: `UsbLmc` (nusb), `SerialLink`/`TcpLink`, `UsbProvider`, read-only `detect`, board lock, bench session, keep-awake assertion (`keepawake`) | types, machine, lmc, driver | nusb, serialport, keepawake, craft-settings (app dirs), serde, serde_json, thiserror |
| `camera` | Webcam capture for camera-guided placement (nokhwa `input-native`: AVFoundation/MSMF/v4l, no `decoding` so no C toolchain — §2.6): device list, latest-wins pump thread, RGBA `Frame`s, `SyntheticSource` behind feature `synthetic`; `calibrate` (point-pair fit — affine at 3, normalised-DLT homography at ≥4 — `Calibration`, `StoredCalibration`, the `CalibrationStore` trait). View state only — no laser state, no UI | — | nokhwa, thiserror, log, serde |
| `engine` | `Session`, command registry and `command!` macro, origin policy, snapshot undo, job pipeline (plan → validate → encode → disassemble → summary), device hub, pending ARM, frame requests and submissions, inspect JSON, the one webcam `Capture` (`camera.*` view-state commands) | types, geom, doc, machine, format, import, plan, safety, lmc, gcode, driver, text, camera | serde, serde_json, schemars, thiserror, log |
| `control` | Control-protocol leaf (std only): token, discovery file, bounded line reader, envelope, method table, budgets, `AuthorizedWorkspace` (cap-std), loopback `ControlServer` with the stop fast path | types | serde, serde_json, cap-std, getrandom, craft-atomic, thiserror |
| `automation` | MCP server (rmcp, stdio), headless backend, bridge client, `serve` (JSON lines), tool table and strict schemas | types, doc, machine, plan, engine, control, render, lmc, lmc-sim | rmcp, tokio, schemars, serde, serde_json, base64, thiserror |
| `ui-egui` | The LightBurn-layout shell: `OpenLaserApp`, panels, canvas, menus catalogue and parity, shortcuts, dock, control dispatcher, STOP chrome, `HumanSurfaces` trait | types, geom, doc, machine, plan, engine, control, render, text | egui (`default_fonts`), eframe (`wgpu`, `accesskit`: `OpenLaserApp` implements `eframe::App`), egui_extras (`svg`), craft-ui, craft-settings, craft-atomic, serde, serde_json, log |
| `human` | Human-only surfaces and the input-provenance gate: ARM dialog, frame approval, connect, device-profile save; the synthetic-input injector; implements `HumanSurfaces` | types, ui-egui, arm | egui |
| `testkit` | Dev-only: golden loaders (T0–T4), sha256, adversarial params | types | serde, serde_json, sha2 |
| `apps/openlaser` | GUI binary: eframe, macOS menu (muda), file dialogs (rfd), control server start, sim + (feature `hw`) USB provider, `DirProfileStore`, panic hook and signal stop (§5.4) | types, engine, control, ui-egui, human, lmc-sim, format, [transport if `hw`] | eframe, egui, muda (macOS), rfd, craft-quit, craft-settings, signal-hook (unix), ctrlc (windows), log |
| `apps/openlaser-cli` | Agent CLI: `mcp`, `serve`, `ui`, `commands`, `info`, `convert`, `run`, `plan`, `preview`, `simulate`, `estimate`, `gcode`, `job-export`. Every subcommand runs with `Origin::Headless` and no `ProfileStore` | types, engine, control, automation, render, lmc-sim | serde_json |
| `apps/openlaser-hw` | Human hardware CLI: `detect`, `status`, `frame`, `run`, `bench`, `profile`; TTY ARM; signal and key stop; `--trace`, `--builtin` patterns | types, machine, engine, driver, arm, lmc-sim, format, [transport if `hw`] | crossterm, signal-hook (unix), ctrlc (windows), serde_json |
| `xtask` | `deps`, `parity`, `golden-verify`, `ci` | — | serde_json, sha2 |

```
types ─┬─ geom ─ doc ─ machine ─┬─ format (+ machines, camera-calibration stores; format ─ camera)
       │                        │                                                              arm ─(types)        control ─(types)
       │                        ├─ import                         testkit ─(types, dev only)
       │                        └─ plan ─┬─ safety ─┬─ gcode ─┐
       │                                 │          └─ lmc ───┴─ driver ─┬─ lmc-sim
       │                                 └─ render                       └─ transport   ◄ hardware edge
camera      = (nokhwa input-native + serde; no internal deps — webcam view state + calibration math, never laser state)
engine      = doc machine format import plan safety lmc gcode driver camera (+ types geom)
automation  = engine control render lmc-sim         ui-egui = engine control render
human       = ui-egui arm
openlaser   = ui-egui human engine control lmc-sim format [transport: feature hw]
openlaser-cli = automation engine control render lmc-sim          (no format edge: cannot name DirProfileStore)
openlaser-hw  = engine driver arm lmc-sim machine format [transport: feature hw]
```

### 2.3 Dependency rules (`cargo xtask deps`, plus clippy)

1. Only `transport` may depend on `nusb` or `serialport`. `transport` is never a dev-dependency, and is a normal
   dependency only of `apps/openlaser` and `apps/openlaser-hw`, optional, behind their feature `hw`.
2. Only `ui-egui`, `human` and `apps/openlaser` may depend on egui, eframe, egui_extras, egui_kittest, wgpu,
   craft-ui, muda or rfd.
3. Only `automation` and `apps/openlaser-cli` may depend on tokio, rmcp, hyper, axum or tungstenite.
4. Normal reverse dependencies of `arm` are exactly {`human`, `apps/openlaser-hw`}.
5. The dependency closures of `automation` and `apps/openlaser-cli` contain none of `arm`, `human`, `transport`,
   `nusb`, `serialport`, egui.
6. The closures of `human` and `apps/openlaser-hw` contain none of rmcp, tokio, hyper, axum, tungstenite, rhai,
   extism, pyo3.
7. `ui-egui`'s closure contains neither `arm` nor `human`.
8. `safety` depends on no encoder or driver crate (`lmc`, `gcode`, `driver`, `lmc-sim`, `transport`).
9. `std::process::Command` appears only in `xtask` and `apps/*` (xtask greps the source).
10. Dev-dependencies: `testkit` anywhere; `lmc-sim` in `driver` and in any crate that depends on `driver`;
    `safety` with feature `unchecked-for-tests`, `lmc` with `py-compat`, `arm` with `test-witness` only as
    dev-dependencies (xtask checks every edge that enables them). Each of those features `compile_error!`s without
    `debug_assertions`. **Dev-dependency cycles** (`types`/`geom` ← `testkit`, `driver` ← `lmc-sim`): a crate uses
    such a dev-dependency only from integration tests in `tests/`, never from `#[cfg(test)]` unit tests, where the
    crate under test and the copy the dev-dependency links are two crates whose types do not unify.
11. Clippy `disallowed-methods` (in `clippy.toml`): `openlaser_types::arm::ArmToken::issue` (allowed only in
    `crates/arm/src/mint.rs`), `openlaser_arm::HumanConfirm::from_gui_gesture` and `GuiGestureWitness::new`
    (only `crates/human/src/gesture.rs`), `openlaser_arm::HumanConfirm::from_tty` and `TtyWitness::new` (only
    `apps/openlaser-hw/src/tty_arm.rs`). xtask fails if any other file contains `allow(clippy::disallowed_methods)`.
12. No crate uses `--all-features`-only code paths; nobody **builds** with `--all-features` or `hw`. xtask's
    `cargo metadata` queries with `--features openlaser/hw,openlaser-hw/hw` are allowed (metadata resolves, never
    compiles) so rules 1, 5 and 6 are checked with `hw` on as well as off.
13. `Origin::Human` is constructed only in `crates/ui-egui`, `crates/human`, `apps/openlaser` and
    `apps/openlaser-hw` (xtask grep): `automation` and `openlaser-cli` always use `Origin::Headless`, so even a
    `run --cmd` subcommand can never pass the human-only policy.
14. `safety::testing`, `arm::testing` and `lmc::py_compat` are referenced only from `tests/` directories and
    `#[cfg(test)]` modules (xtask grep). Feature unification can compile them into a binary built by
    `cargo test --workspace`; this rule keeps any non-test call from compiling in a normal build.
15. `crates/transport` never calls `transfer_blocking`, `reset`, `clear_halt` or `detach_and_claim_interface`
    (xtask grep; PLAN §3.5: no USB reset, no unbounded blocking transfer).
16. In `crates/ui-egui`, `egui::Event` values (`PointerButton`, `PointerMoved`, `Key`, `Text`, `Paste`,
    `AccessKitActionRequest`) are constructed only in `src/control/input.rs`, which builds the `Vec<Event>` handed to
    `HumanSurfaces::inject`, and in tests (xtask grep). No other path pushes synthetic input into egui.
17. `format::machines::DirProfileStore` is named only in `apps/openlaser` and `apps/openlaser-hw` (xtask grep);
    `engine` does not re-export it (S24).

### 2.4 External crates (pinned; versions verified on crates.io 2026-10-23)

Exact `=` pins for everything that shapes bytes, geometry, UI or protocols; caret for ubiquitous infrastructure.
`Cargo.lock` is committed by WP0 and is the real pin: work packages never run `cargo update` and never add a
dependency that is not already in `[workspace.dependencies]` (ask the integrator).

| Crate | Pin | Licence | Used by | Note |
|---|---|---|---|---|
| egui, eframe, egui_extras, egui_kittest | `=0.36.2` | MIT/Apache | ui-egui, human, apps/openlaser | craft-ui requires 0.36; 0.37 waits for craft-libs. Features (PhotoCraft's): egui `default-features = false` + `default_fonts`; eframe `default-features = false` + `wgpu`, `accesskit` (the app adds `persistence`); egui_extras `default-features = false` + `svg`; egui_kittest dev-only with `eframe` (no `wgpu`/`snapshot` this build, §6.4). eframe 0.36's `App` has `logic`, `ui` and `raw_input_hook` (checked in `epi.rs`) |
| craft-ui | git `storytold/craft-libs` rev `5c874d51eeb3de70e735c8ce87a59a7dedd47675`, feature `serde` | MIT/Apache | ui-egui | PhotoCraft's exact pin; repo reachable from this Mac |
| craft-atomic, craft-settings, craft-quit | same repo, rev `3d37da389a3618ab9b76109da11416e9ca4ea719` | MIT/Apache | format, control, ui-egui, apps/openlaser | craft-quit with feature `egui` only (its `menu` feature needs muda 0.21.2) |
| muda | `=0.21.1` | MIT/Apache | apps/openlaser (macOS) | PhotoCraft's pin |
| rfd | `=0.17.2` | MIT | apps/openlaser | async dialogs only; disabled while the machine is active |
| kurbo | `=0.11.3` | MIT/Apache | geom, doc | one kurbo with usvg 0.45 (S7) |
| usvg | `=0.45.1`, `default-features = false` | MIT/Apache | import | true-scale SVG; no text in P0 |
| tiny-skia | `=0.11.4` | BSD-3 | render | deterministic PNG |
| rstar | `=0.13.0` | MIT/Apache | geom | hit-testing |
| image | `=0.25.10`, features png, jpeg, bmp, tiff, webp, gif | MIT/Apache | import | |
| zip | `=9.0.5`, `default-features = false`, feature `deflate-flate2` | MIT | format | `deflate-flate2` alone selects **no** flate2 backend (zip declares flate2 with `default-features = false`, and flate2 then hits `compile_error!("You need to choose a zlib backend")`); the flate2 row below selects it |
| flate2 | `=1.1.10`, `default-features = false`, feature `rust_backend` | MIT/Apache | format | backend selection for zip only (pure Rust miniz_oxide, no C toolchain, the same backend PhotoCraft's lock and `png` use) |
| dxf | `=0.6.2` | MIT | — | declared for P1 DXF export; the P0 importer is the OM-parser port |
| skrifa, harfrust | `=0.44.0`, `=0.12.0` | OFL-1.1/MIT, MIT | text | skrifa reads font tables and glyph outlines (the `Name` family/style index, OS/2 metrics, `OutlinePen`); harfrust (a pure-Rust HarfBuzz port) shapes each line (kerning, ligatures, advances). Already locked via the GUI stack — no new packages |
| i_overlay, quick-xml | `=9.1.0`, `=0.42.0` | MIT/Apache, MIT | — | i_overlay drives geom booleans and offsets (single-threaded; `default-features = false`); quick-xml declared for P1 `.lbrn2`, unused in this build |
| nusb | `=0.2.8` | MIT/Apache | transport | no libusb. API used (checked on docs.rs and the v0.2.8 source): `list_devices()` (IORegistry on macOS, never opens), `DeviceInfo::open`, `Device::active_configuration`/`set_configuration`, `claim_interface(0)`, `Interface::endpoint` Bulk OUT `0x02` / IN `0x88`, `Endpoint::submit` + `wait_next_complete(≤ 20 ms)`, `cancel_all`, `pending`. On macOS `claim_interface` fails "interface not found" on an unconfigured device (§3.15 `usb_set_configuration`). Never `transfer_blocking`, `reset`, `clear_halt` (§2.3 rule 15) |
| serialport | `=4.10.1`, `default-features = false` | MPL-2.0 | transport | |
| nokhwa | `=0.10.11`, `default-features = false`, features `input-native`, `camera-sync-impl` | Apache-2.0 | camera | Native backends only (nokhwa-bindings-macos → AVFoundation via pure-Rust `objc`, nokhwa-bindings-windows → MSMF via the already-locked `windows` 0.62, nokhwa-bindings-linux → v4l). No `decoding`/`mozjpeg`, no `opencv`, no `wgpu`: nothing needs a C toolchain on Windows (§2.6). API used (0.10.11 source): `query(ApiBackend::Auto)`, `Camera::new(CameraIndex::Index, RequestedFormat::with_formats)`, `open_stream`, `frame`, `stop_stream`, `Buffer::decode_image::<RgbAFormat>` (pure-Rust YUYV/NV12/GRAY/raw paths; MJPEG refuses — phase 6a ships no MJPEG decoder). `Camera` is `Send` (camera.rs:582), moved onto the pump thread |
| rmcp | `=3.5.0`, features server, macros, transport-io | Apache-2.0 | automation | PhotoCraft's lockfile version (3.7.0 is newest; stay on PhotoCraft's until the Craft apps move) |
| tokio | `1` (lock: 1.53.1) | MIT | automation, apps/openlaser-cli | |
| schemars | `=1.2.2` | MIT | engine, automation | |
| cap-std | `=4.0.3` | Apache/MIT | control | |
| serde, serde_json (`float_roundtrip`), thiserror `2`, log `0.4` | caret | MIT/Apache | all | `float_roundtrip` is needed to read the goldens exactly |
| blake3 | `=1.8.7`, `default-features = false`, feature `pure` | CC0/Apache | format, plan, import, lmc, gcode | `bytes_hash` (S22) in lmc and gcode |
| getrandom | `=0.3.4` | MIT/Apache | arm, control | |
| crossbeam-channel | `=0.5.17` | MIT/Apache | driver, lmc-sim | |
| base64 | `=0.22.1` | MIT/Apache | automation | |
| crossterm | `=0.29.0` | MIT | apps/openlaser-hw | raw-mode key thread (Esc/Space/Ctrl-C) |
| signal-hook | `=0.4.5` (target `cfg(unix)`) | MIT/Apache | apps/openlaser-hw, apps/openlaser | SIGINT/SIGTERM/SIGHUP set an `AtomicBool` (`signal_hook::flag::register`, async-signal-safe); a watcher thread then trips the latch and waits for the StopReport before exiting (§5.4) |
| ctrlc | `=3.5.2`, feature `termination` (target `cfg(windows)`) | MIT/Apache | apps/openlaser-hw, apps/openlaser | Ctrl-C/Break/console close trip the latch (§2.6) |
| keepawake | `=0.6.1`, `default-features = false` | MIT | transport | keep the machine awake during a session on macOS, Windows, Linux (§2.6) |
| sha2 | `=0.11.0` | MIT/Apache | testkit, xtask | golden packet file names are sha256 |
| proptest, insta (feature json), tempfile | `=1.12.0`, `=1.49.0`, `=3.27.0` | MIT/Apache, Apache | dev-dependencies | |

The board lock uses `std::fs::File::try_lock` (stable since 1.89); no extra crate.

### 2.5 Toolchain, lints, profiles

- `rust-toolchain.toml`: `channel = "stable"`, components rustfmt, clippy. This Mac's only installed toolchain is
  `stable-aarch64-apple-darwin` = rustc 1.99.0; rustup does not alias `"1.99.0"` to it, so pinning the number would
  make rustup download a second ~1 GB toolchain into a tight disk. The gate checks `rustc --version` ≥ 1.99.
  Workspace `rust-version = "1.95"` (craft-libs and egui 0.36 need 1.95; zip 9 and rmcp 3.5 need 1.88, nusb 0.2.8
  1.85). `export PATH="$HOME/.cargo/bin:$PATH"`.
- Root `Cargo.toml` is a virtual manifest with **`resolver = "3"`** stated explicitly (a virtual manifest has no
  edition, and without it Cargo falls back to resolver 1, which would unify dev-only features such as
  `unchecked-for-tests` and `test-witness` into normal builds of the binaries).
- `.cargo/config.toml`: `[alias] xtask = "run -p xtask --"`.
- `rustfmt.toml` and `clippy.toml` exist (Craft style: `max_width = 160`, `use_small_heuristics = "Max"`;
  unwrap/expect/panic/indexing allowed in tests). WP0 adds `disallowed-methods` (§2.3 rule 11).
- Workspace lints:

```toml
[workspace.lints.rust]
unsafe_code = "forbid"
unused_must_use = "deny"

[workspace.lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
unreachable = "deny"
indexing_slicing = "deny"
dbg_macro = "deny"
disallowed_methods = "deny"
```

  Encoding crates (`types`, `lmc`, `gcode`) add `#![deny(clippy::cast_possible_truncation, clippy::cast_sign_loss,
  clippy::cast_possible_wrap)]`: every narrowing goes through `u16::try_from` or an explicit, commented rule.
- Profiles (PhotoCraft's, plus explicit unwind). There is no `[profile.test] panic` key: Cargo ignores `panic`
  for the test profile (tests always unwind) and warns about it.

```toml
[profile.dev]
opt-level = 1
debug = "line-tables-only"
panic = "unwind"
[profile.dev.package."*"]
opt-level = 2
[profile.release]
lto = "thin"
codegen-units = 1
panic = "unwind"
```

### 2.6 Windows is required (S19)

Luiz requires OpenLaser to work on **Windows 10/11 (x64, and ARM64 where the toolchain allows)** as well as macOS.
Linux is kept compiling. This overrides the "Windows/Linux" entries of §1.2 for the code; installers and code
signing stay later work.

1. **Every crate builds for Windows.** The gate (§6.1) adds
   `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings` (check only, from the
   Mac; the targets are installed with rustup). No crate may depend on a C toolchain for the target (blake3 `pure`,
   zip with `deflate-flate2` plus flate2 `rust_backend`, no `ring`/`openssl`). `cargo xtask ci` runs it. Disk: the
   target's standard library is a rustup download (a few hundred MB) and its check artifacts live under
   `<lane>/x86_64-pc-windows-msvc/` (a few GB), so only the integrator runs this check, in the gui lane, once per
   integration step (§6.7), after `df` shows ≥ 8 GB free.
2. **Real Windows CI.** WP0 writes `.github/workflows/ci.yml` modelled on PhotoCraft's: `macos-latest`,
   `windows-latest` and `ubuntu-latest` run fmt, clippy, `cargo test --workspace` (default features, never `hw`)
   and `cargo xtask deps`. It runs once Luiz approves a private GitHub repository.
3. **Platform seams, never `#[cfg(unix)]`-only features:**
   - Paths: per-user app dirs from `craft-settings` (`%APPDATA%\OpenLaser` on Windows,
     `~/Library/Application Support/OpenLaser` on macOS). Never build paths with `/` literals; use `Path::join`.
     Device state that OM also reads (the board lock), the bench session and machine profiles use
     `craft_settings::data_dir("OpenLaser")` (`dirs::data_local_dir`: `%LOCALAPPDATA%\OpenLaser` on Windows, which
     is OM v1's board-lock folder; `~/Library/Application Support/OpenLaser` on macOS). OM v1 locks with `flock`,
     which is what `File::try_lock` uses on Unix, so the two lock each other out.
   - Secret files (control token, discovery): on Unix `0700` dir + `0600` file; on Windows the per-user AppData
     folder (its default ACL is user-only) plus `FILE_ATTRIBUTE_HIDDEN` is acceptable. Tests are
     `#[cfg(unix)] token_file_mode_0600` and `#[cfg(windows)] token_file_in_user_appdata`.
   - Keep-awake during a job: the **`keepawake =0.6.1`** crate (macOS IOKit assertion, Windows
     `SetThreadExecutionState`, Linux inhibit), held by `transport` for the session. It replaces `caffeinate`, so
     rule 9 of §2.3 drops `transport` (no `std::process::Command` there).
   - Stop on signals in `openlaser-hw`: `signal-hook` only under `cfg(unix)` (SIGINT/SIGTERM/SIGHUP); under
     `cfg(windows)` the **`ctrlc =3.5.2`** crate with feature `termination` (Ctrl-C, Ctrl-Break, console close).
     Both trip the same latch. crossterm raw-mode keys (Esc/Space/Ctrl-C) work on both.
   - Board lock: `std::fs::File::try_lock` (works on Windows).
   - Native menu: muda on macOS; on Windows the in-window egui menu bar (the `--in-window-menus` path) is the
     default, matching LightBurn's Windows look.
   - Line endings: G-code and text exports are written with `\n` on every OS; goldens are compared as bytes;
     `.gitattributes` marks `tests/golden/**` binary-safe (`-text`) so Windows checkouts do not rewrite them.
4. **USB on Windows.** nusb talks to the LMC board only through **WinUSB**. The board normally carries EZCAD's
   driver, so Windows users switch it once with Zadig (USBLMCV4, `9588:9899` → WinUSB), exactly as LightBurn's
   fallback and the Python prototype's README describe; EZCAD then cannot use the board until the driver is put
   back. `transport::detect` on Windows reports "found, but not on WinUSB" with those steps when the device is
   present without a WinUSB interface. A libwdi helper is later work.
5. **Serial on Windows.** `serialport` names ports `COMn`; GRBL device settings accept them.
6. **Fonts/text:** none in P0. **File dialogs:** rfd uses the native Windows dialogs.

---

## 3. Interface contracts

### 3.0 Rules for the contracts

- **WP0 pastes these blocks into the crates.** Data types are written in full (fields, derives, serde attributes).
  Function bodies are stubs that never panic **and fail closed** (§1.3 rule 7):
  - a function returning `Result<_, E>` returns `Err(E::NotImplemented("<path>"))`. Every error enum in §3 carries
    that variant during this build, including `UnitError`, `EncodeError`, `XferError`, `LinkError`, `BridgeError`
    and `EngineError`; I3 removes it. A function whose error type is `Refusal`, `Refused` or `JobRefused` returns a
    refusal with `RefusalCode::NotImplemented` (also removed at I3);
  - a function returning `bool` returns the refusing value (`is_human_click` → false, `frame_is_synthetic` → true,
    `surface_open` → true, `covers_job` → false); `Option` returns `None`; collections are empty; numbers 0;
  - **implemented in full by WP0, not stubbed**, because other packages' tests need them before WP3 merges:
    `Refusal` builders, `safety::testing::*`, `arm::testing::*` (including `token_for_tests`, written in
    `crates/arm/src/mint.rs` so the `ArmToken::issue` allowance stays in one file), and testkit's path/hex/sha256
    helpers.

  The workspace must pass `cargo clippy --all-targets -- -D warnings` in both lanes (§6.7) with only stubs.
- `Fingerprint` (and anything else whose JSON form §3 states explicitly) gets **manual** `Debug`, `Display`,
  `Serialize` and `Deserialize` impls; every struct that contains it may then derive `Debug`/`Serialize` as listed.
- **Durations in JSON.** A `Duration` field that crosses a process boundary is serialised as f64 seconds under a key
  ending in `S` (`#[serde(rename = "estimateS", with = "openlaser_types::serde_secs")]`) or as u64 milliseconds
  under a key ending in `Ms` (`serde_ms`), never as serde's default `{secs, nanos}` (S2: keys carry their unit).
  This covers every `Duration` field in §3 that derives `Serialize` (`PassRow`, `EmissionSummary`, `StopReport`,
  `Outcome::Finished`, `FrameOutcome`, budgets echoed in results); `MonoTime` gets manual impls as f64 seconds.
- Signatures below are normative. A work package may **add** public items freely. Changing or removing a listed
  item is a **contract change**: write it at the top of the PR description under `CONTRACT CHANGE`, and it lands
  only through the integrator.
- Common derives unless stated: `Debug, Clone, PartialEq`; types that cross a process boundary also
  `Serialize, Deserialize` with `#[serde(rename_all = "camelCase")]` (enums: `snake_case`). Types marked
  **(sealed)** have private fields and no `Deserialize`; types marked **(no Clone)** must not derive `Clone`,
  `Copy`, `Default`, `Serialize` or `Deserialize`.
- Coordinates are `f64` millimetres, Y up. In `doc` they are design coordinates; from `plan` on they are
  **machine coordinates** (after the machine transform: field angle, axis swap, negate, offset). Galvo units appear
  only inside `types::galvo`, `lmc`, `lmc-sim` and `transport`.
- Every crate has `pub type Result<T, E = Error> = std::result::Result<T, E>;` and one `Error` enum
  (`thiserror`), except where a block below names something else.
- A crate re-exports (`pub use`) every dependency type that appears in its own public signatures, so callers can
  name them without a direct dependency (the engine re-exports `Validated`, `LmcJob`, `Listing`, `StopAck`,
  `MachineState`…; this keeps §2.2's edge list short).

### 3.1 `openlaser-types` (owned by WP1; `src/arm.rs` by WP3; `src/galvo.rs` by WP4)

```rust
// ---- units.rs -------------------------------------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum UnitError {
    #[error("{what} must be a finite number")]
    NonFinite { what: &'static str },
    #[error("{what} = {value} is outside {min}..={max}")]
    OutOfRange { what: &'static str, value: f64, min: f64, max: f64 },
    #[error("{what} = {value} is not a whole number of hertz")]
    NotWholeHz { what: &'static str, value: f64 },
    #[error("not implemented: {0}")]
    NotImplemented(&'static str), // build-only (§3.0); removed at I3
}
// Each f64 newtype has: new(v) -> Result<Self, UnitError>; get(self) -> f64; TryFrom<f64>; From<Self> for f64;
// #[serde(try_from = "f64", into = "f64")]; derives Debug, Clone, Copy, PartialEq, PartialOrd.
pub struct Mm(f64);      // finite, any sign
pub struct MmPerS(f64);  // finite, 0 < v <= 100_000
pub struct Percent(f64); // finite, 0..=100
pub struct Ns(f64);      // finite, 0 < v <= 100_000; requested pulse width, snapped by the planner
pub struct Us(f64);      // finite, -100_000..=100_000; delays (laser-on TC may be negative)
pub struct Ms(f64);      // finite, 0..=600_000
pub struct Deg(f64);     // finite

/// Frequency in whole hertz (as `.lbrn2` stores it); the UI shows kHz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct Hz(u32); // 1..=10_000_000
impl Hz {
    pub fn new(hz: u32) -> Result<Self, UnitError>;
    pub fn from_khz(khz: f64) -> Result<Self, UnitError>; // refuses non-finite, <= 0 and non-whole Hz; never rounds
    pub fn get(self) -> u32;
    pub fn khz(self) -> f64; // hz as f64 / 1000.0: the value the encoders take
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgb(pub [u8; 3]);

// ---- ids.rs ---------------------------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObjectId(pub u64);

/// A LightBurn layer: colour 00..=29 or tool layer T1/T2. JSON: 0..=29, or "C05", "05", "T1", "T2"
/// (case-insensitive). Display: "C05", "T1".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LayerKey { Color(u8), Tool(u8) }
impl LayerKey {
    pub fn color(index: u8) -> Result<Self, UnitError>; // 0..=29
    pub fn tool(which: u8) -> Result<Self, UnitError>;  // 1 | 2
    pub fn is_tool(self) -> bool;
    pub fn palette_slot(self) -> usize; // 0..=31 (T1 = 30, T2 = 31)
}
// impl Display, FromStr, Serialize, Deserialize for LayerKey

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MachineId(pub String);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub u64);

/// blake3 of a canonical encoding (plan fingerprint, S21) or of wire bytes (`bytes_hash`, S22). JSON: 64 lowercase
/// hex. Display, Debug and `short()`: first 6 hex digits. Debug/Display/Serialize/Deserialize are MANUAL impls (§3.0).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint(pub [u8; 32]);
impl Fingerprint {
    pub fn hex(&self) -> String;
    pub fn short(&self) -> String;
    pub fn from_hex(s: &str) -> Result<Self, UnitError>;
}

/// Moved here from `doc` (driver, plan, safety and lmc all name it; driver has no `doc` edge). `doc` re-exports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum DeviceKind { GalvoLmc, Grbl }

// ---- serde_dur.rs: `serde_secs` (f64 seconds) and `serde_ms` (u64 ms) `with` modules for Duration (§3.0) ---------
pub mod serde_secs { /* serialize(&Duration, S) / deserialize(D) -> Duration; refuses negative and non-finite */ }
pub mod serde_ms { /* … */ }

// ---- time.rs --------------------------------------------------------------------------------------------
/// Monotonic time from a `Clock`; virtual in tests. Never wall-clock. Serialize/Deserialize are manual (f64 s, §3.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MonoTime(pub std::time::Duration);
impl MonoTime {
    pub fn saturating_since(self, earlier: MonoTime) -> Duration;
    pub fn plus(self, d: Duration) -> MonoTime;
}
pub trait Clock: Send + Sync {
    fn now(&self) -> MonoTime;
    /// Waits at most `d`. A virtual clock advances by `d` instead of sleeping.
    fn sleep(&self, d: Duration);
}
pub struct SystemClock { /* origin: std::time::Instant */ }
impl SystemClock { pub fn new() -> Self; }
impl Clock for SystemClock { /* … */ }
pub struct VirtualClock { /* now: std::sync::Mutex<Duration>; a poisoned lock is recovered with into_inner */ }
impl VirtualClock { pub fn new() -> Self; pub fn advance(&self, d: Duration); }
impl Clock for VirtualClock { /* … */ }

// ---- refusal.rs -----------------------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RefusalCode {
    // geometry
    OutsideWorkArea, NotEncodable, NonFinite, MalformedPoint, TooManyPaths, TooManyPointsPerPath,
    TooManyPointsPerPass, OutOfBed,
    // laser parameters
    PowerAboveMax, FrequencyOutOfRange, PulseNotInTable, QPulseUnsupported, SpeedZeroUnits, SpeedAboveMax,
    // objects and layers
    OrphanObject, HiddenLayerWithOutput, FillWithoutHatch, NothingToMark, UnsupportedMode, WrongDeviceKind,
    // job flow and ARM
    MachineMismatch, Stale, FrameRequired, FrameReceiptExpired, FrameNotHuman, FrameDoesNotCoverJob,
    ArmTextMismatch, ChecklistIncomplete, ArmExpired, ArmNonceUsed, ArmSessionMismatch,
    BytesMismatch,          // S22: the job handed to the driver is not the bytes the human confirmed
    StoppedAfterQueue,      // §5.3: a stop tripped after this command was queued cancelled it
    // access and devices
    HumanOnly, NeedsBridge, NoDevice, NotConnected, MachineBusy, HumanConfirmationOpen, HardwareDisabled,
    BenchSessionRequired, BoardLocked, CloneBoard, NotATty, GrblPreflight,
    BenchPowerCap,          // S23: first emission of a bench session above 10 %
    EmissionNotAllowed,     // `openlaser-hw run` without --allow-emission (red light only)
    UsbNotConfigured,       // macOS: device unconfigured and the profile's usb_set_configuration is off (H12)
    // build-only (§3.0); removed at I3
    NotImplemented,
}
impl RefusalCode { pub fn as_str(self) -> &'static str; } // the snake_case wire string; stable forever

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Refusal {
    pub code: RefusalCode,
    pub message: String,         // one sentence the user can act on, naming the value and the limit
    pub object: Option<ObjectId>,
    pub layer: Option<LayerKey>,
    pub at: Option<[f64; 2]>,    // mm, machine frame
}
impl Refusal {
    pub fn new(code: RefusalCode, message: impl Into<String>) -> Self;
    pub fn object(self, id: ObjectId) -> Self;
    pub fn layer(self, k: LayerKey) -> Self;
    pub fn at(self, p: [f64; 2]) -> Self;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NoticeCode {
    OpenShapeUnhatched, PulseSnapped, SpeedQuantized, ToolLayerNotBurned,
    ImportEntitySkipped, ImportColorUnmatched, ImportTextIgnored,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice { pub n: u32, pub code: NoticeCode, pub message: String, pub object: Option<ObjectId>, pub layer: Option<LayerKey> }

// ---- emission.rs ----------------------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds { pub min: [f64; 2], pub max: [f64; 2] }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionMode { Line, Fill, Image }

/// Words read back from a marking list (galvo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GalvoWords { pub mark_current: u16, pub q_period: u16, pub pulse_ns: u16, pub speed_units: u16, pub jump_speed_units: u16 }

/// One emitting layer-pass. Every number is decoded from the bytes that will be sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PassRow {
    pub layer: LayerKey,
    pub layer_name: String,
    pub sub: u8,
    pub pass: u16,
    pub of: u16,
    pub mode: SectionMode,
    pub power_pct: f64,              // galvo: mark_current × 100 / 4095; gantry: S / s_max × 100
    pub frequency_khz: Option<f64>,  // galvo: 20000 / q_period
    pub pulse_ns: Option<u16>,
    pub speed_mm_s: f64,             // the ACTUAL speed (galvo: from speed units and F)
    pub galvo: Option<GalvoWords>,
    pub paths: u32,
    pub mark_mm: f64,
    pub bounds: Bounds,
    #[serde(rename = "estimateS", with = "crate::serde_secs")]
    pub estimate: Duration,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmissionSummary {
    pub fingerprint: Fingerprint,
    /// S22: blake3 over, in send order, every single command (12 bytes) and packet (3072 bytes) of one repeat,
    /// then the repeat count as u16 LE. Computed from the job's bytes, never from the plan.
    pub bytes_hash: Fingerprint,
    pub machine: MachineId,
    pub rows: Vec<PassRow>,
    pub notices: Vec<Notice>,
    pub repeats: u16,
    #[serde(rename = "estimateS", with = "crate::serde_secs")]
    pub estimate: Duration,   // all repeats
    #[serde(rename = "crashBoundS", with = "crate::serde_secs")]
    pub crash_bound: Duration,// emission that can still happen after a host crash (run-ahead bound, PLAN §3.5)
    pub packets: u32,
    pub work_area: Bounds,
    pub negate: [bool; 2],    // as applied by the machine transform
    pub swap_xy: bool,
}
/// A burned segment decoded from the bytes (MarkTo while MO is open and laser port bit 0 is set), machine mm.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EmittedSegment { pub a: [f64; 2], pub b: [f64; 2], pub power_pct: f64 }

// ---- frame.rs -------------------------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum FrameStyle { Bounds, Contour, Hull } // Hull is P1: the planner refuses it in this build
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum FrameSource { Job, ToolLayersOnly, JobAndToolLayers }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum FrameMode { Once, Loop }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum FrameOrigin { Human, Agent } // Agent is reserved; nothing in this build creates it
pub const FRAME_RECEIPT_MAX_AGE: Duration = Duration::from_secs(600);
/// Issued by the driver only after the board has EXECUTED the pass (S20): a status read shows !BUSY after the list
/// that carried it. Acknowledged-but-queued packets never count. `fingerprint` is the plan fingerprint (S21, style
/// and source excluded); `style` and `source` record what was traced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameReceipt {
    pub fingerprint: Fingerprint,
    pub machine: MachineId,
    pub session: SessionId,
    pub full_passes: u32,
    pub at: MonoTime,
    pub style: FrameStyle,
    pub source: FrameSource,
    pub origin: FrameOrigin,
}
impl FrameReceipt { pub fn covers_job(&self) -> bool; } // source != ToolLayersOnly

// ---- arm.rs (WP3, red zone) -----------------------------------------------------------------------------
pub const ARM_TOKEN_TTL: Duration = Duration::from_secs(60);
/// The only key to emission. (no Clone) Private fields. Consumed by value by `DriverHandle::run`.
/// `bytes` = `EmissionSummary.bytes_hash` of the summary the human confirmed (S22). `nonce` is a fresh 64-bit
/// getrandom value for single use, independent of the 6-digit TTY challenge.
pub struct ArmToken { fingerprint: Fingerprint, bytes: Fingerprint, machine: MachineId, session: SessionId, expires: MonoTime, nonce: u64 }
impl ArmToken {
    /// clippy `disallowed-methods` everywhere except crates/arm/src/mint.rs.
    #[doc(hidden)]
    pub fn issue(fingerprint: Fingerprint, bytes: Fingerprint, machine: MachineId, session: SessionId, expires: MonoTime, nonce: u64) -> Self;
    /// Refusals: Stale (fingerprint), BytesMismatch, MachineMismatch, ArmSessionMismatch, ArmExpired.
    pub fn check(&self, fp: &Fingerprint, bytes: &Fingerprint, machine: &MachineId, session: SessionId, now: MonoTime) -> Result<(), Refusal>;
    pub fn nonce(&self) -> u64;
    pub fn fingerprint(&self) -> &Fingerprint;
    pub fn bytes(&self) -> &Fingerprint;
    pub fn machine(&self) -> &MachineId;
}
// impl Debug for ArmToken: fingerprint short + expiry only, never the nonce.
```

```rust
// ---- galvo.rs (WP4): the T0 encoders. Python operation order; tests read tests/golden/t0_numeric ----------
pub const CENTER: u16 = 0x8000;
pub const JPT_M7_PULSES_NS: [u16; 20] = [2, 4, 6, 9, 13, 20, 30, 45, 55, 60, 80, 100, 150, 200, 250, 300, 350, 400, 450, 500];
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EncodeError {
    #[error("{what} is not finite")]
    NonFinite { what: &'static str },
    #[error("{what} = {value} does not fit a 16-bit word")]
    OutOfRange { what: &'static str, value: f64 },
    #[error("not implemented: {0}")]
    NotImplemented(&'static str), // build-only (§3.0); removed at I3
}
/// `(32768.0 + v * (65535.0 / f)).round_ties_even()`, then `u16::try_from`. Never clamps: +F/2 gives 65536 → Err.
pub fn mm_to_galvo(v_mm: f64, field_mm: f64) -> Result<u16, EncodeError>;
/// OM/GP behaviour: the same, clamped to 0..=65535. T0 and py-compat only.
pub fn mm_to_galvo_om(v_mm: f64, field_mm: f64) -> u16;
/// `(p * 4095.0 / 100.0).round_ties_even()`; Err outside 0..=65535 (30 % → 1228, 70 % → 2866).
pub fn power_to_mark_current(pct: f64) -> Result<u16, EncodeError>;
/// Shipping: `(20000.0 / khz).round_ties_even()`; Err if non-finite, <= 0, or > 65535. Never wraps.
pub fn frequency_to_period(khz: f64) -> Result<u16, EncodeError>;
/// GP: `round_ties_even(20000 / khz) & 0xFFFF` including the negative wrap; Err only for khz == 0. T0/py-compat.
pub fn frequency_to_period_om(khz: f64) -> Result<u16, EncodeError>;
/// `let gpm = 65535.0 / f; min(trunc(v * gpm / 1000.0), 65535)`; Err for negative or non-finite v.
/// 0 is a legal word here; `safety` refuses a mark speed of 0 units (B7).
pub fn speed_to_units(mm_s: f64, field_mm: f64) -> Result<u16, EncodeError>;
/// Display only: `units * f * 1000 / 65535`.
pub fn units_to_speed(units: u16, field_mm: f64) -> f64;
/// `(trunc(|d|), if d > 0 { 0x0000 } else { 0x8000 })`; Err if trunc(|d|) > 65535. 0 → (0, 0x8000); 0.4 → (0, 0).
pub fn delay_word(d: f64) -> Result<(u16, u16), EncodeError>;
/// `min(trunc(hypot(dx, dy)), 65535)` on integer galvo units.
pub fn distance_word(last: (u16, u16), to: (u16, u16)) -> u16;
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PulseDecision { Exact(u16), Snapped { requested: f64, to: u16 }, Refused { requested: f64, nearest: u16 } }
/// Ascending scan, first minimum wins ties; |Δ| <= 0.5 → Snapped; NaN or ±inf → Refused (divergence: OM snaps NaN).
pub fn nearest_pulse_width(ns: f64, table: &[u16]) -> PulseDecision;
/// OM decision including the NaN quirk. T0 only.
pub fn nearest_pulse_width_om(ns: f64, table: &[u16]) -> PulseDecision;
```

### 3.2 `openlaser-geom` (WP1)

```rust
pub use kurbo::{Affine, BezPath, PathEl, Point, Rect, Vec2};
pub use openlaser_types::emission::Bounds;
pub type P2 = [f64; 2];

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Polyline(pub Vec<P2>);
impl Polyline {
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn length_mm(&self) -> f64;
    pub fn bounds(&self) -> Option<Bounds>;
    /// OM `is_closed`: at least 3 points and |first − last| <= tol.
    pub fn is_closed(&self, tol_mm: f64) -> bool;
    pub fn transformed(&self, a: &Affine) -> Polyline;
    pub fn all_finite(&self) -> bool;
}
pub const CLOSURE_TOL_MM: f64 = 0.05;         // OM raster closure tolerance
pub const DEFAULT_FLATTEN_TOL_MM: f64 = 0.01; // pref `output.curveToleranceMm`

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("line interval must be > 0 (got {0})")] BadInterval(f64),
    #[error("non-finite coordinate")] NonFinite,
    #[error("hatch would produce {0} lines, above the {1} limit")] TooManyLines(usize, usize),
    #[error("not implemented: {0}")] NotImplemented(&'static str),
}

/// One polyline per subpath; ClosePath repeats the first point. Tolerance in mm.
pub fn flatten(path: &BezPath, tol_mm: f64) -> Vec<Polyline>;
pub fn bounds_of(polys: &[Polyline]) -> Option<Bounds>;
/// Auto-Join (OM `geometry.join_lines` port): chains ends closer than tol.
pub fn join_lines(polys: &[Polyline], tol_mm: f64) -> Vec<Polyline>;
/// Close Path: closes open paths whose ends are within tol; returns how many were closed.
pub fn close_paths(polys: &[Polyline], tol_mm: f64) -> (Vec<Polyline>, usize);
/// Delete Duplicates: indices of polylines equal (within tol, either direction) to an earlier one.
pub fn duplicates(polys: &[Polyline], tol_mm: f64) -> Vec<usize>;

/// Exact port of OM `raster.hatch_polylines` (T3: segment count and order exact, coordinates within 1e-9).
pub fn hatch_om(polys: &[Polyline], angle_deg: f64, pitch_mm: f64, passes: u32, angle_step_deg: f64, closure_tol_mm: f64) -> Result<OmHatch, Error>;
#[derive(Debug, Clone, PartialEq)]
pub struct OmHatch { pub segments: Vec<[P2; 2]>, pub closed_input_count: usize, pub skipped_open_count: usize, pub angles_deg: Vec<f64> }

/// LightBurn Fill. Pass i uses angle + i × angle_increment; `bidirectional` = OM boustrophedon (identical
/// segments to `hatch_om` for that pass); one-way flips every other row back; `cross_hatch` adds a +90° pass
/// after each pass. Even-odd over all closed inputs; open inputs are reported, never dropped silently.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HatchParams { pub interval_mm: f64, pub angle_deg: f64, pub passes: u16, pub angle_increment_deg: f64, pub bidirectional: bool, pub cross_hatch: bool, pub closure_tol_mm: f64, pub max_lines: usize }
#[derive(Debug, Clone, PartialEq)]
pub struct HatchResult { pub passes: Vec<Vec<[P2; 2]>>, pub angles_deg: Vec<f64>, pub closed_used: usize, pub open_inputs: Vec<usize> }
pub fn hatch(polys: &[Polyline], p: &HatchParams) -> Result<HatchResult, Error>;

/// Even-odd point-in-region test over closed polylines (used by the frame ⊇ burn property test).
pub fn inside_even_odd(closed: &[Polyline], p: P2) -> bool;

/// R-tree over polyline segments for canvas picking (world mm).
pub struct HitIndex { /* rstar::RTree<…> */ }
impl HitIndex {
    pub fn build<'a>(items: impl IntoIterator<Item = (ObjectId, &'a [Polyline])>) -> Self;
    pub fn pick(&self, at: P2, tol_mm: f64) -> Option<ObjectId>;          // nearest segment within tol
    pub fn in_rect(&self, r: Bounds, crossing: bool) -> Vec<ObjectId>;   // crossing=false: fully enclosed
}
```

### 3.3 `openlaser-doc` (WP1)

```rust
pub const FORMAT_VERSION: u32 = 1;
pub const COLOR_LAYERS: usize = 30;
/// Display colours of palette 00..=29 and T1, T2. Values from LightBurn's public palette documentation
/// (interoperability: SVG colours land on the same layers); provenance recorded in crates/doc/PROVENANCE.md.
pub const PALETTE: [Rgb; 30];
pub const TOOL_COLORS: [Rgb; 2];

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub name: String,
    pub shapes: Vec<Arc<Shape>>,       // draw order: index 0 first (bottom)
    pub layers: CutTable,
    pub workspace: Workspace,
    pub device_hint: Option<DeviceHint>,
    pub job: JobSettings,
    pub notes: String,
    pub next_id: u64,                  // per project: ids are deterministic
    pub extras: BTreeMap<String, serde_json::Value>, // unknown foreign keys, kept for round trips
}
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub id: ObjectId,
    pub name: Option<String>,
    pub layer: LayerKey,
    pub xform: Affine,                 // object space → design mm
    pub geom: Geom,
    pub locked: bool,
    pub power_scale: Percent,          // P1 behaviour; stored now
    pub priority: i32,                 // P1 behaviour; stored now
}
#[derive(Debug, Clone, PartialEq)]
pub enum Geom {
    Path(Arc<BezPath>),                // object-space mm
    Rect { w: Mm, h: Mm, corner_radius: Mm },
    Ellipse { rx: Mm, ry: Mm },
    Polygon { sides: u16, radius: Mm },
    Bitmap(Arc<BitmapData>),
    Group(Vec<Arc<Shape>>),
}
#[derive(Debug, Clone, PartialEq)]
pub struct BitmapData { pub asset: AssetId, pub px: [u32; 2], pub size_mm: [f64; 2], pub gray: Arc<[u8]>, pub source_ext: String }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssetId(pub [u8; 32]); // blake3 of the original file bytes

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct CutTable {
    pub colors: Vec<ColorLayer>,       // exactly 30, index = position
    pub tools: [ToolLayer; 2],
    pub order: Vec<LayerKey>,          // Cuts / Layers order = output order; a layer appears when first used
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct ColorLayer {
    pub index: u8,
    pub name: String,
    pub output: bool,
    pub show: bool,
    pub ops: Vec<CutOp>,               // 1..=11 sub-layers run left to right; ops[0] is the layer. UI for >1 is P1
    pub global_passes: u16,            // galvo: repeats the whole stack
    pub image: ImageCut,
    pub air_assist: bool,              // gantry only
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct ToolLayer { pub which: u8, pub name: String, pub show: bool } // never outputs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct CutOp { pub name: String, pub enabled: bool, pub mode: CutMode, pub passes: u16, pub galvo: GalvoParams, pub gantry: GantryParams, pub fill: FillParams }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum CutMode { Line, Fill, OffsetFill } // OffsetFill is P1: the planner refuses it (UnsupportedMode)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct GalvoParams {
    pub speed: MmPerS, pub power: Percent, pub min_power: Percent, pub frequency: Hz, pub q_pulse: Option<Ns>,
    pub timings: Option<GalvoTimings>, // P1 behaviour; refused if set in this build (UnsupportedMode)
    pub wobble: Option<Wobble>,        // P1 behaviour; refused if set in this build
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct GalvoTimings { pub laser_on_tc: Us, pub laser_off_tc: Us, pub end_tc: Us, pub polygon_tc: Us, pub jump_speed: MmPerS, pub min_jump_delay: Us, pub max_jump_delay: Us, pub jump_distance_limit: Mm }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct Wobble { pub step: Mm, pub size: Mm }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct GantryParams { pub speed: MmPerS, pub power_max: Percent, pub power_min: Percent, pub constant_power: bool }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct FillParams { pub interval: Mm, pub angle: Deg, pub angle_increment: Deg, pub bidirectional: bool, pub cross_hatch: bool }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct ImageCut { pub mode: ImageMode, pub interval: Mm, pub angle: Deg, pub bidirectional: bool, pub negative: bool, pub threshold: u8, pub passes: u16, pub galvo: GalvoParams, pub gantry: GantryParams }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ImageMode { Threshold, Jarvis } // P0 subset (S16); others P1

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct Workspace { pub kind: DeviceKind, pub size_mm: [f64; 2], pub origin: WorkspaceOrigin }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum WorkspaceOrigin { Center, BottomLeft, TopLeft }
pub use openlaser_types::DeviceKind; // defined in types (§3.1) so driver can name it without a doc edge
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct DeviceHint { pub kind: DeviceKind, pub profile: MachineId }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct JobSettings { pub repeats: u16 /* 1..=999 */, pub cut_selected: bool, pub frame_style: FrameStyle, pub frame_source: FrameSource }
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct Selection { pub ids: Vec<ObjectId>, pub key: Option<ObjectId> } // key = last selected (align reference)

/// Defaults for a new colour layer, per device kind (machine profiles hold one per kind; B11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct LayerDefaults { pub galvo: GalvoParams, pub gantry: GantryParams, pub fill: FillParams }

impl Project {
    pub fn new(workspace: Workspace, defaults: &LayerDefaults) -> Self; // 30 colour layers + T1/T2, next_id = 1
    pub fn alloc_id(&mut self) -> ObjectId;
    pub fn shape(&self, id: ObjectId) -> Option<&Arc<Shape>>;
    pub fn color_layer(&self, index: u8) -> Option<&ColorLayer>;
    pub fn color_layer_mut(&mut self, index: u8) -> Option<&mut ColorLayer>;
    pub fn used_layers(&self) -> Vec<LayerKey>;
    /// Flattened world polylines (design mm) of one shape, groups expanded; bitmaps give their outline rectangle.
    pub fn world_polylines(&self, id: ObjectId, tol_mm: f64) -> Vec<Polyline>;
    pub fn bounds(&self, ids: &[ObjectId]) -> Option<Bounds>;
    /// Keeps `layers.order` consistent with the shapes (appends newly used layers). Call after every edit.
    pub fn normalize(&mut self);
}
impl Geom { pub fn to_bez(&self) -> BezPath; } // parametric shapes stay parametric until Convert to Path
pub fn default_layer_name(k: LayerKey) -> String; // "C00".."C29", "T1", "T2"
```

### 3.4 `openlaser-machine` (WP1)

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct MachineProfile { pub id: MachineId, pub name: String, pub kind: Kinematics, pub limits: JobLimits, pub layer_defaults: LayerDefaults, pub checklist: Vec<String> }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "snake_case", tag = "type")]
pub enum Kinematics { Galvo(GalvoMachine), Gantry(GantryMachine) }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct GalvoMachine {
    pub board: LmcBoardFilter,       // vid 0x9588, pid 0x9899, optional serial; pid 0x9980 is refused (clone board)
    pub link: GalvoLink,             // Usb | Sim
    pub encoding_field: Mm,          // nominal FIELDSIZE (200). F = encoding_field × cal.scale, ONLY for unit conversion
    pub work_area: FieldRect,        // the lens's rated field: ±100 mm. Shrinks freely; grows only within h27_cleared
    pub h27_cleared: Option<FieldRect>,
    pub cal: FieldCalibration,
    pub red_dot: RedDotCal,          // applied to lighting lists only; identity until H26
    pub source: LaserSource,
    pub timings: GalvoTimingDefaults,
    pub first_jump: FirstJump,       // long jump delay 200 µs + settle 1 ms until H3/H19
    pub jump_speed: MmPerS,          // 3000
    pub light_speed: MmPerS,         // 3000 (red-light frame speed)
    pub max_speed: MmPerS,           // 6000; above it is refused
    pub mo_settle: Us,               // 8000 µs → DelayTime(800)
    pub ports: PortMap,              // laser bit 0 (0x0001), red light bit 8 (0x0100)
    pub run_ahead: RunAhead,         // ≤ 3 packets and ≤ 2 s estimated emission, never < 1 packet; watchdog factors
    pub transfer_timeout_ms: u32,    // 250: total budget of one USB transfer (sliced ≤ 20 ms); then Fault + stop (H13)
    pub usb_set_configuration: bool, // false until H12: if the device is unconfigured, refuse (UsbNotConfigured)
                                     // instead of calling set_configuration(1); Luiz flips it at a TTY if H12 says so
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct LmcBoardFilter { pub vid: u16, pub pid: u16, pub serial: Option<String> }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum GalvoLink { Usb, Sim }
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct FieldRect { pub min: [f64; 2], pub max: [f64; 2] }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct FieldCalibration {
    pub scale: [f64; 2],     // per galvo axis; OM's uniform 1.1834 imports as [1.1834, 1.1834]; 0.5..=2.0
    pub offset_mm: [f64; 2], // Field Offset X/Y
    pub angle_deg: f64,      // Field Angle
    pub swap_xy: bool,       // "Galvo 1 is X axis" off
    pub negate: [bool; 2],   // "Reverse direction" per axis (the prototype's mirror flags live here now)
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct RedDotCal { pub offset_mm: [f64; 2], pub scale: [f64; 2] }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct LaserSource { pub kind: SourceKind, pub pulse_table_ns: Vec<u16>, pub freq_khz: [f64; 2], pub max_power_pct: f64, pub q_pulse_enabled: bool, pub mo_only_when_running: bool }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum SourceKind { MopaFiberJpt }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct GalvoTimingDefaults { pub laser_on_us: f64, pub laser_off_us: f64, pub polygon_us: f64, pub jump_delay_short_us: f64, pub jump_delay_long_us: f64 } // GP defaults: 100, 100, 100, 8, 200
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct FirstJump { pub delay_us: f64, pub settle_10us: u16 } // 200 µs, 100 (= 1 ms)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct PortMap { pub laser_bit: u8, pub red_bit: u8 }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct RunAhead {
    pub max_packets: u8,       // 3
    pub max_emission_s: f64,   // 2.0
    pub watchdog_factor: f64,  // 1.5 (OM v1 5f7aa63); refined after H1/H9
    pub watchdog_slack_s: f64, // 2.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct GantryMachine {
    pub bed_mm: [f64; 2], pub origin: WorkspaceOrigin, pub max_speed: MmPerS, pub travel_speed: MmPerS, pub s_max: u32,
    pub tool: GantryTool,            // Laser { dynamic: bool /* M4 */ } | Pen
    pub air_assist: bool, pub frame_before_mark: bool, pub home_on_connect: bool /* default false */, pub link: GantryLink,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "snake_case", tag = "type")]
pub enum GantryTool { Laser { dynamic: bool }, Pen }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "snake_case", tag = "type")]
pub enum GantryLink { ExportOnly, Serial { port: String, baud: u32 }, Tcp { host: String, port: u16 }, Sim }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]
pub struct JobLimits { pub max_paths: u32, pub max_points_per_path: u32, pub max_points_per_pass: u32 } // 3000 / 2000 / 20000

impl GalvoMachine {
    pub fn field_f(&self) -> [f64; 2];                // encoding_field × scale, per axis (236.68 for Luiz)
    /// work_area ∩ the strictly encodable square (|v| < F/2 on each axis).
    pub fn effective_work_area(&self) -> FieldRect;
}
pub fn jpt_m7_200() -> MachineProfile;      // id "jpt-m7-200": OM profile + calibration 1.1834, link Usb
pub fn jpt_m7_200_sim() -> MachineProfile;  // id "jpt-m7-200-sim": identical, link Sim (default in non-hw builds)
pub fn om_gantries() -> Vec<MachineProfile>;// co2-grbl-600x400, diode-grbl-400x400, plotter-grbl-a4 (OM machines.py, T2)
/// Collects every problem. Galvo: scale in 0.5..=2.0; work_area ⊆ h27_cleared, or ⊆ ±encoding_field/2 when
/// h27_cleared is None (the rated field); effective_work_area non-empty; freq range within 1..=4000 kHz and lo < hi;
/// pulse table non-empty and ascending; max_power_pct in (0, 100]; run_ahead.max_packets ≥ 1, max_emission_s > 0,
/// watchdog_factor ≥ 1; transfer_timeout_ms in 20..=2000; checklist non-empty (mint needs ≥ 1 tick, §3.9).
/// Gantry: bed and speeds positive, s_max ≥ 1. Every save and every load runs it; a failing profile is not used.
pub fn validate_profile(p: &MachineProfile) -> Result<(), Vec<String>>;

/// S24. Writes are human-only by construction: only the GUI and openlaser-hw construct an implementation.
pub trait ProfileStore: Send + Sync {
    fn save(&self, p: &MachineProfile) -> Result<(), String>;   // validate_profile first; atomic
    fn delete(&self, id: &MachineId) -> Result<(), String>;
}

pub mod markcfg {
    pub struct MarkCfg { pub values: BTreeMap<String, String> }
    pub struct Applied { pub applied: Vec<String>, pub ignored: Vec<String> }
    pub fn parse(bytes: &[u8]) -> Result<MarkCfg, Error>;        // latin-1, [LMC_CFG], case-preserving keys
    pub fn apply(cfg: &MarkCfg, base: &MachineProfile) -> Result<(MachineProfile, Applied), Error>;
}
pub mod om {
    /// OM calibration.json → scale_correction; refuses outside 0.5..=2.0 (OM clamps; we refuse).
    pub fn parse_calibration(bytes: &[u8]) -> Result<f64, Error>;
}
```

### 3.5 `openlaser-format` (WP1)

```rust
pub const FORMAT: &str = "openlaser-project";
pub const FORMAT_VERSION: u32 = 1;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not an OpenLaser project: {0}")] Corrupt(String),
    #[error("project format {found} is newer than this build ({supported})")] TooNew { found: u32, supported: u32 },
    #[error("non-finite number at {path}")] NonFinite { path: String },
    #[error("limit exceeded: {0}")] LimitExceeded(String), // entries, decompressed bytes, group depth, points, zip-slip names
    #[error("io: {0}")] Io(#[from] std::io::Error),
    #[error("not implemented: {0}")] NotImplemented(&'static str),
}
/// zip: manifest.json {format, formatVersion, generator, project: ProjectM, paths: {pN: "M0 0L10 0…Z"}} +
/// assets/<blake3>.<ext> (original image bytes, never re-encoded). ProjectM is a separate serde model converted
/// with exhaustive struct literals both ways, so a new doc field fails to compile until it is saved.
pub fn save(project: &Project) -> Result<Vec<u8>, Error>;
pub fn load(bytes: &[u8]) -> Result<Project, Error>;
pub fn save_to(path: &Path, project: &Project) -> Result<(), Error>; // craft-atomic
pub fn load_from(path: &Path) -> Result<Project, Error>;

/// Machine profile files (S24): `<craft_settings::data_dir("OpenLaser")>/machines/<id>.json`.
pub mod machines {
    pub fn profiles_dir() -> Option<PathBuf>;
    /// Built-ins (`jpt_m7_200`, `jpt_m7_200_sim`, `om_gantries`) shadowed by saved files with the same id. A file
    /// that fails to parse or fails `validate_profile` is skipped with a message (never half-applied).
    pub fn load_all(dir: Option<&Path>) -> (Vec<MachineProfile>, Vec<String>);
    pub struct DirProfileStore { /* dir */ }
    impl DirProfileStore { pub fn new(dir: PathBuf) -> Self; }
    impl ProfileStore for DirProfileStore {} // craft-atomic; ids restricted to [a-z0-9-]{1,64}
}
```

### 3.6 `openlaser-import` (WP2)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format { Svg, Dxf, Png, Jpeg, Bmp, Tiff, Webp, Gif, OmWorkspace }
#[derive(Debug, Clone, PartialEq)]
pub struct ImportOptions {
    pub layer: Option<LayerKey>,      // force a layer; else colour → palette match, else the current layer
    pub current_layer: LayerKey,
    pub center_at: Option<[f64; 2]>,  // design mm; None = keep file coordinates (SVG/DXF) or centre on (0,0) (images)
    pub scale: f64,                   // 1.0
    pub svg_units: SvgUnits,
    pub dxf_units: DxfUnits,
    pub flatten_tol_mm: f64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum SvgUnits { RequirePhysical, Px96, Px72 } // default RequirePhysical: refuse unitless
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum DxfUnits { Auto, Mm, Inch }
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub shapes: Vec<Shape>,                 // ids are ObjectId(0): the engine assigns ids
    pub layers: Vec<(LayerKey, ColorLayer)>,// layer settings the file carries (OM workspace); applied by the engine
    pub notices: Vec<Notice>,
    pub bounds: Option<Bounds>,
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown file type: {0}")] UnknownFormat(String),
    #[error("{0} has no physical units; choose 96 or 72 px per inch")] Unitless(String),
    #[error("malformed {format}: {message}")] Malformed { format: &'static str, message: String },
    #[error("malformed point in {object}: {message}")] MalformedPoint { object: String, message: String }, // B9
    #[error("too large: {0}")] TooLarge(String),
    #[error("not implemented: {0}")] NotImplemented(&'static str),
}
pub fn sniff(name: &str, bytes: &[u8]) -> Option<Format>;
pub fn import(name: &str, bytes: &[u8], opts: &ImportOptions) -> Result<Imported, Error>;

pub mod dxf_om {
    /// Exact port of OM importers.parse_dxf / import_geometry (T4: counts exact, coords within 1e-9).
    /// SPLINE: OM uses control/fit points; `parse_dxf_om` keeps that, the shipping importer evaluates the spline
    /// (listed divergence; T4 SPLINE rows are compared against `parse_dxf_om`).
    pub fn parse_dxf_om(bytes: &[u8]) -> Result<Vec<Polyline>, Error>;
    pub fn import_geometry_om(name: &str, bytes: &[u8]) -> Result<Vec<Polyline>, Error>;
}
pub mod om {
    #[derive(Debug, Clone, PartialEq, Deserialize)] pub struct OmWorkspace { pub version: u32, pub objects: Vec<OmObject> }
    #[derive(Debug, Clone, PartialEq, Deserialize)] pub struct OmObject { pub name: String, pub layer_id: String, pub polylines: Vec<Vec<Vec<f64>>>, pub x: f64, pub y: f64, pub scale: f64, pub rotation: f64 }
    #[derive(Debug, Clone, PartialEq, Deserialize)] pub struct OmLayerSpec { pub layer_id: String, pub name: String, pub operation: String, pub power_percent: f64, pub frequency_khz: f64, pub pulse_width_ns: f64, pub speed_mm_s: f64, pub passes: u32, pub visible: bool, pub output: bool, pub color: String, pub air_assist: bool, pub raster_pitch_mm: f64, pub raster_angle_deg: f64, pub raster_angle_step_deg: f64 }
    pub fn parse_workspace(bytes: &[u8]) -> Result<OmWorkspace, Error>;   // points with != 2 coords → MalformedPoint (B9)
    pub fn parse_layer_settings(bytes: &[u8]) -> Result<Vec<OmLayerSpec>, Error>;
    /// OM's four built-in layers (R05 §4.1), used when no `layer_settings.json` sits next to the workspace
    /// (a notice says so).
    pub fn builtin_layers() -> Vec<OmLayerSpec>;
    /// Mapping in Appendix B. An object whose `layer_id` is not in the layer settings refuses the whole import,
    /// naming the objects (B8: never dropped, never guessed).
    pub fn to_imported(ws: &OmWorkspace, layers: &[OmLayerSpec]) -> Result<Imported, Error>;
}
```

### 3.7 `openlaser-plan` (WP3)

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct PlanOptions {
    pub selected: Option<Vec<ObjectId>>, // Cut Selected Graphics
    pub repeats: u16,
    pub frame_style: FrameStyle,
    pub frame_source: FrameSource,
    pub flatten_tol_mm: f64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct JobPlan {
    pub machine: MachineId,
    pub device: DeviceKind,
    pub sections: Vec<Section>,     // burn order: Cuts/Layers order, then sub-layer, then pass; draw order inside
    pub frame: Vec<Polyline>,       // machine mm; what a Job frame traces (style applied); red-dot NOT applied
    pub tool_frame: Vec<Polyline>,  // T1/T2 shapes, for ToolLayersOnly / JobAndToolLayers
    pub facts: PlanFacts,           // what safety must judge; the planner never drops anything silently
    pub repeats: u16,
    pub estimate: Duration,         // all repeats
    pub fingerprint: Fingerprint,   // S21: blake3 over the canonical encoding of machine, device, sections (layer,
                                    // pass, quantised words, path f64 bit patterns), repeats, the machine's
                                    // calibration, transform, red-dot calibration and first-jump/timing settings.
                                    // NOT frame, tool_frame, frame style or source: those are framing choices, and
                                    // the receipt records them (covers_job decides)
}
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub layer: LayerKey, pub layer_name: String, pub sub: u8, pub pass: u16, pub of: u16,
    pub mode: SectionMode,
    pub cut: CutQ,
    pub paths: Vec<Polyline>,       // machine mm; Fill/Image: 2-point segments in burn order
    pub objects: Vec<ObjectId>,
    pub estimate: Duration,
}
#[derive(Debug, Clone, PartialEq)]
pub enum CutQ { Galvo(GalvoCutQ), Gantry(GantryCutQ) }
/// Quantised once, here (PLAN D9). The encoder reads only the words.
#[derive(Debug, Clone, PartialEq)]
pub struct GalvoCutQ {
    pub requested_speed: MmPerS,
    pub speed_units: u16,           // may be 0: safety refuses (B7)
    pub actual_speed_mm_s: f64,     // display only
    pub power: Percent,
    pub mark_current: u16,
    pub frequency: Hz,
    pub q_period: u16,
    pub pulse: PulseDecision,       // Exact | Snapped | Refused (safety refuses)
    pub timings: ResolvedTimings,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTimings {
    pub laser_on: (u16, u16), pub laser_off: (u16, u16), pub polygon: (u16, u16),
    pub jump_delay_short: (u16, u16), pub jump_delay_long: (u16, u16),
    pub jump_speed_units: u16, pub mo_settle_10us: u16, pub first_jump_settle_10us: u16,
}
#[derive(Debug, Clone, PartialEq)]
pub struct GantryCutQ { pub speed: MmPerS, pub power: Percent, pub s_value: u32, pub air_assist: bool, pub dynamic: bool }
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlanFacts {
    pub orphan_objects: Vec<ObjectId>,             // layer not in the table (import of a forged/foreign file)
    pub hidden_output_layers: Vec<LayerKey>,       // show == false && output == true (B6)
    pub empty_fill_layers: Vec<LayerKey>,          // Fill/Image layer with shapes but no segments
    pub open_fill_shapes: Vec<(ObjectId, LayerKey)>,
    pub non_finite_objects: Vec<ObjectId>,
    pub unsupported: Vec<(LayerKey, String)>,      // OffsetFill, timings, wobble, Hull frame
    pub tool_layer_objects: Vec<ObjectId>,
    pub device_mismatch: Option<String>,           // project hint vs machine kind
}
#[derive(Debug, thiserror::Error)]
pub enum Error { #[error("geometry: {0}")] Geom(#[from] openlaser_geom::Error), #[error("not implemented: {0}")] NotImplemented(&'static str) }

pub fn plan(project: &Project, machine: &MachineProfile, opts: &PlanOptions) -> Result<JobPlan, Error>;
/// Design mm → machine mm: field angle, swap, negate, offset. Never scale (scale lives in F).
pub fn machine_transform(m: &GalvoMachine) -> Affine;
/// Galvo time model: mark length / actual speed + jump length / jump speed + jump delays + laser on/off/polygon
/// delays per vertex + MO settle + first-jump settle.
pub fn estimate_galvo(paths: &[Polyline], q: &GalvoCutQ, m: &GalvoMachine) -> Duration;
pub fn estimate_gantry(paths: &[Polyline], q: &GantryCutQ, m: &GantryMachine) -> Duration;
```

### 3.8 `openlaser-safety` (WP3, red zone)

```rust
/// Validated job. (sealed) Built only by `validate`.
#[derive(Debug, Clone, PartialEq)]
pub struct Validated { emit: EmitPlan, frame: FramePlan, notices: Vec<Notice> }
impl Validated { pub fn emit(&self) -> &EmitPlan; pub fn frame(&self) -> &FramePlan; pub fn notices(&self) -> &[Notice]; pub fn into_parts(self) -> (EmitPlan, FramePlan, Vec<Notice>); }
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Refused { pub fingerprint: Fingerprint, pub refusals: Vec<Refusal>, pub notices: Vec<Notice> }

/// (sealed) Sections that passed every rule. Serialize only.
#[derive(Debug, Clone, PartialEq)]
pub struct EmitPlan { sections: Vec<Section>, fingerprint: Fingerprint, machine: MachineId, device: DeviceKind, repeats: u16 }
impl EmitPlan { pub fn sections(&self) -> &[Section]; pub fn fingerprint(&self) -> &Fingerprint; pub fn machine(&self) -> &MachineId; pub fn device(&self) -> DeviceKind; pub fn repeats(&self) -> u16; }
/// (sealed) What the red light may trace. Has no power, frequency or pulse fields.
#[derive(Debug, Clone, PartialEq)]
pub struct FramePlan { paths: Vec<Polyline>, fingerprint: Fingerprint, machine: MachineId, style: FrameStyle, source: FrameSource }
impl FramePlan { pub fn paths(&self) -> &[Polyline]; pub fn fingerprint(&self) -> &Fingerprint; pub fn machine(&self) -> &MachineId; pub fn style(&self) -> FrameStyle; pub fn source(&self) -> FrameSource; }

/// Pure; runs live in the UI, again before ARM, and inside the engine before every encode. Collects ALL refusals.
pub fn validate(plan: &JobPlan, machine: &MachineProfile) -> Result<Validated, Refused>;
/// Framing a job that cannot burn (e.g. tool layers only, or to look at placement while a power refusal is open)
/// needs only the geometry rules. Its receipt carries the plan fingerprint, but a refused job never yields an
/// `EmissionSummary`, so nothing can be minted from it; fixing the refusal edits the document and changes the
/// fingerprint, so the job must be framed again.
pub fn validate_frame(plan: &JobPlan, machine: &MachineProfile, source: FrameSource) -> Result<FramePlan, Refused>;
/// The rule table, for docs and the `rules_table_complete` test.
pub fn rules() -> &'static [(RefusalCode, &'static str)];

#[cfg(feature = "unchecked-for-tests")] // dev-dependency only; compile_error! without debug_assertions
pub mod testing {
    pub fn emit_plan_unchecked(sections: Vec<Section>, fp: Fingerprint, machine: MachineId, device: DeviceKind, repeats: u16) -> EmitPlan;
    pub fn frame_plan_unchecked(paths: Vec<Polyline>, fp: Fingerprint, machine: MachineId, style: FrameStyle, source: FrameSource) -> FramePlan;
} // implemented in full by WP0 (§3.0); referenced only from tests (§2.3 rule 14)
```

**Rules `validate` enforces** (each is a test named `safety_<code>`; PLAN §3.6 layer 1, GOLD divergences):

| Code | Rule |
|---|---|
| `outside_work_area` | any point outside `effective_work_area()` (B10: (100, 0) accepted, (100.5, 0) refused at scale 1.1834), Frame and Mark alike; never clamped |
| `not_encodable` | `mm_to_galvo` fails on either axis (+F/2 exactly is refused) |
| `non_finite` / `malformed_point` | NaN/inf anywhere; points without exactly 2 coordinates (import, B9) |
| `too_many_paths` / `too_many_points_per_path` / `too_many_points_per_pass` | `JobLimits` (3000 / 2000 / 20000) |
| `power_above_max` | power > source max |
| `frequency_out_of_range` | frequency outside `source.freq_khz` (1..4000 kHz) |
| `pulse_not_in_table` / `q_pulse_unsupported` | `PulseDecision::Refused`; a pulse set while `q_pulse_enabled` is false. `Snapped` → notice `pulse_snapped` |
| `speed_zero_units` | galvo mark speed encodes to 0 units (B7; below 3.6115 mm/s at F = 236.68) |
| `speed_above_max` | speed above `max_speed` (galvo and gantry; never capped) |
| `orphan_object` | `facts.orphan_objects` non-empty (B8) |
| `hidden_layer_with_output` | `facts.hidden_output_layers` non-empty (B6) |
| `fill_without_hatch` | `facts.empty_fill_layers` non-empty; open Fill shapes become numbered notices `open_shape_unhatched` |
| `unsupported_mode` | `facts.unsupported` non-empty |
| `nothing_to_mark` | no emitting section |
| `wrong_device_kind` | plan device ≠ machine kind |
| `out_of_bed` | gantry point outside the bed |

### 3.9 `openlaser-arm` (WP3, red zone)

```rust
/// Proof that a human confirmed. (no Clone) Constructors are clippy disallowed-methods outside one file each.
pub struct HumanConfirm { kind: ConfirmKind, typed: String, ticked: Vec<bool>, at: MonoTime }
pub enum ConfirmKind { GuiGesture, Tty { nonce: u64 } }
pub struct GuiGestureWitness(());
pub struct TtyWitness(());
impl GuiGestureWitness { pub fn new() -> Self; }  // only crates/human/src/gesture.rs
impl TtyWitness { pub fn new() -> Self; }         // only apps/openlaser-hw/src/tty_arm.rs
impl HumanConfirm {
    pub fn from_gui_gesture(w: GuiGestureWitness, typed: &str, ticked: Vec<bool>, at: MonoTime) -> Self; // only crates/human/src/gesture.rs
    pub fn from_tty(w: TtyWitness, typed: &str, nonce: u64, at: MonoTime) -> Self;                       // only apps/openlaser-hw/src/tty_arm.rs
}
/// Mints a single-use token valid 60 s whose `bytes` = `summary.bytes_hash` (S22). Refuses unless ALL hold:
/// summary.fingerprint == receipt.fingerprint (Stale); receipt.machine == summary.machine (MachineMismatch);
/// receipt.session == session (ArmSessionMismatch); receipt.full_passes >= 1 and age <= 10 min
/// (FrameRequired / FrameReceiptExpired); receipt.origin == Human (FrameNotHuman); receipt.covers_job()
/// (FrameDoesNotCoverJob); typed text == "ARM" (GUI, trimmed, case-insensitive) or "ARM <nonce>" (TTY)
/// (ArmTextMismatch). GuiGesture: checklist_len >= 1, ticked.len() == checklist_len and all true
/// (ChecklistIncomplete; an empty checklist is refused, never vacuously true). Tty: the caller printed the checklist
/// before the challenge, and the typed `ARM <nonce>` line is the acknowledgement; `ticked` is ignored. The token's
/// nonce is a fresh getrandom u64, not the TTY challenge.
pub fn mint(summary: &EmissionSummary, receipt: &FrameReceipt, confirm: HumanConfirm, checklist_len: usize, session: SessionId, now: MonoTime) -> Result<ArmToken, Refusal>;
pub fn tty_nonce() -> Result<u64, Refusal>; // getrandom; 6 decimal digits shown to the operator (the challenge only)

#[cfg(feature = "test-witness")] // dev-dependency only; compile_error! without debug_assertions
pub mod testing { // implemented in full by WP0, in crates/arm/src/mint.rs (§3.0); referenced only from tests
    pub fn confirm_for_tests(typed: &str, ticked: Vec<bool>, at: MonoTime) -> HumanConfirm;
    /// Lets driver/engine tests (WP5, WP6) run before WP3's `mint` lands.
    pub fn token_for_tests(fingerprint: Fingerprint, bytes: Fingerprint, machine: MachineId, session: SessionId, expires: MonoTime, nonce: u64) -> ArmToken;
}
```

### 3.10 `openlaser-render` (WP2)

```rust
#[derive(Debug, Clone, Copy, PartialEq)] pub struct Rgba(pub [u8; 4]);
#[derive(Debug, Clone, Copy, PartialEq)] pub enum Stroke { Hairline, Dashed, Width(f32) }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum MarkerKind { Refusal, Notice(u32), Origin }
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Polylines { stroke: Stroke, color: Rgba, lines: Arc<[Polyline]>, object: Option<ObjectId> },
    Segments { color: Rgba, segs: Arc<[[P2; 2]]> },          // hatch, emission, travel
    Rect { stroke: Stroke, color: Rgba, rect: Bounds },      // field, work area, selection box
    Marker { kind: MarkerKind, at: P2, color: Rgba },
    Image { rect: Bounds, gray: Arc<[u8]>, px: [u32; 2] },
}
#[derive(Debug, Clone, PartialEq, Default)] pub struct DisplayList { pub items: Vec<Item> } // world mm, Y up
#[derive(Debug, Clone, PartialEq)]
pub struct ViewOpts { pub dark: bool, pub selection: Vec<ObjectId>, pub refusals: Vec<Refusal>, pub notices: Vec<Notice>, pub show_field: bool, pub show_work_area: bool }
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanShow { pub burn: bool, pub travel: bool, pub frame: bool, pub order: bool }
pub fn document_list(p: &Project, m: Option<&MachineProfile>, o: &ViewOpts) -> DisplayList;
pub fn plan_list(plan: &JobPlan, m: &MachineProfile, show: &PlanShow) -> DisplayList;
pub fn emission_list(burned: &[EmittedSegment], light: &[[P2; 2]], m: &MachineProfile) -> DisplayList;
pub fn fit(list: &DisplayList, margin_frac: f64) -> Option<Bounds>;
#[derive(Debug, Clone, PartialEq)] pub struct RenderView { pub world: Bounds, pub max_side: u32, pub background: Rgba }
/// Deterministic CPU raster (same input → same bytes). max_side <= 2048; output <= 5 MiB or Err.
pub fn render_png(list: &DisplayList, view: &RenderView) -> Result<Vec<u8>, Error>;
```

### 3.11 `openlaser-gcode` (WP2)

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct GcodeProgram { pub text: String, pub lines: u32, pub fingerprint: Fingerprint, pub machine: MachineId, pub distance_mm: f64, pub estimate: Duration }
impl GcodeProgram { pub fn bytes_hash(&self) -> Fingerprint; } // blake3 of `text` (S22)
/// (sealed) A laser-off frame program: built only by `frame_program`, so GRBL's `Backend::Frame` cannot be handed a
/// burning program (the LMC side gets the same guarantee from `LightingList`).
#[derive(Debug, Clone, PartialEq)]
pub struct GcodeFrame { program: GcodeProgram }
impl GcodeFrame { pub fn program(&self) -> &GcodeProgram; }
#[derive(Debug, Clone, PartialEq)] pub struct Header { pub machine_name: String, pub generated: String } // tests pass "<MASKED>"
/// Shipping encoder (B11: only the gantry block is read). Overspeed never reaches here: safety refuses it.
pub fn compile(emit: &EmitPlan, m: &GantryMachine, h: &Header) -> Result<GcodeProgram, Error>;
/// Laser-off frame around the job (G0 only, no M3/M4/S>0; a test greps the text).
pub fn frame_program(frame: &FramePlan, m: &GantryMachine, h: &Header) -> Result<GcodeFrame, Error>;
/// OM `_number`: format!("{:.3}") then trim zeros and dot; "-0" → "0". T2 number_format (100,019 rows).
pub fn number(v: f64) -> String;
pub mod om {
    /// T2 parity port of OM gcode.compile_gcode, including its silent speed capping (py-compat only).
    pub fn compile_gcode_om(machine: &OmMachine, layers: &[OmLayer], objects: &[OmObject], generated: &str) -> Result<String, OmRefusal>;
    // OmMachine/OmLayer/OmObject mirror the fields in tests/golden/t2_gcode/index.json
}
```

### 3.12 `openlaser-lmc` (WP4)

```rust
pub mod op { // Appendix B of R05, every constant; names as GP
    pub const GET_SERIAL_NO: u16 = 0x0009; pub const GET_VERSION: u16 = 0x0007; /* … */
    pub const LIST_JUMP_TO: u16 = 0x8001; pub const LIST_END_OF_LIST: u16 = 0x8002; pub const LIST_MARK_TO: u16 = 0x8005; /* … */
    pub fn single_name(op: u16) -> &'static str; pub fn list_name(op: u16) -> &'static str;
}
pub const PACKET_LEN: usize = 3072;
pub const SLOTS: usize = 256;
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct Single(pub [u16; 6]);
impl Single { pub fn bytes(&self) -> [u8; 12]; pub fn reads_reply(&self) -> bool; } // every single except WriteCorLine
#[derive(Clone, PartialEq, Eq)] pub struct Packet(Box<[u8; PACKET_LEN]>); // manual Debug: sha256 prefix + real command count
impl Packet { pub fn bytes(&self) -> &[u8; PACKET_LEN]; pub fn words(&self) -> Vec<[u16; 6]>; pub fn real_commands(&self) -> usize; }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct StatusWord(pub u16);
impl StatusWord { pub const BUSY: u16 = 0x04; pub const READY: u16 = 0x20; pub const AXIS: u16 = 0x40; pub fn busy(self) -> bool; pub fn ready(self) -> bool; }
pub fn status_from_reply(reply: [u16; 4]) -> StatusWord; // word 3; unknown bits kept (one board idles at 0x0220)

/// R05 §1.4 verbatim: GetSerialNo … EnableZ, then a 50 ms pause (the session sleeps; not in the list).
pub fn init_sequence() -> Vec<Single>;

/// A burn, ready for the driver. One list per layer-pass (PLAN §3.3), each list packetised into 3072-byte packets
/// padded with 0x8002. Every byte is fixed at encode time: nothing in a packet depends on session state (the first
/// JumpTo's `dist` of every list is computed from (0x8000, 0x8000), §5.2 rule 6), so the bytes the summary was
/// decoded from are the bytes sent, and `bytes_hash` is stable.
#[derive(Debug, Clone, PartialEq)]
pub struct LmcJob {
    pub prelude: Vec<Single>,     // sent before EACH list: ResetList, WritePort(0x0001), Fiber_SetMo(1), GetVersion
    pub lists: Vec<LmcList>,      // per layer-pass, in burn order (one repeat)
    pub repeats: u16,             // run by the driver, stop checked between repeats; never expanded
    pub postlude: Vec<Single>,    // after EACH list, only once !BUSY: Fiber_SetMo(0), WritePort(0), GetMarkTime(3)
    pub fingerprint: Fingerprint,
    pub machine: MachineId,
}
impl LmcJob { pub fn bytes_hash(&self) -> Fingerprint; } // S22: the EmissionSummary.bytes_hash definition
#[derive(Debug, Clone, PartialEq)]
pub struct LmcList { pub layer: LayerKey, pub pass: u16, pub packets: Vec<Packet>, pub packet_est: Vec<Duration> }
/// A red-light frame. `once`: ONE list with the OM Frame Once pass count baked in (≥ 2 s of tracing); `pass`: ONE
/// complete list (ReadyMark … EndOfList) holding exactly one pass, which the driver submits as a fresh list per Loop
/// pass (S20). Neither can contain MarkTo, LaserOnPoint or MO open (type-level). `once_passes` is the baked count
/// the receipt reports.
#[derive(Debug, Clone, PartialEq)]
pub struct LmcFrameJob { pub once: Vec<Packet>, pub once_passes: u32, pub once_est: Duration, pub pass: Vec<Packet>, pub pass_est: Duration, pub fingerprint: Fingerprint, pub machine: MachineId, pub style: FrameStyle, pub source: FrameSource }

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("point ({x}, {y}) mm is not encodable")] NotEncodable { x: f64, y: f64 },
    #[error("encode: {0}")] Encode(#[from] openlaser_types::galvo::EncodeError),
    #[error("section has no galvo parameters")] NotGalvo,
    #[error("empty job")] Empty,
    #[error("not implemented: {0}")] NotImplemented(&'static str),
}
pub fn encode_job(emit: &EmitPlan, m: &GalvoMachine) -> Result<LmcJob, Error>;
pub fn encode_frame(frame: &FramePlan, m: &GalvoMachine) -> Result<LmcFrameJob, Error>;

#[derive(Debug, Clone, PartialEq)]
pub enum Decoded {
    ReadyMark, DelayTime(u16), WritePort(u16), JumpSpeed(u16), MarkSpeed(u16), PulseWidth(u16), MarkCurrent(u16),
    QSwitchPeriod(u16), LaserOnDelay(u16, u16), LaserOffDelay(u16, u16), PolygonDelay(u16, u16), JumpDelay(u16, u16),
    JumpTo { x: u16, y: u16, dist: u16 }, MarkTo { x: u16, y: u16, dist: u16 }, EndOfList, Other([u16; 6]),
}
pub fn decode_packet(p: &Packet) -> Vec<Decoded>;            // stops at the first 0x8002 fill run
#[derive(Debug, Clone, PartialEq)] pub struct ListingLine { pub list: Option<u32>, pub packet: Option<u32>, pub slot: Option<u16>, pub words: [u16; 6], pub text: String }
#[derive(Debug, Clone, PartialEq, Default)] pub struct Listing { pub lines: Vec<ListingLine> }
/// e.g. "MarkTo x=34152 y=31384 (5.000, −5.000 mm) dist=2768"
pub fn disassemble(job: &LmcJob, m: &GalvoMachine) -> Listing;
pub fn disassemble_frame(job: &LmcFrameJob, m: &GalvoMachine) -> Listing;
/// Everything numeric comes from decoding `job`'s packets; `plan` supplies only names, notices and repeats.
pub fn emission_summary(job: &LmcJob, m: &GalvoMachine, plan: &JobPlan, notices: &[Notice]) -> Result<EmissionSummary, Error>;
pub fn emitted_segments(job: &LmcJob, m: &GalvoMachine) -> Vec<EmittedSegment>;

/// The only USB-shaped seam (PLAN §3.4). Implemented by transport::UsbLmc, lmc_sim::FakeLmc and test fakes.
pub trait LmcTransport: Send {
    fn submit_single(&mut self, cmd: Single) -> Result<(), XferError>;
    /// Ok(None) = still pending after `slice` (≤ 20 ms); Ok(Some(None)) = no reply expected; Ok(Some(Some(r))) = reply.
    fn poll_single(&mut self, slice: Duration) -> Result<Option<Option<[u16; 4]>>, XferError>;
    fn submit_packet(&mut self, pkt: &Packet) -> Result<(), XferError>;           // at most one in flight
    fn poll_packet(&mut self, slice: Duration) -> Result<Option<()>, XferError>;  // Ok(None) = in flight
    fn cancel_all(&mut self);     // last resort: an in-flight packet overran 20 ms during a stop (H23)
    fn drain_replies(&mut self);  // discard stale 8-byte replies after an abandoned read
}
impl<T: LmcTransport + ?Sized> LmcTransport for Box<T> { /* forwards */ } // lets the driver hold Box<dyn LmcTransport>
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum XferError {
    #[error("transfer timed out")] Timeout,
    #[error("device disconnected")] Disconnected,
    #[error("endpoint stalled")] Stall,
    #[error("hardware access is disabled in this build or environment")] HardwareDisabled,
    #[error("usb: {0}")] Io(String),
    #[error("not implemented: {0}")] NotImplemented(&'static str), // build-only (§3.0)
}

#[cfg(feature = "py-compat")] // dev-dependency only (self dev-dep); compile_error! without debug_assertions
pub mod py_compat {
    /// GP's sequencing: one controller per job, per-job init, parameter cache, elided first jump, per-job abort
    /// with the dummy packet, close (USB reset). Must reproduce tests/golden/t1_lmc/v0/*.json exactly.
    #[derive(Debug, Clone, PartialEq)]
    pub enum WireEvent { Open, Single { hex: String }, Read { status: u16 }, List { sha256: String, n_cmds: u16, bytes: Packet }, Close }
    #[derive(Debug, Clone, PartialEq)] pub struct OmParams { pub power: f64, pub frequency_khz: f64, pub pulse_width_ns: f64, pub speed_mm_s: f64 }
    pub fn mark_polylines(polys: &[Vec<[f64; 2]>], p: &OmParams, field_mm: f64) -> Result<Vec<WireEvent>, Error>;
    pub fn frame_polylines(polys: &[Vec<[f64; 2]>], field_mm: f64) -> Result<Vec<WireEvent>, Error>;
    pub fn init_abort() -> Vec<WireEvent>;
}
```

### 3.13 `openlaser-driver` (WP5, red zone)

```rust
/// Tripped by anyone, from any thread; read by the driver between every transfer. Signal handlers never call
/// `trip()` (a channel send is not async-signal-safe): they only store `true` into `signal_flag()` (signal-hook's
/// `flag::register`), which `is_tripped()` also reads, and the app's watcher thread then calls `trip()`.
/// `generation()` counts trips; every queued command records it at submission (§5.3 "queued-then-stopped").
pub struct StopLatch { /* AtomicBool tripped; Arc<AtomicBool> signal; AtomicU64 generation; AtomicU64 tripped_at_ns; wake: crossbeam Sender<()> */ }
impl StopLatch { pub fn trip(&self); pub fn is_tripped(&self) -> bool; pub fn generation(&self) -> u64; pub fn signal_flag(&self) -> Arc<AtomicBool>; }
/// Cheap, cloneable, Send + Sync handle for the control server's fast path, Esc, signals and the CLI key thread.
#[derive(Clone)]
pub struct Stopper { /* Arc<StopLatch>, Arc<StopMailbox> */ }
impl Stopper {
    pub fn stop(&self);                                       // atomic store + wake; never blocks, never does I/O
    pub fn stop_and_wait(&self, max: Duration) -> StopAck;    // stop(), then wait ≤ max for the StopReport
}
#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
pub struct StopAck { pub latched: bool, pub report: Option<StopReport>, pub elapsed_ms: u64 }
#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
pub struct StopReport { pub confirmed: bool, #[serde(rename = "elapsedMs", with = "openlaser_types::serde_ms")] pub elapsed: Duration, pub possible_desync: bool, pub mo_closed: bool, pub ports_zero: bool, pub notes: Vec<String> }

/// What the driver thread gets each call: stop checks, events and the clock.
pub struct Cx<'a> { /* latch, events, clock */ }
impl Cx<'_> {
    pub fn check(&self) -> Result<(), Stopped>;     // Err as soon as the latch is tripped
    pub fn slice(&self, d: Duration) -> Result<(), Stopped>; // sleep ≤ min(d, 20 ms), then check
    pub fn now(&self) -> MonoTime;
    /// Never blocks: events go through an UNBOUNDED crossbeam channel, so a UI that stops polling (window hidden,
    /// UI thread stalled) can never stall the driver thread or its stop checks. Trace lines beyond 100 000 per
    /// operation are dropped and counted (`TraceLine` "N trace lines dropped").
    pub fn emit(&self, ev: DriverEvent);
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct Stopped;

pub trait EncodedJob: Send + 'static {
    fn fingerprint(&self) -> &Fingerprint;
    fn machine(&self) -> &MachineId;
    fn emits(&self) -> bool;
    fn bytes_hash(&self) -> Fingerprint; // S22; recomputed by the runtime from the object it was handed
}
#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
pub struct Capabilities { pub kind: DeviceKind, pub frame: bool, pub run: bool, pub pause: bool, pub jog: bool, pub home: bool, pub inputs: bool }
pub trait Backend: Send + 'static {
    type Job: EncodedJob;
    type Frame: EncodedJob;
    fn capabilities(&self) -> Capabilities;
    fn connect(&mut self, cx: &mut Cx) -> Result<DeviceInfo, DriverError>;
    /// S20: emits FrameCompleted(receipt) only after the board has EXECUTED the traced list (status !BUSY after it).
    /// Once: one list, one receipt with `full_passes = once_passes`. Loop: one list per pass, a receipt after each
    /// (`full_passes` counts executed passes), until stopped. A stop before !BUSY gives no receipt for that list.
    fn frame(&mut self, f: &Self::Frame, mode: FrameMode, origin: FrameOrigin, session: SessionId, cx: &mut Cx) -> Result<FrameOutcome, DriverError>;
    /// The runtime has already checked the token, the nonce and its own receipt record.
    fn run(&mut self, job: &Self::Job, cx: &mut Cx) -> Result<Outcome, DriverError>;
    /// Never returns early; always ends with MO closed and ports at 0, or reports that it could not confirm.
    fn emergency_stop(&mut self, cx: &mut Cx) -> StopReport;
    fn status(&mut self, cx: &mut Cx) -> Result<MachineStatus, DriverError>; // a failed read is Err, never bits
    fn disconnect(&mut self, cx: &mut Cx);
}
pub fn spawn<B: Backend>(backend: B, machine: MachineId, session: SessionId, clock: Arc<dyn Clock>) -> DriverHandle<B>;
pub struct DriverHandle<B: Backend> { /* ctl: Sender<Control<B>>, stopper: Stopper, events: Receiver<DriverEvent>, caps */ }
impl<B: Backend> DriverHandle<B> {
    /// Every command records `latch.generation()` when queued; the driver thread refuses to start it
    /// (`Refused(StoppedAfterQueue)`) if a trip happened after that, and only then clears the latch for it. A stop
    /// pressed between "Confirm" and the driver dequeuing the run therefore cancels the run instead of being cleared.
    pub fn connect(&self);
    /// Refused with `MachineBusy` by the driver thread itself while another frame or run is active (defence in
    /// depth behind the engine's pipeline step 7).
    pub fn frame(&self, f: B::Frame, mode: FrameMode, origin: FrameOrigin);
    /// Queues the job. The driver thread re-checks: `token.check(job.fingerprint(), &job.bytes_hash(), machine,
    /// session, now)` with the hash RECOMPUTED from the job object (S22); nonce unused (then spent); its OWN last
    /// FrameReceipt has the same fingerprint, ≥ 1 full (executed) pass, ≤ 10 min, origin Human, covers job; no
    /// frame or run active (`MachineBusy`).
    pub fn run(&self, job: B::Job, token: ArmToken);
    pub fn stop(&self);
    pub fn stopper(&self) -> Stopper;
    pub fn events(&self) -> &Receiver<DriverEvent>;
    pub fn capabilities(&self) -> &Capabilities;
    pub fn disconnect(&self);
}
// Drop of the last DriverHandle = dead-man: it trips the latch, then JOINS the driver thread for ≤ 500 ms so the
// abort reaches the wire before a panicking or exiting UI thread can end the process (process exit kills other
// threads mid-sequence). Apps also install a panic hook and signal watchers that stop every machine first (§5.4),
// because on macOS `eframe::run_native` may exit the process without running destructors.

#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "snake_case", tag = "state")]
pub enum MachineState { Disconnected, Connecting, Idle, Framing, Running { packets_acked: u32, total: u32 }, Stopping, Stopped, Fault, StopNotConfirmed }
#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
pub struct MachineStatus { pub state: MachineState, pub status_bits: Option<u16>, pub ready: bool, pub busy: bool }
#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
pub struct DeviceInfo { pub vid: u16, pub pid: u16, pub serial: Option<String>, pub version: Option<[u16; 4]>, pub provider: String }
#[derive(Debug, Clone, PartialEq, Serialize)] pub enum FrameOutcome { Completed { passes: u32 }, Stopped(StopReport) }
#[derive(Debug, Clone, PartialEq, Serialize)] pub enum Outcome { Finished { duration: Duration }, Stopped(StopReport) }
#[derive(Debug, Clone, PartialEq, thiserror::Error, Serialize)]
pub enum DriverError {
    #[error("transport: {0}")] Transport(String),
    #[error("board not READY within the progress budget; stopped")] Watchdog { waited_ms: u64 },
    #[error("refused: {0:?}")] Refused(Refusal),
    #[error("protocol: {0}")] Protocol(String),
    #[error("driver thread panicked; stop was sent")] PanicContained,
    #[error("not connected")] NotConnected,
    #[error("not implemented: {0}")] NotImplemented(&'static str),
}
#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "snake_case", tag = "event")]
pub enum DriverEvent {
    Connected(DeviceInfo), State(MachineState), FrameCompleted(FrameReceipt),
    Progress { packets_acked: u32, total: u32, eta_ms: u64 }, Finished(Outcome), Stopped(StopReport),
    Fault(DriverError), /* sticky until acknowledged */ Trace(TraceLine),
}
#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
pub struct TraceLine { pub at: MonoTime, pub out: bool, pub text: String } // disassembled, for the Console panel

/// LMC session (red zone): §5.
pub struct LmcBackend<T: LmcTransport> { /* transport, machine: GalvoMachine, assumed position, used nonces… */ }
impl<T: LmcTransport> LmcBackend<T> { pub fn new(transport: T, machine: GalvoMachine) -> Self; }
impl<T: LmcTransport + 'static> Backend for LmcBackend<T> { type Job = LmcJob; type Frame = LmcFrameJob; /* … */ }
impl EncodedJob for LmcJob { /* emits() = true; bytes_hash() = LmcJob::bytes_hash */ }
impl EncodedJob for LmcFrameJob { /* emits() = false */ }

/// GRBL streaming (§5.7). Tested on FakeGrbl only in this build.
pub trait ByteLink: Send {
    fn write_all(&mut self, bytes: &[u8], slice: Duration) -> Result<(), LinkError>;
    fn write_realtime(&mut self, byte: u8) -> Result<(), LinkError>;      // ! ~ ? 0x18 0x85: bypass the queue
    fn read_available(&mut self, buf: &mut Vec<u8>, slice: Duration) -> Result<usize, LinkError>;
}
#[derive(Debug, Clone, PartialEq, thiserror::Error)] pub enum LinkError { #[error("timeout")] Timeout, #[error("closed")] Closed, #[error("io: {0}")] Io(String), #[error("hardware access is disabled")] HardwareDisabled, #[error("not implemented: {0}")] NotImplemented(&'static str) }
pub struct GrblBackend<L: ByteLink> { /* … */ }
impl<L: ByteLink> GrblBackend<L> { pub fn new(link: L, machine: GantryMachine) -> Self; }
/// `Frame = GcodeFrame` (sealed, built only by `gcode::frame_program`), so a burning program cannot be framed.
impl<L: ByteLink + 'static> Backend for GrblBackend<L> { type Job = GcodeProgram; type Frame = GcodeFrame; /* … */ }
impl EncodedJob for GcodeProgram { /* emits() = true */ }
impl EncodedJob for GcodeFrame { /* emits() = false */ }

/// What `detect` reports (transport maps its `DetectedBoard` into this).
#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
pub struct DetectedDevice { pub vid: u16, pub pid: u16, pub serial: Option<String>, pub location: String, pub supported: bool, pub note: String }
/// Injected by the apps; the engine never names a concrete transport.
pub trait DeviceProvider: Send + Sync {
    fn name(&self) -> &'static str; // "sim", "usb"
    /// Read-only enumeration for `device.detect` (Find My Laser). Never opens a device.
    fn detect(&self) -> Result<Vec<DetectedDevice>, DriverError>;
    fn open_lmc(&self, m: &GalvoMachine) -> Result<Box<dyn LmcTransport>, DriverError>;
    fn open_link(&self, m: &GantryMachine) -> Result<Box<dyn ByteLink>, DriverError>;
    /// S23 first-emission cap, consulted by `Session::run_armed` and the ARM dialog (the engine cannot link
    /// `transport`). UsbProvider: `bench::emission_cap_pct`; SimProvider: `SimConfig.emission_cap_pct` (None by
    /// default; tests set it).
    fn emission_cap_pct(&self) -> Result<Option<f64>, DriverError>;
    /// After a run finishes (not after a stop or fault). UsbProvider: `bench::record_emission`.
    fn record_emission(&self) -> Result<(), DriverError>;
}
impl<L: ByteLink + ?Sized> ByteLink for Box<L> { /* forwards */ }
pub enum AnyDriver { Lmc(DriverHandle<LmcBackend<Box<dyn LmcTransport>>>), Grbl(DriverHandle<GrblBackend<Box<dyn ByteLink>>>) }
```

### 3.14 `openlaser-lmc-sim` (WP4)

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct SimConfig {
    pub buffer_depth: u8,          // packets the board queues before READY drops; swept 1..=8 until H2
    pub transfer_latency: Duration,// 0 (virtual) or 2 ms (real-time T6 variant)
    pub status_extra_bits: u16,    // e.g. 0x0200 (an idle 0x0220 board)
    pub pid: u16,                  // 0x9899; 0x9980 = clone board (refused by detect/open)
    pub faults: Vec<Fault>,
    pub emission_cap_pct: Option<f64>, // S23 stand-in for tests of the bench cap; None by default
}
#[derive(Debug, Clone, PartialEq)]
pub enum Fault {
    StallWrites { after_packets: u32 }, ReadyNeverReturns { after_execute: bool }, BusyStuck,
    DisconnectMidPacket { packet: u32 }, PartialPacketAfterCancel, // a MODEL until H23 measures the board
    ReplyTimeouts { every: u32 },
}
/// Board model behind `LmcTransport`: READY/BUSY from the galvo time model, MO and port state, list execution,
/// and an emission recorder that decodes the wire independently (it uses only `lmc::op` constants, not
/// `lmc::decode_packet`, so it is a second opinion on the encoder).
pub struct FakeLmc { /* … */ }
impl FakeLmc {
    /// `Arc<dyn Clock>`: `VirtualClock` in T6, `SystemClock` behind `SimProvider` (the GUI's Simulator machine).
    pub fn new(cfg: SimConfig, clock: Arc<dyn Clock>) -> Self;
    pub fn recorder(&self) -> Arc<Recorder>; // shared; readable while the driver owns the FakeLmc
    /// Called on the driver thread for every wire event, before the board model reacts. T6 uses it to trip a
    /// `Stopper` at an exact wire event or virtual tick, so "stop at a random tick" is seeded and deterministic
    /// (a stop from another thread would land at a scheduler-dependent virtual time).
    pub fn set_hook(&self, hook: Box<dyn FnMut(&SimWire, MonoTime) + Send>);
}
impl LmcTransport for FakeLmc {}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Recording {
    pub wire: Vec<SimWire>,                 // single/packet/read with virtual timestamps (for T6 ordering asserts)
    pub burned: Vec<EmittedSegment>,        // MarkTo while executing, MO open, port bit 0 set
    pub lit: Vec<[[f64; 2]; 2]>,            // red-light traces (port bit 8 set, JumpTo)
    pub mo_open: Vec<(MonoTime, MonoTime)>,
    pub writes_while_not_ready: u32,        // must stay 0
    pub mo_closed_while_busy: u32,          // must stay 0 unless stopping
}
pub struct Recorder { /* Mutex<Recording> */ }
impl Recorder { pub fn snapshot(&self) -> Recording; }
/// Offline run of an encoded job (no driver thread): what `job_simulate` and `openlaser-cli simulate` show.
pub fn simulate_job(job: &LmcJob, m: &GalvoMachine) -> Recording;
/// DeviceProvider for the GUI/CLI "Simulator" machines (`GalvoLink::Sim`, `GantryLink::Sim`).
pub struct SimProvider { /* config, clock (SystemClock-paced by default) */ }
impl SimProvider { pub fn new(cfg: SimConfig) -> Self; pub fn last_recorder(&self) -> Option<Arc<Recorder>>; }
impl DeviceProvider for SimProvider {}
pub mod grbl {
    /// Rust port of openlaser-rust-plan/rayforge-test/fake_grbl.py: GRBL 1.1 banner, $$, $I, $G, $#, $H, $X, $J=,
    /// 15-block planner, 128-byte RX buffer, realtime ? ! ~ 0x18 0x85, alarms, status reports, motion time scaled.
    pub struct FakeGrbl { /* … */ }
    impl FakeGrbl { pub fn new(clock: Arc<dyn Clock>) -> Self; pub fn log(&self) -> Vec<String>; }
    impl ByteLink for FakeGrbl {}
}
```

### 3.15 `openlaser-transport` (WP5; compiled, never run by agents)

```rust
pub const ENV_NO_HW: &str = "OPENLASER_NO_HW";           // set → every open/detect returns HardwareDisabled
pub const ENV_HW_PRESENT: &str = "OPENLASER_HW_PRESENT"; // must equal "1", else HardwareDisabled (S23, default deny)
/// The real-device provider. open_lmc requires, in order: OPENLASER_NO_HW unset and OPENLASER_HW_PRESENT=1; a valid
/// bench session (S17); the board lock; vid/pid match (0x9980 → CloneBoard refusal, never firmware); the device
/// configured (macOS `claim_interface` cannot find interface 0 on an unconfigured device: call
/// `set_configuration(1)` only if the profile's `usb_set_configuration` is true, else refuse `UsbNotConfigured`);
/// then claims interface 0, bulk OUT 0x02, bulk IN 0x88. Holds a `keepawake` assertion for the session (§2.6).
pub struct UsbProvider { /* … */ }
impl UsbProvider { pub fn new() -> Self; }
impl DeviceProvider for UsbProvider {} // detect() also needs OPENLASER_NO_HW unset and OPENLASER_HW_PRESENT=1
/// nusb 0.2.8. Writes are exactly 12 or 3072 bytes; reads 8 bytes; `submit` + `wait_next_complete(≤ 20 ms)` slices;
/// each transfer has a total budget of `transfer_timeout_ms` (250), after which it is `XferError::Timeout`. NO
/// reset, NO retry, NO `transfer_blocking` (§2.3 rule 15): every error is mapped to XferError and surfaces as a sticky
/// fault plus an emergency stop.
pub struct UsbLmc { /* … */ }
impl LmcTransport for UsbLmc {}
/// Read-only enumeration (descriptors only; never opens a device).
pub fn detect() -> Result<Vec<DetectedBoard>, XferError>;
#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
pub struct DetectedBoard { pub vid: u16, pub pid: u16, pub serial: Option<String>, pub location: String, pub supported: bool, pub note: String }
/// `<data_dir>/OpenLaser/board.lock` (§2.6 paths), shared with the prototype (OM v1 patch 12, `flock`); File::try_lock.
pub struct BoardLock { /* File */ }
impl BoardLock { pub fn acquire() -> Result<Self, Refusal>; }
pub mod bench {
    /// `<data_dir>/OpenLaser/bench-session.json` {createdUnix, pid, emissions}; valid ≤ 2 h (wall clock; the file
    /// crosses processes). No tty name is stored: reading it needs `ttyname(3)` (unsafe); `IsTerminal` is enough.
    pub fn valid() -> Result<(), Refusal>;
    pub fn start_from_tty() -> Result<(), Refusal>; // only openlaser-hw calls it; stdin and stdout must be TTYs
    /// S23: `Some(10.0)` until one emission has completed in this bench session, then `None`.
    pub fn emission_cap_pct() -> Result<Option<f64>, Refusal>;
    pub fn record_emission() -> Result<(), Refusal>; // after a run finishes (not after a stop or fault)
}
pub struct SerialLink { /* serialport */ } impl ByteLink for SerialLink {}
pub struct TcpLink { /* std::net */ } impl ByteLink for TcpLink {}
```

### 3.16 `openlaser-engine` (WP6)

```rust
// ---- command.rs: the registry (ENG §2.2, with typed params) --------------------------------------------
pub type RunFn = fn(&mut Session, serde_json::Value) -> Result<serde_json::Value, EngineError>;
#[derive(Clone, Copy)]
pub struct CommandSpec {
    pub id: &'static str,                    // "layer.cutSettings.set" (camelCase segments, S2)
    pub label: &'static str,                 // English; also the i18n key
    pub menu: &'static [&'static str],       // LightBurn menu path; [] = not in menus
    pub shortcut: Option<&'static str>,      // "Cmd+Shift+H"; Cmd = ⌘ on macOS, Ctrl elsewhere
    pub kind: Kind,
    pub access: Access,
    pub doc: &'static str,                   // one-line params summary for command_list
    pub schema: fn() -> schemars::Schema,    // from the typed params struct (doc comments become descriptions)
    pub enabled: fn(&Session) -> Result<(), Disabled>,
    pub run: RunFn,
    pub checked: Option<fn(&Session) -> bool>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Kind { Edit, View, Query, Io, Settings, Device(DeviceClass), Request }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum DeviceClass { Stop, Connect, Light, Emit }
/// Who may run it; checked in dispatch before params are decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Agent, // any origin
    Roots, // any origin; for Synthetic/Headless/Replay every path param resolves inside the granted roots
    Ui,    // needs a window: Human or Synthetic; Headless → Disabled "needs the OpenLaser window (bridge mode)"
    Human, // Origin::Human only; anything else → Refused(HumanOnly). No menu path, no shortcut (S18)
    Stop,  // any origin; never disabled, never refused, never journaled
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Origin { Human, Synthetic, Headless, Replay }
#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
pub struct Disabled { pub reason: String, pub needs_document: bool }

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{id}`{}", suggest(.suggestions))] UnknownCommand { id: String, suggestions: Vec<String> },
    #[error("`{id}`: {pointer}: {message}")] BadParams { id: String, pointer: String, message: String },
    #[error("`{id}` is disabled: {reason}")] Disabled { id: String, reason: String, needs_document: bool },
    #[error("refused: {}", .0.message)] Refused(Refusal),          // policy and per-action safety
    #[error("job refused ({} refusals)", .0.refusals.len())] JobRefused(Box<JobRefused>),
    #[error("no document open")] NoDocument,
    #[error("no object {0:?}")] NoShape(ObjectId),
    #[error("this session has no device access")] NoDevice,
    #[error("needs bridge mode: turn on Settings › Automation › Allow agent connections in OpenLaser, then `openlaser-cli mcp --bridge auto`")] NeedsBridge,
    #[error("cancelled")] Cancelled,
    #[error("file: {0}")] Io(String),
    #[error("`{0}` failed with an internal error; the project is unchanged")] Internal(String),
    #[error("not implemented: {0}")] NotImplemented(&'static str),
}
pub struct Registry { /* specs: Vec<CommandSpec>, by_id: HashMap<&'static str, usize> */ }
impl Registry {
    pub fn find(&self, id: &str) -> Option<&CommandSpec>;   // exact, then snake_case alias (`flip_h` → `flipH`)
    pub fn suggestions(&self, id: &str) -> Vec<&'static str>;
    pub fn specs(&self) -> &[CommandSpec];
}
pub fn registry() -> &'static Registry;                     // OnceLock; duplicate ids fail `registry_ids_unique`
/// `command!("arrange.flipH", "Flip Horizontal", menu: ["Arrange"], shortcut: Some("Cmd+Shift+H"),
///   kind: Kind::Edit, access: Access::Agent, enabled: needs_selection, params: IdsParams, run: arrange::flip_h)`
/// expands to a CommandSpec whose `run` decodes `IdsParams` strictly, calls the typed fn and serialises the result.
#[macro_export] macro_rules! command { /* ENG §2.2 */ }
/// Normalises snake_case keys to camelCase (recursively), strips the universal keys `coalesce` and `dryRun`,
/// then decodes with deny_unknown_fields. Errors name the JSON pointer and the accepted keys.
pub fn decode<P: serde::de::DeserializeOwned>(id: &str, params: serde_json::Value) -> Result<P, EngineError>;

#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
pub struct CommandRow {
    pub id: String, pub label: String, pub menu: Vec<String>, pub shortcut: Option<String>, pub default_shortcut: Option<String>,
    pub kind: String, pub agent: AgentClass, pub params: String, pub enabled: bool, pub disabled_reason: Option<String>,
    pub needs_document: bool, pub schema: Option<serde_json::Value>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] #[serde(rename_all = "camelCase")]
pub enum AgentClass { Allowed, RootsOnly, Ui, HumanOnly, Stop }
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListFilter { pub filter: Option<String>, pub enabled_only: bool, pub agent: Option<AgentClass>, pub schema: bool }

// ---- session.rs ----------------------------------------------------------------------------------------
pub trait FileAccess: Send + Sync {                   // automation adapts control::AuthorizedWorkspace to this
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;
    fn write_atomic(&self, path: &str, bytes: &[u8]) -> Result<(), String>;
}
pub enum FsAuthority { Denied, Scoped(Box<dyn FileAccess>), TrustedLocal }
pub enum DeviceAccess { None, Provider(Arc<dyn DeviceProvider>) }
pub struct SessionConfig {
    pub origin: Origin,                 // default origin of execute(): Human (GUI, openlaser-hw), Headless (CLI/MCP)
    pub window: bool,                   // a GUI hosts this session (bridge requests possible)
    pub devices: DeviceAccess,          // None in openlaser-cli; Sim (+ Usb with `hw`) in the GUI and openlaser-hw
    pub machines: Vec<MachineProfile>,  // format::machines::load_all (built-ins shadowed by saved profiles)
    /// S24: Some only in the GUI and openlaser-hw. `device.settings.set`/`create`/`importEzcad`/`calibrateAxis`
    /// write through it and fail with NoDevice-style "profiles are read-only in this session" when None.
    pub profiles: Option<Arc<dyn ProfileStore>>,
    pub files: FsAuthority,
    pub clock: Arc<dyn Clock>,
}
pub struct Session { /* docs, active, registry, journal, devices hub, pending_arm, requests, events, origin… */ }
impl Session {
    pub fn new(cfg: SessionConfig) -> Self;
    pub fn id(&self) -> SessionId;
    // dispatch (pipeline below)
    pub fn execute(&mut self, id: &str, params: serde_json::Value) -> Result<serde_json::Value, EngineError>;
    pub fn execute_as(&mut self, origin: Origin, id: &str, params: serde_json::Value) -> Result<serde_json::Value, EngineError>;
    /// Non-atomic by default (PhotoCraft); `atomic: Some(label)` = one undo step, rolled back on any error;
    /// device commands are refused inside an atomic batch.
    pub fn batch(&mut self, origin: Origin, steps: Vec<(String, serde_json::Value)>, stop_on_error: bool, atomic: Option<String>) -> BatchResult;
    pub fn command_list(&self, origin: Origin, f: &ListFilter) -> Vec<CommandRow>;
    pub fn command_schema(&self, id: &str) -> Result<serde_json::Value, EngineError>;
    // documents
    pub fn docs(&self) -> &[DocState];
    pub fn active(&self) -> Option<&DocState>;
    pub fn active_index(&self) -> Option<usize>;
    /// The one way a document changes: runs `f` on a copy; on Ok swaps it in and records one history state
    /// (or merges into the previous one when `coalesce` matches); on Err nothing changes. Bumps the revision,
    /// voids PendingArm, marks submissions/frame requests stale, invalidates the live check.
    pub fn edit<R>(&mut self, label: Label, coalesce: Option<&str>, f: impl FnOnce(&mut Project, &mut Selection) -> Result<R, EngineError>) -> Result<R, EngineError>;
    pub fn undo(&mut self) -> Result<(), EngineError>;
    pub fn redo(&mut self) -> Result<(), EngineError>;
    /// doc_inspect JSON (§4.3).
    pub fn inspect(&self, index: Option<usize>, all_objects: bool) -> Result<serde_json::Value, EngineError>;
    // machines and jobs
    pub fn machines(&self) -> &[MachineProfile];
    pub fn machine_for(&self, index: Option<usize>) -> Option<&MachineProfile>;
    /// plan → validate → encode → disassemble → summary. Pure; never touches a device.
    pub fn build_job(&self, index: Option<usize>, opts: &JobOptions) -> Result<BuiltJob, JobRefused>;
    /// Cached build_job of the active doc for the canvas, Cuts badges and status bar; recomputed when stale.
    pub fn live_check(&mut self) -> Option<&LiveCheck>;
    // devices: the human path (Access::Human / Stop commands call these)
    pub fn connect(&mut self, origin: Origin, machine: &MachineId) -> Result<(), EngineError>;
    pub fn disconnect(&mut self, origin: Origin, machine: &MachineId) -> Result<(), EngineError>;
    /// `style`/`source` None = the project's JobSettings. Neither changes the fingerprint (S21), so no edit is made.
    pub fn start_frame(&mut self, origin: Origin, mode: FrameMode, style: Option<FrameStyle>, source: Option<FrameSource>) -> Result<Fingerprint, EngineError>;
    /// laser.start: cold-builds the job, requires a matching FrameReceipt, stores PendingArm (with the exact encoded
    /// job), raises ArmRequested. Never emits. The ARM dialog shows only `PendingArm.summary`, never a submission's.
    pub fn request_arm(&mut self, origin: Origin) -> Result<&PendingArm, EngineError>;
    pub fn pending_arm(&self) -> Option<&PendingArm>;
    pub fn cancel_arm(&mut self);
    /// Typed Rust API only (no command, no JSON, no control method takes a token). Checks PendingArm is current
    /// (same revision, fingerprint and bytes_hash), token.check, the provider's `emission_cap_pct` (S23),
    /// then moves `PendingArm.job` (the object whose bytes the summary was decoded from) into DriverHandle::run.
    /// It never re-encodes.
    pub fn run_armed(&mut self, token: ArmToken) -> Result<(), EngineError>;
    /// Also voids PendingArm (closes the ARM dialog) and marks running submissions stopped.
    pub fn stop_all(&mut self) -> Vec<(MachineId, StopAck)>;
    pub fn stop_registry(&self) -> Arc<StopRegistry>;
    pub fn status(&self, machine: &MachineId) -> Option<&CachedStatus>;
    /// Folds driver events into CachedStatus, receipts, requests; expires requests. Call every frame / tool call.
    /// A `Stopped` event or a tripped latch seen here also voids PendingArm (stops that came through the control
    /// fast path or a signal never touched the Session).
    pub fn poll(&mut self) -> Vec<Event>;
    // agent requests (bridge mode; NeedsBridge when !window)
    pub fn submit_job(&mut self, client: &str, note: &str) -> Result<SubmissionView, EngineError>;
    pub fn request_frame(&mut self, client: &str, mode: FrameMode) -> Result<FrameRequestView, EngineError>;
    pub fn approve_request(&mut self, origin: Origin, id: RequestId) -> Result<(), EngineError>; // Human only
    pub fn decline_request(&mut self, origin: Origin, id: RequestId) -> Result<(), EngineError>; // Human only
    pub fn withdraw_request(&mut self, id: RequestId) -> Result<(), EngineError>;
    pub fn requests(&self) -> &Requests;
}
#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
pub struct BatchResult { pub completed: usize, pub failed: usize, pub rolled_back: bool, pub results: Vec<serde_json::Value> }
pub struct DocState {
    pub project: Arc<Project>, pub history: History, pub selection: Selection, pub current_layer: LayerKey,
    pub path: Option<String>, pub machine: MachineId, pub revision: u64, pub saved_revision: u64,
}
pub struct History { /* undo: VecDeque<HistoryState>, redo: Vec<HistoryState>, max_states (default 100) */ }
impl History { pub fn can_undo(&self) -> bool; pub fn can_redo(&self) -> bool; pub fn labels(&self) -> Vec<String>; }
pub struct HistoryState { pub label: Label, pub project: Arc<Project>, pub selection: Selection }
#[derive(Debug, Clone, PartialEq)] pub struct Label { pub wording: &'static str, pub value: Option<String> } // "Set {name} Frequency"

#[derive(Debug, Clone, PartialEq, Default)]
pub struct JobOptions { pub machine: Option<MachineId>, pub selected_only: Option<bool> /* None = project setting */ }
pub enum Encoded { Lmc { job: Arc<LmcJob>, frame: Arc<LmcFrameJob> }, Gcode { job: Arc<GcodeProgram>, frame: Arc<GcodeFrame> } }
pub struct BuiltJob { pub plan: Arc<JobPlan>, pub validated: Validated, pub encoded: Encoded, pub summary: EmissionSummary, pub listing: Listing, pub burned: Vec<EmittedSegment> }
#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
pub struct JobRefused { pub fingerprint: Option<Fingerprint>, pub refusals: Vec<Refusal>, pub notices: Vec<Notice>, pub error: Option<String> }
pub struct LiveCheck { pub revision: u64, pub fingerprint: Option<Fingerprint>, pub refusals: Vec<Refusal>, pub notices: Vec<Notice>, pub estimate: Option<Duration>, pub summary: Option<EmissionSummary> } // display only: never armed from
pub struct PendingArm {
    pub fingerprint: Fingerprint, pub machine: MachineId, pub summary: EmissionSummary, pub receipt: FrameReceipt,
    pub checklist: Vec<String>, pub burned: Vec<EmittedSegment>, pub revision: u64, pub expires: MonoTime,
    /// S22: the encoded job `summary` was decoded from; `run_armed` sends exactly this (Arc → owned clone of the
    /// same value, re-checked by bytes_hash in the driver).
    pub job: Encoded,
}
pub struct CachedStatus {
    pub connected: bool, pub provider: Option<String>, pub state: MachineState, pub status_bits: Option<u16>,
    pub fault: Option<DriverError>, pub progress: Option<(u32, u32)>, pub last_stop: Option<StopReport>,
    pub receipt: Option<FrameReceipt>, pub trace_tail: VecDeque<TraceLine>,
}
pub struct StopRegistry { /* Mutex<Vec<(MachineId, Stopper)>>; a poisoned lock is recovered */ }
impl StopRegistry { pub fn stop_all(&self, max_wait: Duration) -> Vec<(MachineId, StopAck)>; }
#[derive(Debug, Clone, PartialEq)]
pub enum Event { DocChanged { index: usize, revision: u64 }, ArmRequested, ArmVoided { reason: String }, Driver { machine: MachineId, event: DriverEvent }, Request { id: RequestId, state: String }, Status(String) }

// ---- requests.rs (AUT C3.4/C3.5 state machines; red zone review) ----------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)] pub struct RequestId(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] #[serde(rename_all = "snake_case")]
pub enum SubmissionState { AwaitingHuman, Framing, AwaitingArm, Running, Finished, Stopped, Fault, Rejected, Withdrawn, Superseded, Stale, Expired }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] #[serde(rename_all = "snake_case")]
pub enum FrameRequestState { AwaitingHuman, Framing, Done, Declined, Stale, Expired, Stopped }
#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
pub struct SubmissionView { pub submission: RequestId, pub machine: MachineId, pub fingerprint: Fingerprint, pub state: SubmissionState, pub note: String, pub client: String, pub summary: EmissionSummary, pub expires_in_s: u64, pub outcome: Option<Outcome>, pub stop_report: Option<StopReport> }
#[derive(Debug, Clone, PartialEq, Serialize)] #[serde(rename_all = "camelCase")]
pub struct FrameRequestView { pub request: RequestId, pub machine: MachineId, pub fingerprint: Fingerprint, pub mode: FrameMode, pub state: FrameRequestState, pub client: String, pub expires_in_s: u64 }
/// ≤ 1 pending submission and ≤ 1 pending frame request per machine (a new one supersedes); submissions expire
/// after 15 min, frame requests after 2 min; any revision bump → Stale. Nothing here touches a driver except
/// approve_request (Human), which calls start_frame exactly as the human's own Frame button does.
pub struct Requests { /* … */ }
impl Requests { pub fn submissions(&self) -> Vec<SubmissionView>; pub fn frame_requests(&self) -> Vec<FrameRequestView>; }
```

**Dispatch pipeline** (`execute_as`; ENG §2.4):

1. `find(id)` (exact, then alias) or `UnknownCommand` with suggestions.
2. Params must be an object or null. Split the universal keys `coalesce` and `dryRun`.
3. **Policy** `access × origin` (table below) → `Refused(HumanOnly)` / `Disabled` before any decode or side effect.
4. **Roots**: for `Roots` commands with a non-Human origin, every path param resolves through `FsAuthority`
   (`Denied` → `Io("automation filesystem access is not granted")`).
5. Strict decode → `BadParams { pointer }`.
6. `enabled(&Session)` → `Disabled { reason, needs_document }`.
7. Device busy: `Device(Light | Emit | Connect)` while the machine is framing or running → `Refused(MachineBusy)`.
8. `catch_unwind(run)` → `Internal`, project unchanged.
9. After: journal (`Edit`/`Io` only; never `Device`), revision bump on change, `Event`s.

| Access | Human | Synthetic (GUI driven by control) | Headless (CLI, MCP) | Replay (batch steps, macros) |
|---|---|---|---|---|
| Agent | ✓ | ✓ | ✓ | per its origin |
| Roots | ✓ | roots only | roots only | per its origin |
| Ui | ✓ | ✓ | Disabled (needs window) | per its origin |
| Human | ✓ | **Refused(HumanOnly)** | **Refused(HumanOnly)** | **Refused(HumanOnly)** |
| Stop | ✓ | ✓ | ✓ (`NoDevice` error if no device, never a refusal) | ✓ |

`Origin::Human` comes only from the GUI's own untainted input and from `openlaser-hw` (§2.3 rule 13); every
`openlaser-cli` subcommand, `run --cmd` included, is `Headless`. Anything that changes what `build_job` would
produce bumps the revision (so it voids PendingArm and stales requests): document edits, undo/redo,
`device.select`, `laser.repeat`, `laser.cutSelected`, and `prefs.set` of keys the planner reads (`output.*`).

### 3.17 `openlaser-control` (WP7)

```rust
pub mod budgets { // PhotoCraft's constants plus AUT B12
    pub const MAX_REQUEST_BYTES: usize = 1 << 20;   pub const MAX_RESPONSE_BYTES: usize = 8 << 20;
    pub const MAX_PNG_BYTES: usize = 5 << 20;       pub const PREVIEW_DEFAULT_SIDE: u32 = 1024; pub const PREVIEW_MAX_SIDE: u32 = 2048;
    pub const BATCH_MAX_STEPS: usize = 256;         pub const MAX_CONNECTIONS: usize = 16;
    pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30); pub const APP_DEADLINE: Duration = Duration::from_secs(60);
    pub const BRIDGE_TIMEOUT: Duration = Duration::from_secs(55); pub const RECONNECT_IDLE: Duration = Duration::from_secs(25);
    pub const STOP_WAIT: Duration = Duration::from_millis(500);
    pub const SIM_LISTING_MAX_ROWS: usize = 2000;   pub const SIM_LISTING_MAX_BYTES: usize = 256 << 10;
    pub const INSPECT_OBJECTS_DEFAULT: usize = 200; pub const NOTE_MAX_CHARS: usize = 280;
    pub const SUBMISSION_TTL: Duration = Duration::from_secs(900); pub const FRAME_REQUEST_TTL: Duration = Duration::from_secs(120);
}
pub mod token {
    pub struct Token([u8; 32]);                     // 64 lowercase hex on the wire; constant-time compare
    impl Token { pub fn generate() -> Result<Self, Error>; pub fn from_hex(s: &str) -> Result<Self, Error>; pub fn to_hex(&self) -> String; pub fn matches(&self, other: &str) -> bool; }
    pub fn create_or_reuse_file(path: &Path) -> Result<Token, Error>; // create_new, mode 0600
    pub fn read_file(path: &Path) -> Result<Token, Error>;
}
pub mod discovery {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Discovery { pub version: u32, pub pid: u32, pub port: u16, pub token: String, pub started: String, pub roots: Roots }
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] pub struct Roots { pub read: Option<String>, pub write: Option<String> }
    pub fn default_path() -> PathBuf;               // ~/Library/Application Support/OpenLaser/automation/control.json
    pub fn write(path: &Path, d: &Discovery) -> Result<(), Error>; // dir 0700, file 0600, atomic
    pub fn read(path: &Path) -> Result<Discovery, Error>;          // Err if the pid is gone (stale)
    pub fn remove(path: &Path);
}
pub mod wire {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] pub struct Request { pub id: serde_json::Value, pub method: String, #[serde(default)] pub params: serde_json::Value }
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] pub struct Reply { pub id: serde_json::Value, pub ok: bool, #[serde(skip_serializing_if = "Option::is_none")] pub result: Option<serde_json::Value>, #[serde(skip_serializing_if = "Option::is_none")] pub error: Option<String> }
    /// Reads a whole line before returning any byte; a line over `max` is an error (the caller closes or skips).
    pub fn read_line_bounded(r: &mut impl std::io::BufRead, max: usize) -> Result<Option<Vec<u8>>, Error>;
    /// Oversized replies are replaced by a small error reply with the same id ("…operation may have completed").
    pub fn write_reply(w: &mut impl std::io::Write, reply: &Reply) -> std::io::Result<()>;
}
pub mod methods {
    /// The exact control-method table (§4.5). `control_methods_table_is_exact` pins it.
    pub const METHODS: &[&str];
    pub fn is_read_only(method: &str) -> bool;
}
pub mod workspace {
    /// cap-std directory capabilities; separate read and write roots; missing root = denied (fail closed).
    /// Path grammar: non-empty, '/'-separated, relative; no '\', ':', leading '/', empty, '.', '..' component,
    /// no component ending in '.' or space; an absolute path is accepted only beneath the root and re-checked.
    pub struct AuthorizedWorkspace { /* read: Option<cap_std::fs::Dir>, write: Option<Dir> */ }
    impl AuthorizedWorkspace {
        pub fn denied() -> Self;
        pub fn open(read: Option<&Path>, write: Option<&Path>) -> Result<Self, Error>;
        pub fn read(&self, path: &str) -> Result<Vec<u8>, Error>;
        pub fn write_atomic(&self, path: &str, bytes: &[u8]) -> Result<(), Error>;
    }
}
pub mod server {
    pub struct ControlRequest { pub conn: u64, pub client: String, pub request: wire::Request, pub reply: std::sync::mpsc::Sender<wire::Reply>, pub deadline: std::time::Instant, pub dispatch: Arc<Dispatch> }
    /// QUEUED → DISPATCHED (UI thread starts it) or QUEUED → ABANDONED (deadline passed), settled once, so a
    /// timeout says either "not run, safe to resend" or "may still apply".
    pub struct Dispatch { /* AtomicU8 */ }
    impl Dispatch { pub fn try_dispatch(&self) -> bool; pub fn abandon(&self) -> bool; }
    /// `machine.stop` is answered on the transport thread through this, before anything touches the UI queue.
    pub type FastStop = Arc<dyn Fn(Option<&str>) -> serde_json::Value + Send + Sync>;
    pub struct ServerConfig { pub port: u16 /* 0 = free port */, pub token: token::Token, pub fast_stop: FastStop, pub wake: Arc<dyn Fn() + Send + Sync> }
    /// std::net on 127.0.0.1 only; ≤ 16 connections; 30 s idle; first frame must be
    /// {"method":"auth","params":{"token","client":{"name","version"}}}. A connection serves one request at a time
    /// (it waits for the UI's reply before reading the next line), so `machine.stop` sent on a busy connection would
    /// wait behind it: clients send stop on a connection of its own (`BridgeClient::stop_handle`). When the 16
    /// slots are full, a 17th connection is still accepted for exactly one `auth` + `machine.stop` exchange and
    /// then closed; any other method on it gets "connection limit reached".
    pub struct ControlServer { /* … */ }
    impl ControlServer {
        pub fn start(cfg: ServerConfig) -> Result<(Self, std::sync::mpsc::Receiver<ControlRequest>), Error>;
        pub fn port(&self) -> u16;
        pub fn clients(&self) -> Vec<String>;
        pub fn disconnect_all(&self);
        pub fn shutdown(self);
    }
}
```


### 3.18 `openlaser-automation` (WP7)

```rust
pub struct McpConfig { pub backend: BackendChoice, pub read_root: Option<PathBuf>, pub write_root: Option<PathBuf>, pub compact: bool }
pub enum BackendChoice { Headless, Bridge(BridgeTarget) }
pub enum BridgeTarget { Auto /* discovery file */, File(PathBuf), Addr { addr: std::net::SocketAddr, token: TokenSource } }
pub enum TokenSource { Inline(String), File(PathBuf), Env /* OPENLASER_CONTROL_TOKEN[_FILE] */ }
/// stdio MCP (rmcp 3.5) behind PhotoCraft's `serve_io` pre-reader: 1 MiB line cap ends the session, -32700
/// recovery, single writer, `_meta` 2026-07-28 list fields. Server info {name: "openlaser", version}.
pub fn run_mcp_stdio(cfg: McpConfig) -> Result<(), Error>;
pub struct ServeConfig { pub port: Option<u16>, pub token: Option<TokenSource>, pub read_root: Option<PathBuf>, pub write_root: Option<PathBuf> }
/// `openlaser-cli serve`: the control protocol over a headless Session (stdio: no auth; TCP: auth frame).
pub fn run_serve(cfg: ServeConfig) -> Result<(), Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Mode { Headless, Bridge }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum ToolAccess { Read, Agent, Approval, Stop }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct Hints { pub read_only: bool, pub destructive: bool, pub idempotent: bool }
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: fn() -> serde_json::Value, // strict: additionalProperties:false everywhere, $refs inlined
    pub modes: &'static [Mode],
    pub access: ToolAccess,
    pub hints: Hints,
    pub compact: bool,                           // listed under --compact (`stop` always is)
}
pub fn tools() -> &'static [ToolDef];           // §4.1, in that order
pub fn strict_schema(s: schemars::Schema) -> serde_json::Value;
pub struct ClientInfo { pub name: String, pub version: String }
pub enum ToolOutput { Json(serde_json::Value), Png { png: Vec<u8>, note: serde_json::Value }, Error { message: String, data: Option<serde_json::Value> } }
/// The MCP layer is identical over both backends. Tool calls run one at a time behind a mutex (PhotoCraft), EXCEPT
/// `stop`, which the server routes to `stop_handle()` without taking that mutex, so a long `job_simulate` or a
/// 55 s bridge timeout can never delay a stop.
pub trait ToolBackend: Send {
    fn mode(&self) -> Mode;
    fn call(&mut self, tool: &str, args: serde_json::Value, client: &ClientInfo) -> ToolOutput;
    /// Bridge: a handle with its own authenticated connection, used only for `machine.stop`. Headless: None (the
    /// `stop` tool then returns the `needs_bridge:` error).
    fn stop_handle(&self) -> Option<StopHandle>;
}
#[derive(Clone)] pub struct StopHandle { /* Arc<Mutex<dedicated BridgeClient connection>> */ }
impl StopHandle { pub fn stop(&self, machine: Option<&str>) -> ToolOutput; } // waits ≤ STOP_WAIT for the StopAck
/// engine::Session with origin Headless, window false, DeviceAccess::None; files through AuthorizedWorkspace.
pub struct Headless { /* … */ }
impl Headless { pub fn new(read_root: Option<&Path>, write_root: Option<&Path>) -> Result<Self, Error>; }
impl ToolBackend for Headless {}
pub struct Bridge { /* BridgeClient */ }
impl ToolBackend for Bridge {}
pub struct BridgeClient { /* one authenticated loopback connection */ }
impl BridgeClient {
    pub fn connect(target: &BridgeTarget, client: &ClientInfo) -> Result<Self, BridgeError>;
    pub fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, BridgeError>;      // never resent
    pub fn call_read(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, BridgeError>; // one retry
    /// Opens a SECOND authenticated connection reserved for `machine.stop` (reconnects on use if idle-dropped).
    pub fn stop_handle(&self) -> Result<StopHandle, BridgeError>;
}
/// Wording per AUT A8: NotSent "the request was not sent"; Transport "operation may have completed; inspect state
/// before retrying"; TimedOut read "may be retried safely" / write "may have completed"; Definite = the app's error.
pub enum BridgeError { NotSent(String), Transport(String), TimedOut { read: bool }, Definite(String), NotImplemented(&'static str) }
```

### 3.19 `openlaser-ui-egui` (WP8)

```rust
pub struct OpenLaserApp { /* session: Session, ui: UiState, services: Services, control rx, synthetic queue … */ }
impl OpenLaserApp {
    pub fn new(cc: &eframe::CreationContext<'_>, session: Session, services: Services) -> Self;
    pub fn with_control(self, rx: std::sync::mpsc::Receiver<ControlRequest>) -> Self;
    pub fn session(&self) -> &Session;
    pub fn ui_state(&self) -> &UiState;
}
impl eframe::App for OpenLaserApp {
    /// (1) Esc while a machine is framing, armed or marking, or while the ARM dialog is open → session.stop_all()
    /// (latch first, then PendingArm voided) and remove the event before any widget sees it; (2) native-menu key
    /// equivalents; (3) queued synthetic events go in only through services.human.inject() (§2.3 rule 16).
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput);
    /// Every frame, and while hidden whenever a repaint was requested (eframe 0.36 calls `logic` for a hidden window
    /// only then: the control server's `wake` and a 250 ms `request_repaint_after` while a machine is active keep it
    /// running): session.poll(); drain the control channel (control::handle); native menu clicks; shortcuts
    /// (Origin::Synthetic if services.human.frame_is_synthetic()); file drops; close guard (machine active →
    /// CancelClose + stop first; craft_quit last).
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame);
    /// The LightBurn layout (UI §4.1), STOP overlay last at Order::Tooltip.
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame);
}
/// Everything platform- or safety-specific is injected; ui-egui does no file or device I/O itself.
pub struct Services {
    pub human: Box<dyn HumanSurfaces>,
    pub files: Box<dyn FileDialogs>,              // async (rfd) in the app; scripted in tests
    pub settings: Box<dyn SettingsStore>,         // craft-settings in the app; in memory in tests
    pub clipboard: Box<dyn Clipboard>,
    pub automation: Box<dyn AutomationToggle>,    // Settings › Automation › Allow agent connections (a human-only switch)
    pub quit: Box<dyn Fn() + Send>,
}
pub trait FileDialogs: Send { fn open(&mut self, kind: DialogKind) -> DialogTicket; fn poll(&mut self, t: DialogTicket) -> Option<Option<(String, Vec<u8>)>>; fn save(&mut self, kind: DialogKind, bytes: Vec<u8>) -> DialogTicket; }
pub trait SettingsStore: Send { fn load(&self) -> UiSettings; fn save(&mut self, s: &UiSettings); }
pub trait Clipboard: Send { fn get(&mut self) -> Option<String>; fn set(&mut self, text: &str); }
pub trait AutomationToggle: Send { fn set(&mut self, on: bool) -> Result<Option<ControlInfo>, String>; fn info(&self) -> Option<ControlInfo>; }
pub struct ControlInfo { pub port: u16, pub clients: Vec<String> }

/// Implemented by openlaser-human (the app injects it). ui-egui never links openlaser-arm.
pub trait HumanSurfaces: Send {
    /// The only way control-channel input enters egui. Taints this frame, the next two, and everything until 1 s
    /// after the last injected event (UI §7.3), so a click whose press and release span frames, or whose press was
    /// synthetic, never counts. While a human-only surface is open, queued synthetic events are dropped.
    fn inject(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput, events: Vec<egui::Event>);
    /// True if this frame carries automation input.
    fn frame_is_synthetic(&self, ctx: &egui::Context) -> bool;
    /// Gate for every human-only widget (Connect, Frame, Start, Approve, Device-settings Save, Allow Agent
    /// Connections): primary-button click, untainted frame, no AccessKit action this frame, focused window,
    /// widget visible ≥ 300 ms.
    fn is_human_click(&mut self, ctx: &egui::Context, response: &egui::Response) -> bool;
    /// Draws the ARM confirmation as a Modal (STOP stays above). Mints inside; returns the token once.
    fn arm_dialog(&mut self, ctx: &egui::Context, model: &ArmModel) -> ArmOutcome;
    /// Any human-only surface open: the dispatcher refuses mutating control methods (human_confirmation_open).
    fn surface_open(&self) -> bool;
}
pub struct ArmModel { pub summary: EmissionSummary, pub receipt: FrameReceipt, pub checklist: Vec<String>, pub session: SessionId, pub now: MonoTime, pub thumbnail: DisplayList, pub emission_cap_pct: Option<f64> /* S23, from the provider */ }
pub enum ArmOutcome { Open, Cancelled, Armed(ArmToken) }
/// Test double: nothing is ever human, nothing ever arms. ui-egui's own kittests use it.
pub struct NoHuman;
impl HumanSurfaces for NoHuman {}

pub mod control {
    pub enum Outcome { Done(Reply), AfterInput(Reply), Screenshot { max_side: u32, reply: Sender<Reply> } }
    /// Methods §4.5. Human-only commands and surfaces are refused here (AND in engine policy).
    pub fn handle(app: &mut OpenLaserApp, ctx: &egui::Context, req: ControlRequest) -> Outcome;
}
pub mod menu_catalog {
    pub struct Row { pub path: &'static [&'static str], pub label: &'static str, pub shortcut: Option<&'static str>, pub id: &'static str }
    /// Every LightBurn menu item from CAT §3, in LightBurn order, "---" separators, ids converted to camelCase.
    pub static CATALOG: &[Row];
}
pub mod parity { pub const FLOOR: usize; pub fn live_count(reg: &Registry) -> (usize, usize); }
pub mod canvas {
    pub struct ViewXform { pub zoom: f64 /* pt per mm */, pub center: [f64; 2], pub rect: egui::Rect }
    pub trait CanvasRenderer { fn paint(&mut self, painter: &egui::Painter, list: &DisplayList, view: &ViewXform); }
    pub struct PainterRenderer { /* own hairline mesher + mesh cache (UI §5.3) */ }
    /// Paints a display list fitted into `rect` (used by human's ARM-dialog thumbnail and the Preview window).
    pub fn paint_fit(painter: &egui::Painter, list: &DisplayList, rect: egui::Rect);
}
```

### 3.20 `openlaser-human` (WP9, red zone)

```rust
/// The taint record lives in this struct (keyed by `ctx.cumulative_pass_nr()` plus the time of the last injected
/// event), NOT in egui context data: any code holding the `Context` can clear `ctx.data_mut(..)`, while `Human` is
/// reachable only through `&mut dyn HumanSurfaces`.
pub struct Human { /* taint record; surface-visible timers; ARM dialog state */ }
impl Human { pub fn new(clock: Arc<dyn Clock>) -> Self; }
impl openlaser_ui_egui::HumanSurfaces for Human {}
/// (no Clone) The witness for one real click.
pub struct HumanGesture { /* … */ }
/// Some only if: response.clicked_by(PointerButton::Primary); the frame is untainted; no AccessKit action request
/// this frame; window focused; widget visible ≥ 300 ms. (egui 0.36.2: clicked_by excludes keyboard and AccessKit.)
pub fn gesture(ctx: &egui::Context, response: &egui::Response, visible_since: MonoTime, now: MonoTime) -> Option<HumanGesture>;
```

ARM dialog behaviour (normative): built only from `ArmModel.summary` (decoded bytes) and a thumbnail of
`ArmModel.thumbnail` (decoded emitted segments, never the document); per row: layer, mode, pass/of, power %,
kHz, ns, **actual** mm/s, paths, bounds, time; then repeats, numbered notices, total time, post-crash emission
bound, machine name, fingerprint short. The checklist comes from the profile. The `ARM` field accepts text only in
untainted frames; any tainted frame while the dialog is open clears the field and every tick and shows "Automation
input reached this dialog and was ignored." Confirm is a human-only widget; on a human click it calls
`HumanConfirm::from_gui_gesture` then `openlaser_arm::mint`, and returns `Armed(token)` once. Typed text never
persists between openings. Any revision bump or any stop closes it (engine voids `PendingArm`). Over a bench
session's first emission (S23) the dialog shows the 10 % cap and refuses rows above it before minting.

### 3.21 Apps (WP8, WP7, WP9)

```
openlaser [--control <port>] [--control-token-file <path>] [--automation-read-root <dir>]
          [--automation-write-root <dir>] [--in-window-menus] [files…]
  Machines: "JPT M7 · Simulator" (always) and, only in `--features hw` builds, "JPT M7 · USB".
  A bad --control port is a usage error (exit 2). No --mcp-bridge flag: Window › Assistant gives
  `openlaser-cli mcp --bridge auto`.
  Before `run_native`: a panic hook that calls StopRegistry::stop_all(500 ms) before the default hook, and a
  signal watcher (signal-hook flags on unix, ctrlc on windows) that trips every latch, waits ≤ 500 ms for the
  StopReports, then exits 143. Profiles load from `format::machines`; `DirProfileStore` is injected (S24).

openlaser-cli mcp   [--bridge auto|<127.0.0.1:port> | --bridge-file <control.json>]
                    [--control-token <hex> | --control-token-file <path>]
                    [--automation-read-root <dir>] [--automation-write-root <dir>] [--root <dir>] [--compact]
openlaser-cli serve [--port <port>] [token flags] [root flags]
openlaser-cli ui    (--bridge-file <f> | --bridge <addr> --control-token-file <f>) <method> [key=value…] [--out shot.png]
openlaser-cli commands [--json] [--filter <text>]
openlaser-cli info  <project.olp|file.svg|file.dxf|workspace.json> [--compact]
openlaser-cli convert <in> <out>                         .olp ⇄ .svg/.dxf/workspace.json in; .olp out
openlaser-cli run   (<project> | --new <json>) --cmd <id> [--params <json>] … [--out <file.olp>]
openlaser-cli plan  <project> [--machine <id>] [--json]  refusals + decoded summary; exit 1 on refusal
openlaser-cli preview <project> --out <png> [--show burn,frame,travel] [--max-side N]
openlaser-cli simulate <project> --out <png> [--listing <txt>]   FakeLmc run of the exact bytes
openlaser-cli estimate <project>
openlaser-cli gcode <project> --machine <gantry id> --out <file.gcode>
openlaser-cli job-export <project> --out <job.json>      packets (hex) + listing + summary, for review
openlaser-cli frame|mark|run-hw …                         → "hardware commands live in openlaser-hw and are run
                                                             by a person at a terminal" (exit 2; never forwards)
  Unknown flags and non-Unicode args are usage errors (exit 2). 0 ok, 1 failure/refusal, 2 usage.
  Without roots, file access in mcp/serve fails closed; the other subcommands are trusted-local.

openlaser-hw detect                                       read-only enumeration (needs `hw`)
openlaser-hw status  [--machine <id>] [--sim] [--trace <f>]   GetSerialNo + GetVersion only, no init (H12)
openlaser-hw bench   start|status|end                     TTY only; start writes the bench-session file
openlaser-hw profile show|set <key> <value>|import-om <calibration.json>|import-ezcad <markcfg>  [--machine <id>]
                                                          TTY only; the only CLI path that writes a profile (S24):
                                                          prints old → new values and asks `SAVE <id>`; keys are
                                                          cal.negate.x/y, cal.swapXy, cal.offsetMm, cal.scale,
                                                          runAhead.*, transferTimeoutMs, usbSetConfiguration,
                                                          redDot.*, workArea (within h27Cleared), h27Cleared
openlaser-hw frame   (<project> | --builtin <pattern>) [--loop] [--style bounds|contour] [--source job|tools|both]
                     [--extent <mm>] [--machine <id>] [--sim] [--trace <f>]
openlaser-hw run     (<project> | --builtin <pattern>) [--layers <layer_settings.json>] [--repeats N] [--power-pct P]
                     [--allow-emission] [--machine <id>] [--sim] [--trace <f>]
  --builtin: dot | line | box10 | square50 | f-shape | corners. dot, line and box10 are exactly the T1
  dot_test, line_test and mark_box10 inputs, so a hardware trace diffs against the rust-v1 candidate; square50 is
  HIL-2's 50 mm square, f-shape the asymmetric H4 frame, corners the H27 corner frames. `--power-pct` (built-ins only)
  overrides power for every row. `--extent <mm>` (frame only, corners only) traces up to ±extent, allowed past the
  work area but still inside the strictly encodable square: red light only, for H27; `run` never accepts it.
  --trace <f>: writes every wire event as JSON lines in the T1 event format (kind, op, hex, status, packet sha256)
  plus timestamps; HIL sessions commit it (§8.2) and it becomes a FakeLmc fixture.
  run: connect (board lock) → print the decoded summary → frame in a loop → Enter is accepted only after ≥ 1
  fully EXECUTED pass (S20) → stop framing → without --allow-emission stop here (EmissionNotAllowed) → print the
  summary, the profile checklist and a 6-digit challenge → type `ARM <challenge>` → mark. The first emission of a
  bench session refuses any row above 10 % power (BenchPowerCap, S23); after a finished run the engine calls the
  provider's `record_emission` (`poll` on `Finished`).
  Esc, Space, Ctrl-C (key thread, crossterm raw mode) and SIGINT/SIGTERM/SIGHUP (signal-hook flags; ctrlc on
  windows) trip the latch; on a signal the main thread waits ≤ 500 ms for the StopReport, prints it, then exits.
  Refused unless stdin and stdout are TTYs (NotATty). Real USB needs `hw`, OPENLASER_HW_PRESENT=1, a bench session
  and the board lock; `--sim` drives FakeLmc. The binary's logic lives in `src/lib.rs` behind a `Terminal` trait so
  tests run it with a scripted terminal. `Terminal::confirm_arm(...) -> Option<HumanConfirm>` is implemented by
  the real terminal in `src/tty_arm.rs`, the only file that builds a TtyWitness (after `IsTerminal` on stdin and
  stdout); the scripted terminal lives in `tests/` and gets its HumanConfirm from `arm::testing` (dev-dependency).
```

### 3.22 `openlaser-testkit` (WP0 skeleton; each loader module is owned by the WP that uses it)

```rust
pub fn repo_root() -> PathBuf;                // from CARGO_MANIFEST_DIR
pub fn golden_dir() -> PathBuf;               // <repo>/tests/golden
pub fn fixture(name: &str) -> PathBuf;        // <repo>/tests/fixtures/<name>
pub fn sha256_hex(bytes: &[u8]) -> String;
pub fn hex_decode(s: &str) -> Result<Vec<u8>, String>;
pub mod t0 { pub struct Table { pub function: String, pub columns: Vec<String>, pub rows: Vec<Vec<serde_json::Value>>, pub errors: Vec<Vec<serde_json::Value>>, pub specials: Vec<Vec<serde_json::Value>>, pub groups: Vec<Group> } pub fn load(file: &str) -> Result<Table, String>; } // WP4
pub mod t1 { pub enum Event { Open, Single { op: String, hex: String }, Read { status: u16 }, List { sha256: String, n_cmds: u16 }, Close }
             pub struct Scenario { pub scenario: String, pub inputs: serde_json::Value, pub field_size_mm: f64, pub events: Vec<Event>, pub segments: BTreeMap<String, [usize; 2]> }
             pub fn load(oracle: &str, scenario: &str) -> Result<Scenario, String>; pub fn packet(sha256: &str) -> Result<Vec<u8>, String>; } // WP4
pub mod t2 { /* index.json cases, machines, scenarios; number_format rows */ } // WP2
pub mod t3 { /* hatch cases: name, polylines, params, output */ }              // WP1
pub mod t4 { /* import cases: name, kind, function, filename, source, rust_must_match, output */ } // WP2
pub mod adversarial { pub fn params() -> Vec<serde_json::Value>; }           // WP6: PhotoCraft's four + NaN/inf strings, 1e308, 0 Hz, 1e9 kHz, unknown keys, deep arrays
```

---

## 4. Agent surface: MCP tools, control methods and the emission boundary

The design source is AUT Parts B and C; this section is the normative summary. Agents get the whole
LightBurn-equivalent workflow up to the burn, a byte-exact view of the burn (`job_simulate`), status, and stop.
They never get emission, device connection, framing without a human click, or machine-profile writes.

### 4.1 Tools

Conventions: tool names and tool arguments are snake_case; command ids are camelCase with snake aliases; every
result is camelCase JSON in one text content (PNG tools: image content plus a JSON note). Every input schema is
strict (`additionalProperties:false`; unknown key → JSON-RPC `-32602` naming it). **No schema anywhere has a
property named `force`, `proceed`, `arm`, `confirm`, `skip_frame`, `override` or `token`** (CI grep:
`schemas_have_no_override_fields`). Modes: **H** headless (`openlaser-cli mcp`), **B** bridge
(`openlaser-cli mcp --bridge auto`). Hints ro/destr/idem as AUT A4; `stop` is deliberately not destructive.

| Tool | Arguments (bold = required) | Result | Modes | Access |
|---|---|---|---|---|
| `session_list` | — | `{documents:[{index,name,path,dirty,revision,machine}], active}` | H B | read |
| `doc_new` | `name?, machine?` | `{index, name, machine}` | H B | agent |
| `doc_open` | **`path`** (`.olp`; `.svg/.dxf/.png…/workspace.json` open as a new project) | `{index, name, layers, objects, notices}` | H B | agent |
| `doc_import` | **`path`**, `layer?, center?:[x,y], scale?, svg_units?:"physical"\|"px96"\|"px72"` | `{objects:[ids], bounds, notices}` | H B | agent |
| `doc_save` | `path?` (`.olp`) | `{path, warnings}` | H B | agent |
| `doc_export` | **`path`**, `format?:"svg"` | `{path}` (P0 stretch) | H B | agent |
| `doc_inspect` | `index?, objects?:"summary"\|"all"` | §4.3 | H B | read |
| `doc_select` / `doc_close` | **`index`** / `index?` | `session_list` | H B | agent |
| `render_preview` | `index?, max_side?(1024, ≤2048), view?:"design"\|"toolpath"` | PNG of the **document** (never the window) | H B | read |
| `command_list` | `filter?, enabled_only?, agent?, schema?` | bare array of rows (§4.3) | H B | read |
| `command_schema` | **`id`** | `{id, schema}` | H B | read |
| `command_run` | **`id`**, `params?`, `wait?` | the command's JSON | H B | agent |
| `command_batch` | **`steps:[{id, params?, wait?}]`** (≤ 256; `wait` per step as PhotoCraft), `stop_on_error?=true`, `atomic?=false`, `label?` | `{completed, failed, rolledBack, results:[{ok, result\|error}]}` | H B | agent |
| `jobs_list` / `jobs_cancel` | — / `job?` | background tasks | H B | read / agent |
| `machine_list` | — | `[{id, name, kind, workArea, source:{maxPowerPct, freqKhz:[lo,hi], pulsesNs}, link, connected}]` | H B | read |
| `machine_status` | `machine?` | §4.3 (headless: `{connected:false, backend:"headless"}`) | H B | read |
| `job_plan` | `index?, machine?, selected_only?` | §4.3; a refusing plan is a successful call with `ok:false` | H B | read |
| `job_preview` | `index?, max_side?, show?:["burn","frame","travel","order"]` | PNG + `{fingerprint, bounds}` | H B | read |
| `job_estimate` | `index?` | `{fingerprint, estimateS, perSection:[…], crashBoundS}` | H B | read |
| `job_simulate` | `index?, max_side?, listing?` | PNG of the FakeLmc emission of the exact bytes + `EmissionSummary` + `listing?` (≤ 2000 rows / 256 KiB, `truncated`) | H B | read |
| `job_export` | **`path`**, `index?` | `{path, fingerprint}` (packets hex + listing + summary JSON) | H B | agent |
| `gcode_export` | **`path`**, `index?, machine?` | `{path, lines, fingerprint}`; gantry profiles only | H B | agent |
| `frame_request` | `mode?:"once"\|"loop", machine?, index?` | `{request, state:"awaiting_human", fingerprint}` | B | approval |
| `job_submit` | `machine?, index?, note?` (≤ 280 chars, plain text) | `{submission, state:"awaiting_human", fingerprint, summary, expiresInS}`; with refusals: tool error, nothing created | B | approval |
| `job_status` | `submission?` | `{submission, state, fingerprint, outcome?, stopReport?, fault?}` | B | read |
| `job_withdraw` | **`submission`** | `{submission, state:"withdrawn"}` | B | agent |
| `stop` | `machine?` | `{latched:true, confirmed, report?, elapsedMs}` (waits ≤ 500 ms) | B (H: tool error naming bridge) | **always**; served through `StopHandle` on its own connection, never queued behind another tool call (§3.18) |
| `ui_inspect` | — | UI state incl. read-only `humanSurfaces` | B | read |
| `ui_elements` | `query?, role?, limit?` | AccessKit widgets `{id, role, label, rect, enabled, humanOnly}` | B | read |
| `ui_screenshot` | `max_side?` | PNG of the window | B | read |
| `ui_click` | `id` \| `x,y`, `button?, count?` | — (human-only widgets: `human-only:` error) | B | agent |
| `ui_drag` / `ui_key` / `ui_type` / `ui_pointer` | as AUT B5 | — | B | agent |
| `ui_menu_invoke` / `ui_set` | **`id`** / **`fields`** (closed list) | — | B | agent |
| `control_call` | **`method`**, `params?` | raw result | B | agent |
| `tool_search` / `tool_call` | as PdfCraft (`--compact`) | | H B | |

`--compact` lists `command_list, command_run, command_batch, doc_inspect, render_preview, doc_open, doc_import,
doc_save, job_plan, job_preview, job_simulate, job_submit, job_status, machine_status, stop` plus
`tool_search`/`tool_call`. **`stop` is always listed.**

Error strings start with a stable prefix (AUT B13): `refused:` (with `{refusals:[…]}` data), `human-only:`,
`human_confirmation_open:`, `machine_busy:`, `needs_bridge:`, `stale:`, `not_connected:`. JSON-RPC errors,
panic containment, oversized lines and the bridge failure wording are PhotoCraft's (AUT A11).

### 4.2 Server identity, resources, prompts

- Server info `{name:"openlaser", version}`. Instructions string: AUT B4 verbatim, ending in `Backend: headless`
  or `Backend: bridge`.
- Resources: `openlaser://document`, `openlaser://commands`, `openlaser://machines`, `openlaser://job`; templates
  `openlaser://layer/{key}`, `openlaser://object/{id}`, `openlaser://command/{id}`, `openlaser://submission/{id}`.
- Prompts: `prepare-job` (import → assign layers → job_plan → fix refusals → job_simulate → job_submit),
  `mopa-color-test`, `fix-refusals`.
- Progress and cancellation for `job_plan`, `job_simulate` and long `command_run`s (PhotoCraft rules).
- Registration (`~/.claude.json`), same shape as photocraft / photocraft-live:

```json
"openlaser":      {"type": "stdio", "command": "/Users/luizbueno/.local/bin/openlaser-cli",
                   "args": ["mcp", "--automation-read-root", "/Users/luizbueno/Craft", "--automation-write-root", "/Users/luizbueno/Craft"]},
"openlaser-live": {"type": "stdio", "command": "/Users/luizbueno/.local/bin/openlaser-cli", "args": ["mcp", "--bridge", "auto"]}
```

### 4.3 Key result shapes

`command_list` row (bare array, PhotoCraft's keys plus `agent`, `kind`, `defaultShortcut`, optional `schema`):

```json
{"id":"layer.cutSettings.set","label":"Cut Settings","menu":[],"shortcut":null,"defaultShortcut":null,
 "kind":"edit","agent":"allowed","params":"{layer, subLayer?, name?, output?, show?, mode?, passes?, galvo?:{speedMmS, powerPct, frequencyKhz, qPulseNs}, gantry?:{…}, fill?:{intervalMm, angleDeg, angleIncrementDeg, bidirectional, crossHatch}}",
 "enabled":true,"disabledReason":null,"needsDocument":false}
```

Human-only rows are listed with `"agent":"humanOnly","enabled":false,"disabledReason":"human-only: started by a
person in the OpenLaser window"`, so agents learn what they may not do without trying.

`doc_inspect` (objects capped at 200 unless `objects:"all"`, flagged):

```json
{"index":0,"name":"card.olp","path":"jobs/card.olp","dirty":true,"revision":14,"units":"mm",
 "axes":"y-up; galvo origin at field centre",
 "machine":{"id":"jpt-m7-200-sim","kind":"galvo_lmc","workArea":{"min":[-100,-100],"max":[100,100]}},
 "layers":[{"key":"C00","name":"C00","mode":"line","output":true,"show":true,"passes":1,
            "galvo":{"speedMmS":1000,"actualSpeedMmS":996.8,"powerPct":30,"frequencyKhz":30,"qPulseNs":200},
            "fill":{"intervalMm":0.1,"angleDeg":0,"angleIncrementDeg":90,"bidirectional":true,"crossHatch":false}}],
 "objects":{"count":214,"shown":200,"truncated":14,
            "items":[{"id":12,"name":"rect","layer":"C00","bounds":{"min":[-20,-10],"max":[20,10]},"paths":1,"closed":true}]},
 "selection":[12],"history":["Import card.svg","Move","Set C00 Frequency"],"canUndo":true,"canRedo":false,
 "live":{"fingerprint":"7f2a91…","refusals":0,"notices":1}}
```

`job_plan` (runs plan → validate → encode → disassemble → summary, the same pipeline the ARM dialog runs):

```json
{"fingerprint":"7f2a91…64 hex","short":"7f2a91","machine":"jpt-m7-200-sim","ok":true,"refusals":[],
 "notices":[{"n":1,"code":"open_shape_unhatched","object":7,"layer":"C02","message":"…"}],
 "summary":{"bytesHash":"c41e07…64 hex","rows":[{"layer":"C00","layerName":"C00","pass":1,"of":1,"mode":"line","powerPct":29.99,"frequencyKhz":29.99,
            "pulseNs":200,"speedMmS":996.8,"galvo":{"markCurrent":1228,"qPeriod":667,"pulseNs":200,"speedUnits":276,"jumpSpeedUnits":830},
            "paths":3,"markMm":120.0,"bounds":{"min":[-20,-10],"max":[20,10]},"estimateS":3.1}],
            "repeats":1,"estimateS":41.0,"crashBoundS":2.0,"packets":4},
 "frame":{"style":"contour","source":"job","bounds":{"min":[-20,-10],"max":[20,10]},"paths":3}}
```

With refusals: `"ok":false, "summary":null, "refusals":[{"code":"outside_work_area","object":7,"at":[100.5,0],"message":"…"}]`.

`machine_status` (bridge; read from the driver's cached events; **never causes USB I/O**):

```json
{"machine":"jpt-m7-200","connected":true,"provider":"usb","state":"idle","statusBits":"0x0020","ready":true,"busy":false,
 "fault":null,"progress":null,"lastStop":{"elapsedMs":141,"confirmed":true},
 "frameReceipt":{"fingerprint":"7f2a91…","fullPasses":3,"ageS":42,"origin":"human"},
 "pending":{"submission":"3","frameRequest":null},"automation":{"connectedAgents":1}}
```

### 4.4 State machines (engine `requests.rs`; AUT C3.4, C3.5)

```
frame_request ─► awaiting_human ─(human Approve)─► framing ─► done (Once) | until stop (Loop)
                   ├─(human Decline)─► declined     ├─(edit/undo)─► stale      └─(stop from anyone)─► stopped
                   └─(2 min)─► expired
job_submit ─(refusals)─► tool error, nothing created
   ▼
awaiting_human ─(human Frame)─► framing ─(≥ 1 executed pass, S20)─► awaiting_arm ─(human ARM)─► running ─► finished | stopped | fault
   ├─(human Reject)─► rejected   ├─(job_withdraw)─► withdrawn   ├─(newer submit)─► superseded
   ├─(any edit/undo)─► stale     └─(15 min)─► expired           awaiting_arm ─(receipt > 10 min)─► awaiting_human
```

The submission card shows fingerprint, the decoded summary, client name and the agent's note (plain text,
labelled "Note from agent", ≤ 280 chars, **outside** the confirmation block). The ARM dialog never shows the
agent's summary: Start makes the engine cold-build plan → validate → encode → summary again (`request_arm`), and
the dialog renders that `PendingArm.summary`, whose job object is the one that will be sent (S22).

### 4.5 Control methods (GUI bridge; exact table, pinned by `control_methods_table_is_exact`)

| Group | Methods |
|---|---|
| Engine | `engine.execute {command, params?, wait?}`, `engine.commands`, `engine.schema {id}`, `jobs.list`, `jobs.cancel` |
| Document I/O | `app.open {path}`, `app.import {path, …}`, `app.save {path?}`, `app.export {path, format?}`, `doc.inspect {index?, objects?}`, `doc.render {maxSide?, view?}`, `doc.select {index}`, `doc.close {index?}`, `session.list` |
| Laser read/plan | `machine.list`, `machine.status`, `job.plan`, `job.preview`, `job.estimate`, `job.simulate`, `job.export {path}`, `gcode.export {path}` |
| Laser requests | `frame.request {mode, machine?}`, `frame.requests`, `job.submit {note?}`, `job.status {submission?}`, `job.withdraw {submission}` |
| Stop | `machine.stop {machine?}` — answered on the transport thread (fast path) |
| UI | `ui.inspect`, `ui.elements`, `ui.screenshot {maxSide?}`, `ui.focus`, `ui.resize`, `ui.set`, `ui.menu.list`, `ui.menu.invoke`, `ui.dialog.open/set/confirm/cancel` (ordinary dialogs only), `ui.pointer`, `ui.click`, `ui.move`, `ui.drag`, `ui.key`, `ui.type`, `ui.dock.move/float/close/open/resize` |
| App | `app.quit` (refused while busy), `methods` |

**Deliberately absent** (no handler → `unknown method`): connect, disconnect, frame start without approval, arm,
run, mark, start, test dot, red light, jog, home, G-code send, machine-profile/calibration writes, automation
access changes, and `prefs.set` on `machine.*`, `safety.*`, `automation.*`, `humanOnly.*` keys (refused through
`engine.execute` too). While a machine is framing or running, or a human-only surface is open, mutating methods
are refused (`machine_busy` / `human_confirmation_open`); reads, `ui.screenshot` and `machine.stop` are served.

### 4.6 The emission safety boundary

| Capability | Agent (MCP / control) | Human in the GUI | Human at a TTY (`openlaser-hw`) |
|---|---|---|---|
| Design, import, edit, layers, cut settings, undo | yes | yes | — |
| Plan, refusals, preview, estimate, simulate on FakeLmc, export SVG / G-code / job JSON | yes | yes | — |
| Read machine list and cached status | yes | yes | yes |
| **Stop** | **yes, always** | yes (Esc, STOP button, STOP overlay, E-stop) | yes (Esc, Space, Ctrl-C, SIGHUP) |
| Submit a job for approval; withdraw it | yes | — | — |
| Request red-light framing | yes (a request) | approves it with a click | — |
| Connect / disconnect the board; start framing | no | yes | yes |
| **ARM / start emission** | **no, never** | yes: checklist + typed `ARM` behind the provenance gate | yes: `ARM <nonce>` on a TTY |
| Edit machine profiles, calibration, work area, source limits | no | yes | — |
| Change automation access or roots | no | yes | — |

Mechanisms, each independent, so no single bug opens a path:

1. **Structural** (§2.3): `automation` and `openlaser-cli` cannot link `arm`, `human`, `transport` or `nusb`;
   `ui-egui` (the control dispatcher) cannot link `arm`; `arm`'s reverse dependencies are exactly {`human`,
   `openlaser-hw`}; `ArmToken::issue` and the witness constructors are clippy-disallowed outside one file each;
   `DriverHandle::run` requires an `ArmToken` by value.
2. **Policy** (§3.16): `Access::Human` commands are refused for every non-Human origin before decoding, in the
   engine, whatever the transport; the GUI's control dispatcher refuses them too.
3. **Provenance gate** (§3.19, §3.20): synthetic input enters egui only through `HumanSurfaces::inject`, which
   taints the frame; human-only widgets act only on `clicked_by(Primary)` in untainted frames, with no AccessKit
   action, focused window, widget visible ≥ 300 ms. Human-only surfaces have no `ui.dialog.*` kind and are
   reported read-only in `ui.inspect`.
4. **Confirmation from bytes, bound to the bytes** (S22): the ARM dialog renders only `EmissionSummary` decoded
   from the packets; the token carries their `bytes_hash`; `run_armed` sends the very job object the summary came
   from; the driver recomputes the hash from what it was handed and refuses a mismatch (`BytesMismatch`).
5. **Frame ⊇ burn**: `mint` needs a human-originated `FrameReceipt` of the same fingerprint, ≥ 1 pass the board
   has **executed** (S20), ≤ 10 min old, covering the job; the driver re-checks against its **own** receipt record
   and spends the nonce.
6. **Stop fast path**: the control server holds the engine's `StopRegistry` and answers `machine.stop` on its
   transport thread, so `stop` works while the window is occluded or the UI thread is stalled; the MCP server sends
   it on a dedicated connection outside the tool-call mutex (§3.18); a stop after a queued run cancels it (§5.3).
7. **Configuration that bounds safety is human-only** (S9, S24): profiles, calibration, work area, source limits,
   written only through a `ProfileStore` that `openlaser-cli` never constructs.
8. **Hardware is default-deny** (S23): `hw` feature + `OPENLASER_HW_PRESENT=1` + bench session + board lock; the
   first emission of a bench session is capped at 10 %.
9. **Procedural layers** (PLAN §3.6, AUT C3.9): `AGENTS.md` rule 1; USB unplugged while agents work (D19); the
   bench session; Luiz's `hw` builds live in `~/AI/OpenLaser/.target/hw`, never in an agent lane, so no agent ever
   finds a hardware-enabled binary; the PreToolUse hook (Luiz decides) on Bash **and**
   `mcp__terminal__run_in_terminal` commands, blocking:
   - `openlaser-hw` in **command position**, with optional `env`/`sudo`/`exec`/`nohup`/`time`/`command` and
     `VAR=…` prefixes and a path:
     `` (^|[;&|(`]|\$\()[[:space:]]*(((env|exec|nohup|time|sudo|command)|[A-Za-z_][A-Za-z0-9_]*=[^[:space:]]*)[[:space:]]+)*([^[:space:];&|]*/)?openlaser-hw([[:space:]]|$) ``;
   - `cargo … run … openlaser-hw` (`cargo[^;&|]*[[:space:]]run[[:space:]][^;&|]*openlaser-hw`);
   - any `hw` feature: `(--features|-F)[= ]+[^[:space:];&|]*hw`; and `OPENLASER_HW_PRESENT`;
   - `mcp__computer-use__request_access` / `open_application` naming OpenLaser;
   - Write/Edit of files under `…/OpenLaser/machines/` and `…/OpenLaser/bench-session.json`.

   It must match `openlaser-hw run x.olp`, `./target/debug/openlaser-hw status`, `cd a && openlaser-hw frame`,
   `env A=1 openlaser-hw detect`, `$(openlaser-hw status)`, `cargo run -p openlaser-hw -- detect`,
   `cargo build --features openlaser-hw/hw`; it must not match `cargo test -p openlaser-hw --lib`,
   `cargo clippy -p openlaser-hw --all-targets` or `rg openlaser-hw docs/`, so agents can build and test the crate
   (all checked with `grep -E` during this review; revision 1's
   `(^|[;&|[:space:]])openlaser-hw[[:space:]]` matched `-p openlaser-hw --lib` and missed `cargo run`). The
   foot-switch ARM arrives in M3.

**Not covered** (AUT C5): a process posting OS-level mouse/keyboard events to the window before the foot switch
exists; an agent typing into a real terminal running `openlaser-hw`; an agent with a shell that edits files under
the app data folder (profiles, bench session) when the hook is not installed: the human still sees the decoded
values and the work area at ARM, and the first-emission cap still applies, but the limits themselves would have
moved; hostile code in the GUI process; a human who arms without looking.

---

## 5. LMC driver specification (summary; details in R05 §1, PLAN §3.3–§3.5, GOLD)

### 5.1 Device and transport

USB `0x9588:0x9899` (USBLMCV4). Claim interface 0, bulk OUT `0x02`, bulk IN `0x88`; `set_configuration(1)` only
if H12 shows it is needed, and then only through the human-only profile flag `usb_set_configuration` (macOS cannot
claim an interface of an unconfigured device; without the flag the open is refused `UsbNotConfigured`, so HIL-0
needs no code change). Writes are exactly 12 bytes (one single command, six LE u16 words, then an 8-byte read
except `WriteCorLine`) or 3072 bytes (one list packet = 256 × 12-byte slots, unused slots `0x8002`, no read).
Each transfer is sliced (≤ 20 ms) with a total budget of `transfer_timeout_ms` (250 ms, PLAN §3.5; H13 tunes it);
past it the transfer is `Timeout`, never retried. Status = word 3 of the `GetVersion` reply: BUSY `0x04`, READY
`0x20`, AXIS `0x40`; unknown bits kept. A failed read is `Err`, never status bits. PID `0x9980` (clone board) is
detected and refused with an explanation; firmware is never loaded. **No USB reset, no retry, no
auto-reconnect**: any transfer error stops the job and raises a sticky fault. The board lock (`board.lock` in the
app data folder, §2.6; shared with OM, which locks it with `flock`) is taken at connect.

### 5.2 Encoder rules (byte-exact; goldens enforce them)

1. **Rounding**: `round_ties_even` wherever Python calls `round()`, `trunc` wherever it calls `int()`, Python's
   operation order (`types::galvo`, §3.1). 30 % → 1228, 70 % → 2866, 30 kHz → 667, 1000 mm/s → 276 and
   3000 mm/s → 830 at F = 236.68, (5, 5) mm → 34152.
2. **Coordinates**: `u16::try_from` after rounding, never `as u16`; +F/2 exactly is refused (OM clamps).
3. **Field bound**: accepted points lie inside the **work area** = the lens's rated field, ±100 mm for Luiz's
   200 mm lens, intersected with the strictly encodable square |v| < F/2 (F = 200 × 1.1834 = 236.68 mm). F is used
   only for unit conversion. Refused, never clamped (B10).
4. **Speed**: the requested speed is encoded once, in `plan`, to whole units (`trunc`); the encoder reads only
   `speed_units`; 0 units is refused (B7); the UI and summary show the actual speed. Never re-encode a displayed
   value. A proptest sweeps F ∈ [100, 300]. If the two axis scales differ, speed units use galvo 1's F (UNVERIFIED,
   H10); Luiz's profile has equal scales, so this does not arise yet.
5. **Parameter block per list**: every list carries the full block in GP order — JumpSpeed, PulseWidth,
   MarkCurrent, QSwitchPeriod, MarkSpeed, LaserOnDelay, LaserOffDelay, PolygonDelay — then the first jump.
6. **First jump** (rust-v1, B3 + PLAN §3.3): every list starts its motion with `JumpDelay(long = 200)`,
   `JumpTo(x0, y0, 0, dist, 0)`, `DelayTime(100)` (1 ms settle). `dist` uses GP's formula, from
   **(0x8000, 0x8000) for the first jump of every list**: each OM layer-pass ran on a fresh GP controller, so this
   reproduces the v1 packets (`mark_box10` 1957, `dot_test` 0), and it keeps packet bytes independent of session
   state. (Revision 1 said "from the last commanded point in this session", which the encoder cannot know at
   encode time: the driver would have had to patch bytes after the human confirmed them.) The true mirror position
   after an earlier job or a stop is unknown anyway (H3); the long delay and settle cover it. Within a list, `dist`
   follows the previous point as GP does. Before the next `JumpTo` the short delay is set again (`JumpDelay(8)`).
7. **Zero delays** keep the `0x8000` sign word (`delay_word(0) = (0, 0x8000)`) until H9.
8. **One list per layer-pass** (no single-list merge until H21 and H2); an explicit `EndOfList` ends each list;
   packets are padded with `0x8002`; one list transfer in flight; no early packet close until H28.
9. **Type-level split**: `LightingList` can only express ReadyMark, WritePort(light bit), JumpSpeed, JumpDelay,
   JumpTo, DelayTime, EndOfList. It cannot express `MarkTo`, `LaserOnPoint` or MO open. `MarkingList` is built
   only from `EmitPlan` sections.

Marking list for one layer-pass (rust-v1; compare GOLD `mark_box10`):

```
ReadyMark · DelayTime(800) · WritePort(0x0001) · JumpSpeed(830) · PulseWidth(200) · MarkCurrent(2048)
· QSwitchPeriod(667) · MarkSpeed(276) · LaserOnDelay(100,0) · LaserOffDelay(100,0) · PolygonDelay(100,0)
· JumpDelay(200,0) · JumpTo(31384,31384,0,1957,0) · DelayTime(100) · MarkTo(34152,31384,0,2768,0) · … · EndOfList · 0x8002…
```

Lighting list (rust-v1; compare GOLD `frame_box10`, whose v1 packet was decoded for this review: `ReadyMark ·
WritePort(256) · WritePort(0) · JumpDelay(8) · JumpTo(31384,31384,0,1957) · WritePort(256) · JumpSpeed(830) ·
JumpTo…`): `ReadyMark · WritePort(0x0100) · WritePort(0) · JumpSpeed(light) · JumpDelay(200) · JumpTo(start) ·
DelayTime(100) · WritePort(0x0100) · JumpSpeed(light) · JumpTo… · EndOfList`. The **first** `JumpSpeed(light)` is a
proposed rust-v1 divergence (§5.6): GP's first dark jump runs at whatever jump speed the board last had (power-on
default, or the previous marking list's), which makes its duration, and so the time model, unknowable.
Red-dot offset/scale (identity until H26) apply to lighting lists only. Frame Once is one list that retraces
`clamp(ceil(2.0 s / (path_len / light_speed)), 1, min(20000 / points, 500))` passes (OM); Loop sends `pass`, a
complete one-pass list, again for every pass (S20).

### 5.3 Session model (`driver::LmcBackend`; red zone)

```
every command: refused (StoppedAfterQueue) if the latch was tripped after the command was queued; only then is
        the latch cleared for it. frame/run while another frame or run is active → Refused(MachineBusy).
connect (once per connection): board lock → init_sequence() verbatim (R05 §1.4, incl. Reset and WriteCorTable(0))
        → 50 ms pause
frame(job, mode, origin): per list: ResetList · GetVersion · packets (same flow control as run) · SetEndOfList
        after each · ExecuteList (same trigger as run) · wait !BUSY (same watchdog) → FrameCompleted(receipt)
        Once: one list (`once`), one receipt with full_passes = once_passes.
        Loop: the one-pass list `pass` again and again, a receipt after each !BUSY, until stopped (S20). GP's
        FrameLoop instead appended to one endless list, so a pass could only be counted when its packets were
        ACCEPTED, up to the run-ahead window before the light traced them; FrameLoop is not in the goldens.
        At the end or on stop: Fiber_SetMo(0) · WritePort(0) (v1: the light bit off; v0 left it on).
run(job): runtime checks first: token.check(job.fingerprint, bytes_hash RECOMPUTED from job, machine, session,
        now); nonce unused, then spent; the driver's OWN last receipt: same fp, ≥ 1 executed pass, ≤ 10 min,
        origin Human, covers job
  for each repeat, for each list (layer-pass), stop checked between lists:
        ResetList · WritePort(0x0001) · Fiber_SetMo(1) · GetVersion
        per packet: wait READY (poll ≤ 10 ms, stop-aware; NEVER proceed on a deadline)
                    respect the run-ahead window (below; never < 1 packet)
                    submit (one transfer in flight) · SetEndOfList
                    ExecuteList once per list, after the FIRST of: the 3rd packet (GP parity), the packet that
                    fills the run-ahead window, a status read after a submit that shows !READY (the board's
                    buffer is full), or the list's last packet
        wait !BUSY (same watchdog) · only then Fiber_SetMo(0) · WritePort(0) · GetMarkTime(3)
  no abort, Reset or USB reset after success
run-ahead window: before ExecuteList, queued = Σ est of packets sent; after it, queued = Σ est of packets sent −
        (now − t_ExecuteList), floored at the estimate of the newest packet. Submit the next packet only if
        packets_unexecuted < max_packets (3) and queued + est(next) ≤ max_emission_s (2 s), or if queued ≤ 0
        (never < 1 packet). Revision 1 executed only after the 3rd packet while capping the queue at 2 s: a list
        whose packets each estimate > 1 s (the business-card raster: ~9 packets, ~65 s) could never reach a 2nd
        packet, never ExecuteList, and the watchdog would fault every such job (also every job on a FakeLmc with
        buffer_depth < 3).
watchdog: progress = a VALID status read whose READY or BUSY bit changed, or a packet transfer completing; failed
        reads are errors (fault), never progress (OM v1 5f7aa63 found flaky reads keeping MO open). No progress for
        est(packets that may still be queued) × watchdog_factor (1.5) + watchdog_slack_s (2 s) → Fault →
        emergency_stop. Before ExecuteList est = 0, so a board that never raises READY is stopped within 2 s.
transfer error or timeout (transfer_timeout_ms) anywhere: Fault (sticky) → emergency_stop.
emergency_stop: stop submitting → let the in-flight packet complete (≤ 20 ms), else cancel_all + possible_desync
        → StopExecute → Fiber_SetMo(0) → ResetList → ONLY IF READY within 50 ms: dummy EOL packet + SetEndOfList
        + ExecuteList ONLY IF no list was executing (no ExecuteList sent for the current list, or its !BUSY already
        seen: OM v1 `_safe_abort` and GP send it only `if not _list_executing`; revision 1 sent it
        unconditionally, which after a mid-list stop could restart whatever the board still held if ResetList
        did not clear it, H14) → Fiber_SetMo(0) → WritePort(0) → drain stale replies → GetVersion !BUSY →
        StopReport
```

Each list is bracketed exactly as one GP marking session (prelude and postlude), minus GP's per-job init, abort
and USB reset; this is the shipping sequence that becomes the `rust-v1` goldens after review. Every byte inside
the packets is fixed at encode time (§5.2 rule 6), so the packets the summary was decoded from and the packets
on the wire are the same objects; only the timing of single commands (ExecuteList position, status polls)
depends on the board.

### 5.4 Stop path and latency budget

| Stage | Budget | Mechanism |
|---|---|---|
| Esc / STOP / `stop` tool → latch | ≤ 16 ms p95, ≤ 50 ms p99 (GUI, S3) | Esc consumed in `raw_input_hook`; STOP never disabled, never behind a modal, also an `Order::Tooltip` overlay; `machine.stop` on the control transport thread; CLI key thread; signals |
| Latch → driver | ≤ 20 ms | every wait is a ≤ 20 ms slice calling `cx.check()`, single-command reads included |
| In-flight packet | ≤ 20 ms | let the one in-flight transfer complete; `cancel_all` only on overrun, then flag `possible_desync` (H23) |
| Abort on the wire | ≤ 60 ms | §5.3 sequence |
| Confirm | ≤ 50 ms | drain stale replies; GetVersion !BUSY → `StopReport` |
| **Total** | **FakeLmc: p99 ≤ 100 ms, any run > 250 ms fails. Hardware: ≤ 250 ms in 20/20 traced trials and key-to-dark (H24)** | |
| Watchdog | 500 ms | no report → red banner "STOP NOT CONFIRMED: use the key switch / E-stop" + sticky fault |

Crash containment, each layer independent:
- a drop guard runs `emergency_stop` if the driver thread panics;
- dropping the last `DriverHandle` trips the latch and **joins the driver thread for ≤ 500 ms** (dead-man rule);
  without the join, a process that ends right after (a panic unwinding out of `main`) kills the driver thread
  mid-abort;
- the GUI and `openlaser-hw` install a **panic hook** that calls `StopRegistry::stop_all(500 ms)` before the
  default hook: it runs on the panicking thread whatever happens next, including macOS, where `eframe::run_native`
  may terminate the process without running destructors (UI §7.5: "code after `run_native` does not run");
- SIGINT/SIGTERM/SIGHUP (unix, signal-hook flags) and Ctrl-C/Break/close (windows, ctrlc) are handled in **both**
  the GUI and `openlaser-hw`: a watcher thread trips every latch, waits ≤ 500 ms for the StopReports, then exits;
  the driver never runs inside a signal handler;
- a job holds a `keepawake` assertion (transport, §2.6);
- not covered: SIGKILL, Force Quit, OOM, a GPU driver crash, a cable pull (PLAN §3.5); the run-ahead window and
  the E-stop bound those, and H25 measures them.

Faults are sticky until acknowledged; user stops are not.

### 5.5 `py-compat` (test-only)

`lmc` feature `py-compat`, reachable only through dev-dependencies, `compile_error!` without `debug_assertions`.
It replays GP's sequencing (one controller per job, per-job init, parameter cache, elided first jump, per-job abort
with the dummy packet, close) and must reproduce every `tests/golden/t1_lmc/v0/*.json` event list exactly: kinds,
single-command hex, read statuses and packet sha256s.

### 5.6 Intentional divergences (shipping vs the Python oracle; becomes `docs/DIVERGENCES.md`, signed at D13)

From GOLD "Intentional divergences": explicit first jump with the long delay and a settle (v1 has the jump, short
delay, no settle); one session per connection (no per-job init, abort or USB reset); conditional dummy packet
(only if READY within 50 ms); in-flight packet completed before a stop; MO closed and ports zeroed only after
!BUSY; work area ±100 mm (v0: ±118.34); +F/2 refused (v0/v1 clamp to 65535); speed encoded once, 0 units refused;
NaN/inf parameters refused (OM snaps a NaN pulse to 2 ns); frequencies that would wrap refused; hidden+Output
layers, orphan objects, empty Fill layers and gantry overspeed refused; open Fill shapes become numbered notices;
SVG imported at true scale, unitless SVG refused; no STL; DXF SPLINE evaluated; mirror flags live in the machine
profile (`cal.negate`); repeat count shown at confirmation.

Added by the revision 2 review (all need Luiz's signature at D13 like the rest):
- every list's first-jump `dist` is computed from (0x8000, 0x8000) (§5.2 rule 6; matches v1's packets, which each
  started on a fresh controller);
- lighting lists set `JumpSpeed(light)` before their first (dark) jump (§5.2; GP leaves the board's previous speed);
- Frame Loop sends one complete list per pass and counts a pass only after the board reports !BUSY (S20; GP's
  FrameLoop appended to one endless list and was never captured);
- `ExecuteList` may come before the 3rd packet when the run-ahead window fills or READY drops (§5.3; GP waits for
  the 3rd packet); the packet bytes are unchanged, only the single-command order differs;
- the abort's `ExecuteList` after the dummy packet follows OM v1 (`only if no list is executing`), not revision 1.

### 5.7 GRBL (`driver::GrblBackend`; fake-tested only in this build)

PLAN §5.3, R05 §3.3: wait for the banner, then `$I`, `$$`, `$G`; **preflight refusals** (`GrblPreflight`): no
streaming unless `$32=1` on laser profiles, `$30` equals the profile's `s_max`, `$130/$131` cover the bed;
character-counting stream with the RX size from `[OPT:]` (default 128); realtime `!`, `~`, `?`, `0x18`, `0x85`
written by the driver thread only; on `error:` or `ALARM` send `!` then `0x18`, fail the job, require
acknowledgement; frame with the laser off (G0 only); `home_on_connect = false`; the machine never moves on
connect. G-code text is `gcode::compile` (T2 parity minus divergences).

---

## 6. Testing

### 6.1 The gate

Every work package runs this before handing in; the integrator runs the workspace form after every merge
(`cargo xtask ci` runs all of it):

```sh
export PATH="$HOME/.cargo/bin:$PATH"
rustc --version                                               # must be ≥ 1.99 (the `stable` toolchain, §2.5)
export CARGO_TARGET_DIR="$HOME/AI/OpenLaser/.target/core"   # or .target/gui (§6.7)
export OPENLASER_NO_HW=1                                      # belt and braces: transport refuses everything
unset OPENLASER_HW_PRESENT                                    # S23: hardware is default-deny
cargo fmt --all -- --check
cargo clippy -p <your crates> --all-targets -- -D warnings     # integrator: the lane's package set (§6.7)
cargo test -p <your crates>                                     # integrator: --workspace in the gui lane
cargo xtask deps                                                # dependency rules §2.3 (cargo metadata + greps)
cargo xtask golden-verify                                       # sha256 of every fixture in tests/golden/manifest.json
# integrator only, gui lane, once per integration step, df ≥ 8 GB (§2.6 item 1):
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
```

**Never run** (AGENTS.md §1): `--features hw`, `--all-features`, `--release` (unless the integrator asks), the
`openlaser` or `openlaser-hw` binaries against hardware, galvoplotter's `site-packages/test/*.py`, the prototype's
live mode, `cargo update`, `cargo clean` in a shared target directory, or Python inside tests. Running the
`openlaser-cli` binary and `openlaser-hw --sim` from tests is allowed (no device can be opened by them), but
binaries are tested **in process** (`src/lib.rs` entry points with injected stdio and terminal); at most one smoke
test per binary spawns `CARGO_BIN_EXE_*`, because the uplifted `target/debug/<bin>` is shared by every worktree in
a lane and another worktree's build can replace it between build and spawn.

### 6.2 Golden tiers and where each is consumed

| Tier | Fixture (read-only) | Test (owner) | Must match |
|---|---|---|---|
| T0 | `t0_numeric/*.json`: 7 tables, 155,054 rows | `crates/types/tests/t0_numeric.rs` (WP4) | exactly; recompute `inputs_f64_sha256` after parsing; `errors` rows → `Err`; `specials` through the `_om` variants |
| T1 (v0) | `t1_lmc/v0/*.json` + `packets/*.hex` | `crates/lmc/tests/t1_py_compat.rs` (WP4) | `py-compat` event lists exactly (kinds, hex, statuses, packet sha256) |
| T1 (v1) | `t1_lmc/v1/*.json` | `crates/lmc/tests/t1_shipping_vs_v1.rs` (WP4) | shipping list commands equal v1's except the §5.2 rule-6 lines and the lighting lists' first `JumpSpeed` (divergence-aware comparator over decoded commands; every other command, including every `dist` word, equal) |
| rust-v1 | `tests/golden/rust-v1-candidate/lmc/` (WP4: packets and list listings) and `…/session/` (WP5: wire event streams), written with `UPDATE_RUST_V1=1` | `crates/lmc/tests/rust_v1.rs` (WP4), `crates/driver/tests/rust_v1_session.rs` (WP5) | candidate until Luiz signs D13, then frozen as `tests/golden/rust-v1/` |
| T2 | `t2_gcode/index.json`, `*.gcode`, `number_format.json` | `crates/gcode/tests/t2.rs` (WP2) | 100,019 number rows exact; 44 cases byte-exact after masking line 3, via `gcode::om`; shipping refuses the capped cases |
| T3 | `t3_hatch/hatch.json`: 46 cases | `crates/geom/tests/t3_hatch.rs` (WP1) | segment count and order exact; coordinates within 1e-9 |
| T4 | `t4_import/*.json`: 30 cases | `crates/import/tests/t4.rs` (WP2) | DXF counts exact, coordinates within 1e-9; SVG/STL rows informational |
| T5 | Rust-authored from PLAN §3.6 and OM v1 behaviour | `crates/safety/tests/t5_*.rs` (WP3) | same decision as OM v1, as a refusal code |
| T6 | Rust-authored, FakeLmc | `crates/driver/tests/t6_*.rs` (WP5) | §6.3 |

Floats in fixtures are parsed with `serde_json`'s `float_roundtrip`. Fixture paths come from
`openlaser_testkit::golden_dir()`. Fixtures are never edited by work packages; regeneration is a manual Python step
(GOLD "Regenerating") done by the integrator, never in tests.

### 6.3 Simulator suites (T6; FakeLmc, FakeGrbl)

All seeded and deterministic on `VirtualClock`, plus a real-time variant (`SystemClock`, 2 ms transfer latency)
for the stop budget. 200 randomised runs per gate run (`T6_RUNS` env to raise it). Randomised stops are tripped
from `FakeLmc::set_hook` at a seeded wire event or virtual tick, never from another thread (determinism). Driver
T6 tests are integration tests in `crates/driver/tests/` (the `lmc-sim` dev-dependency cycle, §2.3 rule 10).

| Test | Asserts |
|---|---|
| `t6_stop_p99_under_100ms_all_states` | stop from streaming, buffer full, packet in flight, BUSY, framing, connecting: p99 ≤ 100 ms, none > 250 ms (real-time variant) |
| `t6_no_write_while_not_ready` | `Recording.writes_while_not_ready == 0` in every run, aborts included |
| `t6_busy_20s_completes_without_mo_close` | B1: a 20 s BUSY job completes; MO closes only after BUSY clears |
| `t6_abort_no_write_while_not_ready` | B2: never-READY board: no 3072-byte write during abort; MO(0) and WritePort(0) still sent |
| `t6_ready_never_returns_stopped_within_2s` | MO closed, ports 0, fault raised |
| `t6_watchdog_only_without_progress` | a slow but progressing board is never stopped by the watchdog |
| `t6_framing_records_zero_emission` | every frame: `burned` empty, MO never opened |
| `t6_run_ahead_cap_packets_and_seconds` | queued ≤ 3 packets and ≤ 2 s estimated, never < 1 |
| `t6_slow_packets_execute_without_deadlock` | every packet estimated > 2 s (business-card raster at 300 mm/s): ExecuteList after packet 1, the job finishes, no watchdog fault |
| `t6_buffer_depth_1_and_2_complete` | FakeLmc `buffer_depth` 1 and 2: READY drops before packet 3, ExecuteList is sent, the job finishes |
| `t6_inflight_packet_completed_before_stop_execute` | wire order on a stop with a packet in flight |
| `t6_abort_mid_list_sends_no_execute_list` | stop after ExecuteList: the abort's dummy packet (if READY) is not followed by ExecuteList; after a finished list it is (OM v1 parity) |
| `t6_dead_man_handle_drop_stops`, `t6_panic_guard_stops`, `t6_handle_drop_waits_for_stop_report` | MO closed, ports 0; the dropping thread returns only after the StopReport (≤ 500 ms) |
| `t6_driver_never_blocks_on_unpolled_events` | nobody reads `events()` during a 100 000-transfer job; stop latency is unchanged |
| `t6_frame_receipt_only_after_executed_pass` | Once: no receipt until !BUSY after the list; a stop before it gives none. Loop: receipt k only after pass k's !BUSY |
| `t6_stop_after_queue_cancels_queued_run` | trip the latch after `run()` is queued but before the driver dequeues it: the run is refused `StoppedAfterQueue`, nothing is written |
| `t6_run_refused_when_bytes_differ_from_token` | a job whose packets differ by one byte from the token's `bytes`: `BytesMismatch`, nothing written |
| `t6_run_refused_while_framing` | `run()` during a Loop frame: `MachineBusy` from the driver itself |
| `t6_transfer_timeout_faults_and_stops` | a reply that never comes (`Fault::ReplyTimeouts`): Fault after `transfer_timeout_ms`, emergency stop, no retry, no reset |
| `t6_run_refused_without_own_receipt`, `t6_run_refused_tool_layers_receipt`, `arm_token_single_use`, `arm_token_expired_refused` | the driver's own re-checks |
| `t6_business_card_emission_matches_design` | FakeLmc `burned` equals the plan's sections within 1 galvo unit |
| `t6_mo_closed_ports_zero_after_every_stop`, `t6_session_init_once` | |
| `grbl_preflight_refusals`, `grbl_error_sends_hold_and_reset`, `grbl_stop_under_100ms`, `grbl_never_moves_on_connect`, `grbl_char_counting_respects_rx` | FakeGrbl (port of `fake_grbl.py`). An `#[ignore]` interop test may drive the Python `fake_grbl.py` over TCP 127.0.0.1:2323 when `FAKE_GRBL_PY=1`; it is our own research script, not third-party code |

### 6.4 UI tests (egui_kittest 0.36)

- ui-egui panel and whole-app tests use `NoHuman` (nothing human, nothing arms) and real `egui::Event`s, queried
  by AccessKit role and label (`Harness::builder().with_size(..).build_ui_state(..)` / `build_eframe(..)`).
- human tests run the ARM dialog and gate on their own; harness input counts as human; synthetic input goes
  through `Human::inject`, AccessKit actions as `Event::AccessKitActionRequest`.
- whole-app tests in `apps/openlaser/tests/` use `Human` + `SimProvider` (I2).
- Image snapshots (`snapshot` + `wgpu`) are optional this build; when added, baselines are macOS-only and live in
  `crates/ui-egui/tests/snapshots/`. Headless PNG goldens of the canvas come from `render` (tiny-skia).
- Accessibility lint: every interactive AccessKit node has a label (`kittest_accessibility_labels`).

### 6.5 Property tests (proptest; 256 cases per gate run)

`prop_points_inside_work_area_encode` (types/safety), `prop_speed_units_stable_over_field` (F ∈ [100, 300]),
`prop_packets_3072_padded`, `prop_lighting_never_marks` (no `0x8005`, `0x8003`, MO open), `prop_encode_disassemble_roundtrip`,
`prop_summary_matches_plan`, `prop_frame_covers_burn`, `prop_hatch_inside_even_odd`, `prop_flatten_within_tolerance`,
`prop_plan_deterministic`, `prop_undo_redo_roundtrip`, `olp_roundtrip_equal`, `prop_stop_at_random_tick_leaves_mo_closed`,
`prop_encode_is_session_independent` (same EmitPlan → same packet bytes and `bytes_hash`, whatever ran before),
`prop_fingerprint_ignores_frame_style` (S21), and, standing in for cargo-fuzz this build,
`prop_importers_never_panic` (random bytes into SVG, DXF, OM workspace, `.olp`) and
`prop_control_reader_never_panics` (random bytes into the bounded line reader and the discovery-file reader).

### 6.6 Named regression tests for the prototype's bugs

Each is mutation-checked at I1 (revert the fix locally → the test must fail; record the result in the I1 notes).

| Bug | Test | Owner |
|---|---|---|
| B1 3 s wait cap | `t6_busy_20s_completes_without_mo_close` | WP5 |
| B2 abort into a not-READY board | `t6_abort_no_write_while_not_ready` | WP5 |
| B3 first jump elided | `t1_dot_test_has_explicit_jump_and_settle` | WP4 |
| B4 confirm shows Frame values | `kittest_confirm_shows_decoded_layer_params` | WP9 |
| B5 ARM always true / sticky | `arm_token_single_use`, `arm_token_invalid_after_edit`, `cli_arm_refused_without_tty`, `xtask deps` rule 4 | WP5, WP6, WP9 |
| B6 hidden + Output burns unframed | `safety_hidden_output_refused`, `prop_frame_covers_burn` | WP3 |
| B7 speed < 3.61 mm/s → 0 units | `safety_speed_zero_units_refused`, `prop_speed_units_stable_over_field` | WP3 |
| B8 orphan objects dropped | `safety_orphan_object_refused` | WP3 |
| B9 3-coordinate points crash | `import_malformed_points_refused` | WP2 |
| B10 field bound = galvo reach | `safety_outside_work_area_refused` (100.5 mm at scale 1.1834) | WP3 |
| B11 MOPA defaults leak into G-code | `gcode_diode_job_uses_gantry_defaults` | WP2 |

### 6.7 Disk discipline

About 25 GB are free. One debug build of the GUI stack is several GB; nine private target directories would not
fit. Rules:

- **Two shared target lanes**: `~/AI/OpenLaser/.target/core` for WP1–WP7 (no egui in their closures) and
  `~/AI/OpenLaser/.target/gui` for WP8, WP9 and the integrator. Cargo's build-directory lock serialises builds
  within a lane; keep test invocations to your own crates (`-p`), which take seconds once dependencies are built.
  Registry and git dependencies are shared across worktrees; workspace crates are compiled once per worktree
  (their path is part of the build hash), which is small.
- **The core lane never builds the GUI stack.** Its package set is every workspace member except
  `openlaser-ui-egui`, `openlaser-human` and `openlaser` (the app); WP0 warms it with
  `cargo build --tests -p <each of those>` and checks it with `cargo clippy -p … --all-targets`, never
  `--workspace` (that would put a second egui/wgpu build, several GB, in the core lane). The gui lane is warmed with
  `cargo build --workspace --tests`. Prefer `cargo clippy` + `cargo test` and skip a separate `cargo check`:
  check-mode and build-mode artifacts are stored separately.
- **Luiz's lane**: `hw` builds go to `~/AI/OpenLaser/.target/hw` (S23), outside every agent lane, so no agent ever
  finds or runs a hardware-enabled binary, and agents never build there.
- The Windows cross-check (§2.6) adds the target's standard library (rustup) and `x86_64-pc-windows-msvc/` check
  artifacts inside the gui lane; only the integrator runs it.
- WP0 warms both lanes after committing `Cargo.lock`. Work packages never change `Cargo.lock`, so every worktree
  reuses the same dependency artifacts.
- Worktrees live in `~/AI/OpenLaser/wt/<wp>` (git shares objects; `tests/golden` is 12 MB per checkout).
- Debug only (`debug = "line-tables-only"`); no release builds; no `cargo clean` in a shared lane; no `cargo doc`.
- Check `df -h /` before a large build; if less than 8 GB is free, stop and report instead of building.

### 6.8 Review gates (PLAN §6.4)

Red zone: `types::arm`, `safety`, `arm`, `driver` (session, runtime, stop path), `human`, `apps/openlaser-hw`
(TTY ARM), the control dispatcher's refusals in `ui-egui`, the fast stop in `control`, and `engine::requests` +
`run_armed`. A red-zone change needs: its T6/kittest suite green, the mutation check, a second-model review (a
different model reviews the diff), and Luiz's behaviour review (T6 summary, USB-trace listing, the ARM dialog
text) before any hardware session.

---

## 7. Build plan for this session

### 7.0 Process

1. **WP0 runs first, alone**, by the integrator. It commits the scaffold on `main`: every crate compiles with the
   §3 contracts as stubs, `Cargo.lock` is committed, both target lanes are warm.
2. Then nine worktrees are cut from that commit: `git worktree add ~/AI/OpenLaser/wt/wpN-<name> -b wpN-<name>`.
   Each work package is one agent (red-zone packages WP3, WP5, WP9 on Opus; the others may be executors with an
   Opus review, per Luiz's routing rules).
3. **Ownership is by path** (§7.12). A package edits only its own paths. Shared files (root `Cargo.toml`,
   `Cargo.lock`, `clippy.toml`, `deny.toml`, `rustfmt.toml`, `.cargo/`, `xtask/`, `AGENTS.md`, `CLAUDE.md`,
   `docs/SPEC.md`, `docs/DIVERGENCES.md`, root `ATTRIBUTION.md`, `tests/golden/` except `rust-v1-candidate/`)
   belong to the integrator. A crate's own `Cargo.toml` may gain dev-dependencies already listed in
   `[workspace.dependencies]`; normal dependencies are fixed by WP0 from §2.2.
4. **Contract changes** go through the integrator (§3.0). Adding public items is free.
5. Each package commits on its branch (small commits, `wpN: …`, the repository's attribution line). Nothing is
   pushed. Its final hand-off message lists: what is done, the exact gate commands run and their results,
   `CONTRACT CHANGE` items, `REQUESTS` for shared files, known gaps.
6. Tests that need another package's behaviour are written now and marked `#[ignore = "needs I1"]` or
   `#[ignore = "needs I2"]`; the integrator un-ignores them at that step.
6a. **Early merges, so parallelism is real.** Stubs alone leave WP3's planner with nothing to plan
   (`world_polylines` returns nothing) and WP7/WP8 with no commands to dispatch. So the integrator merges two
   partial deliveries into `main` mid-session and the waiting packages rebase onto them:
   **I0** = WP1's `doc` model behaviour (`Project::new`, `world_polylines`, `normalize`), `geom::flatten` and
   `geom::hatch`/`hatch_om` (WP1 does these first and tags the commit `wp1-core`);
   **I1.5** = WP6's registry, `command!`, `decode`, the policy table and the core commands (`file.new`,
   `file.open`, `file.import`, `file.save`, `edit.undo`/`redo`, `layer.cutSettings.set`, `layer.assign`,
   `job.plan`) tagged `wp6-core-b`. WP6 also tags its Phase A end (`wp6-phase-a`) and WP9 keeps `apps/openlaser-hw`
   and `crates/human` in separate commits, so I1 can merge Phase A and the `openlaser-hw` part alone.
7. Provenance: ported code carries a header naming the source file, commit and licence — galvoplotter
   (`// Ported from meerk40t/galvoplotter@fc35136 galvo/controller.py, MIT`), the OM prototype (MIT), PhotoCraft
   and craft-libs (MIT OR Apache-2.0). Never GPL sources (balor, LaserGRBL, K40 Whisperer), never LightBurn code,
   icons, images or text.

### 7.1 WP0 — Scaffold and contracts (integrator; sequential; first)

- **Scope**: the whole repository.
- **Inputs**: this spec (§2, §3), PhotoCraft's `Cargo.toml` and `xtask/src/layers.rs` (pattern).
- **Deliverables**:
  1. Root: `Cargo.toml` (members, `[workspace.package]`, `[workspace.dependencies]` per §2.4 with the craft-libs git
     pins, lints, profiles §2.5), `rust-toolchain.toml`, `.cargo/config.toml`, `clippy.toml` additions
     (`disallowed-methods`), `deny.toml` (PLAN §8 licence policy), root `ATTRIBUTION.md`, `docs/DIVERGENCES.md`
     draft from §5.6.
  2. All 21 crates, 3 apps and `xtask`, each `Cargo.toml` with exactly the §2.2 dependencies **plus the
     dev-dependencies every owner's tests will need** (testkit, proptest, insta, tempfile, serde_json, sha2,
     `safety`/`arm`/`lmc` test features, `lmc-sim` per §2.3 rule 10), so no package edits another's manifest; and
     the §3 contracts as compiling, non-panicking, fail-closed stubs (§3.0), with module files split along the
     ownership lines (`types/src/arm.rs`, `types/src/galvo.rs`, `types/tests/`, `testkit/src/t0.rs` …). The
     §3.0 "implemented in full" items (`safety::testing`, `arm::testing` incl. `token_for_tests`, refusal builders)
     are real code.
  3. `testkit`: `repo_root`, `golden_dir`, `fixture`, `sha256_hex`, `hex_decode` implemented; loaders stubbed.
  4. `xtask`: `deps` (rules §2.3 1–17 from `cargo metadata`, with `hw` off and on, plus the source greps for rules 9,
     11 and 13–17), `golden-verify`, `parity` (reads `ui-egui`'s catalogue once WP8 fills it), `ci`.
  5. `tests/fixtures/`: `square10mm.svg` (mm units), `unitless.svg`, `circle.dxf`, `gray64.png`,
     `om_small/workspace.json` + `layer_settings.json`.
  6. `AGENTS.md`: replace "`openlaser-cli` frame, mark or run against a real device" with "`openlaser-hw`, or any
     binary built with `--features hw`"; add the lanes, the gate (§6.1, §6.7), S23 (`OPENLASER_HW_PRESENT` is
     never set by an agent) and the in-process binary-test rule.
  7. `cargo fetch` (the craft-libs revisions are already in the local git cache); `cargo clippy --all-targets -- -D
     warnings` green for each lane's package set (§6.7: the core lane never builds egui); commit; warm both lanes.
- **Acceptance**: the gate is green on stubs; `cargo xtask deps` green; `cargo tree -d -e normal` shows one egui,
  one kurbo, one usvg, one craft-ui; `cargo tree -i nusb -e normal` shows only `openlaser-transport` (and the apps
  with `hw`); `cargo tree -p openlaser-cli -e normal | grep -E 'openlaser-(arm|human|transport)'` is empty
  (`openlaser-format` is in its closure through `engine`; rule 17's grep, not the graph, keeps `DirProfileStore` out);
  `cargo test -p openlaser-format` builds in the core lane (proves the flate2 backend is selected); no
  `[profile.test]` warning in the build output.
- **Depends on**: nothing. **Blocks**: everything.

### 7.2 WP1 — Model and geometry

- **Scope**: `crates/types` (except `src/arm.rs`, `src/galvo.rs`), `crates/geom`, `crates/doc`, `crates/machine`,
  `crates/format`, `crates/testkit/src/t3.rs`, `tests/fixtures/model/`.
- **Inputs**: §3.1–§3.5; ENG §2.6–§2.9; OM `mopa_luiz/geometry.py`, `raster.py`, `profile.py`, `cli.py`
  (read-only, MIT); GOLD T3; R05 §4; `tests/golden/inputs/` (profile, calibration); LightBurn's public Color Palette
  page (values only, provenance recorded).
- **Deliverables**: units with validating serde; `LayerKey` parse/display; `Fingerprint` hex; clocks; refusal
  builders. geom: flatten, bounds, `join_lines` (OM port), `close_paths`, `duplicates`, `hatch_om` (T3 exact),
  `hatch` (bidirectional, one-way, cross-hatch), `inside_even_odd`, `HitIndex`. doc: `Project::new` (30 layers +
  T1/T2 with per-kind defaults), palette, `world_polylines`, `bounds`, `normalize`, `Geom::to_bez`. machine:
  `jpt_m7_200`/`_sim` (FIELDSIZE 200, scale 1.1834, 1–4000 kHz, JPT table, max 6000 mm/s, jump and light 3000 mm/s,
  MO settle 8 ms, GP timings, first jump 200 µs + 1 ms, run-ahead 3 packets / 2 s, checklist), `om_gantries`
  (from GOLD T2 machines), `validate_profile`, `effective_work_area`, `markcfg::{parse, apply}`,
  `om::parse_calibration`, the `ProfileStore` trait. format: `.olp` save/load with the separate manifest model,
  `TooNew`, `NonFinite`, `LimitExceeded` (zip-slip, > 64 MiB decompressed, > 1 M points, group depth > 32), atomic
  save; `machines::{profiles_dir, load_all, DirProfileStore}` (S24); `types::{serde_secs, serde_ms}`, `DeviceKind`.
  **First, for I0** (§7.0 6a): `Project::new`, `world_polylines`, `normalize`, `flatten`, `hatch_om`, `hatch`,
  tagged `wp1-core`.
- **Acceptance**: `cargo test -p openlaser-types -p openlaser-geom -p openlaser-doc -p openlaser-machine -p
  openlaser-format`, including `t3_hatch` (46 cases; business card 969 segments, 32 closed, 67 open; cross-hatch
  1637), `prop_flatten_within_tolerance`, `prop_hatch_inside_even_odd`, `prop_hatch_bidirectional_equals_om`,
  `jpt_m7_matches_om_profile`, `markcfg_parses_om_profile`, `calibration_out_of_range_refused`,
  `effective_work_area_is_rated_field`, `olp_roundtrip_equal` (proptest), `olp_too_new`, `olp_nonfinite_refused`,
  `olp_zip_slip_refused`, `validate_profile_rejects_empty_checklist`, `work_area_cannot_grow_past_h27_cleared`,
  `saved_profile_shadows_builtin`, `invalid_profile_file_skipped_with_message`, `fingerprint_json_is_64_hex`,
  `durations_serialise_as_seconds`.
- **Depends on**: WP0. **Merge**: `wp1-core` at I0, the rest first at I1.

### 7.3 WP2 — Import, G-code, render

- **Scope**: `crates/import`, `crates/gcode`, `crates/render`, `crates/testkit/src/{t2.rs,t4.rs}`,
  `tests/fixtures/import/`.
- **Inputs**: §3.6, §3.10, §3.11; OM `importers.py`, `gcode.py`, `machines.py` (MIT); GOLD T2, T4; UI §5.3; usvg 0.45
  docs.
- **Deliverables**: `sniff`; SVG through usvg (paths, transforms, viewBox, physical units; unitless refused unless
  `Px96`/`Px72`; stroke/fill colour → exact palette match, else the current layer plus a notice; `<text>` ignored
  with a notice); DXF: `dxf_om` (T4 exact) and the shipping importer (the same parser, SPLINE evaluated); raster
  images → `Geom::Bitmap` (8-bit grey; size from the file's DPI, else 254 DPI with a notice); OM workspace +
  layer settings → `Imported` (Appendix B; B9 refusal). gcode: `number`, `om::compile_gcode_om` (T2, capping
  included), shipping `compile` from `EmitPlan` (gantry block only), `frame_program` → sealed `GcodeFrame`,
  `GcodeProgram::bytes_hash`. render: the three list builders, `fit`, deterministic `render_png` with the size caps.
- **Acceptance**: `t4` (30 cases), `import_malformed_points_refused` (B9), `svg_true_scale_mm`,
  `svg_unitless_refused`, `svg_colour_to_palette_layer`, `om_workspace_business_card_imports` (403 objects, 3057
  points, layers per Appendix B), `t2` (number rows + 44 cases), `gcode_diode_job_uses_gantry_defaults` (B11),
  `gcode_compile_never_caps` (defence in depth: a section above max speed is an error),
  `gcode_frame_program_has_no_laser_on` (no M3/M4, no S>0), `render_png_deterministic`, `render_png_size_capped`,
  `render_business_card_snapshot` (insta binary snapshot, reviewed once), `prop_importers_never_panic`.
- **Depends on**: WP0; WP1 behaviour for doc helpers (rebase on I0; merge after WP1).

### 7.4 WP3 — Plan, safety, ARM minting (red zone)

- **Scope**: `crates/plan`, `crates/safety`, `crates/arm`, `crates/types/src/arm.rs`.
- **Inputs**: §3.7–§3.9, §4.6, §5.2; PLAN §3.3, §3.6; OM v1 `safety.py`, `layers.py`, `live.py` `geometry_errors`
  (read with `git -C ~/AI/OpenLaser/OpenMopa show 5f7aa63:<path>`); GOLD divergences.
- **Deliverables**: plan (section order, sub-layers, passes, global passes, repeats; machine transform; Line
  flattening; Fill through `geom::hatch`; Image runs as a stretch; `GalvoCutQ` through `types::galvo`; gantry
  quantisation; frame Bounds/Contour (Fill layers frame their closed outlines, never the hatch) and tool frame;
  `PlanFacts`; galvo and gantry estimates; canonical fingerprint per S21, frame style and source excluded).
  safety: `validate` with every §3.8 rule, collecting all refusals; `validate_frame`; `rules()` (`testing` is
  WP0's). arm: `mint` (binds `bytes_hash`, GUI checklist ≥ 1, TTY rules of §3.9), `tty_nonce`;
  `ArmToken::check` (incl. `BytesMismatch`).
- **Acceptance**: one `safety_<code>` test per rule including `safety_outside_work_area_refused` (B10),
  `safety_plus_half_field_refused`, `safety_speed_zero_units_refused` (B7), `safety_hidden_output_refused` (B6),
  `safety_orphan_object_refused` (B8), `safety_empty_fill_layer_refused`, `safety_nan_pulse_refused`,
  `safety_pulse_snap_notice` (199.8 → 200 with a notice; 170 refused), `safety_limits_refused_not_truncated`,
  `safety_gantry_overspeed_refused`, `safety_collects_all_refusals`, `rules_table_complete`;
  `prop_frame_covers_burn`, `prop_speed_units_stable_over_field`, `prop_plan_deterministic`,
  `notice_open_fill_shape_numbered`, `plan_business_card_matches_om` (raster pass = T3 business-card segments;
  vector layer 304 paths / 1750 points); `prop_fingerprint_ignores_frame_style`, `fingerprint_changes_with_power`;
  `mint_refuses_<code>` for each mint refusal, `mint_refuses_empty_gui_checklist`, `mint_token_carries_bytes_hash`,
  `mint_ok_issues_60s_token`, `arm_token_check_<case>` incl. `bytes_mismatch`. Second-model review before merge.
- **Depends on**: WP0; WP1 (hatch, doc) for Fill tests: rebase on I0 (`wp1-core`) as soon as it lands.

### 7.5 WP4 — LMC protocol and simulator

- **Scope**: `crates/types/src/galvo.rs`, `crates/types/tests/t0_numeric.rs`, `crates/lmc`, `crates/lmc-sim`,
  `crates/testkit/src/{t0.rs,t1.rs}`, `tests/golden/rust-v1-candidate/lmc/` (new directory).
- **Inputs**: §3.1 (galvo), §3.12, §3.14, §5.1–§5.6; R05 §1 and Appendix B; GOLD; galvoplotter 0.2.0 in
  `~/AI/OpenLaser/OpenMopa/.venv/lib/python3.10/site-packages/galvo/` (read; port with the MIT header; never run
  its tests); `tests/golden/generator/{decode_t1.py,fake_board.py}` (event model); `rayforge-test/fake_grbl.py`.
- **Deliverables**: the T0 encoders; the opcode table; `Single`, `Packet`, status; `init_sequence`; `encode_job` and
  `encode_frame` per §5.2 (session-independent bytes, first-jump `dist` from the centre, the lighting list's
  leading `JumpSpeed`, `once` + `once_passes` and a complete one-pass `pass` list); `LmcJob::bytes_hash`;
  `decode_packet`, `disassemble`, `emission_summary` from bytes (incl. `bytes_hash`), `emitted_segments`;
  `LmcTransport` + the `Box` impl; `py_compat`; FakeLmc (`Arc<dyn Clock>`, buffer depth, galvo-time-model timing,
  faults, `set_hook`, the independent emission recorder); `simulate_job`; `SimProvider` (incl. `detect`);
  `grbl::FakeGrbl`.
- **Acceptance**: `t0_numeric` (all 7 tables); `t1_py_compat` (every v0 scenario and the init/abort slices);
  `t1_shipping_vs_v1` (mark_box10, dot_test, line_test, long_job, both business-card passes, frame_box10, bc_frame);
  `t1_dot_test_has_explicit_jump_and_settle` (B3); `prop_packets_3072_padded`, `prop_lighting_never_marks`,
  `prop_encode_disassemble_roundtrip`, `prop_summary_matches_plan`, `prop_encode_is_session_independent`;
  `bytes_hash_changes_with_any_byte`; `fake_records_mark_box10_emission` (four 10 mm segments),
  `fake_frame_records_zero_emission`, `fake_counts_writes_while_not_ready`, `fake_grbl_streams_square`; the
  candidate rust-v1 lists written by `UPDATE_RUST_V1=1 cargo test -p openlaser-lmc rust_v1`.
- **Depends on**: WP0 (it uses `safety::testing` for `EmitPlan`s; real plans at I1).

### 7.6 WP5 — Driver and transport (red zone)

- **Scope**: `crates/driver`, `crates/transport`.
- **Inputs**: §3.13, §3.15, §5; PLAN §3.4, §3.5, §5.3; R05 §1.9–§1.11, §3.3; nusb 0.2.8 API docs; GP
  `usb_connection.py` (to know what not to port: resets and retries).
- **Scope** addition: `tests/golden/rust-v1-candidate/session/`.
- **Deliverables**: the runtime (one OS thread per machine, crossbeam control channel, `StopLatch`/`Stopper`
  with `stop_and_wait` and trip generations, `Cx` slices, non-blocking unbounded events, dead-man with the ≤ 500 ms
  join, panic guard), `LmcBackend` (§5.3: connect, frame with executed-pass receipts and per-pass Loop lists, run
  with the token/bytes-hash/nonce/receipt re-check, the run-ahead window and ExecuteList trigger, watchdog,
  transfer timeout, `emergency_stop` with OM v1's ExecuteList rule, `status`, MachineBusy), `GrblBackend` (§5.7,
  `Frame = GcodeFrame`), `DeviceProvider` (incl. `detect`)/`ByteLink`; transport: `UsbProvider`/`UsbLmc` (nusb;
  compiled, never run by agents; `usb_set_configuration` rule; `transfer_timeout_ms`), `detect`, `BoardLock`,
  `bench` (incl. the S23 emission cap), `SerialLink`/`TcpLink`, the keepawake assertion, and the
  `OPENLASER_NO_HW` / `OPENLASER_HW_PRESENT` checks before anything else.
- **Acceptance**: the §6.3 table, `rust_v1_session` (connect + mark_box10 + frame_box10 on a READY board equals
  the candidate stream), transport's pure tests (`no_hw_env_refuses`, `hw_present_unset_refuses`,
  `bench_session_expiry` and `bench_first_emission_cap` in a temp dir, `board_lock_second_holder_refused`) — **no
  test enumerates or opens USB**. Second-model review; Luiz reviews the T6 summary (worst stop, p99) before merge.
- **Depends on**: WP0; WP4's FakeLmc for the full suite (until then a scripted transport inside driver tests).

### 7.7 WP6 — Engine

- **Scope**: `crates/engine`, `crates/testkit/src/adversarial.rs`.
- **Inputs**: §3.16, §4.3, §4.4, Appendix A; ENG Part 2; AUT B6–B9, C3.4–C3.5; PhotoCraft `crates/engine`
  (`commands.rs`, `lib.rs` `Session::edit`, `jobs.rs` dispatch) for patterns.
- **Phase A (needed by I1, tagged `wp6-phase-a`)**: `Session` skeleton, machines and `profiles: ProfileStore`,
  `DocState`/`edit`/`undo`/`redo`, `build_job`, `live_check`, the device hub over `DeviceProvider`,
  `connect`/`start_frame` (style/source without an edit)/`request_arm` (PendingArm holds the encoded job)/
  `run_armed` (sends that job, bench cap)/`stop_all` (voids PendingArm)/`StopRegistry`/`poll`/`CachedStatus`,
  `requests`.
- **Phase B (needed by I2)**: registry, `command!`, `decode` (snake normalisation, strict), the policy table, every
  Appendix A command, `batch` (atomic), `command_list`/`command_schema`, `inspect`, prefs. The core subset listed
  in §7.0 6a lands first, tagged `wp6-core-b`, for I1.5.
- **Acceptance**: `e2e_mopa_business_card_sim` (`needs I1`: import the OM workspace → build_job ok with 67
  open-shape notices → connect sim → frame Once → receipt → request_arm → mint with `arm::testing` → run_armed →
  FakeLmc burned segments match the plan; a stop mid-run gives a confirmed StopReport); `pending_arm_voided_by_edit`,
  `arm_token_invalid_after_edit`, `run_armed_requires_receipt`, `tool_layer_receipt_does_not_arm`;
  `policy_human_only_refused_for_all_non_human_origins`, `headless_has_no_device`, `stop_never_refused`,
  `human_only_commands_have_no_menu_or_shortcut`; `registry_ids_unique`, `registry_id_grammar`
  (`^[a-z][a-zA-Z0-9]*(\.[a-z][a-zA-Z0-9]*)+$`), `snake_aliases_resolve`, `every_schema_generates`,
  `device_commands_never_journaled`; `prop_undo_redo_roundtrip`, `atomic_batch_rolls_back`,
  `coalesce_merges_drags`, an undo round-trip test for every Edit command; `commands_snapshot` (insta);
  `panic_hunt` (`#[ignore]`, run at I2); `prefs_set_refuses_machine_and_automation_keys`,
  `submit_with_refusals_creates_nothing`, `job_submit_never_reaches_driver_run`, `edit_after_submit_marks_stale`,
  `run_armed_sends_the_pending_job_object` (the driver receives the same `bytes_hash` the summary showed),
  `stop_voids_pending_arm`, `frame_with_bounds_style_satisfies_start_for_contour_job` (S21),
  `device_select_bumps_revision`, `profiles_read_only_without_store` (`device.settings.set` with `profiles: None`
  fails even for `Origin::Human`).
- **Depends on**: WP0; WP1–WP5 behaviour for the E2E (runs at I1).

### 7.8 WP7 — Control, automation, `openlaser-cli`

- **Scope**: `crates/control`, `crates/automation`, `apps/openlaser-cli`.
- **Inputs**: §3.17, §3.18, §3.21, §4; AUT (all); PhotoCraft `crates/automation/src/{server.rs, server/*.rs,
  bridge.rs, security.rs, budgets.rs, workspace.rs, headless.rs}`, `crates/automation/tests/mcp_wire.rs`,
  `apps/photocraft-cli/src/*`, `apps/photocraft/src/control_server.rs` (read; port with a notice).
- **Deliverables**: control in full (token, discovery, wire, methods table, `AuthorizedWorkspace`,
  `ControlServer` with auth, limits, `Dispatch`, `FastStop`); automation (tool table §4.1, strict schemas,
  `Headless` over the engine, `Bridge` + `BridgeClient`, `run_serve`, `run_mcp_stdio` with PhotoCraft's pre-reader);
  every `openlaser-cli` subcommand.
- **Acceptance**: PhotoCraft wire tests ported (`malformed_line_recovers`, `unknown_argument_is_32602`,
  `oversized_line_ends_session`, `batch_reply_budget_escaped`, `modern_list_fields`, `panic_contained`,
  `bridge_failure_wording`); `schemas_have_no_override_fields`, `tools_table_matches_spec`, `compact_lists_stop`,
  `control_methods_table_is_exact`, `token_file_mode_0600`, `discovery_stale_pid_refused`,
  `workspace_path_grammar`, `roots_fail_closed`; `headless_device_tools_need_bridge`,
  `job_plan_refusal_is_ok_false`, `job_simulate_business_card` (`needs I1`), `render_preview_is_document`;
  `fast_stop_answers_without_ui_thread` (nobody drains the request channel; `machine.stop` still returns within
  500 ms); `mcp_stop_not_blocked_by_inflight_call` (a bridge call parked for 50 s on a fake app; `stop` still
  answers within 500 ms through `StopHandle`), `stop_accepted_on_17th_connection`,
  `prop_control_reader_never_panics`; `cli_unknown_flag_exit_2`, `cli_hw_subcommands_redirect`,
  `cli_plan_exit_1_on_refusal`, `cli_simulate_writes_png`, `cli_run_is_headless_origin` (`run --cmd laser.start`
  and `--cmd device.settings.set` are refused human-only) — all in process through `src/lib.rs` — and one spawned
  smoke test, `cli_mcp_initialize_lists_tools` (`openlaser-cli mcp` with tempdir roots; §6.1).
- **Depends on**: WP0; WP6 for real commands (tests marked `needs I2` where they need them; rebase on I1.5).

### 7.9 WP8 — GUI shell in LightBurn's layout

- **Scope**: `crates/ui-egui` (with `assets/icons/` and its own `ATTRIBUTION.md`), `apps/openlaser`.
- **Inputs**: §1.1, §3.19, §3.21, §4.5; CAT §3–§7 (menus, toolbars, panels, fields); UI (all); PhotoCraft
  `crates/ui-egui/src/{lib.rs, control.rs, menus.rs, menu_catalog.rs, parity.rs, panel_docking.rs, native_menu.rs,
  shortcuts.rs, icons.rs}` and `apps/photocraft/src/{main.rs, mac_menu.rs}` (read; port with a notice); Lucide icons
  already vetted in PhotoCraft/PdfCraft `assets/icons/` (ISC).
- **Deliverables**:
  - the UI §4.1 layout with every region; `menu_catalog` with every CAT §3 row in LightBurn order (camelCase ids);
    parity report; muda native menu on macOS with `--in-window-menus` fallback; shortcuts per CAT §10;
  - canvas: `PainterRenderer` (own hairline mesher, mesh cache, pan translation), zoom/pan, rulers, grid, field and
    work area, selection (click, Shift, marquee enclosing left→right / crossing right→left), move/scale/rotate
    handles (coalesced), refusal and notice markers, the animated frame overlay while framing;
  - panels: Cuts / Layers table + quick editor (actual speed), Cut Settings Editor (P0 galvo/MOPA fields, CAT §6),
    palette bar 00–29 + T1/T2, numeric edits, Laser panel (device dropdown, state chip, Connect / Frame / Start as
    human-only widgets through `services.human`, Stop, Start From disabled on galvo, Cut Selected Graphics, Repeat,
    refusal/notice list, estimate), Live Framing, Preview (plan list), Console (log + USB trace), Device Settings
    (galvo; Save is human-only), submission and frame-request cards (Approve/Reject human-only), status bar with the
    agent indicator;
  - STOP chrome (toolbar STOP outside overflow, `Order::Tooltip` overlay while active, Esc latch, no file dialogs
    while active, close guard); the control dispatcher (§4.5) with human-only refusals; Settings › Automation ›
    Allow agent connections (a human-only switch) starting `ControlServer`, writing the discovery file, showing the indicator;
  - `apps/openlaser`: arguments, native options (1440×900, min 760×480), `Services` (rfd async, craft-settings,
    craft-quit last in `logic`), the Simulator machine, `DirProfileStore`, the panic hook and signal watcher
    (§5.4); feature `hw` adds `UsbProvider` and "JPT M7 · USB". `src/control/input.rs` is the only builder of
    synthetic `egui::Event`s (§2.3 rule 16).
- **Acceptance** (kittest, `NoHuman` unless stated): `kittest_lightburn_regions_present`,
  `kittest_palette_has_30_plus_tools`, `kittest_cuts_columns_and_actual_speed`, `kittest_air_hidden_for_galvo`,
  `kittest_double_click_opens_cut_settings`, `kittest_cut_settings_ok_is_one_undo_step`,
  `kittest_palette_click_assigns_selection`, `kittest_refusal_badge_for_off_table_pulse`,
  `kittest_laser_panel_galvo_disabled_reasons`, `kittest_start_without_receipt_says_frame_first`,
  `kittest_mark_disabled_while_refusals`, `kittest_no_file_dialog_while_active`,
  `kittest_stop_always_visible_and_enabled` (every state × default layout, Laser closed, Laser floated, modal open,
  1024 pt wide, 2× ppp), `kittest_esc_trips_latch_before_widgets`, `control_refuses_human_only` (via
  `engine.execute`, `ui.menu.invoke`, batch), `synthetic_click_on_frame_button_does_not_frame`,
  `kittest_quit_while_marking_stops_first`, `kittest_esc_closes_arm_dialog`, `dock_layout_roundtrip_and_fallback`,
  `menu_catalog_matches_cat`, `menu_catalog_has_no_human_only_ids` (no catalogue row or shortcut resolves to an
  `Access::Human` command), `catalog_shortcuts_match_registry`, `every_control_method_has_a_handler` (every
  `control::methods::METHODS` entry is handled, none falls through to "unknown method"), `parity_floor`,
  `kittest_accessibility_labels`, `canvas_view_xform_roundtrip`, `hit_test_and_marquee`,
  `hidden_window_still_polls_while_active` (with the window hidden and a machine active, `logic` keeps running on
  the 250 ms repaint and `poll` drains driver events). S1/S2 perf harnesses exist (`#[ignore]`; Luiz runs them in
  release later).
- **Depends on**: WP0; WP6 for command behaviour (`needs I2` where required; rebase on I1.5).

### 7.10 WP9 — Human surfaces and `openlaser-hw` (red zone)

- **Scope**: `crates/human`, `apps/openlaser-hw`.
- **Inputs**: §3.9, §3.20, §3.21, §4.6, §5.4; AUT C3; UI §7; PLAN §3.6 layer 5 and the M1.2 flow.
- **Deliverables**: `Human` (taint record held in the struct, 3 frames + 1 s, `inject`, `frame_is_synthetic`,
  `is_human_click`/`gesture`, visibility timers, the freeze while a surface is open), the ARM dialog (§3.20, closed
  by any stop, bench-cap display), approval-card helpers; `openlaser-hw` as a library + thin `main` behind a
  `Terminal` trait, every subcommand of §3.21 (incl. `profile`, `--builtin`, `--trace`, `--allow-emission`,
  `--extent`), bench and the S23 cap, TTY ARM (`tty_arm.rs`), crossterm key thread, signal-hook (unix) / ctrlc
  (windows) watcher that waits for the StopReport, the panic hook, `--sim`, `DirProfileStore`, the `hw` feature
  wiring. `apps/openlaser-hw` and `crates/human` go in separate commits (§7.0 6a).
- **Acceptance**: `synthetic_click_on_arm_confirm_mints_nothing`, `synthetic_type_arm_clears_field_and_checklist`,
  `accesskit_click_on_arm_confirm_mints_nothing`, `human_click_after_300ms_mints_token`,
  `instant_click_before_300ms_ignored`, `unfocused_window_click_ignored`,
  `kittest_confirm_shows_decoded_layer_params` (B4: the layer is changed after the plan; the dialog still shows the
  decoded values), `arm_dialog_text_does_not_persist`, `arm_dialog_closed_by_revision_bump`;
  `arm_dialog_closed_by_stop`, `press_synthetic_release_human_ignored` (taint window);
  `cli_arm_refused_without_tty`, `cli_enter_before_full_frame_pass_refused` (a pass counts only after its
  !BUSY), `cli_wrong_nonce_refused`, `cli_run_sim_marks_business_card` (`needs I1`), `cli_sigint_trips_latch`,
  `cli_signal_waits_for_stop_report`, `cli_space_and_esc_stop`, `cli_usb_requires_hw_feature`,
  `cli_bench_start_requires_tty`, `cli_run_without_allow_emission_stops_after_frame`,
  `cli_first_emission_power_cap_refused` (`SimConfig.emission_cap_pct = Some(10.0)`),
  `cli_builtin_box10_matches_t1_inputs`, `cli_trace_file_matches_t1_event_format`, `cli_profile_set_requires_tty`,
  `cli_extent_refused_for_run`; `cargo xtask deps` rules 4, 6, 11, 13. Second-model review.
- **Depends on**: WP0 (the `HumanSurfaces` trait is in the WP0 contract); WP6 for the CLI run flow (I1).

### 7.11 Integration (integrator; sequential)

**I0 — model core (early).** As soon as WP1 tags `wp1-core`: merge it; WP2, WP3 and WP4 rebase (§7.0 6a).

**I1 — MOPA core first.** Merge WP1 → WP2 → WP3 → WP4 → WP5 → WP6 (`wp6-phase-a`) → WP9 (`openlaser-hw` commits;
the `human` crate may come with it if ready). Resolve contract changes. Un-ignore `needs I1`. Gates: the workspace gate
(§6.1) plus T0, T1 v0, T1 v1, T3, T4, T5, the full T6 table, `e2e_mopa_business_card_sim`,
`cli_run_sim_marks_business_card`, `job_simulate_business_card`, and the B1–B10 mutation checks. Then write
`docs/DIVERGENCES.md` for Luiz with the rust-v1 candidate listings (D13). **At this point the MOPA path
doc → plan → lmc → driver → safety → CLI frame + mark is complete against the simulator.**

**I1.5 — command core (early).** As soon as WP6 tags `wp6-core-b`: merge it; WP7 and WP8 rebase.

**I2 — Agents and GUI.** Merge WP6 (Phase B) → WP7 → WP8 → WP9 (`human`). Un-ignore `needs I2`. Gates: the
workspace gate, `panic_hunt --ignored`, the parity floor, and the whole-app tests in `apps/openlaser/tests/`:
`kittest_business_card_flow` (Human + SimProvider: import → layers → Frame by a human click → receipt → Start →
ARM dialog → tick + type ARM → mark on FakeLmc → emission matches; Esc mid-job stops ≤ 100 ms),
`mcp_bridge_end_to_end` (the app under kittest with its control server on a free port; a `BridgeClient`:
doc_import, command_run, job_plan, job_submit → card appears → human Frame + ARM in the harness → job_status
finished), `frame_request_without_approval_sends_no_lighting_packets`,
`mcp_stop_latches_while_ui_thread_blocked`, `stop_served_while_human_surface_open`,
`app_panic_hook_stops_machine` (a panic in a test-only command stops a running sim job before the harness
unwinds), and the Windows cross-check (§6.1, integrator only).

**I3 — Close-out.** Remove the `NotImplemented` variants (the compiler finds leftovers); `cargo xtask ci`;
`HARDWARE.md` from §8; README quickstart (build, run with the Simulator, register the MCP server); final
`AGENTS.md`. Hand Luiz the commands to try the GUI on the simulator. Only Luiz ever builds with `--features hw`.

### 7.12 Ownership matrix

| Path | Owner |
|---|---|
| repository root files, `.cargo/`, `xtask/`, `docs/`, `tests/golden/` (except `rust-v1-candidate/`), `tests/fixtures/*.{svg,dxf,png}` and `om_small/` | integrator (WP0, I1–I3) |
| `crates/types/**` except `src/arm.rs`, `src/galvo.rs`, `tests/t0_*.rs`, `tests/arm_*.rs` (and except `Cargo.toml`, `src/lib.rs`: WP0's, fixed); `crates/geom`, `crates/doc`, `crates/machine`, `crates/format`; `crates/testkit/src/t3.rs`; `tests/fixtures/model/` | WP1 |
| `crates/import`, `crates/gcode`, `crates/render`; `crates/testkit/src/{t2.rs,t4.rs}`; `tests/fixtures/import/` | WP2 |
| `crates/plan`, `crates/safety` (except `src/testing.rs`, WP0's), `crates/arm` (WP0 writes `testing` inside `src/mint.rs`; WP3 must keep it); `crates/types/src/arm.rs`, `crates/types/tests/arm_*.rs` | WP3 |
| `crates/types/src/galvo.rs`, `crates/types/tests/t0_*.rs`, `crates/lmc`, `crates/lmc-sim`; `crates/testkit/src/{t0.rs,t1.rs}`; `tests/golden/rust-v1-candidate/lmc/` | WP4 |
| `crates/driver`, `crates/transport`; `tests/golden/rust-v1-candidate/session/` | WP5 |
| `crates/engine`; `crates/testkit/src/adversarial.rs` | WP6 |
| `crates/control`, `crates/automation`, `apps/openlaser-cli` | WP7 |
| `crates/ui-egui`, `apps/openlaser` | WP8 |
| `crates/human`, `apps/openlaser-hw` | WP9 |

### 7.13 Order at a glance

```
WP0 ──► WP1 model ───────────┐
    ├─► WP2 import/gcode/render ┤
    ├─► WP3 plan/safety/arm ────┤
    ├─► WP4 lmc/sim ────────────┼─► I1 MOPA core (sim frame + mark, CLI) ─┐
    ├─► WP5 driver/transport ───┤                                         │
    ├─► WP6 engine (A then B) ──┘ (A)                                     ├─► I2 agents + GUI ─► I3 close-out
    ├─► WP7 control/automation/cli ──────────────────────────── (B) ──────┤
    ├─► WP8 GUI shell ────────────────────────────────────────────────────┤
    └─► WP9 human + openlaser-hw ── (hw part at I1, human at I2) ─────────┘
```

All nine packages start together after WP0. Two early merges keep that parallelism real (§7.0 6a): **I0**
(`wp1-core`, which WP2–WP4 rebase on) and **I1.5** (`wp6-core-b`, which WP7 and WP8 rebase on). The critical path
is `wp1-core` → WP3 (plan/safety) and WP4 (lmc/sim) in parallel → WP5 (needs FakeLmc for its full T6 suite) → WP6
Phase A → I1; those agents should finish their I1-blocking deliverables before polishing.

---

## 8. Hardware bring-up checklist (when Luiz reconnects the laser)

The full item list, sessions and procedures are PLAN §5.2 (H0–H29) and R05 §7; this is the order for the Rust
driver and what must hold before the first real burn. **Luiz runs every step at the machine; agents never do.**

### 8.1 Before the USB cable goes back in (software gates)

- [ ] I1, I2 and I3 green; `cargo xtask ci` green on `main`.
- [ ] Luiz has read and signed `docs/DIVERGENCES.md` (D13); the rust-v1 candidate streams are frozen into
      `tests/golden/rust-v1/` and pinned by a test.
- [ ] T6 summary reviewed (stop p99 and worst case), the B1–B10 mutation checks recorded, second-model reviews
      of every red-zone package done.
- [ ] The prototype's HIL-P is done (PLAN §11): **H0 answered** (E-stop and key switch cut the source), oracle-v1
      tagged.
- [ ] Bench rules agreed (D19), the PreToolUse hook installed or the decision recorded (§4.6 item 9).
- [ ] Luiz builds the hardware binaries himself, into his own lane:
      `CARGO_TARGET_DIR=~/AI/OpenLaser/.target/hw cargo build -p openlaser-hw --features hw` (and the GUI with
      `-p openlaser --features hw` for HIL-3). No agent session is running; the agent terminal is closed. Hardware
      runs need `OPENLASER_HW_PRESENT=1` in that terminal only (S23).

### 8.2 Every session (PLAN §5.2 rules)

USB unplugged whenever agents are working; E-stop and key switch within reach of where you stand; eyewear on;
MacBook lid open, OpenLaser window or the terminal focused; `openlaser-hw bench start` at the TTY first; red light
before any emission, and with the source key off if the red pointer works without it (UNVERIFIED on the M7);
**first emission of every session ≤ 10 % power on scrap** (enforced in software too: `BenchPowerCap`, S23); every
session commits a USB trace (`openlaser-hw … --trace <file>`, or the GUI Console's trace export) and a result line
in `HARDWARE.md`; profile changes found by a session (`cal.negate`, `swap_xy`, `run_ahead`,
`usb_set_configuration`, red dot) are made with `openlaser-hw profile set` at the TTY; stop at 2 h.

### 8.3 Sessions for the Rust driver

| Session | Items (PLAN §5.2) | How (Luiz at a TTY) | Passes when |
|---|---|---|---|
| HIL-0 passive | H12 | `openlaser-hw detect`; `openlaser-hw status --trace hil0.jsonl` (if macOS refuses `UsbNotConfigured`: `openlaser-hw profile set usbSetConfiguration true`, then `status` again) | descriptors, GetSerialNo/GetVersion replies and idle status recorded; no init, no `EnableLaser`, no `Reset` sent |
| HIL-1a red | H2, H14, H28; H13 and H23 need the `ol-hw-raw` bench binary (not in this build) | `openlaser-hw frame --builtin box10 --loop --trace …`, 10 stops; a large Loop frame for H2 | READY/BUSY semantics and buffer depth measured → `run_ahead` (`profile set`) and FakeLmc depth updated; abort variant chosen; 10/10 stops ≤ 250 ms, zero USB resets |
| HIL-1b red | H1, H22, H3, H4 (red), H25, H27 | long Frame Once (> 8 s), `--builtin f-shape` frame, SIGKILL during a red frame, `frame --builtin corners --extent 100` then `--extent 110` | a long red-light job runs to the end; first-jump settle confirmed; axis orientation known (`profile set cal.negate.x/y`, `cal.swapXy`); crash behaviour recorded; work area stays ±100 mm unless H27 clears more (`profile set h27Cleared`) |
| **HIL-2 first real burn** | H0 re-check, H4 (mark), H10, H19, H26, H24, H1 with emission, H29 | `openlaser-hw run --builtin dot --power-pct 1 --allow-emission --trace …` first; then `--builtin line`, `box10`, `square50`; then `openlaser-hw run card.olp --allow-emission`: summary → frame loop → Enter → `ARM <challenge>` | first mark ≤ 10 % on scrap (the software cap refuses more); dot test trace starts with JumpTo + settle, diffs clean against the rust-v1 candidate, and the dot lands at the centre; 10 low-power stops ≤ 250 ms (trace) and key-to-dark ≤ 250 ms (H24); ≥ 30 s hatch completes with MO(0) only after BUSY clears; 50 mm square within ±0.1 mm of the Python one; red-dot calibration recorded (H26); OM refused while `openlaser-hw` holds the board (H29) |
| HIL-3 GUI | M1.3 exit (PLAN §7) | GUI with `--features hw`: business card + one MOPA colour test end to end; 20 GUI stops; S3 re-run incl. live resize | PLAN M1 exit criteria 3–10 |

### 8.4 Must be verified before the first real burn (HIL-2)

H0 (E-stop and key switch cut the **source**), H12, H2 (buffer depth), H14 (abort), H1 (long jobs complete), H22,
H3 (first-jump settle), H4 on red light (orientation), H25 (crash behaviour), H27 (corners), and H13/H23 through
`ol-hw-raw` **or** a written acceptance of the residual risk (the Rust driver never writes when not READY and
cancels an in-flight packet only after a 20 ms overrun). In the software, observed on red light: a point at
100.5 mm is refused in the GUI and the CLI; Start says "Frame this job first" without a receipt; the ARM dialog
shows the decoded kHz, ns, power and actual speed of every layer.

---

## 9. Open questions for Luiz

Defaults in §0 apply until he answers; none blocks this build.

1. **Name.** Keep `openlaser-*` now and run the PLAN D4 rename (Kerfwright or other) as one global change before
   the repo goes public?
2. **Hooks** (§4.6 item 9): install the PreToolUse rules for `openlaser-hw`, `--features hw`, the terminal tool,
   app-data writes and computer-use on OpenLaser? They also stop computer-use screenshots of the app (agents use
   `ui_screenshot`).
3. **Bench session for the GUI** (S17): relax after HIL-3, or keep it for every real-USB connect?
4. **Images** (S16): is Threshold/Jarvis burning as on/off runs wanted in this build, or vector-only P0?
5. **⌘W**: Weld (LightBurn) or Close Window (macOS)?
6. **LightBurn 1.5.04 check**: will you compare the "(verify)" items (menu order, default toolbar positions,
   marquee direction, F1, shortcut conflicts, palette colours, status-bar wording) against your licensed copy? The
   comparison stays out of git.
7. **`ol-hw-raw`** for H13/H23: build it next, or accept the documented residual risk for HIL-2?
8. **JPT M7 model and wattage** (label on the source): needed for the pulse-width-dependent frequency limits (H8).
9. **Per-axis speed field**: if calibration ever gives different X/Y scales, which field converts speed (§5.2
   rule 4 proposes galvo 1)?
10. **Licences of the two `.lbrn2` research samples**, before they become fixtures in the next build.
11. **The review's new divergences** (§5.6, last list): first-jump `dist` from the centre for every list, a
    `JumpSpeed` before the lighting list's first jump, per-pass Loop lists, an earlier `ExecuteList` when the
    run-ahead window fills, OM v1's abort `ExecuteList` rule. Sign them with the rest at D13, or veto any one.
12. **First-emission cap** (S23): 10 % per bench session, lifted after one finished run. Keep, lower, or make it
    per job?

---

## Appendix A. P0 commands of this build

Ids follow S2; params are camelCase with units; `ids?` defaults to the selection; `layer` takes 0–29, `"C05"`,
`"T1"`, `"T2"`. Kind and access per §3.16. Commands marked † are stretch goals. Every other CAT row is in
`menu_catalog.rs` and renders disabled ("Not in OpenLaser yet").

| Id | LightBurn item | Where | Key (macOS) | Access | Kind | Params |
|---|---|---|---|---|---|---|
| `file.new` | New | File, toolbar | ⌘N | Agent | Edit | `{name?, machine?}` |
| `file.open` | Open | File, toolbar | ⌘O | Roots | Io | `{path}` |
| `file.import` | Import | File, toolbar, drop | ⌘I | Roots | Edit | `{path, layer?, centerMm?, scale?, svgUnits?}` |
| `file.save` / `file.saveAs` | Save / Save As | File, toolbar | ⌘S / ⇧⌘S | Roots | Io | `{}` / `{path}` |
| `file.export` † | Export (SVG) | File | ⌥X | Roots | Io | `{path, format:"svg", selectedOnly?}` |
| `file.saveMachineFile` | Save GCode | File, Laser panel | ⌥⇧L | Roots | Io | `{path, selectedOnly?}` (gantry profiles) |
| `file.exit` | Quit | app menu | ⌘Q | Ui | — | `{}` (stops first if active) |
| `edit.undo` / `edit.redo` | Undo / Redo | Edit, toolbar | ⌘Z / ⇧⌘Z | Agent | Edit | `{}` |
| `edit.selectAll` | Select All | Edit | ⌘A | Agent | View | `{}` |
| `edit.select` | (canvas click, marquee) | — | — | Agent | View | `{ids?, rectMm?, crossing?, mode:"replace"\|"add"\|"remove"\|"toggle"}` |
| `edit.selectNone` | (Esc when idle) | — | Esc | Agent | View | `{}` |
| `edit.delete` | Delete | Edit, toolbar | ⌫ | Agent | Edit | `{ids?}` |
| `edit.duplicate` | Duplicate | Edit | ⌘D | Agent | Edit | `{ids?}` |
| `edit.closePath` | Close Path | Edit | — | Agent | Edit | `{ids?, toleranceMm?: 0.5}` |
| `edit.autoJoin` | Auto-Join Selected Shapes | Edit | ⌥J | Agent | Edit | `{ids?, toleranceMm?: 0.05}` |
| `edit.deleteDuplicates` | Delete Duplicates | Edit | ⌥D | Agent | Edit | `{ids?}` |
| `edit.settings` / `edit.deviceSettings` | Settings / Device Settings | app menu / Edit, toolbar | ⌘, / — | Ui | View | `{}` |
| `tools.select` | Select | Creation toolbar | — | Ui | View | `{}` |
| `tools.rectangle` | Rectangle | Tools, Creation | ⌘R | Agent | Edit | `{xMm, yMm, wMm, hMm, cornerRadiusMm?, layer?}` (no params in the GUI: arms the tool) |
| `tools.ellipse` | Ellipse | Tools, Creation | ⌘E | Agent | Edit | `{cxMm, cyMm, wMm, hMm, layer?}` |
| `tools.drawLines` | Draw Lines | Tools, Creation | ⌘L | Agent | Edit | `{pointsMm? \| d?, closed?, layer?}` |
| `arrange.flipH` / `arrange.flipV` | Flip Horizontal / Vertical | Arrange, toolbar | ⇧⌘H, H / ⇧⌘V, V | Agent | Edit | `{ids?}` |
| `arrange.rotateCw90` / `arrange.rotateCcw90` | Rotate 90° CW / CCW | Arrange | `.` / `,` | Agent | Edit | `{ids?}` |
| `arrange.transform` | Numeric Edits, handles | toolbar | — | Agent | Edit | `{ids?, xMm?, yMm?, wMm?, hMm?, scalePct?, rotateDeg?, anchor?, lockRatio?}` |
| `arrange.nudge` | arrow keys | — | arrows | Agent | Edit | `{ids?, dxMm, dyMm}` (coalesces) |
| `arrange.rotateBy` | rotate keys | — | ⇧. ⇧, ⌘. ⌘, | Agent | Edit | `{ids?, deg, anchor?}` |
| `arrange.moveSelection.pageCenter` | Move Selected Objects › Page Center | Arrange | P | Agent | Edit | `{ids?}` |
| `view.zoomIn` / `view.zoomOut` / `view.zoomPage` / `view.zoomSelection` / `view.pan` | Zoom / Pan | Window, toolbar | ⌘= / ⌘- / ⌘0 / ⇧⌘A / Space | Ui | View | `{}` |
| `window.show` | Window › (panel) | Window | — | Ui | View | `{panel, visible}` |
| `preview.open` | Preview | Window, toolbar | ⌥P | Ui | View | `{}` |
| `laser.liveFraming` | Live Framing window | Laser panel | F1 (galvo) | Ui | View | `{}` (opens the window; framing itself is `laser.frame`) |
| `layer.select` | click a Cuts row | Cuts / Layers | — | Agent | View | `{layer}` |
| `layer.setCurrent` | palette click, nothing selected | palette | — | Agent | View | `{layer}` |
| `layer.assign` | palette click with a selection | palette | — | Agent | Edit | `{layer, ids?}` |
| `layer.cutSettings.set` | Cut Settings Editor, quick editor | Cuts / Layers | — | Agent | Edit | `{layer, subLayer?, name?, output?, show?, mode?, passes?, galvo?:{speedMmS, powerPct, frequencyKhz \| frequencyHz, qPulseNs}, gantry?:{speedMmS, powerMaxPct, powerMinPct}, fill?:{intervalMm \| lpi, angleDeg, angleIncrementDeg, bidirectional, crossHatch}, image?:{mode, intervalMm, angleDeg, bidirectional, negative, threshold}}` |
| `layer.cutSettings.get` | — | — | — | Agent | Query | `{layer}` |
| `layer.cutSettings.open` | double-click a row | Cuts / Layers | — | Ui | View | `{layer}` |
| `layer.setMode` / `layer.setOutput` / `layer.setShow` | Mode / Output / Show columns | Cuts / Layers | — | Agent | Edit | `{layer, mode}` / `{layer, on}` / `{layer, on}` |
| `layer.reorder` | row arrows, drag | Cuts / Layers | — | Agent | Edit | `{layer, toIndex}` |
| `laser.status` | status readout | Laser panel | — | Agent | Query | `{machine?}` (cached; no I/O) |
| `laser.frame` | Frame / Live Framing Start | Laser panel, Live Framing | — | **Human** | Device(Light) | `{style?:"bounds"\|"contour", source?:"job"\|"toolLayersOnly"\|"jobAndToolLayers", loop?}` (style/source do not change the fingerprint, S21) |
| `laser.frameStop` | Live Framing Stop | Live Framing | Esc | Stop | Device(Stop) | `{machine?}` |
| `laser.start` | Start | Laser panel, Live Framing | — | **Human** | Device(Emit) | `{}` → opens the ARM dialog; never emits by itself |
| `laser.stop` | Stop | Laser panel, toolbar, overlay | Esc (active) | Stop | Device(Stop) | `{machine?}` |
| `laser.redLight` † | Enable Red Light | Laser panel | — | **Human** | Device(Light) | `{on}`. Stretch: `Backend` has no pointer-only operation in this build, so the row renders disabled until one is specified (a lighting list holding one point, ended by the same stop path) |
| `laser.cutSelected` | Cut Selected Graphics | Laser panel | — | Agent | Edit | `{on}` |
| `laser.repeat` | Repeat (galvo counter) | Laser panel, Live Framing | — | Agent | Edit | `{count}` (1–999) |
| `device.list` | Devices | Laser panel | — | Agent | Query | `{}` |
| `device.select` | device dropdown | Laser panel | — | Agent | View | `{device}` (refused while that machine is busy) |
| `device.connect` / `device.disconnect` | (connect) | Laser panel | — | **Human** | Device(Connect) | `{device?}` |
| `device.detect` | Find My Laser | Devices | — | **Human** | Device(Connect) | `{}` → `DeviceProvider::detect` (read-only; USB needs S23) |
| `device.settings.get` | Device Settings | — | — | Agent | Query | `{device?}` |
| `device.settings.set` | Device Settings (OK) | dialog | — | **Human** | Settings | `{device?, …galvo keys of CAT §7}` |
| `device.create` / `device.importEzcad` / `device.calibrateAxis` | New device / markcfg import / Calibrate Axis | Devices, Device Settings | — | **Human** | Settings | `{…}` / `{path}` / `{axis, requestedMm, measuredMm}` |
| `device.inspectProfile` | — | — | — | Agent | Query | `{device?}` |
| `preview.stats` | Preview statistics | Preview | — | Agent | Query | `{}` |
| `job.plan` / `job.validate` / `job.summary` | (OpenLaser) | — | — | Agent | Query | `{selectedOnly?, machine?}` |
| `job.dump` | (OpenLaser) | — | — | Roots | Io | `{path}` |
| `job.submit` / `job.status` / `job.withdraw` / `job.frameRequest` | (OpenLaser, bridge only) | — | — | Agent | Request / Query | `{note?}` / `{submission?}` / `{submission}` / `{mode?}` |
| `document.inspect` / `session.list` / `command.list` / `command.schema` / `help.about` | — | — | — | Agent | Query | per §4.1 |
| `prefs.get` / `prefs.set` | Settings | dialog | — | Agent | Settings | `{key?}` / `{key, value}` (refuses `machine.*`, `safety.*`, `automation.*`, `humanOnly.*`) |
| `laserTools.materialTest` † | Material Test | Laser Tools | — | Agent | Edit | CAT §8.2 keys; generates a project; burning is `laser.start` |

## Appendix B. Prototype (OM) import mapping

| OM (`workspace.json`, `layer_settings.json`) | OpenLaser |
|---|---|
| `layer_settings.json` | read from the workspace's folder; if absent, OM's four built-in layers (R05 §4.1) with a notice |
| layer `vector-engrave` (`vector_engrave`) | C00, Line |
| layer `vector-cut` (`vector_cut`) | C01, Line |
| layer `raster` (`raster_engrave`) | C02, Fill |
| layer `frame-only` (`frame_only`) | T1 (tool layer: framed with "Tool layers only", never burned) |
| any other `layer_id` | the next free colour layer from C03, in file order, mode from `operation` |
| `operation: disabled` | the mapped layer with `output = false` |
| `name` | layer name |
| `power_percent`, `frequency_khz`, `pulse_width_ns`, `speed_mm_s` | `galvo.power`, `galvo.frequency` (kHz × 1000, refused if not whole Hz), `galvo.q_pulse`, `galvo.speed` |
| `passes` (vector) | `passes` |
| `passes` + `raster_angle_step_deg` (raster) | `passes` + `fill.angleIncrement` (OM's raster passes are cross-hatch angles; the segments are identical) |
| `raster_pitch_mm`, `raster_angle_deg` | `fill.interval`, `fill.angle`; `fill.bidirectional = true` (OM is always serpentine); `crossHatch = false` |
| `visible`, `output` | `show`, `output` (hidden + output is then refused at plan, B6) |
| `color` | not used: the target layer's palette colour shows |
| `air_assist` | `air_assist` (gantry only) |
| object `polylines`, `x`, `y`, `scale`, `rotation` | `Geom::Path`; `xform` = translate(x, y) · rotate(rotation° CCW) · scale(scale) (OM order: scale, rotate, translate) |
| object `name` | shape name |
| object whose `layer_id` is missing from the layer settings | import refused, naming the objects (B8) |
| a point that is not exactly `[x, y]` | import refused (B9) |
| `mirrorX` / `mirrorY` (browser localStorage, not in the files) | `cal.negate` in the machine profile (human-only) |
| `calibration.json` `scale_correction` | `cal.scale` on both axes, through `device.importEzcad` / `machine::om::parse_calibration` (human-only) |
| `profiles/openmopa-machine.ini` | `machine::markcfg::apply` onto `jpt_m7_200` (human-only) |

---

## Review log

**Revision 2 (2026-10-23), adversarial review.** The review checked this spec against `docs/design/*`, the golden
README, PLAN.md and research 05. Sources were checked directly: the crates.io API, docs.rs, the cached
registry/git sources (eframe 0.36.2 `epi.rs`, egui `response.rs`, flate2 1.1.10, craft-libs `5c874d5`/`3d37da3`,
nusb v0.2.8 source), OM v1 via `git show 5f7aa63` (read only), and the v1 `frame_box10` packets, decoded by a
script written for the review. Nothing was built, no USB was touched, and no third-party code ran. A concurrent
edit added S19/§2.6 (Windows); this revision merges with it.

**Safety holes closed**

- **R1 (S20).** A receipt was issued per *acknowledged* pass. Packets are acknowledged as soon as the board buffers
  them, up to the 2 s run-ahead window before the light traces them, and Frame Once of a small job is acknowledged
  before it even starts. Fix: a receipt now needs a !BUSY status after the list, and Loop sends one list per pass.
- **R2 (S21).** The fingerprint included the frame style and source, so framing with Bounds from Live Framing could
  never satisfy Start for a Contour job. They are now recorded on the receipt and left out of the fingerprint.
- **R3 (S22).** The token bound only the plan fingerprint. `PendingArm` did not keep the encoded job, so
  `run_armed` re-encoded. Fix: `bytes_hash` is now carried in the summary and the token. The driver recomputes it,
  and `run_armed` sends the exact `PendingArm.job`.
- **First-jump `dist`.** It depended on session state ("last commanded point"), which forced byte patching after
  confirmation. It is now computed from the centre for every list, which also matches v1.
- **Abort.** `ExecuteList` after the dummy packet was sent unconditionally; it now follows OM v1 (only when no list
  is executing).
- **Run-ahead.** The run-ahead cap (2 s) combined with "ExecuteList after packet 3" deadlocked any list whose
  packets each estimate more than 1 s (the business-card raster) and every FakeLmc with `buffer_depth < 3`.
  `ExecuteList` now also fires when the window fills or READY drops.
- **Watchdog progress.** Progress now means a valid read with a READY/BUSY change (OM v1's lesson). The factors are
  profile fields, and the 250 ms transfer budget is restored (PLAN §3.5).
- **Stop path.**
  - A stop that lands between queueing a run and dequeuing it is no longer cleared: latch generations, refusal
    `StoppedAfterQueue`.
  - `Cx::emit` can no longer block the driver: the events channel is unbounded.
  - Dead-man drop now joins the driver thread for ≤ 500 ms.
  - Panic hook and signal watcher in the GUI and `openlaser-hw`, because macOS `run_native` may exit without
    destructors.
  - Signal handlers only store an atomic.
  - The MCP `stop` uses its own connection outside the tool mutex, and a 17th connection accepts `machine.stop`.
  - Esc and any stop void `PendingArm` and close the ARM dialog.
  - The driver refuses `run` while framing.
- **R4: default-deny hardware (S23).** `OPENLASER_HW_PRESENT=1` is now required; Luiz builds `hw` into his own target
  lane; the first emission per bench session is capped at 10 % in software (PLAN §5.2 asked for it); `run` needs
  `--allow-emission`.
- **R5: profiles (S24).** Writes go only through an injected `ProfileStore` that `openlaser-cli` never gets.
  `openlaser-cli` is `Origin::Headless` for every subcommand (xtask rule 13). `validate_profile` rules are now
  explicit, including a non-empty checklist, and `mint` refuses an empty GUI checklist instead of passing it
  vacuously.
- **Synthetic input.** The taint record moves out of egui context data, which any code holding the `Context` could
  clear, into `Human`. Taint now lasts 3 frames + 1 s, so a click whose press was synthetic never counts.
  Synthetic events are built in one file (xtask rule 16).
- **GRBL frames.** GRBL frame programs are now a sealed `GcodeFrame`, so a burning program cannot be framed.
- **Hook regex.** Revision 1's PreToolUse regex blocked `cargo test -p openlaser-hw --lib` and missed `cargo run`
  and path invocations. It is replaced, with must and must-not examples, and extended to the terminal tool and
  app-data writes.

**Contracts that would not compile, and dependency or version fixes**

- `ui-egui` lacked `eframe`, which it needs for `eframe::App` and `CreationContext`.
- `DeviceKind` lived in `doc` but `driver` names it; it moves to `types`.
- `Fingerprint` and `MonoTime` need manual impls.
- `FakeLmc`/`FakeGrbl` took `Arc<VirtualClock>`, which `SimProvider` cannot supply; they now take `Arc<dyn Clock>`.
- `EncodedJob` was missing impls for the GRBL types.
- `frame_plan_unchecked` lacked style and source.
- The stub rule did not cover error types without a `NotImplemented` variant. Stubs now fail closed, and WP0
  implements `safety::testing` and `arm::testing` (plus `token_for_tests`) so WP5 and WP6 can test before WP3 lands.
- `DeviceProvider` gains `detect` and the bench-cap hooks, because the engine cannot reach `transport`.
- zip `deflate-flate2` alone selects no flate2 backend, which is a `compile_error!` in `format`; flate2
  `rust_backend` is now pinned.
- `rust-toolchain` `"1.99.0"` would have made rustup download a second toolchain; it is now `stable`.
- `resolver = "3"` is now stated explicitly.
- `[profile.test] panic` is dropped (Cargo ignores it and warns).
- Durations are now serialised as `…S`/`…Ms` (S2), not `{secs, nanos}`.
- Every other pin was verified to exist: egui 0.36.2 with craft-ui@5c874d51 and craft-quit/settings/atomic@3d37da38
  (no second craft-ui), usvg/resvg 0.45.1 with kurbo 0.11 and tiny-skia 0.11.4, nusb 0.2.8 (API listed in §2.4),
  rmcp 3.5.0 (3.7.0 newest), zip 9.0.5, keepawake 0.6.1 and ctrlc 3.5.2.

**Parallelism and ownership**

- `types/tests/t0_*` belonged to WP1 by path but WP4 by tier; ownership is now split.
- `rust-v1-candidate/` was split into `lmc/` (WP4) and `session/` (WP5).
- WP0 now pre-populates every dev-dependency, so no package edits another's manifest.
- Early merges I0 (`wp1-core`) and I1.5 (`wp6-core-b`) are added, because stubbed doc and engine left WP3, WP7 and
  WP8 nothing real to test.
- WP6 and WP9 commit their phases separately, so I1 can merge parts.
- Dev-dependency cycles (`driver` with `lmc-sim`, `types` with `testkit`) may be used from integration tests only;
  rule 10 previously forbade `driver` from using `lmc-sim` at all.

**Missing P0 items for the MOPA added**

- `openlaser-hw profile` (TTY-only profile edits, needed between HIL sessions before the GUI exists).
- `--trace` in the T1 event format.
- `--builtin dot|line|box10|square50|f-shape|corners`, matching the T1 inputs.
- `frame --extent` for H27, which the ±100 mm work area otherwise made impossible.
- The `usb_set_configuration` flag (macOS cannot claim an interface of an unconfigured device).
- `laser.redLight` marked as a stretch item, since the driver contract has no API for it.

**Testing gaps**

- About 20 named tests added across §6.3, §6.5 and §7, one per fix above.
- Seeded stops via `FakeLmc::set_hook`.
- `prop_importers_never_panic` and `prop_control_reader_never_panics` stand in for cargo-fuzz in this build.
- Binaries are tested in process, because uplifted binaries are shared across worktrees in a lane.

**Disk**

- The core lane never builds egui: its warm-up and clippy run per package, not `--workspace`.
- There is a separate `hw` lane for Luiz.
- The Windows cross-check runs only for the integrator.

**New open questions:** §9 items 11–12.
