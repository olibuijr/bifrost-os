//! Parity sdk-complete-09: headless tests for BLE MIDI packets that carry system exclusive messages across packets, and
//! for the promotions a customer has not already bought. Pure codec and model tests; no radio is used.

use bifrost_ui::midi_ble::{decode, BleMessage, BleMidiError, SysExAssembler};
use bifrost_ui::promotion::{in_display_order, visible_in_display_order, Promotion};

fn promo(product: &str, order: u32) -> Promotion {
    Promotion { product: product.to_string(), order }
}

#[test]
fn a_system_exclusive_message_split_over_two_packets_is_joined_into_one_message() {
    let mut assembler = SysExAssembler::new();
    // Header 0x80 (timestamp 0), timestamp byte 0x80, then the start of the message, which does not end here.
    assert_eq!(assembler.push(&[0x80, 0x80, 0xF0, 1, 2]), Ok(vec![]));
    assert!(assembler.is_open());
    // The continuation packet has a header, a timestamp byte, then the rest of the data and the end byte.
    let joined = assembler.push(&[0x80, 0x81, 3, 0xF7]).unwrap();
    assert_eq!(joined, vec![BleMessage { timestamp: 0, bytes: vec![0xF0, 1, 2, 3, 0xF7] }]);
    assert!(!assembler.is_open());
}

#[test]
fn the_joined_message_keeps_the_timestamp_where_it_started() {
    let mut assembler = SysExAssembler::new();
    // Header 0x81 gives timestamp 128, and the message starts under the timestamp byte 0x80.
    assert_eq!(assembler.push(&[0x81, 0x80, 0xF0, 1]), Ok(vec![]));
    let joined = assembler.push(&[0x80, 0x80, 2, 0xF7]).unwrap();
    assert_eq!(joined, vec![BleMessage { timestamp: 128, bytes: vec![0xF0, 1, 2, 0xF7] }]);
}

#[test]
fn messages_before_an_open_system_exclusive_message_are_returned_at_once() {
    let mut assembler = SysExAssembler::new();
    // Header 0x81 gives timestamp 128. A note on comes first, then a system exclusive message that is still open.
    let first = assembler.push(&[0x81, 0x80, 0x90, 60, 100, 0x80, 0xF0, 1]).unwrap();
    assert_eq!(first, vec![BleMessage { timestamp: 128, bytes: vec![0x90, 60, 100] }]);
    assert!(assembler.is_open());
}

#[test]
fn messages_after_the_end_of_a_joined_system_exclusive_message_are_returned_in_the_same_call() {
    let mut assembler = SysExAssembler::new();
    assembler.push(&[0x81, 0x80, 0xF0, 1]).unwrap();
    // The continuation ends the message, then a note on follows with its own timestamp byte.
    let messages = assembler.push(&[0x80, 0x80, 2, 0xF7, 0x80, 0x90, 60, 0]).unwrap();
    assert_eq!(
        messages,
        vec![
            BleMessage { timestamp: 128, bytes: vec![0xF0, 1, 2, 0xF7] },
            BleMessage { timestamp: 0, bytes: vec![0x90, 60, 0] },
        ]
    );
    assert!(!assembler.is_open());
}

#[test]
fn a_packet_without_an_open_message_decodes_as_a_plain_packet() {
    let mut assembler = SysExAssembler::new();
    let messages = assembler.push(&[0x81, 0x80, 0xB0, 7, 100]).unwrap();
    assert_eq!(messages, vec![BleMessage { timestamp: 128, bytes: vec![0xB0, 7, 100] }]);
    assert!(!assembler.is_open());
}

#[test]
fn decode_still_reports_a_packet_that_ends_inside_a_system_exclusive_message() {
    assert_eq!(decode(&[0x80, 0x80, 0xF0, 1]), Err(BleMidiError::Truncated));
}

#[test]
fn a_continuation_without_a_packet_header_is_refused_and_keeps_the_open_message() {
    let mut assembler = SysExAssembler::new();
    assembler.push(&[0x80, 0x80, 0xF0, 1]).unwrap();
    assert_eq!(assembler.push(&[0x01, 2, 0xF7]), Err(BleMidiError::NotAPacket));
    assert!(assembler.is_open(), "a refused packet does not drop the message it would have continued");
    assert_eq!(assembler.push(&[0x80, 0x80, 2, 0xF7]).unwrap()[0].bytes, vec![0xF0, 1, 2, 0xF7]);
}

#[test]
fn an_empty_packet_is_refused_by_the_assembler() {
    assert_eq!(SysExAssembler::new().push(&[]), Err(BleMidiError::Truncated));
}

#[test]
fn a_promotion_for_an_owned_product_is_not_shown() {
    let list = [promo("b", 2), promo("a", 1), promo("c", 3)];
    assert_eq!(visible_in_display_order(&list, &["a"]), vec!["b", "c"]);
    assert_eq!(visible_in_display_order(&list, &[]), in_display_order(&list));
    assert!(visible_in_display_order(&list, &["a", "b", "c"]).is_empty());
}
