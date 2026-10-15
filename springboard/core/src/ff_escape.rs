//! Hardware escape commands for a force-feedback device: a driver-specific command code with a byte payload that an app
//! sends past the generic effect model, for features such as a wheel's vendor-only tuning registers.
//!
//! The module is pure. It validates an escape, and it frames one into a byte string and reads it back, so a caller
//! that talks to a device node can write the frame to its driver and parse the reply. The frame layout is BifrOSt's
//! own and is not a kernel format:
//!
//! ```text
//! bytes 0..4   command code, little endian
//! bytes 4..8   payload length, little endian
//! bytes 8..    payload
//! ```
//!
//! Nothing here sends a frame to a node. A driver that has no escape path rejects the frame at the caller, so the
//! model only describes what would be sent.

/// The largest payload one escape carries, in bytes.
pub const MAX_PAYLOAD: usize = 4096;

/// The size of the frame header: the command code and the payload length.
pub const HEADER_LEN: usize = 8;

/// Why an escape is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EscapeError {
    /// the payload is longer than [`MAX_PAYLOAD`]; the value is the length that was given
    PayloadTooLarge(usize),
    /// the frame is shorter than its header, or shorter than the length the header declares
    Truncated,
    /// the frame carries bytes after the payload the header declares
    TrailingBytes,
}

/// One escape command: a driver command code and the bytes that go with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Escape {
    pub code: u32,
    pub payload: Vec<u8>,
}

impl Escape {
    /// Makes an escape, refusing a payload longer than [`MAX_PAYLOAD`].
    pub fn new(code: u32, payload: Vec<u8>) -> Result<Self, EscapeError> {
        if payload.len() > MAX_PAYLOAD {
            return Err(EscapeError::PayloadTooLarge(payload.len()));
        }
        Ok(Escape { code, payload })
    }

    /// The frame for this escape: the header, then the payload.
    pub fn encode(&self) -> Vec<u8> {
        let mut frame = Vec::with_capacity(HEADER_LEN + self.payload.len());
        frame.extend_from_slice(&self.code.to_le_bytes());
        frame.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(&self.payload);
        frame
    }

    /// Reads a frame made by [`Escape::encode`]. The frame must hold exactly the payload its header declares.
    pub fn decode(frame: &[u8]) -> Result<Self, EscapeError> {
        let (escape, used) = Escape::split(frame)?;
        if used != frame.len() {
            return Err(EscapeError::TrailingBytes);
        }
        Ok(escape)
    }

    /// Reads the first frame at the start of `buffer`, for a reply that carries several frames back to back. Returns
    /// the escape and the number of bytes the frame used, so the caller can read the next one from the rest.
    pub fn split(buffer: &[u8]) -> Result<(Self, usize), EscapeError> {
        if buffer.len() < HEADER_LEN {
            return Err(EscapeError::Truncated);
        }
        let code = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
        let len = u32::from_le_bytes([buffer[4], buffer[5], buffer[6], buffer[7]]) as usize;
        if len > MAX_PAYLOAD {
            return Err(EscapeError::PayloadTooLarge(len));
        }
        let end = HEADER_LEN + len;
        if buffer.len() < end {
            return Err(EscapeError::Truncated);
        }
        let escape = Escape::new(code, buffer[HEADER_LEN..end].to_vec())?;
        Ok((escape, end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_at_the_limit_is_accepted_and_one_over_it_is_refused() {
        assert!(Escape::new(1, vec![0; MAX_PAYLOAD]).is_ok());
        assert_eq!(
            Escape::new(1, vec![0; MAX_PAYLOAD + 1]),
            Err(EscapeError::PayloadTooLarge(MAX_PAYLOAD + 1))
        );
    }

    #[test]
    fn a_frame_starts_with_the_code_then_the_length_in_little_endian() {
        let frame = Escape::new(0x0102_0304, vec![0xaa, 0xbb]).unwrap().encode();
        assert_eq!(&frame[..4], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&frame[4..8], &[2, 0, 0, 0]);
        assert_eq!(&frame[8..], &[0xaa, 0xbb]);
    }

    #[test]
    fn encode_then_decode_gives_the_same_escape() {
        let escape = Escape::new(7, vec![1, 2, 3, 4, 5]).unwrap();
        assert_eq!(Escape::decode(&escape.encode()), Ok(escape));
    }

    #[test]
    fn an_empty_payload_round_trips() {
        let escape = Escape::new(9, Vec::new()).unwrap();
        let frame = escape.encode();
        assert_eq!(frame.len(), HEADER_LEN);
        assert_eq!(Escape::decode(&frame), Ok(escape));
    }

    #[test]
    fn a_short_frame_or_a_short_payload_is_truncated() {
        assert_eq!(Escape::decode(&[0; 7]), Err(EscapeError::Truncated));
        let mut frame = Escape::new(1, vec![1, 2, 3]).unwrap().encode();
        frame.pop();
        assert_eq!(Escape::decode(&frame), Err(EscapeError::Truncated));
    }

    #[test]
    fn bytes_after_the_declared_payload_are_refused() {
        let mut frame = Escape::new(1, vec![1]).unwrap().encode();
        frame.push(0);
        assert_eq!(Escape::decode(&frame), Err(EscapeError::TrailingBytes));
    }

    #[test]
    fn a_header_that_declares_too_long_a_payload_is_refused_before_reading_it() {
        let mut frame = vec![0u8; HEADER_LEN];
        frame[4..8].copy_from_slice(&((MAX_PAYLOAD as u32) + 1).to_le_bytes());
        assert_eq!(Escape::decode(&frame), Err(EscapeError::PayloadTooLarge(MAX_PAYLOAD + 1)));
    }
}
