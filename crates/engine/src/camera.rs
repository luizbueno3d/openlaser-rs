//! The session's webcam state: one `openlaser_camera::Capture` behind the `camera.*` commands
//! (SPEC §2.2 engine edge). Camera state is *view state*: it lives in no `Snapshot`, is journaled by
//! nothing, touches no laser state, and is safe for `Access::Agent` callers (AGENTS.md §1.2 — design,
//! preview and view commands are all agent-legal; arming stays human-only elsewhere).

use openlaser_camera::{Capture, DeviceInfo, Frame, Pair, StoredCalibration};
use serde_json::{Value, json};

use crate::command::EngineError;
use crate::session::Session;

/// camera::Error → EngineError. The message is the camera crate's own (already actionable).
fn err(e: openlaser_camera::Error) -> EngineError {
    EngineError::Camera(e.to_string())
}

/// Wall-clock seconds for the persisted calibration's timestamp (informational only).
fn unix_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// `camera.calibrate.addPair` may accumulate at most this many pairs before `fit` is refused anyway.
const MAX_PAIRS: usize = openlaser_camera::MAX_PAIRS;

/// The map commands' clean refusal before a fit ever succeeded.
const NOT_CALIBRATED: &str = "the camera is not calibrated — collect at least 4 point pairs in the Camera Control panel, then Fit";

impl Session {
    /// `camera.list`: every camera the OS reports. Lazy by design — callers enumerate on demand (panel
    /// open, refresh, the command) so a missing or permission-denied camera can never stall startup.
    pub fn camera_devices(&self) -> Result<Vec<DeviceInfo>, EngineError> {
        openlaser_camera::list_devices().map_err(err)
    }

    /// The latest decoded frame for the GUI feed, if the pump has produced one.
    pub fn camera_frame(&self) -> Option<Frame> {
        self.camera.as_ref().and_then(Capture::latest_frame)
    }

    /// True while the pump thread is alive (the panel's Start/Stop enable states).
    pub fn camera_running(&self) -> bool {
        self.camera.as_ref().is_some_and(Capture::is_running)
    }

    /// The device index of the open capture, for the dropdown's current selection.
    pub fn camera_index(&self) -> Option<u32> {
        self.camera.as_ref().map(Capture::index)
    }

    /// The last stream/decode error the pump recorded, if any.
    pub fn camera_last_error(&self) -> Option<String> {
        self.camera.as_ref().and_then(Capture::last_error)
    }

    /// The calibration point pairs collected so far (`camera.calibrate.addPair`), for the wizard's dots.
    pub fn camera_cal_pairs(&self) -> &[Pair] {
        &self.camera_pairs
    }

    /// The active camera↔bed calibration, when `camera.calibrate.fit` (or the persisted file) has produced one.
    pub fn camera_calibration(&self) -> Option<&StoredCalibration> {
        self.camera_cal.as_ref()
    }

    /// The `calibration` block of `camera.status`.
    fn camera_calibration_json(&self) -> Value {
        match &self.camera_cal {
            None => json!({ "calibrated": false, "pairs": self.camera_pairs.len() }),
            Some(stored) => json!({
                "calibrated": true,
                "pairs": stored.pairs.len(),
                "kind": stored.calibration.kind(),
                "rmsErrorPx": stored.calibration.rms_error_px(),
                "deviceIndex": stored.device_index,
                "deviceName": stored.device_name,
                "unixSecs": stored.unix_secs,
            }),
        }
    }

    /// `camera.status` and the status half of `camera.start`/`camera.stop`.
    pub fn camera_status_json(&self) -> Value {
        let calibration = self.camera_calibration_json();
        match &self.camera {
            None => json!({ "open": false, "running": false, "calibration": calibration }),
            Some(c) => {
                let negotiated = c.negotiated().map(|f| {
                    let r = f.resolution();
                    json!({ "width": r.width_x, "height": r.height_y, "frameRate": f.frame_rate(), "format": f.format().to_string() })
                });
                json!({
                    "open": true,
                    "running": c.is_running(),
                    "index": c.index(),
                    "name": c.name(),
                    "format": negotiated,
                    "hasFrame": c.latest_frame().is_some(),
                    "lastError": c.last_error(),
                    "calibration": calibration,
                })
            }
        }
    }

    /// `camera.calibrate.addPair`: accumulate one correspondence. Duplicates of an earlier pair's camera or
    /// bed point are refused at add time (the wizard clicks live — a repeat click should say so, not wait
    /// for `fit` to fail).
    pub(crate) fn camera_add_pair(&mut self, pair: Pair) -> Result<Value, EngineError> {
        const ID: &str = "camera.calibrate.addPair";
        if self.camera_pairs.len() >= MAX_PAIRS {
            return Err(EngineError::Camera(format!("at most {MAX_PAIRS} calibration pairs — fit or clear first")));
        }
        for (i, existing) in self.camera_pairs.iter().enumerate() {
            let dup = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-6;
            if dup(existing.cam, pair.cam) {
                return Err(EngineError::bad(ID, "/camX", format!("that image point is already pair {i}")));
            }
            if dup(existing.bed, pair.bed) {
                return Err(EngineError::bad(ID, "/bedX", format!("that bed point is already pair {i}")));
            }
        }
        self.camera_pairs.push(pair);
        Ok(json!({ "pairs": self.camera_pairs.len() }))
    }

    /// `camera.calibrate.removeLast`: drop the newest pair.
    pub(crate) fn camera_remove_last_pair(&mut self) -> Result<Value, EngineError> {
        if self.camera_pairs.pop().is_none() {
            return Err(EngineError::Camera("no calibration pairs to remove".to_owned()));
        }
        Ok(json!({ "pairs": self.camera_pairs.len() }))
    }

    /// `camera.calibrate.fit`: fit the accumulated pairs, keep the result in the session and persist it
    /// through the injected store. A persist failure refuses the command (the session keeps its old state)
    /// so a calibration the user thinks is permanent is never silently kept only in memory.
    pub(crate) fn camera_fit(&mut self) -> Result<Value, EngineError> {
        let calibration = openlaser_camera::fit(&self.camera_pairs).map_err(err)?;
        let stored = StoredCalibration {
            device_index: self.camera.as_ref().map(Capture::index),
            device_name: self.camera.as_ref().map(Capture::name).unwrap_or_default().to_owned(),
            pairs: self.camera_pairs.clone(),
            calibration,
            unix_secs: unix_secs(),
        };
        let mut persisted = false;
        if let Some(store) = &self.camera_store {
            store.save(&stored).map_err(|e| EngineError::Camera(format!("cannot persist the calibration: {e}")))?;
            persisted = true;
        }
        let kind = stored.calibration.kind();
        let rms = stored.calibration.rms_error_px();
        self.camera_cal = Some(stored);
        self.camera_pairs.clear();
        Ok(json!({ "pairs": self.camera_cal.as_ref().map(|s| s.pairs.len()).unwrap_or(0), "kind": kind, "rmsErrorPx": rms, "persisted": persisted }))
    }

    /// `camera.calibrate.clear`: drop the collected pairs AND the fitted calibration (session and store).
    /// The canvas underlay disappears with it — never a stale map.
    pub(crate) fn camera_clear_calibration(&mut self) -> Result<Value, EngineError> {
        self.camera_pairs.clear();
        self.camera_cal = None;
        if let Some(store) = &self.camera_store {
            store.clear().map_err(|e| EngineError::Camera(format!("cannot remove the persisted calibration: {e}")))?;
        }
        Ok(json!({ "pairs": 0, "calibrated": false }))
    }

    /// `camera.map.bedToPx {x, y}` → `{camX, camY}`.
    pub(crate) fn camera_bed_to_px(&mut self, x: f64, y: f64) -> Result<Value, EngineError> {
        let cal = self.camera_cal.as_ref().ok_or_else(|| EngineError::Camera(NOT_CALIBRATED.to_owned()))?;
        let [u, v] = cal.calibration.bed_to_px([x, y]).ok_or_else(|| EngineError::Camera("that bed point maps outside the camera's frame".to_owned()))?;
        Ok(json!({ "camX": u, "camY": v }))
    }

    /// `camera.map.pxToBed {x, y}` → `{bedX, bedY}`.
    pub(crate) fn camera_px_to_bed(&mut self, u: f64, v: f64) -> Result<Value, EngineError> {
        let cal = self.camera_cal.as_ref().ok_or_else(|| EngineError::Camera(NOT_CALIBRATED.to_owned()))?;
        let [x, y] = cal.calibration.px_to_bed([u, v]).ok_or_else(|| EngineError::Camera("that image point maps outside the calibrated field".to_owned()))?;
        Ok(json!({ "bedX": x, "bedY": y }))
    }

    /// `camera.start {index?}`: open (or reuse) the device and run the pump. `index` defaults to the open
    /// capture, else the first enumerated camera; no camera at all is a typed `camera:` error.
    pub(crate) fn camera_start(&mut self, index: Option<u32>) -> Result<Value, EngineError> {
        let index = match index {
            Some(i) => i,
            None => match self.camera.as_ref().map(Capture::index) {
                Some(i) => i,
                None => self.camera_devices()?.first().map(|d| d.index).ok_or_else(|| EngineError::Camera("no camera found".to_owned()))?,
            },
        };
        if self.camera.as_ref().is_none_or(|c| c.index() != index) {
            // Releasing the old Capture stops its pump and frees the device before the new open.
            self.camera = Some(Capture::open(index).map_err(err)?);
        }
        if let Some(c) = self.camera.as_mut()
            && !c.is_running()
        {
            c.start().map_err(err)?;
        }
        Ok(self.camera_status_json())
    }

    /// `camera.stop`: stop the pump and release the device (the panel can reopen it any time). Stopping a
    /// camera that was never started is a no-op success — `stop` is always safe to repeat.
    pub(crate) fn camera_stop(&mut self) -> Result<Value, EngineError> {
        if let Some(mut c) = self.camera.take()
            && c.is_running()
        {
            c.stop().map_err(err)?;
        }
        Ok(self.camera_status_json())
    }
}
