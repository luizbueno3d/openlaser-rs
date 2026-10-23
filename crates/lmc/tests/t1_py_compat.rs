//! T1 (v0), SPEC §5.5 / §6.2: `py_compat` reproduces every `tests/golden/t1_lmc/v0` event list exactly (kinds,
//! single-command hex, read statuses, packet sha256s and bytes).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::{SCENARIOS, fixture, is_frame, params, paths};
use openlaser_lmc::py_compat::{self, WireEvent};
use openlaser_testkit::t1::{self, Event};

fn assert_events(actual: &[WireEvent], expected: &[Event], name: &str) {
    assert_eq!(actual.len(), expected.len(), "{name} event count");
    for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
        let event = match a {
            WireEvent::Open => Event::Open,
            WireEvent::Close => Event::Close,
            WireEvent::Single { op, hex } => Event::Single { op: op.clone(), hex: hex.clone() },
            WireEvent::Read { status } => Event::Read { status: *status },
            WireEvent::List { sha256, n_cmds, bytes } => {
                assert_eq!(bytes.bytes().as_slice(), t1::packet(sha256).unwrap_or_else(|e| panic!("{e}")), "{name} packet {i}");
                Event::List { sha256: sha256.clone(), n_cmds: *n_cmds }
            }
        };
        assert_eq!(&event, b, "{name} event {i}");
    }
}

#[test]
fn t1_py_compat() {
    for name in SCENARIOS {
        let s = fixture("v0", name);
        let result = if is_frame(&s) {
            py_compat::frame_polylines(&paths(&s), s.field_size_mm)
        } else {
            py_compat::mark_polylines(&paths(&s), &params(&s), s.field_size_mm)
        };
        assert_events(&result.unwrap_or_else(|e| panic!("{name}: {e}")), &s.events, name);
    }
    let all = py_compat::init_abort();
    assert_events(&all, &fixture("v0", "init_abort").events, "init_abort");
    assert_events(&all[..39], &fixture("v0", "init").events, "init");
    assert_events(&all[39..], &fixture("v0", "abort").events, "abort");
}
