# Unexpected closes of the GUI: findings, diagnostics and the host-death safety question

Started 2026-11-03 for Alexandre's report on OpenLaser 0.1.3 x86. His PC runs Windows 10 Pro 32-bit with Intel HD
graphics, driver 10.18.10.4358, and OpenGL-only wgpu. The GUI closes by itself every few minutes:

- while framing, even when nobody touches it;
- after an import;
- while he edits settings.

Labels used below:

- **CONFIRMED** means read in the code or in the dependency source, with the place named.
- **HYPOTHESIS** means it fits the reports but is not proven. The diagnostic build exists to prove or refute these.
- **HARDWARE** means only a bench session at the machine can answer it.

## 1. Findings

### CONFIRMED

1. **No trace survived any end of the GUI.**
   - `main.rs` uses `windows_subsystem = "windows"`, so a release build on Windows has no stderr.
   - No `log` logger was installed, so every `log::error!` went nowhere. That includes wgpu's own messages and the
     engine's "panicked; the project was restored".
   - The panic hook stopped the machines, then ran the default hook, which writes only to stderr.
2. **Any wgpu error nobody captures is a `panic!`.**
   - wgpu 30.0.1 `src/backend/wgpu_core.rs` `default_error_handler`: `log::error!("Handling wgpu errors as fatal by
     default"); panic!("wgpu error: {err}")`.
   - It is reached for Validation, OutOfMemory and Internal errors when no error scope and no
     `on_uncaptured_error` handler exist (`handle_error_or_return_handler`).
   - egui-wgpu 0.36.2 and eframe 0.36.2 install no handler (a grep of both finds no `on_uncaptured_error`), and
     neither did OpenLaser.
3. **On OpenGL (WGL), a failed present is such an error.**
   - wgpu-hal 30.0.1 `src/gles/wgl.rs` returns `SurfaceError::Other` when `GetDC` returns an invalid DC, when
     `wglMakeCurrent` fails, or when `SwapBuffers` fails.
   - wgpu-core 30.0.1 `src/present.rs` turns that into `SurfaceError::Invalid`, a Validation-type error.
   - wgpu's `Queue::present` passes it to `handle_error_nolabel`, which reaches the default handler and panics.
   - Surface `configure` failures, which happen after a resize or after `Outdated`, take the same path.
   - Some WGL failures panic with no error object at all: `AdapterContext::lock` waits at most 1 s and then
     `expect`s, and `make_current(...).unwrap()` / `unmake_current().unwrap()` unwrap. A handler cannot catch these.
4. **While a machine is framing or running, the UI repaints every 250 ms** (`ui-egui/src/app.rs`, `safety_active`).
   - That is about 4 presents a second, about 14,000 an hour.
   - A rare per-present driver failure therefore shows up as "every few minutes while framing", with nobody touching
     anything.
   - When idle, the UI repaints only on input.
5. **The frame loop is host-driven.** `driver/src/lmc_session.rs` `frame`:
   - Each pass is a full list. The next pass is sent only after the previous one has executed.
   - The board has no repeat count and no light mode.
   - So a dead host leaves at most the current pass on the board (see §3).
6. **On Luiz's Mac, the closes were not Rust panics.**
   - `~/Applications/OpenLaser.app/Contents/MacOS/openlaser-launch` sends stderr to `~/Library/Logs/OpenLaser.log`.
   - That log shows relaunches every 2–4 minutes (2026-10-31 15:08–15:33) and no "panicked" text.
   - `~/Library/Logs/DiagnosticReports` holds no `openlaser` report, so there was no SIGSEGV or SIGABRT crash either.
   - Those ends were clean exits, kills, or relaunches by hand. The log cannot tell these apart, which is why this
     work adds the marker.
   - (The same log shows `bench start` refused with `not_a_tty`, because the launcher has no terminal.)
7. **The simulator's recording grows without bound.**
   - `lmc-sim/src/fake.rs`: `wire`, `lit` and `burned` gain entries on every status read (every 10 ms) and every
     frame pass while connected.
   - This affects only a GUI running on the simulator. Alexandre's GUI drives the USB board.
   - Left as a follow-up (§5).

### HYPOTHESES, ranked (the diagnostic build decides)

1. **H-A: a wgpu panic on a WGL present or context failure** (CONFIRMED 2 + 3 + 4).
   - Old Intel WGL drivers are known for intermittent `SwapBuffers` and `wglMakeCurrent` failures.
   - It fits "during framing, untouched". After an import or a settings edit, the repaints are just as frequent.
   - **Proof in the new log:** `openlaser::gpu ... wgpu validation error` records. If it kills the app (the unwrap
     paths), a `PANIC on thread 'main'` record mentions `wgl.rs` or `wgpu error`, and the supervisor records exit
     code 101.
2. **H-B: a native crash inside the Intel OpenGL driver** (`ig7icd32.dll`, `ig75icd32.dll`, …).
   - **Proof:** the supervisor records `0xC0000005 ACCESS VIOLATION`. The diagnostics script shows Application
     Error 1000, with the faulting module named.
3. **H-C: running out of the 32-bit address space.**
   - The space is 2 GB, and the GL driver maps into the same process.
   - `refresh_plan` rebuilds the whole job on the UI thread after every import or settings edit. The planner allows
     up to 20 M section points (about 320 MB) and 25 M image cells.
   - **Proof:**
     - the supervisor records `0xC0000409 FAIL-FAST` and the log holds `memory allocation of N bytes failed`; or
     - wgpu logs an `OUT OF MEMORY` error;
     - and in either case the heartbeat's `virtual` size climbs towards 2,000 MB.
4. **H-D: the process is killed from outside**, for example by antivirus or a logoff.
   - **Proof:** exit code 1 or a Ctrl-C status, with no panic record and no Event 1000.

## 2. What this branch changes

| Change | Where | Covers |
|---|---|---|
| A file logger with UTC timestamps, version, pid and thread on every start record. It folds repeats, rotates at 4 MiB to `openlaser.old.log`, and caps each process at 32 MiB. It writes to `%LOCALAPPDATA%\OpenLaser\logs\openlaser.log` (other OSes: `<data dir>/OpenLaser/logs`; or `OPENLASER_LOG_DIR`). | `apps/openlaser/src/diagnostics.rs` | everything below lands here |
| **Supervisor (Windows default).** `openlaser.exe` starts itself again as the window process and appends the child's stderr to the log (panic text, `memory allocation of N bytes failed`, `has overflowed its stack`). It waits and names the exit status: 0, 143, 101 panic, `0xC0000005`, `0xC0000409`, `0xC00000FD`, … After a death it shows a box: "OpenLaser closed unexpectedly … check the machine now: use the E-stop or the key switch". `OPENLASER_SUPERVISOR=0` turns it off; `=1` turns it on elsewhere. | `diagnostics::supervise`, `main.rs` | native crash, OOM abort, stack overflow, kill, panic |
| **Session marker** `session.txt`: `state=running`, then `ended` with the reason, or `died` with the status. The next start logs "THE PREVIOUS RUN DID NOT END CLEANLY" or "DIED". | `diagnostics.rs` | also a supervisor that was itself killed, and power loss |
| Exit reasons, logged under `openlaser::exit`: window close request (X, Alt+F4, Cmd+Q), File › Exit, the close guard (stop confirmed or not), agent `app.quit`, the signal watcher (exit 143), a `run_native` error, and "event loop ended with no close request" (macOS Dock Quit). | `ui-egui/src/app.rs`, `control/mod.rs`, `lib.rs`, `main.rs` | intentional ends |
| The panic hook stops the machines first, as before. It then logs the message, place, thread and a backtrace. | `lib.rs` `install_panic_hook`, `panic_record` | Rust panics, including caught ones |
| **wgpu errors are logged and counted, not panicked.** `on_uncaptured_error` and a device-lost callback are installed on the window's device. | `lib.rs` `install_gpu_error_handlers` | H-A (also a mitigation: a skipped frame instead of a closed window) |
| Operations: every Device, Io, Settings and Request command, with origin, params (≤ 200 chars), result and duration, plus every stop. This covers connect, `laser.frame` and `laser.frameStop`, `laser.start`, import, and Device Settings save. | `engine/src/session.rs` `run_spec` | what was happening before the end |
| Startup records: build, OS and CPU, `WGPU_BACKEND`, simulator or real laser, graphics adapter and driver, max texture size. | `main.rs` | H-A, H-B |
| Heartbeat every 60 s: virtual, private and working-set memory, handles and threads (`wmic` / `ps`). When the UI draws, it also logs frames per second, textures held and the graphics-error count. | `diagnostics::start_heartbeat`, `DesktopApp::logic` | H-C, repaint rate, dating a silent death |
| The diagnostics script prints `session.txt`, the crash markers, the last 120 log lines, and the last 30 days of Application Error 1000, WER 1001 and Hang 1002 entries for `openlaser.exe`. It reads the faulting module and exception code language-independently. It only reads. | `packaging/windows/OpenLaser-diagnostics.ps1` | everything |

**Why no exception filter or minidump crate (no `unsafe`).**

- The supervisor's exit status already names the crash class.
- WER Event 1000 names the faulting DLL and offset.
- The child's stderr carries Rust's own last words.
- `__fastfail`, which Rust's abort and the OOM path use, bypasses `SetUnhandledExceptionFilter` anyway.

If a dump is ever needed, WER LocalDumps can write one without code. It is a registry key under
`HKLM\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\openlaser.exe`. Setting it changes the PC, so it
needs Luiz's approval and is not done by the script.

**Tests.**

- `diagnostics.rs`:
  - timestamps, the level filter, repeat folding, rotation, the exit reason;
  - the marker's clean, unclean and died cases, and a foreign pid;
  - exit-code names;
  - the supervisor recording a child's stderr and its death.
- `lib.rs`:
  - a wgpu error is logged and does not panic (regression for CONFIRMED 2);
  - the panic record.
- `engine/src/tests.rs`: which commands the operations record lists, and the params cut.
- CI `windows-x86-smoke`, on Mesa GL:
  - starts the GUI;
  - checks that the window belongs to a supervised child;
  - kills the child as a crash would;
  - checks the death record and `state=died`;
  - starts again and checks "THE PREVIOUS RUN DIED".

## 3. Safety: what the machine does when the GUI dies

All claims below were checked in code unless they are marked HARDWARE.

- **Who sends the stop bytes.** Always the driver thread (`openlaser-driver <id>`).
  - The panic hook, the signal watcher and the close guard only trip latches (atomic) and wait at most 500 ms.
  - The driver thread holds no UI lock, so a UI-thread panic still gets a full stop.
  - The stop is `StopExecute`, `MO(0)`, `ResetList`, `MO(0)`, `WritePort(0)`, then a confirming read.
- **What nothing in the process can cover**, because the driver thread dies with the process:
  - an access violation in the GL driver;
  - an alloc-error abort (it skips the panic hook);
  - a stack overflow;
  - `TerminateProcess` (`taskkill /F`, Task Manager);
  - power loss;
  - a USB unplug.
- **USB close sends nothing.**
  - The transport has no `Drop` impl that writes anything.
  - `disconnect` parks (`MO(0) · WritePort(0)`) only on a clean driver exit.
  - When the OS closes the WinUSB handle on process death, no bytes are written. Whether the firmware reacts to that
    is HARDWARE.
  - galvoplotter's `device.reset()` on close was deliberately not ported.
- **Framing after a death.**
  - The loop is host-driven, so at most the current pass list finishes (`lmc_session.rs` `frame`).
  - Lighting lists can express only jumps, delays and `WritePort` of the red bit. The encoder and the driver both
    refuse MarkTo, LaserOnPoint and the laser bit.
  - **But a pass list does not end with `Port(0)`** (`lmc/src/encode.rs` lighting packets). Only Frame Once and a
    clean stop send `WritePort(0)` afterwards.
  - So after a death the red pointer may stay on as a still dot. That is an aiming-beam hazard, not a fire hazard.
  - Whether the board clears its ports at the end of a list is HARDWARE.
- **Marking after a death.**
  - The prelude opens MO (`Fiber_SetMo(1)`) and sets the laser bit before any list.
  - The run-ahead window is at most 3 packets and 2 s by estimate (`builtin.rs` `max_packets` 3, `max_emission_s`
    2.0). One packet longer than 2 s is still sent alone.
  - So after a death the board marks what it holds, max(about 2 s, the longest packet), and **MO is never closed**.
  - Whether an open MO with no list running can emit, and how long the board really keeps going, is HARDWARE.
- **No heartbeat or watchdog reaches the board.**
  - The host watchdog (no progress for 1.5 × estimate + 2 s) and the 250 ms transfer timeout live in the process that
    dies.
  - No board-side timeout is documented in SPEC, HARDWARE.md, docs/research or the golden notes.

### Options so the machine cannot keep running unattended (proposals; none is implemented here)

1. **Out-of-process driver** (PLAN.md's `ol-driverd`). A child process with no GPU code owns the USB board and runs
   the existing driver runtime. The GUI talks to it over a pipe, and a closed pipe trips the latch, so today's
   emergency stop runs.
   - Covers GL crashes, OOM, stack overflow and a kill of the GUI.
   - Does not cover the daemon itself dying, power loss, or an unplug.
   - Biggest change: the provider in `main.rs`, `DeviceAccess` in the engine, the driver control source, and the
     board lock.
2. **Watchdog that re-opens USB.** The supervisor added here already waits on the window process. In a `hw` build
   it could, after a death, open the board and send the emergency sequence (`MO(0) · WritePort(0) · StopExecute`).
   - Smaller than option 1.
   - Needs on HARDWARE: re-open latency, whether WinUSB allows it at once, and whether the board accepts
     `StopExecute` from a fresh session without the init sequence (`connect` today refuses an empty init).
   - It only helps if it acts before the buffered 2 s run out, so the bench numbers below decide whether it is worth
     having.
3. **End every lighting list with `Port(0)`.** The red dot cannot then stay lit after a death. This changes golden
   bytes, so it needs a `DIVERGENCES.md` entry. Cheap; recommended once H-CRASH-1 confirms the dot stays.
4. **Tighter marking run-ahead.** Split packets so none is longer than about 0.5 s, and/or lower `max_emission_s`.
   This shrinks the post-death bound. Better still, open MO inside the list rather than in the prelude, if the board
   has such an opcode (HARDWARE / research).
5. **Board-side timeout.** None is known. It would need firmware research (H15, `StopList` / `RestartList`).
6. **Hardware.** The E-stop and key switch remain the primary stop (AGENTS.md §1.5). A USB-power-sensing relay on
   the laser enable would be independent of all software.

### Bench test plan H-CRASH (for Luiz at the machine; agents never run it)

Have the E-stop in reach, goggles on, and a scrap plate in place. Use a `hw` build from Luiz's lane or the diagnostic
zip. Film the work area and the controller LEDs at 60 fps with a visible clock.

1. **Frame Loop, then kill.** Frame Loop a 50 mm square. In a second window run `taskkill /F /IM openlaser.exe`.
   Record whether the dot keeps tracing, stops dead, stays lit as a still dot, or goes dark, and for how long.
2. Repeat step 1 with a large slow shape, where one pass takes several seconds. This measures "finishes the current
   pass".
3. Repeat step 1 with Frame Once on a long shape (more than 8 s).
4. Repeat steps 1–3 with a USB cable pull instead of the kill.
5. After each kill: start OpenLaser again and connect. Record whether connect works and whether the first status
   shows BUSY. Power-cycle the board if it is stuck.
6. **Marking at the lowest power**, inside the 10 % first-emission cap, on scrap:
   - Start a job of about 30 s with several layers. Kill the app about 10 s in.
   - Measure from the kill until marking stops, and note whether the fiber source still shows ready or MO open.
   - **Hit the E-stop if marking goes on for more than about 5 s.**
7. Repeat step 6, killing between two layers. This shows whether an open MO with no list emits.
8. Repeat step 6 with a slow raster, where one packet is long. This checks the max(2 s, longest packet) bound.
9. Repeat step 6 with a cable pull.
10. Write every result in HARDWARE.md and compare it with the crash bound the Start confirmation showed.

    If any case runs longer than that bound, or the red dot or MO stays on, do two things:
    - implement option 3 (Port(0));
    - plan option 1 or option 2 before more unattended framing.

## 4. Collecting evidence on Alexandre's PC (needs Luiz's approval for each step)

1. Install the diagnostic zip from this branch's Release workflow run (`windows-x86` artifact) next to the current
   one. Do not replace the profile, which lives in `%LOCALAPPDATA%\OpenLaser\machines`.
2. Use the PC as usual until the window closes. The supervisor shows "OpenLaser closed unexpectedly (…)".
3. Run "OpenLaser diagnostics" and send the report. These parts of it answer the hypotheses:
   - its "How OpenLaser ended" section;
   - its "Windows Event Log" section;
   - the last 120 log lines.

## 5. Follow-ups (not in this branch)

- Bound the simulator's `Recording` (CONFIRMED 7), or drop it in the GUI.
- Move `refresh_plan` off the UI thread, or cap it for 32-bit, if H-C is confirmed.
- Options 1–4 of §3, decided by H-CRASH.
- The camera panel's `ColorImage::from_rgba_unmultiplied` length assert (`ui-egui/src/panels.rs`, camera frames).
  It applies only with a camera.
