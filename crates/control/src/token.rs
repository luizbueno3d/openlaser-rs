//! The control token: 32 random bytes, 64 lowercase hex on the wire, constant-time compare.
//!
//! The implementation follows PhotoCraft's `crates/automation/src/security.rs` at
//! c699e3cda69f4852b43c0b191ab0091aaeff9d6a (MIT OR Apache-2.0), adapted to the
//! typed OpenLaser control API.

use std::fmt;
use std::io::Write;
use std::path::Path;

use crate::{Error, budgets, fsutil};

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
        for (index, pair) in s.as_bytes().as_chunks::<2>().0.iter().enumerate() {
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

/// `create_new`, mode 0600 (Windows: the per-user AppData folder plus the hidden attribute, §2.6). Folders it has
/// to create are made `0700`; an existing folder is never touched. A reused file must be private (unix: no group or
/// other bits), a regular file and short, or it is refused with the fix in the message.
pub fn create_or_reuse_file(path: &Path) -> Result<Token, Error> {
    match read_file(path) {
        Ok(token) => return Ok(token),
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fsutil::create_private_dirs(parent)?;
    }
    let token = Token::generate()?;
    match fsutil::private_new_file_options().open(path) {
        Ok(mut file) => {
            file.write_all(token.to_hex().as_bytes())?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            Ok(token)
        }
        // Lost a creation race: the winner's file goes through the same checks.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read_file(path),
        Err(error) => Err(Error::Io(error)),
    }
}

/// Reads a token file: a private regular file of at most `TOKEN_FILE_MAX_BYTES` holding 64 lowercase hex digits.
pub fn read_file(path: &Path) -> Result<Token, Error> {
    let bytes = fsutil::read_private_file(path, budgets::TOKEN_FILE_MAX_BYTES, "control token file")?;
    let text = std::str::from_utf8(&bytes).map_err(|_| Error::BadToken(format!("control token file `{}` is not text", path.display())))?;
    Token::from_hex(text.trim())
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
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
        let path = dir.path().join("new").join("token");
        create_or_reuse_file(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn reused_token_file_readable_by_others_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        create_or_reuse_file(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let refused = create_or_reuse_file(&path);
        assert!(matches!(&refused, Err(Error::Insecure(message)) if message.contains("chmod 600")), "{refused:?}");
        assert!(read_file(&path).is_err());
    }

    #[test]
    fn oversized_token_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        let mut file = fsutil::private_new_file_options().open(&path).unwrap();
        file.write_all(&[b'a'; 4096]).unwrap();
        drop(file);
        assert!(matches!(create_or_reuse_file(&path), Err(Error::Insecure(_))));
    }

    #[test]
    fn token_path_that_is_not_a_regular_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        // A folder stands in for any non-regular file (a FIFO would block a naive open).
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(read_file(&path), Err(Error::Insecure(_))));
    }

    #[cfg(windows)]
    #[test]
    fn token_file_in_user_appdata() {
        use std::os::windows::fs::MetadataExt;
        let appdata = std::path::PathBuf::from(std::env::var_os("APPDATA").expect("APPDATA is set on Windows"));
        let default = crate::discovery::default_path().unwrap();
        assert!(default.starts_with(&appdata), "{} is not under %APPDATA%", default.display());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        create_or_reuse_file(&path).unwrap();
        assert_ne!(std::fs::metadata(&path).unwrap().file_attributes() & 0x2, 0, "token file is not hidden");
    }
}
