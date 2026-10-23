# Attribution

OpenLaser is licensed `MIT OR Apache-2.0` (see `LICENSE-MIT`, `LICENSE-APACHE`). This file lists every third-party
source whose code, data or assets enter the repository, and the clean-room boundaries (AGENTS.md §3, PLAN §8).
Rust crate dependencies are not listed here: `Cargo.lock` pins them and `deny.toml` enforces the licence policy.

## Ported code and data

| Source | Licence | What | Where | Status |
|---|---|---|---|---|
| [meerk40t/galvoplotter](https://github.com/meerk40t/galvoplotter) 0.2.0 @ `fc35136` | MIT © 2023 MeerK40t | LMC protocol (opcodes, list commands, controller sequencing for `py-compat`) | `crates/lmc` (header `// Ported from meerk40t/galvoplotter@fc35136 <file>, MIT`) | WP4 |
| MeerK40t `balormk/`, `extra/ezd.py` | MIT © 2021 meerk40t | cross-check only | — | reference |
| OpenMopa prototype (`~/AI/OpenLaser/OpenMopa`, oracle v0 `6aed8ff`, v1 `5f7aa63`) | MIT © 2026 Luiz Bueno | geometry, hatch, DXF parser, safety rules, G-code, machine profiles, fake GRBL | `crates/geom`, `crates/import`, `crates/gcode`, `crates/plan`, `crates/safety`, `crates/machine`, `crates/lmc-sim` (MIT notice kept per file) | WP1–WP4 |
| Python-oracle golden fixtures | MIT © 2026 Luiz Bueno (generated from the two sources above) | `tests/golden/` | `tests/golden/README.md`, `manifest.json` | in repo |
| PhotoCraft, craft-libs (storytold) | MIT OR Apache-2.0 | engine, automation, control, UI patterns (ported with a notice) | `crates/engine`, `crates/automation`, `crates/control`, `crates/ui-egui`, `xtask` | WP6–WP8 |

## Assets

| Asset | Licence | Where | Status |
|---|---|---|---|
| Lucide icons (already vetted in PhotoCraft/PdfCraft `assets/icons/`) | ISC | `crates/ui-egui/assets/icons/` with its own `ATTRIBUTION.md` | WP8 |
| LightBurn layer palette colour values | interoperability facts from LightBurn's public documentation; values only | `crates/doc/src/palette.rs`, provenance in `crates/doc/PROVENANCE.md` | WP1 |

## Never copied

LightBurn's code, icons, images, screenshots or help text (LightBurn was studied for behaviour, layout and file
formats only). GPL or unlicensed sources: balor, LaserGRBL, K40 Whisperer, UGS, potrace, clayfrocker97/OpenLaser,
GP30, earlynerd. No firmware (`clone_loader.py`) and no EZCAD files.
