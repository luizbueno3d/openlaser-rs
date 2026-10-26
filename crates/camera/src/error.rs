//! The typed camera error. Every variant carries a message a person can act on; nothing panics.

/// Everything `openlaser-camera` can fail with.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `list_devices` could not enumerate cameras (backend error, or the OS denied enumeration).
    #[error("camera enumeration failed: {0}")]
    Enumerate(String),
    /// `Capture::open` could not open the device at `index` (busy, unplugged or permission denied).
    #[error("camera {index}: {message}")]
    Open { index: u32, message: String },
    /// The device opened but offers no uncompressed stream we can decode (MJPEG-only; phase 6a ships no
    /// MJPEG decoder — see the crate docs).
    #[error("camera {index} offers only compressed formats ({formats}); this build decodes uncompressed video only")]
    UnsupportedFormat { index: u32, formats: String },
    /// `start` on a capture that is already pumping.
    #[error("capture is already running")]
    AlreadyRunning,
    /// `stop` on a capture that is not pumping.
    #[error("capture is not running")]
    NotRunning,
    /// `open_stream`/`stop_stream` failed.
    #[error("camera stream: {0}")]
    Stream(String),
    /// A delivered frame could not be converted to RGBA.
    #[error("cannot decode {format} frame: {message}")]
    Decode { format: String, message: String },
    /// A calibration fit was refused: under-determined, non-finite, duplicate, collinear or degenerate
    /// input (calibrate.rs — the refusal says which).
    #[error("camera calibration: {0}")]
    Calibrate(String),
}
