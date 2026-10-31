# Material Test

Laser Tools → Material Test… creates an untitled document using the current machine profile. The same generator is available as `laserTools.materialTest` to CLI/MCP clients. It only designs; framing and starting use the normal Laser panel and human ARM path.

```json
{
  "rows": {"param": "powerPct", "min": 1, "max": 5, "count": 5},
  "cols": {"param": "frequencyKhz", "min": 30, "max": 50, "count": 5},
  "cellWMm": 5,
  "cellHMm": 5,
  "gapMm": 2,
  "centerMm": [0, 0],
  "mode": "fill",
  "labels": true,
  "labelPowerPct": 1,
  "labelSpeedMmS": 1000
}
```

Each cell uses one of the project's fixed 30 colour layers. Labels use one more, so the limit is 29 cells with labels and 30 without. The result's `layers` lists used layers in cell order (bottom row first, left to right), followed by Labels. The project's Cuts / Layers output order is independently sorted by highest power, lowest speed, then highest pulse width. Ties retain cell order. The label layer always outputs last.

Axes accept `powerPct`, `speedMmS`, `frequencyKhz`, `qPulseNs`, `intervalMm`, or `passes`; they must differ. GRBL profiles reject frequency and pulse width, including explicit base settings. Counts are 1–30, subject to the total-cell limit. Samples include both endpoints, with a single sample using `min`. **Descending ranges are allowed**: columns proceed from `min` to `max` left to right; rows proceed from `min` to `max` bottom to top. Pass counts round to the nearest integer, with a minimum of one. Frequency samples must be representable as whole hertz, just as in Cut Settings.

`base` accepts the same six value names; unspecified settings come from the profile's layer defaults. Explicit base values are validated even if an axis subsequently overrides them. Label power and speed default independently to the profile values. All energy and fill values pass through the Cut Settings decoder and validator. Machine/source limits remain planning checks; the session emission cap remains an ARM check.

The rectangle grid is centred on `centerMm`; labels extend above and to its left. Labels use the existing text command's default font and cached outlines at 2 mm cap height. Column labels scale uniformly to fit their cell width. Row labels align right; column labels align to the cell centre. The title includes both axis names and the remaining base settings. Cell names use compact units, for example `P 5% · F 50 kHz`. `sizeMm` includes the labels' conservative outline bounds. The complete design must fit the work area, including its position and galvo calibration transform. Rotated tests use a conservative enclosing rectangle for the early fit check. Gantry checks use the new project's configured design origin; the dialog initially centres its grid on the bed.

**New-document semantics match `file.new`:** a new tab is added, and the prior tab, including unsaved edits, is retained. The current implementation of `file.new` has no unsaved-changes confirmation or refusal because it does not replace the old document. Generation validates and constructs the project before opening it; failures leave the active tab unchanged. Opening a document inside an atomic command batch is refused by the existing session path.
