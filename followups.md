
## Resolved at the WP1/WP8 merge (2026-10-24)
- WP1 final-review round FIXED (4d4270f, 034ed26): world queries compose ancestor group transforms (`world_transform` added); `passes = 0` refused by `CutTable::check_passes` on load, save, in `plan()`, and behind `layer.cutSettings.set`. Callers updated at 858501d; contract deltas in SPEC-CHANGES I2.
- WP8 final-check FIXED (95fb965, 5d67ec8): Device Settings is human-only while open; conflicting `command`+`id` refused; honest disabled reasons; `terminate:`/Dock-Quit/logout now stops via `OpenLaserApp::drop` + post-`run_native` registry stop.
- STILL OPEN: grouped-shape end-to-end check for I2 (a grouped SVG/DXF/OM shape lands where designed through plan sections into FakeLmc emitted segments) — WP1 fixed the model; add the integration test at I2.
