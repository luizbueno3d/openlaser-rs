# I2 integration notes

I2 work completed in this checkout:

- `camera.list`, `camera.status`, `camera.stop` and calibrated mapping remain agent-readable/usable. `camera.start` and
  every `camera.calibrate.*` command now require `Access::Human`; the engine refuses Headless/Synthetic/Replay origins
  before decoding parameters. The Camera Control panel still routes its Start click through `execute_widget`, which
  applies the UI human-input provenance gate.
- Unparked the three UI kittests, T2 hatch cases and engine `panic_hunt`. The UI palette test used an obsolete
  `tools.rectangle` parameter shape; it now supplies the current `xMm`/`yMm`/`wMm`/`hMm` fields without changing its
  assignment or undo assertions.

The required `apps/openlaser/tests/` whole-app simulator suite has not been added in this checkout. Its seven I2
acceptance cases (business-card ARM/stop flow, bridge flow, frame approval refusal, both fast-stop cases, synthetic
input refusal and grouped-shape world placement) remain outstanding; I2 is therefore partial.
