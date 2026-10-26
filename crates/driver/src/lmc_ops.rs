//! The LMC commands the session builds itself, and their names for the Console trace.
//!
//! Opcode values are protocol facts from meerk40t/galvoplotter@fc35136 `galvo/consts.py` (MIT, © 2023 MeerK40t),
//! as tabulated in research 05 §1.3 and Appendix B. The protocol crate owns the full table (`openlaser_lmc::op`);
//! the driver keeps only the handful it sends on its own (list control, status, the stop sequence), so it does not
//! depend on table entries another work package fills in.

use openlaser_lmc::{PACKET_LEN, Single};

pub(crate) const ENABLE_LASER: u16 = 0x0004;
pub(crate) const EXECUTE_LIST: u16 = 0x0005;
pub(crate) const SET_PWM_PULSE_WIDTH: u16 = 0x0006;
pub(crate) const GET_VERSION: u16 = 0x0007;
pub(crate) const GET_SERIAL_NO: u16 = 0x0009;
pub(crate) const GET_LIST_STATUS: u16 = 0x000A;
pub(crate) const WRITE_COR_LINE: u16 = 0x0010;
pub(crate) const RESET_LIST: u16 = 0x0012;
pub(crate) const RESTART_LIST: u16 = 0x0013;
pub(crate) const WRITE_COR_TABLE: u16 = 0x0015;
pub(crate) const SET_CONTROL_MODE: u16 = 0x0016;
pub(crate) const SET_DELAY_MODE: u16 = 0x0017;
pub(crate) const SET_END_OF_LIST: u16 = 0x0019;
pub(crate) const SET_FIRST_PULSE_KILLER: u16 = 0x001A;
pub(crate) const SET_LASER_MODE: u16 = 0x001B;
pub(crate) const SET_TIMING: u16 = 0x001C;
pub(crate) const SET_STANDBY: u16 = 0x001D;
pub(crate) const SET_PWM_HALF_PERIOD: u16 = 0x001E;
pub(crate) const STOP_EXECUTE: u16 = 0x001F;
pub(crate) const STOP_LIST: u16 = 0x0020;
pub(crate) const WRITE_PORT: u16 = 0x0021;
pub(crate) const WRITE_ANALOG_PORT_1: u16 = 0x0022;
pub(crate) const SET_FPK_PARAM_2: u16 = 0x002E;
pub(crate) const SET_FLY_RES: u16 = 0x0032;
pub(crate) const FIBER_SET_MO: u16 = 0x0033;
pub(crate) const ENABLE_Z: u16 = 0x003A;
pub(crate) const RESET: u16 = 0x0040;
pub(crate) const GET_MARK_TIME: u16 = 0x0041;

/// List opcodes the frame check looks for.
pub(crate) const LIST_LASER_ON_POINT: u16 = 0x8003;
pub(crate) const LIST_MARK_TO: u16 = 0x8005;
pub(crate) const LIST_WRITE_PORT: u16 = 0x8011;

pub(crate) fn single(op: u16, p1: u16) -> Single {
    Single([op, p1, 0, 0, 0, 0])
}

pub(crate) fn get_version() -> Single {
    single(GET_VERSION, 0)
}

pub(crate) fn opcode(cmd: &Single) -> u16 {
    let [op, ..] = cmd.0;
    op
}

/// GP's name of a single command: the protocol crate's table first, this one for the commands the driver sends.
pub(crate) fn single_name(op: u16) -> &'static str {
    let known = openlaser_lmc::op::single_name(op);
    if known != "Unknown" {
        return known;
    }
    match op {
        ENABLE_LASER => "EnableLaser",
        EXECUTE_LIST => "ExecuteList",
        SET_PWM_PULSE_WIDTH => "SetPwmPulseWidth",
        GET_VERSION => "GetVersion",
        GET_SERIAL_NO => "GetSerialNo",
        GET_LIST_STATUS => "GetListStatus",
        WRITE_COR_LINE => "WriteCorLine",
        RESET_LIST => "ResetList",
        RESTART_LIST => "RestartList",
        WRITE_COR_TABLE => "WriteCorTable",
        SET_CONTROL_MODE => "SetControlMode",
        SET_DELAY_MODE => "SetDelayMode",
        SET_END_OF_LIST => "SetEndOfList",
        SET_FIRST_PULSE_KILLER => "SetFirstPulseKiller",
        SET_LASER_MODE => "SetLaserMode",
        SET_TIMING => "SetTiming",
        SET_STANDBY => "SetStandby",
        SET_PWM_HALF_PERIOD => "SetPwmHalfPeriod",
        STOP_EXECUTE => "StopExecute",
        STOP_LIST => "StopList",
        WRITE_PORT => "WritePort",
        WRITE_ANALOG_PORT_1 => "WriteAnalogPort1",
        SET_FPK_PARAM_2 => "SetFpkParam2",
        SET_FLY_RES => "SetFlyRes",
        FIBER_SET_MO => "Fiber_SetMo",
        ENABLE_Z => "EnableZ",
        RESET => "Reset",
        GET_MARK_TIME => "GetMarkTime",
        _ => "Unknown",
    }
}

/// "ResetList", "WritePort(0x0001)", "Fiber_SetMo(1)", "SetStandby(2000, 20)".
pub(crate) fn describe(cmd: &Single) -> String {
    let [op, p1, p2, p3, p4, p5] = cmd.0;
    let name = single_name(op);
    let name = if name == "Unknown" { format!("0x{op:04X}") } else { name.to_owned() };
    let params: Vec<u16> = [p1, p2, p3, p4, p5].into_iter().rev().skip_while(|p| *p == 0).collect();
    match (op, params.len()) {
        (_, 0) => name,
        (WRITE_PORT, _) => format!("{name}(0x{p1:04X})"),
        _ => format!("{name}({})", params.iter().rev().map(u16::to_string).collect::<Vec<_>>().join(", ")),
    }
}

/// Defence in depth behind the type-level `LightingList` (§5.2 rule 9): a frame packet may not mark. Refuses any
/// MarkTo or LaserOnPoint slot and any list WritePort that sets the laser bit. `laser_mask` is the profile's laser
/// port bit (`1 << ports.laser_bit`).
pub(crate) fn check_frame_packet(bytes: &[u8; PACKET_LEN], laser_mask: u16) -> Result<(), String> {
    let (slots, _) = bytes.as_chunks::<12>();
    for (i, slot) in slots.iter().enumerate() {
        let [o0, o1, p0, p1, ..] = *slot;
        let op = u16::from_le_bytes([o0, o1]);
        let param = u16::from_le_bytes([p0, p1]);
        let bad = match op {
            LIST_MARK_TO => Some("MarkTo"),
            LIST_LASER_ON_POINT => Some("LaserOnPoint"),
            LIST_WRITE_PORT if param & laser_mask != 0 => Some("a WritePort that sets the laser bit"),
            _ => None,
        };
        if let Some(what) = bad {
            return Err(format!("frame packet slot {i} holds {what}; a red-light frame can never mark"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(slots: &[[u16; 6]]) -> [u8; PACKET_LEN] {
        let mut out = [0u8; PACKET_LEN];
        let (chunks, _) = out.as_chunks_mut::<12>();
        for (i, chunk) in chunks.iter_mut().enumerate() {
            let words = slots.get(i).copied().unwrap_or([0x8002, 0, 0, 0, 0, 0]);
            for (j, w) in words.iter().enumerate() {
                let [lo, hi] = w.to_le_bytes();
                chunk[2 * j] = lo;
                chunk[2 * j + 1] = hi;
            }
        }
        out
    }

    #[test]
    fn describe_names_and_params() {
        assert_eq!(describe(&single(RESET_LIST, 0)), "ResetList");
        assert_eq!(describe(&single(WRITE_PORT, 1)), "WritePort(0x0001)");
        assert_eq!(describe(&single(FIBER_SET_MO, 1)), "Fiber_SetMo(1)");
        assert_eq!(describe(&Single([SET_STANDBY, 2000, 20, 0, 0, 0])), "SetStandby(2000, 20)");
        assert_eq!(describe(&Single([0x0099, 0, 0, 0, 0, 0])), "0x0099");
    }

    #[test]
    fn frame_check_refuses_marking_slots() {
        let lighting = packet(&[[0x8051, 0, 0, 0, 0, 0], [LIST_WRITE_PORT, 0x0100, 0, 0, 0, 0], [0x8001, 30000, 30000, 0, 100, 0]]);
        assert!(check_frame_packet(&lighting, 0x0001).is_ok());
        let marks = packet(&[[0x8051, 0, 0, 0, 0, 0], [LIST_MARK_TO, 30000, 30000, 0, 100, 0]]);
        assert!(check_frame_packet(&marks, 0x0001).unwrap_err().contains("MarkTo"));
        let dwell = packet(&[[LIST_LASER_ON_POINT, 100, 0, 0, 0, 0]]);
        assert!(check_frame_packet(&dwell, 0x0001).unwrap_err().contains("LaserOnPoint"));
        let laser_port = packet(&[[LIST_WRITE_PORT, 0x0101, 0, 0, 0, 0]]);
        assert!(check_frame_packet(&laser_port, 0x0001).unwrap_err().contains("laser bit"));
    }
}
