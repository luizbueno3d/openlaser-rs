# 04 · Rust ecosystem survey for OpenLaser (LightBurn-class, Rust)

*Research date: 2026-10-22/23. Author: Claude (research subagent). Status: input to the OpenLaser Rust plan.*

**How this was gathered.** Crate versions, release dates, licenses, MSRVs and download counts come from the crates.io API (`https://crates.io/api/v1/crates/<name>`, queried 2026-10-22/23, 266 crates, full table in Appendix A). Repository activity comes from the GitHub API (`gh api repos/<owner>/<repo>`, Appendix B). Feature claims come from the crate READMEs, docs.rs pages and release notes linked inline. Rayforge facts come from reading its source (clone at `scratchpad/rayforge-src`, read only, never executed) plus PyPI and GitHub metadata.

**Constraints I worked under.** I sent nothing to USB or serial devices. I ran no third-party code. There is **no Rust toolchain on this Mac** (`cargo` and `rustc` are not on PATH), so I compiled nothing. Binary sizes and performance figures are therefore estimates and are marked **UNVERIFIED**. A maturity score (1 to 5) is my judgement: 5 = boring and stable (1.x API, wide production use), 4 = production-used but pre-1.0 churn, 3 = usable with care, 2 = young, alpha or thinly maintained, 1 = experimental or stale.

---

## 0. TL;DR

**Recommended shape:** a Rust workspace whose core (document, geometry, CAM planner, safety checks, drivers) depends on **no UI crate**. On top of it sits a thin **egui/eframe 0.37** desktop shell. The canvas starts on egui's own painter and moves to a custom **wgpu 30** pipeline (through `egui-wgpu` paint callbacks) once documents get large. **tiny-skia** and **vello_cpu** handle offscreen, export and CI rendering. **Tauri 2 plus a web frontend** is the documented Plan B. The web stack can also come back in a smaller role: an optional LAN remote-control page built from the prototype's existing UI.

| Subsystem | Pick | Version (date) | License | Maturity |
|---|---|---|---|---|
| GUI shell | `eframe`/`egui` (+ `egui_dock` or `egui_tiles`, `muda`, `rfd`) | 0.37.0 (2026-10-15) | MIT OR Apache-2.0 | 4 |
| GPU / canvas | `wgpu` via `egui-wgpu` callbacks; `lyon_tessellation` for fills | 30.0.1 (2026-08-22) / 1.0.23 | MIT OR Apache-2.0 | 4 |
| CPU raster (export, tests) | `tiny-skia` (deterministic goldens), `vello_cpu` (fast previews) | 0.12.0 / 0.3.0 | BSD-3 / MIT OR Apache-2.0 | 5 / 3 |
| Curves | `kurbo` | 0.13.1 (2026-05-13) | Apache-2.0 OR MIT | 4 |
| Booleans | `i_overlay` | 9.1.0 (2026-10-14) | MIT OR Apache-2.0 | 4 |
| Offsets / kerf | `cavalier_contours` | 0.9.0 (2026-08-20) | MIT OR Apache-2.0 | 3 |
| Spatial index | `rstar` (+ `kiddo` for nearest-neighbour ordering) | 0.13.0 / 6.3.0 | MIT OR Apache-2.0 | 5 / 4 |
| SVG import | `usvg` | 0.48.1 (2026-08-02) | Apache-2.0 OR MIT | 4 |
| DXF | `dxf` | 0.6.2 (2026-10-03) | MIT | 3 |
| PDF / AI import | `hayro-interpret` (+ `pdfium-render` as an optional fallback) | 0.8.0 (2026-10-04) | Apache-2.0 OR MIT | 2–3 |
| XML (.lbrn2) | `quick-xml` | 0.42.0 (2026-08-22) | MIT | 5 |
| Text | `parley` + `fontique` + `skrifa` (+ `harfrust` underneath) | 0.12.0 / 0.49.0 / 0.15.0 | Apache-2.0 OR MIT / MIT | 3–4 |
| Images | `image` + `imageproc` + `fast_image_resize` | 0.25.10 / 0.27.0 / 6.1.0 | MIT/Apache | 5 / 4 / 4 |
| Dithering | **write our own** (~500 LOC, golden-tested) | – | – | – |
| Tracing (vectorising) | `vtracer` / `visioncortex` (avoid GPL potrace) | 0.6.5 (1.0 alpha) / 0.9.3 | MIT OR Apache-2.0 | 3 |
| USB (JCZ LMC) | `nusb` (pure Rust, no libusb) | 0.2.8 (2026-10-14) | Apache-2.0 OR MIT | 4 |
| Serial (GRBL) | `serialport` (or `serial2`) | 4.10.1 (2026-09-08) | MPL-2.0 | 5 |
| Network (Ruida, FluidNC) | `std`/`tokio` sockets, `tokio-tungstenite`, `mdns-sd` | 1.54.1 / 0.30.0 / 0.21.6 | MIT / MIT / MIT OR Apache-2.0 | 5 / 4 / 4 |
| Camera | `nokhwa` capture; `calib-targets` (ChArUco); `nalgebra` + `levenberg-marquardt` (or `vision-calibration`) | 0.10.11 / 0.15.5 / 0.35.0 | Apache-2.0 / MIT OR Apache-2.0 | 3 / 2 / 5 |
| Document and file | `serde` + `serde_json` inside a `zip` container, versioned migrations | 1.0.229 / 1.0.154 / 9.0.5 | MIT/Apache, MIT | 5 |
| Logging | `tracing` + `tracing-subscriber` + `tracing-appender` | 0.1.44 / 0.3.23 / 0.2.5 | MIT | 5 |
| i18n | `fluent-bundle` / `fluent-templates` | 0.16.0 / 0.15.1 | Apache-2.0 OR MIT | 4 |
| Scripting | `rhai` (later: `extism`/`wasmtime` plugins) | 1.27.0 (2026-10-21) | MIT OR Apache-2.0 | 4 |
| Packaging | `cargo-packager` (.app/.dmg/NSIS/MSI/AppImage/.deb), then **Velopack** for updates | 0.11.8 / 1.2.161 | MIT/Apache / MIT | 3 / 4 |
| Testing | `insta`, `proptest`, `egui_kittest`, `cargo-nextest`, `cargo-deny`, `cargo-fuzz` | 1.49.0 / 1.12.0 / 0.37.0 | Apache-2.0 / MIT/Apache | 5 / 5 / 4 |

**Findings that change the plan:**

1. **Rayforge's heavy lifting is already in Rust, and it is MIT-licensed.** Its geometry core [`barebaric/raygeo`](https://github.com/barebaric/raygeo) is 3.4 MB of Rust, using `clipper2`, `earcut`, `rstar`, `spade`, `swash`, `fontdb` and `nalgebra`. Its drivers [`barebaric/raydriver`](https://github.com/barebaric/raydriver) are Rust on `tokio` + `tokio-serial`. Both ship to Python through PyO3 abi3 wheels ([PyPI raygeo 1.67.1](https://pypi.org/project/raygeo/), [raydriver 0.5.1](https://pypi.org/project/raydriver/)) and neither is on crates.io. The Python and GTK4 parts are only the shell. Rayforge has **no galvo or EZCAD driver**: `rayforge/machine/device/lightburn_importer.py` maps `"EZCAD": None` and `"Galvo": None` (lines 26–35), and `rayforge/machine/driver/` contains only `dummy`, `grbl`, `marlin`, `octoprint`, `ruidarpa` and `smoothie`.
2. **No Rust crate for JCZ/BJJCZ LMC exists** on crates.io (searched "galvo", "ezcad", "jcz", "lmc") or on GitHub (searched "galvo/ezcad/jcz/lmc language:rust"). The Python reference [`meerk40t/galvoplotter`](https://github.com/meerk40t/galvoplotter) is **MIT**, so porting the protocol (bulk OUT `0x02`, bulk IN `0x88`, 100 ms timeouts, in `galvo/usb_connection.py`) to Rust is license-clean. This driver is OpenLaser's main differentiator.
3. **`nusb` removes the libusb problem on macOS.** It talks to IOKit, usbfs and WinUSB directly, with no C dependency. On macOS no permission configuration is needed ([docs.rs/nusb](https://docs.rs/nusb/latest/nusb/)). It also has hotplug (`watch_devices`) and `Endpoint::cancel_all()` for the fast-stop path ([Endpoint docs](https://docs.rs/nusb/latest/nusb/struct.Endpoint.html)). On Windows it works **only** with WinUSB, so the Zadig/EZCAD conflict remains.
4. **The name "OpenLaser" is taken twice, by active projects.** [`clayfrocker97/OpenLaser`](https://github.com/clayfrocker97/OpenLaser) is Rust, GPL-3.0, created 2026-09-17, a fiber and CO2 controller for MCC100 cards with a Rust server and Svelte web UI. [`theanam/openlaser.app`](https://github.com/theanam/openlaser.app) is MIT, TypeScript/Svelte, a browser GRBL app at openlaser.app, created 2026-10-19 with 127★ in three days. The crates.io names `openlaser`, `openlaser-core` and `open-laser` are still free (HTTP 404 on 2026-10-23).
5. **A precedent from vector editors:** Graphite ([GraphiteEditor/Graphite](https://github.com/GraphiteEditor/Graphite), 27.7k★) moved its desktop app from Tauri to **CEF**. It now renders the canvas natively with `winit` + `wgpu` + `vello` (`desktop/Cargo.toml`) and keeps the web UI in a separate CEF process (commit "Desktop: Isolate CEF-rendered UI into separate crate and process", 2026-07-14). That is evidence that WebView differences, especially WebKitGTK on Linux ([Tauri Linux graphics issues](https://v2.tauri.app/develop/debug/linux-graphics/)), hurt canvas-heavy editors.
6. **Vello's renderer line-up changed** ([vello README](https://github.com/linebender/vello)). `vello_cpu` is now "the most mature choice". "Vello Hybrid" was renamed **`vello_gpu`** (0.3.0, 2026-10-02, per `vello_gpu/CHANGELOG.md`) and is "intended to become the primary renderer". The original compute-shader `vello` crate moved under `research/` and is called "experimental". That makes vello a poor primary canvas renderer for 2026–27.
7. **iced has no accessibility support.** Its `Cargo.toml` on main (0.15.0-dev) has no AccessKit. **Slint** has AccessKit and `muda` native menus (`internal/backends/winit/Cargo.toml`) but is GPL-3.0 or royalty-free with attribution ([LICENSE.md](https://github.com/slint-ui/slint/blob/master/LICENSE.md)). **GTK4** reaches macOS and Windows screen readers only through an AccessKit backend that is **not on by default** (GTK 4.18, [GTK blog 2025-05-12](https://blog.gtk.org/2025/05/12/an-accessibility-update/)).

---

## 1. Prior art: Rust laser software, read before writing code

| Project | Stack | License | Relevance |
|---|---|---|---|
| [barebaric/raygeo](https://github.com/barebaric/raygeo) (Rayforge geometry) | Rust 2021, PyO3 optional (`python` feature); deps: clipper2 0.6, earcut 0.4, rstar 0.13, spade 2, roxmltree 0.21, svgtypes 0.16, swash 0.2, fontdb 0.24, nalgebra 0.35, rayon, zstd, rmp-serde | MIT | Battle-tested laser geometry and ops pipeline. **Can be used as a git dependency** (`default-features = false`) or studied. Not on crates.io; 4★, so the bus factor is one maintainer. |
| [barebaric/raydriver](https://github.com/barebaric/raydriver) | Rust, tokio + tokio-serial, async-trait | MIT | GRBL/Marlin/Smoothie streaming reference in Rust. |
| Rayforge `rayforge/image/lightburn/importer.py` (823 lines) | Python, XML via ElementTree | MIT | Best public reference for the `.lbrn2` format (vertex list `V…c0x…`, primitive lists). |
| [clayfrocker97/OpenLaser](https://github.com/clayfrocker97/OpenLaser) | Rust 1.97, edition 2024; axum server + Svelte UI in `wry`/`tao`/`muda`; `usvg`, `read-fonts`, `i_overlay`, `cavalier_contours`, `quick-xml`, `jagua-rs` + `sparrow` (nesting), `insta`, `proptest`, `ts-rs` | **GPL-3.0** (do not copy code) | Shows a Rust laser controller choosing almost the same geometry crates. **Name collision.** |
| [theanam/openlaser.app](https://github.com/theanam/openlaser.app) | TS/Svelte, WebSerial, GRBL | MIT | **Name collision.** Feature checklist (dither modes, tracing, Google Fonts). |
| [feveal/gcodekit5(-design)](https://github.com/feveal/gcodekit5-design) | Rust + GTK4 | NOASSERTION | A GTK4 Rust G-code sender/designer. |
| [aalarchiv/ivacam](https://github.com/aalarchiv/ivacam) | Rust core + Tauri + Svelte (+ Android, Wasm) | GPL-3.0 | CAM for hobby CNC/laser; shows the Tauri route. |
| [sameer/svg2gcode](https://github.com/sameer/svg2gcode) | Rust (`g-code` crate) | MIT | SVG to G-code reference, 445★. |
| [pedro2d10/brulelumiere](https://github.com/pedro2d10/brulelumiere) | Rust, LightBurn-inspired | MIT | Created 2026-10-17; too early to judge. |
| [meerk40t/galvoplotter](https://github.com/meerk40t/galvoplotter) | Python, pyusb | MIT | **The LMC protocol reference** (`galvo/controller.py`, `consts.py`, `usb_connection.py`, `mock_connection.py`). |

---

## 2. GUI framework (subsystem 1)

### 2.1 What OpenLaser needs from the UI

- A large zoomable canvas: thousands of paths, more than 50k segments after tracing, selection handles, node editing, snapping, rubber-band selection, rulers and grid, camera overlay, toolpath preview and simulation.
- Text editing on the canvas and in property panels, with IME.
- Docking panels (Cuts/Layers, Shape properties, Laser, Move, Camera, Library), a native menu bar on macOS, configurable keyboard shortcuts.
- Native file dialogs, HiDPI, dark mode, accessibility.
- macOS first (M2, macOS 26), then Windows and Linux. A small team helped by AI coding agents.

### 2.2 Candidates (crates.io and GitHub facts)

| Framework | Latest (date) | License | Activity | Rendering | A11y | Notes |
|---|---|---|---|---|---|---|
| **egui / eframe** | 0.37.0 (2026-10-15); 0.36.0 2026-08-05 | MIT OR Apache-2.0 | 31.7k★, 27.8M dl, releases every 1–2 months | wgpu 30 or glow | AccessKit built in, improved in 0.37 ([release notes](https://github.com/emilk/egui/releases/tag/0.37.0)) | Immediate mode. `egui::Scene` gives a pan/zoom container ([docs](https://docs.rs/egui/latest/egui/containers/struct.Scene.html)). 0.37 adds system fonts via `fontique`, colour emoji, `CompletionPopup`, an experimental `StyleProvider` theming API and image paste. MSRV 1.95. |
| **iced** | 0.14.0 (2025-12-07); main = 0.15.0-dev | MIT | 31.8k★; slow release cadence (0.13 Sep 2024, then 0.14 Dec 2025) | wgpu (0.14: wgpu 27; main: 30), tiny-skia fallback | **None** (no accesskit in `Cargo.toml`; [oxiui-iced README](https://docs.rs/crate/oxiui-iced/0.2.1/source/README.md)) | Elm architecture. `canvas` + `geometry::Cache`, `shader` widget, `pane_grid`. 0.14 adds headless/e2e testing and time-travel debugging ([0.14 notes](https://github.com/iced-rs/iced/releases/tag/0.14.0)). |
| **Slint** | 1.18.1 (2026-09-21) | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | 24.3k★ | femtovg/skia/software; wgpu interop | AccessKit (optional feature) | `.slint` DSL. Native macOS `MenuBar` ([docs](https://docs.slint.dev/latest/docs/slint/reference/window/menubar)) via `muda`. Custom GPU content only through unstable versioned features (`unstable-wgpu-29/30`, `Image::try_from(wgpu::Texture)`) ([wgpu module docs](https://docs.slint.dev/latest/docs/rust/slint/wgpu_29/)). No docking. Royalty-free licence needs an `AboutSlint` widget or a website badge ([license PDF](https://slint.dev/agreements/slint-royalty-free-license.pdf)). |
| **gtk4-rs** (+ relm4, libadwaita) | gtk4 0.11.5 (2026-09-20); relm4 0.11.0 | MIT (bindings); GTK is LGPL-2.1+ | 2.4k★ bindings | GSK / cairo | Linux: AT-SPI. macOS/Windows: AccessKit backend from GTK 4.18, **non-default** | Most complete widget set (Rayforge uses it). macOS packaging is heavy: Rayforge's `scripts/mac/mac_build.sh` is 648 lines of Homebrew dylib bundling and `install_name_tool` rpath fixes. Not native-looking on macOS. |
| **Tauri 2 + web UI** | tauri 2.12.5 (2026-10-21); wry 0.57.0 | Apache-2.0 OR MIT | 112k★, 38.5M dl | System WebView (WKWebView / WebView2 / WebKitGTK) | Native, through the webview | Reuses the prototype's 3k-line web UI. IPC: events are JSON and "not designed for low latency or high throughput"; **channels** "are designed to be fast and deliver ordered data" ([docs](https://v2.tauri.app/develop/calling-frontend/)). Signed updater built in. Linux WebKitGTK GPU issues ([docs](https://v2.tauri.app/develop/debug/linux-graphics/)). |
| **GPUI + GPUI Kit** | gpui 0.2.2 (official, 2025-10-22); `gpui-pre` 0.3.9 (2026-10-12, published by crates.io user `huacnlee`, the gpui-kit maintainer, not by Zed); gpui-kit/gpui-component 0.8.0 (2026-10-19) | Apache-2.0 | Zed 92k★; gpui-kit 18.5k★ | Metal/Vulkan/DX (Zed's) | AccessKit wired into gpui and gpui-kit (dependency present; [gpui-kit README](https://github.com/longbridge/gpui-kit) claims tested roles) | Gpui-kit has 75+ components, **Dock layout**, virtual tables, a code editor, and "120 FPS". Risk: pre-1.0, Zed-driven API churn, and an **unofficial crates.io republish** (`gpui-pre`). |
| **Dioxus** | 0.7.10 (2026-07-30); 0.8.0-alpha.3 (2026-10-16) | MIT OR Apache-2.0 | 39.5k★ | Desktop = webview (wry); "Native" = Blitz (0.3 beta) | Webview | RSX in Rust, but canvas drawing still goes through JS in the webview. Blitz native renderer not production-ready. |
| **Xilem / Masonry** | 0.4.0 (2025-10-29) | Apache-2.0 | 5.6k★ | vello / vello_cpu / skia | AccessKit | Alpha, no release in a year. |
| **Makepad** | makepad-widgets 1.0.0 (2025-05-13) | MIT OR Apache-2.0 | 7.2k★, active repo | Own shader DSL | UNVERIFIED | Focus moved to "AI-accelerated" studio, streaming and 3D (README). Niche. |
| **Freya** | 0.4.3 (2026-08-30); 0.5.0-beta.12 | MIT | 3.2k★ | Skia (`freya-skia-safe`) | AccessKit 0.25 | Small community. Skia binaries are heavy to ship. |
| **Floem** (Lapce) | 0.2.0 (2024-11-14); git active (2026-09-24) | MIT | 4.3k★ | vello / tiny-skia / skia | UNVERIFIED | crates.io release stale for 2 years. Uses a winit fork. |

### 2.3 Weighted decision matrix

Scores are 1–5 (my judgement, justified above and in §2.4). The weighted total is out of 100.

| Criterion (weight) | egui | Tauri 2 + web | GPUI + Kit | iced | Dioxus 0.7 | Slint | Freya | gtk4-rs | Xilem | Floem | Makepad |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Canvas perf and custom GPU rendering (18) | 4 | 3.5 | 4 | 4 | 3 | 2.5 | 4 | 3.5 | 3.5 | 3.5 | 4 |
| Editor logic stays in Rust: hit-test, snap and node edit share core geometry, no IPC or 2nd language (10) | 5 | 2 | 5 | 5 | 3.5 | 4 | 5 | 4 | 5 | 5 | 4 |
| Desktop widgets: docking, menus, forms, shortcuts (10) | 4 | 5 | 4.5 | 3 | 3.5 | 3.5 | 3 | 5 | 2 | 3 | 3 |
| Text input / IME (4) | 3 | 5 | 4.5 | 3.5 | 5 | 4 | 4 | 5 | 4 | 3.5 | 3 |
| Accessibility (7) | 4 | 4.5 | 3 | 1 | 4.5 | 4 | 4 | 3 | 4 | 1.5 | 1 |
| Native integration: menu bar, dialogs, HiDPI, dark mode (7) | 3 | 4 | 4 | 2.5 | 3.5 | 4 | 2.5 | 2 | 2 | 2.5 | 2 |
| macOS-first packaging and signing simplicity (9) | 5 | 4.5 | 4.5 | 5 | 4.5 | 4.5 | 3.5 | 1.5 | 5 | 5 | 4 |
| Windows/Linux portability and consistency (7) | 5 | 3.5 | 4 | 5 | 3.5 | 5 | 4 | 3 | 4.5 | 4.5 | 4 |
| Maturity / API stability (10) | 3.5 | 4.5 | 2.5 | 3 | 3.5 | 4.5 | 2.5 | 4.5 | 1.5 | 2 | 2.5 |
| Community, docs, AI-assistant familiarity (7) | 5 | 5 | 3.5 | 4.5 | 4.5 | 4 | 2.5 | 3.5 | 3 | 3 | 2.5 |
| License fit for an MIT project (6) | 5 | 5 | 5 | 5 | 5 | 2.5 | 5 | 4 | 5 | 5 | 5 |
| Headless / CI testability (5) | 5 | 3.5 | 4 | 4 | 3 | 4 | 3.5 | 3 | 3.5 | 2.5 | 2 |
| **Weighted total / 100** | **85.6** | **81.0** | **80.4** | 76.4 | 75.4 | 75.1 | 72.6 | 70.2 | 70.2 | 69.0 | 64.3 |

**Sensitivity.** Tauri 2 overtakes egui if you add "reuse the prototype's web UI" as a criterion, or raise native look and accessibility above about 12% each. GPUI overtakes egui if its maturity reaches 4, which would need an official crates.io GPUI release and a stable API. Re-score at milestone M2.

### 2.4 Recommendation

**Primary: egui/eframe 0.37 with a strict core/UI split.**

- **Why.**
  - The whole editor stays in Rust: hit-testing, snapping and node editing call the same `kurbo`, `i_overlay` and `rstar` code as the CAM planner.
  - One binary, simple packaging, no WebView differences across platforms.
  - AccessKit is built in and was improved in 0.37.
  - `egui_kittest` gives AccessKit-driven UI tests with image snapshots.
  - Docking comes from `egui_dock` (0.21.1, MIT, 4.8M dl) or Rerun's `egui_tiles` (0.18.0).
  - egui is proven on heavy custom wgpu visualisation (the [Rerun](https://github.com/rerun-io/rerun) viewer).
  - AI coding agents produce idiomatic egui reliably.
- **Known gaps and how to close them.**
  - **No native macOS menu bar** in eframe. Use `muda` 0.21.2 with `init_for_nsapp()` (works for winit-based apps per [muda docs](https://docs.rs/crate/muda/0.1.1)). **Integration with eframe 0.37 is UNVERIFIED**; it is spike item 2.
  - **Non-native look.** Accept it: LightBurn is not native-looking either. Theme it carefully with the 0.37 `StyleProvider` (experimental).
  - **Breaking releases every 1–2 months.** Pin versions and upgrade quarterly.
  - **Immediate mode means you must cache canvas geometry yourself** (see §3).
- **Plan B: Tauri 2 plus a web frontend**, with the Rust core also compiled to Wasm for in-webview hit-testing. Choose it if the spike fails, or if native widgets and accessibility become the top priority. Expect the WKWebView/WebView2/WebKitGTK variance that pushed Graphite to CEF.
- **Watch: GPUI Kit.** Its feature list (dock, AccessKit, virtual tables, 120 fps) is the closest to LightBurn's needs. It is held back by the unofficial `gpui-pre` crate and pre-1.0 churn. Re-evaluate at M2/M3.
- **Not recommended for OpenLaser:**
  - gtk4-rs: macOS packaging cost, and accessibility is not on by default.
  - Slint: no docking, awkward custom canvas, licence friction for forks.
  - iced: no accessibility.
  - Xilem, Floem, Makepad, Freya: alpha, stale or niche.

**Two-week spike (go/no-go for egui):**
1. Load a 20k-path / 200k-segment document (traced photo plus text). Pan and zoom at ≥ 60 fps on the M2 Air. Hit-test under 2 ms with `rstar`.
2. Native macOS menu bar via `muda`, with Cmd shortcuts, inside eframe.
3. On-canvas text tool: edit, IME, then convert to outline.
4. Dock layout persisted with serde.
5. VoiceOver can reach the layer list and buttons.
6. `egui_kittest` snapshot runs in CI.

If items 1 or 2 fail, switch to Plan B before M1.

**Keep the UI swappable.** The core exposes an `AppState` plus a `Command` enum. The UI renders view-models and sends commands; no business logic lives in the UI crate. `ts-rs` 12.0.1 or `specta` 2.0 RC can later generate TypeScript types for a web UI or LAN remote from the same command and event types. That is exactly the pattern of the user's Altura dashboard (static client plus daemons).

---

## 3. 2D rendering and canvas hosting (subsystem 2)

| Crate | Version (date) | License | Maturity | Role and notes |
|---|---|---|---|---|
| `wgpu` | 30.0.1 (2026-08-22) | MIT OR Apache-2.0 | 4 | GPU API (Metal on macOS). egui-wgpu 0.37, vello 0.11, vello_gpu 0.3 and Slint 1.18 all target **wgpu 30** today; that version lockstep is a recurring risk. |
| `epaint` (egui) | 0.37.0 | MIT OR Apache-2.0 | 4 | CPU-tessellated anti-aliased shapes. Good enough for an MVP canvas if we cache flattened polylines per object and only re-flatten on large zoom changes. |
| `lyon` / `lyon_tessellation` | 1.0.19 / 1.0.23 (2026-10-20) | MIT OR Apache-2.0 | 5 | Fill and stroke tessellation into GPU meshes. Stable 1.x. Use it for cached world-space fill meshes (preview fills, raster bounds). |
| `vello_cpu` | 0.3.0 (2026-10-02) | Apache-2.0 OR MIT | 3 | "Most mature" Vello renderer (README). Multithreaded SIMD CPU raster: export PNG, thumbnails, job-preview bitmaps. |
| `vello_gpu` (ex-vello_hybrid) | 0.3.0 (2026-10-02; created 2026-08-26) | Apache-2.0 OR MIT | 2 | Future primary GPU renderer; too young to bet on now. |
| `vello` (compute) | 0.11.0 (2026-10-02) | Apache-2.0 OR MIT | 2 | Now under `research/`, "experimental". Graphite uses it. Avoid as the primary canvas. |
| `tiny-skia` | 0.12.0 (2026-02-02) | BSD-3-Clause | 5 | Deterministic CPU raster used by resvg. **Golden-image tests in GPU-less CI.** |
| `femtovg` | 0.27.0 (2026-08-31) | MIT OR Apache-2.0 | 3 | NanoVG-style on glow, optional wgpu 30. Slint's renderer. Viable but no advantage here. |
| `skia-safe` | 0.153.3 (2026-09-04) | MIT (Skia BSD-3) | 4 | Powerful but adds a large prebuilt C++ binary and build friction. Only worth it with Freya. |

**How each framework hosts a custom canvas:**
- **egui**: `ui.painter()` with `epaint::Shape` (MVP), or `egui_wgpu::Callback` / `CallbackTrait` (`prepare` + `paint` inside egui's render pass) for our own pipelines, or render offscreen to a wgpu texture and show it via `register_native_texture`. `egui::Scene` handles pan and zoom.
- **iced**: `canvas::Program` + `geometry::Cache`; the `shader` widget for raw wgpu.
- **Slint**: `Path` elements, CPU `SharedPixelBuffer` images, or the unstable wgpu-texture import.
- **GTK4**: `DrawingArea` + cairo, `GLArea`, or custom `snapshot()` render nodes.
- **Tauri / Dioxus desktop**: HTML Canvas2D, WebGL2 or WebGPU inside the webview. WebGPU availability in WKWebView on macOS 26 and in WebKitGTK is **UNVERIFIED**.
- **GPUI**: `canvas()` element with `paint_path`. Raw GPU access is **UNVERIFIED**.
- **Freya**: Skia canvas callbacks.
- **Xilem / Floem**: vello `Scene` painting.

**Recommended canvas design:**
- Document in world units (mm, `f64`); one view transform.
- **Hairline strokes** (layer-coloured cut lines) through a custom wgpu "screen-space polyline" pipeline: instanced segment quads expanded in the vertex shader to a constant pixel width. Vertex buffers are cached per object and rebuilt on edit; level of detail comes from re-flattening at the zoom bucket.
- **Fills and preview**: lyon meshes cached in world space.
- **Raster images**: mipmapped wgpu textures.
- **Overlays** (handles, rubber band, snap markers) through egui's painter.
- The same scene description renders through tiny-skia or vello_cpu for exports and CI goldens.
- Hide this behind a `CanvasRenderer` trait so a vello_gpu backend can be added when it matures.

**Key risk:** wgpu major-version lockstep between egui, any vello crate and Slint. Mitigation: own the canvas pipeline on whatever wgpu egui uses; never depend on two wgpu majors.

---

## 4. Geometry (subsystem 3)

| Need | Crate | Version (date) | License | Maturity | Notes and risk |
|---|---|---|---|---|---|
| Canonical curves (Bézier, arcs, transforms, flattening, arclength, nearest point) | **`kurbo`** | 0.13.1 (2026-05-13) | Apache-2.0 OR MIT | 4 | `f64`, accuracy-focused ([README](https://github.com/linebender/kurbo)); the shared currency of usvg 0.48, vello and parley. Pre-1.0 churn. |
| Alt curves | `lyon_geom` / `lyon_path` | 1.0.20 / 1.0.19 | MIT OR Apache-2.0 | 5 | Stable 1.x flattening; kurbo copied some of its code. |
| Alt curves | `flo_curves` | 0.8.1 (2026-08-25) | Apache-2.0 | 3 | Bézier ops and its own path booleans; 99★. |
| GIS-style polygons | `geo` / `geo-types` | 0.33.1 / 0.7.20 | MIT OR Apache-2.0 | 4 | Uses i_overlay internally. Useful algorithms (simplify, convex hull, Fréchet distance) but GIS-oriented. |
| **Polygon booleans** (weld, subtract, intersect, hatch clipping) | **`i_overlay`** | 9.1.0 (2026-10-14) | MIT OR Apache-2.0 (dep `i_float` MIT) | 4 | Robust (integer snapping), fill rules, **clips polylines by polygons**, buffering ([README](https://github.com/iShape-Rust/iOverlay)). Powers `geo`. 215★, single maintainer. |
| Curve-preserving booleans | `linesweeper` / `i_curve` | 0.5.0 (2026-10-05) / 0.2.0 (2026-09-19) | MIT OR Apache-2.0 / MIT | 2 / 1 | linesweeper "early beta"; **its repo moved to Radicle** (GitHub archived; [README](https://github.com/jneem/linesweeper)). Watch; flatten-then-boolean is fine for lasers. |
| Clipper2 | `clipper2-rust` (pure Rust port) / `clipper2` (C++ FFI via `clipper2c-sys`) | 1.2.0 (2026-09-18) / 0.6.0 | BSL-1.0 / MIT OR Apache-2.0 | 3 / 3 | Rayforge uses the FFI one. Keep as a cross-check oracle in tests. |
| **Offsets** (kerf, outline, inset, multi-island) | **`cavalier_contours`** | 0.9.0 (2026-08-20) | MIT OR Apache-2.0 | 3 | Exact arcs ("bulge" polylines); offsets open, closed and self-intersecting polylines; multi-polyline shapes with islands ([README](https://github.com/jbuckmccready/cavalier_contours)). Used by clayfrocker97/OpenLaser. |
| Spatial index (hit-test, snapping, culling) | **`rstar`** | 0.13.0 (2026-05-24) | MIT OR Apache-2.0 | 5 | R*-tree; 50M dl. |
| Nearest-neighbour (path ordering) | `kiddo` | 6.3.0 (2026-09-06) | MIT OR Apache-2.0 | 4 | k-d tree. |
| Triangulation | `earcut` / `i_triangle` / `spade` | 0.4.11 / 0.50.1 / 2.15.1 | MIT OR Apache-2.0 | 4 / 3 / 4 | Rayforge uses earcut and spade. |
| Nesting (LightBurn 2.x feature) | `jagua-rs` + `sparrow` (git, [JeroenGar/sparrow](https://github.com/JeroenGar/sparrow)) | 1.1.0 / git | MPL-2.0 / MIT | 3 / 2 | The crates.io crate named `sparrow` (0.1.0, 2020) is **unrelated**. Defer to a later milestone. |

**Write ourselves** (no adequate crate):
- **Hatch fill generation:** for angle θ, rotate the polygon set, intersect scanlines, clip with `i_overlay`'s polyline clipping, then un-rotate. Supports cross-hatch, bidirectional, overscan and line interval in mm or DPI.
- **Path ordering:** greedy nearest-neighbour seeded by `kiddo`, then 2-opt and Or-opt with a time budget. It also handles reversing open paths and rotating the start vertex of closed paths. A **containment tree** enforces "inner before outer" for cuts. Crates.io TSP crates are GPL (`travelling_salesman`) or new and unproven (`u-routing`).
- **Normalisation:** join nearly-touching ends, dedupe, close within tolerance, remove zero-length segments.
- **Galvo field mapping:** scale correction (1.1834 today), 9-point or grid correction. Refuse out-of-field points; never clamp.

**Key risk:** robustness on messy imported art (self-intersections, near-coincident edges). Mitigations:
- `proptest` invariants: area(A∪B) ≥ max(area); offset(+d) then offset(−d) ≈ the original for convex input.
- Cross-check against `clipper2-rust`.
- When an operation fails, **refuse with a message** rather than emit questionable toolpaths. This matches the prototype's safety rules.

---

## 5. Import / export (subsystem 4)

LightBurn 2.1 imports **AI, SVG, DXF, PDF and PLT/HPGL** plus bitmaps (JPG, PNG, BMP, GIF, TIFF, WebP). It exports SVG, AI and DXF, and `.lbrn/.lbrn2` imports load graphics only ([LightBurn 2.1 File Management](https://docs.lightburnsoftware.com/2.1/Reference/FileManagement/); [Menus](https://docs.lightburnsoftware.com/UI/Menus.html)).

| Format | Crate | Version (date) | License | Maturity | Notes and risk |
|---|---|---|---|---|---|
| **SVG in** | **`usvg`** (+ `svgtypes`) | 0.48.1 (2026-08-02) | Apache-2.0 OR MIT | 4 | Normalises CSS, transforms and `use`, and converts text to paths (fontdb + harfrust + skrifa). Path data is kurbo-compatible. Watch for unit and DPI traps (96 vs 72 dpi exports, `viewBox` vs `mm`), and keep stroke colours to map onto layers. |
| SVG out | write with `quick-xml` `Writer` | 0.42.0 | MIT | 5 | The `svg` crate (0.18.0, 2024-09) is stale. |
| **DXF in/out** | **`dxf`** (ixmilia) | 0.6.2 (2026-10-03) | MIT | 3 | Entities, blocks, R12 to modern. 142★, one maintainer. Corpus-test SPLINE, ELLIPSE, LWPOLYLINE bulges and INSERT scaling. |
| **PDF in** | **`hayro-interpret`** (custom `Device` to capture paths) or `hayro-svg` → usvg | 0.8.0 (2026-10-04) | Apache-2.0 OR MIT | 2–3 | "Experimental, work-in-progress… most feature-complete [Rust PDF renderer]" ([README](https://github.com/LaurenzV/hayro)). Pure Rust, MSRV 1.92. |
| PDF in (fallback) | `pdfium-render` | 0.9.4 (2026-09-06) | MIT OR Apache-2.0 (PDFium: BSD/Apache) | 4 | Needs a PDFium dynamic library per platform, adding packaging and signing work. Optional feature only. |
| PDF low-level / out | `lopdf` / `pdf-writer` / `krilla` | 0.46.0 / 0.15.0 / 0.8.2 | MIT / MIT OR Apache-2.0 / MIT OR Apache-2.0 | 4 / 4 / 3 | `krilla` (typst ecosystem) for high-level PDF export (job reports, material test sheets). |
| PDF alt | `pdf` (pdf-rs) | 0.10.0 (2026-03-02) | MIT | 3 | Parser only. |
| **AI in** | same PDF path | – | – | – | Modern `.ai` files embed a PDF-compatible stream when saved with "Create PDF Compatible File". Old PostScript-based `.ai` is not covered. **UNVERIFIED** how often users hit this. |
| EPS / PS | `stet` (pure-Rust PostScript L3 interpreter, renders to raster or PDF) | 0.8.7 (2026-10-15; created 2026-03) | Apache-2.0 OR MIT | 2 | Young (18★). Alternative: external Ghostscript, which Rayforge uses (`rayforge/image/postscript/ghostscript.py`); Ghostscript is AGPL, so only invoke a user-installed binary. Defer. |
| **HPGL/PLT** | write ourselves (`hpgl` 0.1.0 2020, `nom-hpgl` 2018 are stale) | – | – | – | The PU/PD/PA/PR/SP subset is roughly 200 LOC. |
| **LightBurn .lbrn2** | **`quick-xml`** (streaming + serde) or `roxmltree` (read-only DOM) | 0.42.0 / 0.21.1 | MIT / MIT OR Apache-2.0 | 5 / 4 | Format reference: Rayforge's MIT importer (`rayforge/image/lightburn/importer.py`, vertex-list regexes `V x y c0x… c1x…`). Writing `.lbrn2` is **UNVERIFIED** (undocumented). |
| Raster in | **`image`** (+ `zune-jpeg` inside) | 0.25.10 (2026-03-10) | MIT OR Apache-2.0 | 5 | 223M dl. EXIF orientation: `kamadak-exif` 0.6.1 (BSD-2); whether `image` 0.25 handles it natively is **UNVERIFIED**. |
| **G-code out** | write our own emitter | – | – | – | GRBL laser dialect: M3/M4 dynamic power, `$30` S-max, G0/G1 compaction. svg2gcode (MIT) and `g-code` 0.6.0 (MIT/Apache) are references. |
| G-code in (preview) | `gcode` (no_std parser) | 0.7.0 (2026-04-02) | MIT OR Apache-2.0 | 3 | Low adoption (29k dl) but small and auditable. |

**Key risk:** import fidelity on real-world files. Mitigations:
- A versioned **corpus** of user files (Illustrator, Inkscape, CorelDRAW, Affinity and AutoCAD exports) with `insta` snapshots of the normalised geometry.
- `cargo-fuzz` on every parser, since imported files are untrusted input.
- A visible import report listing unsupported features, never silently dropping content.

---

## 6. Text and fonts (subsystem 5)

| Crate | Version (date) | License | Maturity | Role |
|---|---|---|---|---|
| `fontique` | 0.12.0 (2026-10-09) | Apache-2.0 OR MIT | 3 | System font enumeration and fallback; egui 0.37's `egui_system_font_provider` uses it too. |
| `fontdb` | 0.24.0 (2026-07-29) | MIT | 4 | Used by usvg (unavoidable for SVG text). Two font databases is acceptable; or feed usvg's db from fontique paths. |
| **`parley`** | 0.12.0 (2026-10-09) | Apache-2.0 OR MIT | 3 | Rich layout: bidi, line breaking, alignment, variable axes, styled spans. Linebender; 766★. |
| **`harfrust`** | 0.15.0 (2026-10-22) | MIT | 4 | HarfBuzz port on `read-fonts`, matches HarfBuzz v15.0.0, "less than 25% slower" ([README](https://github.com/harfbuzz/harfrust)). **Supersedes `rustybuzz`**, whose last release was 0.20.1 on 2024-11-12. |
| **`skrifa`** / `read-fonts` | 0.49.0 / 0.46.0 (2026-10-21) | MIT OR Apache-2.0 | 4 | Glyph outlines (feeds kurbo), variable fonts; Google Fonts "oxidize". |
| `swash` | 0.2.10 (2026-07-17) | Apache-2.0 OR MIT | 3 | Scaling and hinting; Rayforge uses it. Not needed if we take skrifa. |
| `cosmic-text` | 0.19.0 (2026-04-22) | MIT OR Apache-2.0 | 4 | Full text editing buffer (iced and COSMIC use it). Pinned to older fontdb 0.23, harfrust 0.5 and skrifa 0.40, which means duplicate deps. |
| `ttf-parser` / `rustybuzz` | 0.25.1 / 0.20.1 (both 2024-11) | MIT/Apache, MIT | 3 (stale) | Legacy; avoid in new code. |
| Single-line fonts | `hershey` (parser) | 0.1.2 (2025-03-26) | Apache-2.0 OR LGPL-3.0 | 2 | Better: embed the Hershey JHF data ourselves. Its distribution terms need a legal check (**UNVERIFIED**). Also support SVG fonts (EMS "Hershey Text" style) and stroke OTFs such as Relief SingleLine, de-duplicating their doubled-back closed contours (**UNVERIFIED** details). `vector-text-hershey` has a "non-standard" licence, so avoid it. |

**Recommendation:**
- Layout with `parley` (+ `fontique`), outlines with `skrifa`, converted to `kurbo::BezPath`.
- On-canvas editing through our own caret and selection model on the parley layout. An alternative is to edit in an egui `TextEdit` overlay and re-layout on every keystroke (simpler, fine for v1).
- Text-on-path: place glyph runs along `kurbo` arclength.
- **Risk:** parley's API is pre-1.0. Wrap it in an `ol-text` crate.

---

## 7. Raster (subsystem 6)

| Need | Choice | Version | License | Maturity | Notes |
|---|---|---|---|---|---|
| Decode, encode, basic ops | `image` | 0.25.10 | MIT OR Apache-2.0 | 5 | |
| Adjustments (gamma, contrast, histogram eq, unsharp, blur, threshold) | `imageproc` | 0.27.0 (2026-06-02) | MIT | 4 | |
| Resampling to the line interval / DPI | `fast_image_resize` | 6.1.0 (2026-07-21) | MIT OR Apache-2.0 | 4 | SIMD (NEON on M2). |
| **Dithering** | **own implementation** | – | – | – | Floyd–Steinberg, Jarvis, Stucki, Atkinson, Sierra (2/3/lite), Bayer ordered, threshold, halftone/"newsprint", grayscale (power-modulated). Around 500 LOC with serpentine and gamma options; golden PNG tests. Crates are stale (`dither` 1.3.10, 2021) or tiny (`dithr` 0.3.0, MIT, 2k dl) and only useful as references. |
| **Vectorising / tracing** | `vtracer` + `visioncortex` | 0.6.5 (2025-10-17; 1.0.0-alpha.4 2026-08-29) / 0.9.3 | MIT OR Apache-2.0 | 3 | Rayforge uses vtracer through its Python package (`requirements.txt: vtracer==0.6.15`, PyPI versioning). 7.4k★. Pin 0.6.x until 1.0 stabilises. |
| Potrace | **Avoid** (GPL-2.0+) | – | GPL | – | Linking GPL code would force OpenLaser to GPL. A clean-room implementation from Selinger's paper is possible later (**UNVERIFIED**; needs legal review). No Rust binding exists on crates.io (`potrace` → 404). |

Galvo raster output is a bidirectional hatch of the dithered bitmap: on/off per pixel, or power and frequency modulation for MOPA grayscale, which is **UNVERIFIED** on JCZ firmware. It shares the hatch and ordering code in `ol-cam`.

---

## 8. Hardware I/O (subsystem 7)

| Need | Crate | Version (date) | License | Maturity | Notes |
|---|---|---|---|---|---|
| **USB bulk (JCZ LMC 0x9588:0x9899)** | **`nusb`** | 0.2.8 (2026-10-14) | Apache-2.0 OR MIT | 4 | Pure Rust. macOS IOKit (no permissions needed; vendor-class devices may need `set_configuration(1)`). Linux usbfs (needs a udev rule; the prototype already ships `scripts/99-openlaser.rules`). Windows **WinUSB only**. Hotplug via `watch_devices`; async and blocking APIs; `cancel_all`; `transfer_blocking` cancels on timeout ([docs](https://docs.rs/nusb/latest/nusb/)). 539★, very active. |
| USB fallback (Windows libusb0/libusbK-bound devices) | `rusb` | 0.9.4 (**2024-04-27**) | MIT | 3 (stale) | Links libusb (C). Only add it if Windows users already bound the board with Zadig's libusb-win32/libusbK driver. Whether EZCAD's own driver can coexist with any libusb backend is **UNVERIFIED**. |
| **Serial (GRBL/grblHAL/Marlin)** | **`serialport`** | 4.10.1 (2026-09-08) | MPL-2.0 (file-level copyleft; fine for MIT distribution) | 5 | Blocking with timeouts; port enumeration with USB VID/PID. Alternative: `serial2` 0.2.38 (BSD-2 OR Apache-2.0, very active). Async: `tokio-serial` 5.5.0 (MIT; what Rayforge's raydriver uses). |
| Ruida (UDP) | `std::net::UdpSocket` / `tokio::net` | tokio 1.54.1 | MIT | 5 | `ruida` crate 0.1.1 (MIT) exists, but its GitHub repo now returns 404: treat it as abandoned and use as a reference only. Rayforge's `ruidarpa` (MIT, Python) is a better reference. Port numbers and swizzle details are left to the driver research note (**UNVERIFIED** here). |
| GRBL over WiFi (FluidNC, ESP3D) | `std::net::TcpStream` (telnet) / `tokio-tungstenite` (WebSocket) | 0.30.0 (2026-07-11) | MIT | 4 | Rayforge has `grbl_telnet.py` and `grbl_network.py` as references. |
| Discovery | `mdns-sd` | 0.21.6 (2026-10-19) | Apache-2.0 OR MIT | 4 | |
| Thread messaging | `crossbeam-channel` | 0.5.17 | MIT OR Apache-2.0 | 5 | `select!` over a priority control channel and a data channel. |

**Platform notes:**
- **macOS** needs nothing beyond `nusb`, which removes the prototype's libusb bundling (`mopa_luiz/usb_backend.py` + `libusb-package`).
- **Windows** needs WinUSB bound to 0x9588:0x9899 (Zadig or libwdi), which takes the board away from EZCAD. Ship a documented, reversible "Install WinUSB driver" helper. libwdi is LGPL-3 C, so run it as a separate executable.
- **Linux** needs the udev rule. AppImages cannot install it, so add a `pkexec` helper or packaging post-install scripts. Flatpak needs `--device=all`, or a USB portal (**UNVERIFIED** maturity).

**Threads vs tokio.**
- Each machine connection gets a **dedicated OS thread** that owns the device handle. Blocking I/O with short timeouts is easier to reason about for safety than async cancellation.
- Use `tokio` only for network drivers (FluidNC WebSocket, Ruida UDP, optional LAN remote server), and only behind the same `Driver` trait.
- The UI thread never touches I/O.

**Fast-stop design** (encodes the 2026-10-19 lesson: a full list buffer plus USB write timeouts caused a reset storm; the fix was aborting the spooler wait, giving 0.18 s):
1. Stop is a **separate high-priority message** plus an `AtomicBool` checked between every packet. It is never queued behind job data.
2. The job loop never waits without a deadline: `wait_next_complete(≤ 20 ms)`, then re-check stop.
3. On stop: `ep_out.cancel_all()` → drain cancelled completions → send the LMC stop or abort list command and laser-disable with `transfer_blocking(…, 100 ms)` → report `Stopped{latency}`. The exact opcodes come from galvoplotter `consts.py` and are **UNVERIFIED** in this note.
4. GRBL: the real-time bytes `!` (feed hold) and `0x18` (soft reset) "are never placed in Grbl's RX serial buffer" ([GRBL v1.1 Interface](https://github.com/gnea/grbl/wiki/Grbl-v1.1-Interface)). Write them directly to the port from the control path. Character-counting streaming keeps ≤ 128 bytes in flight.
5. CI test: with a mock board that simulates a full buffer and write timeouts, stop latency must be under 250 ms. Re-measure on hardware later, only with the output disabled first.
6. Pre-flight before the first packet: whole-job field check, path and point limits (3000 / 20000 → refuse), and no Frame-Only or output-off geometry in burn lists. These live in `ol-cam`, are **independent of the driver**, and are property-tested.

**Key risk:** porting the LMC protocol correctly. Mitigation: a byte-for-byte golden comparison against the Python prototype + galvoplotter. Generate packet streams from both for the same jobs and compare in CI. The Python code becomes the oracle.

---

## 9. Camera (subsystem 8)

Rayforge does camera work with OpenCV: `cv2.VideoCapture`, `cv2.aruco.CharucoDetector`, `findHomography`, `calibrateCamera`, `getOptimalNewCameraMatrix`, `initUndistortRectifyMap`, `remap` and `warpPerspective` (`rayforge/camera/**`). It supports ArUco, ChArUco and dot-grid targets. Pure-Rust equivalents for each step:

| Step | Crate | Version (date) | License | Maturity | Notes |
|---|---|---|---|---|---|
| Capture | **`nokhwa`** (`input-native`: AVFoundation / MSMF / V4L2) | 0.10.11 (2026-05-15) | Apache-2.0 | 3 | 817★, 67 open issues; its optional wgpu output is pinned to wgpu 27. Fallback on macOS: `objc2-av-foundation` 0.3.2 (Zlib/Apache/MIT) directly. |
| Target detection (chessboard, **ChArUco**, ArUco/AprilTag dictionaries, PuzzleBoard) | **`calib-targets`** (+ `-charuco`, `-aruco`, `-print`) | 0.15.5 (2026-10-17; created 2025-12-28) | MIT OR Apache-2.0 | 2 | "Feature-complete. The public API is being finalised… may still change" ([README](https://github.com/VitalyVorobyev/calib-targets-rs)). 6★; single author. |
| Intrinsics and distortion (Zhang) | `vision-calibration` (calibration-rs), or own code on `nalgebra` + `levenberg-marquardt` | 0.9.0 (2026-09-29) / 0.35.0 / 0.15.0 | MIT / Apache-2.0 / MIT | 2 / 5 / 4 | vision-calibration also does Scheimpflug, rig and laser-plane calibration, with covariance ([README](https://github.com/VitalyVorobyev/calibration-rs)). Own Zhang + Brown–Conrady is about 600 LOC and fully controllable. |
| Homography / bed alignment | own 4-point or DLT solve (`nalgebra` SVD), or `dlt` 0.15.0 | – | MIT OR Apache-2.0 | 4 | |
| Undistort, remap, warp | own precomputed remap LUT; `imageproc` geometric transforms; `kornia-imgproc` 0.2.0 (Apache-2.0) | – | – | 3 | kornia's capture is V4L2-only. |
| Avoid | `camera-intrinsic-calibration` (GPL-3.0+), `lensfun` (LGPL-3.0+) | | | | Licence. |
| Heavy fallback | `opencv` (opencv-rust) | 0.101.1 (2026-10-09) | MIT (OpenCV Apache-2.0) | 4 | Needs OpenCV installed plus libclang at build time, and dylib bundling and signing per OS. Rayforge has to strip a conflicting `libiconv` bundled by cv2 (`mac_build.sh` line 285). Bundle size impact **UNVERIFIED** (likely tens of MB). |

**macOS packaging must-haves for camera:** `NSCameraUsageDescription` in Info.plist, and the hardened-runtime entitlement `com.apple.security.device.camera`. The entitlement key is from general knowledge because the Apple page did not render (**UNVERIFIED**).

**Why the camera should be native even with a web UI:** WKWebView `getUserMedia` support in Tauri on macOS is unreliable per WebKit and Tauri reports ([WebKit bug 220184](https://bugs.webkit.org/show_bug.cgi?id=220184), [tauri#2338](https://github.com/tauri-apps/tauri/issues/2338)). Calibration maths belongs in Rust anyway.

**Key risk:** young detection and calibration crates. Mitigations: implement the calibration maths ourselves (small and testable with synthetic projections), use `calib-targets` only for detection, and validate against OpenCV outputs from Rayforge-style datasets offline.

---

## 10. App infrastructure (subsystem 9)

| Need | Choice | Version (date) | License | Maturity | Notes |
|---|---|---|---|---|---|
| **Undo / redo** | Immutable document snapshots with structural sharing (`Arc<Object>` per shape; or `imbl` / `rpds` persistent maps), plus a command log for labels | imbl 7.0.2 (MPL-2.0+) / rpds 1.2.1 (MIT) | | 4 | Snapshot undo is trivially correct; memory is cheap with Arc-sharing. Command-pattern crates exist (`undo` 0.52.0, 2025-03; `undoredo` 0.15.1) but add little. |
| Document model and file format | `serde` + `serde_json` in a **zip** container (`document.json` + original image assets + thumbnail) | serde 1.0.229, serde_json 1.0.154, zip 9.0.5 (MIT) | | 5 | `"format_version"` field; migrations are `fn(serde_json::Value) -> Value`, one per version bump, with `insta` snapshot tests. JSON keeps diffs and debugging human-friendly. Optional zstd later. |
| Settings and machine profiles | `toml` + `directories` | 1.1.8 / 6.0.0 | MIT OR Apache-2.0 | 5 / 4 | Profiles as TOML files the user can share. `confy` 2.0.0 is fine but adds little. |
| Logging | `tracing` + `tracing-subscriber` + `tracing-appender` (rolling files) | 0.1.44 / 0.3.23 / 0.2.5 | MIT | 5 | Span per job and per device; ring buffer for an in-app log panel; "export diagnostics" zip. |
| i18n | `fluent-bundle` (+ `fluent-templates` for embedding) | 0.16.0 / 0.15.1 | Apache-2.0 OR MIT | 4 | Mozilla Fluent plural and gender rules. Simpler: `rust-i18n` 4.2.4 (MIT, YAML). |
| Scripting (macros, generators, material test grids) | `rhai` | 1.27.0 (2026-10-21) | MIT OR Apache-2.0 | 4 | Pure Rust, sandboxable, no C. `mlua` 0.12.2 (Luau sandbox) is the alternative; it compiles C. |
| Plugins (later) | `extism` (on `wasmtime`) | 1.41.0 / 50.0.0 | BSD-3-Clause / Apache-2.0 WITH LLVM-exception | 4 | wasmtime 50 has **MSRV 1.97** and adds significant binary size (**UNVERIFIED**, likely 10+ MB). Defer until there is demand. |
| Crash reporting | `human-panic` (local report) + `crash-handler` / `minidumper` (native crashes); optional opt-in `sentry` | 2.0.8 / 0.8.1 / 0.11.0 / 0.49.3 | MIT/Apache, MIT | 4 | Privacy-first: crash reports stay local unless the user sends them. |
| Collaboration / CRDT (not needed for v1) | `automerge` / `yrs` / `loro` | 0.12.0 / 0.28.0 / 1.17.1 | MIT | 3 | Only if multi-device editing becomes a goal. |
| Error types | `thiserror` / `anyhow` | 2.0.21 / 1.0.104 | MIT OR Apache-2.0 | 5 | |

---

## 11. Packaging and distribution (subsystem 10)

| Tool | Version (date) | License | Maturity | Produces | Notes |
|---|---|---|---|---|---|
| **`cargo-packager`** (CrabNebula) | 0.11.8 (2025-11-27) | Apache-2.0 OR MIT | 3 | .app, .dmg, .deb, AppImage, pacman, NSIS .exe, WiX .msi ([README](https://github.com/crabnebula-dev/cargo-packager)) | Has egui and Slint examples. Release cadence slowed (latest 2025-11), but the repo is active (pushed 2026-10-22). |
| `cargo-bundle` | 0.12.0 (2026-09-20) | MIT OR Apache-2.0 | 3 | .app (universal via repeated `-t`), .deb, AppImage, .msi (experimental) | Simple, revived. |
| `cargo-dist` | 0.32.0 (2026-05-22) | MIT OR Apache-2.0 | 4 | CLI-oriented installers (shell, PowerShell, MSI, Homebrew) | Not designed for GUI .app or .dmg bundles (**UNVERIFIED** detail). |
| **Velopack** (`velopack` crate + `vpk` CLI) | 1.2.161 (2026-09-29; 1.3 pre-releases) | MIT | 4 | Installers, **delta updates**, self-updating portable builds; Windows, macOS, Linux ([README](https://github.com/velopack/velopack)) | Rust SDK: `VelopackApp::build().run()` + `UpdateManager` ([Rust guide](https://docs.velopack.io/getting-started/rust)). `vpk` needs the **.NET 8 SDK** at build time only. Built-in macOS sign and notarize (`--signAppIdentity`, `--notaryProfile`) and **Azure Artifact Signing** for Windows (`--azureTrustedSignFile`) ([signing docs](https://docs.velopack.io/packaging/signing)). |
| `cargo-packager-updater` | 0.2.3 (2025-07-21) | Apache-2.0 OR MIT | 2 | Signed updates | Slow cadence. |
| `self_update` | 1.3.1 (2026-10-17) | MIT | 3 | Replaces a single binary from GitHub releases | Awkward for .app bundles and installers. |
| Tauri bundler + `tauri-plugin-updater` | 2.14.0 | Apache-2.0 OR MIT | 5 | macOS `.app.tar.gz`, Windows NSIS/MSI, Linux AppImage; **mandatory** signatures ([docs](https://v2.tauri.app/plugin/updater/)) | Only if Plan B. |
| `apple-codesign` (rcodesign) | 0.29.0 (2024-11-29) | MPL-2.0 | 3 | Sign and notarize from Linux CI | Crate stale; repo active (2026-10-17). |
| `cargo-about` / `cargo-deny` | 0.9.2 / 0.20.2 | MIT OR Apache-2.0 | 5 | Third-party notices / licence and advisory gate | |

**Recommended pipeline:**
- **macOS (M1):** `cargo build --release` for `aarch64-apple-darwin` (plus `x86_64` via `lipo` if Intel support is wanted) → `cargo-packager` .app + .dmg → `codesign --options runtime` with Developer ID (Apple Developer Program, $99/yr) → `xcrun notarytool submit --wait` → staple.
  - Entitlements: camera. USB needs no entitlement outside the App Sandbox (**UNVERIFIED** per Apple docs).
  - Do not use the Mac App Store: the sandbox complicates USB and serial.
- **Windows (M3):** NSIS via cargo-packager, or Velopack Setup, signed with Azure Artifact Signing. Add the WinUSB helper.
- **Linux (M3):** AppImage + .deb with a udev-rule post-install; Flatpak later.
- **Updates:** M1 starts with an "update available" check against GitHub Releases. Adopt **Velopack** when distribution widens (delta updates, signing integration).

**Binary size expectations (UNVERIFIED, no toolchain to measure):**

| Build | Stripped release size |
|---|---|
| egui + wgpu app, before features | ~10–20 MB |
| + usvg/hayro/image/fonts | +5–10 MB |
| + wasmtime | +10–20 MB |
| + OpenCV | +30–80 MB (avoid) |
| Tauri app | ~5–15 MB plus frontend assets |

Use `strip = "symbols"`, `lto = "thin"` and `codegen-units = 1` for releases.

---

## 12. Testing (subsystem 11)

| Need | Crate / approach | Version | License | Maturity |
|---|---|---|---|---|
| Snapshot tests (G-code text, LMC packet hex dumps, SVG/DXF exports, normalised imports, file-format migrations) | **`insta`** | 1.49.0 (2026-10-03) | Apache-2.0 | 5 |
| Property tests (geometry invariants, safety limits, field checks, ordering covers all paths exactly once) | **`proptest`** | 1.12.0 (2026-10-10) | MIT OR Apache-2.0 | 5 |
| **Cross-implementation goldens** | Python prototype + galvoplotter generate reference LMC packet streams and G-code for a fixture set; Rust output must match byte-for-byte (or a documented diff) | – | – | – |
| UI tests | **`egui_kittest`** (AccessKit queries, `Harness`, snapshots with the `wgpu` feature; per-OS tolerances in `kittest.toml`; `UPDATE_SNAPSHOTS=true`) ([docs](https://docs.rs/egui_kittest/latest/egui_kittest/)) | 0.37.0 | MIT OR Apache-2.0 | 4 |
| GPU-less render goldens | Render the scene through **tiny-skia** (deterministic) and compare with `image-compare` 0.5.0 (MIT). Avoid `dssim-core` (**AGPL-3.0**). | | | 4 |
| Fuzzing parsers | `cargo-fuzz` + `arbitrary` (or `bolero`) | 0.13.2 / 1.5.0 / 0.13.7 | MIT/Apache | 4 |
| Test runner, perf | `cargo-nextest` 0.9.149; `criterion` 0.8.2 or `divan` 0.1.21 for budgets (e.g. job compile of 20k points < 100 ms) | | | 5 / 4 |
| Hardware-free driver tests | Mock LMC board (port galvoplotter's `mock_connection.py` idea): simulates list-buffer fill, timeouts and stalls; asserts stop latency and packet sequences | – | – | – |
| Licence / advisory gate | `cargo-deny` (deny GPL/AGPL/LGPL-static; allow MPL-2.0 file-level) | 0.20.2 | MIT OR Apache-2.0 | 5 |

---

## 13. Recommended workspace layout ("Rust core + thin UI")

```
openlaser/                      (Rust 1.99 stable, edition 2024, rust-toolchain.toml pinned)
  crates/
    ol-geom      kurbo, i_overlay, cavalier_contours, rstar, kiddo         (no I/O, no UI)
    ol-doc       document model, serde format + migrations, snapshot undo
    ol-text      parley, fontique, skrifa, hershey/SVG-font support
    ol-raster    image, imageproc, fast_image_resize, own dithering, vtracer
    ol-io        usvg, dxf, hayro, quick-xml (.lbrn2), HPGL, G-code import; SVG/DXF/G-code export
    ol-cam       layers → toolpaths: hatch, ordering, kerf, tabs, field mapping, SAFETY CHECKS
    ol-machine   Driver trait; jcz_lmc (nusb), grbl (serialport / tcp / ws), ruida (udp), mock
    ol-camera    nokhwa capture, calib-targets detection, calibration maths, remap
    ol-render    scene → wgpu pipelines | tiny-skia / vello_cpu (export + tests)
    ol-app       AppState, Command/Event enums, view-models (NO egui dependency)
    ol-ui-egui   eframe shell, docking, canvas widget, muda menus, rfd dialogs
    ol-cli       headless: import → plan → dry-run → export / packet dump (CI + scripting)
    ol-remote    (optional) axum LAN server reusing the prototype web UI for machine control
```

**Threading:**
- UI thread: egui.
- Worker pool (rayon 1.12): planning, tracing, rendering of exports.
- One OS thread per connected machine.
- Optional tokio runtime: network drivers and LAN remote.

All talk through `crossbeam-channel`. Stop is a priority path end to end.

---

## 14. Recommended stack: one-page table

| # | Subsystem | Crate(s) | Version | License | Maturity (1–5) | Key risk |
|---|---|---|---|---|---|---|
| 1 | GUI | eframe/egui, egui_dock (or egui_tiles), muda, rfd, accesskit | 0.37.0, 0.21.1 (0.18.0), 0.21.2, 0.17.2, 0.25.1 | MIT/Apache, MIT | 4 | Frequent breaking releases; non-native look; muda+eframe integration unproven |
| 2 | Rendering | wgpu (custom polyline pipeline), lyon_tessellation, tiny-skia, vello_cpu | 30.0.1, 1.0.23, 0.12.0, 0.3.0 | MIT/Apache, BSD-3 | 4 | wgpu major-version lockstep with egui |
| 3 | Geometry | kurbo, i_overlay, cavalier_contours, rstar, kiddo (+ own hatch and ordering) | 0.13.1, 9.1.0, 0.9.0, 0.13.0, 6.3.0 | MIT/Apache | 4 | Robustness on messy input; single-maintainer crates |
| 4 | Import/export | usvg, dxf, hayro-interpret, quick-xml, image, gcode (+ own HPGL and G-code emitter) | 0.48.1, 0.6.2, 0.8.0, 0.42.0, 0.25.10, 0.7.0 | MIT/Apache, MIT | 3 | PDF/AI fidelity (hayro is experimental); undocumented .lbrn2 |
| 5 | Text | parley, fontique, skrifa, harfrust | 0.12.0, 0.12.0, 0.49.0, 0.15.0 | Apache/MIT, MIT | 3 | Pre-1.0 APIs; single-line font licensing |
| 6 | Raster | image, imageproc, fast_image_resize, vtracer (+ own dithering) | 0.25.10, 0.27.0, 6.1.0, 0.6.5 | MIT/Apache, MIT | 4 | vtracer 1.0 transition; no permissive potrace equivalent |
| 7 | Hardware | nusb, serialport, crossbeam-channel, tokio (+ tokio-tungstenite, mdns-sd) | 0.2.8, 4.10.1, 0.5.17, 1.54.1 | Apache/MIT, MPL-2.0, MIT | 4 | LMC protocol port correctness; Windows WinUSB vs EZCAD |
| 8 | Camera | nokhwa, calib-targets, nalgebra, levenberg-marquardt (vision-calibration optional) | 0.10.11, 0.15.5, 0.35.0, 0.15.0 | Apache-2.0, MIT/Apache | 2–3 | Young detection crates; macOS camera entitlements |
| 9 | Infra | serde/serde_json/zip, toml/directories, tracing, fluent, rhai, human-panic/crash-handler | see §10 | MIT/Apache | 5 | File-format migrations discipline |
| 10 | Packaging | cargo-packager (or cargo-bundle), codesign/notarytool, Velopack later | 0.11.8, 1.2.161 | MIT/Apache, MIT | 3–4 | Signing and notarisation setup; Windows driver install UX |
| 11 | Testing | insta, proptest, egui_kittest, cargo-nextest, cargo-deny, cargo-fuzz, image-compare | 1.49.0, 1.12.0, 0.37.0, 0.9.149, 0.20.2, 0.13.2, 0.5.0 | MIT/Apache | 5 | Cross-GPU snapshot flakiness (use tiny-skia goldens) |

---

## 15. The five biggest technical risks and mitigations

1. **Canvas performance and UX in the chosen GUI** (egui immediate mode, 20k+ paths, node editing, native expectations on macOS).
   - Two-week spike with explicit go/no-go metrics (§2.4).
   - Cached GPU geometry with level of detail; `rstar` hit-testing.
   - Keep the UI thin (`ol-app` has no egui dependency) so the Tauri Plan B costs weeks, not months.
   - Re-score GPUI Kit at M2.
2. **JCZ LMC driver correctness and fast-stop safety in Rust** (a new port; timing-sensitive; a full list buffer and timeouts caused a reset storm in Python).
   - Byte-for-byte golden tests against the Python prototype and galvoplotter.
   - Mock board that simulates buffer-full, stall and timeout.
   - Dedicated device thread with deadline-bounded waits and `nusb::Endpoint::cancel_all`.
   - Stop-latency test under 250 ms in CI.
   - Driver-independent pre-flight safety checks in `ol-cam` (field limits, refuse rather than clamp, path and point caps, no output-off geometry burned).
   - First hardware runs with the output disabled.
3. **Geometry robustness** (booleans, offsets and hatching on messy imported art; float precision).
   - `i_overlay` (integer-snapped robust core) plus `cavalier_contours`.
   - A normalisation pass on import.
   - `proptest` invariants and a `clipper2-rust` cross-check oracle.
   - Fail closed (refuse with a reason) instead of emitting questionable toolpaths.
4. **Import fidelity**: SVG units and DPI, PDF/AI via experimental `hayro`, DXF dialects, undocumented `.lbrn2`, EPS.
   - A real-world file corpus with `insta` snapshots.
   - `cargo-fuzz` on parsers.
   - Optional `pdfium-render` fallback feature.
   - Rayforge's MIT LightBurn importer as the format reference.
   - Visible import reports.
5. **Ecosystem churn and version lockstep** (egui ↔ wgpu ↔ winit ↔ accesskit; pre-1.0 core crates kurbo 0.13, usvg 0.48, nusb 0.2, hayro 0.8, parley 0.12; single-maintainer crates such as i_overlay, cavalier_contours, dxf, calib-targets).
   - Pin exact versions in `[workspace.dependencies]`.
   - Quarterly upgrade window; Renovate or Dependabot.
   - Wrap every third-party API behind an `ol-*` crate boundary.
   - `cargo-deny` for licences and advisories.
   - Willingness to vendor or fork small critical crates (as raygeo shows is feasible).

Honourable mentions, just outside the top five:
- **Windows driver binding UX** (WinUSB via Zadig/libwdi breaks EZCAD).
- **Licence contamination**: avoid potrace (GPL), dssim-core (AGPL), PyMuPDF-style AGPL deps (Rayforge's PDF import uses PyMuPDF, AGPL-3.0/commercial per PyPI), camera-intrinsic-calibration (GPL) and lensfun (LGPL).
- **Brand collision** on the name "OpenLaser".

---

## 16. Licence hygiene: suggested `cargo-deny` policy

- **Allow:** MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2/3-Clause, ISC, Zlib, BSL-1.0, CC0-1.0, Unicode-3.0, MPL-2.0. MPL is file-level copyleft and fine to link: `serialport`, `jagua-rs`, `imbl`, `apple-codesign`.
- **Deny:** GPL-*, AGPL-*, and LGPL-* when statically linked. Examples found in this survey: `camera-intrinsic-calibration` (GPL-3.0+), `travelling_salesman` (GPL-3.0), `dssim-core` (AGPL-3.0), `lensfun` (LGPL-3.0+), `hershey` (dual Apache-2.0 OR LGPL-3.0 is fine because Apache can be chosen).
- **Special:** Slint's licence options (GPL-3.0 / royalty-free / commercial) if Slint is ever chosen; `vector-text-hershey` ("non-standard").
- **Tools, not linked:** `cargo-appimage` is GPL-3.0, which is fine because it is a build tool.

---

## 17. Open questions and UNVERIFIED items

1. **Name:** keep "OpenLaser" despite `clayfrocker97/OpenLaser` (Rust, GPL) and `openlaser.app` (MIT, web)? Reserve crates.io names now if keeping it.
2. **Reuse raygeo?** Rayforge's MIT Rust geometry core could save months (ops pipeline, tracing glue, text). It is not on crates.io, has a single maintainer, and pulls `clipper2` C++ FFI. Depend on it via git, fork it, or only study it?
3. **GUI choice confirmation:** do the spike (§2.4) before committing. Does the user value a native macOS look or reuse of the prototype web UI more than an all-Rust editor? That flips egui vs Tauri.
4. **Windows driver strategy:** what driver does EZCAD install for the LMC board, and can a libusb backend (`rusb`) coexist with it? (**UNVERIFIED**.) If not, ship a reversible WinUSB installer and document switching back.
5. **MOPA grayscale on JCZ:** per-pixel power or frequency modulation in list mode needs protocol confirmation from galvoplotter/balor sources (**UNVERIFIED**).
6. **eframe + muda native menu bar** on macOS 26 (**UNVERIFIED**; spike item).
7. **WebGPU in WKWebView (macOS 26) and WebKitGTK** (only matters for Plan B; **UNVERIFIED**).
8. **Hershey and single-line font licensing** for bundled data (**UNVERIFIED**).
9. **Binary sizes and performance numbers** in this document are estimates; install `rustup` (absent on this Mac) and measure in the spike.
10. **Hardened-runtime entitlement keys** (camera) are from general knowledge; the Apple page was not machine-readable (**UNVERIFIED**).
11. **Ruida protocol details** (ports, swizzle) were deliberately left to the driver-research note.

---

## Appendix A: crates.io snapshot (queried 2026-10-22/23)

Downloads are all-time / last 90 days. "Newest" includes pre-releases. Note: the crate `sparrow` here is **unrelated** to JeroenGar/sparrow (git-only). `rfd`'s newest entry is a 0.16.x backport published after 0.17.2.

| Crate | Latest stable | Released | Newest (incl. pre) | License (latest stable) | MSRV | Downloads (all / 90d) |
|---|---|---|---|---|---|---|
| [accesskit](https://crates.io/crates/accesskit) | 0.25.1 | 2026-09-25 | same (2026-09-25) | MIT OR Apache-2.0 | 1.87 | 30.5M / 8.7M |
| [accesskit_winit](https://crates.io/crates/accesskit_winit) | 0.34.1 | 2026-09-25 | same (2026-09-25) | Apache-2.0 | 1.87 | 21.9M / 6.2M |
| [anyhow](https://crates.io/crates/anyhow) | 1.0.104 | 2026-07-18 | same (2026-07-18) | MIT OR Apache-2.0 | 1.68 | 1079.6M / 254.0M |
| [anyrender](https://crates.io/crates/anyrender) | 0.14.0 | 2026-10-03 | same (2026-10-03) | MIT OR Apache-2.0 | - | 216k / 147k |
| [aoer-plotty-rs](https://crates.io/crates/aoer-plotty-rs) | 0.4.1 | 2025-09-19 | same (2025-09-19) | MIT | - | 23k / 60 |
| [apple-codesign](https://crates.io/crates/apple-codesign) | 0.29.0 | 2024-11-29 | same (2024-11-29) | MPL-2.0 | 1.81 | 1.1M / 239k |
| [aprilgrid](https://crates.io/crates/aprilgrid) | 0.8.0 | 2025-12-30 | same (2025-12-30) | MIT OR Apache-2.0 | - | 64k / 23k |
| [apriltag](https://crates.io/crates/apriltag) | 0.4.0 | 2023-01-28 | same (2023-01-28) | BSD-2-Clause | - | 4.5M / 121k |
| [arbitrary](https://crates.io/crates/arbitrary) | 1.5.0 | 2026-10-06 | same (2026-10-06) | MIT OR Apache-2.0 | 1.71.0 | 183.3M / 49.5M |
| [arboard](https://crates.io/crates/arboard) | 3.7.0 | 2026-10-15 | same (2026-10-15) | MIT OR Apache-2.0 | 1.71.0 | 57.5M / 19.9M |
| [argmin](https://crates.io/crates/argmin) | 0.11.0 | 2025-09-28 | same (2025-09-28) | MIT OR Apache-2.0 | - | 5.5M / 1.7M |
| [arrsac](https://crates.io/crates/arrsac) | 0.11.0 | 2025-08-03 | same (2025-08-03) | MIT | - | 88k / 8k |
| [aruco-rs](https://crates.io/crates/aruco-rs) | 0.1.0 | 2026-02-27 | same (2026-02-27) | MIT | - | 1k / 1k |
| [ashpd](https://crates.io/crates/ashpd) | 0.13.13 | 2026-07-17 | same (2026-07-17) | MIT | 1.87 | 18.5M / 7.0M |
| [async-io](https://crates.io/crates/async-io) | 2.6.0 | 2025-09-14 | same (2025-09-14) | Apache-2.0 OR MIT | 1.71 | 243.2M / 49.4M |
| [automerge](https://crates.io/crates/automerge) | 0.12.0 | 2026-09-16 | same (2026-09-16) | MIT | 1.90.0 | 750k / 308k |
| [av-foundation](https://crates.io/crates/av-foundation) | 0.8.1 | 2026-06-09 | same (2026-06-09) | MIT OR Apache-2.0 | - | 64k / 32k |
| [axum](https://crates.io/crates/axum) | 0.8.9 | 2026-04-14 | same (2026-04-14) | MIT | 1.80 | 538.3M / 139.4M |
| [bincode](https://crates.io/crates/bincode) | 3.0.0 | 2025-12-16 | same (2025-12-16) | MIT | 1.85.0 | 352.6M / 72.2M |
| [blitz-dom](https://crates.io/crates/blitz-dom) | 0.2.4 | 2025-10-22 | 0.3.0-beta.4 (2026-10-16) | MIT OR Apache-2.0 | 1.86.0 | 296k / 74k |
| [bolero](https://crates.io/crates/bolero) | 0.13.7 | 2026-10-03 | same (2026-10-03) | MIT | 1.68.0 | 6.1M / 1.4M |
| [calib-targets](https://crates.io/crates/calib-targets) | 0.15.5 | 2026-10-17 | same (2026-10-17) | MIT OR Apache-2.0 | 1.91 | 4k / 2k |
| [calib-targets-charuco](https://crates.io/crates/calib-targets-charuco) | 0.15.5 | 2026-10-17 | same (2026-10-17) | MIT OR Apache-2.0 | 1.91 | 4k / 2k |
| [camera-capture](https://crates.io/crates/camera-capture) | 0.5.0 | 2017-11-20 | same (2017-11-20) | MIT | - | 24k / 510 |
| [camera-intrinsic-calibration](https://crates.io/crates/camera-intrinsic-calibration) | 0.12.0 | 2026-10-15 | same (2026-10-15) | GPL-3.0-or-later | - | 21k / 456 |
| [cargo-about](https://crates.io/crates/cargo-about) | 0.9.2 | 2026-08-18 | same (2026-08-18) | MIT OR Apache-2.0 | 1.88.0 | 1.3M / 307k |
| [cargo-appimage](https://crates.io/crates/cargo-appimage) | 2.4.0 | 2025-11-24 | same (2025-11-24) | GPL-3.0 | - | 31k / 2k |
| [cargo-bundle](https://crates.io/crates/cargo-bundle) | 0.12.0 | 2026-09-20 | same (2026-09-20) | MIT OR Apache-2.0 | - | 181k / 23k |
| [cargo-deny](https://crates.io/crates/cargo-deny) | 0.20.2 | 2026-07-09 | same (2026-07-09) | MIT OR Apache-2.0 | 1.88.0 | 6.7M / 2.0M |
| [cargo-dist](https://crates.io/crates/cargo-dist) | 0.32.0 | 2026-05-22 | same (2026-05-22) | MIT OR Apache-2.0 | 1.74 | 186k / 20k |
| [cargo-fuzz](https://crates.io/crates/cargo-fuzz) | 0.13.2 | 2026-06-09 | same (2026-06-09) | MIT OR Apache-2.0 | - | 5.6M / 1.8M |
| [cargo-nextest](https://crates.io/crates/cargo-nextest) | 0.9.149 | 2026-10-20 | same (2026-10-20) | Apache-2.0 OR MIT | 1.91 | 13.4M / 1.8M |
| [cargo-packager](https://crates.io/crates/cargo-packager) | 0.11.8 | 2025-11-27 | same (2025-11-27) | Apache-2.0 OR MIT | - | 222k / 72k |
| [cargo-packager-updater](https://crates.io/crates/cargo-packager-updater) | 0.2.3 | 2025-07-21 | same (2025-07-21) | Apache-2.0 OR MIT | - | 58k / 28k |
| [cargo-wix](https://crates.io/crates/cargo-wix) | 0.3.9 | 2025-03-13 | same (2025-03-13) | MIT/Apache-2.0 | 1.70.0 | 589k / 189k |
| [cavalier_contours](https://crates.io/crates/cavalier_contours) | 0.9.0 | 2026-08-20 | same (2026-08-20) | MIT OR Apache-2.0 | 1.88 | 92k / 58k |
| [clipper2](https://crates.io/crates/clipper2) | 0.6.0 | 2026-05-06 | same (2026-05-06) | MIT OR Apache-2.0 | 1.85 | 437k / 104k |
| [clipper2-rust](https://crates.io/crates/clipper2-rust) | 1.2.0 | 2026-09-18 | same (2026-09-18) | BSL-1.0 | 1.70 | 152k / 135k |
| [clipper2c-sys](https://crates.io/crates/clipper2c-sys) | 0.2.0 | 2026-05-05 | same (2026-05-05) | MIT OR Apache-2.0 | 1.85 | 408k / 82k |
| [confy](https://crates.io/crates/confy) | 2.0.0 | 2025-10-27 | same (2025-10-27) | MIT/X11 OR Apache-2.0 | - | 2.9M / 199k |
| [core-foundation](https://crates.io/crates/core-foundation) | 0.10.1 | 2025-05-26 | same (2025-05-26) | MIT OR Apache-2.0 | 1.65 | 508.1M / 136.7M |
| [cosmic-text](https://crates.io/crates/cosmic-text) | 0.19.0 | 2026-04-22 | same (2026-04-22) | MIT OR Apache-2.0 | 1.89 | 10.8M / 4.3M |
| [crash-handler](https://crates.io/crates/crash-handler) | 0.8.1 | 2026-09-25 | same (2026-09-25) | MIT OR Apache-2.0 | - | 3.9M / 1.8M |
| [criterion](https://crates.io/crates/criterion) | 0.8.2 | 2026-02-04 | same (2026-02-04) | Apache-2.0 OR MIT | 1.86 | 315.3M / 69.5M |
| [crossbeam-channel](https://crates.io/crates/crossbeam-channel) | 0.5.17 | 2026-09-05 | same (2026-09-05) | MIT OR Apache-2.0 | 1.60 | 657.8M / 137.3M |
| [cv-core](https://crates.io/crates/cv-core) | 0.15.0 | 2020-06-23 | same (2020-06-23) | MIT | - | 57k / 2k |
| [dioxus](https://crates.io/crates/dioxus) | 0.7.10 | 2026-07-30 | 0.8.0-alpha.3 (2026-10-16) | MIT OR Apache-2.0 | 1.83.0 | 3.3M / 1.2M |
| [dioxus-native](https://crates.io/crates/dioxus-native) | 0.7.10 | 2026-07-30 | 0.8.0-alpha.3 (2026-10-16) | MIT OR Apache-2.0 | - | 45k / 24k |
| [directories](https://crates.io/crates/directories) | 6.0.0 | 2025-01-12 | same (2025-01-12) | MIT OR Apache-2.0 | - | 77.3M / 14.8M |
| [dither](https://crates.io/crates/dither) | 1.3.10 | 2021-05-05 | same (2021-05-05) | MIT | - | 47k / 384 |
| [dithr](https://crates.io/crates/dithr) | 0.3.0 | 2026-04-16 | same (2026-04-16) | MIT | 1.75 | 2k / 1k |
| [divan](https://crates.io/crates/divan) | 0.1.21 | 2025-04-10 | same (2025-04-10) | MIT OR Apache-2.0 | 1.80.0 | 10.0M / 4.3M |
| [dlt](https://crates.io/crates/dlt) | 0.15.0 | 2026-08-07 | same (2026-08-07) | MIT OR Apache-2.0 | 1.89 | 74k / 2k |
| [dssim-core](https://crates.io/crates/dssim-core) | 3.5.1 | 2026-08-29 | same (2026-08-29) | AGPL-3.0 | - | 567k / 90k |
| [dxf](https://crates.io/crates/dxf) | 0.6.2 | 2026-10-03 | same (2026-10-03) | MIT | - | 321k / 125k |
| [earcut](https://crates.io/crates/earcut) | 0.4.11 | 2026-07-26 | same (2026-07-26) | MIT OR Apache-2.0 | - | 2.2M / 1.1M |
| [eframe](https://crates.io/crates/eframe) | 0.37.0 | 2026-10-15 | same (2026-10-15) | MIT OR Apache-2.0 | 1.95 | 21.7M / 6.1M |
| [egui](https://crates.io/crates/egui) | 0.37.0 | 2026-10-15 | same (2026-10-15) | MIT OR Apache-2.0 | 1.95 | 27.8M / 7.4M |
| [egui-notify](https://crates.io/crates/egui-notify) | 0.23.0 | 2026-09-04 | same (2026-09-04) | MIT | - | 332k / 46k |
| [egui-snarl](https://crates.io/crates/egui-snarl) | 0.12.0 | 2026-08-26 | same (2026-08-26) | MIT OR Apache-2.0 | 1.92 | 62k / 18k |
| [egui-wgpu](https://crates.io/crates/egui-wgpu) | 0.37.0 | 2026-10-15 | same (2026-10-15) | MIT OR Apache-2.0 | 1.95 | 14.9M / 5.6M |
| [egui_dock](https://crates.io/crates/egui_dock) | 0.21.1 | 2026-08-06 | same (2026-08-06) | MIT | 1.95 | 4.8M / 295k |
| [egui_extras](https://crates.io/crates/egui_extras) | 0.37.0 | 2026-10-15 | same (2026-10-15) | MIT OR Apache-2.0 | 1.95 | 12.7M / 2.6M |
| [egui_glow](https://crates.io/crates/egui_glow) | 0.37.0 | 2026-10-15 | same (2026-10-15) | MIT OR Apache-2.0 | 1.95 | 22.0M / 6.2M |
| [egui_kittest](https://crates.io/crates/egui_kittest) | 0.37.0 | 2026-10-15 | same (2026-10-15) | MIT OR Apache-2.0 | 1.95 | 1.4M / 892k |
| [egui_system_font_provider](https://crates.io/crates/egui_system_font_provider) | 0.37.0 | 2026-10-15 | same (2026-10-15) | MIT OR Apache-2.0 | 1.95 | 35k / 35k |
| [egui_tiles](https://crates.io/crates/egui_tiles) | 0.18.0 | 2026-10-19 | same (2026-10-19) | MIT OR Apache-2.0 | 1.95.0 | 2.5M / 643k |
| [epaint](https://crates.io/crates/epaint) | 0.37.0 | 2026-10-15 | same (2026-10-15) | MIT OR Apache-2.0 | 1.95 | 27.7M / 7.5M |
| [extism](https://crates.io/crates/extism) | 1.41.0 | 2026-10-09 | same (2026-10-09) | BSD-3-Clause | - | 793k / 212k |
| [extism-pdk](https://crates.io/crates/extism-pdk) | 1.4.1 | 2025-05-19 | same (2025-05-19) | BSD-3-Clause | - | 488k / 123k |
| [eye](https://crates.io/crates/eye) | 0.5.0 | 2023-10-28 | same (2023-10-28) | MIT | - | 12k / 248 |
| [fast_image_resize](https://crates.io/crates/fast_image_resize) | 6.1.0 | 2026-07-21 | same (2026-07-21) | MIT OR Apache-2.0 | 1.87.0 | 22.3M / 5.9M |
| [femtovg](https://crates.io/crates/femtovg) | 0.27.0 | 2026-08-31 | same (2026-08-31) | MIT OR Apache-2.0 | 1.88 | 3.0M / 790k |
| [flo_curves](https://crates.io/crates/flo_curves) | 0.8.1 | 2026-08-25 | same (2026-08-25) | Apache-2.0 | - | 373k / 130k |
| [floem](https://crates.io/crates/floem) | 0.2.0 | 2024-11-14 | same (2024-11-14) | MIT | 1.80 | 44k / 18k |
| [fluent](https://crates.io/crates/fluent) | 0.17.0 | 2025-05-23 | same (2025-05-23) | Apache-2.0 OR MIT | 1.67.0 | 16.3M / 6.6M |
| [fluent-bundle](https://crates.io/crates/fluent-bundle) | 0.16.0 | 2025-05-22 | same (2025-05-22) | Apache-2.0 OR MIT | 1.67.0 | 21.9M / 7.8M |
| [fluent-templates](https://crates.io/crates/fluent-templates) | 0.15.1 | 2026-08-05 | same (2026-08-05) | MIT/Apache-2.0 | 1.70.0 | 1.3M / 238k |
| [flume](https://crates.io/crates/flume) | 0.12.0 | 2025-12-08 | same (2025-12-08) | Apache-2.0/MIT | - | 265.8M / 69.0M |
| [fontdb](https://crates.io/crates/fontdb) | 0.24.0 | 2026-07-29 | same (2026-07-29) | MIT | 1.71 | 40.2M / 13.8M |
| [fontique](https://crates.io/crates/fontique) | 0.12.0 | 2026-10-09 | same (2026-10-09) | Apache-2.0 OR MIT | 1.88 | 4.7M / 2.7M |
| [freya](https://crates.io/crates/freya) | 0.4.3 | 2026-08-30 | 0.5.0-beta.12 (2026-10-18) | MIT | - | 47k / 11k |
| [g-code](https://crates.io/crates/g-code) | 0.6.0 | 2026-03-04 | same (2026-03-04) | MIT OR Apache-2.0 | - | 43k / 4k |
| [gcode](https://crates.io/crates/gcode) | 0.7.0 | 2026-04-02 | same (2026-04-02) | MIT OR Apache-2.0 | 1.85 | 30k / 2k |
| [gcode-nom](https://crates.io/crates/gcode-nom) | 0.7.0 | 2026-04-11 | same (2026-04-11) | MIT | 1.94 | 7k / 125 |
| [geo](https://crates.io/crates/geo) | 0.33.1 | 2026-04-20 | same (2026-04-20) | MIT OR Apache-2.0 | 1.88 | 24.6M / 4.9M |
| [geo-clipper](https://crates.io/crates/geo-clipper) | 0.9.0 | 2025-02-08 | same (2025-02-08) | ISC | - | 1.0M / 343k |
| [geo-types](https://crates.io/crates/geo-types) | 0.7.20 | 2026-08-02 | same (2026-08-02) | MIT OR Apache-2.0 | 1.75 | 30.0M / 6.6M |
| [glam](https://crates.io/crates/glam) | 0.34.1 | 2026-10-06 | same (2026-10-06) | MIT OR Apache-2.0 | 1.85.0 | 168.4M / 56.7M |
| [glifo](https://crates.io/crates/glifo) | 0.4.0 | 2026-10-02 | same (2026-10-02) | Apache-2.0 OR MIT | 1.89 | 5.0M / 4.7M |
| [global-hotkey](https://crates.io/crates/global-hotkey) | 0.9.0 | 2026-10-16 | same (2026-10-16) | Apache-2.0 OR MIT | 1.77 | 6.7M / 2.8M |
| [glow](https://crates.io/crates/glow) | 0.18.0 | 2026-07-09 | same (2026-07-09) | MIT OR Apache-2.0 OR Zlib | - | 50.4M / 16.9M |
| [gpui](https://crates.io/crates/gpui) | 0.2.2 | 2025-10-22 | same (2025-10-22) | Apache-2.0 | - | 388k / 219k |
| [gpui-base](https://crates.io/crates/gpui-base) | 0.8.0 | 2026-10-19 | same (2026-10-19) | Apache-2.0 | - | 175k / 175k |
| [gpui-component](https://crates.io/crates/gpui-component) | 0.8.0 | 2026-10-19 | same (2026-10-19) | Apache-2.0 | - | 318k / 245k |
| [gpui-kit](https://crates.io/crates/gpui-kit) | 0.8.0 | 2026-10-19 | same (2026-10-19) | Apache-2.0 | - | 154k / 154k |
| [gpui-pre](https://crates.io/crates/gpui-pre) | 0.3.9 | 2026-10-12 | same (2026-10-12) | Apache-2.0 | - | 235k / 235k |
| [gtk4](https://crates.io/crates/gtk4) | 0.11.5 | 2026-09-20 | same (2026-09-20) | MIT | 1.92 | 5.1M / 1.7M |
| [harfrust](https://crates.io/crates/harfrust) | 0.15.0 | 2026-10-22 | same (2026-10-22) | MIT | 1.85 | 14.2M / 9.6M |
| [hayro](https://crates.io/crates/hayro) | 0.8.0 | 2026-10-04 | same (2026-10-04) | Apache-2.0 OR MIT | 1.92 | 3.9M / 2.7M |
| [hayro-interpret](https://crates.io/crates/hayro-interpret) | 0.8.0 | 2026-10-04 | same (2026-10-04) | Apache-2.0 OR MIT | 1.92 | 5.0M / 3.7M |
| [hayro-postscript](https://crates.io/crates/hayro-postscript) | 0.1.0 | 2026-04-12 | same (2026-04-12) | Apache-2.0 OR MIT | - | 3.4M / 3.2M |
| [hayro-svg](https://crates.io/crates/hayro-svg) | 0.8.0 | 2026-10-04 | same (2026-10-04) | Apache-2.0 OR MIT | 1.92 | 2.6M / 1.5M |
| [hayro-syntax](https://crates.io/crates/hayro-syntax) | 0.8.0 | 2026-10-04 | same (2026-10-04) | Apache-2.0 OR MIT | 1.92 | 5.1M / 3.7M |
| [hershey](https://crates.io/crates/hershey) | 0.1.2 | 2025-03-26 | same (2025-03-26) | Apache-2.0 OR LGPL-3.0 | - | 14k / 2k |
| [hidapi](https://crates.io/crates/hidapi) | 2.6.7 | 2026-08-27 | same (2026-08-27) | MIT | - | 9.6M / 1.9M |
| [human-panic](https://crates.io/crates/human-panic) | 2.0.8 | 2026-04-02 | same (2026-04-02) | MIT OR Apache-2.0 | 1.88 | 14.4M / 2.7M |
| [i18n-embed](https://crates.io/crates/i18n-embed) | 0.16.0 | 2025-07-09 | same (2025-07-09) | MIT | - | 8.4M / 4.2M |
| [i_curve](https://crates.io/crates/i_curve) | 0.2.0 | 2026-09-19 | same (2026-09-19) | MIT | 1.88 | 316 / 316 |
| [i_float](https://crates.io/crates/i_float) | 5.0.0 | 2026-09-19 | same (2026-09-19) | MIT | 1.85 | 10.1M / 4.4M |
| [i_overlay](https://crates.io/crates/i_overlay) | 9.1.0 | 2026-10-14 | same (2026-10-14) | MIT OR Apache-2.0 | 1.88 | 10.4M / 4.5M |
| [i_triangle](https://crates.io/crates/i_triangle) | 0.50.1 | 2026-10-14 | same (2026-10-14) | MIT OR Apache-2.0 | - | 89k / 22k |
| [ibn_battuta](https://crates.io/crates/ibn_battuta) | 0.2.1 | 2026-03-30 | same (2026-03-30) | MIT | - | 3k / 12 |
| [iced](https://crates.io/crates/iced) | 0.14.0 | 2025-12-07 | same (2025-12-07) | MIT | 1.88 | 3.1M / 693k |
| [iced_aw](https://crates.io/crates/iced_aw) | 0.14.1 | 2026-04-27 | same (2026-04-27) | MIT | 1.88 | 569k / 100k |
| [iced_widget](https://crates.io/crates/iced_widget) | 0.14.2 | 2025-12-11 | same (2025-12-11) | MIT | - | 2.5M / 657k |
| [im](https://crates.io/crates/im) | 15.1.0 | 2022-04-29 | same (2022-04-29) | MPL-2.0+ | 1.46.0 | 34.0M / 4.7M |
| [image](https://crates.io/crates/image) | 0.25.10 | 2026-03-10 | same (2026-03-10) | MIT OR Apache-2.0 | 1.88.0 | 223.3M / 66.3M |
| [image-compare](https://crates.io/crates/image-compare) | 0.5.0 | 2025-08-18 | same (2025-08-18) | MIT | - | 2.1M / 655k |
| [imageproc](https://crates.io/crates/imageproc) | 0.27.0 | 2026-06-02 | same (2026-06-02) | MIT | 1.87.0 | 14.3M / 2.9M |
| [imbl](https://crates.io/crates/imbl) | 7.0.2 | 2026-09-09 | same (2026-09-09) | MPL-2.0+ | 1.85 | 8.7M / 4.9M |
| [imgref](https://crates.io/crates/imgref) | 1.12.3 | 2026-08-26 | same (2026-08-26) | CC0-1.0 OR Apache-2.0 | 1.63 | 60.7M / 19.1M |
| [insta](https://crates.io/crates/insta) | 1.49.0 | 2026-10-03 | same (2026-10-03) | Apache-2.0 | 1.66.0 | 116.2M / 33.9M |
| [interprocess](https://crates.io/crates/interprocess) | 2.4.6 | 2026-10-16 | same (2026-10-16) | 0BSD OR Apache-2.0 | 1.75 | 16.5M / 4.6M |
| [io-kit-sys](https://crates.io/crates/io-kit-sys) | 0.5.0 | 2025-10-31 | same (2025-10-31) | MIT OR Apache-2.0 | 1.65 | 15.0M / 3.2M |
| [jagua-rs](https://crates.io/crates/jagua-rs) | 1.1.0 | 2026-10-09 | same (2026-10-09) | MPL-2.0 | 1.90 | 26k / 16k |
| [kamadak-exif](https://crates.io/crates/kamadak-exif) | 0.6.1 | 2024-11-06 | same (2024-11-06) | BSD-2-Clause | 1.60 | 15.6M / 4.0M |
| [keyboard-types](https://crates.io/crates/keyboard-types) | 0.8.3 | 2025-10-02 | same (2025-10-02) | MIT OR Apache-2.0 | 1.61 | 46.6M / 19.5M |
| [kiddo](https://crates.io/crates/kiddo) | 6.3.0 | 2026-09-06 | same (2026-09-06) | MIT OR Apache-2.0 | 1.89.0 | 8.1M / 869k |
| [kittest](https://crates.io/crates/kittest) | 0.5.0 | 2026-10-05 | same (2026-10-05) | MIT OR Apache-2.0 | 1.95 | 1.6M / 971k |
| [kornia](https://crates.io/crates/kornia) | 0.1.9 | 2025-05-07 | 0.1.10-rc.3 (2025-07-01) | Apache-2.0 | 1.76 | 28k / 2k |
| [kornia-apriltag](https://crates.io/crates/kornia-apriltag) | 0.2.0 | 2026-10-12 | same (2026-10-12) | Apache-2.0 | 1.89 | 2k / 678 |
| [kornia-imgproc](https://crates.io/crates/kornia-imgproc) | 0.2.0 | 2026-09-26 | same (2026-09-26) | Apache-2.0 | 1.89 | 49k / 7k |
| [kornia-io](https://crates.io/crates/kornia-io) | 0.2.0 | 2026-09-26 | same (2026-09-26) | Apache-2.0 | 1.89 | 32k / 3k |
| [krilla](https://crates.io/crates/krilla) | 0.8.2 | 2026-06-04 | same (2026-06-04) | MIT OR Apache-2.0 | 1.92 | 3.1M / 1.9M |
| [kurbo](https://crates.io/crates/kurbo) | 0.13.1 | 2026-05-13 | same (2026-05-13) | Apache-2.0 OR MIT | 1.85 | 57.0M / 23.2M |
| [laser-dac](https://crates.io/crates/laser-dac) | 0.14.1 | 2026-10-11 | same (2026-10-11) | MIT | 1.87 | 2k / 775 |
| [lensfun](https://crates.io/crates/lensfun) | 0.7.0 | 2026-04-19 | same (2026-04-19) | LGPL-3.0-or-later | 1.85 | 3k / 2k |
| [levenberg-marquardt](https://crates.io/crates/levenberg-marquardt) | 0.15.0 | 2025-08-03 | same (2025-08-03) | MIT | - | 1.5M / 476k |
| [libadwaita](https://crates.io/crates/libadwaita) | 0.9.2 | 2026-07-07 | same (2026-07-07) | MIT | - | 2.6M / 639k |
| [libusb1-sys](https://crates.io/crates/libusb1-sys) | 0.7.0 | 2024-04-27 | same (2024-04-27) | MIT | - | 9.8M / 2.0M |
| [linesweeper](https://crates.io/crates/linesweeper) | 0.5.0 | 2026-10-05 | same (2026-10-05) | MIT OR Apache-2.0 | - | 96k / 82k |
| [lopdf](https://crates.io/crates/lopdf) | 0.46.0 | 2026-10-21 | same (2026-10-21) | MIT | 1.88 | 25.9M / 12.7M |
| [loro](https://crates.io/crates/loro) | 1.17.1 | 2026-10-22 | same (2026-10-22) | MIT | - | 967k / 591k |
| [lyon](https://crates.io/crates/lyon) | 1.0.19 | 2026-03-08 | same (2026-03-08) | MIT OR Apache-2.0 | - | 7.9M / 3.0M |
| [lyon_algorithms](https://crates.io/crates/lyon_algorithms) | 1.0.22 | 2026-10-20 | same (2026-10-20) | MIT OR Apache-2.0 | - | 10.9M / 3.9M |
| [lyon_geom](https://crates.io/crates/lyon_geom) | 1.0.20 | 2026-10-20 | same (2026-10-20) | MIT OR Apache-2.0 | - | 13.2M / 4.3M |
| [lyon_path](https://crates.io/crates/lyon_path) | 1.0.19 | 2026-03-08 | same (2026-03-08) | MIT OR Apache-2.0 | - | 12.2M / 4.2M |
| [lyon_tessellation](https://crates.io/crates/lyon_tessellation) | 1.0.23 | 2026-10-20 | same (2026-10-20) | MIT OR Apache-2.0 | - | 9.0M / 3.4M |
| [makepad-widgets](https://crates.io/crates/makepad-widgets) | 1.0.0 | 2025-05-13 | same (2025-05-13) | MIT OR Apache-2.0 | - | 20k / 1k |
| [masonry](https://crates.io/crates/masonry) | 0.4.0 | 2025-10-29 | same (2025-10-29) | Apache-2.0 | 1.88 | 23k / 3k |
| [mdns-sd](https://crates.io/crates/mdns-sd) | 0.21.6 | 2026-10-19 | same (2026-10-19) | Apache-2.0 OR MIT | 1.71.0 | 6.6M / 3.0M |
| [minidumper](https://crates.io/crates/minidumper) | 0.11.0 | 2026-07-20 | same (2026-07-20) | MIT OR Apache-2.0 | - | 2.9M / 1.4M |
| [mio-serial](https://crates.io/crates/mio-serial) | 5.0.7 | 2026-06-15 | same (2026-06-15) | MIT | - | 7.9M / 1.9M |
| [mlua](https://crates.io/crates/mlua) | 0.12.2 | 2026-10-03 | same (2026-10-03) | MIT | 1.88 | 8.1M / 2.4M |
| [mrcal](https://crates.io/crates/mrcal) | 2.5.2 | 2026-08-27 | same (2026-08-27) | Apache-2.0 | 1.85 | 19 / 19 |
| [muda](https://crates.io/crates/muda) | 0.21.2 | 2026-10-08 | same (2026-10-08) | Apache-2.0 OR MIT | 1.90 | 43.8M / 18.2M |
| [nalgebra](https://crates.io/crates/nalgebra) | 0.35.0 | 2026-05-24 | same (2026-05-24) | Apache-2.0 | 1.89.0 | 97.2M / 17.0M |
| [nextest](https://crates.io/crates/nextest) | 0.1.1 | 2025-02-11 | same (2025-02-11) | MIT OR Apache-2.0 | - | 31k / 2k |
| [nokhwa](https://crates.io/crates/nokhwa) | 0.10.11 | 2026-05-15 | same (2026-05-15) | Apache-2.0 | - | 623k / 221k |
| [nokhwa-bindings-macos](https://crates.io/crates/nokhwa-bindings-macos) | 0.2.4 | 2026-05-15 | same (2026-05-15) | Apache-2.0 | - | 431k / 164k |
| [nom-gcode](https://crates.io/crates/nom-gcode) | 0.1.1 | 2022-02-04 | same (2022-02-04) | MIT | - | 4k / 51 |
| [notify](https://crates.io/crates/notify) | 8.2.0 | 2025-08-03 | 9.0.0-rc.6 (2026-10-17) | CC0-1.0 | 1.77 | 178.0M / 47.8M |
| [nusb](https://crates.io/crates/nusb) | 0.2.8 | 2026-10-14 | same (2026-10-14) | Apache-2.0 OR MIT | 1.85 | 1.8M / 791k |
| [objc2](https://crates.io/crates/objc2) | 0.6.5 | 2026-10-05 | same (2026-10-05) | MIT | 1.71 | 140.9M / 55.0M |
| [objc2-av-foundation](https://crates.io/crates/objc2-av-foundation) | 0.3.2 | 2025-10-04 | same (2025-10-04) | Zlib OR Apache-2.0 OR MIT | 1.71 | 2.3M / 1.4M |
| [opencv](https://crates.io/crates/opencv) | 0.101.1 | 2026-10-09 | same (2026-10-09) | MIT | 1.88.0 | 5.0M / 1.6M |
| [parley](https://crates.io/crates/parley) | 0.12.0 | 2026-10-09 | same (2026-10-09) | Apache-2.0 OR MIT | 1.88 | 4.6M / 2.6M |
| [pdf](https://crates.io/crates/pdf) | 0.10.0 | 2026-03-02 | same (2026-03-02) | MIT | - | 634k / 74k |
| [pdf-writer](https://crates.io/crates/pdf-writer) | 0.15.0 | 2026-05-27 | same (2026-05-27) | MIT OR Apache-2.0 | - | 6.2M / 3.6M |
| [pdfium-render](https://crates.io/crates/pdfium-render) | 0.9.4 | 2026-09-06 | same (2026-09-06) | MIT OR Apache-2.0 | 1.61 | 3.4M / 1.7M |
| [peniko](https://crates.io/crates/peniko) | 0.6.1 | 2026-05-15 | same (2026-05-15) | Apache-2.0 OR MIT | 1.85 | 9.1M / 6.1M |
| [photon-rs](https://crates.io/crates/photon-rs) | 0.3.3 | 2025-05-10 | same (2025-05-10) | Apache-2.0 | - | 91k / 8k |
| [postcard](https://crates.io/crates/postcard) | 1.1.3 | 2025-07-24 | same (2025-07-24) | MIT OR Apache-2.0 | - | 79.4M / 32.6M |
| [printpdf](https://crates.io/crates/printpdf) | 0.12.8 | 2026-09-05 | same (2026-09-05) | MIT | 1.88 | 3.6M / 1.6M |
| [proptest](https://crates.io/crates/proptest) | 1.12.0 | 2026-10-10 | same (2026-10-10) | MIT OR Apache-2.0 | 1.88 | 219.3M / 61.6M |
| [quick-xml](https://crates.io/crates/quick-xml) | 0.42.0 | 2026-08-22 | same (2026-08-22) | MIT | 1.86 | 489.7M / 145.3M |
| [quickcheck](https://crates.io/crates/quickcheck) | 1.1.0 | 2026-02-10 | same (2026-02-10) | Unlicense OR MIT | 1.85.0 | 75.2M / 13.7M |
| [rayon](https://crates.io/crates/rayon) | 1.12.0 | 2026-04-14 | same (2026-04-14) | MIT OR Apache-2.0 | 1.80 | 616.1M / 147.6M |
| [read-fonts](https://crates.io/crates/read-fonts) | 0.46.0 | 2026-10-21 | same (2026-10-21) | MIT OR Apache-2.0 | 1.85 | 41.1M / 22.6M |
| [relm4](https://crates.io/crates/relm4) | 0.11.0 | 2026-04-08 | same (2026-04-08) | Apache-2.0 OR MIT | 1.93 | 1.3M / 767k |
| [resvg](https://crates.io/crates/resvg) | 0.48.1 | 2026-08-02 | same (2026-08-02) | Apache-2.0 OR MIT | 1.85.0 | 32.3M / 12.0M |
| [rfd](https://crates.io/crates/rfd) | 0.17.2 | 2026-01-12 | 0.16.1 (2026-10-22) | MIT | - | 38.0M / 15.2M |
| [rhai](https://crates.io/crates/rhai) | 1.27.0 | 2026-10-21 | same (2026-10-21) | MIT OR Apache-2.0 | 1.66.0 | 13.4M / 4.4M |
| [rhai-dylib](https://crates.io/crates/rhai-dylib) | 0.11.0 | 2026-09-12 | same (2026-09-12) | MIT OR Apache-2.0 | - | 75k / 939 |
| [rmp-serde](https://crates.io/crates/rmp-serde) | 1.3.1 | 2025-12-23 | same (2025-12-23) | MIT | 1.85 | 143.8M / 29.1M |
| [ron](https://crates.io/crates/ron) | 0.12.2 | 2026-06-22 | same (2026-06-22) | MIT OR Apache-2.0 | 1.64.0 | 122.8M / 22.1M |
| [roxmltree](https://crates.io/crates/roxmltree) | 0.21.1 | 2025-10-12 | same (2025-10-12) | MIT OR Apache-2.0 | 1.60 | 90.7M / 30.8M |
| [rpds](https://crates.io/crates/rpds) | 1.2.1 | 2026-05-15 | same (2026-05-15) | MIT | 1.85.0 | 20.4M / 4.8M |
| [rstar](https://crates.io/crates/rstar) | 0.13.0 | 2026-05-24 | same (2026-05-24) | MIT OR Apache-2.0 | 1.85 | 50.8M / 15.1M |
| [ruida](https://crates.io/crates/ruida) | 0.1.1 | 2025-07-15 | same (2025-07-15) | MIT | - | 840 / 7 |
| [rusb](https://crates.io/crates/rusb) | 0.9.4 | 2024-04-27 | same (2024-04-27) | MIT | - | 9.7M / 2.0M |
| [rust-i18n](https://crates.io/crates/rust-i18n) | 4.2.4 | 2026-09-29 | same (2026-09-29) | MIT | - | 4.2M / 1.5M |
| [rustybuzz](https://crates.io/crates/rustybuzz) | 0.20.1 | 2024-11-12 | same (2024-11-12) | MIT | - | 36.2M / 11.7M |
| [self_update](https://crates.io/crates/self_update) | 1.3.1 | 2026-10-17 | same (2026-10-17) | MIT | 1.88 | 12.7M / 2.4M |
| [sentry](https://crates.io/crates/sentry) | 0.49.3 | 2026-09-21 | same (2026-09-21) | MIT | 1.88 | 60.4M / 15.8M |
| [serde](https://crates.io/crates/serde) | 1.0.229 | 2026-07-18 | same (2026-07-18) | MIT OR Apache-2.0 | 1.56 | 1574.4M / 377.4M |
| [serde_json](https://crates.io/crates/serde_json) | 1.0.154 | 2026-10-11 | same (2026-10-11) | MIT OR Apache-2.0 | 1.71 | 1478.4M / 377.4M |
| [serial2](https://crates.io/crates/serial2) | 0.2.38 | 2026-07-31 | same (2026-07-31) | BSD-2-Clause OR Apache-2.0 | 1.63 | 19.1M / 12.9M |
| [serial2-tokio](https://crates.io/crates/serial2-tokio) | 0.1.25 | 2026-07-31 | same (2026-07-31) | BSD-2-Clause OR Apache-2.0 | - | 195k / 74k |
| [serialport](https://crates.io/crates/serialport) | 4.10.1 | 2026-09-08 | same (2026-09-08) | MPL-2.0 | 1.59.0 | 20.1M / 3.6M |
| [sha2](https://crates.io/crates/sha2) | 0.11.1 | 2026-10-12 | same (2026-10-12) | MIT OR Apache-2.0 | 1.88 | 1102.7M / 321.8M |
| [skia-safe](https://crates.io/crates/skia-safe) | 0.153.3 | 2026-09-04 | same (2026-09-04) | MIT | 1.85 | 4.7M / 1.5M |
| [skrifa](https://crates.io/crates/skrifa) | 0.49.0 | 2026-10-21 | same (2026-10-21) | MIT OR Apache-2.0 | 1.85 | 38.7M / 21.5M |
| [slint](https://crates.io/crates/slint) | 1.18.1 | 2026-09-21 | same (2026-09-21) | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | 1.92 | 2.1M / 772k |
| [smol](https://crates.io/crates/smol) | 2.0.2 | 2024-09-07 | same (2024-09-07) | Apache-2.0 OR MIT | 1.63 | 26.6M / 7.1M |
| [socket2](https://crates.io/crates/socket2) | 0.6.5 | 2026-07-13 | same (2026-07-13) | MIT OR Apache-2.0 | 1.70 | 1374.8M / 341.3M |
| [spade](https://crates.io/crates/spade) | 2.15.1 | 2026-03-24 | same (2026-03-24) | MIT OR Apache-2.0 | - | 21.8M / 5.5M |
| [sparrow](https://crates.io/crates/sparrow) | 0.1.0 | 2020-11-24 | same (2020-11-24) | Apache-2.0 | - | 2k / 29 |
| [specta](https://crates.io/crates/specta) | 1.0.5 | 2023-07-17 | 2.0.0-rc.27 (2026-10-21) | MIT | - | 3.1M / 1.4M |
| [stet](https://crates.io/crates/stet) | 0.8.7 | 2026-10-15 | same (2026-10-15) | Apache-2.0 OR MIT | 1.92 | 222k / 213k |
| [svg](https://crates.io/crates/svg) | 0.18.0 | 2024-09-27 | same (2024-09-27) | Apache-2.0 OR MIT | - | 8.6M / 1.9M |
| [svg2gcode](https://crates.io/crates/svg2gcode) | 0.3.4 | 2026-03-04 | same (2026-03-04) | MIT | - | 28k / 328 |
| [svgtypes](https://crates.io/crates/svgtypes) | 0.16.1 | 2026-01-09 | same (2026-01-09) | Apache-2.0 OR MIT | 1.85 | 36.4M / 13.2M |
| [swash](https://crates.io/crates/swash) | 0.2.10 | 2026-07-17 | same (2026-07-17) | Apache-2.0 OR MIT | - | 16.4M / 6.5M |
| [tao](https://crates.io/crates/tao) | 0.37.1 | 2026-09-26 | same (2026-09-26) | Apache-2.0 | 1.85 | 40.8M / 16.2M |
| [tauri](https://crates.io/crates/tauri) | 2.12.5 | 2026-10-21 | same (2026-10-21) | Apache-2.0 OR MIT | 1.90 | 38.5M / 15.4M |
| [tauri-build](https://crates.io/crates/tauri-build) | 2.7.4 | 2026-10-21 | same (2026-10-21) | Apache-2.0 OR MIT | 1.90 | 38.2M / 15.4M |
| [tauri-plugin-updater](https://crates.io/crates/tauri-plugin-updater) | 2.14.0 | 2026-10-14 | 3.0.0-alpha.3 (2026-10-14) | Apache-2.0 OR MIT | 1.90 | 12.9M / 6.0M |
| [thiserror](https://crates.io/crates/thiserror) | 2.0.21 | 2026-09-23 | same (2026-09-23) | MIT OR Apache-2.0 | 1.77 | 1683.0M / 450.8M |
| [tiny-skia](https://crates.io/crates/tiny-skia) | 0.12.0 | 2026-02-02 | same (2026-02-02) | BSD-3-Clause | - | 59.1M / 20.6M |
| [tokio](https://crates.io/crates/tokio) | 1.54.1 | 2026-10-21 | same (2026-10-21) | MIT | 1.85 | 1097.0M / 269.9M |
| [tokio-serial](https://crates.io/crates/tokio-serial) | 5.5.0 | 2026-06-15 | same (2026-06-15) | MIT | - | 7.8M / 1.8M |
| [tokio-tungstenite](https://crates.io/crates/tokio-tungstenite) | 0.30.0 | 2026-07-11 | same (2026-07-11) | MIT | 1.85 | 322.5M / 95.8M |
| [toml](https://crates.io/crates/toml) | 1.1.8+spec-1.1.0 | 2026-10-09 | same (2026-10-09) | MIT OR Apache-2.0 | 1.85 | 1040.7M / 272.4M |
| [tracing](https://crates.io/crates/tracing) | 0.1.44 | 2025-12-18 | same (2025-12-18) | MIT | 1.65.0 | 961.8M / 233.4M |
| [tracing-appender](https://crates.io/crates/tracing-appender) | 0.2.5 | 2026-04-17 | same (2026-04-17) | MIT | 1.63.0 | 134.1M / 40.4M |
| [tracing-subscriber](https://crates.io/crates/tracing-subscriber) | 0.3.23 | 2026-03-13 | same (2026-03-13) | MIT | 1.65.0 | 687.4M / 169.3M |
| [travelling_salesman](https://crates.io/crates/travelling_salesman) | 1.1.22 | 2022-07-16 | same (2022-07-16) | GPL-3.0 | - | 44k / 874 |
| [tray-icon](https://crates.io/crates/tray-icon) | 0.26.1 | 2026-10-07 | same (2026-10-07) | MIT OR Apache-2.0 | 1.90 | 37.3M / 16.5M |
| [truck-stepio](https://crates.io/crates/truck-stepio) | 0.3.0 | 2024-09-20 | same (2024-09-20) | Apache-2.0 | - | 110k / 80k |
| [ts-rs](https://crates.io/crates/ts-rs) | 12.0.1 | 2026-01-31 | same (2026-01-31) | MIT | 1.78.0 | 19.7M / 8.6M |
| [ttf-parser](https://crates.io/crates/ttf-parser) | 0.25.1 | 2024-11-29 | same (2024-11-29) | MIT OR Apache-2.0 | 1.63.0 | 123.5M / 33.6M |
| [tungstenite](https://crates.io/crates/tungstenite) | 0.30.0 | 2026-07-11 | same (2026-07-11) | MIT OR Apache-2.0 | 1.85 | 359.7M / 106.3M |
| [u-routing](https://crates.io/crates/u-routing) | 0.22.1 | 2026-10-20 | same (2026-10-20) | MIT | 1.87 | 550 / 370 |
| [udev](https://crates.io/crates/udev) | 0.9.3 | 2025-01-23 | same (2025-01-23) | MIT | - | 7.1M / 1.4M |
| [undo](https://crates.io/crates/undo) | 0.52.0 | 2025-03-08 | same (2025-03-08) | MIT OR Apache-2.0 | - | 188k / 9k |
| [undo_2](https://crates.io/crates/undo_2) | 0.2.1 | 2024-12-12 | same (2024-12-12) | MIT OR Apache-2.0 | 1.64 | 14k / 352 |
| [undoredo](https://crates.io/crates/undoredo) | 0.15.1 | 2026-09-06 | same (2026-09-06) | MIT OR Apache-2.0 | 1.92 | 6k / 4k |
| [usvg](https://crates.io/crates/usvg) | 0.48.1 | 2026-08-02 | same (2026-08-02) | Apache-2.0 OR MIT | 1.85.0 | 35.3M / 12.8M |
| [usvg-text-layout](https://crates.io/crates/usvg-text-layout) | 0.38.0 | 2024-01-21 | same (2024-01-21) | MPL-2.0 | - | 3.3M / 489k |
| [v4l](https://crates.io/crates/v4l) | 0.14.0 | 2023-05-12 | same (2023-05-12) | MIT | - | 8.1M / 1.2M |
| [vector-text-hershey](https://crates.io/crates/vector-text-hershey) | 0.1.1 | 2026-02-25 | same (2026-02-25) | non-standard | - | 483 / 283 |
| [vello](https://crates.io/crates/vello) | 0.11.0 | 2026-10-02 | same (2026-10-02) | Apache-2.0 OR MIT | 1.89 | 1.1M / 446k |
| [vello_common](https://crates.io/crates/vello_common) | 0.3.0 | 2026-10-02 | same (2026-10-02) | Apache-2.0 OR MIT | 1.89 | 9.7M / 7.9M |
| [vello_cpu](https://crates.io/crates/vello_cpu) | 0.3.0 | 2026-10-02 | same (2026-10-02) | Apache-2.0 OR MIT | 1.89 | 7.7M / 5.9M |
| [vello_gpu](https://crates.io/crates/vello_gpu) | 0.3.0 | 2026-10-02 | same (2026-10-02) | Apache-2.0 OR MIT | 1.89 | 15k / 15k |
| [vello_hybrid](https://crates.io/crates/vello_hybrid) | 0.2.0 | 2026-08-07 | same (2026-08-07) | Apache-2.0 OR MIT | 1.88 | 62k / 47k |
| [vello_svg](https://crates.io/crates/vello_svg) | 0.11.0 | 2026-09-10 | same (2026-09-10) | Apache-2.0 OR MIT | 1.88 | 98k / 25k |
| [velopack](https://crates.io/crates/velopack) | 1.2.161 | 2026-09-29 | 1.3.39-g8d2844f (2026-10-17) | MIT | 1.75 | 141k / 59k |
| [vision-calibration](https://crates.io/crates/vision-calibration) | 0.9.0 | 2026-09-29 | same (2026-09-29) | MIT | - | 702 / 223 |
| [visioncortex](https://crates.io/crates/visioncortex) | 0.9.3 | 2026-08-14 | same (2026-08-14) | MIT OR Apache-2.0 | - | 231k / 94k |
| [vsvg](https://crates.io/crates/vsvg) | 0.5.0 | 2024-09-21 | same (2024-09-21) | MIT | - | 26k / 432 |
| [vtracer](https://crates.io/crates/vtracer) | 0.6.5 | 2025-10-17 | 1.0.0-alpha.4 (2026-08-29) | MIT OR Apache-2.0 | - | 119k / 53k |
| [wasmtime](https://crates.io/crates/wasmtime) | 50.0.0 | 2026-10-20 | same (2026-10-20) | Apache-2.0 WITH LLVM-exception | 1.97.0 | 41.2M / 11.7M |
| [wgpu](https://crates.io/crates/wgpu) | 30.0.1 | 2026-08-22 | same (2026-08-22) | MIT OR Apache-2.0 | 1.87.0 | 43.3M / 14.4M |
| [whiskers](https://crates.io/crates/whiskers) | 0.5.0 | 2024-09-21 | same (2024-09-21) | MIT | - | 17k / 129 |
| [winit](https://crates.io/crates/winit) | 0.30.13 | 2026-03-02 | 0.31.0-beta.3 (2026-09-04) | Apache-2.0 | 1.70.0 | 60.3M / 13.4M |
| [wry](https://crates.io/crates/wry) | 0.57.0 | 2026-09-08 | same (2026-09-08) | Apache-2.0 OR MIT | 1.85 | 41.6M / 16.3M |
| [xilem](https://crates.io/crates/xilem) | 0.4.0 | 2025-10-29 | same (2025-10-29) | Apache-2.0 | 1.88 | 12k / 2k |
| [yrs](https://crates.io/crates/yrs) | 0.28.0 | 2026-09-17 | same (2026-09-17) | MIT | - | 4.3M / 2.2M |
| [zip](https://crates.io/crates/zip) | 9.0.5 | 2026-10-17 | same (2026-10-17) | MIT | 1.88 | 312.1M / 89.5M |
| [zstd](https://crates.io/crates/zstd) | 0.14.3 | 2026-10-16 | same (2026-10-16) | BSD-3-Clause | 1.64 | 438.5M / 101.1M |
| [zune-image](https://crates.io/crates/zune-image) | 0.5.0 | 2026-01-24 | same (2026-01-24) | MIT OR Apache-2.0 OR Zlib | 1.87.0 | 122k / 46k |
| [zune-jpeg](https://crates.io/crates/zune-jpeg) | 0.5.15 | 2026-03-26 | 0.5.16-rc3 (2026-10-19) | MIT OR Apache-2.0 OR Zlib | 1.75.0 | 139.2M / 55.8M |
| [zune-png](https://crates.io/crates/zune-png) | 0.5.2 | 2026-03-11 | same (2026-03-11) | MIT OR Apache-2.0 OR Zlib | 1.87.0 | 226k / 86k |

## Appendix B: GitHub repository activity (queried 2026-10-22/23)

"Open issues" is GitHub's `open_issues_count`, which also counts open pull requests.

| Repo | Stars | Last push | Open issues | Archived | License (GitHub) |
|---|---|---|---|---|---|
| [emilk/egui](https://github.com/emilk/egui) | 31689 | 2026-10-22 | 1075 | false | Apache-2.0 |
| [iced-rs/iced](https://github.com/iced-rs/iced) | 31820 | 2026-10-21 | 512 | false | MIT |
| [slint-ui/slint](https://github.com/slint-ui/slint) | 24254 | 2026-10-22 | 749 | false | NOASSERTION |
| [gtk-rs/gtk4-rs](https://github.com/gtk-rs/gtk4-rs) | 2356 | 2026-10-19 | 80 | false | MIT |
| [tauri-apps/tauri](https://github.com/tauri-apps/tauri) | 112128 | 2026-10-22 | 1482 | false | Apache-2.0 |
| [DioxusLabs/dioxus](https://github.com/DioxusLabs/dioxus) | 39469 | 2026-10-17 | 860 | false | Apache-2.0 |
| [DioxusLabs/blitz](https://github.com/DioxusLabs/blitz) | 4293 | 2026-10-22 | 318 | false | Apache-2.0 |
| [linebender/xilem](https://github.com/linebender/xilem) | 5579 | 2026-09-14 | 163 | false | Apache-2.0 |
| [makepad/makepad](https://github.com/makepad/makepad) | 7211 | 2026-10-23 | 36 | false | MIT |
| [marc2332/freya](https://github.com/marc2332/freya) | 3245 | 2026-10-22 | 44 | false | MIT |
| [lapce/floem](https://github.com/lapce/floem) | 4302 | 2026-09-24 | 101 | false | MIT |
| [zed-industries/zed](https://github.com/zed-industries/zed) | 92068 | 2026-10-23 | 3059 | false | NOASSERTION |
| [longbridge/gpui-kit](https://github.com/longbridge/gpui-kit) | 18521 | 2026-10-23 | 122 | false | NOASSERTION |
| [GraphiteEditor/Graphite](https://github.com/GraphiteEditor/Graphite) | 27691 | 2026-10-22 | 456 | false | Apache-2.0 |
| [linebender/vello](https://github.com/linebender/vello) | 4439 | 2026-10-22 | 209 | false | Apache-2.0 |
| [nical/lyon](https://github.com/nical/lyon) | 2609 | 2026-10-22 | 18 | false | NOASSERTION |
| [femtovg/femtovg](https://github.com/femtovg/femtovg) | 949 | 2026-10-21 | 64 | false | NOASSERTION |
| [linebender/kurbo](https://github.com/linebender/kurbo) | 1022 | 2026-10-08 | 89 | false | Apache-2.0 |
| [linebender/tiny-skia](https://github.com/linebender/tiny-skia) | 1664 | 2026-08-20 | 51 | false | BSD-3-Clause |
| [iShape-Rust/iOverlay](https://github.com/iShape-Rust/iOverlay) | 215 | 2026-10-14 | 1 | false | Apache-2.0 |
| [jbuckmccready/cavalier_contours](https://github.com/jbuckmccready/cavalier_contours) | 237 | 2026-10-21 | 9 | false | Apache-2.0 |
| [larsbrubaker/clipper2-rust](https://github.com/larsbrubaker/clipper2-rust) | 28 | 2026-10-20 | 0 | false | BSL-1.0 |
| [tirithen/clipper2](https://github.com/tirithen/clipper2) | 28 | 2026-05-06 | 1 | false | Apache-2.0 |
| [jneem/linesweeper](https://github.com/jneem/linesweeper) | 45 | 2026-01-25 | 3 | true | Apache-2.0 |
| [Logicalshift/flo_curves](https://github.com/Logicalshift/flo_curves) | 99 | 2026-08-25 | 17 | false | Apache-2.0 |
| [georust/geo](https://github.com/georust/geo) | 1951 | 2026-10-22 | 134 | false | NOASSERTION |
| [linebender/resvg](https://github.com/linebender/resvg) | 4154 | 2026-10-20 | 173 | false | Apache-2.0 |
| [ixmilia/dxf-rs](https://github.com/ixmilia/dxf-rs) | 142 | 2026-10-03 | 25 | false | MIT |
| [LaurenzV/hayro](https://github.com/LaurenzV/hayro) | 813 | 2026-10-22 | 58 | false | Apache-2.0 |
| [pdf-rs/pdf](https://github.com/pdf-rs/pdf) | 1718 | 2026-10-13 | 38 | false | MIT |
| [J-F-Liu/lopdf](https://github.com/J-F-Liu/lopdf) | 2285 | 2026-10-22 | 86 | false | MIT |
| [ajrcarey/pdfium-render](https://github.com/ajrcarey/pdfium-render) | 716 | 2026-10-08 | 19 | false | NOASSERTION |
| [AndyCappDev/stet](https://github.com/AndyCappDev/stet) | 18 | 2026-10-21 | 6 | false | Apache-2.0 |
| [kevinmehall/nusb](https://github.com/kevinmehall/nusb) | 539 | 2026-10-14 | 19 | false | Apache-2.0 |
| [a1ien/rusb](https://github.com/a1ien/rusb) | 497 | 2025-12-29 | 29 | false | NOASSERTION |
| [serialport/serialport-rs](https://github.com/serialport/serialport-rs) | 755 | 2026-09-08 | 106 | false | NOASSERTION |
| [berkowski/tokio-serial](https://github.com/berkowski/tokio-serial) | 267 | 2026-06-15 | 22 | false | MIT |
| [l1npengfih/nokhwa](https://github.com/l1npengfih/nokhwa) | 817 | 2026-05-15 | 67 | false | Apache-2.0 |
| [kornia/kornia-rs](https://github.com/kornia/kornia-rs) | 740 | 2026-10-21 | 98 | false | Apache-2.0 |
| [twistedfall/opencv-rust](https://github.com/twistedfall/opencv-rust) | 2498 | 2026-10-09 | 16 | false | MIT |
| [VitalyVorobyev/calib-targets-rs](https://github.com/VitalyVorobyev/calib-targets-rs) | 6 | 2026-10-17 | 2 | false | MIT |
| [VitalyVorobyev/calibration-rs](https://github.com/VitalyVorobyev/calibration-rs) | 6 | 2026-10-22 | 0 | false | MIT |
| [visioncortex/vtracer](https://github.com/visioncortex/vtracer) | 7352 | 2026-10-22 | 13 | false | MIT |
| [crabnebula-dev/cargo-packager](https://github.com/crabnebula-dev/cargo-packager) | 496 | 2026-10-22 | 39 | false | Apache-2.0 |
| [burtonageo/cargo-bundle](https://github.com/burtonageo/cargo-bundle) | 1376 | 2026-09-20 | 74 | false | NOASSERTION |
| [axodotdev/cargo-dist](https://github.com/axodotdev/cargo-dist) | 2138 | 2026-10-21 | 356 | false | Apache-2.0 |
| [velopack/velopack](https://github.com/velopack/velopack) | 2409 | 2026-10-23 | 38 | false | MIT |
| [jaemk/self_update](https://github.com/jaemk/self_update) | 962 | 2026-10-17 | 0 | false | MIT |
| [indygreg/apple-platform-rs](https://github.com/indygreg/apple-platform-rs) | 872 | 2026-10-17 | 120 | false | none |
| [pop-os/cosmic-text](https://github.com/pop-os/cosmic-text) | 2174 | 2026-09-30 | 136 | false | Apache-2.0 |
| [linebender/parley](https://github.com/linebender/parley) | 766 | 2026-10-22 | 118 | false | Apache-2.0 |
| [harfbuzz/harfrust](https://github.com/harfbuzz/harfrust) | 320 | 2026-10-22 | 30 | false | MIT |
| [dfrg/swash](https://github.com/dfrg/swash) | 878 | 2026-07-17 | 55 | false | Apache-2.0 |
| [RazrFalcon/fontdb](https://github.com/RazrFalcon/fontdb) | 179 | 2026-07-29 | 17 | false | MIT |
| [rhaiscript/rhai](https://github.com/rhaiscript/rhai) | 5752 | 2026-10-22 | 16 | false | Apache-2.0 |
| [mlua-rs/mlua](https://github.com/mlua-rs/mlua) | 2918 | 2026-10-03 | 61 | false | NOASSERTION |
| [extism/extism](https://github.com/extism/extism) | 5813 | 2026-10-09 | 55 | false | BSD-3-Clause |
| [projectfluent/fluent-rs](https://github.com/projectfluent/fluent-rs) | 1371 | 2026-09-27 | 61 | false | Apache-2.0 |
| [mitsuhiko/insta](https://github.com/mitsuhiko/insta) | 2975 | 2026-10-10 | 78 | false | Apache-2.0 |
| [proptest-rs/proptest](https://github.com/proptest-rs/proptest) | 2267 | 2026-10-10 | 151 | false | Apache-2.0 |
| [rerun-io/kittest](https://github.com/rerun-io/kittest) | 97 | 2026-10-08 | 0 | false | Apache-2.0 |
| [JeroenGar/jagua-rs](https://github.com/JeroenGar/jagua-rs) | 214 | 2026-10-15 | 3 | false | MPL-2.0 |
| [JeroenGar/sparrow](https://github.com/JeroenGar/sparrow) | 415 | 2026-10-22 | 5 | false | MIT |
| [sameer/svg2gcode](https://github.com/sameer/svg2gcode) | 445 | 2026-10-15 | 19 | false | MIT |
| [meerk40t/galvoplotter](https://github.com/meerk40t/galvoplotter) | 23 | 2023-10-27 | 1 | false | MIT |
| [meerk40t/meerk40t](https://github.com/meerk40t/meerk40t) | 440 | 2026-09-25 | 65 | false | MIT |
| [barebaric/rayforge](https://github.com/barebaric/rayforge) | 301 | 2026-10-23 | 26 | false | MIT |
| [barebaric/raygeo](https://github.com/barebaric/raygeo) | 4 | 2026-10-22 | 2 | false | MIT |
| [barebaric/raydriver](https://github.com/barebaric/raydriver) | 0 | 2026-10-17 | 0 | false | MIT |
| [clayfrocker97/OpenLaser](https://github.com/clayfrocker97/OpenLaser) | 6 | 2026-10-04 | 0 | false | GPL-3.0 |
| [theanam/openlaser.app](https://github.com/theanam/openlaser.app) | 127 | 2026-10-22 | 0 | false | MIT |
