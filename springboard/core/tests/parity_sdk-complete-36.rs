//! Parity sdk-complete-36: headless tests for the force-feedback escape frame, including reading several frames
//! from one reply buffer. No device node is opened and no frame is sent.

use bifrost_core::ff_escape::{Escape, EscapeError, HEADER_LEN, MAX_PAYLOAD};

#[test]
fn a_buffer_holding_one_frame_yields_it_and_its_length() {
    let escape = Escape::new(0x10, vec![5, 6, 7]).unwrap();
    let frame = escape.encode();
    assert_eq!(Escape::split(&frame), Ok((escape, frame.len())));
}

#[test]
fn consecutive_frames_in_one_buffer_are_read_one_after_another() {
    let first = Escape::new(1, vec![0xaa]).unwrap();
    let second = Escape::new(2, vec![0xbb, 0xcc]).unwrap();
    let mut buffer = first.encode();
    let first_len = buffer.len();
    buffer.extend_from_slice(&second.encode());

    let (read_first, used) = Escape::split(&buffer).unwrap();
    assert_eq!(read_first, first);
    assert_eq!(used, first_len);

    let (read_second, rest) = Escape::split(&buffer[used..]).unwrap();
    assert_eq!(read_second, second);
    assert_eq!(rest, buffer.len() - used);
}

#[test]
fn a_buffer_shorter_than_the_header_is_truncated() {
    assert_eq!(Escape::split(&[1, 2, 3]), Err(EscapeError::Truncated));
}

#[test]
fn a_frame_whose_payload_has_not_fully_arrived_is_truncated() {
    let frame = Escape::new(4, vec![1, 2, 3, 4]).unwrap().encode();
    assert_eq!(Escape::split(&frame[..frame.len() - 1]), Err(EscapeError::Truncated));
}

#[test]
fn a_header_declaring_too_long_a_payload_is_refused_when_splitting() {
    let mut buffer = vec![0u8; HEADER_LEN];
    buffer[4..8].copy_from_slice(&((MAX_PAYLOAD as u32) + 1).to_le_bytes());
    assert_eq!(Escape::split(&buffer), Err(EscapeError::PayloadTooLarge(MAX_PAYLOAD + 1)));
}

#[test]
fn splitting_an_empty_payload_frame_uses_only_the_header() {
    let escape = Escape::new(9, Vec::new()).unwrap();
    let frame = escape.encode();
    assert_eq!(Escape::split(&frame), Ok((escape, HEADER_LEN)));
}

#[test]
fn decode_still_refuses_bytes_after_the_frame_that_split_would_skip() {
    let mut frame = Escape::new(1, vec![1]).unwrap().encode();
    frame.push(0);
    assert_eq!(Escape::decode(&frame), Err(EscapeError::TrailingBytes));
    assert_eq!(Escape::split(&frame).map(|(_, used)| used), Ok(frame.len() - 1));
}
