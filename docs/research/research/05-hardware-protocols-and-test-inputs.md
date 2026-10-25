# 05: Hardware protocols and test inputs (spec for the Rust LMC driver)

Date: 2026-10-22. Scope: everything a Rust OpenLaser must keep from the Python prototype at
`~/AI/OpenLaser/OpenMopa` (called **OM** below) and from the installed galvoplotter 0.2.0 at
`~/AI/OpenLaser/OpenMopa/.venv/lib/python3.10/site-packages/galvo/` (called **GP** below).

This is the reference spec for the Rust JCZ/BJJCZ LMC driver, the GRBL back end, the data model and the golden tests.

## Evidence legend

| Tag | Meaning |
| --- | --- |
| **[CODE]** | Read directly in source; file:line given. |
| **[SIM]** | Seen by running OM's own code path against a recording fake board. No USB was opened: pyusb's `usb.core.find` and GP's `USBConnection` were replaced by functions that raise. The harness is in Appendix A. |
| **[HW-LOG]** | A real-hardware result recorded in OM commit messages, README or user notes. It was not re-measured here because the laser is disconnected. |
| **[WEB]** | Primary web source, URL given. |
| **[UNVERIFIED]** | An inference, or something not checkable without hardware. Every such item is repeated in section 7. |

What was run: the OM unit suite, using `unittest` because pytest is **not installed** in the venv. Result: 119 tests, all pass. Also run: the Appendix A capture harness, in-process with a fake board, and pure-function calculations. Nothing opened the laser's USB device. The OM `/api/health` test enumerates USB through pyusb but opens nothing.

---

## 0. Findings that change the Rust plan

1. **Long jobs are probably cut short by a hard 3 s wait cap. [SIM, not yet hardware-confirmed]**
   - OM replaces GP's unbounded `wait_idle`, `wait_ready` and `wait_finished` with 3-second-capped versions (`OM/mopa_luiz/live.py:455-482`). When the cap expires the code just carries on.
   - Simulation: a fake board stayed BUSY for 20 s after `ExecuteList`. OM sent `Fiber_SetMo(0)` and `WritePort(0)` at **+3.0 s**, closing the MO and dropping the enable bit. It sent `StopExecute` and `ResetList` at **+6.0 s**, through `shutdown()` → `abort()`.
   - So any job, or Frame Once, still running more than about 3 s after its last list packet is queued loses emission at 3 s and is aborted at 6 s.
   - The same cap lets `_list_end` push a packet into a full board buffer after 3 s. That is the failure mode that caused the reset storm.
   - The Rust driver must never proceed on timeout. A wait must be either satisfied or explicitly stopped, with a progress-aware watchdog.
2. **Every OM job is torn down with a full `abort()`, even after success. [CODE+SIM]** Each job runs in a fresh spawned process. It connects, runs `init_laser` (including `Reset 0x0040`), marks, then `shutdown()`. `shutdown()` always sends `StopExecute`, `Fiber_SetMo(0)`, `ResetList`, a dummy end-of-list packet, `ExecuteList`, `Fiber_SetMo(0)` and `WritePort(0)`, then a USB `device.reset()` on close. This happens per layer pass (`OM/mopa_luiz/ui.py:2859-2871`). Rust should hold one session, initialise once, and abort only on stop.
3. **The first jump of a job can be silently skipped. [CODE+SIM]**
   - GP assumes the galvo starts at `0x8000,0x8000`, and `goto` skips a move to the "current" point (`GP/controller.py:156-157, 517-519`).
   - The built-in **Test Dot** (`[[0,0],[0.05,0]]`, `OM/mopa_luiz/ui.py:2948-2951`) therefore emits **no JumpTo**. It goes straight to `listMarkTo(32782,32768)` from wherever the mirrors really are.
   - Rust must always emit an explicit first jump.
4. **The Mark confirmation dialog shows the wrong parameters. [CODE]**
   - The dialog and Job Summary show the *Frame* panel values (`power`, `frequency`, `pulseWidth`, `markSpeed`; Frame Power defaults to 100 %) (`OM/mopa_luiz/ui.py:809-814, 2210-2222, 1540-1556`).
   - Marking actually uses each layer's own values (`ui.py:2839-2847`).
   - The client always sends `arm: true` (`ui.py:2151`), so the typed "ARM" text is the only real gate, and it stays in the field after a job.
5. **Hidden layers burn without being framed. [CODE]** `emits_when_marking()` ignores `visible` (`layers.py:46-51`), but `framing_jobs()` drops invisible layers (`layers.py:195-196`). A hidden layer with Output on is marked but never traced by the red light.
6. **Python `round()` rounds half to even, and the golden data depends on it. [SIM]** `power_to_mark_current(30)` gives 1228; round-half-away-from-zero gives 1229. 70 % gives 2866 versus 2867. Rust must use `f64::round_ties_even` (stable since Rust 1.77) wherever Python calls `round()`, and `trunc` wherever it calls `int()`.
7. **No galvo correction table is loaded. [CODE+SIM]** `init_laser` sends `WriteCorTable(0)`, a blank table, because `cor_file=None` (`GP/controller.py:789, 853-856, 875-876`). The calibrated `scale_correction = 1.1834` may partly compensate for running without the machine's EZCAD `.cor` file. Hardware item H5.
8. **galvoplotter is MIT (© 2023 MeerK40t, author Tatarize), so porting to an MIT Rust crate is allowed with the notice retained.** Its ancestor, balor, is copyleft (GPLv3 file; SSPL shown on GitLab). Do not copy from balor (section 6).
9. **Test count observed: 119 (not 105).** pytest is not installed in the venv; the suite runs with `python -m unittest discover`.

---

## 1. BJJCZ LMC USB protocol, as OpenLaser actually uses it

### 1.1 Device, transport, OS plumbing

| Item | Value | Source |
| --- | --- | --- |
| USB ID | VID `0x9588`, PID `0x9899`; Windows shows it as "USBLMCV4" | `GP/usb_connection.py:13-14`; `OM/mopa_luiz/cli.py:26-27`; `OM/README.md:138` |
| Configuration / interface | `set_configuration()` (default config), then `get_active_configuration()[(0,0)]` (interface 0, alt 0); kernel-driver detach where supported; claim interface. If the claim fails, release and claim again. | `GP/usb_connection.py:117-144, 208-235` |
| OUT endpoint | `0x02`, bulk | `GP/usb_connection.py:16` |
| IN endpoint | `0x88`, bulk; every read is 8 bytes | `GP/usb_connection.py:17, 279-283` |
| Write/read timeout | **100 ms** (`self.timeout = 100`); the mock uses 500 | `GP/usb_connection.py:36`; `GP/mock_connection.py:26` |
| Allowed write sizes | exactly **12 bytes** (one single command) or **3072 = 0xC00 bytes** (one list packet), enforced by assert | `GP/usb_connection.py:255-256` |
| Write/read error handling | On `USBError`, if `attempt <= 3`: `close()`, which does **`device.reset()`** (USB port reset), then `open()` and retry. That is up to 5 tries per transfer. After that it raises `ConnectionError`, and `controller.send()` returns `(-1,-1,-1,-1)`. | `GP/usb_connection.py:182-191, 237-252, 254-297`; `GP/controller.py:383-396` |
| Connect | `connect_if_needed()` loops `open()` + `init_laser()` with up to 10 failures, `sleep(0.3)` twice per failure, then `_disable_connect` | `GP/controller.py:337-381` |
| OM connect wrapper | Hard 12 s deadline on a thread. On timeout or error: `abort_connect()`, `_sending=False`, `shutdown()`. The board is diagnosed as wedged if the USB log contains "claim: Success" but the connect never finished. Hints for Windows (Zadig/WinUSB) and Linux (udev). | `OM/mopa_luiz/live.py:39, 385-452` |
| libusb | pyusb 1.3.1 tries the system libusb first, then the bundled `libusb-package` (1.0.30.0) | `OM/mopa_luiz/usb_backend.py:20-59` |
| macOS | Nothing extra; bundled libusb works. [HW-LOG] direct control tested on macOS. | `OM/README.md:123, 504-506` |
| Windows | Must swap the BJJCZ/EZCAD driver for **WinUSB with Zadig**. EZCAD then can't use the laser until the driver is restored. Untested on hardware. | `OM/README.md:133-146` |
| Linux | udev rule `SUBSYSTEM=="usb", ATTRS{idVendor}=="9588", ATTRS{idProduct}=="9899", MODE="0666", TAG+="uaccess"`. Untested on hardware. | `OM/scripts/99-openlaser.rules` |

**For Rust (`rusb` or `nusb`):** claim interface 0, bulk-out `0x02`, bulk-in `0x88`. Do **not** copy the "reset the USB device on any error" retry. That retry is the reset-storm amplifier (section 1.10). Map every transfer error to a typed error, stop the job, and leave reconnecting to the user.

### 1.2 Wire format

- **Every command is 12 bytes: six little-endian u16 words** `[opcode, p1, p2, p3, p4, p5]`. `struct.pack("<6H", ...)` (`GP/controller.py:1147-1149, 1153-1155`). There is no header, length or checksum. Python's `struct.pack('H')` raises on values outside 0..65535, so every parameter must be clamped or validated first.
- **Single ("realtime") commands** have opcodes `0x00xx`. One command is written as a 12-byte bulk transfer. If `read=True`, which is the default for every single command except `WriteCorLine`, the code then reads **8 bytes = four u16 LE words** `(b0, b1, b2, b3)` (`GP/controller.py:383-396, 1152-1156`; `GP/consts.py:182-187`).
- **List commands** have opcodes `0x80xx`. They are collected into a **3072-byte packet = 256 slots × 12 bytes**. Unused slots are pre-filled with `0x8002` (EndOfList) as `02 80 00 00 00 00 00 00 00 00 00 00` (`GP/controller.py:22-23, 1135-1138`). Packets are written with **no read**.
- **Status word = `b3`**, the fourth reply word of `GetVersion` (0x0007), which is used as the status poll. Bits: **BUSY = 0x04, READY = 0x20, AXIS = 0x40** (`GP/controller.py:18-20, 398-400, 676-714`). [WEB] MeerK40t `balormk/controller.py` uses the same constants (https://github.com/meerk40t/meerk40t/blob/main/meerk40t/balormk/controller.py).
  - READY means the board can accept another list packet (GP README: "can accept additional packets").
  - BUSY means it is executing ("no longer doing work").
  - Both meanings come from GP docs and were not measured: [UNVERIFIED] H2.
- **GP quirk:** a failed read returns `(-1,-1,-1,-1)`, and in Python `-1 & 0x20` is truthy. So a dead link reads as "READY and BUSY", and `wait_idle` spins until it times out (`GP/controller.py:395-396, 676-682`). Rust must model a failed status read as `Err`, never as status bits.

### 1.3 Opcodes OpenLaser actually sends

#### 1.3.1 Single commands (12-byte write, then an 8-byte read unless noted)

| Opcode | GP name | p1..p5 as sent by OM | When | Source |
| --- | --- | --- | --- | --- |
| `0x0009` | GetSerialNo | 0 | init, step 1 | `GP/controller.py:782, 1449-1450` |
| `0x0007` | GetVersion | 0 | init, step 2; **every status poll** (`status()` = reply word 3) | `GP/controller.py:784, 398-400` |
| `0x0040` | Reset | 0 | init, on **every** connect (OM connects once per job/pass) | `GP/controller.py:787, 1583-1584` |
| `0x0015` | WriteCorTable | p1 = 0 (blank, no table) | init | `GP/controller.py:789, 853-856, 875-876, 1478-1479` |
| `0x0010` | WriteCorLine | dx, dy, non_first; **no read** | only with a .cor file, which **OM never loads** | `GP/controller.py:921-927, 1469-1470` |
| `0x0004` | EnableLaser | 0 | init | `GP/controller.py:791, 1437-1438` |
| `0x0016` | SetControlMode | 0 | init | `GP/controller.py:793` |
| `0x001B` | SetLaserMode | 1 | init | `GP/controller.py:795` |
| `0x0017` | SetDelayMode | 1 | init | `GP/controller.py:797` |
| `0x001C` | SetTiming | 1 | init | `GP/controller.py:799` |
| `0x001D` | SetStandby | 2000 (`0x07D0`), 20 (`0x0014`) | init | `GP/controller.py:801, 56-57` |
| `0x001A` | SetFirstPulseKiller | 200 (`0x00C8`) | init | `GP/controller.py:803, 53` |
| `0x001E` | SetPwmHalfPeriod | 125 | init | `GP/controller.py:805, 55` |
| `0x0006` | SetPwmPulseWidth | 125 | init | `GP/controller.py:807, 54` |
| `0x0033` | Fiber_SetMo | **0 = close, 1 = open** | init (0); entering marking (1); leaving marking (0); abort (0, twice) | `GP/controller.py:810, 434, 456, 754, 765, 1555-1563` |
| `0x002E` | SetFpkParam2 | `0x0FFB`, 1, 409 (`0x0199`), 100 (`0x0064`) | init | `GP/controller.py:812-814, 63-66` |
| `0x0032` | SetFlyRes | 0, 99, 1000, 25 | init | `GP/controller.py:816-821, 67-70` |
| `0x003A` | EnableZ | 0 | init (sent **twice**, before and after the analog write) | `GP/controller.py:823-827` |
| `0x0022` | WriteAnalogPort1 | `0x07FF` | init | `GP/controller.py:825` |
| `0x0012` | ResetList | 0 | entering marking or lighting from idle; abort | `GP/controller.py:452, 498, 755` |
| `0x0021` | WritePort | p1 = port bit mask | before marking (`0x0001`); after the list (`0x0000`); abort | `GP/controller.py:454, 436, 767, 1516-1517` |
| `0x0019` | SetEndOfList | 0 | **after every list packet** | `GP/controller.py:1127` |
| `0x0005` | ExecuteList | 0 | after the **3rd** packet, or at end of job if fewer than 3 were sent | `GP/controller.py:1131-1133, 427-429` |
| `0x0041` | GetMarkTime | **3** (GP: "always called with data 3"; meaning of reply unknown) | after each list completes | `GP/controller.py:437, 1610-1615` |
| `0x001F` | StopExecute | 0 | `abort()`, which runs on every shutdown | `GP/controller.py:752, 1510-1511` |
| `0x000A` | GetListStatus | 0 | only `/api/board` (diagnostics) | `OM/mopa_luiz/live.py:485-503` |

#### 1.3.2 List commands (12-byte slots in a 3072-byte packet)

| Opcode | GP name | Parameters as encoded | Units / notes | Source |
| --- | --- | --- | --- | --- |
| `0x8051` | listReadyMark | none | First command of every new list (marking and lighting) | `GP/controller.py:469, 499, 1422-1428` |
| `0x8004` | listDelayTime | p1 = `abs(t)` | **10 µs units**. Marking start uses `int(delay_open_mo * 100)` = 800, i.e. 8 ms after MO open | `GP/controller.py:470-471, 1178-1185` |
| `0x8011` | listWritePort | p1 = port mask | bit 0 = "laser" pin, bit 8 = red light (section 1.6) | `GP/controller.py:1263-1269` |
| `0x8006` | listJumpSpeed | p1 = `min(int(mm_s * galvos_per_mm / 1000), 0xFFFF)` | galvo units per ms | `GP/controller.py:997-1002, 1086-1096, 1197-1200` |
| `0x800C` | listMarkSpeed | same conversion | galvo units per ms | `GP/controller.py:1004-1009, 1236-1245` |
| `0x8026` | listFiberYLPMPulseWidth | p1 = pulse width in **ns**, raw (e.g. 200 → `0x00C8`) after snapping to the table | Raw ns is [UNVERIFIED] (H6) | `GP/controller.py:1076-1080, 1386-1393`; `OM/mopa_luiz/live.py:395` |
| `0x8012` | listMarkCurrent | p1 = `round_half_even(power% * 0xFFF / 100)` (0..4095) | fiber "power" | `GP/controller.py:1035-1049, 1109-1114` |
| `0x801B` | listQSwitchPeriod | p1 = `round_half_even(20000 / f_kHz) & 0xFFFF` | Period in 50 ns ticks (20 MHz). 30 kHz → 667 | `GP/controller.py:1067-1072, 1098-1107` |
| `0x8007` | listLaserOnDelay | p1 = `abs(d)`, p2 = `0x0000 if d > 0 else 0x8000` | µs per GP docstring; **d == 0 encodes as p2 = 0x8000** ("negative zero") | `GP/controller.py:1202-1208` |
| `0x8008` | listLaserOffDelay | same encoding | µs | `GP/controller.py:1210-1216` |
| `0x800F` | listPolygonDelay | same encoding, raw | µs per GP. **MeerK40t sends `delay/10`**, which conflicts: [UNVERIFIED] H9 | `GP/controller.py:1255-1261`; [WEB] meerk40t balormk |
| `0x800D` | listJumpDelay | same encoding; always **8** in OM (`delay_jump_short`) | GP says µs. 8 µs is suspiciously short: [UNVERIFIED] H9 | `GP/controller.py:76-77, 517-533, 1247-1253` |
| `0x8001` | listJumpTo | x, y, angle = 0, dist, 0 | `dist = min(trunc(hypot(x - lx, y - ly)), 0xFFFF)` in galvo units from the **last commanded** point | `GP/controller.py:1162-1170` |
| `0x8005` | listMarkTo | x, y, 0, dist, 0 | same | `GP/controller.py:1187-1195` |
| `0x8002` | listEndOfList | none | Written explicitly at job end, **and** used as the packet filler | `GP/controller.py:425, 1172-1173` |

Sent by GP but **not by OM's fiber paths**:
- `0x8003 listLaserOnPoint` (dwell, 10 µs units, ≤ 60000 per command)
- CO2-only `0x800A listMarkFreq` = `round(10000/kHz)`
- `0x800B listMarkPowerRatio` = `round(200*power/kHz)`
- `0x801E listSetCo2FPK` = `round(2000/kHz)`, written twice (p1 = p2)

GP's own CO2 golden values: (20 %, 10 kHz) → MarkFreq `0x03E8`, FPK `0x00C8`, PowerRatio `0x0190`. (30 %, 20) → `0x01F4`, `0x0064`, `0x012C`. (40 %, 30) → `0x014D`, `0x0043`, `0x010B`. (50 %, 80) → `0x007D`, `0x0019`, `0x007D` (`GP/../test/test_co2.py:16-117`). Warning: those GP tests build a non-mock controller, so they would open real USB. **Never run `site-packages/test/*.py`.**

The full opcode constant table, including unused opcodes, is in Appendix B.

### 1.4 Init sequence, exact bytes [SIM]

`connect_if_needed()` → `open()` → `init_laser()` (`GP/controller.py:780-830`) produced this stream, each line followed by an 8-byte read:

```
09 00 00 00 00 00 00 00 00 00 00 00   GetSerialNo
07 00 00 00 00 00 00 00 00 00 00 00   GetVersion
40 00 00 00 00 00 00 00 00 00 00 00   Reset
15 00 00 00 00 00 00 00 00 00 00 00   WriteCorTable(0)        <- blank table, no .cor loaded
04 00 00 00 00 00 00 00 00 00 00 00   EnableLaser
16 00 00 00 00 00 00 00 00 00 00 00   SetControlMode(0)
1b 00 01 00 00 00 00 00 00 00 00 00   SetLaserMode(1)
17 00 01 00 00 00 00 00 00 00 00 00   SetDelayMode(1)
1c 00 01 00 00 00 00 00 00 00 00 00   SetTiming(1)
1d 00 d0 07 14 00 00 00 00 00 00 00   SetStandby(2000, 20)
1a 00 c8 00 00 00 00 00 00 00 00 00   SetFirstPulseKiller(200)
1e 00 7d 00 00 00 00 00 00 00 00 00   SetPwmHalfPeriod(125)
06 00 7d 00 00 00 00 00 00 00 00 00   SetPwmPulseWidth(125)
33 00 00 00 00 00 00 00 00 00 00 00   Fiber_SetMo(0)  (close)
2e 00 fb 0f 01 00 99 01 64 00 00 00   SetFpkParam2(0x0FFB, 1, 409, 100)
32 00 00 00 63 00 e8 03 19 00 00 00   SetFlyRes(0, 99, 1000, 25)
3a 00 00 00 00 00 00 00 00 00 00 00   EnableZ
22 00 ff 07 00 00 00 00 00 00 00 00   WriteAnalogPort1(0x07FF)
3a 00 00 00 00 00 00 00 00 00 00 00   EnableZ
(sleep 50 ms)
```

The meaning of standby, FPK2, fly-res and analog port 1 for a JPT M7 is not documented by GP; it is EZCAD-derived [UNVERIFIED]. The Rust driver should send the same values until a hardware or EZCAD capture says otherwise.

### 1.5 List lifecycle: build, flush, execute

From `GP/controller.py:1120-1150, 422-439`:

1. **Build.** `_list_write(cmd, v1..v5)` packs 12 bytes into `_active_list`, a copy of the 0x8002-filled 3072-byte buffer.
   - Before writing, `if _active_index >= 0xC00: _list_end()` flushes a full packet. This check runs **outside** `_list_build_lock`.
   - The packing itself runs under the lock.
2. **Flush (`_list_end`, entirely under `_list_build_lock`).** Runs only if there is an active list with at least one command:
   - `wait_ready()`, then `while paused: sleep(0.3)`
   - write the 3072 bytes (no read)
   - **`SetEndOfList(0)`** single command (with read)
   - `_number_of_list_packets += 1`
   - if `packets > 2 and not _list_executing`, then **`ExecuteList`** and `_list_executing = True`

   So the board receives 3 packets before execution starts. Later packets are flow-controlled by READY.
3. **Finish (`initial_configuration`, at the end of the `with marking()/lighting()` block):**
   - `list_end_of_list()` (explicit `0x8002`)
   - `_list_end()`
   - if fewer than 3 packets were sent, `ExecuteList` now
   - **`wait_idle()`**, meaning wait while BUSY
   - fiber: `Fiber_SetMo(0)`; `port_off(laser_pin)`; `WritePort`
   - `GetMarkTime(3)`; state becomes `initial`
4. **OM job wrapper:**
   - `controller.submit(job)`
   - `wait_for_machine_idle()`, which is `wait_for_spooler_send()` (condition-variable wait, **no timeout**) followed by `wait_finished()` (READY and not BUSY)
   - `finally: controller.shutdown()`, which runs **abort, then join spooler, then disconnect (USB reset)** (`OM/mopa_luiz/live.py:729-751`; `GP/controller.py:196-210, 728-748`)

Golden 10 mm box mark (50 %, 30 kHz, 200 ns, 1000 mm/s, field 236.68 mm) [SIM]. One packet holds 17 real commands and 239 filler slots:

```
51 80 00 00 00 00 00 00 00 00 00 00   listReadyMark
04 80 20 03 00 00 00 00 00 00 00 00   listDelayTime(800)          8 ms MO-open settle
11 80 01 00 00 00 00 00 00 00 00 00   listWritePort(0x0001)
06 80 3e 03 00 00 00 00 00 00 00 00   listJumpSpeed(830)          3000 mm/s
26 80 c8 00 00 00 00 00 00 00 00 00   listFiberYLPMPulseWidth(200)
12 80 00 08 00 00 00 00 00 00 00 00   listMarkCurrent(2048)       50 %
1b 80 9b 02 00 00 00 00 00 00 00 00   listQSwitchPeriod(667)      30 kHz
0c 80 14 01 00 00 00 00 00 00 00 00   listMarkSpeed(276)          1000 mm/s
07 80 64 00 00 00 00 00 00 00 00 00   listLaserOnDelay(100)
08 80 64 00 00 00 00 00 00 00 00 00   listLaserOffDelay(100)
0f 80 64 00 00 00 00 00 00 00 00 00   listPolygonDelay(100)
0d 80 08 00 00 00 00 00 00 00 00 00   listJumpDelay(8)
01 80 98 7a 98 7a 00 00 a5 07 00 00   listJumpTo(31384,31384, 0, dist 1957)
05 80 68 85 98 7a 00 00 d0 0a 00 00   listMarkTo(34152,31384, 0, 2768)
05 80 68 85 68 85 00 00 d0 0a 00 00   listMarkTo(34152,34152, 0, 2768)
05 80 98 7a 68 85 00 00 d0 0a 00 00   listMarkTo(31384,34152, 0, 2768)
05 80 98 7a 98 7a 00 00 d0 0a 00 00   listMarkTo(31384,31384, 0, 2768)
02 80 00 00 00 00 00 00 00 00 00 00   listEndOfList
```

SHA-256 of the full 3072-byte packet: `9ca1e12d5b53b6d56a4ceef65fedccb94c5255341a991529a8ef83d6ec0359f5`.

The surrounding single commands, in order:
- before the list: `ResetList`, `WritePort(1)`, `Fiber_SetMo(1)`, `GetVersion`
- after it: `SetEndOfList(0)`, `ExecuteList`, `GetVersion` (idle poll), `Fiber_SetMo(0)`, `WritePort(0)`, `GetMarkTime(3)`, `GetVersion`
- then the abort block from section 1.9

The parameter order inside a marking list is fixed by `set()` for fiber: JumpSpeed → PulseWidth → MarkCurrent → QSwitchPeriod → MarkSpeed → LaserOnDelay → LaserOffDelay → PolygonDelay. JumpDelay is set lazily before the first jump. Each setter is **cached**: a value equal to the last one sent is not re-sent (`GP/controller.py:933-1080`).

### 1.6 Red light (lighting) versus mark mode

| | Lighting (`lighting_configuration`) | Marking (`marking_configuration`) |
| --- | --- | --- |
| Before the list | `ResetList` | `ResetList`, `WritePort(0x0001)` (laser pin = bit 0), **`Fiber_SetMo(1)`** (MO open) |
| List prologue | `ReadyMark`, `WritePort(0x0100)` (light pin = bit 8 on, laser bit off) | `ReadyMark`, `DelayTime(800)`, `WritePort(0x0001)`, then the `set()` parameter block |
| Motion | `dark(x,y)`: `WritePort` with bit 8 off (if it was on), `JumpDelay`, **JumpTo**. `light(x,y)`: `WritePort` with bit 8 on, `JumpSpeed(light_speed)`, **JumpTo**. Never `MarkTo`. | `goto`: JumpDelay, JumpTo. `mark`: **MarkTo** |
| Speed | `light_speed = 3000 mm/s` (`FRAME_LIGHT_SPEED_MM_S`). The first `dark` jump uses whatever jump speed the board already has. `REDLIGHTSPEED` in the profile is **not** applied. | `travel_speed = 3000`, `mark_speed = layer speed` |
| MO | never opened (closed at init and by every abort) | opened by a single command **before** the list is even sent |

Sources: `GP/controller.py:441-503, 535-571, 625-651`; `OM/mopa_luiz/live.py:308, 392-402`; `OM/mopa_luiz/profile.py:50-51`.

**Invariant for Rust (type-level):** a lighting list must be unable to contain `MarkTo`, `LaserOnPoint` or `Fiber_SetMo(1)`. In OM this holds only by convention.

Port bits: `laser_pin = 0` → mask `0x0001`; `light_pin = 8` → mask `0x0100`; `foot_pin = 15` (input, unused) (`GP/controller.py:49-50, 94-96`). What output bit 0 drives physically is [UNVERIFIED] (H11).

Frame Once (`frame_polylines`):
- retraces the shape `repeats = clamp(ceil(2.0 s / (path_len_mm / 3000)), 1, min(20000 // points, 500))` times (`OM/mopa_luiz/live.py:308-328, 664-702`)
- example: a 10 mm box gives 150 repeats
- continuous framing (`FrameLoop`) is a spooler job that **returns False forever**, so the spooler keeps appending lighting packets until stopped (`live.py:542-610`)
- FrameLoop runs **inside the HTTP server process**, not in a spawned child

### 1.7 Coordinate system

- **Galvo space:** unsigned 16-bit per axis, 0..0xFFFF, with **center `0x8000`** at design origin (0,0).
- `mm_to_galvo(x, y, F) = (clamp(round_half_even(0x8000 + x * 0xFFFF / F), 0, 0xFFFF), same for y)` (`OM/mopa_luiz/live.py:250-254`). The clamp is dead code in practice, because `geometry_errors` refuses out-of-field points first (`live.py:264-305, 331-342`).
- **Field size:**
  - `F = FIELDSIZE (profile, 200) × scale_correction (calibration.json, 1.1834) = 236.68 mm` (`live.py:78-81`)
  - `galvos_per_mm = 0xFFFF / F`: 327.675 at 200 mm, **276.8929 at 236.68 mm**
  - The same F feeds speed conversion (`live.py:399`)
  - scale_correction = measured / intended (README example: 50 mm marked as 59.17 mm gives 1.1834)
  - It is clamped to [0.5, 2.0] on load; save rejects values outside that range; a missing or corrupt file means 1.0 (`live.py:41-75`; `OM/README.md:359-371`)
- **Axis directions:** +x mm maps to +galvo X and +y mm to +galvo Y. **No invert, swap or rotate exists in the driver.** `INVERTX`, `INVERTY`, `SWAPXY` and `GALVOASPECT` are listed as "not applied" (`OM/mopa_luiz/profile.py:66-69`). The UI canvas is y-up (`ui.py:977-983`). Whether galvo +Y points away from the operator on this machine is [UNVERIFIED] (H4).
- **Transform order (client side, `ui.py:990-1000, 2118-2138`):** per object, scale (uniform), then rotate CCW by `rotation` degrees about the object origin, then translate by (x, y). After that, the global **mirror** `x → -x` and/or `y → -y` about the field center. Mirror flags live only in the browser's `localStorage` (`openmopa.mirrorX`, `openmopa.mirrorY`; `ui.py:2430-2437`), not on the server. The Rust app must persist mirror per machine.
- **Assumed start position:** each controller starts with `_last_x = _last_y = 0x8000` (`GP/controller.py:39-40, 156-157`). `goto`, `mark`, `light` and `dark` drop a move to the identical last point (`GP/controller.py:509-571`). Combined with one controller per job, a path starting at design (0,0) gets **no jump**. [SIM] Dot test: `listMarkTo(32782, 32768, 0, 14, 0)` with no prior `JumpTo`. The `dist` word is also computed from the assumed point.
- Out-of-range points (outside 0..0xFFFF) are **silently skipped** by `mark`/`goto` (`GP/controller.py:512-514`). Rust must reject them instead.

Reference values [SIM]:

| mm | F = 200 | F = 236.68 |
| --- | --- | --- |
| (0, 0) | (32768, 32768) | (32768, 32768) |
| (5, 5) | (34406, 34406) | (34152, 34152) |
| (-5, -5) | (31130, 31130) | (31384, 31384) |
| (25, -25) | (40960, 24576) | (39690, 25846) |
| (+F/2, +F/2) | (65535, 65535) | (65535, 65535) |
| (-F/2, -F/2) | (0, 0) | (0, 0) |
| (0.05, 0) | (32784, 32768) | (32782, 32768) |

### 1.8 Laser parameter encoding (fiber / MOPA)

| Parameter | UI unit | Encoding | Range / checks | Examples [SIM] |
| --- | --- | --- | --- | --- |
| Power | % | `listMarkCurrent(round_half_even(p * 4095 / 100))` | 0..100 (`safety.py:69-70`) | 1 % → 41, 10 → 410, 25 → 1024, **30 → 1228**, 50 → 2048, **70 → 2866**, 75 → 3071, 100 → 4095 |
| Frequency | kHz | `listQSwitchPeriod(round_half_even(20000 / f) & 0xFFFF)` | Profile bounds `MINPWMFREQ..MAXPWMFREQ` Hz = 1000..4 000 000 (`safety.py:72-80`) | 1 → 20000, 20 → 1000, 30 → 667, 45 → 444, 60 → 333, 100 → 200, 500 → 40, 1000 → 20, 4000 → 5 |
| Pulse width | ns | Snap with `nearest_pulse_width(v, table)` (ties go to the **first**, lower entry), then `listFiberYLPMPulseWidth(ns)` | Table = profile `PULSEWIDTHS` or the JPT M7 default `2,4,6,9,13,20,30,45,55,60,80,100,150,200,250,300,350,400,450,500`. Off-table by more than 0.5 ns → refuse; within 0.5 ns → warn and snap (`cli.py:28-49, 86-107`; `safety.py:82-91`) | 199.8 → 200 with a warning; 170 → refused |
| Mark speed | mm/s | `listMarkSpeed(min(trunc(v * 0xFFFF / F / 1000), 0xFFFF))` | layer > 0 only (`safety.py:127-128`) | at F = 236.68: 100 → 27 (actual 97.5 mm/s), 300 → 83, 1000 → 276, 3000 → 830, 6000 → 1661. **One unit = 3.61 mm/s, so v < 3.61 mm/s encodes as 0** |
| Jump / travel speed | mm/s | `listJumpSpeed(...)`, same conversion | fixed 3000 (`live.py:397-398`) | 830 |
| Laser on / off / polygon delay | µs | `(abs(d), 0x8000 if d <= 0 else 0)` | GP defaults 100/100/100. **Not taken from the profile** (`profile.py:60-65`) | 100 → `64 00 00 00` |
| Jump delay | µs (?) | same encoding | always 8 (short); "long" 200 is used only with `distance_limit`, which OM never passes | 8 |
| MO-open settle | 10 µs | `listDelayTime(int(8.0 * 100))` | fixed | 800 |
| Laser end delay | µs | `listDelayTime(int(300 / 10))` in `dwell()` only | unused by OM | |

Pulse-width-dependent frequency limits of the JPT M7 (minimum PRR rising with pulse width) are **not enforced**. Only the flat 1 kHz..4 MHz profile bound is. [UNVERIFIED] Needs the JPT datasheet (H8).

Not applied from the profile at all: `REDLIGHTSPEED`, `MARKDELAY`, `JUMPDELAY`, `LASERONDELAY`, `LASEROFFDELAY`, `POLYGONDELAY`, `INVERTX/Y`, `SWAPXY`, `GALVOASPECT`, `ENABLECORFILE`/`CORFILE`, `DOORINPORT`, `STARTMARKPORT`, `m_nEnableFiberLaserStateCheckInMarking` (`OM/mopa_luiz/profile.py:32-70`).

### 1.9 Stop, abort, pause

**`GP.abort(dummy_packet=True)`** runs entirely under `_list_build_lock` (`GP/controller.py:750-768`). [SIM] wire order:

```
1f 00 ...  StopExecute
33 00 00 00 ...  Fiber_SetMo(0)
12 00 ...  ResetList
07 00 ...  GetVersion   (wait_ready poll)
<3072 B packet of 256 x 0x8002>     dummy end-of-list packet
19 00 ...  SetEndOfList(0)
05 00 ...  ExecuteList  (only if !_list_executing)
33 00 00 00 ...  Fiber_SetMo(0)
21 00 00 00 ...  WritePort(0x0000)   laser bit and red-light bit off
```

**`GP.shutdown()`** sets `_shutdown`, clears the queue, runs `abort()`, joins the spooler thread, then `disconnect()`. `disconnect()` closes the device: kernel re-attach, release interface, dispose, **`device.reset()`** (`GP/controller.py:196-210, 328-335`; `GP/usb_connection.py:237-252`).

**Pause/resume** are `StopList 0x0020` + `paused=True` and `RestartList 0x0013` (`GP/controller.py:770-778`). They are **never used by OM**, and their hardware semantics are [UNVERIFIED] (H15).

**OM stop layers:**
1. **`/api/stop`** (`OM/mopa_luiz/ui.py:2546-2556`) first calls `stop_active_hardware_jobs()` to stop marking, then `FRAME_LOOP.stop()`.
2. `stop_active_hardware_jobs()` (`live.py:221-239`):
   - sets `STOP_REQUESTED`
   - sends SIGTERM to every live job process
   - joins each for up to 8 + 2 s, then SIGKILL
3. The child's SIGTERM handler (`live.py:142-175`):
   - `_graceful_hardware_stop()` → `stop_controller(_ACTIVE_CONTROLLER)`
   - an 8 s watchdog `os._exit(137)` covers a mute board
   - then `os._exit(143)`
4. `stop_controller(c)`: `c._openlaser_stop.set()` then `c.shutdown()` (`live.py:134-139`).
5. `/api/mark` checks `stop_requested()` between repeats, layers and passes (`ui.py:2833-2861`).
6. Job timeouts are 60 s by default and **600 s for mark passes**. On timeout: SIGTERM → graceful abort → SIGKILL, plus a "power-cycle" hint (`live.py:28, 177-218`; `ui.py:2870`).

Measured stop latency **0.18 s with no USB errors** on the real board after the fix. Before the fix it was 3-6 s with about 40 USB resets [HW-LOG] (commit `de9f478`, 2026-10-19).

### 1.10 The spooler, `_list_build_lock`, and the stop-latency bug and fix

**Spooler** (`GP/controller.py:182-255`):
- `submit(job)` appends to `_queue` under `_spooler_lock` (a `threading.Condition`), then `start()` creates one `_spooler_thread` if none exists.
- `_spooler_run` loops while the queue is non-empty:
  - if `paused`, wait on the condition
  - take `queue[0]`; return if `_shutdown`
  - call `fully_executed = job(controller)`
  - **`ConnectionAbortedError` → return.** The thread exits; `_spooler_thread` is not cleared.
  - `ConnectionRefusedError` → wait on the condition, then retry
  - if `fully_executed`: `remove(job)` (notify_all), then `initial_configuration()`
  - a job returning False is called again forever (this is FrameLoop)
- Note: if a FrameLoop job returns True after a stop, the following `initial_configuration()` runs **outside** the try. A stop-induced `ConnectionAbortedError` there escapes as an uncaught thread exception (harmless but noisy) [CODE].

**`_list_build_lock`** (an RLock) is held by the whole of `_list_end`: `wait_ready`, packet write, `SetEndOfList`, and maybe `ExecuteList`. It is also held by `_list_new`, by the buffer mutation in `_list_write`, and by **all of `abort()`** (`GP/controller.py:91, 750-768, 1120-1150`). So an `abort()` from another thread blocks while the spooler sits in `_list_end` waiting for READY.

**The bug** [HW-LOG, commit `de9f478`; mechanism CODE]:
1. During continuous framing the board's list buffer fills.
2. The spooler sits in `wait_ready()` holding the lock. OM's capped `wait_ready` polled every 10 ms for up to **3 s**.
3. Stop calls `shutdown()` → `abort()`, which blocks on the lock.
4. After 3 s `wait_ready` gave up and `_list_end` **wrote the packet into the full buffer**. The 100 ms USB write timed out.
5. `usb_connection.write()` then did close + `device.reset()` + open and retried, up to 4 times. Every following command (StopExecute, ResetList, ...) also hit timeouts and resets: "~40 resets per stop", 3-6 s, and a real risk of wedging the board.

**The fix** (`OM/mopa_luiz/live.py:455-482`, `134-139`):
- `apply_wait_timeouts()` attaches `controller._openlaser_stop = threading.Event()`.
- It replaces `wait_idle`, `wait_finished` and `wait_ready` with `wait_until(predicate)`:
  - poll `predicate()` (each poll is a `GetVersion` USB round trip)
  - return when past a 3 s deadline
  - wait 10 ms on the stop event
  - **if the event is set and the current thread is the spooler thread, raise `ConnectionAbortedError`.** That aborts `_list_end` before the write, releases the lock as the `with` exits, and `_spooler_run` returns cleanly.
  - if set on any other thread, return
  - return if `_sending` is False
- `stop_controller()` sets the event **before** `shutdown()`, so `abort()` gets the lock within about 10 ms and its commands reach a board that isn't being flooded.
- Regression tests: `tests/test_galvo_geometry.py:31-59`. Spooler aborted in under 1 s; non-spooler waits released in under 1 s.

**What the fix leaves behind (section 0, finding 1)** [SIM]:
- The 3 s cap still applies to every **normal** wait.
- Long-running lists lose MO at +3 s (`initial_configuration`'s `wait_idle`) and are `StopExecute`d at +6 s (main thread `wait_finished`, then `shutdown`).
- During streaming of a list whose packets take more than 3 s each to drain (large or slow raster, or continuous framing of a large shape), `wait_ready` still times out and **writes into a full buffer**. That is the original storm trigger, reachable without any stop.
- Simulated timeline, fake board BUSY for 20 s, READY always set:

```
t=0.059 ExecuteList (after 3rd of 9 packets)        t=0.063 last packet + SetEndOfList
t=0.063..3.07  GetVersion polls every ~12 ms (BUSY)
t=3.073 Fiber_SetMo(0)  WritePort(0)  GetMarkTime(3)    <- MO closed while board still BUSY
t=3.07..6.08  GetVersion polls (BUSY)
t=6.084 StopExecute  Fiber_SetMo(0)  ResetList  <dummy list>  SetEndOfList  ExecuteList  Fiber_SetMo(0)  WritePort(0)  close
```

**Rust design requirements from this:**
- One session-owning driver task that alone touches USB, with a command channel and a stop flag checked between every transfer.
- Never write a list packet unless the most recent status had READY set. If READY doesn't come back, stay blocked, but stoppably.
- Detect a stuck board with a watchdog derived from the **estimated execution time** of the queued packets, plus margin. Its action is a **stop** (StopExecute, MO close, port off), never "proceed".
- Never issue the end-of-job MO close or port-off while BUSY.
- Never auto-reset the USB device. Bound each transfer to about 100-250 ms (validate on hardware) and surface errors.
- Stop path: set the flag, the driver task drops any unsent packet, then sends StopExecute → Fiber_SetMo(0) → ResetList → (dummy EOL packet + SetEndOfList + ExecuteList, kept for parity until H14 says otherwise) → Fiber_SetMo(0) → WritePort(0). Target at most 0.25 s.

### 1.11 galvoplotter quirks the Rust port must handle deliberately

| Quirk | Where | Rust decision |
| --- | --- | --- |
| `round()` is half-to-even; `int()` truncates | everywhere | `round_ties_even`, `trunc`; the golden suite enforces it |
| Delay of 0 encodes with the sign word `0x8000` | `GP/controller.py:1208, 1216, 1253, 1261` | Reproduce (`d <= 0`) until hardware says otherwise |
| `set_pulse_width` caches `self.pulse_width`, not its argument | `GP/controller.py:1076-1080` | Use the argument |
| `set()` doesn't persist a new `delay_polygon` (`self.delay_polygon = self.delay_polygon`) | `GP/controller.py:977-980` | n/a; store explicitly |
| `_list_write` overflow check runs outside the lock | `GP/controller.py:1141-1142` | Single-owner driver task, no lock needed |
| `_wait_for_input_protocol` references undefined `self._aborting` and `connection.read_port` | `GP/controller.py:594-617` | Don't port; write input waits fresh (H11) |
| `rotary_position` uses `<< 16 &` instead of `\|` | `GP/controller.py:662-667` | Don't port (rotary unsupported) |
| A failed read reads as all status bits set | `GP/controller.py:395-396` | `Result<Status, Error>` |
| `VERSION = "0.1.2"` in `__init__`, but the distribution is 0.2.0 | `GP/__init__.py:4`, `dist-info/METADATA` | cosmetic |
| The wheel installs top-level `examples/` and `test/` packages; those tests open real USB | `galvoplotter-0.2.0.dist-info/RECORD` | Never run them |
| `light_on`/`light_off` emit `listWritePort` only when the cached bit changes | `GP/controller.py:625-651` | Keep the cache, but always write the port in the list prologue |

---

## 2. Safety invariants enforced today

| # | Invariant | Where enforced | Test |
| --- | --- | --- | --- |
| S1 | **Frame Only and disabled layers never burn**; Output-off layers never burn | `layers.py:46-51` (`emits_when_marking`), `186-187` (`emitting_jobs`); `/api/mark` refuses with HTTP 400 "nothing to mark" when no step emits: `ui.py:2815-2822`. The old fallback that marked framing geometry with live params was removed in commit `64f4b1b`. | `test_planner.py:188-204` |
| S2 | Framing never emits | Lighting config only: `live.py:522-527, 572-580, 681-688`; `evaluate_emission(intends_emission=False)` at `live.py:507-517, 554-564, 665-675`. Lighting never sends MarkTo or Fiber_SetMo(1) (`GP/controller.py:475-503, 535-571`) | indirect (`test_live_chain.py:159-172`) |
| S3 | **Out-of-field points refused, not clamped**; NaN/inf refused; edge tolerance 1e-6 mm | `live.py:261, 264-305, 331-342`; per-layer errors on Plan (`ui.py:2744-2750`) and Mark (`ui.py:2825-2831`); framing geometry checked on Plan (`ui.py:2933-2934`) | `test_galvo_geometry.py:83-89`; `test_planner.py:212-220` |
| S4 | **At most 3000 paths, 2000 points per path, 20000 points per layer pass**: refused, not truncated | `live.py:257-259, 272-298` | `test_galvo_geometry.py:91-101` |
| S5 | **The whole job is validated before the first layer burns** | `ui.py:2808-2831`: `validate_layers` (power, speed, frequency, pulse width, raster pitch and angles) and geometry over every emission step, before the repeat loop | `test_planner.py:222-231` |
| S6 | Emission requires the literal "ARM" (case-insensitive, trimmed) plus `arm=true`, and at least 1 path | `safety.py:26, 64, 93-97`; `live.py:613-625, 713-724` (every emitting entry point re-checks) | `test_safety.py:40-75` |
| S7 | Power 0..100 %, frequency within profile Hz bounds, pulse width on the table (±0.5 ns snap) | `safety.py:69-91, 108-145`; `cli.py:270-289` | `test_safety.py`, `test_config.py:58-94` |
| S8 | Scale correction clamped to [0.5, 2.0] | `live.py:41-75` | `test_calibration.py:46-58` |
| S9 | Wedged-controller protection | `live.py:33-39, 142-218, 385-452`; README `449-454` | `test_live_chain.py:69-115` |
| S10 | Stop latency (section 1.10) | `live.py:134-139, 455-482`; `ui.py:2546-2556` | `test_galvo_geometry.py:31-59`; `test_live_chain.py:159-172` (frame after stop is not latched) |
| S11 | Stop is not sticky | `clear_stop_request()` at the start of frame, test and mark (`ui.py:2628, 2788, 2806`) | `test_live_chain.py:159-172` |
| S12 | Direct control only on a `direct_control` machine; frame and mark refused for G-code profiles | `ui.py:2785-2786, 2795-2796, 2803-2804`; `machines.py:72` | `test_planner.py:115-120` |
| S13 | Heartbeat | Client polls `GET /api/health` every 3000 ms with a 2500 ms timeout. Offline: banner, `body.serverOffline` disables hardware controls (`ui.py:2273-2313`). The server answers with pyusb enumeration only and never opens the device (`ui.py:2487-2500`). | `test_live_chain.py:148-157` |
| S14 | Workspace sanitising: at most 3000 objects, at most 500 000 points, finite numbers, name ≤ 200 chars, atomic write | `ui.py:53-54, 88-152` | `test_workspace.py` |
| S15 | G-code: finite coordinates, inside the bed, power 0..100, speed > 0 and capped at the machine max, starts with G21/G90/tool-off | `gcode.py:37-67, 113-120, 92-100` | `test_machines.py:33-60` |

**Why 3000/20000/2000?** No rationale is written down. The constants first appear as silent truncation caps in the initial open-source commit `da86464` (2026-05-06) and became refusals in `64f4b1b`. A plausible reason is keeping each spawned job (600 s timeout, one connection per pass) and its list stream bounded. 20 000 points is about 20 000+ list commands, about 79 packets. [UNVERIFIED] The Rust app should derive limits from estimated execution time and memory, or show them as explicit, documented UI limits.

**Gaps found (should become Rust invariants):**
- G1: Confirm dialog and Job Summary show **Frame** parameters, not the layer parameters that will burn (`ui.py:2210-2222, 1540-1556` versus `2839-2847`).
- G2: `arm` is always `true` from the client (`ui.py:2151`), and the ARM text persists. Make ARM single-use and time-limited.
- G3: Hidden layers with Output on burn but aren't framed (`layers.py:46-51` versus `195-196`).
- G4: The first jump is elided at the design origin (section 1.7).
- G5: Speeds below 1 galvo unit/ms encode to `MarkSpeed(0)`. Large quantisation at low speed: 10 mm/s → 2 units, about 7.2 mm/s.
- G6: The 3 s wait-cap truncation (section 1.10).
- G7: Objects whose `layer_id` matches no layer are **silently dropped** (`layers.py:175-177`).
- G8: No door, interlock or foot-switch input is read (`DOORINPORT` not applied, `profile.py:52-53`).
- G9: Frame and mark parameters are not checked against JPT pulse-width-dependent PRR limits.
- G10: `group_by_layer` crashes on points with more than 2 coordinates (`layers.py:180`, tuple unpack).

---

## 3. GRBL output today, and what a streaming sender needs

### 3.1 Profiles

From `OM/mopa_luiz/machines.py:57-126`:

| id | kind | bed mm | origin | tool | laser cmd | max speed | air assist | notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `jpt-m7-200` | fiber_galvo | 200×200 | center | laser | (LMC) | 6000 | n/a | `direct_control=True`, `uses_frequency`, `uses_pulse_width`, transport `jcz_lmc_usb`, output `jcz_lmc` |
| `co2-grbl-600x400` | co2_gantry | 600×400 | bottom_left | laser | **M3** (constant power) | 500 | yes | `power_scale=1000`, "not Ruida DSP" |
| `diode-grbl-400x400` | diode_gantry | 400×400 | bottom_left | laser | **M4** (dynamic) | 250 | yes | |
| `plotter-grbl-a4` | pen_plotter | 210×297 | bottom_left | pen | pen down **`M3 S1000`**, up **`M5`** | 150 | no | |

`travel_speed_mm_s` is defined but **unused** by the G-code compiler; G0 runs at the controller's rapid rate.

### 3.2 Emitted format

From `OM/mopa_luiz/gcode.py:87-157`:

```
; OpenLaser G-code
; Machine: <name>
; Generated: <UTC ISO8601, seconds>        <- non-deterministic; golden tests must mask line 3
; Design origin: center; units: millimetres
G21
G90
M5
; Layer: <name> · <operation>
M8                                          (laser profiles with air_assist only)
  per pass (vector) / once (raster):
  "; Pass i/n"                              (only if passes > 1, non-raster)
  per path:  M5 / G0 X.. Y.. / (M3|M4) S<round_half_even(1000*p/100)> or "M3 S1000" for pen /
             G1 X.. Y.. F<speed*60> (F repeated on every G1) / M5
M9                                          (if M8 was emitted)
...
M5
M9                                          (always, even for the plotter)
G0 X0 Y0                                    (machine 0,0 = bed corner for bottom_left)
; End OpenLaser job
<trailing newline>
```

- Coordinates: `_number(v) = f"{v:.3f}"` with trailing zeros and the dot stripped; `"-0"` becomes `"0"` (`gcode.py:32-34`).
- `bottom_left`: `X = x + W/2`, `Y = y + H/2`. `top_left`: `Y = H/2 - y` (`gcode.py:37-57`).
- **S range 0..1000**, scaled from %; the README tells users to match `$30` (`README.md:318-320`).
- **Feed** is mm/min, `min(layer speed, machine max) * 60`. Raster example: 600 mm/s capped to 500 → `F30000` on CO2 and 250 → `F15000` on diode.
- Raster: `passes` drives cross-hatch angles inside `hatch_polylines`; the outer pass loop runs once (`gcode.py:74-84, 112`).
- Frame Only, disabled and Output-off layers are skipped (`gcode.py:107-108`).
- Each path emits `M5`, `G0`, `M3/M4 S..`, `G1...`, `M5`, giving **double `M5` lines** between paths. A byte-exact golden must keep them.
- [SIM] example, CO2, 10 mm square at 25 % and 10 mm/s:

```
M8
M5
G0 X295 Y195
M3 S250
G1 X305 Y195 F600
G1 X305 Y205 F600
G1 X295 Y205 F600
G1 X295 Y195 F600
M5
M9
```

- Plotter: no G4 dwell after pen down or up (servo settle). The README asks users to verify `M3 S1000` and `M5` on their machine (`README.md:466-470`).

### 3.3 What a real GRBL streaming sender would also need

Primary sources [WEB]: https://github.com/gnea/grbl/blob/master/doc/markdown/interface.md and https://github.com/gnea/grbl/blob/master/doc/markdown/commands.md. Code tables: `doc/csv/error_codes_en_US.csv`, `doc/csv/alarm_codes_en_US.csv` in the same repo.

1. **Serial and startup:**
   - USB-serial (CH340, FTDI, native CDC), 115200 8N1 by default. 115200 is [UNVERIFIED] for each machine.
   - Opening the port often resets Arduino-class boards (DTR). Wait for the welcome line `Grbl X.Xx ['$' for help]` (or grblHAL's variant) before sending anything.
   - Then query `$I` (build info, `[VER:]`/`[OPT:]`, which carries buffer sizes), `$$` (settings), `$#` (offsets) and `$G` (modal state).
2. **Flow control:**
   - **Character counting.** GRBL documents a 128-character RX buffer; the doc's send-response section says 127. Read actual sizes from `[OPT:...]` or the status `Bf:` field.
   - Track the byte length (+1 for `\n`) of every unacknowledged line. Pop the oldest on each `ok` or `error:N`.
   - Send-response mode is required for `$` settings writes (EEPROM).
   - Overlong lines give `error:11`.
3. **Realtime bytes** (no newline, bypass the buffer):
   - `?` status, `!` feed hold, `~` cycle start/resume, `0x18` soft reset
   - `0x84` safety door, `0x85` jog cancel
   - feed override `0x90` (100 %) / `0x91` +10 / `0x92` −10 / `0x93` +1 / `0x94` −1
   - rapid override `0x95` 100 / `0x96` 50 / `0x97` 25
   - spindle/laser power override `0x99` 100 / `0x9A` +10 / `0x9B` −10 / `0x9C` +1 / `0x9D` −1
   - `0x9E` spindle stop toggle (only in HOLD), `0xA0` flood toggle, `0xA1` mist toggle (needs `ENABLE_M7`)
4. **Status reports:**
   - Format: `<State|MPos:x,y,z or WPos:...|Bf:planner,rx|Ln:n|FS:feed,speed or F:feed|Pn:XYZPDHRS|Ov:f,r,s|WCO:x,y,z|A:SCFM>`
   - States: Idle, Run, Hold:0/1, Jog, Alarm, Door:0-3, Check, Home, Sleep
   - `WPos = MPos − WCO`
   - Poll `?` at about 5-10 Hz. `$10` selects MPos or WPos.
5. **Messages:**
   - `ok`, `error:N`, `ALARM:N`, `[MSG:...]`, `[GC:...]`, `[HLP:...]`
   - `[G54:]..[G59:]`, `[G28:]`, `[G30:]`, `[G92:]`, `[TLO:]`, `[PRB:]`, `[VER:]`, `[OPT:]`, `[echo:]`
   - `>...:ok` startup-line echo; do **not** count it as a streaming ack
6. **Error policy for lasers:** in character-counting mode GRBL keeps executing lines already buffered after an `error:`. On any error or alarm, send `!` and then `0x18`, mark the job failed, and require operator acknowledgement.
7. **Alarms:** `ALARM:N` locks the machine. Unlock with `$X` (position may be lost) or home with `$H`.
8. **Homing and jogging:**
   - `$H`
   - jog with `$J=G91 G21 X.. Y.. F..` (F mandatory, not modal, mm/min); cancel with `0x85`
   - `$C` check mode is a useful pre-flight dry run: parse the whole program with no motion, laser off, then auto-reset
9. **Work coordinates:**
   - `G54`-`G59`, `G10 L20 P1 X0 Y0` sets the current position as origin, `G92`, `G53` for machine moves
   - Today's export assumes absolute **machine** coordinates from the bed corner (`bottom_left` origin). A sender must offer machine origin, current-position origin, or user origin, LightBurn-style.
10. **Laser mode:** `$32=1` is needed for M4 dynamic power and for the laser to be off during G0. Check `$30`/`$31` against `power_scale=1000`. Check `$130`/`$131` against the profile bed, and `$20`/`$21`/`$22` soft/hard limits and homing.
11. **Framing on gantry machines:** G0 or G1 around the bbox with the laser off, or M3 at a tiny S only if the operator enables it. Rapids don't fire in laser mode.
12. **Pen plotter:** add a configurable G4 dwell after pen up and pen down.
13. **Ruida:** out of scope (UDP protocol). OM doesn't support it (`README.md:33-34, 538`).

---

## 4. Data model

### 4.1 Layer settings

`LayerSpec`: `OM/mopa_luiz/layers.py:28-54`. Loader defaults: `layers.py:106-126`. Built-in defaults: `layers.py:57-103`. Persisted to `layer_settings.json` (`ui.py:51, 57-75`).

| Field | Type / unit | `layer_from_dict` default | Notes |
| --- | --- | --- | --- |
| `layer_id` | str | `payload.layer_id or payload.id or "layer"` | Key objects refer to |
| `name` | str | "Layer" | |
| `operation` | enum | `vector_engrave` | one of `vector_engrave`, `vector_cut`, `raster_engrave`, `frame_only`, `disabled`; anything else is an error |
| `power_percent` | float % | **1.0** (built-in layers use 100.0) | |
| `frequency_khz` | float kHz | 30.0 | |
| `pulse_width_ns` | float ns | 200.0 | must hit the pulse table |
| `speed_mm_s` | float mm/s | 1000.0 | |
| `passes` | int | 1 | vector: repeat each layer job; raster: number of cross-hatch angles |
| `visible` | bool | true | used by framing only (gap G3) |
| `output` | bool | true | |
| `color` | str `#rrggbb` | `#26d0a8` | |
| `air_assist` | bool | false | G-code M8/M9 only |
| `raster_pitch_mm` | float mm | 0.1 | > 0 |
| `raster_angle_deg` | float deg | 0.0 | finite |
| `raster_angle_step_deg` | float deg | 90.0 | finite; per-pass increment |

Built-in defaults: Vector Engrave (100 %, 30 kHz, 200 ns, 1000 mm/s); Vector Cut / Deep Mark (100 %, 30, 200, 300); Raster (100 %, 30, 200, 1000, pitch 0.1, 0°, step 90°); Frame Only (0 %, 30, 200, 1000).

The user's saved `layer_settings.json` differs: Raster uses 45 kHz, 300 mm/s, angle 90°, step 0° (`OM/layer_settings.json:37-53`).

**Operations at mark time** (`ui.py:2901-2923`):
- `vector_engrave` and `vector_cut` burn their polylines `passes` times. Each pass is a **separate hardware job**.
- `raster_engrave` turns closed polylines into hatch segments (`raster.py:65-162`):
  - closure tolerance 0.05 mm
  - scan offset `pitch * 0.0173`
  - even-odd rule
  - boustrophedon ordering
  - 2-point segments; open paths are skipped and counted
  - **one** hardware job
- `frame_only` is traced in red light only.
- `disabled` is neither framed nor burned.

### 4.2 Workspace

`workspace.json`, written atomically by `save_workspace` (`ui.py:142-152`). Observed file: 403 objects, `saved_at` `2026-10-19T19:16:45+00:00`.

```jsonc
{
  "version": 1,                         // int
  "saved_at": "2026-10-19T19:16:45+00:00",
  "objects": [                          // ≤ 3000 objects, ≤ 500 000 points total
    {
      "name": "businessCard.dxf · path 1",   // str ≤ 200
      "layer_id": "raster",                  // str (default "vector-engrave")
      "polylines": [[[x_mm, y_mm], ...]],    // list of polylines, each ≥ 2 finite [x,y]
      "pointCount": 73,                      // int, recomputed on save
      "x": 0.0, "y": 0.0,                    // float mm translation
      "scale": 1.0,                          // float, uniform
      "rotation": 0.0                        // float deg, CCW
    }
  ]
}
```

Observed contents [SIM]:
- every object has exactly one polyline
- transforms are all baked: x = y = 0, scale = 1, rotation = 0
- 304 objects on `vector-engrave` (1750 points), 99 on `raster`
- raw bbox x 9.39..48.58, y −13.41..44.35 mm
- largest object 73 points
- with the saved raster settings (0.1 mm pitch, 90°), the raster layer yields **969 hatch segments from 32 closed shapes; 67 open shapes are skipped**
- both layers pass `geometry_errors` at F = 200 and F = 236.68
- **mirror flags are not in the workspace** (browser localStorage only)

### 4.3 Machine profile (EZCAD-style markcfg) and calibration

`profiles/openmopa-machine.ini` holds an `[LMC_CFG]` section:

```
LASERTYPE=4   FIELDSIZE=200   MINPWMFREQ=1000   MAXPWMFREQ=4000000
m_nIPGSerialNo=7   m_bEnableIPGSetPulseWidth=1
m_nEnableFiberLaserStateCheckInMarking=1   REDLIGHTSPEED=3000
; optional PULSEWIDTHS=2,4,...,500
```

- Parsing uses latin-1, `ConfigParser(interpolation=None)`, case-preserving keys; a missing section raises (`cli.py:76-83`). The default path can be overridden by `OPENLASER_PROFILE` / `OPENMOPA_PROFILE` (`cli.py:17-23`).
- Of these keys, only `FIELDSIZE`, `MINPWMFREQ`, `MAXPWMFREQ` and `PULSEWIDTHS` change behaviour.
- `LASERTYPE=4`: GP's docstring calls laser_type 4 "CO2" (`GP/controller.py:1220-1224`), yet this is a fiber MOPA profile. The meaning of EZCAD's LASERTYPE codes is [UNVERIFIED]. OM never sends it.
- `calibration.json` is `{"scale_correction": 1.1834}` (gitignored; `live.py:41, 53-75`).

### 4.4 Local HTTP API

`ui.py:2477-2887`. Useful for parity tests if the Rust app keeps a web front end.

- **GET:** `/`, `/api/health`, `/api/detect`, `/api/machines`, `/api/config` (adds `SCALE_CORRECTION`), `/api/board` (**opens USB in the server process**), `/api/frame/status`, `/api/safety`, `/api/layers/defaults`, `/api/layers`, `/api/workspace`, `/api/profile/inspect`
- **POST:** `/api/stop`, `/api/import`, `/api/join`, `/api/raster/preview`, `/api/layers/save`, `/api/workspace/save`, `/api/calibration/save`, `/api/test/preview`, `/api/test/run`, `/api/export/gcode`, `/api/plan`, `/api/frame`, `/api/frame/start`, `/api/frame/stop`, `/api/mark`
- **Job payload:** `{machine_id, objects:[{name, layer_id, polylines (already transformed and mirrored)}], layers:[LayerSpec...], live:{text, power, frequency_khz, pulse_width_ns, mark_speed, repeat_count (1..999), size_mm, confirm, arm}}`
- **Errors:** `PermissionError` → 403, other exceptions → 400

### 4.5 Importers

`importers.py`:
- **SVG:** line, polyline, polygon, rect, circle/ellipse (73 points), and paths with M/L/H/V/Z only, no curves. Normalised to fit **60 mm** and centered, with Y flipped.
- **DXF:** LINE, LWPOLYLINE (bulge flattened, 6-96 steps), POLYLINE/VERTEX/SEQEND, CIRCLE (73 points), ARC (48), ELLIPSE (96), SPLINE (control or fit points, no evaluation). `$INSUNITS` is honoured; units are preserved and centered; no Y flip.
- **STL:** at most 900 triangles projected to XY, normalised to 60 mm.

---

## 5. Test inputs and the golden-test strategy

### 5.1 Existing suite (OM `tests/`, 119 tests, all passing via `unittest`)

| File | Tests | Covers |
| --- | --- | --- |
| `test_safety.py` | 16 | ARM gate, power 0..100, frequency bounds, pulse table and snap (199.8 → 200 warns, 170/120 refuse), custom `PULSEWIDTHS`, empty job, layer validation |
| `test_planner.py` | 14 | `/api/plan` and `/api/mark` via `do_POST` with `run_hardware_job` mocked: grouping, G-code profile plan/export, refusal on export-only machines, ARM 403, per-layer job count, raster to 2-point hatch, open raster skip, Frame Only/Output-off never marked, out-of-field refusal, bad later layer blocks whole job |
| `test_galvo_geometry.py` | 10 | stop-latency regression (spooler aborts, other waits release), Frame Once repeats, `mm_to_galvo`/`sanitize`, out-of-field/NaN/inf refusal, 3000/20000 limits refused not truncated |
| `test_raster.py` | 10 | `is_closed`, 10 lines per 10 mm at 1 mm pitch, open skip, donut hole, cross-hatch doubles, 90° vertical, pitch > 0, passes 0 |
| `test_geometry.py` | 9 | `join_lines` (touch, tolerance, gap, closed square, T-junction, internal vertices, empty), `bounding_box` |
| `test_install_support.py` | 9 | libusb backend selection, detect hints (Zadig, udev, cable), second-launch reuse, port-in-use error |
| `test_calibration.py` | 8 | `scale_correction` load/save/clamp/corrupt, `effective_field_size` = 236.68 |
| `test_live_chain.py` | 8 | `OPENMOPA_MOCK`; wedge diagnosis (stub hangs, "Power-cycle"); graceful stop; **real HTTP server + spawned job on MockConnection**: health, frame/stop/frame, mark reaches galvo |
| `test_config.py` | 6 | markcfg parsing, nearest pulse snapping table, test-box plan bounds |
| `test_importers.py` | 6 | DXF LINE/LWPOLYLINE/POLYLINE/bulge/ELLIPSE, `$INSUNITS` inches → mm |
| `test_layers.py` | 6 | defaults valid, raster pitch 0 flagged, 101 % flagged, layer order, emitting/framing filters, dict round-trip |
| `test_machines.py` | 6 | catalog kinds, diode `M4 S250` / `G0 X190 Y200` / `G1 X210 Y200 F600`, CO2 `M3 S250` + M8/M9 + 2 passes, plotter `M3 S1000` and `G0 X105 Y148.5`, out-of-bed refusal |
| `test_ui_launch.py` | 6 | browser URL for wildcard binds, browser-open failure tolerance, `--no-browser` |
| `test_workspace.py` | 5 | persistence round trip, corrupt file, sanitiser, atomic write, HTTP round trip |

Hermeticity notes:
- `test_planner.py` and `test_live_chain.py` read the **real** `calibration.json` through `effective_field_size`, so results depend on the 1.1834 file. For example, the 150 mm out-of-field test holds at both 200 and 236.68.
- `/api/health` enumerates USB.
- The live chain uses GP's MockConnection, whose status reads are **random bytes** (`GP/mock_connection.py:100-113`), so wait loops end non-deterministically. That is unusable for golden streams; use the Appendix A fake instead.

### 5.2 Fixtures to reuse

- `OM/workspace.json`: real business-card job; 403 objects, 3057 points, mixed vector and raster, with 67 open raster shapes (exercises skipping).
- `OM/layer_settings.json`: real saved layers.
- `OM/profiles/openmopa-machine.ini` and `OM/calibration.json` (1.1834).
- In-test fixtures: DXF snippets (`test_importers.py:9-157`), squares and donut (`test_raster.py`), join cases (`test_geometry.py`), edge geometry (`test_galvo_geometry.py`).
- GP CO2 encodings (section 1.3.2).
- Captured streams from this report: init, 10 mm box mark, 10 mm box frame, dot test, 1100-segment long job, abort (Appendix A regenerates them).

### 5.3 Golden strategy

Generate goldens from the Python implementation, which is the behaviour proven on hardware, and commit them as JSON under `rust/tests/golden/`. Each golden records generator version, OM git SHA (`6aed8ff` at time of writing) and GP version.

| Tier | Python oracle | Fixture form | Rust must match |
| --- | --- | --- | --- |
| T0 numeric encoders | `mm_to_galvo`, `power_to_mark_current`, `frequency_to_period`, GP `_convert_speed`, delay encoding, `nearest_pulse_width`, distance word | Table of thousands of seeded random inputs plus every edge case: ties like 30 % / 70 %, ±F/2, 0, 1e-6 tolerance, speed < 1 unit | **Exactly** (integers) |
| T1 LMC packet encoders | Appendix A harness with the deterministic fake board (READY=1, BUSY=0) | For scenarios mark_box10, frame_box10, dot_test, long_job (1100 segments → 9 packets), business-card per layer at F = 236.68, abort: ordered events `{single hex / list 3072-B hex or sha256 / read}` | **Byte-exact for packet contents and single-command bytes.** Sequencing must match a *divergence-aware* comparator: Rust intentionally adds the explicit first jump (G4), keeps one session (no per-job init/Reset/USB reset), and drops the end-of-job abort. Keep a `py-compat` mode or a test-only shim that replays the Python sequence, so the encoders are proven against the exact Python stream; the intended Rust sequence is reviewed once and frozen as `rust-v1` goldens. |
| T2 G-code | `compile_gcode` for 4 profiles × {vector, multi-pass, air assist, raster cross-hatch, plotter} | text | Byte-exact after masking line 3 (`; Generated:`). Both languages format with correctly rounded shortest decimal semantics for `{:.3f}`; confirm with a 100k random-value test. |
| T3 raster | `hatch_polylines` | segments JSON | Segment **count and order exact**; coordinates within 1e-9 mm. Trig may differ by 1 ulp; keep Python's operation order. |
| T4 importers | `import_geometry` for DXF/SVG/STL fixtures plus `businessCard.dxf` if the original file is located | polylines JSON | point count exact; coordinates within 1e-9 |
| T5 safety decisions | `evaluate_emission`, `validate_layer_settings`, `geometry_errors`, `/api/mark` refusal paths | (inputs) → {ok, error *codes*} | Same decision and same error category. Strings may differ, so Rust uses error enums. |
| T6 timing (fake board) | none; written fresh in Rust | Scripted board model: READY drops after N packets, BUSY for T seconds, stalled writes | (a) stop under 250 ms from any state; (b) **no write while not READY**; (c) **no MO close or StopExecute while BUSY unless stopped**, so the 20 s simulation must complete; (d) watchdog fires only when no progress is made past the estimated time |
| T7 hardware acceptance | none | Section 7 checklist | Manual, with recorded USB logs |

Determinism rules for the generator:
- Patch `CALIBRATION_PATH` to a temp file, never the user's.
- Set `OPENMOPA_MOCK=1`.
- Make `usb.core.find` and `galvo.controller.USBConnection` raise.
- Mask timestamps.
- Record GP's cache-dependent parameter emission as-is.

---

## 6. galvoplotter license and authorship; porting

- **galvoplotter 0.2.0** on PyPI (released 2023-09-27; earlier 0.1.0-0.1.2 in July 2023). METADATA: `License: MIT`, `Author: Tatarize <tatarize@gmail.com>`, home page https://github.com/meerk40t/galvoplotter. The `LICENSE` file is MIT, **"Copyright (c) 2023 MeerK40t"** (`.venv/.../galvoplotter-0.2.0.dist-info/LICENSE`, `METADATA:1-24`). [WEB] https://pypi.org/pypi/galvoplotter/json and the GitHub repo confirm MIT.
- Credits in the README: Bryce Schroeder ("initial work for reverse engineering"; balor), inpain/tiger12506, Sandor Konya (`METADATA:178-185`).
- **balor** (https://gitlab.com/bryce15/balor) is **not** permissive. The repository `LICENSE` file is **GNU GPL v3**; the GitLab project page reports "Server Side Public License, v 1". The two disagree, and either is copyleft. **Do not copy balor code or comments into the MIT crate.**
- **MeerK40t** (whose `balormk` driver is a richer LMC reference) is MIT [WEB] https://github.com/meerk40t/meerk40t, so it is also usable with attribution.
- **OpenLaser prototype:** MIT, "Copyright (c) 2026 Luiz Bueno" (`OM/LICENSE`).
- **Conclusion (not legal advice):** translating galvoplotter's logic and constants into an MIT Rust crate is permitted. MIT's only condition is that the copyright and permission notice go with all copies or substantial portions.
  - Ship a `THIRD_PARTY_NOTICES` (or `LICENSE-galvoplotter`) with the MeerK40t MIT text.
  - Add a header in the `lmc` module crediting galvoplotter (MeerK40t / Tatarize) and MeerK40t `balormk`.
  - Opcode values and protocol behaviour are interoperability facts. Keeping the attribution is still the right thing to do.
- `pyproject.toml` leaves `galvoplotter` **unpinned** (`OM/pyproject.toml:28-32`). Pin it in the Python prototype, or at least record 0.2.0 in the golden metadata.

---

## 7. Unknowns that need a hardware session before the Rust driver is trusted

Ordered by risk. Unless a step needs emission, use the red light (lighting lists), low power and a scrap plate. Log every USB transfer with timestamps, using a Rust "trace" transport or the Python harness pointed at real USB under supervision.

| # | Question | Why it matters | Procedure |
| --- | --- | --- | --- |
| H1 | **Do jobs that run more than 3-6 s after the last packet get truncated?** | Finding 1; G6 | Red light only: Frame Once a job whose single pass takes more than 8 s, e.g. long jumps across the field (each pass is then a single repeat). Watch whether the light stops at about 6 s. Then a long low-power raster on scrap. |
| H2 | READY/BUSY semantics and **buffer depth**: how many 3072-B packets fit, when READY drops and comes back, BUSY during and after the list | Rust flow control and watchdog | Stream N lighting packets of long jumps without ExecuteList, then with it. Poll GetVersion after each and record status words with timing. |
| H3 | Mirror position after `Reset`, after a job, and after abort (`GetPositionXY 0x000C`) | G4 (elided first jump) | Read the position at connect, after a job ending off-center, and after reconnect. |
| H4 | Axis orientation and mirroring with `mirror` off | correctness | Frame an asymmetric "F" plus arrow; mark once at low power; record the invert/swap needed per machine. |
| H5 | Correction table: blank (current) versus the machine's EZCAD `.cor`; what 1.1834 really compensates | accuracy | Locate `CORFILE` in the EZCAD install. Mark a 100 mm grid with each. Re-derive scale_correction and pincushion. Use GP's `.cor` readers (`GP/controller.py:864-919`). |
| H6 | **Pulse-width encoding for the JPT M7** (raw ns in `0x8026`?) and whether `m_bEnableIPGSetPulseWidth` mode needs more setup | MOPA correctness | Low-power lines at 4/20/200/500 ns, same frequency and power; compare marks. Best: Windows USBPcap/Wireshark capture of EZCAD setting known values. |
| H7 | Power mapping `0x8012` 0..4095 to optical power | settings parity with EZCAD | EZCAD capture at 10/50/100 %; power-meter or visual ramp |
| H8 | Frequency encoding (50 ns ticks) and JPT M7 PRR-versus-pulse-width limits | safety G9 | EZCAD capture at 30 and 500 kHz; JPT datasheet table |
| H9 | Delay units and EZCAD defaults: jump delay (8?), polygon (µs or ÷10 per MeerK40t), laser on/off; whether "−0" (`0x8000`) matters | mark quality | EZCAD capture with known delays; mark corners and dashed lines |
| H10 | Low-speed quantisation (1 unit = 3.61 mm/s at F = 236.68) | G5 | Mark at 5, 10 and 50 mm/s; time and measure |
| H11 | Output port map (bit 0 = "laser"? bit 8 = red light confirmed visually) and inputs (door, foot pedal via `ReadPort 0x0025` / `InputPort 0x0031`) | interlock G8 | Toggle bits with the laser disabled; read inputs while operating the door or pedal |
| H12 | USB descriptors: speed (FS/HS), wMaxPacketSize, interfaces; GetVersion and GetSerialNo replies | transport sizing, board fingerprint | `system_profiler SPUSBDataType` / `ioreg -p IOUSB -l` with the laser on; log the replies |
| H13 | Write behaviour when the buffer is full (NAK until timeout?) and the safe transfer timeout | reset-storm avoidance | Deliberately send one packet while READY = 0, red light only, then recover; measure |
| H14 | Minimal correct abort: are ResetList and the dummy EOL packet + ExecuteList needed after StopExecute? | stop latency, cleanliness | Compare abort variants during red-light streaming; measure latency and the next job's behaviour |
| H15 | Pause/resume (`StopList 0x0020` / `RestartList 0x0013`) | LightBurn-class pause | Red-light streaming pause and resume; check position continuity |
| H16 | `GetMarkTime(3)` and `GetListStatus` reply meaning | progress UI | Log replies around jobs of known duration |
| H17 | Wedge recovery without power cycle (USB reset? `Reset 0x0040`?) | UX | Only if a wedge happens naturally; never provoke one with emission |
| H18 | Windows WinUSB (Zadig) and Linux udev paths on real hardware | cross-platform claims | `rusb`/`nusb` on each OS, red light only |
| H19 | MO-open settle (8 ms) and first-pulse behaviour on the M7 | first-mark quality | Mark short dashes at job start versus mid-job |
| H20 | Does the board care about the configured source type (`SetLaserMode=1`, EZCAD `LASERTYPE=4`)? | init correctness | Compare the init sequence with an EZCAD capture |

---

## Appendix A: capture harness (Python, no hardware)

Saved at `/private/tmp/claude-501/-Users-luizbueno--claude-spares-spare-HuLL3w/ba10990c-59a4-470b-ab69-9ee5e8b41d86/scratchpad/lmc-capture/capture_lmc.py`. The scratch path may not persist, so the full text is reproduced here. Run with the OM venv:

```
.venv/bin/python capture_lmc.py mark_box10 > mark_box10.json
.venv/bin/python capture_lmc.py long_job 20 > long_job.json   # board BUSY 20 s after ExecuteList
```

```python
"""Capture the exact LMC USB byte stream OpenLaser's Python path produces.
NO HARDWARE: pyusb's usb.core.find and galvoplotter's USBConnection are replaced
with functions that raise; RecordingBoard stands in for the device."""
from __future__ import annotations
import json, os, struct, sys, tempfile, time
from pathlib import Path

os.environ["OPENMOPA_MOCK"] = "1"
sys.path.insert(0, "/Users/luizbueno/AI/OpenLaser/OpenMopa")
import usb.core
def _no_usb(*a, **k):
    raise RuntimeError("USB access blocked by capture harness")
usb.core.find = _no_usb
import galvo.controller as gc
from galvo.consts import list_command_lookup, single_command_lookup
gc.USBConnection = _no_usb
from mopa_luiz import live
from mopa_luiz.cli import MarkConfig

READY, BUSY = 0x20, 0x04
T0 = time.monotonic()

class RecordingBoard:
    """Fake LMC: READY always set; BUSY for `busy_s` after ExecuteList."""
    def __init__(self, busy_s=0.0):
        self.devices, self.events, self.busy_s = {}, [], busy_s
        self.exec_at = None; self._last_single = None
    def is_open(self, index=0): return index in self.devices
    def open(self, index=0): self.devices[index] = True; return index
    def close(self, index=0):
        self.devices.pop(index, None)
        self.events.append({"t": round(time.monotonic() - T0, 3), "kind": "close"})
    def write(self, index=0, packet=None):
        data = bytes(packet); assert len(data) in (12, 0xC00)
        t = round(time.monotonic() - T0, 3)
        if len(data) == 12:
            words = struct.unpack("<6H", data)
            if words[0] == 0x0005: self.exec_at = time.monotonic()
            if words[0] in (0x001F, 0x0012): self.exec_at = None
            self._last_single = words[0]
            self.events.append({"t": t, "kind": "single", "hex": data.hex(), "words": list(words),
                                "name": single_command_lookup.get(words[0], "?")})
        else:
            cmds = [list(struct.unpack("<6H", data[i:i + 12])) for i in range(0, len(data), 12)]
            nop = [0x8002, 0, 0, 0, 0, 0]
            last = max((i for i, c in enumerate(cmds) if c != nop), default=-1)
            self.events.append({"t": t, "kind": "list", "n_real": last + 1, "fill": len(cmds) - last - 1,
                                "cmds": [c + [list_command_lookup.get(c[0], "?")] for c in cmds[: last + 2]]})
    def read(self, index=0):
        busy = self.exec_at is not None and (time.monotonic() - self.exec_at) < self.busy_s
        status = READY | (BUSY if busy else 0)
        self.events.append({"t": round(time.monotonic() - T0, 3), "kind": "read",
                            "after": single_command_lookup.get(self._last_single, "?"), "status": status})
        return struct.pack("<4H", 0, 0, 0, status)

BOARD = RecordingBoard(busy_s=float(sys.argv[2]) if len(sys.argv) > 2 else 0.0)

class CapturedController(gc.GalvoController):
    def __init__(self, *a, **k):
        super().__init__(*a, **k); self.connection = BOARD

def main(scenario, scale=1.1834):
    tmp = Path(tempfile.mkdtemp()) / "calibration.json"
    tmp.write_text(json.dumps({"scale_correction": scale}))
    live.CALIBRATION_PATH = tmp
    live.GalvoController = CapturedController
    live.ensure_libusb_backend = lambda: {"available": True, "source": "capture"}
    cfg = MarkConfig(path=Path("mem"), values={"FIELDSIZE": "200", "MINPWMFREQ": "1000", "MAXPWMFREQ": "4000000"})
    params = live.JobParams(text="", power=50.0, frequency_khz=30.0, pulse_width_ns=200.0, size_mm=10.0, mark_speed=1000.0)
    box = [[[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0], [-5.0, -5.0]]]
    if scenario == "mark_box10":
        result = live.mark_polylines(cfg, params, box, arm=True, confirm="ARM")
    elif scenario == "frame_box10":
        result = live.frame_polylines(cfg, params, box)
    elif scenario == "dot_test":
        p = live.JobParams(text="", power=1.0, frequency_khz=30.0, pulse_width_ns=200.0, size_mm=10.0, mark_speed=1000.0)
        result = live.mark_polylines(cfg, p, [[[0.0, 0.0], [0.05, 0.0]]], arm=True, confirm="ARM")
    elif scenario == "long_job":
        paths = [[[-50 + (i % 100), -50 + (i // 100) * 5], [-49.5 + (i % 100), -50 + (i // 100) * 5]] for i in range(1100)]
        result = live.mark_polylines(cfg, params, paths, arm=True, confirm="ARM")
    else:
        raise SystemExit("unknown scenario")
    print(json.dumps({"scenario": scenario, "scale": scale, "result_keys": sorted(result), "events": BOARD.events}, indent=1))

if __name__ == "__main__":
    main(sys.argv[1])
```

Reference outputs from this run:
- `mark_box10`: 38 single commands; list 0 sha256 `9ca1e12d…59f5`; dummy abort packet sha256 `e479558e2e4b35e14d00d3990477b8cb9646a94ca9502790fe12bb71c56b6a18` (256 × `0x8002`).
- `dot_test`: single list of 12 commands, **no JumpTo**, `listMarkTo(32782,32768,0,14,0)`, `listMarkCurrent(41)`.
- `long_job` with a 20 s BUSY board: 9 packets (8 × 256 + 1 × 164 commands); ExecuteList after packet 3; MO closed at +3.0 s; StopExecute at +6.0 s.

## Appendix B: every opcode constant in galvoplotter

From `GP/consts.py:1-87`.

**List (0x80xx):**

| Opcode | Name | Opcode | Name |
| --- | --- | --- | --- |
| 8001 | JumpTo | 8002 | EndOfList |
| 8003 | LaserOnPoint | 8004 | DelayTime |
| 8005 | MarkTo | 8006 | JumpSpeed |
| 8007 | LaserOnDelay | 8008 | LaserOffDelay |
| 800A | MarkFreq | 800B | MarkPowerRatio |
| 800C | MarkSpeed | 800D | JumpDelay |
| 800F | PolygonDelay | 8011 | WritePort |
| 8012 | MarkCurrent | 8013 | MarkFreq2 (GP raises NotImplemented) |
| 801A | FlyEnable | 801B | QSwitchPeriod |
| 801C | DirectLaserSwitch (NotImplemented) | 801D | FlyDelay |
| 801E | SetCo2FPK | 801F | FlyWaitInput |
| 8021 | FiberOpenMO | 8022 | WaitForInput |
| 8023 | ChangeMarkCount | 8024 | SetWeldPowerWave |
| 8025 | EnableWeldPowerWave | 8026 | FiberYLPMPulseWidth |
| 8028 | FlyEncoderCount | 8029 | SetDaZWord |
| 8050 | JptSetParam ("Unknown"; never used, possibly relevant to JPT, [UNVERIFIED]) | 8051 | ReadyMark |

**Single (0x00xx):**

| Opcode | Name | Opcode | Name |
| --- | --- | --- | --- |
| 0002 | DisableLaser | 0004 | EnableLaser |
| 0005 | ExecuteList | 0006 | SetPwmPulseWidth |
| 0007 | GetVersion | 0009 | GetSerialNo |
| 000A | GetListStatus | 000C | GetPositionXY |
| 000D | GotoXY (x, y, angle, distance; realtime jog) | 000E | LaserSignalOff |
| 000F | LaserSignalOn | 0010 | WriteCorLine |
| 0012 | ResetList | 0013 | RestartList |
| 0015 | WriteCorTable | 0016 | SetControlMode |
| 0017 | SetDelayMode | 0018 | SetMaxPolyDelay |
| 0019 | SetEndOfList | 001A | SetFirstPulseKiller |
| 001B | SetLaserMode | 001C | SetTiming |
| 001D | SetStandby | 001E | SetPwmHalfPeriod |
| 001F | StopExecute | 0020 | StopList |
| 0021 | WritePort | 0022 | WriteAnalogPort1 |
| 0023 | WriteAnalogPort2 | 0024 | WriteAnalogPortX |
| 0025 | ReadPort | 0026 | SetAxisMotionParam |
| 0027 | SetAxisOriginParam | 0028 | AxisGoOrigin |
| 0029 | MoveAxisTo | 002A | GetAxisPos |
| 002B | GetFlyWaitCount | 002D | GetMarkCount |
| 002E | SetFpkParam2 | 002F | FiberPulseWidth |
| 0030 | FiberGetConfigExtend | 0031 | InputPort (4 = clear lock, 2 = enable lock, 1 = disable lock) |
| 0032 | SetFlyRes | 0033 | Fiber_SetMo |
| 0034 | Fiber_GetStMO_AP | 0036 | GetUserData |
| 0038 | GetFlySpeed | 0039 | DisableZ |
| 003A | EnableZ | 003B | SetZData |
| 003C | SetSPISimmerCurrent | 0040 | Reset |
| 0041 | GetMarkTime | 0062 | SetFpkParam |
