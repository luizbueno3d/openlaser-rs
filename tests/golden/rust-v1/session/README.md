# rust-v1 golden: driver session streams (frozen)

The wire event streams of the shipping LMC session (SPEC §5.3) on a READY FakeLmc (buffer depth 8, no latency, no
faults), checked by `crates/driver/tests/rust_v1_session.rs`. Fresh candidates for a re-review:

```sh
OPENLASER_NO_HW=1 UPDATE_RUST_V1=1 cargo test -p openlaser-driver --test rust_v1_session
```

Signed by Luiz on 2026-10-26 (PLAN D13, `docs/DIVERGENCES.md`) **for controlled hardware validation only; this is
not a hardware-safety certification.** Frozen from the candidates of commit `7128a6f` (only `meta.tier` changed) and
listed in `tests/golden/manifest.json`. The tests compare with these files on every run; `UPDATE_RUST_V1=1` writes a
fresh candidate to `tests/golden/rust-v1-candidate/session/` for a new review and never touches this folder.

| File | Scenario |
| --- | --- |
| `connect_frame_box10.json` | connect (the R05 §1.4 init once, 50 ms settle) + Frame Once of the 10 mm box |
| `connect_frame_box10_mark_box10.json` | the same, then `mark_box10` through the shipping encoder with an ARM token |

Each file: `meta`, `scenario`, `description`, `board`, and `events` in wire order, in GOLD's T1 shape without
timestamps: `{"kind":"single","hex":"<24 hex digits>"}`, `{"kind":"read","status":<word 3>}`,
`{"kind":"list","sha256":"<packet sha256>"}`. The status polls (`GetVersion`) depend on FakeLmc's virtual-time
board model, which is deterministic.

Differences from the Python oracle's T1 streams are the session divergences of SPEC §5.6: one init per
connection (no per-job init, abort or USB reset), the explicit first jump with its settle, the lighting list's
leading `JumpSpeed`, MO closed only after !BUSY, and no abort after a successful job.
