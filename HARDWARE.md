# Hardware bring-up: OpenLaser on the JPT M7 MOPA

This page is for Luiz, at the machine. **No AI agent runs any of this.** It is the order from `docs/SPEC.md` §8
(items H0–H29 are explained in `docs/research/PLAN.md` §5.2). Tick each box and add a line to the log at the bottom.

> **Status (2026-10-26):**
> - **What is approved:** the D1–D11 protocol changes, for **controlled hardware validation** only
>   (`docs/DIVERGENCES.md` § Sign-off). The reference bytes are frozen in `tests/golden/rust-v1/`. Before any physical
>   test, D7 (bounded stop latency) and D9 (no premature execution, no overrun) were verified in software.
> - **What is not established:** the implementation has **not** been declared hardware-safe. That is what these
>   sessions find out.
> - **Read before the cable goes in:** [Remaining safety risks](#remaining-safety-risks) and
>   [Stop the session if](#stop-the-session-if).

## Rules for every session

- Close every AI agent session (Claude, Codex, Devin) before the USB cable goes in. While agents work, the laser's
  USB stays **unplugged**.
- Eyewear on. E-stop and key switch within reach of where you stand. Scrap material only. Stop after 2 hours.
- MacBook lid open (closing it sleeps the Mac mid-job), on power, other heavy apps closed (stop timing depends on an
  idle host). Keep the terminal (or the OpenLaser window) focused: **Esc, Space or Ctrl-C stop the machine**.
- After every stop, copy the printed `stop …: … in N ms (MO closed: …, ports at 0: …)` line into the log.
- Start every session with `openlaser-hw bench start`. There is no software power cap: the job's own power
  applies from the first emission, so start low (`--power-pct 1`).
- Record a USB trace for every step (`--trace <file>`), and keep the files: they are how problems get diagnosed.

## 0. One-time setup (before the cable goes in)

- [x] **`docs/DIVERGENCES.md` signed** (D1–D11, 2026-10-26, controlled hardware validation only). The reference bytes
      are frozen in `tests/golden/rust-v1/`; a hardware trace of dot/line/box10/mark_box10 diffs against them.
- [ ] **H0, E-stop and key switch.** With the source powered and the key on, press the E-stop: the source must
      power down (LEDs/fan off), not just the controller. Release, then turn the key off: same check. Write down what
      each one actually cuts.
- [ ] If you still have the old EZCAD PC: copy its `.cor` file and `markcfg*` files. They likely explain the 1.1834
      scale factor.
- [ ] **Build the hardware binaries yourself, at home, before you leave** (no laser attached: compiling never touches
      a device; close agent sessions first). Nobody has compiled the `hw` feature yet — agents and CI are not
      allowed to — so a compile error must show up here, not at the machine. The first build takes 5–10 minutes and
      about 5 GB of disk.

```sh
cd ~/AI/OpenLaser/openlaser && git fetch && git checkout preflight-hil && git pull
export PATH="$HOME/.cargo/bin:$PATH" CARGO_INCREMENTAL=0
CARGO_TARGET_DIR=~/AI/OpenLaser/.target/hw cargo build -p openlaser-hw -p openlaser --features hw
alias olhw=~/AI/OpenLaser/.target/hw/debug/openlaser-hw
```

      If it fails, send Claude the error (it can fix the code; it cannot build `hw` itself).
- [ ] **Use the MacBook.** Every step here is written for macOS, where the board needs no driver. On a Windows PC the
      board needs the WinUSB driver (Zadig), which replaces EZCAD's driver: EZCAD stops seeing the laser until the
      old driver is restored in Device Manager. Don't do that on the company's EZCAD PC today.

- [ ] **Rehearse on the simulator first** (no laser, same keys and prompts):
      `olhw run --builtin dot --sim`, then `olhw run --builtin box10 --sim` (practise Enter, the `ARM <code>` prompt,
      and Esc).

## 1. Each hardware session starts like this

```sh
cd ~/AI/OpenLaser/hil            # traces and results land here; card.olp is already in it
export OPENLASER_HW_PRESENT=1; unset OPENLASER_NO_HW
olhw bench start
```

## 2. HIL-0: passive (nothing moves, nothing fires)

- [ ] `olhw detect`, then `olhw status --trace hil0.jsonl`
      Pass: the board is found, serial/version replies are recorded, no `Reset` or laser command is sent.
      If macOS refuses with `UsbNotConfigured`: `olhw profile set usbSetConfiguration true`, then `status` again.

## 3. HIL-1: red light only (key OFF if the red pointer works without it)

- [ ] **Box loop + stops:** `olhw frame --builtin box10 --loop --trace hil1-box.jsonl`. Stop it with Esc 10 times
      (restart in between). Pass: every stop ≤ 250 ms, no USB resets.
- [ ] **Long frame:** `olhw frame card.olp --trace hil1-long.jsonl` (the business card's Frame Once is 75 packets).
      Pass: it runs to the end (the old Python version cut long jobs at 3 s).
- [ ] **H-UR, underrun (red light only, never with `--allow-emission`):** start the long frame again; from a second
      terminal run `kill -STOP $(pgrep -n openlaser-hw); sleep 1; kill -CONT $(pgrep -n openlaser-hw)`. While the
      program is frozen, Esc does nothing: the key switch and E-stop are the stop.
      Note what the red pointer does during the freeze (holds still? keeps going? goes off?) and after it (resumes
      the outline? stops?). Then repeat with `sleep 5`: the program should report a watchdog fault and stop the board
      (if the freeze happened to land on a packet transfer it may carry on instead; note which).
      Pass: no USB reset, the stop report says MO closed and ports at 0, and the trace shows no write while not
      READY. If the outline does **not** resume after the 1 s freeze, write it down: real jobs could then end
      incomplete (risk 2 below).
- [ ] **Orientation:** `olhw frame --builtin f-shape --trace hil1-f.jsonl`. Pass: the "F" reads correctly from where
      you stand. If mirrored or rotated: `olhw profile set cal.negate.x true` (or `.y`, or `cal.swapXy true`) and
      repeat.
- [ ] **Corners:** `olhw frame --builtin corners --extent 100`, then `--extent 110`. Pass: 100 mm traces, 110 mm is
      refused (work area ±100 mm on the 200 mm lens).
- [ ] **Crash test:** during a red frame, kill the program (`kill -9` from another terminal). Note what the board
      does and for how long: this is what a crash mid-burn would do with the laser (risk 1 below).

## 4. HIL-2: first real burns (scrap, eyewear, start at 1 %)

- [ ] `olhw run --builtin dot --power-pct 1 --allow-emission --trace hil2-dot.jsonl` — summary → frame loop →
      Enter → type `ARM <code>`. Pass: one dot at the field centre.
- [ ] Then `--builtin line`, `--builtin box10`, `--builtin square50` (low power). Pass: the 50 mm square measures
      50 ± 0.1 mm (compare with the Python version's).
- [ ] **Key switch mid-job:** during a low-power mark, turn the key off. Pass: emission stops immediately.
- [ ] **10 stops while marking** (Esc). Pass: dark within 250 ms each time.
- [ ] **≥ 30 s hatch job** (low power). Pass: it completes; it isn't cut short.
- [ ] **Business card:** `olhw run card.olp --allow-emission --trace hil2-card.jsonl` (`card.olp` is your OpenLaser
      business-card workspace, already converted).
- [ ] While `olhw` holds the board, try the Python OpenLaser: it must refuse ("another process holds the board").

## 5. HIL-3: the GUI

- [ ] From the same terminal as the bench session (so `OPENLASER_HW_PRESENT=1` is set and Ctrl-C there stops the
      machine): `~/AI/OpenLaser/.target/hw/debug/openlaser`. The window title must read **"OpenLaser — REAL LASER
      (USB)"**; plain "OpenLaser" means it is on the simulator. Choose the **JPT M7 · USB** machine, import the business card,
      Frame (human click), Start → ARM dialog (check the power, kHz, ns and actual speed shown) → mark. 20 STOPs from
      the GUI. One MOPA colour test.

## 6. Rotary (next session, after HIL-2 passes)

D12 is **UNSIGNED, proposed** and is not covered by the D1–D11 sign-off. All axis units,
sign-magnitude replies, direction, acceleration, moving-bit timing and timeout assumptions remain unverified.
Rotary jobs and the human-only `rotary` CLI refuse until a human saves `rotary.enabled=true` in the profile
(default off). The job currently requires controller position **0** at start; it refuses a nonzero position
rather than change the exact absolute bytes approved at ARM. `rotary pos` shows the reported pulses.

- [ ] **R0 — key switch OFF.** After checking the profile's pulses/rev, diameter and conservative speeds,
      `olhw rotary move --deg 90 --trace rotary-r0.jsonl` (motion only, laser off). Check direction and pulses/rev;
      `olhw rotary pos` must show the expected sign-magnitude position. No homing command is sent.
- [ ] **R1 — one full turn.** `olhw rotary move --deg 360 --trace rotary-r1.jsonl`; the same physical mark must
      return to the same place. Check chuck object diameter versus roller drive diameter. Undo bench motion
      with the opposite degrees so the reported controller position is 0 before the job.
- [ ] **R2 — red light.** Frame a rotary job and compare the union of centred strip outlines with the intended
      tube layout; check receipt/ARM strip count, split, total degrees/pulses and return choice.
- [ ] **R3 — low-power scrap.** Burn a ring of text on a scrap tube, with framing and typed ARM. Check seams,
      overlap, direction, scale, rounding and return registration. Record the trace and stop timing.

**Residual risk:** no stop-axis command is known. StopExecute may or may not stop the motor. A stop closes
MO and ports with the unchanged emergency sequence, but **the rotary axis may finish its move**. A confirmed
StopReport confirms laser-off/list-idle, not motor-stopped; the report notes this explicitly. Keep hands clear
and use the physical E-stop/key switch. Never attempt a rotary bench move while any machine is active.

## 7. Foot switch (H11, SPEC M3)

The pedal does nothing until the profile names its input. Nothing assumes `foot_pin = 15`: the probe decides.

- [ ] **F0: measure (key switch OFF is fine: nothing moves or fires).** `olhw ports --watch`. Press and hold the
      pedal about 1 s, then release it, three or four times; Esc ends. The summary names the bit that moved and its
      rest level (active high or low), and prints the `profile set` lines to use. If two bits moved, watch again
      pressing only the pedal. If nothing moved, check which input connector the pedal uses (EZCAD's "Start
      Marking" port setting shows the pin).
- [ ] **F1: set it.** `olhw profile set footSwitch.bit <bit>` (plus `footSwitch.word <w>` if it was not word 1, and
      `footSwitch.activeLow true` if it rests at 1); `SAVE <id>`. Optional: `footSwitch.fullPulse false` (count on
      the press instead of press + release; default true, LightBurn's "Require full pulse"),
      `footSwitch.debounceMs <10..1000>` (default 30). `footSwitch.bit none` turns the pedal off again.
- [ ] **F2: GUI, red light.** Start the GUI with "Start OpenLaser with laser", Connect. The Laser panel shows
      "Foot switch (word w bit b): released"; pressing it shows "pressed". With no job, a press only prints why Start
      is not available.
- [ ] **F3: GUI, first burn (scrap, 1 %).** Frame the job, press the pedal: the ARM dialog opens (the first ARM of
      the session is never skipped). Tick every item, press the pedal again: it arms instead of typing ARM. After
      that, in the same session, a press on a framed job arms at once (like Start with "ARM once per session").
- [ ] **F4: stop wins.** During a job, press Stop (or Esc): the job stops and the pedal is blocked until you click
      Frame or Start once. A pedal held through the stop does nothing when released.

The pedal never bypasses anything: the job still has to pass every safety check, the frame rules of `mint` still
apply, the checklist still has to be ticked, and Stop, Esc and the E-stop always win. The CLI's `run` still arms
only on the typed `ARM <challenge>`.

## Stop the session if

- Any stop prints `MO closed: false` or `ports at 0: false`, or "no StopReport … use the key switch / E-stop". Key off
  first, then E-stop if needed.
- Any stop prints a note with `possible desync` / `H23`. Turn the key off and power-cycle the controller before
  anything else.
- A stop takes more than 250 ms, there is a USB reset or disconnect, or a watchdog fault you did not cause.
- Emission continues after Esc, or after the program exits.
- Anything lands somewhere other than where the frame showed.

Write it in the log and tell Claude. Do not retry the same step in the same session.

## Remaining safety risks

The software has been checked on a simulator, never on this board. These risks stay open until the sessions above
close them:

1. **Software stop is not a safety system.** The E-stop and key switch (H0) are the only stops that do not depend on
   the Mac, USB and the board's firmware.
   - **A crash keeps marking.** If the program dies mid-job (crash, `kill -9`, Force Quit, cable pull, Mac sleep),
     MO stays open and the board runs what it already holds. That is at most 3 packets and about 2 s by the time
     estimate, and one long slow packet can hold more. If the board runs slower than estimated, it is longer.
   - **Power refusals are software-only.** Every power check is a software check; there is no first-emission cap.
2. **Underrun behaviour is unknown (H-UR).**
   - **The board may run dry.** If the Mac stalls longer than the board's queue (a few hundred ms to 2 s), the board
     runs out of work mid-list. The software side is tested: no early write, no extra ExecuteList, and a watchdog stop
     after a long stall.
   - **Unverified: does the laser stay off?** We have not seen whether the real board holds with the laser off and
     then resumes. If it does not resume, a job can end incomplete without an error.
3. **A cancelled packet can desync the board (H23).** If a stop has to cancel a half-sent packet, the board may read
   later commands as packet data. Writes are gated on READY, so this needs a packet that takes more than 20 ms to send
   (normally about 2 ms), but it is unmeasured. That is why `possible desync` means a power-cycle.
4. **Stop timing is a simulator number.**
   - **Simulator results:** StopExecute ≤ 40 ms after Esc (≤ 80 ms with a cancelled packet), MO(0) ≤ 60 ms later,
     the whole stop < 100 ms.
   - **Under load:** under heavy CPU load the real-time test measured p99 up to about 210 ms.
   - **The bench decides:** the 250 ms pass mark is measured here, not assumed. Windows is untested.
5. **Board behaviour the software only assumes.** These items are still unverified on hardware:
   - H1 and H2: READY and BUSY semantics and the real buffer depth. The post-crash bound in risk 1 depends on them.
   - H3: mirror settle and first-jump timing.
   - H4: power, frequency and pulse width actually reaching the source.
   - H12: USB configuration on macOS.
   - H13: transfer timeouts.
   - H14: the abort variants.
   - H22: position feedback for the watchdog.
   - H25 and H27: field size and calibration (1.1834) and the safe work area.
6. **Agents.**
   - **The rule:** no agent may activate the laser. The `openlaser-cli`/MCP surfaces have no emission path (checked
     by `xtask deps`). In the GUI, an agent can at most request a frame, and that waits for your click. Only the human
     ARM dialog can start a burn, and synthetic input cannot ARM.
   - **What enforces it for Claude Code:** a hook blocks hardware builds and runs.
   - **What enforces it for Codex and Devin:** only `AGENTS.md`. The real guard is physical: close every agent
     session before the USB cable goes in, and build the `hw` binaries yourself.
7. **The GUI's camera.** Opening the camera runs on the UI thread and can freeze the window; it is disabled while
   framing or marking. Start the GUI from a terminal, so Ctrl-C there still stops the machine if the window hangs.

## Log

| Date | Step | Result (pass/fail + numbers) | Trace file | Notes |
|---|---|---|---|---|
| 2026-10-30 | HIL-0 `detect`, `status` | pass: 9588:9899 "20200507" USBLMCV4 found; GetSerialNo `0000 0007 0006 0220`, GetVersion `0000 8421 0009 0220`; status 0x0220 = READY, idle; 6 trace events in 2.6 ms, no Reset or laser command | `hil0.jsonl` | Bit 0x0200 set, not used by our code; `usbSetConfiguration` not needed on macOS |
| 2026-10-30 | GUI frame (red light) | fail → fixed: the frame was X/Y transposed against the canvas ("rotated 90° CW and mirrored" seen from the front) | none (GUI) | Fixed with `cal.swapXy = true` in the saved profile `machines/jpt-m7-200.json` (Galvo 1 is X Axis OFF); confirmed. The built-in default stays identity: the rust-v1 frozen bytes are identity-mapped |
| 2026-10-30 | GUI first burns | ARM dialog reached for C00 Line at 74.99 %, 2 passes; burn result not yet reported | none (GUI) | Bench cap refused 30 % on the first emission as designed. Field feedback → branch `phase3-field`: ARM once per session, framing optional, Disconnect, Device Settings save, marquee selection |
