# Proposal A: galvo-first vertical slice

*2026-10-23. This is a planning proposal for the Rust successor to the OpenLaser/OpenMopa Python prototype. It is written to be compared against the other angles.*

*Evidence base: research reports 01–06 and `rayforge-test/REPORT.md`, all in `openlaser-rust-plan/`. Citations look like "05 §1.10" (report 05, section 1.10) or "06 C7" (report 06, contradiction row C7). Prototype paths are relative to `~/AI/OpenLaser/OpenMopa`. Nothing was installed or run, and no hardware was touched.*

---

## 0. TL;DR

- **The goal is the shortest path to Luiz marking real jobs on the JPT M7 from a native Rust app that is measurably safer than the Python prototype.** Width comes after that. M1 is the "MOPA vertical slice", about 6 focused weeks after a 2-week M0. It contains:
  - a byte-exact Rust LMC encoder
  - a single-owner USB driver with a stop path of 250 ms or less
  - a safety crate that makes the five known prototype bugs structurally impossible
  - a minimal egui app: import, layers (Line/Fill/Frame-only), red-light frame, Mark, STOP
- **The Python prototype becomes the golden oracle.** Its features freeze now, and it gets seven small safety patches this week because Luiz keeps using it until M1. It is retired at M2.
- **Rayforge: interoperate and borrow, never fork, do not compete on gantries yet.** Its GRBL support is solid (`REPORT.md`), so Luiz can use it, or the prototype's export, for gantries while OpenLaser goes galvo-deep. raygeo and raydriver are MIT reference code to vendor selectively, never dependencies (06 §0.4).
- **Stack:** egui/eframe 0.37 + **egui_tiles 0.18** (not egui_dock; 06 C9), nusb 0.2.8, OS-thread drivers with no tokio in M1 (06 C7), MIT OR Apache-2.0.
- **Rename before the first public release.** "OpenLaser" collides with an active GPL Rust fiber-laser controller and with an MIT web GRBL app (03 §1.6).

---

## 1. Strategy and stances

**Strategy.** Build the narrowest end-to-end slice through every layer: document → plan → safety → LMC encoder → USB driver → UI. Ship it on the one machine Luiz owns and tests, before broadening any layer. Every slice layer is the final architecture, not a throwaway; M1 just populates each crate with the galvo case only.

- **Correctness is anchored externally.** Encoders must reproduce the Python prototype's hardware-proven bytes exactly (05 §5.3).
- **Safety is anchored structurally.** The prototype's bugs are fixed by types and by a single-owner driver state machine, not by more checks scattered through the UI.
- **Gantry support is deliberately second.** It begins in M2 (G-code export, golden-identical to Python) and M4 (live GRBL). Rayforge already covers live GRBL well on this Mac (`REPORT.md`: streaming, pause, stop and framing all worked), while nobody covers LMC galvos in licensed Rust (06 §0.2).

**Rayforge.**
- **Don't fork.** It is a 266k-line Python/GTK codebase with a bus factor of 1 and more than 1,100 commits per quarter (01 §7, §9).
- **Don't depend.** raygeo installs an unconditional mimalloc `#[global_allocator]` and is only ever built through PyO3 (06 §0.4, C3).
- **Borrow and interoperate:**
  - its architecture lessons: typed Ops IR, a separate machine-transform stage, a typed encoded-artifact enum, and raydriver's stop ordering (01 §8.1)
  - selected MIT files, vendored with attribution when a milestone needs them (§6)
  - its `.lbrn2` importer as the format reference (M5)
- **Contribute upstream later, optionally.** After M3, offer the LMC protocol notes and emulator spec as a Rayforge galvo addon design. Luiz's hardware is the scarce resource its maintainer asked for (01 §2.6). Not before, because it would split focus.

**Python prototype.**
1. Now: a **safety-patched freeze** with no new features (§8).
2. From M0: the **golden oracle**, pinned at a tagged SHA (`oracle-v1` = `6aed8ff` plus the safety patches).
3. **Strangler-fig, by machine class.** MOPA work moves to Rust at the M1 exit, and GRBL export at the M2 exit, after which the repo is archived read-only. The golden generator keeps working from the pinned tag forever.

**Name.** Rename the Rust app before any public binary or crate (decision D1). Collisions:
- `clayfrocker97/OpenLaser`: Rust, GPL, fiber and CO2, the same niche
- `openlaser.app`: MIT, 127★ in 3 days, owns the domain (03 §1.6, 04 §0.4)

Until then, "OpenLaser" is the codename and `ol-` is the crate prefix. All crates are `publish = false`, so a rename is a single mechanical commit. Pick the name by the M0 exit, after a GitHub, crates.io and trademark check (the trademark question is UNVERIFIED, 06 G15).

---

## 2. Architecture

### 2.1 Cargo workspace

New repository (D6). Pinned `rust-toolchain.toml`: whatever stable `rustup` installs in M0, which must be ≥ 1.95 for egui 0.37 (06 G14). Exact versions are pinned in `[workspace.dependencies]`.

| Crate | Responsibility | Depends on | Milestone |
|---|---|---|---|
| `ol-geom` | Polylines, `kurbo` flattening, affine transforms, bbox, join/close/dedupe, hatch fill (port of `raster.py`), later ordering | kurbo | M1 |
| `ol-doc` | `Document`/`Object`/`Layer` model, project file (zip with `document.json`, `format_version`, migrations), snapshot undo | ol-geom, serde, zip | M1 |
| `ol-machine` | `MachineProfile`, kinematics blocks, calibration, importers for `openmopa-machine.ini` + `calibration.json`. Holds **no device I/O.** | serde, toml | M1 |
| `ol-plan` | Document + machine → `JobPlan` (sections, resolved params, frame geometry), galvo time model, fingerprint | ol-doc, ol-machine | M1 |
| `ol-safety` | **All hard refusals.** It is the only constructor of `EmitPlan`/`FramePlan` and `ArmToken`. Refusal codes are an enum. | ol-plan | M1 |
| `ol-driver` | Generic driver runtime: `Backend` trait, driver thread, `StopLatch`, event channel, stop watchdog | crossbeam-channel, ol-safety | M1 |
| `ol-lmc` | Pure LMC protocol: field transform, parameter encoders, `LightingList`/`MarkingList`, packetizer, status parser, and the **session state machine** generic over `LmcTransport` | ol-safety, ol-driver | M1 |
| `ol-lmc-usb` | `LmcTransport` on `nusb`; discovery of `0x9899`, plus clone `0x9980` with a refusal message (06 §0.6) | ol-lmc, nusb | M1 |
| `ol-lmc-emu` | Fake LMC board: packet queue, READY/BUSY timing, fault injection, emission recorder and renderer | ol-lmc, tiny-skia | M1 |
| `ol-io` | SVG import (`usvg`), DXF import (`dxf`), prototype `workspace.json` import. Later `.lbrn2`, SVG/DXF export. | ol-doc | M1 |
| `ol-gcode` | GRBL G-code emitter (golden-identical to `gcode.py`); later the streaming sender | ol-safety | M2 / M4 |
| `ol-app` | `AppState`, `Command`/`Event` enums, planning worker, arming flow. **No egui dependency.** | everything above | M1 |
| `ol-ui` | eframe shell, egui_tiles layout, canvas widget, dialogs (`rfd`) | ol-app, eframe, egui_tiles | M1 |
| `ol-cli` | Headless commands: `detect`, `status`, `plan`, `dump` (hex trace), `frame`, `mark`, `import-prototype`, `golden-check`, `hw-session` | ol-app | M1 |
| `xtask` + `tools/golden/` (Python) | Golden regeneration from the pinned prototype; packaging | none | M0 |

**Dependency rules, enforced by `cargo-deny` bans plus a CI script on `cargo tree`:**
- Only `ol-lmc-usb` may depend on `nusb`.
- Only `ol-ui` may depend on egui or wgpu.
- No crate below `ol-app` may depend on tokio.
- `ol-safety` may not depend on any encoder.
- Encoders accept only `ol-safety` output types.

The text, raster, camera and render crates that 04 §13 proposes are created when their milestone arrives, never as empty scaffolding.

### 2.2 Core data model

```rust
// ol-doc: units are mm, f64, design space Y-up. A galvo origin is the field centre.
pub struct Document { pub format_version: u32, pub objects: Vec<Object>, pub layers: Vec<Layer>, pub notes: String }
pub struct Object  { pub id: ObjectId, pub name: String, pub layer: LayerId,
                     pub transform: kurbo::Affine, pub geometry: Geometry }
pub enum Geometry { Paths(Vec<kurbo::BezPath>) /* M3+: Bitmap(AssetRef), Qr(QrSpec), Text(TextSpec) */ }

pub struct Layer {
    pub id: LayerId, pub name: String, pub color: Rgb,
    pub mode: LayerMode, pub output: bool /* burns */, pub show: bool /* canvas only */,
    pub passes: u16,
    pub galvo: GalvoCut,    // used only when the target machine is a galvo
    pub gantry: GantryCut,  // used only when the target machine is a gantry
    // M3: sub_layers: Vec<SubLayer>, global_passes: u16
}
pub enum LayerMode { Line, Fill(FillSpec), FrameOnly }   // "disabled" == output:false
pub struct FillSpec { pub interval_mm: f64, pub angle_deg: f64, pub angle_step_deg: f64, pub bidirectional: bool }
pub struct GalvoCut  { pub speed_mm_s: f64, pub power_pct: f64, pub frequency_hz: u32,
                       pub q_pulse_ns: Option<u32>, pub timings: Option<GalvoTimings> /* M3 override */ }
pub struct GantryCut { pub speed_mm_s: f64, pub power_pct: f64, pub air_assist: bool, pub power_mode: Option<PowerMode> }
```

**Deliberate divergence from LightBurn.** LightBurn shares speed and power across device kinds (02 §7.7). OpenLaser keeps them **inside** the galvo and gantry blocks.

- **Why:** the Rayforge test caught the prototype leaking MOPA "Vector Cut" defaults (300 mm/s, 100 %) into a diode job (`REPORT.md`, Takeaway 1). Separate blocks make that leak unrepresentable.
- **On `.lbrn2` import (M5):** shared values go into the block of the file's device kind.
- **On retarget:** switching a document to a machine of the other kind uses that machine's `layer_defaults` for any block never set.

```rust
// ol-machine
pub struct MachineProfile { pub id: MachineId, pub name: String, pub kind: Kinematics,
    pub limits: JobLimits, pub layer_defaults: LayerDefaults,
    pub require_frame_before_mark: bool, pub checklist: Vec<String> }
pub enum Kinematics { Galvo(GalvoMachine), Gantry(GantryMachine) }
pub struct GalvoMachine {
    pub board: LmcBoardFilter,          // vid 0x9588, pid 0x9899, optional serial
    pub field_mm: f64,                  // nominal lens field (200)
    pub cal: FieldCalibration,          // scale_x/scale_y (1.1834 today), offset, swap_xy, invert_x/y; M3: cor
    pub mirror: Mirror,                 // persisted per machine (it lived in browser localStorage, 05 §1.7)
    pub source: LaserSource,            // MopaFiber { pulse_table_ns, freq_hz: RangeInclusive<u32>, max_power_pct }
    pub timings: GalvoTimings,          // on/off/polygon/jump delays; defaults = GP 100/100/100/8 µs
    pub jump_speed_mm_s: f64, pub light_speed_mm_s: f64, pub mo_settle_10us: u16,
    pub ports: PortMap,                 // laser bit 0, red light bit 8; M3: door/foot inputs
    pub init: LmcInitParams,            // EZCAD-derived init words (05 §1.4), "expert" view only
}
pub struct GantryMachine { pub bed_mm: (f64, f64), pub origin: Corner, pub dialect: GrblDialect,
    pub max_speed_mm_s: f64, pub link: GantryLink /* ExportOnly | Serial | Tcp (M4) */ }
```

The effective field is `field_mm × scale` per axis. Coordinates encode with `0xFFFF / F`. `65536/scale` is used only when deriving a field from a `.cor` (06 C11).

### 2.3 Job pipeline and IR

```
Document ──plan()──▶ JobPlan ──ol_safety::validate()──▶ EmitPlan + FramePlan ──▶ machine transform
   (mm, Y-up)         (sections,                         (only ol-safety can        (galvo: FieldTransform → u16;
                       resolved params,                   construct these)           gantry: bed origin)
                       frame geometry)                         │
                                                               ▼
                       Encoded::{Lmc(LmcJob) | Gcode(String)} ──▶ DriverHandle::run(job, ArmToken)
```

```rust
// ol-plan
pub struct JobPlan { pub machine: MachineId, pub sections: Vec<Section>, pub frame: Vec<Polyline>,
                     pub estimate: Duration, pub fingerprint: Fingerprint /* blake3 of canonical plan + calibration */ }
pub struct Section { pub layer: LayerId, pub pass: u16, pub kind: SectionKind /* Line | Fill{angle_deg} */,
                     pub params: Resolved /* Galvo(GalvoCutQ) | Gantry(GantryCut) */, pub paths: Vec<Polyline> }
pub struct GalvoCutQ { pub requested_mm_s: f64, pub encoded_speed: u16, pub actual_mm_s: f64,   // quantised (05 §1.8)
                       pub mark_current: u16, pub q_period: u16, pub pulse_ns: u16, pub timings: GalvoTimings }
pub struct Polyline { pub pts: Vec<P2>, pub source: ObjectId }

// ol-safety: the only way to get these is validate()
pub struct EmitPlan  { sections: Vec<Section>, fingerprint: Fingerprint, frame_required: bool }
pub struct FramePlan { paths: Vec<Polyline>, fingerprint: Fingerprint }   // has no power fields at all
pub struct ArmToken  { fingerprint: Fingerprint, machine: MachineId, session: SessionId, expires: Instant, nonce: u64 }

// ol-lmc
pub enum ListCmd { ReadyMark, DelayTime(u16), WritePort(PortMask), JumpSpeed(u16), MarkSpeed(u16),
    FiberPulseWidth(u16), MarkCurrent(u16), QSwitchPeriod(u16),
    LaserOnDelay(SDelay), LaserOffDelay(SDelay), PolygonDelay(SDelay), JumpDelay(SDelay),
    JumpTo { x: u16, y: u16, dist: u16 }, MarkTo { x: u16, y: u16, dist: u16 }, EndOfList }
pub struct LightingList(ListBuf);   // API: jump_to(), light_to(); no MarkTo, no MO, no LaserOnPoint (05 §1.6)
pub struct MarkingList(ListBuf);    // built only from &EmitSection
pub struct LmcJob { pub prelude: Vec<Single>, pub packets: Vec<Packet /* Box<[u8; 3072]> */>,
                    pub postlude: Vec<Single>, pub packet_est: Vec<Duration>, pub fingerprint: Fingerprint, pub emits: bool }
```

**Encoder rules.** The golden suite enforces each one.
- Rounding: `f64::round_ties_even` wherever Python calls `round()`, and `trunc` wherever it calls `int()` (05 §0.6).
- **Every list starts with an explicit `JumpTo`** (fixes the elision bug, 05 §1.7).
- The parameter cache resets at every list start, so every list carries the full parameter block. That gives byte parity with Python's one-controller-per-job model and is safer after an abort.
- M1 keeps Python's **one list per layer-pass** structure. Merging layers into one continuous list is an M3 optimisation, once H2 is known.

### 2.4 Driver and transport traits

```rust
// ol-lmc: the only USB-shaped seam. Implemented by ol-lmc-usb (nusb), ol-lmc-emu, and a Recording wrapper.
pub trait LmcTransport: Send {
    fn single(&mut self, cmd: [u16; 6], read: bool, timeout: Duration) -> Result<Option<[u16; 4]>, XferError>;
    fn submit_packet(&mut self, pkt: &[u8; 3072]) -> Result<(), XferError>;           // non-blocking submit
    fn poll_packet(&mut self, slice: Duration) -> Result<Option<Completion>, XferError>; // wait_next_complete(≤20 ms)
    fn cancel_all(&mut self);                                                           // nusb Endpoint::cancel_all
}

// ol-driver: the runtime every machine kind plugs into
pub trait Backend: Send + 'static {                 // runs only on its own driver thread
    fn capabilities(&self) -> Capabilities;         // kinematics, red_light, pause: Unverified|Supported|No, reads_inputs
    fn connect(&mut self, cx: &mut Cx) -> Result<DeviceInfo, DriverError>;
    fn frame(&mut self, plan: &FramePlan, mode: FrameMode /* Once | Loop */, cx: &mut Cx) -> Result<Outcome, DriverError>;
    fn run(&mut self, job: &Encoded, cx: &mut Cx) -> Result<Outcome, DriverError>;
    fn pause(&mut self, cx: &mut Cx) -> Result<(), DriverError>;   // Unsupported for galvo until H15
    fn resume(&mut self, cx: &mut Cx) -> Result<(), DriverError>;
    fn emergency_stop(&mut self) -> StopReport;     // bounded; called by the runtime when the latch is set
    fn status(&mut self) -> Result<MachineStatus, DriverError>;
}
pub struct Cx<'a> { latch: &'a StopLatch, events: &'a Sender<DriverEvent> }   // cx.check()? returns Err(Stopped) when latched

pub struct DriverHandle { ctl: Sender<Control>, latch: Arc<StopLatch>, pub events: Receiver<DriverEvent>, pub caps: Capabilities }
impl DriverHandle {
    pub fn connect(&self);
    pub fn frame(&self, plan: FramePlan, mode: FrameMode);
    pub fn run(&self, job: Encoded, arm: ArmToken);   // the driver re-checks fingerprint, expiry and the frame requirement
    pub fn stop(&self);                               // never blocks: atomic store + wake, < 1 ms
}
pub enum DriverEvent { Connected(DeviceInfo), State(MachineState), Progress { packets_acked: u32, total: u32, eta: Duration },
                       Finished(Outcome), Stopped(StopReport), Fault(DriverError), Trace(TraceLine) }
pub struct StopReport { pub latch_seen: Duration, pub quiet: Duration, pub mo_closed: bool, pub ports_off: bool }
```

`MachineStatus` is parsed from GetVersion word 3. A failed read is an `Err`, **never** a set of status bits; galvoplotter treats `-1` as READY|BUSY (05 §1.2). Unknown bits are expected: the GP30 board idles at `0x0220` (06 §0.2).

### 2.5 Threading and the real-time stop path

**Threads:**
- **UI thread (eframe).** It never touches I/O or does long work.
- **One planning worker** (`std::thread` plus a channel; rayon later). It runs plan, validate, encode and fingerprint.
- **One driver thread per connected machine.** It alone owns the `nusb` device (05 §1.10 requirement 1; 06 C7).
- **No tokio in M1.** The GRBL sender in M4 uses the same thread model on `serialport`.

**Driver thread rules:**
- No wait is longer than 20 ms. Every loop calls `cx.check()`.
- Packets go out with `submit` + `wait_next_complete(≤20 ms)`, never `transfer_blocking`. That nusb call cancels on timeout but then loops until the cancelled transfer returns (06 §2.6).
- Single commands use a 250 ms timeout (H13 tunes it).
- **There is no automatic USB reset, ever** (05 §1.1). A transfer error stops the job, emits a typed fault, and leaves reconnecting to the operator.

| Stage | Budget | Mechanism |
|---|---|---|
| Esc key or STOP click → latch set | ≤ 16 ms | Esc is consumed before widgets every frame. The STOP button is always visible, never disabled, and never behind a modal. |
| Latch → driver notices | ≤ 20 ms | Waits are sliced at 20 ms or less; the latch also wakes the control channel. |
| Cancel the in-flight packet | ≤ 20 ms | `cancel_all()`, then drain completions. **Assume a partial packet reached the board** (06 C16). |
| Abort on the wire | ≤ 60 ms | `StopExecute` → `Fiber_SetMo(0)` → `ResetList` → (only if READY within 50 ms: dummy EOL packet + `SetEndOfList` + `ExecuteList`, kept for parity until H14) → `Fiber_SetMo(0)` → `WritePort(0)` (05 §1.9). |
| Confirmation | ≤ 50 ms | GetVersion shows !BUSY, so the `StopReport` is emitted. |
| **Total** | **target ≤ 150 ms; hard ≤ 250 ms in CI on the emulator; measured on hardware against Python's 0.18 s** | |
| Stop watchdog | 500 ms | If no `StopReport` arrives, the UI shows a red banner: **"STOP NOT CONFIRMED — use the laser's key switch / E-stop."** Software stop is never presented as the primary safety device (06 G12). |

H14 also tests one ordering question: does `Fiber_SetMo(0)` first cut emission faster? Until it answers, keep the hardware-proven order.

### 2.6 LMC session: the algorithm Python got wrong

```
connect (once per session): init sequence from 05 §1.4 verbatim; position := (0x8000,0x8000) until H3 says otherwise
run(job):
  driver re-checks: fingerprint == ArmToken.fingerprint, token unexpired & unused, frame-of-same-fingerprint done (if required)
  prelude: ResetList, WritePort(0x0001), Fiber_SetMo(1), GetVersion
  for each packet i:
      wait READY: poll ≤10 ms, stop-aware; NO deadline-then-proceed (fixes 05 §0.1).
                  If no READY and no BUSY progress for est(queued)×1.5 + 2 s → Fault::BoardNotReady → emergency_stop
      submit packet; SetEndOfList; if i == 2 or (last and i < 2): ExecuteList          // Python parity (05 §1.5)
  wait !BUSY: same watchdog rule (progress-aware, derived from packet_est, never proceeds)
  postlude ONLY when !BUSY: Fiber_SetMo(0), WritePort(0), GetMarkTime(3)               // never close MO while BUSY
  no abort, no Reset, no USB reset after success (05 §0.2)
```

Continuous framing streams `LightingList` packets under the same READY rule, so it can never overfill the board, which was the original storm trigger (05 §1.10). Stopping a frame uses the same abort; it is cheap and the laser is never on.

### 2.7 Safety model

**Principle: refusals, not warnings.** There is no "Proceed anyway" anywhere; contrast Rayforge's advisory dialog (01 §8.2). The only way past a refusal is to change the document or the machine profile.

| Layer | What it guarantees | Where |
|---|---|---|
| 1. `ol_safety::validate` (pure, deterministic, property-tested) | Field bounds with no clamping. Finite coordinates. Path/point limits: the 3000/2000/20000 parity limits in M1, then limits derived from time and memory after H2. Power ≤ `max_power_pct`. Frequency inside the source range. Pulse width on the table (±0.5 ns snaps with a notice). **Speed must encode to ≥ 1 unit with ≤ 10 % quantisation error.** **An object on an unknown layer is refused.** **A hidden layer with Output on is refused.** Nothing-to-mark is refused. The whole job is validated before byte 1. | It runs live in the UI (refusal list), again at arm time, and the driver re-checks the fingerprint. |
| 2. Types | `FramePlan` has no power. `LightingList` cannot express `MarkTo`/MO/LaserOnPoint. `MarkingList` needs an `EmitSection`. `EmitPlan`'s constructor is private to `ol-safety`. | Compile time, plus emulator property tests |
| 3. Frame ⊇ burn | The `FramePlan` is derived from the same `JobPlan` as the `EmitPlan`: every emitting section plus Frame-only layers, regardless of `show`. | Property: every emitted segment lies on the traced contour, or inside the traced bounds |
| 4. ARM | The confirm dialog renders the **`EmitPlan`'s own per-layer resolved values**: actual quantised speed, power, kHz, ns, passes, bbox, time estimate and the short fingerprint. Typing `ARM` mints a token that is **single-use, expires after 60 s, is bound to the fingerprint, and is invalidated by any edit**. The text field clears afterwards. The machine's checklist items must be ticked. | `ol-app` mints the token; the driver verifies it |
| 5. Framed-before-mark | Default ON for galvos: Mark is refused unless a frame of the *same fingerprint* finished in this session. | `ol-app` + driver |
| 6. Board invariants | Never write without READY. Never close MO or StopExecute while BUSY unless stopping. Progress-aware watchdog. No auto reset. A typed fault stops the job. | `ol-lmc` session (§2.6) |
| 7. Interlocks (M3, after H11) | The door and stop inputs are read before MO opens and between packets. They are **additional** protection, never primary, and the docs say so. | `ol-lmc` session |

**How each known prototype bug becomes impossible in Rust:**

| Bug (05) | Rust structural fix | Named regression test |
|---|---|---|
| 3 s wait cap truncates long jobs (§0.1) | No deadline-then-proceed code path exists; watchdog action = stop | `t6_busy_20s_completes_without_mo_close` |
| First jump elided (§0.3, G4) | Explicit `JumpTo` opens every list | `t1_dot_test_has_explicit_jump` |
| Confirm dialog shows Frame values (§0.4, G1) | Dialog renders `EmitPlan` only; there is no "frame power" in mark flow | `kittest_confirm_shows_layer_params` |
| Hidden + output layer burns unframed (§0.5, G3) | Refusal + frame derived from emit sections | `safety_hidden_output_refused`, `prop_frame_covers_burn` |
| Speed < 3.61 mm/s → MarkSpeed(0) (G5) | Refusal on 0 units or > 10 % error; UI shows actual speed | `safety_speed_quantisation` |
| ARM always true, sticky text (G2) | Single-use, expiring, fingerprint-bound token | `arm_token_single_use`, `arm_token_invalid_after_edit` |
| Unknown `layer_id` silently dropped (G7) | Refusal | `safety_orphan_object_refused` |

---

## 3. UI approach

**Framework: egui/eframe 0.37 + egui_tiles 0.18 + rfd.** I follow 04's matrix (egui 85.6 against Tauri 81.0) with 06's correction (egui_dock 0.21.1 requires egui 0.36). Two reasons outweigh Tauri's sensitivity win from "reuse the web UI":
- **One process and one language for the AI executors to reason about.** That includes the stop button's path to the latch.
- **`egui_kittest`.** It lets safety UI behaviour be tested headlessly, for example that the confirm dialog shows layer values.

The prototype's 3k-line `ui.py`, with its embedded HTML, is not a reusable asset in practice.

**M1 window layout (egui_tiles):**
- **Left:** Cuts/Layers, with colour, mode, output, show and parameters.
- **Centre:** canvas showing the field square ±F/2, crosshair, objects coloured by layer, the selection, and numeric move/scale/rotate/mirror.
- **Right:** Laser panel with Connect, status, Frame Once/Loop, Mark, the refusal list, the estimate, and a large STOP.
- **Bottom:** log and USB trace.
- **Menus:** an in-window egui menu bar. A native `muda` menu is a one-day timebox in the spike and **not a gate**. That differs from 04 §2.4, which made muda a go/no-go; a galvo-first MVP does not need a native menu.

**Canvas strategy.**
- **M1:** egui `Painter` with world-space polylines cached per object and re-flattened only on large zoom changes, plus `rstar` hit-testing. M1 documents are bounded by the 20k-points-per-pass limit; the business card is 403 objects and 3,057 points (05 §4.2).
- **Later:** the custom wgpu hairline pipeline via `egui_wgpu::Callback` (04 §3) is scheduled for M5, when traced images and big documents arrive, behind a `CanvasRenderer` trait.

**Spike (M0, 5 days), with go/no-go metrics measured on the M2 Air:**

| # | Metric | GO | NO-GO |
|---|---|---|---|
| S1 | p95 frame time while panning/zooming a synthetic 20k-path, 200k-segment doc | ≤ 16.7 ms. 16.7–33 ms is still GO, with the wgpu pipeline pulled forward to M3. | > 33 ms after caching |
| S2 | Hit-test p99 at 20k paths | ≤ 2 ms | > 10 ms |
| S3 | Esc → `StopLatch` set while a 500 ms planning job runs; longest UI-thread stall | ≤ 16 ms; no stall > 50 ms | either exceeded |
| S4 | `cargo tree -d` shows a single egui and a single wgpu across eframe, egui_tiles and egui_kittest | yes | unresolved after 2 days |
| S5 | egui_kittest test plus image snapshot pass on the GitHub macOS runner | yes | no |
| S6 | VoiceOver announces STOP and the layer rows (AccessKit) | soft gate | — |

**Fallback.** Tauri 2 with a web UI over `ol-app`'s serde `Command`/`Event` types (04 §2.4 Plan B). Because `ol-app` has no UI dependency, the fallback costs about 3–4 weeks of UI, not a rewrite.

---

## 4. Testing and verification

### 4.1 Golden oracle (prototype → Rust)

**Generator.** `tools/golden/capture.py` is the 05 Appendix A harness, extended. It runs in the prototype venv against the `oracle-v1` tag with `OPENMOPA_MOCK=1`; `usb.core.find` and `USBConnection` are patched to raise; it uses a temporary `CALIBRATION_PATH`. Its JSON output is committed under `crates/*/tests/golden/` with metadata: OM SHA, galvoplotter 0.2.0, scale. **Never run `site-packages/test/*.py`, which opens real USB** (05 §1.3.2).

| Tier (05 §5.3) | Content | Rust must match |
|---|---|---|
| T0 numeric | `mm_to_galvo`, power, frequency, speed, delay sign word, pulse snap, distance word. 10,000 seeded random cases plus edges: 30 %/70 % ties, ±F/2, 1e-6 tolerance, sub-unit speeds. | exactly |
| T1 LMC | Scenarios mark_box10, frame_box10, dot_test, long_job (9 packets), the business card per layer at F = 236.68, and abort | Packet bytes **and** prelude/postlude singles byte-exact (`mark_box10` packet sha256 `9ca1e12d…59f5`, abort packet `e479558e…6a18`) |
| T2 G-code (M2) | 4 profiles × {vector, multi-pass, air assist, hatch, plotter} | Byte-exact after masking `; Generated:`, including the double `M5` |
| T3 hatch | `hatch_polylines` | Count and order exact; coordinates within 1e-9 |
| T4 import | DXF fixtures from `test_importers.py` | Point count exact; within 1e-9 |
| T5 safety | Refusal paths | Same decision, mapped to a refusal **code** |

**How the divergences are handled.** Python reconnects, re-inits and aborts on every job (05 §0.2); Rust deliberately does not. Rather than building a "py-compat" sequencing mode (05 §5.3), the split is:
- the **encoder** (`LmcJob` = prelude + packets + postlude) is byte-exact against goldens;
- the **session** is verified behaviourally (§4.2).

Goldens come from `oracle-v1`, the patched prototype, so `dot_test` already contains the explicit jump. A one-time `oracle-v0` vs `oracle-v1` diff is committed as evidence that the patches changed only what they intended.

### 4.2 Emulators

**`ol-lmc-emu`** is a Rust board model:
- a configurable packet queue depth (default 4 until H2 measures it)
- READY drops when the queue is full
- BUSY while executing, with simulated time from jump/mark speeds and delays
- MO state and port bits
- an **emission recorder**: MarkTo segments executed while MO is open and port bit 0 is set, rendered to a tiny-skia image
- fault injection: writes stall until timeout, READY never returns, BUSY stuck, a disconnect mid-packet, short reads, unknown status bits (`0x0220`)

The **T6 timing suite** runs entirely on it:
1. stop ≤ 250 ms from every state, including a full buffer with stalled writes
2. zero writes while not READY
3. a 20 s BUSY job completes, with MO closed only after BUSY clears
4. the watchdog fires only without progress
5. **every frame job records zero emission**
6. the emission image for the business card matches the design rasterised at layer level

**Recorded hardware traces.** The `Recording` transport wraps the USB transport during HIL sessions. Its traces are replayed into the emulator as status-timing scripts, so real READY/BUSY behaviour becomes a regression fixture.

**Fake GRBL (M4):** port `rayforge-test/fake_grbl.py` (15-block planner, realtime bytes) to Rust, or run it in CI.

### 4.3 Property tests and fuzzing

**`proptest`** (10k cases nightly, 256 per PR):
- in-field points encode within 0..=0xFFFF, monotonic per axis
- any out-of-field point is refused, never clamped
- accepted speeds encode to ≥ 1 unit
- every packet is 3072 bytes, `0x8002`-padded
- lighting lists contain no `0x8005`/`0x8003`
- the plan covers each emitting object's paths exactly once per pass
- hatch segments lie inside their polygon (even-odd)
- stopping at a random emulator tick ends with MO closed and ports at 0

**`cargo-fuzz`:** SVG, DXF and prototype-JSON importers from M2; `.lbrn2` from M5.

### 4.4 Hardware-in-the-loop protocol

**Rules:**
- Luiz is present for every step; the agent never runs emission.
- Tests are `#[ignore]` behind `--features hw` and need `OPENLASER_HW_PRESENT=1`. Emission steps also require `--allow-emission` and the typed ARM.
- Red light first; emission only on scrap at low power, with the key switch in reach.
- Every session saves a committed USB trace plus a one-page result note.

Unknowns are ordered by risk **to this plan** (05 §7):

| Order | Unknown | Session | Why first |
|---|---|---|---|
| 1 | H12 descriptors, `set_configuration` need on macOS 26 (06 §2.6) | HIL-1 (red light) | nusb bring-up blocker |
| 2 | **H2** READY/BUSY semantics and buffer depth | HIL-1 | flow control and watchdog depend on it |
| 3 | **H13** behaviour of a write when full; safe timeout | HIL-1 | reset-storm avoidance |
| 4 | **H14** minimal abort and stop latency (10 trials each: framing loop, streaming) | HIL-1 | the 0.18 s bar |
| 5 | H1 a >8 s red-light job continues (Rust), not truncated | HIL-1 | the 3 s cap regression |
| 6 | H3 mirror position after Reset, jobs and abort (`GetPositionXY`) | HIL-1 | first-jump `dist` word |
| 7 | H4 axis orientation (asymmetric "F") | HIL-2 (low power) | mirrored burns |
| 8 | H10 low-speed quantisation (5/10/50 mm/s) | HIL-2 | validates the 10 % rule |
| 9 | H1 (emission) a ≥30 s low-power hatch completes | HIL-2 | M1 exit |
| 10 | H11 port map and inputs (door/foot) | HIL-4 (M3) | interlocks |
| 11 | H5 `.cor` vs blank table; cause of 1.1834 | HIL-4 | accuracy; **read the EZCAD `.cor` first, which needs no laser** |
| 12 | H6/H7/H8/H9/H20 pulse, power, frequency, delay encodings and init | HIL-5 / EZCAD capture | Python parity already proves "works"; this is fidelity |
| 13 | H15 pause/resume | HIL-5 | pause stays hidden until it passes |
| 14 | H16, H19 mark time, MO settle | HIL-5 | progress UI, first-mark quality |
| 15 | H18 Windows/Linux | M4 | platforms |
| 16 | H17 wedge recovery | only if it happens naturally | never provoke it |

### 4.5 CI matrix (GitHub Actions)

| Runner | Jobs |
|---|---|
| `macos-15` arm64 (primary) | fmt, clippy `-D warnings`, nextest (T0–T6 + properties), kittest snapshots, `cargo-deny`, the dependency-rule script, release build |
| `ubuntu-24.04` | Core crates + emulator + goldens; `ol-ui` build only |
| `windows-2022` (from M4) | Build + core tests |
| Nightly | proptest 10k, fuzz 10 min per target, `cargo tree -d` drift report |
| Manual | Golden regeneration (needs the prototype venv), packaging/notarisation |

### 4.6 How the AI workflow plugs in

`/route-implementation` fits this plan because golden tests turn executor output into a pass/fail check, not a review judgement.

- **Executor tasks (Codex/OpenCode):**
  - encoders against T0/T1
  - importers against T4 fixtures
  - hatch against T3
  - the G-code emitter against T2
  - UI panels against kittest specs
- **Red-zone crates and files.** These are designed by Claude (Opus), and any change to them requires the T6 suite plus Luiz's review before a hardware run:
  - `ol-safety`
  - `ol-lmc/session.rs`
  - `ol-driver`
  - the arming flow in `ol-app`
- **Executor spec rule.** Every executor spec lists allowed sources (§6). It forbids reproducing code from GPL projects (balor, LaserGRBL) or from the unlicensed GP30 driver, including from memory.

---

## 5. Roadmap

Efforts are **focused weeks** of Luiz-directed work with AI executors. Expect 1.5–2× in calendar time.

| Milestone | Scope | Exit criteria (observable) | Effort | What Luiz can do afterwards |
|---|---|---|---|---|
| **M0 Foundations** | `rustup`; new repo, workspace, CI, `cargo-deny`; Python safety patches + `oracle-v1` tag (§8); golden generator + committed T0/T1/T3/T5 goldens; T0 encoders in Rust passing; emulator skeleton; egui spike (§3); name chosen | Goldens committed with metadata; `golden-check` passes all T0 cases; spike report with S1–S5 numbers and a GO/NO-GO; patched prototype passes its 119 tests plus the new regression tests | 2 | Run a *safer* Python prototype; see Rust reproduce its encoder math |
| **M1 MOPA vertical slice** | Below | Below | 6 | Run his MOPA jobs from the Rust app |
| **M2 Prototype retired** | G-code export (T2 parity) + plotter dwell; editable profiles (4 built-ins, duplicate); per-machine layer defaults; time estimate in UI; travel ordering (greedy NN + 2-opt; vendor raygeo `optimize.rs` only if it builds standalone in under a day, else own code); path hygiene (join/close/dedupe); **MOPA material test grid** (frequency × pulse, power × speed); per-axis Calibrate Axis wizard; frame styles (bounds/contour) with live update while looping; repeat count; Developer-ID signed + notarised universal DMG | T2 goldens byte-exact; DMG installs on a clean Mac with no `xattr`; Luiz works 2 weeks with no Python; prototype repo archived | 5 | Everything the prototype did, plus a test grid and signed installs |
| **M3 Galvo depth** | `.cor` read and upload (MeerK40t / corfile2meerk40t formats, 03 §4.6); per-layer timing overrides; sub-layers + global passes; wobble; bitmap import + dithering (Threshold, Floyd–Steinberg, Jarvis, Stucki, Atkinson, Ordered) → galvo raster using MeerK40t's per-run power model (06 §0.8); QR/DataMatrix + Hershey serial numbers; door/foot interlocks (H11); pause if H15 passes; append-only job log; time/memory-derived limits | Grid of 100 mm marked with `.cor` within ±0.1 mm without the 1.1834 trim (or documented why not); dithered photo on anodised plate; interlock opens → job stops ≤ 250 ms (HIL); sub-layer goldens | 6 | Production MOPA work: colour tests, photos, serial plates |
| **M4 Gantry live + platforms** | GRBL sender (serialport, OS thread, character counting, realtime `!`/`~`/`0x18`, error → hold + reset, 06 C7/05 §3.3), gantry frame/jog/home/console; Windows NSIS + libwdi helper exe + clone `0x9980` detection; Linux AppImage/.deb + udev | Fake-GRBL suite: stop ≤ 250 ms, no streaming after `error:`; Windows red-light frame on real board (H18); Linux same | 5 | Drive CO2/diode/plotter live (needs machine access) and share builds |
| **M5 Design suite (T1)** | Shapes, node edit, booleans (`i_overlay`), offset (`cavalier_contours`), text (parley/skrifa + Hershey), align/arrays/snapping/clipboard; `.lbrn2` import (Y-up, 06 §0.7; 2nd fixture from local LightBurn 1.5.04 if licensed); SVG/DXF export; wgpu canvas if S1 required it; muda menus | Corpus snapshot tests; `.lbrn2` MOPA sample imports with correct layers and orientation | 8 | Design in-app instead of in Illustrator/Inkscape |
| **M6+ Differentiators** | Pick by demand: galvo camera alignment (LightBurn can't, 02 §20); cylinder correction / rotary; EZCAD `.ezd` import; Ruida; nesting; Pi headless + gated LAN API | per feature | months | — |

About 32 focused weeks to a credible galvo-first LightBurn alternative (M5). Full parity is multi-year (06 G9).

### M1 in detail: the MOPA vertical slice (6 weeks)

**In scope:**
- import SVG (full paths, transforms, units via usvg) and DXF (`dxf` crate), plus the prototype's `workspace.json`, `layer_settings.json`, `calibration.json` and `.ini`
- layers: Line, Fill (interval, angle, angle step, bidirectional) and FrameOnly; output and show; passes
- the JPT M7 profile with per-axis scale (1.1834 imported), mirror and pulse table
- Frame Once/Loop (contour)
- Mark with the confirm dialog and ARM token; STOP
- galvo time estimate; project save/load; undo/redo
- `ol-cli` equivalents of all of the above

**Out of scope:** G-code, editing tools beyond select/move/scale/rotate/mirror/delete, images, text, `.cor`, pause, Windows/Linux, notarisation.

**Checkpoints:**
- **M1.1 (weeks 1–2): "Rust red light."**
  - T1-exact encoder, session, emulator T6 suite, nusb transport
  - `ol detect` / `ol status` / `ol frame --loop`
  - **HIL-1** (items 1–6)
- **M1.2 (week 3): "First Rust burn."**
  - `ol-plan`, `ol-safety`, `ol mark --prototype ~/AI/OpenLaser/OpenMopa --mirror-x …`
  - `ol mark` reads the prototype's autosaved workspace, prints per-layer resolved values, frames, asks for typed ARM, then marks.
  - **HIL-2** (items 7–9).
  - This is the strangler bridge with **zero Python changes**: Luiz can keep designing in the Python UI and burn through the Rust driver three weeks before the Rust GUI exists.
- **M1.3 (weeks 4–6): "Rust GUI."**
  - the egui slice
  - **HIL-3:** Luiz runs his business card and one MOPA colour test end to end.

**Exit criteria (all required):**
1. CI green on macOS:
   - T0 (≥ 10,000 cases) and T1 byte-exact, T3, T5 code parity
   - the T6 suite, including stop ≤ 250 ms from every emulator state and a 20 s BUSY job completing
   - zero frame emission
2. Every row of the §2.7 bug table has its named regression test, and each test fails when its fix is reverted. That is checked once by mutation: revert the fix, watch the test go red.
3. Hardware stop latency ≤ 0.25 s in 10 of 10 trials in continuous framing and 10 of 10 while marking at low power. There are no USB resets in the trace and no power-cycle is needed.
4. A low-power hatch job of 30 s or more completes. The trace shows `ExecuteList` → BUSY cleared → `Fiber_SetMo(0)`, in that order.
5. The Test Dot trace starts with `JumpTo`, and the dot lands at field centre.
6. A 50 mm square from Rust matches the Python output within ±0.1 mm (calipers, same calibration).
7. kittest asserts the confirm dialog shows each emitting layer's kHz, ns, power and actual speed, and that Mark is disabled while refusals exist.
8. Luiz signs off that MOPA work no longer needs the Python app.

---

## 6. Reuse, licensing, packaging, platforms

**Licence:** MIT OR Apache-2.0 (03 §11). Luiz owns the prototype and relicenses his own ported code. The repo also carries `THIRD_PARTY_NOTICES.md`, `PROVENANCE.md`, DCO sign-off, and `cargo-deny` that denies GPL/AGPL/LGPL-static and allows MPL-2.0 (04 §16).

| What | Source | Licence | How | When |
|---|---|---|---|---|
| LMC opcodes, init/list/abort sequences, status bits | galvoplotter 0.2.0 | MIT © 2023 MeerK40t | Translate, with per-file header `// Ported from meerk40t/galvoplotter@fc35136 galvo/controller.py, MIT` | M1 |
| Cross-check of LMC behaviour; `.cor` parse/upload; raster "penbox" model; ezd later | MeerK40t `balormk/`, `extra/ezd.py` | MIT © 2021 meerk40t | Port with attribution (galvoplotter stays primary, 03 §4.2) | M1 ref, M3, M6 |
| Safety rules, hatch, DXF behaviour, G-code emitter, frame repeats | OpenMopa prototype | MIT © 2026 Luiz | Port + relicense | M1–M2 |
| `.cor` byte layouts, scanner calibration | corfile2meerk40t `SPEC.md` | MIT | Implement from spec | M3 |
| Travel optimisation | raygeo `ops/transform/optimize.rs` | MIT © 2026 Barebaric | Vendor-and-adapt, strip the allocator, add Rust tests (06 §4.3), or write our own | M2 |
| GRBL stop/stall semantics | raydriver `session.rs` | MIT | Port logic into our thread model | M4 |
| `.lbrn2` | Rayforge importer (MIT), MeerK40t `lbrn.py` (MIT; no Y flip, so beware), baseCreator `lightburn.rs` (MIT/Apache; review) | permissive | Port and consult | M5 |
| Hershey fonts | Hershey data | free use with acknowledgement | Bundle with NOTICE (06 G8) | M3/M5 |
| Crates | nusb, egui family, kurbo, usvg, dxf, zip, serde, blake3, crossbeam, tiny-skia, proptest, insta… | MIT/Apache/BSD; serialport MPL-2.0 | Dependency | — |
| **Never** | balor (GPL-3+); NicoPowers GP30 (no licence; facts only, e.g. idle `0x0220`); `clone_loader.py` firmware bytes; any EZCAD blob; clayfrocker97/OpenLaser (GPL); potrace; LaserGRBL; libusb-linked `rusb` unless a Windows need appears (06 C8); PyMuPDF; dssim-core | | | |

**Packaging (macOS):**
- **M1:** a locally built `.app` via `cargo-packager`, ad-hoc signed. That is fine for Luiz's own Mac: there is no quarantine flag on local builds.
- **M2:** Developer ID ($99/yr, D7), hardened runtime, `notarytool`, stapled universal DMG on GitHub Releases.
- **Entitlements: none.** USB needs no entitlement outside the App Sandbox (06 §0.9). Camera (M6) will add `com.apple.security.device.camera` + `NSCameraUsageDescription`.
- **Mac App Store: never**, because of the sandbox.
- **Updates:** an M2 "update available" check against GitHub Releases. Velopack once there are other users.

**Windows (M4):**
- nusb is WinUSB-only, and **no tool shares the board with EZCAD**; LightBurn swaps drivers too (06 §0.5).
- Ship `ol-driver-helper.exe`, a separate executable on unmodified libwdi (LGPL-3), with "Install WinUSB for USBLMCV4" and "Restore EZCAD driver". The restore path's mechanics are UNVERIFIED; test in M4.
- A board still bound to the vendor driver gets a guided message (the prototype's `live.py:425-445` hint logic, ported).
- PID `0x9980` gets "clone board needs its FPGA loaded by EZCAD; not supported", with no firmware handling (06 §0.6).

**Linux (M4):**
- `.deb` postinst installs the udev rule (from `scripts/99-openlaser.rules`).
- The AppImage offers a one-time `pkexec` install.
- Flatpak is deferred.

---

## 7. Risks and decisions

### 7.1 Top-10 risk register

| # | Risk | L | I | Mitigation |
|---|---|---|---|---|
| R1 | Misread LMC flow control (buffer depth, READY semantics) → overfill, timeouts, wedge | M | H | Never write without READY; configurable emulator depth; H2/H13 in HIL-1 on red light; no auto reset; traces replayed in CI |
| R2 | Rust stop slower than 0.18 s or unconfirmed | L–M | H | 20 ms slices, `cancel_all`, CI bar of 250 ms, 20 hardware trials, 500 ms watchdog banner, hardware E-stop messaging |
| R3 | Subtle encoder mismatch (ties, trunc, the `0xFFFF` vs `65536` field) → wrong power or size | M | H | T0 10k cases, T1 byte-exact, a 50 mm caliper check |
| R4 | Scope creep toward LightBurn parity before M1 ships | H | H | Hard in/out list for M1; a "parking lot" file; design tools blocked until M5 |
| R5 | AI-generated code silently "proceeds on timeout" or clamps (the prototype's own failure class) | M | H | Red-zone crates; mutation check on regression tests; no deadline-then-proceed code reachable by review rule; emulator invariants |
| R6 | egui gaps (canvas, menus, IME) force UI rework | L–M | M | M0 spike gates; UI-free `ol-app`; Tauri fallback costs weeks, not months |
| R7 | Ecosystem churn: egui every 1–2 months, nusb 0.2, wgpu lockstep | H | L–M | Exact pins; quarterly upgrade; `cargo tree -d` nightly; third-party APIs wrapped behind `ol-*` |
| R8 | Hardware unknowns (H5 `.cor`, H6–H9 encodings) limit accuracy or quality | M | M | Ship at parity with the proven Python first; `.cor` in M3; EZCAD capture if a Windows PC exists (D12) |
| R9 | Licence contamination (balor, unlicensed GP30, vendor firmware, AI recall) | L–M | H | PROVENANCE rules, executor spec rule, `cargo-deny`, no blobs, per-file headers |
| R10 | Solo bandwidth and laser availability make HIL sessions the bottleneck | M | M | Batch scripted sessions (2 h each); the emulator absorbs everything else; recorded traces become fixtures |

### 7.2 Decisions Luiz must make (with my recommendation)

| # | Decision | Recommendation |
|---|---|---|
| D1 | Rename before public release? | **Yes.** Choose by M0 exit after GitHub, crates.io and trademark checks; keep "OpenLaser" as codename until then. |
| D2 | Licence | **MIT OR Apache-2.0** |
| D3 | GUI | **egui 0.37 + egui_tiles**, confirmed by the M0 spike; Tauri only on NO-GO |
| D4 | M1 galvo-only, with GRBL export staying in Python/Rayforge until M2 | **Yes** |
| D5 | Freeze prototype features and apply the 7 safety patches now | **Yes, this week** (§8) |
| D6 | Repo | **New repo** for Rust; prototype archived read-only at M2 exit; golden generator pinned to `oracle-v1` |
| D7 | Apple Developer Program ($99/yr) | **At M2**, not before |
| D8 | Require frame-before-mark by default on galvos | **Yes** (per-machine setting, not per-job) |
| D9 | Keep typed "ARM" | **Yes**, but single-use, 60 s, fingerprint-bound |
| D10 | Speed quantisation | **Refuse** at 0 units or > 10 % error; always show actual speed |
| D11 | Limits | Keep 3000/2000/20000 in M1; derive from H2 + time estimate in M3 |
| D12 | EZCAD access | **Find the original EZCAD install's `.cor` and `markcfg` now** (no laser needed, 03 §13). If a Windows PC with EZCAD exists, schedule one USB capture session (H6–H9, H20) for M3. |
| D13 | Hardware time | Commit to 3 supervised HIL sessions of about 2 h each inside M1 |
| D14 | Rayforge | No fork; consider an upstream galvo addon/spec only after M3 |
| D15 | Clone boards | Detect `0x9980` and refuse with an explanation; never load firmware |
| D16 | Exact JPT M7 model and wattage | Look it up on the source label; needed for PRR-versus-pulse limits (06 G7) in M3 |
| D17 | Telemetry | None; local logs plus "export diagnostics" zip |

---

## 8. The Python prototype in the meantime

Freeze features now. Apply these seven patches on a `safety-fixes` branch, each with a unit test, then tag `oracle-v1`.
- **Effort:** about 1–2 days with Claude Code.
- **Before daily use:** run one red-light check (frame loop + stop) and one low-power dot.

1. **3 s wait cap** (`mopa_luiz/live.py:455-482`). Remove "return on deadline":
   - `wait_ready` raises `ConnectionAbortedError("board did not become READY")` on deadline, so it never writes into a full buffer.
   - `wait_idle` and `wait_finished` keep the 10 ms stop-event polling but take their deadline from the job: `max(30 s, 2 × estimated duration)`. The deadline action is `shutdown()` (abort) with a surfaced error, never "continue".
   - New test: the Appendix A `long_job 20` scenario shows no `Fiber_SetMo(0)` before BUSY clears.
2. **First-jump elision** (`live.py:641, 685, 732`, and the lighting paths at `:523-524` and `:577`). Before the first point of every list, call `c.set_delay_jump(c.delay_jump_short); c.list_jump(x0, y0)` (GP's `list_jump` has no equality check) instead of relying on `goto`/`dark`. In lighting lists, call `c.light_off()` first so the jump stays dark.
   - Test: `dot_test` contains `0x8001`.
3. **Confirm dialog shows Frame values** (`ui.py:2206-2222`, Job Summary `:1540-1556`). Render one row per emitting layer from the layers payload: power, kHz, ns, speed, passes. Relabel the Frame panel "Frame / Test-dot settings" and drop it from the Mark dialog.
4. **Hidden + output layers** (`layers.py:46-51` vs `:195-196`). `/api/mark` refuses with 400 "Layer X is hidden but Output is on — show it or turn Output off". Framing is unchanged.
5. **Speed quantisation.** `validate_layer_settings` computes `int(v * 0xFFFF / F / 1000)` with the effective field. It refuses when the result is 0 and also when the error is above 10 %, matching D10.
6. **ARM** (`ui.py:2151`). Send `arm: true` only from the confirm dialog's confirm button. Clear the ARM field after every job and after 60 s.
7. **Orphan objects** (`layers.py:175-177`). Refuse objects whose `layer_id` matches no layer instead of dropping them.

Also pin `galvoplotter==0.2.0` in `pyproject.toml` (05 §6).

**From M1.2:** prefer `ol mark --prototype …` for long or unattended-feeling jobs, since the Rust driver is the safer back end. Keep the Python UI as the design front end until M1.3.

**At M2 exit:** archive the repo with a README pointer to the successor and keep `oracle-v1` for golden regeneration.

---

### Appendix: where the reports disagree, and what this proposal follows

| Topic | Disagreement | Followed | Why |
|---|---|---|---|
| Driver concurrency | 03 §4.5 `tokio::select!`; 04 §8 OS thread; 05 "session task" | **04/06 C7**: OS thread, nusb `submit` + ≤20 ms waits | Single owner, no async cancellation subtleties, no runtime |
| Docking crate | 04 egui_dock or egui_tiles | **06 C9**: egui_tiles 0.18 | egui_dock 0.21.1 needs egui 0.36 |
| raygeo | 03/04: git dependency; 01/06: vendor | **06 C3**: vendor selected files only, and only if needed | Global allocator, PyO3-only CI |
| USB crate | 05: rusb or nusb | **nusb** (06 C8) | Pure Rust, no LGPL libusb |
| Units per mm | `0xFFFF/F` vs `65536/scale` | `0xFFFF/F` for coordinates (06 C11) | Byte parity with proven bytes |
| Golden sequencing | 05: py-compat mode | Encoder byte-exact + session behaviour tests | Avoids maintaining a deliberately unsafe mode |
| Spike gates | 04: muda menu is go/no-go | Canvas, stop latency, version lockstep, kittest are gates; muda is not | A native menu is not MVP-critical |
| T0 scope | 02 T0 includes time estimate, travel ordering, device delays, checklist | M1 = galvo slice + estimate + checklist; ordering/delays in M2/M3 | Earliest safe real use first |
| Path/point limits | 04 keep; 05 derive | Keep in M1, derive in M3 | Parity first, then evidence from H2 |
| `.lbrn2` Y axis | 02 open | **Y-up** (06 §0.7), confirm on a 2nd fixture | Verified on `fire_flower.lbrn2` |
| Windows coexistence | 03 hoped LightBurn coexists | **No tool coexists** (06 C12) | LightBurn swaps drivers too |
