//! GRBL byte links: serial (`serialport`, `COMn` on Windows, §2.6 item 5) and TCP (`std::net`, e.g. a GRBL-over-WiFi
//! board). Both refuse to open while hardware is disabled (S23); both hold a keep-awake assertion once they write.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use keepawake::KeepAwake;
use openlaser_driver::{ByteLink, LinkError};

use crate::hardware_disabled;

/// How long a TCP connect may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// The smallest timeout handed to the OS (a zero timeout means "block forever" or is an error).
const MIN_TIMEOUT: Duration = Duration::from_millis(1);
/// Realtime bytes are written with this timeout.
const REALTIME_TIMEOUT: Duration = Duration::from_millis(20);

fn io(e: &std::io::Error) -> LinkError {
    match e.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => LinkError::Timeout,
        ErrorKind::BrokenPipe | ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::UnexpectedEof | ErrorKind::NotConnected => {
            LinkError::Closed
        }
        _ => LinkError::Io(e.to_string()),
    }
}

fn keep_awake() -> Option<KeepAwake> {
    keepawake::Builder::default()
        .idle(true)
        .reason("OpenLaser is driving a laser")
        .app_name("OpenLaser")
        .app_reverse_domain("org.openlaser.OpenLaser")
        .create()
        .ok()
}

/// serialport (`COMn` on Windows, §2.6 item 5).
pub struct SerialLink {
    port: Box<dyn serialport::SerialPort>,
    awake: Option<KeepAwake>,
}

impl std::fmt::Debug for SerialLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SerialLink").field("port", &self.port.name()).field("awake", &self.awake.is_some()).finish()
    }
}

impl SerialLink {
    /// Opens `path` at `baud` (8N1). Refused while hardware is disabled.
    pub fn open(path: &str, baud: u32) -> Result<Self, LinkError> {
        if hardware_disabled() {
            return Err(LinkError::HardwareDisabled);
        }
        let port = serialport::new(path, baud).timeout(REALTIME_TIMEOUT).open().map_err(|e| LinkError::Io(format!("{path}: {e}")))?;
        Ok(SerialLink { port, awake: None })
    }

    fn set_timeout(&mut self, d: Duration) -> Result<(), LinkError> {
        self.port.set_timeout(d.max(MIN_TIMEOUT)).map_err(|e| LinkError::Io(e.to_string()))
    }
}

impl ByteLink for SerialLink {
    fn write_all(&mut self, bytes: &[u8], slice: Duration) -> Result<(), LinkError> {
        if self.awake.is_none() {
            self.awake = keep_awake();
        }
        self.set_timeout(slice)?;
        self.port.write_all(bytes).map_err(|e| io(&e))?;
        self.port.flush().map_err(|e| io(&e))
    }

    fn write_realtime(&mut self, byte: u8) -> Result<(), LinkError> {
        self.set_timeout(REALTIME_TIMEOUT)?;
        self.port.write_all(&[byte]).map_err(|e| io(&e))?;
        self.port.flush().map_err(|e| io(&e))
    }

    fn read_available(&mut self, buf: &mut Vec<u8>, slice: Duration) -> Result<usize, LinkError> {
        self.set_timeout(slice)?;
        let mut tmp = [0u8; 512];
        match self.port.read(&mut tmp) {
            Ok(n) => {
                buf.extend_from_slice(tmp.get(..n).unwrap_or_default());
                Ok(n)
            }
            Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => Ok(0),
            Err(e) => Err(io(&e)),
        }
    }
}

/// std::net.
pub struct TcpLink {
    stream: TcpStream,
    awake: Option<KeepAwake>,
}

impl std::fmt::Debug for TcpLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TcpLink").field("peer", &self.stream.peer_addr().ok()).field("awake", &self.awake.is_some()).finish()
    }
}

impl TcpLink {
    /// Connects to `host:port` (≤ 2 s). Refused while hardware is disabled.
    pub fn connect(host: &str, port: u16) -> Result<Self, LinkError> {
        if hardware_disabled() {
            return Err(LinkError::HardwareDisabled);
        }
        let addr = (host, port)
            .to_socket_addrs()
            .map_err(|e| LinkError::Io(format!("{host}:{port}: {e}")))?
            .next()
            .ok_or_else(|| LinkError::Io(format!("{host}:{port}: no address")))?;
        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| LinkError::Io(format!("{addr}: {e}")))?;
        stream.set_nodelay(true).map_err(|e| io(&e))?;
        Ok(TcpLink { stream, awake: None })
    }
}

impl ByteLink for TcpLink {
    fn write_all(&mut self, bytes: &[u8], slice: Duration) -> Result<(), LinkError> {
        if self.awake.is_none() {
            self.awake = keep_awake();
        }
        self.stream.set_write_timeout(Some(slice.max(MIN_TIMEOUT))).map_err(|e| io(&e))?;
        self.stream.write_all(bytes).map_err(|e| io(&e))
    }

    fn write_realtime(&mut self, byte: u8) -> Result<(), LinkError> {
        self.stream.set_write_timeout(Some(REALTIME_TIMEOUT)).map_err(|e| io(&e))?;
        self.stream.write_all(&[byte]).map_err(|e| io(&e))
    }

    fn read_available(&mut self, buf: &mut Vec<u8>, slice: Duration) -> Result<usize, LinkError> {
        self.stream.set_read_timeout(Some(slice.max(MIN_TIMEOUT))).map_err(|e| io(&e))?;
        let mut tmp = [0u8; 512];
        match self.stream.read(&mut tmp) {
            Ok(0) => Err(LinkError::Closed),
            Ok(n) => {
                buf.extend_from_slice(tmp.get(..n).unwrap_or_default());
                Ok(n)
            }
            Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => Ok(0),
            Err(e) => Err(io(&e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_refuse_to_open_while_hardware_is_disabled() {
        assert!(hardware_disabled(), "tests must run with hardware disabled (OPENLASER_NO_HW=1, OPENLASER_HW_PRESENT unset)");
        assert_eq!(SerialLink::open("/dev/null", 115_200).unwrap_err(), LinkError::HardwareDisabled);
        assert_eq!(TcpLink::connect("127.0.0.1", 9).unwrap_err(), LinkError::HardwareDisabled);
    }
}
