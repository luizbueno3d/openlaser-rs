# Attribution: openlaser-ui-egui

LightBurn was studied for behavior, layout and interoperability only. No LightBurn assets or code are included.

## Icons

Every SVG in `assets/icons/` is a Lucide icon (lucide-static 1.49.0), copied unchanged from the set PhotoCraft and
PdfCraft already vetted (`assets/icons/` there). Authors: Lucide Icons and Contributors. Source:
<https://github.com/lucide-icons/lucide> (`icons/<name>.svg`). Licence: ISC; the icons Lucide derives from Feather
(marked below) are also under Feather's MIT licence (Cole Bemis). Both notices are in
[`assets/icons/LICENSE-lucide.txt`](assets/icons/LICENSE-lucide.txt). They are embedded by `src/icon_data.rs`;
the test `icon_data::tests::every_icon_has_an_attribution_row` fails if an embedded SVG has no row here. No
LightBurn icon, image or screenshot is used (AGENTS.md §3).

| File | Icon | Licence |
|---|---|---|
| `assets/icons/camera.svg` | camera | ISC |
| `assets/icons/circle.svg` | circle | ISC; MIT (Feather-derived) |
| `assets/icons/clipboard-paste.svg` | clipboard-paste | ISC |
| `assets/icons/copy.svg` | copy | ISC |
| `assets/icons/eye.svg` | eye | ISC |
| `assets/icons/file-input.svg` | file-input | ISC |
| `assets/icons/file-plus.svg` | file-plus | ISC |
| `assets/icons/flip-horizontal-2.svg` | flip-horizontal-2 | ISC |
| `assets/icons/flip-vertical-2.svg` | flip-vertical-2 | ISC |
| `assets/icons/folder-open.svg` | folder-open | ISC |
| `assets/icons/hand.svg` | hand | ISC |
| `assets/icons/layers.svg` | layers | ISC |
| `assets/icons/link-2.svg` | link-2 | ISC; MIT (Feather-derived) |
| `assets/icons/lock-open.svg` | lock-open | ISC |
| `assets/icons/lock.svg` | lock | ISC; MIT (Feather-derived) |
| `assets/icons/maximize.svg` | maximize | ISC; MIT (Feather-derived) |
| `assets/icons/mouse-pointer-2.svg` | mouse-pointer-2 | ISC |
| `assets/icons/move.svg` | move | ISC; MIT (Feather-derived) |
| `assets/icons/pen-tool.svg` | pen-tool | ISC |
| `assets/icons/pentagon.svg` | pentagon | ISC |
| `assets/icons/rectangle-horizontal.svg` | rectangle-horizontal | ISC |
| `assets/icons/redo-2.svg` | redo-2 | ISC |
| `assets/icons/rotate-ccw.svg` | rotate-ccw | ISC |
| `assets/icons/rotate-cw.svg` | rotate-cw | ISC |
| `assets/icons/ruler.svg` | ruler | ISC |
| `assets/icons/save.svg` | save | ISC |
| `assets/icons/scan.svg` | scan | ISC |
| `assets/icons/scissors.svg` | scissors | ISC |
| `assets/icons/settings-2.svg` | settings-2 | ISC |
| `assets/icons/settings.svg` | settings | ISC |
| `assets/icons/spline.svg` | spline | ISC |
| `assets/icons/square-terminal.svg` | square-terminal | ISC |
| `assets/icons/swatch-book.svg` | swatch-book | ISC |
| `assets/icons/trash-2.svg` | trash-2 | ISC; MIT (Feather-derived) |
| `assets/icons/triangle-alert.svg` | triangle-alert | ISC |
| `assets/icons/type.svg` | type | ISC; MIT (Feather-derived) |
| `assets/icons/undo-2.svg` | undo-2 | ISC |
| `assets/icons/unlink.svg` | unlink | ISC |
| `assets/icons/wand-sparkles.svg` | wand-sparkles | ISC |
| `assets/icons/zoom-in.svg` | zoom-in | ISC; MIT (Feather-derived) |
| `assets/icons/zoom-out.svg` | zoom-out | ISC; MIT (Feather-derived) |

## Libraries and implementation

craft-ui (docking, toolbar fitting, SVG icon preparation) is used as a library at revision `5c874d51eeb3de70e735c8ce87a59a7dedd47675` (MIT OR Apache-2.0). The shortcut parser (`src/shortcuts.rs`) and the menu-catalogue pattern (`src/menu_catalog.rs`) follow PhotoCraft's (MIT OR Apache-2.0), as their headers say; the shell, catalogue rows, canvas, renderer and dispatcher are OpenLaser's own. The palette colour values come from `openlaser_doc::PALETTE` / `TOOL_COLORS`, whose provenance WP1 records.

## Fonts

egui default fonts ship their notices in epaint_default_fonts.
