//! USB (nusb 0.2.8; WinUSB on Windows, §2.6 item 4). Compiled, never run by agents.
//!
//! Writes are exactly 12 bytes (one single command) or 3072 bytes (one list packet); replies are read as one
//! short IN transfer (8 bytes used). Every wait is `wait_next_complete(≤ slice)` after a pending check, so no call
//! blocks longer than its slice. NO reset, NO retry, NO `transfer_blocking`, NO halt clearing, NO kernel-driver
//! detach (§2.3 rule 15): every error is mapped to `XferError`, and the driver stops the job and raises a fault.
//! Nothing from galvoplotter's `usb_connection.py` (its reset-and-retry loop) is ported.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use keepawake::KeepAwake;
use nusb::transfer::{Buffer, Bulk, In, Out, TransferError};
use nusb::{DeviceInfo, Endpoint, Interface, MaybeFuture};
use openlaser_driver::{ByteLink, DetectedDevice, DeviceProvider, DriverError};
use openlaser_lmc::{LmcTransport, PACKET_LEN, Packet, Single, XferError};
use openlaser_machine::{GalvoLink, GalvoMachine, GantryLink, GantryMachine};
use openlaser_types::{Refusal, RefusalCode};
use serde::Serialize;

use crate::links::{SerialLink, TcpLink};
use crate::lock::BoardLock;
use crate::{bench, hardware_disabled, hardware_refusal};

/// BJJCZ.
pub const LMC_VID: u16 = 0x9588;
/// USBLMCV4, the board OpenLaser drives.
pub const LMC_PID: u16 = 0x9899;
/// The clone board that needs firmware loaded: detected and refused, never flashed.
pub const CLONE_PID: u16 = 0x9980;
const EP_OUT: u8 = 0x02;
const EP_IN: u8 = 0x88;
const SINGLE_LEN: usize = 12;
const REPLY_LEN: usize = 8;
/// The longest a cancel waits for the cancelled transfers to come back.
const CANCEL_COLLECT: Duration = Duration::from_millis(20);

/// The real-device provider. open_lmc requires, in order: OPENLASER_NO_HW unset and OPENLASER_HW_PRESENT=1; a valid
/// bench session (S17); the board lock; vid/pid match (0x9980 → CloneBoard refusal, never firmware); the device
/// configured (macOS `claim_interface` cannot find interface 0 on an unconfigured device: call
/// `set_configuration(1)` only if the profile's `usb_set_configuration` is true, else refuse `UsbNotConfigured`);
/// then claims interface 0, bulk OUT 0x02, bulk IN 0x88. Holds a `keepawake` assertion for the session (§2.6).
#[derive(Debug, Default)]
pub struct UsbProvider {
    _private: (),
}

impl UsbProvider {
    pub fn new() -> Self {
        UsbProvider { _private: () }
    }
}

fn refused(r: Refusal) -> DriverError {
    DriverError::Refused(r)
}

fn transport(what: &str, e: impl std::fmt::Display) -> DriverError {
    DriverError::Transport(format!("{what}: {e}"))
}

fn xfer_to_driver(e: XferError) -> DriverError {
    match e {
        XferError::HardwareDisabled => refused(hardware_refusal()),
        other => DriverError::Transport(other.to_string()),
    }
}

/// detect() also needs OPENLASER_NO_HW unset and OPENLASER_HW_PRESENT=1.
impl DeviceProvider for UsbProvider {
    fn name(&self) -> &'static str {
        "usb"
    }

    fn detect(&self) -> Result<Vec<DetectedDevice>, DriverError> {
        if hardware_disabled() {
            return Err(refused(hardware_refusal()));
        }
        detect().map(|boards| boards.into_iter().map(DetectedDevice::from).collect()).map_err(xfer_to_driver)
    }

    fn open_lmc(&self, m: &GalvoMachine) -> Result<Box<dyn LmcTransport>, DriverError> {
        UsbLmc::open(m).map(|u| Box::new(u) as Box<dyn LmcTransport>)
    }

    fn open_link(&self, m: &GantryMachine) -> Result<Box<dyn ByteLink>, DriverError> {
        if hardware_disabled() {
            return Err(refused(hardware_refusal()));
        }
        bench::valid().map_err(refused)?;
        let link_err = |e: openlaser_driver::LinkError| match e {
            openlaser_driver::LinkError::HardwareDisabled => refused(hardware_refusal()),
            other => DriverError::Transport(other.to_string()),
        };
        match &m.link {
            GantryLink::Serial { port, baud } => SerialLink::open(port, *baud).map(|l| Box::new(l) as Box<dyn ByteLink>).map_err(link_err),
            GantryLink::Tcp { host, port } => TcpLink::connect(host, *port).map(|l| Box::new(l) as Box<dyn ByteLink>).map_err(link_err),
            GantryLink::ExportOnly => {
                Err(refused(Refusal::new(RefusalCode::NoDevice, "this machine is export-only: save the G-code and run it with your sender")))
            }
            GantryLink::Sim => Err(refused(Refusal::new(RefusalCode::NoDevice, "this machine profile uses the simulator link, not a real port"))),
        }
    }

    /// `bench::emission_cap_pct`.
    fn emission_cap_pct(&self) -> Result<Option<f64>, DriverError> {
        bench::emission_cap_pct().map_err(refused)
    }

    /// `bench::record_emission`.
    fn record_emission(&self) -> Result<(), DriverError> {
        bench::record_emission().map_err(refused)
    }
}

/// Which OUT transfer a completion belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutKind {
    Single,
    Packet,
    /// Its caller gave up on it (a stop or a timeout): its completion is discarded.
    Stale,
}

/// nusb 0.2.8. Writes are exactly 12 or 3072 bytes; reads 8 bytes; `submit` + `wait_next_complete(≤ 20 ms)` slices;
/// each transfer has a total budget of `transfer_timeout_ms` (250), after which it is `XferError::Timeout`. NO
/// reset, NO retry, NO `transfer_blocking` (§2.3 rule 15): every error is mapped to XferError and surfaces as a sticky
/// fault plus an emergency stop.
pub struct UsbLmc {
    out: Endpoint<Bulk, Out>,
    inp: Endpoint<Bulk, In>,
    _interface: Interface,
    _lock: BoardLock,
    budget: Duration,
    started: Instant,
    outs: VecDeque<OutKind>,
    /// IN replies still to come for abandoned reads: discarded when they complete.
    stale_replies: usize,
    /// The single being polled expects a reply.
    want_reply: bool,
    awake: Option<KeepAwake>,
    awake_tried: bool,
}

impl std::fmt::Debug for UsbLmc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UsbLmc").field("budget", &self.budget).field("outs", &self.outs).field("stale_replies", &self.stale_replies).finish_non_exhaustive()
    }
}

fn map_transfer(e: TransferError) -> XferError {
    match e {
        TransferError::Stall => XferError::Stall,
        TransferError::Disconnected => XferError::Disconnected,
        TransferError::Cancelled => XferError::Io("transfer cancelled".to_owned()),
        other => XferError::Io(other.to_string()),
    }
}

impl UsbLmc {
    /// Opens the board for one session (see [`UsbProvider`] for the order of the checks).
    pub fn open(m: &GalvoMachine) -> Result<UsbLmc, DriverError> {
        if hardware_disabled() {
            return Err(refused(hardware_refusal()));
        }
        if m.link != GalvoLink::Usb {
            return Err(refused(Refusal::new(RefusalCode::NoDevice, "this machine profile uses the simulator link, not USB")));
        }
        bench::valid().map_err(refused)?;
        let lock = BoardLock::acquire().map_err(refused)?;
        let all: Vec<DeviceInfo> = nusb::list_devices().wait().map_err(|e| transport("USB enumeration", e))?.filter(|d| d.vendor_id() == m.board.vid).collect();
        let wanted: Vec<&DeviceInfo> =
            all.iter().filter(|d| d.product_id() == m.board.pid).filter(|d| m.board.serial.as_deref().is_none_or(|s| d.serial_number() == Some(s))).collect();
        let info = match wanted.as_slice() {
            [one] => *one,
            [] if all.iter().any(|d| d.product_id() == CLONE_PID) => {
                return Err(refused(Refusal::new(
                    RefusalCode::CloneBoard,
                    "this is the clone board (9588:9980): it needs firmware that OpenLaser never loads; it is not supported",
                )));
            }
            [] => return Err(refused(Refusal::new(RefusalCode::NoDevice, "no JCZ LMC board (9588:9899) is connected: check the cable and the key switch"))),
            many => {
                return Err(refused(Refusal::new(
                    RefusalCode::NoDevice,
                    format!("{} LMC boards are connected: set the board serial in the machine profile to choose one", many.len()),
                )));
            }
        };
        let (on_winusb, note) = winusb_check(info);
        if !on_winusb {
            return Err(refused(Refusal::new(RefusalCode::NoDevice, note)));
        }
        let device = info.open().wait().map_err(|e| transport("open the board", e))?;
        if device.active_configuration().is_err() {
            if !m.usb_set_configuration {
                return Err(refused(Refusal::new(
                    RefusalCode::UsbNotConfigured,
                    "the board is unconfigured and this profile does not let OpenLaser configure it (usb_set_configuration is off until H12; it is changed only at a TTY)",
                )));
            }
            device.set_configuration(1).wait().map_err(|e| transport("set configuration 1", e))?;
        }
        let interface = device.claim_interface(0).wait().map_err(|e| transport("claim interface 0", e))?;
        let out = interface.endpoint::<Bulk, Out>(EP_OUT).map_err(|e| transport("bulk OUT 0x02", e))?;
        let inp = interface.endpoint::<Bulk, In>(EP_IN).map_err(|e| transport("bulk IN 0x88", e))?;
        Ok(UsbLmc {
            out,
            inp,
            _interface: interface,
            _lock: lock,
            budget: Duration::from_millis(u64::from(m.transfer_timeout_ms)),
            started: Instant::now(),
            outs: VecDeque::new(),
            stale_replies: 0,
            want_reply: false,
            awake: None,
            awake_tried: false,
        })
    }

    /// Created on the first transfer, so on Windows (`SetThreadExecutionState` is per thread) it belongs to the
    /// driver thread that owns this transport. Failing to get it is not fatal (the machine may sleep: HARDWARE.md).
    fn keep_awake(&mut self) {
        if !self.awake_tried {
            self.awake_tried = true;
            self.awake = keepawake::Builder::default()
                .idle(true)
                .reason("OpenLaser is driving a laser")
                .app_name("OpenLaser")
                .app_reverse_domain("org.openlaser.OpenLaser")
                .create()
                .ok();
        }
    }

    /// Anything still pending belongs to an operation its caller gave up on.
    fn abandon_pending(&mut self) {
        for k in self.outs.iter_mut() {
            *k = OutKind::Stale;
        }
        if self.want_reply {
            self.want_reply = false;
            self.stale_replies = self.stale_replies.saturating_add(1);
        }
    }

    fn begin(&mut self) -> Result<(), XferError> {
        if hardware_disabled() {
            return Err(XferError::HardwareDisabled);
        }
        self.keep_awake();
        self.abandon_pending();
        self.started = Instant::now();
        Ok(())
    }

    fn over_budget(&self) -> bool {
        self.started.elapsed() >= self.budget
    }

    /// Waits ≤ until `deadline` for the next OUT completion; returns its kind. A stale completion's status is
    /// ignored; any other failed completion is the error.
    fn next_out(&mut self, deadline: Instant) -> Result<Option<OutKind>, XferError> {
        if self.out.pending() == 0 {
            self.outs.clear();
            return Err(XferError::Io("no OUT transfer pending".to_owned()));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        let Some(c) = self.out.wait_next_complete(left) else { return Ok(None) };
        let kind = self.outs.pop_front().unwrap_or(OutKind::Stale);
        if kind != OutKind::Stale {
            c.status.map_err(map_transfer)?;
        }
        Ok(Some(kind))
    }

    /// Collects cancelled transfers for ≤ 20 ms so the bookkeeping matches what is still pending.
    fn collect_cancelled(&mut self) {
        let deadline = Instant::now() + CANCEL_COLLECT;
        while self.out.pending() > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if self.out.wait_next_complete(left).is_none() {
                break;
            }
            self.outs.pop_front();
        }
        while self.inp.pending() > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if self.inp.wait_next_complete(left).is_none() {
                break;
            }
            self.stale_replies = self.stale_replies.saturating_sub(1);
        }
        for k in self.outs.iter_mut() {
            *k = OutKind::Stale;
        }
        self.stale_replies = self.inp.pending();
        self.want_reply = false;
    }
}

impl LmcTransport for UsbLmc {
    fn submit_single(&mut self, cmd: Single) -> Result<(), XferError> {
        self.begin()?;
        let mut buf = Buffer::new(SINGLE_LEN);
        buf.extend_from_slice(&cmd.bytes());
        self.out.submit(buf);
        self.outs.push_back(OutKind::Single);
        if cmd.reads_reply() {
            let n = self.inp.max_packet_size();
            if n < REPLY_LEN {
                return Err(XferError::Io(format!("bulk IN max packet size {n} is smaller than a reply")));
            }
            self.inp.submit(Buffer::new(n));
            self.want_reply = true;
        }
        Ok(())
    }

    fn poll_single(&mut self, slice: Duration) -> Result<Option<Option<[u16; 4]>>, XferError> {
        let deadline = Instant::now() + slice.min(Duration::from_millis(20));
        while self.outs.contains(&OutKind::Single) {
            match self.next_out(deadline)? {
                Some(_) => {}
                None if self.over_budget() => return Err(XferError::Timeout),
                None => return Ok(None),
            }
        }
        if !self.want_reply {
            return Ok(Some(None));
        }
        loop {
            if self.inp.pending() == 0 {
                self.want_reply = false;
                self.stale_replies = 0;
                return Err(XferError::Io("the reply transfer was lost".to_owned()));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            let Some(c) = self.inp.wait_next_complete(left) else {
                return if self.over_budget() { Err(XferError::Timeout) } else { Ok(None) };
            };
            if self.stale_replies > 0 {
                self.stale_replies -= 1;
                continue;
            }
            self.want_reply = false;
            c.status.map_err(map_transfer)?;
            let data: &[u8] = &c.buffer;
            let Some(reply) = data.get(..REPLY_LEN) else {
                return Err(XferError::Io(format!("short reply: {} bytes", data.len())));
            };
            let mut words = [0u16; 4];
            let (pairs, _) = reply.as_chunks::<2>();
            for (w, pair) in words.iter_mut().zip(pairs) {
                *w = u16::from_le_bytes(*pair);
            }
            return Ok(Some(Some(words)));
        }
    }

    fn submit_packet(&mut self, pkt: &Packet) -> Result<(), XferError> {
        self.begin()?;
        let mut buf = Buffer::new(PACKET_LEN);
        buf.extend_from_slice(pkt.bytes());
        self.out.submit(buf);
        self.outs.push_back(OutKind::Packet);
        Ok(())
    }

    fn poll_packet(&mut self, slice: Duration) -> Result<Option<()>, XferError> {
        let deadline = Instant::now() + slice.min(Duration::from_millis(20));
        loop {
            if !self.outs.contains(&OutKind::Packet) {
                return Err(XferError::Io("no packet in flight".to_owned()));
            }
            match self.next_out(deadline)? {
                Some(OutKind::Packet) => return Ok(Some(())),
                Some(_) => {}
                None if self.over_budget() => return Err(XferError::Timeout),
                None => return Ok(None),
            }
        }
    }

    /// Last resort: an in-flight packet overran 20 ms during a stop (H23).
    fn cancel_all(&mut self) {
        self.out.cancel_all();
        self.inp.cancel_all();
        self.collect_cancelled();
    }

    /// Discard stale 8-byte replies after an abandoned read: completed ones are dropped, outstanding ones cancelled.
    fn drain_replies(&mut self) {
        if self.want_reply {
            self.want_reply = false;
            self.stale_replies = self.stale_replies.saturating_add(1);
        }
        while self.stale_replies > 0 && self.inp.pending() > 0 {
            if self.inp.wait_next_complete(Duration::ZERO).is_none() {
                break;
            }
            self.stale_replies -= 1;
        }
        if self.inp.pending() > 0 {
            self.inp.cancel_all();
            let deadline = Instant::now() + CANCEL_COLLECT;
            while self.inp.pending() > 0 {
                let left = deadline.saturating_duration_since(Instant::now());
                if self.inp.wait_next_complete(left).is_none() {
                    break;
                }
            }
        }
        self.stale_replies = self.inp.pending();
    }
}

/// Read-only enumeration (descriptors only; never opens a device).
pub fn detect() -> Result<Vec<DetectedBoard>, XferError> {
    if hardware_disabled() {
        return Err(XferError::HardwareDisabled);
    }
    let devices = nusb::list_devices().wait().map_err(|e| XferError::Io(format!("USB enumeration: {e}")))?;
    Ok(devices.filter(|d| d.vendor_id() == LMC_VID).map(|d| board(&d)).collect())
}

fn board(d: &DeviceInfo) -> DetectedBoard {
    let pid = d.product_id();
    let (supported, note) = match pid {
        LMC_PID => winusb_check(d),
        CLONE_PID => (false, "clone board (9588:9980): it needs firmware that OpenLaser never loads; not supported".to_owned()),
        other => (false, format!("BJJCZ board 9588:{other:04x} is not the LMC board OpenLaser drives (9588:9899)")),
    };
    DetectedBoard { vid: d.vendor_id(), pid, serial: d.serial_number().map(str::to_owned), location: location(d), supported, note }
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn location(d: &DeviceInfo) -> String {
    format!("bus {} address {}", d.bus_id(), d.device_address())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn location(_d: &DeviceInfo) -> String {
    "unknown".to_owned()
}

/// §2.6 item 4: on Windows nusb reaches the board only through WinUSB (Zadig: USBLMCV4, 9588:9899 → WinUSB).
#[cfg(target_os = "windows")]
fn winusb_check(d: &DeviceInfo) -> (bool, String) {
    match d.driver() {
        Some(drv) if drv.eq_ignore_ascii_case("winusb") => (true, "JCZ LMC (USBLMCV4) on WinUSB".to_owned()),
        other => (
            false,
            format!(
                "found, but not on WinUSB (driver: {}): switch it once with Zadig (USBLMCV4, 9588:9899 → WinUSB); EZCAD cannot use the board until its driver is put back",
                other.unwrap_or("none")
            ),
        ),
    }
}

#[cfg(not(target_os = "windows"))]
fn winusb_check(_d: &DeviceInfo) -> (bool, String) {
    (true, "JCZ LMC (USBLMCV4)".to_owned())
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedBoard {
    pub vid: u16,
    pub pid: u16,
    pub serial: Option<String>,
    pub location: String,
    pub supported: bool,
    pub note: String,
}

impl From<DetectedBoard> for DetectedDevice {
    fn from(b: DetectedBoard) -> Self {
        DetectedDevice { vid: b.vid, pid: b.pid, serial: b.serial, location: b.location, supported: b.supported, note: b.note }
    }
}

#[cfg(test)]
mod tests {
    use openlaser_machine::{GantryTool, MmPerS, WorkspaceOrigin};

    use super::*;

    /// No test may reach USB, a port or a socket (AGENTS.md §1): refuse to run at all if the environment enables
    /// hardware, before anything is called.
    fn assert_hardware_disabled() {
        assert!(hardware_disabled(), "tests must run with hardware disabled (OPENLASER_NO_HW=1, OPENLASER_HW_PRESENT unset)");
    }

    fn galvo() -> GalvoMachine {
        match openlaser_machine::jpt_m7_200().kind {
            openlaser_machine::Kinematics::Galvo(g) => g,
            openlaser_machine::Kinematics::Gantry(_) => panic!("the JPT M7 is a galvo"),
        }
    }

    #[test]
    fn every_open_and_detect_is_refused_while_hardware_is_disabled() {
        assert_hardware_disabled();
        let p = UsbProvider::new();
        let code = |e: DriverError| match e {
            DriverError::Refused(r) => r.code,
            other => panic!("{other:?}"),
        };
        assert_eq!(code(p.detect().unwrap_err()), RefusalCode::HardwareDisabled);
        assert_eq!(code(p.open_lmc(&galvo()).err().unwrap()), RefusalCode::HardwareDisabled);
        assert_eq!(code(UsbLmc::open(&galvo()).unwrap_err()), RefusalCode::HardwareDisabled);
        assert_eq!(detect().unwrap_err(), XferError::HardwareDisabled);
        let gantry = GantryMachine {
            bed_mm: [400.0, 400.0],
            origin: WorkspaceOrigin::BottomLeft,
            max_speed: MmPerS::new(250.0).unwrap(),
            travel_speed: MmPerS::new(250.0).unwrap(),
            s_max: 1000,
            tool: GantryTool::Laser { dynamic: true },
            air_assist: false,
            frame_before_mark: true,
            home_on_connect: false,
            link: GantryLink::Tcp { host: "127.0.0.1".into(), port: 9 },
        };
        assert_eq!(code(p.open_link(&gantry).err().unwrap()), RefusalCode::HardwareDisabled);
    }

    #[test]
    fn detected_boards_map_to_devices() {
        let b = DetectedBoard { vid: LMC_VID, pid: CLONE_PID, serial: None, location: "bus 1 address 4".into(), supported: false, note: "clone".into() };
        let d = DetectedDevice::from(b);
        assert_eq!((d.vid, d.pid, d.supported), (LMC_VID, CLONE_PID, false));
    }
}
