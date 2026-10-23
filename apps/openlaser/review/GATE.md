# WP8 round 1 gate

Rust 1.99.0. Hardware disabled (`OPENLASER_NO_HW=1`, `OPENLASER_HW_PRESENT` unset). Target lane: `~/AI/OpenLaser/.target/wp8`, as requested. Free disk stayed above 12 GiB.

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy -p openlaser-ui-egui -p openlaser --all-targets -- -D warnings` | pass |
| `cargo clippy -p openlaser-ui-egui -p openlaser --all-targets --target x86_64-pc-windows-msvc -- -D warnings` | pass |
| `cargo test -p openlaser-ui-egui -p openlaser` | pass |
| `cargo xtask deps` | pass |
| `cargo xtask golden-verify` | pass |

macOS tests: 8 application library tests and 44 UI tests passed. Five I2 tests and two performance harnesses remain ignored. Windows was cross-checked with clippy; Windows tests were not run on this Mac. All 174 golden fixtures match their manifest.
