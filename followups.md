
## Resolved at the WP1/WP8 merge (2026-10-24)
- WP1 final-review round FIXED (4d4270f, 034ed26): world queries compose ancestor group transforms (`world_transform` added); `passes = 0` refused by `CutTable::check_passes` on load, save, in `plan()`, and behind `layer.cutSettings.set`. Callers updated at 858501d; contract deltas in SPEC-CHANGES I2.
- WP8 final-check FIXED (95fb965, 5d67ec8): Device Settings is human-only while open; conflicting `command`+`id` refused; honest disabled reasons; `terminate:`/Dock-Quit/logout now stops via `OpenLaserApp::drop` + post-`run_native` registry stop.
- STILL OPEN: grouped-shape end-to-end check for I2 (a grouped SVG/DXF/OM shape lands where designed through plan sections into FakeLmc emitted segments) — WP1 fixed the model; add the integration test at I2.

## Camera phase 6a (integrator)
- Phase 6b: lens calibration (checkerboard → correction data) and the workspace overlay (frame the camera feed under the design, LightBurn Camera Control style). The panel draws the raw feed only for now.
- MJPEG-only cameras refuse `Error::UnsupportedFormat` — this build ships no MJPEG decoder (nokhwa `decoding` pulls mozjpeg, a C toolchain). If a user's camera only compresses, evaluate a pure-Rust decoder (zune-jpeg is already in the lock via `image`) or ask the backend for a raw stream first.
- `SyntheticSource` exists behind the `synthetic` feature but nothing outside the crate uses it yet — wiring `camera.start` to it (e.g. a `OPENLASER_FAKE_CAMERA` env or a `camera.start {synthetic}` param for demos/screenshot tests) is a small followup.
- `Capture::stop` joins the pump thread — bounded by one frame interval in the normal case, but a wedged backend blocking inside `frame()` would hang `stop`. If it ever bites, join on a helper thread with a timeout and detach.
- The pump decodes every frame to RGBA even when the UI lags (latest-wins on the *decoded* frame). If CPU shows up in profiles, keep the raw `Buffer` latest-wins and decode on read instead.
- `openlaser-engine` now links nokhwa through `openlaser-camera`, so `automation`/`openlaser-cli` do too (SPEC §2.3 rule 5 allows it — camera is not in the forbidden set). If headless deploy size ever matters, make the edge optional behind an engine feature.
