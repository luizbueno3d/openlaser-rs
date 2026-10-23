//! GRBL byte links (serialport; std::net). WP0 stubs: refuse; nothing is opened.

use std::time::Duration;

use openlaser_driver::{ByteLink, LinkError};

use crate::hardware_disabled;

fn refused(what: &'static str) -> LinkError {
    if hardware_disabled() { LinkError::HardwareDisabled } else { LinkError::NotImplemented(what) }
}

/// serialport (`COMn` on Windows, §2.6 item 5).
#[derive(Debug)]
pub struct SerialLink {
    _private: (),
}

impl ByteLink for SerialLink {
    fn write_all(&mut self, bytes: &[u8], slice: Duration) -> Result<(), LinkError> {
        let _ = (bytes, slice);
        Err(refused("openlaser_transport::SerialLink::write_all"))
    }

    fn write_realtime(&mut self, byte: u8) -> Result<(), LinkError> {
        let _ = byte;
        Err(refused("openlaser_transport::SerialLink::write_realtime"))
    }

    fn read_available(&mut self, buf: &mut Vec<u8>, slice: Duration) -> Result<usize, LinkError> {
        let _ = (buf, slice);
        Err(refused("openlaser_transport::SerialLink::read_available"))
    }
}

/// std::net.
#[derive(Debug)]
pub struct TcpLink {
    _private: (),
}

impl ByteLink for TcpLink {
    fn write_all(&mut self, bytes: &[u8], slice: Duration) -> Result<(), LinkError> {
        let _ = (bytes, slice);
        Err(refused("openlaser_transport::TcpLink::write_all"))
    }

    fn write_realtime(&mut self, byte: u8) -> Result<(), LinkError> {
        let _ = byte;
        Err(refused("openlaser_transport::TcpLink::write_realtime"))
    }

    fn read_available(&mut self, buf: &mut Vec<u8>, slice: Duration) -> Result<usize, LinkError> {
        let _ = (buf, slice);
        Err(refused("openlaser_transport::TcpLink::read_available"))
    }
}
