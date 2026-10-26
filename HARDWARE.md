# Hardware bring-up: OpenLaser on the JPT M7 MOPA

This page is for Luiz, at the machine. **No AI agent runs any of this.** It is the order from `docs/SPEC.md` §8
(items H0–H29 are explained in `docs/research/PLAN.md` §5.2). Tick each box and add a line to the log at the bottom.

## Rules for every session

- Close every AI agent session (Claude, Codex, Devin) before the USB cable goes in. While agents work, the laser's
  USB stays **unplugged**.
- Eyewear on. E-stop and key switch within reach of where you stand. Scrap material only. Stop after 2 hours.
- MacBook lid open; keep the terminal (or the OpenLaser window) focused: **Esc, Space or Ctrl-C stop the machine**.
- Start every session with `openlaser-hw bench start`. The software caps the first emission of a session at
  **10 % power**.
- Record a USB trace for every step (`--trace <file>`), and keep the files: they are how problems get diagnosed.

## 0. One-time setup (before the cable goes in)

- [ ] **Read and sign `docs/DIVERGENCES.md`.** It lists how the Rust bytes differ from the Python OpenLaser and why.
      Tell Claude when you sign; Claude then freezes the reference bytes (`tests/golden/rust-v1/`).
- [ ] **H0, E-stop and key switch.** With the source powered and the key on, press the E-stop: the source must
      power down (LEDs/fan off), not just the controller. Release, then turn the key off: same check. Write down what
      each one actually cuts.
- [ ] If you still have the old EZCAD PC: copy its `.cor` file and `markcfg*` files. They likely explain the 1.1834
      scale factor.
- [ ] Build the hardware binaries yourself, in a terminal with no agent running:

```sh
cd ~/AI/OpenLaser/openlaser && git pull
export PATH="$HOME/.cargo/bin:$PATH"
CARGO_TARGET_DIR=~/AI/OpenLaser/.target/hw cargo build -p openlaser-hw -p openlaser --features hw
alias olhw=~/AI/OpenLaser/.target/hw/debug/openlaser-hw
```

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
- [ ] **Long frame:** a large Frame Once that runs more than 8 s. Pass: it runs to the end (the old Python version cut
      long jobs at 3 s).
- [ ] **Orientation:** `olhw frame --builtin f-shape --trace hil1-f.jsonl`. Pass: the "F" reads correctly from where
      you stand. If mirrored or rotated: `olhw profile set cal.negate.x true` (or `.y`, or `cal.swapXy true`) and
      repeat.
- [ ] **Corners:** `olhw frame --builtin corners --extent 100`, then `--extent 110`. Pass: 100 mm traces, 110 mm is
      refused (work area ±100 mm on the 200 mm lens).
- [ ] **Crash test:** during a red frame, kill the program (`kill -9` from another terminal). Note what the board
      does.

## 4. HIL-2: first real burns (scrap, eyewear, start at 1 %)

- [ ] `olhw run --builtin dot --power-pct 1 --allow-emission --trace hil2-dot.jsonl` — summary → frame loop →
      Enter → type `ARM <code>`. Pass: one dot at the field centre; the software refuses anything above 10 % this
      session.
- [ ] Then `--builtin line`, `--builtin box10`, `--builtin square50` (low power). Pass: the 50 mm square measures
      50 ± 0.1 mm (compare with the Python version's).
- [ ] **Key switch mid-job:** during a low-power mark, turn the key off. Pass: emission stops immediately.
- [ ] **10 stops while marking** (Esc). Pass: dark within 250 ms each time.
- [ ] **≥ 30 s hatch job** (low power). Pass: it completes; it isn't cut short.
- [ ] **Business card:** `olhw run card.olp --allow-emission --trace hil2-card.jsonl` (`card.olp` is your OpenLaser
      business-card workspace, already converted).
- [ ] While `olhw` holds the board, try the Python OpenLaser: it must refuse ("another process holds the board").

## 5. HIL-3: the GUI

- [ ] `~/AI/OpenLaser/.target/hw/debug/openlaser`, choose the **JPT M7 · USB** machine, import the business card,
      Frame (human click), Start → ARM dialog (check the power, kHz, ns and actual speed shown) → mark. 20 STOPs from
      the GUI. One MOPA colour test.

## Log

| Date | Step | Result (pass/fail + numbers) | Trace file | Notes |
|---|---|---|---|---|
| | | | | |
