# Judge: product and laser-safety review of proposals A, B and C

*2026-10-23. Point of view: Luiz's product and laser-safety advocate. The question is which plan gets a hands-on maker to safe, real results on the JPT M7 soonest, without boxing the project in, and how hard and verifiable each plan's safety guarantees are.*

*Evidence: research reports 01–06 and `rayforge-test/REPORT.md`. I also read the prototype at `~/AI/OpenLaser/OpenMopa` @ `6aed8ff` and the installed galvoplotter 0.2.0 (`.venv/.../galvo/controller.py`). Nothing was run against hardware and nothing was installed.*

---

## 1. Verdict

**Winner: Proposal A (galvo-first vertical slice), with specific grafts from B and C (§4).**

- **A reaches safe Rust MOPA output first, with no throwaway layer.**
  - A Rust CLI burn of Luiz's own designs arrives at about week 5 (M1.2).
  - A native GUI arrives at about week 8 (M1.3).
  - The prototype retires at about week 13 (M2).
  - These are focused weeks, and A itself warns that calendar time runs 1.5–2× longer.
- **A has the most verifiable safety story.** Each known prototype bug gets a named regression test. Each test is mutation-checked: revert the fix and the test must go red. The red-zone crates are designed by Opus and need T6 plus Luiz's review. The emulator runs the T6 suite, and red-light hardware sessions (HIL) start in M1.1.
- **B has the best long-term architecture but the worst time to value.**
  - The first Rust mark arrives only at the end of M2, after 12–17 weeks. The prototype is replaced only after 20–27 weeks.
  - M1's deliverable, byte-identical G-code export, is something Luiz already has.
  - B lists "platform over-build" as its own top risk, rated H/H.
- **C has the right instinct (earliest hardware truth) but spends 3 weeks on a throwaway PyO3 bridge.**
  - That bridge runs a Rust engine under a 3,000-line HTML-in-Python UI, with two engines and an engine flag in the safety path.
  - A's M1.2 CLI bridge delivers the same "design in Python, burn in Rust" value about two weeks sooner, with zero Python changes and no FFI.

---

## 2. Scorecard

1–10 per criterion. For technical risk, a higher score means lower risk.

| Criterion | A: galvo vertical slice | B: platform first | C: strangler + ecosystem |
|---|---|---|---|
| Time to real value (safe MOPA jobs from Rust) | **9** | 3 | 7.5 |
| Path to LightBurn-class parity without rewrites | 7 | **9** | 6.5 |
| Technical risk (higher = lower risk) | **7** | 5 | 6 |
| Solo + AI feasibility, scope discipline | **8** | 4 | 6 |
| Safety (hard guarantees, verification rigour) | **9** | 8.5 | 8 |
| Correctness of claims vs research | **8** | 8 | 7 |

---

## 3. Proposal by proposal

### A: galvo-first vertical slice

**Strengths**
- **Every crate is the final architecture, populated with only the galvo case.** No layer is thrown away later. `ol-app` has no egui dependency, so the Tauri Plan B costs weeks, not a rewrite.
- **The safety model is structural, not advisory:**
  - `EmitPlan`, `FramePlan` and `ArmToken` can only be constructed by `ol-safety`.
  - `FramePlan` has no power fields.
  - `LightingList` cannot express `MarkTo`, MO open or `LaserOnPoint`.
  - The ARM token is single-use, expires after 60 s, is bound to the job fingerprint and is invalidated by any edit.
  - Mark is refused unless a frame of the same fingerprint ran in this session.
- **Each known bug from 05 §0/§2 has a named test**, and §5 exit criterion 2 mutation-checks each one. No other proposal verifies its own regression tests.
- **The LMC session algorithm (§2.6) directly answers 05 §1.10:**
  - There is no deadline-then-proceed path.
  - The watchdog is progress-aware, and its only action is to stop.
  - MO is never closed while BUSY.
  - There is never an automatic USB reset.
- **The abort sends the dummy end-of-list packet only if READY returns within 50 ms.** This is the only proposal that avoids writing the 3,072-byte packet into a possibly full buffer during an abort, which is the storm trigger.
- **The stop budget has a watchdog.** After 500 ms with no confirmation, a "STOP NOT CONFIRMED — use the key switch / E-stop" banner appears, so software stop is never presented as the safety device (06 G12).
- **Layer parameters live inside separate galvo and gantry blocks.** That makes the REPORT.md leak (MOPA 300 mm/s at 100 % reaching a diode job) unrepresentable, not merely checked.
- **D12 says to read the EZCAD `.cor` and `markcfg` off the old PC now.** No laser is needed (03 §13), and this attacks the 1.1834 mystery early.
- **The HIL plan is ordered by risk to this plan:** H12, H2, H13, H14 and H1 on red light in M1.1. It has software gates (`--features hw`, `OPENLASER_HW_PRESENT`, `--allow-emission`) and commits a USB trace for every session.

**Weaknesses**
- **M1 is aggressive.** Weeks 1–2 hold the encoder, session, emulator, nusb transport and HIL-1. Weeks 4–6 hold the whole GUI slice: canvas, layers, import, project file and undo. Expect slip; the CLI bridge is the real M1 value, so let the GUI absorb the slip.
- **The CLI bridge (`ol mark --prototype … --mirror-x`) rebuilds the job** from `workspace.json`, `layer_settings.json` and mirror flags typed on the command line. Mirror lives only in browser `localStorage` (05 §1.7), so a forgotten flag gives a mirrored burn. The `/api/mark` payload already carries transformed and mirrored polylines (05 §4.4); consume that instead (graft from C).
- **Prototype patch 1 (`wait_ready`) is unsafe as specified.** See §5, item 1.
- **Prototype patch 6 (ARM) is client-side only.** The server still accepts `arm: true` from any client (`ui.py:2151`).
- **Hidden cost: no time estimator exists.** The wait-cap patch needs an "estimated duration", which the prototype cannot compute (02 App. A, PRV-03: distance only), so 1–2 days is optimistic.
- **The galvo/gantry split diverges from LightBurn's shared-parameter model.** That is fine for safety, but `.lbrn2` round-trip (M5/M6) needs an explicit mapping layer.
- **No disassembler, units newtypes or payload-decoded confirm are planned.** The confirm dialog renders the `EmitPlan`, not the bytes that will be sent.

### B: platform first

**Strengths**
- **It has the best parity foundations:**
  - a LightBurn-shaped `CutTable` with `Extras`
  - typed unit newtypes (`Hz`, `Ns`, `MmPerS`, `Percent`, …)
  - capability sub-traits instead of GRBL-shaped stubs
  - an explicit driver registry with a conformance suite
- **Its safety ideas are the strongest of the three:**
  - The confirm summary is **decoded from the encoded payload** (`Encoded.summary`). An encoder bug therefore shows up on screen before the burn.
  - **Cold rebuild plus hash compare at ARM**, so a stale cache can never burn old geometry.
  - **CLI ARM is refused when stdin is not a TTY.** That structurally stops an AI agent's shell, or a pipe, from arming emission.
  - **No API, script or LAN path can ever ARM** (the same stance as LightBurn REST, 02 §16).
  - A speed above the gantry maximum is **refused, not silently capped**.
  - `home_on_connect` defaults to false (REPORT.md: Rayforge auto-homed with no prompt).
- **Its prototype patch 1 is the only correct one.** Every normal wait gets a progress-aware deadline (estimate × 1.5 + 10 s, extended while status changes), and a timeout aborts. It never writes or closes MO.
- **Rule 5 is a useful discipline:** a trait needs two real implementations before it is frozen.
- B wants publishable `lmc-proto` and `lmc-sim` crates.
- B suggests keeping the source key off during red-light sessions (UNVERIFIED for the M7).

**Weaknesses**
- **Time to value is poor.**
  - M1 (6–8 weeks) touches no hardware, and its user value is G-code export that the prototype already produces.
  - The first Rust mark comes after 12–17 weeks.
  - Luiz keeps running the Python stack for 5–7 months.
- **Hardware truths arrive late.** H2, H13 and H14 come in M2, and H5 (`.cor`) in M5, even though reading the `.cor` needs no laser.
- **M1 contains work that A and C defer:** an incremental intent graph with counter-asserted recompute (E7), a lossless LightBurn document, and about 20 crate stubs. For a solo developer that is over-build, and B rates it H/H itself.
- **The muda native menu is a "must" spike gate.** The only precedent is eframe 0.36 with a patched egui (06 spot-check 5), so a polish item could force the Tauri Plan B.
- **`py-compat` sequencing lives in the shipping `ol-lmc` until M5.** That mode reproduces galvoplotter's sequencing, including the elided first jump (C's description of the mode).
- **The abort sends the dummy packet unconditionally** (§5, item 3), and the run-ahead bound is overclaimed (§5, item 7).

### C: strangler + ecosystem

**Strengths**
- **Hardware risk is retired earliest.** HW-1 (H2, H13, H14, H3, H12, H1, H4 on red light) and HW-2 (first Rust mark) both fall inside M1. C adds two useful items:
  - **H21:** do current and Q-pulse changes take effect mid-list? This gates the move to one list per job.
  - **H22:** can `GetPositionXY` serve as a progress signal?
- **Its golden anchoring is the most honest.** The oracle stays pinned at the hardware-proven `6aed8ff`. `py-compat` is a **test-only Cargo feature that never ships**, and every intentional difference is listed in a `DIVERGENCES.md` that Luiz signs.
- **The engine generates the confirm text**, so the UI cannot compose wrong values.
- **The ARM permit needs a `FrameReceipt`** for the same job hash, from this session, within 10 minutes.
- **A panic in the session thread triggers `emergency_stop()` through a drop guard.**
- **Faults are sticky and stops are not:** a USB error or unconfirmed stop stays latched until the operator acknowledges it.
- **The first emission in each session is at ≤ 10 % power.**
- **Its prototype ARM patch is server-side.** `/api/plan` issues a single-use 60 s nonce that `/api/mark` must echo.
- **The spike has a stop-key latency gate:** keypress to `stop()` must be ≤ 50 ms p99 under canvas load.
- **The CLI job input is the prototype's own plan payload**, already transformed and mirrored.

**Weaknesses**
- **M2 (PyO3 under the legacy UI) is throwaway work in the safety path:**
  - GIL handling, panics and threads inside an HTTP server process
  - two engines plus an `OPENLASER_ENGINE` flag
  - the wheel's whole purpose ends at M5
- **It avoids a two-language boundary the research warns against (01 §8.2 item 6) only by adding one.**
- **The native app arrives last:** M4 starts at about week 14 and the prototype is replaced at about week 27.
- **Its prototype first-jump patch changes hardware-proven bytes** (§5, item 2).
- **Its prototype `wait_ready` patch has the same hang as A's** (§5, item 1).
- **It has internal inconsistencies and overstatements** (§5, items 4–6).
- **It never states a remote-ARM policy** for the M7 `ol-daemon` LAN control panel.

---

## 4. Ideas the final plan must graft onto A

**From B**
1. **Confirm summary decoded from the encoded payload.** Add an LMC disassembler to `ol-lmc` (C has one too). The confirm dialog and the CLI summary render per-layer power, kHz, ns, *encoded* mm/s, bounds and time **from the packets that will be sent**, not from the plan.
2. **CLI ARM only on a TTY.** Refuse emission when stdin is not a TTY, and print a per-session nonce that must be typed. This turns "the agent never runs emission" from a procedure into a guarantee for Luiz's Claude/Codex workflow.
3. **Typed unit newtypes** (`Hz`, `Ns`, `MmPerS`, `Percent`, `GalvoUnits`) at least across `ol-plan`, `ol-safety` and the encoders. The kHz/Hz and µs/ns mix-ups already exist across OM, LightBurn and Rayforge (01 §3.1, 02 §9).
4. **"No remote ARM, ever" as written policy, plus a dependency lint:** no script, API or LAN path can reach the arm path.
5. **Refuse instead of silently capping gantry speed.** `gcode.py` uses `min(layer speed, machine max)` (05 §3.2), and a slower feed means more energy per mm. Record it as a documented T2 golden divergence.
6. **`home_on_connect = false`, and never move a gantry on connect.**
7. **Prototype patch 1 written B's way:** a progress-aware deadline for *every* wait, `wait_ready` included, and on expiry set the stop event *and* call `shutdown()`.
8. **Rule 5 (two real implementations before a trait freezes)**, and cold-build-plus-hash-compare at ARM once any incremental cache exists (M5 or later).
9. **The red-light HIL rule:** keep the source key off if the red dot works without it (verify on the M7).
10. **Publish `lmc-proto` and `lmc-sim` later** as the low-cost ecosystem contribution, instead of a Rayforge addon.
11. **Declare SVG import an intentional divergence:** true scale through usvg, not the prototype's fit-to-60 mm.

**From C**
1. **Pin goldens to the hardware-proven `6aed8ff`.** Use a **test-only** `py-compat` Cargo feature for byte-exact stream proof, with `DIVERGENCES.md` signed by Luiz. Keep A's oracle-v0/v1 diff as well.
2. **CLI bridge input = the exact `/api/mark` payload.** That payload is already transformed and mirrored, so nothing is reconstructed from flags.
3. **Server-side single-use ARM nonce** in the prototype patch, instead of A's client-only patch 6.
4. **Panic drop guard → `emergency_stop()`.** Sticky faults, non-sticky stop, no auto-reconnect mid-job.
5. **FrameReceipt with a 10-minute limit** on top of A's same-fingerprint frame requirement.
6. **First emission of a session at ≤ 10 % power.**
7. **H21 (mid-list parameter changes) and H22 (`GetPositionXY` progress)** in the HIL plan. Gate A's M3 merge to one list per job on H21 as well as H2.
8. **Stop-key latency under load as a hard spike gate**, together with A's S3.
9. **A first-run screen** saying the software is not a safety device and never overrides interlocks.

---

## 5. Factual errors and contradictions found

1. **A and C, prototype patch 1 (`wait_ready` raises `ConnectionAbortedError` on deadline).**
   - galvoplotter's `_spooler_run` returns on `ConnectionAbortedError` **without removing the job**.
   - `wait_for_spooler_send` loops `while self._queue: self._spooler_lock.wait()` with **no timeout** (`galvo/controller.py:182-260, 737-748`).
   - Outside a user stop, nobody calls `shutdown()`. The main thread therefore blocks until the 60 s or 600 s job timeout (`live.py:28`, `ui.py:2870`) while **MO stays open**, because `Fiber_SetMo(1)` is sent before the list (05 §1.6).
   - Neither proposal changes the 3 s deadline on `wait_ready`. Slow-draining packets would still be cut off; they are just aborted instead of overfilling (05 §1.10).
   - Fix: B's spec (graft 7).
2. **C (and B's first option), sentinel `_last_x`/`_last_y` for the first-jump patch.**
   - `list_jump` computes the `dist` word from `_last_x`/`_last_y` (`controller.py:1162-1170`). A `(-1,-1)` sentinel changes `mark_box10`'s first-jump `dist` from 1957 to about 44,385, and changes every job's first jump. The effect on hardware is unknown.
   - B asserts that `mark_box10`'s SHA `9ca1e12d…` "must not change". That holds only for the explicit-`list_jump` option, which is A's patch 2.
3. **B and C send the abort's dummy 3,072-byte packet unconditionally.**
   - galvoplotter polls `GetVersion` (`wait_ready`) before that packet (05 §1.9).
   - Writing while not READY is the reset-storm trigger (05 §1.10).
   - It also contradicts B's and C's own T6 invariant, "zero writes while not READY".
   - A's "only if READY within 50 ms" is correct.
4. **C overstates the Rayforge addon.** A nusb wheel does remove libusb, but 01 §3.1 notes that addons cannot ship pip dependencies into frozen, snap or flatpak builds. The PyO3 wheel is itself such a dependency, so the bundling blocker remains.
5. **C contradicts itself on two points.**
   - **Timing:** the TL;DR says "Rust engine within about 8 weeks", but M0 2 + M1 4(+1) + M2 3 adds up to 9–10.
   - **libusb:** the M2 exit says "installer no longer bundles libusb", yet legacy galvoplotter mode is kept for one release as the escape hatch. Legacy mode needs pyusb plus libusb (05 §1.1).
6. **C's T4 golden claims SVG and STL importer parity within 1e-9.**
   - OM's SVG import fits to 60 mm, approximates circles with 73 points and has no curves (05 §4.5).
   - C plans a usvg true-scale importer and no STL importer.
   - B correctly declares SVG an intentional divergence.
7. **B overclaims the run-ahead bound.** B says "at most 3 unexecuted packets … galvoplotter parity … a crash leaves only a bounded amount of marking".
   - galvoplotter gates later packets on READY, not on a count (05 §1.5).
   - The real bound is the board's unknown buffer depth (H2).
8. **B keeps `py-compat` inside the shipping protocol crate until M5.** C shows that it can be test-only.
9. **All three Python wait-cap patches assume an "estimated duration"** the prototype does not compute (02 App. A; PRV-03 "distance only"). Their 1–2 day, 3–5 day and 4 day estimates omit building a galvo time estimate in Python.
10. **Minor: B gates the spike on muda**, despite 06 spot-check 5. **B also schedules H5 (`.cor`) at M5**, although reading the `.cor` needs no laser (03 §13).

---

## 6. Recommended shape of the final plan (A plus grafts)

- **M0 (2 weeks)**
  - Install rustup and create the repo.
  - Patch the prototype using the merged spec:
    - waits: B's progress-aware version, with deadline → stop
    - first jump: A's explicit `list_jump`
    - ARM: C's server nonce
    - per-layer confirm rows
    - hidden + output layers refused
    - speeds refused below 1 unit or above the error threshold
    - orphan objects refused
    - pin galvoplotter
  - Validate the patches in one red-light session, then tag `oracle-v1`.
  - Goldens from both `6aed8ff` and `oracle-v1`.
  - T0 encoders and the emulator skeleton.
  - egui spike, with A's S1–S5 plus C's stop-under-load gate; muda is soft.
  - **Copy the EZCAD `.cor` and `markcfg` off the old PC.**
- **M1.1: Rust red light.** Encoder, session, emulator T6 suite and LMC disassembler. HIL-1 on red light, key off if possible: H12, H2, H13, H14, H3, H1, H4.
- **M1.2: first Rust burn.**
  - `ol mark` takes the exact `/api/mark` payload.
  - It prints the payload-decoded summary and requires TTY-only ARM.
  - The first emission is at ≤ 10 % power.
  - The FrameReceipt is required.
- **M1.3: egui slice.** The confirm dialog is rendered from decoded packets. The GUI absorbs schedule slip; the CLI bridge does not slip.
- **M2: retire the prototype (A's M2).** Add `.cor` read, pulled forward because it needs no laser, and the MOPA material test grid.
- **M3–M5:** as in A, with B's architecture rules arriving when their features do:
  - unit newtypes from M1
  - capability sub-traits when GRBL lands
  - cold-build-at-ARM when an incremental cache lands
  - LightBurn-shaped import mapping when `.lbrn2` lands
- **Rayforge:** no PyO3 addon. Publish `lmc-proto` and `lmc-sim` later.

**Unchanged for every option:**
- Software stop is never the primary safety device.
- No path proceeds after a timeout.
- Nothing is clamped silently.
- Agents never run emission. With the TTY graft, this is now enforced rather than just a rule.
