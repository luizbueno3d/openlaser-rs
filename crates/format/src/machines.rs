//! Machine profile files (S24): `<craft_settings::data_dir("OpenLaser")>/machines/<id>.json`.
//!
//! `DirProfileStore` is named only in `apps/openlaser` and `apps/openlaser-hw` (SPEC §2.3 rule 17); the engine does
//! not re-export it. WP0 stubs fail closed: no directory, no profiles, every write refused.

use std::path::{Path, PathBuf};

use openlaser_machine::{MachineId, MachineProfile, ProfileStore};

/// WP0 stub: None.
pub fn profiles_dir() -> Option<PathBuf> {
    None
}

/// Built-ins (`jpt_m7_200`, `jpt_m7_200_sim`, `om_gantries`) shadowed by saved files with the same id. A file
/// that fails to parse or fails `validate_profile` is skipped with a message (never half-applied).
/// WP0 stub: no profiles and one message.
pub fn load_all(dir: Option<&Path>) -> (Vec<MachineProfile>, Vec<String>) {
    let _ = dir;
    (Vec::new(), vec!["not implemented: openlaser_format::machines::load_all".to_owned()])
}

/// craft-atomic; ids restricted to [a-z0-9-]{1,64}.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirProfileStore {
    dir: PathBuf,
}

impl DirProfileStore {
    pub fn new(dir: PathBuf) -> Self {
        DirProfileStore { dir }
    }
}

/// WP0 stub: every write is refused.
impl ProfileStore for DirProfileStore {
    fn save(&self, p: &MachineProfile) -> Result<(), String> {
        Err(format!("not implemented: openlaser_format::machines::DirProfileStore::save({} in {})", p.id.0, self.dir.display()))
    }

    fn delete(&self, id: &MachineId) -> Result<(), String> {
        Err(format!("not implemented: openlaser_format::machines::DirProfileStore::delete({} in {})", id.0, self.dir.display()))
    }
}
