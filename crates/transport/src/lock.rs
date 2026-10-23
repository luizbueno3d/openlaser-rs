//! The board lock: `<data_dir>/OpenLaser/board.lock` (§2.6 paths), shared with the prototype (OM v1 patch 12,
//! `flock`); `File::try_lock`.

use std::fs::File;

use openlaser_types::{Refusal, RefusalCode};

/// `<data_dir>/OpenLaser/board.lock` (§2.6 paths), shared with the prototype (OM v1 patch 12, `flock`); File::try_lock.
#[derive(Debug)]
pub struct BoardLock {
    _file: File,
}

impl BoardLock {
    /// WP0 stub: refuses.
    pub fn acquire() -> Result<Self, Refusal> {
        Err(Refusal::new(RefusalCode::NotImplemented, "not implemented: openlaser_transport::BoardLock::acquire"))
    }
}
