# Research, design decisions and project history

This directory is the **complete planning corpus** for OpenLaser, copied unmodified from
`~/AI/ClaudeCode/openlaser-rust-plan/` (2026-10-25). It explains *why* the codebase is the way
it is. The authoritative current-state spec is [`../SPEC.md`](../SPEC.md); behaviour changes since
the spec are logged in [`../SPEC-CHANGES.md`](../SPEC-CHANGES.md); known gaps in
[`../../followups.md`](../../followups.md).

**Read these first** when you need context that the code does not carry:

- [`PLAN.md`](PLAN.md) — the final plan (revision 2, post red-team). Milestones M0–M4, the
  hardware-question register (H0–H29), the prototype's known safety bugs (B1–B12), acceptance
  criteria, and citations into everything below.
- [`design/redteam-review.md`](design/redteam-review.md) — adversarial review of the plan; the
  critical findings shaped the safety posture (refuse-never-warn, byte-decoded confirmation,
  human-only ARM).

## Research reports (`research/`)

| File | What it answers |
|---|---|
| [01-rayforge-source-deep-dive.md](research/01-rayforge-source-deep-dive.md) | Why we did not fork Rayforge: no galvo/LMC support anywhere, maintainer's PR #288 never worked, bus factor 1. |
| [02-lightburn-feature-inventory.md](research/02-lightburn-feature-inventory.md) | The LightBurn feature catalogue the UI and command layout are built against (the "CAT" references in `menu_catalog.rs` and tests). |
| [03-open-source-prior-art.md](research/03-open-source-prior-art.md) | balor, MeerK40t, LaserGRBL, galvoplotter and others — behaviour references and their licences (clean-room provenance). |
| [04-rust-ecosystem-survey.md](research/04-rust-ecosystem-survey.md) | Crate choices (egui/eframe, nusb, serialport, …), maturity scores, Windows constraints. |
| [05-hardware-protocols-and-test-inputs.md](research/05-hardware-protocols-and-test-inputs.md) | The LMC board protocol, USB traces, the Python oracle (`OpenMopa`/`galvoplotter`) and where the golden fixtures come from. |
| [06-critic-gaps-and-corrections.md](research/06-critic-gaps-and-corrections.md) | A critic pass over reports 01–05: corrections and remaining gaps. |

## Design proposals (`design/`)

Three architectures were proposed and judged; **proposal A won** (galvo-first vertical slice —
the shortest safe path from a design to a burn on the JPT M7), with grafts the judges demanded.

- [`proposal-A-galvo-vertical-slice.md`](design/proposal-A-galvo-vertical-slice.md) — the winner.
- [`proposal-B-platform-first.md`](design/proposal-B-platform-first.md) — rejected: platform before hardware.
- [`proposal-C-strangler-ecosystem.md`](design/proposal-C-strangler-ecosystem.md) — rejected: wrap the Python prototype.
- [`judge-product-safety.md`](design/judge-product-safety.md), [`judge-engineering-risk.md`](design/judge-engineering-risk.md) — the two judging passes.
- [`redteam-review.md`](design/redteam-review.md) — see above.

## Rayforge evaluation (`rayforge-test/`)

Evidence behind the "do not fork" decision: a fake-GRBL test rig, the generated G-code
comparison (Rayforge's 1,337 lines vs the prototype's 4,298) and the report. Kept for
provenance; none of this code is part of the build.

## Lineage

OpenLaser descends from **`OpenMopa`** (`~/AI/OpenLaser/OpenMopa`, the Python prototype on
GitHub at `luizbueno3d/OpenMopa`/`OpenLaser`, built on the MIT `galvoplotter` library). Its
eleven-plus known safety bugs (B1–B12) were patched on its `fix/safety-b1-b12` branch as
milestone M0 and are the reason for several refusals and invariants in this codebase — every
one has a regression test.

*Historical document: the corpus is preserved as written. Where it and the code disagree,
the code and `docs/SPEC-CHANGES.md` win.*
