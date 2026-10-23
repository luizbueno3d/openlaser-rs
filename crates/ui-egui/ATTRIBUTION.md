# Attribution: `openlaser-ui-egui`

Third-party assets and ported code in this crate (AGENTS.md §3). No LightBurn code, icons, images, screenshots or
help text are used: LightBurn was studied for its layout, menus and behaviour only, and its name never appears in
the UI.

## Icons (`assets/icons/`)

Every embedded SVG has a row here; `icons::tests::every_icon_has_an_attribution_row` fails otherwise.

| File | What | Author | Source | Licence |
|---|---|---|---|---|
| `assets/icons/camera circle clipboard-paste copy eye file-input file-plus flip-horizontal-2 flip-vertical-2 folder-open hand layers link-2 lock-open lock maximize mouse-pointer-2 move pen-tool pentagon rectangle-horizontal redo-2 rotate-ccw rotate-cw ruler save scan scissors settings-2 settings spline square-terminal swatch-book trash-2 triangle-alert type undo-2 unlink wand-sparkles zoom-in zoom-out .svg` | Lucide icon `camera circle clipboard-paste copy eye file-input file-plus flip-horizontal-2 flip-vertical-2 folder-open hand layers link-2 lock-open lock maximize mouse-pointer-2 move pen-tool pentagon rectangle-horizontal redo-2 rotate-ccw rotate-cw ruler save scan scissors settings-2 settings spline square-terminal swatch-book trash-2 triangle-alert type undo-2 unlink wand-sparkles zoom-in zoom-out ` | Lucide Icons and Contributors | <https://github.com/lucide-icons/lucide> `icons/camera circle clipboard-paste copy eye file-input file-plus flip-horizontal-2 flip-vertical-2 folder-open hand layers link-2 lock-open lock maximize mouse-pointer-2 move pen-tool pentagon rectangle-horizontal redo-2 rotate-ccw rotate-cw ruler save scan scissors settings-2 settings spline square-terminal swatch-book trash-2 triangle-alert type undo-2 unlink wand-sparkles zoom-in zoom-out .svg` (lucide-static 1.49.0, copied from PdfCraft/PhotoCraft `assets/icons/`, already vetted there) | ISC, [`assets/icons/LICENSE-lucide.txt`](assets/icons/LICENSE-lucide.txt) |

Some of these Lucide icons derive from Feather (Cole Bemis, MIT); `LICENSE-lucide.txt` carries both notices and
the list of derived icons.

## Ported code

| Source | Licence | What | Where |
|---|---|---|---|
| PhotoCraft (`~/AI/PDF_Craft_Edit/photocraft`, storytold) | MIT OR Apache-2.0 | patterns: `menu_catalog.rs`, `parity.rs`, `shortcuts.rs` (strict parser), `icons.rs`, the `logic`/`ui` split, the control dispatcher shape, the docking adapter | `src/menu_catalog.rs`, `src/parity.rs`, `src/shortcuts.rs`, `src/icons.rs`, `src/app.rs`, `src/control/`, `src/dock.rs` (each file names its source) |
| craft-libs `craft-ui` @ `5c874d51` | MIT OR Apache-2.0 | docking, icon set, buttons (used as a library, not copied) | dependency |

## Fonts

egui's `default_fonts` feature embeds Ubuntu-Light (Ubuntu Font Licence 1.0), Hack (MIT), Noto Emoji (OFL 1.1)
and emoji-icon-font (MIT/OFL); their licences ship inside the `epaint_default_fonts` crate.
