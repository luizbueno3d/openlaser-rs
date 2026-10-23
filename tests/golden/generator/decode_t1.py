"""Pretty-print a T1 LMC golden stream. Standard library only (no pyusb, no galvoplotter).

  python3 tests/golden/generator/decode_t1.py tests/golden/t1_lmc/v0/mark_box10.json
  python3 tests/golden/generator/decode_t1.py tests/golden/t1_lmc/v1/bc_frame.json --brief
"""

from __future__ import annotations

import json
import struct
import sys
from pathlib import Path

# galvoplotter 0.2.0 galvo/consts.py list opcodes (MIT, MeerK40t); names as galvoplotter spells them.
LIST_OPS = {
    0x8001: "listJumpTo", 0x8002: "listEndOfList", 0x8003: "listLaserOnPoint", 0x8004: "listDelayTime",
    0x8005: "listMarkTo", 0x8006: "listJumpSpeed", 0x8007: "listLaserOnDelay", 0x8008: "listLaserOffDelay",
    0x800A: "listMarkFreq", 0x800B: "listMarkPowerRatio", 0x800C: "listMarkSpeed", 0x800D: "listJumpDelay",
    0x800F: "listPolygonDelay", 0x8011: "listWritePort", 0x8012: "listMarkCurrent", 0x8013: "listMarkFreq2",
    0x801A: "listFlyEnable", 0x801B: "listQSwitchPeriod", 0x801C: "listDirectLaserSwitch", 0x801D: "listFlyDelay",
    0x801E: "listSetCo2FPK", 0x801F: "listFlyWaitInput", 0x8021: "listFiberOpenMO", 0x8022: "listWaitForInput",
    0x8023: "listChangeMarkCount", 0x8024: "listSetWeldPowerWave", 0x8025: "listEnableWeldPowerWave",
    0x8026: "listFiberYLPMPulseWidth", 0x8028: "listFlyEncoderCount", 0x8029: "listSetDaZWord",
    0x8050: "listJptSetParam", 0x8051: "listReadyMark",
}


def load_packet(golden_root: Path, digest: str) -> bytes:
    text = (golden_root / "t1_lmc" / "packets" / f"{digest}.hex").read_text(encoding="ascii")
    return bytes.fromhex("".join(text.split()))


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    path = Path(sys.argv[1]).resolve()
    brief = "--brief" in sys.argv[2:]
    golden_root = next(p for p in path.parents if (p / "t1_lmc").is_dir())
    doc = json.loads(path.read_text(encoding="utf-8"))
    print(f"# {doc.get('scenario')} [{doc['meta']['oracle']} {next(iter(doc['meta']['om_git_sha'].values()))[:7]}]: "
          f"{doc.get('description', '')}")
    for index, event in enumerate(doc["events"]):
        kind = event["kind"]
        if kind == "single":
            words = struct.unpack("<6H", bytes.fromhex(event["hex"]))
            args = ", ".join(f"0x{w:04x}" for w in words[1:])
            print(f"{index:5d} SINGLE {event['hex']}  {event['op']}({args})")
        elif kind == "read":
            print(f"{index:5d}   <- read 8 B, status 0x{event['status']:04x}")
        elif kind == "list":
            data = load_packet(golden_root, event["sha256"])
            print(f"{index:5d} LIST 3072 B sha256 {event['sha256'][:16]}: {event['n_cmds']} commands, "
                  f"{256 - event['n_cmds']} x 0x8002 fill")
            if not brief:
                for slot in range(event["n_cmds"]):
                    words = struct.unpack_from("<6H", data, slot * 12)
                    name = LIST_OPS.get(words[0], f"0x{words[0]:04x}")
                    print(f"            {' '.join(f'{w:04x}' for w in words)}  {name}{tuple(words[1:])}")
        else:
            print(f"{index:5d} {kind.upper()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
