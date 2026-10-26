# 06 · Critic pass: contradictions, gaps and corrections across reports 01–05

Date: 2026-10-23. Author: critic subagent for the OpenLaser (Rust) plan. Scope: reports `01-rayforge-source-deep-dive.md`, `02-lightburn-feature-inventory.md`, `03-open-source-prior-art.md`, `04-rust-ecosystem-survey.md` and `05-hardware-protocols-and-test-inputs.md`, all read in full.

**Method and constraints**
- No USB or serial device was touched, and no third-party code was run or built.
- I cloned `kevinmehall/nusb` at tag `v0.2.8` (commit `c80218d`) **read-only** into `$SP/critic-nusb/`.
- I read the existing scratchpad clones (`rayforge-src`, `raygeo-src`, `raydriver-src`, `priorart-meerk40t`, `priorart-balor`, `priorart-galvoplotter`) and the installed galvoplotter in the prototype venv.
- My own data-parsing scripts live in `$SP/critic-scripts/`; outputs are in `$SP/critic-out/`. They parse `.lbrn2` XML as data and execute nothing from it.
- `$SP` = `/private/tmp/claude-501/-Users-luizbueno--claude-spares-spare-HuLL3w/ba10990c-59a4-470b-ab69-9ee5e8b41d86/scratchpad`.
- Registry, GitHub and GitLab facts come from `gh api`, the crates.io API and the GitLab v4 API on 2026-10-23.
- Evidence tags: **VERIFIED** (checked against a primary source or code here), **REFUTED**, **PARTLY**, **UNVERIFIED**.

---

## 0. What changes the plan

1. **Rayforge has no galvo support anywhere: VERIFIED.** That covers `main`, the 1.12.0 release, the 1.13.0 betas, the addon registry, open PRs and issues. The one attempt, PR #288, is closed and unmerged. Its branch `feat/galvo-driver` still exists and contains the clamping and half-scale bugs report 01 describes. The verdict in 01/02/03/04 stands.
2. **"No public Rust code drives JCZ/LMC boards" (03 §1.1, 04 finding 2) is REFUTED.**
   - [NicoPowers/GP30-Galvo-Driver](https://github.com/NicoPowers/GP30-Galvo-Driver) is a Rust (`rusb`) driver verified on a Monport GP30 that enumerates as `9588:9899 BJJCZ USBLMCV4`.
   - It is **red-light preview only**, a single commit (2026-07-02), and has **no licence**, so it is facts only and its code cannot be copied.
   - The differentiator still holds in substance: no *licensed*, *marking-capable* Rust LMC driver exists.
   - It adds one useful hardware fact. On its board the idle status word reads `0x0220` (READY plus an undocumented bit `0x0200`), so the Rust status parser must not assume the other bits are zero.
3. **The recommended GUI stack has a version break.**
   - `egui_dock` 0.21.1, the latest (2026-08-06), requires `egui ^0.36`, so it does **not** work with the recommended egui/eframe **0.37**.
   - Only `egui_tiles` 0.18.0 (2026-10-19) targets egui 0.37. egui_dock's repo moved to `anhosh/egui_dock` and has no code commits since 2026-08-06.
   - Fix: use `egui_tiles`, or pin egui 0.36.x until egui_dock catches up.
4. **raygeo is not a drop-in dependency (03 §7 and 04 §1 overstate this).**
   - The `python` feature is optional, but `src/lib.rs:141-142` installs an **unconditional `#[global_allocator]`** (a mimalloc wrapper). Any binary that links raygeo inherits it, and it conflicts with any other global allocator.
   - CI builds it only through `maturin-action` with the Python feature on.
   - Treat raygeo as **vendor-and-adapt source**: copy selected MIT files, add attribution and remove the allocator. Do not use it as a git dependency.
   - raydriver has no global allocator and is the cleaner candidate. It is still unbuilt and untested without Python (UNVERIFIED).
5. **Windows: no tool shares the board with EZCAD's driver, LightBurn included.**
   - LightBurn's installer swaps in its own driver (`dpinst-x64.exe`), and users "return to the original driver" to use EZCAD again. LightBurn's Zadig fallback installs WinUSB.
   - So a WinUSB swap is industry-normal, and nusb's WinUSB-only stance costs OpenLaser nothing extra. Report 03's open question of whether OpenLaser could coexist like LightBurn is answered: **neither coexists**.
   - Plan a reversible "Install / restore driver" helper built on **libwdi**. libwdi is LGPL-3, so run it as a separate executable. It auto-generates and installs a signing certificate, the same way Zadig works.
6. **Clone boards are a detection case nobody covered.**
   - MeerK40t's `balormk/clone_loader.py` shows that boards enumerating as **`0x9588:0x9980`** must have their FPGA/firmware uploaded at every power-up, from EZCAD's `Lmcv2u.sys`.
   - The file **embeds those vendor bytes in its source**. Do not port it, even though MeerK40t is MIT; the vendor firmware is not MeerK40t's to license.
   - OpenLaser's detect step should recognise PID `0x9980` and explain the situation instead of failing silently.
7. **`.lbrn2` coordinates are Y-up: VERIFIED on one MOPA sample.**
   - Plotting `fire_flower.lbrn2` against its embedded thumbnail, the Y-up reading matches and the Y-down reading is upside down (`$SP/critic-out/fire_flower-yaxis.png`).
   - Rayforge's importer (`is_y_down=False`) is right. MeerK40t's `extra/lbrn.py` applies no Y flip, so it would invert designs unless its bed config compensates.
   - This resolves 02 §17.2's open conflict, but confirm it on a second fixture.
8. **Galvo grayscale and MOPA raster have MIT prior art.** Reports 04 §7 and 05 left them UNVERIFIED.
   - MeerK40t `balormk/driver.py:459-498, 535-575` turns raster into jump/mark runs and **re-sends power (`MarkCurrent`) whenever the pixel level changes**.
   - Optionally, a **"value penbox"** maps each gray level to a complete settings set (power, frequency, pulse width, speed). That is exactly the shape a MOPA colour-preset feature needs.
   - What it does on real hardware is still UNVERIFIED.
9. **macOS signing: USB needs no entitlement.**
   - `com.apple.security.device.usb` is an **App Sandbox** entitlement. The hardened-runtime entitlement list has Camera but no USB.
   - A notarised, non-sandboxed OpenLaser needs only `com.apple.security.device.camera` plus `NSCameraUsageDescription`. This resolves 04 §9/§11's UNVERIFIED items.
10. **The prototype has 119 tests, not 105.** It has **14** test modules, not 15 (02 App. A). The "105" in the task context and memory is stale.
11. **The hands-on part of the user's request is not written up in any report.**
    - Luiz asked to "install it and test it here in this mac". Reports 01–05 are all read-only research.
    - A sibling run *did* install Rayforge 1.12.0 and drive it against a fake GRBL; artifacts are in `$SP` (§3, G1), but no report captures the results. That write-up is still owed to Luiz.

---

## 1. Contradictions between reports, and how each was resolved

| # | Topic | What the reports say | Resolution | Evidence |
|---|---|---|---|---|
| C1 | Does Rust LMC code exist? | 03 §1.1 and 04 finding 2: none on GitHub or crates.io. | **REFUTED for GitHub, confirmed for crates.io.** One unlicensed experiment exists (see §0.2). crates.io has nothing for `ezcad`, `bjjcz`, `jcz` or `balor`. Its `galvo` results are `laser-dac` and an unrelated crate; its `lmc` results are Little-Man-Computer and MQTT crates. | `gh api search/repositories` "galvo language:Rust"; GP30 `README.md`, `Cargo.toml` (`rusb = "0.9"`), `THIRD_PARTY_NOTICES.md` (cites galvoplotter, MIT); crates.io `/api/v1/crates?q=…` |
| C2 | balor licence | 03: GPL-3.0-or-later, and GitLab's API "misdetects" SSPL. 05: the LICENSE file says GPLv3, GitLab says SSPL, "the two disagree; either is copyleft". | **GPL-3.0-or-later.** The LICENSE file is GPLv3 text. `balor/sender.py` carries "either version 3 … or (at your option) any later version". The repo contains no SSPL text. GitLab's `license.key = sspl-1.0` is a detection artefact (SSPL is a modified GPLv3 text; the exact cause is UNVERIFIED). Practical outcome for both reports: never copy. | `$SP/priorart-balor/LICENSE`, `balor/sender.py` header; `GET gitlab.com/api/v4/projects/bryce15%2Fbalor?license=true` |
| C3 | Using raygeo as a dependency | 03 §7: "python feature optional, so the pure-Rust core can be used as a git dependency". 04 §1: "can be used as a git dependency (`default-features = false`)". 01 §8.2/§10: UNVERIFIED, with a global-allocator problem. | **01 is right.** `[features] default = ["python"]` is optional, but `#[global_allocator] static GLOBAL: RaygeoAlloc` at `src/lib.rs:141-142` is unconditional. `src/error.rs` uses pyo3 behind `cfg`. CI uses only `PyO3/maturin-action` (`.github/workflows/ci.yml:17,54`). A non-Python build is UNVERIFIED: there is no toolchain here, and the rule forbids building third-party code. Verdict: vendor files, do not depend. | `$SP/raygeo-src` @ `eab6fa8` |
| C4 | raygeo release cadence | 01: "123 releases since 2026-05-15". 03: "121 PyPI releases since May 2026". | **Both correct**: 123 GitHub releases versus 121 PyPI releases in the same window. | `gh api repos/barebaric/raygeo/releases`; `pypi.org/pypi/raygeo/json` |
| C5 | Nesting in the Rayforge ecosystem | 01 §4: nesting only via the **paid** Deepnest addon (BSL-1.1). 03 §1.4: raygeo "includes … nesting". | **Both true.** raygeo has an MIT `src/geo/algo/nest2d/` (NFP, IFP, genetic, gravity placement; about 2.3k lines). Rayforge exposes nesting to users only through the paid Deepnest addon (`registry.yaml`: `license: BSL-1.1`, Gumroad). For OpenLaser, raygeo's `nest2d` is an MIT alternative or complement to `sparrow`/`jagua-rs` (MPL). | `$SP/raygeo-src/src/geo/algo/nest2d/mod.rs`; `barebaric/rayforge-registry/registry.yaml` |
| C6 | Number of Rayforge dither algorithms | 03: "nine" (from the README). 01: 13 (from code). | **The code has 13** (`rayforge/image/dither.py:21-34`). The README is stale. | 01 §4 |
| C7 | Driver concurrency model | 03 §4.5: make waits cancellable with `tokio::select!` on a cancel token. 04 §8: one **OS thread** per machine with blocking I/O and short deadlines; tokio only for network. 05 §1.10: one "session-owning driver task". | **Compatible once one model is chosen.** The invariants matter more than the runtime: a single owner of the USB handle, a priority stop path, no unbounded wait, and no write unless READY. Pick **04's model for LMC**: a dedicated thread on nusb's blocking API (`wait_next_complete(timeout)`), an `AtomicBool` stop flag and a priority channel. nusb is "async-first, while not requiring an async runtime". If raydriver is reused for GRBL, it brings tokio behind the same `Driver` trait, which is acceptable. | nusb README; 04 §8; raydriver `Cargo.toml` (tokio, tokio-serial) |
| C8 | USB crate | 05 §1.1: "for Rust (`rusb` or `nusb`)". 03/04: `nusb`. | **nusb.** It is pure Rust with no libusb (LGPL), it is on macOS IOKit, and the Windows WinUSB requirement is shared by every non-EZCAD tool (C12). Use `rusb` only if a Windows user's board must stay on a libusbK/libusb0 driver (UNVERIFIED need). The only Rust LMC precedent (GP30) uses rusb on Linux. That is evidence the protocol works from Rust, not a reason to pick rusb. | nusb `src/lib.rs:132-175` @ v0.2.8 |
| C9 | Docking crate versus egui version | 04 §2.4/§14 pair egui 0.37.0 with `egui_dock` 0.21.1 "or" `egui_tiles` 0.18.0. | **egui_dock 0.21.1 is incompatible** (`egui ^0.36`). egui_tiles 0.18.0 requires `egui ^0.37.0`. | `crates.io/api/v1/crates/egui_dock/0.21.1/dependencies`, `…/egui_tiles/0.18.0/dependencies` |
| C10 | Prototype test count | Task context: 105. 02 App. A: "15 test modules". 05: 119 tests. | **119 `def test_` functions in 14 modules** (static count; nothing run). 05 is right. | `grep -c 'def test_' ~/AI/OpenLaser/OpenMopa/tests/test_*.py` |
| C11 | Galvo units per mm | 05 and the prototype: `0xFFFF / F` (327.675 at 200 mm). 03 §4.6: `.cor` scale = `65536 / field_mm`. | **Both conventions exist, even inside MeerK40t**: coordinates use `0xFFFF` (`balormk/device.py:1077,1143`), and lens size from a `.cor` is `65536/scale` (`gui/balorconfig.py:222`). The difference is 1/65536, about 3.6 µm over 236.68 mm: physically negligible but breaks byte-exact goldens. Use `0xFFFF` everywhere coordinates are encoded, for parity with the prototype. Use `65536/scale` only when deriving a field size from a `.cor`. | files cited |
| C12 | Windows coexistence with EZCAD | 03 §4.4/§13 UNVERIFIED: "If [LightBurn's dpinst route] is replicable, OpenLaser could coexist with EZCAD". 04/05: WinUSB takes the board from EZCAD. | **No one coexists.** LightBurn: "you'll need to return to the original driver" to use EZCAD, "You can switch back and forth". Zadig fallback is WinUSB ("Driver box should read LMCV2U"). Thunder Laser KB: only one can be in use at a time. Still UNVERIFIED: whether LightBurn's own driver package is WinUSB. If it is, nusb would work on a board already switched for LightBurn. | [LightBurn galvo driver guide](https://docs.lightburnsoftware.com/latest/Guides/GalvoDriverInstallation/); [Thunder Laser KB](https://support.thunderlaser.com/portal/en/kb/articles/connecting-an-aurora-to-ezcad-lightburn) |
| C13 | `.lbrn2` Y axis | 02 §17.2: third-party claims conflict. Rayforge: Y-up. MeerK40t: applies no flip. | **Y-up** (§0.7). | `$SP/critic-scripts/lbrn_render.py`; Rayforge `image/lightburn/importer.py:786-789`; MeerK40t `extra/lbrn.py:196-208` |
| C14 | Serial crate licence | 03 §1.4 and 04 §8: `tokio-serial` is MIT and `serialport` is MPL-2.0, as if choosing tokio-serial avoided MPL. | **MPL-2.0 arrives either way**: `tokio-serial` 5.5.0 and `mio-serial` 5.0.x both depend on `serialport ^4`. That is fine under 04's cargo-deny policy (MPL allowed). The choice is about the runtime model (C7), not the licence. | crates.io dependencies endpoints for both crates |
| C15 | LightCreator licence | 03: MIT OR Apache-2.0. GitHub API: `NOASSERTION`. | **MIT OR Apache-2.0** (`Cargo.toml license = "MIT OR Apache-2.0"`, plus `LICENSE` and `LICENSE-APACHE`). GitHub's NOASSERTION is the usual dual-licence artefact. | `gh api repos/michalsarna/LightCreator/contents` |
| C16 | Abort and stop sequence detail | 04 §8 leaves the LMC opcodes UNVERIFIED ("from galvoplotter consts.py"). 05 §1.9/§1.10 gives exact bytes. | **No conflict: 05 supersedes 04.** One addition from nusb (§2, item 6): after `cancel_all()` on a 3072-byte list write, assume the board may hold a **partial packet**. Always follow with `StopExecute` and `ResetList` before anything else. | nusb `src/device.rs:715-726, 845-898` |

---

## 2. Spot-checks of the most load-bearing claims

| # | Claim (reports) | Verdict | How it was checked / source |
|---|---|---|---|
| 1 | **Rayforge has no galvo/LMC support in any form** (01, 02, 03, 04; task context) | **VERIFIED** | Driver list in `rayforge/machine/driver/__init__.py` (main @ `a120f72`, 2026-10-23): GRBL ×4 (serial, telnet, network, serial-simple), Marlin, OctoPrint, Ruida, Smoothie, plus `NoDeviceDriver` in `dummy.py`. Galvo terms appear only in `lightburn_importer.py:30,33` (`"EZCAD": None`, `"Galvo": None`) and its test. Website docs: no hits. `rayforge-registry/registry.yaml` (147 lines): no galvo/fiber/EZCAD addon. GitHub: PR #288 `merged=false`, closed 2026-07-23; #261 and #308 closed `not_planned`; the only galvo branch is `feat/galvo-driver`. Latest stable release is 1.12.0 (2026-10-02); 1.13.0-beta4 (2026-10-21) is a pre-release. PR #288 also confirmed to have `(mm/field)*32768 + 32768` with a clamp (`galvo_encoder.py:94-96`). |
| 2 | **galvoplotter is MIT** (03, 05) | **VERIFIED** | `LICENSE`: "MIT License / Copyright (c) 2023 MeerK40t", in both the clone and the installed `galvoplotter-0.2.0.dist-info`. `METADATA`: `License: MIT`, `Author: Tatarize`. GitHub `spdx_id=MIT`; last push 2023-10-27. |
| 3 | **balor is GPL (do not copy)** (03, 05) | **VERIFIED** as GPL-3.0-or-later | See C2. GitLab's API label `sspl-1.0` is wrong. Either way, do not copy. |
| 4 | **MeerK40t is MIT, and `balormk` contains no balor code** (03) | **Licence VERIFIED; provenance claim UNVERIFIED** | `LICENSE`: "MIT License / Copyright (c) 2021 meerk40t"; GitHub `MIT`; last push 2026-09-25. `balormk/README.md:200-202` says it is "based on some of the original research and none of the original code". No line-by-line audit was done. **New caveat:** `balormk/clone_loader.py` embeds vendor firmware bytes from EZCAD's `.sys` (§0.6), so not everything in an MIT repo is MIT-clean. |
| 5 | **egui/eframe 0.37 is a mature enough primary GUI** (04) | **PARTLY**: facts VERIFIED, stack REFUTED, muda PLAUSIBLE | egui and eframe 0.37.0 were published 2026-10-15. The release notes list substantial AccessKit work (roles, panel and menu trees, `TextEdit` SetValue, and kittest failing when an input has no accessible name) plus several breaking changes, which confirms the 1–2-month breaking cadence. **Refuted:** the `egui_dock` pairing (C9). **muda + eframe:** precedent exists. [crmne/zapfast](https://github.com/crmne/zapfast) (2.1k★, MIT) builds a native macOS menu with muda (via `tray_icon::menu`) in an eframe 0.36 app, routing edit events into egui through a plugin (`src/macos.rs`). It patches egui to a fork (`crmne/egui` rev `6ce633b`), so 0.37 integration stays a spike item. |
| 6 | **nusb: pure Rust, works on macOS without setup, Windows needs WinUSB** (03, 04) | **VERIFIED** (v0.2.8, 2026-10-14, Apache-2.0 OR MIT, MSRV 1.85) | `src/lib.rs` docs: Windows "your device or interface must be associated with the WinUSB driver", via WCID, Zadig or libwdi. macOS "uses IOKit … Users have access to USB devices by default, with no permission configuration needed. Devices with a kernel driver are not accessible", and vendor-class devices may need `set_configuration(1)`. API names in 04 exist: `Endpoint::cancel_all` (`device.rs:723`), `wait_next_complete` (`:853`), `transfer_blocking` (`:872`), `watch_devices` (`lib.rs:399`). **Nuance for the stop path:** `transfer_blocking` cancels on timeout but then loops until the cancelled transfer returns, and `wait_next_complete` does *not* cancel on timeout. The stop path should use `submit` plus a `wait_next_complete(≤20 ms)` loop with stop checks, not long blocking transfers. |
| 7 | **No public Rust code drives LMC boards** (03, 04) | **REFUTED** | See C1. |
| 8 | **galvoplotter uploads a blank correction table and the prototype never passes a `.cor`** (03, 05; basis of the 1.1834 hypothesis) | **VERIFIED** | Installed `galvo/controller.py:780-790`: `init_laser()` calls `write_correction_file(self.cor_file)`. Lines `853-856`: `filename is None` leads to `write_blank_correct_file()`, which is `write_cor_table(False)`. Prototype: `live.py:392-399` passes `galvos_per_mm=0xFFFF/field_size` with no `cor_file`; `profile.py:56-57` says "Correction file is not loaded by this prototype." The **cause** of 1.1834 remains a hypothesis (hardware item H5). |
| 9 | Prototype test count (05) | **VERIFIED: 119** | C10 |
| 10 | Rayforge is MIT, macOS build is ad-hoc signed (01) | **VERIFIED** | GitHub `spdx_id=MIT` (raygeo and raydriver also MIT). `codesign -dv /Applications/Rayforge.app` gives `Signature=adhoc`, `TeamIdentifier=not set`. New: the bundle's `CFBundleShortVersionString` is **`0.0.0`** and `LSMinimumSystemVersion` is 12.0, so Finder and "About" will not show 1.12.0 (packaging quirk). |

---

## 3. Gaps: questions a LightBurn-class Rust plan needs answered that no report covers

Status: **FILLED** (answered here with sources), **PARTIAL**, **OPEN** (the plan must schedule it).

| # | Gap | Status | What we now know / what remains |
|---|---|---|---|
| G1 | **Hands-on Rayforge install and test on this Mac** (the user's explicit ask) | **OPEN** (needs a write-up) | **Evidence that it happened:**<br>• `$SP/rayforge-dl/rayforge-1.12.0-macos-arm-app.zip` (161,564,249 bytes) was installed to `/Applications/Rayforge.app` (ad-hoc signed).<br>• `$SP/rayforge-run1.log` and `rayforge-run2.log`: the app launched, the PyOpenGL 3D canvas was available, and it connected to "GRBL version 1.1h". That was a fake server, `$SP/tools/fake_grbl.py` on `127.0.0.1:2323` (Telnet driver).<br>• Two jobs ran: one completed, one was cancelled.<br>• The fake server logged 1,053 G-code lines plus feed-hold, cycle-start and soft-reset (the latter with 15 blocks queued).<br>• Warnings: GL shader validation fallback ×6 (the known #148 class), and libvips "5 bands, cropping to 4" ×22.<br>• State was left behind in `~/Library/Application Support/rayforge/` (machines, dialects, recipes, config) and `~/Library/Logs/rayforge/`.<br>**What is missing:** no report covers UX findings, stop latency from these logs, or a galvo-relevance verdict for the user. Note also that **LightBurn 1.5.04** (`com.LightBurnSoftware.LightBurn`, dated 2024-03-15, Developer-ID signed, team UWZQ3LL82C) is installed. None of 01–05 notice it (G10). |
| G2 | **Windows driver strategy** | **FILLED** (one sub-question OPEN) | C12 and §0.5. Vendor kernel drivers are `Lmcv2u.sys` / `LMCV4U.sys`. A LightBurn forum thread reports Windows **blocking `LMCV4U.sys` after an April 2026 update** ([forum](https://forum.lightburnsoftware.com/t/driver-issue-with-windows-update/189822); anecdotal), which strengthens the case for WinUSB. Plan: a libwdi-based helper (LGPL-3 separate exe; "Automated catalog file creation and signing, using autogenerated certificate", [libwdi README](https://github.com/pbatard/libwdi)) plus a documented "restore EZCAD driver" path. **OPEN:** is LightBurn's dpinst package WinUSB? |
| G3 | **Clone or unprogrammed boards** | **FILLED** | §0.6. PIDs: genuine `0x9899` (Luiz's board); clone FPGA-uninitialised `0x9980` (MeerK40t `clone_loader.py:16-17`); old USBLM-V1 `0x9999`/loader `0x9990` (report 03). The loader identifies `.sys` variants by size/offset (`clone_loader.py:370-381`), including "LMCV4". Do not ship firmware bytes. Offer to load from the user's own EZCAD install only if Luiz approves the legal posture (UNVERIFIED). |
| G4 | **Galvo raster and grayscale encoding** | **FILLED** (prior art); hardware effect OPEN | §0.8. MeerK40t also implements dot/dwell via `list_laser_on_point` in 10 µs units, chunked at ≤ 60000 per command (`balormk/driver.py:286-313, 499-523, 799-828`), which is relevant for dithered "dot" engraving. MeerK40t has the same power-per-run model for vectors (`PlotCut`). |
| G5 | **`.lbrn2` Y orientation** | **FILLED** (one sample) | §0.7. Confirm on a second file. LightBurn 1.5.04 on this Mac could generate fixtures if its licence or trial is valid (UNVERIFIED). |
| G6 | **macOS notarisation entitlements** | **FILLED** | §0.9. Apple doc data: [hardened runtime](https://developer.apple.com/documentation/security/hardened-runtime) lists Camera, Audio Input, Location, Address Book, Calendars, Photos, Apple Events and the code-execution exceptions, with no USB. [`com.apple.security.device.usb`](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.device.usb) is enabled via the "App Sandbox capability". [`com.apple.security.device.camera`](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.device.camera) is confirmed. |
| G7 | **JPT M7 frequency-versus-pulse-width limits** (05 G9/H8) | **PARTIAL** | No official datasheet found. Forum-sourced, from users quoting JPT tables ([LightBurn forum](https://forum.lightburnsoftware.com/t/ydflp-e2-100-m7-m-r-frequency-table/166830)):<br>• Each Q-pulse width has a **cutoff frequency**, the frequency at which 100% power is reached. Example: 500 ns gives 67 kHz on a 100 W E2.<br>• Below the cutoff, average power scales with frequency; above it, power stays flat and pulses weaken.<br>• Short pulses (2–9 ns) run to 4000 kHz.<br>• Per the JPT manual as quoted, the **source snaps invalid Q-pulse values to the nearest valid one**.<br>So G9 is mainly an *output-accuracy and UX* issue (show effective power), not an established damage risk. **OPEN:** Luiz's exact M7 model and wattage, and the manual's table. |
| G8 | **Single-line font licensing** (04 §6 UNVERIFIED) | **FILLED** | Hershey data: "may be used by anyone for any purpose, commercial or otherwise". Conditions: distribute the acknowledgements (A. V. Hershey at NBS; James Hurt, Cognition Inc.) with the data, and do not redistribute in the NTIS format ([Fedora wiki](https://fedoraproject.org/wiki/Licensing/HersheyFontLicense); [Ghostscript Hershey doc](https://web.mit.edu/ghostscript/src/gs6.01/doc/Hershey.htm)). OK to bundle with a NOTICE. |
| G9 | **Effort and scale**: no report sizes OpenLaser | **FILLED** (anchors only) | See the table below. |
| G10 | **LightBurn 1.5.04 already installed locally** | **OPEN** (opportunity) | It could act as a reference oracle without EZCAD: create `.lbrn2` fixtures (Y axis, sub-layers, galvo timing tags), and later run a USBLMCV4 side-by-side under supervision. Its licence state and macOS USB capture feasibility are UNVERIFIED. Note that it is pre-2.0 (Qt 5 era; 02 describes 2.2). |
| G11 | **Hardware-free LMC test targets** | **PARTIAL** | earlynerd/LaserEngraver (RP2350 "LMCV4 interface"): no licence, 0★, last push 2025-12-02, so not a dependable target. Best path stays 05's Python-oracle goldens plus a Rust fake board. GP30's idle status `0x0220` adds a second real-board datapoint (C1). |
| G12 | **Laser-safety and regulatory posture** | **OPEN** | No report addresses it. Things to decide in the plan:<br>• A disclaimer that OpenLaser is not a safety device.<br>• Never bypass or override hardware interlocks.<br>• Door/foot inputs (05 G8) as additional, not primary, protection.<br>• Whether FDA 21 CFR 1040.10/.11 or IEC 60825-1 obligations attach to software-only distribution (UNVERIFIED; they apply to laser *product* manufacturers).<br>MIT/Apache already disclaim warranty. |
| G13 | **One MVP definition** | **OPEN** | 02 uses tiers T0–T3, 04 milestones M1–M3, 05 golden tiers T0–T7. Nothing reconciles them. The plan must define M1 explicitly. The proposal implied by 01/02/05: LMC driver plus safety core plus SVG/DXF plus layers, Line/Fill, frame, mark, stop, GRBL export, and golden parity with the prototype. |
| G14 | **Rust toolchain on this Mac** | **OPEN** (action) | `cargo` and `rustc` are absent (04 notes this in passing). Every size and performance number in 04 is unmeasured until `rustup` is installed. Current stable: Rust 1.99 per 04 §13 (UNVERIFIED here). egui 0.37's MSRV is 1.95. |
| G15 | **Name and trademark** | **PARTIAL** | 03/04 cover the two GitHub collisions. No trademark search was done (UNVERIFIED). |
| G16 | **raydriver reuse for GRBL** | **PARTIAL** | Cleaner than raygeo: the `python` feature is optional, with no global allocator found, and it depends on tokio and tokio-serial. Untested without Python (UNVERIFIED). Its GRBL emulator is Python (01 §1.2), so an OpenLaser Rust CI would need its own emulator or `$SP/tools/fake_grbl.py`-style fixtures. |

**Size anchors for G9.** These are counted, not estimated; the implication below is my judgement.

| Codebase | Size | Notes |
|---|---|---|
| OpenLaser Python prototype (`mopa_luiz/*.py`) | 5,828 lines, 119 tests | Proven on hardware |
| galvoplotter | about 2.2k lines (03) | LMC protocol core |
| MeerK40t `balormk/` (incl. GUI) | 8,779 lines Python | Most complete open LMC driver |
| MeerK40t total | about 267k lines Python | Multi-controller laser suite |
| Rayforge | about 266k lines Python plus raygeo 106k Rust plus raydriver 6k Rust | One AI-assisted maintainer, about 4.4k commits, about 15 months |
| clayfrocker97/OpenLaser (GPL) | about 3.1 MB of Rust source (GitHub language bytes) | Active since 2026-09-17 |
| LightCreator + baseCreator (MIT/Apache) | about 1.3 MB of Rust each by GitHub language bytes; possibly shared or vendored code, UNVERIFIED | 2 weeks, likely AI-generated |

Implication: LightBurn/Rayforge-class parity is a 100k-plus-line effort. A galvo-first M1 that replaces the prototype is plausibly a few tens of thousands of lines of Rust. Plan for a strict M1 and treat parity as multi-year.

---

## 4. Corrections to apply when the plan is written

1. **03 §1.1 and 04 finding 2:** replace "no public Rust LMC code" with "one unlicensed, red-light-only Rust experiment (NicoPowers/GP30-Galvo-Driver); no licensed, marking-capable Rust LMC driver exists".
2. **04 §2.4 and §14:** change "egui_dock 0.21.1 or egui_tiles" to "**egui_tiles 0.18** (egui_dock 0.21.1 is egui 0.36 only)". Add a spike check that every egui-ecosystem crate shares one egui major.
3. **03 §1.4/§7 and 04 §1/§17 Q2:** change "raygeo usable as a git dependency" to "**vendor selected MIT modules** (optimize/TSP, tabs, nest2d, raster, text) with attribution. Strip `#[global_allocator]` (`src/lib.rs:141-142`). Add Rust tests (raygeo has none in Rust)."
4. **05 §1.1:** change "`rusb` or `nusb`" to "**nusb**; rusb only as a Windows fallback if a non-WinUSB driver must be supported".
5. **04 §8, fast-stop:**
   - Do not rely on `transfer_blocking` for stop-critical paths.
   - Use `submit` plus `wait_next_complete(≤20 ms)` loops with the stop flag.
   - After `cancel_all()`, assume a partial list packet may have reached the board, so always send `StopExecute` and `ResetList` (05 §1.9) before anything else.
6. **04 §9/§11:** drop the UNVERIFIED marks. No USB entitlement is needed outside the App Sandbox; Camera needs `com.apple.security.device.camera`.
7. **03 §4.4/§13:** resolve the "LightBurn coexists via dpinst?" item. It does not; the driver swap is required (C12).
8. **03 §4.2:** add "**Do not port `balormk/clone_loader.py`.** It embeds vendor firmware bytes." Add PID `0x9980` to detection with an explanatory message.
9. **02 §17.2:** `.lbrn2` is **Y-up** (verified on `fire_flower.lbrn2`; confirm on a second fixture).
10. **02 App. A and the task context:** the prototype has **119 tests in 14 modules**.
11. **04 §7 and 05 §7 H6/H7:** cite MeerK40t's galvo raster model (power change per run; per-gray-level "penbox" settings) as the starting design. Keep the hardware effect UNVERIFIED.
12. **03 §2 / 04 §16 licence notes:** `tokio-serial` also pulls in `serialport` (MPL-2.0). `libwdi` is LGPL-3 (GitHub shows GPL-3.0; the README says LGPLv3), so ship it only as a separate executable.
13. **Golden tests (05 §5.3):** fix the coordinate conversion to `0xFFFF / F`, and document `65536/scale` as `.cor`-only (C11).
14. **Status parsing (05 §1.2):** treat bits other than BUSY/READY/AXIS as unknown but expected. GP30's idle word is `0x0220`.

---

## 5. Still UNVERIFIED after this pass

1. Whether raygeo and raydriver compile with `--no-default-features`. Not checked: no toolchain, and the rule against building third-party code.
2. Whether LightBurn's Windows galvo driver package is WinUSB-based (decides whether nusb works on boards already switched for LightBurn).
3. Whether MeerK40t `balormk` is entirely free of balor-derived code (project assertion only).
4. muda native menus with eframe **0.37** on macOS 26 (precedent exists only on 0.36 with a patched egui).
5. The physical effect of per-run power changes and dot dwell on the JPT M7. JPT M7 per-pulse-width cutoff table for Luiz's exact model.
6. `.lbrn2` Y-up on a second sample; XML tags for galvo timing, wobble and sub-layers (02 §21).
7. The cause of the 1.1834 factor (blank `.cor` hypothesis; hardware item H5).
8. Regulatory obligations, if any, for distributing laser-control software (G12).
9. Trademark status of "OpenLaser" (G15).
10. Rayforge stop latency on real hardware (still not measured; the sibling fake-GRBL logs could give a software-side figure).

---

## 6. Sources (beyond those cited inline)

- Rayforge: `$SP/rayforge-src` @ `a120f72` (`rayforge/machine/driver/__init__.py`, `rayforge/machine/device/lightburn_importer.py:24-35`, `rayforge/image/lightburn/importer.py:786-789`, `rayforge/image/base_importer.py:208`); branch `origin/feat/galvo-driver:rayforge/machine/driver/galvo/galvo_encoder.py:94-96`; `gh api repos/barebaric/rayforge/{pulls/288,issues/261,issues/308,branches,releases,releases/latest}`; `gh api repos/barebaric/rayforge-registry/contents/registry.yaml`.
- raygeo: `$SP/raygeo-src` @ `eab6fa8` (`Cargo.toml`, `src/lib.rs:100-142`, `src/error.rs`, `src/geo/algo/nest2d/`, `.github/workflows/ci.yml`). raydriver: `$SP/raydriver-src/Cargo.toml`.
- nusb: <https://github.com/kevinmehall/nusb> @ `v0.2.8` / `c80218d` (`src/lib.rs:107-175`, `src/device.rs:715-726, 845-898`).
- egui: <https://github.com/emilk/egui/releases/tag/0.37.0>; crates.io API for `egui`, `eframe`, `egui_dock` 0.21.1 and `egui_tiles` 0.18.0 dependencies; <https://github.com/anhosh/egui_dock> commits; <https://github.com/crmne/zapfast> (`src/macos.rs`, `Cargo.toml`).
- MeerK40t: `$SP/priorart-meerk40t` @ `d511944` (`LICENSE`, `meerk40t/balormk/README.md:200-202`, `clone_loader.py`, `driver.py`, `device.py:1076-1144`, `gui/balorconfig.py:222`, `extra/lbrn.py:180-210`).
- galvoplotter: `$SP/priorart-galvoplotter/LICENSE`; `~/AI/OpenLaser/OpenMopa/.venv/lib/python3.10/site-packages/galvo/controller.py:780-790, 850-880`; `galvoplotter-0.2.0.dist-info/{LICENSE,METADATA}`.
- balor: `$SP/priorart-balor` @ `dfc7faf` (`LICENSE`, `balor/sender.py`); GitLab API `projects/bryce15%2Fbalor?license=true`.
- Prototype: `~/AI/OpenLaser/OpenMopa` @ `6aed8ff` (`tests/`, `mopa_luiz/live.py:392-399`, `mopa_luiz/profile.py:56-58`).
- Rust LMC precedent: <https://github.com/NicoPowers/GP30-Galvo-Driver> (README, `Cargo.toml`, `THIRD_PARTY_NOTICES.md`).
- Windows: <https://docs.lightburnsoftware.com/latest/Guides/GalvoDriverInstallation/>; <https://forum.lightburnsoftware.com/t/driver-issue-with-windows-update/189822>; <https://support.thunderlaser.com/portal/en/kb/articles/connecting-an-aurora-to-ezcad-lightburn>; <https://github.com/pbatard/libwdi>.
- Apple: <https://developer.apple.com/documentation/security/hardened-runtime>; <https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.device.usb>; <https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.device.camera> (read via the `/tutorials/data/…json` endpoints).
- Fonts: <https://fedoraproject.org/wiki/Licensing/HersheyFontLicense>; <https://web.mit.edu/ghostscript/src/gs6.01/doc/Hershey.htm>.
- JPT MOPA behaviour (forum, secondary): <https://forum.lightburnsoftware.com/t/ydflp-e2-100-m7-m-r-frequency-table/166830>.
- Serial deps: crates.io `tokio-serial/5.5.0/dependencies`, `mio-serial/5.0.6/dependencies`.
- Local apps: `/Applications/Rayforge.app` and `/Applications/LightBurn.app` (`Info.plist`, `codesign -dv`); sibling test artifacts `$SP/{rayforge-dl,rayforge-run1.log,rayforge-run2.log,fake_grbl.log,tools/}`.
- My scripts (data parsing only): `$SP/critic-scripts/lbrn_yaxis.py`, `$SP/critic-scripts/lbrn_render.py`; output `$SP/critic-out/fire_flower-yaxis.png`.
