#!/usr/bin/env python3
"""Minimal GRBL 1.1 emulator over raw TCP, for testing senders without hardware.

- Replies to realtime bytes: '?' status, '!' hold, '~' resume, 0x18 reset, 0x85 jog cancel.
- Accepts $ commands ($$, $I, $G, $#, $H, $X, $J=...) and G-code lines.
- Simulates motion time (scaled by --speedup) with a 15-block planner buffer, so
  pause/stop latency and flow control are exercised.
- Logs every received byte stream and event with timestamps to --log.
"""
import argparse
import math
import re
import socket
import threading
import time
from collections import deque

SETTINGS = {
    0: 10, 1: 25, 2: 0, 3: 0, 4: 0, 5: 0, 6: 0, 10: 1, 11: 0.010, 12: 0.002,
    13: 0, 20: 0, 21: 0, 22: 1, 23: 3, 24: 25.0, 25: 500.0, 26: 250, 27: 1.0,
    30: 1000, 31: 0, 32: 1, 100: 80.0, 101: 80.0, 102: 80.0, 110: 6000.0,
    111: 6000.0, 112: 500.0, 120: 1000.0, 121: 1000.0, 122: 100.0, 130: 400.0,
    131: 400.0, 132: 50.0,
}

WORD = re.compile(r"([A-Z])\s*(-?\d*\.?\d+)")


class Grbl:
    def __init__(self, conn, log, speedup):
        self.conn = conn
        self.log = log
        self.speedup = speedup
        self.lock = threading.RLock()
        self.pos = [0.0, 0.0, 0.0]
        self.state = "Idle"
        self.feed = 1000.0
        self.spindle = 0.0
        self.laser_on = False
        self.modal_motion = "G0"
        self.absolute = True
        self.queue = deque()
        self.hold = False
        self.reset_flag = False
        self.alive = True
        self.t0 = time.monotonic()
        self.lines = 0
        self.burn_mm = 0.0
        self.travel_mm = 0.0
        threading.Thread(target=self.executor, daemon=True).start()

    def ts(self):
        return f"{time.monotonic() - self.t0:9.3f}"

    def note(self, msg):
        self.log.write(f"{self.ts()} {msg}\n")
        self.log.flush()

    def send(self, text):
        try:
            self.conn.sendall(text.encode())
        except OSError:
            self.alive = False

    def status(self):
        with self.lock:
            x, y, z = self.pos
            buf = 15 - len(self.queue)
            return (f"<{self.state}|MPos:{x:.3f},{y:.3f},{z:.3f}|Bf:{buf},128|"
                    f"FS:{self.feed if self.state == 'Run' else 0:.0f},{self.spindle:.0f}|WCO:0.000,0.000,0.000>\r\n")

    def banner(self):
        self.send("\r\nGrbl 1.1h ['$' for help]\r\n")

    def executor(self):
        while self.alive:
            with self.lock:
                if self.hold or not self.queue:
                    if not self.queue and self.state in ("Run", "Jog"):
                        self.state = "Idle"
                    item = None
                else:
                    item = self.queue[0]
                    self.state = "Jog" if item.get("jog") else "Run"
            if item is None:
                time.sleep(0.005)
                continue
            dist = math.dist(self.pos[:2], item["target"][:2])
            rate = max(item["feed"], 1.0) / 60.0  # mm/s
            dur = dist / rate / self.speedup
            end = time.monotonic() + dur
            while time.monotonic() < end:
                if self.reset_flag or not self.alive:
                    break
                if self.hold:
                    time.sleep(0.005)
                    end += 0.005
                    continue
                time.sleep(0.002)
            with self.lock:
                if self.reset_flag:
                    continue
                if self.queue and self.queue[0] is item:
                    self.queue.popleft()
                    if item["burn"]:
                        self.burn_mm += dist
                    else:
                        self.travel_mm += dist
                    self.pos = list(item["target"])

    def enqueue(self, target, feed, burn, jog=False):
        while True:
            with self.lock:
                if self.reset_flag:
                    return
                if len(self.queue) < 15:
                    self.queue.append({"target": target, "feed": feed, "burn": burn, "jog": jog})
                    return
            time.sleep(0.002)

    def handle_line(self, line):
        self.lines += 1
        raw = line
        line = line.strip().upper()
        if not line:
            self.send("ok\r\n")
            return
        if line.startswith("$"):
            self.note(f"CMD {raw.strip()}")
            if line == "$$":
                for k, v in SETTINGS.items():
                    self.send(f"${k}={v}\r\n")
            elif line == "$I":
                self.send("[VER:1.1h.20190825:fake-grbl]\r\n[OPT:VML,15,128]\r\n")
            elif line == "$G":
                g = "G1" if self.modal_motion == "G1" else "G0"
                m = "M4" if self.laser_on else "M5"
                self.send(f"[GC:{g} G54 G17 G21 G90 G94 {m} M9 T0 F{self.feed:.0f} S{self.spindle:.0f}]\r\n")
            elif line == "$#":
                for w in ("G54", "G55", "G56", "G57", "G58", "G59", "G28", "G30", "G92"):
                    self.send(f"[{w}:0.000,0.000,0.000]\r\n")
                self.send("[TLO:0.000]\r\n[PRB:0.000,0.000,0.000:0]\r\n")
            elif line == "$H":
                with self.lock:
                    self.state = "Home"
                time.sleep(0.3 / self.speedup)
                with self.lock:
                    self.pos = [0.0, 0.0, 0.0]
                    self.state = "Idle"
            elif line.startswith("$J="):
                words = dict(WORD.findall(line[3:]))
                rel = "G91" in line
                target = list(self.pos)
                for i, ax in enumerate("XYZ"):
                    if ax in words:
                        target[i] = (target[i] if rel else 0) + float(words[ax])
                self.enqueue(target, float(words.get("F", 1000)), False, jog=True)
            self.send("ok\r\n")
            return
        self.note(f"GC  {raw.strip()}")
        words = WORD.findall(line)
        target = list(self.pos)
        moved = False
        for letter, value in words:
            v = float(value)
            if letter == "G":
                if v in (0, 1):
                    self.modal_motion = f"G{int(v)}"
                elif v == 90:
                    self.absolute = True
                elif v == 91:
                    self.absolute = False
            elif letter == "M":
                if v in (3, 4):
                    self.laser_on = True
                elif v == 5:
                    self.laser_on = False
            elif letter == "S":
                self.spindle = v
            elif letter == "F":
                self.feed = v
            elif letter in "XYZ":
                i = "XYZ".index(letter)
                target[i] = v if self.absolute else target[i] + v
                moved = True
        if moved:
            burn = self.modal_motion == "G1" and self.laser_on and self.spindle > 0
            feed = self.feed if self.modal_motion == "G1" else 6000.0
            self.enqueue(target, feed, burn)
        self.send("ok\r\n")

    def realtime(self, b):
        if b == ord("?"):
            self.send(self.status())
        elif b == ord("!"):
            self.note("RT  feed-hold")
            with self.lock:
                self.hold = True
                if self.queue:
                    self.state = "Hold:0"
        elif b == ord("~"):
            self.note("RT  cycle-start")
            with self.lock:
                self.hold = False
        elif b == 0x18:
            self.note(f"RT  soft-reset (queue had {len(self.queue)} blocks)")
            with self.lock:
                self.reset_flag = True
                self.queue.clear()
                self.hold = False
                self.state = "Idle"
                self.laser_on = False
            time.sleep(0.01)
            with self.lock:
                self.reset_flag = False
            self.banner()
        elif b == 0x85:
            self.note("RT  jog-cancel")
            with self.lock:
                self.queue = deque(q for q in self.queue if not q.get("jog"))
        else:
            self.note(f"RT  0x{b:02x}")


def serve(port, logpath, speedup):
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", port))
    srv.listen(1)
    with open(logpath, "a") as log:
        log.write(f"listening on 127.0.0.1:{port} speedup={speedup}\n")
        log.flush()
        while True:
            conn, addr = srv.accept()
            conn.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            g = Grbl(conn, log, speedup)
            g.note(f"CONNECT {addr}")
            g.banner()
            pending = deque()

            # Lines are processed on their own thread (it may block on a full
            # planner, like real GRBL withholding "ok"), while realtime bytes
            # are handled immediately on the receive thread.
            def process():
                while g.alive:
                    if pending:
                        line = pending.popleft()
                        if line is None:
                            continue
                        g.handle_line(line)
                    else:
                        time.sleep(0.001)

            threading.Thread(target=process, daemon=True).start()
            buf = b""
            try:
                while g.alive:
                    data = conn.recv(4096)
                    if not data:
                        break
                    for b in data:
                        if b in (ord("?"), ord("!"), ord("~"), 0x18) or b >= 0x80:
                            if b == 0x18:
                                pending.clear()
                            g.realtime(b)
                        elif b in (10, 13):
                            if buf:
                                pending.append(buf.decode(errors="replace"))
                                buf = b""
                        else:
                            buf += bytes([b])
            except OSError:
                pass
            g.alive = False
            g.note(f"DISCONNECT lines={g.lines} burn_mm={g.burn_mm:.1f} travel_mm={g.travel_mm:.1f}")
            conn.close()


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=2323)
    ap.add_argument("--log", default="fake_grbl.log")
    ap.add_argument("--speedup", type=float, default=5.0)
    a = ap.parse_args()
    serve(a.port, a.log, a.speedup)
