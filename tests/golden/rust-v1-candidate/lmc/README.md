# rust-v1 LMC list candidates

Unsigned review candidates for SPEC S15 / D13. These do not replace the immutable Python fixtures.

Regenerate with hardware disabled:

```sh
OPENLASER_NO_HW=1 UPDATE_RUST_V1=1 cargo test -p openlaser-lmc rust_v1
```

Each scenario file contains every packet's SHA-256, meaningful command count, all 256 wire slots as
little-endian hexadecimal (including EOL/padding), and a readable opcode/parameter listing.
Frame scenarios contain the complete Frame Once list; `_loop_pass` files contain one complete Frame Loop pass.
The v1 comparison permits only D1/D2/D3 (SPEC §5.2 rule 6 and §5.6), transforming the expected oracle stream
before checking every remaining word. A normal test run checks these candidate files without writing them.
Session wire-stream candidates belong to WP5. Luiz's D13 signature is required before freezing either set.
