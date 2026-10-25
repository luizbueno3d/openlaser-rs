//! Webcam capture for camera-guided placement (SPEC §2.2 `camera`): device enumeration and a latest-wins
//! background pump that converts each frame to RGBA, built on `nokhwa`'s native backends (AVFoundation on
//! macOS, MSMF on Windows, v4l on Linux — `input-native` only, so no C toolchain and no compressed-codec
//! decoders).
//!
//! This crate knows nothing about lasers, documents or egui (§2.3 rule 2 keeps it out of the GUI-less rule
//! set): it is a pure capture source. `engine` owns the one live [`Capture`] and exposes `camera.*` commands;
//! `ui-egui` draws [`Frame`]s. A camera that only offers MJPEG refuses with [`Error::UnsupportedFormat`]
//! — this build compiles no MJPEG decoder (phase 6a; a pure-Rust decoder may land later).
//!
//! Safety (AGENTS.md §1): the camera is view state only. Nothing here can touch laser state, and the pump
//! thread never blocks a caller longer than one frame interval.

#![forbid(unsafe_code)]

mod capture;
mod error;
#[cfg(feature = "synthetic")]
mod synthetic;

pub use capture::{Capture, DeviceInfo, Frame, list_devices};
pub use error::Error;
#[cfg(feature = "synthetic")]
pub use synthetic::SyntheticSource;

/// The crate-wide result type.
pub type Result<T> = std::result::Result<T, Error>;
