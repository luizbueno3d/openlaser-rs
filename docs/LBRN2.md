# LightBurn `.lbrn2` import (P3d)

The importer is a pure Rust XML-to-document conversion. `file.open` creates a new project with the target profile's layer defaults, overlays supported LightBurn settings and restores ascending layer priority. `file.import` adds graphics on their source colour indices and preserves the current document's settings. A supplied `layer` overrides source indices, including group children. Import notices explain that source layer settings, including Output changes, are not applied by `file.import`.

## Geometry and compact path grammar

This implementation was written from format observations, the two authorized local samples and [community format documentation](https://github.com/jlucaso1/lbrn2-to-svg/blob/main/docs/main.md). No implementation code or LightBurn assets were copied. Community documentation was treated as a hypothesis, not authoritative evidence. The referenced ducktape research URL was unavailable during this task.

- Rectangles are centred at the origin, with W/H in mm and Cr as a corner radius. An invalid radius is refused, not clamped. Ellipses use Rx/Ry.
- `XForm` contains `a b c d e f`: `x' = ax + cy + e`, `y' = bx + dy + f`. Parent and child transforms multiply in that order. Reflections in individual XForms are preserved.
- `VertList` is a concatenation of `V x y` records. Decimal and exponent numbers are accepted. A record can contain `c0x number`, `c0y number`, `c1x number`, `c1y number` tokens with no separators between tokens. The optional controls are absolute coordinates.
- **The shorthand matters:** `c0x1` without `c0y` means control 0 coincides with the vertex; similarly `c1x1` without `c1y`. Thus `V2 3c0x1c1x1` has both controls at `(2,3)`, not `(1,0)`. A full pair `c0x1c0y7` instead means the literal point `(1,7)`. Absent controls also coincide with the vertex. Other incomplete pairs are refused. The real sample's many shorthand handles render without the large spurious spikes produced by treating `1` as an absolute X coordinate.
- `PrimList` concatenates `L i j` lines and `B i j` cubics. A cubic uses vertex i's control 0 and vertex j's control 1. Indices are checked. Disconnected runs start new subpaths; returning to the initial index closes the subpath. `LineClosed` joins consecutive vertices and closes; `LineOpen` joins them without closing.
- VertID and PrimID identify independently shared lists. Definitions are collected once; later instances can reuse either or both. Duplicate definitions and unresolved references are refused. Each output instance still consumes the geometry budget.
- Groups preserve their hierarchy and compose nested transforms.
- Text becomes its BackupPath outline, with a counted notice explaining that fonts and editable text are not reproduced. Text without a backup is skipped with a notice naming the text. In the real sample, BackupPath XForms sometimes duplicate the live Text XForm and sometimes retain an older translation. The importer uses the **live Text XForm once** on the backup's object-space vertices; composing the cached backup transform would double-transform or misplace the text. Other producer/version conventions remain unverified.
- Bitmaps use embedded base64 PNG data, W/H in mm, and their centred XForm. The existing raster decoder supplies pixel limits and alpha-over-white conversion. No paths, URLs, fonts or external images are loaded. Non-default bitmap processing adjustments produce notices. Unresolved bitmap deduplication references are refused.
- Unknown shapes (including Polygon, Star, Barcode and virtual-array types), unsupported shape/group children, SubLayer and unused project metadata are counted in notices. Notes, optimization preferences and variable-text behavior are not imported.

## Coordinates and visual evidence

**Conclusion for the two 1.5.04 samples: design coordinates and backup text outlines are Y-up, in millimetres. No global Y inversion or X inversion is needed.**

The opt-in render test uses `openlaser-render::document_list` and its existing CPU preview renderer. The logo's saved thumbnail is only 256×256 and includes a tiny artwork cluster at the upper right and remote attribution text at the lower left. Our full preview preserves that arrangement. A Y reflection would exchange their vertical positions. The separate attribution render reads **Created by Olena Panasovska** left-to-right and upright; it is neither mirrored nor upside-down. Main-artwork text is readable too. This is visual verification, not a pixel equality claim: preview styles, image handling and viewport padding differ.

Reviewer files (private local artifacts, excluded from the repository):

```
/private/tmp/claude-501/-Users-luizbueno-AI-ClaudeCode/ba10990c-59a4-470b-ab69-9ee5e8b41d86/scratchpad/lbrn2/thumb.png
/private/tmp/claude-501/-Users-luizbueno-AI-ClaudeCode/ba10990c-59a4-470b-ab69-9ee5e8b41d86/scratchpad/lbrn2/ours.png
/private/tmp/claude-501/-Users-luizbueno-AI-ClaudeCode/ba10990c-59a4-470b-ab69-9ee5e8b41d86/scratchpad/lbrn2/ours-detail.png
/private/tmp/claude-501/-Users-luizbueno-AI-ClaudeCode/ba10990c-59a4-470b-ab69-9ee5e8b41d86/scratchpad/lbrn2/ours-text-0.png
```

Root MirrorX/MirrorY semantics remain unverified because both samples set them False. Non-false values produce a notice; individual affine reflections are fully supported. Different LightBurn versions, gantry-origin metadata and alternative text-backup conventions need additional real samples.

## Cut settings and representational limits

Cut → Line, Scan → Fill, Offset → OffsetFill, Scan+Cut → Fill followed by Line. OffsetFill is retained so the existing planner can issue its own UnsupportedMode refusal. Unknown cut modes disable the colour's Output.

Speed maps directly to mm/s in both the galvo and gantry blocks; powers are percentages; frequency is preserved as whole Hz (the model's `khz()` view divides by 1000); QPulseWidth is ns. Interval, angle, crossHatch, bidir, numPasses and globalRepeat map to their corresponding fields. Subname takes precedence over name. Image settings use the separate ImageCut block on the same colour. Threshold, Jarvis, Stucki, Floyd/dither, Atkinson and Ordered are supported. Unsupported dithering disables Output with a notice.

Speed normalization follows the task's stated XML contract and is consistent with the MOPA values 25–900 in the sample. LightBurn's [documentation](https://docs.lightburnsoftware.com/LightBurnDocs.pdf) describes the same physical speed displaying as 100 mm/s or 6000 mm/min. That supports separating storage from UI display units, but independent gantry XML round trips have **not** been verified. Its current [REST API documentation](https://github.com/LightBurnSoftware/LightBurn-REST/blob/main/docs/api-reference.md) explicitly uses display units for laser cut values, so the REST API is not used as proof of XML speed units.

Missing parameters use **OpenLaser target-profile defaults**, never guessed LightBurn hidden defaults. One notice per colour lists missing vector and `image.` fields. Ignored keys are consolidated into one notice per colour. Values rejected by the engine's cut-setting ranges disable Output; invalid values cannot enter typed fields, so those fields retain their target defaults and the notice includes the invalid source value. No value is clamped.

The current document model has one Output flag, priority slot and global-pass count per colour, whereas LightBurn can assign independent vector/image values. This cannot be represented losslessly without a model/planner change, which is outside this task. Differing explicit vector/image priorities or differing effective global-pass counts therefore **disable the colour's Output** and produce an explicit notice. Disabled conflicting priorities sort at their earlier source priority; the vector global-pass value remains available for inspection. Unsupported image settings also disable the whole colour, including vectors, because the model has no independent image Output switch. No planner, ARM or safety behavior was changed to work around these limits.

## Input limits

The existing 32 MiB vector-file cap applies before XML parsing. DTDs are disabled. The existing raw XML guard bounds parser nesting, attributes, namespace declarations and duplicate-attribute comparison work before DOM allocation. roxmltree is capped at one million nodes. Additional limits are 100,000 shape/backup elements, XML depth 64, 200,000 vertices per list, two million defined vertices, and the existing entity/total point budgets on produced paths. Geometry reuse is charged per instance. All transformed geometry/control points must be finite and within the existing ±1e9 mm coordinate cap.

Embedded PNGs reuse the existing raster limits: 64 MiB encoded bytes, 16,777,216 pixels, 16,384 pixels per side and a 256 MiB decoder allocation limit. Total retained decoded grayscale bytes are also capped at 256 MiB. Limit failures refuse the entire import; shapes and bitmaps are never truncated.

## Files and dependencies

Changed implementation and integration files:

- `crates/import/src/lbrn2.rs`, `crates/import/src/lib.rs`, `crates/import/Cargo.toml`.
- `crates/engine/src/cmds/file.rs`, `apps/openlaser/src/native_files.rs`.
- `Cargo.toml`, `xtask/src/deps.rs`, `docs/SPEC.md`, and this report.
- `crates/import/tests/lbrn2.rs`, `crates/engine/tests/engine.rs`, `crates/render/tests/lbrn2.rs`.
- Hand-written synthetic fixtures `tests/fixtures/lbrn2/shapes.lbrn2` and `settings.lbrn2`.

Added direct importer dependencies: roxmltree **0.20.0** and base64 **0.22.1**, already present in Cargo.lock. roxmltree is now pinned in workspace dependencies; both importer edges are registered in the SPEC §2.2 table and xtask. Cargo.lock's only change is adding those two existing package names to the importer's dependency list; no package version or checksum changed. No customer artwork or extracted thumbnail is stored in the repository.

## Verification report

Synthetic tests cover exact supported geometry, rounded corners, mirrored and nested transforms, compact-control shorthands, cubics, closure, sharing, text fallbacks/notices, bitmap size and alpha, settings/defaults/ranges, dithering, priority conflicts, open/import behavior, input budgets and truncated/random input. Local-sample tests are ignored by default and skip cleanly when LBRN2_SAMPLE is unset.

Final gate results and the exact local-logo notices are recorded below after validation.
