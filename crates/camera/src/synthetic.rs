//! `SyntheticSource`: a deterministic fake camera for tests and demos (feature `synthetic`, never default).
//! UI and engine tests can exercise the whole capture path — open, start, latest-wins frames, stop —
//! without a webcam. `frame(n)` is a pure function of the tick number, so tests compare pixels exactly.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::{Error, Frame, Result};

/// A fake camera producing an animated test pattern at `fps`. Same lifecycle as [`crate::Capture`]:
/// `open` → `start` → `latest_frame` → `stop`.
pub struct SyntheticSource {
    width: u32,
    height: u32,
    frame_time: Duration,
    tick: Arc<AtomicU64>,
    latest: Arc<Mutex<Option<Frame>>>,
    stop: Arc<AtomicBool>,
    pump: Option<JoinHandle<()>>,
}

impl SyntheticSource {
    /// A source producing `width`×`height` frames at `fps` (clamped to 1..=240).
    pub fn open(width: u32, height: u32, fps: u32) -> Result<SyntheticSource> {
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            return Err(Error::Open { index: 0, message: format!("synthetic frame {width}x{height} is outside 1..=8192") });
        }
        let fps = fps.clamp(1, 240);
        Ok(SyntheticSource {
            width,
            height,
            frame_time: Duration::from_secs_f64(1.0 / f64::from(fps)),
            tick: Arc::new(AtomicU64::new(0)),
            latest: Arc::new(Mutex::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
            pump: None,
        })
    }

    /// True while the pump thread is alive.
    pub fn is_running(&self) -> bool {
        self.pump.is_some()
    }

    /// Start generating frames.
    pub fn start(&mut self) -> Result<()> {
        if self.pump.is_some() {
            return Err(Error::AlreadyRunning);
        }
        self.stop.store(false, Ordering::Relaxed);
        let (tick, latest, stop) = (Arc::clone(&self.tick), Arc::clone(&self.latest), Arc::clone(&self.stop));
        let (width, height, frame_time) = (self.width, self.height, self.frame_time);
        self.pump = Some(std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let n = tick.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut slot) = latest.lock() {
                    *slot = Some(frame(n, width, height));
                }
                std::thread::sleep(frame_time);
            }
        }));
        Ok(())
    }

    /// Stop and join the pump.
    pub fn stop(&mut self) -> Result<()> {
        let Some(pump) = self.pump.take() else {
            return Err(Error::NotRunning);
        };
        self.stop.store(true, Ordering::Relaxed);
        if pump.join().is_err() {
            log::error!("synthetic camera pump thread panicked");
        }
        Ok(())
    }

    /// The latest generated frame.
    pub fn latest_frame(&self) -> Option<Frame> {
        self.latest.lock().ok().and_then(|f| f.clone())
    }
}

impl Drop for SyntheticSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The deterministic pattern at tick `n`: a vertical gradient with a white bar sweeping left→right and a
/// grey box bouncing — enough motion for a live-feed test to tell frames apart by tick alone.
fn frame(n: u64, width: u32, height: u32) -> Frame {
    let w = width as usize;
    let h = height as usize;
    let mut rgba = vec![0u8; w.saturating_mul(h).saturating_mul(4)];
    let bar = (n as usize) % w.max(1);
    let bx = (n as usize * 7) % w.max(1);
    let by = (n as usize * 5) % h.max(1);
    let box_half = (w.min(h) / 8).max(2);
    for (y, row) in rgba.chunks_exact_mut(w * 4).enumerate() {
        for (x, px) in row.chunks_exact_mut(4).enumerate() {
            let grad = (y * 255 / h.max(1)) as u8;
            let mut p = [grad, (x * 255 / w.max(1)) as u8, 96u8, 255u8];
            if x == bar || (x.abs_diff(bx) < box_half && y.abs_diff(by) < box_half) {
                p = [255, 255, 255, 255];
            }
            px.copy_from_slice(&p);
        }
    }
    Frame { rgba, width, height }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_moving() {
        assert_eq!(frame(3, 64, 48), frame(3, 64, 48));
        assert_ne!(frame(3, 64, 48), frame(4, 64, 48));
        let f = frame(0, 10, 8);
        assert_eq!(f.rgba.len(), 10 * 8 * 4);
        assert_eq!(f.width, 10);
        assert_eq!(f.height, 8);
    }

    #[test]
    fn lifecycle_latest_wins() {
        let mut s = SyntheticSource::open(16, 12, 60).expect("open");
        s.start().expect("start");
        std::thread::sleep(Duration::from_millis(120));
        let f = s.latest_frame().expect("a frame after start");
        assert_eq!((f.width, f.height), (16, 12));
        s.stop().expect("stop");
        assert!(matches!(s.stop(), Err(Error::NotRunning)));
        assert!(matches!(s.start().and_then(|()| s.start()), Err(Error::AlreadyRunning)));
        s.stop().expect("stop again");
    }

    #[test]
    fn rejects_bad_geometry() {
        assert!(SyntheticSource::open(0, 10, 30).is_err());
        assert!(SyntheticSource::open(10, 0, 30).is_err());
    }
}
