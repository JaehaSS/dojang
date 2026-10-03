//! Bounded JSONL transport. Limits apply to a single wire event, not conversation history.
use std::io::{self, BufRead, Read};

// Allows image-bearing tool results without allowing an unbounded stdout allocation.
pub(super) const MAX_EVENT_BYTES: usize = 16 * 1024 * 1024;

fn oversized() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "이벤트 크기 제한 초과 (16 MiB)")
}

/// Nonblocking readers feed each byte once; do not rescan the growing partial frame.
#[derive(Default)]
pub(super) struct FrameBuffer {
    pending: Vec<u8>,
}

impl FrameBuffer {
    pub(super) fn push(
        &mut self,
        chunk: &[u8],
        mut emit: impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        for part in chunk.split_inclusive(|b| *b == b'\n') {
            let complete = part.last() == Some(&b'\n');
            let data = if complete {
                &part[..part.len() - 1]
            } else {
                part
            };
            if data.len() > MAX_EVENT_BYTES - self.pending.len() {
                return Err(oversized().to_string());
            }
            self.pending.extend_from_slice(data);
            if complete {
                emit(&self.pending)?;
                self.pending.clear();
            }
        }
        Ok(())
    }
}

/// Read at most one event plus its delimiter/overflow byte, including at EOF.
pub(super) fn read_line(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut bytes = Vec::new();
    let n = reader
        .take((MAX_EVENT_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    if n == 0 {
        return Ok(None);
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(oversized());
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn large_fragmented_image_and_following_event_are_preserved() {
        let payload = format!("{{\"image\":\"{}한글\"}}", "a".repeat(5 * 1024 * 1024));
        let wire = format!("{payload}\n{{\"done\":true}}\n");
        let mut frames = FrameBuffer::default();
        let mut got = Vec::new();
        for chunk in wire.as_bytes().chunks(8191) {
            frames
                .push(chunk, |line| {
                    got.push(String::from_utf8(line.to_vec()).unwrap());
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(got, [payload.clone(), "{\"done\":true}".to_string()]);
        let mut reader = Cursor::new(wire);
        assert_eq!(read_line(&mut reader).unwrap(), Some(payload));
        assert_eq!(
            read_line(&mut reader).unwrap().as_deref(),
            Some("{\"done\":true}")
        );
        assert_eq!(read_line(&mut reader).unwrap(), None);
    }

    #[test]
    fn exact_limit_and_overflow_with_or_without_newline() {
        for newline in [false, true] {
            for overflow in [false, true] {
                let mut wire = vec![b'a'; MAX_EVENT_BYTES + usize::from(overflow)];
                if newline {
                    wire.push(b'\n');
                }
                let result = read_line(&mut Cursor::new(&wire));
                assert_eq!(result.is_err(), overflow);
                let mut frames = FrameBuffer::default();
                let result = frames.push(&wire, |_| Ok(()));
                assert_eq!(result.is_err(), overflow);
                assert!(frames.pending.len() <= MAX_EVENT_BYTES);
            }
        }
    }

    #[test]
    fn blocking_reader_preserves_eof_crlf_and_rejects_invalid_utf8() {
        let mut reader = Cursor::new(b"first\r\nlast");
        assert_eq!(read_line(&mut reader).unwrap().as_deref(), Some("first"));
        assert_eq!(read_line(&mut reader).unwrap().as_deref(), Some("last"));
        assert_eq!(read_line(&mut reader).unwrap(), None);
        assert!(read_line(&mut Cursor::new([0xff, b'\n'])).is_err());
    }

    #[test]
    fn callback_failure_stops_before_next_frame() {
        let mut count = 0;
        let result = FrameBuffer::default().push(b"one\ntwo\n", |_| {
            count += 1;
            Err("invalid JSON".into())
        });
        assert_eq!(result.unwrap_err(), "invalid JSON");
        assert_eq!(count, 1);
    }
}
