# LightBurn command catalogue for OpenLaser

*Status: design reference, revision 1, 2026-10-23.*
*Sources: LightBurn 2.2 documentation (all 174 `Reference/` pages under <https://docs.lightburnsoftware.com/latest/>, fetched 2026-10-23); the research inventory `~/AI/ClaudeCode/openlaser-rust-plan/research/02-lightburn-feature-inventory.md` (cited as **02** with its feature IDs, such as GDEV-03); the plan `PLAN.md` (cited as **PLAN §n**); research 05 for the LMC encodings; the Python prototype `~/AI/OpenLaser/OpenMopa` (cited as **OM**); PhotoCraft's `menu_catalog.rs`, `commands.rs` and `docs/mcp.md` for the Craft-family conventions.*
*LightBurn names appear as short labels only. Every description is written for this document. Doc paths in the "Source" lines are relative to `Reference/`.*

This is the list of every LightBurn menu item, toolbar button, creation tool, dock window and dialog that OpenLaser will mirror. For each one it gives the OpenLaser command id, its parameters, which machines it applies to, its build tier and whether an AI agent may call it. It is the input for OpenLaser's equivalent of PhotoCraft's `menu_catalog.rs` and for a generated parity report like PhotoCraft's `docs/parity-checklist.md`.

---

## 1. Conventions

### 1.1 Command ids

- **Dotted `snake_case`.** The first segment is the LightBurn menu or window the item lives in, for example `file.import`, `arrange.group`, `layer.cut_settings.set`, `laser.frame`. Parameters are a JSON object.
- **One registry for every surface,** as in PhotoCraft's `CommandSpec {id, label, menu, shortcut, params, enabled, run, journal}`. The menu bar, toolbars, hotkeys, the `ol` CLI, the stdio MCP server and the bridge all dispatch the same ids. `command_list` returns `{id, label, menu, shortcut, params, enabled, disabledReason, needsDocument}` plus one extra field, `agent` (§1.5).
- **Interactive canvas tools take two call shapes.** Called with no parameters from the GUI, the id arms the tool (for example, the rectangle tool waits for a drag). Called with parameters, from any surface, it does the operation directly (it creates the rectangle). Agents always use the second shape.
- **Toggles** take `{on: bool}`. Called without it, they flip.
- **Settings** use one setter per dialog (`layer.cut_settings.set`, `device.settings.set`, `optimize.set`, `prefs.set`). The setter takes only the keys being changed, and the matching `…get` returns the full set. The key names are listed in §6, §7 and §8.

| Namespace | LightBurn origin |
|---|---|
| `file.*` | File menu |
| `edit.*` | Edit menu, including selection |
| `tools.*` | Tools menu, Creation and Modifiers toolbars |
| `nodes.*` | Edit Nodes toolbar and its keys |
| `arrange.*` | Arrange menu, Arrange, Numeric Edits and Docking toolbars |
| `laser_tools.*` | Laser Tools menu (tests, wizards, production modes) |
| `view.*` | Window menu view items (zoom, pan, view style) |
| `window.*` | Showing and hiding dock windows and toolbars; layout |
| `help.*` | Help menu |
| `layer.*` | Color Palette, Cuts / Layers window, Cut Settings Editor |
| `laser.*` | Laser window and the galvo Live Framing window |
| `move.*`, `console.*`, `macro.*`, `file_list.*` | Move, Console, Macros and File List windows |
| `preview.*`, `optimize.*` | Preview window, Optimization Settings |
| `device.*` | Devices window, Device Settings, Machine Settings, Calibrate Axis |
| `shape.*`, `text.*`, `vartext.*`, `image.*` | Shape Properties, Text Options, Variable Text, Adjust Image |
| `material.*`, `art.*`, `pages.*`, `camera.*` | Material Library, Art Library, Pages, Cameras |
| `prefs.*` | Settings window, Language menu |
| `job.*`, `bench.*`, `dev.*` | OpenLaser only (§9) |

**Divergence from PhotoCraft.** PhotoCraft writes multi-word segments in lowerCamel (`file.saveACopy`). This catalogue follows the `snake_case` style the OpenLaser task specified (`layer.cut_settings.set`). See open question 1.

### 1.2 Parameters and units

Every key that carries a quantity names its unit, so an agent cannot confuse mm/s with mm/min or kHz with Hz:

| Suffix | Unit | Suffix | Unit |
|---|---|---|---|
| `_mm` | millimetres (design space, Y up; galvo origin at the field centre, PLAN §3.2) | `_khz` | kilohertz (stored internally in Hz, as `.lbrn2` does) |
| `_mm_s` | mm per second (galvo always; gantry stores mm/s and shows mm/min if the device asks) | `_ns` | nanoseconds |
| `_pct` | percent, 0–100 | `_us` | microseconds |
| `_deg` | degrees, positive = clockwise in the UI | `_ms` | milliseconds |

Internally the values become the PLAN §3.2 newtypes (`Mm`, `MmPerS`, `Hz`, `Ns`, `Us`, `Percent`). Objects are addressed by `ObjectId` and layers by index `0`–`29` or `"T1"`/`"T2"`. A missing `ids` parameter means "the current selection".

### 1.3 Shortcuts

- Written macOS-first: `Cmd`, `Opt`, `Shift`, `Ctrl`. On Windows and Linux, `Cmd` → `Ctrl` and `Opt` → `Alt`.
- **Plain letter keys** (`H`, `L`, `C`, …) act only while the canvas has focus, as in LightBurn.
- **A dagger (†)** marks a shortcut whose LightBurn docs disagree with each other, or one that collides with a macOS system shortcut. §10 resolves each one.

### 1.4 Columns

| Column | Values |
|---|---|
| **Tier** | **P0** = Luiz's MOPA runs a real job end to end from the app (import SVG/DXF/images, select/move/scale/rotate, layers and galvo cut settings including frequency and Q-pulse, preview, red-light frame, start/stop, device settings including field size, scale correction and mirror/rotate, project save/load). **P1** = a credible LightBurn daily driver (shapes, text, node editing, booleans, offset, align/distribute, arrays, image modes and dithering, material test, optimization, GRBL connection). **P2** = parity. **P3** = later, or deliberately not planned (the row says which). |
| **Applies** | `galvo`, `gantry` (GRBL and other G-code controllers, DSP), or `both` |
| **Agent** | Who may call it; see §1.5 |

### 1.5 Agent policy (the `agent` field)

Luiz asked for the same agent access as the Craft family. PLAN §3.6 adds two hard limits: agents never run hardware commands, and only a human at the GUI or at a real terminal can ARM. Every command carries one of five classes, and the registry enforces them:

| Class | Meaning | Headless MCP / CLI | Bridge (live GUI) | GUI and TTY CLI |
|---|---|---|---|---|
| `ok` | Changes the document, a profile or a preference. No device I/O | yes | yes | yes |
| `ro` | Read-only query | yes | yes | yes |
| `ui` | Only meaningful in the live window: a dialog, a canvas tool, a panel | no | yes | yes |
| `hw` | Talks to the laser board or another machine (USB, serial, TCP): red light, motion, status reads, firmware settings. Never opens the marking laser | **refused** | **refused** | inside a bench session (PLAN §3.6) |
| `arm` | Can emit laser energy | **not registered** | **not registered**; synthetic input is refused on every ARM, START and confirmation widget | GUI with real input events, or a TTY, each behind a single-use ARM token |

Two rules sit on top of the table:

1. **`laser.stop` is accepted from every surface,** agents included, at all times. Stopping never needs permission.
2. **Bridge mode must not become an ARM path.** PhotoCraft's bridge forwards synthetic clicks, keys and `control_call` to the live app (`photocraft/docs/control-protocol.md`, "MCP bridge"). Copied unchanged, it would let an agent click Start and type `ARM` in OpenLaser's own window, inside the very process that is allowed to mint tokens. OpenLaser's bridge therefore tags every injected event as synthetic. `ol_arm::mint` accepts only a `HumanConfirm` built from real OS input, and while a frame or job runs, the bridge accepts nothing but `laser.stop` and read-only calls.

---

## 2. Tier roll-up

| Tier | What it covers |
|---|---|
| **P0** | File New/Open/Import/Save/Save As/Exit; SVG, DXF and raster import; undo/redo; select, delete, move, scale, rotate, flip, rotate 90°, numeric edits, nudge, centre on page; join/close/dedupe paths; zoom/pan; 30-colour palette plus tool layers; Cuts / Layers (mode, speed, power, passes, interval, frequency, Q-pulse, output, show, order); Line, Fill and Image layers (Threshold plus one photo dither); galvo Device Settings (field size/offset/angle, galvo swap and negate, per-axis scale with Calibrate Axis, red-dot offset/scale, frame and max speed, jump and delay defaults, laser source and MOPA options, job checklist); device profiles from an EZCAD `markcfg`; connect; status; Preview with time estimate; Live Framing (bounds, contour, tool layers only); Start, Stop, Repeat, Cut Selected Graphics, Red Light; the OpenLaser job/safety commands (§9) and the agent surface (§11) |
| **P1** | Shapes, pen tool, text and variable text, barcodes, node editing, booleans and weld, offset, align/distribute, group, arrays, clipboard, snapping, measure; Offset Fill, sub-layers, global passes, wobble, per-layer galvo timings, flood fill, dot width; all common image modes and Adjust Image; Material Test, Interval Test, 9-point lens wizard, COR files, bulge/skew/trapezoid; Optimization Settings; Preview playback; Material Library; job counter; pause/resume; door and foot-switch inputs; GRBL live control (connection, Move window, Console, Macros, framing, homing, origins, G-code save/run, firmware `$$` editor); SVG/DXF export; `.lbrn2` import |
| **P2** | Cameras, Print and Cut, rotary, cylinder correction, split and repeat marking, taper warp, 3D sliced images and 3D-model import, trace image, warp/deform, nesting, pages, virtual arrays, art library, DSP controllers, Custom G-code, undo history, hotkey editor, languages, most remaining preferences |
| **P3** | Web Tools, vendor-specific hardware, LiDAR, Camera Print & Cut, printing to paper, bundles, update checks; plus the rows marked "not planned" (usage data, licence management, galvo laser-on framing, unbounded continuous runs) |

Row counts per tier are in §12.

---

## 3. Menus

### 3.1 File menu
*Source: `UI/FileMenu`, `FileManagement`, `ManagingPreferences`, `UserBundles`, `Print`, `ShowNotes`, `NewWindow`.*

| Tier | Id | LightBurn item | Also in | Key | What it does | Params | Applies | Agent |
|---|---|---|---|---|---|---|---|---|
| P0 | `file.new` | New | Main toolbar | `Cmd+N` | Starts an empty project in place of the current one, offering to save changes first | `{}` | both | ok |
| P2 | `file.new_window` | New Window | — | — | Opens a second, independent window on another project (edit one while another runs) | `{}` | both | ui |
| P1 | `file.open_recent` | Recent Projects | — | — | Reopens one of the last 24 projects | `{index}` or `{path}` | both | ok |
| P2 | `file.open_recent_template` | Recent Templates | — | — | Starts a new project from a recently used template | `{index}` | both | ok |
| P0 | `file.open` | Open | Main toolbar | `Cmd+O` | Loads a project and replaces the current one. Opening an artwork file starts a new project from it. P0 reads OpenLaser's own project format; `.lbrn2`/`.lbrn` read is P1 (02 §17) | `{path}` | both | ok |
| P0 | `file.import` | Import | Main toolbar, drag and drop | `Cmd+I` | Adds artwork to the open project, matching colours to palette layers. Formats and tiers are in §3.1.1 | `{path, layer?, group?, at_mm?}` | both | ok |
| P2 | `file.notes` | Show Notes | — | `Cmd+Opt+N` | Shows and edits free-text notes saved inside the project, optionally shown on open | `{text?, show_on_load?}` | both | ok |
| P0 | `file.save` | Save | Main toolbar | `Cmd+S` | Saves to the current path; behaves as Save As if the project is unnamed | `{}` | both | ok |
| P0 | `file.save_as` | Save As | — | `Cmd+Shift+S` | Saves under a new path | `{path}` | both | ok |
| P2 | `file.save_as_template` | Save As Template | — | — | Saves the project as a reusable starting template | `{path}` | both | ok |
| P1 | `file.save_machine_file` | Save Machine Files (Save GCode / RD / OUT / UD5) | Laser window | `Opt+Shift+L` | Compiles the job into a controller file without connecting. G-code is P1 (PLAN M2 delivers it early for prototype parity); Ruida `.rd` P2; TopWisdom/Trocen P3. Galvos have no machine file in LightBurn; OpenLaser's byte dump is `job.dump` (§9) | `{path, selected_only?}` | gantry | ok |
| P1 | `file.export` | Export | — | `Opt+X` | Writes the selection (or everything if nothing is selected) as SVG or DXF (P1), AI or a single image (P2). DXF can keep arcs or flatten them | `{path, format, selected_only?, dxf_arcs?}` | both | ok |
| P2 | `file.prefs.import` | Preferences › Import Prefs | — | — | Loads a preferences file; needs a fresh project | `{path}` | both | ok |
| P2 | `file.prefs.export` | Preferences › Export Prefs | — | — | Writes the current preferences to a file | `{path}` | both | ok |
| P3 | `file.prefs.open_folder` | Preferences › Open Prefs Folder | — | — | Reveals the settings folder in Finder | `{}` | both | ui |
| P2 | `file.prefs.load_backup` | Preferences › Load Prefs Backup | — | — | Restores one of the automatic settings backups (LightBurn keeps the last 50) | `{backup}` | both | ok |
| P2 | `file.prefs.edit_hotkeys` | Preferences › Edit Hotkeys | — | — | Hotkey editor: rebinds any command id | `{id, shortcut}` | both | ok |
| P3 | `file.bundle.import` | Bundles › Import Bundles | drag and drop | — | Loads a migration archive (settings, hotkeys, image presets, devices, material and art libraries) | `{path, parts?}` | both | ok |
| P3 | `file.bundle.export` | Bundles › Export Bundle | — | — | Writes that archive | `{path, parts?}` | both | ok |
| P3 | `file.print_black` | Print (black only) | — | `Cmd+P` | Prints the design on paper or to PDF in black | `{}` | both | ui |
| P3 | `file.print_colors` | Print (keep colors) | — | `Cmd+Shift+P` | Prints with the layer colours | `{}` | both | ui |
| P2 | `file.save_processed_bitmap` | Save Processed Bitmap | — | — | Saves the selected image as it will actually be engraved (after adjustments and dithering) | `{path}` | both | ok |
| P2 | `file.save_background_capture` | Save Background Capture | — | — | Saves the current camera overlay image | `{path}` | gantry | ok |
| P0 | `file.exit` | Exit (macOS: app menu › Quit) | — | `Cmd+Q` | Quits, offering to save. OpenLaser refuses while framing or emitting: stop first | `{}` | both | ui |

#### 3.1.1 Import and export formats

| Tier | Direction | Formats | Notes |
|---|---|---|---|
| P0 | Import vector | `.svg` (paths, Béziers, arcs, transforms, viewBox and physical units), `.dxf` (LINE, LWPOLYLINE with bulge, POLYLINE, CIRCLE, ARC, ELLIPSE, SPLINE evaluated properly) | OM lacks SVG Béziers, arcs and transforms (02 §17.1). PLAN §3.1: `ol-io` requires physical units |
| P0 | Import raster | `.png .jpg .jpeg .bmp .tif .tiff .webp .gif .tga` | An image needs an Image layer to burn; see §6.5 |
| P1 | Import project | `.lbrn2`, `.lbrn` (graphics only through Import; Open also loads cut settings). `.lbrn2` is Y-up (06 C13) | Explicit galvo/gantry mapping layer, PLAN §2 |
| P1 | Import vector | `.pdf` | |
| P2 | Import vector | `.ai`, `.plt/.hpgl`, `.gc/.gcode/.nc` (geometry only), `.rd` | |
| P2 | Import 3D | `.stl .3mf .amf .obj .off .ply` → depth map (`Import3DModelCreateDepthMap`) | OM imports STL as triangle outlines only |
| P3 | Import other | Gerber and drill files, `.mage`, `.sc/.scpro2`, `.lbt` | |
| P1 | Export | `.svg`, `.dxf` | |
| P2 | Export | `.ai`, `.png/.jpg/.bmp` (one image) | |
| P2 | Project write | `.lbrn2` | P1 reads it; writing waits until fixtures from a real LightBurn exist (06 G10) |

### 3.2 Edit menu
*Source: `UI/EditMenu`, `Selection`, `Clipboard`, `UndoRedo`, `ClosePath`, `CloseSelectedPathsWithTolerance`, `AutoJoinSelectedShapes`, `OptimizeSelectedShapes`, `DeleteDuplicates`, `ConvertToPath`, `ConvertToBitmap`, `GeneratePathFromCut`, `ImageOptions`.*

| Tier | Id | LightBurn item | Also in | Key | What it does | Params | Applies | Agent |
|---|---|---|---|---|---|---|---|---|
| P0 | `edit.undo` | Undo | Main toolbar | `Cmd+Z` | Steps back through document changes (no limit) | `{}` | both | ok |
| P0 | `edit.redo` | Redo | Main toolbar | `Cmd+Shift+Z` | Re-applies an undone change | `{}` | both | ok |
| P2 | `edit.undo_history` | Undo History (2.1) | — | — | Window listing the history; picking an entry previews that state and can jump back to it | `{state?}` | both | ok |
| P0 | `edit.select_all` | Select All | — | `Cmd+A` | Selects every object | `{}` | both | ok |
| P1 | `edit.select_invert` | Invert Selection | — | `Cmd+Shift+I` | Swaps selected and unselected objects | `{}` | both | ok |
| P1 | `edit.cut` | Cut | Main toolbar | `Cmd+X` | Copies the selection to the clipboard and removes it | `{}` | both | ok |
| P1 | `edit.copy` | Copy | Main toolbar | `Cmd+C` | Copies the selection to the clipboard | `{}` | both | ok |
| P1 | `edit.duplicate` | Duplicate | — | `Cmd+D` | Places a copy exactly on top of the selection | `{}` | both | ok |
| P1 | `edit.paste` | Paste | Main toolbar | `Cmd+V` | Pastes centred on the pointer; agents pass a position | `{at_mm?}` | both | ok |
| P1 | `edit.paste_in_place` | Paste in Place | — | `Opt+V` | Pastes at the coordinates the objects were copied from | `{}` | both | ok |
| P0 | `edit.delete` | Delete | Main toolbar | `Del` (`Backspace` on Mac) | Removes the selected objects | `{ids?}` | both | ok |
| P1 | `edit.convert_to_path` | Convert to Path | — | `Cmd+Shift+C` | Turns parametric shapes, text and barcodes into plain editable paths | `{ids?}` | both | ok |
| P2 | `edit.convert_to_bitmap` | Convert to Bitmap | — | `Cmd+Shift+C` † | Rasterises the selected vectors into an image and removes the vectors | `{render: outlines/fill_all/cut_settings, dpi, brightness_pct?}` | both | ok |
| P2 | `edit.generate_path_from_cut` | Generate Path From Cut | — | — | Bakes cut-setting effects (tabs, kerf, perforation) into real geometry | `{ids?}` | gantry | ok |
| P0 | `edit.close_path` | Close Path | — | — | Closes an open path whose two ends are within 0.5 mm | `{ids?, tolerance_mm?: 0.5}` | both | ok |
| P1 | `edit.close_paths_tolerance` | Close Selected Paths With Tolerance | — | — | Dialog: closes or joins open ends within a chosen distance, optionally moving the ends together | `{tolerance_mm, join_ends?}` | both | ok |
| P0 | `edit.auto_join` | Auto-Join Selected Shapes | — | `Opt+J` | Chains segments whose ends nearly touch (default 0.05 mm) into continuous paths, so DXF art can be filled | `{ids?, tolerance_mm?: 0.05}` | both | ok |
| P1 | `edit.optimize_shapes` | Optimize Selected Shapes | — | `Opt+Shift+O` | Smooths paths and/or refits them as lines and arcs within a tolerance | `{smooth?, fit_lines?, fit_arcs?, tolerance_mm}` | both | ok |
| P0 | `edit.delete_duplicates` | Delete Duplicates | — | `Opt+D` | Removes objects that match another in shape, size and position (avoids double burns) | `{ids?}` | both | ok |
| P1 | `edit.reverse_direction` | Reverse Shape Direction | — | — | Reverses the direction a path is traced | `{ids?}` | both | ok |
| P1 | `edit.select_open_shapes` | Select Open Shapes | — | — | Selects every path that does not close on itself | `{}` | both | ok |
| P1 | `edit.select_open_fill_shapes` | Select Open Shapes Set to Fill | — | — | Selects open paths on Fill or Offset Fill layers, which cannot be filled. OpenLaser also lists them as numbered notices before a burn (PLAN §3.6) | `{}` | both | ok |
| P1 | `edit.select_layer_shapes` | Select All Shapes in Current Layer | Cuts / Layers (Shift+click, context menu) | — | Selects everything on the active layer | `{layer?}` | both | ok |
| P1 | `edit.select_contained` | Select Contained Shapes | — | — | Adds every shape inside the selected closed shape | `{}` | both | ok |
| P2 | `edit.select_smaller` | Select Shapes Smaller Than Selected | — | — | Adds every shape smaller than the selected one | `{}` | both | ok |
| P2 | `edit.select_circles` | Select Circles (2.1) | — | — | Selects circles of one diameter or within a range | `{diameter_mm}` or `{min_mm, max_mm}` | both | ok |
| P2 | `edit.image.refresh` | Image Options › Refresh Image | — | — | Reloads the selected image from its file | `{id?}` | both | ok |
| P2 | `edit.image.replace` | Image Options › Replace Image | — | — | Swaps in another image file at the same transform | `{id?, path}` | both | ok |
| P2 | `edit.image.replace_fit` | Image Options › Replace Image to Fit | — | — | Swaps in another image, scaled to the old one's box | `{id?, path}` | both | ok |
| P1 | `edit.settings` | Settings (macOS: app menu › Settings) | Main toolbar | `Cmd+,` | Opens the app-wide preferences (§8.7) | `{}` | both | ui |
| P0 | `edit.device_settings` | Device Settings | Main toolbar, Laser Tools | — | Opens the active device's settings (§7) | `{}` | both | ui |

**Selection without the mouse** (no LightBurn menu item; the canvas does this by clicking):

| Tier | Id | Equivalent | Key | What it does | Params | Agent |
|---|---|---|---|---|---|---|
| P0 | `edit.select` | Click, Shift+click, Cmd+click, marquee | — | Sets, adds to, removes from or toggles the selection by object ids or by a rectangle (enclosing or crossing) | `{ids?, rect_mm?, crossing?, mode: replace/add/remove/toggle}` | ok |
| P0 | `edit.select_none` | Esc, or click on empty canvas | `Esc` † | Clears the selection | `{}` | ok |
| P1 | `edit.select_next` / `edit.select_prev` | Tab / Shift+Tab | `Tab`, `Shift+Tab` | Cycles the selection through objects in creation order | `{}` | ok |

### 3.3 Tools menu, Creation and Modifiers toolbars
*Source: `UI/ToolsMenu`, `UI/CreationToolbar`, `UI/ModifiersToolbar`, `DrawLines`, `PrimaryShapes`, `Text`, `BarCode`, `AddTabs`, `Measure`, `PositionLaser`, `OffsetShapes`, `BooleanTools`, `TrimShapes`, `CutShapes`, `AdjustImage`, `TraceImage`, `MultiFileTraceImage`, `ApplyPathToText`, `ApplyMaskToImage`, `ResizeSlots`, `WarpDeform`, `TangentCircleGenerator`, `SetStartPoint`, `RadiusFillet`.*

| Tier | Id | LightBurn item | Also in | Key | What it does | Params | Applies | Agent |
|---|---|---|---|---|---|---|---|---|
| P0 | `tools.select` | Select | Creation toolbar | `Esc` † | Pointer tool: click to pick, drag right for an enclosing box, drag left for a crossing box | `{}` | both | ui |
| P1 | `tools.draw_lines` | Draw Lines | Creation toolbar | `Cmd+L` | Pen tool for straight and curved segments; agents pass points or SVG path data | `{points_mm? or d?, closed?, layer?}` | both | ok |
| P1 | `tools.rectangle` | Draw Shape › Rectangle | Creation toolbar | `Cmd+R` | Parametric rectangle with optional corner radius (negative = notched corner) | `{x_mm, y_mm, w_mm, h_mm, corner_radius_mm?, layer?}` | both | ok |
| P1 | `tools.ellipse` | Draw Shape › Ellipse | Creation toolbar | `Cmd+E` | Parametric ellipse or circle | `{cx_mm, cy_mm, w_mm, h_mm, layer?}` | both | ok |
| P1 | `tools.triangle` | Draw Shape › Triangle | Creation toolbar | — | Parametric triangle | `{x_mm, y_mm, w_mm, h_mm, layer?}` | both | ok |
| P1 | `tools.pentagon` | Draw Shape › Pentagon | Creation toolbar | — | Parametric pentagon | same | both | ok |
| P1 | `tools.polygon` | Draw Shape › Polygon | Creation toolbar | — | Regular polygon, 6 sides by default | `{…, sides: 6}` | both | ok |
| P1 | `tools.octagon` | Draw Shape › Octagon | Creation toolbar | — | Parametric octagon | same | both | ok |
| P1 | `tools.star` | Draw Shape › Star | Creation toolbar | — | Star with point count, bulge and inner ratio | `{…, points, bulge: 0–1, ratio: 0–1}` | both | ok |
| P1 | `tools.dual_star` | Draw Shape › Dual Star | Creation toolbar | — | Two overlaid stars with separate ratios | `{…, points, bulge, ratio, ratio2}` | both | ok |
| P1 | `tools.edit_nodes` | Edit Nodes | Creation toolbar | `` Cmd+` `` † | Enters node editing; the node operations are §3.3.1 | `{id?}` | both | ui |
| P2 | `tools.trim_shapes` | Trim Shapes | Creation toolbar | `Cmd+K` | Removes the segment under the pointer back to its nearest intersections | `{id, at_mm}` | both | ok |
| P1 | `tools.add_tabs` | Add Tabs | Creation toolbar | `Cmd+Tab` † | Places uncut bridges by hand; automatic tabs are Line-mode cut settings (§6.3) | `{id, at_mm[]}` | gantry | ok |
| P1 | `tools.text` | Edit Text | Creation toolbar | `Cmd+T` | Creates or edits a text object; formatting is `text.set` (§5.10) | `{text, x_mm, y_mm, font?, height_mm?, layer?}` | both | ok |
| P1 | `tools.position_laser` | Position Laser | Creation toolbar | `Opt+L` † | Moves the head to the clicked canvas point | `{x_mm, y_mm}` | gantry | hw |
| P1 | `tools.measure` | Measure | Creation toolbar | `Opt+M` † | Shows perimeter, size and node data under the pointer, or a dragged distance; agents get the numbers back | `{id? or from_mm, to_mm}` | both | ro |
| P1 | `tools.barcode` | Create Bar Code | — | — | Barcode object. P1: QR, Data Matrix, Code 128. P2: the other symbologies (02 DES-06), WiFi/contact QR helpers, GS1 control characters | `{type, data, ecc?, show_text?, force_square?, vartext?, offset?}` | both | ok |
| P1 | `tools.offset_shapes` | Offset Shapes | Modifiers toolbar | `Opt+O` | Makes outlines at a distance outward, inward or both, with round, bevel or sharp corners | `{distance_mm, direction: out/in/both, corner: round/bevel/sharp, outer_only?, select_result?, delete_original?, optimize?}` | both | ok |
| P1 | `tools.weld` | Weld Shapes | Modifiers toolbar | `Cmd+W` † | Merges any number of overlapping shapes into one outline | `{ids?}` | both | ok |
| P1 | `tools.boolean_union` | Boolean Union | Modifiers toolbar | `Opt+Plus` | Merges exactly two closed shapes or groups | `{a?, b?}` | both | ok |
| P1 | `tools.boolean_subtract` | Boolean Subtract | Modifiers toolbar | `Opt+Minus` | Removes the second shape from the first where they overlap | `{a?, b?}` | both | ok |
| P1 | `tools.boolean_intersect` | Boolean Intersection | Modifiers toolbar | `Opt+Asterisk` | Keeps only the overlap of two shapes | `{a?, b?}` | both | ok |
| P2 | `tools.boolean_assistant` | Boolean Assistant | — | `Cmd+B` | Dialog that previews each boolean result before applying one | `{}` | both | ui |
| P2 | `tools.cut_shapes` | Cut Shapes | — | `Opt+Shift+C` | Splits shapes (and images) along a closed cutter shape | `{cutter, ids?}` | both | ok |
| P1 | `tools.adjust_image` | Adjust Image | — | `Opt+I` | Image dialog with a live before/after view: tone adjustments, layer image settings, presets (§8.3) | `{id?}` | both | ui |
| P2 | `tools.trace_image` | Trace Image | — | `Opt+T` | Vectorises a bitmap: cutoff/threshold, transparency, centre-line or sketch trace, ignore small specks, smoothness, optimize, delete the source | `{id?, cutoff, threshold, transparency?, centerline?, sketch?, ignore_less_than?, smoothness?, optimize?, delete_image?}` | both | ok |
| P3 | `tools.trace_multi` | Multi-File Trace Image (Labs) | — | — | Batch-traces several image files | `{paths[], …trace params}` | both | ok |
| P2 | `tools.apply_path_to_text` | Apply Path to Text | — | — | Makes a text object follow a chosen path | `{text_id, path_id}` | both | ok |
| P2 | `tools.apply_mask` | Apply Mask to Image | — | — | Clips an image to a closed vector; both stay editable | `{image_id, mask_id}` | both | ok |
| P2 | `tools.crop_image` | Crop Image | — | — | Masks and flattens in one step (permanent) | `{image_id, mask_id}` | both | ok |
| P3 | `tools.resize_slots` | Resize Slots in Selection | — | — | Resizes slot and tab features to a new material thickness | `{from_mm, to_mm, tolerance_mm?}` | gantry | ok |
| P2 | `tools.warp` | Warp Selection (4 Points) | — | — | Free-form distortion by four corner handles; vectors, text and images | `{corners_mm[4]}` | both | ok |
| P2 | `tools.deform` | Deform Selection (16 Points) | — | — | Distortion by a 4 × 4 handle grid | `{grid_mm[16]}` | both | ok |
| P3 | `tools.tangent_circle` | Generate Tangent Circle (2.1) | — | — | Circle tangent to three selected circles | `{ids[3]}` | both | ok |
| P1 | `tools.set_start_point` | Set Start Point | Modifiers toolbar | — | Chooses where (and in which direction) a closed path starts burning | `{id, node or at_mm, reverse?}` | both | ok |
| P2 | `tools.radius` | Radius / Fillet | Modifiers toolbar | — | Rounds a corner; a negative radius bites a notch; zero restores the sharp corner | `{id, node, radius_mm}` | both | ok |

The Modifiers toolbar also carries Grid Array and Circular Array (§3.4).

#### 3.3.1 Edit Nodes toolbar and keys
*Source: `EditNodes`. Each key acts on the node or segment under the pointer; agents name the object and the node or segment index.*

| Tier | Id | LightBurn action | Key | What it does | Params |
|---|---|---|---|---|---|
| P1 | `nodes.delete` | Delete node | `D` (on a node) | Removes the node and merges its neighbours | `{id, node}` |
| P1 | `nodes.insert` | Insert | `I` | Adds a node at a point on the path | `{id, segment, t}` |
| P1 | `nodes.insert_midpoint` | Insert node at midpoint | `M` | Adds a node halfway along a segment | `{id, segment}` |
| P1 | `nodes.to_corner` | Convert smooth node to corner | `C` | Lets the two handles move independently | `{id, node}` |
| P1 | `nodes.to_smooth` | Smooth corner node | `S` (on a node) | Locks the two handles to one tangent | `{id, node}` |
| P1 | `nodes.break` | Break shape at point | `B` | Opens the path at that node | `{id, node}` |
| P1 | `nodes.delete_segment` | Delete line | `D` (on a segment) | Removes a segment, opening or splitting the path | `{id, segment}` |
| P1 | `nodes.to_line` | Convert curve to line | `L` | Straightens a curve | `{id, segment}` |
| P1 | `nodes.to_curve` | Convert line to smooth curve | `S` (on a segment) | Gives a line Bézier handles | `{id, segment}` |
| P2 | `nodes.align` | Align Selected | `A` | Rotates the selection so that segment lies at the nearest 0°, 45° or 90° | `{id, segment}` |
| P2 | `nodes.trim` | Trim line | `T` | Cuts a segment back to the next intersection | `{id, segment}` |
| P2 | `nodes.extend` | Extend line | `E` | Extends an open end to the nearest path it would meet | `{id, end}` |
| P1 | `nodes.move` | Drag a node or handle (Shift constrains to 45°/90°) | — | Moves a node or a control handle; dropping one end on another joins them | `{id, node, to_mm, handle?}` |

### 3.4 Arrange menu, Arrange, Numeric Edits and Docking toolbars
*Source: `UI/ArrangeMenu`, `UI/ArrangeToolbar`, `NumericEditsToolbar`, `Grouping`, `FlipMirror`, `TwoPointRotateScale`, `Align`, `Distribute`, `Nesting/*`, `Dock`, `MoveSelectedObjects`, `MoveLaserToSelection`, `GridArray`, `CircularArray`, `CopyAlongPath`, `CreateRubberBandOutline`, `BreakApart`, `DrawOrder`, `LockShapes`, `MakeSameWidthHeight`, `TransformControls`.*

| Tier | Id | LightBurn item | Also in | Key | What it does | Params | Applies | Agent |
|---|---|---|---|---|---|---|---|---|
| P1 | `arrange.group` | Group | Arrange toolbar | `Cmd+G` | Makes the selection behave as one object | `{ids?}` | both | ok |
| P1 | `arrange.ungroup` | Ungroup | Arrange toolbar | `Cmd+U` | Splits a group back into its members | `{ids?}` | both | ok |
| P1 | `arrange.auto_group` | Auto-Group | — | — | Groups each closed outline with everything inside it | `{ids?}` | both | ok |
| P0 | `arrange.flip_h` | Flip Horizontal | Arrange toolbar | `Cmd+Shift+H` †, `H` | Mirrors the selection left-right about its own centre | `{ids?}` | both | ok |
| P0 | `arrange.flip_v` | Flip Vertical | Arrange toolbar | `Cmd+Shift+V` †, `V` | Mirrors the selection top-bottom about its own centre | `{ids?}` | both | ok |
| P2 | `arrange.mirror_across_line` | Mirror Across Line | Arrange toolbar | `Cmd+Shift+M` | Mirrors the selection about a chosen line | `{ids?, line_id}` | both | ok |
| P0 | `arrange.rotate_cw90` | Rotate 90° Clockwise | — | `.` | Quarter turn clockwise | `{ids?}` | both | ok |
| P0 | `arrange.rotate_ccw90` | Rotate 90° Counter-Clockwise | — | `,` | Quarter turn anticlockwise | `{ids?}` | both | ok |
| P2 | `arrange.two_point` | Two-Point Rotate / Scale | — | `Cmd+2` | Rotates and scales about picked points, for example to match an edge to a known length | `{from_mm[2], to_mm[2], scale?}` | both | ok |
| P1 | `arrange.align.left` | Align Left | Arrange toolbar | `Opt+Left`, `L` | Lines up left edges with the last-selected object | `{ids?}` | both | ok |
| P1 | `arrange.align.right` | Align Right | Arrange toolbar | `Opt+Right`, `R` | Lines up right edges | `{ids?}` | both | ok |
| P1 | `arrange.align.top` | Align Top | Arrange toolbar | `Opt+Up`, `T` | Lines up top edges | `{ids?}` | both | ok |
| P1 | `arrange.align.bottom` | Align Bottom | Arrange toolbar | `Opt+Down`, `B` | Lines up bottom edges | `{ids?}` | both | ok |
| P1 | `arrange.align.v_centers` | Align Vertical Centers | Arrange toolbar | `Opt+Fn+Up` (Win `Alt+PgUp`), `C` | Lines up the vertical centre lines | `{ids?}` | both | ok |
| P1 | `arrange.align.h_centers` | Align Horizontal Centers | Arrange toolbar | `Opt+Fn+Down` (Win `Alt+PgDn`), `E` | Lines up the horizontal centre lines | `{ids?}` | both | ok |
| P1 | `arrange.align.centers` | Align Centers | Arrange toolbar | `Opt+Fn+Left` (macOS only) | Puts all centres on one point | `{ids?}` | both | ok |
| P2 | `arrange.same_width` | Make Same Width | Arrange toolbar | — | Resizes the selection to the reference object's width | `{ids?}` | both | ok |
| P2 | `arrange.same_height` | Make Same Height | Arrange toolbar | — | Resizes to the reference object's height | `{ids?}` | both | ok |
| P1 | `arrange.distribute.h_spaced` | Distribute H-Spaced | Arrange toolbar | — | Equal horizontal gaps between edges | `{ids?}` | both | ok |
| P1 | `arrange.distribute.h_centered` | Distribute H-Centered | Arrange toolbar | — | Equal horizontal distance between centres | `{ids?}` | both | ok |
| P1 | `arrange.distribute.v_spaced` | Distribute V-Spaced | Arrange toolbar | — | Equal vertical gaps between edges | `{ids?}` | both | ok |
| P1 | `arrange.distribute.v_centered` | Distribute V-Centered | Arrange toolbar | — | Equal vertical distance between centres | `{ids?}` | both | ok |
| P1 | `arrange.move_together.h` | Distribute › Move H Together | — | `Opt+Shift+H` | Packs objects side by side with touching edges | `{ids?}` | both | ok |
| P1 | `arrange.move_together.v` | Distribute › Move V Together | — | `Opt+Shift+V` | Stacks objects with touching edges | `{ids?}` | both | ok |
| P2 | `arrange.nest` | Nesting (Quick Nest; True Nest) | — | — | Packs shapes into a target rectangle. Quick Nest uses bounding boxes and 90° turns; True Nest uses real outlines and finer angles; either can spill onto new pages | `{mode: quick/true, target_id, padding_mm, edge_buffer_mm, lock_inner?, lock_rotation?, multipage?, orientations?, optimize: time/quality}` | both | ok |
| P3 | `arrange.svgnest` | SVGNest (External Tool) | — | — | Sends shapes to an online nesting service. **Not planned:** OpenLaser does not upload artwork to web services | — | both | — |
| P2 | `arrange.dock.left` / `.right` / `.up` / `.down` | Dock | Docking toolbar | — | Slides the selection until it meets another object or the work-area edge | `{ids?}` | both | ok |
| P0 | `arrange.move_selection.page_center` | Move Selected Objects › Page Center | Arrange toolbar | `P` | Centres the selection in the work area (galvo: on the field centre) | `{ids?}` | both | ok |
| P1 | `arrange.move_selection.{upper_left, upper_right, lower_left, lower_right, left, right, top, bottom}` | Move Selected Objects (8 positions) | Arrange toolbar | — | Snaps the selection to a corner or edge midpoint of the work area | `{ids?}` | both | ok |
| P1 | `arrange.move_selection.laser_position` | Move to Laser Position | Arrange toolbar | — | Moves the selection to the head's last reported position | `{ids?}` | gantry | hw |
| P1 | `arrange.move_laser.{center, upper_left, upper_right, lower_left, lower_right, left, right, top, bottom}` | Move Laser to Selection (9 positions) | Arrange toolbars | — | Drives the head to that point of the selection | `{ids?}` | gantry | hw |
| P1 | `arrange.jog` | Move Laser to Selection › Jog Laser | — | — | Jogs the head one Move-window step in a direction | `{direction}` | gantry | hw |
| P1 | `arrange.grid_array` | Grid Array | Modifiers toolbar | — | Rows and columns of copies with spacing, shifts, mirroring, random orientation and serial-number increments | §8.4 | both | ok |
| P2 | `arrange.virtual_array.{create, edit, duplicate, flatten, remove, add, exit}` | Grid Array › Create Virtual Array | — | — | Linked copies that follow one master; flatten turns them into real objects | `{id}` | both | ok |
| P1 | `arrange.circular_array` | Circular Array | Modifiers toolbar | — | Copies around a centre point or the last-selected object | §8.4 | both | ok |
| P2 | `arrange.copy_along_path` | Copy Along Path | — | — | Copies spaced along a path, optionally rotated or scaled to it | §8.4 | both | ok |
| P2 | `arrange.rubber_band_outline` | Create Rubber-Band Outline from Selection | — | — | Adds the tightest outline around the selection (a hull) | `{ids?}` | both | ok |
| P1 | `arrange.break_apart` | Break Apart | — | `Opt+B` | Splits paths into their sub-paths | `{ids?}` | both | ok |
| P2 | `arrange.order.forward` | Push forward in draw order | — | `PgDn` † | Moves the selection one step later in the draw list (visible in wireframe and output order without optimization) | `{ids?}` | both | ok |
| P2 | `arrange.order.backward` | Push backward in draw order | — | `PgUp` † | One step earlier | `{ids?}` | both | ok |
| P2 | `arrange.order.front` | Push to front | — | `Cmd+PgUp` | First in the draw list | `{ids?}` | both | ok |
| P2 | `arrange.order.back` | Push to back | — | `Cmd+PgDn` | Last in the draw list | `{ids?}` | both | ok |
| P1 | `arrange.lock` | Lock Selected Shapes | Shape Properties, context menu | — | Blocks every edit to the selection | `{ids?}` | both | ok |
| P1 | `arrange.unlock` | Unlock Selected Shapes | Shape Properties, context menu | — | Removes the lock | `{ids?}` | both | ok |

**Transform without a menu item** (handles, Numeric Edits toolbar, arrow keys):

| Tier | Id | LightBurn equivalent | Key | What it does | Params | Applies | Agent |
|---|---|---|---|---|---|---|---|
| P0 | `arrange.transform` | Numeric Edits toolbar (XPos, YPos, Width, Height, Rotate, 9-dot anchor, ratio lock) and the Move/Size/Rotate handles | — | Sets position, size and rotation numerically about an anchor point | `{ids?, x_mm?, y_mm?, w_mm?, h_mm?, scale_pct?, rotate_deg?, anchor: tl/t/tr/l/c/r/bl/b/br, lock_ratio?}` | both | ok |
| P0 | `arrange.nudge` | Arrow keys | arrows 5 mm, `Cmd`+arrows 1 mm, `Shift`+arrows 20 mm, `Cmd+Shift`+arrows 0.1 mm (defaults, set in §8.7) | Moves the selection by a fixed step | `{ids?, dx_mm, dy_mm}` | both | ok |
| P0 | `arrange.rotate_by` | Rotate keys | `Shift+.`/`Shift+,` 45°, `Cmd+.`/`Cmd+,` 15°, `Cmd+Shift+.`/`Cmd+Shift+,` 1.5° | Rotates by a fixed step about the anchor | `{ids?, deg, anchor?}` | both | ok |
| P2 | `arrange.shear` | Shear handles | — | Slants the selection | `{ids?, x_deg?, y_deg?}` | both | ok |
| P1 | — (field behaviour) | Equations and unit suffixes in numeric fields (`58+37`, `5in`) | — | Numeric fields evaluate arithmetic and convert units on Enter | — | both | — |

### 3.5 Laser Tools menu
*Source: `UI/LaserToolsMenu`, `PrintAndCut`, `Cameras/CameraSelection`, `RotaryMode/*`, `CylinderCorrection`, `RepeatMarking`, `FocusTest`, `IntervalTest`, `MaterialTest`, `CenterFinder`, `TaperWarp`, `CalibrateGalvoLens`, `Devices`, `DeviceSettings`, `MachineSettings`.*

| Tier | Id | LightBurn item | Also in | Key | What it does | Params | Applies | Agent |
|---|---|---|---|---|---|---|---|---|
| P2 | `laser_tools.print_and_cut` | Print and Cut | Modes toolbar (gantry) | — | Registers the design to two marks on pre-printed stock by jogging to each and recording it | `{target1_id, target2_id, scale: scaled/none}` | gantry | hw |
| P2 | `laser_tools.cameras` | Manage Cameras | — | — | Opens the device's camera list (§5.13) | `{}` | gantry | ui |
| P2 | `laser_tools.rotary_setup` | Rotary Setup | Modes toolbar | `Cmd+Shift+R` | Rotary configuration (chuck or roller, steps per turn, diameters, axis, galvo split size and overlap, motor speeds) | §8.5 | both | ok |
| P2 | `laser_tools.cylinder_correction` | Cylinder Correction Setup | Modes toolbar (galvo) | — | Pre-distorts output so marks on a cylinder keep their shape without a rotary (EZCAD "ProjectMark") | `{enabled, mirror_distance_mm, object_diameter_mm, axis: x/y, surface: convex/concave, show_boundary?}` | galvo | ok |
| P2 | `laser_tools.split_marking` | Split Marking (2.1) | Modes toolbar (galvo) | — | Marks designs bigger than the field in slices, moving a linear or rotary table between slices | §8.5 | galvo | hw |
| P2 | `laser_tools.repeat_marking` | Repeat Marking | — | — | Runs the same job N times, advancing an external table by a fixed step between runs | `{count, increment, pause_after_move_ms?}` | galvo | hw |
| P2 | `laser_tools.focus_test` | Focus Test | — | — | Burns a pattern across Z heights to find focus; needs a motorised Z (galvo support UNVERIFIED, 02 CAL-04) | `{start_z_mm, end_z_mm, steps, speed_mm_s, power_min_pct, power_max_pct}` | gantry | ok (generate); run = arm |
| P1 | `laser_tools.interval_test` | Interval Test | — | — | Generates a strip of fills at different line intervals for one speed and power | `{speed_mm_s, power_pct, steps, min_interval_mm, max_interval_mm, size_mm, style: simple_fill/dithered}` | both | ok (generate); run = arm |
| P1 | `laser_tools.material_test` | Material Test | — | — | Generates a two-parameter test grid; for the MOPA the useful pair is frequency × Q-pulse (§8.2). 02 marks it T0-stretch | §8.2 | both | ok (generate); run = arm |
| P2 | `laser_tools.center_finder` | Center Finder | — | — | Finds the centre of a round (or square, triangular, hexagonal) part from three jogged edge points | `{points_mm[3], shape?, action: move/guide_circle/set_origin}` | gantry | hw |
| P2 | `laser_tools.taper_warp` | Taper Warp | — | — | Compensates for a tapered cup (different top and bottom diameters) in rotary or cylinder modes | `{top: top/left/right, top_diameter_mm, bottom_diameter_mm, length_mm}` | both | ok |
| P1 | `laser_tools.calibrate_galvo_lens` | Calibrate Galvo Lens | — | — | Nine-point lens wizard: burn a test box at 85–90 % of the field, pick the orientation, enter horizontal and vertical measurements, get per-galvo scale, bulge, skew and trapezoid; can enlarge the field. P0 needs only Calibrate Axis (§7.3) | `{field_mm, box_mm, orientation, h_measurements_mm[], v_measurements_mm[], maximize_area?}` | galvo | ok (compute); burn = arm |
| P0 | `device.manage` | Manage Devices | Laser window (Devices) | — | Device list: add, import, export, edit, duplicate, remove, set default (§5.15) | `{}` | both | ui |
| P0 | `device.settings` | Device Settings | Edit menu, Main toolbar | — | Settings for the active device (§7) | `{}` | both | ui |
| P1 | `device.machine_settings` | Machine Settings | — | — | Reads and writes controller firmware parameters, with save/load to file and automatic backups. P1 for GRBL `$$`; P2 Ruida and TopWisdom | `{action: read/write/save/load/load_backup/calibrate_axis, …}` | gantry | hw |

### 3.6 Web Tools menu (2.1)
*Source: `UI/WebTools`.* LightBurn embeds third-party project generators (Cuttle, My Laser Tools) and user-added URLs in an in-app browser. **P3, likely never:** it needs an embedded browser, and Craft-family apps do not ship one. Reserved ids: `web_tools.open {tool}`, `web_tools.manage`.

### 3.7 Window menu
*Source: `UI/WindowMenu`, `ResetToDefaultLayout`, `ZoomPan`, `ViewStyle`, `Preview`.*

| Tier | Id | LightBurn item | Also in | Key | What it does | Params | Applies | Agent |
|---|---|---|---|---|---|---|---|---|
| P2 | `window.reset_layout` | Reset to Default Layout | — | — | Restores the default docking layout | `{}` | both | ui |
| P0 | `preview.open` | Preview | Main toolbar | `Opt+P` | Opens the job simulation (§5.5) | `{}` | both | ui |
| P0 | `view.zoom_in` | Zoom › Zoom In | Main toolbar, wheel | `+` or `Cmd+=` | Zooms the canvas in | `{}` | both | ui |
| P0 | `view.zoom_out` | Zoom › Zoom Out | Main toolbar, wheel | `-` or `Cmd+-` | Zooms out | `{}` | both | ui |
| P0 | `view.zoom_page` | Zoom › Zoom to Page | Main toolbar | `Cmd+0` | Fits the whole work area (galvo field) in view | `{}` | both | ui |
| P0 | `view.zoom_selection` | Zoom › Frame Selection | Main toolbar | `Cmd+Shift+A` | Fits the selection, or everything if nothing is selected | `{}` | both | ui |
| P0 | `view.pan` | Pan / Drag View | Main toolbar | hold `Space`; middle-drag | Pans the canvas | `{dx_px?, dy_px?}` | both | ui |
| P0 | `view.style.wireframe` | View Style › Wireframe | — | — | Draws every shape as an outline (the P0 default) | `{}` | both | ui |
| P1 | `view.style.filled` | View Style › Filled | — | — | Draws Fill-layer shapes solid, showing what the hatch covers | `{}` | both | ui |
| P2 | `view.edge_quality` | View Style › edge rendering (smooth/fast) | — | — | Trades drawing quality for speed | `{smooth: bool}` | both | ui |
| P2 | `window.toggle_side_panels` | Toggle Side Panels | — | `F12` | Hides every docked window; again to restore them | `{}` | both | ui |
| — | `window.show` | Windows and Toolbars › (each panel) | — | — | Shows or hides one panel; panel ids and tiers are in §5 | `{panel, visible}` | both | ui |

### 3.8 Language menu
*Source: `UI/LanguageMenu`.* LightBurn offers about 25 interface languages. **P2:** `prefs.language {code}` (`ui`). English first; Portuguese is the obvious second language for Luiz (PLAN S7 tests the Portuguese dead-key IME).

### 3.9 Help menu
*Source: `UI/HelpMenu`, `HelpAndNotes`, `EnableDebugLog`, `ViewUsageData`, `CheckForUpdates`, `LicenseManagement`, `Hidden/DebugDrawing`.*

| Tier | Id | LightBurn item | Key | What it does in OpenLaser | Agent |
|---|---|---|---|---|---|
| P1 | `help.about` | About LightBurn | — | Version, build, git hash, licences (agents read it through `ro`) | ro |
| P2 | `help.license` | End User License Agreement | — | OpenLaser's licence and third-party notices | ui |
| P3 | `help.forum` | LightBurn Support Forum | — | Opens the project's issue tracker or discussions | ui |
| P3 | `help.feature_request` | Submit Feature Request | — | Same destination | ui |
| P2 | `help.quick_help` | Quick Help and Notes | `F1` † | Hotkey and gesture cheat sheet | ui |
| P2 | `help.docs` | Online Documentation | — | Opens the docs | ui |
| P3 | `help.docs_pdf` | PDF Documentation Link | — | Printable docs | ui |
| P3 | `help.videos` | Online Video Tutorials | — | Tutorial links | ui |
| P3 | — | CorelDRAW Macro Setup Help | — | **Not planned** (Windows and CorelDRAW only) | — |
| P3 | — | View Usage Data | — | **Not planned:** OpenLaser collects no usage data | — |
| P3 | `help.report_bug` | Report a Bug | — | Pre-fills an issue with `help.support_data` attached | ui |
| P2 | `help.support_data` | Generate Support Data | — | Writes a diagnostics bundle (versions, profile, recent log and USB trace, no tokens) | ok |
| P3 | `help.camera_selection` | Camera Selection Help | — | Camera-choice helper | ui |
| P3 | `help.check_updates` | Check for Updates | — | Opt-in release check | ui |
| P3 | — | License Management | — | **Not planned:** no licence keys | — |
| P1 | `help.debug_log` | Enable Debug Log | — | Turns on the detailed log, including the USB trace PLAN §4 shows in the bottom panel | ok |
| P3 | `help.debug_drawing` | Debug Drawing | — | Draws shape bounds on the canvas (developer aid) | ui |
| P3 | `help.network_camera_stats` | Real-Time Statistics | — | Network camera bandwidth | ui |

---

## 4. Toolbars

Most toolbar buttons are menu items already listed above; the "Also in" columns say which. This section lists only the toolbars and the buttons that exist nowhere else.

| Toolbar (LightBurn) | Panel id | Tier | Contents | Extra commands |
|---|---|---|---|---|
| Main | `main_toolbar` | P0 | New, Open, Save, Import, Undo/Redo, Copy, Cut, Paste, Delete, Pan, Zoom (page, in, out, selection), Update camera overlay, Preview, Settings, Device Settings | `camera.update_overlay` (P2, gantry, hw: captures a camera frame) |
| Creation (Tools) | `creation_toolbar` | P0 (Select); P1 (rest) | Select, Draw Lines, Draw Shapes flyout, Edit Nodes, Trim Shapes, Add Tabs, Create/Edit Text, Position Laser, Measure | — |
| Edit Nodes | `edit_nodes_toolbar` | P1 | Appears in node mode; one button per §3.3.1 action | — |
| Modifiers | `modifiers_toolbar` | P1 | Offset, Boolean tools and Weld, Grid Array, Circular Array, Set Start Point, Radius | — |
| Arrange and Arrange (long) | `arrange_toolbar`, `arrange_long_toolbar` | P1 (long: P2) | Group/Ungroup, Flip/Mirror, Align, Make Same Width/Height, Distribute, Move Selected Objects and Move Laser to Selection dropdowns | — |
| Numeric Edits | `numeric_edits_toolbar` | P0 | XPos, YPos, Width, Height, ratio lock, Rotate, 9-dot anchor, mm/in toggle | `arrange.transform` (§3.4); `prefs.set {design_units}` (P1) |
| Text Options | `text_options_toolbar` | P1 | Font, Height, HSpace, VSpace, Align X/Y, Bold, Italic, Upper Case, Welded, Distort, Right-to-Left, Text Mode, Offset | `text.set` (§5.10) |
| Modes | `modes_toolbar` | P2 | Gantry: Rotary, Print and Cut, Split Marking. Galvo: Rotary, Cylinder Correction, Split Marking | — |
| Docking | `docking_toolbar` | P2 | Dock left, right, up, down | — |
| Transform control toggles (bottom left) | `transform_toggles` | P2 | Switches that disable moving, scaling or rotating across the project, to prevent accidents | `prefs.set {allow_move, allow_scale, allow_rotate}` |
| Status bar | `status_bar` | P0 | Pointer coordinates; tool status (selection bounds and count, pen dx/dy/length); connection message (galvo: waiting/connected); active mode | Agents read the same data from `doc_inspect` and `laser.status` |

---

## 5. Dock windows

| Tier | Panel id | LightBurn window | Default place | Applies | Section |
|---|---|---|---|---|---|
| P0 | `cuts_layers` | Cuts / Layers | right | both | 5.1 |
| P0 | `color_palette` | Color Palette | bottom | both | 5.2 |
| P0 | `laser` | Laser | right, below | both | 5.3 |
| P0 | `live_framing` | Live Framing (galvo framing window) | modal-less window | galvo | 5.4 |
| P0 | `preview` | Preview | window | both | 5.5 |
| P1 | `move` | Move | right (tab) | gantry; galvo only EZCad3/BSL (P2) | 5.6 |
| P1 | `console` | Console | right (tab) | gantry (G-code) | 5.7 |
| P1 | `macros` | Macros | right (tab) | gantry (G-code) | 5.8 |
| P1 | `shape_properties` | Shape Properties | right (tab) | both | 5.9 |
| P1 | `text_options` | Text Options (toolbar) | top | both | 5.10 |
| P1 | `variable_text` | Variable Text | right (tab) | both | 5.11 |
| P1 | `material_library` | Material Library | right (tab) | both | 5.12 |
| P2 | `camera_control` | Camera Control / Overlays | right (tab) | gantry | 5.13 |
| P2 | `art_library` | Art Library | right (tab) | both | 5.14 |
| P0 | `devices` | Devices (dialog) | modal | both | 5.15 |
| P2 | `file_list` | File List | right (tab) | gantry (DSP) | 5.16 |
| P2 | `pages` | Pages (2.2) | right (tab) | both | 5.17 |
| P2 | `undo_history` | Undo History (2.1) | right (tab) | both | `edit.undo_history` |
| P2 | `docking` | Docking (toolbar) | top | both | `arrange.dock.*` |
| P1 (new) | `log` | — (OpenLaser: log and USB trace, PLAN §4) | bottom | both | `help.debug_log` |

### 5.1 Cuts / Layers window
*Source: `CutsLayersWindow`, `CutSettingsEditor/*`. 02 LAY-01…05.*

| Tier | Id | LightBurn control | What it does | Params | Applies | Agent |
|---|---|---|---|---|---|---|
| P0 | `layer.select` | Click a row | Makes the layer current; its key settings show below the list | `{layer}` | both | ok |
| P0 | `layer.cut_settings.open` | Double-click a row | Opens the Cut Settings Editor (§6) | `{layer}` | both | ui |
| P0 | `layer.cut_settings.set` | Cut Settings Editor fields; the quick fields below the list (Passes, Speed, Power Max, Power Min, Interval, Frequency, Q-Pulse) | Changes any layer field listed in §6 | `{layer, sub_layer?, …fields}` | both | ok |
| P0 | `layer.cut_settings.get` | — | Returns the full settings of a layer and its sub-layers | `{layer}` | both | ro |
| P0 | `layer.set_mode` | Mode column | Line, Fill (P0), Offset Fill (P1); Image is automatic for images; "Multi" shows when sub-layers exist | `{layer, mode: line/fill/offset_fill}` | both | ok |
| P0 | `layer.set_output` | Output column | Includes or excludes the layer from preview, frame and burn. OpenLaser refuses a layer that is hidden but set to output (PLAN §3.6, B6) | `{layer, on}` | both | ok |
| P0 | `layer.set_show` | Show column | Shows or hides the layer on the canvas | `{layer, on}` | both | ok |
| P1 | `layer.set_air` | Air column | Air assist on or off for the layer | `{layer, on}` | gantry | ok |
| P0 | `layer.reorder` | Up/down arrows, drag | Changes burn order (top runs first unless optimization reorders) | `{layer, to_index}` | both | ok |
| P1 | `layer.copy_settings` / `layer.paste_settings` | Copy / Paste buttons | Copies one layer's settings onto another | `{from}` / `{to}` | both | ok |
| P1 | `layer.delete` | Delete button | Deletes the layer **and every object on it**; asks first (Shift skips the question in LightBurn) | `{layer}` | both | ok |
| P1 | `layer.output_all.{enable, disable, invert}` | Column-header menu | Bulk output switches | `{}` | both | ok |
| P1 | `layer.show_all.{show, hide, invert}` | Column-header menu | Bulk visibility | `{}` | both | ok |
| P1 | `layer.air_all.{on, off, invert}` | Column-header menu | Bulk air assist | `{}` | gantry | ok |
| P2 | `layer.sort_cuts_last` | Column-header menu › Sort Cuts Last | Orders layers from weakest to strongest so that cut-through layers run last | `{}` | gantry | ok |
| P1 | `layer.disable`, `layer.disable_others` | Row menu › Disable this / all but this | Output switches from the row menu | `{layer}` | both | ok |
| P1 | `layer.hide`, `layer.hide_others` | Row menu › Hide this / all but this | Visibility from the row menu | `{layer}` | both | ok |
| P1 | `layer.flash` | Row menu › Flash (or Shift+right-click) | Blinks the layer's objects on the canvas | `{layer}` | both | ui |
| P1 | `layer.make_default` | Cut Settings Editor › Make Default | Saves this layer's settings as the device's default for that colour | `{layer}` | both | ok |
| P1 | `layer.reset_default` | Cut Settings Editor › Reset to Default (Shift: universal defaults) | Restores the colour's default (or the universal one) | `{layer, universal?}` | both | ok |
| P1 | `layer.make_default_all` | Cut Settings Editor › Make Default for All | Makes these settings the default for every colour | `{layer}` | both | ok |
| P1 | `layer.sub_layer.{add, duplicate, delete, move, rename, set_output}` | Cut Settings Editor sub-layer tabs | Manages up to 11 sub-layers that run left to right on the same geometry (§6.7) | `{layer, sub_layer, …}` | both | ok |

### 5.2 Color Palette
*Source: `UI/ColorPalette`.* 30 layer colours `00`–`29` (00 black `#000000`, 01 blue `#0000FF`, 02 red `#FF0000`, 03 green `#00E000`, 04 `#D0D000`, 05 orange `#FF8000`, and so on; the full table is in the source page) and two tool colours **T1**, **T2**.

| Tier | Id | LightBurn action | What it does | Params | Agent |
|---|---|---|---|---|---|
| P0 | `layer.assign` | Click a colour with objects selected | Moves the selection onto that layer | `{layer, ids?}` | ok |
| P0 | `layer.set_current` | Click a colour with nothing selected | Sets the layer that new objects go to | `{layer}` | ok |
| P0 | — (behaviour) | Tool layers T1/T2 | Never burned; used for guides, material outlines, text paths and masks. The OM "Frame Only" mode maps here: Live Framing can trace tool layers alone | — | — |
| P2 | `layer.palette.export` | Downloadable palettes (`.gpl`, `.xml`, `.ase`) | Exports the palette for Inkscape, CorelDRAW, Adobe and Affinity so imports land on the right layers | `{path, format}` | ok |

### 5.3 Laser window
*Source: `LaserWindow`, `CoordinatesOrigin`, `CutSelectedGraphics`, `Devices`. 02 CTL-01…16.*

| Tier | Id | LightBurn control | Key | What it does | Params | Applies | Agent |
|---|---|---|---|---|---|---|---|
| P0 | `laser.status` | Status readout (Disconnected, Ready, Busy with progress and time left, Framing) | — | Returns the driver's last known state, progress and estimate. No new device I/O | `{}` | both | ro |
| P0 | `device.select` | Device dropdown | — | Picks the active device profile; does not connect | `{device}` | both | ok |
| P0 | `device.connect` / `device.disconnect` | (galvo connects on its own; status bar shows "Waiting for connection") | — | Opens or releases the board: init sequence, board lock (PLAN §3.5). Disconnect while busy is a stop | `{}` | both | hw |
| P1 | `device.reconnect` | Right-click Devices (reset USB connection) | — | Closes and reopens the connection. Never automatic mid-job (PLAN §3.5) | `{}` | both | hw |
| P1 | `device.port` | Serial port dropdown | — | Chooses the serial port (or host and port for TCP) | `{port}` or `{host, port}` | gantry | ok |
| P0 | `device.manage` | Devices button | — | §5.15 | — | both | ui |
| P0 | `laser.frame` | Frame (galvo: opens Live Framing) | `F1` (galvo) † | Traces the job with the red light. Galvo options in §5.4. Every frame comes from the same plan as the burn (PLAN §3.6 layer 3) | `{style?, continuous?, …}` | both | hw |
| P1 | `laser.frame_bbox` | Bounding Box Frame | — | Gantry: moves the head around the job's bounding box | `{}` | gantry | hw |
| P1 | `laser.frame_rubber_band` | Rubber Band Frame | — | Gantry: moves around the job's hull | `{}` | gantry | hw |
| P3 | — | Laser on When Framing (Shift+Frame) | — | Fires the beam at Move-window power while framing. **Not planned for galvo**; gantry only behind an explicit profile flag, and then it is an `arm` command | — | gantry | arm |
| P0 | `laser.start` | Start | — | Burns the job. Galvo: Live Framing first, always. OpenLaser also requires a fresh frame receipt of the same plan, the decoded confirmation and a typed ARM (PLAN §3.3, §3.6) | `{selected_only?}` | both | arm |
| P0 | `laser.stop` | Stop | `Esc` while framing or burning †; LightBurn: `Ctrl+Break` | Aborts the frame or job at once and confirms the board is idle (PLAN §3.5: ≤ 250 ms) | `{}` | both | ok (always) |
| P1 | `laser.pause` / `laser.resume` | Pause / Resume | LightBurn: `Pause` key | Holds and resumes a running job. Galvo after hardware item H15 | `{}` | both | hw |
| P2 | `laser.send` | Send (DSP) | — | Uploads the job to controller memory; Shift+Send also starts it | `{name, start?}` | gantry | arm |
| P1 | `file.save_machine_file` | Save GCode / RD / OUT / UD5 | `Opt+Shift+L` | §3.1 | — | gantry | ok |
| P1 | `laser.run_machine_file` | Run GCode / RD / OUT / UD5 | — | Streams a saved controller file to the connected machine | `{path}` | gantry | arm |
| P1 | `laser.home` | Home | — | Runs the homing cycle (G-code) or returns to machine origin (DSP) | `{}` | gantry | hw |
| P1 | `laser.go_to_origin` | Go to Origin | — | Moves the head to the user origin | `{}` | gantry | hw |
| P1 | `laser.start_from` | Start From (Absolute Coords, Current Position, User Origin) | — | Where the job is placed. Galvos always use absolute coordinates | `{mode}` | gantry | ok |
| P1 | `laser.job_origin` | Job Origin (9-point) | — | Which point of the job sits at the start position | `{anchor}` | gantry | ok |
| P0 | `laser.cut_selected` | Cut Selected Graphics | — | Preview, frame and burn only the selection. OM has this ("Selected only") | `{on}` | both | ok |
| P1 | `laser.use_selection_origin` | Use Selection Origin | — | Measures the job origin from the selection only | `{on}` | gantry | ok |
| P1 | `laser.show_last_position` | Show Last Position | — | Marks the head's last reported position on the canvas | `{}` | gantry | ro |
| P2 | `laser.rotary_enable` | Enable Rotary | — | Turns rotary output on | `{on}` | both | ok |
| P2 | `laser.cylinder_enable` | Enable Cylinder Correction | — | Turns cylinder correction on | `{on}` | galvo | ok |
| P0 | `laser.red_light` | Enable Red Light | — | Switches the red pointer on or off through its IO port (OM: port bit 8) | `{on}` | galvo | hw |
| P1 | `optimize.open` | Optimization Settings | — | §8.1 | `{}` | both | ui |

### 5.4 Live Framing window (galvo)
*Source: `GalvoFraming`. 02 GLV-08, GLV-09.*

| Tier | Id or param | LightBurn control | Key | What it does | Agent |
|---|---|---|---|---|---|
| P0 | `laser.frame {style: bounds}` | Bounds | `F1` cycles styles | Red light traces the job's bounding rectangle (fastest) | hw |
| P1 | `laser.frame {style: hull}` | Hull | `F1` | Traces the tightest outline around everything | hw |
| P0 | `laser.frame {style: contour}` | Contour | `F1` | Traces the exact paths (slowest, most accurate). OM has this | hw |
| P1 | `laser.frame {individually: true}` | Frame Individually | — | One bounds or hull frame per shape (not with Contour) | hw |
| P1 | `laser.frame {image_bounds: true}` | Image Bounds | — | Traces the non-blank outline of images instead of their rectangle | hw |
| P1 | `laser.frame {outside_only: true}` | Outside Shapes Only | — | Contour mode skips inner shapes for a faster refresh | hw |
| P0 | `laser.frame {tool_layers_only: true}` | Tool Layers Only | — | Frames only T1/T2 shapes (material outline, jig) | hw |
| P0 | `laser.start`, `laser.stop` | Start, Stop | `Esc` stops | As §5.3 | arm, ok |
| P1 | `laser.pause` | Pause | — | As §5.3 | hw |
| P1 | `laser.counter.get` / `laser.counter.reset` | Count, Reset Count | — | Runs completed; reset also resets Auto-Rotate | ro, ok |
| P0 | `laser.repeat` | Repeat | — | Runs the whole job N times then stops; N is shown in the confirmation (OM has Repeat, 1–999) | ok (setting) |
| P1 | `laser.run_continuously` | Run Continuously | — | **OpenLaser diverges:** an unbounded loop cannot be confirmed from bytes, so it becomes Repeat with an explicit cap, shown at ARM | ok (setting) |
| P1 | `laser.stop_at` | Stop At | — | Blocks Start (and the foot switch) once the count reaches N | ok |
| P1 | `laser.frame.nudge` | Arrow keys while framing | arrows 5 mm, `Shift` 20 mm, `Cmd` 0.5 mm | Moves the artwork and re-frames live | hw |
| P1 | `laser.frame.scale` | Page Up / Page Down while framing | `PgUp`/`PgDn` | Scales the artwork and re-frames | hw |
| P1 | `laser.frame.rotate` | `.` `,` while framing | 15°; `Shift` 1.5°; `Cmd` 45° | Rotates the artwork and re-frames | hw |

The nudge, scale and rotate keys also edit the document. Each edit voids any ARM token, and the next Start needs a new frame receipt (PLAN §3.6).

### 5.5 Preview window
*Source: `Preview`. 02 PRV-01…06.*

| Tier | Id | LightBurn control | What it does | Params | Agent |
|---|---|---|---|---|---|
| P0 | `preview.open` | Window › Preview (`Opt+P`) | Simulates the job: burn moves in one colour, travel in another, work area outlined | `{}` | ui |
| P0 | `preview.stats` | Job time statistics | Burn and travel distance and time, total estimate. Galvo estimate uses mark and jump speeds plus delays (PLAN §3.2 time model) | `{}` | ro |
| P0 | `render_preview` (MCP tool) | — | PNG of the canvas or of the simulation for agents | `{what: canvas/preview, max_side?}` | ro |
| P1 | `preview.seek` | Time slider | Shows the job up to a point in time | `{t_s}` | ui |
| P1 | `preview.play` | Play, playback speed (1/10× to 40×) | Animates the job | `{speed}` | ui |
| P2 | `preview.start_here` | Start Here | Resumes an interrupted job from the slider position | `{t_s}` | arm |
| P1 | `preview.save_image` | Save Image | Saves the simulation as an image | `{path}` | ok |
| P1 | `preview.settings` | Preview settings (shade by power, invert, legend, show travel) | Display options for the simulation only | `{shade_by_power?, invert?, legend?, show_travel?}` | ok |

### 5.6 Move window (gantry)
*Source: `MoveWindow`. 02 CTL-06. Every motion command is `hw`. On galvos LightBurn shows this window only for EZCad3/BSL (Z axis), so the galvo variant is P2.*

| Tier | Id | LightBurn control | What it does | Params |
|---|---|---|---|---|
| P1 | `move.get_position` | Get Position | Reads X, Y, Z, U from the controller | `{}` |
| P1 | `move.go_to` | Move to Position, Go | Moves to typed coordinates | `{x_mm, y_mm}` |
| P1 | `move.saved.{add, remove, go}` | Saved Positions | Named positions to return to | `{name, x_mm?, y_mm?}` |
| P1 | `move.jog` | Arrow buttons, keyboard, numeric keypad | Steps the head by Distance at Speed | `{direction, distance_mm, speed_mm_s}` |
| P1 | `move.continuous_jog` | Continuous Jog | Moves while the button is held (GRBL `$J`) | `{on}` |
| P2 | `move.from_machine_zero` | Move From Machine Zero | Measures moves from machine zero | `{on}` |
| P2 | `move.focus_z` | Focus Z | Runs the controller's autofocus | `{}` |
| P1 | `move.set_origin` / `move.clear_origin` | Set Origin / Clear Origin | Sets or clears the user origin at the current position | `{}` |
| P1 | `move.set_finish_position` | Set Finish Position | Where the head parks after a job (G-code only) | `{}` |
| P2 | `move.fire` | Fire, Power | Fires the beam at a set power while held. **arm**, and diode-only by default | `{power_pct}` |
| P1 | `move.override.{speed, power}` | Adjust Speed / Power during a G-code job | Live feed and power overrides (GRBL real-time bytes) | `{pct}` |
| P2 | `move.z.{jog, home, go}` | Z readout, Home Z, Move to Z | Galvo Z on EZCad3/BSL; gantry Z | `{z_mm}` |

### 5.7 Console (G-code)
*Source: `ConsoleWindow`.* **P1, gantry, `hw`.** `console.send {line}` sends a raw line (`$I`, `$$`, `$#`, `?`, `$X`, …); `console.show_all {on}` shows every exchanged line instead of startup, alarms and errors only. The log is readable through `console.log` (`ro`).

### 5.8 Macros (G-code)
*Source: `MacrosWindow`.* **P1, gantry.** `macro.run {name}` (`hw`) sends a stored multi-line G-code snippet; `macro.{add, edit, delete, reorder}` (`ok`) manage them; per-macro hotkeys are P2.

### 5.9 Shape Properties
*Source: `ShapeProperties`.* **P1, `ok`.** `shape.set {ids?, …}`:

| Key | Unit | Applies to | Meaning |
|---|---|---|---|
| `priority` | integer ≥ 0 | any object | Cut order when Order by Priority is on (§8.1) |
| `power_scale_pct` | % | any object | Scales this object's power within the layer's min–max |
| `locked` | bool | any object | Same as `arrange.lock` |
| `gamma`, `contrast`, `brightness`, `enhance_radius`, `enhance_amount`, `denoise` | — | images | Same fields as Adjust Image (§8.3) |
| `w_mm`, `h_mm` | mm | ellipses, polygons, rectangles | Parametric size |
| `sides` | count | polygons | Number of sides |
| `corner_radius_mm` | mm | rectangles | Negative = notched corner |
| `max_width_mm`, `squeeze` | mm, bool | text | Shrinks text that exceeds a width; squeeze shrinks only horizontally |

### 5.10 Text Options toolbar
*Source: `Text`, `VariableText/VariableTextFormatting`.* **P1, `ok`.** `text.set {ids?, font, height_mm, hspace_pct, vspace_pct, align_x: left/middle/right, align_y: bottom/middle/top, bold, italic, upper, welded, distort, rtl, mode: normal/datetime/csv/serial/cut_setting, offset}`. Also: `text.shx_path {path}` (P1: single-line SHX fonts matter for galvo and plotter work), `text.font_manager` (P2: favourites, hide fonts). Text saved in a project keeps a vector backup for machines without the font.

### 5.11 Variable Text window
*Source: `VariableText`.* **P1, `ok`.** Serial plates are a core MOPA use (02 DES-04).

| Tier | Id | LightBurn control | What it does |
|---|---|---|---|
| P1 | `vartext.set {current, start, end, advance_by, auto_advance}` | Current, Start, End, Advance By, Auto-Advance | Serial-number counter and its range |
| P1 | `vartext.next` / `vartext.prev` / `vartext.reset` | Next, Previous, Reset | Steps the counter |
| P1 | `vartext.test` | Test | Shows the substituted text without burning |
| P1 | `vartext.bake` | Bake | Replaces variable text with its current literal value |
| P1 | `vartext.csv.{browse, clear}` | Browse, Clear | Attaches or detaches a CSV for merge fields |

The formatting codes (date, serial, CSV, cut-setting fields including `q`/`Q` Q-pulse and `k`/`K` kHz) follow LightBurn's syntax for compatibility (02 DES-05).

### 5.12 Material Library
*Source: `MaterialLibrary`.* **P1, `ok`.** Libraries hold presets organised as material › thickness › description.

| Id | LightBurn control | What it does |
|---|---|---|
| `material.assign {preset, layer}` | Assign | Copies a preset's settings onto a layer |
| `material.link {preset, layer}` | Link | Ties the layer to the preset so library edits follow it |
| `material.create {from_layer, material, thickness_mm?, title?, description}` | Create new from layer | New preset from the current layer |
| `material.{edit_cut, update, edit_description, duplicate, delete} {preset}` | Edit Cut, Update, Edit Description, Duplicate, Delete | Preset maintenance |
| `material.library.{load, save, save_as, new, merge, select, rename, unload}` | Library buttons | File-level library management; per-device auto-load |

OpenLaser stores galvo and gantry blocks separately in presets too (PLAN §2). MOPA colour libraries per material are a P3 extra (02 §19).

### 5.13 Camera Control / Overlays (P2, gantry)
*Source: `Cameras/*`.* `camera.update_overlay` (hw), `camera.trace` (opens Trace Image on the capture), `camera.clear`, `camera.opacity {pct}`, `camera.adjust {w_mm, h_mm, x_shift_mm, y_shift_mm}`, `camera.add`, `camera.manage`, `camera.import {path}`, `camera.calibrate_lens` (AprilTag wizard: auto or manual capture, model choice, export), `camera.align` (burn a target, then automatic or manual alignment; burning it is `arm`), `camera.edit {name, source, resolution, …}`. LiDAR scanning (ACCELaser only) is P3. LightBurn has no camera alignment for galvos; doing it for the MOPA field is a P3 differentiator (02 §14).

### 5.14 Art Library (P2)
*Source: `ArtLibrary`.* `art.library.{new, load, unload}`, `art.add {ids? or paths}`, `art.place {item, at_mm?}`, `art.{rename, delete} {item}`. Artwork only, no cut settings.

### 5.15 Devices window
*Source: `Devices`, `GetStarted/AddGalvo`.*

| Tier | Id | LightBurn control | What it does | Agent |
|---|---|---|---|---|
| P0 | `device.list` | Device list with summary | Lists profiles, the default and the active one | ro |
| P1 | `device.find` | Find My Laser | Scans USB (and, later, the network) for supported boards. Same probe as `device.detect` (§9) | hw |
| P0 | `device.create` | Create Manually (New Device Wizard) | New profile: kind (galvo LMC, GRBL, export-only), name, field or bed size, origin. For the galvo the wizard imports an EZCAD `markcfg` (`device.import_ezcad`, §9) and, at P1, its COR file | ok |
| P3 | — | LightBurn Bridge | Ruida-only network relay. Not planned | — |
| P1 | `device.import` | Import (`.lbzip`, `.lbdev`, `.lbvendor`) | `.lbdev` (JSON) first; it is cheap interop (02 DEV-10) | ok |
| P1 | `device.export` | Export | Writes the selected profiles | ok |
| P1 | `device.make_default` | Make Default | Profile used at launch | ok |
| P0 | `device.edit` | Edit (double-click) | Reopens the wizard for the profile | ok |
| P1 | `device.remove` | Remove | Deletes the profile | ok |
| P1 | `device.duplicate` | Duplicate (context menu) | Copy of a profile; LightBurn recommends one profile per galvo lens | ok |
| P1 | `device.rename` | Rename | | ok |
| P3 | `device.set_icon`, `device.order_by` | Set Icon, Order by Name/Type | Cosmetic | ok |

### 5.16 File List (P2, DSP)
*Source: `FileListWindow`.* `file_list.{refresh, start, upload, delete, calc_time, delete_all}` for jobs stored on Ruida-class controllers. `start` is `arm`; the rest are `hw`.

### 5.17 Pages (P2)
*Source: `Pages`.* Multi-page projects (2.2): `pages.{add, duplicate, delete, rename, select}`. Shapes belong to a page; cut settings are shared. Nesting can spill onto new pages.

---

## 6. Cut Settings Editor: layer fields

*Source: `CutSettingsEditor/SharedSettings`, `LineMode`, `FillMode`, `OffsetFillMode`, `ImageMode`, `GalvoSpecificCutSettings`, `SubLayers`, `WorkingWithDefaults`; the LightBurn REST field list (02 §7.7); research 05 §1.8 for LMC encodings.*

All fields are written through `layer.cut_settings.set {layer, sub_layer?, …}` and read through `layer.cut_settings.get`. **Layer parameters live in separate `galvo` and `gantry` blocks** (PLAN §2; bug B11). One project can carry both and retarget between the MOPA and a GRBL machine. Keys below are shown with their block prefix.

### 6.1 Shared fields

| Tier | Key | LightBurn field | Unit | Range and notes | Applies |
|---|---|---|---|---|---|
| P0 | `name` | Name | text | | both |
| P0 | `output` | Output switch | bool | Hidden + output is refused (PLAN §3.6) | both |
| P0 | `mode` | Mode | enum | `line`, `fill` (P0); `offset_fill` (P1); `image` automatic for images; `tool` for T1/T2 | both |
| P0 | `passes` | Number of Passes | count | ≥ 1 | both |
| P1 | `global_passes` | Global Passes | count | Repeats the whole sub-layer stack: total = global × per-sub-layer passes | galvo |
| P1 | `priority` | (layer order) | integer | Same as `layer.reorder` | both |
| P1 | `gantry.air_assist` | Air Assist | bool | M7 or M8 chosen in device settings | gantry |
| P2 | `gantry.z_offset_mm`, `gantry.z_step_mm` | Z Offset, Z Step per Pass | mm | Gantry Z; galvo Z only on EZCad3/BSL | gantry |
| P3 | `galvo.use_laser2` | Use Laser 2 | bool | Needs a second source port | galvo |

### 6.2 Galvo per-layer fields (the MOPA layer)

Speed and power have no shared block here: the galvo has its own.

| Tier | Key | LightBurn field | Unit | Range, default and encoding | Notes |
|---|---|---|---|---|---|
| P0 | `galvo.speed_mm_s` | Speed | mm/s | > 0 and ≤ the device Max Speed. Encoded once to whole units: `trunc(v · 65535 / F / 1000)`, with F = 200 × 1.1834 = 236.68 mm, so one unit ≈ 3.61 mm/s. **A speed that encodes to 0 units is refused** (PLAN §3.3, B7). The UI shows the requested and the actual (encoded) speed | OM default 1000 mm/s |
| P0 | `galvo.power_pct` | Max Power | % | 0–100 → MarkCurrent `round_half_even(p × 4095 / 100)`, so 30 % → 1228 | LightBurn: UV power is a different scale |
| P1 | `galvo.min_power_pct` | Min Power | % | Only for Grayscale image mode | |
| P0 | `galvo.frequency_khz` | Frequency | kHz (stored as Hz) | Must lie within the profile's `MINPWMFREQ`…`MAXPWMFREQ`, 1–4000 kHz on Luiz's profile; outside → refused. Encoded as Q-switch period `round_half_even(20000 / kHz)` | JPT M7: each pulse width has a cutoff frequency for full power (forum data, 06 G7). Showing effective power is a P1 UX item |
| P0 | `galvo.q_pulse_ns` | Q-Pulse Width | ns | Must be on the source's table. JPT M7 default: 2, 4, 6, 9, 13, 20, 30, 45, 55, 60, 80, 100, 150, 200, 250, 300, 350, 400, 450, 500. Within 0.5 ns it snaps with a notice; further off it is refused (OM `safety.py`). Shown only when the device enables Q-pulse | OM default 200 ns |
| P0 | `fill.interval_mm` | Line Interval (or Lines per Inch = 25.4 / interval) | mm | > 0. Fill, Offset Fill and Image | OM calls it Pitch |
| P0 | `fill.angle_deg` | Scan Angle | deg | 0 = horizontal lines advancing upward | OM Hatch angle |
| P0 | `fill.bidirectional` | Bi-directional Fill | bool | OM always serpentine; P0 adds the one-way option | |
| P0 | `fill.cross_hatch` | Cross-Hatch | bool | Second pass at +90° | |
| P0 | `fill.angle_increment_deg` | Angle Increment | deg per pass | Rotates the scan angle on each pass of a multi-pass layer | OM Angle step/pass |
| P2 | `fill.auto_rotate` | Auto-Rotate | bool | Advances the start angle on each run of the job; reset with the counter | |
| P1 | `fill.grouping` | Fill Grouping | enum | `all_at_once` (OM behaviour), `by_group`, `individually` | |
| P1 | `fill.flood_fill` | Flood Fill | bool | Fewer travel moves; similar to EZCAD's snake hatch | |
| P1 | `fill.dot_width_mm` | Dot Width Adjust | mm | Shortens scan lines by about half the spot size; Fill and Image | |
| P0 | — (behaviour) | Nested shapes on one layer | — | Even-odd: holes stay blank, overlaps cancel. Shapes on different layers each fill fully. OM does this | |
| P1 | `line.wobble.enabled`, `line.wobble.step_mm`, `line.wobble.size_mm` | Wobble (Step, Size) | bool, mm, mm | Small circles along the path for a wider, deeper line | |
| P1 | `galvo.timings.override` | Override Default Timings (Show Timings) | bool | Off = the device defaults in §7.4 | |
| P1 | `galvo.timings.jump_speed_mm_s` | Jump Speed | mm/s | | |
| P1 | `galvo.timings.min_jump_delay_us` | Min Jump Delay | µs | | |
| P1 | `galvo.timings.max_jump_delay_us` | Max Jump Delay | µs | | |
| P1 | `galvo.timings.jump_distance_limit_mm` | Jump Distance Limit | mm | Jumps longer than this get the max delay; shorter ones are interpolated | |
| P1 | `galvo.timings.laser_on_tc_us` | Laser On TC | µs | Can be negative in EZCAD; LMC encodes the sign; 0 encodes as the `0x8000` sign word until H9 | EZCAD name LASERONDELAY |
| P1 | `galvo.timings.laser_off_tc_us` | Laser Off TC | µs | | LASEROFFDELAY |
| P1 | `galvo.timings.end_tc_us` | End TC | µs | | MARKDELAY |
| P1 | `galvo.timings.polygon_tc_us` | Polygon TC | µs | | POLYGONDELAY |
| P1 | `galvo.pause_before_ms`, `galvo.pause_after_ms` | Pause Before, Pause After | ms | Per sub-layer | |
| P1 | `galvo.dot.start_delay_us`, `galvo.dot.end_delay_us` | (galvo dot delays, REST `start_delay`/`end_delay`) | µs | | 02 LIN-07 |

### 6.3 Line mode (gantry-relevant extras)

| Tier | Key | LightBurn field | Unit | Notes | Applies |
|---|---|---|---|---|---|
| P0 | `gantry.speed_mm_s` | Speed | mm/s (shown as mm/min when the device's control units ask) | Above the machine maximum is refused, not capped (PLAN §3.6) | gantry |
| P0 | `gantry.power_pct` | Max Power | % | Scaled to the device's S-value max | gantry |
| P1 | `gantry.min_power_pct` | Min Power | % | DSP, or Grayscale images | gantry |
| P1 | `gantry.constant_power` | Constant Power Mode (GRBL only) | bool | M3 instead of the default M4 | gantry |
| P1 | `line.kerf_mm` | Kerf Offset | mm | Closed shapes only; positive = outward | gantry |
| P1 | `line.tabs.{enabled, size_mm, even_spacing, count_per_shape, spacing_mm, limit_max, max_count, cut_power_pct, skip_inner}` | Tabs / Bridges | mixed | Manual tabs are `tools.add_tabs`; `layer.tabs.clear` removes all | gantry |
| P2 | `line.perforation.{enabled, cut_mm, skip_mm}` | Perforation Mode | mm | | gantry |
| P2 | `line.start_pause_ms`, `line.end_pause_ms` | Start / End Pause Time | ms | Not for galvo | gantry |
| P2 | `line.overcut_mm` | Overcut | mm | Closed shapes, final pass | gantry |
| P2 | `line.lead_in.{enabled, angle_deg, length_mm, style: line/arc}`, `line.lead_out.{…}` | Lead-In / Lead-Out | mixed | With start points from Set Start Point | gantry |
| P3 | `line.dot_mode.{enabled, time_ms, spacing_mm}` | Dot Mode (Ruida) | ms, mm | | gantry |
| P3 | `gantry.pwm_override_khz`, `gantry.ppi`, `gantry.u_offset_mm` | Override PWM Frequency, Enable PPI, U Offset | kHz, pulses/in, mm | RF tubes, Trocen, U axis | gantry |
| P2 | `gantry.cut_through.{enabled, power_pct}` | Cut Through | % | DSP | gantry |

### 6.4 Fill and Offset Fill extras (gantry)

| Tier | Key | LightBurn field | Unit | Notes |
|---|---|---|---|---|
| P1 | `fill.overscan.{enabled, pct}` | Overscanning | % of speed | Not for DSP or galvo |
| P2 | `fill.ramp.{length_mm, outer}` | Ramp Length, Ramp Outer Edge | mm | Sloped edges (stamps); not galvo |
| P1 | — | Offset Fill mode | — | Concentric contours (EZCAD ring hatch); interval, passes and grouping apply; angle, bidirectional, cross-hatch and overscan do not. LightBurn's docs warn it can hang on complex art; a robust Rust offsetter is a chance to do better (02 §7.5) |

### 6.5 Image mode

| Tier | Key | LightBurn field | Unit | Notes |
|---|---|---|---|---|
| P0 | `image.mode` | Image Mode | enum | **P0: `threshold` and `jarvis`** (so an imported photo can be burned end to end). P1: `ordered`, `atkinson`, `dither` (Floyd–Steinberg), `stucki`, `grayscale`. P2: `newsprint`, `halftone`, `sketch`, `sliced_3d` (galvo only) |
| P0 | `image.interval_mm` / `image.dpi` | Line Interval / DPI | mm / dots per inch | DPI = 25.4 / interval |
| P0 | `image.bidirectional` | Bi-directional Scanning | bool | |
| P0 | `image.negative` | Negative Image | bool | For materials that burn lighter (slate, some metals) |
| P0 | `image.angle_deg` | Scan Angle | deg | |
| P1 | `image.angle_increment_deg` | Angle Increment | deg per pass | |
| P1 | `image.pass_through` | Pass-Through | bool | Burns pre-dithered pixels 1:1 |
| P1 | `image.dot_width_mm` | Dot Width Correction | mm | |
| P1 | `image.overscan` | Overscanning | bool | Not galvo |
| P2 | `image.halftone.{cells_per_inch, angle_deg}` | Cells per Inch, Halftone Angle | — | Halftone only |
| P2 | `image.ramp_length_mm` | Ramp Length | mm | Threshold only |
| P2 | `image.sliced_3d.{cleanup, clean_after_passes, clean_whole_image}` | 3D Sliced: Enable cleanup pass, Clean after # of passes, Clean whole image | — | Galvo depth maps; one pass per grey level |
| P1 | `fill.grouping` | Fill Grouping | enum | Shared with Fill |

How LightBurn does Grayscale on a fiber galvo (per-pixel power or not at all) is UNVERIFIED (02 §21 q4).

### 6.6 Defaults
*Source: `CutSettingsEditor/WorkingWithDefaults`.* LightBurn's universal default is a Line layer at 100 mm/s and 20 % power. OpenLaser takes defaults from the device profile's per-kind `layer_defaults` (PLAN §3.2), never from another machine kind. Commands: `layer.make_default`, `layer.reset_default`, `layer.make_default_all` (§5.1); preference `prefs.load_default_layers_on_new` (§8.7).

### 6.7 Sub-layers
*Source: `CutSettingsEditor/SubLayers`.* **P1** (02 marks it T0-stretch: outline plus fill plus a cleanup pass on one colour is everyday MOPA work). Up to 11 sub-layers per layer run left to right on the same geometry, each with its own mode, settings, output switch and name. The layer shows "Multi" in Cuts / Layers. Galvo Global Passes repeats the whole stack. EZCAD's Mark Contour and Hatch 1/2/3 translate to sub-layers (02 GLV-20). Commands: `layer.sub_layer.{add, duplicate, delete, move, rename, set_output}`.

---

## 7. Device Settings

*Source: `DeviceSettings/GalvoBasicSettings`, `DeviceSettings/GalvoPorts`, `CalibrateGalvoLens`, `DeviceSettings/*` (gantry tabs). 02 §5.*

All galvo keys are written with `device.settings.set {device?, …}` (`ok`: it edits the profile, not the board) and read with `device.settings.get` (`ro`). PLAN §3.2 `GalvoMachine` holds them. **Unless a row says otherwise, everything in this section is galvo-only.**

### 7.1 Field and control units

| Tier | Key | LightBurn setting | Unit | Notes |
|---|---|---|---|---|
| P0 | `galvo.field.width_mm`, `galvo.field.height_mm` | Field Width/Height | mm | The lens's rated field, which is also the canvas work area. Luiz: 200 × 200. The work area may shrink, and grows only as far as hardware item H27 cleared (PLAN §3.2). The encoding field F = nominal × scale is used only for unit conversion |
| P0 | `galvo.field.offset_x_mm`, `galvo.field.offset_y_mm` | Field Offset X/Y | mm | Shifts the whole field |
| P0 | `galvo.field.angle_deg` | Field Angle | deg | Compensates for a head mounted rotated against the jig. Part of the task's "mirror/rotate" P0 item (02 rated it T1) |
| P1 | `device.control_units` | Control Units | enum | mm / mm/s (P0 fixed); inches variants P1 |

### 7.2 Red dot

| Tier | Key | LightBurn setting | Unit | Notes |
|---|---|---|---|---|
| P0 | `galvo.red_dot.offset_x_mm`, `galvo.red_dot.offset_y_mm` | Red Dot Offset X/Y | mm | Applied to the lighting list only, so the red frame lands where the beam burns. Identity until hardware item H26 measures it (PLAN §3.2, §3.6 layer 3) |
| P0 | `galvo.red_dot.scale_x`, `galvo.red_dot.scale_y` | Red Dot Scale X/Y | factor (LightBurn's unit UNVERIFIED: % or ratio) | |
| P1 | `galvo.red_dot.rest_x_mm`, `galvo.red_dot.rest_y_mm` | Rest Pos X/Y | mm | Where the pointer parks when idle |

### 7.3 Galvo configuration (lens correction)

| Tier | Key | LightBurn setting | Unit | Notes |
|---|---|---|---|---|
| P1 | `galvo.cor.load {path}` / `galvo.cor.clear` | Load COR File / Clear | file | EZCAD lens correction table. P1, since OM runs without one today. PLAN M2 reads `.cor` |
| P0 | `galvo.axis[1/2].is_x` | Galvo 1/2 is X Axis | bool | Swaps the axes, which fixes output rotated by 90° (markcfg `SWAPXY`) |
| P0 | `galvo.axis[1/2].negate` | Reverse Direction (Negate) | bool | Mirrors one axis (markcfg `INVERTX`/`INVERTY`). OM stores mirroring in browser storage; PLAN §3.2 moves it into the profile |
| P0 | `galvo.axis[1/2].scale_pct` | Scale | % | Per-axis size correction. OM has one uniform factor, 1.1834 (in the gitignored `calibration.json`); P0 generalises it per axis and imports the old value |
| P0 | `device.calibrate_axis` | "…" button › Calibrate Axis | mm | `{axis: 1/2, requested_mm, measured_mm}` → new scale = old × requested / measured. Typical test: a 100 mm line on each axis |
| P1 | `galvo.axis[1/2].bulge` | Bulge | — | Barrel or pincushion correction near the centre |
| P1 | `galvo.axis[1/2].skew` | Skew | — | Edge distortion |
| P1 | `galvo.axis[1/2].trapezoid` | Trapezoid | — | Keystone correction |
| P0 | `galvo.frame_speed_mm_s` | Frame Speed | mm/s | Red-light speed; OM fixes 3000 mm/s and ignores the profile's `REDLIGHTSPEED` |
| P0 | `galvo.max_speed_mm_s` | Max Speed | mm/s | Ceiling for layer speeds; above it is refused. OM profile: 6000 mm/s |

### 7.4 Jump and delay defaults

| Tier | Key | LightBurn setting | Unit | Notes |
|---|---|---|---|---|
| P0 | `galvo.jump.speed_mm_s` | Jump Speed | mm/s | OM: 3000 |
| P0 | `galvo.jump.min_delay_us` | Min Jump Delay | µs | OM/galvoplotter short delay: 8 µs, unverified (H9) |
| P0 | `galvo.jump.max_delay_us` | Max Jump Delay | µs | galvoplotter long delay: 200 µs; PLAN uses it plus a 1 ms settle for every list's first jump |
| P0 | `galvo.jump.distance_limit_mm` | Jump Distance Limit | mm | Between min and max delay, interpolated by distance |
| P0 | `galvo.delay.laser_on_tc_us` | Laser On TC | µs | galvoplotter default 100. OM parses the profile value but never applies it (05 §1.8) |
| P0 | `galvo.delay.laser_off_tc_us` | Laser Off TC | µs | Default 100 |
| P0 | `galvo.delay.end_tc_us` | End TC | µs | |
| P0 | `galvo.delay.polygon_tc_us` | Polygon TC | µs | Default 100 |

### 7.5 Other options

| Tier | Key | LightBurn setting | Notes |
|---|---|---|---|
| P2 | `galvo.play_sound_on_finish` | Play Sound on Job Finished | |
| P0 | — (fixed) | Require Framing Before Start | **Always on for galvos in OpenLaser; not a setting.** Start needs a frame receipt of the same plan from this session, at most 10 minutes old (PLAN §3.6 layer 5) |
| P0 | `device.checklist` | Enable Job Checklist (and its Edit button) | The checklist ticked at ARM. LightBurn makes it optional; OpenLaser always shows it |
| P2 | `device.save_rotary_with_project` | Save/Load Rotary Settings with Project Files | |

### 7.6 IO ports
*Each port is chosen from the board's input or output bits, with a high or low active level. LightBurn shows live input states.*

| Tier | Key | LightBurn setting | Notes |
|---|---|---|---|
| P0 | `galvo.ports.red_dot` | Red Dot | Output bit that switches the pointer; OM uses bit 8. Needed for `laser.red_light` |
| P2 | `galvo.ports.red_dot_always_on` | Red Dot Always On | |
| P1 | `galvo.ports.blanking` | Enable Blanking | Turns the pointer off between shapes; disable it if contour framing fails |
| P2 | `galvo.ports.ready_light`, `galvo.ports.busy_light` | Ready Light, Busy Light | |
| P2 | `galvo.ports.start_framing` | Start Framing (input) | |
| P1 | `galvo.ports.start_marking`, `galvo.ports.full_pulse_to_start` | Start Marking (foot switch), Require 'Full Pulse' to Start | PLAN M3 makes a foot switch the physical ARM input (after H11). Never set unless a switch is wired, to avoid noise triggering |
| P1 | `galvo.ports.stop_marking` | Stop Marking (input) | External stop button; 02 marks it T0-stretch |
| P2 | `galvo.ports.done_marking` | Done Marking (output) | |
| P1 | `galvo.ports.door_protect` | Door Protect (interlock input) | Read before MO opens and in every wait loop. It adds protection and is never the primary protection (PLAN §3.6 layer 7). 02 marks it T0-stretch |
| P3 | `galvo.ports.second_source`, `galvo.ports.analog_laser2` | Select 2nd Laser Source, Enable Analog Output (Laser 2) | |
| P2 | `galvo.ports.focus_light` | Focus Light | Second pointer for focusing |

### 7.7 Laser source

| Tier | Key | LightBurn setting | Unit | Notes |
|---|---|---|---|---|
| P0 | `galvo.source.type` | Laser Type | enum | `fiber` at P0; `co2`, `uv`, `spi` at P2 |
| P0 | `galvo.source.fiber_type` | Fiber Type | enum | `jpt` (Luiz's M7) at P0; `raycus`, `ipg_ylp`, `ipg_ylpm`, `max` at P1 |
| P0 | `galvo.source.q_pulse_enabled` | Enable Q-Pulse Width Setting | bool | MOPA only; shows the per-layer Q-pulse field (markcfg `m_bEnableIPGSetPulseWidth`) |
| P0 | `galvo.source.pulse_table_ns` | — (OpenLaser addition) | ns list | Accepted pulse widths; default JPT M7 table (§6.2); profile key `PULSEWIDTHS` overrides |
| P0 | `galvo.source.mo_only_when_running` | MO Enabled ONLY When Running | bool | MOPA: the master oscillator is opened at job start and closed after the board reports idle, never while busy (PLAN §3.5). P0 fixes it on |
| P0 | `galvo.source.open_mo_delay_us` | Open MO Delay | µs | Settle time after opening MO; OM: 8 ms (`DelayTime(800)` in 10 µs units, 05 §1.8) |
| P0 | `galvo.source.pwm_min_khz`, `galvo.source.pwm_max_khz` | Enable PWM, min/max frequency | kHz | Always on for fiber; the bounds layer frequencies are checked against (markcfg `MINPWMFREQ`/`MAXPWMFREQ`, 1–4000 kHz) |
| P0 | `galvo.source.max_power_pct` | — (OpenLaser addition, PLAN §3.2 `LaserSource`) | % | Power ceiling for the source |
| P3 | `galvo.source.tickle.*`, `galvo.source.fpk.*` | Enable Tickle, First Pulse Killer | — | CO2 galvo only |
| P3 | `galvo.source.uv_min_pulse`, `galvo.source.uv_max_pulse` | UV Min/Max Pulse | — | UV only |
| P3 | `galvo.source.spi_simmer` | SPI Simmer Current | — | SPI only |

### 7.8 Z axis and external axes (P2)
`galvo.z.{enabled, motor, steps_per_unit_count, steps_unit_count, accel_ms, min_steps_s, max_steps_s, homing_timeout_s, homing_steps_s}` (LightBurn removed EZCad2 Z control in 2.1). External X/Y/Z axes and "Additional EZCad3 Settings" belong to EZCad3/BSL boards: P2.

### 7.9 Gantry device tabs (summary)
*02 DEV-01…DEV-10 has the full field lists.*

| Tier | Tab | Key fields |
|---|---|---|
| P1 | Dimensions / Units | Working size, origin corner, machine coordinates, laser (red-dot) offset, design and control units. Z options P2 |
| P1 | Basic Settings (G-code) | Auto-home on startup (OpenLaser default off), swap X/Y, `$J` jogging, laser fire button (opt-in), out-of-bounds check (refusal in OpenLaser), air assist M7/M8, S-value max, baud rate, network port, transfer mode buffered/synchronous, position polling, return to finish position, job checklist, frame continuously |
| P1 | GCode | Start and end scripts, G0 for overscan, S on every G1 |
| P1 | Additional Settings | Max speeds, accelerations, corner accuracy, speed scales for the time estimate; read from controller |
| P2 | Custom GCode | Dialect templates and variables (02 DEV-05) |
| P2 | DSP basic | Ruida/Trocen/TopWisdom options |
| P2 | Scanning Offset Adjust | Speed-to-offset table for bidirectional ghosting |

---

## 8. Dialogs and wizards

### 8.1 Optimization Settings
*Source: `OptimizationSettings`. 02 OPT-01…11.* **P1** overall. P0 ships the fixed order the prototype uses (layer order, then drawing order); PLAN M2 adds travel ordering. `optimize.set {…}` / `optimize.get` (`ok`/`ro`); `optimize.save_defaults`, `optimize.reset_defaults`.

| Tier | Key | LightBurn setting | Notes |
|---|---|---|---|
| P1 | `enabled` | Enable Optimization and Output Re-ordering | Off = drawing order |
| P1 | `order_by[]` | Order By: Layer, Group, Priority (stackable) | Layer order is P0 behaviour |
| P1 | `inner_first` | Cut inner shapes first | Matters for gantry cutting |
| P2 | `direction_order` | Cut in direction order | |
| P1 | `reduce_travel` | Reduce travel moves | 02 rates it T0 for galvo jump time; PLAN M2 |
| P2 | `hide_backlash` | Hide backlash | Gantry |
| P2 | `reduce_direction_changes` | Reduce direction changes | Gantry |
| P1 | `best_start_point`, `prefer_corners` | Choose best starting point; Choose corners, if possible | |
| P1 | `best_direction` | Choose best direction | |
| P1 | `remove_overlaps`, `overlap_distance_mm` | Remove Overlapping Lines | |

`.lbrn2` files carry these settings (`Optimize_*` in `<UIPrefs>`), and opening one overwrites the current values (02 §17.2).

### 8.2 Material Test
*Source: `MaterialTest`. 02 CAL-01.* **P1** (02: T0-stretch). `laser_tools.material_test {…}` builds the grid as an ordinary project or overlay; preview and frame use the normal commands, and Start is `laser.start` (`arm`).

| Key | LightBurn setting | Notes |
|---|---|---|
| `x.param`, `y.param` | Param (per axis) | `power_pct`, `speed_mm_s`, `interval_mm`, `passes`, and on the galvo `frequency_khz` and `q_pulse_ns`. **Frequency × Q-pulse is the MOPA colour grid**; Q-pulse steps come from the source table |
| `x.count`, `y.count` | Count | Default 10 × 10 |
| `x.min`, `x.max`, `y.min`, `y.max` | Min, Max | |
| `cell_w_mm`, `cell_h_mm` | Width, Height | |
| `center_mm` | X/Y Center | Galvo: absolute, about the field centre |
| `base` | Edit Material Setting | The fixed settings for every cell |
| `labels.{enabled, settings}` | Enable Text, Edit Text Setting | Row and column labels |
| `border.{enabled, settings}` | Enable Border, Edit Border Setting | |
| `preset.{save, load, import, export}` | Presets | LightBurn ships 4 diode/CO2 presets; OpenLaser should ship MOPA presets |
| — | Order | Cells run from least to most aggressive (fastest, weakest, widest interval, fewest passes first) |

### 8.3 Adjust Image
*Source: `AdjustImage`.* **P1.** `image.adjust {id?, gamma, contrast, brightness, enhance_radius, enhance_amount, denoise}` plus the layer's image settings (§6.5); `image.invert_display`; `image.preset.{save, delete, import, export, apply}` (P2; built-ins "Basic" and "Black Paint on White").

### 8.4 Arrays and offsets
*Sources: `GridArray`, `CircularArray`, `CopyAlongPath`, `OffsetShapes`.*

| Id | Parameters |
|---|---|
| `arrange.grid_array` (P1) | `{cols, rows, spacing_x_mm, spacing_y_mm, spacing: center_to_center/edge_padding, total_w_mm?, total_h_mm?, col_shift_mm?, row_shift_mm?, shift_by_half?, reverse_x?, reverse_y?, mirror_alt_cols_x?, mirror_alt_cols_y?, mirror_alt_rows_x?, mirror_alt_rows_y?, random_orientation?, seed?, vartext_increment?, virtual?, group_results?, select_results?}` |
| `arrange.circular_array` (P1) | `{copies, center: point/last_selected, center_mm?, start_deg, end_deg, step_deg?, rotate_copies?, vartext_increment?, group_results?, select_results?}` |
| `arrange.copy_along_path` (P2) | `{path_id, count? or spacing_mm?, rotate?, scale?, vartext_increment?, select_results?}` |
| `tools.offset_shapes` (P1) | §3.3 |

### 8.5 Production modes (P2)
*Sources: `RotaryMode/*`, `SplitMarking`, `RepeatMarking`.*

- **Rotary setup** (`laser_tools.rotary_setup`): `{type: chuck/roller, enabled, steps_per_rotation, roller_diameter_mm, object_diameter_mm, circumference_mm, axis, mirror_output?}`. The galvo variant adds `{split_size_mm, overlap_mm, min_speed, max_speed, accel_ms, return_speed, reverse?, return_to_start?}`. The **Rotary Marking window** (galvo) adds run-whole-shapes, max shape size, output centre, shape-order or per-slice running, position/jog/set-zero, frame previous/next slice and a sanity check. Motion is `hw`; marking is `arm`.
- **Split Marking** (`laser_tools.split_marking`): axis setup `{motor, axis_type: linear/rotary, orientation, reverse?, steps_per, unit_count, min_speed, max_speed, accel_ms, return_to_start?, return_speed, home_on_start?, home_speed, home_timeout_s}` and the same window controls as Rotary Marking.
- **Repeat Marking**: axis setup as Split Marking, then `{count, increment, pause_after_move_ms}` with a Completed counter.

### 8.6 Smaller dialogs
Close Selected Paths With Tolerance, Optimize Selected Shapes, Convert to Bitmap, Boolean Assistant, Trace Image, Bar Code, Nesting, Cylinder Correction, Taper Warp, Interval Test, Focus Test, Center Finder, Import 3D Model (orientation presets, size and rotation, 8/16-bit depth, resolution, depth cutoff, outline) and the Hotkey editor: their parameters are in the rows of §3.

### 8.7 Settings / Preferences
*Source: `SettingsPreferences`.* `prefs.set {key: value}` / `prefs.get` (`ok`/`ro`).

| Tier | Group | Keys (LightBurn setting) |
|---|---|---|
| P1 | Editor | `autosave_minutes` (Auto-save Interval; OM autosaves the workspace), `load_default_layers_on_new`, `select_after_drawing` (Switch to Select mode after drawing), `use_system_clipboard`, `save_job_log` (OpenLaser: always on, PLAN §3.6), `start_button_no_focus` (Don't Let Start Button Hold Keyboard Focus; on by default) |
| P2 | Editor | `auto_check_updates`, `auto_launch_in_running_copy`, `default_cut_selected`, `default_use_selection_origin`, `ignore_start_when_display_asleep`, `invert_wheel_zoom`, `load_save_dialog_style`, `error_sound_on_send_fail`, `save_cut_selected_to_project`, `save_job_origin_to_project`, `text.*` |
| P3 | Editor | `beginner_mode` (hides advanced tools), `save_transform_switch_state` |
| — | Editor | **Ignore out-of-bounds shapes if possible: not offered.** Out-of-area geometry is refused (PLAN §3.6) |
| P1 | Units and Grids | `design_units` (mm/in), `visual_grid_spacing_mm`, `shape_move_increments_mm` (the nudge steps), `snapping.{objects, grid}`, `object_snap_px`, `grid_snap_mm` |
| P2 | Units and Grids | `click_tolerance_px`, `grid_contrast` |
| P1 | Display | `theme` (system, dark, light), `dark_background`, `curve_quality`, `show_center_cross` |
| P2 | Display | `cursor_style`, `antialiasing`, `high_dpi`, `filled_rendering`, `font_size`, `reduce_motion`, `show_rotary_enable`, `show_cylinder_enable`, `full_screen_cursor`, `palette_labels` |
| P3 | Display | `toolbar_icon_size` |
| P0 | Import / Export | `import.group_shapes`, `import.auto_close_tolerance_mm`, `import.svg_fallback_dpi` (96/72; used only when a file lacks physical units), `import.dxf_units` (auto or fixed), `output.curve_tolerance_mm` (0.05 mm flattening; feeds the planner) |
| P1 | Import / Export | `import.ignore_dxf_text`, `export.dxf.*` |
| P2 | Import / Export | `import.ai_hidden_layers`, `import.hpgl_precision`, `print.*` |
| P1 | Preview | `preview.*` (as §5.5) |
| P2 | Camera | `camera.capture_system`, `camera.defaults.*` |
| P2 | Language | `language` |

---

## 9. OpenLaser-only commands (no LightBurn equivalent)

These come from PLAN §3 and the prototype. They are the commands that make agent and CLI work safe: an agent can do everything up to the burn and see exactly what would be emitted.

| Tier | Id | What it does | Params | Applies | Agent |
|---|---|---|---|---|---|
| P0 | `job.plan` | Builds the `JobPlan` from document and machine: sections per layer-pass, frame paths, estimate, fingerprint | `{selected_only?}` | both | ro |
| P0 | `job.validate` | Runs `ol_safety::validate` and returns every refusal and every numbered notice (for example, open shapes on a Fill layer) | `{}` | both | ro |
| P0 | `job.summary` | Returns the `EmissionSummary` decoded from the encoded bytes: per layer-pass power, kHz, ns, encoded mm/s, passes, bounds, time, repeats, post-crash emission bound. The same data the ARM dialog shows | `{}` | both | ro |
| P0 | `job.dump` | Writes the encoded job (LMC packets or G-code) and its disassembly to a file, for review and golden tests | `{path}` | both | ok |
| P0 | `device.detect` | Read-only USB probe for 0x9588:0x9899; refuses 0x9980 with an explanation. Sends no laser command | `{}` | galvo | hw |
| P0 | `device.status` | Reads the board status word (GetVersion) | `{}` | both | hw |
| P0 | `device.import_ezcad` | Creates or updates a galvo profile from EZCAD's `markcfg7`/`markcfg0` (plug folder); COR file at P1 | `{path, device?}` | galvo | ok |
| P0 | `device.inspect_profile` | Lists which markcfg keys are applied and which are ignored (OM `inspect-profile`) | `{device?}` | galvo | ro |
| P0 | `laser.arm` | The ARM step: shows the checklist and the decoded summary, takes the typed `ARM` (GUI) or `ARM <nonce>` (TTY) and mints a single-use token | — | both | arm |
| P0 | `bench.start` / `bench.status` | Opens or reports a hardware bench session, created only at a TTY and valid for 2 h. Device I/O is refused without one | `{}` | both | arm-class (human only) |
| P1 | `dev.golden_check` | Compares the encoders with the Python oracle's fixtures | `{suite?}` | both | ok |
| — | `hw.<item>` | Hardware session items H0–H29 live in the `ol-hw-raw` bench binary, not in the app's registry (PLAN §3.1) | — | galvo | — |

---

## 10. Shortcut decisions (†)

| Shortcut | Problem | OpenLaser decision |
|---|---|---|
| `Esc` | LightBurn: Select tool and clear selection. PLAN §3.5: Esc is STOP, consumed before any widget | Esc is STOP whenever a frame or job is active, and Select/clear otherwise. A frame is always active before a burn, so Esc always stops a running job |
| `F1` | LightBurn: Quick Help, and on galvos it opens Live Framing (and cycles styles while framing). Mac keyboards need `Fn` | Galvo profile: F1 = frame. Help moves to `Cmd+?` (the macOS help shortcut) |
| `Cmd+H` (Flip H in the Flip page) | macOS hides the app | Use `Cmd+Shift+H` (the Arrange-menu value) and `H` on the canvas |
| `Cmd+V` (Flip V in the Flip page) | It is Paste | Use `Cmd+Shift+V` and `V` |
| `Cmd+M` (Measure in the Tools-menu page) | macOS minimises the window; the Creation-toolbar page says `Alt+M` | `Opt+M` |
| `Cmd+L` (Position Laser in the Tools-menu page) | Same key as Draw Lines; the Creation-toolbar page says `Alt+L` | `Opt+L` |
| `Cmd+Tab` (Add Tabs) | macOS app switcher; it never reaches the app | `Ctrl+Tab` on macOS |
| `` Cmd+` `` (Edit Nodes) | macOS cycles the app's windows | `` Ctrl+` `` on macOS |
| `Cmd+W` (Weld) | macOS uses `Cmd+W` to close a window; LightBurn's docs give it to Weld | Provisionally Weld, for LightBurn muscle memory; the window closes from its title bar. Open question 7 |
| `Cmd+Shift+C` | The Edit-menu page gives it to both Convert to Path and Convert to Bitmap | Convert to Path keeps it; Convert to Bitmap has none |
| `PgUp`/`PgDn` (draw order) | The Draw Order page gives forward = PgDn and backward = PgUp, with the same description for both | Follow the page's key assignment; verify on a LightBurn install |
| `Pause`, `Ctrl+Break` | LightBurn's keyboard pause and stop; Mac keyboards have neither | Not used. Esc stops; Pause is a button |
| `Opt+Fn+arrows` (align centres) | macOS-only mapping; Windows uses `Alt+PgUp`/`PgDn` | Keep LightBurn's per-platform keys |

---

## 11. Agent and CLI surface

The Craft-family pattern, applied to OpenLaser. **Binary name is open question 2**; the examples use `ol`, the name PLAN §3.1 gives `ol-cli`.

```sh
ol mcp --automation-read-root ~/Craft --automation-write-root ~/Craft           # headless stdio MCP server
ol mcp --bridge 127.0.0.1:<port> --control-token-file <path>                     # drives a running GUI
ol cmd list [--filter layer.]                                                    # = command_list
ol cmd run layer.cut_settings.set --params '{"layer":1,"galvo":{"frequency_khz":30,"q_pulse_ns":200}}'
```

| MCP tool | Maps to | Notes |
|---|---|---|
| `command_list` | the registry | Returns every id with `agent`, `enabled`, `disabledReason`. `arm` commands are absent; `hw` commands are listed as disabled with the reason |
| `command_run`, `command_batch` | any `ok`/`ro` id | Batches stop at the first error and do not roll back (as PhotoCraft). Each step is one history entry |
| `doc_new`, `doc_open`, `doc_save`, `doc_export`, `doc_close` | `file.new`, `file.open`, `file.save(_as)`, `file.export` | Paths confined to the automation roots |
| `doc_import` | `file.import` | |
| `doc_inspect` | — | Objects (ids, layer, bounds, open/closed), layers with full settings, selection, history, active device, refusals and notices |
| `render_preview` | — | Canvas or job-simulation PNG |
| `job_plan`, `job_validate`, `job_summary` | `job.*` (§9) | What would be burned, without burning |
| `device_list`, `device_settings_get` | `device.list`, `device.settings.get` | Profile data only |
| `laser_stop` | `laser.stop` | Always accepted |
| `ui_inspect`, `ui_screenshot`, `ui_click`, `ui_key`, `ui_type`, `ui_menu_invoke` | bridge mode | Synthetic input is refused on ARM, Start, confirmation and device-I/O widgets. While a frame or job runs, the bridge accepts only `laser_stop` and read-only calls (§1.5) |

**Never exposed to agents:** `laser.start`, `laser.arm`, `laser.run_machine_file`, `laser.send`, `move.fire`, `preview.start_here`, the Start of any test generator, and every `hw` command, including red-light framing (PLAN §3.6: agents never run hardware commands). Allowing agent framing on red light is open question 3.

---

## 12. Coverage counts

Counted from the table rows above on 2026-10-23. A family row such as `arrange.move_selection.{…}` counts once, and the prose-only subsections (§3.6, §3.8, §5.7–5.10, §5.12–5.14, §5.16–5.17) are not counted. P3 includes the rows marked "not planned".

| Section | P0 | P1 | P2 | P3 | Rows |
|---|---:|---:|---:|---:|---:|
| §3 Menus (incl. Edit Nodes and transform rows) | 36 | 87 | 65 | 22 | 210 |
| §5 Dock windows (panels plus their controls) | 37 | 70 | 16 | 3 | 126 |
| §6 Cut-setting fields | 21 | 30 | 11 | 3 | 65 |
| §7 Device-setting fields | 30 | 14 | 10 | 4 | 58 |
| §8 Dialog and preference groups | 1 | 12 | 9 | 2 | 24 |
| §9 OpenLaser-only commands | 10 | 1 | 0 | 0 | 11 |
| **Total** | **135** | **214** | **111** | **34** | **494** |

P0 is about a quarter of the catalogue. Nearly half of P0 (62 of 135 rows) is settings fields and the OpenLaser job and safety commands rather than menu items. When the registry exists, an `xtask parity` (as in PhotoCraft) should regenerate this table from the live registry plus this catalogue.

---

## 13. Deliberate divergences from LightBurn

1. **Refuse; never ignore or clamp.** LightBurn warns about out-of-bounds shapes and offers to ignore them. OpenLaser refuses (PLAN §3.6). The same goes for speeds above the machine maximum.
2. **Framing before a galvo burn is mandatory**, and so is the job checklist. Neither is a preference.
3. **The confirmation shows decoded bytes,** not the editor's values (`job.summary`).
4. **No unbounded runs.** Run Continuously becomes a capped Repeat.
5. **No laser-on framing on the galvo**, and Fire is diode-only by default.
6. **No automatic USB reset or reconnect mid-job.**
7. **Agents can do everything except emit or touch hardware.** LightBurn's REST API also refuses to start jobs; OpenLaser enforces it in the type system and the registry rather than by API omission.
8. **No telemetry, licence keys, update nags or embedded web tools.**
9. **Galvo and gantry layer blocks are separate,** so one machine's defaults never leak into another's output.

---

## 14. Open questions for Luiz

1. **Id style.** This catalogue uses `snake_case` segments (`layer.cut_settings.set`), as the task specified. PhotoCraft uses lowerCamel (`file.saveACopy`). Exact family parity means switching now, before any code exists.
2. **CLI binary name.** PLAN §3.1 calls it `ol`. The Craft family uses `<app>-cli` (`photocraft-cli mcp …`). The prototype already installs `openlaser`. Options: `ol`, `openlaser-cli`, or both.
3. **Red-light framing by agents.** PLAN §3.6 bars agents from all hardware commands, red light included. A supervised exception (bench session open, Luiz present) would let an agent iterate on placement. Default here: no.
4. **Images in P0.** The task puts image import in P0 and image modes in P1. This catalogue makes Threshold and Jarvis P0 so an imported image can actually be burned, and leaves the other modes in P1. The alternative is vector-only P0.
5. **Material Test tier.** P1 here, as the task says. MOPA colour work depends on a frequency × Q-pulse grid, and 02 called it T0-stretch; PLAN M2 already schedules it.
6. **G-code export.** P1 here because the MOPA does not need it. PLAN M2 ships it early for prototype parity (the CO2, diode and plotter profiles).
7. **`Cmd+W`.** Weld (LightBurn) or Close Window (macOS convention).
8. **"Mirror/rotate" in P0.** Read here as axis negate, axis swap and Field Angle. 02 rated Field Angle T1.
9. **Bridge mode.** §1.5 forbids synthetic input on ARM and Start even with Luiz watching. It is the only way to keep PLAN's "no script path can ARM" once a bridge exists inside the GUI process.

## 15. Items to verify

1. **Shortcut conflicts inside LightBurn's own docs** (§10): Flip, Measure, Position Laser, Convert to Bitmap, draw-order Page keys. LightBurn 1.5.04 is installed at `/Applications/LightBurn.app` (06 G10); its built-in hotkey list (Help › Quick Help) settles them without running a laser.
2. **Red Dot Scale unit** (percent or ratio).
3. **Per-layer galvo XML keys in `.lbrn2`** (timings, wobble, sub-layers); 02 §21 q2.
4. **Grayscale on fiber galvos** (02 §21 q4).
5. **JPT M7 frequency-versus-pulse cutoffs** for Luiz's exact model and wattage (06 G7).
