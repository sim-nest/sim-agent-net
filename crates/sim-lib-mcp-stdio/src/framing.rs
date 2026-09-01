use std::{
    fmt,
    io::{BufRead, Write},
};

use serde_json::Value;

/// Deterministic rejection produced by [`JsonLineFramer`].
#[derive(Debug)]
pub enum FrameError {
    /// The input ended with a partial, unterminated protocol message.
    TrailingMaterial,
    /// A frame exceeded the configured byte limit.
    TooLarge {
        /// Configured maximum payload bytes.
        limit: usize,
    },
    /// A frame was not valid UTF-8.
    InvalidUtf8,
    /// A frame was not exactly one JSON value.
    InvalidJson(String),
    /// A JSON string contains a literal newline byte and therefore spans frames.
    EmbeddedNewline,
    /// The underlying stream failed.
    Io(std::io::Error),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TrailingMaterial => f.write_str("unterminated trailing protocol material"),
            Self::TooLarge { limit } => write!(f, "JSON line exceeds {limit} bytes"),
            Self::InvalidUtf8 => f.write_str("JSON line is not valid UTF-8"),
            Self::InvalidJson(error) => write!(f, "invalid JSON line: {error}"),
            Self::EmbeddedNewline => f.write_str("embedded newline in JSON message"),
            Self::Io(error) => write!(f, "stdio I/O failed: {error}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Strict one-JSON-value-per-newline framing with a fixed memory bound.
#[derive(Clone, Copy, Debug)]
pub struct JsonLineFramer {
    max_bytes: usize,
}

impl JsonLineFramer {
    /// Constructs a non-zero frame bound.
    pub fn new(max_bytes: usize) -> Result<Self, FrameError> {
        if max_bytes == 0 {
            return Err(FrameError::TooLarge { limit: 0 });
        }
        Ok(Self { max_bytes })
    }
    /// Maximum payload bytes excluding the newline terminator.
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }
    /// Reads one terminated JSON message. A final partial line is rejected.
    pub fn read(&self, reader: &mut impl BufRead) -> Result<Option<Value>, FrameError> {
        let mut bytes = Vec::with_capacity(self.max_bytes.min(4096) + 1);
        loop {
            let available = reader.fill_buf().map_err(FrameError::Io)?;
            if available.is_empty() {
                break;
            }
            let consumed = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |index| index + 1);
            let remaining = self.max_bytes.saturating_add(2).saturating_sub(bytes.len());
            let copied = consumed.min(remaining);
            bytes.extend_from_slice(&available[..copied]);
            let terminated = available[..copied].last() == Some(&b'\n');
            reader.consume(consumed);
            if consumed > copied {
                return Err(FrameError::TooLarge {
                    limit: self.max_bytes,
                });
            }
            if terminated {
                break;
            }
        }
        if bytes.is_empty() {
            return Ok(None);
        }
        if bytes.last() != Some(&b'\n') {
            return if bytes.len() > self.max_bytes {
                Err(FrameError::TooLarge {
                    limit: self.max_bytes,
                })
            } else {
                Err(FrameError::TrailingMaterial)
            };
        }
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        if bytes.len() > self.max_bytes {
            return Err(FrameError::TooLarge {
                limit: self.max_bytes,
            });
        }
        if bytes.contains(&b'\n') || bytes.contains(&b'\r') {
            return Err(FrameError::EmbeddedNewline);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| FrameError::InvalidUtf8)?;
        let mut stream = serde_json::Deserializer::from_str(text).into_iter::<Value>();
        let value = stream
            .next()
            .ok_or_else(|| FrameError::InvalidJson("empty frame".into()))?
            .map_err(|error| FrameError::InvalidJson(error.to_string()))?;
        if let Some(extra) = stream.next() {
            extra.map_err(|error| FrameError::InvalidJson(error.to_string()))?;
            return Err(FrameError::InvalidJson("multiple JSON values".into()));
        }
        Ok(Some(value))
    }
    /// Writes exactly one compact JSON message and one newline.
    pub fn write(&self, writer: &mut impl Write, value: &Value) -> Result<(), FrameError> {
        let bytes = serde_json::to_vec(value)
            .map_err(|error| FrameError::InvalidJson(error.to_string()))?;
        if bytes.len() > self.max_bytes {
            return Err(FrameError::TooLarge {
                limit: self.max_bytes,
            });
        }
        if bytes.contains(&b'\n') || bytes.contains(&b'\r') {
            return Err(FrameError::EmbeddedNewline);
        }
        writer
            .write_all(&bytes)
            .and_then(|_| writer.write_all(b"\n"))
            .map_err(FrameError::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn admits_exactly_one_bounded_terminated_json_value() {
        let framer = JsonLineFramer::new(32).unwrap();
        let mut input = Cursor::new(b"{\"ok\":true}\n".to_vec());
        assert_eq!(
            framer.read(&mut input).unwrap(),
            Some(serde_json::json!({"ok": true}))
        );
        assert_eq!(framer.read(&mut input).unwrap(), None);
        let mut output = Vec::new();
        framer
            .write(&mut output, &serde_json::json!({"ok": true}))
            .unwrap();
        assert_eq!(output, b"{\"ok\":true}\n");
    }

    #[test]
    fn rejects_invalid_utf8_json_oversize_and_trailing_material() {
        let framer = JsonLineFramer::new(8).unwrap();
        assert!(matches!(
            framer.read(&mut Cursor::new(vec![0xff, b'\n'])),
            Err(FrameError::InvalidUtf8)
        ));
        assert!(matches!(
            framer.read(&mut Cursor::new(b"oops\n")),
            Err(FrameError::InvalidJson(_))
        ));
        assert!(matches!(
            framer.read(&mut Cursor::new(b"123456789\n")),
            Err(FrameError::TooLarge { .. })
        ));
        assert!(matches!(
            framer.read(&mut Cursor::new(b"{}")),
            Err(FrameError::TrailingMaterial)
        ));
    }
}
