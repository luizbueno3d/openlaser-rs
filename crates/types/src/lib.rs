//! OpenLaser's shared vocabulary (SPEC §3.1).
//!
//! Unit newtypes, ids, [`Fingerprint`], [`DeviceKind`], the `serde_secs`/`serde_ms` duration helpers, monotonic clocks, refusal and notice codes, the [`EmissionSummary`]
//! decoded from bytes, [`FrameReceipt`], the opaque [`ArmToken`] (check only; minted by `openlaser-arm`) and the
//! T0 numeric encoders in [`galvo`].
//!
//! Ownership (SPEC §7.12): WP1 owns this crate except `src/arm.rs` and `tests/arm_*.rs` (WP3, red zone),
//! `src/galvo.rs` and `tests/t0_*.rs` (WP4), and this file and `Cargo.toml` (WP0, fixed).
#![forbid(unsafe_code)]
#![deny(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_possible_wrap)]

pub mod arm;
pub mod emission;
pub mod frame;
pub mod galvo;
pub mod ids;
pub mod refusal;
mod serde_dur;
pub mod time;
pub mod units;

pub use arm::{ARM_TOKEN_TTL, ArmToken};
pub use emission::{Bounds, EmissionSummary, EmittedSegment, GalvoWords, PassRow, SectionMode};
pub use frame::{FRAME_RECEIPT_MAX_AGE, FrameMode, FrameOrigin, FrameReceipt, FrameSource, FrameStyle};
pub use ids::{DeviceKind, Fingerprint, LayerKey, MachineId, ObjectId, SessionId, TextAlign};
pub use refusal::{Notice, NoticeCode, Refusal, RefusalCode};
pub use serde_dur::{serde_ms, serde_secs};
pub use std::time::Duration;
pub use time::{Clock, MonoTime, SystemClock, VirtualClock};
pub use units::{Deg, Hz, Mm, MmPerS, Ms, Ns, Percent, Rgb, UnitError, Us};
