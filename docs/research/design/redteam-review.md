# Red-team review of PLAN.md

*2026-10-23. Adversarial review of `PLAN.md` (844 lines) against research reports 01–06, `rayforge-test/REPORT.md`, both judgements, and the prototype source at `~/AI/OpenLaser/OpenMopa` @ `6aed8ff`. Nothing was installed or built, and no hardware was touched. The only things run were three throwaway `python3` arithmetic checks (§6) and read-only `git log`/`grep` over the prototype.*

Citation style: `PLAN §x (line n)`, report sections as `05 §1.7`, prototype files as `OM/mopa_luiz/file.py:line`.

---

## 0. Verdict

The plan's overall shape is sound: galvo first, a CLI bridge before the GUI, byte-exact goldens, refusals instead of warnings, and no remote ARM. Its research grounding is unusually good. **But it is not ready to execute.** Three things must be fixed first:

1. Its "out of field" refusal bounds the galvo's **addressable** range (236.68 mm), not the machine's **rated** 200 mm field. T5 parity with the prototype locks that in.
2. The ARM-minting design is internally contradictory (a crate cycle). Its "enforced, not just stated" guarantee also fails under Cargo feature unification.
3. Python safety patch #1, which Luiz will rely on daily for about five months, is specified with no defined deadline before `ExecuteList` and no defined behaviour for FrameLoop. Appendix B says it is progress-aware; §11 does not make it so.

Beyond those, the stop path and crash containment carry several unverified hardware assumptions. M1's 7-week estimate is roughly half of what the listed scope needs, and several T0 and distribution items are missing.

---

## 1. Critical (must fix before M0 starts)

### C1. "Out of field" means out of the galvo's addressable range, not out of the lens's rated field

- **Claim.** PLAN §3.2 (line 229) encodes against the *effective* field F = `field × scale` = 236.68 mm. §3.6 layer 1 refuses "points outside the field". §4 draws "the field square at ±F/2". T5 (§6.1, line 519) requires "same decision" as the prototype.
- **Evidence.** The prototype bounds-checks against the effective field.
  - `geometry_errors` uses `limit = field_size_mm / 2.0 + 1e-6` (`OM/mopa_luiz/live.py:264-277`).
  - Every caller passes `effective_field_size(cfg)` = `FIELDSIZE × scale_correction` (`live.py:78-81`; `ui.py:2745, 2825`).
  - So OM accepts points out to **±118.34 mm** on a machine whose profile is `jpt-m7-200`, bed 200×200, centre origin (05 §3.1), with `FIELDSIZE=200`.
  - The calibration story (05 §1.7, README) says galvo full scale physically spans about 236.7 mm. The lens is rated for 200 mm.
- **Why it matters.** The plan's headline safety rule ("refuse out-of-field") protects u16 encoding, not the optics. A design placed 100–118 mm from centre is accepted, framed and burned outside the lens's rated field. The likely effects are spot distortion and defocus, and possibly clipping on the lens or scan-head aperture. Fiber sources are sensitive to back-reflection; the physical severity is UNVERIFIED. T5 parity makes this behaviour a *requirement*.
- **Fix.**
  - Split `GalvoMachine.field` into two things: the **encoding field** (F_eff, used only for unit conversion) and a **work area** (default: the rated lens field, 200×200, user-reducible). Refuse anything outside the work area.
  - List this in `DIVERGENCES.md`, so the T5 decision differs from v1 on purpose.
  - Add a red-light HIL item: frame the four corners at ±100 mm and at ±110 mm and look for clipping or vignetting.
  - Add the same refusal as a Python patch in §11. Luiz uses OM for another five months.

### C2. The ARM-mint design has a dependency cycle, and its "lint-enforced" guarantee does not hold

- **Claim.** §3.1 (line 190) says only `ol-ui`/`ol-cli` may enable `ol-safety/arm-mint`, and that "this is how 'no API, script or LAN path can ever ARM' is enforced, not just stated". The table (line 173) and the rules say `ol-safety` "may not depend on any encoder".
- **Contradiction.**
  - `ArmToken::mint(job: &Encoded, frame: &FrameReceipt, …)` (line 250) needs `Encoded`. `Encoded` is declared in the `// ol-lmc` block (line 258) and wraps `GcodeProgram` from `ol-gcode`. `FrameReceipt` is an `ol-driver` event type (line 300).
  - So `ol-safety` would depend on `ol-lmc` and `ol-driver`.
  - But `ol-lmc`'s `MarkingList` is "built only from `&EmitSection`" (line 253), so `ol-lmc` depends on `ol-safety`. `ol-driver` also re-checks the token, so it depends on `ol-safety` too.
  - Cargo rejects cycles, so the red-zone crate graph cannot be built as written.
- **Feature unification.** Cargo unifies a dependency's features across every package in one build. In `cargo build --workspace` or `cargo test --workspace` (what CI and the executors run), any crate that depends on `ol-safety` compiles with `arm-mint` on if `ol-ui` is in the build. That includes `ol-app`, and later a headless/LAN server crate (M6+). Such a crate can then call `mint`. The xtask rule only forbids crates that *enable* the feature from depending on servers; it does not stop a server crate from *using* `mint` in a unified build. `cargo test --all-features` turns it on everywhere.
- **Fix.**
  - Put the shared types (`Fingerprint`, `EmissionSummary`, receipt data) in a leaf crate.
  - Make `mint` take fingerprints and decoded summaries, not `&Encoded`/`&FrameReceipt`.
  - Move minting into a separate crate (for example `ol-arm`) whose **reverse-dependency set** xtask asserts is exactly `{ol-ui, ol-cli}`. Reverse dependencies are not subject to feature unification.
  - Also add clippy `disallowed-methods` for `ArmToken::mint` in every other crate.
  - Rewrite §3.1's "enforced" sentence to say what is actually enforced.

### C3. Python patch #1 (the daily-use safety fix) is ambiguous exactly where the original hang lived

- **Claim.** §11 #1 (line 723): "Every wait, `wait_ready` included, gets a deadline of estimate × 1.5 + 10 s **from `ExecuteList`**." Appendix B (line 824) claims the fix is a "progress-aware deadline".
- **Problems.**
  - **Before `ExecuteList` there is no deadline origin.** galvoplotter sends `ExecuteList` only after the 3rd packet (05 §1.5). `Fiber_SetMo(1)` and `WritePort(1)` are sent *before* the first packet (05 §1.6). So a board that never raises READY for packets 1–3 recreates the judges' "`wait_ready` hang with MO open" (JP §5 item 1), which Appendix B lists as fixed.
  - **The deadline is not progress-aware.** The §11 text never says it extends while status changes; B's original did (JP §3, B strengths).
  - **FrameLoop is undefined.** FrameLoop "returns False forever" and runs inside the server process (05 §1.6). With "estimate × 1.5 + 10 s from ExecuteList" it either has no estimate or is killed after a fixed time. Which one is unspecified.
  - **M0 validation (line 735) never exercises the patched *marking* wait path on hardware.** It checks only frame stops, a red-light job over 8 s and one dot. The ≥ 30 s low-power hatch exists only for the *Rust* driver in HIL-2. So the patched Python path that keeps MO open through long `wait_idle` calls ships to daily use untested on hardware.
- **Fix.**
  - Define a per-wait, progress-aware deadline: the clock starts when the wait starts and resets on any READY/BUSY change. The budget is `est(queued packets) × 1.5 + 2 s`.
  - FrameLoop gets a per-packet deadline based on that packet's estimated drain time.
  - On expiry: set the stop event, then the patched `abort()`.
  - Add "a ≥ 30 s low-power hatch completes, and the trace shows MO(0) only after BUSY clears" to M0 step 3 before tagging `oracle-v1`.

---

## 2. Important

### I1. The stop path's first move (`cancel_all` on a list write) may desync the board's command framing

§3.5 (lines 327, 345) and 06 C16 assume that after `cancel_all` "a partial packet landed" and that `StopExecute` + `ResetList` fix it. Nothing says how the board delimits a 3072-byte list transfer from a 12-byte single. If the board fills its 3072-byte buffer by byte count, the next singles (`StopExecute`, `Fiber_SetMo(0)`) are swallowed as list-slot bytes, and the stop silently fails. Short-packet termination would avoid this; the actual firmware behaviour is UNVERIFIED. FakeLmc's "partial packet after a cancel" fault models behaviour nobody has measured.

List writes are only submitted when READY, and at most 3 are queued. A queued 3072-byte transfer normally completes in a few milliseconds. Prefer **letting an in-flight READY-gated packet complete (bounded wait, around 20 ms)** and cancelling only packets not yet started. Add a new HIL item (H23: behaviour after a cancelled partial list write, red light only).

### I2. Stop latency is measured from the USB trace, not from light, and H14's key variant can't be judged that way

M1 exit #3 (line 632) measures "request → `WritePort(0)` done" from the trace. H14's question, whether MO(0)-first stops emission faster (line 350), is about *photons*, which a USB trace cannot show. The UI leg (key → latch) is measured only in the synthetic spike (S3), never on the finished M1.3 GUI or on hardware.

**Fix:** a ~$5 photodiode on a scope or Arduino, or at minimum 240 fps phone video of the red dot plus a screen-visible key press. Repeat S3 on the real M1.3 build as an M1 exit criterion.

### I3. Crash containment covers only unwinding panics

The panic guard and dead-man rule (§3.5, line 354) need Rust destructors and channel drops to run. They do not run on:
- `abort` (double panic, `panic=abort`, or a panic crossing AppKit FFI);
- a wgpu or Metal driver segfault;
- OOM;
- SIGKILL, or a Force Quit;
- SIGHUP when the CLI's terminal closes;
- a USB cable pull.

In those cases MO stays open, `WritePort(1)` stays set, and the board drains its queue. The run-ahead cap is in **packets**, not time (line 223). Three packets of 256 slow `MarkTo` segments can be minutes of marking. The plan also has no policy for **macOS sleep or App Nap**: closing the MacBook Air lid or idle sleep mid-job suspends USB, and App Nap can coalesce the driver thread's 10 ms timers.

**Fix:**
- Set `panic = "unwind"` explicitly and install SIGTERM/SIGHUP/SIGINT handlers that trip the latch.
- Hold a power assertion (`IOPMAssertion`, or spawn `caffeinate -i -w <pid>`) and an `NSProcessInfo` latency-critical activity for the whole job.
- Cap run-ahead by **estimated seconds** (for example ≤ 2 s of queued emission), not packets.
- Add a HIL item: SIGKILL `ol` during a red-light frame and record what the board does.
- Consider running the driver as a small separate process, so a GPU crash in the GUI cannot take the latch-holder with it.

### I4. UI-side STOP availability has holes the S3 spike won't see

- `rfd` file dialogs are synchronous on macOS and block the egui main thread, so Esc and STOP are dead while an Import or Save dialog is open during a job. Use `AsyncFileDialog`, or disable dialogs while emitting.
- The Esc key reaches OpenLaser only while it has focus. The plan should say so, in the UI and in `HARDWARE.md`.
- macOS live-resize runs a modal loop on the main thread.

### I5. First `JumpTo` with an unknown mirror position can fire while the mirrors are still moving

§3.5 assumes the start position is `(0x8000,0x8000)` "until H3". The explicit first jump therefore gets a `dist` computed from the assumed point, and `dist` = 0 for designs that start at the centre (the Test Dot). That jump also gets the short jump delay (8 µs, which 05 §1.8 itself flags as suspicious), then a 100 µs laser-on delay. If the mirrors were actually parked off-centre after the previous job or an abort, `MarkTo` can open the beam mid-travel and streak.

**Fix:** until H3 and H19 answer, open every list with `JumpTo` using `dist = 0xFFFF` (or the GP "long" 200 µs delay) plus an explicit settle `DelayTime`. Record this in `DIVERGENCES.md`; it changes bytes, so do it in the Rust path only and keep `py-compat` parity.

### I6. The red-pointer offset and scale are missing, so the frame may not show the real burn location

02 GDEV-04 lists "Red Dot: Offset X/Y, Scale X/Y" as **T0** ("matters for framing accuracy"). The plan's `GalvoMachine` (lines 213-224) has no red-dot calibration. The "Frame ⊇ burn" guarantee (§3.6 layer 3) is therefore proven in galvo units only. A pointer that is not coaxial with the fiber beam makes Luiz frame in one place and burn in another.

**Fix:** add `red_dot: {offset, scale}` to the profile, a HIL item to measure pointer-vs-burn offset (mark a cross at ≤ 10 % power, then frame it), and apply it to `LightingList` only.

### I7. The physical backstop the plan relies on is never verified

The plan repeats that "the key switch and E-stop remain the real backstop" (lines 331, 352, 368), yet no HIL item checks:
- that the M7 has an E-stop at all;
- that the E-stop cuts the **source**, not just the controller or galvo;
- that the key switch interrupts emission mid-mark;
- that both are reachable from where Luiz stands at the Mac.

Add HIL-1 item 0: red light, then ≤ 5 % on scrap, then hit the E-stop and the key switch mid-job. Record what stops.

### I8. Mirror and axis transforms can be applied twice on the M1.2 bridge

Patch 9's export carries "sha256, `machine_id`, effective field and `scale_correction`" (line 731) but **not** the mirror flags. The browser bakes `mirrorPolylines(…, mx, my)` into the payload (`OM/mopa_luiz/ui.py:2118-2140`). The Rust `GalvoMachine` meanwhile has its own `mirror` and `cal` (swap/invert/offset) (lines 216-217), and H4's result will be written there. If H4 reveals the machine needs `invert_y` and Luiz has been compensating with the browser's "Mirror vertically", every exported job is flipped twice.

**Fix:** export `mirrorX`/`mirrorY` (and `selectedOnly`) in the job JSON. `ol run` applies **no** profile mirror or invert to `--job` input, and refuses unless the export's flags match the profile's. Also specify `live.repeat_count` (1–999, honoured by `/api/mark`, `ui.py:2807-2833`): either honour it and show it in `EmissionSummary`, or refuse a value above 1. Silently ignoring it changes what gets burned.

### I9. The "never skip" scope rule contradicts the hatch golden and the HIL-3 business card

§2 (line 151) says "Refuse; never clamp, truncate, **skip** or 'proceed anyway'". T3 (line 517) requires "969 segments, **67 open shapes skipped**", and the business card is the HIL-3 part 1 exit job (line 624). OM also silently skips an empty raster step (`"raster_skip"`, `ui.py:2849-2858`). Either the business card is refused under the plan's own rule, or the rule is false.

**Decide:** open shapes in a Fill layer are a refusal, with a one-click "close within 0.05 mm" or "move to a Line layer" fix, or else an **explicit, listed notice** in `EmissionSummary`. Then fix T3 or §2 to match.

### I10. The field-edge encoding contradicts "never clamp"; report 05's "dead clamp" claim is wrong

- The plan's formula (line 229) is `round_ties_even(0x8000 + v·0xFFFF/F)` with no clamp.
- At v = +F/2 this is exactly 65535.5, which rounds to **65536**. I checked this at F = 200 and F = 236.68, and also at +F/2 + 1e-6 (§6).
- OM's `mm_to_galvo` **does** clamp (`live.py:250-254`), so the reference value "(+F/2) → 65535" (05 §1.7 table) depends on that clamp. 05 §1.7 calls the clamp "dead code in practice"; it is live at the + edge.
- In Rust, `as u16` saturates silently (a hidden clamp), while `try_from` refuses a point exactly on the edge (a T0 mismatch).

**Decide and document:** saturate the half-unit at +F/2 as a named, tested exception, or move the accepted bound inside by half a unit. Make the T0 ±F/2 cases normative.

### I11. D9 "snap speed to encodable values" has a floating-point trap that can double energy per mm

If the snapped speed is stored as a float `u / k` and re-encoded with `trunc(v·0xFFFF/F/1000)`, it drops one unit in about 13 % of cases at non-default fields. In about 29 % of random field sizes, the minimum (1-unit) speed re-encodes to 0 units (§6). The default F = 236.68 happens to be clean, so tests at the default won't catch it. It will appear when `.cor` work changes F in M2/M3. At low speed, losing a unit is large: a 2-unit speed (7.22 mm/s) becoming 1 unit (3.61 mm/s) **doubles energy per mm**. Confirm-from-bytes would show it, but only if Luiz reads it.

**Fix:** store speed as integer units in `GalvoCutQ`, or snap to (u + 0.5)/k. Add a proptest over F ∈ [100, 300].

### I12. Python is not patched for the G-code default leak REPORT found

REPORT takeaway 1 shows OM exporting **F15000, `M4 S1000`** (MOPA defaults, 100 % power) for the diode gantry. The plan uses this as the rationale for separate galvo/gantry blocks in Rust (§2) but freezes OM with only §11's patches. So OM's G-code export keeps leaking 100 % power into gantry jobs until M2. Add patch 10: per-machine-kind layer defaults in OM, or refuse G-code export when a layer still carries a fiber default (`frequency_khz`/`pulse_width_ns` set and power 100 %). Otherwise, disable OM's G-code export and say so.

### I13. Rayforge is endorsed for three real gantries on the strength of a fake-GRBL Telnet test

§0 (line 40) says "Use it for the CO₂, diode and plotter until M4, with auto-home turned off". REPORT tested only `fake_grbl.py` over TCP: no serial port, no real planner, no pen dwell, no raster. The plan dismisses Rayforge for galvos partly *because* its safety is advisory, yet:
- its out-of-bounds dialog always offers "Proceed";
- a failed check proceeds silently (01 L311);
- it auto-reconnects (REPORT);
- it homes on start by default.

The CO₂ machine is the biggest fire risk Luiz owns. Rayforge stop latency on real hardware is unmeasured (06 §5.10).

**Add a per-machine bring-up checklist:**
- `$32=1`; `$30` matches S-max; soft limits (`$20=1`) as the real bounds backstop;
- `home_on_start` off;
- frame with laser off;
- stop at low power with a timed video;
- first job at minimum power.

Also ask what Luiz uses for the gantries **today** (missing decision, related to D12).

### I14. The TTY gate is defeatable in Luiz's actual setup, and AGENTS.md is the wrong file for Claude

§3.6 (line 374) concedes that an agent driving a real terminal can ARM, then relies on an `AGENTS.md` rule.
- This environment exposes a terminal tool that runs commands in the user's Terminal tab (a real PTY) and a tool that reads that terminal's output. An agent can run `ol run`, read the printed nonce and type `ARM <nonce>`. Computer-use tools can do the same in the GUI.
- Claude Code loads `CLAUDE.md`; whether it also reads `AGENTS.md` is UNVERIFIED. Codex and OpenCode read `AGENTS.md`.
- Red-light commands (`ol frame --loop`, `ol hw` red items) need no ARM at all. Executors run on the same Mac the laser is plugged into.

**Fix:**
- Put the rule in `CLAUDE.md` too, or import it with `@AGENTS.md`.
- Add a PreToolUse hook (Luiz already has hookify) that blocks `ol (detect|status|frame|run|hw)`, `--features hw` and `OPENLASER_HW_PRESENT`.
- Have `ol` refuse *any* device I/O unless a human-created, short-lived "bench session" file exists.
- Longer term, require a physical ARM input: a foot switch on the board's input port once H11 confirms it.
- Rule of practice: unplug the board's USB while executors run.

### I15. M1 effort is roughly half of what its scope needs

Proposal A's M1 was 6 weeks, and both judges called it "overloaded" (JE §3, JP §3). The final plan grafted on about 15 items and added one week. Those items include:
- the disassembler and `EmissionSummary`;
- a `py-compat` mode replaying GP sequencing byte-exactly;
- a FakeLmc with virtual clock, faults, emission recorder and renderer;
- T6 at 1,000 runs per PR plus a real-time variant;
- `FrameReceipt`, TTY ARM, unit newtypes and mutation checks;
- kittest, and a zip project format with migrations and undo.

Specific problems:
- M1.2 ("week 4; this checkpoint does not slip") must build `ol-plan` (including the T3 hatch port, since the business card payload has raster layers), all of `ol-safety`, the interactive raw-mode CLI and a hardware session in **one week**. A checkpoint that "does not slip" is a wish, not a plan: if HIL-1 shows READY/BUSY semantics differ from GP's docs, the session code is redesigned.
- The red-zone review gate is "Luiz's review" (§6.4), but no Rust toolchain has ever been installed on this Mac. A behaviour-level review (traces, T6 results, a second independent model review) is needed, not line review.

**Realistic:** M0 3–4 focused weeks; M1.1 4–5; M1.2 2–3; M1.3 4–5. So M1 is about 10–13 focused weeks and the first Rust burn lands around February–March 2027, not December. That falls over the holidays and alongside Luiz's other active projects. Either re-baseline or cut M1 scope (for example, move the zip project format, undo and kittest snapshots to M2).

### I16. D7's hardware time does not match the sessions scheduled

D7 allows "five supervised sessions of 2 h or less across M0–M2". The plan schedules at least six or seven:
- M0 validation;
- HIL-0;
- HIL-1;
- HIL-2;
- HIL-3 part 1 (M1.3);
- HIL-3 (M2);
- plus "any stop-path change needs a hardware re-run".

HIL-1 alone (H2 buffer sweep, H13 deliberate full-buffer write with possible power-cycle recovery, H14 abort variants with 10 stops, H1, H22, H3, H4) is first contact for a brand-new driver and will not fit in 2 h. Budget 8–10 sessions, and make HIL-1 two sessions.

### I17. Several exit criteria are not testable, and one contradicts the next milestone

- **M1 #9**, "Luiz signs off that MOPA work no longer needs the Python app": this is subjective. It also conflicts with M2, whose exit is "2 weeks of real work without Python; OM archived" and during which "the prototype stays the design front end". M1 excludes things OM has today: text (`FONT_5X7`, `live.py:356`), first-burn test patterns (`/api/test/run`), G-code export and repeat count. Make #9 a checklist of named OM workflows.
- **M3** "dithered photo on anodised plate" has no metric. **M4** "Rayforge compared on the same emulator" is not pass/fail. **M5** "imports correctly" is undefined.
- **M1 #1** omits T4 (DXF import), although DXF import is in M1 scope.

### I18. CI cost, private versus public repo, and a dangerous cost-saving temptation

M0 creates a **private** repo with macOS as the primary CI runner. It runs nightly proptest (10k), fuzzing (10 min × 6+ targets), `cargo-mutants` and T6 at 1,000 runs per PR. GitHub bills private-repo macOS minutes at about 10× (the free tier is 2,000 minutes, roughly 200 macOS minutes a month; verify current pricing). That will run out within days.

The obvious workaround, a self-hosted runner on the M2 Air, would execute PR code **on the machine wired to the laser**. Forbid it in writing.

The M2 "GitHub Releases update check" and the signed DMG also assume a public repo. **Decide when the repo goes public** (this also affects D4). Until then, run heavy jobs on Linux only.

### I19. Windows distribution has unexamined blockers

- **Azure Artifact Signing** (line 686). Microsoft's public eligibility has been limited to organisations with history in the US, Canada, EU and UK, and to individuals in the US and Canada (UNVERIFIED for 2026). Luiz is likely outside those (the plan itself plans for Portuguese input). Fallbacks: Certum's open-source code-signing certificate, an OV certificate on a cloud HSM, or unsigned with SmartScreen friction. Decide this before M4.
- **libwdi** is LGPL-3. "Run it as a separate process" does not exempt the helper exe that *links* it: static linking still triggers relinking obligations (and conflicts with the plan's own "deny static LGPL"). Ship the helper with libwdi as a DLL plus a source offer, or ship upstream `wdi-simple` unmodified.
- libwdi installs a self-generated root certificate. Expect antivirus and SmartScreen flags, and document it.
- M4's exit needs a Windows PC and a Linux box physically connected to the board, but D11 only says "if you have one".

### I20. Python and Rust both touch the board during the bridge phase, with no cross-process exclusivity

From M1.2 to M2, OM is "the design front end" while `ol run` burns. OM's FrameLoop opens USB in the server process, and GP's error path does `close()` → `device.reset()` → `open()` up to 5 times (05 §1.1). libusb on macOS opens with "seize" semantics. Whether a stray OM frame, or `/api/board` (which opens USB, 05 §4.4), can reset the device under a running `ol` job is UNVERIFIED.

**Fix:** a shared lock file that both OM (patch 11) and `ol` honour, plus a HIL check that a second opener fails cleanly.

---

## 3. Minor

1. **Tauri fallback vs the safety model.** §4 prices Tauri at "3–4 weeks, not a rewrite". Under Tauri, though, ARM and STOP live in WebView JavaScript behind IPC: a scripting path, extra latency, and S3 untested. The fallback must re-pass S3 and define how a WebView ARM squares with "no script path can ARM".
2. **`FrameReceipt` on early Enter.** The CLI issues the receipt when Enter ends the frame loop (line 619), even if no full pass completed. Require ≥ 1 complete traced pass (packets acked through the end of pass 1). Also, `require_frame_before_mark: bool` (line 211) and D8 "on by default" make a §3.6 *hard* layer a profile toggle. Either remove the toggle for galvos or gate it like ARM.
3. **Single-command timeout vs 20 ms slicing.** `single(…, timeout)` is 250 ms (line 320) while "no wait lasts longer than 20 ms". A GetVersion read in flight when STOP arrives can delay `StopExecute` by up to 250 ms, the whole budget. Make the IN read sliced and cancellable, and send `StopExecute` without waiting on it.
4. **"Frame ⊇ burn" property wording.** "Every emitted segment lies **on** a traced contour" (line 367) is false for Fill hatches. Say "within the even-odd region of a traced closed contour".
5. **`.cor` plus `scale_correction` double-scaling (M2/M3).** Make "uploaded `.cor`" and a non-1.0 `scale_correction` mutually exclusive unless calibrated together. Bound-check `.cor` displacement tables before upload, and red-light-frame the corners after.
6. **SVG true scale.** usvg's px → mm depends on a DPI assumption (96 vs 72; 04 §4 warns). A unitless SVG mis-scales by 1.333×. Refuse or ask when the root has no physical units.
7. **GRBL M4 preflight.** Refuse to stream unless `$32=1` for laser profiles, `$30` matches S-max, and `$130/$131` ≥ the profile bed (05 §3.3 item 10).
8. **H13 needs a bypass.** It must write while READY = 0, which the driver forbids. Specify a separate raw-transport test binary that is never compiled into `ol`. Also, HIL-0 "passive" must not run the full init (which includes `EnableLaser` and `Reset`); only GetSerialNo and GetVersion.
9. **Provenance of OM.** 9 of OM's 25 commits (+8,365 lines, including the stop fix `de9f478` and the refusal fix `64f4b1b`) are authored by `Manufatu <info@manufatu.com>`, while `LICENSE` says "© 2026 Luiz Bueno". Confirm that Manufatu is Luiz, or get an assignment. Because OM is MIT, keeping the MIT notice is enough in any case; "Luiz can relicense" is unnecessary.
10. **`.cor` reader source.** The plan ports it from MeerK40t `balormk`, whose freedom from balor-derived code is UNVERIFIED (06 §5.3, 03 §13). galvoplotter already has `.cor` readers (`controller.py:864-919`, MIT, cited in 05 H5). Prefer them, plus corfile2meerk40t's SPEC.
11. **Rayforge framing.** "The galvo issues were closed as *not planned*" (line 32) reads as a maintainer decision. They were closed by a **stale bot**, and PR #288 was written by the maintainer himself (01 L30, L275). That makes upstream galvo support plausible later. Add "Rayforge ships galvo" to "what would change the answer", and watch it.
12. **LightBurn 1.5.04 is installed.** Ask whether Luiz holds a LightBurn licence that covers galvo (the pre-2024 Galvo licence). It affects D1 ("buy LightBurn Pro") and offers an EZCAD-free reference for H6–H9 and `.lbrn2` fixtures (06 G10).
13. **Internal inconsistencies.**
    - "Seven known safety bugs" (line 120) vs "eight safety fixes" (line 65) vs nine bug rows covered.
    - "HIL-1 (items 1–8)" (line 612) includes HIL-0's H12.
    - "HIL-3, part 1" sits in M1 while "HIL-3" is M2.
    - M0 tells Luiz to copy files "off the original EZCAD PC" while D11 treats owning one as conditional.
    - The T2 "byte-exact" G-code goldens must run with M2's new ordering, joining and dedupe *off*. Say so.
14. **Linux udev rule.** The reused `99-openlaser.rules` grants `MODE="0666"`, so any local user or process can fire the laser. Use `TAG+="uaccess"` with `MODE="0660"` and a group.
15. **Duplicate-crate drift check covers only egui/wgpu (S4).** Extend `cargo tree -d` to `kurbo` (shared by `ol-doc`, usvg 0.48, parley and vello) and `raw-window-handle`.

---

## 4. What the plan gets right (keep it)

- Hardware truth first: H12 → H2/H13/H14 on red light before any GUI.
- A single owner for the USB handle, no automatic USB reset, no proceed-on-timeout, a stop-only watchdog, and no MO close while BUSY. These answer 05 §1.10 precisely.
- The dummy packet sent only if READY, which is the only correct variant (JP §5 item 3).
- The confirmation decoded from the bytes, typed `LightingList`/`MarkingList`, and a mutation-checked regression test per known bug.
- `py-compat` as a test-only feature, a v0/v1 oracle pair, and a signed `DIVERGENCES.md`.
- The licensing hygiene: galvoplotter MIT with headers, never `clone_loader.py`, balor and GP30 facts only, raygeo vendored without its allocator.
- egui_tiles instead of egui_dock (06 C9 respected); nusb over rusb; no USB entitlement needed outside the sandbox.
- The Rayforge verdict for the MOPA is correct and well evidenced (01 §2.6, 06 §0.1). The hands-on test Luiz asked for is written up in REPORT and used.

---

## 5. Hardware items to add (all red light unless noted)

| New | Question | Gates |
|---|---|---|
| H0 | Does the E-stop cut the **source**? Does the key switch stop emission mid-mark? (≤ 5 % on scrap) | Every emission session |
| H23 | Board behaviour after a cancelled partial 3072-byte write: are the following singles honoured? | Stop path (I1) |
| H24 | Optical stop latency (photodiode or 240 fps video): `StopExecute`-first vs MO(0)-first | H14 verdict, M1 exit #3 |
| H25 | Board behaviour on host death (SIGKILL) and on a cable pull mid-frame | Crash policy, run-ahead in seconds |
| H26 | Red-pointer offset and scale relative to the burn | Frame ⊇ burn on hardware (I6) |
| H27 | Corners at ±100 mm and ±110 mm: vignetting or clipping | Work-area bound (C1) |
| H28 | Is `0x8002` mid-list a NOP or end-of-list? | Time-bounded run-ahead |
| H29 | Does a second process (OM) opening the device disturb a running `ol` session? | Bridge phase (I20) |

---

## 6. Checks I ran

All of these are plain arithmetic in `python3`. No project code and no device was involved.

- **Field edge.** For F ∈ {200, 236.68}, `0x8000 + (F/2)·0xFFFF/F` = 65535.5, and `round()` gives **65536** in both operation orders. At +F/2 + 1e-6 it is 65535.5003, also 65536. At −F/2 it is 0. Over 10⁶ random in-field floats, OM's `x*(0xFFFF/F)` and the plan's `x*0xFFFF/F` never differed.
- **Speed snapping.** Snapped speed `v = u/k`, re-encoded with `trunc(v·0xFFFF/F/1000)`. At F = 236.68 there were 0 failures for u = 1..6000. Across 37 field sizes from 100 to 300 mm, 94,777 of 739,963 (u, F) pairs re-encoded one unit low. u = 1 re-encoded to 0 in 5,738 of 20,000 random F.
- **Prototype facts.** I read `OM/mopa_luiz/live.py:78-81, 240-345` (field bound uses the effective F; `mm_to_galvo` clamps) and `ui.py:2118-2150` (the payload bakes in mirror and carries no flags), `ui.py:2595-2660, 2800-2880` (`repeat_count` honoured; `raster_skip`; test-dot power 1 %), and `ui.py:2938-2951`. I also read `git log` authorship (Luiz Bueno 16 commits, Manufatu 9).
