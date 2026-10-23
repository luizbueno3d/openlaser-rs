//! The LMC protocol, pure (SPEC §3.12, §5.2; owned by WP4). No I/O: [`LmcTransport`] is the only USB-shaped seam.
//!
//! WP0 stubs: the data types and the transport trait are complete; encoders return `Error::NotImplemented`,
//! decoders return empty results.
#![forbid(unsafe_code)]
#![deny(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_possible_wrap)]

#[cfg(all(feature = "py-compat", not(debug_assertions)))]
compile_error!("openlaser-lmc: the `py-compat` feature is test-only (dev-dependencies) and needs debug assertions");

mod decode;
mod encode;
pub mod op;
mod packet;
mod transport;

#[cfg(feature = "py-compat")]
pub mod py_compat;

pub use decode::{Decoded, Listing, ListingLine, decode_packet, disassemble, disassemble_frame, emission_summary, emitted_segments};
pub use encode::{LmcFrameJob, LmcJob, LmcList, encode_frame, encode_job};
pub use packet::{PACKET_LEN, Packet, SLOTS, Single, StatusWord, init_sequence, status_from_reply};
pub use transport::{LmcTransport, XferError};

// Re-exports for callers that name these through the protocol crate (SPEC §3.0).
pub use openlaser_machine::GalvoMachine;
pub use openlaser_plan::JobPlan;
pub use openlaser_safety::{EmitPlan, FramePlan};
pub use openlaser_types::{EmissionSummary, EmittedSegment, Fingerprint, FrameSource, FrameStyle, LayerKey, MachineId, Notice};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("point ({x}, {y}) mm is not encodable")]
    NotEncodable { x: f64, y: f64 },
    #[error("encode: {0}")]
    Encode(#[from] openlaser_types::galvo::EncodeError),
    #[error("section has no galvo parameters")]
    NotGalvo,
    #[error("empty job")]
    Empty,
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}
