//! Opcodes: Appendix B of R05, every constant; names as GP. WP0 lists the constants the SPEC names; WP4 adds the
//! rest.

pub const GET_SERIAL_NO: u16 = 0x0009;
pub const GET_VERSION: u16 = 0x0007;

pub const LIST_JUMP_TO: u16 = 0x8001;
pub const LIST_END_OF_LIST: u16 = 0x8002;
pub const LIST_MARK_TO: u16 = 0x8005;

/// GP's name of a single command. WP0 stub.
pub fn single_name(op: u16) -> &'static str {
    let _ = op;
    "Unknown"
}

/// GP's name of a list command. WP0 stub.
pub fn list_name(op: u16) -> &'static str {
    let _ = op;
    "Unknown"
}
