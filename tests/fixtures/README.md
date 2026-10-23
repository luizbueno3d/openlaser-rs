# Test fixtures

Small hand-made inputs (SPEC §7.1 item 5). Integrator-owned; work packages add their own under
`model/` (WP1) and `import/` (WP2). Read them through `openlaser_testkit::fixture("<name>")`.

| File | What it is |
|---|---|
| `square10mm.svg` | a 10 mm square (black stroke, layer C00) on a 20 mm × 20 mm page, physical units (mm) |
| `unitless.svg` | a 50 × 50 square with no physical units: refused unless 96 or 72 px per inch is chosen |
| `circle.dxf` | a minimal ASCII DXF: one CIRCLE, centre (0, 0), radius 10, `$INSUNITS` = 4 (mm) |
| `gray64.png` | 64 × 64 8-bit greyscale diagonal gradient, pHYs 254 dpi (so 6.4 mm either way) |
| `om_small/workspace.json` | an OM prototype workspace: a 10 mm square (vector-engrave), a triangle (vector-cut), a 24-gon disc (raster) |
| `om_small/layer_settings.json` | OM's four built-in layers (a copy of `tests/golden/inputs/layer_settings.json`) |
