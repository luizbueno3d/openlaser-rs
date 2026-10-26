# Rayforge 1.12.0 — hands-on test on this Mac (2026-10-22)

**Machine:** MacBook Air M2, macOS 26.5.2. **Laser:** not connected.
**Install:** `rayforge-1.12.0-macos-arm-app.zip` (161.6 MB, SHA-256
`2f956238…79388363`) from the official GitHub release, copied to
`/Applications/Rayforge.app` (392 MB on disk). The latest beta is 1.13.0-beta4 (2026-10-21).

## How it was tested without a laser

- **Fake GRBL controller:** [`fake_grbl.py`](fake_grbl.py) is a GRBL 1.1 emulator on `127.0.0.1:2323`. It has a 15-block planner, simulated motion time, realtime `? ! ~ 0x18 0x85`, and `$$ $I $G $# $H $J=`. Every byte Rayforge sent is in [`fake_grbl.log`](fake_grbl.log).
- **Artwork:** the same DXFs used in the OpenMopa/OpenLaser tests,
  `~/Documents/Playground/tests/DXF/square.dxf` and `businessCard.dxf` (403 paths).
- **Reference output:** the Python OpenLaser prototype compiled the same DXFs for its
  `diode-grbl-400x400` profile ([`openlaser_gcode.py`](openlaser_gcode.py)).

## Results

| Area | Result | Evidence |
| --- | --- | --- |
| Install / launch | ✅ Works; launches in ~6 s. ⚠️ Ad-hoc signed and **not notarized**. `spctl` reports "a sealed resource is missing or invalid", so a browser-downloaded copy will trip Gatekeeper. The main executable is a bash wrapper around a PyInstaller binary (GTK4, libadwaita, OpenCV, libvips, ezdxf bundled). The plist version is `0.0.0`. | `codesign -dv`, `spctl -a` |
| Network on launch | ⚠️ Contacts GitHub on every start (addon registry plus update check). | session log |
| Machine wizard | ✅ Auto-probes GRBL (`$I`, `$$`) and correctly read the 400 × 400 mm work area. Offers an "AI provider" step (OpenAI-compatible key); skipped. | log lines `CMD $I / $$` |
| **Galvo / fiber support** | ❌ **None.** Searching templates for "fiber" or "galvo" returns nothing. The drivers are GRBL (Network, **Rust**, Serial, Serial Simple, Telnet), Marlin, OctoPrint, Ruida, Smoothie and a dummy. **It cannot drive the JCZ/LMC MOPA.** | wizard screenshots |
| First-run bug #1 | 🐞 "Create Machine" created the machine but the dialog stayed open. A second click errors: `No machine found with ID 8941323e…`. | screenshot + log |
| First-run bug #2 | 🐞 Until restart, every jog raised `ValueError: No machine found with ID …`: the UI kept the stale default machine. A restart fixed it. | session log traceback |
| Auto-home | ⚠️ `home_on_start: true` by default. It sent `$H` automatically on app start, so the machine moves with no prompt. | `fake_grbl.log` @1.174 s |
| Auto-reconnect | ✅ Reconnected by itself after the controller restarted. | log |
| Jog | ✅ `$J=G91 G21 F1000 X10.0`. | log |
| Frame | ✅ Traces the bounding rectangle 20× with `G1 F6000` and the laser off, then `M5` and `G0 X0 Y0`. | log |
| Run job | ✅ Streams with GRBL character counting (planner kept full at 15 blocks). 1337 lines for both DXFs. | log |
| Pause / resume | ✅ `!` feed-hold and `~` cycle-start, delivered immediately. | log @44.5 s / 48.8 s |
| Stop | ✅ `0x18` soft-reset with 15 blocks queued, streaming cancelled at line 961/1337, then `M5`, `M9` 0.2 s later. | log @74.0 s |
| Import DXF | ✅ True scale (70 × 50 mm extents), normalized to the origin. | G-code |
| G-code quality | ✅ Joins the square into one closed loop, fits arcs (`G2`/`G3`: 254 arcs), uses `M4` dynamic power and returns home. | [`rayforge-test-square-businesscard.gcode`](rayforge-test-square-businesscard.gcode) |
| Operations | Contour, Engrave (Variable Power / Constant Power / Dither / Multiple Depths; angle, cross-hatch, segmented or full-sweep scan, line spacing), Frame, Shrink Wrap, Wavefront, Command. Per-step tabs: Step, Laser (M3/M4, air assist), Post Processing. | screenshots |
| Material test grid | ✅ Generates a labelled speed × power matrix in one click. | screenshot |
| 3D simulation | ✅ OpenGL toolpath playback with scrubber and speed control. | [`rayforge-3d-simulation.jpg`](rayforge-3d-simulation.jpg) |
| Camera | ❌ The setup wizard found no camera, although this Mac has a FaceTime camera. The wizard text is V4L/Linux-centric. "Align to Physical Position" is disabled. | wizard screenshot |
| Save dialog | ✅ Native macOS save panel. | — |

## Rayforge vs the OpenLaser Python prototype (same two DXFs, GRBL diode 400 × 400)

| | Rayforge 1.12.0 | OpenLaser (Python) |
| --- | --- | --- |
| Lines | 1337 | 4298 |
| Arcs | Fitted `G2`/`G3` | None (polylines only) |
| Path joining | Square becomes 1 closed loop | 4 separate edges with laser off/on between them |
| Placement | Origin (0,0) | Centered on the bed (X165–235, Y175–225) |
| Feed / power | F500, `M4 S800` (step settings) | **F15000, `M4 S1000`.** The MOPA "Vector Cut" defaults (300 mm/s, 100%) leak into a diode job. |
| Ordering | Optimized travel | File order |

**Takeaways for OpenLaser:**
1. Layer defaults and material settings must be per machine type. A galvo's 300 mm/s at 100% is dangerous on a diode gantry.
2. Path joining, arc fitting and travel optimization are table stakes.
3. Rayforge's GRBL streaming, pause, stop and framing are solid. Its weak spots are first-run UX, macOS polish (notarization, camera) and **no galvo support at all**.

## Not tested (and why)

- Real hardware (no laser connected), Ruida, Marlin, rotary, print-and-cut, AI generation
  (needs an API key; not entered), camera calibration (no camera detected), raster images.
