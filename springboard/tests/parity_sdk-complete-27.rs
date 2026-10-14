//! Parity sdk-complete-27: headless tests of the Thread operational dataset that accessory commissioning hands to a device.
//! Only data is checked: parsing, record validation, the attach checklist and the hex form. No radio, no network.

use springboard::net::thread_dataset::{tlv, Dataset, DatasetError};

/// One record as the wire form has it: type, one-byte length, value.
fn record(kind: u8, value: &[u8]) -> Vec<u8> {
    let mut out = vec![kind, value.len() as u8];
    out.extend_from_slice(value);
    out
}

/// A dataset that carries every record an attach needs, as hex text.
fn complete_hex() -> String {
    // Records in ascending type order, so the hex form written back is the same text.
    let mut bytes = Vec::new();
    bytes.extend(record(tlv::CHANNEL, &[0, 0, 15]));
    bytes.extend(record(tlv::PAN_ID, &[0xab, 0xcd]));
    bytes.extend(record(tlv::EXTENDED_PAN_ID, &[0x11; 8]));
    bytes.extend(record(tlv::NETWORK_NAME, b"hub-net"));
    bytes.extend(record(tlv::NETWORK_MASTER_KEY, &[0x22; 16]));
    bytes.extend(record(tlv::MESH_LOCAL_PREFIX, &[0xfd, 0, 0, 0, 0, 0, 0, 0]));
    bytes.extend(record(tlv::ACTIVE_TIMESTAMP, &[0, 0, 0, 0, 0, 1, 0, 0]));
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn a_complete_hex_dataset_reads_its_channel_pan_and_name() {
    let d = Dataset::from_hex(&complete_hex()).unwrap();
    assert_eq!(d.channel(), Some(15));
    assert_eq!(d.pan_id(), Some(0xabcd));
    assert_eq!(d.network_name(), Some("hub-net"));
    assert!(d.is_complete());
}

#[test]
fn a_dataset_read_from_hex_round_trips_to_the_same_hex() {
    let text = complete_hex();
    let d = Dataset::from_hex(&text).unwrap();
    assert_eq!(d.to_hex(), text, "records come out in ascending type order, which the complete dataset already uses");
}

#[test]
fn a_dataset_missing_the_master_key_and_timestamp_cannot_attach() {
    let mut bytes = record(tlv::CHANNEL, &[0, 0, 20]);
    bytes.extend(record(tlv::PAN_ID, &[0, 9]));
    bytes.extend(record(tlv::EXTENDED_PAN_ID, &[0x33; 8]));
    bytes.extend(record(tlv::NETWORK_NAME, b"x"));
    bytes.extend(record(tlv::MESH_LOCAL_PREFIX, &[0xfd; 8]));
    let d = Dataset::from_bytes(&bytes).unwrap();
    assert!(!d.is_complete());
    assert_eq!(d.missing_for_attach(), vec![tlv::ACTIVE_TIMESTAMP, tlv::NETWORK_MASTER_KEY]);
}

#[test]
fn a_channel_below_eleven_is_refused_while_the_band_edges_are_accepted() {
    assert_eq!(Dataset::from_bytes(&record(tlv::CHANNEL, &[0, 0, 10])), Err(DatasetError::BadValue { kind: tlv::CHANNEL }));
    assert!(Dataset::from_bytes(&record(tlv::CHANNEL, &[0, 0, 11])).is_ok());
    assert!(Dataset::from_bytes(&record(tlv::CHANNEL, &[0, 0, 26])).is_ok());
    assert_eq!(Dataset::from_bytes(&record(tlv::CHANNEL, &[0, 0, 27])), Err(DatasetError::BadValue { kind: tlv::CHANNEL }));
}

#[test]
fn a_network_name_of_sixteen_bytes_is_the_longest_allowed() {
    let sixteen = vec![b'n'; 16];
    let d = Dataset::from_bytes(&record(tlv::NETWORK_NAME, &sixteen)).unwrap();
    assert_eq!(d.network_name().map(str::len), Some(16));
    assert_eq!(
        Dataset::from_bytes(&record(tlv::NETWORK_NAME, &vec![b'n'; 17])),
        Err(DatasetError::BadLength { kind: tlv::NETWORK_NAME, len: 17 })
    );
}

#[test]
fn a_record_type_listed_twice_in_the_hex_is_refused() {
    let mut bytes = record(tlv::PAN_ID, &[0, 1]);
    bytes.extend(record(tlv::PAN_ID, &[0, 2]));
    assert_eq!(Dataset::from_bytes(&bytes), Err(DatasetError::Duplicate(tlv::PAN_ID)));
}

#[test]
fn hex_with_a_stray_letter_or_an_odd_digit_count_is_refused() {
    let text = complete_hex();
    let broken = format!("{text}z0");
    assert_eq!(Dataset::from_hex(&broken), Err(DatasetError::InvalidHex));
    assert_eq!(Dataset::from_hex(&text[1..]), Err(DatasetError::OddLength));
}

#[test]
fn a_mesh_local_prefix_of_the_wrong_length_names_the_record() {
    assert_eq!(
        Dataset::from_bytes(&record(tlv::MESH_LOCAL_PREFIX, &[0xfd; 7])),
        Err(DatasetError::BadLength { kind: tlv::MESH_LOCAL_PREFIX, len: 7 })
    );
}

#[test]
fn an_empty_hex_text_is_an_empty_dataset_that_cannot_attach() {
    let d = Dataset::from_hex("   ").unwrap();
    assert_eq!(d, Dataset::new());
    assert_eq!(d.to_hex(), "");
    assert_eq!(d.missing_for_attach().len(), 7);
}
