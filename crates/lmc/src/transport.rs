//! The only USB-shaped seam (PLAN §3.4). Implemented by transport::UsbLmc, lmc_sim::FakeLmc and test fakes.

use std::time::Duration;

use crate::packet::{Packet, Single};

pub trait LmcTransport: Send {
    fn submit_single(&mut self, cmd: Single) -> Result<(), XferError>;
    /// Ok(None) = still pending after `slice` (≤ 20 ms); Ok(Some(None)) = no reply expected; Ok(Some(Some(r))) = reply.
    fn poll_single(&mut self, slice: Duration) -> Result<Option<Option<[u16; 4]>>, XferError>;
    /// At most one in flight.
    fn submit_packet(&mut self, pkt: &Packet) -> Result<(), XferError>;
    /// Ok(None) = in flight.
    fn poll_packet(&mut self, slice: Duration) -> Result<Option<()>, XferError>;
    /// Last resort: an in-flight packet overran 20 ms during a stop (H23).
    fn cancel_all(&mut self);
    /// Discard stale 8-byte replies after an abandoned read.
    fn drain_replies(&mut self);
}

/// Lets the driver hold `Box<dyn LmcTransport>`.
impl<T: LmcTransport + ?Sized> LmcTransport for Box<T> {
    fn submit_single(&mut self, cmd: Single) -> Result<(), XferError> {
        (**self).submit_single(cmd)
    }

    fn poll_single(&mut self, slice: Duration) -> Result<Option<Option<[u16; 4]>>, XferError> {
        (**self).poll_single(slice)
    }

    fn submit_packet(&mut self, pkt: &Packet) -> Result<(), XferError> {
        (**self).submit_packet(pkt)
    }

    fn poll_packet(&mut self, slice: Duration) -> Result<Option<()>, XferError> {
        (**self).poll_packet(slice)
    }

    fn cancel_all(&mut self) {
        (**self).cancel_all();
    }

    fn drain_replies(&mut self) {
        (**self).drain_replies();
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum XferError {
    #[error("transfer timed out")]
    Timeout,
    #[error("device disconnected")]
    Disconnected,
    #[error("endpoint stalled")]
    Stall,
    #[error("hardware access is disabled in this build or environment")]
    HardwareDisabled,
    #[error("usb: {0}")]
    Io(String),
    /// WP0 stub marker (SPEC §3.0); removed at I3.
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}
