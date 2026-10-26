# Proposal C: Strangler-fig + ecosystem (engine first, risk first)

*Date: 2026-10-23. Angle C of the OpenLaser Rust plan. Evidence base: research reports 01–06 and `rayforge-test/REPORT.md`, cited as "05 §1.10", "06 C7" and so on. "OM" means the Python prototype at `~/AI/OpenLaser/OpenMopa` @ `6aed8ff`. Nothing was run against hardware and nothing was installed while writing this.*

---

## 0. Strategy and stances

**Strategy.** The risky part of OpenLaser is not the editor. It is the code that turns geometry into laser emission: the LMC protocol, flow control, the stop path and the safety gate. That code is also where the working Python prototype is weakest today (3 s wait caps, elided first jump, wrong confirm values, unframed hidden layers, speeds that encode to 0; 05 §0, §2). So build that first, in Rust, as UI-free library crates with a CLI. Prove them byte-for-byte against the Python stack, which has been validated on hardware. Then put them **under the prototype's existing web UI through PyO3**, so Luiz's daily tool gets the Rust engine within about 8 weeks without waiting for a new editor. That is the strangler fig: the old app keeps working while its insides are replaced, one validated piece at a time. Only after the engine has survived real jobs does a native **egui** editor grow around the same crates and retire the prototype. Every milestone ends with a supervised session on the JPT M7, with USB traces recorded.

**Rayforge: interoperate and contribute; do not fork, depend on it or compete on gantries in year one.**
- Do not fork. It is a single-maintainer project moving at about 1,100 commits per quarter (01 §7, §9).
- Do not depend on raygeo or raydriver. raygeo installs an unconditional mimalloc `#[global_allocator]` and is built only through maturin. Instead, **vendor selected MIT files**, starting with `ops/transform/optimize.rs` (M5), later `nest2d` and `tabs`. Strip the allocator, pin the commit, add Rust tests (06 §0.4, C3).
- Write our own synchronous GRBL session. Use raydriver's cancel ordering (01 §8.1 item 6) as the reference, so every driver follows one threading model (06 C7).
- Interoperate through `.lbrn2` import (Y-up, 06 §0.7) and `.lbdev` import.
- Contribute downstream. Our PyO3 wheel `openlaser_lmc` can back a Rayforge LMC driver addon. nusb is pure Rust, so the wheel brings **no libusb**, which removes one of the two upstream blockers 01 §3.2 found. This is an optional side track after M3, and only if the maintainer engages.

**Python prototype: strangler host first, golden oracle forever.**
- A pinned copy (OM @ `6aed8ff` + galvoplotter 0.2.0) is the oracle that generates goldens (05 §5.3).
- The working copy gets safety hotfixes now (§8), the Rust engine at M2, and is archived at M5. Its oracle environment keeps running in CI after that.

**Name: rename the Rust product before the first public release.**
- `clayfrocker97/OpenLaser` is Rust, GPL-3.0 and controls fiber/CO2 lasers: same language, same domain, so it is the worst kind of collision. `openlaser.app` (MIT, 127★ in 3 days) owns the obvious domain (03 §1 item 6; 04 finding 4).
- Recommendation: **Kerfwright**. On 2026-10-23 the crates.io name returned 404, `github.com/kerfwright` returned 404, and a web search found no product by that name.
- Runner-up: "Beamwright". It is free on crates.io and GitHub, but a stage-lighting beam calculator from John McKernon Software already uses the name.
- No trademark search has been done.
- Keep `luizbueno3d/OpenLaser` as the prototype's repo. This document uses the working crate prefix `ol-`; it changes at M0, before anything is published.

**Where the reports disagree, this proposal follows:**
- 06 C7 (a dedicated OS thread per machine) over 03 §4.5 (`tokio::select!`).
- nusb over rusb (06 C8).
- egui_tiles over egui_dock (06 C9).
- Vendoring raygeo over using it as a git dependency (06 C3 and 01, over 03 and 04).
- `0xFFFF/F` for coordinates (06 C11).
- This document's M-numbers supersede the T-tiers in 02, the milestones in 04 and the golden tiers in 05 (06 G13). **02's T0 equals this plan's M5 exit.**

---

## 1. Architecture

### 1.1 Cargo workspace (dependency direction is strictly downward)

```
L5  frontends   ol-cli   ol-py (PyO3)   ol-app → ol-ui-egui   ol-daemon (M7)
L4  service     ol-engine  (session threads, ARM permits, interlocks, event stream)
L3  machines    ol-lmc   ol-gcode   ol-transport   ol-sim
L2  planning    ol-plan   ol-safety   ol-io
L1  data        ol-model   ol-ir
L0  math        ol-geom
```

| Crate | Responsibility (one line) | Key deps |
|---|---|---|
| `ol-geom` | Polylines/Bézier paths, affine transforms, bbox, flatten, join/close/dedupe, R-tree index; home of vendored raygeo files | kurbo, rstar, (i_overlay, cavalier_contours later) |
| `ol-model` | Document, objects, layers/cut settings, machine profiles, versioned serde format and migrations, legacy (OM JSON/INI) import | serde, serde_json, zip, toml |
| `ol-ir` | The job IR: `EmitJob`, `LightJob`, ops, source map; no machine units | ol-geom |
| `ol-plan` | Document + profile → IR: hatch (byte-compatible port of `raster.py`), passes, ordering, frame derivation, time estimate | ol-model, ol-ir |
| `ol-safety` | Pure whole-job validator → `ValidatedJob` or `Vec<Refusal>`; ARM permit issuance; confirm summary | ol-ir, ol-model |
| `ol-io` | SVG (usvg), DXF (dxf), `.lbrn2` (quick-xml), SVG export | ol-model |
| `ol-lmc` | LMC protocol: numeric encoders, packet builder, galvo field map, session state machine generic over `Transport`, disassembler | ol-ir, ol-safety (no USB crate) |
| `ol-gcode` | GRBL encoder (byte-compatible with `gcode.py`), sync streaming session generic over `Transport` | ol-ir, ol-safety |
| `ol-transport` | `Transport` trait; nusb USB bulk, serialport, TCP; `TraceTransport` (timestamped recording) and `ReplayTransport` | nusb, serialport |
| `ol-sim` | `FakeLmc` and `FakeGrbl` scripted devices with a virtual clock, used by tests, `--sim` and the app's demo machine | ol-transport |
| `ol-engine` | One session thread per machine, job lifecycle, stop path, watchdog, interlocks, `MachineHandle`, `Event` stream | all of the above |
| `ol-cli` | `detect / frame / mark / stop / encode --dump / golden / hw <test>` | ol-engine |
| `ol-py` | PyO3 abi3 wheel `openlaser_core` (and `openlaser_lmc` for Rayforge) | pyo3 |
| `ol-app` | AppState, `Command`/`Event` enums, view-models; **no egui dependency** | ol-engine |
| `ol-ui-egui` | eframe shell, egui_tiles docking, canvas, menus | egui 0.37, egui_tiles 0.18 |
| `ol-daemon` | (M7) axum localhost/LAN server: headless Pi, LAN remote, Tauri Plan B | axum, tokio |

**Rules.**
- `#![forbid(unsafe_code)]` in every crate we write.
- Nothing below L5 depends on a UI or on Python.
- `ol-lmc` and `ol-gcode` never name a USB or serial crate. That is what makes the fake board and replay tests exact.
- `ol-safety` never depends on a driver. Drivers depend on `ol-safety` and re-assert its invariants (defence in depth).
- `cargo-deny` gates licences (§5).

### 1.2 Core data model

The layer model mirrors LightBurn's REST schema. Each cut setting carries shared parameters plus **both** a `galvo` and a `gantry` block, so one project can retarget between the MOPA and the GRBL machines and `.lbrn2` import stays lossless (02 §7.7).

```rust
pub struct Document { pub format_version: u32, pub objects: Vec<Object>, pub layers: Vec<Layer>,
                      pub machine: Option<MachineId>, pub meta: Meta }
pub struct Object  { pub id: ObjectId, pub name: String, pub layer: LayerId,
                     pub geometry: Geometry, pub transform: kurbo::Affine, pub locked: bool }
pub enum Geometry  { Paths(Vec<kurbo::BezPath>), Image(ImageRef), Text(TextSpec) }

pub struct Layer { pub id: LayerId, pub name: String, pub color: Rgb, pub kind: LayerKind, // Cut | Tool
                   pub output: bool, pub show: bool, pub global_passes: u16,
                   pub subs: Vec<CutSetting> }                  // 1..=11 sub-layers (02 §7.6)
pub struct CutSetting { pub mode: Mode,                         // Line | Fill | OffsetFill | Image
                        pub shared: Shared, pub fill: Fill, pub galvo: GalvoCut, pub gantry: GantryCut }
pub struct Shared  { pub speed_mm_s: f64, pub power_max_pct: f64, pub power_min_pct: f64, pub passes: u16,
                     pub freq_hz: Option<u32>, pub q_pulse_ns: Option<u32>, pub air_assist: bool }
pub struct GalvoCut { pub override_timings: bool, pub laser_on_us: i32, pub laser_off_us: i32,
                      pub polygon_us: i32, pub end_us: i32, pub jump_speed_mm_s: f64,
                      pub min_jump_delay_us: u32, pub max_jump_delay_us: u32, pub jump_dist_limit_mm: f64 }

pub struct MachineProfile { pub id: MachineId, pub name: String, pub kinematics: Kinematics,
                            pub source: LaserSource, pub limits: Limits, pub layer_defaults: Vec<CutSetting> }
pub enum Kinematics { Galvo(GalvoBlock), Gantry(GantryBlock) }
pub struct GalvoBlock { pub field_mm: f64, pub field_offset_mm: [f64; 2], pub axis: AxisMap,  // swap, negate x/y
                        pub scale_correction: [f64; 2], pub cor: Option<CorTable>, pub red_dot: RedDot,
                        pub mirror: [bool; 2], pub delays: DelayDefaults, pub io: IoMap, pub usb: UsbMatch }
pub struct GantryBlock { pub bed_mm: [f64; 2], pub origin: Origin, pub dialect: GrblDialect, // M3|M4, S-max, M7|M8
                         pub max_speed_mm_s: f64, pub link: GantryLink }                // Serial | Tcp | ExportOnly
pub enum LaserSource { Fiber { mopa: bool, pulse_table_ns: Vec<u32>, prr: Option<PrrTable> },
                       Co2, Diode, Pen }
```

Decisions this encodes:
- **Units.** Frequency is integer Hz, as `.lbrn2` stores it; pulse width is integer ns.
- **Layer defaults are per machine kind.** Rayforge's test showed MOPA defaults (300 mm/s, 100 %) leaking into a diode job (REPORT takeaway 1).
- **Mirror and calibration belong to the machine profile.** Today they live in browser `localStorage` and a global `calibration.json` (05 §1.7, §4.3).
- **Limits are data, not constants.** The prototype's 3000/2000/20000 caps carry over as defaults (05 §2, S4).

### 1.3 Job pipeline and IR

```
Document ─ol-plan─▶ EmitJob / LightJob (mm, IR) ─ol-safety─▶ ValidatedJob
   ─machine transform─▶ MachineJob<GalvoPt | GantryPt> ─encoder─▶ EncodedJob ─ArmPermit─▶ ArmedJob ─▶ session thread
```

```rust
pub struct EmitJob  { pub sections: Vec<Section>, pub map: SourceMap, pub est: Estimate }
pub struct Section  { pub layer: LayerId, pub sub: u8, pub pass: u16, pub params: EmitParams, pub ops: Vec<Op> }
pub enum Op         { Travel(P2), Burn(P2), Dwell { us: u32 }, Mark(Marker) }   // Marker: PathStart(ObjectId) …
pub struct EmitParams { pub power_pct: f64, pub speed_mm_s: f64, pub freq_hz: Option<u32>,
                        pub q_pulse_ns: Option<u32>, pub timings: Option<GalvoTimings>, pub air: bool }

pub struct LightJob { pub ops: Vec<LightOp>, pub speed_mm_s: f64 }     // red light only
pub enum LightOp    { Dark(P2), Lit(P2) }                              // there is no Burn variant

pub enum EncodedJob { Lmc(EncodedLmc), Gcode(GcodeProgram) }           // typed, never lossy (01 §8.2 item 4)
pub struct EncodedLmc { pub prologue: Vec<Single>, pub packets: Vec<Packet>,   // Packet = Box<[u8; 3072]>
                        pub epilogue: Vec<Single>, pub est_exec: Vec<Duration>, pub map: PacketMap }
```

**Invariants that hold by construction.**
- **Lighting can never emit.** `LightOp` has no burn variant. `encode_light` can only produce `JumpTo`, `WritePort(0x0100/0)`, `JumpSpeed` and `ReadyMark`. A test disassembles every light stream and fails if it finds `MarkTo`, `LaserOnPoint` or `Fiber_SetMo(1)` (05 §1.6).
- **Frame what you burn.** The frame `LightJob` is derived from the `ValidatedJob`'s emit geometry, not from the visible layers. That removes the hidden-layer gap (G3) structurally.
- **Refuse out-of-field points; never clamp.** `GalvoMap::map(P2) -> Result<GalvoPt, OutOfField>` applies, in order: scale correction, axis swap/negate, mirror, offset, then `round_ties_even(0x8000 + v·0xFFFF/F)`. Applying `.cor` correction in software is UNVERIFIED; the board does it once the table is uploaded (M3).
- **Every LMC job begins with an explicit `JumpTo`.** This fixes G4.
- **A speed that encodes below 1 unit is refused.** The minimum is about 3.61 mm/s at F = 236.68 (G5). This is a refusal, not `MarkSpeed(0)`.
- **Numeric parity with Python.** `round_ties_even` is used wherever Python calls `round()`, and `trunc` wherever it calls `int()` (05 §0.6). A delay of 0 keeps the `0x8000` sign word until H9 says otherwise.
- **Python sequencing is test-only.** A `py-compat` Cargo feature (test builds only, never in shipping binaries) reproduces galvoplotter's sequencing, including its parameter cache and the elided first jump, so encoders can be proven against the exact Python bytes.

**Sequencing, risk first.** Through M2 an emit job is streamed as **one list per layer-pass**. That is the shape proven on hardware, but now inside one session: no per-pass `Reset`, abort or USB reset (05 §0.2). M3 moves to one continuous list with mid-list parameter changes, but only after hardware item H21 (§3.4) shows that pulse-width and current changes take effect mid-list on the M7.

### 1.4 Driver and Transport traits

```rust
pub trait Transport: Send {
    /// Bulk/serial write. Implemented as submit + wait_next_complete(≤20 ms) slices that re-check
    /// `cancel` (never nusb's transfer_blocking on the stop-critical path, 06 §2 item 6).
    fn write(&mut self, data: &[u8], deadline: Duration, cancel: &Cancel) -> Result<(), TxError>;
    fn read(&mut self, buf: &mut [u8], deadline: Duration, cancel: &Cancel) -> Result<usize, TxError>;
    fn cancel_pending(&mut self) -> Result<(), TxError>;          // Endpoint::cancel_all + drain
    fn info(&self) -> TransportInfo;                               // VID/PID, serial, speed, max packet
}

pub trait Driver: Send {                                           // owned by exactly one session thread
    fn capabilities(&self) -> &Capabilities;
    fn connect(&mut self, t: Box<dyn Transport>) -> Result<DeviceInfo, DriverError>;
    fn frame(&mut self, job: &LightJob, mode: FrameMode, cx: &RunCtx) -> Result<RunOutcome, DriverError>;
    fn run(&mut self, job: &ArmedJob, cx: &RunCtx) -> Result<RunOutcome, DriverError>;
    fn pause(&mut self, cx: &RunCtx) -> Result<(), DriverError>;  // Err(Unsupported) until H15
    fn resume(&mut self, cx: &RunCtx) -> Result<(), DriverError>;
    fn emergency_stop(&mut self) -> StopReport;                    // infallible signature; faults are inside
    fn status(&mut self) -> Result<Status, DriverError>;           // a failed read is Err, never "all bits set"
    fn read_inputs(&mut self) -> Result<Inputs, DriverError>;      // door/foot (H11)
}
pub struct Capabilities { pub kinematics: KinematicsKind, pub red_light: bool, pub pause: PauseSupport,
                          pub granular_progress: bool, pub stop_budget: Duration, pub inputs: InputCaps }
pub struct Status { pub raw: u16, pub ready: bool, pub busy: bool, pub axis: bool, pub unknown: u16 } // 0x0220 ok

#[derive(Clone)]
pub struct MachineHandle { /* cmd: Sender<Cmd>, prio: Sender<Prio>, stop: Arc<StopFlag>, events: Bus */ }
impl MachineHandle {
    pub fn stop(&self) -> StopTicket;                              // sets the atomic, wakes the thread; < 1 ms; any thread
    pub fn frame(&self, j: LightJob, m: FrameMode) -> Result<JobTicket, EngineError>;
    pub fn arm(&self, v: &ValidatedJob, typed: &str, framed: Option<FrameReceipt>) -> Result<ArmPermit, Refusal>;
    pub fn run(&self, v: ValidatedJob, p: ArmPermit) -> Result<JobTicket, EngineError>;
    pub fn subscribe(&self) -> Receiver<Event>;                    // Status, Progress, Stopped, Fault, Done, Interlock
}
```

This deliberately avoids Rayforge's GRBL-shaped abstract base class, which forces about 10 stub methods on non-gantry drivers (01 §8.2 item 3). Homing, jogging, WCS and console become optional capability traits (`Homes`, `Jogs`, `HasConsole`) implemented by the GRBL driver only.

**LMC stop budget.** Target p99 ≤ 250 ms, end to end; the prototype measured 0.18 s after its fix (05 §1.9).

| Step | Budget |
|---|---|
| `MachineHandle::stop()` sets the flag (any thread) | < 1 ms |
| Session thread notices it (it waits in ≤ 20 ms slices) | ≤ 20 ms |
| Unsent packets dropped; `cancel_pending()` and drain | ≤ 30 ms |
| `StopExecute` → `Fiber_SetMo(0)` → `ResetList` → dummy EOL packet + `SetEndOfList` + `ExecuteList` → `Fiber_SetMo(0)` → `WritePort(0)`, sent unconditionally after a cancel because the board may hold a partial packet (06 C16) | ~6 singles + 1 packet, nominally < 50 ms |
| `StopReport` emitted | `latency` = request time to the acknowledged `WritePort(0)` |

If any stop write fails, the session enters `Faulted(StopUnconfirmed)`. The UI then shows "laser state unknown: switch off the laser source". There is no USB reset and no retry storm (05 §1.10).

**GRBL stop.** The realtime bytes `!` then `0x18` are written straight to the port from the session thread, bypassing the line queue. The thread then waits for the reset banner, sends `M5`, and marks the job failed (05 §3.3).

### 1.5 Threading and the real-time stop path

- **One OS thread per connected machine** owns the `Driver` and the `Transport`. It loops: check the priority channel, then the stop flag, then do at most one bounded transfer. It uses crossbeam `select_biased!` with the control channel ahead of the job channel. There is no tokio in the core; tokio arrives only with `ol-daemon` and future network drivers behind the same trait (06 C7).
- **All CPU work happens before arming.** Planning, encoding and hashing run on rayon, so the session thread only streams pre-built packets.
- **Flow control.** A packet is written only if the latest status read had READY set. While waiting, the thread polls `GetVersion` every 10 ms, checks the stop flag, and **never proceeds on a timeout** (05 §0.1). The end-of-job MO close and port-off wait for `!BUSY`. That wait is unbounded but stoppable.
- **Watchdog.** It fires if no progress is seen past `1.5 × Σ est_exec(queued) + 2 s`. Its only action is a stop plus a fault. What counts as progress depends on H2 (READY toggles, list status or position).
- **Interlocks.** If configured, `read_inputs()` runs between packets and in every wait loop. An open door means stop. This is additional protection, never primary (06 G12).
- **Frontends.**
  - The egui UI thread only sends commands and reads events.
  - PyO3 methods release the GIL (`py.allow_threads`). `stop()` never blocks, so a `ThreadingHTTPServer` request thread in the prototype can stop a mark running on another thread (`ui.py:2996`).
  - Panics are caught at the FFI boundary and become Python exceptions. A panic inside the session thread triggers `emergency_stop()` from a drop guard before the thread dies.

### 1.6 Safety model: hard refusals, and where they live

1. **`ol-safety::validate`** is pure, covers the whole job, and runs before any USB traffic. It returns typed `Refusal` codes, never strings, so they can be compared against the Python decisions (05 §5.3, T5). Checks:
   - S1–S8 from 05 §2.
   - G5: minimum encodable speed.
   - G7: an object whose layer does not exist is a refusal, not a silent drop.
   - G10: malformed points are a refusal.
   - Non-finite numbers.
   - Path, point and estimated-time limits.
   - The pulse-width table.
   - Frequency bounds, and the JPT PRR limits once known (a warning until H8 is answered; 06 G7 says invalid Q-pulse values are snapped by the source).
2. **ARM permit** (fixes G1 and G2).
   - `arm()` accepts a typed "ARM" only for the hash of a specific `ValidatedJob`.
   - The permit is single-use and expires after 60 s.
   - The confirm dialog text is **generated by the engine** from the validated job: per-layer power, frequency, Q-pulse, speed and passes, estimated emission time, bounding box, hidden-but-output layers, and the machine identity. The UI cannot show the wrong parameters because it doesn't compose them.
   - "Require framing before start" is on by default for galvos. The permit then needs a `FrameReceipt` for the same job hash from this session within 10 minutes.
3. **Type gates.** `ArmedJob` has a private constructor in `ol-safety`. Drivers accept only `ArmedJob` for `run` and only `LightJob` for `frame`. **No API offers "proceed anyway"**, unlike Rayforge's advisory dialog (01 §8.2 item 1).
4. **Driver invariants**, re-asserted at stream time:
   - an explicit first jump
   - no write unless READY
   - MO opened only inside an emit run
   - the stop sequence is unconditional
5. **Sticky faults, non-sticky stop.** A user stop clears at the next frame or arm (S11). A USB error or unconfirmed stop latches `Faulted` until the operator reconnects and acknowledges it. There is no automatic reconnect mid-job, and no automatic homing on connect (Rayforge sent `$H` unprompted; REPORT).
6. **Posture.** The docs and the first-run screen say OpenLaser is not a safety device and never overrides hardware interlocks. Regulatory obligations are UNVERIFIED (06 G12).

---

## 2. UI approach

**Decision: an egui/eframe 0.37 native editor with egui_tiles 0.18 docking, behind an egui-free `ol-app`. Not Tauri.** The legacy web UI stays alive on the Rust engine through PyO3 until M5. After that it survives only as a control-panel page served by `ol-daemon` (M7).

**Why not Tauri, even though the strangler angle favours reuse.**
- 04 §2.3 shows Tauri overtakes egui only when "reuse the prototype's web UI" is a criterion. The reuse is real, but it is captured by M2 without Tauri. The prototype is already a browser UI over a local server, and it gets the Rust engine underneath it.
- What would be left to "reuse" in a Tauri app is a 900×900 `<canvas>` polyline viewer inside a 3,000-line Python string (`ui.py:155-2470`). A LightBurn-class editor (node editing, booleans, text, 20k+ paths) has to be written anyway.
- In egui, that editor's hit-testing and snapping call the same `ol-geom` code as the planner. In a webview they would be duplicated in JS or pushed through IPC or Wasm.
- Graphite left Tauri for canvas reasons (04 finding 5).
- Linux matters because LightBurn dropped it (02 §0 item 2), and WebKitGTK is the weakest webview there.

**Canvas strategy.**
- **Phase 1:** egui `Painter` with per-object cached flattened polylines in world mm, re-flattened per zoom bucket, rstar culling and hit-testing, and overlays (handles, rubber band) drawn by egui.
- **Phase 2**, when profiling shows painter cost dominating above about 50k segments: a custom wgpu instanced "hairline" pipeline through `egui_wgpu::Callback`, plus lyon meshes for fills and mipmapped textures for images (04 §3).
- A `CanvasRenderer` trait also has a tiny-skia backend for deterministic CI image goldens.
- Pin one wgpu major, the one egui uses.

**De-risking: a 2-week spike at the start of M4.** Each metric is measured and recorded in `SPIKE.md`.

| # | Metric | Gate |
|---|---|---|
| 1 | 20k paths / 200k segments: pan/zoom p95 frame time ≤ 16.7 ms on the M2 Air (Retina); hit-test ≤ 2 ms | **Hard**. Failure → Plan B |
| 2 | **Stop-key latency** (Esc and the on-screen Stop) under metric 1's load: keypress to `MachineHandle::stop()` ≤ 50 ms p99 | **Hard**. Failure → Plan B |
| 3 | Text field with IME (Portuguese dead keys, Japanese) in the properties panel | Hard |
| 4 | egui_tiles layout persisted with serde; every egui-ecosystem crate on one egui major (06 §4 item 2) | Hard |
| 5 | muda native menu bar with Cmd shortcuts in eframe 0.37 on macOS 26 | **Soft**: fall back to an in-window menu bar |
| 6 | VoiceOver reaches the layer list and the Stop button; `egui_kittest` snapshot runs in CI | Soft (fix in M5) |
| 7 | Release binary ≤ 40 MB; cold start ≤ 1.5 s | Soft |

04 §2.4 treats a muda failure as a reason for Plan B. This proposal makes it soft: LightBurn is not native-looking either, and an in-window menu costs polish, not safety. The stop-latency gate is new, because immediate mode means a slow frame delays input.

**Plan B (if a hard gate fails):** pull `ol-daemon` forward. Serve a rewritten TypeScript/WebGL editor in Tauri 2 over the same `ol-app` `Command`/`Event` types, exported with `ts-rs`. Because the engine and app layer are UI-free, Plan B costs about 4 extra weeks, not a rewrite.

---

## 3. Testing and verification

### 3.1 Golden oracle

- `oracle/` in the Rust repo pins Python 3.10, OM @ `6aed8ff` and **galvoplotter==0.2.0**, plus the 05 Appendix A capture harness, extended.
- The harness makes `usb.core.find` and `USBConnection` raise, uses a temporary `CALIBRATION_PATH`, sets `OPENMOPA_MOCK=1`, and masks timestamps (05 §5.3).
- `just goldens` regenerates `tests/golden/*.json`. Each file records the generator version, OM SHA and GP version. Regeneration is never automatic in CI; CI only checks.

| Tier (05 §5.3) | Content | Rust must match |
|---|---|---|
| T0 | `mm_to_galvo`, power→current, frequency→period, speed, delay words, pulse snap, distance word; ≥ 10k seeded cases each, plus every tie (30 %→1228, 70 %→2866) and edge (±F/2, speed < 1 unit) | Exactly |
| T1 | Init sequence; `mark_box10` (packet sha256 `9ca1e12d…59f5`); `frame_box10`; `dot_test`; `long_job` (9 packets); business card per layer at F = 236.68; abort dummy packet (`e479558e…6a18`) | Byte-exact in `py-compat`; `rust-v1` goldens frozen after review |
| T2 | `compile_gcode` for 4 profiles × {vector, multi-pass, air, raster, plotter}, line 3 masked | Byte-exact; 100k random values for `{:.3f}` formatting |
| T3 | `hatch_polylines` | Segment count and order exact; coordinates within 1e-9 mm |
| T4 | DXF/SVG/STL importer fixtures | Point count exact; coordinates within 1e-9 |
| T5 | Safety decisions, including the §8 hotfix fixtures | Same decision and same refusal category |

`DIVERGENCES.md` lists every intentional `rust-v1` difference:
- the explicit first jump
- one session per connection (no per-job init, `Reset`, abort or USB reset)
- refusing speeds below 1 unit
- one list per job after H21
- the single-use permit

Luiz signs this list off (D12).

### 3.2 Emulators

**`FakeLmc`** runs on a virtual clock and can be scripted with:
- buffer depth N (from H2)
- READY drop and return
- BUSY for the encoder's estimated time
- a stalled write (NAK until timeout)
- an unresponsive board
- a partial packet after a cancel
- status words with unknown bits (`0x0220`)
- PID `0x9980` (clone board needing firmware: detected and explained, never loaded)

It records every transfer, and the `ol-lmc` disassembler turns recordings into readable listings.

**`FakeGrbl`** is a Rust port of `rayforge-test/fake_grbl.py`: a 15-block planner, realtime bytes, `$` commands, alarms. The Python original also runs in CI as a cross-check.

**T6 timing properties** run on FakeLmc, 1,000 randomised runs per PR:
- (a) Stop ≤ 250 ms (virtual time) from each of: streaming, waiting on READY with a full buffer, waiting on BUSY, continuous framing, and connecting.
- (b) Zero writes while not READY.
- (c) No MO close or `StopExecute` while BUSY unless a stop was requested. The 20 s-BUSY `long_job` therefore completes, which is the regression test for the prototype's 3 s cap.
- (d) The watchdog fires only without progress.

One real-time test (2 ms simulated transfer latency) checks the 250 ms budget on the CI clock.

### 3.3 Property and fuzz tests

**proptest:**
- LMC encode → disassemble round trip.
- Every emit point maps inside the field, or the job is refused; nothing is ever clamped.
- Light streams contain no emission opcodes.
- `GalvoMap` is monotonic per axis.
- Ordering visits every path exactly once.
- Hatches stay inside their even-odd regions.
- G-code never has `S` above S-max and never burns outside the bed.

**cargo-fuzz** targets the SVG, DXF and `.lbrn2` importers and the PyO3 job-payload parser.

### 3.4 Hardware-in-the-loop protocol

**Rules.**
- Luiz runs every hardware command himself. AI agents never do.
- Wear goggles, keep the enclosure closed, use scrap plate.
- Use the red light before any emission.
- The first emission in a session is at ≤ 10 % power.
- Every transfer is logged by `TraceTransport` to `hw-logs/<date>/`.
- Results are recorded per item in `HARDWARE.md`.
- `ol hw <test>` scripts run each procedure from 05 §7.

The order below is by risk *to trusting the Rust driver*. This differs from 05's ordering: H1 is a prototype bug whose Rust fix is proven in simulation, whereas H2, H13 and H14 define the Rust driver's own assumptions.

| Session | Milestone | Items (in order) | Emission |
|---|---|---|---|
| HW-1 | M1 | **H2** READY/BUSY and buffer depth → **H13** write into a full buffer → **H14** minimal abort and its latency → **H3** mirror position after Reset/abort → **H12** descriptors → **H1** long job past 6 s → **H4** axis orientation ("F" plus arrow) | Red light only |
| HW-2 | M1 exit | First Rust mark (10 mm box, then the business-card vector layer) side by side with Python marks; dot test; **H19** MO-open settle | ≤ 10 %, then working power |
| HW-3 | M3 | **H11** ports and door/foot inputs → **H5** `.cor` versus blank → **H6** pulse-width encoding → **H8** frequency and PRR limits → **H7** power map → **H9** delay units → **H10** low-speed quantisation → **H21** (new) do mid-list current and Q-pulse changes take effect at the next `MarkTo`? → **H22** (new) `GetPositionXY` as a progress signal | Low power, scrap |
| HW-4 | M5–M7 | **H15** pause/resume → **H16** `GetMarkTime`/`GetListStatus` → **H20** laser mode → **H18** Windows WinUSB and Linux udev | Red light, then low power |
| never | — | **H17**: wedge recovery is observed only if a wedge happens naturally | — |

H6–H9 and H20 are best answered by capturing EZCAD's USB traffic (USBPcap) on a Windows PC. This depends on decision D6.

### 3.5 CI matrix

| Runner | From | Jobs |
|---|---|---|
| macOS 26 arm64 | M0 | fmt, clippy `-D warnings`, nextest (unit, golden, T6), insta, cargo-deny, maturin abi3 wheel + **the prototype's test suite (119+) run against the wheel** (M2), egui_kittest (M4), notarisation dry run (M5) |
| Ubuntu 24.04 x86_64 | M0 | Same minus macOS packaging; nightly cargo-fuzz (10 min per target) and a 100k-case proptest run |
| Windows x86_64 | M1 | Build + unit/golden/T6 tests; packaging from M7 |
| Linux aarch64 (Pi) | M7 | Build + tests for headless `ol-daemon` |

- `rust-toolchain.toml` pins the stable release installed at M0 (≥ 1.95 for egui 0.37). Edition 2024.
- Dependencies are pinned exactly in `[workspace.dependencies]`, with a quarterly upgrade window (04 §15 item 5).

---

## 4. Roadmap

Effort is in **focused weeks** for Luiz with Claude Code planning and reviewing, and Codex/OpenCode executing through `/route-implementation`. Everything under `ol-lmc`, `ol-safety` and `ol-engine` is safety-critical. Claude writes the spec, executors write code against goldens, and no change to the stop path merges without a T6 run plus a HIL re-run at the next session.

| M | Name | Weeks | Ships to the machine |
|---|---|---|---|
| M0 | Foundations + prototype hotfix | 2 | Safer Python app |
| **M1** | **Rust drives the MOPA from the CLI** | **4 (+1 buffer)** | Rust frame/mark/stop |
| M2 | Python UI on the Rust engine (PyO3) | 3 | Daily driver on Rust |
| M3 | Galvo correctness and calibration | 4 | `.cor`, delays, test grids |
| R | (optional) Rayforge LMC addon | 2–3 | — |
| M4 | egui spike + native skeleton | 5 | First native mark |
| M5 | Native app replaces the prototype (= 02's T0) | 8 | Notarised daily driver |
| M6 | T1 slice: images, text, GRBL live, `.lbrn2` | 10–12 | Photo/MOPA raster, GRBL streaming |
| M7 | Windows/Linux, headless Pi daemon | 5 | Other OSes |

M0 to M5 is about **26 focused weeks**. Parity with LightBurn is a multi-year effort of 100k+ lines (06 G9) and is explicitly not promised.

### M0: Foundations and prototype hotfix (2 weeks)

**Scope:**
- Luiz installs `rustup`, which is absent today (06 G14).
- Workspace skeleton and CI, `cargo-deny`, `PROVENANCE.md`, `THIRD_PARTY_NOTICES.md`.
- Name and licence decisions (D1, D2).
- `oracle/` environment and capture harness; T0–T2 goldens committed.
- The Python safety hotfix release (§8).

**Exit:**
- Empty-crate CI green on macOS and Linux.
- Goldens committed with metadata; the `mark_box10` sha matches 05.
- The prototype's 119 tests plus the new hotfix tests pass.
- Hotfix tag pushed.

**Luiz can:** keep using the same app, without the five known safety bugs.

### M1: Rust drives the MOPA from the CLI (4 weeks + 1 buffer), tightly defined

**In scope:**
- `ol-geom` (polylines only), `ol-ir`, `ol-safety` (S1–S8 plus G1, G3, G4, G5, G7, G10), `ol-lmc` (numeric encoders, packet builder, init sequence 05 §1.4, mark, light, abort, `py-compat`, disassembler), `ol-transport` (nusb + Trace/Replay), `ol-sim::FakeLmc`, and a minimal `ol-engine` (one session, stop path, watchdog, events).
- `ol-cli` commands:
  - `ol detect`
  - `ol frame JOB.json [--loop]`
  - `ol mark JOB.json`: prints the engine-generated confirm summary, then takes the typed ARM; Esc, Space or Ctrl-C triggers stop.
  - `ol encode JOB.json --dump`
  - `ol golden check`
  - `ol hw <H-item>`
- **Job input** is the prototype's own plan payload (objects + layers + `machine_id`, 05 §4.4). It comes from a ~30-line "Export job JSON" button added to the Python UI. Luiz keeps designing in the old UI and marks from the Rust CLI.
- Init values identical to galvoplotter's. One list per layer-pass, inside one session.

**Out of scope:** PyO3, `.cor`, G-code, images, any GUI, Windows hardware.

**Exit criteria (all observable):**
1. T0: 100 % exact on ≥ 10k seeded cases per encoder.
2. T1 in `py-compat`: byte-identical for all seven captures; `rust-v1` goldens reviewed and frozen; `DIVERGENCES.md` signed off.
3. T5: decision parity on every prototype refusal fixture, plus new refusals for G1/G3/G4/G5/G7/G10.
4. T6 green: 1,000 randomised runs, stop ≤ 250 ms from every state, zero writes while not READY, the 20 s-BUSY job completes.
5. HW-1 done, with H2, H13, H14, H3, H12, H1 and H4 answered in `HARDWARE.md`.
6. 20 hardware stops measured from trace timestamps: all ≤ 250 ms, **zero USB resets**.
7. A red-light frame lasting more than 10 s runs to completion (proves the H1 fix on the board).
8. HW-2: the Rust-marked 10 mm box matches the Python-marked one within 0.1 mm (calipers), and the dot test shows a mark at the commanded point.

**Luiz can:** design in the Python UI, export a job, then frame and mark it from Rust, with a sub-250 ms stop and no truncation of long jobs.

### M2: Strangler step 1, the Python UI on the Rust engine (3 weeks)

**Scope:**
- `ol-py` wheel `openlaser_core` (maturin, abi3, cp310+, macOS arm64 + Linux x86_64).
- `live.py`'s galvoplotter path is replaced behind `OPENLASER_ENGINE=rust|legacy`; the default becomes `rust` at exit. In rust mode there is no per-job process spawning, one engine owns USB inside the server process, and `FrameLoop` becomes engine-side continuous framing.
- `gcode.py`, `raster.hatch_polylines` and `safety.py` decisions delegate to Rust (T2, T3, T5 parity).
- The mark modal renders the engine's confirm summary; ARM uses the engine permit.
- The window title shows `engine: rust` or `engine: legacy`.

```python
eng = openlaser_core.Engine(profile_ini, calibration_json)  # opens nothing yet
plan = eng.plan(payload)              # -> {job_hash, summary, refusals[]}
permit = eng.arm(plan["job_hash"], typed="ARM")
eng.mark(plan["job_hash"], permit)    # releases the GIL; returns a StopReport if stopped
eng.stop()                            # non-blocking, callable from any request thread
```

**Exit:**
- The prototype suite passes in both engine modes in CI.
- An import-guard test proves galvo, pyusb and libusb-package are never imported in rust mode.
- Two weeks of Luiz's real jobs logged with zero fallbacks to legacy.
- Every stop logged at ≤ 250 ms.
- The business card marks identically to M1.

**Luiz can:** use the same UI on a Rust engine. The macOS installer no longer bundles libusb.

### M3: Galvo correctness and calibration (4 weeks)

**Scope:**
- HW-3.
- `.cor` V1/V2 reader and upload (`WriteCorTable(1)` + 4,225 × `WriteCorLine`, sign-magnitude; 03 §4.6). The EZCAD `.cor` file is imported from the original PC.
- Per-axis Calibrate Axis replaces the single 1.1834 factor; axis negate/swap.
- markcfg delays and jump parameters actually applied (GDEV-06/07).
- Red-dot offset/scale.
- Galvo time estimator (mark/jump lengths, speeds, delays).
- JPT PRR warnings.
- Door/foot interlock if H11 finds the inputs.
- One list per job if H21 passes.
- Material test grid generator (frequency × Q-pulse, power × speed) in the CLI and the Python UI, exposed through PyO3.

**Exit:**
- On a 100 mm grid, max error ≤ 0.1 mm over the central 80 % of the field with the `.cor` loaded (or a documented reason why not).
- Time estimate within ±10 % on 5 reference jobs.
- H5–H11 and H21 answered.
- With inputs wired, opening the door stops a running job in ≤ 250 ms.

**Luiz can:** get correctly sized marks without the magic factor, and MOPA colour test grids.

### R: Rayforge LMC addon (optional, 2–3 weeks, after M3)

Before writing code, open an issue upstream, referencing #261, with the proposed design:
- `uses_gcode=False`
- an encoder that emits a deterministic text transcript and re-encodes at `run()`, because `driver_data` is dropped by the pipeline (01 §1.4, §3.2)
- `register_driver()` called from `rayforge_init`, since `register_machines` is dead code
- refusals raised from `run()`
- maturity `EXPERIMENTAL`

Build only if the maintainer responds positively.

**Exit:** the addon frames and marks the business card from Rayforge 1.13 on Luiz's machine.

### M4: egui spike and native skeleton (5 weeks)

**Scope:**
- The 2-week spike (§2), recorded as go or Plan B.
- `ol-app` and `ol-ui-egui`: a Phase-1 canvas, layers panel, machine panel (connect, frame, engine-generated confirm, a big Stop bound to Esc/Space), full SVG import (usvg: Béziers, transforms, units) and DXF import (`dxf`).
- Project file v1 (zip + JSON + migrations), snapshot undo.

**Exit:**
- Spike metrics recorded.
- Luiz takes the business card end to end in the native app: import the DXF, assign layers, frame, confirm, mark.
- Project round-trip test passes; kittest snapshots run in CI.

### M5: Native app replaces the prototype (8 weeks; this is 02's T0)

**Scope:** the remaining 02 §19 T0 list:
- transforms, alignment, join/close/dedupe
- Line and Fill modes (interval, angle, angle increment, cross-hatch, bidirectional toggle)
- sub-layers
- reduce-travel ordering (vendored raygeo `optimize.rs` + Rust tests)
- bounds/contour/continuous framing
- preview with travel moves and time estimate
- G-code export for the three GRBL profiles with per-kind defaults
- legacy import (`workspace.json`, `layer_settings.json`, markcfg, `calibration.json`, `localStorage` mirror flags re-entered once)
- Developer-ID-signed, notarised DMG
- update check against GitHub Releases
- log/diagnostics export

**Exit:**
- The prototype-parity checklist in the repo is 100 % ticked.
- Luiz uses only the native app for 2 consecutive weeks of real jobs.
- The DMG installs on a fresh macOS user account with no `xattr` step.
- The prototype is tagged `final` and archived; the oracle keeps running.

### M6 and M7 (sketch)

**M6 (10–12 weeks):**
- Image import, own dithering (about 500 LOC, golden PNGs), galvo raster per MeerK40t's power-per-run and "penbox" model (06 §0.8).
- Hershey single-line fonts (bundled with their notice) and system fonts (parley/skrifa); QR/DataMatrix; serial-number text.
- GRBL live streaming (character counting, realtime bytes, alarms → `!` + `0x18`), tested against `FakeGrbl` and a real GRBL machine (D10).
- `.lbrn2` import (Y-up, confirmed on a second fixture); material library v1.

**M7 (5 weeks):**
- Windows: WinUSB helper built on libwdi as a separate executable, plus a documented "restore EZCAD driver" path (06 §0.5).
- Linux: AppImage and `.deb` with the udev rule.
- `ol-daemon` on a Pi, serving a trimmed legacy web UI as a LAN control panel.

**M7 exit:** red-light frame and low-power mark on Windows and Linux (H18).

---

## 5. Reuse, licensing and packaging

**Licence:** MIT OR Apache-2.0 (03 §11). Luiz owns the prototype and can relicense his own code.

| Source | Licence | Use |
|---|---|---|
| galvoplotter 0.2.0 (© 2023 MeerK40t) | MIT | Port the protocol logic and constants into `ol-lmc`, with a per-file header and an entry in THIRD_PARTY_NOTICES |
| MeerK40t `balormk` | MIT | Port the `.cor` reader, raster/penbox model and dwell chunking. **Never** `clone_loader.py`, which embeds vendor firmware (06 §0.6). Detect PID `0x9980` and explain it |
| raygeo @ pinned SHA | MIT | Vendor files into `ol-geom/src/vendored/raygeo/` with the commit and notice; strip `#[global_allocator]`; add Rust tests |
| raydriver, Rayforge `.lbrn2` importer, corfile2meerk40t SPEC | MIT | Reference implementations; port with attribution where code is translated |
| balor | GPL-3.0+ | Never opened by humans or agents; its facts arrive via MeerK40t |
| NicoPowers GP30, earlynerd | no licence | Facts only (idle status `0x0220`) |
| crates: nusb, serialport (MPL-2.0, file-level), kurbo, usvg, dxf, quick-xml, i_overlay, cavalier_contours, rstar, egui/eframe/egui_tiles, pyo3, crossbeam | permissive | Dependencies, pinned |
| denied | — | potrace, dssim-core, PyMuPDF, camera-intrinsic-calibration, lensfun; rusb/libusb unless a Windows need appears; libwdi (LGPL-3) only as a separate executable |
| Hershey font data | permissive with notice | Bundled with the NBS/Hurt acknowledgement (06 G8) |

**Provenance rule for the AI workflow.** `/route-implementation` specs name the exact MIT file and commit to port from. Executors are never given GPL or unlicensed sources in their context. Each ported file carries `// Ported from <url>@<sha>, MIT`. Captured USB traces come only from Luiz's own machine. No vendor blobs ever enter the repo.

**Packaging.**
- **macOS.** cargo-packager builds the `.app`/`.dmg`; Developer ID ($99/yr, D7); hardened runtime; notarytool and stapler. **No USB entitlement is needed outside the sandbox.** Camera support later needs only `com.apple.security.device.camera` + `NSCameraUsageDescription` (06 §0.9). No App Store.
- **PyO3 wheels (M2).** Built by maturin in CI and fetched by the prototype's existing one-step installer.
- **Updates.** A GitHub Releases check at M5; Velopack when distribution widens.
- **Windows.** WinUSB is unavoidable, since no tool shares the board with EZCAD's driver (06 C12). Provide the libwdi helper and the restore guide; sign with Azure Artifact Signing.
- **Linux.** The `.deb` post-install installs the udev rule. AppImage users get a `pkexec` helper that prints exactly what it will install.

---

## 6. Risk register (top 10)

| # | Risk | L | I | Mitigation |
|---|---|---|---|---|
| 1 | The real board's flow control or stop behaviour differs from FakeLmc (H2/H13/H14) | M | H | HW-1 runs first, red light only, fully traced. FakeLmc is re-parameterised from the traces. `py-compat` sequencing is available as a fallback mode |
| 2 | Safety regression during the strangler phase (two engines, flag confusion) | M | H | One `ol-safety` serves both. Engine mode shown in the title bar. Legacy mode removed one release after M2's exit. Python tests run against the wheel in CI |
| 3 | AI-executor code in safety-critical crates is subtly wrong, with one human reviewer | H | H | Spec plus goldens before code. Claude reviews every diff in `ol-lmc`/`ol-safety`/`ol-engine`. CODEOWNERS = Luiz. Stop-path changes require T6 plus a HIL re-run |
| 4 | PyO3 friction: GIL, panics, threads in an HTTP server process | M | M | `allow_threads`, `catch_unwind` at the boundary, a drop guard that stops on panic, no multiprocessing in rust mode, abi3 |
| 5 | The egui canvas or input latency fails the spike | M | H | Spike before investing; `ol-app` is UI-free; Plan B (Tauri over `ol-daemon`) costs about 4 weeks |
| 6 | Scope creep toward LightBurn parity | H | H | Exit criteria are the only definition of done. Nothing from T1 lands before M5 except the named T0-stretch items |
| 7 | Unknown JPT M7 behaviour (pulse encoding, PRR, mid-list changes) gives bad marks or stresses the source | M | M | EZCAD captures (D6), conservative defaults, PRR warnings, H21 gating the one-list change |
| 8 | Ecosystem churn (egui every 1–2 months, wgpu lockstep, nusb 0.2, kurbo/usvg pre-1.0) | H | M | Exact pins, quarterly upgrade window, wrapper crates, one wgpu major |
| 9 | Hardware damage or a wedged board during tests (single machine) | L | H | Red light first, ≤ 10 % power for first emission, never provoke H17, no USB resets in code |
| 10 | Name or trademark confusion, and the Rayforge side track draining focus or creating liability on other people's machines | M | M | Rename at M0. R is optional, after M3, EXPERIMENTAL, gated on the maintainer's response |

---

## 7. Decisions Luiz must make (with recommendations)

| # | Decision | Recommendation |
|---|---|---|
| D1 | Product name | **Kerfwright**; decide at M0 before anything is published |
| D2 | Licence | **MIT OR Apache-2.0**, with the provenance policy |
| D3 | GUI | **egui**, gated by the spike; Tauri only as Plan B |
| D4 | Put Rust under the Python UI (PyO3) before building the native app | **Yes**: it is what makes every step ship to the machine |
| D5 | Rayforge side track | **Offer after M3**; build only on a positive maintainer response |
| D6 | Access to a Windows PC with EZCAD, the machine's original `.cor` and `markcfg7` | **Get it before M3**. H5–H9 depend on it |
| D7 | Apple Developer Program ($99/yr) | **Yes, by M5** |
| D8 | T0 scope | Material test grid and sub-layers **in** (M3/M5); GRBL live streaming and image engraving in **M6** |
| D9 | Wire a door switch or foot pedal to the board inputs | **Yes**, if the enclosure has a door (M3) |
| D10 | Which GRBL machines are physically available for M6 tests | Name them at M5 |
| D11 | Load firmware for clone boards from the user's own EZCAD install | **No**: detect and explain only |
| D12 | Sign off the `rust-v1` divergences (explicit first jump, single session, refusing speeds below 1 unit, later one list per job) | **Approve** at M1 |

---

## 8. The Python prototype in the meantime

**Freeze features now and ship one safety hotfix release in M0.** These are well-specified executor tasks, about 4 focused days. Each fix gets a regression test, and each test becomes a T5 golden for Rust.

1. **3 s wait cap (G6, 05 §0.1).** In `apply_wait_timeouts` (`live.py:455-482`):
   - `wait_idle` and `wait_finished` lose their deadline. They stay stoppable through `_openlaser_stop`.
   - `wait_ready` raises `ConnectionAbortedError` when its deadline passes instead of returning, so a packet is never written into a full buffer.
   - Replace the flat 600 s per-pass timeout with `2 × estimated time + 60 s`.
   - Test: the Appendix A `long_job` with a 20 s BUSY board must finish with `Fiber_SetMo(0)` only after BUSY clears.
2. **Elided first jump (G4).** Before each job, reset galvoplotter's cached `_last_x`/`_last_y` (`controller.py:156, 518`) to a sentinel that cannot equal the first point, forcing an explicit `JumpTo`. Test: the `dot_test` capture now contains `listJumpTo`. Check the distance word in the capture.
3. **Confirm dialog shows Frame values (G1).**
   - `/api/plan` returns per-layer emission parameters (from `emission_steps`).
   - The Mark modal renders only those (`ui.py:2210-2222, 1540-1556`).
   - Frame Power never appears in a mark summary.
4. **ARM always true and persistent (G2).**
   - `/api/plan` issues a single-use nonce valid for 60 s, which `/api/mark` must echo.
   - The client sends `arm` only from the confirm dialog.
   - The ARM field clears after every job and whenever the dialog closes.
5. **Hidden layers burn unframed (G3).** `framing_jobs()` uses the same emit set as marking, regardless of `visible`. The confirm dialog lists hidden-but-output layers in red.
6. **Speeds below 3.61 mm/s encode to 0 (G5).** `validate_layers` refuses when `trunc(v·0xFFFF/F/1000) < 1` and reports the minimum speed.
7. **Also:**
   - G7: an object whose layer is unknown is refused, not dropped.
   - G10: points with more than two coordinates are sanitised.
   - Pin `galvoplotter==0.2.0` in `pyproject.toml` (05 §6).

**After the hotfix:**
- The prototype remains the daily driver through M1.
- At M2 it switches internally to the Rust engine, and legacy mode is kept for one release as an A/B escape hatch.
- At M5 the repo is archived read-only, and its README points to the new app.
- Its pinned copy lives on as `oracle/` in the Rust repo, so every future encoder change is still checked against the behaviour proven on the real JPT M7.

*Name-check sources (2026-10-23): crates.io API and github.com lookups for `kerfwright` and `beamwright`; web searches ["Beamwright" laser software](https://en.freedownloadmanager.org/Windows-PC/amp/Beamwright-FREE.html) (found the stage-lighting calculator) and "Kerfwright" (no product hits; e.g. [Wright disambiguation](https://en.wikipedia.org/wiki/Wright_(disambiguation)) was the closest result).*
