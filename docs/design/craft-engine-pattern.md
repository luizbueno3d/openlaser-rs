# The Craft engine pattern, and OpenLaser's engine

*2026-10-23. Design research for OpenLaser (LightBurn-equivalent, Rust, Craft-family style).*

**Status: proposal.** Part 1 describes how the Craft family builds its engine, read from source. Part 2 applies that pattern to OpenLaser and changes it where a laser needs something different. The open questions are listed in Part 3.

**Sources.** Everything was read without building or running anything:
- PhotoCraft `~/AI/PDF_Craft_Edit/photocraft` (`crates/engine`, `doc`, `ops`, `format`, `testkit`, `ui-egui`, `automation`, `apps/photocraft-cli`).
- PdfCraft `~/AI/PDF_Craft_Edit/pdfcraft` (`crates/engine/src/commands.rs`, `crates/automation/src/conventions.rs`).
- craft-libs checkouts `3d37da3` and `5c874d5` (`atomic`, `settings`, `quit`, `ui`, `print`).
- The live VectorCraft and FilmCraft MCP servers, through read-only `command_list` calls.
- The OpenLaser plan `~/AI/ClaudeCode/openlaser-rust-plan/PLAN.md` §3 and research 02, 04 and 06.
- The OM prototype `~/AI/OpenLaser/OpenMopa/mopa_luiz/layers.py`.

Citations use `file:line`. Code excerpts are trimmed and kept to 15 lines or fewer. An ellipsis `…` marks omitted lines.

---

## Part 1. The pattern, as the Craft apps build it

### 1.1 The shape of a Craft app

PhotoCraft's `AGENTS.md` §2 states the layering, and `cargo xtask layers` enforces it:

```text
L0 geom/units/foundation      L1 doc (pure data)        L2 ops (history), algorithms
L3 format (native file)       L4 io (foreign formats)   L5 engine: Session + command registry
L6 ui-egui (thin shell), automation (MCP + JSON-lines)  apps: GUI (--control), CLI (run/batch/commands/mcp/serve)
```

The seam rule (`docs/architecture.md` §4) is the heart of the pattern: **a frontend needs only `Session`, the registry, view-models and a viewport.** The CLI and the MCP server must be able to do everything the GUI does except pointer gestures.

Golden rule 1 in `AGENTS.md` says it outright: "Everything is a command." New user-visible behaviour is an engine command with an id, label, menu path, shortcut, params doc, `enabled` and `run`, plus tests. Menus, shortcuts, the palette, the control channel, the CLI and MCP all dispatch by id. Only pure view and window state stays in the shell.

### 1.2 The command spec and the registry

PhotoCraft (`crates/engine/src/commands.rs:14-28`):

```rust
type Run = fn(&mut Session, &Value) -> Result<Value>;
type Enabled = fn(&Session) -> std::result::Result<(), String>;

pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub menu: &'static [&'static str],      // ["Layer","New"]; empty = not in menus
    pub shortcut: Option<&'static str>,     // "Cmd+Shift+N", mapped per platform by the UI
    pub params: &'static str,               // human/agent-readable params doc (JSON-ish)
    pub enabled: Enabled,
    pub run: Run,
    pub journal: bool,                      // false for queries
}
```

How the registry works:
- **Plain data plus function pointers.** There are no trait objects. A `cmd!` macro (`:129-133`) fills the struct with `journal: true`.
- **One `specs()` per module.** Each area module (`align_cmds.rs`, `layer_nav_cmds.rs`, …) exposes `pub fn specs() -> Vec<CommandSpec>`.
- **One assembly point.** `commands::build()` lists them with `v.extend(crate::xxx_cmds::specs())` (`:1263-1360`). It is the only shared edit point when parallel agents add commands (`AGENTS.md` §6).
- **Lookup.** The registry is built once into a `OnceLock<Registry { specs, by_id: HashMap }>` (`:152-177`). `find(id)` is exact: `FILE.NEW` and `file.new\0` miss (`tests/command_lookup.rs`). A duplicated id silently keeps the first match.
- **Scale.** The engine holds about 82k lines in its top-level modules. A grep finds 500+ literal ids, plus families generated from tables.

**Families generated from one table.** Each of the 16 adjustments becomes two commands, `layer.newAdjustmentLayer.<kind>` and `image.adjustments.<kind>`, sharing one `run` (`:1145-1259`). The kind is recovered at dispatch by `inject_kind`, which adds a hidden `__kind` param from the id suffix (`:1379-1389`). Generated ids are `Box::leak`ed `&'static str`.

**PdfCraft differs.** Its `CommandSpec` is **metadata only** (`pdfcraft/crates/engine/src/commands.rs:118-130`): `id, label, menu: Option<&str>, shortcut: Option<Shortcut>, needs: Needs, in_text, icon`. "A frontend implements *what* each id does." Enablement is declarative (`enum Needs { Nothing, Document, Assembly, Undo, Redo, … }`, `:15-55`). For automation, each command maps to a headless tool that carries a real JSON Schema (`automation/src/conventions.rs:11-45`).

### 1.3 Command ids

The rules, from `docs/contributing.md` "Adding a command" step 1 and `ui-egui/src/menu_catalog.rs`:
- **The id is the menu path.** Each menu segment is lowercased and the item label is camelCased: `file.export.quickExportAsPng`, `layer.newAdjustmentLayer.levels`, `filter.blur.gaussianBlur`.
- **The catalog decides ids.** `menu_catalog.rs` holds the reference app's full menu tree, about 770 rows of `(path, label, default shortcut, id)`, generated from the recorded menus. Using an id from it makes the menu item live with no UI work. Rows without an implementation render disabled, and `cargo xtask parity` turns that into `docs/parity-checklist.md`. **This is how "PhotoCraft is Photoshop" is measured.**
- **Commands outside the menus** use a descriptive id in the same style (`layer.smartFilter.delete`) and an empty menu path.
- **Shell-only ids** (window, panels, themes, tool picks `tools.select.<tool>`) live in `ui-egui/src/menus.rs` `UI_COMMANDS`. The engine never sees them.

The family is not uniform:

| App | Id style | List keys | Notes |
|---|---|---|---|
| PhotoCraft | `file.saveAs` (camelCase) | `disabledReason`, `needsDocument` | one id per menu item |
| FilmCraft (live MCP) | `file.saveAs`, `file.autoSaveNow` | `defaultShortcut`, `shortcuts[]` | user-remapped shortcuts listed |
| VectorCraft (live MCP) | `object.align` (camelCase) | `disabled_reason` | **one parameterized command** `object.align {horizontal, vertical, to}` instead of one id per menu item |
| PdfCraft | `file.save_as` (snake_case) | n/a | metadata-only registry |

Three of the four use camelCase segments.

### 1.4 Parameters

**PhotoCraft.** `params` is a free-form doc string in a de-facto grammar, `{"key":type|range=default,…}`. For example (`commands.rs`, `file.new`):

```text
{"width":1..300000=1920,"height":1..300000=1080,"mode":"rgb|gray|cmyk|lab"="rgb","depth":8|16|32=8,…}
```

How PhotoCraft reads params:
- **Hand-rolled parsing.** Each `run` parses `&Value` with small helpers (`int`, `int_i32`, `u32_id`, `color_param`, `layer_param`; `commands.rs:180-285`). The helpers turn hostile numbers into `BadParams` instead of wrapping (`int_i32` refuses `2^32+50` instead of moving 50 px).
- **Unknown keys are mostly ignored.** `param_check.rs` (#4336) rejects undocumented keys only for the commands listed in `STRICT`, and today that is two. Its own comment explains why: a misspelled key "ran the command on its defaults and reported success."
- **Universal keys.** Every command accepts `"coalesce"` (history coalescing) and `"target"` (the Channels-panel target).
- **Targeting.** Layers are named by id or by name (`explicit_layer`, `:260-279`). Without one, the command uses the active layer.

**PdfCraft** checks params against each tool's `input_schema`. Unknown params are dropped *with warnings* (`conventions.rs:20-30`).

**Lesson for OpenLaser:** the string docs are readable for agents but not checkable. That is the pattern's weakest point, and OpenLaser fixes it (§2.4).

### 1.5 Enablement and what agents see

`enabled: fn(&Session) -> Result<(), String>` returns a *reason*, not a bool. Examples are `"no document open"`, `"nothing to undo"`, and `"active layer is a type layer, not a pixel layer"`.

`list_entry` (`commands.rs:143-150`) turns the reason into the agent contract:

```rust
json!({
    "id": c.id, "label": c.label, "menu": c.menu, "shortcut": c.shortcut, "params": c.params,
    "enabled": reason.is_none(), "disabledReason": reason, "needsDocument": needs_document,
})
```

`needsDocument` tells an agent "open or create a document first" apart from a precondition that a document alone won't meet.

The UI adds one more layer, `enable_rules.rs`. It covers reference-app greying that the engine doesn't express, such as Background-layer restrictions. It is applied on top of `is_enabled`, so menus, shortcuts and context menus grey the same items.

### 1.6 Dispatch

There is one pipeline for every transport: `Session::execute` → `jobs.rs:620-703`. In order:

1. `find(id)`, else `UnknownCommand`.
2. Params must be an object or `null`.
3. **Authorization.** `self.authorize` is an optional `fn(&str, &Value) -> Result<()>` gate that untrusted sessions (MCP, the control channel) install. It also applies to the commands a command runs on its own behalf.
4. `param_check`, plus per-area validation.
5. Hidden params are injected (`__kind`, the Channels target).
6. The `enabled` precondition.
7. **Job conflict.** A background job on this document disables any command that would edit it.
8. The run, under a **last-resort guard**:

```rust
let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (spec.run)(self, &run_params)))
    .unwrap_or_else(|_| Err(EngineError::Other(format!(
        "`{id}` failed with an internal error (logged); the document is unchanged"))));
```

9. `after_command`: post-command hooks, then the journal push (if `journal`), with nested entries collapsed for replayable commands.

`EngineError` (`engine/src/lib.rs:141-158`) has these variants: `UnknownCommand`, `Disabled(id, why)`, `BadParams{cmd,msg}`, `NoDocument`, `NoLayer`, `Other`, `Cancelled`.

### 1.7 Edits, history and undo

**Every document change goes through one closure** (`engine/src/lib.rs:692-756`, condensed):

```rust
pub fn edit<R>(&mut self, label: &str, f: impl FnOnce(&mut Document, &mut Option<LayerId>) -> Result<R>) -> Result<R> {
    let st = self.active_mut().ok_or(EngineError::NoDocument)?;
    let before = st.doc.clone();                 // Arc<Document>: O(1)
    let prior = st.layer_target();
    let mut doc = (*before).clone();             // O(layers + tiles) pointer copies, no pixels
    let mut active = st.active_layer;
    let r = f(&mut doc, &mut active)?;           // Err => nothing committed
    st.doc = Arc::new(doc); st.active_layer = active;
    …
    if key.is_none() || st.coalesce != key || !st.history.can_undo() {
        st.history.set_current_layers(prior);
        st.history.record(label, before, layers);
    } else { st.history.set_current_layers(layers); }   // same coalesce key: same step
    st.coalesce = key; st.revision += 1; st.last_damage = damage;
```

**History stores whole-document snapshots** (`crates/ops/src/lib.rs:1-9, 39-53`). This is "simple, obviously correct". It is cheap because pixel tiles are `Arc`-shared copy-on-write:

```rust
pub struct HistoryState { pub label: String, pub doc: Arc<Document>, pub layers: LayerTarget }
pub struct History {
    undo: VecDeque<HistoryState>,   // last = the state before the current document
    redo: Vec<HistoryState>,
    pub max_states: usize,          // default 50; Preferences can change it
    pub max_bytes: usize,           // optional pixel budget; trim() drops the oldest
    current_label: String,
    current_layers: LayerTarget,
}
```

Details that matter:
- **Undo swaps `Arc`s** (`:121-145`). Undo and redo also restore the layers each state targeted. Selecting a layer is not a step, so it is stored with the states (#1356).
- **Coalescing.** A `"coalesce": "<key>"` param folds consecutive edits with the same key into one step: a slider drag, a typing session.
- **Labels are English strings.** PdfCraft does better for translation: `Label { wording: &'static str, value: Option<String> }`, where the wording is a catalog key with one `{name}` hole (`pdfcraft/crates/engine/src/lib.rs:1564-1580`).
- **Revisions.** `revision` and `saved_revision` drive the dirty flag. `view_revision` lets pixel caches ignore view-only changes.
- **The design doc and the code disagree.** `architecture.md` §6 plans ops and transactions; the code ships snapshots. The snapshot approach won in practice.

### 1.8 Journal, actions, batching, jobs

- **Journal.** Every `journal: true` command appends `(id, params)` to `Session.journal`. Action recording (Photoshop Actions), droplets and `photocraft-cli batch` replay these pairs. A command marked replayable stands for the steps it ran itself, which are truncated from the journal (#3067).
- **`command_batch` (MCP).** `automation/src/server.rs:851-895` runs the steps in order, **one history step per command**. It is **not atomic**: "earlier successful steps stay applied (no rollback)". There is a maximum of 256 steps (PdfCraft: 1000, with every envelope validated before any edit). `stop_on_error` defaults to true, and a reply-size budget applies.
- **Background jobs** (`jobs.rs` header). A long command does its work through `jobs::run`, against an `Arc` snapshot on a worker thread, with progress and a cancel flag. The result is applied on the UI thread as one history step through the normal dispatch path. `Session::execute` runs it inline, so tests and the CLI see no difference, while `Session::start` backgrounds it. While a job runs on a document, edits to that document are disabled with a message naming the job.

### 1.9 How the UI dispatches

`ui-egui` is thin. Its data flow:

- **Menus** (`menus.rs:970-1010`). The tree comes from (1) the reference catalog, live where the id is known, (2) `UI_COMMANDS`, and (3) engine specs with a menu path. Labels are localized by `tr_id(lang, id, label)`, so translations key on the command id.
- **One entry point for every source** (`menus.rs:148-165`): `menus::invoke(app, ctx, id, params)`. It runs:
  1. the automation gate if the input is synthetic;
  2. an unsaved-changes intercept;
  3. `invoke_unguarded`, a chain of `if let Some(r) = <area>_ui::menu(app, id, &params) { return r }` intercepts that open dialogs (for example `"file.new" if params are empty` opens the New dialog);
  4. `app.run(id, params)`, which falls through to `jobs_ui::run` and then the engine.
- **Modal gate.** `modal_allows` blocks commands while a dialog is open, except four view-navigation commands.
- **Shortcuts** (`shortcuts.rs`, `shortcut_dispatch.rs`):
  - `parse("Cmd+Shift+N")`, where `Cmd` means ⌘ on macOS and Ctrl elsewhere.
  - The binding table merges the catalog default, `CommandSpec::shortcut` and `UI_COMMANDS`, with Edit › Keyboard Shortcuts overrides on top, stored as prefs `shortcuts.<command id>`.
  - A shortcut for a disabled command puts its reason on the status bar.
- **Tools.** `tools.select.<tool>` ids set `app.ui.tool`. Tool gestures end in ordinary commands (`shape.create`, `paint.stroke`) with full params, so agents can replay them.
- **Synthetic input.** Events queued by `ui.click`, `ui.key` and friends are injected into the next frame, and `automation_input` is set "only while a frame is processing synthetic automation input" (`ui-egui/src/lib.rs:655-660, 2206`). Every command triggered under it goes through the automation authorization (`lib.rs:973-1011`), and tests prove it (`control.rs` `synthetic_shortcuts_use_the_automation_command_policy`). So a `ui_click` on a menu cannot do what `command_run` may not.
- **UI state** lives in `state.rs` (serde). `ui_inspect` and `ui_set` read and drive it.

### 1.10 Localization

From the `ui-egui/src/i18n/mod.rs` header:
- **Keys.** English strings in code are the lookup keys. Each language has one TSV catalog (`context ⇥ source ⇥ translation`). `@id` rows key a menu label on the command id.
- **Never translated.** Command ids, menu paths used for logic, the control channel, the CLI and MCP. Agents and scripts are unaffected by language.
- **API.** `tr`, `tr_ctx`, `tr_id`, `trn` (plurals) and `fmt` (named placeholders).
- **Adding a language** means adding one TSV file and one row in `LANGUAGES`. Right-to-left languages mirror the chrome.

### 1.11 The document model (`crates/doc`)

- **Pure data.** No rendering, no UI, `#![forbid(unsafe_code)]`, and the clippy deny-list for panics.
- **Ids.** `LayerId(u64)` and `DocId(u64)` come from a global `AtomicU64` (`doc/src/lib.rs:52-90`). On load, `reserve_ids_through(max)` moves the counter past the file's ids. That is fine for one process, but ids are not deterministic across runs.
- **Copy-on-write.** Big payloads are `Arc`'d, so `Clone` is cheap.
- **Ordering convention, documented once.** `children[0]` is the bottom layer, and UIs display the list reversed.
- **Non-destructive kinds** are in the model from day one, even where rendering lands later.
- **Foreign-format blocks** are kept verbatim for lossless round trips (`psd_blocks`).

### 1.12 The native format and autosave (`crates/format`)

**`.pcraft` layout** (`format/src/lib.rs:1-21`): `manifest.json` (a versioned tree), plus `tiles/<blake3>.zst` and `blobs/<blake3>.zst` (content-addressed), plus optional `thumb.png` and `composite/preview.png`. The container is a ZIP (STORE) or a directory with the same layout. Saves are incremental.

How the format stays honest:
- **A separate serde model.** The manifest structs are deliberately not the doc structs: "the on-disk schema is versioned and migrated independently of in-memory refactors" (`manifest.rs:1-8`). Binary data never goes in JSON.
- **Migrations** (`migrate.rs`). `STEPS: &[fn(&mut Value) -> Result<()>]`, where `STEPS[i]` upgrades version i+1 to i+2. Version 0 is corrupt. A newer file than the build fails with `TooNew { found, supported }`.
- **Nothing silently dropped.** `convert.rs` builds `Document { … }` and `DocM { … }` with **exhaustive struct literals and destructuring**, so adding a doc field fails to compile until it is saved (`AGENTS.md` §8). New fields get `#[serde(default)]`.
- **Hostile input.** Errors include `Corrupt`, `LimitExceeded` (nesting depth) and `NonFinite { path }` (refuses to save NaN or infinity). The reader has fuzz targets.
- **Atomic writes** (`atomic.rs`, now also `craft-atomic`). Write a temp file in the same directory, `sync_all` (F_FULLFSYNC on macOS, with a fallback for SMB), rename, then fsync the directory. Failures are injectable through the `AtomicFs` trait.
- **Autosave** (`autosave.rs`). An `Autosaver` per document runs on a background thread and coalesces requests to the newest `Arc<Document>`. It writes a directory bundle plus a `RecoveryInfo` sidecar (key, name, original path, time, revision). `RecoveryStore` deletes an entry **only when the document is saved or closed, never just because it was recovered**. FilmCraft exposes the same lifecycle as commands: `file.recover`, `file.discardRecovery`, `file.autoSaveNow`, `file.listAutoSaves`.
- **Settings.** `craft-settings` (TOML) keeps app settings in platform folders, with atomic saves, a backup, migrations, and hand edits preserved.

### 1.13 Automation and CLI (as far as the engine is concerned)

- **One headless core.** `Headless` (in-process `Session` plus file I/O) backs both `photocraft-cli` and the MCP server. The MCP server runs on rmcp 3.5.0 with schemars 1.2.2 (PhotoCraft's `Cargo.lock`).
- **Tools mirror the registry.** `command_list`, `command_run` and `command_batch` are generated from it. `doc_*`, `render_preview` and `jobs_*` sit beside them, and bridge mode adds `ui_*`.
- **Untrusted sessions install a gate.** `automation/src/workspace.rs:244-310`, `authorize_engine_command`, refuses every `file.*` command except a safe list, plus any param that names an ambient path. All file access goes through `AuthorizedWorkspace` read and write roots, which are cap-std directory capabilities.
- **CLI subcommands** are declared as data (`Subcommand { name, values, bare, run }`), and unknown flags are errors. The subcommands are `convert`, `info`, `run --cmd <id> --params <json> …`, `batch`, `commands`, `mcp` and `serve`.

### 1.14 Tests (`crates/testkit` and the conventions around it)

The testkit crate itself is thin: a GPU lock and perf helpers (`RssSampler`, percentile `Summary`). The real pattern is in how tests are written:
- **Behaviour tests drive the public API.** `Session::new()`, `execute("file.new", json!({…}))`, then assert on `active().doc`. Each fixed bug gets its own file in `crates/engine/tests/` (53 files), named for the behaviour (`undo_keeps_active_layer.rs`).
- **`panic_hunt.rs`.** Every command in `command_specs()` runs against four adversarial param sets (empty, negative or huge numbers, NaN-ish values, empty arrays and strings), each on its own thread with a 4 s deadline. A panic hook counts *contained* panics as failures too. The test is `#[ignore]`d and slow; it runs with `--ignored` before shipping new commands.
- **Registry tests.** For example, every `find(id)` returns the same spec as a linear scan.
- **Format tests.** Round-trip, migration and synthetic-generator tests, plus fuzzing.
- **Clippy** allows `unwrap`, `expect`, `panic` and indexing in tests only (`clippy.toml`).

### 1.15 What the pattern gets right, and what OpenLaser should not copy

**Keep:**
- one registry as the single source of truth for menus, shortcuts, palette, CLI and MCP;
- reasons instead of booleans;
- `Session::edit` closures with snapshot history;
- coalescing;
- the dispatch panic guard;
- the panic hunt;
- the separate manifest model with migrations and exhaustive conversion;
- atomic saves and a recovery store;
- a reference-app menu catalog as the parity meter;
- English-as-key localization with `tr_id`;
- an automation gate applied to synthetic UI input as well.

**Don't copy:**
1. **Free-form params strings.** Unknown keys run on defaults (#4336); there are no schemas for MCP, and parsing is hand-rolled in every `run`.
2. **Global atomic ids.** They are not deterministic, which hurts goldens and agent replays.
3. **Non-atomic `command_batch`.** An agent cannot make "create 20 shapes" a single undo step, or roll back.
4. **A 1,600-line `commands.rs`** with inline closures, and string `matches!(id, …)` special cases spread through the UI (`invoke_unguarded`, `names_existing_layer`). Behaviour should be declared on the spec instead.
5. **Silent duplicate ids**, where the first one wins.

---

## Part 2. OpenLaser's engine

### 2.0 What is different about a laser app

The Craft pattern assumes that the worst a command can do is damage a document, and undo fixes that. OpenLaser's commands can also **move mirrors and fire a Class 4 laser**. `AGENTS.md` §1 and PLAN §3.6 set the rules:
- **Agents never emit and never open the real USB device.**
- **Only a human can ARM.** The ARM token is minted only in `ol-arm`, whose reverse dependencies are exactly the GUI and the CLI.
- **Refusals, never warnings.**

So OpenLaser keeps "everything is a command", but each command declares **what it touches**, and dispatch enforces a policy per *origin* (§2.3). The emitting step can never complete inside the command system.

The second difference: OpenLaser is "PhotoCraft for LightBurn". The parity meter is a **LightBurn menu catalog** built from LightBurn's public documentation, clean-room: names, placement and behaviour only (§2.5).

### 2.1 Crate map in Craft layering

This matches `AGENTS.md` (`openlaser-cli`) and PLAN §3.1. The plan's `ol-*` names are shown for traceability.

| Layer | Crate | Plan name | Role |
|---|---|---|---|
| L0 | `openlaser-units`, `openlaser-types`, `openlaser-geom` | `ol-units`, `ol-types`, `ol-geom` | `Mm`, `MmPerS`, `Hz`, `Ns`, `Us`, `Percent` (ranges checked on deserialize); `ArmToken` (checking only); kurbo/i_overlay geometry |
| L1 | `openlaser-doc` | `ol-doc` | `Project`, shapes, layers and cut settings: **pure data** (§2.6) |
| L1 | `openlaser-machine` | `ol-machine` | Device profiles and calibration, no I/O |
| L2 | `openlaser-ops` (or a module of `-doc`) | — | `History` (§2.8) |
| L2 | `openlaser-plan`, `openlaser-safety` | same | `plan()`; `validate()`, the sole constructor of `EmitPlan`/`FramePlan` |
| L3 | `openlaser-format` | `ol-doc` (M2) | `.olp` project bundle, migrations, autosave (§2.9) |
| L3 | `openlaser-lmc`, `-lmc-emu`, `-driver` | same | Encoder, disassembler, FakeLmc, `Backend`, `DriverHandle` |
| L3 | `openlaser-lmc-usb` | `ol-lmc-usb` | The **only** `nusb` user. Linked **only** by `apps/openlaser` and the CLI's hardware subcommands |
| L4 | `openlaser-io` | `ol-io` | SVG, DXF, OM workspace; `.lbrn2` later (§2.10) |
| **L5** | **`openlaser-engine`** | `ol-app` | **`Session` + command registry: this document** |
| L6 | `openlaser-automation` | (new) | MCP + JSON-lines `serve`. **Depends on no USB crate and not on `ol-arm`** |
| L6 | `openlaser-ui` (egui) | `ol-ui` | Thin shell; uses `ol-arm` |
| apps | `openlaser` (GUI, `--control`), `openlaser-cli` | `ol-ui`, `ol-cli` | |

The engine depends on `-driver`, `-lmc` (encode and disassemble), `-plan` and `-safety`. It does **not** depend on `-lmc-usb` or `ol-arm`. The app injects a device provider, and headless sessions get none (§2.3).

### 2.2 Command spec, registry and typed params

```rust
// openlaser-engine/src/command.rs
pub struct CommandSpec {
    pub id: &'static str,                     // "arrange.alignLeft" (§2.5)
    pub label: &'static str,                  // English; also the i18n key via tr_id(id, label)
    pub menu: &'static [&'static str],        // LightBurn menu path; [] = not in menus
    pub shortcut: Option<&'static str>,       // "Cmd+G"; Cmd = ⌘ on macOS, Ctrl elsewhere
    pub icon: Option<&'static str>,
    pub kind: Kind,                           // what it touches (§2.3); drives policy, journal, history
    pub enabled: fn(&Session) -> Result<(), Disabled>,   // Disabled { reason, needs_document }
    pub doc: &'static str,                    // one-line params summary for command_list
    pub schema: fn() -> schemars::Schema,     // generated from the typed params struct
    pub run: fn(&mut Session, Value) -> Result<Value>,  // erased; built by command! below
    pub checked: Option<fn(&Session) -> bool>,// checkable menu items (Output, Show, Snap…)
}
```

**Typed params, erased at the registry boundary.** A command is written against a serde and `JsonSchema` struct. The `command!` macro generates the JSON shim. Non-capturing closures coerce to `fn` pointers, so the registry stays plain static data, as in PhotoCraft.

```rust
macro_rules! command {
    ($id:literal, $label:literal, menu: [$($m:literal),*], shortcut: $sc:expr, kind: $kind:expr,
     enabled: $en:expr, params: $P:ty, run: $f:path $(, checked: $ck:expr)?) => {
        CommandSpec {
            id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, icon: None, kind: $kind,
            enabled: $en, doc: <$P as ParamsDoc>::DOC, schema: || schemars::schema_for!($P),
            run: |s, v| {
                let p: $P = crate::params::decode($id, v)?;          // → EngineError::BadParams
                serde_json::to_value($f(s, p)?).map_err(EngineError::internal)
            },
            checked: command!(@ck $($ck)?),
        }
    };
}
```

```rust
#[derive(Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SetFrequency {
    /// Layer: 0..=29, "C05" or the layer's name. Tool layers have no cut settings.
    pub layer: LayerRef,
    /// Sub-layer index (0 = the layer itself). Default 0.
    #[serde(default)] pub sub: u8,
    /// Galvo pulse frequency. Exactly one of frequencyKhz or frequencyHz.
    pub frequency_khz: Option<KHz>, pub frequency_hz: Option<Hz>,
}
```

The rules:
- **Strict by construction.** `#[serde(deny_unknown_fields)]` everywhere. This fixes PhotoCraft #4336 for free. Avoid `#[serde(flatten)]` in params, because it disables `deny_unknown_fields`.
- **Ranges live in the types.** `Percent` only deserializes finite values in 0..=100; `MmPerS` only finite values > 0; `Hz` is `u32`. A bad value fails at decode with a JSON-pointer path (`/powerPct: 150 is outside 0..=100`). There is no hand-rolled `int_i32`.
- **Physical quantities carry their unit in the key**: `speedMmPerS`, `powerPct`, `frequencyKhz` or `frequencyHz`, `qPulseNs`, `intervalMm` (or `lpi`), `angleDeg`, `xMm`. A kHz/Hz mix-up is a 1000× error; the key makes it impossible to type by accident.
- **Universal keys** are stripped before decode. `coalesce` folds history (PhotoCraft). `dryRun` validates and returns the result that *would* happen without committing; it runs the edit closure and then discards it.
- **Edit-time vs job-time validation.** Params are checked for *physical sanity* only (schema ranges). Checks against the machine (pulse table, frequency range, work area) belong to `openlaser-safety` at plan and ARM time, as in PLAN §3.6. A project can be designed for another machine. The live notices are visible in `document.inspect` and `laser.preview`.
- **Return values** are typed too. Creators return `{ "ids": [..] }`, edits `{ "changed": n }`, and notices go in `{ "notices": [...] }`, so agents can chain without re-inspecting.

**Registry.** It is assembled as in PhotoCraft (`fn build() { v.extend(file_cmds::specs()); … }`, then `OnceLock` + `HashMap`), with three changes:
1. **Duplicate ids fail a test** instead of silently keeping the first.
2. **`find` accepts snake_case aliases.** `arrange.align_left` resolves to `arrange.alignLeft`: try exact first, then normalize each segment `_x` → `X`. Agents (and this task's own examples) will type both.
3. **Unknown ids get suggestions** (`did you mean layer.setFrequency?`), by edit distance over the ids.

**`command.list` / MCP `command_list` row** (camelCase JSON, FilmCraft's superset):

```json
{"id":"layer.setFrequency","label":"Frequency","menu":[],"shortcut":null,"defaultShortcut":null,
 "kind":"edit","agent":"allowed","params":"{layer, sub?, frequencyKhz|frequencyHz}",
 "enabled":true,"disabledReason":null,"needsDocument":false}
```

`agent` is one of `allowed`, `rootsOnly` (file I/O inside the granted roots) or `refused` (device). Agents learn what they may not do without trying. Full schemas come from `command_schema {id}` (or `command_list {schema: true, filter}`), which keeps the default listing small.

### 2.3 Kinds, origins and the device policy

```rust
pub enum Kind { Edit, View, Query, Io, Settings, Device(DeviceClass) }
pub enum DeviceClass {
    Stop,     // laser.stop, laser.pause: safe direction; allowed from every origin
    Connect,  // opens or closes the USB device
    Read,     // status/info that does I/O (laser.status reads a cached state and is a Query)
    Light,    // red light, framing, jog/home: mirrors or head move, no emission
    Emit,     // start, run machine file, resume: needs an ArmToken; never completes inside a command
}
pub enum Origin { Human, Synthetic, Headless, Replay }
```

What each kind means:

| Kind | History | Journal | Dirty | Examples |
|---|---|---|---|---|
| `Edit` | exactly one step (or one per `coalesce` run) | yes | yes | `arrange.alignLeft`, `layer.setPower`, `shape.create` |
| `View` | no (stored with states, like PhotoCraft's layer target) | no | no | `select.set`, `layer.setCurrent`, `layer.flashContent` |
| `Query` | no | no | no | `document.inspect`, `preview.compute`, `laser.status` (cached) |
| `Io` | no (an import is an Edit that also does Io) | yes | — | `file.open`, `file.import`, `file.export`, `laser.saveMachineFile` |
| `Settings` | no; machine-local, not project data | no | no | `device.set`, `prefs.set`, `material.save` |
| `Device(_)` | no | **never** (and never replayable) | no | `laser.frame`, `laser.start` |

**Policy per origin.** `Origin` is set by the transport:
- **`Human`**: real winit input in the GUI.
- **`Synthetic`**: the GUI driven by the control channel or `ui_*` tools, which is PhotoCraft's `automation_input`.
- **`Headless`**: the CLI's `run`, `batch`, `serve` and `mcp`.
- **`Replay`**: macro or journal replay and `command_batch` steps.

| Kind | Human | Synthetic | Headless | Replay |
|---|---|---|---|---|
| Edit, View, Query | ✓ | ✓ | ✓ | ✓ |
| Io | ✓ | read/write roots only | read/write roots only | per the replaying origin |
| Settings | ✓ | refused (machine calibration and work area are safety data) | refused for `device.*`; `prefs.*` allowed | refused |
| Device(Stop) | ✓ | ✓ | no device: no-op error | ✓ |
| Device(Connect, Read, Light) | ✓ | **refused** | **refused**: the session has no device provider | **refused** |
| Device(Emit) | **only as "request ARM"** (below) | **refused** | **refused** | **refused** |

**Three independent mechanisms back that table**, so no single bug opens a hole:
1. **Structural.** `openlaser-automation` cannot link `openlaser-lmc-usb` or `ol-arm` (checked by the xtask dependency lint). A headless `Session` is built with `DeviceAccess::None`, so device commands return `EngineError::NoDevice` ("automation sessions never open the laser"). Tests use `DeviceAccess::Fake(FakeLmc)`.
2. **Policy.** `dispatch` checks `kind × origin` before decoding params. The check also covers commands a command runs on its own behalf, as PhotoCraft's `authorize` does.
3. **ARM.** `laser.start` (Kind `Device(Emit)`) **never emits**:
   - **What it does.** It plans, validates, encodes and disassembles, then stores a `PendingArm { fingerprint, summary, expires }` on the session. It returns `{"armRequired": true, "summary": EmissionSummary}` and raises `Event::ArmRequested`.
   - **Who arms.** The GUI's ARM dialog (human checklist plus typed `ARM`), or `openlaser-cli run` at a TTY, mints the token through `ol-arm`. It then calls the typed Rust API `Session::run_armed(pending, token)`. **No JSON command takes a token.**
   - **What voids it.** Any revision bump clears `PendingArm`, and the driver re-checks fingerprint, session and `FrameReceipt` (PLAN §3.3).

   This is `AGENTS.md`'s "submitting a job for human approval": an agent may get as far as `armRequired`.

**Synthetic input in the GUI** extends PhotoCraft's `automation_input`. While it is set:
- device-class commands are refused;
- the ARM dialog and the Laser window's Start, Frame and Fire controls ignore injected pointer and key events, so `ui_type "ARM"` does nothing;
- an ARM dialog that is open refuses while a control client is connected.

This cannot catch OS-level synthetic input (computer use). PLAN §3.6's physical layers cover that: the USB is unplugged while agents work, and a foot-switch ARM arrives in M3.

**STOP bypasses dispatch.** Esc and the STOP button write the driver's `StopLatch` directly, before widgets and never behind `modal_allows` (PLAN §3.5, ≤ 16 ms). `laser.stop` still exists as a command for menus and agents, and it calls the same latch.

### 2.4 Dispatch pipeline

```text
execute_as(origin, id, params)
 1 find(id) | snake alias | UnknownCommand + suggestions
 2 params is object|null; split universal keys (coalesce, dryRun)
 3 policy(kind, origin)                 → Refused{code}      (before any decode or side effect)
 4 authorize hook (Io paths vs roots)   → Refused{code}
 5 decode typed params                  → BadParams{cmd, pointer, msg}
 6 enabled(&Session)                    → Disabled{reason, needsDocument}
 7 conflicts: background job on doc; device busy for Device(Light|Emit)
 8 catch_unwind(run)                    → Internal("…the project is unchanged")
 9 after: journal (Edit/Io only), revision++, clear PendingArm + preview cache if the project changed,
          push events (DocChanged{damage}, Notice, ArmRequested, Driver(..))
```

`EngineError` has these variants: `UnknownCommand{suggestions}`, `BadParams`, `Disabled`, `Refused(RefusalCode)` (policy and safety, machine-readable), `NoDocument`, `NoShape(id)`, `NoDevice`, `Cancelled`, `Internal`. The CLI and MCP show `Refused` codes verbatim, so agents can explain them to Luiz.

### 2.5 Command ids for LightBurn actions

**Scheme.** It is PhotoCraft's scheme, applied to LightBurn's menus and windows:
- **Ids that follow a menu:** `<topMenu>.<itemCamelCase>`, with submenus as extra segments.
- **Ids for windows and panels:** `<window>.<action>`.
- **Agent-only operations:** a descriptive namespace.
- **Case:** camelCase segments, matching PhotoCraft, VectorCraft and FilmCraft. snake_case aliases are accepted (§2.2), so `arrange.align_left`, `layer.set_speed`, `laser.frame` and `tools.offset` all work.

**Menu items and parameterized cores.** Each LightBurn menu item gets its own id, so it can carry its own shortcut and translation. A family shares one parameterized core that agents can call directly, which is VectorCraft's `object.align` lesson:
- `arrange.alignLeft` and its siblings are generated from a table and call `arrange.align {edge, ids?, to?}`.
- `layer.setSpeed` and its siblings call `layer.set {layer, sub?, …partial cut settings}`.

**Namespaces.** LightBurn's menus are File, Edit, Tools, Arrange, Laser Tools, Window, Help (research 02 cites `Reference/UI/*Menu`).

| Namespace | Kind | Source in LightBurn | Representative ids |
|---|---|---|---|
| `file.*` | Io/Edit | File menu | `file.new` ⌘N, `file.open` ⌘O, `file.openRecent`, `file.import` ⌘I, `file.save` ⌘S, `file.saveAs` ⇧⌘S, `file.export {format: svg\|dxf\|png}`, `file.close`, `file.recover`, `file.discardRecovery`, `file.autoSaveNow`, `file.importPrefs`, `file.exportPrefs` |
| `edit.*` | Edit/View | Edit menu | `edit.undo` ⌘Z, `edit.redo` ⇧⌘Z, `edit.cut`/`copy`/`paste` ⌘X/C/V, `edit.pasteInPlace`, `edit.duplicate` ⌘D, `edit.delete` ⌫, `edit.selectAll` ⌘A, `edit.invertSelection`, `edit.selectOpenShapes`, `edit.selectAllInLayer`, `edit.selectContained`, `edit.convertToPath`, `edit.convertToBitmap`, `edit.closePath`, `edit.closeSelectedPathsWithTolerance`, `edit.autoJoin`, `edit.optimizeSelectedShapes`, `edit.deleteDuplicates`, `edit.breakApart`, `edit.reverseShapeDirection`, `edit.setStartPoint` |
| `tools.select.*` | View (shell) | Tools palette | `tools.select.select`, `.editNodes`, `.line`, `.rectangle`, `.ellipse`, `.polygon`, `.text`, `.measure`, `.tabs`, `.positionLaser` |
| `tools.*` | Edit | Tools menu | `tools.offsetShapes {distanceMm, direction, corner}`, `tools.weld`, `tools.booleanUnion` Alt++, `tools.booleanSubtract` Alt+−, `tools.booleanIntersection` Alt+* (shortcuts per 02 MOD-01), `tools.traceImage` (job), `tools.adjustImage`, `tools.applyMaskToImage`, `tools.applyPathToText`, `tools.cutShapes`, `tools.trimShapes`, `tools.radiusFillet`, `tools.rubberBandOutline`, `tools.generatePathFromCut` |
| `arrange.*` | Edit | Arrange menu | `arrange.group` ⌘G, `arrange.ungroup` ⌘U, `arrange.autoGroup`, `arrange.flipHorizontal`, `arrange.flipVertical`, `arrange.mirrorAcrossLine`, `arrange.rotate90Cw`, `arrange.rotate90Ccw`, `arrange.align` + `alignLeft/Right/Top/Bottom/CentersVertical/CentersHorizontal/Centers`, `arrange.distribute` + `distributeHorizontal…`, `arrange.moveTogether`, `arrange.moveToPage {anchor}` + `moveToPageCenter`, `arrange.gridArray`, `arrange.circularArray`, `arrange.copyAlongPath`, `arrange.quickNest` (job), `arrange.order.{toFront,toBack,up,down}` |
| `laserTools.*` | Edit (generates a test project) | Laser Tools menu | `laserTools.materialTest {rows: power\|speed\|frequency\|qPulse…}`, `laserTools.intervalTest`, `laserTools.calibrateGalvoLens` (wizard), `laserTools.centerFinder`, `laserTools.cylinderCorrection` |
| `layer.*` | Edit/View | Cuts/Layers window + Cut Settings Editor | `layer.setCurrent` (View: palette click without selection), `layer.assign {layer, ids?}` (palette click with selection), `layer.set {layer, sub?, …}`, sugar `layer.setSpeed`, `.setPower`, `.setFrequency`, `.setQPulse`, `.setPasses`, `.setMode`, `.setInterval`, `.setOutput`, `.setShow`, `.rename`, `.moveUp/Down/To`, `.copySettings`, `.pasteSettings`, `.resetToDefaults`, `.sortCutsLast`, `.selectShapes`, `.flashContent` (View), `layer.subLayer.{add,duplicate,remove,move}` |
| `shape.*` | Edit/Query | Shape Properties, drawing tools | `shape.create {kind: rect\|ellipse\|polygon\|star\|line\|path\|text, …, layer?}` (each drawing tool's commit, and the agent's way to draw), `shape.set {ids, powerScalePct?, priority?, name?, locked?}`, `shape.inspect {id}` |
| `transform.*` | Edit | Numeric Edits toolbar, handles, arrow nudges | `transform.set {ids?, xMm?, yMm?, widthMm?, heightMm?, rotationDeg?, anchor?, lockRatio?}`, `transform.move {dxMm, dyMm}` (coalescable nudges), `transform.scale`, `transform.rotate`, `transform.skew`, `transform.matrix {affine}` |
| `node.*` | Edit | Edit Nodes tool | `node.delete` (D), `node.insert` (I), `node.insertMidpoint` (M), `node.toCorner` (C), `node.toSmooth` (S), `node.break` (B), `node.toLine` (L), `node.trim` (T), `node.extend` (E), `node.move` (single-key shortcuts per 02 EDT-05) |
| `variableText.*` | Edit | Variable Text window | `variableText.next`, `.previous`, `.reset`, `.setRange`, `.bake` |
| `select.*` | View | none (agent-facing) | `select.set {ids}`, `select.add`, `select.remove`, `select.none`, `select.byLayer` |
| `optimize.*` | Edit | Optimization Settings (saved in the project, as LightBurn's `<UIPrefs>` is) | `optimize.set {…}`, `optimize.reset`, `optimize.setAsDefaults` |
| `preview.*` | Query (job) | Preview window | `preview.compute` (time estimate, cut/travel distance, per-layer rows, notices), `preview.render` (PNG; MCP `render_preview`) |
| `laser.*` | **Device** / Query / Io | Laser window | `laser.devices` (Q), `laser.status` (Q, cached), `laser.connect` (Connect), `laser.disconnect` (Stop), `laser.frame {style: bounds\|hull\|contour, continuous?, selectedOnly?}` (Light), `laser.frameStop` (Stop), `laser.redLight {on}` (Light), `laser.start {selectedOnly?}` (**Emit → armRequired**), `laser.pause` (Stop), `laser.resume` (Emit), `laser.stop` (Stop), `laser.home` (Light, gantry), `laser.saveMachineFile {format: gcode}` (Io), `laser.runMachineFile` (Emit) |
| `move.*` | Device(Light) | Move window (gantry) | `move.jog {dxMm, dyMm}`, `move.toPosition`, `move.savePosition` |
| `device.*` | Settings / Query | Devices, Device Settings, galvo calibration | `device.list` (Q), `device.select`, `device.add`, `device.duplicate`, `device.importEzcadConfig {path}`, `device.set {path, value}`, `device.calibrateAxis {axis, requestedMm, measuredMm}` |
| `material.*` | Edit / Settings | Material Library | `material.open`, `material.assign {layer, entry}`, `material.link`, `material.saveFromLayer` |
| `history.*` | Edit/Query | Undo History window (2.1) | `history.list` (Q), `history.goTo {index}` |
| `window.*`, `help.*`, `view.*` | shell (`UI_COMMANDS`) | Window and Help menus, canvas view | `window.toggle.cuts`, `.laser`, `.move`, `.preview`, `.undoHistory`, `.shapeProperties`, `.library`, `.variableText`, `view.zoomIn`, `view.zoomToPage`, `window.resetLayout` |
| `session.*`, `document.*`, `command.*`, `jobs.*`, `prefs.*` | Query/Settings | none | `session.inspect`, `document.inspect`, `document.activate`, `command.list`, `command.schema`, `jobs.list`, `jobs.cancel`, `prefs.set {path, value}` (shortcut overrides at `shortcuts.<id>`, as in PhotoCraft) |

**Shortcuts.** Only well-known defaults and those cited in research 02 are filled in above.
- **The rest.** Take them from LightBurn's Edit Hotkeys list into `menu_catalog.rs` by clean-room observation (names, placement, default keys). The local LightBurn 1.5.04 install could serve as the observation source (06 G10).
- **A conflict to settle.** Research 02 cites LightBurn's Weld as Ctrl+W, which collides with macOS's ⌘W (close). The catalog row records LightBurn's key; Luiz decides the Mac binding.

**`menu_catalog.rs` is the parity meter.** Every LightBurn menu item becomes `(path, label, default shortcut, id)`. Items without an implementation render disabled. `cargo xtask parity` generates `docs/parity-checklist.md`, exactly like PhotoCraft's.

### 2.6 Document model (`openlaser-doc`)

Conventions:
- **Units.** mm, `f64`, **Y-up**. On a galvo, the origin is the **field centre** (PLAN §3.2).
- **`.lbrn2` is also Y-up**, verified on one sample (06 §0.7). Where its origin sits on galvo files is UNVERIFIED.
- **Draw order.** Index 0 is the bottom, documented once (the PhotoCraft convention).
- **Ids.** A **per-project counter**, saved in the file, so they are deterministic for goldens, journals and agent replays.

```rust
pub struct Project {
    pub id: ProjectId, pub name: String,
    pub shapes: Vec<Arc<Shape>>,            // structural sharing: an edit clones only the touched Arcs
    pub layers: CutTable,                   // all 30 colour layers + T1/T2, always present
    pub workspace: Workspace,               // size + origin of the target field/bed (display, import)
    pub device_hint: Option<DeviceHint>,    // kind + profile name; NEVER calibration (machine-local)
    pub optimization: Optimization,         // LightBurn <UIPrefs> Optimize_*; saved with the project
    pub variable_text: VariableTextState,   // Start/End/Current/Increment/AutoAdvance
    pub notes: Notes,
    pub next_id: u64,
    pub extras: Extras,                     // unknown foreign keys (.lbrn2) kept for round trip
}
pub struct Shape {
    pub id: ShapeId, pub layer: LayerRef,   // Color(0..=29) | Tool(1|2)
    pub xform: kurbo::Affine,               // object → workspace mm (lbrn2 <XForm> a b c d e f)
    pub geom: Geom, pub name: Option<String>, pub locked: bool,
    pub power_scale: Percent, pub priority: i32, pub start: Option<StartPoint>,
}
pub enum Geom {
    Path(Arc<PathData>),                    // kurbo BezPath + closed flags; shared like VertID/PrimID
    Rect { w: Mm, h: Mm, corner_radius: Mm }, Ellipse { rx: Mm, ry: Mm },
    Polygon { sides: u16, radius: Mm }, Star { points: u16, radius: Mm, ratio: f64, bulge: f64 },
    Text(Arc<TextSpec>),                    // font, size, spacing, weld, variable text; backup path
    Bitmap(Arc<BitmapSpec>),                // asset hash, px size, gamma/contrast/brightness/enhance
    Group(Vec<Arc<Shape>>),                 // later: Barcode, VirtualArray, depth map
}
```

Parametric shapes stay parametric until `edit.convertToPath`, as in LightBurn. The planner flattens through one `Geom::to_bez(&self) -> BezPath` with a fixed tolerance.

### 2.7 Layers and cut settings (LightBurn model, with galvo and MOPA first)

These follow research 02 §7, with one deliberate divergence from PLAN §2: **energy parameters live in separate galvo and gantry blocks**, so MOPA defaults can never leak into a diode job (bug B11). Geometry and mode parameters are shared, so one project retargets between Luiz's MOPA and his GRBL machines, as LightBurn's REST model stores both blocks.

```rust
pub struct CutTable { pub colors: [ColorLayer; 30], pub tools: [ToolLayer; 2], pub order: Vec<u8> /* Cuts window order = output order */ }
pub struct ColorLayer {
    pub index: u8, pub name: String, pub output: bool, pub show: bool, pub priority: i32,
    pub ops: SubLayers,          // 1..=11 CutOps run left→right on the same geometry; ops[0] is the layer itself; >1 = "Multi"
    pub global_passes: u16,      // galvo: repeats the whole sub-layer stack (total = global × per-op passes)
    pub image: ImageCut,         // LightBurn's separate image cut for bitmaps on this colour
}
pub struct ToolLayer { pub which: u8 /* 1|2 */, pub name: String, pub show: bool }   // no cut params: cannot output
pub struct CutOp {
    pub name: String, pub enabled: bool, pub mode: CutMode /* Line | Fill | OffsetFill */, pub passes: u16,
    pub galvo: GalvoParams, pub gantry: GantryParams, pub fill: FillParams, pub line: LineParams,
}
pub struct GalvoParams {
    pub speed: MmPerS,            // as requested; the planner encodes once to whole units, refuses 0 (D9)
    pub power: Percent, pub frequency: Hz /* lbrn2 stores Hz; UI shows kHz */, pub q_pulse: Option<Ns>,
    pub timings: Option<GalvoTimings>,  // per-layer override: laser on/off/end/polygon TC (µs), jump speed,
                                        // min/max jump delay (µs), jump distance limit
    pub dot: Option<DotDelays>, pub wobble: Option<Wobble /* step, size mm */>, pub alt_source: bool,
}
pub struct FillParams { pub interval: Mm, pub angle_deg: f64, pub angle_increment_deg: f64, pub bidirectional: bool,
    pub cross_hatch: bool, pub flood_fill: bool, pub scan_opt: ScanOpt, pub dot_width: Mm, pub auto_rotate: bool, pub overscan: Percent /* gantry */ }
pub struct GantryParams { pub speed: MmPerS, pub power_max: Percent, pub power_min: Percent, pub air_assist: bool, pub constant_power: Option<bool> }
```

**Sub-layers are in the model from day one** (`ops` is a vector), even though the editor UI comes later. That avoids a format migration and lets `.lbrn2` `<SubLayer>` import losslessly.

**Field mapping.** Keys seen in the two research `.lbrn2` samples are marked ✓; the others follow LightBurn's REST names and are UNVERIFIED as XML.

| LightBurn (UI / REST) | `.lbrn2` key | OpenLaser | OM prototype (`layers.py`) |
|---|---|---|---|
| Name / Output / Show | `name` ✓ / ? / ? | `ColorLayer.name/output/show` | `name`, `output`, `visible` |
| Mode Line/Fill/Offset Fill/Image/Tool | `type="Cut\|Scan\|Scan+Cut"` ✓ | `CutOp.mode`, `ImageCut`, `ToolLayer` (`Scan+Cut` → two ops) | `operation`: vector_engrave/cut → Line, raster_engrave → Fill, frame_only → T1, disabled → `output=false` |
| Speed | `speed` ✓ (mm/s on galvo) | `galvo.speed` / `gantry.speed` | `speed_mm_s` |
| Max / Min power | `maxPower` ✓ `minPower` ✓ | `galvo.power`; `gantry.power_max/min` | `power_percent` |
| Frequency (Hz) | `frequency` ✓ | `galvo.frequency` | `frequency_khz × 1000` |
| Q-Pulse width (ns) | `QPulseWidth` ✓ | `galvo.q_pulse` | `pulse_width_ns` |
| Passes / Global passes | ? | `CutOp.passes` / `global_passes` | `passes` |
| Interval / Angle / Angle increment | `interval` ✓ / ? / ? | `fill.interval/angle_deg/angle_increment_deg` | `raster_pitch_mm`, `raster_angle_deg`, `raster_angle_step_deg` |
| Bi-directional / Fill grouping | `bidir` ✓ / `scanOpt` ✓ | `fill.bidirectional/scan_opt` | always serpentine / merge-all |
| Cross-hatch, Flood fill, Dot width, Auto-rotate | ? | `fill.*` | — |
| Timings override, Wobble, Dot delays | ? (UNVERIFIED, 02 §17.2) | `galvo.timings/wobble/dot` | device-level only |
| Priority, Tabs | `priority` ✓, `tabCount` ✓ | `priority`, `line.tabs` | layer order |
| Air assist | ? | `gantry.air_assist` | `air_assist` |

**Where device-kind parameters come from.**
- **Per-device-kind defaults** (LAY-05, B11) live in the device profile. `layer.resetToDefaults` copies the active kind's block only.
- **Importers** fill the block of the file's device kind. A `.lbrn2` with `frequency` or `QPulseWidth` reads as galvo; otherwise the active device's kind is used, with a notice.

### 2.8 Undo model

- **Snapshots, as PhotoCraft and PdfCraft do.** `History { undo: VecDeque<State>, redo: Vec<State>, max_states }`, where `State { label: Label, project: Arc<Project>, selection: SelectionTarget }`. An edit clones `Vec<Arc<Shape>>` (pointer copies) and replaces only the touched shapes. Bitmaps and paths are shared `Arc`s, so 100 steps of a 20k-shape project cost pointers, not geometry. This is research 04's recommendation too.
- **In history:** everything in `Project`, including shapes, layers and **cut settings** (Luiz should confirm the cut-settings choice; open question 6), optimization, notes and variable text.
- **Not in history:**
  - Selection: stored *with* each state and restored by undo, like PhotoCraft's layer target.
  - The current layer, view, prefs and device profiles (machine-local).
  - Connection and driver state, the preview cache and `PendingArm`.
- **Coalescing.** The universal `coalesce` key covers Cuts-window slider drags, Numeric Edits typing, arrow-key nudges and text typing.
- **Atomic groups (new).** `Session::group(label, |s| …)` records one step around many commands and **rolls back to the snapshot on any error**. MCP `command_batch {atomic: true, label}` uses it. PhotoCraft's batch cannot do this; with snapshots it is about 30 lines. Device commands are refused inside a group.
- **Labels are translatable.** `Label { wording: &'static str, value: Option<String> }` (PdfCraft), for example `Label::with("Set {name} Frequency", "C05")`. The Undo History window (`history.goTo`) shows translated text, while `history.list` returns English plus the command id.
- **Safety coupling.** Every revision bump (edit, undo, redo) voids `PendingArm`. The driver's fingerprint check already refuses stale jobs. A running job keeps marking its encoded snapshot whatever the editor does: undo cannot change bytes in flight.
- **Limits.** `max_states` comes from Preferences (LightBurn's undo-levels setting; its default is UNVERIFIED), with an optional byte budget for bitmap-heavy projects.

### 2.9 Native file format and autosave (`openlaser-format`)

**`.olp`** ("OpenLaser Project") is a working name until the rename decision (PLAN D4). It is a ZIP: DEFLATE for JSON, STORE for assets that are already compressed.

```text
manifest.json          {"format":"openlaser-project","formatVersion":1,"generator":"openlaser 0.1.0",
                        "project":{…ProjectM…},"paths":{"p7":"M0 0L10 0C…Z", …}}
assets/<blake3>.<ext>  original image bytes exactly as imported (never re-encoded)
thumb.png              optional, caller-rendered (the format crate does not render)
```

- **Paths inline, as compact strings.**
  - **Form.** An SVG-path-like string, deduplicated in a `paths` table, with shapes holding `pathRef`. This mirrors `.lbrn2` VertID/PrimID sharing.
  - **Why it is safe.** Rust's `f64` `Display` prints the shortest string that round-trips, so it is exact.
  - **Trade-off against PhotoCraft's binary blobs.** A string is diffable and `insta`-snapshot friendly, at roughly half the size of JSON number arrays.
  - **Escape hatch.** If huge traced images make it slow, a later `formatVersion` moves paths to `paths/<blake3>.bin`.
- **Taken from `.pcraft`:**
  - a separate `ProjectM` serde model;
  - the `STEPS` migration table and `TooNew`;
  - exhaustive `Project { … }` / `ProjectM { … }` literals in `convert.rs`, so a new field fails the build until saved, with `#[serde(default)]` on added fields;
  - `NonFinite { path }` refuses to save NaN or infinity;
  - `LimitExceeded` caps entry count, decompressed size, group depth and point count, and refuses zip-slip names;
  - a fuzz target for the reader.
- **Save** goes through `craft-atomic`, pinned by rev as PhotoCraft pins `craft-print` (`3d37da38`).
- **Autosave and recovery.**
  - **Mechanism.** The `.pcraft` `Autosaver` and `RecoveryStore` design: a background thread coalescing to the newest `Arc<Project>`.
  - **Location.** `<app support>/openlaser/recovery/<project-id>.olp` plus a sidecar.
  - **Lifetime.** An entry is deleted only on save or close.
  - **Commands.** `file.recover`, `file.discardRecovery`, `file.autoSaveNow` (FilmCraft's ids); the interval is a pref (02 UI-07).
- **Machine-local data never goes in a project.**
  - **What it covers.** Device profiles, calibration (today's `scale_correction` 1.1834, red-dot offset and scale, `.cor` data), layer defaults and the material library.
  - **Where it lives.** Under the app's config folder through `craft-settings` (atomic, backed up, migrated), as `devices/<id>.json`, plus `.clb`/`.lbdev` import later.
  - **What the project keeps.** Only a `device_hint` and a workspace snapshot. This keeps the OM rule that `calibration.json` is machine-local.
- **Other formats** are `file.import`/`file.export` (`openlaser-io`), never the native save: SVG and DXF, the OM `workspace.json` + `layer_settings.json` (a migration path for Luiz's existing work), and `.lbrn2` (§2.10).

### 2.10 `.lbrn2` import, later (T1); write is T2 and UNVERIFIED

This uses `quick-xml` and treats the file purely as data. The structure comes from research 02 §17.2:
- **Cut settings.** `<CutSetting type=…>` and `<CutSetting_Img>` map through §2.7's table. Only non-default keys are written, so the importer needs a LightBurn defaults table taken from the REST OpenAPI. `<SubLayer>` maps to `ops[1..]`, and `Scan+Cut` becomes two ops.
- **Shapes.** `Path` parses `VertList` (`V x y c0x… c1x…`) and `PrimList` (`L i j`/`B i j`/`LineClosed`), with VertID/PrimID sharing becoming `Arc<PathData>`. `Rect` maps from `W/H/Cr`, `Ellipse` from `Rx/Ry`, and `Group` from `<Children>`. `Text` keeps the font string and uses `<BackupPath>` when the font is missing. `Bitmap` reads base64 `Data` into an asset.
- **Other sections.** `<UIPrefs>` `Optimize_*` maps to `optimization`, and `<VariableText>` and `<Notes>` to their fields.
- **Leftovers.** Unknown elements go to `extras` and raise one numbered import notice. Nothing is dropped silently.
- **Fixtures first.** Y-up is verified on one file. Origin, sub-layers and galvo timing tags need fixtures, ideally generated with Luiz's LightBurn 1.5.04 (06 G10).

### 2.11 Session and UI integration

```rust
pub struct Session {
    docs: Vec<DocState>, active: Option<usize>,
    pub prefs: PrefsStore, pub clipboard: Option<Clip>, pub journal: Vec<(String, Value)>,
    devices: DeviceHub,            // profiles + at most one DriverHandle per machine; DeviceAccess::None in headless
    pending_arm: Option<PendingArm>, jobs: Jobs, events: Vec<Event>,
    pub authorize: Option<fn(&str, &Value) -> Result<()>>, origin: Origin,
}
pub struct DocState {
    pub project: Arc<Project>, pub history: History, pub selection: Selection /* ids + key shape */,
    pub current_layer: LayerRef, pub path: Option<PathBuf>, pub revision: u64, pub saved_revision: u64,
    coalesce: Option<String>, preview: Option<Arc<PreviewCache>>,
}
```

**How the UI is built.** `openlaser-ui` follows PhotoCraft's shell:
- **Menus.** `menu_catalog.rs` (LightBurn's tree), plus `UI_COMMANDS` (shell-only), plus engine specs.
- **Entry point.** One `invoke(app, id, params)`. An area interceptor opens the dialog when a parameterized command arrives with empty params (`tools.offsetShapes {}` opens the Offset dialog); otherwise the command runs straight away.
- **Shortcuts.** One merged binding table, with overrides in prefs `shortcuts.<id>`. A disabled command shows its reason on the status bar.
- **Localization.** `tr_id` with TSV catalogs; ids and params never translated.
- **Palette.** A command palette over the registry.
- **Panels** (Cuts/Layers, Shape Properties, Laser window) are views over `document.inspect`-style view-models. Every change they make is a command, so `ui_*` automation and the Undo History see it.

**Two deliberate breaks from the shell pattern:**
1. STOP bypasses `invoke` and the modal gate (§2.3).
2. The ARM dialog and the Laser window refuse synthetic input.

### 2.12 Automation surface (engine side)

**MCP tools.** Same names as the family, so Luiz's existing agent prompts transfer:
- `command_list`, `command_schema`, `command_run`, `command_batch` (`atomic` optional);
- `doc_new`, `doc_open`, `doc_import`, `doc_save`, `doc_export`, `doc_inspect`, `doc_close`, `session_list`;
- `render_preview`, `preview_compute`, `jobs_list`, `jobs_cancel`;
- bridge mode only: `ui_inspect`, `ui_screenshot`, `ui_click`, `ui_key`, `ui_type`, `ui_menu_invoke`.

**MCP launch.** `openlaser-cli mcp --automation-read-root ~/Craft --automation-write-root ~/Craft`.

**CLI.**
- **Headless subcommands:** `convert`, `info`, `run --cmd <id> --params <json> … --out`, `batch`, `commands [--json]`, `mcp`, `serve`.
- **Hardware subcommands, separate and human-only:** `detect`, `status`, `frame`, `run --job`, PLAN §3.6 style. They need a TTY plus a bench session, and they are the only CLI code linking `-lmc-usb` and `ol-arm`.

`run --cmd laser.start` on a headless session returns `armRequired` and nothing else.

### 2.13 Testkit and test conventions

`openlaser-testkit` is a dev-dependency only:
- `session()` (headless, `DeviceAccess::None`) and `session_fake()` (FakeLmc from `-lmc-emu`, virtual clock, emission recorder).
- `fixture(name)`, which loads from `tests/fixtures/`: OM workspaces, synthetic SVG and DXF, and `.lbrn2` samples once their licences are checked.
- `assert_undo_roundtrip(&mut s, id, params)`. It runs the command, checks that undo returns a `Project` equal to the one before and that redo returns the one after, and that the selection is restored.
- `assert_refused(&mut s, origin, id, params, RefusalCode)`.
- `adversarial_params()`. The PhotoCraft four, plus NaN and infinity as strings and numbers, `1e308`, negative and zero speed, 0 Hz, `"frequencyKhz": 1e9`, unknown keys, wrong types, huge arrays and deep groups.
- `golden::*`, which compares plans and encoder bytes with the committed Python-oracle outputs in `tests/golden/`. The tests read committed files; Python is never run in CI.

Required test files:

| Test | Asserts |
|---|---|
| `tests/registry.rs` | Unique ids. Every id matches `^[a-z][a-zA-Z0-9]*(\.[a-z][a-zA-Z0-9]*)+$`. Every schema generates. Every catalog id is live or listed as planned. snake aliases resolve. `Device` kinds are never journaled or replayable |
| `tests/policy.rs` | For **every** `Device(Connect\|Read\|Light\|Emit)` command and every origin except `Human`: `Refused`. Headless `laser.start` returns `armRequired` and never touches the device provider. Synthetic `ui_type "ARM"` arms nothing |
| `tests/panic_hunt.rs` (`--ignored`, nightly) | Every command × `adversarial_params()`, on a thread with a deadline, with the panic hook counting contained panics |
| `tests/undo_prop.rs` | proptest: random sequences of Edit commands, then undo all equals the original and redo all equals the final; atomic groups roll back on an injected error |
| `tests/commands_snapshot.rs` | `insta` snapshot of `command.list` (ids, kinds, docs, schemas): the agent contract changes only in review |
| `openlaser-format` tests | Save → load equality; migrations; `TooNew`; `NonFinite`; zip-slip and limits; fuzz the reader |

Lints: the PhotoCraft deny-list, with `allow-unwrap-in-tests`, workspace `unsafe_code = "forbid"` and `panic = "unwind"` (PLAN §3.5).

### 2.14 Adding a command in OpenLaser

This is PhotoCraft's checklist, adapted:
1. **Pick the id.** Find the LightBurn item in `menu_catalog.rs` and use its id. Otherwise use a descriptive id in the namespace table's style.
2. **Put the algorithm** in the lowest crate that fits (`geom`, `plan`, `io`), with unit tests and no I/O.
3. **Write the command** in `engine/src/<area>_cmds.rs` → `specs()`. It needs a typed params struct (doc comments become the schema), a `kind`, an `enabled` with a reason, and a `run` that makes exactly one `edit()` (or none for View and Query).
4. **Tests:** behaviour, `assert_undo_roundtrip`, disabled reasons, bad params. If the kind is `Device`, a policy test as well.
5. **Regenerate** `cargo xtask parity` and the `command.list` snapshot.

### 2.15 When to build it (against PLAN §7)

The plan moves undo and zip projects to M2. With the Craft pattern, **the registry is the spine that the GUI, CLI and MCP all call**, so it should land with the first UI work:
- **M1.3:** registry, `Session::edit`, snapshot `History` (cheap) and the policy table.
- **M2:** the `.olp` format and autosave.

That ordering adds an estimated 3–5 focused days to M1.3 and saves retrofitting every GUI action later (open question 4).

---

## Part 3. Open questions

1. **Id casing.** This design uses camelCase (PhotoCraft, VectorCraft, FilmCraft) and accepts snake_case aliases. This task's examples, and PdfCraft, use snake_case. Pick one canonical form before the first command lands.
2. **Names.** Crate prefix `openlaser-*` (matches `AGENTS.md`) or the plan's `ol-*`, and the project extension `.olp`. Both depend on the rename decision (PLAN D4). "LightCraft" is already a sibling's name in PhotoCraft's `AGENTS.md`.
3. **Agent device policy.** This design gives agents `laser.stop`, `laser.pause` and cached `laser.status`, and refuses everything else device-side, red-light framing included, per `AGENTS.md` §1. Should agents ever get read-only *live* status (`Device(Read)`) in bridge mode?
4. **Schedule.** Move the registry, `Session::edit`, snapshot undo and policy into M1.3 (recommended), leaving the file format in M2?
5. **Shared vs per-device parameters.** Should interval, angle and passes stay shared while speed, power and frequency are per-kind, as proposed? This diverges from LightBurn's shared speed and power, on purpose (PLAN §2, B11).
6. **Cut settings in undo.** Are cut-setting edits undoable? Proposed yes. LightBurn's behaviour is UNVERIFIED.
7. **The LightBurn menu tree and default hotkeys** for `menu_catalog.rs`. Capture them clean-room, ideally from the local LightBurn 1.5.04 (06 G10; licence or trial state UNVERIFIED). Also choose the Mac binding for Weld, which research 02 cites as Ctrl+W and which collides with ⌘W (close).
8. **Galvo workspace origin.** Centre (plan) vs what LightBurn shows and writes in galvo `.lbrn2` files: UNVERIFIED. It needs a fixture with known geometry before the importer is built.
9. **Path storage.** Inline compact strings in `manifest.json` (proposed for v1) or binary blobs from day one.
10. **Family baseline.** PhotoCraft's `craft-ui` is pinned to egui 0.36, while PLAN D6 says egui 0.37. Using `craft-ui` forces 0.36. MCP stack per PhotoCraft's lockfile: rmcp 3.5, schemars 1.x.
11. **Fixture licences** for the two research `.lbrn2` samples (`fire_flower`, `coat-hook-v4`) before committing them.
12. **Atomic `command_batch`** is new to the family. Propose it upstream to craftrules too?
