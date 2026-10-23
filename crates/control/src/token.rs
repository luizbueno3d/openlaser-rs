//! The control token: 32 random bytes, 64 lowercase hex on the wire, constant-time compare. WP0 stubs.

use std::fmt;
use std::path::Path;

use crate::Error;

/// 64 lowercase hex on the wire; constant-time compare. Debug never shows the secret.
pub struct Token([u8; 32]);

impl Token {
    pub fn generate() -> Result<Self, Error> {
        Err(Error::NotImplemented("openlaser_control::token::Token::generate"))
    }

    pub fn from_hex(s: &str) -> Result<Self, Error> {
        let _ = s;
        Err(Error::NotImplemented("openlaser_control::token::Token::from_hex"))
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// WP0 stub: never matches (fail closed).
    pub fn matches(&self, other: &str) -> bool {
        let _ = other;
        false
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// create_new, mode 0600 (Windows: the per-user AppData folder, §2.6).
pub fn create_or_reuse_file(path: &Path) -> Result<Token, Error> {
    let _ = path;
    Err(Error::NotImplemented("openlaser_control::token::create_or_reuse_file"))
}

pub fn read_file(path: &Path) -> Result<Token, Error> {
    let _ = path;
    Err(Error::NotImplemented("openlaser_control::token::read_file"))
}
