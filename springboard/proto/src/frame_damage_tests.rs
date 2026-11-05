use super::*;

#[test]
fn a_frame_with_more_damage_than_a_packet_holds_is_clamped_and_still_decodes() {
    let rects: Vec<Rect> = (0..5000u32).map(|i| [i, i, 1, 1]).collect();
    let bytes = ToShell::Frame { buf: 1, damage: rects.clone() }.encode();
    assert!(bytes.len() <= MAX_PACKET, "the encoder never builds a packet the decoder refuses");
    let ToShell::Frame { buf, damage } = ToShell::decode(&bytes).unwrap() else { panic!("not a frame") };
    assert_eq!(buf, 1);
    assert_eq!(damage.len(), (MAX_PACKET - 16) / 16, "the clamp keeps the first rectangles");
    assert_eq!(damage[..], rects[..damage.len()]);
}

#[test]
fn a_frame_claiming_more_rectangles_than_a_packet_can_hold_is_refused() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&tag::FRAME.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&((MAX_PACKET / 16 + 1) as u32).to_le_bytes());
    bytes.resize(bytes.len() + (MAX_PACKET / 16 + 1) * 16, 0);
    assert_eq!(ToShell::decode(&bytes), Err(DecodeError::BadValue));
}

#[test]
fn an_empty_frame_is_a_small_packet() {
    let bytes = ToShell::Frame { buf: 0, damage: Vec::new() }.encode();
    assert_eq!(bytes.len(), 12, "tag, buffer and a zero count");
    assert_eq!(ToShell::decode(&bytes), Ok(ToShell::Frame { buf: 0, damage: Vec::new() }));
}
