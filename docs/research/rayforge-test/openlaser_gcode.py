"""Compile OpenLaser (Python prototype) G-code for the same DXFs on the GRBL diode profile."""
import sys, time
from mopa_luiz.importers import import_geometry
from mopa_luiz.layers import LayerSpec, LayerJob, default_layers
from mopa_luiz.machines import machine_by_id
from mopa_luiz.gcode import compile_gcode

machine = machine_by_id("diode-grbl-400x400")
layer = next(l for l in default_layers() if l.layer_id == "vector-cut")
polys = []
for path in sys.argv[2:]:
    t = time.perf_counter()
    g = import_geometry(path.rsplit("/", 1)[-1], open(path, encoding="utf-8", errors="ignore").read())
    b = g["source_bounds"]
    print(f"import {path.rsplit('/',1)[-1]}: {g['polyline_count']} polylines, {g['point_count']} points, bounds={b} ({(time.perf_counter()-t)*1000:.0f} ms)", file=sys.stderr)
    polys += g["polylines"]
prog = compile_gcode(machine, [LayerJob(layer=layer, polylines=polys)])
open(sys.argv[1], "w").write(prog.content)
print(f"paths={prog.path_count} points={prog.point_count} dist={prog.estimated_distance_mm:.1f}mm", file=sys.stderr)
