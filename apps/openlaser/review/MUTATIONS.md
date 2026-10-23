# WP8 round 1 mutation checks

Each mutation was applied alone, its named regression test was run in the WP8 lane with hardware disabled, then the original file was restored in a `finally` block. Compile failures do not count as killed mutations.

| Mutation | Result |
|---|---|
| panic stop omitted | KILLED (test assertion failed) |
| signal stop omitted | KILLED (test assertion failed) |
| wrong signal exit status | KILLED (test assertion failed) |
| active close skips stop | KILLED (test assertion failed) |
| close accepts missing report | KILLED (test assertion failed) |
| menu flag inverted | KILLED (test assertion failed) |
| control mutation busy gate bypassed | KILLED (test assertion failed) |
| widget origin forged as human | KILLED (test assertion failed) |
| STOP overlay loses label | KILLED (test assertion failed) |
| StopNotConfirmed omitted | KILLED (test assertion failed) |
| pending ARM ignored | KILLED (test assertion failed) |
| non-Unicode accepted | KILLED (test assertion failed) |
| live menu parity regresses | KILLED (test assertion failed) |
| numeric accessibility label removed | KILLED (test assertion failed) |
| device accessibility label removed | KILLED (test assertion failed) |
| handler missing from dispatch | KILLED (test assertion failed) |
| disabled Home reason removed | KILLED (test assertion failed) |
| Start prerequisite tooltip removed | KILLED (test assertion failed) |
| synthetic Frame provenance bypassed | KILLED (test assertion failed) |
| Esc stop omitted | KILLED (test assertion failed) |
| palette replaced by arbitrary color | KILLED (test assertion failed) |
| tool palette mislabeled | KILLED (test assertion failed) |
| fixed swatch width overflows | KILLED (test assertion failed) |
| Cuts Output column abbreviated | KILLED (test assertion failed) |
| wheel zoom removed | KILLED (test assertion failed) |
| texture reuploaded each frame | KILLED (test assertion failed) |
| dashed strokes painted solid | KILLED (test assertion failed) |
| modal STOP overlay suppressed | KILLED (test assertion failed) |
| CAT menu order swapped | KILLED (test assertion failed) |

The initial run exposed three surviving test assertions (shared expected-state helper, pending-arm predicate, and missing spin-button role). The expectations were corrected and those mutations rerun. Texture-delta cleanup was moved before its assertion so a detected regression reports an ordinary test failure.

Additional lifecycle and surface mutations:

| Mutation | Result |
|---|---|
| native Quit bypasses close hook | KILLED (test assertion failed) |
| file task runs on UI thread | KILLED (test assertion failed) |
| human surface refusal bypassed | KILLED (test assertion failed) |

Canvas and approval-card mutations:

| Mutation | Result |
|---|---|
| canvas grid removed | KILLED (test assertion failed) |
| galvo field outline missing | KILLED (test assertion failed) |
| ruler labels removed | KILLED (test assertion failed) |
| synthetic approval-card provenance bypassed | KILLED (test assertion failed) |
