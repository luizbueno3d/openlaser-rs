//! The bounded JSON-lines envelope.

use serde::{Deserialize, Serialize};

use crate::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: serde_json::Value,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub id: serde_json::Value,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Reads a whole line before returning any byte; a line over `max` is an error (the caller closes or skips).
pub fn read_line_bounded(r: &mut impl std::io::BufRead, max: usize) -> Result<Option<Vec<u8>>, Error> {
    let mut line = Vec::new();
    let mut too_long = false;
    loop {
        let available = r.fill_buf()?;
        if available.is_empty() {
            if too_long {
                return Err(Error::LineTooLong { max });
            }
            return if line.is_empty() { Ok(None) } else { Ok(Some(line)) };
        }
        let take = available.iter().position(|byte| *byte == b'\n').map_or(available.len(), |index| index.saturating_add(1));
        if !too_long {
            let remaining = max.saturating_sub(line.len());
            if take > remaining {
                too_long = true;
            } else {
                line.extend_from_slice(available.get(..take).unwrap_or(available));
            }
        }
        let found_newline = available.get(take.saturating_sub(1)).is_some_and(|byte| *byte == b'\n');
        r.consume(take);
        if found_newline {
            return if too_long { Err(Error::LineTooLong { max }) } else { Ok(Some(line)) };
        }
    }
}

/// Oversized replies are replaced by a small error reply with the same id ("…operation may have completed").
pub fn write_reply(w: &mut impl std::io::Write, reply: &Reply) -> std::io::Result<()> {
    let mut encoded = serde_json::to_vec(reply).map_err(std::io::Error::other)?;
    encoded.push(b'\n');
    if encoded.len() > crate::budgets::MAX_RESPONSE_BYTES {
        let replacement = Reply {
            id: reply.id.clone(),
            ok: false,
            result: None,
            error: Some("response exceeds 8388608 bytes; operation may have completed; inspect state before retrying".to_owned()),
        };
        encoded = serde_json::to_vec(&replacement).map_err(std::io::Error::other)?;
        encoded.push(b'\n');
    }
    w.write_all(&encoded)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn bounded_reader_consumes_an_oversized_line() {
        let mut input = Cursor::new(b"123456\nok\n".to_vec());
        assert!(matches!(read_line_bounded(&mut input, 4), Err(Error::LineTooLong { max: 4 })));
        assert_eq!(read_line_bounded(&mut input, 4).unwrap(), Some(b"ok\n".to_vec()));
    }

    #[test]
    fn bounded_reader_accepts_eof_without_newline() {
        let mut input = Cursor::new(b"ok".to_vec());
        assert_eq!(read_line_bounded(&mut input, 4).unwrap(), Some(b"ok".to_vec()));
        assert_eq!(read_line_bounded(&mut input, 4).unwrap(), None);
    }
}
