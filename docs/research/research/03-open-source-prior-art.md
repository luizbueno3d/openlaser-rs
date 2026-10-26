# 03 · Open-source prior art and protocol references for OpenLaser (Rust)

Research date: 2026-10-22. Author: research subagent for the OpenLaser Rust plan.

## How this was researched

- Repository metadata (licence, stars, last push, releases, top contributors) comes from the GitHub REST API, queried with `gh api` on 2026-10-22/23. GitLab data comes from the GitLab v4 API. Crate data comes from the crates.io API, and Python package data from the PyPI JSON API.
- Four repositories were shallow-cloned and read only. Nothing from them was built, installed or executed:
  - `priorart-meerk40t` at `d511944`
  - `priorart-galvoplotter` at `fc35136`
  - `priorart-balor` at `dfc7faf`
  - `priorart-rayforge` at `a120f72`

  All four sit under `/private/tmp/claude-501/-Users-luizbueno--claude-spares-spare-HuLL3w/ba10990c-59a4-470b-ab69-9ee5e8b41d86/scratchpad/`. The source of the `ruida` crate (0.1.1) was downloaded to `priorart-ruida-crate/` in the same place and read only. The Rayforge Rust sources `raygeo-src/` and `raydriver-src/` were already in the scratchpad, put there by a sibling agent. I read them too.
- No USB or serial device was touched.
- Every licence below was checked against the actual LICENSE file or source headers. GitHub's "NOASSERTION" label was not taken at face value.
- **UNVERIFIED** marks anything I could not confirm from a primary source.

---

## 1. Summary of findings

1. **The Rust galvo gap is real.** No public Rust code drives JCZ/BJJCZ LMC galvo boards. I searched GitHub repos and code for `0x9588`, `bjjcz`, `lmcv4`, `ezcad` and `galvo` restricted to Rust. I also searched crates.io for `ezcad`, `bjjcz` and `galvo`. The only galvo-adjacent Rust code drives laser-show DACs (`laser-dac`, `nannou_laser`) or XY2-100 on an MCU.
2. **Rayforge has no galvo driver.** The built-in drivers in `rayforge/machine/driver/` are `dummy`, `grbl`, `marlin`, `octoprint`, `ruidarpa` and `smoothie`.
   - Its LightBurn device importer maps `"EZCAD": None` and `"Galvo": None` (`rayforge/machine/device/lightburn_importer.py:26-33`).
   - A "semi vibe coded" EzCad2 driver was proposed in [PR #288](https://github.com/barebaric/rayforge/pull/288) (+2,979 lines). It was **closed unmerged on 2026-07-23**.
   - The feature request [#261](https://github.com/barebaric/rayforge/issues/261) was closed as *not planned*.
   - A galvo/MOPA driver would therefore set OpenLaser apart from Rayforge.
3. **All the BJJCZ protocol knowledge OpenLaser needs is available under MIT.** Two sources carry it:
   - [galvoplotter](https://github.com/meerk40t/galvoplotter), the library the Python prototype already uses on real hardware.
   - MeerK40t's `balormk` module.

   Bryce Schroeder's original *balor* is **GPL-3.0-or-later**. MeerK40t says `balormk` was rewritten and keeps "none of the original code" (`meerk40t/balormk/README.md`, "Licensing" section). That makes MeerK40t and galvoplotter, not balor, the sources to port.
4. **Rayforge already ships MIT-licensed Rust.**
   - [raygeo](https://github.com/barebaric/raygeo) is about 106 k lines of Rust: geometry, booleans via clipper2, arc fitting, nesting, dithering and raster power modulation.
   - [raydriver](https://github.com/barebaric/raydriver) is about 6.2 k lines of Rust: a tokio GRBL driver with character-counting flow control, stall and deadlock recovery, and a firmware emulator.
   - Neither is on crates.io. Each has one dominant maintainer and changes very fast (raygeo has had 121 PyPI releases since May 2026).
   - Both are legally portable into an MIT/Apache project and are the most directly reusable Rust code found.
5. **Lead on the 1.1834 mark-size factor (hypothesis, unverified on hardware).**
   - When `cor_file=None`, galvoplotter's `init_laser()` uploads a *blank* correction table (`galvo/controller.py:781-790, 854-861`).
   - The prototype never passes a `.cor` file. `profile.py:56-57` says *"Correction file is not loaded by this prototype"*, and `live.py:392-399` builds the controller with `galvos_per_mm=0xFFFF/field_size`.
   - A JCZ `.cor` file stores a `scale` in galvo units per mm, and `lens_size = 65536/scale`:
     - MeerK40t: `balormk/gui/balorconfig.py:216-223`
     - [corfile2meerk40t SPEC](https://github.com/kausalinverse/corfile2meerk40t/blob/main/SPEC.md) §3.2
   - The measured effective field of 236.68 mm is what the uncorrected galvo would show if the factory `.cor` was calibrated for a larger real field. Reading the machine's original EZCAD `.cor` file would probably remove the need for a hand-tuned factor and also correct barrel/pincushion distortion. This is the most valuable concrete takeaway for the galvo work.
6. **Name collision.** Two active projects already use "OpenLaser" in exactly this domain:
   - [theanam/openlaser.app](https://github.com/theanam/openlaser.app): MIT, browser GRBL control, created 2026-10-19, 127 stars after 3 days, domain `openlaser.app`.
   - [clayfrocker97/OpenLaser](https://github.com/clayfrocker97/OpenLaser): GPL-3.0, a Rust server plus touch UI for MCC100 fiber/CO2 controllers, created 2026-09-17.
   - The crates.io name `openlaser` and the PyPI name `openlaser` were both free on 2026-10-22.
7. **LightBurn published an MIT protocol.** The [LightBurn Protocol (LBP)](https://github.com/LightBurnSoftware/lbp) is a binary command/response protocol (magic `DRGN`, big-endian, CRC16) with a C++ reference library and simulator. It is at version 0.9, "Alpha 10", committed 2026-10-19. It is a candidate future firmware target and is safe to implement.
8. **Licensing (§11).** The recommended licence is **MIT OR Apache-2.0**: the Rust convention, compatible with the MIT prototype and with every reference that matters. GPL projects are read for behaviour only and never copied. A clean-room/provenance rule keeps that true.

---

## 2. How each licence class can be used

| Licence class | Projects here | Can OpenLaser (MIT OR Apache-2.0) copy or port code? |
|---|---|---|
| MIT, BSD-3-Clause, Apache-2.0, MIT/Apache | MeerK40t, galvoplotter, Rayforge, raygeo, raydriver, cncjs, ioSender, svg2gcode, gcode-rs, ruida-pa, lbrnts, baseCreator/LightCreator, vpype, vsvg, Deepnest-next, SVGnest, Kiri:Moto, LBP, corfile2meerk40t, Graphite (Apache-2.0) | **Yes.** Keep the original copyright and licence notice in the ported files and in `THIRD_PARTY_NOTICES`. A Python-to-Rust translation is a derivative work, so the notice applies. |
| MPL-2.0 (file-level copyleft) | jagua-rs, `serialport` crate | Using the crate as a dependency is fine. Modified MPL files stay MPL. Copying snippets into OpenLaser files is not OK. |
| LGPL-2.1/3.0 | VisiCut, LibLaserCut (LGPL-3.0-or-later), libusb (LGPL-2.1+), GTK4 runtime | Code is reference only. Linking a library is fine if users can relink it, which is awkward for static Rust binaries. Prefer pure-Rust replacements. |
| GPL-2.0 / GPL-3.0 / AGPL-3.0 | balor, LaserGRBL, K40 Whisperer, bCNC, UGS, Candle, gSender, OpenBuilds CONTROL, LaserWeb4, grblHAL, FluidNC, Grbl, jnweiger/ruida-laser, potrace, lmc_corfile_viewer, ivaCAM, SnartCut, clayfrocker97/OpenLaser | **No copying, translating or close paraphrasing.** Facts such as opcodes, byte layouts, protocol behaviour and feature ideas can be reimplemented from understanding. Only possible if OpenLaser itself goes GPL (see §11). |
| CC BY-NC-SA 3.0 (docs) | EduTech Wiki "Ruida" ([copyright page](https://edutechwiki.unige.ch/en/EduTech_Wiki:Copyrights)) | Use the facts. Do not paste the prose or tables into OpenLaser docs, because NC conflicts with permissive redistribution. |
| No licence (all rights reserved) | earlynerd/LaserEngraver, charlie-x/fibrelasertools, Stefan Schuermans' RDCAM pages, gcodekit4 (none detected) | Facts only. |

---

## 3. Master table

Status is as of 2026-10-22. "Last release" means the latest GitHub release unless noted otherwise.

| Project | Language / UI | SPDX (verified) | Activity | Controllers | Port into MIT/Apache Rust? |
|---|---|---|---|---|---|
| [MeerK40t](https://github.com/meerk40t/meerk40t) | Python / wxPython (AUI) | MIT | 0.9.8100 on 2025-08-15. `main` is at 0.9.9040 with "weekly" tags; last commit 2026-09-25. 440★, 28 contributors (tatarize, jpirnay). README says **maintenance mode**. | Lihuiyu M2/M3 Nano (K40), GRBL, **JCZ/EZCAD2 galvo (balormk)**, Moshiboard, NewlyDraw, Ruida, Ruida emulator | **Yes (MIT)** |
| [galvoplotter](https://github.com/meerk40t/galvoplotter) | Python library (pyusb) | MIT | 0.2.0 on 2023-09-27; last commit 2023-10-27. 23★, 1 author (tatarize). Dormant but stable. | BJJCZ LMC (fiber, CO2) | **Yes (MIT)** |
| [balor](https://gitlab.com/bryce15/balor) (Bryce Schroeder) | Python CLI | GPL-3.0-or-later. GitLab's API misdetects it as SSPL; the LICENSE file is GPLv3 and the headers say "or any later version". | Last activity 2022-11-06. 23★. | BJJCZ LMCV4-FIBER-M | **No (GPL).** Read for ideas only. |
| [Rayforge](https://github.com/barebaric/rayforge) | Python / GTK4 + libadwaita | MIT | 1.12.0 on 2026-10-02; 1.13.0-beta4 on 2026-10-21. 301★, 23 contributors (knipknap ≈ 4.4 k commits). Very active. | GRBL (serial/telnet/websocket), Marlin, Smoothie, OctoPrint, Ruida (via ruida-pa), dummy. **No galvo.** | **Yes (MIT)** |
| [raygeo](https://github.com/barebaric/raygeo) / [raydriver](https://github.com/barebaric/raydriver) | **Rust** + PyO3 | MIT / MIT | raygeo v1.67.1 on 2026-10-22; raydriver v0.5.1 on 2026-10-17 | GRBL (raydriver) | **Yes (MIT), and already Rust** |
| [LaserGRBL](https://github.com/arkypita/LaserGRBL) | C# / WinForms, Windows only | GPL-3.0-or-later (`LICENSE.md`) | v7.14.1 on 2025-03-01; last commit 2025-05-29. 1,663★. | GRBL 0.9/1.1 | No |
| [LaserWeb4](https://github.com/LaserWeb/LaserWeb4) (+ [lw.comm-server](https://github.com/LaserWeb/lw.comm-server)) | JS / React web app + Node server | AGPL-3.0 (+ GPL-3.0 for the comm server) | Last binaries v4.1.000 on 2021-12-24; only a Docker fix commit in 2026-04. 807★. Effectively legacy. | Grbl, Grbl-Mega, Grbl-LPC, Smoothieware, TinyG, ESP8266 WebSocket bridge; Marlin/MarlinKimbra/RepRapFirmware "not finished" ([lw.comm-server README](https://github.com/LaserWeb/lw.comm-server)) | No |
| [cncjs](https://github.com/cncjs/cncjs) | JS / Node + React web UI | MIT | v1.11.5 on 2026-09-13. 2,658★. | Grbl, Grbl-Mega, Marlin, Smoothieware, TinyG, g2core | **Yes (MIT)** |
| [VisiCut](https://github.com/t-oster/VisiCut) + [LibLaserCut](https://github.com/t-oster/LibLaserCut) | Java / Swing | LGPL-3.0-or-later (per `copyrightheader`) | 2.1 on 2024-05-06; "continuous" build 2026-10-12. 296★. | Epilog, LAOS, Lasersaur, FullSpectrum, GRBL, Marlin, Smoothie, Ruida, K40 Nano, GoldCut HPGL, iModela, and more | No (LGPL) |
| [K40 Whisperer](https://www.scorchworks.com/K40whisperer/k40whisperer.html) | Python / Tkinter | GPL-3.0-or-later (header in [mirror](https://github.com/jkramarz/K40-Whisperer)) | 0.71 per the official page (date not given). No official git repo. | Lihuiyu M2/M3 Nano (no Moshi) | No |
| [OpenBuilds CONTROL](https://github.com/OpenBuilds/OpenBuilds-CONTROL) | JS / Electron | **Ambiguous:** LICENSE is GPL-3.0 text, `package.json` says `AGPL-3.0` | v1.0.390 on 2025-06-02, no commits since. 269★. | Grbl (README: "all CNC style machines running Grbl"); grblHAL BlackBox X32 support is UNVERIFIED | No |
| [bCNC](https://github.com/vlachoudis/bCNC) | Python / Tkinter | GPL-2.0-only (`setup.py`) | PyPI release 2025-03-27; commits 2026-10-22. 1,751★. Mac support "seems broken" per README. | grblHAL/GRBL, Smoothie | No |
| [Universal G-Code Sender](https://github.com/winder/Universal-G-Code-Sender) | Java / NetBeans Platform | GPL-3.0-or-later (headers) | v2.1.27 on 2026-09-10. 2,273★. | GRBL, grblHAL, FluidNC, TinyG, g2core, Smoothieware | No |
| [Candle](https://github.com/Denvi/Candle) / [gSender](https://github.com/Sienci-Labs/gsender) | C++/Qt · TypeScript/Electron | GPL-3.0 · GPL-3.0-only | v11.2 on 2026-02-09 · v1.6.4 on 2026-09-17 | GRBL (+ grblHAL in gSender) | No |
| [ioSender](https://github.com/terjeio/ioSender) | C# / WPF, Windows | **BSD-3-Clause** | Commits 2026-10-20. 288★. | grblHAL, GRBL | **Yes (BSD-3)** |
| [Grbl](https://github.com/gnea/grbl) / [grblHAL](https://github.com/grblHAL/core) / [FluidNC](https://github.com/bdring/FluidNC) | C (AVR) / C (32-bit HAL) / C++ (ESP32) | GPL-3.0-or-later for all three | Grbl v1.1h on 2019-08-26 (frozen); grblHAL build 20261015; FluidNC v4.1.1 on 2026-09-25 | Firmware **targets** | Implement the protocol from the docs. Never copy firmware code. |
| [LightBurn Protocol (LBP)](https://github.com/LightBurnSoftware/lbp) | C++ library + Qt simulator | MIT | v0.9 "Alpha 10" on 2026-10-19 | Future LBP-speaking firmware | **Yes (MIT).** Branding is governed separately by `POLICY.md`. |
| [svg2gcode](https://github.com/sameer/svg2gcode) | **Rust** (lib + CLI + Yew/WASM web) | MIT | crate 0.3.4 on 2026-03-04; commits 2026-10-15. 445★. | G-code output | **Yes** |

---

## 4. JCZ / BJJCZ LMC galvo references (OpenLaser's differentiator)

### 4.1 galvoplotter (MIT) is the cleanest reference

- **What it is.** A "low level command system for ezcad2 lmc-controller galvo lasers", supporting BJJCZ `fiber` and `co2` ([README](https://github.com/meerk40t/galvoplotter/blob/main/README.md)). It has about 2.2 k lines: `galvo/controller.py`, `galvo/usb_connection.py`, `galvo/consts.py` and `galvo/mock_connection.py`.
- **Architecture worth copying.** Commands are split into *realtime* (status, jog, pause, resume, abort) and *sequential* (list) commands. There are two threads (main and spooler) and a job abstraction that may be infinite, such as light/preview loops. The connection has five states (`init`, `opening`, `opened`, `closed`, …). A mock connection prints every packet.
- **Relevance.** The user's prototype runs on it today: `from galvo.controller import GalvoController` in `mopa_luiz`. A 1:1 Rust port is the lowest-risk path to a working galvo driver, because behaviour can be diffed packet for packet against the Python stack the user has already validated on the JPT M7.
- **Caveat: correction table.** `init_laser()` writes the correction table on every connect. With no file it sends a *blank* table (`controller.py:781-790, 854-861`); see §4.6.

### 4.2 MeerK40t `balormk` (MIT) is the most complete and maintained BJJCZ driver

- **Location.** `meerk40t/balormk/` in [meerk40t/meerk40t](https://github.com/meerk40t/meerk40t/tree/main/meerk40t/balormk), about 7.2 k lines not counting `gui/`. Files:
  - `controller.py` (1,682 lines)
  - `galvo_commands.py`
  - `driver.py`
  - `device.py`
  - `livelightjob.py` (live red-light preview)
  - `cylindermod.py` (rotary)
  - `clone_loader.py` (for clone boards)
  - `usb_connection.py`
  - `mock_connection.py`
- **Provenance.** The README's "Licensing" section says the original GPL balor code "was completely scrapped" and replaced. The new code is "based on some of the original research and none of the original code." So the code is MIT. I did not audit it line by line against balor (UNVERIFIED), and a casual comparison shows different structure. A cautious port should still treat galvoplotter (single author, MIT) as the primary source and MeerK40t as secondary.
- **Extras galvoplotter lacks:**
  - correction-file parsing and lens size derived from `.cor` (`device.py:115-133`, `gui/balorconfig.py:216-223`)
  - per-operation MOPA parameters
  - live-light jobs
  - rotary
  - a full GUI configuration surface for timing and delays (first-pulse killer, PWM half-period and pulse width, standby, jump/laser-on/off/polygon delays)
- **Other relevant MeerK40t modules:**
  - `meerk40t/extra/ezd.py`: EZCAD2 `.ezd` parser, 2,175 lines, see §8
  - `extra/lbrn.py`: LightBurn import
  - `extra/xcs_reader.py`: xTool Creative Space
  - `ruida/`: Ruida driver and **emulator**
  - `grbl/`: GRBL driver and **emulator**
  - `camera/camera.py`: OpenCV fisheye chessboard calibration and 4-point perspective warp
- **Licence hygiene.** The optional potrace tracer is GPL and deliberately *not bundled* (`extra/potrace.py` header). `meerk40t/lmc_corfile_viewer` is **GPL-3.0**, so don't copy from it.
- **Version discrepancy.** The README links "latest released version 0.9.9000 (Jan 17, 2026)", but no such GitHub release exists. `releases/tags/0.9.9000` returns 404, and the latest release is 0.9.8100 from 2025-08-15. Treat 0.9.8100 as the last stable release (UNVERIFIED whether 0.9.9000 was published elsewhere).

### 4.3 balor (Bryce Schroeder, GPL-3.0-or-later): read for ideas only

- **Origin.** It came from watching EZCAD's USB traffic (Hackaday coverage: <https://hackaday.com/tag/balor/>; project page <https://www.bryce.pw/engraver.html>). Its repo includes a USB capture (`Documentation/ThreeOvals.pcap`), board photos and `balor/BJJCZ_LMCV4_FIBER_M_blobs.py`. That file holds replayable captured init/transaction sequences, including correction-table writes.
- **Idea worth reimplementing.** `balor/Cal.py` builds a calibration from measured mm↔galvo point pairs with RBF interpolation, using a vendored SciPy `RBFInterpolator` (BSD-3). This is a *software* lens correction that is independent of `.cor` files. The idea is free to reimplement; the code is GPL.

### 4.4 Other galvo-related projects (2025–2026)

| Project | Licence | What it offers OpenLaser |
|---|---|---|
| [kausalinverse/corfile2meerk40t](https://github.com/kausalinverse/corfile2meerk40t) (2026-08-26) | MIT | Replaces EZCAD's `corfile2.exe` on Linux: mark a grid, scan it on a flatbed at 1200 dpi, fit, then write a `.cor`. `SPEC.md` documents both `.cor` layouts byte-exactly (§4.6). Directly applicable to the 1.1834 problem. |
| [Meliadolin/usblm-v1-meerk40t](https://github.com/Meliadolin/usblm-v1-meerk40t) | MIT | Supports the older BJJCZ **USBLM-V1** board (VID 9588 / PID 9999, loader PID 9990, firmware upload). Includes `docs/REVERSE_ENGINEERING.md`. The repo has `data/ezcad_chunk_*.bin`, which *appear* to be firmware chunks captured from EZCAD (UNVERIFIED). Redistributing vendor firmware is a copyright risk, so OpenLaser should not ship such blobs. |
| [edsonpavoni/studio-tools → raio](https://github.com/edsonpavoni/studio-tools) | none detected | Open control for a ComMarker B6 MOPA (BJJCZ). Shows that real users want MOPA presets. |
| [earlynerd/LaserEngraver](https://github.com/earlynerd/LaserEngraver) | none | RP2350 firmware that claims to implement the "standard LMCV4 interface (lightburn compatible)". It could become a hardware-free test target (UNVERIFIED how complete). |
| [Lokahi-Shop/FocuZ](https://github.com/Lokahi-Shop/FocuZ) | proprietary (30-day trial) | Commercial "3D-first" BJJCZ marking software. Market signal only. |
| [charlie-x/fibrelasertools](https://github.com/charlie-x/fibrelasertools) + [blog](https://charliex2.wordpress.com/2022/01/24/ezcad-ezd-parsing-the-pens/) | none | EZD pen-table decoding (EzPenEditor). Facts only. |
| [GalvOS](https://github.com/Andre1Becker/GalvOS), [phycrax/qspi4xy2-100-embassy](https://github.com/phycrax/qspi4xy2-100-embassy) | GPL-3.0 | Laser-show and XY2-100 galvo firmware. Out of scope, but proves Rust/embassy can drive XY2-100 directly if a custom controller is ever built. |
| LightBurn (commercial) | proprietary | Supports **EZCAD2 (LMC, USBLMCV2/V4), EZCAD3 (DLC) and BSL** galvo families. On Windows it installs the EZCAD2/3 driver via `dpinst-x64.exe` and falls back to Zadig/WinUSB. On macOS it says "no driver needed" ([LightBurn galvo driver guide](https://docs.lightburnsoftware.com/latest/Guides/GalvoDriverInstallation/)). EZCAD3/DLC and BSL boards are separate protocols with **no open reference found**. |

### 4.5 Consolidated LMC protocol facts

Sources: MIT code.

| Fact | Value | Source |
|---|---|---|
| USB IDs | VID `0x9588`, PID `0x9899` | galvoplotter `galvo/usb_connection.py:13-14`; balormk `usb_connection.py:13-14` |
| Endpoints | bulk OUT `0x02`, IN `0x88` (with commented-out alternative `0x82`) | same files, lines 16-23 |
| Single command | 12 bytes = 6 little-endian `u16` (opcode + 5 params). Reply read is 8 bytes. | balormk `controller.py` `_command` (`struct.pack("<6H")`); `usb_connection.py:272-302` |
| List packet | exactly `0xC00` = 3,072 bytes = 256 × 12-byte list commands, padded with `0x8002` (end-of-list). | balormk `controller.py:23-24, 594-617`; `usb_connection.py:274` |
| List opcodes | `0x8001` JumpTo, `0x8005` MarkTo, `0x8006` JumpSpeed, `0x800C` MarkSpeed, `0x800A` MarkFreq, `0x800B` MarkPowerRatio, `0x8007/8` laser on/off delay, `0x800D` jump delay, `0x800F` polygon delay, `0x8011` WritePort, `0x801B` QSwitchPeriod, `0x8021` FiberOpenMO, `0x8026` FiberYLPMPulseWidth (MOPA pulse width), `0x8050` JptSetParam, `0x8051` ReadyMark | balormk `controller.py:26-57` |
| Realtime opcodes | `0x0002/0x0004` Disable/EnableLaser, `0x0005` ExecuteList, `0x0007` GetVersion, `0x0009` GetSerialNo, `0x000A` GetListStatus, `0x000C` GetPositionXY, `0x000D` GotoXY, `0x0010` WriteCorLine, `0x0012` ResetList, `0x0013` RestartList, `0x0015` WriteCorTable, `0x0016-0x001E` mode/timing/standby/PWM, `0x001F` StopExecute, `0x0020` StopList, `0x0021-0x0025` ports/analog, `0x0026-0x002A` axis, `0x0033` Fiber_SetMo, `0x0062` SetFpkParam | balormk `controller.py:59-112`; galvoplotter `galvo/consts.py` |
| Status bits | `BUSY = 0x04`, `READY = 0x20`, `AXIS = 0x40` | balormk `controller.py:206-208`; galvoplotter `controller.py:18-19` |
| Flow control | Before sending each list packet, the host polls until `READY`. `ExecuteList` is issued once more than 2 list packets are queued. | balormk `controller.py:567-588` |
| Abort sequence | `StopExecute`; `Fiber_SetMo(0)`; `ResetList`; send a dummy end-of-list packet (then `ExecuteList` if not executing); clear counters; `Fiber_SetMo(0)` again; port bit 0 off; `WritePort` | balormk `controller.py:821-839`; galvoplotter `controller.py:751+` |

The flow-control row is exactly where the prototype's 2026-10-19 stop-latency bug lived. With a full board buffer, `READY` never comes, so the host sits in `wait_ready()`. MeerK40t exits that loop on `is_shutdown`, and the prototype's fix was to abort the spooler wait. A Rust design should make every wait cancellable, using `tokio::select!` on a cancel token, and should send the abort as **realtime** commands on a path that never queues behind list packets.

### 4.6 Correction (`.cor`) files

| Variant | Layout | Source |
|---|---|---|
| **V1 "LMC1COR_1.0"** (float) | label `0x16` bytes UTF-16, 2 unknown bytes, then 63 doubles (`[43]` = scale), then 65×65 × (f64 dx, f64 dy), then an int32 image index | balormk `controller.py:1071-1133`; corfile2meerk40t `SPEC.md` §3.2 |
| **V2 "JCZ_COR_2_1"** (int) | label `0x16` bytes, 6 unknown bytes, 1 f64 scale, then 65×65 × (i32 dx, i32 dy) little-endian signed, then an int32 image index. **Exactly 33,840 bytes.** | same |
| Upload | `WriteCorTable(1)`, then 4,225 × `WriteCorLine(dx, dy, first?0:1)`. Negative values are encoded as `-v + 0x8000` (sign-magnitude). | balormk `controller.py:1100-1139, 1530-1540` |
| Scale meaning | `scale = 65536 / field_mm` (galvo units per mm), so `lens_size = 65536/scale` | `balorconfig.py:222`; corfile2meerk40t SPEC §3.2 |

Other correction prior art:
- balor's RBF calibration (GPL, ideas only).
- LightBurn's own galvo lens correction (proprietary; format UNVERIFIED).
- SCANLAB `.ct5` for RTC cards (proprietary; not researched in depth, UNVERIFIED).

**Action for OpenLaser.** Ship:
1. a `.cor` reader and writer,
2. a "use the EZCAD `.cor` from the original PC" import path,
3. a scanner-based calibration wizard modelled on corfile2meerk40t.

Keep the explicit `scale_correction` only as a final trim.

---

## 5. GRBL-family hosts and senders: what to learn

- **LaserGRBL** (GPL-3.0+). It is laser-specific: raster import with line-to-line/diagonal modes, 1-bit dithering, image vectorization, colour schemes tuned for laser safety glasses, GRBL alarm and error decoding with tooltips, overrides UI, and ESP8266 WebSocket ([README](https://github.com/arkypita/LaserGRBL)). It is a UX benchmark for raster engraving on diode lasers. Don't copy its code.
- **LaserWeb4** (AGPL-3.0). It splits the job into a front end and a comm-server process (`lw.comm-server`), a pattern OpenLaser's "local daemon + UI" may echo. It is legacy now (last binaries 2021).
- **cncjs** (MIT). This is a mature Node serial-port multiplexer with pendant/widget plugins and support for six firmware families. Its controller state machines for Grbl, Marlin and Smoothie can be studied and ported. It is a good permissive reference for "many clients, one serial port" (phone pendant + desktop).
- **ioSender** (BSD-3-Clause, by grblHAL's author). It is the most authoritative *permissive* reference for grblHAL extensions such as extended status, tool tables and probing.
- **UGS** (GPL-3.0+). It has the broadest firmware matrix, with separate `GrblHalController`, `FluidNCController`, `G2CoreController` and `SmoothieController` classes. It is a good map of the *differences* between firmwares (behaviour only).
- **bCNC** (GPL-2.0-only). Autoleveling, a probe UI and a CAM toolbox. Not laser-centric.
- **OpenBuilds CONTROL** (GPL/AGPL ambiguity). Its firmware flashing and troubleshooting wizards are a UX idea. It has been dormant since 2025-06.
- **K40 Whisperer** (GPL-3.0+). It shows the colour convention (red = cut, blue = engrave, other = raster), inside-first cut ordering, and halftone for boards without power control. It is the reference UX for the K40/Lihuiyu niche; for code, MeerK40t's MIT `lihuiyu/` module is the better source.
- **VisiCut / LibLaserCut** (LGPL-3.0+). Its *material/profile database* concept, Inkscape extension integration and driver breadth (Epilog, Lasersaur, LAOS, Ruida, …) are worth noting. If OpenLaser ever adds Epilog support, LibLaserCut's `EpilogCutter.java` is the only open reference, and it is LGPL, so behaviour only.
- **gSender, Candle**: GPL. These are polished GRBL CNC senders. UX only.

### Protocol facts to implement

These come from primary docs, which may be read freely; only firmware *code* is GPL.

- **Grbl 1.1** ([Interface](https://github.com/gnea/grbl/wiki/Grbl-v1.1-Interface), [Commands](https://github.com/gnea/grbl/wiki/Grbl-v1.1-Commands), [Laser Mode](https://github.com/gnea/grbl/wiki/Grbl-v1.1-Laser-Mode)):
  - 128-byte RX buffer. Streaming is either send-response or character counting.
  - Realtime bytes: `0x18` reset, `?`, `!`, `~`, `0x84` door, `0x85` jog cancel, `0x90–0x97` feed/rapid overrides, `0x99–0x9D` spindle overrides, `0x9E` spindle-stop toggle, `0xA0/0xA1` coolant.
  - Status reports look like `<State|MPos|…|FS|Ov|…>`. Poll at no more than 5 Hz.
  - `$J=` jogging. `$32=1` laser mode with `M4` dynamic power.
- **grblHAL** ([core README](https://github.com/grblHAL/core)): a grbl 1.1f-compatible superset with many extensions that "may cause issues with some senders". It defaults to normally-closed inputs, so a fresh board may boot in alarm (`$14=73` workaround). The driver-dependent RX buffer is larger.
- **FluidNC** (GPL-3.0+): ESP32 firmware configured by YAML, with WiFi/telnet/WebSocket transports. Laser runs as a spindle type. It is the most common networked GRBL-dialect target for DIY CO2/diode builds.

---

## 6. Ruida references (CO2 DSP controllers; not in the prototype's scope yet)

| Reference | Type | Licence | Notes |
|---|---|---|---|
| [EduTech Wiki "Ruida"](https://edutechwiki.unige.ch/en/Ruida) (rev. 2021-05-22) | Docs | CC BY-NC-SA 3.0 | UDP 50200 (from 40200) for job data in about 1,000-byte chunks with ACK `0xCC`/NAK `0xCF`. UDP 50207 for the unswizzled `0xA5` "interface" keys (jog, start). USB is an FT245R at 19,200 8N1. 16-bit BE checksum computed before swizzling. Per-model magic: `0x88` (644XG, 644XS, 320, 633X, 654XG), `0x11` (634XG, altered algorithm), `0x38` (RDL9635). 7-bit data bytes (int7/int14/int35). Coordinates in µm. Key opcodes listed. |
| [Stefan Schuermans RDCAM pages](https://stefan.schuermans.info/rdcam/) | Docs | no licence ("educational purposes") | Scrambling algorithm, messages, `.rd` decoding, PC connection. |
| [jnweiger/ruida-laser](https://github.com/jnweiger/ruida-laser) | Docs + tools | GPL-2.0 | `doc/protocol.md`, pcaps, a UDP proxy and an Inkscape extension. |
| [kkaempf/ruida](https://github.com/kkaempf/ruida) | Ruby `.rd` decoder | Ruby licence (permissive inclusion clause 4) | `.rd` file decoding. |
| MeerK40t `meerk40t/ruida/` | Driver + `.rd` loader + **Ruida emulator** | MIT | `rdjob.py` (2,244 lines) encodes jobs. The magic default is `0x88` (`ruida/device.py:272-280`). UDP, TCP, USB and serial transports. The emulator lets LightBurn/RDWorks drive non-Ruida machines, and is a great test oracle. **Portable.** |
| [StevenIsaacs/ruida-pa](https://github.com/StevenIsaacs/ruida-pa) | Python protocol analyzer + TUI + RPC | MIT | Rayforge's Ruida path depends on it (`ruida-pa==0.22.0`). It captures jobs to `.rd`, `.rds` and `.cglu`. **Portable.** |
| crates.io [`ruida`](https://crates.io/crates/ruida) 0.1.1 | Rust | MIT | **A placeholder.** The source has 92 lines (a `greet()` function) and the repository URL returns 404. It squats the name and offers nothing to reuse. |

---

## 7. Rayforge (brief; covered in depth by another agent)

- **Stack.** Python ≥ 3.10 with GTK4/libadwaita, MIT, by Samuel Abels (knipknap). Distributed via Flathub, Snap, a PPA, PyPI and a macOS app. 1.12.0 was released 2026-10-02 and 1.13.0-beta4 on 2026-10-21.
- **Features to benchmark.** From the [README](https://github.com/barebaric/rayforge):
  - parametric constraint sketcher
  - multi-layer operations and a stock-material system
  - holding tabs, overscan and kerf compensation
  - nine dithering algorithms
  - 3D animated simulation and true 4th-axis support
  - G-code dialect editor and per-layer WCS
  - no-go zones and pre-flight bounds checks
  - material library (60+) with recipe auto-matching, and a material test grid
  - machine hours and maintenance counters
  - import of LightBurn device profiles (`.lbdev`)
  - `.rd` import and camera alignment
- **Verified galvo status.** None (see §1, item 2).
- **Rust crates.** Two are directly reusable (§1, item 4):
  - **raydriver** is a full port of Rayforge's GRBL serial driver. It is async (tokio + `tokio-serial`) with serial and telnet transports, a mock transport, a firmware emulator, dialect templates passed in as data, and the `raydriver run --emulator` CLI.
  - **raygeo** includes `geo/`, `ops/`, `pipeline/`, `image/` (dither, Wu lines, grayscale ramps) and `cnc/`. Its dependencies are `clipper2` (Rust FFI to C++ Clipper2), `earcut`, `rstar`, `nalgebra`, `swash` + `fontdb`, `rayon` and `roxmltree` + `svgtypes`.
  - The `python` feature is the default but optional, so the pure-Rust core can be used as a git dependency.
  - Risks: one maintainer, no crates.io release, and an API shaped for Rayforge.
- **Licence observations:**
  - `requirements.txt` pins **PyMuPDF**, which PyPI lists as dual AGPL-3.0 / Artifex commercial. An MIT app that bundles it takes on AGPL obligations for the binary. OpenLaser should avoid MuPDF-based PDF import if it wants to stay permissive; `pdfium` bindings or pure-Rust parsers are alternatives.
  - Rayforge has `rayforge/license/` with Gumroad and Patreon providers and a `private_addons` path (pyproject). Some add-ons appear to be paid or closed (UNVERIFIED which ones).

---

## 8. File-format interoperability references

| Format | Best open reference | Licence | Notes |
|---|---|---|---|
| **EZCAD2 `.ezd`** | MeerK40t [`meerk40t/extra/ezd.py`](https://github.com/meerk40t/meerk40t/blob/main/meerk40t/extra/ezd.py) | MIT | See the details below this table. |
| **LightBurn `.lbrn` / `.lbrn2`** (XML) | MeerK40t `extra/lbrn.py` (436 lines); [tscircuit/lbrnts](https://github.com/tscircuit/lbrnts) (TS, parse **and write**); [michalsarna/baseCreator `crates/base-core/src/lightburn.rs`](https://github.com/michalsarna/baseCreator) (**Rust**); [jlucaso1/lbrn2-to-svg](https://github.com/jlucaso1/lbrn2-to-svg) | MIT; MIT; MIT OR Apache-2.0; MIT | `.lbrn2` shapes use compact `VertList`/`PrimList` strings that MeerK40t parses with regexes. baseCreator is 10 days old, single-author and likely AI-assisted, so useful but needs review. [ranaur/lightburn-cli](https://github.com/ranaur/lightburn-cli) is GPL-3.0; avoid it. |
| LightBurn device `.lbdev` | Rayforge `rayforge/machine/device/lightburn_importer.py` | MIT | Maps device types to drivers. Useful for migrating existing LightBurn users' machine settings. |
| LightBurn REST API | [LightBurnSoftware/LightBurn-REST](https://github.com/LightBurnSoftware/LightBurn-REST) | public spec; implementation proprietary | Spec for automation *of* LightBurn. Reference only. |
| xTool `.xcs` (JSON) | MeerK40t `extra/xcs_reader.py` | MIT | |
| RDWorks `.rd` | MeerK40t `ruida/loader.py` + `rdjob.py`; kkaempf/ruida; Rayforge (import) | MIT / Ruby / MIT | `jnweiger/inkscape-import-rd` is GPL-2.0. |

EZCAD2 `.ezd` details (MeerK40t `ezd.py`):
- Header magic is `EZCADUNI` (UTF-16, 16 bytes) followed by a 7-offset seek table: preview, pens, fonts, vectors, and so on.
- The vector section is Huffman-compressed.
- Object types include Curve 1, Rect 3, Circle 4, Ellipse 5, Polygon 6, Text `0x800`, Image `0x40`, Group `0x10`, Hatch `0x20` (V1 and V2), Combine `0x30`, Spiral `0x60`, and Timer/Input/Output/Encoder/ExtendAxis control objects.
- There are 256 pens. Mapped fields are colour, passes, speed, power, frequency, start/end/polygon TC, jump speed and delays, time and pulses per point, laser-off TC, wobble, and end-point options.
- **MOPA pulse width is not among the mapped pen fields** (UNVERIFIED whether it sits in the unmapped args). charliex2's blog decodes the pens independently.

---

## 9. Rust ecosystem: apps and crates worth studying

### 9.1 Rust laser/CNC applications

| Project | UI | Licence | Maturity (2026-10-22) | Take-away |
|---|---|---|---|---|
| [michalsarna/LightCreator](https://github.com/michalsarna/LightCreator) (+ baseCreator) | egui/eframe 0.37 | **MIT OR Apache-2.0** | Created 2026-10-09, about 400 commits in 2 weeks, 1★. Very likely AI-generated at speed. | The closest "LightBurn in Rust" attempt. GRBL/Marlin streaming over serial or TCP; Ruida/Trocen *export only* (HPGL/DXF); `.lbrn2` import; camera overlay; web API; 30 languages. **No galvo.** Portable, but quality is unknown. |
| [thawkins/gcodekit5](https://github.com/thawkins/gcodekit5) / [feveal/gcodekit5-design](https://github.com/feveal/gcodekit5-design) | GTK4 (gtk4-rs) | Apache-2.0 (LICENSE file; README badges also claim MIT) | about 370 / 120 commits | A 7-crate workspace split (core, camtools, designer, communication with five firmware types, visualizer, devicedb, settings) and bounds checking in the designer. Its repo history (gcodekit 1→5 in about 2 months) is a warning about churn. |
| [sameer/svg2gcode](https://github.com/sameer/svg2gcode) | CLI + Yew/WASM web | MIT | 445★, mature | SVG flattening with tolerance, DPI/unit handling, tool on/off sequences, settings JSON, arc output. A good **test oracle** for OpenLaser's GRBL export. |
| [aalarchiv/ivaCAM](https://github.com/aalarchiv/ivacam) | Tauri + web frontend | GPL-3.0 | about 1,100 commits | Mill/laser/plasma/drag-knife CAM with a `.cps` (Fusion post) runtime. Ideas only. |
| [larsnygard/SnartCut](https://github.com/larsnygard/SnartCut) | iced | GPL-3.0 | 21 commits | LightBurn-inspired, early. Ideas only. |
| [clayfrocker97/OpenLaser](https://github.com/clayfrocker97/OpenLaser) | Rust server + web/touch UI | GPL-3.0 | 0.1.0-alpha.4.1 | MCC100 (Au3tech NexCut X1) fiber/CO2 control. **Same name.** Its loopback controller simulator is a good pattern. |
| [yayahiwo/rustcnc](https://github.com/yayahiwo/rustcnc) | web UI | Apache-2.0 | 6 commits | GRBL/grblHAL sender for a Raspberry Pi. |
| [pedro2d10/brulelumiere](https://github.com/pedro2d10/brulelumiere) | – | MIT | 3 commits | Negligible. |
| [GraphiteEditor/Graphite](https://github.com/GraphiteEditor/Graphite) | wgpu/web | Apache-2.0 | 27.7 k★ | Not laser software, but the best Rust reference for a node-based 2D vector editor and document model. |
| [JeroenGar/sparrow](https://github.com/JeroenGar/sparrow) + [jagua-rs](https://github.com/JeroenGar/jagua-rs) | lib | MIT + MPL-2.0 | 415★ / 214★ | State-of-the-art 2D irregular nesting in Rust, a LightBurn-class feature. |
| [abey79/vsvg](https://github.com/abey79/vsvg) (Rust) / [vpype](https://github.com/abey79/vpype) (Python) | lib/viewer | MIT | 155★ / 978★ | Pen-plotter path optimisation (linemerge, linesort, reloop) for the A4 plotter target. |

### 9.2 Crates.io findings

- **None exist for:** `ezcad`, `bjjcz`, `lmc`, `lbrn` or `galvo`-control. The `ruida` crate is a placeholder (§6).
- **G-code:**
  - [`gcode`](https://crates.io/crates/gcode) 0.7.0: MIT OR Apache-2.0, no_std parser, 29.5 k downloads.
  - [`cnccoder`](https://crates.io/crates/cnccoder): MIT OR Apache-2.0, a G-code generator.
  - `grbli`: non-standard licence, 2022, dead.
  - `rscnc`: MIT, 2026, DIN 66025 decoder plus look-ahead.
- **Infrastructure licensing notes for the plan:**
  - **USB:** use [`nusb`](https://crates.io/crates/nusb) 0.2.8 (Apache-2.0 OR MIT, pure Rust, 539★). It needs no libusb, so it removes the Homebrew/bundled-libusb pain the prototype had on macOS. On macOS it uses IOKit with no permissions needed; on Linux, usbfs plus a udev rule; on Windows the device must still be bound to **WinUSB** (Zadig/libwdi or a WCID descriptor) ([docs.rs/nusb](https://docs.rs/nusb/latest/nusb/)). `rusb` (MIT) links **libusb (LGPL-2.1+)**.
  - **Serial:** `serialport` is MPL-2.0 (fine as a dependency); `tokio-serial` is MIT.
  - **Geometry and text:** `usvg`/`resvg`, `kurbo`, `lyon`, `i_overlay`, `geo`, `ttf-parser` and `image` are all MIT/Apache. `fontdb` and `rustybuzz` are MIT. `clipper2` is MIT/Apache FFI to the C++ Clipper2. Tracing: `vtracer` (MIT/Apache) instead of potrace (GPL-2.0).
  - **UI:** `egui` (MIT/Apache) and `iced` (MIT) are clean. `gtk4-rs` is MIT but the GTK runtime is LGPL. **`slint` is `GPL-3.0-only OR` a royalty-free/commercial Slint licence**, which is not OSI-permissive and adds attribution terms. Avoid it if OpenLaser stays MIT/Apache.

---

## 10. Plotting, nesting, tracing and web references (short)

- **Kiri:Moto** ([GridSpace/grid-apps](https://github.com/GridSpace/grid-apps)): MIT, browser slicer with a laser mode, 4.7.3 on 2026-08-16. Its slice-3D-into-laser-layers idea is reusable.
- **Deepnest-next** (MIT) and **SVGnest** (MIT): JS nesting, superseded by sparrow for Rust.
- **potrace** ([tatarize/potrace](https://github.com/tatarize/potrace) port): GPL-2.0, avoid. The algorithm is published (Selinger 2003) and could be reimplemented, but `vtracer` is already permissive.
- **openlaser.app** ([theanam/openlaser.app](https://github.com/theanam/openlaser.app)): MIT, browser/Web Serial GRBL control for GRBL 1.1, grblHAL and FluidNC. It has dithering, tracing, Google Fonts text, pre-flight checks, framing loops, and an always-visible Stop that Esc also triggers. A strong UX benchmark, and a **naming conflict**.

---

## 11. Licensing recommendation for OpenLaser

**Recommendation: `MIT OR Apache-2.0` (dual), with a written clean-room/provenance policy.**

1. **Continuity.** The Python prototype is MIT and Luiz owns it, so he can relicense his own code as MIT OR Apache-2.0 freely.
2. **Everything essential is available under permissive terms:**
   - BJJCZ LMC protocol: galvoplotter and MeerK40t `balormk` (MIT)
   - `.cor` files: MeerK40t (MIT), corfile2meerk40t (MIT)
   - GRBL: public docs, raydriver (MIT), cncjs (MIT), ioSender (BSD-3)
   - Ruida: MeerK40t (MIT), ruida-pa (MIT), public docs
   - File formats: MeerK40t (MIT), lbrnts (MIT), baseCreator (MIT/Apache)
   - Geometry: raygeo (MIT) and the MIT/Apache crate ecosystem
   - Future firmware protocol: LBP (MIT)

   GPL would unlock only nice-to-haves: LaserGRBL's raster/vectorizer code, balor's RBF calibration, UGS/bCNC autolevel, K40 Whisperer, LibLaserCut's Epilog driver and potrace. All of these can be reimplemented from published algorithms.
3. **Rust convention and patents.** Dual MIT/Apache is what rustc and most crates use, so code can flow to and from the ecosystem. Apache-2.0 adds an explicit patent grant and retaliation clause, which suits hardware-adjacent code. MIT keeps GPL-2.0 compatibility.
4. **Distribution.** A permissive licence keeps future options open:
   - signed and notarised macOS bundles, and Mac App Store if ever wanted (GPL and App Store terms are widely considered incompatible)
   - vendor partnerships, such as a laser maker bundling OpenLaser
   - upstreaming fixes to MeerK40t and Rayforge, which are both MIT and could not accept GPL code
5. **The trade-off accepted.** Someone may ship a closed fork. If preventing that matters more than the points above, choose **GPL-3.0-or-later**. That would allow porting balor, LaserGRBL, UGS, K40W and LibLaserCut code, and MIT inputs (MeerK40t, galvoplotter, raygeo) remain usable inside GPL. The decision is one-way: going back later needs every contributor's consent.

**Policy items to adopt with the licence:**
- `THIRD_PARTY_NOTICES.md` listing each ported MIT/BSD source with its copyright line, for example "Copyright (c) 2021 meerk40t", "Copyright (c) 2023 MeerK40t" for galvoplotter, and "Copyright (c) 2026 Barebaric" for raygeo/raydriver.
- Per-file `// Ported from <url>@<commit>, MIT` headers.
- A `PROVENANCE.md` rule: GPL, AGPL and LGPL sources may be read for behaviour and facts but never copied, translated or paraphrased. Note in the PR when a GPL project informed a design.
- No vendor firmware or EZCAD blobs in the repo. Captured USB traces should be ones the user records on his own machine.
- Trademarks: say "works with EZCAD2-compatible JCZ LMC boards" or "GRBL-compatible". Follow LBP `POLICY.md` before claiming LightBurn-protocol compatibility, and avoid LightBurn logos.
- Use a DCO sign-off for contributions.
- Run `cargo-deny` in CI to block GPL/AGPL/SSPL crates and flag LGPL/MPL.

---

## 12. Ranked list: the 5 most valuable references

1. **MeerK40t** (MIT; `balormk/`, `extra/ezd.py`, `extra/lbrn.py`, `ruida/` with its emulator, `grbl/` with its emulator, `camera/`). It is the only open, maintained, multi-controller laser application with a JCZ galvo driver, `.cor` support, EZCAD and LightBurn importers, and Ruida and GRBL emulators. It is all portable.
2. **galvoplotter** (MIT). This is the exact library the prototype runs on the user's JPT M7. A packet-for-packet Rust port, checked against the prototype's mock-connection logs, is the safest first galvo milestone. Fix the blank-`.cor`-on-connect behaviour during the port.
3. **Rayforge's Rust crates `raygeo` + `raydriver`** (MIT), with Rayforge itself as the feature benchmark. This is ready Rust for geometry, dithering, nesting and an emulator-tested GRBL driver. Depend on them through a pinned git rev or a fork, or port selectively.
4. **GRBL 1.1 / grblHAL / FluidNC interface docs**, with **cncjs (MIT)** and **ioSender (BSD-3)** as permissive sender implementations. These cover the CO2 600×400, diode 400×400 and plotter targets.
5. **The `.cor` correction toolchain: corfile2meerk40t (MIT) and MeerK40t's `.cor` reader.** It gives byte-exact formats, a scanner-based calibration workflow, and the likely explanation for the 1.1834 mark-size factor.

Honourable mentions: LightBurn Protocol (LBP, MIT, future-proofing); svg2gcode (MIT, Rust export oracle); sparrow (MIT, Rust nesting); LightCreator/baseCreator (MIT/Apache, Rust `.lbrn2` reader, to be reviewed carefully); MeerK40t `ruida/` with ruida-pa for a later Ruida driver.

---

## 13. Unverified items and open questions

- **UNVERIFIED:** whether the user's machine shipped with an EZCAD `.cor` file whose `scale` ≈ 65536/236.68 ≈ 276.9 units/mm. That would confirm the 1.1834 hypothesis. It needs the file from the EZCAD PC; no hardware is required to read it.
- **UNVERIFIED:** whether MeerK40t `balormk` is entirely free of balor-derived code. The project asserts it is; I did not audit line by line.
- **UNVERIFIED:** how LightBurn talks to JCZ boards on Windows through JCZ's own driver (`dpinst`) without Zadig. If that is replicable, OpenLaser could coexist with EZCAD on Windows, which the WinUSB route cannot.
- **UNVERIFIED:** completeness of earlynerd/LaserEngraver's "LMCV4 interface" firmware as a hardware-free test target.
- **UNVERIFIED:** EZCAD3/DLC and BSL galvo protocols. No open reference was found, and both are separate from LMC.
- **UNVERIFIED:** MeerK40t's "0.9.9000 (Jan 17, 2026)" release mentioned in its README; no GitHub release exists.
- **UNVERIFIED:** OpenBuilds CONTROL's true licence (the LICENSE file says GPL-3.0, `package.json` says AGPL-3.0).
- **UNVERIFIED:** SCANLAB `.ct5` and LightBurn galvo-correction file formats.

---

## Sources

Primary sources consulted, in addition to the inline links:

- GitHub repos (metadata via API): [meerk40t/meerk40t](https://github.com/meerk40t/meerk40t), [meerk40t/galvoplotter](https://github.com/meerk40t/galvoplotter), [meerk40t/lmc_corfile_viewer](https://github.com/meerk40t/lmc_corfile_viewer), [barebaric/rayforge](https://github.com/barebaric/rayforge), [barebaric/raygeo](https://github.com/barebaric/raygeo), [barebaric/raydriver](https://github.com/barebaric/raydriver), [arkypita/LaserGRBL](https://github.com/arkypita/LaserGRBL), [LaserWeb/LaserWeb4](https://github.com/LaserWeb/LaserWeb4), [cncjs/cncjs](https://github.com/cncjs/cncjs), [t-oster/VisiCut](https://github.com/t-oster/VisiCut), [t-oster/LibLaserCut](https://github.com/t-oster/LibLaserCut), [OpenBuilds/OpenBuilds-CONTROL](https://github.com/OpenBuilds/OpenBuilds-CONTROL), [vlachoudis/bCNC](https://github.com/vlachoudis/bCNC), [winder/Universal-G-Code-Sender](https://github.com/winder/Universal-G-Code-Sender), [Denvi/Candle](https://github.com/Denvi/Candle), [Sienci-Labs/gsender](https://github.com/Sienci-Labs/gsender), [terjeio/ioSender](https://github.com/terjeio/ioSender), [gnea/grbl](https://github.com/gnea/grbl), [grblHAL/core](https://github.com/grblHAL/core), [bdring/FluidNC](https://github.com/bdring/FluidNC), [LightBurnSoftware/lbp](https://github.com/LightBurnSoftware/lbp), [LightBurnSoftware/LightBurn-REST](https://github.com/LightBurnSoftware/LightBurn-REST), [jnweiger/ruida-laser](https://github.com/jnweiger/ruida-laser), [kkaempf/ruida](https://github.com/kkaempf/ruida), [StevenIsaacs/ruida-pa](https://github.com/StevenIsaacs/ruida-pa), [kausalinverse/corfile2meerk40t](https://github.com/kausalinverse/corfile2meerk40t), [Meliadolin/usblm-v1-meerk40t](https://github.com/Meliadolin/usblm-v1-meerk40t), [tscircuit/lbrnts](https://github.com/tscircuit/lbrnts), [michalsarna/LightCreator](https://github.com/michalsarna/LightCreator), [michalsarna/baseCreator](https://github.com/michalsarna/baseCreator), [thawkins/gcodekit5](https://github.com/thawkins/gcodekit5), [sameer/svg2gcode](https://github.com/sameer/svg2gcode), [Michael-F-Bryan/gcode-rs](https://github.com/Michael-F-Bryan/gcode-rs), [aalarchiv/ivacam](https://github.com/aalarchiv/ivacam), [larsnygard/SnartCut](https://github.com/larsnygard/SnartCut), [clayfrocker97/OpenLaser](https://github.com/clayfrocker97/OpenLaser), [theanam/openlaser.app](https://github.com/theanam/openlaser.app), [JeroenGar/sparrow](https://github.com/JeroenGar/sparrow), [JeroenGar/jagua-rs](https://github.com/JeroenGar/jagua-rs), [abey79/vsvg](https://github.com/abey79/vsvg), [abey79/vpype](https://github.com/abey79/vpype), [GridSpace/grid-apps](https://github.com/GridSpace/grid-apps), [deepnest-next/deepnest](https://github.com/deepnest-next/deepnest), [GraphiteEditor/Graphite](https://github.com/GraphiteEditor/Graphite), [kevinmehall/nusb](https://github.com/kevinmehall/nusb).
- GitLab: [bryce15/balor](https://gitlab.com/bryce15/balor) (API and LICENSE).
- Docs: [Grbl v1.1 Interface](https://github.com/gnea/grbl/wiki/Grbl-v1.1-Interface), [Grbl v1.1 Commands](https://github.com/gnea/grbl/wiki/Grbl-v1.1-Commands), [EduTech Wiki Ruida](https://edutechwiki.unige.ch/en/Ruida), [Schuermans RDCAM](https://stefan.schuermans.info/rdcam/), [LightBurn galvo driver guide](https://docs.lightburnsoftware.com/latest/Guides/GalvoDriverInstallation/), [K40 Whisperer page](https://www.scorchworks.com/K40whisperer/k40whisperer.html), [docs.rs/nusb](https://docs.rs/nusb/latest/nusb/), [Hackaday on balor](https://hackaday.com/tag/balor/).
- Registries: crates.io API (`ruida`, `gcode`, `svg2gcode`, `nusb`, `rusb`, `serialport`, `slint`, `egui`, `iced`, `clipper2`, `jagua-rs`, …), PyPI JSON (`raygeo`, `raydriver`, `ruida-pa`, `PyMuPDF`, `vtracer`).
- Local prototype files read: `~/AI/OpenLaser/OpenMopa/mopa_luiz/profile.py:56-58`, `mopa_luiz/live.py:44-81, 392-400`, `profiles/openmopa-machine.ini:3`, `LICENSE`.
