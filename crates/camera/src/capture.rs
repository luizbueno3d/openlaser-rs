//! Device enumeration, the `Capture` lifecycle (open → start → latest_frame → stop) and the pump thread.
//!
//! The pump is *latest-wins*: it blocks in `Camera::frame()` (which returns when the backend delivers a
//! frame), converts to RGBA and overwrites one shared slot. Consumers never see a queue; stale frames are
//! dropped by construction.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType};

use crate::{Error, Result};

/// Frame formats this build can turn into RGBA in pure Rust. MJPEG is deliberately absent: the `decoding`
/// feature (mozjpeg, a C toolchain) is off, so an MJPEG-only camera is refused as
/// [`Error::UnsupportedFormat`] instead of failing later inside the pump.
const DECODABLE: &[FrameFormat] = &[FrameFormat::YUYV, FrameFormat::NV12, FrameFormat::GRAY, FrameFormat::RAWRGB, FrameFormat::RAWBGR];

/// One enumerated camera (stable OS index plus a human name for dropdowns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub index: u32,
    pub name: String,
}

/// One decoded frame: tightly packed RGBA8, `width` × `height`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Enumerate every camera the OS reports. Never touches a device handle; safe to call lazily (panel open,
/// Refresh) so a missing or permission-denied camera can never block startup.
pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    let infos = nokhwa::query(ApiBackend::Auto).map_err(|e| Error::Enumerate(e.to_string()))?;
    Ok(infos.iter().filter_map(|info| info.index().as_index().ok().map(|index| DeviceInfo { index, name: info.human_name() })).collect())
}

/// What the pump publishes. `error` records the last stream/decode failure (a hard stream error also stops
/// the pump; `running` reports whether the pump thread is alive).
#[derive(Debug, Default)]
struct Shared {
    frame: Option<Frame>,
    error: Option<String>,
    running: bool,
}

/// An open camera. Owns the `nokhwa::Camera` until `start()` moves it onto the pump thread; `stop()`
/// signals the pump and joins it. Dropping a running `Capture` signals the pump and detaches: the thread
/// ends the stream when the current frame read returns (at most one frame interval).
pub struct Capture {
    camera: Option<nokhwa::Camera>,
    index: u32,
    name: String,
    /// The negotiated stream format once `open_stream` ran (for `status`).
    negotiated: Option<CameraFormat>,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    pump: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Capture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Capture").field("index", &self.index).field("name", &self.name).field("running", &self.is_running()).finish()
    }
}

impl Capture {
    /// Open the device at `index` (a `DeviceInfo::index` from [`list_devices`]). Prefers the highest
    /// resolution the device offers in a decodable format; a device that can only deliver MJPEG is
    /// refused with [`Error::UnsupportedFormat`].
    pub fn open(index: u32) -> Result<Capture> {
        let request = RequestedFormat::with_formats(RequestedFormatType::AbsoluteHighestResolution, DECODABLE);
        let camera = match nokhwa::Camera::new(CameraIndex::Index(index), request) {
            Ok(c) => c,
            Err(first) => {
                // Is this a camera that only compresses? Probe with every frame format the enum knows:
                // if that resolves where the decodable list failed, the honest error is UnsupportedFormat.
                let any = RequestedFormat::with_formats(RequestedFormatType::AbsoluteHighestResolution, nokhwa::utils::frame_formats());
                if let Ok(mut probe) = nokhwa::Camera::new(CameraIndex::Index(index), any) {
                    let formats = probe.compatible_fourcc().unwrap_or_default().iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
                    return Err(Error::UnsupportedFormat { index, formats: if formats.is_empty() { "MJPEG".to_owned() } else { formats } });
                }
                return Err(Error::Open { index, message: first.to_string() });
            }
        };
        let name = camera.info().human_name();
        Ok(Capture {
            camera: Some(camera),
            index,
            name,
            negotiated: None,
            shared: Arc::new(Mutex::new(Shared::default())),
            stop: Arc::new(AtomicBool::new(false)),
            pump: None,
        })
    }

    /// The device index this capture was opened with.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// The device name (for status rows).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The negotiated stream format once [`start`](Self::start) opened the stream.
    pub fn negotiated(&self) -> Option<CameraFormat> {
        self.negotiated
    }

    /// True while the pump thread is alive.
    pub fn is_running(&self) -> bool {
        self.shared.lock().map(|s| s.running).unwrap_or(false)
    }

    /// The last error the pump recorded (stream or decode), if any.
    pub fn last_error(&self) -> Option<String> {
        self.shared.lock().ok().and_then(|s| s.error.clone())
    }

    /// Start streaming. Spawns the pump thread; `latest_frame` then returns fresh frames. Refuses with
    /// [`Error::AlreadyRunning`] when already pumping.
    pub fn start(&mut self) -> Result<()> {
        if self.pump.is_some() {
            return Err(Error::AlreadyRunning);
        }
        let mut camera = self.camera.take().ok_or_else(|| Error::Stream("the camera device was already consumed".to_owned()))?;
        if let Err(e) = camera.open_stream() {
            self.camera = Some(camera);
            return Err(Error::Stream(e.to_string()));
        }
        self.negotiated = Some(camera.camera_format());
        self.stop.store(false, Ordering::Relaxed);
        let shared = Arc::clone(&self.shared);
        let stop = Arc::clone(&self.stop);
        if let Ok(mut s) = shared.lock() {
            s.running = true;
        }
        self.pump = Some(std::thread::spawn(move || pump(camera, shared, stop)));
        Ok(())
    }

    /// Stop streaming and join the pump (it returns within one frame interval; a backend that reports a
    /// hard error exits on its own). Refuses with [`Error::NotRunning`] when not pumping.
    pub fn stop(&mut self) -> Result<()> {
        let Some(pump) = self.pump.take() else {
            return Err(Error::NotRunning);
        };
        self.stop.store(true, Ordering::Relaxed);
        if pump.join().is_err() {
            log_pump_panic();
        }
        if let Ok(mut s) = self.shared.lock() {
            s.running = false;
        }
        Ok(())
    }

    /// The latest decoded frame, if the pump has produced one. `None` before the first frame, after a
    /// stream failure, or when not running.
    pub fn latest_frame(&self) -> Option<Frame> {
        self.shared.lock().ok().and_then(|s| s.frame.clone())
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The JoinHandle detaches here: the pump owns the camera, ends the stream and exits when the
        // in-flight frame() call returns. Joining in Drop could block a UI frame for one camera interval.
        if let Ok(mut s) = self.shared.lock() {
            s.running = false;
        }
    }
}

/// The pump loop: read (blocking until a frame arrives), decode to RGBA, overwrite the slot. A hard stream
/// error records itself and ends the loop; decode errors are recorded but the stream keeps running.
fn pump(mut camera: nokhwa::Camera, shared: Arc<Mutex<Shared>>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        match camera.frame() {
            Ok(buffer) => match decode(&buffer) {
                Ok(frame) => {
                    if let Ok(mut s) = shared.lock() {
                        s.frame = Some(frame);
                        s.error = None;
                    }
                }
                Err(e) => {
                    if let Ok(mut s) = shared.lock() {
                        s.error = Some(e.to_string());
                    }
                }
            },
            Err(e) => {
                if let Ok(mut s) = shared.lock() {
                    s.error = Some(e.to_string());
                    s.running = false;
                }
                break;
            }
        }
    }
    if camera.is_stream_open() {
        let _ = camera.stop_stream();
    }
    if let Ok(mut s) = shared.lock() {
        s.running = false;
    }
}

/// Buffer → RGBA `Frame` via nokhwa's pure-Rust decoders (YUYV, NV12, GRAY, RAWRGB/RAWBGR). An MJPEG buffer
/// (or anything the backend mislabelled) is a typed [`Error::Decode`], never a panic.
fn decode(buffer: &nokhwa::Buffer) -> Result<Frame> {
    let resolution = buffer.resolution();
    let image = buffer.decode_image::<RgbAFormat>().map_err(|e| Error::Decode { format: buffer.source_frame_format().to_string(), message: e.to_string() })?;
    Ok(Frame { rgba: image.into_raw(), width: resolution.width_x, height: resolution.height_y })
}

/// A panicking pump must not poison `stop()` — log and carry on (the shared slot already shows the loss).
fn log_pump_panic() {
    log::error!("camera pump thread panicked");
}
