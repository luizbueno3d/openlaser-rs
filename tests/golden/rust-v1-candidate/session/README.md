# rust-v1 candidate: driver session streams (WP5)

The wire event streams of the shipping LMC session (SPEC §5.3) on a READY FakeLmc (buffer depth 8, no latency, no
faults), written by `crates/driver/tests/rust_v1_session.rs`:

```sh
UPDATE_RUST_V1=1 cargo test -p openlaser-driver --test rust_v1_session -- --ignored
```

and compared byte for byte by the same tests without `UPDATE_RUST_V1`. They are **candidates**: they become
`tests/golden/rust-v1/` only after Luiz signs `docs/DIVERGENCES.md` (PLAN D13). Nothing here is used by
`cargo xtask golden-verify` until then.

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
