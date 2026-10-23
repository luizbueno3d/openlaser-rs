//! `.olp` read/write (SPEC §3.5, S14) and machine profile files (S24); owned by WP1. WP0 stubs.
#![forbid(unsafe_code)]

pub mod machines;

use std::path::Path;

pub use openlaser_doc::Project;

pub const FORMAT: &str = "openlaser-project";
pub const FORMAT_VERSION: u32 = 1;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not an OpenLaser project: {0}")]
    Corrupt(String),
    #[error("project format {found} is newer than this build ({supported})")]
    TooNew { found: u32, supported: u32 },
    #[error("non-finite number at {path}")]
    NonFinite { path: String },
    /// Entries, decompressed bytes, group depth, points, zip-slip names.
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

/// zip: manifest.json {format, formatVersion, generator, project: ProjectM, paths: {pN: "M0 0L10 0…Z"}} +
/// assets/<blake3>.<ext> (original image bytes, never re-encoded). ProjectM is a separate serde model converted
/// with exhaustive struct literals both ways, so a new doc field fails to compile until it is saved.
pub fn save(project: &Project) -> Result<Vec<u8>, Error> {
    let _ = project;
    Err(Error::NotImplemented("openlaser_format::save"))
}

pub fn load(bytes: &[u8]) -> Result<Project, Error> {
    let _ = bytes;
    Err(Error::NotImplemented("openlaser_format::load"))
}

/// craft-atomic.
pub fn save_to(path: &Path, project: &Project) -> Result<(), Error> {
    let _ = (path, project);
    Err(Error::NotImplemented("openlaser_format::save_to"))
}

pub fn load_from(path: &Path) -> Result<Project, Error> {
    let _ = path;
    Err(Error::NotImplemented("openlaser_format::load_from"))
}
