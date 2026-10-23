//! The control token: 32 random bytes, 64 lowercase hex on the wire, constant-time compare.
//!
//! The implementation follows PhotoCraft's `crates/automation/src/security.rs` at
//! c699e3cda69f4852b43c0b191ab0091aaeff9d6a (MIT OR Apache-2.0), adapted to the
//! typed OpenLaser control API.

use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use crate::Error;

/// 64 lowercase hex on the wire; constant-time compare. Debug never shows the secret.
pub struct Token([u8; 32]);

impl Token {
    pub fn generate() -> Result<Self, Error> {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(|error| Error::BadToken(format!("cannot generate control token: {error}")))?;
        Ok(Self(bytes))
    }

    pub fn from_hex(s: &str) -> Result<Self, Error> {
        if s.len() != 64 || !s.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
            return Err(Error::BadToken("control token must contain exactly 64 lowercase hexadecimal characters".to_owned()));
        }
        let mut bytes = [0_u8; 32];
        for (index, pair) in s.as_bytes().chunks_exact(2).enumerate() {
            let Some(high) = pair.first().and_then(|byte| hex_nibble(*byte)) else {
                return Err(Error::BadToken("control token contains a non-hexadecimal character".to_owned()));
            };
            let Some(low) = pair.get(1).and_then(|byte| hex_nibble(*byte)) else {
                return Err(Error::BadToken("control token contains a non-hexadecimal character".to_owned()));
            };
            let Some(slot) = bytes.get_mut(index) else {
                return Err(Error::BadToken("control token has an invalid length".to_owned()));
            };
            *slot = (high << 4) | low;
        }
        Ok(Self(bytes))
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn matches(&self, other: &str) -> bool {
        let expected = self.to_hex();
        if other.len() != expected.len() {
            return false;
        }
        expected.bytes().zip(other.bytes()).fold(0_u8, |difference, (a, b)| difference | (a ^ b)) == 0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// create_new, mode 0600 (Windows: the per-user AppData folder, §2.6).
pub fn create_or_reuse_file(path: &Path) -> Result<Token, Error> {
    match read_file(path) {
        Ok(token) => return Ok(token),
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
        set_private_dir(parent)?;
    }
    let token = Token::generate()?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(token.to_hex().as_bytes())?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            Ok(token)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read_file(path),
        Err(error) => Err(Error::Io(error)),
    }
}

pub fn read_file(path: &Path) -> Result<Token, Error> {
    let text = std::fs::read_to_string(path)?;
    Token::from_hex(text.trim())
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn set_private_dir(path: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_round_trip_and_compare() {
        let token = Token::generate().unwrap();
        let hex = token.to_hex();
        assert_eq!(hex.len(), 64);
        assert!(token.matches(&hex));
        assert!(!token.matches(&format!("0{}", &hex[1..])) || hex.starts_with('0'));
        assert_eq!(Token::from_hex(&hex).unwrap().to_hex(), hex);
        assert!(Token::from_hex(&hex.to_ascii_uppercase()).is_err());
    }

    #[test]
    fn token_file_is_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        let first = create_or_reuse_file(&path).unwrap().to_hex();
        let second = create_or_reuse_file(&path).unwrap().to_hex();
        assert_eq!(first, second);
    }

    #[cfg(unix)]
    #[test]
    fn token_file_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        create_or_reuse_file(&path).unwrap();
        assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
