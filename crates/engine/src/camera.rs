//! The session's webcam state: one `openlaser_camera::Capture` behind the `camera.*` commands
//! (SPEC §2.2 engine edge). Camera state is *view state*: it lives in no `Snapshot`, is journaled by
//! nothing, touches no laser state, and is safe for `Access::Agent` callers (AGENTS.md §1.2 — design,
//! preview and view commands are all agent-legal; arming stays human-only elsewhere).

use openlaser_camera::{Capture, DeviceInfo, Frame};
use serde_json::{Value, json};

use crate::command::EngineError;
use crate::session::Session;

/// camera::Error → EngineError. The message is the camera crate's own (already actionable).
fn err(e: openlaser_camera::Error) -> EngineError {
    EngineError::Camera(e.to_string())
}

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

    /// `camera.status` and the status half of `camera.start`/`camera.stop`.
    pub fn camera_status_json(&self) -> Value {
        match &self.camera {
            None => json!({ "open": false, "running": false }),
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
                })
            }
        }
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
