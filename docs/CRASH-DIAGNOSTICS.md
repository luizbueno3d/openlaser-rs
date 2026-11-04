# Unexpected closes of the GUI: findings, diagnostics and the controller's execution model

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

## 1b. Field evidence: the first diagnostic build on Alexandre's PC (2026-11-04)

The diagnostic build was installed in `C:\OpenLaser-diag`, with Luiz's approval. Alexandre imported a 150 × 40 mm
outlined logo (DXF, one 5,817-point path), connected, and changed the layer to fill at 0.1 mm. 92 s after start the
window died. The log, in UTC, shows:

```
14:41:49 graphics: Intel(R) HD Graphics 2500 (Gl, IntegratedGpu), driver 4.0.0 - Build 10.18.10.4358
14:41:51 openlaser::gpu #1: wgpu validation error ... In Surface::configure: Failed to wait for GPU to come idle
14:42:49 heartbeat ui: 667 frames in 60 s (11.1 per s), 75 textures
14:42:49 heartbeat memory: virtual 1091 MB, private 803 MB, working set 804 MB, handles 1975, threads 26
14:43:20 openlaser::gpu: the graphics device was lost (Unknown): Device is lost
14:43:21 PANIC on thread 'main' at egui-wgpu-0.36.2/src/renderer.rs:1030: Failed to create staging buffer for
         vertex data. Vertex count: 67442. Required vertex buffer size: 1348840
14:43:21 supervisor: THE WINDOW PROCESS (pid 6196) DIED: exit code 101
```

### What is confirmed

- **The crash is a device loss, then a panic in egui-wgpu.**
  - After the device is lost, `queue.write_buffer_with` returns `None`, and egui-wgpu `panic!`s (`renderer.rs:1029`).
  - No handler we install can catch this.
- **The 14:41:51 fence-wait error would have closed v0.1.3.**
  - It is `ConfigureSurfaceError::GpuWaitTimeout`, a Validation error, so the default handler panics on it.
  - The new handler logged it and the app kept running.
- **The engine is not where the memory goes.**
  - The CLI imports a synthetic DXF with the same shape (one 5,817-point outline and 16 letter contours).
  - Its peak resident memory is 15 MB, for the import, the line plan and the fill plan at 0.1 mm alike.
- **The GUI is small on Metal.**
  - On the Mac (Metal), the GUI stays at about 90 MB resident after 60 s, idle or with that drawing.

### HYPOTHESIS H-E (ranked first): wgpu's OpenGL fences never report done on this driver, so every frame's upload buffers leak

1. On OpenGL, wgpu frees a frame's upload buffers (egui's vertex and index data) only after a GL fence says the GPU
   finished that frame (wgpu-hal `gles/fence.rs`).
2. The fence wait at 14:41:51 timed out.
3. 803 MB ÷ 667 frames ≈ 1.2 MB a frame. That matches the 1.35 MB vertex upload for this drawing.
4. A detailed import raises the bytes per frame, and framing repaints 4 times a second. Both fit "after an import"
   and "every few minutes while framing".

**The fix under test:** `GlFenceBehavior::AutoFinish` (`openlaser::gl_fence_behavior`). It is the default now, and
`OPENLASER_GL_FENCE=normal` turns it off. The heartbeat now runs every 20 s, so the next log shows the memory slope.

**How to confirm or refute it:**
- If the private memory stays flat while Alexandre works the same way, H-E is confirmed.
- If it still climbs at about 1.2 MB a frame, H-E is refuted.

## 2. What this branch changes

| Change | Where | Covers |
|---|---|---|
| A file logger with UTC timestamps, version, pid and thread on every start record. It folds repeats, rotates at 4 MiB to `openlaser.old.log`, and caps each process at 32 MiB. It writes to `%LOCALAPPDATA%\OpenLaser\logs\openlaser.log` (other OSes: `<data dir>/OpenLaser/logs`; or `OPENLASER_LOG_DIR`). | `apps/openlaser/src/diagnostics.rs` | everything below lands here |
| **Supervisor (Windows default).** `openlaser.exe` starts itself again as the window process and appends the child's stderr to the log (panic text, `memory allocation of N bytes failed`, `has overflowed its stack`). It waits and names the exit status: 0, 143, 101 panic, `0xC0000005`, `0xC0000409`, `0xC00000FD`, … After a death it shows a box: "OpenLaser closed unexpectedly (…). A job the laser board had already received may still be running". It opens no device and stops nothing. `OPENLASER_SUPERVISOR=0` turns it off; `=1` turns it on elsewhere. | `diagnostics::supervise`, `main.rs` | native crash, OOM abort, stack overflow, kill, panic |
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

## 3. The USBLMCV4 execution model: what happens to a submitted job in each failure

Luiz's direction (2026-11-03) is production reliability. A GUI failure should not needlessly interrupt a job the
controller is running on its own. **This branch changes no stop or shutdown behaviour.** The panic hook stop,
`stop_all_bounded`, the close guard, ARM, Stop, the driver's fault handling and the watchdog are untouched. The new
supervisor opens no device and stops nothing.

This section is read-only analysis, with no recommendations. **CONFIRMED** means read in the code, at the place
named. **HARDWARE** means only a bench test can answer it.

### The model, common to every case (CONFIRMED)

- **The driver runs inside `openlaser.exe`.** It is a thread of the GUI process (`main.rs` `device_provider` →
  `DeviceAccess::Provider`; thread `openlaser-driver <id>`, `driver/src/runtime.rs`). No other process talks to the
  board.
- **A job is streamed, never stored whole on the board.**
  - `lmc_session.rs` `may_send` sends the next packet only while fewer than `max_packets` packets are unexecuted and
    the queued time estimate stays within `max_emission`.
  - The built-in profile sets 3 packets and 2.0 s (`machine/src/builtin.rs`).
  - A single packet longer than 2 s is still sent on its own.
  - So the board holds at most about 3 packets / 2 s, or one longer packet. The rest of the job exists only in the
    host process.
  - Exception: a job small enough to fit inside that window is entirely on the board once `ExecuteList` is sent.
- **Before and after a list.**
  - Before each list the prelude is `ResetList · WritePort(laser bit) · Fiber_SetMo(1) · GetVersion`
    (`lmc/src/encode.rs`).
  - After the list, and only once the host has seen !BUSY, the postlude is `MO(0) · WritePort(0) · GetMarkTime`.
  - The host sends the postlude: the board does not close MO on its own (as far as the code tells).
- **Framing.**
  - Frame Loop is host-driven. Each pass is a separate list, sent after the previous pass finished executing.
  - The board has no repeat count (`lmc_session.rs` `frame`).
  - Lighting lists can express only jumps, delays and `WritePort` of the red bit.
  - A pass list does not end with `Port(0)`. Frame Once and a clean stop send `MO(0) · WritePort(0)` afterwards.
- **What the host reads.** The status word has `BUSY` 0x04, `READY` 0x20 and `AXIS` 0x40
  (`lmc/src/packet.rs`). Nothing else is read: no laser-source alarm, no galvo error, no input pins. Inputs are
  only probed by PR #13's `ports`.

### (a) GUI process crash

There are two sub-cases, and they behave differently.

- **(a1) A Rust panic on any thread.** This includes one the engine catches (`session.rs` `catch_unwind`).
  - The panic hook trips every stop latch and waits ≤ 500 ms (`apps/openlaser/src/lib.rs` `install_panic_hook`).
  - The driver thread is still alive, so it runs the full stop sequence: `StopExecute · MO(0) · ResetList · … ·
    WritePort(0)`, then a confirming read.
  - **The job is aborted by design.** It does not finish.
  - Changed by this branch: a wgpu error used to become such a panic, so it stopped the job and closed the window.
    It is now logged instead (`install_gpu_error_handlers`), and the job and window continue. **No stop code was
    changed, but the outcome of that one cause did change. This is Luiz's decision to keep or revert.**
- **(a2) A native crash, OOM abort, stack overflow, `taskkill /F` or power loss of the PC.**
  - The driver thread dies with the process. No stop and no postlude are sent. Releasing the USB handle writes no
    bytes (the transport has no `Drop` that writes).
  - **Can the controller finish the job? No, except for a job that fit entirely inside the run-ahead window.** The
    board can execute only what it already holds (≤ about 3 packets / 2 s, or one longer packet). The remainder
    was never sent.
  - The job therefore ends partway through, and MO is left open (no postlude).
  - A Frame Loop finishes its current pass, possibly leaving the red pointer lit.
  - HARDWARE:
    - whether the board executes the held packets to the end;
    - whether it closes MO or clears the ports by itself when the list ends;
    - whether it reacts to the USB handle closing;
    - what a later connect finds.

### (b) Hardware communication failure (USB error, timeout, cable pulled, board unpowered)

- A transfer that fails or exceeds `transfer_timeout_ms` (250 ms) is never retried. It becomes a sticky fault
  (`lmc_session.rs` `xfer_fault`, the "not retried" errors).
- `runtime.rs` `fault` follows every fault with the emergency stop, over the same link. If the link is gone, the
  stop report says **STOP NOT CONFIRMED** (`post_report`).
- **Can the controller finish the job? No.** The host stops streaming at the fault, and also tries to stop.
  - If the link is truly gone, the board is left with what it holds, as in (a2).
  - If the link still works (a transient error), the board receives the stop and the job is aborted.
- HARDWARE:
  - what the board does with its held packets when USB is pulled;
  - whether the board is powered from USB or on its own;
  - how often transient transfer errors occur on this PC's USB.

### (c) Controller or job execution failure

- **Host watchdog.** Progress means a READY/BUSY change or a completed transfer. If there is none for 1.5 × the
  queued estimate + 2 s, the host faults (`lmc_session.rs` `Watchdog`). A missing status reply is a protocol fault.
  Both are followed by the emergency stop (`runtime.rs` `fault`).
- **Can the controller finish the job? No.** The host aborts it by design once the board stops making progress.
- **Board-side failures.** A laser-source alarm, a galvo error or a board reset are **not visible** to the
  software: nothing reads them (see the model). They would appear only as a watchdog or transfer fault.
- HARDWARE:
  - which board or source failures exist, and how they show in the status word;
  - whether a slow but healthy board can trip the watchdog. The run-ahead estimate is the input to that.

### (d) Explicit operator Stop

- The Stop button and `laser.stop` / `laser.frameStop` trip the latch. The driver checks it between ≤ 20 ms slices
  and runs the emergency sequence, ending with a confirming read (`runtime.rs` `service_stop`, `stop_now`;
  `lmc_session.rs` `emergency`).
- The stop reports **Stopped**, or **STOP NOT CONFIRMED** (sticky fault).
- **Can the controller finish the job? No, by design: Stop aborts it.**
- HARDWARE: the time from the click to the beam off. The bench sessions measured parts of this; see HARDWARE.md.

### (e) Emergency Stop (the machine's E-stop / key switch)

- **The software has no E-stop input.** "E-stop" appears in the code only in messages that tell the operator to use
  it. The software learns of it only indirectly: a watchdog or transfer fault if the E-stop also removes the
  board's power or stops it answering.
- **Can the controller finish the job?** That depends only on the machine's wiring (what the E-stop cuts: the laser
  source, the galvos, the board). This is HARDWARE, not software.
- HARDWARE:
  - what this JPT M7 cabinet's E-stop and key switch cut;
  - what the software then reports (fault, watchdog, nothing).

### Hardware tests that would settle the HARDWARE items (for Luiz at the machine; agents never run them)

These measurements describe the model; they do not change anything. Have the E-stop in reach, goggles on and a scrap
plate in place. Use the lowest power, inside the 10 % first-emission cap. Film the work area and the controller LEDs
at 60 fps with a visible clock.

1. **(a2) during framing.** Frame Loop a 50 mm square, then run `taskkill /F /IM openlaser.exe`. Record whether the
   dot finishes the pass, then goes dark or stays lit as a still dot.
2. **(a2) during marking.**
   - Run a job of about 30 s and kill the app 10 s in. Record:
     - how long marking continues;
     - whether it ends at a packet boundary;
     - whether the fiber source still shows MO open or ready afterwards.
   - Repeat the kill between two layers.
   - Repeat with one long slow packet.
3. **(b).** Repeat steps 1 and 2 with the USB cable pulled instead of the kill.
4. **After each of steps 1–3.** Start OpenLaser again and connect. Record whether connect works and what the first
   status word shows (BUSY or READY).
5. **(e).** With the board idle and then framing, press the E-stop and turn the key switch. Record:
   - what goes dark (source, galvos, board LEDs);
   - what OpenLaser reports.
6. **Write the results in HARDWARE.md.**

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
- The camera panel's `ColorImage::from_rgba_unmultiplied` length assert (`ui-egui/src/panels.rs`, camera frames).
  It applies only with a camera.
