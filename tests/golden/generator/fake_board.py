"""A deterministic recording stand-in for the JCZ/BJJCZ LMC board. No USB.

It implements the four methods galvoplotter's GalvoController calls on its
connection (is_open, open, close, write, read) and records every transfer in
order. Every status read answers (0, 0, 0, READY): the board always accepts
another list packet and is never BUSY, so galvoplotter's and OpenLaser's
wait loops each poll exactly once and the recorded stream does not depend on
timing. No timestamps are recorded.

Event kinds (the T1 fixture format):
  {"kind": "open"}                              connect: device opened
  {"kind": "single", "op": NAME, "hex": 24 hex} one 12-byte single command
  {"kind": "read", "status": INT}               the 8-byte reply that followed it
  {"kind": "list", "sha256": HEX, "n_cmds": N}  one 3072-byte list packet;
                                                N = slots before the trailing 0x8002 fill
  {"kind": "close"}                             disconnect (galvoplotter does a USB reset here)
"""

from __future__ import annotations

import struct
import threading

READY = 0x20
BUSY = 0x04
LIST_PACKET_BYTES = 0xC00
END_OF_LIST_SLOT = bytes([0x02, 0x80] + [0] * 10)


class VirtualClock:
    """Replaces OM v1's WAIT_CLOCK: time only moves when a wait pauses or a transfer happens."""

    def __init__(self) -> None:
        self._t = 0.0
        self._lock = threading.Lock()

    def now(self) -> float:
        with self._lock:
            return self._t

    def advance(self, seconds: float) -> None:
        with self._lock:
            self._t += max(0.0, seconds)

    def pause(self, event: threading.Event, seconds: float) -> bool:
        self.advance(seconds)
        return event.is_set()


class RecordingBoard:
    def __init__(self, single_names: dict, clock: VirtualClock | None = None, status: int = READY) -> None:
        self._names = single_names
        self.clock = clock
        self.status = status
        self.devices: dict[int, bool] = {}
        self.events: list[dict] = []
        self.packets: dict[str, bytes] = {}
        self._lock = threading.RLock()

    # -- galvoplotter connection interface ---------------------------------
    def is_open(self, index: int = 0) -> bool:
        return index in self.devices

    def open(self, index: int = 0) -> int:
        with self._lock:
            self.devices[index] = True
            self.events.append({"kind": "open"})
        return index

    def close(self, index: int = 0) -> None:
        with self._lock:
            self.devices.pop(index, None)
            self.events.append({"kind": "close"})

    def write(self, index: int = 0, packet=None) -> None:
        data = bytes(packet)
        if len(data) not in (12, LIST_PACKET_BYTES):
            raise AssertionError(f"galvoplotter wrote {len(data)} bytes; only 12 or 3072 are legal")
        with self._lock:
            if self.clock is not None:
                self.clock.advance(0.0002)
            if len(data) == 12:
                opcode = struct.unpack_from("<H", data, 0)[0]
                self.events.append({"kind": "single", "op": self._names.get(opcode, "?"), "hex": data.hex()})
                return
            from hashlib import sha256

            digest = sha256(data).hexdigest()
            self.packets[digest] = data
            n_cmds = 0
            for slot in range(256):
                if data[slot * 12:(slot + 1) * 12] != END_OF_LIST_SLOT:
                    n_cmds = slot + 1
            self.events.append({"kind": "list", "sha256": digest, "n_cmds": n_cmds})

    def read(self, index: int = 0) -> bytes:
        with self._lock:
            if self.clock is not None:
                self.clock.advance(0.0002)
            self.events.append({"kind": "read", "status": self.status})
        return struct.pack("<4H", 0, 0, 0, self.status)
