//! Camera-guided-placement calibration file (phase 6b): `<data_dir>/camera-calibration.json`.
//!
//! The calibration belongs to the physical setup — the camera bolted over the bed — not to any project,
//! so it lives next to `machines/` and `bench-session.json`, not in `.olp`. One file per user (one camera
//! looking at the field); the file records which device it was fitted on so a stale match is visible.

use std::fs;
use std::path::PathBuf;

use openlaser_camera::{CalibrationStore, StoredCalibration};

/// The per-user calibration file path, when the platform provides a data directory (§2.6 paths:
/// `%LOCALAPPDATA%\OpenLaser` on Windows, `~/Library/Application Support/OpenLaser` on macOS).
pub fn calibration_path() -> Option<PathBuf> {
    craft_settings::paths::data_dir("OpenLaser").map(|dir| dir.join("camera-calibration.json"))
}

/// Atomic store over [`calibration_path`], injected into the session like `DirProfileStore` (S24). The
/// engine stays filesystem-free: it holds the `CalibrationStore` trait, and only apps that can name this
/// type (a `format` edge) can persist calibrations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirCalibrationStore {
    path: PathBuf,
}

impl DirCalibrationStore {
    /// A store at an explicit path (the app passes `calibration_path()`; tests a temp file).
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Where this store reads and writes.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl CalibrationStore for DirCalibrationStore {
    /// The stored calibration. An absent or unparsable file is `None` — a corrupt calibration must never
    /// block startup; the user simply calibrates again.
    fn load(&self) -> Option<StoredCalibration> {
        let bytes = fs::read(&self.path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn save(&self, calibration: &StoredCalibration) -> Result<(), String> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        }
        let bytes = serde_json::to_vec_pretty(calibration).map_err(|error| error.to_string())?;
        craft_atomic::atomic_write(&self.path, &bytes).map_err(|error| error.to_string())
    }

    fn clear(&self) -> Result<(), String> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use openlaser_camera::{Calibration, Pair};

    use super::*;

    fn stored() -> StoredCalibration {
        StoredCalibration {
            device_index: Some(0),
            device_name: "Desk Cam".to_owned(),
            pairs: vec![
                Pair { cam: [10.0, 20.0], bed: [-50.0, -50.0] },
                Pair { cam: [100.0, 25.0], bed: [50.0, -50.0] },
                Pair { cam: [95.0, 150.0], bed: [50.0, 50.0] },
                Pair { cam: [15.0, 145.0], bed: [-50.0, 50.0] },
            ],
            calibration: Calibration::Affine { fwd: [1.0, 0.0, 60.0, 0.0, 1.0, 80.0], inv: [1.0, 0.0, -60.0, 0.0, 1.0, -80.0], rms_px: 0.3 },
            unix_secs: 1_760_000_000,
        }
    }

    #[test]
    fn save_load_clear_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = DirCalibrationStore::new(dir.path().join("camera-calibration.json"));
        assert!(store.load().is_none());
        store.save(&stored()).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded, stored());
        store.clear().unwrap();
        assert!(store.load().is_none());
        // Clearing an absent file is success.
        store.clear().unwrap();
    }

    #[test]
    fn a_corrupt_file_loads_none_and_save_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("camera-calibration.json");
        fs::write(&path, b"{not json").unwrap();
        let store = DirCalibrationStore::new(path);
        assert!(store.load().is_none());
        store.save(&stored()).unwrap();
        assert!(store.load().is_some());
    }
}
