# rust-v1 LMC list goldens (frozen)

Signed by Luiz on 2026-10-26 (SPEC S15 / D13, `docs/DIVERGENCES.md`) **for controlled hardware validation only;
this is not a hardware-safety certification.** Frozen byte for byte from the candidates of commit `7128a6f`; only the
first header line changed. They do not replace the immutable Python fixtures and are listed in
`tests/golden/manifest.json` (`cargo xtask golden-verify`).

`crates/lmc/tests/rust_v1.rs` compares the shipping lists with these files on every run. A change needs a new
review: write fresh candidates (hardware disabled) to `tests/golden/rust-v1-candidate/lmc/` with

```sh
OPENLASER_NO_HW=1 UPDATE_RUST_V1=1 cargo test -p openlaser-lmc rust_v1
```

then diff them against this folder, get the change signed, and only then replace these files and the manifest.

Each scenario file contains every packet's SHA-256, meaningful command count, all 256 wire slots as
little-endian hexadecimal (including EOL/padding), and a readable opcode/parameter listing.
Frame scenarios contain the complete Frame Once list; `_loop_pass` files contain one complete Frame Loop pass.
The v1 comparison permits only D1/D2/D3 (SPEC §5.2 rule 6 and §5.6), transforming the expected oracle stream
before checking every remaining word. A normal test run checks these files and never writes them.
