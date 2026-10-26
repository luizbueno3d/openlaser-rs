# OpenLaser in Rust: the plan

*2026-10-23, revision 2. Final plan for Luiz. Base: proposal A (galvo-first vertical slice), the winner in both judgements, with every graft the judges asked for (Appendix B). Revision 2 answers the red-team review [RT]: all three critical findings are fixed, every important finding is fixed or explicitly scoped, and the minor ones are taken. The Review log at the end lists each change. Nothing was installed, built or run against hardware to write this.*

*How to read it:*
- *Citations like "[05 §1.10][R05]" mean research report 05, section 1.10. "[RT I5]" means red-team finding I5.*
- *"OM" is the Python prototype at `~/AI/OpenLaser/OpenMopa` @ `6aed8ff`. "GP" is galvoplotter 0.2.0 in OM's venv.*
- *B1–B11 are OM's known safety bugs (§11). H0–H29 are hardware questions and HIL-x are supervised hardware sessions (§5.2).*
- *Effort is in **focused weeks**: Luiz directing Claude (planner and reviewer) and Codex/OpenCode (executors). Calendar time runs 1.5–2× longer.*

[R01]: research/01-rayforge-source-deep-dive.md
[R02]: research/02-lightburn-feature-inventory.md
[R03]: research/03-open-source-prior-art.md
[R04]: research/04-rust-ecosystem-survey.md
[R05]: research/05-hardware-protocols-and-test-inputs.md
[R06]: research/06-critic-gaps-and-corrections.md
[RF]: rayforge-test/REPORT.md
[PA]: design/proposal-A-galvo-vertical-slice.md
[PB]: design/proposal-B-platform-first.md
[PC]: design/proposal-C-strangler-ecosystem.md
[JP]: design/judge-product-safety.md
[JE]: design/judge-engineering-risk.md
[RT]: design/redteam-review.md

---

## 0. Executive summary

### Recommendation

**Build OpenLaser in Rust, galvo-first.** Take the shortest safe path from a design to a burn on the JPT M7 through a Rust driver, and only then widen. M1's crates are the final architecture, filled in only for the galvo case, so nothing is thrown away.

**Should we still build our own, given Rayforge? Yes, for the MOPA.**
- **It cannot drive your machine.** There is no galvo or LMC code in Rayforge's main branch, in 1.12.0, in the 1.13 betas or in the addon registry. The one attempt, the maintainer's own PR #288, never sends a packet, clamps out-of-field points and marks at half scale. It and the galvo issues were closed by the stale bot ([01 §2.6][R01], [06 §0.1][R06]).
- **A fork or an addon is a bad trade.** A fork means owning 266k lines of Python plus 106k of Rust from a project with a bus factor of 1 that lands about 1,100 commits a quarter. An addon needs upstream core changes and inherits advisory safety ([01 §1.4, §3.1, §7, §8.2][R01]).
- **The gap is real, and it is the product.** No licensed, marking-capable Rust LMC driver exists ([06 §0.2][R06]). LightBurn-class parity takes years ([06 G9][R06]), so M1 stays narrow.
- **For the gantries, use Rayforge until M4, but only after a bring-up checklist on each machine** (§5.3). Its streaming, stop and framing worked against a fake GRBL, and its G-code beats the prototype's: 1,337 lines against 4,298 ([REPORT][RF]). It has never been tested on real serial hardware, and its safety is advisory.

**Safety posture.**
- Refusals, never warnings.
- The confirmation is decoded from the bytes about to be sent.
- Galvos always frame before marking, and only a human at the GUI or a TTY can ARM.
- The work area is the lens's rated 200 mm field, not the galvo's 236.68 mm reach ([RT C1]).
- The software stop is never the primary safety device. The E-stop and key switch are, and the first hardware session checks that they cut the source (H0).

### What happens first

- **M0, foundations and a safer prototype (3 weeks).**
  - Patch the prototype's eleven known safety bugs (B1–B11, §11), and add the job export and a board lock.
  - Validate on the M7, including a low-power hatch of 30 s or more.
  - Install the toolchain. Set up the repo, CI, T0 encoders, a FakeLmc skeleton and the agent guard rails (§3.6).
- **M1.1, Rust red light (weeks 1–5).** A byte-exact LMC encoder and disassembler, a single-owner `nusb` driver with a stop of 250 ms or less, the emulator, and three red-light sessions.
- **M1.2, first Rust burn (weeks 6–7).** `ol run --job <export>` takes the prototype's exact payload. It frames the job, shows a confirmation decoded from the bytes, accepts ARM only on a TTY, and marks.
- **M1.3, Rust GUI (weeks 8–11).** Import SVG, DXF or the prototype's workspace; Line, Fill and Frame-only layers; Frame; Mark; STOP.

Out of M1: G-code, images, text, `.cor` upload, pause, zip projects and undo (moved to M2), Windows/Linux, notarisation.

**M1 is done when** (the full list is in §7):
- every known galvo bug (B1–B10) has a mutation-checked test;
- 20 of 20 traced hardware stops take 250 ms or less with zero USB resets, and filmed stops agree;
- a low-power hatch of 30 s or more completes;
- a 50 mm square matches Python within ±0.1 mm;
- a named list of prototype workflows runs in Rust.

### Timeline at a glance

The first draft's 7-week M1 was about half of what its scope needs ([RT I15]). This is the re-baseline.

| Milestone | Focused weeks (cumulative) | Calendar if started 2026-10-26 | You can then |
|---|---|---|---|
| M0 Foundations + safer prototype | 3 | late Nov – early Dec 2026 | Use the prototype without its known bugs |
| M1.2 First Rust burn (CLI) | 10 | Feb – mid-Mar 2027 | Design in Python, burn through the Rust driver |
| M1 MOPA slice (GUI) | 14 | late Mar – mid-May 2027 | Run MOPA jobs from the Rust app |
| M2 Prototype retired | 21 | Jun – mid-Aug 2027 | Do everything the prototype did, plus test grids and signed installs |
| M3 Galvo depth | 28 | mid-Aug – Nov 2027 | Use `.cor` correction, photos, serial plates, interlocks |
| M4 Gantry live + Windows/Linux | 34 | late Oct 2027 – mid-Feb 2028 | Drive the GRBL machines; share builds |
| M5 Design suite + `.lbrn2` | 42 | Jan – Jun 2028 | Design in the app; open LightBurn projects |

### Decisions for you

| # | Decision | Recommendation |
|---|---|---|
| D1 | Build our own instead of extending Rayforge | **Yes, galvo-first.** Also tell me whether your LightBurn 1.5.04 licence covers galvo. If it does, it is an EZCAD-free reference for H6–H9 and for `.lbrn2` fixtures ([06 G10][R06]) |
| D2 | Plan shape | **A plus grafts plus the red-team fixes**, as written here |
| D3 | Licence | **MIT OR Apache-2.0**, plus the §8 provenance rules ([03 §11][R03]) |
| D4 | Name | **Rename before the repo goes public (D17).** Shortlist "Kerfwright": crates.io and GitHub were free on 2026-10-23; trademark not yet checked ([PC §0][PC]). "OpenLaser" is taken twice ([03 §1.6][R03]) |
| D5 | Prototype | **Freeze it now.** Only the §11 patches are allowed (safety fixes, job export, board lock). Archive it at M2 |
| D6 | GUI | **egui 0.37 + egui_tiles 0.18** if the spike passes; Tauri only on NO-GO |
| D7 | Hardware time | **8–10 supervised sessions of 2 h or less across M0–M2**: seven planned, up to three in reserve (§5.2) |
| D8 | Frame before Mark | **Always required on galvos; it is not a setting.** Per machine on gantries |
| D9 | Speed quantisation | **Keep the requested speed, encode it once to whole units, show the actual speed, and refuse 0 units.** Never re-encode a snapped value ([RT I11]) |
| D10 | Apple Developer Program ($99/yr) | **At M2** |
| D11 | Original EZCAD PC | **If you still have it** (or a disk image): copy the `.cor`, `markcfg*` and `.ezd` files in M0, and do one USB capture session in M3 (H6–H9, H20) |
| D12 | Gantries | **Tell me which GRBL machines you can reach, and what drives them today.** That decides how urgent the §5.3 checklist is and whether M4 validates on hardware |
| D13 | Sign `DIVERGENCES.md` (intentional differences from the Python bytes) | **At M1.1** |
| D14 | Rayforge contribution | **None before M3**; publish `lmc-proto`/`lmc-sim` later |
| D15 | Clone boards (PID `0x9980`) | **Detect and explain; never load firmware** ([06 §0.6][R06]) |
| D16 | Telemetry | **None.** Local logs plus an "Export diagnostics" zip |
| D17 | Repo visibility | **Private until M2, then public under the new name.** Until then heavy CI runs on Linux (§6.3) |
| D18 | Windows code signing | **Decide before M4** (§9) |
| D19 | Bench rules | **Agree to them in M0.** The board's USB is unplugged whenever executors are working. The E-stop and key switch are reachable from the Mac. Never a self-hosted CI runner on this Mac (§3.6) |

---

## 1. Why build, compared with Rayforge, LightBurn and MeerK40t

| Option | What it gives you today | Why it is not the answer for the M7 |
|---|---|---|
| **Rayforge 1.12** (MIT, Python/GTK + Rust cores) | GRBL streaming, framing, material test grid, 3D preview, `.lbrn2` import | No galvo anywhere. The addon route needs upstream changes. Advisory safety: the bounds dialog always offers "Proceed", and a failed check proceeds silently ([01 §3.1][R01]). Bus factor 1. Not notarised. Homes on start. Found no camera on this Mac ([REPORT][RF]) |
| **LightBurn 2.2 Pro** ($199 + $40/yr) | Drives EZCAD2-class boards on macOS with no driver; mature MOPA Q-pulse settings | Closed. No Linux since 1.7. Out-of-bounds shapes get a *warning*, not a refusal. The REST API can't start jobs. No galvo camera. USBLMCV4 support unconfirmed ([02 §0, §20, §21][R02]) |
| **MeerK40t** (MIT, Python/wx) | A working JCZ driver (`balormk`), `.cor` support, emulators | Maintenance mode; Python packaging; not galvo/MOPA-first ([03 §3][R03]) |
| **OM prototype** (MIT, yours) | Proven on the M7, including a 0.18 s stop | Eleven known safety bugs (B1–B11, §11); HTML-in-Python UI; SVG import has no curves or transforms ([05 §2][R05], [RT C1]) |

**Plain answer.** If the goal were only "mark today with polish", buy LightBurn Pro. OpenLaser is justified by a combination no tool offers:
1. A licensed, open LMC driver with **refusals instead of warnings** ([02 §20.6][R02]).
2. Galvo/MOPA depth: a centred field, delays, `.cor` correction, pulse tables, colour presets.
3. One signed Rust binary for macOS, Linux and a Pi. LightBurn dropped Linux, and Rayforge ships 154–452 MB bundles ([01 §5][R01]).
4. Hardware truth you already own: a prototype proven on the board, and the board itself.
5. A codebase one person can direct with AI executors, because safety-relevant output is checked against byte-exact goldens.

**What we borrow, without ever depending on it:**
- **Rayforge:** its typed ops IR, its separate machine-transform stage, its typed encoded artifact, and raydriver's stop ordering. Its `.lbrn2` importer serves as the format reference ([01 §8.1][R01]).
- **raygeo:** selected files only, vendored with its allocator stripped ([06 §0.4][R06]).
- **galvoplotter:** the LMC protocol and its own MIT `.cor` readers. **MeerK40t:** the galvo raster model ([06 §0.8][R06]).

**What would change the answer:**
- If MOPA work stopped being your priority, Rayforge plus LightBurn would be cheaper than building.
- If Rayforge shipped a working galvo driver. Its galvo issues were closed by a stale bot, not by a decision, and the maintainer wrote PR #288 himself, so this is plausible. Watch it ([01 §2.6][R01]).
- If your LightBurn licence already covers galvo (D1), "mark today with polish" is already paid for.

---

## 2. Product scope and tiers

Report 02 holds the LightBurn catalogue and the T0–T3 tiers ([02 §19][R02]). This plan maps them as follows:

| 02 tier | Delivered by |
|---|---|
| **T0** (replace the prototype) | M1 (galvo core, including red-dot offset/scale, 02 GDEV-04) + M2 (G-code export, ordering, calibration wizard, signed DMG). T0-stretch: material test grid in M2; sub-layers, QR and serial numbers, images and door/stop inputs in M3 |
| **T1** (credible alternative) | M3 (galvo depth) + M4 (GRBL live, Windows/Linux) + M5 (design suite, `.lbrn2`) |
| **T2** (parity) | M6+, by demand. Ruida only if a Ruida machine is available for testing |
| **T3** (beyond) | M6+: galvo camera alignment, `.ezd` import, headless Pi, MOPA colour libraries ([02 §20][R02]) |

**Scope rules:**
- **Galvo before gantry.**
- **Refuse; never clamp, truncate or "proceed anyway". Nothing is dropped silently:** anything in the design that will not be burned is either a refusal or a numbered notice in the confirmation (§3.6).
- **Anything outside the current milestone goes to a parking-lot file.** Design tools wait for M5.
- **Layer parameters live in separate `galvo` and `gantry` blocks.** This is a deliberate divergence from LightBurn's shared speed and power ([02 §7.7][R02]). The Rayforge test caught OM leaking MOPA defaults (F15000, `M4 S1000`) into a diode job ([REPORT, takeaway 1][RF]); that is bug B11. The M5 `.lbrn2` importer therefore gets an explicit mapping layer: shared values go into the block for the file's device kind.

---

## 3. Architecture

### 3.1 Crate map

The project is a new Cargo workspace (edition 2024, resolver 3):
- `rust-toolchain.toml` pins the stable version installed in M0 (1.95 or later). `[workspace.dependencies]` pins exact versions, upgraded quarterly.
- Every crate is `publish = false` with `#![forbid(unsafe_code)]`. Every profile sets `panic = "unwind"` explicitly (§3.5).
- A crate is created only when its milestone needs it; there are no empty stubs.

| Crate | Responsibility | From |
|---|---|---|
| `ol-units` | Newtypes `Mm`, `MmPerS`, `Hz`, `Ns`, `Us`, `Percent`, `GalvoUnits`; Python-compatible rounding (`round_ties_even` for `round()`, `trunc` for `int()`, [05 §0.6][R05]) | M0 |
| `ol-types` | **Leaf shared types:** `Fingerprint`, `MachineId`, `SessionId`, `EmissionSummary`, `FrameReceipt`, refusal codes, and the opaque `ArmToken` (checking only) | M1 |
| `ol-geom` | Polylines, `kurbo` flattening, affine transforms, bbox, join/close/dedupe, hatch (port of `raster.py`); ordering in M2 | M1 |
| `ol-doc` | Document/Object/Layer; versioned JSON in M1; zip project file with migrations and snapshot undo in M2 | M1 |
| `ol-machine` | Machine profiles, calibration, `.ini`/`calibration.json` import; `.cor` reading in M2. **No I/O** | M1 |
| `ol-plan` | Document + machine → `JobPlan`; galvo time model; fingerprint | M1 |
| `ol-safety` | **Every hard refusal.** Sole constructor of `EmitPlan`/`FramePlan`. Depends on no encoder and no driver | M1 |
| `ol-arm` | `mint()`: the only code allowed to create an `ArmToken`. Depends only on `ol-types` | M1 |
| `ol-driver` | `Backend` trait, driver thread, `StopLatch`, watchdog, dead-man rule, panic guard, signal handling, events. Depends only on `ol-types` | M1 |
| `ol-lmc` | Pure LMC protocol: field transform, encoders, `LightingList`/`MarkingList`, packetiser, status parser, **disassembler + `EmissionSummary`**, session state machine over `LmcTransport`; implements `Backend`; a **test-only** `py-compat` feature | M1 |
| `ol-lmc-usb` | `LmcTransport` on `nusb` 0.2.8; finds `0x9899`; refuses `0x9980` with an explanation | M1 |
| `ol-lmc-emu` | FakeLmc: virtual clock, fault injection, emission recorder, tiny-skia renderer | M0 |
| `ol-hw-raw` | Bench binary for the hardware items that must break driver rules (H13, H23). Red light only; never a dependency of `ol`, never shipped | M1 |
| `ol-io` | SVG (`usvg`; physical units required), DXF (`dxf`), OM workspace and job payload; `.lbrn2` and exports later | M1 |
| `ol-gcode` | GRBL emitter matching `gcode.py` (M2); streaming sender (M4) | M2 |
| `ol-app` | `AppState`, the `Encoded` enum, serde `Command`/`Event`, orchestration. **No egui and no `ol-arm`** | M1 |
| `ol-ui` | eframe 0.37, `egui_tiles` 0.18, canvas, `rfd`; uses `ol-arm` | M1 |
| `ol-cli` | `detect`, `status`, `plan`, `dump`, `frame`, `run`, `bench`, `golden-check`, `hw <item>`; uses `ol-arm` | M1 |
| `xtask`, `tools/golden/` (Python) | Golden regeneration, dependency lint, packaging | M0 |

**Layering, with no cycles** ([RT C2]):
- `ol-types` depends only on `ol-units`.
- `ol-geom`, `ol-machine`, `ol-driver` and `ol-arm` depend only on those two leaf crates plus third-party crates.
- `ol-doc` depends on `ol-geom`. `ol-plan` depends on `ol-doc` and `ol-machine`. `ol-safety` depends on `ol-plan` and `ol-machine`.
- `ol-lmc` depends on `ol-safety`, `ol-machine` and `ol-driver` (it implements `Backend`). `ol-lmc-usb` and `ol-lmc-emu` depend on `ol-lmc`.
- `ol-app` depends on everything above except `ol-arm`. `ol-ui` and `ol-cli` depend on `ol-app` plus `ol-arm`.

**Dependency rules.** `cargo-deny` `[bans]` (with `wrappers`) and `xtask deps` over `cargo metadata` enforce them:
- Only `ol-lmc-usb` may use `nusb`.
- Only `ol-ui` may use egui or wgpu.
- No crate below `ol-app` may use tokio.
- `ol-safety` may not depend on any encoder or driver crate.
- **Only the two human-input crates can mint an ARM token:**
  - `ArmToken` lives in `ol-types` with private fields. Its one constructor is banned workspace-wide by clippy `disallowed-methods`. The single `#[allow]` sits in `ol-arm/src/mint.rs`, and xtask greps for any other.
  - xtask asserts that the reverse-dependency set of `ol-arm` is exactly {`ol-ui`, `ol-cli`}. Neither of them may pull in a server, scripting or FFI crate (axum, hyper, tungstenite, rhai, extism, pyo3).
  - No Cargo feature is involved, so feature unification in `--workspace` or `--all-features` builds cannot widen it.
- **What that enforces, and what it doesn't.** No accidental or AI-written path outside `ol-ui` and `ol-cli` can create a token, and a future LAN server built on `ol-app` cannot ARM. It does not stop hostile code inside the same process, or an agent typing at a real terminal; §3.6 covers that case.
- `py-compat` (§6.1) is enabled only through dev-dependencies, which resolver 3 does not unify into normal builds. It `compile_error!`s without `debug_assertions`.

**Rule 5 (from B): a trait freezes only after two real implementations exist.** `Backend` needs LMC (M1) and GRBL (M4). The `Encoder` needs LMC (M1) and G-code (M2). FakeLmc doesn't count toward `Backend`: it is a board model behind `LmcTransport`.

### 3.2 Core data model

```rust
// ol-doc: mm, Y-up; a galvo's origin is the field centre
pub struct Document { pub format_version: u32, pub objects: Vec<Object>, pub layers: Vec<Layer>, pub notes: String }
pub struct Object   { pub id: ObjectId, pub name: String, pub layer: LayerId, pub transform: kurbo::Affine, pub geometry: Geometry }
pub enum Geometry   { Paths(Vec<kurbo::BezPath>) /* M3: Bitmap, Code(QrSpec), Text */ }
pub struct Layer    { pub id: LayerId, pub name: String, pub color: Rgb, pub mode: LayerMode,
                      pub output: bool, pub show: bool, pub passes: u16,
                      pub galvo: GalvoCut, pub gantry: GantryCut /* M3: sub_layers, global_passes */ }
pub enum LayerMode  { Line, Fill(FillSpec), FrameOnly }     // "disabled" == output: false
pub struct GalvoCut { pub speed: MmPerS /* as requested; encoded once, §3.3 */, pub power: Percent, pub frequency: Hz,
                      pub q_pulse: Option<Ns>, pub timings: Option<GalvoTimings> }
pub struct GantryCut { pub speed: MmPerS, pub power: Percent, pub air_assist: bool, pub power_mode: Option<PowerMode> }

// ol-machine
pub struct MachineProfile { pub id: MachineId, pub name: String, pub kind: Kinematics, pub limits: JobLimits,
    pub layer_defaults: LayerDefaults /* per kind, never shared */, pub checklist: Vec<String> }
pub enum Kinematics { Galvo(GalvoMachine), Gantry(GantryMachine) }
pub struct GalvoMachine {
    pub board: LmcBoardFilter,   // 0x9588:0x9899 (+ serial); 0x9980 refused
    pub encoding_field: Mm,      // nominal 200. F = encoding_field × cal.scale (236.68) is used ONLY to convert units
    pub work_area: FieldRect,    // default: the lens's rated field, ±100 mm. May shrink; grows only to what H27 cleared
    pub cal: FieldCalibration,   // per-axis scale (1.1834 imported), offset, swap, invert; M3: .cor-derived field
    pub mirror: Mirror,          // persisted here, not in browser localStorage (05 §1.7)
    pub red_dot: RedDotCal,      // offset + per-axis scale, applied to LightingList only; identity until H26
    pub source: LaserSource,     // MopaFiber { pulse_table, freq range, max_power, prr: Option<PrrTable> }
    pub timings: GalvoTimings,   // laser on/off/polygon 100 µs, jump 8 µs (GP defaults)
    pub first_jump: FirstJump,   // long jump delay 200 µs + 1 ms settle until H3/H19 (§3.3)
    pub jump_speed: MmPerS, pub light_speed: MmPerS, pub mo_settle: Us,
    pub ports: PortMap,          // laser bit 0, red light bit 8; M3: door/foot inputs
    pub init: LmcInitParams,     // 05 §1.4 words, expert view only
    pub run_ahead: RunAhead,     // ≤ 3 packets AND ≤ 2 s of estimated queued emission; never < 1 packet
}
pub struct GantryMachine { pub bed: (Mm, Mm), pub origin: Corner, pub dialect: GrblDialect, pub max_speed: MmPerS,
    pub frame_before_mark: bool, pub home_on_connect: bool /* default false */, pub link: GantryLink /* ExportOnly | Serial | Tcp */ }
```

**Coordinates and the work area** ([RT C1, I10]).
- **Encoding.** `round_ties_even(0x8000 + v·0xFFFF/F)`, then `u16::try_from`, never `as u16`, which saturates silently. `65536/scale` is used only to derive a field from a `.cor` file ([06 C11][R06]).
- **Acceptance is bounded by the work area, not by F.** OM accepts anything within ±F/2 = ±118.34 mm (`live.py:78-81, 264-277`; callers `ui.py:2745, 2825`). That is up to 18 mm per side beyond the lens's rated 200 mm field on a machine profiled as 200×200 ([05 §3.1][R05]). What the optics do out there (distortion, clipping, back-reflection) is UNVERIFIED; H27 checks it on red light. Rust refuses anything outside `work_area`, and so does OM after the B10 patch (§11).
- **The +F/2 edge.** Exactly at +F/2 the formula gives 65535.5, which rounds to 65536 (checked at F = 200 and 236.68). OM clamps it to 65535 (`live.py:250-254`), so 05 §1.7's "dead clamp" is in fact live at that edge. Rust refuses the point instead, and T0 lists this as a divergence. With the default work area inside F/2 it cannot arise. If a calibration ever makes F/2 smaller than the work area, the work area shrinks to the encodable range and the UI says so.

### 3.3 Job pipeline and ops IR

```
Document ─plan()─▶ JobPlan ─ol_safety::validate()─▶ EmitPlan + FramePlan ─encode─▶ LmcJob + LightingJob
                                                      disassemble(bytes) ─▶ EmissionSummary (shown at confirm)
driver.frame(LightingJob) ─▶ FrameReceipt (after ≥ 1 fully acknowledged pass)
human in ol-ui / ol-cli + EmissionSummary + FrameReceipt ─ol_arm::mint()─▶ ArmToken ─▶ DriverHandle::run(job, token)
```

```rust
// ol-types (leaf)
pub struct Fingerprint([u8; 32]);                    // blake3(plan + calibration)
pub struct EmissionSummary { pub fingerprint: Fingerprint, pub machine: MachineId, pub rows: Vec<PassRow>,
                             pub notices: Vec<Notice>, pub repeats: u16, pub estimate: Duration, pub crash_bound: Duration }
pub struct FrameReceipt    { pub fingerprint: Fingerprint, pub session: SessionId, pub full_passes: u32, pub at: Instant }
pub struct ArmToken { fingerprint: Fingerprint, machine: MachineId, session: SessionId, expires: Instant, nonce: u64 }
impl ArmToken {
    pub fn check(&self, fp: &Fingerprint, m: &MachineId, s: SessionId, now: Instant) -> Result<(), Refusal>;
    #[doc(hidden)] pub fn issue(/* … */) -> ArmToken;   // clippy disallowed-methods everywhere except ol-arm
}
// ol-arm: depends only on ol-types; reverse dependencies exactly {ol-ui, ol-cli}
pub fn mint(s: &EmissionSummary, f: &FrameReceipt, human: HumanConfirm) -> Result<ArmToken, Refusal>;
// ol-plan
pub struct JobPlan   { pub machine: MachineId, pub sections: Vec<Section>, pub frame: Vec<Polyline>,
                       pub estimate: Duration, pub fingerprint: Fingerprint }
pub struct GalvoCutQ { pub requested: MmPerS, pub speed_units: u16 /* ≥ 1; the only speed the encoder reads */,
                       pub actual: MmPerS /* display only */, pub mark_current: u16, pub q_period: u16,
                       pub pulse: Ns, pub timings: GalvoTimings }
// ol-safety: validate() is the only constructor
pub struct EmitPlan  { sections: Vec<Section>, fingerprint: Fingerprint, notices: Vec<Notice> }
pub struct FramePlan { paths: Vec<Polyline>, fingerprint: Fingerprint }   // has no power fields
// ol-lmc
pub struct LightingList(ListBuf);  // jump_to/light_to only: cannot express MarkTo, MO open, LaserOnPoint
pub struct MarkingList(ListBuf);   // built only from &EmitSection
pub struct LmcJob { pub prelude: Vec<Single>, pub packets: Vec<Box<[u8; 3072]>>, pub postlude: Vec<Single>,
                    pub packet_est: Vec<Duration>, pub fingerprint: Fingerprint, pub emits: bool }
pub fn disassemble(job: &LmcJob) -> Listing;
pub fn emission_summary(job: &LmcJob, m: &GalvoMachine) -> EmissionSummary;  // decoded from the bytes
// ol-app
pub enum Encoded { Lmc(LmcJob), Gcode(GcodeProgram) }                         // typed, never lossy (01 §8.2.4)
```

`mint` refuses unless all of these hold:
- the summary and the receipt share a fingerprint;
- the receipt is from this session, has at least one full pass and is at most 10 minutes old;
- the human typed the expected text.

The driver repeats every check against its **own** receipt record, then spends the nonce.

**Encoder rules.** The goldens enforce each one.
- **Rounding.** `round_ties_even`/`trunc` exactly as Python does it, so 30 % power encodes to 1228, not 1229.
- **First jump** ([RT I5]).
  - An explicit `JumpTo` opens every list. Until H3 and H19 say otherwise, it uses the long jump delay (200 µs, GP's own `delay_jump_long`), followed by a 1 ms settle `DelayTime(100)` before the first `MarkTo` or light-on.
  - Reason: the mirrors' real position is unknown (H3), and the Test Dot's first jump would otherwise get `dist` 0 and the 8 µs delay.
  - The `dist` word is computed as GP does. This is listed in `DIVERGENCES.md`; `py-compat` keeps GP's bytes.
- **Speed** ([RT I11]).
  - The requested speed is encoded once to whole units with `trunc`. It is never re-encoded from the displayed actual speed: storing `u/k` as a float and re-truncating loses a unit in roughly 13–20 % of (units, field) pairs, and turns 1 unit into 0 in about 29 % of random fields.
  - F = 236.68 happens to be clean, so a proptest sweeps F ∈ [100, 300] mm.
- **Coordinates.** `u16::try_from` after rounding; a refusal on overflow (§3.2).
- **Parameters.** The parameter cache resets at each list start, so every list carries the full parameter block.
- **Zero delays.** A delay of 0 keeps the `0x8000` sign word until H9 says otherwise.
- **List structure.** M1 keeps one list per layer-pass. Merging into a single list waits until both **H21** (do parameter changes mid-list take effect?) and H2 are answered ([JE §5][JE]).
- **Packets.** Only one list transfer is ever in flight, because each packet is followed by `SetEndOfList`, as in GP. The packetiser may close a packet early, padded with `0x8002`, only after H28 shows mid-list padding is harmless.

**Confirmation from the bytes.** The GUI dialog and the CLI summary are rendered **only** from `emission_summary()` over the packets that will be sent.
- **Per layer-pass:** power %, kHz, ns, *encoded* mm/s, passes, bounds against the work area, time and the short fingerprint.
- **Per job:** the repeat count, numbered notices (for example, open shapes left unhatched), and the post-crash emission bound (§3.5).

An encoder bug therefore shows on screen before the burn ([PB §2.4][PB]).

### 3.4 Driver and transport traits

```rust
// ol-lmc: the only USB-shaped seam. Implemented by ol-lmc-usb (nusb), FakeLmc and Recording<T>
pub trait LmcTransport: Send {
    fn submit_single(&mut self, cmd: [u16; 6], read: bool) -> Result<(), XferError>;
    fn poll_single(&mut self, slice: Duration) -> Result<Option<Option<[u16; 4]>>, XferError>; // ≤ 20 ms; abandonable
    fn submit_packet(&mut self, pkt: &[u8; 3072]) -> Result<(), XferError>;                    // at most one in flight
    fn poll_packet(&mut self, slice: Duration) -> Result<Option<Completion>, XferError>;        // wait_next_complete(≤ 20 ms)
    fn cancel_all(&mut self);     // last resort: only when an in-flight packet overruns 20 ms during a stop (H23)
    fn drain_replies(&mut self);  // discard stale 8-byte replies after an abandoned read
}

// ol-driver: runs only on its machine's own OS thread; depends only on ol-types
pub trait EncodedJob: Send { fn fingerprint(&self) -> &Fingerprint; fn emits(&self) -> bool; }
pub trait Backend: Send + 'static {
    type Job: EncodedJob;
    type Frame: EncodedJob;
    fn capabilities(&self) -> Capabilities;
    fn connect(&mut self, cx: &mut Cx) -> Result<DeviceInfo, DriverError>;
    fn frame(&mut self, f: &Self::Frame, mode: FrameMode, cx: &mut Cx) -> Result<FrameOutcome, DriverError>;
    fn run(&mut self, job: &Self::Job, cx: &mut Cx) -> Result<Outcome, DriverError>;  // token checked by the runtime first
    fn emergency_stop(&mut self) -> StopReport;
    fn status(&mut self, cx: &mut Cx) -> Result<MachineStatus, DriverError>;       // failed read = Err, never bits
    fn pause(&mut self)  -> Option<&mut dyn Pause>  { None }   // capability sub-traits; LMC: after H15
    fn jog(&mut self)    -> Option<&mut dyn Jog>    { None }
    fn home(&mut self)   -> Option<&mut dyn Home>   { None }
    fn inputs(&mut self) -> Option<&mut dyn Inputs> { None }   // LMC: after H11
}
pub struct DriverHandle<B: Backend> { ctl: Sender<Control<B>>, latch: Arc<StopLatch>, pub events: Receiver<DriverEvent>, pub caps: Capabilities }
impl<B: Backend> DriverHandle<B> {
    pub fn frame(&self, f: B::Frame, mode: FrameMode);
    pub fn run(&self, job: B::Job, arm: ArmToken);   // runtime re-checks token, fingerprint and its own FrameReceipt
    pub fn stop(&self);                              // atomic store + wake; never blocks, never does I/O
}
pub enum DriverEvent { Connected(DeviceInfo), State(MachineState), FrameCompleted(FrameReceipt) /* per acked pass */,
    Progress { packets_acked: u32, total: u32, eta: Duration }, Finished(Outcome), Stopped(StopReport),
    Fault(DriverError) /* sticky until acknowledged */, Trace(TraceLine) }
```

`Encoded` lives in `ol-app`, so `ol-driver` needs no encoder crate and `ol-lmc` can implement `Backend` without a cycle.

**Status.** `MachineStatus` is word 3 of GetVersion: BUSY `0x04`, READY `0x20`, AXIS `0x40`. Unknown bits are kept, since one board idles at `0x0220` ([06 §0.2][R06]).

**Stop never blocks.** GRBL realtime bytes are written by the driver thread within one wait slice, never by `stop()` itself.

### 3.5 Threading and the real-time stop path

**Threads.**
- The UI thread never does I/O.
- One planning worker runs plan, validate, encode, disassemble and fingerprint.
- **One OS thread per machine** solely owns its device.
- There is no tokio in M1 ([06 C7][R06]).

**Driver-thread rules.**
- No wait lasts longer than 20 ms, and every loop calls `cx.check()`.
- Packets go out with `submit` + `wait_next_complete(≤ 20 ms)`, never with `transfer_blocking`, which loops until a cancelled transfer returns ([06 §2.6][R06]).
- Single commands write and read in slices of 20 ms or less, with 250 ms in total; H13 tunes the total. A stop abandons an in-flight read without waiting for it, and stale replies are drained before the confirming GetVersion ([RT minor 3]).
- **There is no automatic USB reset and no auto-reconnect mid-job.** A transfer error stops the job and raises a sticky fault ([05 §1.1][R05]).
- `ol` takes the board lock it shares with OM (§11 #12) at connect, and refuses if another process holds it ([RT I20]).

| Stage | Budget | Mechanism |
|---|---|---|
| Esc/STOP → latch | ≤ 16 ms; ≤ 50 ms p99 under canvas load (spike S3, re-run on the M1.3 build) | Esc is consumed before widgets. STOP is always visible and never disabled or behind a modal. No modal file dialog can open while framing or emitting. CLI: Esc, Space and Ctrl-C on a key thread. SIGINT, SIGTERM and SIGHUP trip the latch |
| Latch → driver | ≤ 20 ms | Sliced waits, single-command reads included; the latch also wakes the control channel |
| In-flight packet | ≤ 20 ms | Let the one in-flight list transfer complete (it was submitted only when READY). `cancel_all` only on overrun, then flag `possible_desync` (H23) |
| Abort on the wire | ≤ 60 ms | Sequence below |
| Confirm | ≤ 50 ms | Drain stale replies; GetVersion !BUSY → `StopReport` |
| **Total** | **Emulator: p99 ≤ 100 ms; any run > 250 ms fails. Hardware: ≤ 250 ms in 20/20 traced trials, and ≤ 250 ms key-to-dark optically (H24). Python: 0.18 s ([05 §1.9][R05])** | |
| Watchdog | 500 ms | No report → red banner **"STOP NOT CONFIRMED: use the key switch / E-stop"** plus a sticky fault. Software stop is never the primary safety device ([06 G12][R06]) |

**Where STOP can fail on the UI side** ([RT I4]).
- Esc and the CLI keys reach OpenLaser only while its window, or its terminal, has focus. The STOP panel says so, and `HARDWARE.md` says to keep it focused. The E-stop has no such condition.
- `rfd` dialogs are synchronous on macOS and block the egui thread. Import, Open and Save are therefore disabled while framing or emitting, and use `AsyncFileDialog` otherwise.
- macOS live resize runs a modal loop. The S3 re-run measures STOP during a resize; if it fails, the window becomes non-resizable while framing or emitting.

**LMC session.** This fixes [05 §0.1, §1.10][R05]:
```
connect (once): take the board lock; 05 §1.4 init verbatim; assumed position (0x8000,0x8000) until H3
run(job, token):
  runtime: token.check(job.fp, machine, session, now); nonce unused;
           driver's own last FrameReceipt: same fp, ≥ 1 full pass, ≤ 10 min
  hold a no-idle-sleep assertion for the whole job (caffeinate -i -w <pid>)
  prelude: ResetList, WritePort(0x0001), Fiber_SetMo(1), GetVersion
  per packet: wait READY (poll ≤ 10 ms, stop-aware, inputs read after H11; never proceed on a deadline)
              respect run_ahead (≤ 3 packets and ≤ 2 s estimated queued emission; never < 1 packet)
              watchdog: no progress (READY/BUSY change, H22 position) for est(queued)×1.5 + 2 s → Fault → emergency_stop
                        (before ExecuteList est(queued) = 0, so a board that never raises READY is stopped within 2 s)
              submit (one list transfer in flight); SetEndOfList; ExecuteList after packet 3, or at the end if fewer  // GP parity (05 §1.5)
  wait !BUSY (same watchdog); only then Fiber_SetMo(0), WritePort(0), GetMarkTime(3)   // never close MO while BUSY
  no abort, Reset or USB reset after success (05 §0.2)
emergency_stop: stop submitting → in-flight packet completes (≤ 20 ms), else cancel_all + possible_desync
  → StopExecute → Fiber_SetMo(0) → ResetList
  → ONLY IF READY within 50 ms: dummy EOL packet + SetEndOfList + ExecuteList (parity until H14)
  → Fiber_SetMo(0) → WritePort(0) → drain stale replies → GetVersion !BUSY → StopReport
```

**The dummy packet is conditional.** Writing while the board is not READY is what triggers the reset storm; B and C sent the packet unconditionally ([JP §5][JP]). H14 compares abort variants on the trace, and H24 measures optically whether sending MO(0) first darkens the beam sooner.

**In-flight packets are completed, not cancelled** ([RT I1]).
- A list packet is submitted only when READY and normally finishes within a few milliseconds.
- Cancelling it mid-transfer could leave the board counting bytes toward a 3,072-byte packet, so that the following `StopExecute` and `Fiber_SetMo(0)` are swallowed as list data. Whether that happens is UNVERIFIED either way.
- A stop therefore waits up to 20 ms for the in-flight packet and cancels only on overrun. H23 measures what the board does after a cancel; until then FakeLmc's "partial packet" fault is labelled a model, not a fact.

**Run-ahead cap, in packets and in seconds** ([RT I3]).
- The cap bounds how much is queued beyond the packet the board is known to be executing: at most 3 packets and at most about 2 s of estimated emission, but never less than one packet. This bounds marking after an app crash.
- A single slow packet can still exceed 2 s, since 256 long segments at low speed can take minutes. The `EmissionSummary` shows each job's post-crash emission bound, and splitting packets by time waits for H28.
- The evidence for any bound is only as good as READY semantics and H22, so **the documented bound is the board's buffer depth until HIL-1 measures it**; B overstated this ([JP §5][JP]).

**Crash containment** ([RT I3]).
- **Panic guard.** A drop guard calls `emergency_stop()` if the driver thread panics ([PC §1.5][PC]). `panic = "unwind"` is set explicitly, so nobody can switch it to abort by accident.
- **Dead-man rule.** If the control channel disconnects because the UI crashed, the job stops ([JE §5][JE]).
- **Signals.** SIGINT, SIGTERM and SIGHUP trip the latch (`signal-hook`, an async-signal-safe store). Closing the CLI's terminal therefore stops the job.
- **Sleep.** A job holds a no-idle-sleep assertion (`caffeinate -i -w <pid>`, so no unsafe code). The `.app` opts out of App Nap (`LSAppNapIsDisabled`, verified in the spike). Closing the lid still sleeps the Mac, so `HARDWARE.md` says to keep it open, and H25 records what the board does when USB suspends.
- **Not covered:** abort, a wgpu/Metal segfault, OOM, SIGKILL or Force Quit, and a cable pull. In those cases MO stays open and the board drains its queue. The protection is the run-ahead bound plus the E-stop (H0), and H25 measures what actually happens. If the board keeps emitting longer than the run-ahead bound, M2 moves the driver into a small `ol-driverd` child process with no GPU code, whose dead-man rule fires when its pipe closes.
- **Faults are sticky; stops are not.** A user stop clears at the next frame or arm. A USB error or an unconfirmed stop stays latched until acknowledged.

### 3.6 Safety model: refusals, not warnings

There is no "Proceed anyway". The only way past a refusal is to change the document or the machine profile.

| Layer | Guarantee |
|---|---|
| 0. Physical | The E-stop and key switch, verified by H0 before any emission and re-checked at every emission session ([RT I7]). OpenLaser never overrides interlocks. A first-run screen states that it is not a safety device |
| 1. `ol_safety::validate` (pure, property-tested, runs live in the UI, again at ARM, and is re-checked by the driver) | Validates the whole job before byte 1 and refuses:<br>• **Geometry:** points outside the **work area** (never clamped); points the encoder cannot represent; non-finite values; path/point limits (3000/2000/20000 in M1, derived from H2 and time in M3).<br>• **Laser parameters:** power above the source maximum; frequency out of range; pulse width off the table (within ±0.5 ns it snaps with a notice instead); **a speed that encodes to 0 units**.<br>• **Objects and layers:** **an object on an unknown layer**; **a hidden layer with Output on**; **a Fill layer that yields no hatch** (OM skips it silently, `ui.py:2849-2858`); nothing to mark.<br>• **Gantry:** **speed above the machine maximum (refused, not capped like `gcode.py`)**; out-of-bed coordinates.<br>**Never dropped silently:** open shapes in a Fill layer have no interior to hatch. They become a numbered notice in the confirmation and are highlighted on the canvas ([RT I9]) |
| 2. Types | `FramePlan` has no power fields. `LightingList` cannot express `MarkTo`, MO open or `LaserOnPoint`. `MarkingList` needs an `EmitSection`. `EmitPlan` has a gated constructor, and only `ol-arm` can create an `ArmToken` (§3.1) |
| 3. Frame ⊇ burn | The frame comes from the same `JobPlan`: every emitting section plus Frame-only layers, whatever `show` says. A property test checks that every Line segment lies on a traced contour and every Fill segment lies within the even-odd region of a traced closed contour. On hardware, the `red_dot` calibration (H26) makes the red frame land where the beam burns ([RT I6]) |
| 4. Confirm from bytes | `EmissionSummary` (§3.3) |
| 5. ARM | **GUI:** tick the machine checklist and type `ARM`.<br>**CLI:** refused unless stdin and stdout are TTYs; the operator types `ARM <nonce>`, with a per-session nonce.<br>**Both:** the token is single-use, valid for 60 s, bound to fingerprint, machine and session, and voided by any edit. It needs a **FrameReceipt** of the same fingerprint from this session, with at least one fully acknowledged pass, at most 10 minutes old. On galvos this is not a setting.<br>**Never:** an API, script or LAN path (`ol-arm`, §3.1) |
| 6. Driver invariants | No write without READY; one packet in flight; run-ahead capped in packets and seconds; the watchdog only stops; no MO close while BUSY unless stopping; no auto-reset or auto-reconnect; dead-man rule; panic guard; signals trip the latch; board lock |
| 7. Interlocks (M3, after H11) | Door and foot inputs read before MO opens and in every wait loop. **They add protection; they are never the primary protection** |
| 8. Cold build at ARM (M5) | Once an incremental cache exists, the job is rebuilt without cache at ARM and refused if its hash differs from the preview |

**Agents and the bench** ([RT I14]). The TTY gate stops pipes and non-interactive shells, nothing more. In your setup an agent can type into a real terminal and read the nonce back, or drive the GUI with computer use, and red-light commands need no ARM at all. M0 therefore sets up five layers, weakest first:
1. **Written rules.** "Agents never run hardware commands" lives in `AGENTS.md` (Codex, OpenCode) **and** in `CLAUDE.md` through an `@AGENTS.md` import. Whether Claude Code reads `AGENTS.md` by itself is UNVERIFIED. The same file forbids self-hosted CI runners on this Mac.
2. **A PreToolUse hook** (hookify). It blocks shell and terminal-tool commands matching `ol (detect|status|frame|run|hw|bench)`, `ol-hw-raw`, `--features hw` and `OPENLASER_HW_PRESENT`.
3. **A bench session.** `ol` refuses any device I/O unless a bench-session file exists that `ol bench start` created at a TTY within the last 2 h. It is a speed bump, not a lock.
4. **USB unplugged.** The board's USB is unplugged whenever executors are working (D19). This is the rule that actually holds.
5. **A physical ARM input.** From M3, a foot switch on an input port (after H11) replaces the typed ARM for emission.

**Every known prototype bug becomes structurally impossible.** Each named test is **mutation-checked** at its milestone exit: revert the fix and the test must go red. After that, `cargo-mutants` runs nightly on `ol-safety`, `ol-arm` and the session code.

| Bug (§11; [05][R05]) | Structural fix | Named test |
|---|---|---|
| B1: 3 s wait cap truncates long jobs (G6) | No deadline-then-proceed path; the watchdog can only stop | `t6_busy_20s_completes_without_mo_close` |
| B2: Abort writes into a not-READY board (GP `abort` → `_list_end` → `wait_ready`) | Dummy packet only if READY | `t6_abort_no_write_while_not_ready` |
| B3: First jump elided (G4) | Explicit `JumpTo` with the long delay and a settle at the start of every list | `t1_dot_test_has_explicit_jump_and_settle` |
| B4: Confirm shows Frame values (G1) | Confirm rendered from `EmissionSummary` | `kittest_confirm_shows_decoded_layer_params` |
| B5: ARM always true; ARM text sticks (G2) | Single-use, fingerprint-bound token minted only in `ol-arm`; TTY-only CLI | `arm_token_single_use`, `arm_token_invalid_after_edit`, `cli_arm_refused_without_tty`, `xtask_arm_reverse_deps` |
| B6: Hidden + Output layer burns unframed (G3) | Refusal; frame derived from the emit sections | `safety_hidden_output_refused`, `prop_frame_covers_burn` |
| B7: Speed < 3.61 mm/s → `MarkSpeed(0)` (G5) | Whole units encoded once; refuse 0 units | `safety_speed_zero_units_refused`, `prop_speed_units_stable_over_field` |
| B8: Unknown `layer_id` silently dropped (G7) | Refusal | `safety_orphan_object_refused` |
| B9: 3-coordinate points crash (G10) | Refusal | `import_malformed_points_refused` |
| B10: Field bound is the galvo's reach, not the rated lens field ([RT C1]) | Work-area refusal | `safety_outside_work_area_refused` (100.5 mm at scale 1.1834) |
| B11: MOPA defaults leak into gantry G-code ([REPORT][RF]) | Separate galvo/gantry blocks, per-kind defaults (M2) | `gcode_diode_job_uses_gantry_defaults` (M2 exit) |

---

## 4. UI strategy and spike

**Stack: egui/eframe 0.37 with egui_tiles 0.18 and rfd.** egui_dock 0.21.1 needs egui 0.36, so it is out ([06 C9][R06]). egui scored 85.6 against Tauri's 81.0 ([04 §2.3][R04]). Two things settle it:
- one process and one language, all the way from the STOP button to the latch;
- `egui_kittest`, which tests safety UI behaviour headlessly.

**M1.3 layout:**
- **Left:** Cuts/Layers, showing the *actual* speed.
- **Centre:** the canvas. The work area square (and, faintly, the encodable field at ±F/2), a crosshair, objects coloured by layer, numeric move/scale/rotate/mirror.
- **Right:** the Laser panel. Connect, status, Frame Once/Loop, Mark, the refusal and notice list, the estimate, and a big STOP with the line "STOP works while this window is focused; the E-stop always works".
- **Bottom:** the log and the USB trace.

The canvas starts on egui's `Painter`, with cached world-space polylines and `rstar` hit-testing. A custom wgpu hairline pipeline ([04 §3][R04]) arrives in M5, or in M3 if S1 lands in the middle band, behind a `CanvasRenderer` trait.

**The spike** is an executor task on synthetic data. It runs **in parallel with M1.1–M1.2** and must be decided before M1.3, which keeps it out of M0. Measurements are taken on the M2 Air with a release build:

| # | Metric | GO | NO-GO |
|---|---|---|---|
| S1 | p95 frame time panning/zooming 20k paths / 200k segments | ≤ 16.7 ms (16.7–33 ms is still GO, and pulls wgpu forward to M3) | > 33 ms |
| S2 | Hit-test p99 at 20k paths | ≤ 2 ms | > 10 ms |
| S3 | **Keypress → `stop()` while S1's load and a 500 ms plan are running.** Re-run on the real M1.3 build at HIL-3, including during a live resize (M1 exit #8) | ≤ 16 ms p95, **≤ 50 ms p99**, no stall > 50 ms | exceeded |
| S4 | `cargo tree -d`: one egui, one wgpu, one `kurbo` and one `raw-window-handle` across eframe, egui_tiles, egui_kittest and usvg 0.48 | yes | unresolved after 2 days |
| S5 | egui_kittest test plus snapshot on the GitHub macOS runner; `LSAppNapIsDisabled` takes effect | yes | no |
| S6–S8 | VoiceOver reaches STOP and layers; Portuguese dead-key IME; `muda` native menu (1-day timebox; precedent exists only on egui 0.36 with a patch, [06][R06]) | soft, never a gate | — |

**Fallback.** Tauri 2 with a web UI over `ol-app`'s serde types ([04 §2.4][R04]), at about 3–4 weeks, not a rewrite. But under Tauri, STOP and ARM would cross from WebView JavaScript over IPC. The fallback must re-pass S3 and define a native ARM path, so that "no script path can ARM" still holds ([RT minor 1]).

---

## 5. Hardware and protocol plan

### 5.1 Porting the LMC driver from galvoplotter

galvoplotter (MIT, © 2023 MeerK40t) already runs your M7. Port its behaviour with per-file attribution, and fix its quirks on purpose ([05 §1.11][R05]):

| GP source | Rust home | Decision |
|---|---|---|
| `consts.py` | `ol-lmc::op` | Ported with attribution |
| `_list_write`/`_list_end`, `0x8002` fill | `ListBuf`, packetiser | One owner, so no lock; check overflow before writing |
| `init_laser` (05 §1.4) | `session::connect` | Same words until a hardware or EZCAD capture says otherwise; no `.cor` upload until M3 |
| marking/lighting configuration | prelude/postlude, `LightingList`/`MarkingList` | Split at the type level |
| `abort` | `emergency_stop` | Conditional dummy packet; in-flight packet completed first |
| `usb_connection.py` (resets on error) | `ol-lmc-usb` | **No reset, no retries**; interface 0, OUT `0x02`, IN `0x88`; `set_configuration(1)` if H12 shows it's needed; board lock shared with OM |
| failed read → `(-1,…)` = READY\|BUSY | `Result<MachineStatus, _>` | Never fake bits |
| `goto` skips the "current" point | explicit first `JumpTo` + long delay + settle | Fixes B3 |
| `.cor` readers (`controller.py:865-919`) | `ol-machine::cor` (M2) | Ported with attribution, cross-checked against corfile2meerk40t's `SPEC.md` |
| `_wait_for_input_protocol`, `rotary_position` | — | Not ported (broken) |

MeerK40t `balormk` (MIT) is a cross-check and the source of the raster penbox (M3). It is not the `.cor` source, because its freedom from balor-derived code is UNVERIFIED ([06 §5.3][R06]). **Never port `clone_loader.py`**: it embeds vendor firmware. balor (GPL) and GP30 (no licence) are facts-only sources and are never shown to executors.

### 5.2 Hardware sessions and checklist (H0–H29)

**Rules for every session:**
- Luiz runs every step; agents never do (§3.6).
- Tests sit behind `--features hw`, `OPENLASER_HW_PRESENT=1` and a bench session. Emission also needs `--allow-emission` and a TTY ARM.
- Items that must break driver rules (H13 writes while not READY; H23 cancels a packet mid-write) run from `ol-hw-raw`, a separate bench binary that is never compiled into `ol`. Red light only.
- Red light comes first. If the red dot works with the source key off, keep the key off (UNVERIFIED on the M7).
- **The first emission of each session is at 10 % power or less**, on scrap, with eyewear on and the E-stop and key switch within reach of where you stand. `ol hw` scripts enforce the power cap.
- Keep the MacBook lid open and the OpenLaser window (or terminal) focused.
- Every session commits a USB trace plus a result in `HARDWARE.md`. The traces become FakeLmc fixtures.
- If a session runs over 2 h, the rest moves to a reserve session. Never rush an item.

**Sessions (D7: seven planned plus up to three in reserve through M2)** ([RT I16]):

| Session | When | Emission | Items |
|---|---|---|---|
| **HIL-P** | M0 step 3 | ≤ 5 %, then low | H0, then the patched-prototype validation (§11) |
| **HIL-0** passive | M1.1 | none | H12 |
| **HIL-1a** | M1.1 | red | H2, H13, H23, H14 (trace), H28 |
| **HIL-1b** | M1.1 | red | H1, H22, H3, H4 (red), H25, H27 |
| **HIL-2** | M1.2 | low | H0 re-check, H4 (mark), H10, H19, H26, H24, H1 with emission, H29 |
| **HIL-3** | M1.3 | low | GUI end to end: business card + one MOPA colour test; 20 GUI stops; S3 re-run |
| **HIL-4** | M2 | none/red/low | H21, H11 |
| Reserve ×3 | M1–M2 | — | Stop-path re-runs after any red-zone change; overruns |
| HIL-5 | M3 | low | H5 (`.cor` upload + 100 mm grid), H15, H16 |

| # | Item | Session | Emission | What it gates |
|---|---|---|---|---|
| 0 | **H0**: E-stop and key switch. Red light first, then ≤ 5 % on scrap; hit the E-stop mid-job, then the key switch mid-job. Does each cut the **source**, not just the controller or galvos? Are both reachable from the Mac? If neither stops emission, there are no more emission sessions until that is fixed | HIL-P; re-checked at HIL-2 | ≤ 5 % | every emission session |
| 1 | **H12**: descriptors, `set_configuration` on macOS 26, GetVersion/GetSerialNo replies, idle status. **Only** GetSerialNo and GetVersion: no init, which includes `EnableLaser` and `Reset` (05 §1.4) | HIL-0 | none | nusb bring-up, status parser |
| 2 | **H2**: READY/BUSY semantics, buffer depth | HIL-1a | red | flow control, run-ahead, watchdog |
| 3 | **H13**: a write into a full buffer; safe timeout (`ol-hw-raw`) | HIL-1a | red | avoiding the reset storm |
| 4 | **H23**: after a cancelled partial 3072-byte write, are the following singles honoured? (`ol-hw-raw`) | HIL-1a | red | stop path ([RT I1]) |
| 5 | **H14**: minimal abort; 10 stops while continuously framing; MO(0)-first variant (on the trace; optics in H24) | HIL-1a | red | stop path |
| 6 | **H28**: is `0x8002` mid-list a no-op or an end of list? | HIL-1a | red | time-based packet splitting |
| 7 | **H1**: a red-light job over 8 s runs to the end | HIL-1b | red | B1 regression |
| 8 | **H22**: `GetPositionXY` as a progress signal | HIL-1b | red | watchdog, run-ahead evidence |
| 9 | **H3**: mirror position after Reset, job and abort | HIL-1b | red | first-jump settle and `dist` |
| 10 | **H4**: axis orientation (asymmetric "F") | HIL-1b red light, confirmed by a mark in HIL-2 | red → ≤ 10 % | mirrored burns |
| 11 | **H25**: SIGKILL `ol` during a red-light frame; pull the cable mid-frame; close the lid. What does the board do, and for how long? | HIL-1b | red | crash policy; out-of-process driver decision |
| 12 | **H27**: frame the corners at ±100 mm and ±110 mm; look for clipping or vignetting | HIL-1b | red | work area (B10) |
| 13 | **H10**: low-speed quantisation at 5/10/50 mm/s | HIL-2 | low | speed rule (D9) |
| 14 | **H19**: MO settle, first pulse | HIL-2 | low | first-mark quality; settle value |
| 15 | **H26**: red pointer vs burn. Mark a cross at ≤ 10 %, then frame it; record offset and scale | HIL-2 | ≤ 10 % | frame ⊇ burn on hardware |
| 16 | **H24**: optical stop latency, from a photodiode on an Arduino or 240 fps video with the key press in frame. `StopExecute`-first vs MO(0)-first | HIL-2 (GUI again at HIL-3) | low | H14 verdict, M1 exit #3 |
| 17 | **H1 with emission**: ≥ 30 s hatch; 10 low-power stops; 10 mm box and 50 mm square against Python | HIL-2 | low | M1 exit |
| 18 | **H29**: OM tries to open the device while `ol` holds a session. Does the lock refuse it cleanly, with no reset? | HIL-2 | red | bridge phase ([RT I20]) |
| 19 | **H21**: do mid-list `MarkCurrent`/`QSwitchPeriod`/pulse changes take effect at the next `MarkTo`? | HIL-4 (M2) | low | M3 single-list merge; raster power per run |
| 20 | **H11**: port map; door/foot inputs | HIL-4 | none/red | M3 interlocks, physical ARM input |
| 21 | **H5**: `.cor` vs blank; what 1.1834 compensates | file in M0 (D11), parser in M2, upload + 100 mm grid in HIL-5 (M3) | low | accuracy |
| 22 | **H6–H9, H20**: pulse/power/frequency/delay encodings, init | EZCAD USB capture on Windows (M3, D11) | none | fidelity |
| 23 | **H15** pause/resume; **H16** GetMarkTime/GetListStatus | HIL-5 | red | Pause hidden until H15 passes; progress UI |
| 24 | **H18**: Windows WinUSB, Linux udev | M4 | red | platforms |
| — | **H17**: wedge recovery | never provoked | — | — |

### 5.3 GRBL

**Until M4: Rayforge, after a bring-up checklist on each machine** ([RT I13]). The Rayforge test used a fake GRBL over TCP: no serial port, no real planner, no pen dwell, no raster, and no measured stop on real hardware ([06 §5.10][R06]). Its safety is advisory, it auto-reconnects, and it homes on start. Before the first real job on each gantry, starting with the CO₂, which is the biggest fire risk:
1. `$32=1` (laser mode), and `$30` equal to Rayforge's maximum power. Soft limits on (`$20=1`), with `$130`/`$131` matching the real travel.
2. `home_on_start` off, unless homing has been checked by hand first.
3. Frame at zero power and watch the whole path.
4. Time a stop at low power on video.
5. Run the first job at minimum power on scrap, with air assist and extraction on. Never leave the CO₂ unattended.
6. Plotter: check the pen-up/down dwell.

**M2: export only.** The output is byte-identical to `gcode.py` (T2), with two documented divergences: overspeed is refused instead of capped ([05 §3.2][R05]), and a plotter pen dwell is added.

**M4: live sender** on `serialport` (MPL-2.0), on the machine's OS thread, per [05 §3.3][R05]:
- Wait for the banner, then query `$I`, `$$` and `$G`.
- **Preflight refusals** ([RT minor 7]): no streaming unless `$32=1` on laser profiles, `$30` equals the profile's S-max, and `$130`/`$131` cover the profile's bed ([05 §3.3 item 10][R05]).
- Stream with character counting, using buffer sizes from `[OPT:]`.
- The driver thread writes the realtime `!`/`~`/`0x18`.
- On `error:` or `ALARM`, send `!` then `0x18`, fail the job and require acknowledgement.
- Frame with the laser off; jog with `$J=`.
- **`home_on_connect = false`, and the machine never moves on connect** (Rayforge homed unprompted, [REPORT][RF]).

raydriver's cancel ordering (MIT) is the reference. FakeGrbl ports [`fake_grbl.py`](rayforge-test/fake_grbl.py) and also serves TCP `:2323`, so Rayforge and OpenLaser can be compared on the same emulator.

### 5.4 Ruida and others

Ruida comes in M6+, and only with a Ruida machine available for testing. The references are MeerK40t `ruida/` with its emulator and ruida-pa, all MIT ([03 §6][R03]). EZCAD3 and BSL have no open reference and stay out of scope.

---

## 6. Verification

### 6.1 Golden oracle

`tools/golden/capture.py` extends the [05 App. A][R05] harness. It runs in OM's venv with USB patched to raise, `OPENMOPA_MOCK=1`, a temporary `CALIBRATION_PATH` and masked timestamps, and records the OM SHA, GP version and scale. **Never run `site-packages/test/*.py`: those tests open real USB.**

**Two oracles:**
- **`oracle-v0` = `6aed8ff`**: the bytes proven on hardware.
- **`oracle-v1`**: the patched prototype (§11).

A committed v0-vs-v1 diff proves the patches changed only what was intended. `mark_box10`'s packet SHA `9ca1e12d…59f5` must be identical in both.

**`py-compat` is a test-only Cargo feature.** It replays galvoplotter's sequencing (one controller per job, parameter cache, elided first jump, per-job init and abort) and must reproduce the v0 streams byte-for-byte. It is reachable only through dev-dependencies, and a `compile_error!` blocks it without `debug_assertions` (§3.1). The shipping sequence is frozen as the `rust-v1` goldens.

**`DIVERGENCES.md`** (signed under D13) lists every intentional difference:
- the explicit first jump, with the long jump delay and a settle;
- one session per connection; no per-job Reset, abort or USB reset;
- the conditional dummy packet; an in-flight packet completed before a stop;
- the work-area bound (±100 mm, against v0's ±118.34 mm; v1 matches);
- +F/2 refused rather than clamped to 65535 (T0);
- speed encoded once to whole units, and refusal at 0 units;
- refusing hidden+Output layers, orphan objects, empty Fill layers (OM's `raster_skip`) and gantry overspeed;
- open shapes in Fill layers shown as notices instead of silently skipped;
- bridge input: mirror flags checked against the profile, repeat count shown;
- SVG imported at true scale (OM fits it to 60 mm), and unitless SVG refused or asked about.

| Tier ([05 §5.3][R05]) | Content | Must match |
|---|---|---|
| T0 numeric | `mm_to_galvo`, power, frequency, speed, delay word, pulse snap, distance; ≥ 10k cases each, plus ties (30 % → 1228, 70 % → 2866), ±F/2 (the + edge is a listed divergence), sub-unit speeds | exactly |
| T1 LMC | init, `mark_box10`, `frame_box10`, `dot_test`, `long_job` (9 packets), business card per layer at F = 236.68, abort (`e479558e…6a18`) | `py-compat` vs v0; packets vs v1; `rust-v1` frozen |
| T2 G-code (M2) | 4 profiles × {vector, multi-pass, air, hatch, plotter}, with M2's ordering, joining and dedupe **off** | byte-exact with `; Generated:` masked, minus listed divergences |
| T3 hatch | `hatch_polylines` (workspace: 969 segments, 67 open shapes reported as skipped, which the planner turns into a notice) | count and order exact; coordinates within 1e-9 |
| T4 import | DXF fixtures (M1) | point count exact; within 1e-9. **SVG diverges on purpose; no STL** |
| T5 safety | v1 refusal paths, including B10 | same decision, as a refusal **code** |

### 6.2 Emulators

**FakeLmc** models:
- a configurable buffer depth (swept 1–8 until H2, then the measured value);
- READY/BUSY timing from the galvo time model;
- MO and port state;
- an **emission recorder**: MarkTo with MO open and bit 0 set, rendered with tiny-skia.

It also injects faults: stalled writes, READY never returning (before and after `ExecuteList`), BUSY stuck, a mid-packet disconnect, **a partial packet after a cancel** (a model until H23 replaces it with measured behaviour), unknown status bits, **PID `0x9980`** and a held board lock.

**The T6 suite** runs 200 randomised runs per PR (on Linux, §6.3) and 10k nightly. It asserts:
1. Stop p99 ≤ 100 ms, with nothing over 250 ms, from every state: streaming, full buffer, packet in flight, BUSY, framing, connecting.
2. Zero writes while not READY, including during an abort.
3. A 20 s BUSY job completes, and MO closes only after BUSY clears.
4. The watchdog fires only when there is no progress, and a board that never raises READY is stopped within 2 s with MO closed.
5. **Framing records zero emission.**
6. The run-ahead cap holds, in packets and in estimated seconds.
7. The dead-man rule, the panic guard and SIGTERM/SIGHUP all end with MO closed and ports at 0.
8. A stop with a packet in flight waits for it (≤ 20 ms) before `StopExecute`.
9. The business card's emission image matches the design.

A real-time variant with 2 ms transfer latency checks the budget on the CI clock. **FakeGrbl** (M4) has a 15-block planner, a 128-byte RX buffer, realtime bytes and alarms, and runs in-process or over TCP.

### 6.3 Property tests, fuzzing, CI

**proptest** (256 cases per PR, 10k nightly) checks that:
- every accepted point lies inside the work area and encodes into `0..=0xFFFF` without overflow, monotonic per axis, while points outside are refused;
- accepted speeds encode to at least 1 unit, and the units stay stable for F ∈ [100, 300] mm;
- packets are 3072 bytes, padded with `0x8002`;
- lighting lists never contain `0x8005`, `0x8003` or MO open;
- encode → disassemble round-trips;
- the plan covers each emitting path exactly once per pass;
- hatches stay inside their even-odd region;
- a stop at a random tick leaves MO closed and the ports at 0.

**cargo-fuzz** covers the SVG, DXF, workspace and job-payload importers, plus `.cor` (M2) and `.lbrn2` (M5).

| Runner | Jobs |
|---|---|
| Ubuntu 24.04 (per PR) | Full nextest (T0–T6 with 200 T6 runs, properties), goldens, `cargo-deny`, `xtask deps`; `ol-ui` build only |
| macOS arm64 (per PR) | fmt; clippy `-D warnings` (including `disallowed-methods`); core nextest; kittest; release build |
| Nightly (Ubuntu while the repo is private) | proptest 10k; T6 10k; fuzz 10 min per target; `cargo-mutants` on the red zone; `cargo tree -d` drift (egui, wgpu, kurbo, raw-window-handle) |
| Windows (from M4) | Build plus core tests |
| Manual | Golden regeneration (OM venv); notarisation |

**CI cost and safety** ([RT I18]). While the repo is private, macOS minutes are billed at roughly 10× Linux minutes (verify current pricing), so heavy jobs run on Linux. After D17 makes the repo public at M2, macOS joins the nightly. **A self-hosted runner on the laser-connected Mac is forbidden** (D19): it would run PR code on the machine wired to the laser.

### 6.4 How the AI workflow plugs in

Goldens turn executor output into pass/fail checks, so these go to **executors**:
- encoders against T0/T1;
- importers against T4;
- hatching against T3;
- the G-code emitter against T2;
- panels against kittest specs.

**The red zone** is designed by Claude (Opus):
- `ol-safety`, `ol-arm` and `ArmToken` in `ol-types`;
- `ol-lmc/session.rs`;
- `ol-driver`;
- the confirm and ARM flows in `ol-ui` and `ol-cli`.

**Review gate for the red zone** ([RT I15]). Any change there needs:
- T6 and the mutation check;
- a second, independent model review (Codex reviews Claude's red-zone code, or the reverse);
- **Luiz's behaviour review**: the USB trace, the T6 summary and the confirm text, not a line-by-line Rust review.

Any stop-path change also needs a hardware re-run at the next session (a reserve session if needed). Every executor spec lists its allowed sources (§8) and forbids reproducing GPL or unlicensed code, even from memory.

---

## 7. Roadmap

| Milestone | Scope | Exit criteria | Effort | What Luiz can do afterwards |
|---|---|---|---|---|
| **M0 Foundations + safer prototype** | See below | See below | 3 | Use a prototype without its known bugs |
| **M1 MOPA vertical slice** | See below | See below | 11 | Run MOPA jobs from the Rust app |
| **M2 Prototype retired** | G-code export (T2); per-machine layer defaults (B11); editable profiles; time estimate shown; travel ordering (greedy NN + 2-opt; vendor raygeo `optimize.rs` only if it builds standalone within a day); join/close/dedupe; zip project format with migrations; snapshot undo; kittest snapshots; text; first-burn test patterns; repeat count in the GUI; **MOPA material test grid** (frequency × pulse, power × speed); per-axis Calibrate wizard; **`.cor` read** and a report of the field it implies vs 1.1834; frame styles; first-run safety screen; diagnostics export; `ol-driverd` if H25 requires it; Developer-ID signed, notarised universal DMG; repo public under the new name (D4, D17); **HIL-4** (H21, H11) | T2 byte-exact apart from listed divergences; B11 test mutation-checked; DMG installs on a clean Mac without `xattr`; H21/H11 answered; every OM workflow (including text, test patterns and G-code export) works in Rust; 2 weeks of real work without Python; OM archived (oracles kept) | 7 | Do everything the prototype did, plus test grids, calibration and signed installs |
| **M3 Galvo depth** | `.cor` upload (HIL-5, H5); **`.cor` and a `scale_correction` other than 1.0 are mutually exclusive unless calibrated together; `.cor` tables bound-checked before upload; corners red-lit after upload**; per-layer timing overrides; sub-layers + global passes; wobble; bitmaps + own dithering (Threshold, Floyd–Steinberg, Jarvis, Stucki, Atkinson, Ordered) → galvo raster using MeerK40t's power-per-run model, **gated on H21**; QR/DataMatrix + Hershey serials; interlocks and the foot-switch ARM (H11); pause if H15 passes; job log; limits from H2 + time; single-list merge if H21 + H2 allow; EZCAD capture (D11); JPT pulse-repetition-rate (PRR) limits once the model is known | 100 mm grid within ±0.1 mm using the `.cor` and no 1.1834 trim (or a documented reason); a 16-step grey ramp on anodised aluminium gives monotonically darker patches in a photo under fixed light, and the FakeLmc emission image of the reference photo matches its golden; interlock stops a job in ≤ 250 ms on hardware; sub-layer goldens | 7 | Production MOPA work: colour tests, photos, serial plates |
| **M4 Gantry live + platforms** | GRBL sender with preflight refusals (§5.3), jog/home/console; FakeGrbl over TCP; Windows NSIS + driver helper (§9) and signing (D18); Linux AppImage/.deb + udev (`uaccess`, `0660`); H18 | FakeGrbl: stop ≤ 100 ms, no streaming after `error:`, each preflight refusal tested. The same DXFs run through Rayforge and OpenLaser on FakeGrbl, with line count and run time recorded (information, not a gate). Red-light frame on the board from Windows and Linux if D11/D12 provide the machines; otherwise emulator-only, and those builds are labelled "untested on hardware" | 6 | Drive the GRBL machines live (per D12); share builds |
| **M5 Design suite + `.lbrn2`** | Shapes, node editing, booleans (`i_overlay`), offset (`cavalier_contours`), text (parley/skrifa + Hershey), align/arrays/snap; `.lbrn2` import (Y-up) through an explicit galvo/gantry mapping and an `Extras` bag; fixtures from the local LightBurn 1.5.04 if licensed ([06 G10][R06]); SVG/DXF export; wgpu canvas if S1 needed it; muda menus; incremental planning cache **with cold rebuild + hash compare at ARM** | Corpus snapshot tests. On the MOPA `.lbrn2` sample, the layer count and each layer's speed, power, frequency and Q-pulse equal the file's values, and every shape's bbox is within 0.01 mm of the file's geometry; Y-up confirmed on a second fixture. Incremental-equals-cold proptest over 1,000 edit sequences | 8 | Design in the app; open LightBurn projects |
| **M6+ Differentiators** | Galvo camera alignment ([02 §20][R02]); rotary/cylinder; `.ezd` import; nesting; Ruida (with hardware); headless Pi whose LAN panel **can never ARM** (it cannot link `ol-arm`); publish `lmc-proto`/`lmc-sim`; Rayforge addon spec only if the maintainer engages (bundling a native wheel stays blocked, [01 §3.1][R01]) | Per feature | months | — |

M5 lands after about 42 focused weeks. Full parity is not promised ([06 G9][R06]).

### M0 in detail (3 focused weeks)

1. **You, at your desk (no laser):**
   - Decide at least D1–D9, D12, D17 and D19.
   - If you still have the original EZCAD PC (D11), copy its `.cor`, `markcfg*` and any `.ezd` files. This is the cheapest test of the 1.1834 hypothesis ([03 §13][R03]).
   - Note the JPT model and wattage from the source label.
   - Confirm that the 9 OM commits by "Manufatu <info@manufatu.com>" are yours ([RT minor 9]).
   - Install `rustup` stable, version 1.95 or later ([06 G14][R06]).
2. **Agents:**
   - Apply the §11 patches on a `safety-fixes` branch through `/route-implementation`, each with a test.
   - Commit the golden capture harness and the `oracle-v0` (`6aed8ff`) goldens.
3. **You, HIL-P:**
   - H0 first.
   - Then validate the patched prototype (§11, "Validation before daily use"): stops, a long red-light frame, a ≥ 30 s low-power hatch with a trace, a dot, and a refusal at 105 mm.
   - Then tag `oracle-v1` and commit the v1 goldens and the v0-vs-v1 diff.
4. **Agents:** set up a new private repo with only the M1.1 crates.
   - CI (§6.3), `cargo-deny`, `xtask deps`, `PROVENANCE.md`.
   - The bench guard rails: `AGENTS.md`, the `CLAUDE.md` import and the PreToolUse hook (§3.6).
   - `ol-units` and the T0 encoders passing at least 10k golden cases each.
   - A FakeLmc skeleton.
   - The egui spike spec.

**M0 exit:**
- OM passes its 119 tests plus the new ones;
- HIL-P is recorded: H0 answered, and the ≥ 30 s hatch trace shows MO(0) only after BUSY clears;
- both golden sets and the diff are committed;
- T0 has ≥ 10k cases per encoder;
- `cargo-deny` and `xtask deps` are green;
- the hook blocks a test `ol frame` command.

### M1 in detail (11 focused weeks)

**In scope:**
- Import: SVG (paths, transforms; physical units required, so a unitless SVG is refused or asked about rather than guessed at 96 or 72 dpi, [RT minor 6]), DXF, and the OM workspace, layer settings, calibration and `.ini`.
- Layers: Line, Fill (interval, angle, angle step, bidirectional) and Frame-only; output, show and passes.
- The JPT M7 profile: encoding field and scale, work area, mirror, red-dot calibration.
- Frame Once/Loop; Mark with the byte-decoded confirmation and the ARM token; repeat count; STOP.
- Galvo time estimate; save/load as versioned JSON; CLI equivalents of all of the above.

**Out of scope:** G-code; editing beyond select, move, scale, rotate, mirror and delete; undo and zip projects (M2); images; text; `.cor` upload; pause; Windows/Linux; notarisation.

**M1.1 Rust red light (weeks 1–5).**
- T1 encoder (`py-compat` against v0), disassembler, `EmissionSummary`, session, FakeLmc + T6, `ol-lmc-usb` + `Recording`, `ol-hw-raw`.
- `ol detect`, `ol status`, `ol frame --loop`, `ol bench start`.
- **HIL-0**, **HIL-1a** and **HIL-1b**.
- `DIVERGENCES.md` signed. The spike runs in parallel.

**M1.2 First Rust burn (weeks 6–7; the priority checkpoint).**
- `ol-plan` (including the T3 hatch port, since the business card has Fill layers), `ol-safety`, `ol-arm` and the interactive `ol run --job <export>.json`.
- **The input is the exact `/api/mark` payload** ([05 §4.4][R05]), already transformed and mirrored by the browser, plus the §11 #9 export metadata. Nothing is rebuilt from `workspace.json` ([JP §5][JP]).
- **No double mirror** ([RT I8]). `ol` applies no profile mirror, invert or swap to `--job` input. It refuses unless the export's `mirrorX`/`mirrorY` equal the profile's mirror and the profile has no invert or swap set. Both are printed in the summary.
- `repeat_count` (1–999, as OM honours it, `ui.py:2807-2833`) is honoured and shown in the summary, with a stop check between repeats. `selectedOnly` is shown too.
- Any `arm`/`confirm` field in the file is ignored. If the export's `scale_correction` differs from the profile's, the job is refused.
- The flow:
  1. Connect (taking the board lock).
  2. Print the byte-decoded summary.
  3. Frame in a loop. Enter is accepted only after at least one fully acknowledged pass, which issues the FrameReceipt ([RT minor 2]).
  4. Type `ARM <nonce>` on the TTY.
  5. Mark.

  Esc, Space or Ctrl-C stops it, and so does closing the terminal (SIGHUP).
- **HIL-2.**
- If HIL-1 shows that READY/BUSY behave differently from GP's docs, the session is redesigned and M1.2 moves. M1.3 shrinks first; the safety work never does.

**M1.3 Rust GUI (weeks 8–11).**
- The egui slice, built after the spike decision.
- **HIL-3:** the business card (its open Fill shapes shown as a notice) and one MOPA colour test end to end; 20 GUI stops; the S3 re-run.

**Exit criteria (all required):**
1. CI green:
   - T0, T1, T3, T4 (DXF) and T5;
   - T6 with stop p99 ≤ 100 ms and the 20 s BUSY job completing;
   - zero framing emission.
2. Every §3.6 bug row from B1 to B10 has its named test, and each test went red when its fix was reverted. B11's test lands in M2.
3. Hardware stops:
   - ≤ 250 ms in 20 of 20 trials (10 while framing, 10 while marking at low power), measured from the trace (request → `WritePort(0)` done), with zero USB resets and no power-cycles;
   - ≤ 250 ms key-to-dark in 10 of 10 optical trials (H24).
4. A low-power hatch of 30 s or more completes, with the trace in the order `ExecuteList` → BUSY cleared → `Fiber_SetMo(0)`.
5. The Test Dot trace starts with `JumpTo` and the settle, and the dot lands at the field centre.
6. A Rust-marked 50 mm square matches the Python one within ±0.1 mm (calipers, same calibration).
7. Tests prove the confirm and ARM rules:
   - kittest: the confirm dialog shows each emitting layer's *decoded* kHz, ns, power and actual speed, plus repeats and notices;
   - kittest: Mark is disabled while refusals exist, and no file dialog opens while framing or emitting;
   - CLI tests: ARM is refused without a TTY, and Enter before a full frame pass is refused;
   - `xtask`: the reverse dependencies of `ol-arm` are exactly {`ol-ui`, `ol-cli`}.
8. The S3 re-run on the M1.3 build: keypress → `stop()` ≤ 50 ms p99 while panning the business card, and during a live resize.
9. `HARDWARE.md` answers H0, H12, H2, H13, H23, H14, H28, H1, H22, H3, H4, H25, H27, H10, H19, H26, H24 and H29.
10. These OM workflows run in Rust (GUI or `ol`): import SVG/DXF/OM workspace; Line and Fill layers; Frame Once/Loop; Test Dot; Mark with a repeat count; calibration import. Text, the `/api/test/run` patterns and G-code export stay on OM or Rayforge until M2, by name ([RT I17]).

---

## 8. Reuse, licensing and provenance

**MIT OR Apache-2.0** ([03 §11][R03]). OM is MIT, so code ported from it simply keeps its MIT notice. Confirming authorship of the "Manufatu" commits (M0) is housekeeping; it does not block anything ([RT minor 9]).

The repo carries `LICENSE-MIT`/`LICENSE-APACHE`, `THIRD_PARTY_NOTICES.md` (via cargo-about), `PROVENANCE.md` and DCO sign-off. `cargo-deny` allows MIT, Apache, BSD, ISC, Zlib, BSL-1.0, Unicode and MPL-2.0, and denies GPL, AGPL and statically linked LGPL ([04 §16][R04]).

| Source | Licence | Use | When |
|---|---|---|---|
| galvoplotter 0.2.0 @ `fc35136` | MIT © 2023 MeerK40t | Translate the LMC protocol and the `.cor` readers, with a header like `// Ported from meerk40t/galvoplotter@fc35136 galvo/controller.py, MIT` | M1–M2 |
| MeerK40t `balormk/`, `extra/ezd.py` | MIT © 2021 meerk40t | Cross-check; port the raster penbox and `.ezd` | M1–M6 |
| OM prototype | MIT © 2026 Luiz Bueno | Port the safety rules, hatch, DXF handling and G-code with the MIT notice kept | M1–M2 |
| corfile2meerk40t `SPEC.md` | MIT | `.cor` layouts | M2–M3 |
| raygeo (pinned SHA) | MIT © 2026 Barebaric | Vendor selected files; strip `#[global_allocator]` (`src/lib.rs:141-142`); add Rust tests | M2+ |
| raydriver; Rayforge `.lbrn2` importer; baseCreator (review first) | MIT, MIT/Apache | Logic reference, ported with attribution | M4–M5 |
| Hershey fonts | free with acknowledgement | Bundle with a NOTICE ([06 G8][R06]) | M3 |
| libwdi (Windows helper only, not in the Cargo graph) | LGPL-3 | Upstream `wdi-simple.exe` unmodified, or a helper loading libwdi as a DLL, with the LGPL notice and a source offer (§9) | M4 |
| **Never** | — | balor (GPL); GP30 and earlynerd (unlicensed, facts only); `clone_loader.py` firmware; any EZCAD blob; clayfrocker97/OpenLaser, LaserGRBL, UGS, potrace (GPL); dssim-core (AGPL); PyMuPDF; `rusb`/libusb unless Windows needs it ([06 C8][R06]) | — |

**Provenance rules:**
- Specs name the exact MIT file and commit to port from.
- Executors never see GPL or unlicensed code.
- Each ported file carries `// Ported from <url>@<sha>, <licence>`.
- USB traces come only from Luiz's machine.
- No firmware or EZCAD files enter the repo; tests use synthetic `.cor` fixtures.
- Use "works with EZCAD2-compatible JCZ LMC boards" wording, and no LightBurn logos.

---

## 9. Packaging, signing and USB per OS

**macOS:**
- **M1:** a locally built `.app` via `cargo-packager`, ad-hoc signed, with `LSAppNapIsDisabled`. Local builds have no quarantine flag.
- **M2:** Developer ID, hardened runtime, `notarytool` and stapling, shipped as a universal DMG on GitHub Releases once the repo is public (D17).
- **No USB entitlement is needed** outside the App Sandbox ([06 §0.9][R06]). The camera (M6) adds `com.apple.security.device.camera` and `NSCameraUsageDescription`.
- **Never the App Store.**
- Updates: a GitHub Releases check from M2, then Velopack once there are users ([04 §11][R04]).
- Expected size is 10–30 MB (unmeasured until the spike), against Rayforge's 154–325 MB ([01 §5][R01]).

**Windows (M4)** ([RT I19]):
- nusb works only with WinUSB, and **no tool shares the board with EZCAD; LightBurn swaps drivers too** ([06 §0.5, C12][R06]).
- **Driver helper:** two actions, "Install WinUSB" and "Restore EZCAD driver"; the restore path is UNVERIFIED and gets tested in M4. Ship upstream `wdi-simple.exe` unmodified, or a helper that loads libwdi as a DLL, with a source offer. Statically linking LGPL-3 libwdi would carry relinking obligations.
- libwdi installs a self-generated root certificate, so expect antivirus and SmartScreen flags, and document them.
- Port the vendor-driver hint from `live.py:425-445`.
- PID `0x9980` gets an explanation and no firmware handling.
- **Signing (D18).** Use Azure Artifact Signing only if you are eligible. Its public eligibility has covered organisations in a few regions and individuals only in the US and Canada; that is UNVERIFIED for 2026. Otherwise use Certum's open-source code-signing certificate or an OV certificate on a cloud HSM.

**Linux (M4):**
- The `.deb` post-install installs a udev rule with `TAG+="uaccess"`, `MODE="0660"` and a group. That replaces OM's `MODE="0666"`, which lets any local user or process drive the laser ([RT minor 14]).
- The AppImage offers a `pkexec` helper that shows exactly what it installs.
- Flatpak is deferred.

---

## 10. Risk register

| # | Risk | L | I | Mitigation |
|---|---|---|---|---|
| R1 | LMC flow control misread → overfill, timeouts, wedged board | M | H | Never write without READY; conditional dummy packet; no auto-reset; H2/H13/H14/H23 on red light in M1.1; emulator tuned from traces |
| R2 | Rust stop slower than 0.18 s, or unconfirmed | L–M | H | 20 ms slices; in-flight packet completed, not cancelled; CI p99 ≤ 100 ms; 20 traced trials plus optical H24; banner plus sticky fault; E-stop verified (H0) |
| R3 | Encoder mismatch (ties, trunc, `0xFFFF` vs `65536`, the +F/2 edge) → wrong power or size | M | H | 10k-case T0; `py-compat` against v0; byte-decoded confirmation; caliper check |
| R4 | AI code "proceeds on timeout" or clamps, the prototype's own failure class | M | H | Red zone; second-model review; mutation check plus nightly `cargo-mutants`; T6 invariants |
| R5 | Scope creep toward parity | H | H | Hard in/out lists; parking lot; design tools wait for M5; no empty stubs |
| R6 | 1.5–2× calendar slip erodes momentum | H | M | Re-baselined M1 (11 weeks); M1.2 is the priority checkpoint and moves rather than losing safety work; the GUI shrinks first; every milestone ships something usable |
| R7 | egui gaps force UI rework | L–M | M | Spike gates; UI-free `ol-app`; Tauri fallback with an S3 re-run |
| R8 | Ecosystem churn (egui every 1–2 months, nusb 0.2, wgpu lockstep) | H | L–M | Exact pins; quarterly upgrades; nightly `cargo tree -d`; wrappers |
| R9 | MOPA semantics wrong (H6–H9) or JPT PRR limits ignored | M | M | Python parity first; EZCAD capture (D11); PRR table once the model is known (M0 step 1) |
| R10 | Licence contamination (balor, GP30, firmware, AI recall) | L–M | H | Provenance rules, spec rule, `cargo-deny`, headers; `.cor` from galvoplotter, not balormk |
| R11 | Hardware sessions become the bottleneck | M | M | 8–10 sessions budgeted (D7); HIL-1 split in two; reserves; emulator absorbs the rest |
| R12 | Residual prototype risk until M2 | M | M | §11 patches (B1–B11), board lock, HIL-P with a ≥ 30 s hatch; `ol run` for long jobs from M1.2 |
| R13 | Name collision forces a late rename | H | L | Rename before the repo goes public; `publish = false` |
| R14 | An agent drives the laser | L–M | H | §3.6 bench layers: USB unplugged while executors work, hook, bench session, both rule files; physical ARM input in M3 |
| R15 | A host crash leaves MO open while the board drains | L | H | Run-ahead capped in packets and seconds; signals; sleep assertion; H25; `ol-driverd` if H25 demands; E-stop (H0) |
| R16 | CI minutes run out on a private repo | H | L | Heavy jobs on Linux; public at M2 (D17); never a self-hosted runner on the laser Mac |
| R17 | Windows signing unavailable to an individual | M | M | D18 before M4; Certum open-source or OV on a cloud HSM as fallbacks |

---

## 11. The Python prototype in the meantime

**Freeze its features now.** The only changes allowed are the patches below: the safety fixes, the job export the M1.2 bridge needs (#9), and the board lock (#12). That resolves the freeze contradiction in proposal C.

Work on a `safety-fixes` branch. Each patch gets a test, and each test becomes a T5 golden. Realistic effort is **6–8 focused days**. Patch 1 alone needs per-packet time estimates that the prototype lacks, since it computes only distance ([JP §5][JP]).

| # | Bug ([05][R05]) | Where | Patch | Test |
|---|---|---|---|---|
| 1 | **B1: 3 s wait cap.** MO closes at +3 s and an abort fires at +6 s; a packet can be written into a full buffer (§0.1, §1.10) | `mopa_luiz/live.py:455-482` (`apply_wait_timeouts`, applied at `:403`); job timeouts `live.py:28` (60 s), `ui.py:2870` (600 s per pass); GP `_list_write`/`_list_end` (`controller.py:1120-1150`) for the estimate hook | **Per-wait, progress-aware deadlines** ([RT C3]).<br>• **Estimate per packet.** Hook `_list_write` to sum each command's time: mark length ÷ mark speed, jump length ÷ jump speed, jump delay, laser on/off/polygon delays per vertex, and `DelayTime`. Record the total when the packet is written. The *projected end* is the `ExecuteList` time plus the running sum of packet estimates. (READY is not used to retire estimates: before `ExecuteList` it is set while nothing has run, and the buffer depth is unknown, H2.)<br>• **Each wait gets its own clock.** It starts when `wait_ready`, `wait_idle` or `wait_finished` is called and restarts whenever the status word changes. Budget = 1.5 × max(projected end − now, the largest estimate among the last three packets written) + 2 s. A generous budget only delays a stop; a tight one aborts good jobs, which is B1 again.<br>• **Before `ExecuteList` nothing is draining, so the budget is 2 s.** A board that never raises READY after `Fiber_SetMo(1)` and `WritePort(1)` (sent before packet 1, 05 §1.5–1.6) is therefore stopped within about 2 s.<br>• **FrameLoop** needs no special case: its waits are budgeted the same way, floored by the recent lighting packets' estimates, so it runs indefinitely while the board makes progress and is stopped when the status freezes.<br>• The 10 ms stop polling stays, so the 0.18 s stop fix survives.<br>• **On expiry:** set the stop event and a fault reason; **clear the spooler queue under `_spooler_lock` and `notify_all()`**; raise on the spooler thread. Without the clear, GP's `wait_for_spooler_send` (`controller.py:737-748`) waits forever, because `_spooler_run` (`:218-255`) returns without removing the job. Then the main thread runs the patched abort (#2), which closes MO only after `StopExecute`. Never write a packet after expiry.<br>• `frame_polylines`/`mark_polylines` (`live.py:661-751`) reach `finally: shutdown()` and raise a visible "board not READY/idle within N s" error.<br>• Outer timeouts become max(60 s, 2 × estimate + 60 s).<br>• An opt-in timestamped USB trace (`OPENMOPA_USB_TRACE=<file>`) wraps GP's connection reads and writes, for HIL-P | • `long_job` with a 20 s BUSY board: no `Fiber_SetMo(0)` before BUSY clears.<br>• A board that never raises READY: no packet written; MO(0) and `WritePort(0)` within 2.5 s of MO(1); the main thread returns.<br>• FrameLoop for 60 s on a board that drains each packet in 1 s: no deadline fires.<br>• FrameLoop with READY stuck: stopped within budget |
| 2 | **B2: Abort path** (engineering judge). GP `abort()` → `_list_end()` → `wait_ready()` runs before the dummy packet, so a raise skips the final MO(0)/`WritePort(0)`. OM runs `abort()` **after every job** | GP `controller.py:750-768, 1120-1133, 704-708`; OM `live.py:134-139, 751` | Replace `controller.abort` with a safe version:<br>• StopExecute → MO(0) → ResetList;<br>• poll READY for at most 50 ms, and send the dummy packet + SetEndOfList + ExecuteList only if READY;<br>• in a `finally`, always MO(0), `port_off`, `WritePort(0)`.<br>Waits inside it never raise | READY board: byte-identical to 05 §1.9 (`e479558e…`). Never-READY board: no 3072-byte write, MO(0) and `WritePort(0)` still sent |
| 3 | **B3: First jump elided** (G4) | GP `controller.py:156-157, 517-519`; OM `live.py:641, 732` (mark), `:524, 577, 685` (light) | **An explicit jump, not a sentinel.** A `(-1,-1)` sentinel would change every job's first `dist` word (1957 → about 44,385), which is untested on hardware. Before each list's first point: set the travel speed if `goto_speed` is set, `set_delay_jump(short)`, `list_jump(x0, y0)`; for light lists, call `light_off()` first. (The Rust settle in §3.3 is not back-ported, to keep v0/v1 packet diffs minimal) | `dot_test` contains `0x8001`; `mark_box10` SHA unchanged |
| 4 | **B4: Confirm shows Frame values** (G1) | `ui.py:2206-2222` (dialog), `:1540-1556` (summary), `:809-814` (Frame defaults); real values `:2839-2847` | `/api/plan` returns one row per emitting layer (power, kHz, ns, **actual** speed, passes) plus the repeat count; the dialog renders only these. The Frame panel becomes "Frame / Test-dot settings" and leaves the Mark dialog | Rows equal layer values; no Frame Power |
| 5 | **B5: ARM always true and sticky** (G2) | `ui.py:2151`; `safety.py:26, 64, 93-97` | **Server-side nonce** (C): `/api/plan` issues a single-use nonce, valid 60 s and bound to a hash of the payload; `/api/mark` must echo it or returns 403. `arm` is sent only from the confirm button, and the ARM field clears after each job and when the dialog closes | Replayed, expired or changed-payload nonce → 403 |
| 6 | **B6: Hidden + Output layers burn unframed** (G3) | `layers.py:46-51` vs `:195-196` | `/api/mark` refuses with 400: "Layer X is hidden but Output is on" | planner test |
| 7 | **B7: Speed < 3.61 mm/s → `MarkSpeed(0)`** (G5) | GP `controller.py:1004-1009`; OM `safety.py:108-145` (speed check `:127-128`) | Refuse when `trunc(v·0xFFFF/F/1000) < 1` with the effective F, and report the minimum. The actual speed appears in the patch 4 rows | 3 mm/s refused at F = 236.68 |
| 8 | **B8: Orphan objects dropped** (G7); **B9: 3-coordinate points crash** (G10) | `layers.py:175-177`; `:180` | Refuse with a message | two tests |
| 9 | **Bridge export** (the allowed exception) | `ui.py` `buildJobPayload` (`:2124-2140`) | An "Export job JSON" button posts the exact payload to `/api/export/job`. That writes `exports/job-<ts>.json` with sha256, `machine_id`, effective field, `scale_correction`, **`mirrorX`, `mirrorY` and `selectedOnly`**, keeps `live.repeat_count`, and **strips `arm`/`confirm`** ([RT I8]) | round-trip test; flags present |
| 10 | **B10: Field bound is the galvo's reach, not the lens's rated field** ([RT C1]) | `live.py:264-305` (`geometry_errors`: limit = effective F/2 + 1e-6), `:331` (`sanitize_polylines`); callers `ui.py:2745, 2825, 2933-2934`; F from `live.py:78-81` | Bound = min(effective F, nominal `FIELDSIZE`)/2 + 1e-6, where `FIELDSIZE` (200) is not multiplied by `scale_correction`. Applies to Frame and Mark. The message names the rated field | At scale 1.1834: (100, 0) accepted; (100.5, 0) refused for Mark and for Frame |
| 11 | **B11: G-code export leaks MOPA defaults** ([REPORT, takeaway 1][RF]: F15000, `M4 S1000` on the diode) | `/api/export/gcode`; its UI button | **Disable G-code export in OM**: hide the button, and have the endpoint return 410 "use Rayforge for gantries until OpenLaser M2". `gcode.py` stays for the golden harness, which calls it directly ([RT I12]) | endpoint returns 410; harness still runs |
| 12 | **Board lock** (no cross-process exclusivity, [RT I20]) | `live.py` `connect_controller` (`:385-452`), `FrameLoop.start` (`:553-591`), `/api/board` | Before opening USB, take an exclusive `flock` on `~/Library/Application Support/OpenLaser/board.lock` (the same path `ol` uses). If it is held, refuse with "another OpenLaser process holds the board" and never touch USB, so GP's reset-and-retry never runs against a live `ol` session | two-process test; H29 on hardware |

Also **pin `galvoplotter==0.2.0`** (`pyproject.toml:28-32`).

**Validation before daily use** (HIL-P, M0 step 3), with the USB trace on:
- **H0 first:** the E-stop and key switch stop emission mid-job (§5.2).
- 10 frame-loop stops of 0.25 s or less, with no USB resets;
- a red-light Frame Once over 8 s that runs to the end;
- **a low-power hatch of 30 s or more on scrap that completes, with the trace showing `Fiber_SetMo(0)` only after BUSY clears** ([RT C3]);
- a low-power dot at the field centre, whose trace starts with a `JumpTo`;
- a design point at 105 mm refused.

Then tag `oracle-v1`.

**Until M2:**
- The prototype stays the design front end. Its G-code export is off, so the gantries use Rayforge (§5.3).
- From M1.2, send long jobs through `ol run --job`.
- Per-job process spawning and the USB reset on close stay as they are (proven on hardware), behind the board lock.
- At M2 exit the repo is archived read-only, and the oracles keep generating goldens.

---

## Appendix A: evidence index

| File | Contents |
|---|---|
| [research/01-rayforge-source-deep-dive.md][R01] | Rayforge architecture; raygeo/raydriver; no galvo; PR #288; fork/addon/build decision |
| [research/02-lightburn-feature-inventory.md][R02] | LightBurn catalogue, T0–T3 tiers, `.lbrn2`, LightBurn's weak spots |
| [research/03-open-source-prior-art.md][R03] | Licences; galvoplotter/MeerK40t vs balor; `.cor` formats; name collisions |
| [research/04-rust-ecosystem-survey.md][R04] | Crate picks; egui vs Tauri; nusb; packaging; `cargo-deny` policy |
| [research/05-hardware-protocols-and-test-inputs.md][R05] | LMC spec; prototype bugs with file:line; golden tiers; H1–H20 |
| [research/06-critic-gaps-and-corrections.md][R06] | egui_tiles; vendoring raygeo; GP30; Windows driver swap; clone boards; Y-up; entitlements |
| [rayforge-test/REPORT.md][RF] | Hands-on Rayforge 1.12.0 test on this Mac against a fake GRBL |
| [design/proposal-A-galvo-vertical-slice.md][PA] | Base of this plan |
| [design/proposal-B-platform-first.md][PB] | Source of the structural safety grafts |
| [design/proposal-C-strangler-ecosystem.md][PC] | Source of the oracle, bridge and runtime-hygiene grafts |
| [design/judge-product-safety.md][JP] | Product/safety judgement |
| [design/judge-engineering-risk.md][JE] | Engineering-risk judgement |
| [design/redteam-review.md][RT] | Red-team review of revision 1; answered by revision 2 (Review log) |

## Appendix B: graft and fix ledger

### Grafts from proposal B

| Graft | Section |
|---|---|
| Confirmation decoded from the payload | §3.3 |
| TTY-only ARM with a nonce | §3.6, §7 |
| Unit newtypes | §3.1 |
| No remote ARM, enforced by `ol-arm`'s reverse-dependency check | §3.1 |
| Gantry overspeed refused; `home_on_connect = false` | §3.6, §5.3 |
| Python wait patch | §11 #1 |
| Rule 5 | §3.1 |
| Cold build at ARM | §3.6, M5 |
| HIL-0 | §5.2 |
| Capability sub-traits | §3.4 |
| CI stop p99 ≤ 100 ms | §3.5 |
| Run-ahead cap, with its bound stated honestly | §3.5 |
| Key off during red-light sessions | §5.2 |
| SVG divergence | §6.1 |
| `lmc-proto`/`lmc-sim` | M6+ |
| `Extras` bag and LightBurn fixtures | M5 |
| FakeGrbl over TCP | §5.3 |

### Grafts from proposal C

| Graft | Section |
|---|---|
| Test-only `py-compat` against `6aed8ff`, plus `DIVERGENCES.md` | §6.1 |
| Exact payload as the bridge input | §7, §11 #9 |
| Server-side ARM nonce | §11 #5 |
| Panic guard, `forbid(unsafe_code)`, sticky faults, no auto-reconnect | §3.1, §3.5 |
| FrameReceipt valid for 10 minutes | §3.6 |
| First emission at 10 % or less | §5.2 |
| H21/H22, with H21 gating the merge and raster | §3.3, §5.2 |
| Stop-under-load gate S3 | §4 |
| IME as a soft gate | §4 |
| First-run screen | §3.6 |
| FakeLmc scenarios | §6.2 |
| Inputs read in every wait loop | §3.5 |
| Rayforge addon only after M3 | D14 |

### New items from the judges

| Item | Section |
|---|---|
| Python abort-path fix | §11 #2 |
| Dead-man rule | §3.5 |
| Speed snapped at input; refuse 0 units only | D9 |
| H12 first | §5.2 |
| Spike in parallel with M1.1–M1.2 | §4 |
| `.cor` work pulled forward | M0, M2 |

### Errors fixed

| Error found | Fix |
|---|---|
| `wait_ready` hang with MO open | Per-wait progress-aware deadline, including before `ExecuteList`, plus the queue clear (§11 #1, revised in revision 2) |
| Sentinel changes `dist` | Explicit `list_jump`, SHA asserted (§11 #3) |
| Unconditional dummy packet | Sent only if READY (§3.5, §11 #2) |
| Rayforge addon overstated | §1, M6+ |
| C's timing and libusb contradictions | One timeline; libusb leaves with the prototype |
| SVG/STL parity claim | T4 is DXF only |
| Run-ahead overclaim | §3.5 |
| `py-compat` in a shipping crate | Test-only, dev-dependency-only, plus `compile_error!` |
| Missing Python estimator | Added, per packet |
| Mirror flags in the bridge | Exact payload, plus exported flags checked against the profile (revision 2) |
| `muda` as a must-pass gate | Soft gate |
| H5 too late | Pulled forward |
| M3 merge gated on H2 alone | H21 + H2 |
| B's Rule 5 wording | Restated |
| `stop()` writing to serial | The driver thread writes it |
| "Every crate stub" | Crates created per milestone |
| Undocumented T2/T5 divergences | Listed in `DIVERGENCES.md` |
| Two oracle SHAs needed | v0 + v1 |
| Freeze contradiction | Named exceptions only (#9 export, #12 lock) |
| H12 after H13 | H12 first |

---

## Review log (revision 2, answering [RT])

**Critical**
- **C1, field bound.** Split `encoding_field` (unit conversion only) from `work_area` (rated 200 mm lens field), and refuse outside the work area (§3.2, §3.6). Added B10 and Python patch #10, plus H27 (corners at ±100/±110 mm) and a `DIVERGENCES.md` entry. v1 now matches Rust, so T5 parity holds by design.
- **C2, ARM mint.** Removed the `arm-mint` feature and the crate cycle. Added a leaf `ol-types` crate (fingerprints, summary, receipt, opaque token) and an `ol-arm` crate whose reverse dependencies xtask pins to {`ol-ui`, `ol-cli`}, plus clippy `disallowed-methods` on the token constructor. `Encoded` moved to `ol-app` and `Backend` gained associated types. The "enforced" claim is rewritten to say exactly what is enforced (§3.1, §3.3, §3.4).
- **C3, Python patch #1.** Per-wait, progress-aware deadlines with per-packet estimates. A 2 s budget before `ExecuteList` covers the MO-open hang, and FrameLoop is defined. Added an opt-in USB trace, and a ≥ 30 s low-power hatch in HIL-P before `oracle-v1` (§11 #1, §7 M0).

**Important**
- **I1:** in-flight packets are completed (≤ 20 ms), not cancelled; one transfer in flight; H23 added; FakeLmc's partial-packet fault labelled a model (§3.5, §6.2).
- **I2:** H24 optical latency added to M1 exit #3; S3 re-run on the M1.3 build is exit #8.
- **I3:** explicit `panic = "unwind"`; SIGINT/SIGTERM/SIGHUP trip the latch; `caffeinate` assertion and App Nap opt-out; run-ahead capped in seconds as well as packets; post-crash bound shown at confirm; H25 and H28; `ol-driverd` decided at M2 from H25 (not built in M1, because H25 may show it isn't needed).
- **I4:** no modal file dialogs while framing or emitting; the focus caveat is shown at STOP; a live-resize rule (§3.5).
- **I5:** the first `JumpTo` uses the long jump delay and a 1 ms settle until H3/H19 (§3.3).
- **I6:** `red_dot` calibration in `GalvoMachine`; H26 (§3.2, §3.6).
- **I7:** H0 (E-stop and key switch cut the source) before any emission, re-checked at HIL-2; Physical layer 0 added to §3.6.
- **I8:** export carries `mirrorX`/`mirrorY`/`selectedOnly`; `ol run` applies no profile mirror and refuses a mismatch; `repeat_count` honoured and shown (§7 M1.2, §11 #9).
- **I9:** open Fill shapes become numbered notices; an empty Fill layer is refused; scope rule reworded; T3 unchanged (§2, §3.6).
- **I10:** `u16::try_from`; the +F/2 point is refused and listed as a T0 divergence; the "dead clamp" claim corrected (§3.2).
- **I11:** speed encoded once to whole units and never re-encoded from a float; proptest over F ∈ [100, 300] (§3.3, D9). The failure rate re-checked here was 13–20 % depending on sampling, and the u = 1 → 0 rate ~29 %.
- **I12:** OM's G-code export disabled (patch #11, B11); gantries use Rayforge.
- **I13:** per-gantry Rayforge bring-up checklist; D12 now asks what drives the gantries today (§5.3).
- **I14:** five bench layers: `AGENTS.md` + `CLAUDE.md` import, PreToolUse hook, bench session, USB unplugged (D19), physical ARM input in M3 (§3.6).
- **I15:** re-baselined M0 to 3 weeks and M1 to 11 (5 + 2 + 4). Zip projects, undo and kittest snapshots moved to M2. "Does not slip" removed. The red-zone gate is now behaviour review plus a second-model review. The timeline moves to 42 weeks to M5.
- **I16:** D7 is now 8–10 sessions; HIL-1 is split into 1a/1b; the session table is in §5.2.
- **I17:** M1 #9 replaced by a named workflow list (#10); T4 added to exit #1; M3, M4 and M5 exits made pass/fail.
- **I18:** heavy CI on Linux while private; repo public at M2 (D17); self-hosted runner on this Mac forbidden (§6.3).
- **I19:** Windows signing decision D18 with fallbacks; libwdi only as unmodified `wdi-simple.exe` or a DLL; root-certificate warning; M4 exit made conditional on D11/D12 hardware (§9, §7).
- **I20:** shared board lock in OM (patch #12) and `ol`; H29 (§3.5, §11).

**Minor (all taken)**
1. Tauri fallback must re-pass S3 and define a native ARM path (§4).
2. FrameReceipt needs ≥ 1 acknowledged pass; frame-before-mark is no longer a galvo setting (D8).
3. Single-command reads sliced and abandonable; stale replies drained (§3.4, §3.5).
4. Frame ⊇ burn wording for Fill (§3.6).
5. `.cor` and `scale_correction` mutually exclusive; `.cor` tables bound-checked; corners red-lit (M3).
6. Unitless SVG refused or asked about (M1).
7. GRBL preflight refusals (§5.3).
8. `ol-hw-raw` for H13/H23; HIL-0 sends no init.
9. "Manufatu" authorship check (M0); "can relicense" removed (§8).
10. `.cor` reader from galvoplotter `controller.py:865-919`, not balormk (§5.1).
11. Rayforge closures attributed to the stale bot; "Rayforge ships galvo" added to what would change the answer (§1).
12. The LightBurn galvo licence question added to D1.
13. Inconsistencies fixed: bugs numbered B1–B11 everywhere; sessions renamed HIL-P/0/1a/1b/2/3/4/5; EZCAD PC conditional in M0; T2 run with ordering, joining and dedupe off.
14. udev `uaccess` + `0660` (§9).
15. Duplicate-crate check extended to `kurbo` and `raw-window-handle` (S4, nightly).
