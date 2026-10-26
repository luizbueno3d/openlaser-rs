# 02 — LightBurn feature inventory (target spec for OpenLaser)

*Research date: 2026-10-22. Author: research subagent for the OpenLaser (Rust) plan.*
*Scope: what LightBurn does, catalogued as the spec OpenLaser should eventually match. Each feature has an OpenLaser tier and a note on whether the current Python prototype has it.*

---

## How to read this document

**Sources.** Doc paths are relative to `https://docs.lightburnsoftware.com/latest/`. At fetch time (2026-10-22), "latest" served the **LightBurn 2.2** docs. For example, `Reference/CutSettingsEditor/FillMode` means
<https://docs.lightburnsoftware.com/latest/Reference/CutSettingsEditor/FillMode/>. Other sources (blog, store, forum, GitHub) appear as full URLs. **UNVERIFIED** marks anything I could not confirm from a primary source.

**Tiers for OpenLaser:**

| Tier | Meaning |
|---|---|
| **T0** | MVP. Replaces the current Python prototype (`~/AI/OpenLaser/OpenMopa`) for the user's JPT M7 MOPA on a JCZ/LMC galvo and the GRBL export targets. Includes the prototype's proven safety rules. |
| **T1** | Credible LightBurn alternative. What a GRBL or EZCAD2 galvo owner needs so they don't miss LightBurn day to day. |
| **T2** | LightBurn parity: DSP controllers, cameras, EZCAD3/BSL, nesting, rotary galvo, and similar. |
| **T3** | Beyond LightBurn or nice-to-have, including vendor-locked hardware. |

`T0-stretch` marks a T1 item that is cheap and high-value enough to pull into T0.

**Proto column** (the Python prototype, read from `README.md` and `mopa_luiz/*.py`): **Y** = has it, **P** = partial, **N** = missing.

---

## 0. Key findings (TL;DR)

1. **LightBurn versions.** 2.2 was released on **2026-10-19** with True Nest (Pro-only), Center Line Trace, multi-page projects, 3D-model → depth-map import, a REST API (Labs), Camera Print & Cut (Labs), and **native ARM macOS** builds. 2.1 shipped on 2026-05-19 and 2.0 on 2025-07-07 (Qt 6). Sources: <https://lightburnsoftware.com/blogs/news/lightburn-2-2-true-nest-center-line-trace-camera-print-and-cut-and-more>, <https://lightburnsoftware.com/blogs/news/lightburn-2-1-quick-nest-enhanced-camera-support-undo-history-and-more>, <https://lightburnsoftware.com/blogs/news/lightburn-2-0-00-new-shapes-edit-nodes-submenu-dark-mode-and-more>.
2. **No Linux since 2.0.** "LightBurn version 1.7 is the last version compatible with Linux" (`GetStarted/InstallLightBurn`; announcement: <https://forum.lightburnsoftware.com/t/linux-support-to-end-after-v1-7/144605>). A cross-platform Rust OpenLaser that runs on Linux and Raspberry Pi fills a real gap.
3. **Licensing (2026).** Perpetual licenses with a 1-year update window. **Core $99** covers G-code devices. **Pro $199** covers G-code, DSP (Ruida/Trocen/TopWisdom), galvo (EZCAD2, EZCAD3, BSL) and Epilog. The Core→Pro upgrade is $100, updates are $40/yr, and each key has 3 seats. The separate 2022 "Galvo" license ($150) was folded into Pro on 2024-10-01. Sources: <https://lightburnsoftware.com/pages/which-version-of-lightburn-is-right-for-you>, <https://lightburnsoftware.com/blogs/news/new-lightburn-license-types-and-pricing-coming-oct-1-2024>.
4. **Galvo support.** LightBurn drives **EZCAD2 (JCZFiber)** and **EZCAD2 Lite** boards over USB, **EZCAD3 (Labs, since 2.1)** and **BSL/SeaCAD**. The user's board (BJJCZ "USBLMCV4", 0x9588:0x9899) is an EZCAD2-class LMC board. LightBurn publishes no per-revision LMC compatibility list (UNVERIFIED for V4 specifically).
   - MOPA support is first-class: "Q-Pulse Width" per layer, plus "Enable Q-Pulse Width Setting", "MO Enabled ONLY When Running" and "Open MO Delay" in device settings.
   - On macOS, galvos need no driver (`Guides/GalvoDriverInstallation`).
5. **Gaps OpenLaser can exploit:**
   - Print & Cut and Camera Print & Cut are **gantry-only**. There is no camera alignment workflow for galvos.
   - The Move window is **not available for EZCAD2** galvos, and EZCAD2 Z-axis control was removed in 2.1.
   - LightBurn **cannot import EZCAD projects**.
   - The REST API deliberately **cannot start jobs**.
   - There is no documented "marking on the fly" (encoder) support.
   - There is no job queue beyond Ruida on-controller files.
6. **Cut-setting data model.** The REST API's OpenAPI spec (<https://github.com/LightBurnSoftware/LightBurn-REST>, `docs/openapi.yaml`, pushed 2026-10-21) is the best **primary** machine-readable description of LightBurn's cut-setting model:
   - Each project has 30 cut layers, 2 tool layers, 1 virtual-array cut and 30 image cuts.
   - Each layer carries a shared `LaserCutParams`, plus `galvo{}` and `gantry{}` blocks and sub-layers.
   - OpenLaser should mirror this shape to make `.lbrn2` import lossless.
7. **`.lbrn2` format.** It is undocumented XML (Qt-based app). I parsed two real files, including a MOPA galvo project from <https://github.com/benkrasnow/MOPA_Laser_Stainless_Colors>. The structure is simple:
   - `<CutSetting>` per layer, holding `<param Value=…/>` children.
   - `<Shape>` elements with an affine `<XForm>`.
   - Compact `VertList`/`PrimList` encoding for paths, with ID-based geometry sharing.
   - Text carries a vector `BackupPath`.

   Import compatibility is very feasible. See §17.
8. **Prototype coverage.** The Python prototype covers roughly the **T0 core**: galvo direct mark and red-light frame, safety gating, hatch fill and G-code export. Its main T0 gaps against this spec are:
   - SVG Béziers, arcs and transforms
   - a real project file format
   - time estimates
   - axis-calibration UX
   - per-layer galvo timings
   - any image (bitmap) engraving

---

## 1. Version, platform and release baseline

| Item | LightBurn fact | Source |
|---|---|---|
| Current stable | **2.2.00**, announced 2026-10-19. RC-3 through RC-8 ran 2026-09-30 to 2026-10-16. | 2.2 blog (above); <https://forum.lightburnsoftware.com/tag/release-candidate/593> |
| 2.1 | 2026-05-19. Quick Nest (Core+Pro), Undo History window, Tangent Circle, Font Manager, galvo Split Marking, 16-bit depth maps, **EZCad3 support (Labs)**. EZCad2 Z control was removed. | 2.1 blog |
| 2.0 | 2025-07-07. "Move to Qt 6 framework", dark mode, new shapes, Edit Nodes toolbar, AprilTag camera calibration (Labs), Macros window, split design/machine units. **Linux dropped.** | 2.0 blog |
| OS support (2.2) | Windows 10/11 (x64 and, from 2.2, ARM); macOS 12–26. 2.2 is native ARM; earlier builds ran under Rosetta. **No Linux after 1.7.** | `GetStarted/InstallLightBurn`, 2.2 blog |
| "Labs" | In-development features marked "(Labs)": EZCad3, Epilog, xTool rework, REST API, Camera Print & Cut, Multi-File Trace. | `NewFeatures`, 2.0 blog |
| Sister product | MillMage (CNC router) shares the platform and REST API. | REST repo README |

**Implication for OpenLaser.** LightBurn ships every 5–6 months with long RC cycles and ~1-year update windows. Its moat is device breadth plus polish, not algorithms. OpenLaser's edge is galvo depth (the user's MOPA), openness, Linux, and safety.

---

## 2. Licensing and pricing (as of 2026-10-22)

| Item | Value | Source |
|---|---|---|
| LightBurn Core | **$99**, perpetual, 1 year of updates. G-code devices (GRBL, Marlin, Smoothieware, …). | store <https://lightburnsoftware.com/collections/frontpage>; "which version" page |
| LightBurn Pro | **$199**. Core plus DSP (Ruida, Trocen, TopWisdom), galvo (EZCad2, EZCad3, BSL; "fiber, CO2, or UV … connected with a USB cable") and Epilog. | store; <https://lightburnsoftware.com/products/lightburn-galvo> |
| Upgrade Core→Pro | $100 | store |
| Updates renewal | $40/yr, same for Core and Pro. Early renewal adds a 2-month bonus. | `Licensing/LicenseFAQ` |
| Seats | 3 activations per key, cross-OS. Extra seats are sold separately. | `Licensing/LicenseFAQ` |
| Trial | 30-day full trial. Core users get "Trial Pro" for 2 weeks. | <https://lightburnsoftware.com/pages/trial-version-try-before-you-buy>; 2.2 blog |
| Pro-only features | True Nest (2.2) is the first Pro-only *design* feature. | 2.2 blog |
| History | 2022: Galvo license $150 (add-on $90). 2024-10-01: DSP and Galvo licenses merged into Pro, GCode license renamed Core. | <https://lightburnsoftware.com/blogs/news/lightburn-for-galvos-release-date-pricing-and-other-details>; Oct-2024 license blog |
| Price increase "April 2026" | Claimed only by a third-party site. The official store still shows $99/$199. **UNVERIFIED.** | thepricer.org (not trusted) |
| LightBurn Bridge Kit | Store lists it as "(Discontinued)", $120. | store |

**What this means for the user:** driving a JCZ MOPA galvo in LightBurn costs the **$199 Pro** license plus $40/yr for updates.

---

## 3. Supported controllers and device types

### 3.1 Device Types table (28 rows, verbatim from `Reference/DeviceTypes`)

| # | Device type | Connection | License | OpenLaser tier |
|---|---|---|---|---|
| 1 | No Machine | none | Core/Pro | T0 (offline/placeholder profile) |
| 2 | Custom GCode — flavors: GRBL, GRBL M3, grblHAL, FluidNC, GRBL-STM, Smoothie, Marlin, LinuxCNC, UCCNC, Duet/RepRap, Duet/RepRap HTTP, Creality Falcon HTTP, WeCreat, WeCreat HTTP, Masso, Generic | USB, Network | Core/Pro | T2 (templated dialects); grblHAL/FluidNC as GRBL variants T1 |
| 3 | ACCELaser (HD1 Pro/Ultra; hybrid gantry+galvo, LiDAR) | USB, Network | Pro | T3 |
| 4 | BSLFiber (SeaCAD galvo) | USB | Pro | T2 |
| 5 | CanCam | USB | Core/Pro | T3 |
| 6 | Cubiio 2 | Network | Core/Pro | T3 |
| 7–10 | Emblaser 1 A3 / 1 A4 / 2 & Core / Pro | USB (+Network for 2/Pro) | Core/Pro | T3 |
| 11 | Epilog (Labs) | Network | Pro | T3 |
| 12 | **EZCad 2 (JCZFiber)** | **USB** | Pro | **T0** (the user's board) |
| 13 | EZCad 3 (Labs) | USB | Pro | T2 |
| 14 | Fabkit | USB | Core/Pro | T3 |
| 15 | Gerbil-STM | USB | Core/Pro | T3 |
| 16 | **GRBL** | USB, Network | Core/Pro | **T0** export / **T1** live control |
| 17 | GRBL-LPC | USB, Network | Core/Pro | T2 |
| 18 | GRBL-M3 (1.1e or earlier) | USB | Core/Pro | T2 |
| 19 | iLaser | USB | Core/Pro | T3 |
| 20 | LinuxCNC | USB | Core/Pro | T3 |
| 21 | Marlin | USB | Core/Pro | T2 |
| 22 | Ruida | USB, Network, LightBurn Bridge | Pro | T2 |
| 23 | Smoothieware | USB | Core/Pro | T2 |
| 24 | SnapMaker (Marlin) | USB | Core/Pro | T3 |
| 25 | TopWisdom | USB | Pro | T2/T3 |
| 26 | Trocen-AWC | USB, Network | Pro | T2/T3 |
| 27 | xTool (Labs; D1/S1/F1/F2/P2…) | USB, Network | Core/Pro | T3 |
| 28 | ZhiYuan | Network | Pro | T3 |

Bambu Lab R1 gets G-code export only in 2.2, via a vendor device config; direct connection is planned for 2.3 (2.2 blog). Controller families as LightBurn groups them (`GetStarted/LaserID`, `Explainers/LaserTypes`):

- **GCode:** GRBL, Smoothieware, Marlin, FluidNC, grblHAL, xTool
- **DSP:** Ruida, Trocen, TopWisdom
- **Galvo:** EZCAD2, EZCAD2 Lite, EZCAD3, BSL

### 3.2 Galvo controllers and laser sources (detail)

| Topic | LightBurn fact | Source | Proto | Tier |
|---|---|---|---|---|
| EZCAD2 / JCZFiber (BJJCZ LMC boards) | Supported over USB. The device shows "LMC" in its Windows name once the driver is installed; the Zadig fallback mentions "USBLMCV2". There is **no published list of LMC revisions** (V2/V4/clones), so the user's USBLMCV4 is presumed covered by "EZCad 2 (JCZFiber)" (**UNVERIFIED** for V4). | `GetStarted/AddGalvo`, `Guides/GalvoDriverInstallation` | Y (galvoplotter, 0x9588:0x9899) | T0 |
| EZCAD2 Lite | Listed as supported. | `GetStarted/LaserID` | N | T1 (likely same protocol, UNVERIFIED) |
| EZCAD3 (DLC boards; "DLC" shows in the device name) | Labs since 2.1: standard marking, rotary, split marking, Z jog, external axes tab, "Additional EZCad3 Settings". Imports `markcfg0`, `Motors.ini` and a COR file. | `Reference/DeviceTypes/EZCad3`, 2.1 blog | N | T2 |
| BSL / SeaCAD | Supported over USB with a Cypress driver. Imports `BslCAD.cfg`/`LmcPar.cfg`. 2.2 added door protect and split marking. | `GetStarted/AddGalvo`, RC-3 notes | N | T2 |
| Laser Type | CO2, Fiber, UV, SPI | `Reference/DeviceSettings/GalvoPorts` | P (fiber only) | T0 fiber/MOPA; T2 others |
| Fiber Type (source vendor) | Supported: Raycus, JPT, IPG (YLP/YLPM), MAX (MAX/JPT), CO2, UV. Not yet confirmed: Quantel M20HF/M20/M20EG/M30EG, Manlight PL, VGEN YPFL, EO, IPG_YLP_E, HFB, Orion. | `Explainers/AboutGalvos` | P (JPT M7 only) | T0 JPT; T1 Raycus/IPG/MAX |
| MOPA | Q-switched pulse width is fixed at about 80–140 ns. MOPA is about 2–500 ns, adjustable independently of frequency. | `Explainers/AboutGalvos` | Y | T0 |
| Connection | "USB is currently the only supported type for Galvo devices." | `GetStarted/AddGalvo` | Y | T0 |

**Note on LightBurn galvo naming.** LightBurn calls the field "Field" (lens working area) and the timings "Laser On TC", "Laser Off TC", "End TC" and "Polygon TC". EZCAD and the prototype's markcfg call these LASERONDELAY, LASEROFFDELAY, (end) MARKDELAY and POLYGONDELAY. OpenLaser should show both names.

---

## 4. Connection methods and OS drivers

| ID | Method | LightBurn detail | Source | Proto | Tier |
|---|---|---|---|---|---|
| CON-01 | USB serial (CDC/CH340/FTDI) to G-code controllers | Serial port dropdown, baud rate, DTR option, Transfer Mode **Buffered or Synchronous**, "Query Regularly for Laser Position". | `Reference/DeviceSettings/BasicSettings`, `Reference/LaserWindow` | N (export only) | T1 |
| CON-02 | GRBL over Ethernet/WiFi TCP | "Ethernet or Wifi/TCP" connection plus host/IP and a "Network Port" setting. Default port not stated (**UNVERIFIED**; 23/telnet is common). Recommends 2.4 GHz WiFi, and Synchronous mode if jobs stutter. | `Guides/GRBLNetworkConnection` | N | T1 |
| CON-03 | USB bulk (libusb) to galvo boards | Windows: LightBurn installer ships EZCAD2/EZCAD3 drivers; Zadig/WinUSB is the legacy fallback. "macOS users do not need to install a driver." Linux (≤1.7): libusb device, udev `MODE="0666"` rule. **EZCAD and LightBurn cannot share the device without a driver swap.** | `Guides/GalvoDriverInstallation`, `Troubleshooting/GalvoTroubleshooting` | Y (macOS tested; Win/Linux documented) | T0 |
| CON-04 | Ruida over Ethernet (UDP) | Static IP on the controller ("IP config+"). Docs warn UDP can be unreliable and recommend the Bridge, especially on macOS. Ports per meerk40t source: send 50200, listen 40200, control 50207 (not LightBurn-documented). | `Guides/RuidaEthernet`; <https://github.com/meerk40t/meerk40t/blob/main/meerk40t/ruida/udp_connection.py> | N | T2 |
| CON-05 | Ruida over USB | Two modes: "Packet/USB" and "Serial/USB" (FTDI driver). | `GetStarted/FindMyLaser` | N | T2 |
| CON-06 | LightBurn Bridge | Raspberry Pi kit, preconfigured for a laser at 10.0.3.3, creates a WiFi AP; **Ruida only**. The kit is now discontinued. | `Guides/LightBurnBridge`; store | N | T3 (OpenLaser could run *on* a Pi instead) |
| CON-07 | HTTP G-code flavors | Duet/RRF HTTP, Creality Falcon HTTP, WeCreat HTTP; OctoPrint is not listed. | `NewFeatures`, `Reference/DeviceTypes` | N | T3 |
| CON-08 | Vendor network protocols | xTool native protocol (USB for D1/S1; network for F1/F2/P2), Epilog network, ACCELaser. | `Reference/DeviceTypes/xTool` | N | T3 |
| CON-09 | Find My Laser (discovery) | Scans USB-connected devices. 2.2 adds "Search Network" for xTool, Creality Falcon A1C and Epilog. Marlin is not detectable. Networked Ruida must be added manually. Warns that only one app can own the port. | `GetStarted/FindMyLaser` | P (`openlaser detect`, read-only USB probe) | T1 |
| CON-10 | Multiple devices / instances | Device list with default, duplicate and per-device cameras and libraries. Multiple LightBurn instances are possible. | `Reference/Devices`, `Guides/MultipleLBInstances` | P (4 built-in profiles, not editable) | T1 |

---

## 5. Device settings (machine profiles)

### 5.1 Gantry / G-code devices

| ID | Setting group | Fields (exact names) | Source | Proto | Tier |
|---|---|---|---|---|---|
| DEV-01 | Dimensions / Units | Working Size W/H, **Origin** (corner), CNC Machine (rear-right/negative coords), Machine Coordinates (G54), **Laser Offset** (red dot vs beam), Enable U Axis, Device Units, Control Units (mm/s vs mm/min), Link Control and Design Units, Z Control (Enable Z Axis, Reverse Z, Relative Z Moves Only, Optimize Z Moves, OMTech Polar) | `Reference/DeviceSettings/DimensionsUnits` | P (hard-coded W/H/origin per profile) | T1 (editable profiles); Z T2 |
| DEV-02 | Basic Settings (G-code) | Auto-Home on Startup, Swap X/Y Output, Fast Whitespace Scan, Enable $J Jogging, Enable DTR, Enable Laser Fire Button, Laser on When Framing, 'Out of Bounds' Warning, Enable GCode Clustering, Return to Finish Position, Air Assist (M7 or M8), **S-value Max**, Baud Rate, Network Port, Transfer Mode, Query Regularly for Position | `Reference/DeviceSettings/BasicSettings` | P (S max 1000, M8/M9 fixed) | T1 |
| DEV-03 | Basic Settings (general) | Tab Pulse Width, Enable Job Checklist, Frame Continuously, Overwrite Files on Device, Save/Load Rotary Settings with Project | same | N | T1 (checklist T0-stretch) |
| DEV-04 | GCode tab | Start/End Scripts, Use G0 Moves for Overscan, Emit S Value With Every G1, Fetch Settings on Connect, Check For Workpiece Offsets on Connect | `Reference/DeviceSettings/GCode` | P (fixed header/footer) | T1 |
| DEV-05 | Custom GCode templates | Flavor, Dwell Units, Comment/Escape chars, Suppress Comments, Verbose, extension (default `.gc`), Tool State Automatic, Variable Laser Power, Enable Q-Pulse Options, Supports G53. Templates: Home, Focus Z, User Start/End, Tool On/2nd Tool On/Off, Fire On/Off, Pierce, Dwell, Air On/Off, Layer Start, Cut Move, Raster Move, Rapid Move, Safe Rapid, Abort, Pause, Resume, Job End, Get Position, Get Status, Metric/Imperial modal, WCS, custom error/alarm codes. Variables: `{x} {y} {z} {a/u/rot} {s|speed} {p|power} {dwell} {tool}`. Syntax: `\|REL\|`/`\|ABS\|`/`\|RESTORE\|`, `[X]{x}` (omit if unchanged), `*var` (skip line if unchanged), `\x18` escapes | `Reference/DeviceSettings/CustomGCode` | P (profile has tool_on/off strings for the plotter) | T2 (T1 for a minimal dialect table) |
| DEV-06 | Additional Settings (simulation) | Maximum Speed X/Y, Cut Acceleration X/Y, Engrave Acceleration X/Y, Corner Accuracy, Minimum Corner Speed, Cut Speed Scale, Global Speed Scale, **Read From Controller** | `Reference/DeviceSettings/AdditionalSettings` | N | T1 |
| DEV-07 | Scanning Offset Adjust (bidirectional ghosting) | Speed→line-shift table, Initial Offset, per-laser, `.lbso` import/export, Ruida read/write | `Guides/ScanningOffsetAdjustment` | N | T2 |
| DEV-08 | DSP basic | Start/End Delay, Laser 1/2 names, Laser Is RF Tube, Laser 2 controls/offset, Increment Filename on Send, **Disable Start Button**, Check Bounds When Framing, Network Timeout, IP | `Reference/DeviceSettings/BasicSettings` | N | T2 |
| DEV-09 | Machine Settings (firmware params) | Ruida, TopWisdom and GRBL. Read/Write, Save to File/Load from File (`.lbset`), Load From Backup (automatic backup on each Write), Calibrate Axis (steps/mm) | `Reference/MachineSettings` | N | T1 (GRBL `$$` editor with backup); T2 Ruida |
| DEV-10 | Device import/export | `.lbzip` User Bundle (since 2.0), legacy `.lbdev` (JSON; Rayforge imports it: <https://github.com/barebaric/rayforge/blob/main/rayforge/machine/device/lightburn_importer.py>), `.lbvendor` vendor bundles | `Reference/Devices`, `Reference/UserBundles` | N | T1 (`.lbdev` import is cheap interop) |

### 5.2 Galvo devices

| ID | Setting | Fields (exact names) | Source | Proto | Tier |
|---|---|---|---|---|---|
| GDEV-01 | Config import wizard | EZCAD2: `markcfg7` (sometimes `markcfg0`) from EZCAD's `plug` folder; auto-imports lens correction if present. BSL: `BslCAD.cfg`/`LmcPar.cfg`. EZCAD3: `markcfg0`, `Motors.ini`, COR. Shows a preview before import. One profile per lens, via Duplicate. | `GetStarted/AddGalvo` | P (reads markcfg; `inspect-profile` reports applied vs unused keys) | T0 |
| GDEV-02 | Field | Width/Height (lens field = workspace), **Offset X/Y**, **Angle** (head tilt) | `Reference/DeviceSettings/GalvoBasicSettings` | P (FIELDSIZE; no offset/angle) | T0 (size+offset); T1 angle |
| GDEV-03 | Galvo correction | **Load COR File** / Clear. Per galvo (1/2): "Galvo # is X Axis" (swap), "Reverse Direction (Negate)" (mirror), Scale (+ "…" calibrate), **Bulge**, **Skew**, **Trapezoid** | same | P (single uniform scale correction 1.1834; COR not loaded) | T0: per-axis scale + negate/swap. T1: bulge/skew/trapezoid + COR load |
| GDEV-04 | Red Dot | Offset X/Y, Scale X/Y, Rest Pos X/Y | same | N | T0 (red-dot offset/scale matters for framing accuracy) |
| GDEV-05 | Speeds | Frame Speed (red-dot), Max Speed | same | P (frame light speed hard-coded at 3000 mm/s) | T0 |
| GDEV-06 | Jump defaults | Jump Speed, Min Jump Delay, Max Jump Delay, Jump Distance Limit | same | N (galvoplotter defaults) | T0 |
| GDEV-07 | Delay defaults | Laser On TC, Laser Off TC, End TC, Polygon TC | same | N (parsed, not applied) | T0 |
| GDEV-08 | Other options | Play Sound on Job Finished, **Require Framing Before Start**, **Enable Job Checklist**, Save/Load Rotary Settings with Project | same | P (ARM + confirm gate) | T0 |
| GDEV-09 | IO ports | Ready Light, Busy Light, Red Dot (external switch), Red Dot Always On, Enable Blanking, Start Framing (input), **Start Marking** (foot switch), Require 'Full Pulse' to Start, **Stop Marking** (input), Done Marking (output), **Door Protect** (interlock input), Select 2nd Laser Source, Enable Analog Output (Laser 2), Focus Light. Each has a High/Low trigger and live state indicators. | `Reference/DeviceSettings/GalvoPorts` | N (DOORINPORT/STARTMARKPORT parsed only) | T1 (Door Protect and stop input are **T0-stretch** safety items) |
| GDEV-10 | Laser source | Laser Type (CO2/Fiber/UV/SPI), Fiber Type, **Enable Q-Pulse Width Setting**, **MO Enabled ONLY When Running**, **Open MO Delay**, Enable PWM (min/max freq), Enable Tickle (CO2), First Pulse Killer (CO2), UV Min/Max Pulse, SPI Simmer Current | `Reference/DeviceSettings/GalvoPorts` | P (JPT M7 pulse table, MIN/MAXPWMFREQ enforced) | T0 (fiber/MOPA); T2 CO2/UV/SPI |
| GDEV-11 | Z axis (galvo) | Enable Z Axis, Motor ½, Steps Per Unit Count, Steps Unit Count, Accel time, Min/Max Steps Per Sec, Homing Timeout, Homing Steps Per Sec | same | N | T2 |
| GDEV-12 | External Axes (EZCad3/BSL) | Z, and X/Y table axes | `Reference/DeviceSettings/ExternalAxesSettings` (nav) | N | T2 |
| GDEV-13 | Galvo Lens Calibration wizard | **"9 Point Correction (Easy)"**: field size, test box 85–90% of field, output settings, orientation ("LightBurn Dragon"), horizontal and vertical measurements → per-galvo corrections, optional "Maximize working area". Follow-up: **Calibrate Axis** (requested vs measured, e.g. 100 mm line). | `Reference/CalibrateGalvoLens` | P (Calibrate-Axis equivalent: 50 mm square → 1.1834) | T0: Calibrate Axis per axis. T1: 9-point wizard |

---

## 6. Design / workspace tools

### 6.1 Creation tools

| ID | Feature | LightBurn detail | Source | Proto | Tier |
|---|---|---|---|---|---|
| DES-01 | Draw Lines | Polyline and Bézier pen tool | `Reference/DrawLines` | N | T1 |
| DES-02 | Primary shapes | Rectangle (W, H, **Corner Radius**), Ellipse, Triangle, Pentagon, Polygon (Sides, default 6), Octagon, Star (Points, Bulge 0–1, Ratio 0–1), Dual Radius Star (Ratio2). Parametric until "Convert to Path". | `Reference/PrimaryShapes` | N | T1 (rect/ellipse T0-stretch for test pieces) |
| DES-03 | Text | System TTF/OTF fonts and **SHX single-line fonts** (Set SHX Font Path). Font Manager (favorites, hide). Height, HSpace, VSpace, Align X/Y, Bold, Italic, Upper Case, **Welded**, Distort, Right-to-Left, bend handle, Apply Path to Text, Convert to Path/Text. Saved files embed a path backup when the font is missing. | `Reference/Text` | P (built-in stroke font for test-box labels only) | T1 (single-line font T1, important for galvo/plotter) |
| DES-04 | Variable text | Modes: Normal, **Date/Time**, **Merge/CSV**, **Serial Number**, **Cut Setting**. Window: Current/Start/End/Advance By/**Auto-Advance**/Prev/Next/Reset/Test/**Bake**. Per-object Offset. Max Width, Squeeze, Ignore Empty Vars. | `Reference/VariableText` | N | T1 (serial plates are a core MOPA use; T0-stretch) |
| DES-05 | Variable text syntax | Date: `d dd ddd dddd w W M MM MMM MMMM yy yyyy h hh H HH m mm s ss z zzz AP ap t 'literal'`. Serial: `d`/`h`/`H` digits, `0` prefix pads. CSV: `%0 %1 …`. Cut setting: `C03` layer prefix, `s/S p/P m/M n d/D i/I L z/Z q/Q k/K` (speed, power, min, passes, DPI, interval, laser name, Z, **Q-pulse**, **kHz**). | `Reference/VariableText/VariableTextFormatting` | N | T1 (reuse the syntax for compatibility) |
| DES-06 | Barcodes | 28 types: 2of5 (std/IATA/Industrial), **AprilTag** 16h5/25h9/36h10/36h11, **ArUco** MIP36h11/7x7, Aztec, Chinese Sensible, CodaBar, Code128, Code93, **Data Matrix**, EAN, Grid Matrix, ISBN, PDF417 (+Compact, Micro), Plessey, **QR** (+Micro, Micro Rect.), UPC-A/E, VIN. Options: Error Correction Level, Show Bar Code Text, Extra Data Spacing, Force Square (DM), Process Control Characters (GS1), Variable Text + Offset, WiFi/Contact QR tabs. | `Reference/BarCode` | N | T1 (QR + DataMatrix + Code128); rest T2 |
| DES-07 | Tabs/bridges tool | Manual tabs (click to place, 0.5 mm default) or Automatic: Even Spacing, Limit Max Tabs, Tabs Per Shape, Tab Size, **Tab Cut Power**, Skip Inner Shapes, Clear Tabs | `Reference/AddTabs` | N | T1 (gantry cutting); galvo N/A |
| DES-08 | Measure | Hover readout: perimeter, dimensions | `Reference/Measure` | N | T1 |
| DES-09 | Tangent Circle Generator | Circle tangent to 3 circles (2.1) | `Reference/TangentCircleGenerator` | N | T3 |
| DES-10 | Art Library | `.lbart` libraries (artwork only, no settings), import files/selection, drag-drop, downloadable libraries | `Reference/ArtLibrary` | N | T2 |
| DES-11 | Web/User Tools | Cuttle.xyz, My Laser Tools, custom URL entries (Manage User Tools) | 2.1/2.2 blogs | N | T3 |

### 6.2 Editing

| ID | Feature | LightBurn detail | Source | Proto | Tier |
|---|---|---|---|---|---|
| EDT-01 | Selection | Click, shift, marquee, Select All/Invert, **Select Open Shapes**, Select Open Shapes Set to Fill, Select All in Layer, Select Contained, Select Smaller Than, Select Circles (2.1) | `Reference/UI/EditMenu`, `Reference/Selection` | P (click, shift, marquee, double-click connected) | T0 basic; T1 the rest |
| EDT-02 | Transform | Move, size, rotate, shear handles; Numeric Edits toolbar (X/Y/W/H/rotation, 9-point anchor, lock ratio); Two-Point Rotate/Scale; Flip H/V; Mirror Across Line; Rotate 90° | `Reference/TransformControls`, `Reference/UI/ArrangeMenu` | P (X/Y, scale, rotate, center, mirror at job level) | T0 |
| EDT-03 | Undo/Redo + **Undo History** window (2.1) | | `Reference/UndoRedo`, 2.1 blog | Y (stack) | T0 undo; T2 history UI |
| EDT-04 | Clipboard | Cut/Copy/Paste/**Paste in Place**/Duplicate, system clipboard option | `Reference/Clipboard` | N | T1 |
| EDT-05 | Node editing | Delete (D), Insert (I), Midpoint (M), Corner (C), Smooth (S), Break (B), Delete line, Curve→Line (L), Line→Curve, Align (A), **Trim (T)**, **Extend (E)**; drag nodes together to join | `Reference/EditNodes` | N | T1 |
| EDT-06 | Path hygiene | Close Path (0.5 mm), **Close Selected Paths With Tolerance**, **Auto-Join** (0.05 mm), **Optimize Selected Shapes** (fit arcs/lines), **Delete Duplicates**, Break Apart, **Reverse Shape Direction**, Convert to Path | `Reference/UI/EditMenu` | P (Join Connected Lines with tolerance) | T0 (join/close/dedupe); T1 rest |
| EDT-07 | Set Start Point | Per-shape start node (also used for lead-ins) | `Reference/SetStartPoint` | N | T1 |
| EDT-08 | Trim Shapes / Cut Shapes | Trim to next intersection; split shapes using a closed cutter shape | `Reference/TrimShapes`, `Reference/CutShapes` | N | T2 |
| EDT-09 | Warp (4 pt) / Deform (16 pt) | Applies to vectors, text, images | `Reference/WarpDeform` | N | T2 |
| EDT-10 | Rubber-band outline | Shortest enclosing outline (convex-ish hull) | `Reference/CreateRubberBandOutline` | N | T2 |
| EDT-11 | Generate Path From Cut | Bakes tabs, kerf, perforation into geometry | `Reference/GeneratePathFromCut` | N | T2 |
| EDT-12 | Radius/Fillet tool | Click corner; negative radius = "bite"; rectangle corner handle | `Reference/RadiusFillet` | N | T2 |
| EDT-13 | Resize Slots in Selection; Make Same Width/Height | | nav | N | T3 |
| EDT-14 | Lock shapes, draw order, snapping (grid/object), automatic guidelines, docking | | `Reference/LockShapes`, `Reference/Snapping`, `Reference/AutomaticGuidelines`, `Reference/Dock` | N | T1 (snapping); T2 rest |

### 6.3 Modifying, combining and arranging

| ID | Feature | LightBurn detail | Source | Proto | Tier |
|---|---|---|---|---|---|
| MOD-01 | Booleans | Union (Alt++), Subtract (Alt+-), Intersection (Alt+*). Exactly two closed shapes/groups. **Weld** (Ctrl+W) takes any number. Boolean Assistant (preview). | `Reference/BooleanTools` | N | T1 |
| MOD-02 | Offset Shapes | Distance; Direction Outward/Inward/Both; Corner Style Round/Bevel/Corner; Outer shapes only, Select results, Delete originals, Optimize/Simplify. Inward needs closed shapes. | `Reference/OffsetShapes` | N | T1 |
| MOD-03 | Align / Distribute / Move Together | Edges or centers; distribute by centers or edges | `Reference/Align`, `Reference/Distribute` | N | T1 |
| MOD-04 | Group / Ungroup / Auto-Group | Auto-Group groups contents inside closed outlines | `Reference/Grouping` | N | T1 |
| MOD-05 | Grid Array | Columns/Rows, total size, spacing (center-to-center or edge padding), column/row shift, shift by half, reverse, mirror alternate rows/cols, random orientation (seed), **Auto-Increment Variable Text**, **Create Virtual Array** (linked clones: Edit/Duplicate/Flatten/Remove/Add/Exit) | `Reference/GridArray` | N | T1 (virtual arrays T2) |
| MOD-06 | Circular Array | Copies, center (or last-selected object), Start/End/Step angle, Rotate copies, auto-increment variable text | `Reference/CircularArray` | N | T1 |
| MOD-07 | Copy Along Path | Number of copies or spacing, Rotate, Scale, auto-increment variable text | `Reference/CopyAlongPath` | N | T2 |
| MOD-08 | Apply Path to Text | Text follows arbitrary path | `Reference/ApplyPathToText` | N | T2 |
| MOD-09 | Nesting | **Quick Nest** (2.1, Core): bounding boxes, 90° rotations, rectangular container. **True Nest** (2.2, Pro-only): true shape, Allowed Orientations 1–90°, Optimize time/quality, Stop Nesting. Shared: Padding, Edge Buffer, Lock Inner Objects, Lock Objects for Rotation, **Multipage**. SVGNest is an external option. | `Reference/Nesting/TrueNest` | N | T2 (true-shape nesting is a T3 differentiator if done well) |
| MOD-10 | Pages (multi-page projects, 2.2) | Add/duplicate/delete/rename pages. Shapes are per page; cut settings are shared. Nesting can spill onto new pages. | `Reference/Pages` | N | T2 |
| MOD-11 | Move Selected Objects / Move Laser to Selection / Jog Laser | Snap selection to bed corners/center or to the last head position; move head to selection | `Reference/UI/ArrangeMenu` | P (Center Selection) | T1 |

### 6.4 Image and import-related design tools

| ID | Feature | LightBurn detail | Source | Proto | Tier |
|---|---|---|---|---|---|
| IMT-01 | Trace Image | Cutoff, Threshold (0–255), Trace Transparency, **Center Line Trace** (2.2), **Sketch Trace**, Ignore Less Than, Smoothness (0–1.33), Optimize (default 0.2), Fade Image, Show Points, boundary select, Delete Image After | `Reference/TraceImage` | N | T2 (T1 for basic threshold trace) |
| IMT-02 | Multi-File Trace (Labs) | Batch tracing | `Reference/MultiFileTraceImage` | N | T3 |
| IMT-03 | Apply Mask to Image / Crop Image | Vector mask, editable until flattened | `Reference/ApplyMaskToImage` | N | T2 |
| IMT-04 | Convert to Bitmap | Render Type (Outlines / Fill All / Use Cut Settings), DPI, Brightness (deletes the vector) | `Reference/ConvertToBitmap` | N | T2 |
| IMT-05 | Image Options | Refresh Image, Replace Image, Replace Image to Fit | `Reference/UI/EditMenu` | N | T2 |
| IMT-06 | Save Processed Bitmap | Export the dithered result | `Reference/SaveProcessedBitmap` | N | T2 |
| IMT-07 | Import 3D Model → depth map (2.2) | `.stl .3mf .amf .obj .off .ply`. Preset orientations, size/rotation, **Bit Depth 8/16**, resolution, transparent background, invert, equalize, Depth Cutoff (auto/manual), generate outline. Shown red-tinted; "Edit 3D Model". | `Reference/Import3DModelCreateDepthMap` | P (STL imported as triangle outlines, max 900) | T2 |

---

## 7. Layers and cut settings

### 7.1 Layer model

- **Palette:** 30 numbered layers (00–29) plus two **Tool layers T1/T2**. Tool layers "have no cut parameters and will never be output to the laser". They hold guides, boundaries, masks, text paths and P&C targets (`Reference/UI/ColorPalette`).
- **REST view of a project:** fixed list of 30 cut layers, then 2 tool layers, then 1 internal virtual-array cut, then **30 image cuts**. A vector and an image on the same color are separate cut entries (`LightBurn-REST docs/cut-settings.md`).
- **Cuts/Layers window columns:** #, Layer, **Mode** (Line/Fill/Offset Fill; Image locked; "Multi" with sub-layers), Spd/Pwr, **Output**, **Show**, **Air** (gantry). Fields: Pass Count, Speed, Power Max/Min, Interval, Frequency (galvo), Q-Pulse (MOPA/UV).
- **Layer operations:** order top→bottom; drag reorder; **Sort Cuts Last**; copy/paste settings; context menu (disable others, hide others, select shapes, **flash content**) (`Reference/CutsLayersWindow`).

| ID | Feature | Proto | Tier |
|---|---|---|---|
| LAY-01 | Color layers with output/show toggles, reorder, rename | Y (4 default layers, rename, color, reorder, hide, disable) | T0 |
| LAY-02 | Tool layer (non-output) | Y ("Frame Only" mode: framed but never burned) | T0 |
| LAY-03 | 30-layer palette parity + separate image cut per color | N | T1 (needed for lossless `.lbrn2` import) |
| LAY-04 | Copy/paste layer settings, Sort Cuts Last, flash layer | N | T1 |
| LAY-05 | Default Layer Settings (per-device defaults, "Load default layer settings on new") | P (`layer_settings.json`) | T1 |

### 7.2 Shared/main settings (`Reference/CutSettingsEditor/SharedSettings`)

| ID | Setting | Notes | Proto | Tier |
|---|---|---|---|---|
| CUT-01 | Name, **Output** switch | | Y | T0 |
| CUT-02 | **Speed** | Galvo units mm/s; G-code often mm/min (Control Units). Ruida "Default Speed" switch. | Y | T0 |
| CUT-03 | **Max Power**, **Min Power** | Min Power appears only for DSP, or G-code Grayscale images. Ruida "Default Power" switch. | P (single power) | T0 max; T1 min |
| CUT-04 | **Constant Power Mode (GRBL Only)** (M3 vs default M4) | | P (per machine, not per layer) | T1 |
| CUT-05 | **Air Assist** per layer (M7/M8 chosen in device settings) | | Y (M8/M9) | T0 |
| CUT-06 | Mode: Line / Fill / Offset Fill (Image automatic) | REST `mode` enum: cut, scan, offset, image, tool | P (vector engrave/cut, raster hatch, frame-only) | T0 Line+Fill; T1 Offset Fill |
| CUT-07 | Number of Passes | | Y | T0 |
| CUT-08 | **Z Offset**, **Z Step per Pass** | Gantry Z; galvo Z only on EZCad3/BSL | N | T2 |
| CUT-09 | Priority (layer order), **Power Scale** per shape, **Cut Order Priority** per shape | `Reference/ShapeProperties` | N | T1 |
| CUT-10 | Dual laser per layer (Laser 1/2, separate powers) | DSP only (`Reference/DualLaserControl`); galvo "Use Laser 2"/`alt_source` | N | T3 |

### 7.3 Line mode (`Reference/CutSettingsEditor/LineMode`)

| ID | Setting | Notes | Proto | Tier |
|---|---|---|---|---|
| LIN-01 | **Kerf Offset** (closed shapes; + outward) | | N | T1 |
| LIN-02 | **Perforation Mode** (Cut / Skip lengths) | | N | T2 |
| LIN-03 | Tabs/Bridges (see DES-07) | | N | T1 |
| LIN-04 | Start/End Pause Time, **Cut Through** (+ power) | Advanced tab, **not for galvo** | N | T2 |
| LIN-05 | **Overcut** (closed shapes, final pass) | | N | T2 |
| LIN-06 | **Lead-In / Lead-Out** (Angle, Length, Style Line/Arc, start points) | gantry | N | T2 |
| LIN-07 | Dot Mode (Time, Spacing) | Ruida only. Galvo has its own "dot delays" (`start_delay`/`end_delay` in `params.galvo`). | N | T2 (galvo dot delays T1) |
| LIN-08 | Override PWM Frequency (Ruida RF tubes), Enable PPI (Trocen), U Offset (Ruida) | | N | T3 |

### 7.4 Fill mode (`Reference/CutSettingsEditor/FillMode`)

| ID | Setting | Notes | Proto | Tier |
|---|---|---|---|---|
| FIL-01 | **Line Interval** / Lines Per Inch | | Y (Pitch mm) | T0 |
| FIL-02 | **Scan Angle** | 0 = horizontal bottom→top, 90 = vertical L→R | Y (Hatch angle) | T0 |
| FIL-03 | **Bi-directional Fill** | | P (always serpentine: `raster._hatch_one_pass` flips each line; no unidirectional option) | T0 |
| FIL-04 | **Cross-Hatch** (second pass +90°) | | P (via angle step 90°/pass) | T0 |
| FIL-05 | **Angle Increment** per pass (galvo; also Image) | | Y (Angle step/pass) | T0 |
| FIL-06 | **Overscanning** (% of speed) | **Not available for DSP or galvo** | N | T1 (gantry) |
| FIL-07 | **Fill Grouping**: Fill All Shapes at Once / Fill Groups Together / Fill Shapes Individually (`scanOpt` = merge_all / by_group / individual) | | P (all at once) | T1 |
| FIL-08 | Nested-shape semantics | Same layer: even-odd (holes left blank, overlaps cancel). Different layers: each filled fully. | `Explainers/LayerModes` | Y (even-odd: pairs sorted scan-line intersections across all closed shapes in the job) | T0 |
| FIL-09 | **Ramp Length** / Ramp Outer Edge (stamp ramps) | Advanced, not galvo | N | T2 |
| FIL-10 | **Flood Fill** (fewer travel moves; on Common tab for galvo; ≈ EZCAD "snake hatch") | | N | T1 for galvo |
| FIL-11 | **Dot Width Adjust/Correction** (shorten scan lines ≈ half beam width; Fill and Image) | | N | T1 |
| FIL-12 | Auto-Rotate (start angle advances per job run; reset in framing) | galvo | N | T2 |

### 7.5 Offset Fill mode (`Reference/CutSettingsEditor/OffsetFillMode`)

Concentric contour fill (≈ EZCAD "ring-like / pyramid hatch"). Settings: Line Interval/LPI, Passes, Z Offset/Step, Fill Grouping. Bidirectional, crosshatch, overscan and scan angle are greyed out. Docs warn it is computationally expensive and can hang on complex art (`Explainers/LayerModes`). **Proto: N. Tier: T1** (common for galvo deep engraving; a robust Rust offsetter is a chance to beat LightBurn's hang).

### 7.6 Sub-layers (`Reference/CutSettingsEditor/SubLayers`)

Up to **11 sub-layers per layer** run left→right on the same geometry. Each has its own mode, settings and output switch, with Add/Duplicate/drag-reorder. Layer mode shows "Multi". Galvo adds **Global Passes**, which repeats the whole sub-layer stack: total = global × per-sub-layer passes (REST). Legacy "Fill+Line" (`type="Scan+Cut"`) converts to two sub-layers. The EZCAD equivalents are Mark Contour, Hatch 1/2/3 (`Explainers/EZCADHatchQuickTranslation`). **Proto: N** (one operation per layer). **Tier: T1** (galvo cleanup passes and outline+fill on one color are everyday MOPA workflow; T0-stretch).

### 7.7 Full cut-setting field list (primary: LightBurn REST OpenAPI, 2026-10-21)

Use this as OpenLaser's layer-settings schema checklist (<https://github.com/LightBurnSoftware/LightBurn-REST/blob/main/docs/openapi.yaml>).

- **Cut-level:** `name, enabled, priority, kind(normal|image), mode(cut|scan|offset|image|tool), global_passes (galvo), negative, pass_through, enable_cleanup, sort_within_layer, tabs{enabled, manual, size, use_spacing, count, count_max, limit_max, spacing, skip_inner, cut_power}, sub_layers[]`
- **LaserCutParams (shared):** `speed, min_power, max_power, min_power_2, max_power_2, default_speed, default_power1/2, laser1_enabled, laser2_enabled, alt_source (galvo), num_passes, z_offset, z_per_pass, u_offset, angle_per_pass, kerf, frequency (Hz), override_frequency, q_pulse_width (ns, MOPA), fiber_pulse_width (gantry fiber), bi_directional, cross_hatch, flood_fill, auto_rotate, scan_opt(individual|by_group|merge_all), angle, interval, dot_width, use_dot_correction, ramp_length, ramp_outer, perforate, perf_len, perf_skip, tabs_enabled, overscan, overscan_percent`
- **galvo{}:** `override_timings, laser_on_tc, laser_off_tc, end_tc, polygon_tc (µs), jump_speed, min_jump_delay, max_jump_delay (µs), jump_dist_limit, start_delay, end_delay (dot delays), wobble_enable, wobble_step, wobble_size`
- **gantry{}:** `run_blower, auto_blower, blower_speed_override, blower_speed_percent, cut_through_start/end, through_power(_2), start_delay, end_delay (pauses), lead_in, lead_out, lead_angle, lead_length, lead_style(line|arc), enable_ppi, ppi, dot_mode, dot_time, dot_spacing, force_constant_power, dcc_value, overcut`
- **Image cut params:** `dpi, dither_mode, cells_per_inch, halftone_angle, link_dpi_to_interval` (plus speed/power/source/pulse fields)

The project stores **both** galvo and gantry blocks regardless of the active device. The active device decides which applies (REST `cut-settings.md`). OpenLaser's project format should do the same, so one project can retarget between the user's MOPA and GRBL machines.

---

## 8. Image engraving (`Reference/CutSettingsEditor/ImageMode`, `Reference/ShapeProperties`, `Reference/AdjustImage`)

| ID | Feature | LightBurn detail | Proto | Tier |
|---|---|---|---|---|
| IMG-01 | Import raster | `.png .jpg .bmp .gif .tif .webp` | N | T1 (T0-stretch) |
| IMG-02 | **Threshold** | Two-tone; supports Ramp Length | N | T1 |
| IMG-03 | **Ordered** | Ordered (Bayer) dither | N | T1 |
| IMG-04 | **Atkinson** | Error diffusion | N | T1 |
| IMG-05 | **Dither** | Floyd–Steinberg error diffusion | N | T1 |
| IMG-06 | **Stucki** | Error diffusion | N | T1 |
| IMG-07 | **Jarvis** | Error diffusion (docs call it the best for photos) | N | T1 |
| IMG-08 | **Newsprint** | Decorative spot pattern | N | T2 |
| IMG-09 | **Halftone** | Cells per Inch, Halftone Angle | N | T2 |
| IMG-10 | **Sketch** | Edge-only engraving | N | T2 |
| IMG-11 | **Grayscale** | Power varies Min→Max. "Not compatible with UV lasers." | N | T1 (gantry M4); T1 galvo via power modulation, UNVERIFIED how LightBurn does galvo grayscale |
| IMG-12 | **3D Sliced** (galvo only) | Depth map: one pass per gray level (darker = deeper). Negative Image, Enable cleanup pass, Clean after # of passes, Clean whole image, Angle Increment. >256 passes needs 16-bit. | N | T2 |
| IMG-13 | Line Interval / **DPI** (DPI = 25.4/interval), Scan Angle, Angle Increment, Passes, Bi-directional, **Negative Image**, **Pass-Through** (use pixels 1:1), Overscanning (not DSP/galvo), Dot Width Correction, Fill Grouping, Z Offset | | N | T1 |
| IMG-14 | Image adjustments (per image) | **Gamma**, **Contrast**, **Brightness**, **Enhance** (unsharp): Radius, Amount, **Denoise**. Adjust Image dialog shows side-by-side live preview, Invert Display, presets (built-ins "Basic", "Black Paint on White"; save/import/export). | N | T1 (presets T2) |
| IMG-15 | Interval Test helps pick interval (see CAL-03) | | N | T1 |

---

## 9. Galvo-specific features (consolidated)

| ID | Feature | LightBurn detail | Source | Proto | Tier |
|---|---|---|---|---|---|
| GLV-01 | Frequency (kHz) per layer | Lower = fewer, stronger pulses. Stored in Hz in `.lbrn2` (e.g. `300000`). | `Reference/CutSettingsEditor/GalvoSpecificCutSettings`; sample file | Y | T0 |
| GLV-02 | **Q-Pulse Width** (ns) per layer | MOPA fiber and UV only. Stored as `QPulseWidth` (e.g. `60`). | same | Y (snapped to JPT table) | T0 |
| GLV-03 | Per-layer timings override | Override Default Timings: Pause Before/After (ms), Jump Speed, Min/Max Jump Delay, Jump Distance Limit, Laser On/Off TC, End TC, Polygon TC | same | N | T0 (at least device-level; per-layer T1) |
| GLV-04 | **Wobble** | Step, Size (mm) | same | N | T1 |
| GLV-05 | Angle Increment / Auto-Rotate | | same | Y / N | T0 / T2 |
| GLV-06 | Dot Width Adjust | | same | N | T1 |
| GLV-07 | Global Passes + sub-layers + cleanup | | same; `Reference/CutSettingsEditor/SubLayers` | N | T1 |
| GLV-08 | **Live Framing** window (F1) | Styles **Bounds**, **Hull**, **Contour**; Frame Individually, Image Bounds, Outside Shapes Only, Tool Layers Only. While framing: nudge/scale/rotate by keyboard; Start/Stop/Pause from the window. | `Reference/GalvoFraming` | P (frame once / continuous contour; no hull/bounds choice, no live nudge) | T0 (bounds+contour+continuous); T1 hull, live nudge |
| GLV-09 | Job counter | Count, Reset Count, **Run Continuously**, **Repeat** N, **Stop At** (with foot switch) | `Reference/GalvoFraming` | P (Repeat) | T1 |
| GLV-10 | Require framing before start; job checklist; play sound on finish | | `Reference/DeviceSettings/GalvoBasicSettings` | P (ARM + final confirm) | T0 |
| GLV-11 | Red dot / red light controls | Enable Red Light toggle in Laser window (needs IO port); offsets/scale (GDEV-04) | `Reference/LaserWindow` | P | T0 |
| GLV-12 | **Rotary (galvo)** | Setup: Chuck/Roller, Steps Per Rotation (9,600/12,800/25,600 typical), Test, Roller/Object Diameter, Circumference, **Split Size**, **Overlap**, Rotary Axis X/Y, Min/Max Speed, Accel Time, Return Speed, Reverse, Return to Start. Rotary Marking window: Run whole shapes if possible, Max Shape Size, **Output Center** (+Show), Run in shape order / all shapes per slice, position/jog/set zero, Frame Prev/Next Slice, **Sanity Check**. | `Reference/RotaryMode/RotaryModeGalvo` | N | T2 |
| GLV-13 | **Split Marking** (2.1) | Linear or rotating table: Motor #, Axis Type, orientation, steps per unit, speeds, accel, return, homing; split size, overlap, whole shapes, output center, table positioning, frame slices, sanity check. Table shear correction (2.2). | `Reference/SplitMarking`; 2.2 notes | N | T2 |
| GLV-14 | **Repeat Marking** | External table: Count, Increment (deg or mm), Calculate, Pause After Move, Completed counter | `Reference/RepeatMarking` | N | T2 |
| GLV-15 | **Cylinder Correction** (EZCAD "ProjectMark") | Mirror Distance (mirror→work incl. focus), Object Diameter, X or Y axis, **Convex/Concave**, Show Valid Boundary (T1 rectangle). Warns before clipping outside the valid area. | `Reference/CylinderCorrection` | N | T2 (high value for tumblers/rings on MOPA) |
| GLV-16 | Taper Warp | Object Top (Top/Left/Right), Top/Bottom Diameter, Length | `Reference/TaperWarp` | N | T2 |
| GLV-17 | 3D Sliced Image mode | See IMG-12 | `Guides/3DSlicedImage` | N | T2 |
| GLV-18 | Galvo Move window | **EZCad3/BSL only** (Z readouts, Home Z, Move to Z, saved positions, jog). Not for EZCad2. | `Reference/MoveWindow`, `Reference/DeviceTypes/EZCad3` | N | T2 |
| GLV-19 | Lens/field per profile | "Create one device profile per lens"; Changing a lens guide | `GetStarted/AddGalvo` | P | T1 |
| GLV-20 | EZCAD hatch translation | Mark Contour → Line+Fill sub-layers. Hatch 1/2/3 → sub-layers. Cross-hatch = Cross-Hatch. Uni/bi-directional = Bi-directional off/on. Ring/pyramid = Offset Fill. Snake = Flood Fill. Auto-rotate = Angle Increment. EZCAD bow hatch, line reduction, edge/start/end offset and loop count: **no documented LightBurn equivalent (UNVERIFIED)**. | `Explainers/EZCADHatchQuickTranslation` | N | T1 (support EZCAD vocabulary in UI); extra hatch types T3 |
| GLV-21 | EZCAD project import | **Not supported** ("cannot import EZCAD project, parameter, or file formats"; config files only) | `Troubleshooting/GalvoTroubleshooting` | N | T3 (differentiator: `.ezd` import, UNVERIFIED feasibility) |
| GLV-22 | Marking on the fly (encoder/conveyor MOTF) | **Not documented in LightBurn (UNVERIFIED absent)** | — | N | T3 |
| GLV-23 | Galvo Print & Cut / camera alignment on galvo | **Not supported** (both P&C variants gantry-only) | `Reference/PrintAndCut`, `Reference/CameraPrintAndCut` | N | T3 (differentiator) |
| GLV-24 | Stop behaviour | Not documented. The prototype learned that a full list buffer plus USB write timeouts caused a reset storm; fixed by aborting the spooler wait on stop (0.18 s). | proto history | Y | T0 (must keep) |

---

## 10. Optimization settings (`Reference/OptimizationSettings`)

| ID | Setting | Proto | Tier |
|---|---|---|---|
| OPT-01 | Master switch "Enable Optimization and Output Re-ordering" (off = drawing order) | N | T1 |
| OPT-02 | Order By list (stackable): **Order by Layer**, **by Group**, **by Priority** | P (layer order) | T0 layer; T1 rest |
| OPT-03 | **Cut inner shapes first** | N | T1 (gantry cutting); T0-stretch |
| OPT-04 | Cut in direction order (direction buttons) | N | T2 |
| OPT-05 | **Reduce travel moves** (nearest-neighbour / TSP-like) | N | T0 (galvo jump time matters) |
| OPT-06 | Hide backlash | N | T2 |
| OPT-07 | Reduce direction changes | N | T2 |
| OPT-08 | **Choose best starting point**, Choose corners if possible | N | T1 |
| OPT-09 | **Choose best direction** (allow reverse) | N | T1 |
| OPT-10 | **Remove Overlapping Lines** (distance) | N | T1 |
| OPT-11 | Set as Defaults / Reset; settings saved in `.lbrn2` `<UIPrefs>` as `Optimize_*` keys | N | T1 |

---

## 11. Preview, simulation and time estimate (`Reference/Preview`)

| ID | Feature | Proto | Tier |
|---|---|---|---|
| PRV-01 | Preview window: cut moves black, travel red, bed boundary green | P (canvas draws plan; Plan Job summary) | T0 |
| PRV-02 | **Time slider** + Play (1/10× to 40×) + **Start Here** (resume from a point) | N | T1 (Start Here T2) |
| PRV-03 | Stats: cut distance/time, rapid distance/time, **total estimated time** | P (distance only) | T0 (galvo: mark/jump speeds + delays; gantry: accel model) |
| PRV-04 | Shade According to Power, Invert, Show Legend, Show Traversal, Save Image | N | T1 |
| PRV-05 | Simulation accuracy from Additional Settings (accel, corner, speed scales; Read From Controller) | N | T1 |
| PRV-06 | Out-of-bounds warning / "Ignore out-of-bounds shapes if possible" | Y (proto **refuses** instead; keep that as the default) | T0 |

---

## 12. Machine control and job execution

| ID | Feature | LightBurn detail | Source | Proto | Tier |
|---|---|---|---|---|---|
| CTL-01 | Start / Pause / Resume / Stop | Galvo Start opens Live Framing first unless disabled | `Reference/LaserWindow` | P (Mark, STOP; no pause) | T0 (pause T1) |
| CTL-02 | Frame (gantry) | **Bounding Box Frame**, **Rubber Band Frame**, Frame Continuously, **Laser On When Framing** (Shift+Frame) at Move-window power | `Reference/LaserWindow` | N | T1 |
| CTL-03 | **Start From**: Absolute Coords / Current Position / User Origin + **9-point Job Origin** | Galvos always use Absolute Coords. Finish Position is G-code only. | `Reference/CoordinatesOrigin` | P (absolute; G-code origin conversion) | T1 |
| CTL-04 | **Cut Selected Graphics** + Use Selection Origin | | `Reference/CutSelectedGraphics` | Y (Selected only) | T0 |
| CTL-05 | Home, Go to Origin, Show Last Position | | `Reference/LaserWindow` | N | T1 |
| CTL-06 | Move window | Get Position, Move to Position, **Saved Positions**, jog arrows (Distance/Speed), Z and A jog, **Continuous Jog** ($J), keyboard/numpad jog, Move From Machine Zero, Focus Z, Set/Clear Origin, Set Finish Position, **Fire** (+Power), live **Adjust Speed/Power** overrides (G-code) | `Reference/MoveWindow` | N | T1 |
| CTL-07 | Position Laser tool (click workspace → head moves) | | `Reference/PositionLaser` | N | T1 |
| CTL-08 | Console | G-code only. Raw command entry, `$I $$ $# ? $X`, Show All toggle, alarm/error display. FluidNC [MSG] filtering (2.2). | `Reference/ConsoleWindow` | N | T1 |
| CTL-09 | Macros window | Any number of buttons (label + multi-line G-code), reorder, custom hotkeys (2.2); G-code only | `Reference/MacrosWindow` | N | T1 |
| CTL-10 | **Save machine file** | Save GCode / RD (Ruida) / OUT / UD5 (OUT↔TopWisdom and UD5↔Trocen mapping **UNVERIFIED**) | `Reference/LaserWindow` | Y (G-code) | T0 |
| CTL-11 | Run machine file | Run GCode / RD / OUT / UD5 | same | N | T1 (Run GCode) |
| CTL-12 | **Send** to controller storage (DSP) + **File List** window | Refresh, Start, Upload, Delete, **Calc Time**, Delete All; Increment Filename on Send | `Reference/FileListWindow` | N | T2 |
| CTL-13 | Job queue | None beyond DSP file storage, galvo Run Continuously/Repeat, and UDP/REST loading. **No multi-job queue (UNVERIFIED absent).** | — | N | T3 |
| CTL-14 | Get Controller Info | | `Reference/GetControllerInfo` | P (`/api/board` status) | T1 |
| CTL-15 | Status bar / device status | Disconnected / Ready / Busy (progress + ETA) / Framing | `Reference/LaserWindow` | P | T0 |
| CTL-16 | Safety-relevant options | Job checklist, require framing, out-of-bounds warning, check bounds when framing, Disable Start Button, ignore Start when monitor asleep, "Don't let Start hold keyboard focus", door protect input, Fire button opt-in, play error sound if send fails, **Save Job Log** | `Reference/DeviceSettings/*`, `Reference/SettingsPreferences` | P (ARM token, final confirm, refuse out-of-field, refuse >3000 paths / >20000 points, frame-only never burns, whole-job check before first layer, finite coordinates, M5 before travel) | T0 (proto rules are stricter than LightBurn's; keep them) |

---

## 13. Calibration and quality tools

| ID | Feature | LightBurn detail | Source | Proto | Tier |
|---|---|---|---|---|---|
| CAL-01 | **Material Test** generator | 2-axis grid (default 10×10). Param: Power, Speed, Interval, Passes, **Frequency**, **Q-Pulse width** (if supported). Count, Min/Max, cell W/H, X/Y center. Labels (Enable Text, Edit Text Setting). Border. Base settings via Edit Material Setting. Riskiest cells first. Presets in `material_test_presets.lbmt` (4 built-in for diode/CO2). Preview/Frame/Start/Save/Send. | `Reference/MaterialTest` | P ("first burn" dot/box test) | **T0-stretch** (MOPA color work needs frequency×pulse grids) / T1 |
| CAL-02 | **Material Library** | `.clb` files. Material → Thickness (or "No Thickness" + Title) → Description. **Assign** (copy) vs **Link** (live). Load/Save/Save As/New/Merge/Rename/Unload. Per-device auto-load. Shared via network drive. Relinks by name match. | `Reference/MaterialLibrary` | N | T1 |
| CAL-03 | Interval Test | Speed, Power, Steps, Min/Max Interval (0.08–0.16 suggested), Size, Simple Fill vs Dithered Image | `Reference/IntervalTest` | N | T1 |
| CAL-04 | Focus Test | Needs motorized Z. Start Z, End Z, Steps, Speed, Max/Min Power, High Power Laser Mode. Galvo support not stated (UNVERIFIED). | `Reference/FocusTest` | N | T2 |
| CAL-05 | Galvo lens calibration (9-point) + Calibrate Axis | see GDEV-13 | `Reference/CalibrateGalvoLens` | P | T0/T1 |
| CAL-06 | Scanning offset adjustment | see DEV-07 | | N | T2 |
| CAL-07 | Center Finder | 3 jogged points → center (also squares, triangles, hexagons), Move to Center, guide circle on T1 | `Reference/CenterFinder` | N | T2 (galvo variant via red dot = T3) |
| CAL-08 | Red-dot/laser offset, Laser 2 offset guides | | `Guides/LaserOffsetSetup`, `Guides/SetLaser2Offset` | N | T1 / T3 |

---

## 14. Cameras (`Reference/Cameras/*`)

| ID | Feature | LightBurn detail | Proto | Tier |
|---|---|---|---|---|
| CAM-01 | Camera support model | Per-device cameras. **Overhead** and **Head-mounted**. Network cameras and dual cameras with "wall view" (2.1). Exposure/focus/white-balance controls. Preview FPS. | N | T2 |
| CAM-02 | **Lens calibration** | **AprilTag** pattern (printed or on a tablet); Auto Capture or manual; Refine Corners; Mirror Camera View. Models: **Extended Camera Model**, **Extended Model**, **Fisheye Model**. Score = mean reprojection error. Export/import `.lbcm`. | N | T2 |
| CAM-03 | **Camera alignment** | Burn a target (crosshairs + AprilTags; Fill Features). **Automatic** (≥4 of 8 tags detected) or **Manual** (click 4 crosshairs); Revert to Manual. Feature Width, Pattern W/H, presets (Two Zones). Must redo if the camera moves. Overhead Auto Alignment (2.2). | N | T2 |
| CAM-04 | Overlay | Update Overlay (head cam: Single / Selected / Workspace scan), Opacity (min 10%), overlay W/H and X/Y shift, Calibrated/Uncalibrated capture, Save Background Capture | N | T2 |
| CAM-05 | **Trace Overlay** (trace from camera) | Opens Trace Image on the capture | N | T2 |
| CAM-06 | **Print and Cut** (2 targets, jog + set) | Scaled or No Scaling; Absolute Coords; **gantry only** | N | T2 (T3 for galvo via red dot) |
| CAM-07 | **Camera Print & Cut** (Labs, 2.2) | Head camera finds ≥3 marks (filled circle 5 mm, square or donut). Fit modes Auto / Keep size / Resize evenly / Resize and stretch. Light-on-dark option. **Galvo not supported.** | N | T3 |
| CAM-08 | LiDAR | ACCELaser HD1 only. Scan Selected / Full Workspace, Scan Interval, Smooth, depth overlay, "Include Lidar Data" per layer. | N | T3 |
| CAM-09 | Camera/Depth Overlay Registration widget (2.2), Calibration Target Collector | | N | T3 |

**Note for the MOPA user:** a fixed overhead camera above a galvo field, calibrated with AprilTags and aligned by marking a target, is technically straightforward. LightBurn does not offer it for galvos, which makes it a **T3 differentiator** worth considering early in the architecture.

---

## 15. Rotary and special production modes

| ID | Feature | LightBurn detail | Source | Proto | Tier |
|---|---|---|---|---|---|
| ROT-01 | Rotary (G-code) | Chuck/Roller, Enable Rotary, Mirror Output, Rotary Axis **Y / Z / A**, **MM Per Rotation**, Test, Roller/Object Diameter, Circumference. 2.2 adds Rotary Park Position. | `Reference/RotaryMode/RotaryModeGCode`; `NewFeatures` | N | T1 (Y-swap) / T2 (A axis) |
| ROT-02 | Rotary (DSP/Ruida) | Steps Per Rotation, Read Settings, writes to controller (power-cycle) | `Reference/RotaryMode/RotaryModeDSP` | N | T2 |
| ROT-03 | Rotary (galvo) | see GLV-12 | | N | T2 |
| ROT-04 | Full-wrap tumblers guide | Graphic height = circumference | `Guides/FullWrapTumblers` | N | T2 |
| ROT-05 | Feeder (auto-feed conveyor) | Ruida/Trocen: Enable Auto-Feed, feed before cutting, pause after feed, distance (Job Height / Y length / Manual), adjust, Repeat Count | `Reference/FeederSetup` | N | T3 |
| ROT-06 | Dual laser (DSP) | Enable Laser 2 Controls, one at a time, Laser 2 Offset | `Reference/DualLaserControl` | N | T3 |

---

## 16. Automation and integration

| ID | Feature | LightBurn detail | Source | Proto | Tier |
|---|---|---|---|---|---|
| AUT-01 | UDP command channel | Listens on **19840**, replies on **19841**. `LOADFILE:`, `FORCELOAD:`, `IMPORT:`, `LASER:`, `START`, `STATUS`, `PING`, `CLOSE`, `FORCECLOSE`. Responses `OK` / `!` / `?`. SendUDP.exe on Windows. | `Guides/AutomationWithUDP` | N | T2 (cheap to emulate for drop-in compatibility) |
| AUT-02 | **REST API** (Labs, 2.2) | Port **19520**, localhost by default, "Allow API Access From Network" opt-in. Pairing via `POST /api/connect` plus 30-second on-screen approval; Bearer tokens; capability scopes `state`, `project`, `upload`, `project_write`. Endpoints: `/api/status`, `/api/position`, `/api/jog/settings`, `/api/goto/saved`, `/api/units`, `/api/events` (SSE), `/api/events/poll`, `/api/project`, `/api/layers`, `/api/cuts`, `/api/cuts/{i}` (GET/POST), `/api/material-library`, `/api/overlay` (PNG), `/api/overlay/metadata`, `/api/file/upload`, `/api/file/open`. **No endpoint starts a job**; the documented intent is that the operator always starts it. Samples: Inkscape extension, FreeCAD workbench. | `Reference/RestAPI`; <https://github.com/LightBurnSoftware/LightBurn-REST> (`docs/api-reference.md`, `SAFETY.md`) | P (proto has its own local JSON HTTP API on 127.0.0.1:8765 *including* mark, gated by ARM) | T1 (OpenLaser's own API is architectural; LightBurn-compatible subset T2) |
| AUT-03 | CorelDRAW macro | Send from CorelDRAW | `Guides/CorelDRAWMacroSetup` | N | T3 |
| AUT-04 | Shared settings for multi-user / education | | `Guides/SharedSettingsMultiUser` | N | T3 |
| AUT-05 | Auto-launch files in running copy | | `Reference/SettingsPreferences` | N | T2 |

---

## 17. File formats

### 17.1 Import/export matrix (`Reference/FileManagement`)

| Direction | Formats | Proto | Tier |
|---|---|---|---|
| Import vector | `.ai`, **`.svg`**, **`.dxf`**, `.pdf`, `.plt/.hpgl`, `.gc/.gcode/.nc` (geometry only), **`.lbrn/.lbrn2/.lbt`** (graphics only via Import; Open loads settings), `.mage` (MillMage), `.rd/.rld` (Ruida), `.sc`, `.scpro2` | P: SVG (line/polyline/polygon/rect/circle/ellipse + path M/L/H/V/Z only; **no Béziers/arcs, no `transform`, no viewBox units**); DXF (LINE, LWPOLYLINE with bulge, POLYLINE, CIRCLE, ARC, ELLIPSE, SPLINE control points) | T0: full SVG path + transforms + units, DXF incl. proper splines. T1: `.lbrn2`, PDF. T2: AI, HPGL, G-code, RD. |
| Import raster | `.png .jpg .bmp .gif .tif .webp` | N | T1 |
| Import PCB (2.2) | Gerber `.gbr .gtl .gbl .gts .gbs .gto .gbo .gtp .gbp .gko .gm1 …` + drill `.drl .xln .exc .xnc .gbrjob` | N | T3 |
| Import 3D (2.2) | `.stl .3mf .amf .obj .off .ply` → depth map | P (STL → triangle outlines) | T2 |
| Export | `.ai`, `.svg`, `.dxf` (arcs or lines), image (`.png/.jpg/.bmp`, single image) | N | T1 (SVG/DXF) |
| Project | `.lbrn2` (default; `FormatVersion="1"`), `.lbrn` (legacy, `FormatVersion="0"`, more verbose), `.lbt` template | P (`workspace.json` + `layer_settings.json`; machine-local, git-ignored) | T0 (own project file) / T1 (`.lbrn2` read) / T2 (`.lbrn2` write) |
| Machine files | G-code, `.rd`, OUT, UD5 | Y (G-code) | T0 / T2 |
| LightBurn side formats | `.clb` (material library), `.lbart` (art library), `.lbzip` (user bundle), `.lbdev` (device, JSON), `.lbvendor`, `.lbset` (machine settings backup), `.lbso` (scanning offsets), `.lbmt` (material test presets), `.lbcm` (camera calibration) | N | T1: `.lbdev`, `.clb`. T2: others. |

The settings docs mention import options: group imported shapes, AI hidden layers, DXF auto-detect units, auto-close tolerance, ignore DXF text, SVG 96/72 DPI, HPGL precision, output curve tolerance 0.05 mm (`Reference/SettingsPreferences`).

### 17.2 `.lbrn2` structure (for import compatibility)

LightBurn does not document this format. The structure below comes from two real files:

- `fire_flower.lbrn2`: MOPA galvo, AppVersion 1.6.00, from <https://github.com/benkrasnow/MOPA_Laser_Stainless_Colors>
- `coat-hook-v4.lbrn2`: gantry, AppVersion 1.1.03, from <https://github.com/dwyl/workshop>

Both files were downloaded to the scratchpad and parsed with a local script, never executed. Two community references were cross-checked as untrusted data: `agentydragon/ducktape laser/lightburn/FORMAT_RESEARCH.md` and <https://github.com/jlucaso1/lbrn2-to-svg> `docs/main.md`.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<LightBurnProject AppVersion="1.6.00" FormatVersion="1" MaterialHeight="0" MirrorX="False" MirrorY="False">
  <Thumbnail Source="<base64 PNG>"/>
  <VariableText> <Start Value="0"/><End Value="999"/><Current Value="0"/><Increment Value="1"/><AutoAdvance Value="0"/> </VariableText>
  <UIPrefs> <Optimize_ByLayer Value="0"/><Optimize_ByGroup Value="-1"/><Optimize_ByPriority Value="1"/>
            <Optimize_WhichDirection/><Optimize_InnerToOuter/><Optimize_ByDirection/><Optimize_ReduceTravel/>
            <Optimize_HideBacklash/><Optimize_ReduceDirChanges/><Optimize_ChooseCorners/><Optimize_AllowReverse/>
            <Optimize_RemoveOverlaps/><Optimize_OptimalEntryPoint/><Optimize_OverlapDist Value="0.025"/> </UIPrefs>
  <!-- MOPA galvo layer, verbatim keys from fire_flower.lbrn2 -->
  <CutSetting type="Scan">            <!-- type: Cut (Line) | Scan (Fill) | Scan+Cut (legacy) ; Offset Fill also exists -->
    <index Value="0"/><name Value="C00"/>
    <minPower Value="19"/><maxPower Value="30"/><maxPower2 Value="20"/>
    <speed Value="800"/>              <!-- galvo mm/s -->
    <frequency Value="300000"/>       <!-- Hz -->
    <QPulseWidth Value="60"/>         <!-- ns, MOPA -->
    <scanOpt Value="individual"/><bidir Value="0"/><interval Value="0.003"/>
    <priority Value="0"/><tabCount Value="1"/><tabCountMax Value="1"/>
  </CutSetting>
  <CutSetting_Img type="Image"> <index Value="0"/> … <ditherMode Value="stucki"/> … </CutSetting_Img>
  <Shape Type="Path" CutIndex="4" VertID="148949">
    <XForm>0.1317 0 0 0.1317 106.97 136.76</XForm>        <!-- affine a b c d e f -->
    <VertList>V108.1 -67.8c0x122.58c0y-74.1c1x…V…</VertList>   <!-- vertex + optional Bézier control points c0/c1 -->
    <PrimList>B0 1B1 2B2 3B3 0</PrimList>                   <!-- primitives: L i j (line), B i j (Bézier); or "LineClosed" -->
  </Shape>
  <Shape Type="Path" CutIndex="0" VertID="170791" PrimID="17"> <XForm>…</XForm> <VertList>…</VertList> </Shape>  <!-- PrimList shared by ID -->
  <Shape Type="Text" CutIndex="0" Font="Arial,-1,100,5,50,0,0,0,0,0" Str="Coat Hook V.4" H="7.5" LS="0" LnS="0" Ah="1" Av="1" Weld="1" HasBackupPath="1">
    <BackupPath …>(Path shape)</BackupPath> <XForm>…</XForm> </Shape>
  <Shape Type="Bitmap" CutIndex="0" W="40.22" H="39.96" Gamma="1" Contrast="0" Brightness="0" EnhanceAmount="0" EnhanceRadius="0" EnhanceDenoise="0" File="" SourceHash="0" Data="<base64 PNG>"> <XForm>…</XForm> </Shape>
  <Shape Type="Group"> <XForm>…</XForm> <Children> <Shape …/> … </Children> </Shape>
  <Notes ShowOnLoad="0" Notes=""/>
</LightBurnProject>
```

**Observations relevant to an importer:**

- **Element style.** Each layer is a `<CutSetting>` whose parameters are child elements of the form `<key Value="…"/>`. Only non-default keys are written, so the importer needs LightBurn's defaults table (§7.7 is the best proxy).
- **Images.** Image layers are separate `<CutSetting_Img>` elements with the same `index`, matching REST's "30 image cuts".
- **Galvo units.** `frequency` is stored in **Hz** and `QPulseWidth` in **ns**. Speed is in the device's control unit (800 = mm/s on the galvo).
- **Geometry sharing.** `VertID`/`PrimID` share geometry. The first shape with an ID carries the list; later shapes with the same ID omit `VertList`/`PrimList`. Verified in both files: in `coat-hook-v4`, one shape carries only `<XForm>`.
  - Bitmaps may also be de-duplicated: copies lacked `Data` but showed no explicit ID, so the reference mechanism is **UNVERIFIED**.
- **Text.**
  - `Font` is Qt's `QFont::toString()`, which matches LightBurn being Qt 6.
  - `HasBackupPath="1"` plus `<BackupPath>` stores a vector fallback. Importers should use it when the font is missing.
  - Other shape types seen in community docs: `Rect` (W, H, `Cr` corner radius; XForm e,f = centre) and `Ellipse` (Rx, Ry).
  - XML tags for 2.0+ shapes (Polygon/Star/Barcode) and virtual arrays are **UNVERIFIED**.
- **Sub-layers.** These appear as `<SubLayer type=… index=…>` children carrying only overridden params, with a `subname` label (community doc; **UNVERIFIED** against a 1.2+ sample).
- **Coordinate system.** **Conflicting** third-party claims: one says Y-down, while `lbrn2-to-svg` treats it as Y-up and flips for SVG. **UNVERIFIED.** Settle it with a known-geometry test file before writing the importer.
- **Galvo timing and wobble XML keys** (Laser On TC, wobble, etc.) did not appear in the sample, which used defaults. Their names are **UNVERIFIED**; the REST field names in §7.7 are the best guide.
- **Opening in LightBurn.** Import loads graphics only; Open loads cut settings too. Opening a file also **overwrites optimization settings** from `<UIPrefs>` (`Reference/OptimizationSettings`).

---

## 18. Settings, UI and application features

| ID | Feature | LightBurn detail | Source | Proto | Tier |
|---|---|---|---|---|---|
| UI-01 | Workspace | Zoom/pan, grid, rulers, view styles (wireframe/filled), Show Notes, New Window, Print (to paper), work-area center cross, full-screen cursor | `Reference/UI/Workspace`, `Reference/ViewStyle`, `Reference/Print` | P (zoom/pan/fit) | T0 basic; T1 rest |
| UI-02 | Units | Design units vs machine (control) units, linked or split (2.0/2.1) | 2.0/2.1 blogs | P (mm only) | T1 |
| UI-03 | Themes | System / Classic Dark / Classic Light (2.0) | 2.0 blog | P (dark web UI) | T1 |
| UI-04 | Hotkeys editor; custom macro hotkeys | `Reference/EditHotkeys` | P (fixed shortcuts) | T1 |
| UI-05 | Beginner Mode (hides advanced tools) | `Reference/BeginnerMode` | N | T2 |
| UI-06 | Dockable, customizable windows; reset layout | `Guides/CustomizingTheLightBurnWindow` | N | T2 |
| UI-07 | Auto-save interval; job log; usage data; debug log; in-app bug report (2.2) | `Reference/SettingsPreferences`, 2.2 blog | P (workspace autosave) | T1 |
| UI-08 | Localization (Language menu) | `Reference/UI/LanguageMenu` | N | T2 |
| UI-09 | Topic-aware help/tooltips; vector troubleshooter (2.2) | `Reference/UI/Tooltips`; `NewFeatures` | N | T2 |
| UI-10 | Check for updates; license management | `Reference/CheckForUpdates` | N | T2 (no licensing needed) |

---

## 19. Tier roll-up

### T0 — MVP (replace the Python prototype; JCZ/LMC MOPA + GRBL export)

**Prototype features to carry over (Y/P today):**

- **Galvo direct control:** JCZ/LMC (0x9588:0x9899) over libusb, macOS first. Covers detect (no laser command), markcfg import and inspection, frequency plus a Q-pulse table snapped to the JPT M7, power, speed, passes.
- **Framing and marking:** red-light framing (once/continuous, contour + bounds), Mark, fast Stop (spooler-abort fix), Repeat, Selected-only, mirror H/V, mock backend.
- **Safety:** ARM + final confirmation, refuse out-of-field, refuse >3000 paths / >20000 points, frame-only never burns, whole-job check before first emission, finite coordinates.
- **G-code export:** GRBL CO2 (M3) / diode (M4) / A4 plotter, S-max scaling, air assist, tool-off before travel.
- **Layers:** Line + Fill (interval, angle, angle increment, crosshatch) + tool/frame-only; output/show; rename/reorder.
- **Workspace:** select/move/scale/rotate/center/delete, undo/redo, zoom/pan/fit, join connected lines.
- **Calibration:** per-axis scale correction (generalized from the single 1.1834 factor).

**Missing in the prototype but needed for a solid T0:**

- **Import:** full SVG (Béziers, arcs, transforms, viewBox/units), better DXF splines.
- **Project file:** a real, versioned project file (not machine-local JSON).
- **Galvo settings:** device-level jump/mark delays (Laser On/Off TC, Polygon/End TC, jump speed/delays), field offset, axis negate/swap, red-dot offset/scale.
- **Preview:** time estimate (galvo: mark/jump speeds + delays) and preview with travel moves.
- **Optimization:** reduce-travel ordering.
- **Safety:** job checklist / require-framing semantics.

**T0-stretch (cheap, high value for this user):** Material Test grid (frequency × Q-pulse / power × speed), sub-layers (outline + fill + cleanup), serial-number variable text plus QR/DataMatrix, image import with Jarvis/Stucki/Threshold, door-protect/stop IO inputs.

### T1 — Credible LightBurn alternative

- **GRBL control:** live control over USB serial and TCP/WiFi (buffered/synchronous streaming), console, macros, Move window, Start From + 9-point origin, frame / rubber-band frame, pause/resume, overrides, `$$` editor with backup.
- **Images:** raster import with Threshold/Ordered/Atkinson/Dither/Stucki/Jarvis/Grayscale, plus image adjustments (gamma/contrast/brightness/enhance).
- **Cut settings:** full Line/Fill/Offset Fill, sub-layers + global passes, kerf, tabs, overscan, fill grouping, flood fill, dot width correction, min power, constant power, per-shape power scale/priority.
- **Galvo:** per-layer timing override, wobble, COR load + bulge/skew/trapezoid + 9-point wizard, IO ports (foot switch, door, lights), job counter / run continuously / Stop At, Raycus/IPG/MAX sources, EZCAD2 Lite.
- **Design suite:** shapes, pen tool, node editing, booleans/weld, offset, align/distribute, group, grid/circular arrays, text with system + single-line (SHX) fonts, variable text, common barcodes, snapping, measure, clipboard.
- **Optimization:** order by layer/group/priority, inner-first, best start point/direction, remove overlaps.
- **Preview:** playback + accel-model estimate + shade-by-power.
- **Material workflow:** material library (assign/link), material test, interval test.
- **Files:** `.lbrn2` import (graphics + cut settings), SVG/DXF export, `.lbdev`/`.clb` import.
- **Devices:** multiple editable device profiles, Find-My-Laser-style discovery.
- **Platforms:** Linux and Windows parity, native ARM macOS.

### T2 — Parity

- **Controllers:** Ruida (UDP/USB, RD files, file list, machine settings), Trocen/TopWisdom, Smoothieware, Marlin, GRBL-LPC/M3, Custom G-code templating; EZCAD3 and BSL galvos; external axes.
- **Galvo production:** galvo rotary, split marking, repeat marking, cylinder correction, taper warp, 3D sliced image mode + 3D model → depth map, galvo Z/Move window, auto-rotate.
- **Cameras:** lens calibration (AprilTag), alignment, overlay, trace overlay, head camera; 2-point Print & Cut; Center Finder; Focus Test; scanning offset; gantry rotary (A axis, chuck/roller); DSP dual laser.
- **Design extras:** trace image (incl. center-line, sketch), warp/deform, apply path to text, fillet, cut/trim shapes, rubber-band outline, generate path from cut, convert to bitmap, masks/crop, Halftone/Newsprint/Sketch dithers.
- **Layout:** nesting (quick + true-shape), pages, virtual arrays, art library, undo history, beginner mode, localization.
- **Integration:** UDP automation compatibility, a LightBurn-REST-compatible subset, `.lbrn2` write, PDF/AI/HPGL import.

### T3 — Beyond / nice-to-have

- **Vendor hardware:** xTool / Epilog / ACCELaser / Bambu / Emblaser / Cubiio / ZhiYuan protocols, LiDAR, LightBurn Bridge-style relay, feeder, HTTP G-code flavors, Gerber import, Camera Print & Cut.
- **Beyond LightBurn:**
  - camera alignment and print & cut **for galvos**
  - marking on the fly with encoder
  - EZCAD `.ezd` import
  - more EZCAD hatch types (bow, edge/start/end offsets, loop count)
  - a scriptable plugin API
  - a headless job server / CLI on a Raspberry Pi next to the laser
  - an open, documented project schema
  - deterministic planner golden tests
  - MOPA color-preset libraries per material
  - galvo-dynamics-aware time estimates
  - signed job logs / audit trail
  - a multi-job queue

---

## 20. Where LightBurn is weak (opportunities for OpenLaser)

1. **Linux:** abandoned after 1.7. This rules out Pi-based shop controllers and Linux makerspaces (`GetStarted/InstallLightBurn`).
2. **Galvo + camera:** no alignment or Print & Cut for galvos (`Reference/PrintAndCut`, `Reference/CameraPrintAndCut`).
3. **EZCAD2 ergonomics:** no Move window, no Z (removed 2.1), and no EZCAD project import.
4. **Offset Fill robustness:** the docs admit Offset Fill can hang LightBurn (`Explainers/LayerModes`). Rust plus a robust polygon offsetter is an opportunity.
5. **Closed format:** `.lbrn2` is undocumented. OpenLaser can read it and also publish an open schema.
6. **Safety posture:** LightBurn *warns* on out-of-bounds and offers "Ignore out-of-bounds shapes if possible". The prototype *refuses*. Keep refusal as the default and document it as a feature.
7. **Remote automation:** LightBurn's REST API deliberately can't start jobs. OpenLaser can offer a gated, local-only, ARM-token start for production lines, with an explicit opt-in.
8. **Price:** $199 plus $40/yr for galvo or DSP use.

---

## 21. Open questions and UNVERIFIED items

1. Does LightBurn's EZCAD2/JCZFiber driver support the **USBLMCV4** board revision and "EZCAD2 Lite" boards identically? No public compatibility matrix exists. The only test is a LightBurn trial on the user's machine, which this task did not do (laser disconnected).
2. `.lbrn2` **Y-axis orientation** (third-party sources conflict), the XML tag names for galvo timings, wobble, sub-layers and newer shape types, and how de-duplicated `Bitmap`s reference data. Resolve with LightBurn-saved fixture files covering each feature (needs a LightBurn trial install or community samples).
3. Machine-file mapping: is **`.OUT` TopWisdom and `.UD5` Trocen**? Not stated in the docs.
4. How does LightBurn implement **Grayscale** image mode on galvo fiber? Power modulation per pixel, or not offered? The docs only exclude UV.
5. **GRBL network default port** and protocol (raw TCP/telnet vs WebSocket) are not stated in LightBurn docs.
6. Does LightBurn support **marking on the fly** or **galvo Focus Test**? Neither is documented.
7. The April 2026 **price increase** reported by a third party is not reflected in the official store on 2026-10-22.
8. Tier calls for the user: should **GRBL live streaming** (vs export) be in T0? Should **image engraving** and **Material Test** be in T0 given MOPA color/photo work? These are product decisions for Luiz.
9. LightBurn **Rayforge interop:** Rayforge (MIT) already imports LightBurn `.lbdev` profiles and maps "Galvo" to no driver, so it has no galvo support (<https://github.com/barebaric/rayforge> `rayforge/machine/device/lightburn_importer.py`; the driver dirs list only dummy, grbl, marlin, octoprint, ruidarpa, smoothie as of 2026-10-23T01:56Z). It also has a Rust/PyO3 GRBL driver, `raydriver` (<https://github.com/barebaric/raydriver>). Is either reusable for OpenLaser? Licensing and fit need a separate check.

---

## Appendix A — Python prototype capability snapshot (for the Proto column)

From `~/AI/OpenLaser/OpenMopa/README.md` and `mopa_luiz/*.py` (≈5.8k lines; 15 test modules):

- **`machines.py`** defines 4 built-in profiles:
  - `jpt-m7-200`: fiber_galvo, `jcz_lmc_usb`, 200×200 centre origin, frequency + pulse width, direct control
  - `co2-grbl-600x400`: M3, air assist
  - `diode-grbl-400x400`: M4
  - `plotter-grbl-a4`: `M3 S1000` / `M5`

  Bed sizes are not editable in the UI.
- **`layers.py`:** operations `vector_engrave`, `vector_cut`, `raster_engrave` (hatch fill of closed shapes), `frame_only`, `disabled`. Per layer: power %, frequency kHz, pulse ns, speed mm/s, passes, pitch, angle, angle-step-per-pass, air assist, visible/output, color.
- **`raster.py`:** hatch-line generation only. There is **no bitmap/dither engraving**.
- **`importers.py`:**
  - SVG: line, polyline, polygon, rect, circle, ellipse, and path with M/L/H/V/Z only. **No transforms, viewBox or Béziers.**
  - DXF: LINE, LWPOLYLINE (bulge), POLYLINE, CIRCLE, ARC, ELLIPSE, SPLINE (control points), with units detection.
  - STL: triangle outlines, at most 900 triangles.
- **`geometry.py`:** bounding box, join connected lines with tolerance.
- **`live.py`:** galvoplotter-based connect, frame (once/continuous loop), mark, stop (graceful + spooler abort), board status, scale correction (0.5–2.0 clamp, persisted in `calibration.json`), limits (3000 paths, 2000 points/path, 20000 points), refuse out-of-field, mock mode.
- **`safety.py`:** ARM token, frequency within MIN/MAXPWMFREQ, pulse width in the profile table, power 0–100, at least one markable path.
- **`gcode.py`:** GRBL compiler. Absolute mm, S-scaling to 1000, M3/M4, M8/M9, tool-off before travel, distance estimate.
- **`profile.py`:** markcfg field inspection. MARKDELAY, JUMPDELAY, LASERON/OFFDELAY, POLYGONDELAY, INVERTX/Y, SWAPXY and CORFILE are **parsed but not applied**.
- **`ui.py`:** local HTTP UI on 127.0.0.1:8765 with endpoints `/api/import /api/join /api/plan /api/frame(/start|/stop) /api/mark /api/stop /api/export/gcode /api/test/preview|run /api/calibration/save /api/workspace(/save) /api/layers(/save) /api/detect /api/board /api/profile/inspect`. Undo/redo, autosaved `workspace.json`.
- **`cli.py`:** `machines`, `detect`, `show-config`, `pulse-widths`, `inspect-profile`, `plan-test-box`, `ui`.

## Appendix B — Primary sources used

- LightBurn docs 2.2 (paths cited inline): <https://docs.lightburnsoftware.com/latest/>
- LightBurn 2.2 / 2.1 / 2.0 release posts: <https://lightburnsoftware.com/blogs/news>
- Store and pricing: <https://lightburnsoftware.com/collections/frontpage>, <https://lightburnsoftware.com/pages/which-version-of-lightburn-is-right-for-you>, <https://lightburnsoftware.com/products/lightburn-galvo>, <https://lightburnsoftware.com/blogs/news/new-lightburn-license-types-and-pricing-coming-oct-1-2024>, <https://lightburnsoftware.com/blogs/news/lightburn-for-galvos-release-date-pricing-and-other-details>
- RC cadence: <https://forum.lightburnsoftware.com/tag/release-candidate/593>, <https://forum.lightburnsoftware.com/t/lightburn-2-2-00-early-access-rc-3/192831>
- Linux sunset: <https://forum.lightburnsoftware.com/t/linux-support-to-end-after-v1-7/144605>
- REST API repo (OpenAPI, cut-settings, safety): <https://github.com/LightBurnSoftware/LightBurn-REST>
- `.lbrn2` samples: <https://github.com/benkrasnow/MOPA_Laser_Stainless_Colors> (`fire_flower.lbrn2`), <https://github.com/dwyl/workshop> (`coat-hook-v4.lbrn2`); community notes (treated as data): <https://github.com/jlucaso1/lbrn2-to-svg>, `agentydragon/ducktape/laser/lightburn/FORMAT_RESEARCH.md`
- Ruida UDP ports (non-LightBurn): <https://github.com/meerk40t/meerk40t/blob/main/meerk40t/ruida/udp_connection.py>
- Rayforge driver tree and LightBurn importer: <https://github.com/barebaric/rayforge>
