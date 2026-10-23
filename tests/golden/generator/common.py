"""Shared helpers for the golden generator: deterministic JSON, hashing, metadata.

Pure standard library. Safe to import after usb_guard.install().
"""

from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
from typing import Any

GENERATOR_VERSION = "1.0.0"
SCHEMA = "openlaser-golden/1"
GENERATOR_PATH = "tests/golden/generator"

# The calibration the real machine runs with (OM calibration.json, 2026-10).
SCALE_CORRECTION = 1.1834
NOMINAL_FIELD_MM = 200.0


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(Path(path).read_bytes())


def _scalar(value: Any) -> str:
    if isinstance(value, float) and not math.isfinite(value):
        raise ValueError(f"non-finite float {value!r} cannot go into a golden JSON file")
    return json.dumps(value, ensure_ascii=False, allow_nan=False)


def _is_scalar(value: Any) -> bool:
    return value is None or isinstance(value, (bool, int, float, str))


def _is_flat(value: Any, depth: int = 3) -> bool:
    """A scalar, or a list nested at most `depth` deep whose leaves are scalars."""
    if _is_scalar(value):
        return True
    if isinstance(value, list) and depth > 0:
        return all(_is_flat(item, depth - 1) for item in value)
    return False


def _compact(value: Any) -> str:
    if _is_scalar(value):
        return _scalar(value)
    if isinstance(value, list):
        return "[" + ",".join(_compact(item) for item in value) + "]"
    if isinstance(value, dict):
        return "{" + ",".join(f"{_scalar(str(k))}:{_compact(v)}" for k, v in value.items()) + "}"
    raise TypeError(f"cannot serialise {type(value).__name__}")


def dumps(value: Any, indent: int = 0) -> str:
    """Deterministic, diff-friendly JSON.

    * dicts are multi-line in insertion order, except small all-scalar dicts,
      which go on one line (events, table headers);
    * lists of scalars, and short lists of flat lists, go on one line;
    * long lists of rows put one row per line (T0 tables, polylines).
    Floats are written with Python's shortest round-trip repr.
    """
    pad = " " * indent
    inner = " " * (indent + 1)
    if _is_scalar(value):
        return _scalar(value)
    if isinstance(value, dict):
        if not value:
            return "{}"
        if all(_is_flat(v, 1) for v in value.values()):
            one_line = _compact(value)
            if len(one_line) <= 160:
                return one_line
        items = [f"{inner}{_scalar(str(k))}: {dumps(v, indent + 1)}" for k, v in value.items()]
        return "{\n" + ",\n".join(items) + "\n" + pad + "}"
    if isinstance(value, list):
        if not value:
            return "[]"
        if all(_is_scalar(item) for item in value):
            return _compact(value)
        if all(_is_flat(item) for item in value):
            one_line = _compact(value)
            if len(one_line) <= 160:
                return one_line
            return "[\n" + ",\n".join(inner + _compact(item) for item in value) + "\n" + pad + "]"
        return "[\n" + ",\n".join(inner + dumps(item, indent + 1) for item in value) + "\n" + pad + "]"
    raise TypeError(f"cannot serialise {type(value).__name__}")


def write_json(path: Path, value: Any) -> bytes:
    data = (dumps(value) + "\n").encode("utf-8")
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return data


def read_json(path: Path) -> Any:
    return json.loads(Path(path).read_text(encoding="utf-8"))


def packet_to_hex_lines(packet: bytes) -> str:
    """A 3072-byte LMC list packet as 256 lines of 24 hex digits (one 12-byte slot per line)."""
    if len(packet) != 0xC00:
        raise ValueError(f"list packet must be 3072 bytes, got {len(packet)}")
    return "".join(packet[i:i + 12].hex() + "\n" for i in range(0, len(packet), 12))


def hex_lines_to_packet(text: str) -> bytes:
    return bytes.fromhex("".join(text.split()))


def float_bits_sha256(values: list[float]) -> str:
    """sha256 over the IEEE-754 little-endian bit patterns, so a reader can prove it parsed every float exactly."""
    import struct

    return hashlib.sha256(b"".join(struct.pack("<d", float(v)) for v in values)).hexdigest()
