# Judgement: engineering risk review of proposals A, B and C

*2026-10-23. Reviewer stance: skeptical principal engineer. I weighed technical risk, whether one developer with AI agents can do it, scope discipline, rewrite risk, crate maturity, testability, and whether each claim matches the research. I read all three proposals in full, plus reports 01–06 and `rayforge-test/REPORT.md`. I spot-checked claims against the prototype (`~/AI/OpenLaser/OpenMopa`) and the installed galvoplotter 0.2.0. Nothing was installed or run, and no hardware was touched.*

---

## 1. Verdict

**Winner: Proposal A (galvo-first vertical slice), with targeted grafts from B and C (§5).**

A puts the riskiest unknowns first: LMC flow control, the stop path and the READY/BUSY semantics (05 §7, H2/H13/H14). It also gets Luiz onto a safer engine soonest, about 5 focused weeks to the first Rust-driven burn. It does this without a throwaway bridge (C) and without months of hardware-free platform work (B). Its crates are the final architecture, filled in only as far as the galvo case needs. The UI-free `ol-app` keeps the Tauri fallback cheap.

B has the best structural safety ideas and the most credible route to LightBurn parity. It is still the wrong *order* for a solo developer. The first Rust emission arrives after 12–17 focused weeks, the prototype replacement after 20–27, and the hardware semantics everything rests on (H2/H13) are only measured *after* the platform is built.

C rightly puts risk first, and its M1 is about as good as A's. Its M2 PyO3 strangler, though, spends about 3+ weeks putting the Rust engine inside a Python `ThreadingHTTPServer` (`ui.py:2996`) that is archived at M5. That brings back the two-language boundary 01 §8.2.6 warns against, and it removes the prototype's per-job process isolation (the SIGKILL backstop, 05 §1.9 item 6).

## 2. Scores (1–10; for technical risk, higher means *lower* risk)

| Proposal | Time to real value | Path to parity | Technical risk | Solo feasibility | Safety | Claim correctness |
|---|---|---|---|---|---|---|
| **A**, galvo vertical slice | **9** | 7 | **8** | **8** | **9** | **9** |
| **B**, platform first | 3 | **9** | 5 | 4 | **9** | 7 |
| **C**, strangler + ecosystem | 8 | 7 | 6 | 6 | 8 | 6.5 |

## 3. Per-proposal assessment

### A: galvo-first vertical slice

**Strengths**
- **The bridge to the first burn is cheap.** At M1.2 (`ol mark --prototype`), the Rust driver burns from Python-designed jobs about 3 weeks before the Rust GUI exists, with no Python changes.
- **It plugs the stop-path holes in nusb.** It uses `submit` plus `wait_next_complete` (≤ 20 ms) and never `transfer_blocking`. After `cancel_all` it assumes a partial packet reached the board (06 §2.6, C16). The dummy end-of-list packet is sent only if READY arrives within 50 ms. That is better than galvoplotter, whose `abort()` waits on READY inside `_list_end`.
- **Each known prototype bug maps to a structural fix and a named regression test.** A one-time **mutation check** then reverts each fix and requires its test to go red. That is unique to A and very effective against AI-written tests that pass vacuously.
- **The data model closes the leak the Rayforge test caught.** Separate `galvo`/`gantry` blocks inside each layer make the cross-machine-kind leak from `REPORT.md` Takeaway 1 unrepresentable. A cites 02 §7.7 for this deliberate divergence from LightBurn.
- **There is no "py-compat" sequencing mode to maintain.** The encoder is byte-exact (prelude + packets + postlude), and the session is verified by its behaviour on the emulator.
- **A egui spike runs in M0 with numeric GO/NO-GO gates.** muda is deliberately *not* a gate (06 spot-check 5). `cargo tree -d` must show one egui and one wgpu, and Esc must set the stop latch in 16 ms or less while a plan is running.
- **It sets the right "do it now" items.** D12 says to read the EZCAD `.cor` and `markcfg` now, since that needs no laser (03 §13). A also pins galvoplotter and tags `oracle-v1`, with a committed v0-vs-v1 diff.

**Weaknesses**
- **M0 and M1 are overloaded.** M0 (2 weeks) holds: rustup, repo and CI, 7 Python patches, the golden generator, T0 encoders, an emulator skeleton, the egui spike and the name choice. M1.3 (3 weeks) holds: the whole egui shell, canvas, layers, laser panel, confirm dialog, project file, undo, and SVG/DXF/prototype import.
- **The M1.2 bridge has to reproduce a JavaScript transform.** It re-derives geometry from `workspace.json` plus a `--mirror-x` flag, re-implementing the browser-side transform order (05 §1.7, JS inside `ui.py`). No Python oracle exists for that code. C's exported `/api/mark` payload is already transformed and mirrored, so it is more faithful (§5).
- **The speed refusal threshold is uneven.** A refuses quantisation error above 10 %. At F = 236.68, 10 mm/s and 25 mm/s are refused while 15 mm/s and 35 mm/s pass. It is better to snap the speed field to encodable values at input and refuse only a speed that encodes to 0 units.
- **The M3 list-merge gate names the wrong item.** A gates merging layers into one list on H2. The real question is whether parameter changes mid-list take effect, which is C's H21.
- **Some trait shapes are slightly GRBL-like.** `pause`/`resume` are required methods on `Backend`. B's and C's optional capability sub-traits are cleaner (01 §8.2.3).
- **Planning has no incremental cache.** That is fine through M2, but large M5 documents will need B's rules (§5).

### B: platform first

**Strengths**
- **It has the strongest safety ideas of the three:**
  - **Cold rebuild at ARM**: rebuild without any cache and compare hashes (rule 3).
  - **The confirm summary is decoded from the encoded payload** (`EmissionSummary`), not from the plan.
  - **Run-ahead limit**: at most 3 unexecuted packets on the board, which bounds marking if the app crashes.
  - **Unit newtypes** (`Mm`, `Hz`, `Ns`, …).
  - **No API, script or LAN path can ARM.**
  - A fake-board CI stop budget of 100 ms, tighter than the 250 ms hardware target.
- **It best avoids a rewrite on the way to parity.** The cut table matches LightBurn's REST schema, an `Extras` bag carries unknown `.lbrn2` content through, the intent graph is checked by an incremental-equals-cold proptest, and drivers expose capability sub-traits.
- **Its T4 note is correct.** The prototype's SVG importer fits everything to 60 mm (05 §4.5), so SVG parity is impossible and B says so.
- It adds a HIL-0 passive session (descriptors and replies only).

**Weaknesses**
- **Time to value is poor.** There is no Rust emission until the end of M2 (about 12–17 focused weeks), so Luiz runs the patched Python stack for 5–7 months.
- **Hardware truth arrives late.** Flow-control semantics (H2/H13) are only measured after the IR, intent graph, document model and safety layer are built on assumptions about them.
- **It names the over-build risk but still schedules it.** M0 builds "every crate stub", and M1 builds a 30-slot `CutTable`, the `Extras` bag and an intent graph with a 1,000-sequence proptest, all before any I/O or UI. Ruida, rhai and extism appear on a hardware-limited roadmap, although Luiz owns no Ruida machine (05 §3.1).
- **muda is a *must* gate.** A cosmetic native menu could force the Tauri fallback, against 06 spot-check 5's caution.
- **`py-compat` lives in the production `ol-lmc` crate until M5.** That keeps the first-jump elision bug in shipping code. It should be test-only, as C has it.

### C: strangler fig + ecosystem

**Strengths**
- **It puts risk first.** The M1 Rust CLI marks the MOPA with HW-1 and HW-2 done.
- **`py-compat` is a test-only Cargo feature.**
- **`DIVERGENCES.md` lists every intentional difference, and Luiz signs it off.**
- **It has the best runtime hygiene:**
  - `#![forbid(unsafe_code)]`
  - a **panic drop guard that triggers `emergency_stop`**
  - `catch_unwind` at the FFI boundary
  - faults that stay set until acknowledged, and a stop that does not
  - interlocks read inside wait loops
- **It adds new hardware items.** **H21** asks whether current and Q-pulse changes mid-list take effect. **H22** asks whether `GetPositionXY` can serve as a progress signal.
- **It sets HIL rules**: the first emission of a session is at 10 % power or less.
- **Its spike includes IME as a gate.** Portuguese dead keys matter for this user.
- **The exported `/api/mark` payload** is an exact, already-transformed bridge input (05 §4.4).

**Weaknesses**
- **The PyO3 phase is costly.** M2 plus the material-grid work in M3 is effort spent on a UI archived at M5. It brings GIL and FFI-panic handling into a threaded HTTP server and drops per-job process isolation.
- **It runs two engines behind a flag.** C lists this as its own risk 2.
- **It needs three toolchains**: Rust, maturin abi3 wheels, and Python prototype upkeep. On top of that sits an optional Rayforge addon side track.
- **The native app arrives late.** It starts at M4 (about week 14), and the prototype is not replaced until about week 27.
- **The HW-1 order is wrong.** It runs H12 (descriptors, the nusb bring-up prerequisite) *after* H13, which deliberately writes into a full buffer.
- **It contains several internal contradictions and one misreading of the research** (§4).

## 4. Factual errors and contradictions with the research

**Shared by all three**
1. **The Python "3 s wait cap" patch ignores galvoplotter's own abort path.** All three change `wait_ready` (or the waits) to raise or abort on deadline. `GP.abort()` itself calls `_list_end()`, and `_list_end()` calls `wait_ready()` before the dummy packet (verified in the installed `galvo/controller.py`, `abort()` and `_list_end()`). A raise there would skip the final `Fiber_SetMo(0)` / `port_off` / `WritePort(0)`. OM also runs `shutdown()` → `abort()` after *every* successful job (05 §0.2), so this path runs every time, not only on stop. The patch must catch the exception inside `abort()`, skip the dummy packet when the board is not READY, and still send MO close and port-off. That is the behaviour A specifies for Rust.

**A**
2. The M3 rationale gates the one-continuous-list optimisation on H2 (buffer depth). The deciding unknown is whether mid-list parameter changes take effect (C's H21). It is an omission, not a false statement.
3. Its other citations check out. `galvoplotter@fc35136` matches 03's clone commit. The 02 §7.7 shared speed/power claim, the 403-object / 3,057-point business card, the effort arithmetic (32 weeks to M5) and A's own Python patch #2 premise all hold. The patch #2 premise is that `list_jump` has no equality check (`controller.py:1162-1170`), which is correct.

**B**
4. Rule 5 requires "`Driver` … LMC, FakeLmc and FakeGrbl implementations by M2". FakeGrbl and `ol-drv-grbl` are scheduled for **M4**. FakeLmc is also a board simulator in `ol-sim`, not a `Driver`.
5. `MachineHandle::stop()` is described as "never blocks, < 1 µs". §2.7 says GRBL realtime bytes are "written by `stop()` itself through a `try_clone`d serial handle". A serial write is a syscall that can block.
6. M0 builds "every crate stub" (Ruida, script, and so on). That contradicts B's own risk-1 mitigation: "anything not needed by M3 lives only as an ADR".
7. The Rust side refuses quantisation error above 5 %, while Python patch #5 only *warns* above 5 %. T5 decision parity with `oracle-v1` will therefore diverge unless this is listed as a divergence. Separately, refusing speeds above the gantry maximum contradicts T2 byte-parity with `gcode.py`, which silently caps the feed (05 §3.2). That too must be a documented divergence.
8. B makes `muda` a **must** gate even though it cites R06 spot-check 5. There, muda has precedent only on egui 0.36 with a patched egui, so a failure here is likely.

**C**
9. **This is a misreading of the research.** C says the `openlaser_lmc` wheel "removes one of the two upstream blockers 01 §3.2 found", because nusb brings no libusb. But 01 §3.1 says the addon manifest has **no way to ship pip dependencies into frozen, snap or flatpak builds**. A native wheel is a pip dependency, so the bundling blocker stays; only its shape changes.
10. M2 says "the macOS installer no longer bundles libusb", while legacy mode stays "one release as an A/B escape hatch". Legacy mode needs pyusb and libusb.
11. The oracle is pinned at OM `6aed8ff`, yet T5 includes "§8 hotfix fixtures". Those exist only in a later commit, so two oracle SHAs are needed.
12. T4 asks for "DXF/SVG/STL importer fixtures … point count exact". The prototype's SVG import fits to 60 mm and supports only M/L/H/V/Z (05 §4.5). The planned usvg importer cannot match it, which B correctly documents.
13. "Freeze features now" contradicts adding an "Export job JSON" button (M1) and a material test grid to the Python UI (M3).
14. In HW-1, H12 (descriptors, `set_configuration`) comes after H13. You cannot run H2 or H13 until the transport is up.

## 5. Grafts the final plan must include (base: A)

**From B**
1. **Unit newtypes** (`Mm`, `MmPerS`, `Hz`, `Ns`, `Us`, `Percent`, `GalvoUnits`), at least at the `ol-safety` / `ol-lmc` boundary.
2. **Decode the confirm summary from the encoded `LmcJob`** (an `EmissionSummary` via a disassembler), not from the plan.
3. **Run-ahead limit**: at most 3 unexecuted packets on the board until H2 measures the buffer depth.
4. **Cold rebuild and hash compare at ARM** as a rule, before any planning cache is added (M5).
5. **Policy: no API, script or LAN path can ARM.**
6. **A HIL-0 passive session**: descriptors, `GetVersion` and `GetSerialNo` replies only.
7. **Optional capability sub-traits** (`Pause`, `Jog`, `Home`) instead of required stub methods.
8. **CI stop budget on the fake board: 100 ms p99**, with hardware held at 250 ms or less.
9. **Document the T4 SVG divergence.** Refuse gantry speeds above the machine maximum as a *documented* T2 divergence.
10. **Gantry defaults**: `home_on_connect` defaults to false. Run FakeGrbl on TCP so Rayforge can be compared on the same emulator.
11. **An `Extras` bag and `.lbrn2` fixtures from the local LightBurn 1.5.04** when M5 arrives.
12. **Run the egui spike as a parallel executor task** during M1.1–M1.2, decided before M1.3. That takes it out of A's overloaded M0.

**From C**
13. **The bridge input to M1.2 is the prototype's exact `/api/mark` payload**, already transformed and mirrored (a small "Export job JSON"). At minimum, check it against A's `workspace.json` re-derivation.
14. **A panic drop guard that calls `emergency_stop`**, plus `#![forbid(unsafe_code)]`. Faults stay set until acknowledged; a stop does not.
15. **`DIVERGENCES.md` signed off by Luiz.** If `py-compat` is ever used, it is a test-only Cargo feature.
16. **New HIL items H21 and H22.** H21 gates both the M3 list merge and the per-run raster power model (06 §0.8). H22 tests `GetPositionXY` as a progress signal.
17. **HIL rules**: the first emission of a session is at 10 % power or less, and the door and foot inputs are read inside every wait loop once H11 confirms them.
18. **FakeLmc scenarios**: a virtual clock, a partial packet after a cancel, and a PID `0x9980` clone board.
19. **IME (Portuguese dead keys) as a soft spike gate.**
20. **A Rayforge LMC addon only after M3**, and only if the maintainer engages, since bundling a native wheel is still blocked (§4 item 9).

**New from this review (none of the proposals has them)**
21. **Fix the Python patch #1 abort path** (§4 item 1).
22. **A dead-man rule**: if the driver thread's control channel disconnects (the UI crashed or panicked), stop the job. Together with the run-ahead limit, this bounds what happens after a crash.
23. **Quantise speed at input.** The speed field offers only encodable values, the actual speed is always shown, and only a speed that encodes to 0 is refused. This replaces the uneven 5 %/10 % refusals.
24. **Order HIL-1 so H12 comes first**, then H2, H13, H14 and H1, and keep it red-light only.

## 6. Rationale in one paragraph

The binding constraints are one developer, AI executors whose output must be checked mechanically, a laser that has to be safe, and a working but buggy Python stack Luiz uses daily. That rewards the plan that turns hardware unknowns into recorded traces soonest, verifies AI code with byte-exact goldens and emulator invariants instead of review, and keeps scope small until the MOPA path is proven. A does all three. Its known weaknesses are an overloaded M0/M1 and a bridge that re-derives transforms, and both are fixed by grafts 12 and 13. B's safety mechanisms are excellent and cheap to graft, but its sequencing defers truth from hardware and value to Luiz by months. C's engine work is sound, but its PyO3 bridge is a costly detour into the architecture the research says to avoid.
