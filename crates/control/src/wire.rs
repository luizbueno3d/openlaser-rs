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
///
/// It stops reading as soon as the line is known to be longer than `max` (after at most `max + 1` bytes of it
/// plus one buffer fill), so an endless line can neither grow memory nor keep the caller reading. On
/// `LineTooLong` the reader is left inside that line: close the stream, or call [`discard_line`] to skip it.
pub fn read_line_bounded(r: &mut impl std::io::BufRead, max: usize) -> Result<Option<Vec<u8>>, Error> {
    let mut line = Vec::new();
    loop {
        let available = r.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() { Ok(None) } else { Ok(Some(line)) };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(available.len(), |index| index.saturating_add(1));
        if take > max.saturating_sub(line.len()) {
            return Err(Error::LineTooLong { max });
        }
        line.extend_from_slice(available.get(..take).unwrap_or(available));
        r.consume(take);
        if newline.is_some() {
            return Ok(Some(line));
        }
    }
}

/// Skips the rest of the current line (after a `LineTooLong`), in constant memory. Returns false at end of input.
pub fn discard_line(r: &mut impl std::io::BufRead) -> Result<bool, Error> {
    loop {
        let available = r.fill_buf()?;
        if available.is_empty() {
            return Ok(false);
        }
        if let Some(index) = available.iter().position(|byte| *byte == b'\n') {
            r.consume(index.saturating_add(1));
            return Ok(true);
        }
        let all = available.len();
        r.consume(all);
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
    fn oversized_line_can_be_skipped() {
        let mut input = Cursor::new(b"123456\nok\n".to_vec());
        assert!(matches!(read_line_bounded(&mut input, 4), Err(Error::LineTooLong { max: 4 })));
        assert!(discard_line(&mut input).unwrap());
        assert_eq!(read_line_bounded(&mut input, 4).unwrap(), Some(b"ok\n".to_vec()));
    }

    #[test]
    fn exact_limit_is_accepted() {
        let mut input = Cursor::new(b"123\n1234\n".to_vec());
        assert_eq!(read_line_bounded(&mut input, 4).unwrap(), Some(b"123\n".to_vec()));
        assert!(read_line_bounded(&mut input, 4).is_err(), "4 digits + newline is 5 bytes");
    }

    /// A source that serves `limit` bytes of `a` with no newline and counts what was taken from it.
    struct Endless {
        served: usize,
        limit: usize,
    }

    impl std::io::Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(self.limit.saturating_sub(self.served));
            buf.iter_mut().take(n).for_each(|b| *b = b'a');
            self.served += n;
            Ok(n)
        }
    }

    #[test]
    fn endless_line_stops_reading_at_max_plus_one() {
        let max = 1000;
        let capacity = 64;
        let mut reader = std::io::BufReader::with_capacity(capacity, Endless { served: 0, limit: 64 << 20 });
        assert!(matches!(read_line_bounded(&mut reader, max), Err(Error::LineTooLong { .. })));
        let served = reader.get_ref().served;
        assert!(served <= max + 1 + capacity, "read {served} bytes of an endless line with max {max}");
    }

    #[test]
    fn bounded_reader_accepts_eof_without_newline() {
        let mut input = Cursor::new(b"ok".to_vec());
        assert_eq!(read_line_bounded(&mut input, 4).unwrap(), Some(b"ok".to_vec()));
        assert_eq!(read_line_bounded(&mut input, 4).unwrap(), None);
    }

    #[test]
    fn batch_reply_budget_escaped() {
        let escaped = "\\n".repeat(crate::budgets::MAX_RESPONSE_BYTES / 3);
        let reply = Reply { id: serde_json::json!(7), ok: true, result: Some(serde_json::json!({"steps":escaped})), error: None };
        let mut bytes = Vec::new();
        write_reply(&mut bytes, &reply).unwrap();
        assert!(bytes.len() < crate::budgets::MAX_RESPONSE_BYTES);
        let result: Reply = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(result.id, serde_json::json!(7));
        assert!(!result.ok);
        assert!(result.error.as_deref().unwrap_or_default().contains("operation may have completed"));
    }

    proptest::proptest! {
        #[test]
        fn prop_control_reader_never_panics(data in proptest::collection::vec(proptest::num::u8::ANY, 0..4096), max in 0usize..1024) {
            let mut input = Cursor::new(data);
            let _ = read_line_bounded(&mut input, max);
        }
    }
}
