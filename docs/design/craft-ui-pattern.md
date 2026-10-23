# OpenLaser UI on the Craft shell

*Design note, 2026-10-23. It reverse-engineers the Craft-family UI shell (craft-libs and PhotoCraft)
and maps LightBurn's window layout onto it for OpenLaser. It is an input to `docs/SPEC.md`; where
the two disagree, SPEC wins.*

*Sources read (read-only; nothing in them was built or run): craft-libs checkouts in
`~/.cargo/git/checkouts/craft-libs-d830b991a08c6c3d/{5c874d5,f03ec27,3d37da3}` (`crates/ui`,
`settings`, `quit`, `atomic`; `README.md`, `docs/ui-docking.md`, `docs/theme-infrastructure.md`,
`docs/ui-components-audit.md`, `docs/ui-visual-regressions.md`), PhotoCraft
`~/AI/PDF_Craft_Edit/photocraft` (`crates/ui-egui`, `crates/automation`, `apps/photocraft`,
`packaging/macos`), the PdfCraft manifests, the egui 0.36.2 sources in the cargo registry, and the plan
in `~/AI/ClaudeCode/openlaser-rust-plan/` (`PLAN.md` §3–§4, research 02, 04 and 06). The
evidence index is in Appendix A.*

*Clean-room rule (AGENTS.md §3): this note describes LightBurn's **layout and behaviour** as a user
sees them. No LightBurn code, icons, images or help text are copied. LightBurn details marked
**(verify)** come from memory and the research inventory, not from a running copy. Check them against
Luiz's licensed LightBurn 1.5.04 before they are frozen in the parity catalog (§4.2).*

---

## 0. Summary of decisions

| # | Decision | Why |
|---|---|---|
| U1 | **egui/eframe/egui_extras/egui_kittest 0.36.x (0.36.2), not 0.37.** | craft-ui at every revision requires egui 0.36. All twelve Craft apps are on 0.36. This replaces PLAN §4/D6 ("egui 0.37 + egui_tiles 0.18"). |
| U2 | **Docking with `craft_ui::docking` (`Layout`, `DockArea`), not egui_tiles or egui_dock.** The canvas is a `Permissions::PROTECTED` panel inside the dock tree. | This is the Craft standard. Its layouts are transactional and validated, and its `Action`s serialise (serde), so agents and the UI share one command path. A lone protected panel renders with no tab header. |
| U3 | **Pin every craft-* crate at one revision, `3d37da389a3618ab9b76109da11416e9ca4ea719` (2026-10-13).** | It is the newest local checkout and the only one that has `ui`, `settings`, `quit`, `atomic` and `print` together. Its craft-ui is the same code PhotoCraft and PdfCraft pin (`5c874d51`), plus the `printing` module. It builds offline from the local cargo git database. |
| U4 | **Copy PhotoCraft's shell structure.** A UI library crate (`eframe::App` with `logic` + `ui`, plain-data `UiState`, injected `Services`, `control.rs` dispatcher) plus a thin desktop binary (`main.rs`, control server, muda macOS menu, platform services). | It is proven on macOS with eframe 0.36, including muda native menus (§1.3). Every UI action dispatches by command id, so the CLI, the MCP server and the GUI agree. |
| U5 | **A full LightBurn menu catalog**, in LightBurn's order. Live items run commands; the rest render disabled. A parity report has a never-lower floor. | This is how PhotoCraft does "exactly Photoshop" (`menu_catalog.rs`, `parity.rs`). |
| U6 | **Canvas v1: egui `Painter`, with our own hairline mesher and cached meshes.** A UI-free display list feeds both egui and tiny-skia (headless `render_preview`, golden PNGs). `egui::Scene` is not used. A wgpu hairline pipeline arrives later behind a `CanvasRenderer` trait. | It meets PLAN S1/S2 without a GPU dependency. Agents get previews without a window. |
| U7 | **Icons: Lucide (ISC) through `craft_ui::icons::SvgIconSet`, plus original laser glyphs.** No LightBurn assets. | Clean-room. PhotoCraft and PdfCraft hold 239 distinct SVGs between them, about 210 of them Lucide (PdfCraft records `lucide-static 1.49.0`), to seed from with attribution. |
| U8 | **Safety chrome outranks layout.** STOP lives in fixed chrome and in an `Order::Tooltip` overlay above modals, not only in the closable Laser panel. Hardware commands are **human-only**: the control channel refuses them, and the ARM surface ignores synthetic input and AccessKit actions. | PLAN §3.5/§3.6 and AGENTS.md §1. PhotoCraft's control channel can click, type and invoke any menu item, so OpenLaser must fence its hardware surfaces explicitly (§7). |

---

## 1. The Craft shell, reverse-engineered

### 1.1 craft-libs: revisions and what each crate gives

| Checkout | Commit date | Crates present | Who pins it |
|---|---|---|---|
| `5c874d5` (`5c874d51eeb3…`) | 2026-10-10 | `ui` only | PhotoCraft and PdfCraft: `craft-ui` |
| `f03ec27` (`f03ec27ab2c4…`) | 2026-10-12 | atomic, print, quit, settings, ui | PdfCraft: `craft-print` |
| `3d37da3` (`3d37da389a36…`) **newest** | 2026-10-13 | atomic, print, quit, settings, ui | PhotoCraft: `craft-print` |

Between `5c874d5` and `3d37da3`, `crates/ui` changed only by adding `printing` (three commits: b700ae7, d27646a, 4cae45d). Docking, buttons, tabs, layout, icons and theme are identical. The workspace is edition 2024, `rust-version = "1.95"`, `unsafe_code = "forbid"`, and licensed MIT OR Apache-2.0, the same as OpenLaser.

| Crate | What OpenLaser uses it for | Key API |
|---|---|---|
| `craft-ui` (feature `serde`) | Toolbar icons, palette and tab chrome, dock, splits, theme state colours | `icons::{SvgIconSet, SvgStyle}`; `buttons::{IconButton, IconButtonStyle, ButtonVisuals}`; `tabs::{fit, Tab}`; `layout::{split_rect, SplitAxis, SplitSize, SplitLayout, ResizeHandle, fit_toolbar}`; `theme::WidgetPalette`; `docking::{Layout, Node, Action, Placement, Zone, Permissions, PanelLimits, Location, DockArea, DockContent, DockStyle, DockOutput, begin_drag, cancel_drag, resolve_drop}` |
| `craft-settings` | UI preferences only (`config.toml`): theme, language, units, hotkeys, dock layout, recent files | `Settings` trait (`APP_ID = "openlaser"`, `SCHEMA_VERSION`, `migrate`); `SettingsStore::open(Location::Auto)`, `update`, `tick`, `flush`. Saves are atomic and tolerant of hand edits (toml_edit); damaged files are moved aside. |
| `craft-atomic` | Every safety-relevant write: projects, machine profiles, calibration, job exports | `atomic_write`, `atomic_write_with`, `write_new` (same-directory staging, fsync, `F_FULLFSYNC` on Apple) |
| `craft-quit` (feature `egui` only) | Quit through `-[NSApplication terminate:]`, which avoids the Touch Bar KVO crash, after the app's own close decision | `on_close_request(ctx)`, called **last** in `logic`; `quit()`; `QUIT_MENU_ID` and `on_menu_event` for an app-built muda menu |
| `craft-print` | Not in M1. LightBurn's File › Print (paper) arrives in T2 at the earliest. | — |

Boundaries craft-ui keeps, which OpenLaser must therefore supply itself: assets, complete themes, fonts, localisation, command handling, menus, numeric fields, rulers and swatch grids. The audit lists numeric editing (priority 3), swatch grids (4), menu helpers (5) and ruler ticks (6) as extraction candidates, none of them implemented. craft-ui does **not** install image loaders or an egui theme.

### 1.2 PhotoCraft `crates/ui-egui` anatomy

PhotoCraft has about 187k lines of UI. The parts that matter for OpenLaser:

- **App object.** `PhotocraftApp { session: Session, ui: UiState, services: Services, … }` (`lib.rs:504`). The crate doc says: "Every action goes through `Session::execute`, and all UI state lives in `state::UiState` (plain data). The `control` module exposes both to automation."
- **egui 0.36 frame contract** (`impl eframe::App`, `lib.rs:1689`):
  - `raw_input_hook` runs first. It folds native-menu key equivalents back into egui input (`native_menu.raw_input`), and it injects queued synthetic control events (`take_synthetic_step`), which sets `automation_input = true` for that frame.
  - `logic(ctx, frame)` runs even while the window is minimised. It: styles once; drains the control channel (`drain_control`); runs native menu clicks; ticks jobs and preferences; handles shortcuts; drains file drops and OS events; guards the window close (`discard_ui::guard_window_close`); and polls file dialogs.
  - `ui(ui, frame)` lays out, in order: `panels::title_bar` (`egui::Panel::top`), `options_bar` (top), `status_bar` (`Panel::bottom`), `toolbar` (left), `right_dock`, then `egui::CentralPanel` → `canvas::document_area`, then floating windows, dialogs and notices. After that it persists tool memory and view settings and calls `native_menu::sync`.
- **Services injection** (`Services`, `lib.rs:401`). The binary injects file dialogs, import/export, clipboard, preferences I/O, autosave, OS events, the native menu, `quit` and `assistant_bridge`, plus the `automation_*` read/write/authorise hooks that are capability-scoped to `--automation-read-root/--automation-write-root`. This keeps the UI crate free of platform I/O and testable headless.
- **Theme.** `theme::Tokens` (about 37 colour and metric fields) is stored in egui context data (`Tokens::get(ctx)`). `ThemeKind` covers 14 presets: four Photoshop "Pro" brightness levels, Studio, Classic and others. Widgets never hard-code colours.
- **Icons** (`icons.rs` + `icon_data.rs`). 146 SVGs are embedded with `include_bytes!`. `SvgIconSet::new(ICONS, SvgStyle { current_color: "#ffffff", stroke_width: Some("1.75") }, "bytes://icons/", "square")` recolours them to white, and they are tinted per use. `egui_extras::install_image_loaders` (svg feature) rasterises at exact pixel size. `rail_button` wraps `craft_ui::buttons::IconButton`.
- **Menus.**
  - `menu_catalog::CATALOG` holds `(path, label, shortcut, command id)` rows for Photoshop's full tree, in order, with `"---"` separators. `menus::menu_items` marks a row live when the engine, the shell or a panel alias implements its id. The rest render disabled.
  - `parity.rs` reports live/total per menu and enforces `FLOOR = 631`, which only ever rises.
  - `native_menu.rs` is a pure, platform-free model of the macOS menu bar. It builds the app menu (About, Settings, Language, Appearance, Quit = `file.exit`) and resolves clashes with system shortcuts. Its `Backend` trait is implemented with muda in the binary (`mac_menu.rs`). Key equivalents come back as egui key presses, so every shortcut runs through one dispatcher. `--in-window-menus` keeps egui menus.
- **Shortcuts** (`shortcuts.rs`, `shortcut_dispatch`). `Cmd+Shift+N` strings are parsed strictly; `Cmd` is ⌘ on macOS and Ctrl elsewhere. A malformed binding is rejected rather than reinterpreted.
- **Docking adapter** (`panel_docking.rs`).
  - Panel ids are stable, camelCase and not localised (`"layers"`, `"history"`). Labels are translated.
  - `DockArea::show_customized` gets a `DockStyle` built from Tokens (tab height 28 Pro / 40 Studio, localised action labels), `Permissions` (all false when the workspace is locked) and per-panel `PanelLimits` (Layers needs at least 220 pt height).
  - Output actions are applied after rendering. The layout is validated against the panel registry; an invalid layout is replaced by the default with a status message.
  - Docking is also exposed as commands to the control channel (`panel_docking::command`).
- **Control channel** (`control.rs`, about 3k lines).
  - It is transport-agnostic: a `ControlRequest { method, params, reply, deadline, dispatch }` is handled on the UI thread between frames.
  - Methods: `engine.execute`, `engine.commands`; `ui.inspect`; `ui.set` (fields not in the allow-list are an error); `ui.menu.list`; `ui.menu.invoke`; `ui.dialog.open/set/confirm/cancel/apply`; `ui.pointer` (document coordinates); `ui.click/move/key/type` (synthetic input); `ui.resize`; `ui.screenshot`; `ui.focus`; `app.open/save` (roots only); `app.quit`; `jobs.list/cancel`.
  - `Dispatch` settles "queued, dispatched or abandoned" atomically. A timeout therefore says either "not run, safe to re-send" or "may still apply".
- **Tests.**
  - egui_kittest 0.36 uses `Harness::builder().with_size(..).build_ui_state(|ui, app| panel(app, ui), app)` for panels and `build_eframe(|cc| PhotocraftApp::new(..))` for the whole app at 1440×900. Queries use the AccessKit role and label (`query_all_by_role(Role::Button)` filtered by label).
  - Real pointer and modifier events are sent as `h.event(..)`, followed by `run_steps(n)`.
  - Optional PNG captures use `.wgpu()` and `h.render()`, gated by an output-directory environment variable.

### 1.3 PhotoCraft `apps/photocraft` (the desktop binary)

- **Usage:** `photocraft [--control <port>] [--control-token <hex> | --control-token-file <path>] [--automation-read-root <dir>] [--automation-write-root <dir>] [--safe-gpu] [--in-window-menus] [files…]`. An unparseable `--control` port fails the launch with exit code 2; it never starts silently without control.
- **`--mcp-bridge` is handled before anything else.** `PhotoCraft --mcp-bridge 127.0.0.1:<port> --control-token-file <f>` runs the MCP server on stdio, the same one as `photocraft-cli mcp --bridge`, and exits. No window opens.
- **`native_options`.** 1440×900 inner size, minimum 760×480, drag-and-drop on, `with_fullsize_content_view(true)` with the title bar hidden (integrated title strip on macOS), eframe persistence to `<config>/ui.ron`, and a `window_builder` that corrects restored geometry.
- **App creator closure.**
  1. Build `Services::native(workspace)` and `PhotocraftApp::new(Session::new(), services)`.
  2. Set up the GPU and the CPU fallback.
  3. If `--control` was given, `control_server::start_bound(port, token, ctx)` binds 127.0.0.1 only and returns the `Receiver<ControlRequest>`, wired in with `app.with_control(rx)`.
  4. Install `assistant_bridge::bridge_fn`. Window › Assistant starts the channel on demand with port 0, a fresh 256-bit token and an owner-only token file, and returns the MCP command line for the agent.
  5. On macOS, install `apple_events`, set `quit = mac_window::terminate_later`, and call `mac_menu::install(ctx, &app)` unless in-window menus were asked for.
- **Control server.**
  - JSON lines over loopback TCP. The first line must authenticate; the token is never passed on argv for the bridge.
  - Requests are bounded (`MAX_REQUEST_BYTES`) and connections limited (`ConnectionLimiter`). The reply deadline is 60 s.
  - It wakes the UI with `ctx.request_repaint` on arrival; there is no polling.
- **Packaging** (`packaging/macos`). `Info.plist.in` sets `NSHighResolutionCapable` and declares the document UTIs. `entitlements.plist`; `package.sh` builds a universal or arm64 bundle with `MACOSX_DEPLOYMENT_TARGET=11.0`, then signs (ad hoc `-` by default) and builds the DMG.

**This settles PLAN S8 for egui 0.36.** PhotoCraft runs muda 0.21.1 native menus with eframe 0.36 on macOS, with **no** egui patch: key equivalents are fed back into egui. Research 06 found precedent only with a patched egui. PdfCraft also vendors patched `winit` and `egui-wgpu` 0.36.2 (Windows DPI, surface size limits, IME). Those patches matter for Windows and Linux (M4), not for the M1 Mac.

### 1.4 What OpenLaser copies, adapts or skips

Both PhotoCraft and OpenLaser are MIT OR Apache-2.0, so code can be ported with a notice. Port behaviour first. Bring code over only where it is small and self-contained.

| PhotoCraft piece | OpenLaser | Notes |
|---|---|---|
| `logic`/`ui` split, `UiState` plain data, `Services` injection | **Copy** | `ol-ui` stays free of platform I/O and testable headless |
| `control.rs`: dispatcher, `ControlRequest`, `Dispatch`, deadline semantics, allow-listed `ui.set` | **Adapt** | Add the human-only fence (§7.3), `laser.stop`, and `ui.elements` (§8) |
| `control_server.rs`, token handling, connection limiter | **Copy** | Loopback only. The token goes in a file, never on argv |
| `assistant_bridge.rs` (`--mcp-bridge` in the GUI binary) | **Adapt (open question O2)** | Prefer starting `openlaser-cli mcp --bridge …`, so rmcp and tokio stay out of the binary that links `ol-arm` |
| `menu_catalog.rs` + `menus.rs` + `parity.rs` | **Copy the pattern** | A LightBurn catalog (§4.2), a parity floor and `cargo xtask parity` |
| `native_menu.rs` model + `mac_menu.rs` (muda 0.21.1, exact pin) | **Copy** | LightBurn uses the macOS menu bar on Mac. Keep `--in-window-menus` |
| `shortcuts::parse`, dispatcher | **Copy** | LightBurn defaults (§10) |
| `icons.rs`, Tokens and `ThemeKind` structure | **Adapt** | Fewer presets (§9). Lucide plus laser glyphs |
| `panel_docking.rs` (`DockArea` adapter, registry, locked workspace, `ui.dock.*`) | **Adapt** | The OpenLaser panel registry (§4.7) |
| Custom title bar, GPU canvas, colour management, tablet, Camera Raw, i18n catalogue compression, web build | **Skip** | Not needed for a vector laser app. A standard decorated window is enough for M1 |
| kittest harness patterns, AccessKit queries, `render()` captures | **Copy** | Plus kittest snapshots (§12) |

---

## 2. egui version and dependency constraints

1. **egui 0.36 is mandatory while OpenLaser uses craft-ui.** craft-libs' root `Cargo.toml` pins `egui = { version = "0.36", default-features = false }`, and `craft-ui` depends on it at every cached revision. Cargo cannot unify two egui majors in one `Ui`. Pin the workspace to `egui`, `eframe`, `egui_extras` and `egui_kittest` **0.36** (0.36.2 is in the local registry; it needs rustc 1.95, and we have 1.99).
2. **Upgrading egui is a suite decision.** Move to 0.37 only after craft-libs moves. Then re-run `cargo tree -d` (PLAN S4: one egui and one wgpu across eframe, craft-ui and egui_kittest).
3. **PLAN corrections.** §4/D6 "egui 0.37 + egui_tiles 0.18" becomes **egui 0.36 + craft_ui::docking**. egui_tiles 0.18 needs egui 0.37 and cannot be used. egui_dock 0.21.1 is not needed.
4. **egui 0.36 API notes, verified in the 0.36.2 source and in PhotoCraft:**
   - `eframe::App` has `logic(&mut self, ctx, frame)` and `ui(&mut self, ui: &mut egui::Ui, frame)`.
   - Panels are `egui::Panel::{top,bottom,left,right}(id)…show(ui, …)`.
   - `egui::Modal` sets the top modal layer at `Order::Foreground`. Input then reaches only layers at or above it (`Memory::is_above_modal_layer`), so an `Area` with `Order::Tooltip` stays interactive above a modal. The STOP overlay relies on this (§7.1).
   - `egui::Scene` exists, but it scales strokes and widgets with the zoom (§5.3).
   - AccessKit 0.24. egui-wgpu 0.36.2 → wgpu 30.
   - `egui_extras::TableBuilder` is always compiled.
   - egui_kittest features: `wgpu`, `snapshot`, `eframe`.
5. **The renderer is wgpu.** Use eframe features `wgpu` and `accesskit`. AccessKit is required by VoiceOver (S6), `ui.elements` (§8) and kittest queries.
6. **Duplicate kurbo/usvg.** `egui_extras[svg]` → resvg 0.45.1 → usvg 0.45.1 → **kurbo 0.11**. If `ol-io` takes usvg 0.48 and `ol-geom` kurbo 0.13 (PLAN §3.1), the tree carries two usvg and two kurbo versions, which breaks S4's "one kurbo". Either align on usvg 0.45 / kurbo 0.11, or exempt the icon loader path from S4 (open question O5).
7. **Offline availability** (disk and network are tight):
   - The three craft-libs revisions are cached in `~/.cargo/git/db`.
   - `craft-settings` needs `dirs 7`, which is not in the local registry. The first build that includes it needs the network.
   - `craft-quit` feature `menu` needs muda **0.21.2**; only 0.21.1 is cached, and PhotoCraft pins `=0.21.1`. Use `craft-quit` with feature `egui` only, and build the menu the PhotoCraft way (own muda `=0.21.1` + `QUIT_MENU_ID`).
8. **Fonts.** egui's `default_fonts` feature embeds about 1.4 MB uncompressed. PhotoCraft disables it for its web size gate. OpenLaser is desktop-only, so it may keep the feature in M1 (attribution rows for Ubuntu Light, Hack, Noto Emoji and emoji-icon-font).

---

## 3. Where the UI sits in the crate map

The PLAN §3.1 crates map onto the Craft layout as follows. Final names are SPEC's call.

| Craft (PhotoCraft) | OpenLaser (PLAN name) | Role |
|---|---|---|
| `crates/engine` (`Session::execute`, command registry) | `ol-app` | `AppState`, the command registry (id, label, menu path, shortcut, params, `enabled`, `run`), events. No egui, no `ol-arm` |
| `crates/automation` (rmcp MCP server, headless, bridge client, `AuthorizedWorkspace`) | `ol-automation` (new) | MCP tools for the CLI. **Must not depend on `ol-arm`** |
| `crates/ui-egui` | `ol-ui` | `OpenLaserApp: eframe::App`, panels, canvas, `control.rs`, the ARM widget (the one GUI user of `ol-arm`) |
| `apps/photocraft` | `apps/openlaser` (binary) | `main.rs`, `control_server`, `mac_menu`, platform services, packaging |
| `apps/photocraft-cli` | `openlaser-cli` (agent CLI and MCP) plus `ol` (hardware CLI with TTY ARM) | See O2: one binary must not both serve MCP and mint ARM |

The layering rules carry over from PLAN: only `ol-ui` uses egui and wgpu, and only `ol-lmc-usb` uses nusb. The ARM reverse-dependency check (`xtask`: `ol-arm` ← {`ol-ui`, `ol-cli`}) must count `apps/openlaser` as part of the GUI. Either the binary is the only reverse dependency, or `ol-ui` is and the binary is allowed transitively. SPEC must state which.

---

## 4. LightBurn's layout on the Craft shell

### 4.1 Window regions and the egui container for each

```
+--------------------------------------------------------------------------------------------+
| macOS menu bar (muda): OpenLaser | File Edit Tools Arrange Laser-Tools Window Language Help   |
+--------------------------------------------------------------------------------------------+
| Main toolbar  [New Open Import Save | Undo Redo | Cut Copy Paste | Pan Zoom+ Zoom- Page Sel |  |
|               Settings Device | Trace | Preview | Camera]          (state chip)  [ STOP ]    |
+--------------------------------------------------------------------------------------------+
| Numeric edits: (9-pt) XPos[  ] YPos[  ] mm | Width[  ] Height[  ] mm | [ ]% [ ]% (lock) |   |
|                Rotate[  ] deg              || Text options (contextual): Font Height HSpace..|
+----+----------------------------------------------------------+----------------------------+
| T  |  top ruler (mm, origin = field centre)                   | Cuts / Layers              |
| o  |  +----------------------------------------------------+  | #  Layer Mode Spd/Pwr O S A|
| o  |l | canvas: grid; encodable field +-F/2 (faint);       |  | 00 C00 Fill 1000/80  x x   |
| l  |e | work area 200x200 (rated lens field); centre cross;|  | 01 C01 Line  300/100 x x   |
| s  |f | objects in layer colours; selection and handles;   |  | ...                        |
|    |t | frame overlay; refusal markers                     |  | [quick editor: Mode Speed  |
| S  |  |                                                    |  |  (actual) Power kHz ns     |
| e  |r |                                                    |  |  Passes Interval Angle]    |
| l  |u |                                                    |  |                [Edit...]   |
| e  |l |                                                    |  +----------------------------+
| c  |e |                                                    |  | [Laser|Library|Move|Console]
| t  |r |                                                    |  | Devices [JPT M7 LMC v] Ready
| .. |  |                                                    |  | [Pause] [Stop] [Start]     |
|    |  |                                                    |  | [Frame] [Send] [Home] [Org]|
|    |  +----------------------------------------------------+  | Start From [Absolute v] (9)|
|    |                                                          | [ ] Cut Selected Graphics  |
|    |                                                          | [ ] Use Selection Origin   |
|    |                                                          | [x] Optimize Cut Path      |
+----+----------------------------------------------------------+----------------------------+
| Arrange: [Group Ungroup FlipH FlipV Align.. Distrib..] | Palette 00 01 02 .. 29 | T1 T2    |
+--------------------------------------------------------------------------------------------+
| x 12.34  y -5.60 mm | 2 shapes 40.0 x 25.0 mm | JPT M7: Ready | est 0:42 | 1 refusal | agent |
+--------------------------------------------------------------------------------------------+
```

`Org` is Go to Origin. In the layer table, `O S A` are the Output, Show and Air columns.

| Region | egui container (in this order inside `ui()`) | craft-ui component | Notes |
|---|---|---|---|
| Menu bar | muda native (`mac_menu`); on Win/Linux, or with `--in-window-menus`: `Panel::top("menu_bar")` with `egui::MenuBar` | — | Same catalog either way (§4.2) |
| Main toolbar | `Panel::top("main_toolbar")` | `IconButton` per command; `layout::fit_toolbar` per group with an overflow chevron | The right end holds the machine state chip and the fixed STOP (§7.1) |
| Numeric edits + Text options | `Panel::top("props_bar")`. The text group shows only for the Text tool or a selected text object | Own numeric field widget (craft-ui has none yet); `fit_toolbar` | LightBurn shows these as two toolbars on one row **(verify)** |
| Status bar | `Panel::bottom("status_bar")` | — | Added before the palette so it takes the outermost row |
| Arrange toolbar + colour palette | `Panel::bottom("palette_bar")` | `IconButton`; swatches are own painter cells with AccessKit labels ("Layer 02") | LightBurn docks Arrange at the bottom by default **(verify)**; any toolbar can be moved to the top later |
| Creation and modify tools | `Panel::left("tools")`, fixed width (36 pt compact, 44 pt normal) | `IconButton` with `.selected(tool == t)`; the left tool palette mirrors PhotoCraft's toolbar | Groups separated by rules |
| Canvas + docks | `CentralPanel` → one `DockArea` whose root holds `canvas` (PROTECTED) and the dock panels | `docking::DockArea::show_customized` | Users can dock panels left, right or bottom of the canvas, or float them, as in LightBurn |
| Cut Settings Editor, Preview, Live Framing, Device Settings, ARM confirmation | `egui::Window` / `egui::Modal` (§6, §7) | `tabs::Tab` for the editor tabs; `split_rect` + `ResizeHandle` for the sub-layer list | No modal while framing or emitting, except the ARM confirmation itself, which STOP stays above |
| STOP overlay | `egui::Area` at `Order::Tooltip`, shown only while framing, armed or emitting | — | Its rectangle is passed as `DockArea::blocked_rects` |

### 4.2 Menus: the LightBurn catalog

Model it on `menu_catalog.rs`: `pub static CATALOG: &[(&[&str], &str, Option<&str>, &str)]` = (path, label, default shortcut, command id), in LightBurn order, with `"---"` separators.

- A row is live when `ol-app` registers its id. Otherwise it renders disabled with the tooltip "Not in OpenLaser yet".
- `parity.rs` keeps a `FLOOR` that only rises. `cargo xtask parity` writes `docs/parity-checklist.md`.
- Shortcuts below are LightBurn's Windows defaults, with Ctrl becoming ⌘ on macOS. All of them are **(verify)**.

| Menu | v0 items (ids in brackets; M1.3 live items in **bold**) |
|---|---|
| **OpenLaser** (macOS app menu, built by `native_menu`) | About (`help.about`), Settings… ⌘, (`edit.settings`), Language ▸, Appearance ▸, Hide/Show, **Quit** (`file.exit`, which goes through the close guard, §7.5) |
| **File** | **New** Ctrl+N (`file.new`), **Open…** Ctrl+O (`file.open`), Open Recent ▸, **Import…** Ctrl+I (`file.import`), --- **Save** Ctrl+S, **Save As…** Ctrl+Shift+S, Export… (`file.export`: SVG/DXF), Save Processed Bitmap…, --- Save GCode… (`file.saveGcode`, M2), Run GCode… (gantry), --- Import Prefs…, Export Prefs…, --- Print… (T2), --- Exit |
| **Edit** | **Undo** Ctrl+Z, **Redo** Ctrl+Shift+Z, --- **Select All** Ctrl+A, Invert Selection, Select Open Shapes, Select Open Shapes Set to Fill, Select All Shapes in Current Layer, Select Contained Shapes, Select Smaller Than…, --- Cut Ctrl+X, Copy Ctrl+C, **Duplicate** Ctrl+D, Paste Ctrl+V, Paste in Place Alt+V, **Delete** Del, --- Convert to Path Ctrl+Shift+C, Convert to Bitmap, Close Path, Close Selected Paths with Tolerance, **Auto-Join Selected Shapes** Alt+J (prototype "Join connected"), Optimize Selected Shapes, Delete Duplicates Alt+D, Image Options ▸, --- Notes…, --- Settings… (`edit.settings`), **Device Settings…** (`edit.deviceSettings`), Machine Settings… (gantry, M4) |
| **Tools** | **Select**, Edit Nodes Ctrl+\`, Draw Lines Ctrl+L, **Rectangle** Ctrl+R, **Ellipse** Ctrl+E, Polygon, Text Ctrl+T, Position Laser, Measure, Tabs, --- Offset Shapes Alt+O, Weld Ctrl+W, Boolean Union / Subtract / Intersection, Boolean Assistant, --- Apply Path to Text, Apply Mask to Image, Cut Shapes, Break Apart Alt+B, --- Trace Image Alt+T, --- Grid Array, Circular Array, Copy Along Path, Create Rubber-band Outline, Warp / Deform, Set Start Point, Barcode…, Variable Text… |
| **Arrange** | Group Ctrl+G, Ungroup Ctrl+U, Auto-Group, --- **Flip Horizontal** Ctrl+Shift+H, **Flip Vertical** Ctrl+Shift+V, Mirror Across Line, **Rotate 90° CW / CCW**, --- Align ▸ (Left, Right, Top, Bottom, H-centre, V-centre, Centres), Distribute ▸, Move Together ▸, Dock ▸, **Move Selected Objects ▸** (page corners and centre; prototype "Center Selection"), Move Laser to Selection ▸, --- Make Same Width / Height, Resize Slots, Nesting ▸ (Quick, T2), --- Lock / Unlock Selected Shapes |
| **Laser Tools** | Material Test… (frequency × Q-pulse grid; T0-stretch/M2), Interval Test, Focus Test (gantry), Center Finder (gantry), --- **Galvo Lens Calibration ▸ Calibrate Axis…** (M2, prototype 1.1834), Red Dot Calibration…, Cylinder Correction… (T2), Rotary / Split Marking… (T2), --- Print and Cut ▸ (gantry), Camera ▸ Calibrate Lens / Alignment (T2), Rotary Setup… |
| **Window** | **Cuts / Layers**, **Laser**, Library, Move, **Console**, Camera Control, Shape Properties, Variable Text, Undo History, Art Library, File List, **Live Framing**, **Preview** Alt+P, --- Toolbars ▸ (Main, Tools, Arrange, Numeric Edits, Text Options, Colour Palette, Status Bar), --- Lock Window Layout, Reset to Default Layout, --- Theme ▸ (System, Light, Dark) |
| **Language** | **English**, **Português (Brasil)**, then others as catalogues land |
| **Help** | OpenLaser Manual (local docs), Keyboard Shortcuts, --- **Laser Safety…** (the first-run screen, PLAN §3.6 layer 0), Export Diagnostics… (D16), --- Check for Updates (T2), About |

Command id convention, as in Craft: `menu.item` in camelCase, for example `file.import`, `tools.rect`, `arrange.flipHorizontal`, `window.cuts`, `laser.frame`, `laser.start`, `laser.stop`. A command can appear in a menu, a toolbar and a panel; every place dispatches the same id. **Laser-window actions (§4.9) are commands too, but some are human-only (§7.3).**

### 4.3 Main toolbar

The groups are fitted by `fit_toolbar`; groups are indivisible, and anything that does not fit goes to an overflow chevron menu. Never drop a command silently.

| Group | Buttons (command id · Lucide icon) |
|---|---|
| File | New `file.new` · `file-plus`; Open `file.open` · `folder-open`; Import `file.import` · `file-input`; Save `file.save` · `save` |
| History | Undo `edit.undo` · `undo-2`; Redo `edit.redo` · `redo-2` |
| Clipboard | Cut · `scissors`; Copy · `copy`; Paste · `clipboard-paste` |
| View | Pan `view.pan` · `hand`; Zoom In · `zoom-in`; Zoom Out · `zoom-out`; Zoom to Page `view.fitPage` · `maximize`; Zoom to Selection `view.fitSelection` · `scan` |
| Setup | Settings `edit.settings` · `settings`; Device Settings `edit.deviceSettings` · `cpu` |
| Tools | Trace Image · `wand-sparkles` (T2); Preview `window.preview` · `scan-eye`; Camera · `camera` (T2) |
| Machine (right-aligned, never in overflow) | State chip (Disconnected / Ready / Framing / Armed / Marking / Fault / STOP NOT CONFIRMED) and the **STOP** button (§7.1) |

### 4.4 Numeric edits (shape-properties toolbar)

Fields: a 9-point origin selector (which point of the selection's bounds XPos/YPos refer to), **XPos**, **YPos**, **Width**, **Height**, **W%**, **H%**, a lock-aspect toggle, **Rotate** (°), and a units toggle (mm, or in for display only).

- **Coordinates.** Values are in mm, Y-up. On a galvo the origin is the field centre (PLAN §3.2), so XPos/YPos run from −100 to +100 on the 200 mm field.
- **Commit.** Fields commit on Enter or focus loss, as one undoable command (`transform.set {x?, y?, w?, h?, rotate?, anchor}`). Esc reverts the field, **except while the machine is active**, when Esc is STOP (§7.2).
- **Numeric widget (own).** Bounded, finite-only parsing. Units can be typed (`12.5`, `1in`, `3/8in`) using PhotoCraft's `numeric_expression` idea. It refuses non-finite or out-of-range input with an inline message rather than clamping silently.
- **Multiple selection.** Values apply to the group bounding box, as in LightBurn.
- **Live safety check.** If a value would push geometry outside the work area, the field accepts it, but the live `ol_safety::validate` turns it into a refusal shown on the canvas and in the Laser panel. Geometry is never clamped.

### 4.5 Text options toolbar (contextual, T1)

Font (system TTF/OTF; single-line SHX/Hershey later), Height, HSpace, VSpace, Align X (L/M/R), Align Y (T/M/B), Offset, Bold, Italic, Upper Case, Welded, Distort, Variable Text mode. It shows only for the Text tool or a selected text object. The M1 prototype has a stroke font for test labels only, so this toolbar arrives with text in M2.

### 4.6 Left tool palette

Order follows LightBurn's creation column, then its modify column **(verify)**. Each button is an `IconButton` whose AccessKit label is the command label.

| Tool | Command | Icon | Tier |
|---|---|---|---|
| Select | `tools.select` | `mouse-pointer-2` | M1.3 |
| Edit Nodes | `tools.editNodes` | `spline` | T1 |
| Draw Lines | `tools.drawLines` | `pen-tool` | T1 |
| Rectangle | `tools.rect` | `rectangle-horizontal` | M1.3 stretch (test pieces) |
| Ellipse | `tools.ellipse` | `circle` | M1.3 stretch |
| Polygon | `tools.polygon` | `pentagon` / `hexagon` | T1 |
| Text | `tools.text` | `type` | M2 |
| Position Laser | `tools.positionLaser` | `crosshair` | T1 (gantry; on galvo, the red dot to a point, **human-only**) |
| Measure | `tools.measure` | `ruler` | T1 |
| Tabs | `tools.tabs` | original glyph | T1 (gantry) |
| — modify — | | | |
| Offset Shapes | `tools.offset` | original glyph | T1 |
| Weld | `tools.weld` | original glyph | T1 |
| Union / Subtract / Intersect | `tools.boolean.*` | `squares-unite` / `squares-subtract` / `squares-intersect` | T1 |
| Mirror Across Line | `arrange.mirrorLine` | `flip-horizontal-2` variant | T1 |

### 4.7 Dock: panel registry and default layout

Panel ids are stable and never localised. Labels go through `tl!`.

| Id | Label | Default place | M1.3 | Content |
|---|---|---|---|---|
| `canvas` | (none) | centre, `Permissions::PROTECTED` | yes | §5 |
| `cuts` | Cuts / Layers | right, top | yes | §4.8 |
| `laser` | Laser | right, bottom tab group | yes | §4.9 |
| `library` | Library | right, bottom tab group | stub | Material library (`.clb`), T1 |
| `move` | Move | right, bottom tab group | stub | Gantry jog (M4). On LMC galvo, LightBurn's Move window is EZCad3/BSL only (GLV-18), so this shows an explanation |
| `console` | Console | right, bottom tab group | yes | Tabs: Log, USB trace (decoded packets, from FakeLmc in tests), G-code console (M4). Read-only for galvo |
| `camera` | Camera Control | hidden | — | T2 |
| `shapeProperties` | Shape Properties | hidden | — | Power scale, cut-order priority (T1) |
| `variableText` | Variable Text | hidden | — | T1 |
| `undoHistory` | Undo History | hidden | — | T2 |
| `artLibrary` | Art Library | hidden | — | T2 |
| `fileList` | File List | hidden | — | DSP only (T2) |
| `liveFraming` | Live Framing | floating when opened | yes | §4.10 |
| `preview` | Preview | floating when opened | M2 | §4.10 |

The default layout uses craft-ui's types, with right-dock widths as in LightBurn:

```rust
Layout {
    root: Some(Node::Split {
        axis: SplitAxis::Horizontal,
        size: SplitSize::FixedSecond(340.0),
        first: Box::new(Node::Tabs { panels: vec!["canvas".into()], active: 0 }),
        second: Box::new(Node::Split {
            axis: SplitAxis::Vertical,
            size: SplitSize::Ratio(0.48),
            first: Box::new(Node::Tabs { panels: vec!["cuts".into()], active: 0 }),
            second: Box::new(Node::Tabs {
                panels: vec!["laser".into(), "library".into(), "move".into(), "console".into()],
                active: 0,
            }),
        }),
    }),
    floating: vec![],
}
```

Rules, adapted from `panel_docking.rs`:

- **Validation and permissions.**
  - `valid(layout)` = `layout.validate().is_ok()` plus every id in the registry plus `canvas` present exactly once at the root level. An invalid saved layout falls back to the default with a status message.
  - Permissions: `canvas` → `PROTECTED`. `laser` → `movable`, `floatable`, `closable: true` (as in LightBurn), because STOP does **not** depend on it (§7.1). Everything else → `Permissions::default()`, or all false when Window › Lock Window Layout is on.
  - Limits: `cuts` min 260×220 pt, `laser` min 260×300 pt, `canvas` min 320×240 pt.
- **Apply after render.** `DockOutput.actions` are applied after rendering through `Layout::apply`, the same path as `ui.dock.*` control commands (`Action` is serde). Hiding a panel stores `Layout::location(panel)` so the panel can be restored to the same place; when the anchor is gone, the fallback is the right column.
- **`DockStyle` from Tokens.** `tab_height` is 26 compact and 32 normal. Localise `float_label`, `close_label`, `move_label`, `panels_label`, `resize_label` and `resize_window_label`.
- **Esc and dragging.** When the machine is active, Esc is consumed for STOP before `DockArea` sees it, and any dock drag is cancelled (`cancel_drag`).

### 4.8 Cuts / Layers panel

**Table** (`egui_extras::TableBuilder` with a striped body and fixed row height). Columns follow LightBurn: **#** (colour swatch with the 2-digit index, or T1/T2), **Layer** (name), **Mode** (Line / Fill / Offset Fill / Image / Multi; Tool for T1/T2), **Spd/Pwr** (for example `1000 / 80`), **Output** (checkbox), **Show** (checkbox) and **Air** (checkbox).

- **Air is hidden when the active device is a galvo**, which has no air assist.
- **Spd shows the *actual* encoded speed** (PLAN D9). Its tooltip gives requested vs actual, plus kHz and ns.

**Rows.**
- Rows are ordered by priority. Drag reorders. Up/down arrow buttons sit at the right edge **(verify)**. Click selects the layer and fills the quick editor. Double-click opens the Cut Settings Editor (§6).
- Context menu: Select All Shapes in Layer, Disable Output on Others, Hide Others, Copy / Paste Settings, Flash Layer Content (blinks its shapes on the canvas), Sort Cuts Last (T1).
- **Refusal badge.** A layer `ol_safety` refuses gets a red badge with the refusal code; hovering lists the reasons, for example "pulse 7 ns not in the machine's pulse table", "hidden layer with Output on", "Fill layer yields no hatch" or "speed encodes to 0 units". Clicking it focuses the field. Nothing is clamped or skipped.

**Quick editor** (below the table; the split is a `layout::split_rect` with a `ResizeHandle`). For galvo/MOPA: Mode, Speed (mm/s) with an "actual" readout, Power (%), Frequency (kHz), Q-Pulse (ns, a drop-down of the profile's table), Passes; for Fill also Interval (mm) and Angle (°); then an **Edit…** button for the full editor. Each change is one undoable `layer.set` command.

**Layer model mapping.**
- LightBurn's palette indices 00–29 map to cut layers C00–C29. **T1/T2 are tool layers**, which never output. PLAN's `LayerMode::FrameOnly` from the prototype becomes a tool layer that the Live Framing option *Tool Layers Only* (GLV-08) can trace. This keeps "frame-only never burns" structural (open question O4).
- The project stores both galvo and gantry cut blocks per layer (02 §7.7). The table shows the active device's block.

### 4.9 Laser panel (galvo first)

Top to bottom **(verify the order against LightBurn's Laser window)**:

1. **Device row.**
   - **Devices…** opens Device Settings (§4.11), which edits the machine profile; agents may edit it.
   - The device drop-down (for example "JPT M7 · LMC 0x9899") shows no port selector for galvo.
   - **Connect / Disconnect** opens and closes the USB device; **human-only**.
   - The status chip shows Disconnected / Ready / Busy n% / Framing / Armed / Marking / Fault (sticky) / **STOP NOT CONFIRMED** (sticky red banner: "use the key switch / E-stop", PLAN §3.5).
2. **Job buttons**, as in LightBurn: **Pause**, **Stop**, **Start**; second row **Frame**, **Send**, **Home**, **Go to Origin**.
   - **Start** is human-only. It requires a FrameReceipt for the current fingerprint (one full pass at most 10 minutes old); without one it says "Frame this job first". With one, it opens the ARM confirmation (§7.4). Disabled while refusals exist.
   - **Frame** is human-only. It opens Live Framing (§4.10) and starts the red-light frame of the current plan.
   - **Pause** is T1 for galvo; disabled with a tooltip in M1.
   - **Stop** is always enabled. It mirrors the fixed STOP (§7.1).
   - **Send** is for DSP controllers only (CTL-12); disabled with the tooltip "Not available for this device".
   - **Home** and **Go to Origin**: disabled on LMC galvos with an explanation. Gantry, M4.
3. **Start From**: Absolute Coords / User Origin / Current Position. For galvos LightBurn always uses Absolute Coords (CTL-03), so the drop-down is disabled with that explanation. **Job Origin** (9-point): same, disabled for Absolute.
4. **Checkboxes.**
   - **Cut Selected Graphics** (prototype "Selected only") and **Use Selection Origin**.
   - **Optimize Cut Path** with **Optimization Settings…** (OPT-*, T1).
   - The galvo job counter (Repeat N, Run Continuously; GLV-09): Repeat N in M2, continuous T1.
5. **Estimate.** Time, mark distance and jump distance from `ol-plan`, so they never drift from the bytes.
6. **Refusals and notices list.** Every refusal from `ol_safety::validate` with its code; notices are numbered and linked to canvas markers (for example open shapes in a Fill layer).
7. **STOP line:** "STOP works while this window is focused; the E-stop always works." (PLAN §3.5)

The panel body reads only `AppState` plus driver events. It never performs I/O. Connect, frame and start are requests to the driver thread through `ol-app`.

### 4.10 Live Framing, Preview and the other panels

- **Live Framing** (`liveFraming`; LightBurn opens it with F1 on galvos **(verify)**).
  - Style: Bounds / Contour (M1); Hull (T1).
  - Options: Frame Individually, Outside Shapes Only, Tool Layers Only. Controls: Start/Stop framing, Loop.
  - Keyboard nudge, scale and rotate while framing come in T1, and each nudge is an edit that voids any ARM token.
  - It is a non-modal floating dock group, so STOP stays reachable. While framing, the canvas draws the `FramePlan` path in red, dashed and animated.
- **Preview** (`preview`, Alt+P, M2).
  - The planned job from `ol-plan`, not the document: cut moves in layer colour, travel/jump moves red, field boundary green (PRV-01).
  - Stats (PRV-03), a time slider (T1), Shade by power (T1).
  - Floating and non-modal. Rendered from the same display list as the canvas (§5.3).
- **Console** (`console`). Log, plus a USB trace of decoded LMC opcodes from the driver's disassembler, never raw guesses; PLAN §3.3 `EmissionSummary`. A G-code console arrives for GRBL in M4.

### 4.11 Device Settings dialog (galvo)

A window with tabs (`craft_ui::tabs`) mapped onto `GalvoMachine` (PLAN §3.2) and research 02 §5.2:

- **Basic**
  - Field size and offset X/Y. Angle is T1.
  - Work area. It defaults to the rated 200 mm field; it may shrink, and grows only to what H27 cleared. The UI says why it cannot grow.
  - Galvo correction: per-axis scale (1.1834 imported), swap X/Y and negate. Bulge, skew and trapezoid are T1; `.cor` load is M2/M3.
  - Red Dot offset and scale, frame speed, jump defaults and delay defaults (expert view).
  - **Require framing before start** is shown **checked and locked** on galvos (D8), with the note "always required on galvos".
- **Laser source.** Type MOPA fiber, pulse table, frequency range (`MINPWMFREQ`..`MAXPWMFREQ` from `markcfg`), max power, "MO enabled only when running" (locked on).
- **IO ports.** Read-only until H11, then door and foot inputs (M3).
- **Import.** EZCAD2 `markcfg7` (GDEV-01) with a preview of applied vs unused keys, as the prototype's `inspect-profile` does.

Edits go through `ol-machine` validation and are saved with `craft-atomic`. Profiles are safety data, not preferences, so they are **not** stored in craft-settings.

### 4.12 Colour palette bar

- **Swatches.** 30 numbered swatches (00–29) plus **T1** and **T2**. Swatches are fixed-size painter cells; each is an AccessKit button labelled "Layer 05" and so on, so agents and kittest can address them.
- **Click.** With shapes selected, assigns them to that layer (`layer.assign {index}`), creating the cut row if it is absent. With nothing selected, it sets the layer for newly drawn shapes **(verify)**. Hover shows the layer name and its Spd/Pwr.
- **Colours.** The index semantics must match `.lbrn2`. The display colours are not decided yet (O6): ideally measured from Luiz's licensed install for interoperability, or our own close set; record provenance either way.

### 4.13 Status bar

Left to right:

1. Cursor position in mm, Y-up, field-centred.
2. Selection summary: count, bounds W×H, min/max **(verify LightBurn's wording)**.
3. Machine state and name.
4. Job progress with ETA while marking.
5. A refusal count chip; clicking it focuses the Laser panel list.
6. **Agent indicator.** "agent: none / connected (read) / driving", shown whenever a control client is authenticated. It is a transparency feature (§8).

---

## 5. Canvas

### 5.1 Coordinates and view transform

- **World.** mm as `f64`, Y-up. The galvo origin is the field centre (PLAN §3.2). Gantries use the bed corner chosen in the profile (M4).
- **View state** (`ui.views[doc]`, plain data, as PhotoCraft's `View`): `zoom` (screen points per mm, `f64`), `center` (world mm at the canvas centre), `fit_pending`. There is no rotation in v1.
- **Transform:** `screen = canvas_center + (world − center) · zoom · (1, −1)`. Compute in `f64` and convert to `f32` only for emitted vertices. Zoom range: 0.2 to 5000 pt/mm (about 0.04 mm to 4 m visible on a 1000 pt canvas).
- **Fit to Page** frames the encodable field ±F/2 with a 5% margin. Fit to Selection frames the selection bounds.

### 5.2 Paint order (back to front)

1. **Pasteboard** in `Tokens.canvas`, with the **grid** in the work area. Minor and major lines are adaptive, using a 1-2-5 step series as in PhotoCraft's `rulers::nice_step`, at least 8 pt apart; major lines every tenth step.
2. **Encodable field** ±F/2 (F = 236.68 mm): faint dashed outline labelled "encodable".
3. **Work area** (the rated lens field, 200×200 by default): solid outline. Centre cross at (0, 0).
4. **Objects** by layer priority. Wireframe in layer colour by default. The "Filled" view style draws closed shapes with an even-odd fill at 25% alpha (T1, lyon meshes).
5. **Refusal overlays.**
   - Points outside the work area: red dots, with the object outline in a red dash.
   - Open shapes in a Fill layer: numbered notice markers, matching the Laser panel list.
   - Orphan or hidden-output objects: a red bounding box.
6. **Frame overlay** while framing: the `FramePlan` path, red, dashed, animated by dash offset (one repaint per 50 ms only while framing).
7. **Selection:** selected shapes redrawn dashed in a contrast colour, plus the bounding box and handles (§5.6). **Hover:** the hovered shape thickened.
8. **Tool feedback:** rubber-band marquee, shape being drawn, measure readout, snap markers (T1).
9. **Rulers** are painted outside the canvas rect (top strip 16 pt, left strip 16 pt), with a cursor marker.

### 5.3 Rendering pipeline

The display list is UI-free so both egui and tiny-skia can render it. Generating it lives in `ol-app` (or a small `ol-render`), never in `ol-ui`:

```rust
pub struct DisplayList { pub items: Vec<Item> }          // world mm
pub enum Item {
    Polylines { style: StrokeStyle, color: Rgba, lines: Arc<[Arc<[DVec2]>]> }, // per layer, flattened
    Fill { color: Rgba, mesh: Arc<WorldMesh> },          // even-odd, lyon (T1)
    Marker { kind: MarkerKind, at: DVec2 },              // refusal/notice markers
    Rect { style: StrokeStyle, color: Rgba, rect: DRect }// field, work area
}
```

**Flattening.** Done in `ol-geom` with kurbo, **per object and per zoom bucket**: tolerance = 0.25 px / zoom. Buckets are powers of two of the zoom, so a re-flatten happens only when the bucket changes. Results are cached by `(object id, object revision, bucket)` in an LRU bounded by vertex count.

**egui backend (`ol-ui::canvas`).**

1. **Cull.** Query an `rstar` R-tree of object bounds (world mm) with the visible world rect plus a 2 px margin.
2. **Project and decimate.** Transform to screen in `f64`. Drop consecutive points closer than 0.5 px. Clip runs that leave the canvas rect.
3. **Mesh.** Build one `epaint::Mesh` per (layer colour, style) with **our own hairline mesher**. Each segment is a quad of constant screen width, 1 physical px by default (0.5 pt at 2× Retina), with a 1 px alpha feather. Joins are not mitred; hairlines don't need it. This avoids `PathShape` stroke tessellation, the main CPU cost at 200k segments. Normal 1 pt and selected/hover 2 pt widths are user preferences.
4. **Cache.** Keep `Arc<Mesh>` keyed by `(doc revision, layer, view transform, canvas rect, ppp, theme)`. An idle frame paints `Shape::Mesh(arc.clone())` with no work. On a **pan-only** change, translate the cached vertices (O(n) adds, no re-flatten). Zoom-bucket changes re-flatten. Within a bucket, a zoom change re-projects from the flattened cache.
5. **Overlays** (selection, hover, handles, marquee, markers, rulers) are painted separately with `Painter` shapes, so hover never invalidates the big meshes.

**Why not `egui::Scene`.** Scene applies a `TSTransform` to the whole layer. Strokes and handles would then scale with zoom, and positions would be `f32` in scene space. Laser editing needs constant-width hairlines, `f64` world precision and our own hit-testing.

**`CanvasRenderer` trait** (PLAN §4): `fn paint(&mut self, painter: &egui::Painter, list: &DisplayList, view: &ViewXform)`.
- v1 is `PainterRenderer`, as above.
- v2 is `WgpuHairlineRenderer`: `egui_wgpu::Callback` with instanced segment quads expanded in the vertex shader to a constant pixel width (04 §3), on egui's own wgpu 30. It moves to M3 if S1 lands at 16.7–33 ms, otherwise M5.

**Headless backend.** `tiny-skia` renders the same `DisplayList` to PNG for:
- MCP `render_preview`, `openlaser-cli preview` and the CLI, all with no window or GPU;
- golden-image tests in CI;
- the ARM confirmation thumbnail, rendered from `EmissionSummary` geometry, not from the document.

### 5.4 Zoom and pan input (LightBurn-like, configurable)

| Input | Action |
|---|---|
| Mouse wheel / trackpad pinch (`zoom_delta`) | Zoom about the cursor. LightBurn zooms on the wheel; an "invert wheel" preference is available |
| Two-finger trackpad scroll (`smooth_scroll_delta`) | Pan |
| Middle-drag, or Space + left-drag, or the Pan tool | Pan |
| Zoom In / Out buttons and shortcuts | Zoom about the canvas centre, by a factor of √2 |
| Zoom to Page / Selection | Fit (§5.1) |

The canvas reacts to wheel and pinch only while hovered. A docked panel scrolling never zooms the canvas.

### 5.5 Hit-testing and selection

- **Index.** An `rstar` R-tree over *segments*: per object, flattened at the finest cached bucket, with segment AABBs in world mm. It is rebuilt incrementally per edited object (PLAN S2 target: p99 ≤ 2 ms at 20k paths).
- **Pick.**
  - Query a box of cursor ± tolerance, where tolerance = 4 pt / zoom mm. Take the minimum exact point-to-segment distance.
  - In the Filled view style, a click inside a closed shape (even-odd) also picks it.
  - A group picks as a unit.
  - Ties go to the topmost priority layer, then the latest object.
- **Marquee.** A left-to-right drag selects objects **fully enclosed**; right-to-left selects objects **touched**. The two rectangles are drawn differently **(verify LightBurn's convention)**.
- **Modifiers.** Shift-click adds or removes. Ctrl/⌘-click toggles **(verify)**. Esc deselects only when the machine is idle.
- **Double-click.** Selects connected shapes (as the prototype does), or edits text (T1).

### 5.6 Transform handles

As in LightBurn **(verify)**: the first click on a selection shows **scale** handles (8 handles plus the 9-point anchor). Clicking the selection again toggles to **rotate/skew** handles. Dragging the body moves it.

- Shift keeps the aspect ratio. Ctrl/⌘ scales from the centre **(verify)**.
- Arrow keys nudge by 0.1 mm; Shift + arrow by 1 mm (preference).
- Every drag is coalesced into one undo step and dispatched as `transform.apply {matrix, ids}`. `ui.pointer` from agents drives the same tool code.

### 5.7 Performance gates (PLAN §4)

| Spike | Target on the M2 (release build) | Harness |
|---|---|---|
| S1 | p95 frame ≤ 16.7 ms while panning/zooming 20k paths / 200k segments | kittest `build_eframe` with a synthetic document and scripted pointer events; per-frame CPU timing |
| S2 | Hit-test p99 ≤ 2 ms at 20k paths | Pure R-tree benchmark (criterion) |
| S3 | Keypress → `stop()` ≤ 16 ms p95, ≤ 50 ms p99, under S1 load plus a 500 ms plan | Scripted Esc through `raw_input_hook`. **New cases:** an in-window menu open, the native menu open (manual: AppKit menu tracking blocks the egui loop), and a live resize |

---

## 6. Cut Settings Editor (galvo / MOPA)

LightBurn opens it by double-clicking a layer. OpenLaser shows an `egui::Window` that is modal only while the machine is idle; while framing or emitting it opens read-only, and STOP stays above it.

```
+-- Cut Settings Editor: C02 ---------------------------------------------------------+
| Sub-layers (T1)   | Name [ Deep mark      ]  Output [x]   Mode [ Fill        v]      |
| > 1 Fill          | [ Common ] [ Advanced ]                                          |
|   2 Line          | Speed (mm/s)   [ 1000   ]   actual 999.82 mm/s (12 units)        |
| [+][dup][-]       | Power (%)      [ 80     ]   max 100 (source)                     |
| Global passes [1] | Frequency (kHz)[ 30     ]   allowed 1 - 4000                      |
|                   | Q-Pulse (ns)   [ 200   v]   JPT M7 table: 2 4 6 9 13 20 30 45 ...  |
|                   | Passes         [ 1      ]                                        |
|                   | -- Fill ------------------------------------------------------   |
|                   | Interval (mm) [ 0.05 ] <-> LPI [ 508 ]  Angle (deg) [ 0 ]        |
|                   | [x] Bi-directional  [ ] Cross-hatch  Angle increment [ 90 ]/pass |
|                   | [ ] Flood fill (T1) [ ] Auto-rotate (T2) [ ] Dot width corr. (T1)|
|                   | -- Line ------------------------------------------------------   |
|                   | Wobble (T1): [ ] Enable  Step [ 0.05 ] mm  Size [ 0.3 ] mm       |
| ------------------+----------------------------------------------------------------- |
| refusals: none    | [Make Default] [Make Default (all)] [Reset]      [Cancel] [ OK ] |
+-------------------------------------------------------------------------------------+
```

| Field | Unit / widget | Validation (`ol_safety`, live) | Tier |
|---|---|---|---|
| Name, Output | text, checkbox | Hidden layer + Output on → refusal | M1 |
| Mode | Line / Fill / Offset Fill (T1) / Image (M3); a Tool layer has no mode | A Fill layer that yields no hatch → refusal | M1 |
| Speed | mm/s numeric; shows **actual** speed and whole units | Requested speed kept; encoded once; **0 units → refusal** (B7) | M1 |
| Power | % | Above the source maximum → refusal | M1 |
| Frequency | kHz numeric, stored as Hz (`.lbrn2` compatible) | Outside the profile's `MINPWMFREQ..MAXPWMFREQ` (1–4000 kHz for this machine) → refusal. Pulse-dependent limits are UNVERIFIED for the M7 and shown as a notice only after confirmation | M1 |
| Q-Pulse width | ns drop-down of the profile's `PULSEWIDTHS` (JPT M7 default: 2, 4, 6, 9, 13, 20, 30, 45, 55, 60, 80, 100, 150, 200, 250, 300, 350, 400, 450, 500) | Off-table within ±0.5 ns → snap with a notice; otherwise refusal | M1 |
| Passes | integer ≥ 1 | — | M1 |
| Interval / LPI | mm ↔ lines per inch, linked | > 0, finite; hatch must produce lines | M1 (Fill) |
| Angle; Angle increment per pass | ° | finite | M1 (Fill) |
| Bi-directional, Cross-hatch | checkboxes | — | M1 (prototype is always serpentine; the unidirectional option is new) |
| Flood fill, Dot width correction, Auto-rotate | checkboxes | — | T1 / T1 / T2 |
| Wobble: enable, step, size | mm | finite, step > 0 | T1 ("wobble later") |
| **Advanced**: Override default timings: Pause before/after (ms), Jump speed, Min/Max jump delay (µs), Jump distance limit, Laser On / Off / End / Polygon TC (µs) | Expert numeric | Range from the profile; read-only showing profile defaults in M1 | M2+ |
| Sub-layers (up to 11), Global passes | List with Add / Duplicate / Delete and drag | Total passes = global × per-sub-layer | T1 |

- **Tooltips** carry the EZCAD vocabulary (GLV-20), for example "Cross-hatch ≈ EZCAD hatch 2 at +90°", because Luiz knows EZCAD.
- **Footer buttons.** OK dispatches one `layer.setCut {layer, cut}` command, a single undo step. Make Default writes the per-device defaults (M2).
- **Agent access.** It is `ui.dialog.open {kind: "cutSettings", layer}` / `ui.dialog.set` / `ui.dialog.confirm`, which is allowed: it edits the document, not the machine.

---

## 7. Safety UI

### 7.1 STOP is fixed chrome

STOP lives in three places, and none of them can be hidden while the machine is active:

1. **The right end of the main toolbar.** It is never fitted into overflow: `fit_toolbar` measures the machine group outside the overflowable groups.
2. **An `egui::Area` at `Order::Tooltip`**, drawn after everything else while state ∈ {Framing, Armed, Marking, Stopping, StopNotConfirmed}. It sits above any `Modal` (§2.4). Its rectangle goes to `DockArea::blocked_rects`, so no drag or drop lands under it.
3. **The Laser panel's Stop**, which may be closed. It is a convenience only.

STOP is **never disabled**, and it is always a real button with the AccessKit label "Stop laser". A kittest checks this for every state × every panel layout × modal open × narrow window (§12).

### 7.2 Esc is consumed before widgets

- **Esc → latch in `raw_input_hook`.** It runs before egui begins the pass. If the machine is active and `raw_input.events` contains `Key { key: Escape, pressed: true }`, call `stop_latch.trip()` (an atomic store; the driver reacts within 20 ms), then remove the event so no widget or dialog consumes it.
- **No modal file dialogs while active.** `rfd` dialogs are synchronous on macOS and block the egui thread, so Import, Open and Save are disabled while framing or emitting, and use `AsyncFileDialog` otherwise (PLAN §3.5).
- **Remaining macOS blind spots are measured in S3 and documented:** AppKit menu tracking (a native menu held open) and live resize. If S3 fails during resize, the window becomes non-resizable while active (PLAN §3.5). The E-stop is the primary device.

### 7.3 Human-only commands and the synthetic-input fence

The Craft control channel can do far more than call commands. It can click anywhere (`ui.click`), type (`ui.type`), press keys (`ui.key`), invoke any menu item (`ui.menu.invoke`) and confirm dialogs (`ui.dialog.confirm`). AGENTS.md rule 1 says agents never emit and never open the laser USB device. OpenLaser therefore adds a fence in the GUI process itself, not in the MCP server, since `control_call` passes any method through:

- **Human-only registry** (`ol-app`). Each such command carries `human_only: true`: `laser.connect`, `laser.disconnect`, `laser.frame` (red light), `laser.start` (opens ARM), `laser.resume`, `laser.home`, `laser.goToOrigin`, `laser.move.*`, `tools.positionLaser` (galvo), `device.flashFirmware` (never exists; PLAN D15).
  - `engine.commands` lists them with `humanOnly: true, enabled: false, reason`.
  - `engine.execute` and `ui.menu.invoke` from the control channel refuse them with `error: "human-only: <id> drives the laser hardware; ask the operator"`. This is the "refuse, never warn" rule.
- **Allowed for agents:** everything that edits or plans the document or profile, `render_preview`, `laser.status` (read), and **`laser.stop` / `laser.pause`, which are the safe direction.** An agent can always stop.
- **Synthetic-input fence.** PhotoCraft already marks frames that contain injected events (`automation_input = true` in `take_synthetic_step`). OpenLaser extends this:
  - Human-only widgets (Connect, Frame, Start, ARM field and button, Device "Connect" and the checklist ticks) render as normal but are **inert in any frame that contains synthetic events**, and for 1 s afterwards.
  - `ui.click/move/key/type/pointer` are **refused** while a human-only surface has focus or the ARM confirmation is open.
- **AccessKit fence.** macOS accessibility clients, including agent "computer-use" tools that press buttons through AX, reach egui as `Event::AccessKitActionRequest`. Human-only widgets ignore AccessKit `Click`/`SetValue`/`Focus` requests. The ARM text field ignores AX `SetValue`. They still expose role and label to VoiceOver, and real keyboard and pointer still work. This trades a little accessibility on two controls for safety. Record it in `ACCESSIBILITY.md`.
- **What the fence cannot stop:** real OS-level input (CGEvents) from a process that controls the mouse and keyboard. That is why PLAN §3.6 keeps "USB unplugged while executors work", the bench-session file and, from M3, the physical foot-switch ARM. The status bar's agent indicator (§4.13) at least makes a connected agent visible.

### 7.4 ARM confirmation

It is opened by a human pressing Start, with a fresh FrameReceipt. It is an `egui::Modal`; STOP stays above it (§7.1).

- **Rendering.** Built **only from `EmissionSummary`**, which is decoded from the exact bytes (PLAN §3.3, B4). For each emitting layer it shows the decoded kHz, ns, power and actual speed, plus passes, path count, repeats, notices, estimated time, post-crash emission bound and machine name. A tiny-skia thumbnail of the decoded geometry is shown, with the work area.
- **Gesture.** Tick the machine checklist (from the profile), then type `ARM`. Only then does `ol-arm::mint` run, inside `ol-ui`.
- **Token.** Single-use, 60 s, bound to fingerprint, machine and session. Any document edit, plan change or profile change closes the dialog and voids the token. Typed text never persists between openings (B5).
- **Agents.** Absent from the control protocol: no `ui.dialog.*` kind exists for it, and `ui.inspect` reports only `{armDialog: "open"}` without the nonce or checklist state.

### 7.5 Quit, close and crashes

- **Close while active.** `logic` checks `close_requested` first. If the machine is active (or the driver has not confirmed stopped), it sends `CancelClose`, trips the STOP latch, waits for the `StopReport` (shown in the banner), and only then allows the close.
- **Normal close.** The unsaved-changes prompt runs next. `craft_quit::on_close_request(ctx)` is called **last**, as the craft-quit docs require, so it sees the app's `CancelClose`.
- **Save on exit.** `on_exit` flushes `craft-settings`. Code after `run_native` does not run on macOS.
- **Crashes.** The dead-man rule and panic guard stop the job if the UI dies (PLAN §3.5). The UI makes no claim beyond that.

---

## 8. The UI's agent surface (bridge mode)

This mirrors PhotoCraft's `control.rs`; transport and MCP live in the binary and the CLI.

| Method | Purpose | Notes |
|---|---|---|
| `engine.commands` / `engine.execute {command, params}` | Discover and run commands by id | Human-only ids are refused (§7.3) |
| `ui.inspect` | Full UI state | Tool; selection ids and bounds; view (zoom, centre); active layer; dock `Layout` (serde); open panels and dialogs; machine state; refusals and notices; FrameReceipt age; `armDialog` open/closed; window size; theme; language |
| `ui.elements` | Clickable elements from the AccessKit tree: `{id, role, label, rect, enabled, humanOnly}` | Like FilmCraft's `ui_elements`; lets agents click by id rather than coordinates |
| `ui.set {tool?, zoom?, center?, fit?, theme?, language?, activeLayer?, panels?, dock?, units?, viewStyle?}` | UI state | Fields not in the allow-list are an error (`UI_SET_FIELDS` pattern) |
| `ui.dock.move/float/close/open/resize` | `craft_ui::docking::Action<String>` through serde | Same path as pointer docking |
| `ui.menu.list` / `ui.menu.invoke {id}` | Menu tree with live/enabled/checked; invoke | Human-only ids are refused |
| `ui.dialog.open/set/confirm/cancel` | Cut Settings Editor, Device Settings (non-I/O fields), Import options, Settings | No ARM kind |
| `ui.pointer {events:[{kind,x,y}], tool?, modifiers?}` | Drive canvas tools in **world mm** | Same code path as the mouse |
| `ui.click/move/key/type` | Synthetic input in screen points | Fenced (§7.3) |
| `ui.screenshot {path?}` | PNG of the window | Allowed in every state |
| `render.preview {maxSide}` | tiny-skia render of the display list | Also headless (no window) |
| `laser.status` / `laser.stop` / `laser.pause` | Read state; stop or pause | Always allowed |
| `app.open/save` | File I/O under `--automation-read-root/--automation-write-root` | Capability-scoped like PhotoCraft's `AuthorizedWorkspace` |

The MCP tool names stay Craft-standard so agents transfer their habits: `command_list`, `command_run`, `command_batch`, `doc_inspect`, `render_preview`, `doc_open/new/save/export`, and in bridge mode `ui_inspect`, `ui_elements`, `ui_click`, `ui_set`, `ui_menu_invoke`, `ui_pointer`, `ui_screenshot`, `control_call`. The exact tool list belongs to the CLI/MCP design note, not this one.

---

## 9. Theme, icons, fonts, language

### 9.1 Tokens and themes

- **Tokens.** Modelled on PhotoCraft's: chrome, canvas, canvas_grid, dock, card, card_border, field, field_border, hover, pressed, text, text_dim, text_faint, icon, accent, accent_soft, separator, danger, warning, row_selected, radius_sm, radius, plus laser-specific `work_area`, `encodable_field`, `frame_overlay`, `refusal`, `notice`, `stop_bg`, `stop_text`.
- **Storage.** Kept in context data (`Tokens::get(ctx)`). Widgets never hard-code colours, except document content such as layer colours.
- **Presets**, as in LightBurn 2.x: **System**, **Light** and **Dark** (UI-03). Layer colours keep their identity in both. The canvas is near-white in Light and charcoal in Dark, and black layer 00 is drawn as a light grey in Dark **(verify how LightBurn shows layer 00 on dark)**.
- **Applying.** `theme::WidgetPalette { noninteractive, inactive, hovered, active, open }.apply_to(&mut visuals.widgets, radius, fg_stroke)` (VectorCraft, DesignCraft and LightCraft do the same), followed by app-specific exceptions. `IconButtonStyle` comes from Tokens, with selection taking precedence over pressed and hovered states. `DockStyle` too (§4.7).
- **STOP colours** are fixed and theme-independent: white on saturated red, contrast ≥ 7:1, tested.

### 9.2 Icons

- **Catalog.** `craft_ui::icons::SvgIconSet::new(ICONS, SvgStyle { current_color: "#ffffff", stroke_width: Some("1.75") }, "bytes://openlaser/icons/", "square")`. Images are tinted per use. `egui_extras::install_image_loaders` (feature `svg`) is called once in `setup_context`. Use a unique URI prefix per catalog/style.
- **Base set: Lucide (ISC).** Seed from PhotoCraft's and PdfCraft's `assets/icons/*.svg` (239 distinct files, about 210 of them Lucide, already vetted, with `LICENSE-lucide.txt`). Missing names (`crosshair`, `hexagon`, `group`, `ungroup`, `squares-unite`, `squares-intersect`, `scan-eye`, `cpu`, `octagon-x`, `history`, `usb`, `zap`) come from Lucide at one pinned version, the same `lucide-static 1.49.0` PdfCraft's ATTRIBUTION records. Lucide renames icons between releases (PhotoCraft notes `align-*` → `text-align-*`), so names are checked by a test that every catalog name exists.
- **Original glyphs** (24 px grid, Lucide stroke style, MIT OR Apache-2.0, OpenLaser contributors): frame (dashed rect + red dot), live framing, start/mark (beam into a work piece), weld, offset shapes, tabs, position laser, job origin (9-point), layer modes Line / Fill / Offset Fill / Image / Tool, Q-pulse (pulse waveform), galvo field, MOPA source.
- **ATTRIBUTION.md** gets one row per file or group, as PhotoCraft does. A test fails if an embedded SVG has no row.
- **Never** use LightBurn icons, screenshots or images, not even as references committed to the repo. Side-by-side comparisons stay untracked and local (§12).

### 9.3 Fonts

M1 uses egui's default fonts (feature `default_fonts`, with attribution rows). A UI font such as Inter can follow in M2 if wanted. Text *objects* on the canvas are geometry (M2 text, `ol-geom`), never egui fonts.

### 9.4 Language menu

Use PhotoCraft's `tl!` macro pattern with catalogues per language. Ship **English** and **Português (Brasil)** first; Luiz is Brazilian, and S7 tests the Portuguese dead-key IME. Panel ids, command ids and control-protocol fields are never translated.

---

## 10. Keyboard shortcuts

Use the PhotoCraft dispatcher: `Cmd+…` strings, strict parse, and one table that drives menus, native menu key equivalents and the shortcut editor (UI-04, T1).

- **Defaults** are LightBurn's (Ctrl → ⌘ on macOS) **(verify all)**: New ⌘N, Open ⌘O, Import ⌘I, Save ⌘S, Save As ⇧⌘S, Undo ⌘Z, Redo ⇧⌘Z, Select All ⌘A, Cut/Copy/Paste ⌘X/⌘C/⌘V, Paste in Place ⌥V, Duplicate ⌘D, Delete ⌫/⌦, Group ⌘G, Ungroup ⌘U, Flip H ⇧⌘H, Flip V ⇧⌘V, Rectangle ⌘R, Ellipse ⌘E, Lines ⌘L, Text ⌘T, Edit Nodes ⌘\`, Offset ⌥O, Weld ⌘W, Auto-Join ⌥J, Delete Duplicates ⌥D, Trace ⌥T, Preview ⌥P, Live Framing F1 **(verify)**, nudge with the arrows.
- **macOS clashes**, resolved by the `native_menu` rules:
  - ⌘W is Weld in LightBurn and Close Window on macOS. Keep Weld; Close stays in the window menu.
  - ⌘H stays Hide (system).
  - Flip H moves off ⇧⌘H only if AppKit claims it (report clashes as PhotoCraft does).
- **Reserved.** **Esc = STOP while the machine is active** (§7.2), and never rebindable. With the machine idle, Esc cancels the tool or clears the selection.

---

## 11. Persistence

| What | Where | Mechanism |
|---|---|---|
| Window geometry | `<config>/ui.ron` | eframe persistence, corrected on restore (PhotoCraft's `restored_window`) |
| Dock layout, toolbar visibility, theme, language, units, shortcuts, recent files | `<config>/config.toml` | `craft-settings` (`APP_ID = "openlaser"`, schema migrations). The layout is stored as `craft_ui::docking::Layout<String>` (serde) and validated against the registry on load |
| Projects | user path | `craft-atomic` (M2 zip with migrations) |
| Machine profiles, calibration, red-dot calibration | `<config>/machines/` | `craft-atomic` plus `ol-machine` validation. **Not** craft-settings: safety data never merges silently with hand edits |
| Job exports, FrameReceipts | in memory / session | never persisted across sessions (PLAN §3.6) |

---

## 12. Test strategy (kittest and screenshots)

Executors build panels against these specs (PLAN §6.4). Safety-UI tests are red-zone and mutation-checked.

1. **Pure logic (no egui):** the view transform round-trip (`f64`, at the ±F/2 edges); flattening buckets; decimation; hairline mesher vertex counts; R-tree pick and marquee (enclosed vs touching); numeric field parsing (units, non-finite refusal).
2. **Panel kittests** (`Harness::builder().with_size(..).build_ui_state(|ui, app| panel(app, ui), app)`), driven with real `egui::Event`s, queried by AccessKit role and label:
   - Cuts/Layers:
     - columns present; Air hidden for galvo;
     - double-click opens the editor;
     - a palette click assigns the selection;
     - a refusal badge appears for an off-table pulse;
     - Spd shows the actual speed.
   - Cut Settings Editor:
     - Fill shows interval/angle and Line hides them;
     - LPI ↔ interval linked;
     - a 7 ns pulse snaps with a notice and 1 µs is refused;
     - 0-unit speed is refused;
     - OK produces exactly one undo step.
   - Laser panel:
     - Send, Home and Go to Origin are disabled with reasons on galvo;
     - Start without a FrameReceipt says "Frame this job first";
     - Start is disabled while refusals exist (M1 exit 7);
     - no file dialog opens while framing or emitting (M1 exit 7).
3. **Safety kittests** (red zone):
   - `kittest_confirm_shows_decoded_layer_params` (B4): the ARM dialog values come from `EmissionSummary`. A test mutates layer state after the plan, and the dialog must still show the decoded values.
   - `kittest_stop_always_visible_and_enabled`: every machine state × {default layout, Laser panel closed, Laser floated, Cut Settings modal open, 1024 pt-wide window, 2× ppp} → a "Stop laser" button exists, is enabled, and clicking it trips the latch.
   - `kittest_esc_trips_latch_before_widgets`: with a text field focused and a menu open, Esc trips the latch in the same frame, and the field keeps its text.
   - `kittest_arm_ignores_synthetic_input`: a `ui.type "ARM"` plus `ui.click` through an in-process `ControlRequest` channel leaves no minted token and returns a refusal.
   - `kittest_arm_ignores_accesskit_actions`: `Event::AccessKitActionRequest` Click on ARM is ignored.
   - `control_refuses_human_only`: every id with `human_only` returns the error through `engine.execute`, `ui.menu.invoke` and `command_batch`.
   - `kittest_quit_while_marking_stops_first`.
4. **Whole-app kittests** (`build_eframe`, 1440×900, as PhotoCraft's `fx_drag_tests.rs`):
   - every LightBurn region exists (AccessKit nodes for the menu bar in-window mode, toolbars, palette 00–29 + T1/T2, the dock panels);
   - docking through real pointer drags (move Laser to the left of the canvas, float it, redock);
   - the layout survives a serde round-trip; a corrupt layout falls back to the default;
   - Lock Window Layout disables moves.
5. **Image snapshots** (egui_kittest `snapshot` + `wgpu`; `tests/snapshots/<name>.png`; thresholds in `kittest.toml`; `UPDATE_SNAPSHOTS=1` accepts; `.new.png` and `.diff.png` on failure):
   - default layout, Light and Dark;
   - Cuts/Layers with refusal badges;
   - Cut Settings Editor in Line and Fill;
   - Laser panel in Disconnected / Ready / Framing / Armed / Marking / Fault / STOP NOT CONFIRMED;
   - the ARM confirmation;
   - the canvas with field, work area, objects, selection handles and frame overlay at 1× and 2×.

   Baselines are macOS-only (fonts and Metal differ). They are generated and reviewed on the macOS runner or this Mac; PLAN S5 checks this works on the GitHub macOS runner.
6. **Canvas goldens without a GPU:** the tiny-skia backend renders fixed display lists to PNGs compared exactly (deterministic CPU raster). Spot tests check that egui and tiny-skia agree on geometry positions, not pixels.
7. **Parity.** A menu catalog parity floor (`parity.rs`), plus a manual layout-parity checklist filled in by comparing against Luiz's LightBurn 1.5.04. Comparison screenshots stay out of git (clean-room, §9.2).
8. **Accessibility lint.** A test walks the AccessKit tree of the full app and fails on any interactive node without a label. egui 0.37 makes kittest enforce this; on 0.36 we enforce it ourselves. It also keeps `ui.elements` useful to agents.
9. **Performance:** S1/S2/S3 harnesses (§5.7), run in release mode on the M2, never in CI gating.

**Hardware.** No UI test opens USB. The driver behind the UI in tests is FakeLmc (`ol-lmc-emu`) or a scripted fake `Backend`. A `cargo xtask` lint fails if any test target depends on `ol-lmc-usb` (AGENTS.md rule 1).

---

## 13. Build order for the UI (maps to PLAN milestones)

| Step | Content | Milestone |
|---|---|---|
| UI-0 | Workspace deps (egui 0.36, craft-* at `3d37da38…`); `ol-ui` skeleton with `OpenLaserApp` (`logic`/`ui`), Tokens and Light/Dark, icon catalog, `apps/openlaser` with native options and craft-quit; an empty LightBurn layout with every region present; whole-app kittest of regions | Spike / M1.3 start |
| UI-1 | Canvas v1 (display list, PainterRenderer, cache, zoom/pan, rulers, grid, field and work area); S1/S2 harness | Spike (parallel with M1.1–M1.2) |
| UI-2 | Menu catalog + parity, shortcuts, muda menu, in-window fallback | M1.3 |
| UI-3 | Dock registry and default layout (`DockArea`), persistence through craft-settings, `ui.dock.*` | M1.3 |
| UI-4 | Cuts/Layers + quick editor + palette bar + Cut Settings Editor (Line/Fill/Tool) | M1.3 |
| UI-5 | Laser panel, Live Framing, STOP chrome and overlay, Esc latch, human-only fence, ARM confirmation (**red zone**: Opus designs, second-model review, Luiz's behaviour review) | M1.3 |
| UI-6 | Selection, transform handles, numeric edits, Import, Console (log + USB trace) | M1.3 |
| UI-7 | Control channel + `ui.elements` + agent indicator; bridge to `openlaser-cli mcp` | M1.3 / M2 |
| UI-8 | Preview window, kittest snapshots, Device Settings editor, Material Test dialog | M2 |
| UI-9 | Text toolbar, node edit, booleans, offset, arrays, Library, sub-layers | T1 (M3–M5) |

---

## 14. Risks

| Risk | Effect | Mitigation |
|---|---|---|
| craft-libs moves to egui 0.37 mid-project | Forced upgrade, or a fork | Pin one revision. Upgrade with the suite, behind a full kittest + snapshot run |
| PLAN's ARM isolation vs the Craft pattern (control server and `--mcp-bridge` inside the GUI binary that links `ol-arm`) | A remote path near ARM | §7.3 fence; MCP kept in `openlaser-cli` (O2); xtask reverse-dependency rule updated for the binary |
| Native menu tracking or live resize blocks egui | Esc/STOP button unavailable for the duration | S3 cases; non-resizable while active if needed; E-stop primary; documented on the STOP line |
| Painter canvas too slow at 200k segments | Janky pan | Own mesher + mesh cache + pan translation; wgpu hairline pipeline behind `CanvasRenderer` (M3 if S1 is 16.7–33 ms) |
| LightBurn details remembered wrong (menu order, marquee direction, handle toggling, F1) | "Not exactly LightBurn" | Every **(verify)** item goes into the parity checklist, checked against the licensed 1.5.04 before freezing |
| Duplicate kurbo/usvg via `egui_extras[svg]` | S4 fails; larger binary | O5: align versions or exempt the icon path |
| Offline build gaps (`dirs 7`, muda 0.21.2) | First build needs the network | Avoid craft-quit `menu`; allow one networked fetch for craft-settings |

---

## 15. Open questions

- **O1.** Is `github.com/storytold/craft-libs` reachable from Luiz's machine and from CI (private org?), or should OpenLaser vendor the pinned revision (licence allows it)? The local cargo git database has it, which is enough for local builds.
- **O2.** Where does the MCP server live? The Craft pattern embeds it in the GUI binary (`--mcp-bridge`). PLAN forbids server crates in the crates that can mint ARM. Proposal: `openlaser-cli` (agent CLI + MCP, no `ol-arm`) and `ol` (hardware CLI with TTY ARM, no server crates) as **two binaries**. The GUI's Assistant bridge launches `openlaser-cli mcp --bridge`.
- **O3.** Should the GUI start its loopback control channel by default, only with `--control`, or only on demand (Window › Assistant), as PhotoCraft does? Recommendation: on demand or with `--control`, never by default, with the agent indicator always visible.
- **O4.** Confirm the mapping of the prototype's "Frame-only" layer to LightBurn tool layers T1/T2 plus Live Framing's "Tool Layers Only". The alternative keeps a separate Frame-only mode, which LightBurn lacks.
- **O5.** kurbo/usvg version alignment (§2.6).
- **O6.** Palette colours 00–29: measure from Luiz's LightBurn install (interoperability) or define our own close set?
- **O7.** All **(verify)** items: menu contents and order, toolbar default positions (is Arrange at the bottom?), marquee direction semantics, the second click toggling rotate handles, F1 for Live Framing, status-bar wording, how layer 00 shows on a dark canvas.
- **O8.** Should agents be allowed to *open* the ARM confirmation (stage a job for human approval, which AGENTS.md permits) by invoking `laser.start` as a request that only shows a "Job ready for review" banner and needs a human click to open the dialog? Proposal: yes, through `laser.stage`, not `laser.start`.

---

## Appendix A: evidence index

| Claim | Source |
|---|---|
| craft-ui requires egui 0.36 / Rust 1.95 | `craft-libs/3d37da3/Cargo.toml`; `crates/ui/README.md` "Compatibility and checks" |
| craft-ui at 3d37da3 = 5c874d5 + printing | `diff -rq` of the two checkouts; `git log 5c874d5..3d37da3 -- crates/ui` |
| PhotoCraft/PdfCraft pins | `photocraft/Cargo.toml:13,41`; `pdfcraft/Cargo.toml:16,18` |
| Docking model, actions, permissions, protected canvas without a header | `crates/ui/src/docking/{model.rs:23-163, surface.rs:14-110, 497, 550}`; `docs/ui-docking.md` |
| WidgetPalette contract | `docs/theme-infrastructure.md` "Boundary and API" |
| craft-quit ordering and the macOS crash rationale | `crates/quit/src/lib.rs` docs |
| craft-settings API | `crates/settings/src/lib.rs` |
| PhotoCraft frame order, Services, automation_input | `photocraft/crates/ui-egui/src/lib.rs:401-520, 1689-1960, 2198-2209` |
| Panels with `egui::Panel` in 0.36 | `panels.rs:579, 931, 1609, 1669-1760` |
| Docking adapter | `panel_docking.rs:1-140, 364-445` |
| Menu catalog / parity / native menu / muda backend | `menu_catalog.rs`, `parity.rs` (`FLOOR = 631`), `native_menu.rs:1-120, 638-725`, `apps/photocraft/src/mac_menu.rs:1-40` |
| Control methods and deadlines | `control.rs:1-130` |
| Desktop bootstrap, control server, assistant bridge | `apps/photocraft/src/main.rs:1-170, 375-520`; `control_server.rs:1-60`; `assistant_bridge.rs:1-60` |
| MCP bridge tools | `crates/automation/src/server.rs:890-960` |
| Icons, Lucide licence | `icons.rs:1-30`; `ATTRIBUTION.md:29-48`; `assets/icons/` |
| egui Modal / layer ordering | `egui-0.36.2/src/containers/modal.rs:39-120`; `memory/mod.rs:830-960`; `layers.rs:41-47` |
| kittest snapshot behaviour | `egui_kittest-0.36.2/src/snapshot.rs:20-50, 391-430` |
| egui_extras[svg] → usvg 0.45 → kurbo 0.11 | registry `Cargo.toml`s of egui_extras 0.36.2, resvg 0.45.1, usvg 0.45.1 |
| JPT M7 pulse table, frequency bounds | `OpenMopa/mopa_luiz/cli.py:28-110`; `profiles/openmopa-machine.ini:3-13`; `mopa_luiz/ui.py:881` |
| LightBurn features (layers, cut fields, galvo, Laser window) | `openlaser-rust-plan/research/02-lightburn-feature-inventory.md` §5.2, §7, §9, §11, §12, §18 |
| Safety, threading and UI budgets | `openlaser-rust-plan/PLAN.md` §3.5, §3.6, §4, §7 (M1 exit) |
