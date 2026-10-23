"""Golden-oracle worker. Runs inside ONE OpenLaser-prototype (OM) source tree.

Started by generate.py, never by hand (it expects a git-archived OM tree and a
sanitised environment). The very first thing it does is make real USB
unreachable (usb_guard); only then does it import pyusb-dependent code.

  python -s -B worker.py --om-tree DIR --inputs DIR --out DIR t0|t2|t3|t4
  python -s -B worker.py --om-tree DIR --inputs DIR --out DIR t1 SCENARIO
  python -s -B worker.py --om-tree DIR --inputs DIR --out DIR list-t1
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import usb_guard  # noqa: E402  (must precede every other non-stdlib import)

usb_guard.install()
SELF_TEST = usb_guard.self_test()

import argparse  # noqa: E402
import base64  # noqa: E402
import json  # noqa: E402
import math  # noqa: E402
import random  # noqa: E402
import struct  # noqa: E402
import tempfile  # noqa: E402
import textwrap  # noqa: E402
import traceback  # noqa: E402
from pathlib import Path  # noqa: E402
from types import SimpleNamespace  # noqa: E402

import common  # noqa: E402
from fake_board import RecordingBoard, VirtualClock  # noqa: E402


# ---------------------------------------------------------------------------
# Loading one OM tree
# ---------------------------------------------------------------------------
def load_om(tree: Path) -> SimpleNamespace:
    tree = tree.resolve()
    # The venv has setuptools editable finders that map mopa_luiz to the OM
    # working copy. Drop them so the archived tree under test is imported.
    sys.meta_path[:] = [
        finder for finder in sys.meta_path
        if "__editable__" not in (getattr(finder, "__module__", "") or "")
        and "__editable__" not in type(finder).__module__
    ]
    sys.path.insert(0, str(tree))
    os.environ["OPENMOPA_MOCK"] = "1"  # OM never takes the board lock in mock mode
    os.environ.pop("OPENMOPA_USB_TRACE", None)
    import mopa_luiz

    if not Path(mopa_luiz.__file__).resolve().is_relative_to(tree):
        raise SystemExit(f"imported mopa_luiz from {mopa_luiz.__file__}, not from {tree}")
    import galvo.consts as gp_consts
    import galvo.controller as gc
    from mopa_luiz import cli, gcode, importers, layers, live, machines, raster, safety, ui

    return SimpleNamespace(tree=tree, gc=gc, consts=gp_consts, cli=cli, gcode=gcode, importers=importers,
                           layers=layers, live=live, machines=machines, raster=raster, safety=safety, ui=ui)


def markcfg(om: SimpleNamespace):
    return om.cli.load_markcfg(om.tree / "profiles" / "openmopa-machine.ini")


def jsonable(value):
    return json.loads(json.dumps(value, default=lambda o: list(o) if isinstance(o, tuple) else repr(o)))


def error_record(exc: BaseException) -> dict:
    return {"type": type(exc).__name__, "message": str(exc)}


# ---------------------------------------------------------------------------
# T0: numeric encoders
# ---------------------------------------------------------------------------
class Encoder:
    """galvoplotter's own list encoders, writing into a buffer that is never sent."""

    def __init__(self, om: SimpleNamespace, galvos_per_mm: float = 0xFFFF / 200.0) -> None:
        self.c = om.gc.GalvoController(galvos_per_mm=galvos_per_mm)  # connection stays None

        def refuse(*args, **kwargs):
            raise AssertionError("the T0 encoder must never send or flush")

        self.c.send = refuse
        self.c._list_end = refuse

    def words(self, fn, *args) -> tuple[int, ...]:
        self.c._active_list = None
        self.c._active_index = 0
        fn(*args)
        return struct.unpack("<6H", bytes(self.c._active_list[0:12]))


def _err_name(exc: BaseException) -> str:
    return f"{type(exc).__module__}.{type(exc).__name__}" if type(exc).__module__ != "builtins" else type(exc).__name__


def f64_hex(value: float) -> str:
    return float(value).hex()


def t0_mm_to_galvo(om) -> dict:
    fields = [common.NOMINAL_FIELD_MM, common.NOMINAL_FIELD_MM * common.SCALE_CORRECTION]
    groups = []
    for seed, F in enumerate(fields, start=1):
        rng = random.Random(1000 + seed)
        scale = 0xFFFF / F
        half = F / 2.0
        edges = [0.0, -0.0, half, -half, half - 1e-6, -(half - 1e-6), half + 1e-6, -(half + 1e-6),
                 math.nextafter(half, 0.0), math.nextafter(half, math.inf), math.nextafter(-half, 0.0),
                 math.nextafter(-half, -math.inf), 5.0, -5.0, 25.0, -25.0, 0.05, -0.05, 1e-9, -1e-9,
                 1e-300, -1e-300, 0.5 / scale, -0.5 / scale, 1.5 / scale, -1.5 / scale, F, -F, 1000.0, -1000.0,
                 99.9, -99.9, 100.0, -100.0, 100.5, -100.5, 118.34, -118.34]
        rows = []
        for e in edges:
            for x, y in ((e, 0.0), (0.0, e), (e, e), (e, -e)):
                rows.append([x, y, *om.live.mm_to_galvo(x, y, F)])
        # Exact ties: 0x8000 + x*scale lands on n + 0.5 exactly (half-even vs half-away differ on odd n).
        ties = 0
        for n in range(1, 0xFFFF, 13):
            x = (n + 0.5 - 0x8000) / scale
            for candidate in (x, math.nextafter(x, math.inf), math.nextafter(x, -math.inf)):
                v = 0x8000 + candidate * scale
                if v - math.floor(v) == 0.5:
                    ties += 1
                    for probe in (candidate, math.nextafter(candidate, math.inf), math.nextafter(candidate, -math.inf)):
                        rows.append([probe, -probe, *om.live.mm_to_galvo(probe, -probe, F)])
                    break
        for _ in range(10000):
            x = rng.uniform(-half, half)
            y = rng.uniform(-half, half)
            rows.append([x, y, *om.live.mm_to_galvo(x, y, F)])
        groups.append({
            "field_size_mm": F,
            "field_size_f64_hex": f64_hex(F),
            "scale_galvo_per_mm": scale,
            "seed": 1000 + seed,
            "exact_ties_found": ties,
            "n_rows": len(rows),
            "inputs_f64_sha256": common.float_bits_sha256([v for r in rows for v in r[:2]]),
            "rows": rows,
        })
    return {
        "function": "mm_to_galvo",
        "oracle_source": "OM mopa_luiz/live.py mm_to_galvo",
        "formula": "s = 0xFFFF / F; g = int(round(0x8000 + v * s)) (Python round = ties-to-even); clamp to 0..0xFFFF",
        "rust_rule": "let s = 65535.0 / f; ((32768.0 + v * s).round_ties_even() as i64).clamp(0, 65535). Same operation order. The +F/2 edge maps to 65535 only through the clamp.",
        "columns": ["x_mm", "y_mm", "gx", "gy"],
        "groups": groups,
    }


def t0_power(om) -> dict:
    enc = Encoder(om)
    c = enc.c
    rows, errors = [], []
    ties = []
    inputs = [i / 100.0 for i in range(0, 10001)]
    for i in range(0, 100001):
        p = i / 1000.0
        v = p * 0xFFF / 100.0
        if v - math.floor(v) == 0.5:
            ties.append(p)
    rng = random.Random(2001)
    inputs += ties + [rng.uniform(0.0, 100.0) for _ in range(10000)]
    inputs += [-0.0, 1e-9, 0.0122, 0.0123, 99.9999, 100.0, 100.01, 101.0, 150.0, 1600.0, 1600.4, 1600.5, -0.001, -0.01, -1.0]
    for p in inputs:
        try:
            w = enc.words(c.list_mark_current, c._convert_power(p))
        except Exception as exc:  # struct.error for words outside u16
            errors.append([p, _err_name(exc)])
            continue
        if 0.0 <= p <= 100.0 and om.cli.power_to_mark_current(p) != w[1]:
            raise AssertionError(f"OM cli.power_to_mark_current({p}) disagrees with galvoplotter")
        rows.append([p, w[1]])
    return {
        "function": "power_to_mark_current",
        "oracle_source": "galvoplotter GalvoController._convert_power + list_mark_current (0x8012 p1); equals OM cli.power_to_mark_current on 0..100",
        "formula": "word = int(round(p * 0xFFF / 100.0)) (ties-to-even); struct.pack refuses words outside 0..65535",
        "rust_rule": "(p * 4095.0 / 100.0).round_ties_even(); refuse p outside 0..=100 before encoding (safety). 30 % -> 1228, 70 % -> 2866.",
        "opcode": "0x8012 listMarkCurrent",
        "columns": ["power_percent", "mark_current_word"],
        "exact_ties": len(ties),
        "seed": 2001,
        "n_rows": len(rows),
        "inputs_f64_sha256": common.float_bits_sha256([r[0] for r in rows]),
        "rows": rows,
        "errors_columns": ["power_percent", "python_error"],
        "errors": errors,
    }


def t0_frequency(om) -> dict:
    enc = Encoder(om)
    c = enc.c
    rows, errors, ties = [], [], []
    inputs = [float(f) for f in range(1, 4001)] + [i / 10.0 for i in range(10, 1001)]
    for n in range(4, 20001):
        f = 20000.0 / (n + 0.5)
        for candidate in (f, math.nextafter(f, math.inf), math.nextafter(f, -math.inf)):
            v = 20000.0 / candidate
            if v - math.floor(v) == 0.5:
                ties.append(candidate)
                break
    rng = random.Random(3001)
    inputs += ties + [math.exp(rng.uniform(math.log(1.0), math.log(4000.0))) for _ in range(10000)]
    # Outside the profile bounds (1..4000 kHz): pins galvoplotter's & 0xFFFF wrap and its sign handling.
    inputs += [0.25, 0.3, 0.305, 0.3051, 0.31, 0.5, 0.9999, 1e-4, 4000.0001, 5000.0, 40000.0, 1e6, -1.0, -30.0, 0.0]
    for f in inputs:
        try:
            w = enc.words(c.list_qswitch_period, c._convert_frequency(f, base=20000.0))
        except Exception as exc:
            errors.append([f, _err_name(exc)])
            continue
        if f > 0 and om.cli.frequency_to_period(f) != w[1]:
            raise AssertionError(f"OM cli.frequency_to_period({f}) disagrees with galvoplotter")
        rows.append([f, w[1]])
    return {
        "function": "frequency_to_period",
        "oracle_source": "galvoplotter GalvoController._convert_frequency(base=20000) + list_qswitch_period (0x801B p1); equals OM cli.frequency_to_period for f > 0",
        "formula": "word = int(round(20000.0 / f_khz)) & 0xFFFF (ties-to-even; Python & on a negative int is two's complement)",
        "rust_rule": "((20000.0 / f).round_ties_even() as i64) & 0xFFFF for py-compat; the shipping encoder refuses f outside the profile's MINPWMFREQ..MAXPWMFREQ (1..4000 kHz) instead of wrapping.",
        "opcode": "0x801B listQSwitchPeriod (50 ns ticks)",
        "columns": ["frequency_khz", "q_switch_period_word"],
        "exact_ties": len(ties),
        "seed": 3001,
        "n_rows": len(rows),
        "inputs_f64_sha256": common.float_bits_sha256([r[0] for r in rows]),
        "rows": rows,
        "errors_columns": ["frequency_khz", "python_error"],
        "errors": errors,
    }


def t0_speed(om) -> dict:
    groups = []
    errors = []
    fields = [common.NOMINAL_FIELD_MM, common.NOMINAL_FIELD_MM * common.SCALE_CORRECTION]
    for F in fields:
        enc = Encoder(om, galvos_per_mm=0xFFFF / F)
        c = enc.c
        gpm = 0xFFFF / F
        inputs = [float(v) for v in range(0, 6001)]
        inputs += [k * 1000.0 / gpm for k in range(1, 400)]  # exact unit boundaries (as computed in f64)
        inputs += [math.nextafter(k * 1000.0 / gpm, 0.0) for k in range(1, 50)]
        inputs += [0.5, 1.0, 2.0, 3.0, 3.05, 3.0518, 3.06, 3.6, 3.61, 3.62, 7.2, 10.0, 9999.0, 199999.0, 200000.0, 236680.0, 1e7]
        rows = []
        for v in inputs:
            w = enc.words(c.list_mark_speed, c._convert_speed(v))
            j = enc.words(c.list_jump_speed, c._convert_speed(v))
            if j[1] != w[1]:
                raise AssertionError("jump and mark speed encodings differ")
            rows.append([v, w[1]])
        for v in (-1.0, -1000.0):
            try:
                enc.words(c.list_mark_speed, c._convert_speed(v))
            except Exception as exc:
                errors.append([v, F, _err_name(exc)])
        groups.append({
            "field_size_mm": F,
            "field_size_f64_hex": f64_hex(F),
            "galvos_per_mm": gpm,
            "n_rows": len(rows),
            "inputs_f64_sha256": common.float_bits_sha256([r[0] for r in rows]),
            "rows": rows,
        })
    rng = random.Random(4001)
    sweep = []
    for _ in range(10000):
        F = rng.uniform(100.0, 300.0)
        v = math.exp(rng.uniform(math.log(0.1), math.log(30000.0)))
        enc = Encoder(om, galvos_per_mm=0xFFFF / F)
        sweep.append([v, F, enc.words(enc.c.list_mark_speed, enc.c._convert_speed(v))[1]])
    groups.append({
        "field_size_mm": "random in [100, 300] per row (column 2)",
        "seed": 4001,
        "n_rows": len(sweep),
        "inputs_f64_sha256": common.float_bits_sha256([x for r in sweep for x in r[:2]]),
        "columns": ["speed_mm_s", "field_size_mm", "speed_word"],
        "rows": sweep,
    })
    return {
        "function": "speed_to_units",
        "oracle_source": "galvoplotter GalvoController._convert_speed + list_mark_speed (0x800C) / list_jump_speed (0x8006); galvos_per_mm = 0xFFFF / F as OM passes it",
        "formula": "gpm = 0xFFFF / F; word = min(int(v * gpm / 1000.0), 0xFFFF) (trunc); identical for MarkSpeed and JumpSpeed",
        "rust_rule": "((v * gpm) / 1000.0).trunc(), saturating at 65535; same operation order. Shipping refuses a mark speed that encodes to 0 units (B7).",
        "opcode": "0x800C listMarkSpeed, 0x8006 listJumpSpeed (galvo units per ms)",
        "columns": ["speed_mm_s", "speed_word"],
        "groups": groups,
        "errors_columns": ["speed_mm_s", "field_size_mm", "python_error"],
        "errors": errors,
    }


def t0_delay(om) -> dict:
    enc = Encoder(om)
    c = enc.c
    rng = random.Random(5001)
    inputs: list = list(range(-1000, 1001))
    inputs += [round(rng.uniform(-500.0, 500.0), 3) for _ in range(2000)]
    inputs += [-0.0, 0.4, -0.4, 0.999, -0.999, 8, 100, 200, 800, 30, 65535, -65535, 32768, -32768]
    rows, errors = [], []
    for d in inputs:
        try:
            on = enc.words(c.list_laser_on_delay, d)
        except Exception as exc:
            errors.append([d, _err_name(exc)])
            continue
        for fn in (c.list_laser_off_delay, c.list_polygon_delay, c.list_jump_delay):
            if enc.words(fn, d)[1:3] != on[1:3]:
                raise AssertionError(f"{fn.__name__} encodes {d} differently")
        rows.append([d, on[1], on[2]])
    for d in (65536, -65536, 1e6):
        try:
            enc.words(c.list_laser_on_delay, d)
        except Exception as exc:
            errors.append([d, _err_name(exc)])
    return {
        "function": "delay_word",
        "oracle_source": "galvoplotter list_laser_on_delay / list_laser_off_delay / list_polygon_delay / list_jump_delay (identical encoding, checked per row)",
        "formula": "p1 = int(abs(d)) (struct packs int(); trunc), p2 = 0x0000 if d > 0 else 0x8000 (so 0 and -0.0 give 0x8000)",
        "rust_rule": "p1 = d.abs().trunc() as u16 (refuse > 65535), p2 = if d > 0 {0} else {0x8000}. Keep the 0x8000 'negative zero' until hardware item H9 says otherwise.",
        "opcode": "0x8007 LaserOnDelay, 0x8008 LaserOffDelay, 0x800F PolygonDelay, 0x800D JumpDelay",
        "columns": ["delay", "p1", "p2"],
        "seed": 5001,
        "n_rows": len(rows),
        "inputs_f64_sha256": common.float_bits_sha256([float(r[0]) for r in rows]),
        "rows": rows,
        "errors_columns": ["delay", "python_error"],
        "errors": errors,
    }


def t0_pulse_width(om) -> dict:
    table = om.cli.JPT_M7_PULSE_WIDTHS_NS

    def decide(v: float) -> str:
        result = om.safety.evaluate_emission(cfg=None, power_percent=50.0, frequency_khz=30.0, pulse_width_ns=v,
                                             intends_emission=False, arm=False, confirm="", operation="frame_only")
        if result.errors:
            return "refuse"
        if result.warnings:
            return "snap"
        return "exact"

    rng = random.Random(6001)
    inputs = [i * 0.05 for i in range(0, 12001)]
    for a, b in zip(table, table[1:]):
        mid = (a + b) / 2.0
        inputs += [mid, math.nextafter(mid, 0.0), math.nextafter(mid, math.inf)]
    for t in table:
        inputs += [t - 0.5, t + 0.5, math.nextafter(t - 0.5, 0.0), math.nextafter(t + 0.5, math.inf), t - 0.25, t + 0.25]
    inputs += [rng.uniform(0.0, 600.0) for _ in range(2000)]
    inputs += [-1.0, -0.5, 1e6, 199.8, 170.0, 120.0]
    rows = [[v, om.cli.nearest_pulse_width(v, table), decide(v)] for v in inputs]
    specials = []
    for label, v in (("inf", math.inf), ("-inf", -math.inf), ("nan", math.nan)):
        specials.append([label, om.cli.nearest_pulse_width(v, table), decide(v)])
    return {
        "function": "nearest_pulse_width",
        "oracle_source": "OM cli.nearest_pulse_width with cli.JPT_M7_PULSE_WIDTHS_NS; decision from OM safety.evaluate_emission (cfg=None -> JPT table)",
        "formula": "nearest = min(table, key=|t - v|) (first, i.e. lower, entry wins ties); decision: v in table -> exact; |nearest - v| <= 0.5 -> snap (warning); else refuse",
        "rust_rule": "Scan the table in ascending order keeping the first minimum (strict <). Shipping: exact/snap/refuse as here; NaN and infinities are refused before snapping (py-compat rows in `specials` show Python returns the first entry, 2 ns, for them).",
        "table_ns": list(table),
        "columns": ["pulse_width_ns", "nearest_ns", "decision"],
        "seed": 6001,
        "n_rows": len(rows),
        "inputs_f64_sha256": common.float_bits_sha256([r[0] for r in rows]),
        "rows": rows,
        "specials_columns": ["input (string: inf | -inf | nan)", "nearest_ns", "decision"],
        "specials": specials,
    }


def t0_distance(om) -> dict:
    enc = Encoder(om)
    c = enc.c
    rng = random.Random(7001)
    quads = [(0x8000, 0x8000, 0x8000, 0x8000), (0, 0, 0xFFFF, 0xFFFF), (0xFFFF, 0xFFFF, 0, 0), (0, 0xFFFF, 0xFFFF, 0),
             (0x8000, 0x8000, 0, 0), (0x8000, 0x8000, 0xFFFF, 0xFFFF), (0, 0, 0xFFFF, 0), (0, 0, 0, 0xFFFF),
             (0x8000, 0x8000, 31384, 31384), (0x8000, 0x8000, 32782, 32768), (31384, 31384, 34152, 31384)]
    for k in range(1, 2000, 7):
        quads.append((0x8000, 0x8000, 0x8000 + 3 * k % 30000, 0x8000 + 4 * k % 30000))  # 3-4-5 style
        quads.append((1000, 1000, 1000 + k * 3, 1000 + k * 4))  # exact integer hypot
        quads.append((0, 0, k, k))  # k * sqrt(2)
        quads.append((0, 0, k * 30, 1))  # sqrt(n^2 + 1), just above an integer
    for _ in range(10000):
        quads.append(tuple(rng.randrange(0, 0x10000) for _ in range(4)))
    rows = []
    for lx, ly, x, y in quads:
        c._last_x, c._last_y = lx, ly
        jump = enc.words(c.list_jump, x, y)
        c._last_x, c._last_y = lx, ly
        mark = enc.words(c.list_mark, x, y)
        if jump[1:] != mark[1:]:
            raise AssertionError("JumpTo and MarkTo encode differently")
        rows.append([lx, ly, x, y, jump[4]])
    return {
        "function": "distance_word",
        "oracle_source": "galvoplotter list_jump / list_mark (0x8001 / 0x8005 p4)",
        "formula": "dist = int(abs(complex(x, y) - complex(last_x, last_y))) = trunc(hypot(dx, dy)); min(dist, 0xFFFF)",
        "rust_rule": "let d = ((dx*dx + dy*dy) as f64).sqrt().trunc() (exact for |d| < 2^17), saturating at 65535. Inputs are integer galvo units; last = the last commanded point (0x8000, 0x8000 at controller start).",
        "columns": ["last_x", "last_y", "x", "y", "dist_word"],
        "seed": 7001,
        "n_rows": len(rows),
        "rows": rows,
    }


def run_t0(om, out: Path) -> None:
    tables = {
        "mm_to_galvo": t0_mm_to_galvo,
        "power_to_mark_current": t0_power,
        "frequency_to_period": t0_frequency,
        "speed_to_units": t0_speed,
        "delay_word": t0_delay,
        "nearest_pulse_width": t0_pulse_width,
        "distance_word": t0_distance,
    }
    for name, fn in tables.items():
        common.write_json(out / "t0_numeric" / f"{name}.json", fn(om))


# ---------------------------------------------------------------------------
# T1: LMC streams on the recording fake board
# ---------------------------------------------------------------------------
BOX10 = [[[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0], [-5.0, -5.0]]]
DOT = [[[0.0, 0.0], [0.05, 0.0]]]
LINE = [[[-2.5, 0.0], [2.5, 0.0]]]
LONG_JOB = [[[-50.0 + (i % 100), -50.0 + (i // 100) * 5.0], [-49.5 + (i % 100), -50.0 + (i // 100) * 5.0]]
            for i in range(1100)]


def params(om, power=50.0, frequency_khz=30.0, pulse_width_ns=200.0, speed=1000.0):
    return om.live.JobParams(text="", power=power, frequency_khz=frequency_khz, pulse_width_ns=pulse_width_ns,
                             size_mm=10.0, mark_speed=speed)


def load_business_card(om, inputs: Path):
    workspace = common.read_json(inputs / "workspace.json")
    layer_payloads = common.read_json(inputs / "layer_settings.json")["layers"]
    for obj in workspace["objects"]:
        if (obj.get("x", 0.0), obj.get("y", 0.0), obj.get("scale", 1.0), obj.get("rotation", 0.0)) != (0.0, 0.0, 1.0, 0.0):
            raise SystemExit("workspace.json has an un-baked object transform; the generator assumes identity")
    objects = [{"name": o["name"], "layer_id": o["layer_id"], "polylines": o["polylines"]} for o in workspace["objects"]]
    layer_specs = [om.layers.layer_from_dict(item) for item in layer_payloads]
    jobs = om.layers.group_by_layer(objects, layer_specs)
    return objects, layer_specs, jobs


def t1_scenarios(om, inputs: Path) -> dict:
    """name -> (description, inputs record, runner(cfg) -> result)."""
    live = om.live
    p50 = params(om)
    test_p = params(om, power=1.0)  # OM ui first_burn_params: 1 %, 30 kHz, 200 ns, 1000 mm/s
    frame_p = params(om, power=100.0)  # Frame panel defaults; lighting lists carry no power
    scenarios = {
        "init_abort": (
            "connect_controller (USB open + init_laser) then stop_controller (abort + disconnect), no job. "
            "init.json and abort.json are slices of this stream.",
            {"params": "50 %, 30 kHz, 200 ns, 1000 mm/s (unused: no list is built)"},
            lambda cfg: _init_abort(om, cfg, p50),
        ),
        "mark_box10": (
            "mark_polylines: 10 mm box centred on the field, 50 %, 30 kHz, 200 ns, 1000 mm/s",
            {"polylines": BOX10, "power": 50.0, "frequency_khz": 30.0, "pulse_width_ns": 200.0, "speed_mm_s": 1000.0},
            lambda cfg: live.mark_polylines(cfg, p50, BOX10, arm=True, confirm="ARM"),
        ),
        "frame_box10": (
            "frame_polylines (Frame Once, red light): the same 10 mm box",
            {"polylines": BOX10, "frame_light_speed_mm_s": 3000.0},
            lambda cfg: live.frame_polylines(cfg, frame_p, BOX10),
        ),
        "dot_test": (
            "the built-in Test Dot (ui /api/test/run kind=dot): 0.05 mm line at the field centre, 1 %",
            {"polylines": DOT, "power": 1.0, "frequency_khz": 30.0, "pulse_width_ns": 200.0, "speed_mm_s": 1000.0},
            lambda cfg: live.mark_polylines(cfg, test_p, DOT, arm=True, confirm="ARM"),
        ),
        "line_test": (
            "the built-in Test Line (ui /api/test/run kind=line): 5 mm line through the centre, 1 %",
            {"polylines": LINE, "power": 1.0, "frequency_khz": 30.0, "pulse_width_ns": 200.0, "speed_mm_s": 1000.0},
            lambda cfg: live.mark_polylines(cfg, test_p, LINE, arm=True, confirm="ARM"),
        ),
        "long_job": (
            "mark_polylines: 1100 separate 0.5 mm segments (9 list packets; ExecuteList after the 3rd), 50 %",
            {"polylines_formula": "[[-50 + i % 100, -50 + (i // 100) * 5], [-49.5 + i % 100, same y]] for i in 0..1099",
             "polylines": LONG_JOB, "power": 50.0, "frequency_khz": 30.0, "pulse_width_ns": 200.0, "speed_mm_s": 1000.0},
            lambda cfg: live.mark_polylines(cfg, p50, LONG_JOB, arm=True, confirm="ARM"),
        ),
    }
    _objects, _layer_specs, jobs = load_business_card(om, inputs)
    for step in om.ui.emission_steps(om.layers.emitting_jobs(jobs)):
        layer = step["job"].layer
        if step["raster"] is not None and not step["raster"].segments:
            continue
        layer_p = params(om, power=layer.power_percent, frequency_khz=layer.frequency_khz,
                         pulse_width_ns=layer.pulse_width_ns, speed=layer.speed_mm_s)
        polylines = [[list(pt) for pt in poly] for poly in step["polylines"]]
        for pass_index in range(step["passes"]):
            name = f"bc_mark_{layer.layer_id}_pass{pass_index + 1}"
            record = {
                "source": "inputs/workspace.json + inputs/layer_settings.json via OM ui.emission_steps (raster -> hatch segments)",
                "layer": layer.to_dict(),
                "paths": len(polylines),
                "points": sum(len(p) for p in polylines),
                "polylines": polylines,
            }
            if step["raster"] is not None:
                record["raster"] = {k: v for k, v in step["raster"].to_dict().items() if k != "segments"}
            scenarios[name] = (
                f"business card, layer {layer.layer_id!r} ({layer.operation}), pass {pass_index + 1}/{step['passes']}, "
                "exactly as /api/mark runs it (one hardware job per layer pass)",
                record,
                (lambda polys, lp, op: (lambda cfg: live.mark_polylines(cfg, lp, polys, arm=True, confirm="ARM", operation=op)))(
                    polylines, layer_p, layer.operation),
            )
    framing = [[list(pt) for pt in poly] for job in om.layers.framing_jobs(jobs) for poly in job.polylines]
    scenarios["bc_frame"] = (
        "business card Frame Once: every visible, non-disabled layer's outlines (ui framing_polylines), red light",
        {"source": "inputs/workspace.json + inputs/layer_settings.json via OM layers.framing_jobs",
         "paths": len(framing), "points": sum(len(p) for p in framing), "polylines": framing},
        lambda cfg: live.frame_polylines(cfg, frame_p, framing),
    )
    return scenarios


def _init_abort(om, cfg, job_params):
    controller = om.live.connect_controller(cfg, job_params)
    om.live.stop_controller(controller)
    return {"ok": True, "operation": "init_abort"}


def segment_events(events: list[dict]) -> dict:
    """Index ranges [start, end) of the init, job and abort parts of one stream."""
    seg: dict = {}
    singles = [(i, e["op"]) for i, e in enumerate(events) if e["kind"] == "single"]
    enable_z = [i for i, op in singles if op == "EnableZ"]
    if len(enable_z) >= 2:
        end = enable_z[1] + 1
        if end < len(events) and events[end]["kind"] == "read":
            end += 1
        seg["init"] = [0, end]
    stops = [i for i, op in singles if op == "StopExecute"]
    if stops:
        seg["abort"] = [stops[-1], len(events)]
    if "init" in seg and "abort" in seg:
        seg["job"] = [seg["init"][1], seg["abort"][0]]
    return seg


def run_t1(om, scenario: str, inputs: Path, out: Path) -> None:
    gc = om.gc
    live = om.live
    tmp = Path(tempfile.mkdtemp(prefix="olgold-t1-"))
    calibration = tmp / "calibration.json"
    calibration.write_text(json.dumps({"scale_correction": common.SCALE_CORRECTION}), encoding="utf-8")
    clock = VirtualClock()
    board = RecordingBoard(om.consts.single_command_lookup, clock=clock)
    controllers = []

    class CapturedController(gc.GalvoController):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, **kwargs)
            self.connection = board  # never USBConnection / MockConnection
            controllers.append(self)

    live.CALIBRATION_PATH = calibration
    live.GalvoController = CapturedController
    live.ensure_libusb_backend = lambda: {"available": True, "source": "golden-fake-board"}
    if hasattr(live, "WAIT_CLOCK"):
        live.WAIT_CLOCK = clock
    if hasattr(live, "BOARD_LOCK_PATH"):
        live.BOARD_LOCK_PATH = tmp / "board.lock"
    list_estimates: list[list[float]] = []
    if hasattr(live, "BoardWaits"):
        # Observe (never alter) OM v1's per-packet time estimates: keep each list's
        # packet estimates before ResetList clears them.
        original_reset_list = live.BoardWaits.reset_list

        def observing_reset_list(self):
            if self.packets:
                list_estimates.append(list(self.packets))
            original_reset_list(self)

        live.BoardWaits.reset_list = observing_reset_list

    scenarios = t1_scenarios(om, inputs)
    description, record, runner = scenarios[scenario]
    cfg = markcfg(om)
    result, error = None, None
    try:
        result = jsonable(runner(cfg))
    except Exception as exc:  # recorded: an oracle that refuses is itself a golden
        error = error_record(exc)
        error["traceback_tail"] = traceback.format_exc().strip().splitlines()[-3:]
    clean = usb_guard.assert_clean()

    packets_dir = out / "packets"
    packets_dir.mkdir(parents=True, exist_ok=True)
    for digest, data in board.packets.items():
        (packets_dir / f"{digest}.hex").write_text(common.packet_to_hex_lines(data), encoding="ascii")

    body = {
        "scenario": scenario,
        "description": description,
        "inputs": jsonable(record),
        "field_size_mm": live.effective_field_size(cfg),
        "scale_correction": common.SCALE_CORRECTION,
        "profile": "profiles/openmopa-machine.ini (from the OM tree)",
        "fake_board": "every read answers (0, 0, 0, 0x0020): READY, never BUSY",
        "result": result,
        "error": error,
        "counts": {
            "events": len(board.events),
            "singles": sum(1 for e in board.events if e["kind"] == "single"),
            "reads": sum(1 for e in board.events if e["kind"] == "read"),
            "list_packets": sum(1 for e in board.events if e["kind"] == "list"),
            "distinct_packets": len(board.packets),
            "list_commands": sum(e["n_cmds"] for e in board.events if e["kind"] == "list"),
        },
        "segments": segment_events(board.events),
        "events": board.events,
        "usb_isolation": clean,
    }
    for waits in (getattr(c, "_openlaser_waits", None) for c in controllers):
        if waits is not None and waits.packets:
            list_estimates.append(list(waits.packets))
    if list_estimates:
        body["informational"] = {
            "packet_estimates_s": list_estimates,
            "note": "OM v1 BoardWaits estimated execution seconds per list packet, one array per list (job list(s), then the "
                    "abort's dummy packet); input to the B1 wait budget. Not a byte golden.",
        }
    polylines = body["inputs"].get("polylines")
    if isinstance(polylines, list) and sum(len(p) for p in polylines) > 50:
        # Large inputs live in a shared file (identical for v0 and v1), keyed by scenario.
        common.write_json(out / "t1_lmc" / "inputs" / f"{scenario}.json", {
            "scenario": scenario,
            "units": "mm, design coordinates (centre origin, +y up), exactly what the OM function received",
            "polylines": polylines,
        })
        del body["inputs"]["polylines"]
        body["inputs"]["polylines_file"] = f"t1_lmc/inputs/{scenario}.json"
        # sha256 of the compact JSON of the polylines array (no spaces), independent of file formatting/meta.
        body["inputs"]["polylines_compact_sha256"] = common.sha256_bytes(common._compact(polylines).encode("utf-8"))
    common.write_json(out / "t1_lmc" / f"{scenario}.json", body)


# ---------------------------------------------------------------------------
# T2: G-code
# ---------------------------------------------------------------------------
def _circle(cx, cy, r, n=73):
    return [[cx + math.cos(2 * math.pi * i / (n - 1)) * r, cy + math.sin(2 * math.pi * i / (n - 1)) * r] for i in range(n)]


def _square(cx, cy, s):
    h = s / 2.0
    return [[cx - h, cy - h], [cx + h, cy - h], [cx + h, cy + h], [cx - h, cy + h], [cx - h, cy - h]]


def _spiral(turns=4, n=200, r=30.0):
    return [[r * (i / n) * math.cos(2 * math.pi * turns * i / n), r * (i / n) * math.sin(2 * math.pi * turns * i / n)] for i in range(n + 1)]


def _layer(layer_id, operation, **kw):
    base = {"layer_id": layer_id, "name": kw.pop("name", layer_id.title()), "operation": operation, "power_percent": 50.0,
            "frequency_khz": 30.0, "pulse_width_ns": 200.0, "speed_mm_s": 20.0, "passes": 1, "visible": True,
            "output": True, "color": "#26d0a8", "air_assist": False, "raster_pitch_mm": 0.1,
            "raster_angle_deg": 0.0, "raster_angle_step_deg": 90.0}
    base.update(kw)
    return base


def t2_scenarios(inputs: Path) -> dict:
    triangle = [[-30.0, 20.0], [-15.0, 20.0], [-22.5, 33.3333333], [-30.0, 20.0]]
    zig = [[20.0, -20.0], [25.125, -15.0625], [30.0, -20.0], [35.5555, -15.4444]]
    donut_outer, donut_inner = _square(-20.0, -20.0, 30.0), _square(-20.0, -20.0, 10.0)
    star = []
    for i in range(11):
        r = 15.0 if i % 2 == 0 else 6.0
        a = math.pi / 2 + i * math.pi / 5
        star.append([25.0 + r * math.cos(a), 25.0 + r * math.sin(a)])
    star[-1] = list(star[0])
    scenarios = {
        "vector": {
            "about": "one vector_engrave layer: square, triangle, open zig-zag, 73-point circle; 25 %, 10 mm/s",
            "layers": [_layer("vec", "vector_engrave", power_percent=25.0, speed_mm_s=10.0)],
            "objects": [{"layer_id": "vec", "polylines": [_square(0.0, 0.0, 20.0), triangle, zig, _circle(20.0, 25.0, 12.0)]}],
        },
        "multipass": {
            "about": "vector_cut with passes=3 (each pass re-emits every path); 80 %, 5 mm/s",
            "layers": [_layer("cut", "vector_cut", power_percent=80.0, speed_mm_s=5.0, passes=3)],
            "objects": [{"layer_id": "cut", "polylines": [_square(0.0, 0.0, 20.0), _circle(20.0, 25.0, 12.0)]}],
        },
        "air_assist": {
            "about": "vector_cut with air_assist (M8/M9 on laser profiles that support it); 60 %, 20 mm/s",
            "layers": [_layer("air", "vector_cut", power_percent=60.0, speed_mm_s=20.0, air_assist=True)],
            "objects": [{"layer_id": "air", "polylines": [_square(0.0, 0.0, 20.0), triangle]}],
        },
        "raster_crosshatch": {
            "about": "raster_engrave passes=2, pitch 0.5, angle 15, step 90 (cross-hatch) over a donut and a circle; the open zig-zag is skipped",
            "layers": [_layer("fill", "raster_engrave", power_percent=40.0, speed_mm_s=100.0, passes=2,
                              raster_pitch_mm=0.5, raster_angle_deg=15.0, raster_angle_step_deg=90.0)],
            "objects": [{"layer_id": "fill", "polylines": [donut_outer, donut_inner, _circle(20.0, 25.0, 6.0), zig]}],
        },
        "plotter_drawing": {
            "about": "pen-plotter style drawing: open spiral, zig-zag, star outline on one vector layer; 100 %, 50 mm/s",
            "layers": [_layer("pen", "vector_engrave", power_percent=100.0, speed_mm_s=50.0)],
            "objects": [{"layer_id": "pen", "polylines": [_spiral(), zig, star]}],
        },
        "mixed_layers": {
            "about": "skip rules: frame_only, disabled, output=false and an empty raster layer emit nothing; a hidden layer with output on IS emitted",
            "layers": [
                _layer("frame", "frame_only"),
                _layer("off", "disabled"),
                _layer("muted", "vector_engrave", output=False),
                _layer("hidden", "vector_engrave", visible=False, power_percent=33.0, speed_mm_s=15.0),
                _layer("shown", "vector_engrave", power_percent=66.0, speed_mm_s=30.0),
                _layer("emptyfill", "raster_engrave", raster_pitch_mm=1.0),
            ],
            "objects": [
                {"layer_id": "frame", "polylines": [_square(0.0, 0.0, 60.0)]},
                {"layer_id": "off", "polylines": [triangle]},
                {"layer_id": "muted", "polylines": [_circle(20.0, 25.0, 12.0)]},
                {"layer_id": "hidden", "polylines": [zig]},
                {"layer_id": "shown", "polylines": [_square(0.0, 0.0, 20.0)]},
                {"layer_id": "emptyfill", "polylines": [zig]},
                # No orphan object here: v0 drops it silently, v1 refuses it (B8). T2 stays oracle-independent.
            ],
        },
        "speed_power_edges": {
            "about": "speed above every machine max (capped), power ties: 12.25 % -> S122.5 -> 122 (ties-to-even), 0.05 % -> S0, 0.15 % -> S2",
            "layers": [
                _layer("fast", "vector_engrave", power_percent=12.25, speed_mm_s=1000.0),
                _layer("faint", "vector_engrave", power_percent=0.05, speed_mm_s=0.123456),
                _layer("faint2", "vector_engrave", power_percent=0.15, speed_mm_s=149.9999),
            ],
            "objects": [
                {"layer_id": "fast", "polylines": [_square(0.0, 0.0, 20.0)]},
                {"layer_id": "faint", "polylines": [[[0.0004, -0.0004], [10.00049, 0.00051]]]},
                {"layer_id": "faint2", "polylines": [[[-1.23456789, 2.34567891], [3.4567891, -4.5678912]]]},
            ],
        },
        "business_card": {
            "about": "the real business-card workspace with the saved layer settings (fiber values on gantry machines: speeds are capped)",
            "inputs_ref": ["inputs/workspace.json", "inputs/layer_settings.json"],
        },
        "refuse_out_of_bed": {
            "about": "a point outside every gantry bed is refused (never clamped)",
            "layers": [_layer("vec", "vector_engrave")],
            "objects": [{"layer_id": "vec", "polylines": [[[0.0, 0.0], [1000.0, 1000.0]]]}],
        },
        "refuse_power": {
            "about": "power above 100 % is refused",
            "layers": [_layer("vec", "vector_engrave", power_percent=101.0)],
            "objects": [{"layer_id": "vec", "polylines": [_square(0.0, 0.0, 20.0)]}],
        },
        "refuse_nothing_to_output": {
            "about": "only frame_only geometry: nothing to export",
            "layers": [_layer("frame", "frame_only")],
            "objects": [{"layer_id": "frame", "polylines": [_square(0.0, 0.0, 20.0)]}],
        },
    }
    return scenarios


def _number_format_inputs() -> list[float]:
    rng = random.Random(8001)
    values: list[float] = []
    values += [rng.uniform(-1000.0, 1000.0) for _ in range(30000)]
    values += [round(rng.uniform(0.0, 600.0), 4) for _ in range(20000)]
    values += [rng.randrange(-600000, 600000) / 1000.0 + 0.0005 for _ in range(20000)]
    values += [rng.randrange(-2 ** 20, 2 ** 20) / float(2 ** rng.choice((4, 5, 6, 7, 8))) for _ in range(10000)]
    values += [rng.uniform(-0.0006, 0.0006) for _ in range(10000)]
    values += [float(rng.randrange(-10 ** 6, 10 ** 6)) * rng.choice((1.0, 0.5, 0.25, 60.0)) for _ in range(10000)]
    values += [0.0, -0.0, 0.0005, -0.0005, 0.0004999, -0.0004999, 0.0015, 0.0025, 0.0625, -0.0625, 0.1875,
               1e-12, -1e-12, 123456789.0005, 1e15, -1e15, 59.17, 236.68, 0.1 + 0.2]
    return values


def run_t2(om, inputs: Path, out: Path) -> None:
    scen = t2_scenarios(inputs)
    cases = []
    gdir = out / "t2_gcode"
    for machine in om.machines.BUILTIN_MACHINES:
        for name, spec in scen.items():
            if name == "business_card":
                _objects, _specs, jobs = load_business_card(om, inputs)
            else:
                specs = [om.layers.layer_from_dict(item) for item in spec["layers"]]
                jobs = om.layers.group_by_layer(spec["objects"], specs)
            case = {"machine_id": machine.machine_id, "scenario": name}
            try:
                program = om.gcode.compile_gcode(machine, jobs)
            except Exception as exc:
                case["ok"] = False
                case["error"] = error_record(exc)
                cases.append(case)
                continue
            lines = program.content.split("\n")
            if not lines[2].startswith("; Generated: "):
                raise AssertionError("G-code line 3 is not the timestamp")
            lines[2] = "; Generated: <MASKED>"
            text = "\n".join(lines)
            rel = f"{machine.machine_id}/{name}.gcode"
            path = gdir / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(text.encode("utf-8"))
            case.update({
                "ok": True,
                "file": rel,
                "sha256": common.sha256_bytes(text.encode("utf-8")),
                "bytes": len(text.encode("utf-8")),
                "lines": text.count("\n"),
                "path_count": program.path_count,
                "point_count": program.point_count,
                "estimated_distance_mm": program.estimated_distance_mm,
            })
            cases.append(case)
    common.write_json(gdir / "index.json", {
        "function": "compile_gcode",
        "oracle_source": "OM mopa_luiz/gcode.py compile_gcode over layers.group_by_layer(objects, [layer_from_dict(...)])",
        "masking": "line 3 ('; Generated: <UTC ISO-8601>') is replaced by '; Generated: <MASKED>' before hashing and storing",
        "byte_rules": "lines joined with '\\n', file ends with '\\n' (the program's last element is ''), no '\\r'; UTF-8 ('·' and '₂' appear in comments)",
        "machines": [jsonable(m.to_dict()) for m in om.machines.BUILTIN_MACHINES],
        "scenarios": jsonable(scen),
        "cases": cases,
    })
    values = _number_format_inputs()
    rows = [[v, om.gcode._number(v)] for v in values]
    common.write_json(gdir / "number_format.json", {
        "function": "gcode._number",
        "oracle_source": "OM mopa_luiz/gcode.py _number",
        "formula": "t = f'{v:.3f}'.rstrip('0').rstrip('.'); '0' if t in ('', '-0') else t  ({:.3f} is correctly rounded, ties-to-even on the exact binary value)",
        "rust_rule": "format with 3 decimals using exact decimal expansion and ties-to-even (check Rust's {:.3} against every row), then strip trailing zeros and the dot; '-0' -> '0'",
        "seed": 8001,
        "columns": ["value", "text"],
        "n_rows": len(rows),
        "inputs_f64_sha256": common.float_bits_sha256(values),
        "rows": rows,
    })


# ---------------------------------------------------------------------------
# T3: hatch
# ---------------------------------------------------------------------------
def _sq(size):
    s = size / 2.0
    return [[-s, -s], [s, -s], [s, s], [-s, s], [-s, -s]]


def t3_cases(om, inputs: Path) -> list[dict]:
    cases = [
        {"name": "square10_pitch1", "polylines": [_sq(10.0)], "params": {"angle_deg": 0.0, "pitch_mm": 1.0, "passes": 1}},
        {"name": "square10_angle90", "polylines": [_sq(10.0)], "params": {"angle_deg": 90.0, "pitch_mm": 1.0, "passes": 1}},
        {"name": "square10_crosshatch", "polylines": [_sq(10.0)], "params": {"angle_deg": 0.0, "pitch_mm": 1.0, "passes": 2, "angle_step_deg": 90.0}},
        {"name": "open_path_skipped", "polylines": [[[0.0, 0.0], [5.0, 0.0]]], "params": {"angle_deg": 0.0, "pitch_mm": 1.0, "passes": 1}},
        {"name": "donut", "polylines": [_sq(20.0), _sq(6.0)], "params": {"angle_deg": 0.0, "pitch_mm": 1.0, "passes": 1}},
        {"name": "passes_zero", "polylines": [_sq(10.0)], "params": {"angle_deg": 0.0, "pitch_mm": 1.0, "passes": 0}},
        {"name": "pitch_zero_refused", "polylines": [_sq(10.0)], "params": {"angle_deg": 0.0, "pitch_mm": 0.0}},
        {"name": "pitch_negative_refused", "polylines": [_sq(10.0)], "params": {"angle_deg": 0.0, "pitch_mm": -0.1}},
        {"name": "closure_tolerance_closed", "polylines": [[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.04]]],
         "params": {"angle_deg": 0.0, "pitch_mm": 0.7, "passes": 1}},
        {"name": "closure_tolerance_open", "polylines": [[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.06]]],
         "params": {"angle_deg": 0.0, "pitch_mm": 0.7, "passes": 1}},
        {"name": "two_point_closed_rejected", "polylines": [[[1.0, 1.0], [1.0, 1.0]]], "params": {"pitch_mm": 1.0}},
        {"name": "triangle_angle33_pitch0.37", "polylines": [[[-8.0, -5.0], [9.0, -4.0], [1.0, 11.0], [-8.0, -5.0]]],
         "params": {"angle_deg": 33.0, "pitch_mm": 0.37, "passes": 3, "angle_step_deg": 60.0}},
        {"name": "negative_angle", "polylines": [_sq(12.0)], "params": {"angle_deg": -45.0, "pitch_mm": 0.8, "passes": 1}},
        {"name": "vertex_on_scanline", "polylines": [[[0.0, 0.0], [4.0, 1.0], [8.0, 0.0], [8.0, 3.0], [0.0, 3.0], [0.0, 0.0]]],
         "params": {"angle_deg": 0.0, "pitch_mm": 0.5, "passes": 1}},
    ]
    rng = random.Random(9001)
    for k in range(30):
        n = rng.randrange(5, 13)
        cx, cy = rng.uniform(-40.0, 40.0), rng.uniform(-40.0, 40.0)
        r1, r2 = rng.uniform(4.0, 15.0), rng.uniform(1.0, 4.0)
        poly = []
        for i in range(2 * n):
            r = r1 if i % 2 == 0 else r2
            a = 2 * math.pi * i / (2 * n) + rng.uniform(-0.1, 0.1)
            poly.append([round(cx + r * math.cos(a), 4), round(cy + r * math.sin(a), 4)])
        poly.append(list(poly[0]))
        polys = [poly]
        if k % 3 == 0:
            polys.append([[round(cx + 0.5 * x, 4), round(cy + 0.5 * y, 4)] for x, y in _sq(2.0)])  # a hole
        cases.append({
            "name": f"random_star_{k:02d}",
            "polylines": polys,
            "params": {"angle_deg": round(rng.uniform(-180.0, 180.0), 3), "pitch_mm": round(rng.uniform(0.15, 1.0), 3),
                       "passes": rng.randrange(1, 4), "angle_step_deg": round(rng.choice((90.0, 45.0, 0.0, rng.uniform(1.0, 179.0))), 3)},
            "seed": 9001,
        })
    _objects, layer_specs, jobs = load_business_card(om, inputs)
    raster_layer = next(spec for spec in layer_specs if spec.operation == "raster_engrave")
    raster_polys = next(job.polylines for job in jobs if job.layer.layer_id == raster_layer.layer_id)
    cases.append({
        "name": "business_card_raster_saved_settings",
        "polylines_ref": f"inputs/workspace.json objects with layer_id {raster_layer.layer_id!r}, in file order",
        "polylines_count": len(raster_polys),
        "params": {"angle_deg": raster_layer.raster_angle_deg, "pitch_mm": raster_layer.raster_pitch_mm,
                   "passes": max(1, raster_layer.passes), "angle_step_deg": raster_layer.raster_angle_step_deg},
        "_polylines": raster_polys,
    })
    cases.append({
        "name": "business_card_raster_crosshatch",
        "polylines_ref": f"inputs/workspace.json objects with layer_id {raster_layer.layer_id!r}, in file order",
        "polylines_count": len(raster_polys),
        "params": {"angle_deg": 0.0, "pitch_mm": 0.1, "passes": 2, "angle_step_deg": 90.0},
        "_polylines": raster_polys,
    })
    return cases


def run_t3(om, inputs: Path, out: Path) -> None:
    results = []
    for case in t3_cases(om, inputs):
        polylines = case.pop("_polylines", None) or case["polylines"]
        try:
            result = om.raster.hatch_polylines(polylines, **case["params"])
            case["output"] = jsonable(result.to_dict())
        except Exception as exc:
            case["error"] = error_record(exc)
        results.append(case)
    common.write_json(out / "t3_hatch" / "hatch.json", {
        "function": "hatch_polylines",
        "oracle_source": "OM mopa_luiz/raster.py hatch_polylines (defaults: angle 0, pitch 0.1, passes 1, step 90, closure_tol 0.05)",
        "rust_rule": "segment count and order exact; coordinates within 1e-9 mm; keep Python's operation order (y += pitch accumulation, rotation formulas, half-open [lo, hi) crossing test, x1 - x0 > 1e-6 pair filter)",
        "cases": results,
    })


# ---------------------------------------------------------------------------
# T4: importers
# ---------------------------------------------------------------------------
def _dedent(text: str) -> str:
    return textwrap.dedent(text)


IN_TEST_DXF = {
    "test_line_and_lwpolyline": ("parse_dxf", _dedent("""\
        0
        SECTION
        2
        ENTITIES
        0
        LINE
        10
        0
        20
        0
        11
        10
        21
        0
        0
        LWPOLYLINE
        70
        1
        10
        0
        20
        0
        10
        4
        20
        0
        10
        4
        20
        3
        0
        ENDSEC
    """)),
    "test_old_polyline_vertex_seqend": ("parse_dxf", _dedent("""\
        0
        POLYLINE
        70
        1
        0
        VERTEX
        10
        0
        20
        0
        0
        VERTEX
        10
        5
        20
        0
        0
        VERTEX
        10
        5
        20
        5
        0
        SEQEND
    """)),
    "test_import_geometry_preserves_dxf_millimeters": ("import_geometry", "0\nLINE\n10\n0\n20\n0\n11\n10\n21\n0\n"),
    "test_import_geometry_converts_dxf_inches_to_mm": ("import_geometry", _dedent("""\
        0
        SECTION
        2
        HEADER
        9
        $INSUNITS
        70
        1
        0
        ENDSEC
        0
        LINE
        10
        0
        20
        0
        11
        1
        21
        0
    """)),
    "test_lwpolyline_bulge_is_flattened": ("parse_dxf", _dedent("""\
        0
        LWPOLYLINE
        10
        1
        20
        0
        42
        0.41421356237
        10
        0
        20
        1
    """)),
    "test_ellipse_entity": ("parse_dxf", _dedent("""\
        0
        ELLIPSE
        10
        0
        20
        0
        11
        5
        21
        0
        40
        0.5
        41
        0
        42
        6.283185307179586
    """)),
}


def _dxf(*entities: str, units: int | None = None) -> str:
    header = "" if units is None else f"0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n{units}\n0\nENDSEC\n"
    return header + "0\nSECTION\n2\nENTITIES\n" + "".join(entities) + "0\nENDSEC\n0\nEOF\n"


EXTRA_DXF = {
    "circle_arc_cm": _dxf("0\nCIRCLE\n10\n1.5\n20\n-2\n40\n3\n", "0\nARC\n10\n0\n20\n0\n40\n2\n50\n300\n51\n45\n", units=5),
    "spline_fit_and_control": _dxf("0\nSPLINE\n10\n0\n20\n0\n10\n1\n20\n2\n10\n3\n20\n1\n11\n0\n21\n0\n11\n2\n21\n2\n11\n4\n21\n0\n",
                                   "0\nSPLINE\n10\n0\n20\n5\n10\n2\n20\n6\n10\n4\n20\n5\n"),
    "lwpolyline_closed_negative_bulge_meters": _dxf("0\nLWPOLYLINE\n70\n1\n10\n0\n20\n0\n42\n-0.5\n10\n0.01\n20\n0\n10\n0.01\n20\n0.01\n42\n1\n10\n0\n20\n0.01\n", units=6),
    "polyline_vertex_bulge_feet": _dxf("0\nPOLYLINE\n70\n0\n0\nVERTEX\n10\n0\n20\n0\n42\n0.25\n0\nVERTEX\n10\n0.1\n20\n0\n0\nVERTEX\n10\n0.1\n20\n0.1\n0\nSEQEND\n", units=2),
    "ellipse_partial_ratio": _dxf("0\nELLIPSE\n10\n10\n20\n10\n11\n0\n21\n8\n40\n0.25\n41\n1\n42\n4\n"),
    "unknown_units_code_99": _dxf("0\nLINE\n10\n-3\n20\n4\n11\n7\n21\n-1\n", units=99),
    "crlf_and_blank_lines": "0\r\nSECTION\r\n2\r\nENTITIES\r\n\r\n0\r\nLINE\r\n10\r\n1\r\n20\r\n1\r\n11\r\n2\r\n21\r\n3\r\n0\r\nENDSEC\r\n",
}

SVG_CASES = {
    "svg_shapes": '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50"><rect x="10" y="5" width="30" height="20"/>'
                  '<circle cx="70" cy="25" r="12"/><polygon points="50,40 60,45 55,48"/>'
                  '<line x1="0" y1="0" x2="100" y2="50"/><ellipse cx="20" cy="40" rx="8" ry="4"/></svg>',
    "svg_path_relative": '<svg xmlns="http://www.w3.org/2000/svg"><path d="M10 10 L20 10 l0 10 H5 v-5 Z m30 0 l5 5 h3 V30 z"/></svg>',
}


def _stl_binary() -> str:
    tris = [((0, 0, 1), (0, 0, 0), (10, 0, 0), (0, 10, 0)), ((0, 0, 1), (10, 0, 0), (10, 10, 0), (0, 10, 0))]
    raw = b"golden".ljust(80, b"\0") + struct.pack("<I", len(tris))
    for n, a, b, c in tris:
        raw += struct.pack("<12fH", *n, *a, *b, *c, 0)
    return "data:application/octet-stream;base64," + base64.b64encode(raw).decode("ascii")


STL_ASCII = ("solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 4 0 0\nvertex 0 3 0\nendloop\nendfacet\n"
             "facet normal 0 0 1\nouter loop\nvertex 4 0 0\nvertex 4 3 0\nvertex 0 3 0\nendloop\nendfacet\nendsolid t\n")


def run_t4(om, inputs: Path, out: Path) -> None:
    imp = om.importers
    cases = []

    def run(name, kind, function, filename, payload, source, must_match=True):
        case = {"name": name, "kind": kind, "function": function, "filename": filename, "source": source,
                "rust_must_match": must_match}
        try:
            if function == "parse_dxf":
                polys = imp.parse_dxf(payload)
                case["output"] = {"polylines": jsonable(polys), "polyline_count": len(polys),
                                  "point_count": sum(len(p) for p in polys)}
            else:
                case["output"] = jsonable(imp.import_geometry(filename, payload))
        except Exception as exc:
            case["error"] = error_record(exc)
        cases.append(case)

    for name, (function, text) in IN_TEST_DXF.items():
        run(name, "dxf", function, "part.dxf" if "inches" not in name else "inch-part.dxf", text,
            {"inline_text": text, "from": f"OM tests/test_importers.py::{name}"})
    for name, text in EXTRA_DXF.items():
        for function in ("parse_dxf", "import_geometry"):
            run(f"{name}.{function}", "dxf", function, f"{name}.dxf", text, {"inline_text": text})
    for file_name in ("square.dxf", "businessCard.dxf"):
        path = inputs / "dxf" / file_name
        text = path.read_bytes().decode("utf-8", errors="ignore")
        for function in ("import_geometry", "parse_dxf"):
            run(f"{file_name}.{function}", "dxf", function, file_name, text,
                {"file": f"inputs/dxf/{file_name}", "sha256": common.sha256_file(path),
                 "read_as": "bytes decoded as UTF-8 with errors='ignore' (what /api/import receives as text)"})
    for name, text in SVG_CASES.items():
        run(name, "svg", "import_geometry", f"{name}.svg", text, {"inline_text": text}, must_match=False)
    run("stl_ascii", "stl", "import_geometry", "t.stl", STL_ASCII, {"inline_text": STL_ASCII}, must_match=False)
    binary = _stl_binary()
    run("stl_binary", "stl", "import_geometry", "b.stl", binary, {"data_url": binary}, must_match=False)
    run("unsupported_extension", "other", "import_geometry", "x.pdf", "hello", {"inline_text": "hello"})
    run("dxf_without_geometry", "dxf", "import_geometry", "empty.dxf", "0\nEOF\n", {"inline_text": "0\nEOF\n"})

    big = [c for c in cases if c["name"].startswith("businessCard.dxf")]
    small = [c for c in cases if not c["name"].startswith("businessCard.dxf")]
    common.write_json(out / "t4_import" / "import.json", {
        "function": "import_geometry / parse_dxf",
        "oracle_source": "OM mopa_luiz/importers.py",
        "rust_rule": "DXF: polyline and point counts exact, coordinates within 1e-9 (trig may differ by an ulp). SVG and STL rows are informational (rust_must_match=false): the plan imports SVG at true scale and drops STL.",
        "cases": small,
    })
    common.write_json(out / "t4_import" / "businessCard_dxf.json", {
        "function": "import_geometry / parse_dxf",
        "oracle_source": "OM mopa_luiz/importers.py",
        "cases": big,
    })


# ---------------------------------------------------------------------------
def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--om-tree", type=Path, required=True)
    ap.add_argument("--inputs", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("command", choices=["t0", "t1", "t2", "t3", "t4", "list-t1"])
    ap.add_argument("scenario", nargs="?")
    args = ap.parse_args()
    om = load_om(args.om_tree)
    if args.command == "t0":
        run_t0(om, args.out)
    elif args.command == "t1":
        run_t1(om, args.scenario, args.inputs, args.out)
    elif args.command == "t2":
        run_t2(om, args.inputs, args.out)
    elif args.command == "t3":
        run_t3(om, args.inputs, args.out)
    elif args.command == "t4":
        run_t4(om, args.inputs, args.out)
    elif args.command == "list-t1":
        print(json.dumps(list(t1_scenarios(om, args.inputs))))
    clean = usb_guard.assert_clean()
    if args.command != "list-t1":
        common.write_json(args.out / "usb_isolation" / f"{args.command}{'-' + args.scenario if args.scenario else ''}.json",
                          {"self_test_blocked": SELF_TEST, **clean})


if __name__ == "__main__":
    main()
