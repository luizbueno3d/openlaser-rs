# Proposal B: Platform-first OpenLaser in Rust

*Date: 2026-10-23. Angle: architecture-first and aimed at parity. Author: planning subagent (Claude) for Luiz.*
*Evidence base (cited as R01–R06 and RF):*
- *R01* `research/01-rayforge-source-deep-dive.md`
- *R02* `02-lightburn-feature-inventory.md`
- *R03* `03-open-source-prior-art.md`
- *R04* `04-rust-ecosystem-survey.md`
- *R05* `05-hardware-protocols-and-test-inputs.md`
- *R06* `06-critic-gaps-and-corrections.md`
- *RF* `rayforge-test/REPORT.md`

*The Python prototype is cited as **OM** (`~/AI/OpenLaser/OpenMopa` @ `6aed8ff`).*

---

## 1. Strategy and stances

### 1.1 Strategy

Build OpenLaser as a **Rust platform first and an app second**. The first months go into the contracts that every later feature lands on:
- a document model that can hold a LightBurn `.lbrn2` file without loss
- a typed operations IR (intermediate representation) that knows galvos natively
- an incremental planning graph keyed by content hashes
- a capability-based driver layer with typed encoded artifacts
- a safety layer that sits below every extension point and that no caller can bypass

The contracts are filled in vertically, in order of what Luiz can verify:
1. the JCZ/LMC galvo first (the hardware he owns, and the gap nobody fills: R01 §2.6, R06 §0.1)
2. GRBL second (export goldens now, an emulator-tested sender later)
3. Ruida third, behind the same traits

Each early milestone ends in an **observable, hardware-free test result** or a **CLI Luiz can use**. Examples are byte-exact parity with the Python prototype, and G-code he can send to a GRBL machine today. The cost is accepted: the first desktop build that replaces the prototype arrives after about 20–27 focused weeks, not the roughly 12–16 a product-first plan would need. In return, Ruida, `.lbrn2` round-trip, sub-layers, scripting and cameras slot into existing seams instead of forcing rewrites.

### 1.2 Stance on Rayforge: interoperate and borrow, never depend at runtime

- **No fork and no runtime dependency.** Rayforge is a bus-factor-1, fast-moving Python/GTK app. raygeo ships about 123 releases in 5 months and has no Rust tests (R01 §7). raygeo also installs an unconditional `#[global_allocator]` (R06 §0.4, C3).
- **Vendor raygeo algorithms, not its types.** Copy selected MIT modules into `ol-geom/src/vendor/raygeo/` with a per-file `// Ported from barebaric/raygeo@eab6fa8, MIT` header. Candidates are `ops/transform/optimize.rs` (k-d tree plus 2-opt), `tabs.rs`, arc fitting, and later `geo/algo/nest2d`. Strip the allocator and add Rust tests on intake (R06 §4.3).
- **Design our own IR**, using Rayforge's shape as a reference (R01 §1.5, §8.1). Their `Ops` cannot carry galvo delays or wobble, uses CO₂ PWM semantics for pulse width (µs, not ns; R01 §3.1), and is a Python-facing API.
- **raydriver is a reference only.** Port its cancel ordering with attribution: abort the streamer *before* resetting flow control (R01 §2.3, raydriver `session.rs:1757-1830`). Do not depend on it. It is untested without Python (R06 G16) and brings tokio into a design that uses blocking threads (R06 C7).
- **Interoperate:**
  - `.lbrn2` and `.lbdev` import, with Rayforge's MIT importer as a format reference (R01 §8.1.8).
  - One shared TCP fake-GRBL so OpenLaser and Rayforge can be compared on the same emulator (RF method).
  - After M2, publish `lmc-proto` and `lmc-sim` as standalone MIT crates that Rayforge could adopt. This is the "contribute upstream" option at no cost to our roadmap (R01 §9).
- **Compete** only where we differ: galvo/MOPA depth, hard safety (Rayforge always offers "Proceed": R01 §8.2.1), Linux support (LightBurn dropped it: R02 §0.2), and a notarised single binary.

### 1.3 Stance on the Python prototype: golden oracle, safety fixes only, no strangler-fig

- OM becomes the **oracle**. Its LMC byte streams, G-code and safety decisions are the reference that Rust must match (R05 §5.3).
- **No strangler-fig.** Wrapping a Rust core behind OM's HTTP API or PyO3 would recreate the two-language boundary that hurts Rayforge (R01 §8.2.6). OM's `/api/*` is not a contract worth keeping.
- OM receives **only the safety fixes in §8** until the M3 desktop build replaces it. It is then tagged `python-final` and kept read-only as the oracle source, pinned by SHA.

### 1.4 Name

"OpenLaser" collides with two active projects in this exact domain. One is `clayfrocker97/OpenLaser` (Rust, GPL-3.0, fiber controller). The other is `theanam/openlaser.app` (MIT, 127★ in three days) (R03 §1.6, R04 finding 4).

**Recommendation:**
- Keep "OpenLaser" as the private working name through M2.
- Pick a new public name before the first public Rust binary (M3).
- On 2026-10-23 these crates.io names were free: `fieldmark`, `beamwright`, `markwright`, `pulsewright`. `fluence` is taken. GitHub and trademark searches were not done (UNVERIFIED).
- Reserve the chosen crate names at M0.
- Crate prefixes stay `ol-` internally, so renaming costs nothing.

### 1.5 Where the reports disagree, and what this plan follows

| Topic | Followed | Why |
|---|---|---|
| Docking crate | `egui_tiles` 0.18 (R06 C9), not `egui_dock` (R04 §2.4) | egui_dock 0.21.1 requires egui ^0.36 |
| raygeo | Vendor files (R01 §8.2.7, R06 C3), not a git dependency (R03 §7, R04 §1) | Unconditional global allocator; CI never builds it without Python |
| Rust LMC prior art | "One unlicensed red-light-only experiment" (R06 C1) | GP30 exists; facts only. Its idle status `0x0220` goes into the status parser |
| Concurrency | One OS thread per machine, blocking nusb in ≤ 20 ms slices (R04 §8, R06 C7, spot-check 6) | `transfer_blocking` loops after a timeout; a `tokio::select!` design (R03 §4.5) adds runtime complexity for no stop-latency gain |
| USB crate | `nusb` (R06 C8) | Pure Rust; works on macOS without setup; WinUSB on Windows is unavoidable anyway (R06 C12) |
| Galvo scale constant | `0xFFFF/F` for coordinates; `65536/scale` only when reading a `.cor` (R06 C11) | Byte-exact parity with OM |
| `.lbrn2` Y axis | Y-up (R06 §0.7) | Verified on `fire_flower.lbrn2`; a second fixture is still required |
| Windows coexistence with EZCAD | None; the driver swap is required (R06 C12) | LightBurn swaps drivers too |
| Serial crate | `serialport`, blocking (R06 C14) | MPL-2.0 is pulled in either way; the blocking model matches C7 |
| Test count | 119 tests in 14 modules (R06 C10) | Counted statically and run by R05 |
| Stop target | Hardware ≤ 250 ms (R05 §1.10); **fake board CI ≤ 100 ms p99** (tighter than R04's 250 ms) | The fake board has no USB latency; a looser budget hides regressions |
| MVP definition (R06 G13) | M3 = R02's T0; M4–M5 = T1; M6+ = T2 | Reconciles the three tier schemes |

---

## 2. Architecture

### 2.1 Five architectural rules

1. **The core never depends on UI, I/O runtimes or plugins.** An `xtask deps` lint enforces the crate DAG in CI. Rayforge's dead `register_machines` hook (R01 §3.1) is the cautionary example: every extension point has an end-to-end CI test.
2. **Units are types.** These are newtypes, not raw `f64`: `Mm`, `MmPerS`, `Hz`, `Ns`, `Us`, `Percent` and `GalvoUnits`. LightBurn stores frequency in Hz, OM uses kHz, and Rayforge labels pulse width in µs (R01 §3.1, R02 §17.2). Mixing these is a safety bug, not a cosmetic one.
3. **Every burn is built cold.** The incremental cache serves preview only. At ARM time, the emitting job is rebuilt from scratch, bypassing every cache, and its hash must equal the previewed hash; otherwise the job is refused (§2.8). A stale cache can never burn old geometry.
4. **Refuse, never clamp, truncate or skip.** This holds in geometry, encoding and drivers (OM S3/S4, R05 §1.7). Quantisation that changes meaning is also refused, for example a speed that encodes to 0.
5. **Every trait needs two real implementations before it is frozen.** `Driver` must have LMC, FakeLmc and FakeGrbl implementations by M2. `Encoder` must have LMC and G-code by M1. This is the guard against platform over-design.

### 2.2 Cargo workspace (dependency direction is top to bottom; a crate may only depend on rows above it)

| Layer | Crate | One-line responsibility |
|---|---|---|
| 0 Foundation | `ol-units` | Unit newtypes, checked conversions, Python-compatible rounding (`round_ties_even`, `trunc`) |
| | `ol-geom` | kurbo paths, flattening, booleans (`i_overlay`), offsets (`cavalier_contours`), `rstar` index, hatch, join, ordering; vendored raygeo modules |
| 1 Model | `ol-doc` | Project/Page/Shape/CutTable model, immutable `Arc` snapshots, snapshot undo, `.olp` format and migrations, `Extras` bag |
| | `ol-machine` | Machine profiles (galvo and gantry blocks), laser-source descriptors, safety limits, IO maps; markcfg `.ini`, `calibration.json`, `.cor` and `.lbdev` readers |
| 2 Planning | `ol-ir` | Typed ops IR (`BurnOps`, `FrameOps`, `MachineJob<O>`), provenance and op-id spans |
| | `ol-plan` | Intent graph (content-hash tokens, cache, rayon executor); assemblers Line, Fill, OffsetFill, Image; frame-job derivation; time estimates |
| | `ol-kinematics` | World → machine transforms: galvo field mapping (scale correction, axis map, offset, angle), gantry origin and WCS, arc linearisation from capabilities |
| | `ol-safety` | Pure preflight over `MachineJob` plus profile; sealed `VerifiedJob`, `ArmToken`, `ArmedJob` types |
| 3 Protocol (pure, no I/O) | `ol-lmc` | LMC opcodes, wire types, status word, unit encoders, list packer, `MachineJob → lmc::Program`, `py-compat` sequencing mode, listing decoder |
| | `ol-gcode` | Dialect tables (GRBL first), emitter with op map, response parser |
| | `ol-ruida` (M6) | Swizzled Ruida binary encoder |
| 4 Runtime | `ol-transport` | `Transport` trait; `UsbBulk` (nusb), `Serial` (serialport), `Tcp`, `Udp`; `Recording<T>` trace and `Replay` wrappers |
| | `ol-driver` | `Driver`/`Encoder` traits, `Capabilities`, `Payload` enum, `MachineHandle`, session loop, stop latch, watchdog, driver registry |
| | `ol-drv-lmc`, `ol-drv-grbl`, `ol-drv-ruida` | Protocol sessions: flow control, status, abort sequences |
| | `ol-sim` | Fake LMC board, fake GRBL (Rust port of `rayforge-test/fake_grbl.py`), later a fake Ruida; in-process and TCP modes |
| 5 Edges | `ol-io` | Importers: SVG (`usvg`), DXF (`dxf`), OM `workspace.json`/`layer_settings.json`, `.lbrn2` (M4). Exporters: SVG, DXF, G-code file |
| | `ol-script` (M4) | `rhai` engine: generators and post-processors that output IR. It cannot reach `arm()` (enforced by the dependency lint) |
| | `ol-app` | `AppState`, serde `Command`/`Event` enums, view-models, job orchestration. **No egui dependency** |
| | `ol-render` | Scene → egui painter / wgpu hairline pipeline; tiny-skia for golden images |
| 6 Shells | `ol-ui-egui` | eframe shell, `egui_tiles` docking, canvas widget, `muda` menus, `rfd` dialogs |
| | `ol-cli` | Headless binary: import, plan, encode, check, frame, mark, `hil` session scripts |
| Tooling | `xtask`, `tools/oracle/` (Python) | CI orchestration, dependency lint, golden generation from OM |

### 2.3 Core data model

The document mirrors LightBurn's cut-setting model from the REST OpenAPI, with `galvo{}` and `gantry{}` blocks always stored (R02 §7.7). That way one project retargets between the MOPA and the GRBL machines, and `.lbrn2` maps one to one.

```rust
// ol-doc. World space: millimetres, Y-up (matches .lbrn2 per R06 §0.7 and the OM canvas).
pub struct Project {
    pub format_version: u32,                 // .olp = zip{project.json, assets/<blake3>, thumbnail.png}
    pub meta: ProjectMeta,
    pub pages: Vec<Page>,                    // LightBurn 2.2 pages; exactly 1 until M6
    pub cuts: CutTable,                      // shared across pages (LightBurn semantics)
    pub machine: MachineRef,                 // profile id + profile content hash at last plan
    pub variable_text: VariableTextState,    // .lbrn2 <VariableText>
    pub optimize: OptimizeSettings,          // .lbrn2 <UIPrefs> Optimize_*
    pub extras: Extras,                      // unknown .lbrn2 elements and attributes, preserved verbatim
}
pub struct Shape {
    pub id: ShapeId, pub kind: ShapeKind, pub xform: kurbo::Affine,
    pub cut: CutIndex,                       // 0..=29 | T1 | T2: layer identity = palette slot, as in LightBurn
    pub props: ShapeProps,                   // locked, power_scale, cut_order_priority
    pub extras: Extras,
}
pub enum ShapeKind {
    Path(PathRef),                           // Arc<PathData> keyed by content hash: mirrors VertID/PrimID sharing
    Rect { w: Mm, h: Mm, corner_r: Mm }, Ellipse { rx: Mm, ry: Mm },
    Polygon { sides: u16, r: Mm }, Text(TextShape /* Qt font string + backup path */),
    Bitmap(BitmapShape /* asset hash + non-destructive adjustments */),
    Group(Vec<Arc<Shape>>),
}
pub struct CutEntry { pub vector: Option<CutSetting>, pub image: Option<CutSetting> } // LightBurn's 30 + 30 image cuts
pub struct CutSetting {
    pub name: String, pub mode: CutMode,     // Line | Fill | OffsetFill | Image | Tool
    pub output: bool, pub show: bool, pub priority: i32, pub global_passes: u16,
    pub common: LaserCutParams,              // speed, min/max power, passes, interval, angle, angle_per_pass,
                                             // frequency: Option<Hz>, q_pulse: Option<Ns>, bidir, crosshatch, scan_opt…
    pub galvo: GalvoCutParams,               // override_timings, laser_on/off/end/polygon TC, jump speed and delays, wobble
    pub gantry: GantryCutParams,             // air, constant power, lead-in/out, overcut, dot mode…
    pub sub_layers: Vec<SubLayer>,           // ≤ 11 deltas over this setting (LightBurn sub-layers)
    pub image: Option<ImageCutParams>,
    pub extras: Extras,
}
```

**What "lossless" means here, as testable claims:**
1. Every field in the REST cut-setting schema (R02 §7.7) has a typed slot.
2. Unknown `.lbrn2` keys round-trip through `Extras`.
3. Curves stay curves; they are flattened only in `ol-plan`.
4. Text keeps the font string and the backup path.
5. Bitmaps keep the source bytes plus their adjustment parameters.

The M6 exit test is that reading and writing `fire_flower.lbrn2` and `coat-hook-v4.lbrn2` gives a canonical-XML diff that is empty apart from `AppVersion`.

```rust
// ol-machine. TOML files in the config directory; a project references them by id.
pub struct MachineProfile {
    pub id: MachineId, pub name: String,
    pub kinematics: Kinematics, pub source: LaserSource,
    pub connection: ConnectionSpec,          // UsbLmc{vid,pid,serial} | Serial | Tcp | Udp | ExportOnly
    pub driver: DriverId,                    // "lmc" | "grbl" | "ruida"
    pub limits: SafetyLimits,                // max paths/points (3000/20000 by default, for OM parity), require_framing, arm_ttl
    pub defaults: LayerDefaults,             // per machine kind: never shared (RF takeaway 1: F15000/S1000 leaked into the diode)
    pub io: IoMap,                           // laser bit 0, red-light bit 8, door/foot inputs (H11)
}
pub enum Kinematics { Galvo(GalvoBlock), Gantry(GantryBlock) }
pub struct GalvoBlock {
    pub field: Size<Mm>, pub offset: Vec2<Mm>, pub angle: Deg,
    pub scale_correction: Axes<f64>,         // generalises 1.1834; clamped to 0.5..=2.0 on load (OM S8)
    pub axis: AxisMap,                       // swap, negate_x, negate_y; persisted per machine, not in localStorage (R05 §1.7)
    pub correction: Correction,              // Blank (OM today) | CorFile{hash, scale} | Grid9
    pub red_dot: RedDot, pub jump: JumpDefaults, pub timing: GalvoTiming, pub mo_settle: Us,
}
pub struct GantryBlock {
    pub bed: Size<Mm>, pub origin: Corner, pub max_speed: MmPerS, pub rapid: MmPerS,
    pub accel: Axes<MmPerS2>, pub s_max: u32, pub power_mode: PowerMode, pub dialect: DialectId,
    pub home_on_connect: bool,               // default false; Rayforge auto-homes without a prompt (RF)
}
pub enum LaserSource {
    FiberMopa { model: String, pulse_table: Vec<Ns>, freq: RangeInclusive<Hz>, prr_cutoff: Option<Vec<(Ns, Hz)>> },
    FiberQ { freq: RangeInclusive<Hz> }, Co2 { .. }, Diode { .. },
    Pen { down: String, up: String, dwell: Ms },
}
```

### 2.4 Job pipeline and IR

```
Project snapshot ──ol-plan (intent graph, rayon)──▶ BurnOps / FrameOps      (world mm, typed units)
   ──ol-kinematics──▶ MachineJob<Op> / MachineJob<FrameOp>                 (machine mm, linearised per caps)
   ──ol-safety::preflight──▶ VerifiedJob(Encoded)  ◀── Encoder (ol-lmc / ol-gcode)
   ──UI/CLI confirm → ArmToken──▶ ArmedJob ──MachineHandle──▶ driver thread ──Transport──▶ board
```

Preflight runs on the **encoded** job. Its coordinates are checked twice:
- in machine millimetres, against the field or bed
- after quantisation, where every LMC coordinate must lie in `0..=0xFFFF`; the encoder returns `Err` rather than clamping

```rust
// ol-ir
pub enum Op {
    // structure markers (drive progress, op maps and per-layer confirm summaries)
    LayerStart { cut: CutIndex, sub: u8, pass: u16 }, LayerEnd,
    ShapeStart(ShapeId), ShapeEnd, Section(SectionKind),            // Outline | Hatch | Raster
    // state, in typed units
    Power(Percent), Speed(MmPerS), TravelSpeed(MmPerS),
    Frequency(Hz), PulseWidth(Ns), GalvoTiming(GalvoTiming), Wobble(Option<Wobble>),
    PowerMode(PowerMode), Air(bool),
    // motion
    Travel(P2), Line(P2), Arc { to: P2, center: P2, ccw: bool }, Cubic { c1: P2, c2: P2, to: P2 },
    ScanRow { to: P2, levels: Arc<[u8]> },   // image rows; the LMC encoder splits them into mark runs (MeerK40t model, R06 §0.8)
    Dot { dwell: Us },
}
pub enum FrameOp { Travel(P2), Trace(P2) }   // no power and no emission variants exist: framing cannot burn by construction
pub struct MachineJob<O> { pub ops: Vec<O>, pub bounds: Rect, pub estimate: Estimate, pub provenance: Provenance }

// ol-driver: typed artifacts, so binary payloads cannot be dropped (Rayforge's driver_data bug, R01 §1.4)
pub struct Encoded {
    pub payload: Payload, pub listing: Listing, pub op_map: OpMap,       // int32 spans, as in Rayforge
    pub summary: EmissionSummary,            // per-layer parameters decoded *from the payload*, used by the confirm UI
    pub estimate: Estimate, pub hash: blake3::Hash,
}
pub enum Payload { Lmc(ol_lmc::Program), Gcode(ol_gcode::Program), Ruida(ol_ruida::Program), Opaque(Arc<dyn OpaquePayload>) }

// ol-lmc: the lighting program has its own command enum without MarkTo/MarkCurrent/Fiber_SetMo(1) (R05 §1.6 invariant)
pub struct Program { pub prologue: Vec<Single>, pub packets: Vec<ListPacket>, pub epilogue: Vec<Single> }
pub struct LightingProgram { pub packets: Vec<LightPacket> }
pub struct Status(u16);                      // ready() = 0x20, busy() = 0x04; unknown bits kept (GP30 idles at 0x0220)
```

**Galvo time model**, used by the estimate and the watchdog: mark length ÷ mark speed, plus jump length ÷ jump speed, plus laser-on/off, polygon and jump delays per vertex and per jump. This replaces Rayforge's gantry trapezoid, which is wrong for galvos (R01 §3.1).

### 2.5 Incremental intent graph

The graph is hand-rolled, about 1.5k lines; it does not use salsa. The document is already made of immutable `Arc` snapshots, so a memo table keyed by `(NodeKey, Token)` is enough. salsa's compiler-shaped API and churn are not worth it here.

| Node | Key | Token = blake3 of |
|---|---|---|
| `ShapeGeom` | shape id | kind params, path content hash, flatten tolerance |
| `ShapeOps` (Line mode) | shape, cut, sub | `ShapeGeom` token, geometry-affecting params (kerf, tabs), planner version; the transform is included unless it is a pure translation *and* the step is translation-invariant (R01 §1.4) |
| `LayerFill` (Fill, `merge_all`) | cut, sub, group | sorted member tokens and transforms, interval, angle, bidir |
| `LayerOps` | cut | children plus ordering settings (TSP) |
| `Job` | machine | layer tokens, order, passes |
| `MachineJob` | machine | `Job` token plus a hash of the kinematics block |
| `Encoded` | driver | `MachineJob` token plus encoder version |

The cache budget defaults to 256 MB with LRU eviction. Rayforge's default is 2 GiB (R01 §1.3).

The correctness property, run as proptest in CI: for 1,000 random edit sequences, the incremental result equals a cold rebuild, byte for byte. Rule 3 (cold build at ARM) is the runtime backstop.

### 2.6 Transport and Driver traits

```rust
pub trait Transport: Send {
    fn write(&mut self, data: &[u8], limit: Duration, stop: &StopLatch) -> Result<(), TxError>;
    fn read(&mut self, buf: &mut [u8], limit: Duration, stop: &StopLatch) -> Result<usize, TxError>;
    fn cancel_all(&mut self) -> Result<(), TxError>;       // nusb Endpoint::cancel_all; serial: purge buffers
}
// TxError::{Timeout, Stopped, Disconnected, Io}. Implementations wait in ≤ 20 ms slices using
// submit + wait_next_complete (R06 spot-check 6) and never reset the device (R05 §1.1 reset-storm lesson).

pub trait Driver: Send + 'static {
    fn descriptor(&self) -> &'static DriverDescriptor;      // id, label, maturity (Experimental/Beta/Stable)
    fn capabilities(&self, p: &MachineProfile) -> Capabilities;
    fn encoder(&self) -> &dyn Encoder;                      // pure; used for export, goldens and preflight without a device
    fn connect(&mut self, t: Box<dyn Transport>, s: &Session) -> Result<DeviceInfo, DriverError>; // init once per session
    fn run(&mut self, job: ArmedJob, s: &Session) -> Result<RunOutcome, DriverError>;           // blocking, driver thread
    fn frame(&mut self, f: FramePayload, mode: FrameMode, s: &Session) -> Result<RunOutcome, DriverError>;
    fn abort(&mut self, s: &Session) -> StopReport;         // called only by the session loop after the latch is set
    fn status(&mut self, s: &Session) -> Result<DeviceStatus, DriverError>; // Err on a failed read, never fake bits
    fn pause(&mut self) -> Option<&mut dyn Pause> { None }  // capability sub-traits, not GRBL-shaped stubs (R01 §8.2.3)
    fn jog(&mut self)   -> Option<&mut dyn Jog>   { None }
    fn home(&mut self)  -> Option<&mut dyn Home>  { None }
}
pub struct Capabilities {
    pub kinematics: KinematicsKind, pub arcs: bool, pub beziers: bool,
    pub emission: EmissionCaps,              // frequency range, pulse table, per-segment power, dynamic power mode
    pub framing: FramingCaps,                // red-light pointer | beam-off trace; continuous
    pub pause: Support,                      // None | Native | Unverified (LMC until H15 is answered: hidden in the UI)
    pub progress: Granularity,               // Packet (LMC acknowledgements) | Line (GRBL ok) | None (never simulated, R01 §8.2.2)
    pub stop: StopCaps,                      // realtime abort available, expected latency
    pub io: IoCaps, pub native_overscan: bool,
}
pub struct MachineHandle { /* cmd_tx, prio_tx, latch: Arc<StopLatch>, events_rx, join handle */ }
impl MachineHandle {
    pub fn stop(&self) -> StopTicket;        // latch.set() + prio_tx.try_send(); never blocks, < 1 µs
    pub fn submit(&self, job: ArmedJob) -> Result<JobId, Busy>;
    pub fn frame(&self, f: FramePayload, mode: FrameMode) -> Result<(), Busy>;
    pub fn events(&self) -> &Receiver<MachineEvent>; // Status, Progress{acked_ops}, Stopped{latency, clean}, Fault
}
```

The **driver registry** is explicit code (`DriverRegistry::builtin()`), with no dynamic loading. A CI conformance suite iterates over every registered driver and runs it against its simulator. Third-party *drivers* are out of scope until a signed-plugin story exists, because a driver can bypass safety.

### 2.7 Threads and the real-time stop path

- **UI thread:** egui plus `ol-app` state. Commands are cheap; heavy work goes to rayon.
- **rayon pool:** planning, tracing, export rendering.
- **One OS thread per connected machine:** it owns the transport and is the *only* code that touches the USB handle (R05 §1.10).
- **tokio:** only inside network transports (Ruida UDP, FluidNC WebSocket), as a current-thread runtime inside that machine's thread, so the `Driver` trait stays blocking everywhere.
- **GRBL:** realtime bytes (`!`, `0x18`) are written by `stop()` itself through a `try_clone`d serial handle. GRBL extracts realtime bytes anywhere in the stream (R05 §3.3). The session thread then hard-aborts streaming *before* resetting character-count state (raydriver #428 lesson).

**LMC stop sequence** (R05 §1.9–1.10, R06 C16):
1. Latch set.
2. The session loop sees it within one wait slice.
3. `cancel_all()`, then drain completions.
4. Assume a partial packet may have reached the board.
5. Send `StopExecute → Fiber_SetMo(0) → ResetList → [dummy EOL packet, SetEndOfList, ExecuteList; kept until H14 says otherwise] → Fiber_SetMo(0) → WritePort(0)`.
6. Emit `Stopped{latency}`.

If any of these transfers fails, the session becomes **Faulted** and needs a manual reconnect. It never auto-resets the device.

| Stage | Budget (hardware) | Fake-board CI |
|---|---|---|
| `stop()` call to latch visible | < 1 µs | asserted |
| Latch to driver reaction (wait slice) | ≤ 20 ms | ≤ 20 ms |
| `cancel_all` and drain | ≤ 50 ms (cap 100 ms) | ≤ 20 ms |
| Abort sequence (6 singles plus 1 packet, ≤ 100 ms each) | ≤ 80 ms typical | ≤ 40 ms |
| **Total** | **≤ 250 ms p99** (OM measured 0.18 s, R05 §1.9) | **≤ 100 ms p99** |

**Watchdog.** It is progress-aware: the deadline is the estimated remaining time × 1.5 + 5 s, and it is extended whenever the board acknowledges a packet. When it fires, it **stops**. It never "proceeds", unlike OM's 3 s cap (R05 §0.1).

**Run-ahead limit.** The driver keeps at most 3 unexecuted packets on the board: execution starts after the 3rd packet, and later packets are gated by READY, which is galvoplotter parity. A crash of the app therefore leaves only a bounded amount of marking on the board. Retune this after H2 measures the buffer depth.

**Unconfirmed stop.** If `Stopped` does not arrive within 1 s, the UI shows a full-width banner: "Stop not confirmed: use the key switch or E-stop".

### 2.8 Safety model: where the hard refusals live

| Layer | Mechanism | Covers |
|---|---|---|
| Types | `FrameOp` and `LightingProgram` have no emitting variants. `VerifiedJob` can only be constructed by `ol_safety::preflight`. `ArmToken` is single-use, expires after 30 s, and is bound to the job hash and session id. `ArmedJob::new` re-checks all three | OM S2; G2 (sticky ARM) |
| Preflight (pure, exhaustive, no "proceed anyway") | Finite coordinates; machine-space field/bed check after kinematics; path/point/time limits; power/frequency/pulse against `LaserSource` (±0.5 ns snap with warning, otherwise refuse); **encoded speed of 0, or quantisation error > 5 %, is refused** (G5); speed above the gantry maximum is refused, not silently capped (RF); unknown cut reference is refused (G7); **frame set equals emit set**, so a hidden layer with Output on is refused with "show it or turn Output off" (G3); job features beyond the driver's capabilities are refused | OM S1, S3–S5, S7, S15 plus 05 gaps G3, G5, G7, G9 (PRR, when the table is known) |
| Cold build | At ARM, rebuild the job without any cache and compare hashes | Stale-cache burns |
| Confirm | The summary comes from `Encoded.summary`, decoded from the payload: per-layer power, kHz, ns, **encoded** mm/s, time, bounds drawn against the field. If `require_framing` is set (default on for galvo), the frame must have run in this session for the same hash. Confirmation is a checkbox ("area clear, eyewear on") plus a 1 s press-and-hold. CLI: `--arm` plus typing `ARM` on a TTY (refused when stdin is not a TTY) | G1 (wrong parameters shown) |
| Encoder | Quantise or refuse, never clamp. Always emit an **explicit first JumpTo** | G4 |
| Driver | No list write unless the last status was READY; no MO close while BUSY unless stopping; watchdog stops; failed status read is `Err`; no auto USB reset; never moves a gantry on connect | R05 §1.10 requirements; G6 |
| Interlocks | Door input (`IoMap`, H11) is polled with status: open means refuse ARM and stop a running job. Documented as secondary protection only | G8 |
| Policy | No API, script or LAN path can ARM; only a human at the UI or a TTY can (LightBurn REST makes the same choice, R02 §16). Docs state that OpenLaser is not a safety device and never overrides hardware interlocks (R06 G12) | — |

### 2.9 Scripting and plugins

- **`rhai` from M4** (R04 §10). It is sandboxed: no file system or network by default. Scripts can build documents, generators (material-test grids, serial plates) and IR post-processors. They have no dependency path to `arm()`, which the dependency lint enforces.
- **WASM plugins (extism) in M7 at the earliest**, and only for importers, assemblers and post-processors. Their output still goes through preflight.

---

## 3. UI approach

**Framework: egui/eframe 0.37 + `egui_tiles` 0.18 + `muda` + `rfd`** (R04 §2.3 scores egui 85.6 against Tauri at 81.0; R06 C9 fixes the docking crate).

**Shell boundary.** `ol-app` exposes serde `Command`/`Event` types and view-models. This keeps **Tauri 2 plus a web UI** as Plan B at a cost of weeks, not months, and later allows a LAN remote that uses the same types.

**Canvas strategy:**
1. **M3:** epaint painter with flattened polylines cached per shape and per zoom bucket, `rstar` hit-testing, overlays on egui's painter.
2. **When the spike or real documents demand it:** a custom wgpu screen-space hairline pipeline through `egui_wgpu::Callback`, with instanced segment quads (R04 §3), behind a `CanvasRenderer` trait.
3. **Goldens:** the same scene description renders through tiny-skia for CI golden images, so there are no GPU-dependent snapshots.

**De-risking spike: 2 weeks of throwaway code, delegated to an executor during M1 weeks 3–4.** It uses only synthetic data, so it does not wait for the core. The decision must be recorded before M2 ends.

| # | Metric (M2 MacBook Air, macOS 26, release build) | Gate |
|---|---|---|
| 1 | 20k paths / 200k segments: pan and zoom frame time p95 ≤ 16.7 ms | **must** |
| 2 | Hit-test p99 ≤ 2 ms; dragging 1k selected shapes ≥ 60 fps | **must** |
| 3 | `muda` native menu bar with Cmd shortcuts inside eframe 0.37 (only precedent is 0.36, R06 spot-check 5) | **must** |
| 4 | `cargo tree -d` shows one egui and one wgpu major across every egui-ecosystem crate | **must** |
| 5 | Idle CPU < 2 % with no input (repaint on demand) | should |
| 6 | `egui_tiles` layout persisted with serde; VoiceOver reaches the layer list | should |
| 7 | `egui_kittest` snapshot runs headless in CI | should |

**Outcome rules:**
- **No-go** (any *must* fails after one fix attempt): switch to Plan B before M3. Re-score GPUI Kit at the same time (R04 §2.4).
- **Go with debt:** a failed *should* becomes an M3 ticket.

---

## 4. Testing and verification

### 4.1 Golden oracle from the Python prototype

- **`tools/oracle/`** extends the R05 Appendix A harness. It uses the OM venv, blocks USB in both pyusb and galvoplotter, patches the calibration path to a temp file, and masks timestamps.
- It runs against an **oracle tag** of OM, `oracle-v1`: the commit after the §8 P0 fixes, once those are hardware-validated. Goldens record the OM SHA and galvoplotter 0.2.0.
- The `6aed8ff` hashes in R05 act as a cross-check for streams the fixes do not touch. For example, `mark_box10` starts off-origin, so its packet SHA `9ca1e12d…59f5` must not change.
- **Never run `site-packages/test/*.py`**; those tests open real USB (R05 §1.3.2).

| Tier (R05 §5.3) | Rust target | Starts |
|---|---|---|
| T0 numeric encoders | Exact over ≥ 100k seeded cases plus edges (30 % → 1228, 70 % → 2866, ±F/2, speed < 1 unit) | M1 |
| T1 LMC streams | `py-compat` mode byte-exact: init, mark_box10, frame_box10, dot_test, long_job (9 packets = 8×256 + 164), business card per layer at F = 236.68, abort (dummy packet `e479558e…`). `rust-v1` sequencing (one session, explicit first jump, no per-job reset) is reviewed once and frozen with `insta` | M1 |
| T2 G-code | Byte-exact after masking line 3, for 4 profiles × {vector, multi-pass, air, raster cross-hatch, plotter} | M1 |
| T3 hatch | Segment count and order exact; coordinates within 1e-9 (workspace raster layer: 969 segments, 67 open shapes skipped) | M1 |
| T4 importers | DXF parity within 1e-9. SVG diverges **on purpose** (true scale with units, no fit-to-60 mm); documented | M1 |
| T5 safety decisions | Same decision and same error *category* for every OM safety test, plus new refusals for G1–G7 | M1 |
| T6 timing | Fake board, §4.2 | M2 |
| T7 hardware | HIL sessions, §4.4 | M2+ |

`py-compat` stays until M5, when the `rust-v1` goldens become the only reference.

### 4.2 Emulators (`ol-sim`)

- **FakeLmcBoard.**
  - Models READY/BUSY with a configurable buffer depth from 1 to 8 (H2 is unknown, so tests sweep it).
  - Execution time per packet comes from the galvo time model.
  - Full-buffer write behaviour is selectable: NAK-until-timeout or accept-and-drop (H13 is unknown).
  - Status carries unknown bits (`0x0220`).
  - Injected faults: read error, write timeout, BUSY forever, disconnect, slow ACK.
  - Its event log uses the oracle JSON schema, so fake runs and oracle runs diff directly.
- **T6 assertions:**
  - (a) Stop p99 ≤ 100 ms from every state across 10k randomised schedules.
  - (b) No write while not READY.
  - (c) No MO close or StopExecute while BUSY unless stopped. The R05 20 s BUSY scenario must **complete**.
  - (d) The watchdog fires only without progress past the estimate.
- **FakeGrbl:** a Rust port of `rayforge-test/fake_grbl.py` (written for Luiz in that sibling run). It has a 15-block planner, a 128-byte RX buffer, realtime bytes and alarms, and runs in-process or on TCP `:2323`, so Rayforge can be pointed at it too.
- **FakeRuida (M6):** behaviour taken from MeerK40t's MIT Ruida emulator (R03 §6).

### 4.3 Property and fuzz tests

**Property tests (proptest):**
- If preflight passes, every quantised coordinate is within `0..=0xFFFF` and every point is inside the field, under random transforms including mirror, swap and correction.
- Hatch segments lie inside their polygons.
- Path ordering visits every path exactly once.
- Unit encoders are monotonic and match the Python table.
- Decoding an LMC listing reconstructs the IR path within quantisation error.
- Incremental planning equals a cold rebuild.
- The stop budget holds under random fake-board schedules.

**Fuzzing (`cargo-fuzz`):** SVG, DXF, `.lbrn2`, `.olp`, markcfg, `.cor`, and the GRBL response parser.

### 4.4 Hardware-in-the-loop protocol

**Rules:**
- Luiz runs every session. Agents never execute hardware commands.
- Sessions are `ol hil <name>` scripts. They record every transfer through `Recording<UsbBulk>` into `hil/traces/<date>-<name>.jsonl` and fill in a checklist in `hil/sessions/`.
- Red light comes before emission. Emission happens only at low power, on scrap, wearing eyewear.
- Where the red dot works with the source key off, keep the key off (UNVERIFIED for the M7).

My order runs by **what the Rust driver's correctness depends on first**. This differs from R05's ordering, which was driven by the Python bugs:

| Session | Items | Why first |
|---|---|---|
| HIL-0 passive (M2 week 1) | H12 descriptors and `GetVersion`/`GetSerialNo` replies | Zero risk; fixes nusb configuration and the status parser |
| HIL-1 red light (M2) | **H2** buffer depth and READY/BUSY timing; **H13** full-buffer write behaviour (one deliberate write, red light only); **H14** minimal abort; **H3** position after Reset, after a job and after abort; H1 (a 20 s single-pass frame must run to the end); **H11** output bits, and the door/foot inputs if wired; **H4** orientation with an asymmetric "F" | Flow control and stop correctness; everything else rests on them |
| HIL-2 low-power emission (M2 exit) | H10 low-speed quantisation; H19 MO settle and first pulse; H1 long raster; 10 mm box size check | First Rust-driven marks |
| HIL-3 EZCAD capture on Windows (M5; needs a PC with EZCAD) | **H5** original `.cor` (likely explains 1.1834, R03 §1.5); H6 pulse-width encoding; H7 power; H8 frequency and PRR limits; H9 delay units; H20 laser mode | Parameter fidelity rather than safety. Before this, refuse to rely on undocumented semantics |
| HIL-4 (M5) | H15 pause/resume; H16 `GetMarkTime`/`GetListStatus` | Unlocks the pause capability and progress quality |
| HIL-5 (M6) | H18 WinUSB and Linux udev on real hardware | Cross-platform claims |
| Opportunistic only | H17 wedge recovery | Never provoked with emission |

### 4.5 CI matrix

| Job | Runner | When |
|---|---|---|
| fmt, clippy `-D warnings`, nextest, insta, proptest (short), `xtask deps`, cargo-deny, cargo-about | macos-26 arm64 (primary) and ubuntu-24.04 | Every PR |
| Goldens verify (committed JSON) and T6 fake-board suite | both | Every PR |
| kittest UI snapshots (tiny-skia) | ubuntu | Every PR from M3 |
| Oracle regeneration (checks out `luizbueno3d/OpenLaser@oracle-v1`, pinned venv), compared with the committed goldens | ubuntu | Nightly |
| Long proptest, fuzz (10 min per target), criterion budgets (business-card plan and encode < 200 ms) | macos | Nightly |
| windows-2025 build and test | — | From M4 |
| Unsigned `.app` build smoke test (`--version`, headless `ol check`) | macos | Every PR from M3 |

The toolchain is pinned in `rust-toolchain.toml` to the stable release installed at M0, which must be ≥ 1.95 for egui 0.37 (R06 G14).

---

## 5. Roadmap

Effort is in **focused weeks for one person directing AI executors** (Claude plans and reviews; Codex or OpenCode execute through `/route-implementation`). These are judgement estimates. No Rust has been measured on this Mac yet (R06 G14).

### 5.1 M1 in detail: headless core, golden parity (6–8 weeks)

**In scope:**
1. Crates `ol-units`, `ol-geom` (flatten, bbox, join, hatch ported from `raster.py` including the `pitch × 0.0173` offset), `ol-doc` v1, `ol-machine`, `ol-ir`, `ol-plan`, `ol-kinematics`, `ol-safety`, `ol-lmc`, `ol-gcode`, `ol-io`, `ol-cli`, `xtask` and `tools/oracle`.
   - `ol-doc` v1 covers Project, one Page, `Shape{Path, Group}`, CutTable with all three parameter blocks, and `Extras`.
   - `ol-machine` covers the 4 built-in profiles as TOML plus the markcfg `.ini` and `calibration.json` importers.
   - `ol-plan` has the Line and Fill (`merge_all`) assemblers and the intent graph.
   - `ol-safety` covers S1–S8, S14, S15 and G1/G3/G4/G5/G7.
   - `ol-lmc` covers T0 encoders, the packer, the lighting program, `py-compat` and `rust-v1`.
   - `ol-gcode` gives GRBL parity with `gcode.py`.
   - `ol-io` covers SVG via usvg, DXF, and OM `workspace.json` + `layer_settings.json`.
2. CLI commands:
   - `ol import <svg|dxf|workspace.json> -o x.olp`
   - `ol plan x.olp --machine <id> [--json]` (stats, bounds, galvo time estimate, refusals)
   - `ol encode x.olp --machine <id> (--gcode out.nc | --lmc-listing out.txt | --lmc-bin out.bin)`
   - `ol check`
   - `ol golden verify`

**Out of scope** (fields may exist in the model, but the planner refuses them with an explicit "not supported yet"):
- any USB or serial I/O, and any driver runtime
- the UI
- images, text, offset fill, sub-layers, wobble
- `.lbrn2`, scripting, Ruida

**Exit criteria (all observable):**
- E1: `cargo xtask ci` is green on macOS arm64 and Ubuntu.
- E2: T0 matches exactly on ≥ 100k cases per encoder.
- E3: T1 `py-compat` streams are byte-exact as listed in §4.1. The `rust-v1` dot_test begins with an explicit `JumpTo`.
- E4: T2 is byte-exact for 20 scenarios.
- E5: T3 gives exactly 969 segments in oracle order.
- E6: T5 categories match, and the 5 known OM bugs each have a refusing or covering test.
- E7: moving one shape in the 403-shape workspace recomputes exactly 1 `ShapeOps` node plus aggregates (counter-asserted), and the incremental-equals-cold proptest passes 1,000 sequences.
- E8: the business-card plan and encode, cold, takes < 200 ms.
- E9: cargo-deny is clean, and `THIRD_PARTY_NOTICES.md` credits galvoplotter and MeerK40t.

**What Luiz can do afterwards:**
- Turn SVG/DXF into GRBL G-code for the CO₂, diode and plotter profiles with byte-identical output to the prototype, usable with any sender.
- Read exactly what the MOPA would receive for any job, as a decoded LMC listing.
- Import his current workspace.

### 5.2 All milestones

| M | Scope | Exit criteria | Effort | What Luiz can do afterwards |
|---|---|---|---|---|
| **M0 Foundations** | Install rustup and pin the toolchain; workspace skeleton with every crate stub and the `xtask deps` DAG lint; licence files, `PROVENANCE.md`, `AGENTS.md` provenance rules; `tools/oracle` producing T0–T2 goldens; ADRs for every §7 decision; **OM P0 safety fixes** (§8) | CI green on 2 OSes; goldens committed with OM SHA; ADR-001..010 merged; OM fixes merged with regression tests and validated in one red-light session | 2–3 | Use a safer prototype |
| **M1 Headless core** | §5.1 | E1–E9 | 6–8 | CLI G-code export; inspect LMC streams |
| **M2 Machine runtime** | `ol-transport` (nusb, Recording), `ol-driver` (handle, session loop, latch, watchdog), `ol-drv-lmc`, `ol-sim` FakeLmc, conformance suite; `ol frame`, `ol mark --arm`, `ol hil`; HIL-0..2; **UI spike decision recorded** | T6 green; on hardware over 20 trials: stop ≤ 250 ms p99 measured from the trace (request → `WritePort(0)` done), 0 USB resets; a 20 s red-light frame completes; 10 mm box measures 10.0 ± 0.1 mm; `lmc-proto`/`lmc-sim` publishable | 4–6 | Frame and mark the MOPA from the terminal with the Rust engine |
| **M3 Desktop T0 (replaces the prototype)** | egui shell, canvas, layers and cut editor (Line/Fill, galvo timing at device level), laser window (Frame, Mark with confirm, STOP and Esc, status), `.olp` save/load, undo, SVG/DXF import, G-code export, galvo time estimate, travel ordering (vendored raygeo optimize), per-axis calibrate wizard; signed and notarised `.app` (D5) | Prototype parity checklist (R02 App. A plus R05 S1–S15) passes as kittest plus manual; 3 real jobs run from the app; 2 weeks of daily use without opening OM; OM tagged `python-final` | 8–10 | **Daily driver for the MOPA**; the prototype retires |
| **M4 GRBL live, `.lbrn2` read, scripting, Linux** | `ol-drv-grbl` (character counting, `$I/$$`, frame, jog, pause/resume, realtime stop), FakeGrbl; `rust-v1` G-code quality (path joining, arc fitting, ordering: RF takeaway 2); `.lbrn2`/`.lbdev` import (Y-up, second fixture verified); `rhai` with material-test generator; Linux AppImage plus udev | GRBL stop ≤ 100 ms on FakeGrbl; Rayforge-vs-OpenLaser comparison on the shared emulator; `.lbrn2` fixtures import with visual goldens; Linux build passes CI | 6–8 | Run GRBL machines live (HIL depends on D8); open LightBurn projects; script test grids |
| **M5 Galvo depth (T1)** | Sub-layers and global passes, per-layer timing override, wobble, `.cor` load (H5), 9-point lens wizard, image engraving (own dithers; MeerK40t run model, R06 §0.8), MOPA material test (frequency × pulse grids), presets library, variable text plus serial numbers and QR/DataMatrix, HIL-3/4, pause if H15 passes | `.cor` makes a 100 mm grid accurate to ≤ 0.2 mm without manual scale correction (or explains why not); dithered image goldens; material test grid marked on steel | 10–14 | LightBurn-Pro-level galvo work for his MOPA |
| **M6 Breadth (T1→T2)** | Windows (WinUSB helper through libwdi, `0x9980` clone detection), `.lbrn2` **write** plus lossless round-trip, Offset Fill, text with system and Hershey fonts, Ruida encoder plus FakeRuida (driver HIL only if D9 says yes), pages | Canonical-XML round-trip diff empty on fixtures; Windows HIL-5 | 10–14 | Share projects with LightBurn users; Windows users |
| **M7+ Parity (T2)** | Camera (galvo overlay is a differentiator, R02 §14), nesting (raygeo nest2d / sparrow), rotary and split marking, cylinder correction, extism plugins | Per-feature | Multi-year | — |

The prototype is replaced after M0–M3, which is **20–27 focused weeks**. Each milestone before that still ships something testable or usable.

---

## 6. Reuse, licensing, packaging, USB per OS

**Licence:** MIT OR Apache-2.0 (R03 §11). Luiz owns OM (MIT), so it can be relicensed freely.

| Source | Use | Rule |
|---|---|---|
| galvoplotter (MIT, © 2023 MeerK40t) | Port the LMC protocol logic into `ol-lmc`/`ol-drv-lmc` | Per-file header; notice in `THIRD_PARTY_NOTICES`. Fix its known quirks deliberately (R05 §1.11) |
| MeerK40t `balormk`, `.cor` reader, `ruida/`, `extra/lbrn.py` (MIT) | Secondary reference; `.cor` V1/V2 parsing; Ruida later | **Never port `clone_loader.py`**, which embeds vendor firmware (R06 §0.6). Detect PID `0x9980` and explain the situation |
| raygeo (MIT) | Vendor optimize/TSP, tabs, arc fit, nest2d | Strip the allocator; pin the commit; add Rust tests |
| raydriver, Rayforge (MIT) | Reference for GRBL cancel ordering and the `.lbrn2` importer | Port only with attribution |
| corfile2meerk40t (MIT) | `.cor` byte layouts and the scan-based calibration idea | Attribution |
| baseCreator `lightburn.rs` (MIT/Apache) | Cross-check for `.lbrn2` parsing | Review before porting; probably AI-generated (R03 §8) |
| NicoPowers GP30, earlynerd LaserEngraver (no licence) | **Facts only** | — |
| balor, LaserGRBL, UGS, potrace, clayfrocker97/OpenLaser (GPL) | **Never** copied or paraphrased; facts only | Noted in the PR when consulted |
| Hershey fonts | Bundle with the NBS/Hurt acknowledgements (R06 G8) | NOTICE file |

`AGENTS.md` tells executors: cite source URL and commit for any ported logic, never paste from GPL or unlicensed repos, and never add vendor blobs or EZCAD files. cargo-deny allows MIT, Apache, BSD, ISC, Zlib, BSL, MPL-2.0 and denies GPL, AGPL and static LGPL (R04 §16).

**Packaging:**
- `cargo-packager` builds `.app` and `.dmg` (aarch64 first).
- `codesign --options runtime` with a Developer ID, then `notarytool`, then staple.
- **No USB entitlement is needed** outside the App Sandbox. Camera needs `com.apple.security.device.camera` plus `NSCameraUsageDescription`, and only from M7 (R06 §0.9).
- No App Store and no sandbox. Updates start as a GitHub-release check; Velopack when distribution widens (R04 §11).

**USB by OS:**

| OS | Approach |
|---|---|
| macOS | nusb over IOKit, no setup. Call `set_configuration(1)` if needed (R06 spot-check 6) |
| Linux | udev rule (from OM's `99-openlaser.rules`) installed by the `.deb` post-install script; the AppImage ships a `pkexec` helper |
| Windows | WinUSB is unavoidable; nobody coexists with EZCAD (R06 C12). Ship a separate `ol-driver-helper.exe` on libwdi (LGPL-3, separate process) with "Install" and "Restore EZCAD driver" actions; recognise PID `0x9980`. If a board must stay on libusbK, the `rusb` fallback is the only option, and only if a user needs it (R06 C8) |

---

## 7. Risks and decisions

### 7.1 Risk register (top 10)

| # | Risk | L | I | Mitigation |
|---|---|---|---|---|
| 1 | **Platform over-build:** abstractions outrun use, the first usable app slips past 7 months, momentum is lost | H | H | Rule 5 (two implementations before freezing); every milestone ships a CLI or test result; hard week caps per milestone with an M-gate review; anything not needed by M3 lives only as an ADR |
| 2 | LMC flow-control semantics (H2/H13) differ from galvoplotter's assumptions, causing reset storms or wedges | M | H | Never write unless READY; no auto-reset; red-light HIL-1 before any emission; trace transport; fake board sweeps buffer depth and full-buffer behaviour |
| 3 | Stop latency regresses above 250 ms | M | H | Fake-board CI budget of 100 ms p99; hardware measurement on every release; the stop path is reviewed by Claude/Opus and never delegated unreviewed |
| 4 | Stale incremental cache burns the wrong geometry | L | H | Cold build plus hash compare at ARM; incremental-equals-cold proptest |
| 5 | egui canvas or `muda` fails at scale; egui/egui_tiles/wgpu version lockstep (R06 C9) | M | M | Spike gates in M1–M2; `ol-app` serde boundary keeps Plan B Tauri open; pin versions; quarterly upgrades |
| 6 | Document model mistakes force painful migrations | M | M | `format_version` plus migration functions from day one; `Extras` bag; `.lbrn2` fixtures early (LightBurn 1.5.04 is on this Mac, R06 G10) |
| 7 | AI executors introduce GPL code or vendor blobs | M | H | Provenance rules in `AGENTS.md`, cargo-deny, review checklist item, no unlicensed sources in specs |
| 8 | Single-maintainer and pre-1.0 crates churn or die (i_overlay, cavalier_contours, dxf, nusb 0.2, usvg) | M | M | Wrap them behind `ol-*` APIs; pin; vendor if abandoned |
| 9 | MOPA parameter semantics are wrong (pulse ns, PRR limits, delay units: H6–H9) | M | M | Refuse values outside the known table; flag "UNVERIFIED encoding" in the confirm UI until HIL-3; EZCAD capture |
| 10 | Name or trademark collision forces a late rename | H | L | Decide by M2; `ol-` internal prefixes; reserve crate names |

### 7.2 Decisions for Luiz (recommendation in bold)

| # | Decision | Recommendation |
|---|---|---|
| D1 | Licence | **MIT OR Apache-2.0** with the provenance policy |
| D2 | Public name | **Rename before M3** (candidates: Fieldmark, Beamwright, Markwright; crates.io free on 2026-10-23); keep "OpenLaser" privately until then |
| D3 | Repository | **New repo for the Rust workspace**; OM repo stays as the oracle, pinned by tag |
| D4 | GUI | **egui**, conditional on the §3 spike; Plan B Tauri |
| D5 | Apple Developer Program ($99/yr) | **Yes, by M3**, for notarisation; ad-hoc signing until then |
| D6 | Fix OM safety bugs now and freeze its features | **Yes** (§8) |
| D7 | Can a Windows PC with EZCAD and the original `.cor` be used for HIL-3 capture? | **Yes if available**: it answers H5–H9 and H20 at once |
| D8 | Which GRBL machines does Luiz physically have access to? | Decides whether M4 gets hardware validation or only emulator validation |
| D9 | Ruida in the 1.0 roadmap | **Architecture yes; driver only if a Ruida machine is available for HIL**; otherwise defer |
| D10 | Remote or scripted job start | **Never** (human ARM only) |
| D11 | Use the installed LightBurn 1.5.04 to make `.lbrn2` fixtures | **Yes**, if its licence or trial is valid |
| D12 | Upstream contribution to Rayforge | **Not now**; offer `lmc-proto`/`lmc-sim` after M2 |
| D13 | Require framing before marking on galvo | **On by default**; a per-profile opt-out is shown in the confirm summary |
| D14 | Hidden layer with Output on | **Refuse** ("show it or turn Output off") |
| D15 | Platform order | **macOS until M3, Linux M4, Windows M6** |

---

## 8. The Python prototype in the meantime

**P0 fixes, in M0, through `/route-implementation`.** Codex executes; Claude reviews. Each fix gets a regression test, and a red-light-only hardware check before the next marking session.

1. **3 s wait caps** (R05 §0.1).
   - Replace `wait_until`'s fixed 3 s deadline for *normal* waits with a progress-aware deadline: estimated job time × 1.5 + 10 s, extended while status changes.
   - Keep the stop-event checks exactly as they are; the 0.18 s stop fix must survive.
   - On timeout, **abort**; never write a packet or close MO.
   - Test: the R05 20 s-BUSY fake must run to completion with no MO close before BUSY clears.
2. **First-jump elision** (G4). Before each job, reset the controller's cached last position to a sentinel, or emit an explicit `list_jump` to the first point. Test: the `dot_test` stream contains a `JumpTo` before the first `MarkTo`.
3. **Confirm dialog shows Frame values** (G1, G2).
   - Build the confirm summary from the exact layer list sent to `/api/mark`.
   - Send `arm` from a real checkbox.
   - Clear the ARM text after each job (single use).
4. **Hidden layers with Output on burn unframed** (G3). Refuse to mark when any emitting layer is hidden. That uses one predicate shared by framing and marking.
5. **Speeds below 3.61 mm/s encode to 0** (G5). Refuse when the encoded `MarkSpeed` is < 1, and warn when the quantisation error is > 5 %.
6. **Cheap extras in the same PR:**
   - refuse unknown `layer_id` instead of dropping it (G7)
   - fix the `group_by_layer` crash on 3-coordinate points (G10)
   - pin `galvoplotter==0.2.0` (R05 §6)

Expected cost: 3–5 days including review.

**After the fixes:**
- Tag `oracle-v1`.
- Freeze features: bug and safety fixes only, with no new UI or machines.
- Use the prototype for real work until M3 exits, then tag it `python-final`.
- HIL-1 answers (H1, H2, H13, H14) also go back into the prototype's README as documented limits, because Luiz keeps using it until M3.
