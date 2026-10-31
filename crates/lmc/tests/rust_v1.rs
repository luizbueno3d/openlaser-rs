//! rust-v1 goldens (SPEC §6.2): the shipping lists of every T1 scenario, checked byte for byte against the frozen
//! `tests/golden/rust-v1/lmc/` (signed by Luiz 2026-10-26, D13, for controlled hardware validation). A change needs a
//! new review: `UPDATE_RUST_V1=1` writes the new lists to `tests/golden/rust-v1-candidate/lmc/` and still fails here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::fmt::Write;

use common::{SCENARIOS, fixture, is_frame, shipping, shipping_frame};
use openlaser_lmc::op;

#[test]
fn rust_v1_frozen_lists() {
    let frozen = openlaser_testkit::golden_dir().join("rust-v1").join("lmc");
    let candidates = openlaser_testkit::golden_dir().join("rust-v1-candidate").join("lmc");
    let update = std::env::var("UPDATE_RUST_V1").is_ok_and(|v| v == "1");
    if update {
        std::fs::create_dir_all(&candidates).unwrap_or_else(|e| panic!("{e}"));
    }
    for name in SCENARIOS {
        let s = fixture("v1", name);
        let mut outputs = vec![(name.to_owned(), shipping(&s))];
        if is_frame(&s) {
            outputs.push((format!("{name}_loop_pass"), shipping_frame(&s).pass));
        }
        for (name, packets) in outputs {
            let mut text = format!(
                "# {name}; millimetre inputs: t1_lmc/v1/{}.json\n# Includes all 256 slots per packet, including explicit EOL and padding.\n",
                s.scenario
            );
            for (i, packet) in packets.iter().enumerate() {
                writeln!(text, "packet {i} sha256={} commands={}", packet.sha256_hex(), packet.real_commands()).unwrap_or_else(|e| panic!("{e}"));
                for slot in packet.bytes().as_chunks::<12>().0 {
                    for byte in slot {
                        write!(text, "{byte:02x}").unwrap_or_else(|e| panic!("{e}"));
                    }
                    text.push('\n');
                }
                text.push_str("listing (slot, opcode, five parameters)\n");
                for (slot, word) in packet.words().iter().take((packet.real_commands() + 1).min(256)).enumerate() {
                    writeln!(text, "{slot:03}: {} {:?}", op::list_name(word[0]), &word[1..]).unwrap_or_else(|e| panic!("{e}"));
                }
            }
            if update {
                let candidate = format!("# rust-v1 CANDIDATE (unsigned, SPEC S15 / D13)\n{text}");
                std::fs::write(candidates.join(format!("{name}.txt")), candidate).unwrap_or_else(|e| panic!("{e}"));
            }
            let path = frozen.join(format!("{name}.txt"));
            let want = format!("# rust-v1 (signed 2026-10-26 for controlled hardware validation, SPEC S15 / D13)\n{text}");
            assert_eq!(
                std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
                want,
                "the shipping list differs from the signed rust-v1 golden {} (a change needs a new D13 review)",
                path.display()
            );
        }
    }
}
