//! LMC opcode table.
//!
//! Ported from galvoplotter 0.2.0 `galvo/consts.py` (MeerK40t contributors,
//! MIT) and checked against R05 Appendix B. Names intentionally retain GP's
//! spelling because golden traces use them.

//! Protocol source: meerk40t/galvoplotter@fc35136 galvo/consts.py (0.2.0), MIT.

/*
MIT License

Copyright (c) 2023 MeerK40t

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
*/

pub const LIST_JUMP_TO: u16 = 0x8001;
pub const LIST_END_OF_LIST: u16 = 0x8002;
pub const LIST_LASER_ON_POINT: u16 = 0x8003;
pub const LIST_DELAY_TIME: u16 = 0x8004;
pub const LIST_MARK_TO: u16 = 0x8005;
pub const LIST_JUMP_SPEED: u16 = 0x8006;
pub const LIST_LASER_ON_DELAY: u16 = 0x8007;
pub const LIST_LASER_OFF_DELAY: u16 = 0x8008;
pub const LIST_MARK_FREQ: u16 = 0x800a;
pub const LIST_MARK_POWER_RATIO: u16 = 0x800b;
pub const LIST_MARK_SPEED: u16 = 0x800c;
pub const LIST_JUMP_DELAY: u16 = 0x800d;
pub const LIST_POLYGON_DELAY: u16 = 0x800f;
pub const LIST_WRITE_PORT: u16 = 0x8011;
pub const LIST_MARK_CURRENT: u16 = 0x8012;
pub const LIST_MARK_FREQ_2: u16 = 0x8013;
pub const LIST_FLY_ENABLE: u16 = 0x801a;
pub const LIST_Q_SWITCH_PERIOD: u16 = 0x801b;
pub const LIST_DIRECT_LASER_SWITCH: u16 = 0x801c;
pub const LIST_FLY_DELAY: u16 = 0x801d;
pub const LIST_SET_CO2_FPK: u16 = 0x801e;
pub const LIST_FLY_WAIT_INPUT: u16 = 0x801f;
pub const LIST_FIBER_OPEN_MO: u16 = 0x8021;
pub const LIST_WAIT_FOR_INPUT: u16 = 0x8022;
pub const LIST_CHANGE_MARK_COUNT: u16 = 0x8023;
pub const LIST_SET_WELD_POWER_WAVE: u16 = 0x8024;
pub const LIST_ENABLE_WELD_POWER_WAVE: u16 = 0x8025;
pub const LIST_FIBER_YLPM_PULSE_WIDTH: u16 = 0x8026;
pub const LIST_FLY_ENCODER_COUNT: u16 = 0x8028;
pub const LIST_SET_DA_Z_WORD: u16 = 0x8029;
pub const LIST_JPT_SET_PARAM: u16 = 0x8050;
pub const LIST_READY_MARK: u16 = 0x8051;

pub const DISABLE_LASER: u16 = 0x0002;
pub const ENABLE_LASER: u16 = 0x0004;
pub const EXECUTE_LIST: u16 = 0x0005;
pub const SET_PWM_PULSE_WIDTH: u16 = 0x0006;
pub const GET_VERSION: u16 = 0x0007;
pub const GET_SERIAL_NO: u16 = 0x0009;
pub const GET_LIST_STATUS: u16 = 0x000a;
pub const GET_POSITION_XY: u16 = 0x000c;
pub const GOTO_XY: u16 = 0x000d;
pub const LASER_SIGNAL_OFF: u16 = 0x000e;
pub const LASER_SIGNAL_ON: u16 = 0x000f;
pub const WRITE_COR_LINE: u16 = 0x0010;
pub const RESET_LIST: u16 = 0x0012;
pub const RESTART_LIST: u16 = 0x0013;
pub const WRITE_COR_TABLE: u16 = 0x0015;
pub const SET_CONTROL_MODE: u16 = 0x0016;
pub const SET_DELAY_MODE: u16 = 0x0017;
pub const SET_MAX_POLY_DELAY: u16 = 0x0018;
pub const SET_END_OF_LIST: u16 = 0x0019;
pub const SET_FIRST_PULSE_KILLER: u16 = 0x001a;
pub const SET_LASER_MODE: u16 = 0x001b;
pub const SET_TIMING: u16 = 0x001c;
pub const SET_STANDBY: u16 = 0x001d;
pub const SET_PWM_HALF_PERIOD: u16 = 0x001e;
pub const STOP_EXECUTE: u16 = 0x001f;
pub const STOP_LIST: u16 = 0x0020;
pub const WRITE_PORT: u16 = 0x0021;
pub const WRITE_ANALOG_PORT_1: u16 = 0x0022;
pub const WRITE_ANALOG_PORT_2: u16 = 0x0023;
pub const WRITE_ANALOG_PORT_X: u16 = 0x0024;
pub const READ_PORT: u16 = 0x0025;
pub const SET_AXIS_MOTION_PARAM: u16 = 0x0026;
pub const SET_AXIS_ORIGIN_PARAM: u16 = 0x0027;
pub const AXIS_GO_ORIGIN: u16 = 0x0028;
pub const MOVE_AXIS_TO: u16 = 0x0029;
pub const GET_AXIS_POS: u16 = 0x002a;
pub const GET_FLY_WAIT_COUNT: u16 = 0x002b;
pub const GET_MARK_COUNT: u16 = 0x002d;
pub const SET_FPK_PARAM_2: u16 = 0x002e;
pub const FIBER_PULSE_WIDTH: u16 = 0x002f;
pub const FIBER_GET_CONFIG_EXTEND: u16 = 0x0030;
pub const INPUT_PORT: u16 = 0x0031;
pub const SET_FLY_RES: u16 = 0x0032;
pub const FIBER_SET_MO: u16 = 0x0033;
pub const FIBER_GET_ST_MO_AP: u16 = 0x0034;
pub const GET_USER_DATA: u16 = 0x0036;
pub const GET_FLY_SPEED: u16 = 0x0038;
pub const DISABLE_Z: u16 = 0x0039;
pub const ENABLE_Z: u16 = 0x003a;
pub const SET_Z_DATA: u16 = 0x003b;
pub const SET_SPI_SIMMER_CURRENT: u16 = 0x003c;
pub const RESET: u16 = 0x0040;
pub const GET_MARK_TIME: u16 = 0x0041;
pub const SET_FPK_PARAM: u16 = 0x0062;

pub fn single_name(op: u16) -> &'static str {
    match op {
        DISABLE_LASER => "DisableLaser",
        ENABLE_LASER => "EnableLaser",
        EXECUTE_LIST => "ExecuteList",
        SET_PWM_PULSE_WIDTH => "SetPwmPulseWidth",
        GET_VERSION => "GetVersion",
        GET_SERIAL_NO => "GetSerialNo",
        GET_LIST_STATUS => "GetListStatus",
        GET_POSITION_XY => "GetPositionXY",
        GOTO_XY => "GotoXY",
        LASER_SIGNAL_OFF => "LaserSignalOff",
        LASER_SIGNAL_ON => "LaserSignalOn",
        WRITE_COR_LINE => "WriteCorLine",
        RESET_LIST => "ResetList",
        RESTART_LIST => "RestartList",
        WRITE_COR_TABLE => "WriteCorTable",
        SET_CONTROL_MODE => "SetControlMode",
        SET_DELAY_MODE => "SetDelayMode",
        SET_MAX_POLY_DELAY => "SetMaxPolyDelay",
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
        WRITE_ANALOG_PORT_2 => "WriteAnalogPort2",
        WRITE_ANALOG_PORT_X => "WriteAnalogPortX",
        READ_PORT => "ReadPort",
        SET_AXIS_MOTION_PARAM => "SetAxisMotionParam",
        SET_AXIS_ORIGIN_PARAM => "SetAxisOriginParam",
        AXIS_GO_ORIGIN => "AxisGoOrigin",
        MOVE_AXIS_TO => "MoveAxisTo",
        GET_AXIS_POS => "GetAxisPos",
        GET_FLY_WAIT_COUNT => "GetFlyWaitCount",
        GET_MARK_COUNT => "GetMarkCount",
        SET_FPK_PARAM_2 => "SetFpkParam2",
        FIBER_PULSE_WIDTH => "FiberPulseWidth",
        FIBER_GET_CONFIG_EXTEND => "FiberGetConfigExtend",
        INPUT_PORT => "InputPort",
        SET_FLY_RES => "SetFlyRes",
        FIBER_SET_MO => "Fiber_SetMo",
        FIBER_GET_ST_MO_AP => "Fiber_GetStMO_AP",
        GET_USER_DATA => "GetUserData",
        GET_FLY_SPEED => "GetFlySpeed",
        DISABLE_Z => "DisableZ",
        ENABLE_Z => "EnableZ",
        SET_Z_DATA => "SetZData",
        SET_SPI_SIMMER_CURRENT => "SetSPISimmerCurrent",
        RESET => "Reset",
        GET_MARK_TIME => "GetMarkTime",
        SET_FPK_PARAM => "SetFpkParam",
        _ => "Unknown",
    }
}

pub fn list_name(op: u16) -> &'static str {
    match op {
        LIST_JUMP_TO => "listJumpTo",
        LIST_END_OF_LIST => "listEndOfList",
        LIST_LASER_ON_POINT => "listLaserOnPoint",
        LIST_DELAY_TIME => "listDelayTime",
        LIST_MARK_TO => "listMarkTo",
        LIST_JUMP_SPEED => "listJumpSpeed",
        LIST_LASER_ON_DELAY => "listLaserOnDelay",
        LIST_LASER_OFF_DELAY => "listLaserOffDelay",
        LIST_MARK_FREQ => "listMarkFreq",
        LIST_MARK_POWER_RATIO => "listMarkPowerRatio",
        LIST_MARK_SPEED => "listMarkSpeed",
        LIST_JUMP_DELAY => "listJumpDelay",
        LIST_POLYGON_DELAY => "listPolygonDelay",
        LIST_WRITE_PORT => "listWritePort",
        LIST_MARK_CURRENT => "listMarkCurrent",
        LIST_MARK_FREQ_2 => "listMarkFreq2",
        LIST_FLY_ENABLE => "listFlyEnable",
        LIST_Q_SWITCH_PERIOD => "listQSwitchPeriod",
        LIST_DIRECT_LASER_SWITCH => "listDirectLaserSwitch",
        LIST_FLY_DELAY => "listFlyDelay",
        LIST_SET_CO2_FPK => "listSetCo2FPK",
        LIST_FLY_WAIT_INPUT => "listFlyWaitInput",
        LIST_FIBER_OPEN_MO => "listFiberOpenMO",
        LIST_WAIT_FOR_INPUT => "listWaitForInput",
        LIST_CHANGE_MARK_COUNT => "listChangeMarkCount",
        LIST_SET_WELD_POWER_WAVE => "listSetWeldPowerWave",
        LIST_ENABLE_WELD_POWER_WAVE => "listEnableWeldPowerWave",
        LIST_FIBER_YLPM_PULSE_WIDTH => "listFiberYLPMPulseWidth",
        LIST_FLY_ENCODER_COUNT => "listFlyEncoderCount",
        LIST_SET_DA_Z_WORD => "listSetDaZWord",
        LIST_JPT_SET_PARAM => "listJptSetParam",
        LIST_READY_MARK => "listReadyMark",
        _ => "Unknown",
    }
}
