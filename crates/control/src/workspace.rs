//! cap-std directory capabilities; separate read and write roots; missing root = denied (fail closed).
//! Path grammar: non-empty, '/'-separated, relative; no '\', ':', leading '/', empty, '.', '..' component,
//! no component ending in '.' or space; an absolute path is accepted only beneath the root and re-checked.
//! WP0 stubs: everything is denied.

use std::path::Path;

use cap_std::fs::Dir;

use crate::Error;

pub struct AuthorizedWorkspace {
    read: Option<Dir>,
    write: Option<Dir>,
}

impl AuthorizedWorkspace {
    pub fn denied() -> Self {
        AuthorizedWorkspace { read: None, write: None }
    }

    pub fn open(read: Option<&Path>, write: Option<&Path>) -> Result<Self, Error> {
        let _ = (read, write);
        Err(Error::NotImplemented("openlaser_control::workspace::AuthorizedWorkspace::open"))
    }

    pub fn read(&self, path: &str) -> Result<Vec<u8>, Error> {
        let _ = (path, &self.read);
        Err(Error::NotImplemented("openlaser_control::workspace::AuthorizedWorkspace::read"))
    }

    pub fn write_atomic(&self, path: &str, bytes: &[u8]) -> Result<(), Error> {
        let _ = (path, bytes, &self.write);
        Err(Error::NotImplemented("openlaser_control::workspace::AuthorizedWorkspace::write_atomic"))
    }
}
