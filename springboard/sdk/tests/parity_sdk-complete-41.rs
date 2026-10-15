//! Parity sdk-complete-41: headless tests for device inquiry reports, typed keychain keys and HTTP Digest answers.
//! No test opens a radio, a keychain service or a socket.

use bifrost_app::bluetooth_inquiry::{parse_device_found, Event, Inquiry, ParseError};
use bifrost_app::crypto_key::{Error as KeyError, SecretKey};
use bifrost_app::http_digest::{authorization, parse_digest_challenge, response_digest, DigestAnswer};
use bifrost_app::keychain::{self, Reply};
use bifrost_app::keychain_keys::{decode, decode_reply, encode, KeyKind, RecordError, StoreError};
use bifrost_core::bt_address::{BdAddr, ClassOfDevice};

/// Builds a `Device Found` payload. The address is given in printing order and sent least significant byte first.
fn device_found(printed: [u8; 6], rssi: u8, eir: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    for byte in printed.iter().rev() {
        payload.push(*byte);
    }
    payload.push(0); // address type
    payload.push(rssi);
    payload.extend_from_slice(&[0, 0, 0, 0]); // flags
    payload.extend_from_slice(&(eir.len() as u16).to_le_bytes());
    payload.extend_from_slice(eir);
    payload
}

fn field(kind: u8, value: &[u8]) -> Vec<u8> {
    let mut out = vec![value.len() as u8 + 1, kind];
    out.extend_from_slice(value);
    out
}

const PRINTED: [u8; 6] = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];

#[test]
fn a_device_found_payload_gives_its_address_in_printing_order() {
    let report = parse_device_found(&device_found(PRINTED, 0xC4, &[])).unwrap();
    assert_eq!(report.address, BdAddr(PRINTED));
    assert_eq!(report.address.to_string(), "AA:BB:CC:DD:EE:FF");
}

#[test]
fn the_rssi_is_read_as_a_signed_value_and_127_means_unknown() {
    assert_eq!(parse_device_found(&device_found(PRINTED, 0xC4, &[])).unwrap().rssi, Some(-60));
    assert_eq!(parse_device_found(&device_found(PRINTED, 127, &[])).unwrap().rssi, None);
}

#[test]
fn the_complete_local_name_wins_over_the_shortened_name() {
    let mut eir = field(0x08, b"Mou");
    eir.extend(field(0x09, b"Mouse\0\0"));
    let report = parse_device_found(&device_found(PRINTED, 0, &eir)).unwrap();
    assert_eq!(report.name.as_deref(), Some("Mouse"));
}

#[test]
fn a_shortened_name_is_used_when_no_complete_name_arrives() {
    let report = parse_device_found(&device_found(PRINTED, 0, &field(0x08, b"Spea"))).unwrap();
    assert_eq!(report.name.as_deref(), Some("Spea"));
}

#[test]
fn an_empty_name_counts_as_no_name() {
    let report = parse_device_found(&device_found(PRINTED, 0, &field(0x09, b"\0\0"))).unwrap();
    assert_eq!(report.name, None);
}

#[test]
fn the_class_of_device_is_read_from_three_little_endian_bytes() {
    let report = parse_device_found(&device_found(PRINTED, 0, &field(0x0D, &[0x80, 0x25, 0x00]))).unwrap();
    assert_eq!(report.class, ClassOfDevice::new(0x002580));
}

#[test]
fn a_zero_length_field_ends_the_extended_data() {
    let mut eir = field(0x09, b"Pad");
    eir.push(0);
    eir.extend(field(0x09, b"Ignored"));
    let report = parse_device_found(&device_found(PRINTED, 0, &eir)).unwrap();
    assert_eq!(report.name.as_deref(), Some("Pad"));
}

#[test]
fn a_payload_shorter_than_its_header_is_refused() {
    assert_eq!(parse_device_found(&[0; 13]), Err(ParseError::Truncated));
}

#[test]
fn extended_data_longer_than_the_payload_is_refused() {
    let mut payload = device_found(PRINTED, 0, &[]);
    payload[12] = 5;
    assert_eq!(parse_device_found(&payload), Err(ParseError::Truncated));
}

#[test]
fn a_field_that_runs_past_the_extended_data_is_refused() {
    let eir = [9u8, 0x09, b'x'];
    assert_eq!(parse_device_found(&device_found(PRINTED, 0, &eir)), Err(ParseError::Truncated));
}

#[test]
fn feeding_a_found_payload_starts_the_device_once_per_search() {
    let mut inq = Inquiry::new(10);
    inq.start();
    let payload = device_found(PRINTED, 0xC4, &field(0x09, b"Mouse"));
    assert_eq!(inq.feed(&payload), Ok(Some(Event::Found(BdAddr(PRINTED)))));
    assert_eq!(inq.feed(&payload), Ok(None));
    assert_eq!(inq.found().len(), 1);
    assert_eq!(inq.found()[0].label(), "Mouse");
}

#[test]
fn feeding_while_the_inquiry_is_idle_changes_nothing() {
    let mut inq = Inquiry::new(10);
    assert_eq!(inq.feed(&device_found(PRINTED, 0, &[])), Ok(None));
    assert!(inq.found().is_empty());
}

#[test]
fn a_bad_payload_is_an_error_and_leaves_the_found_list_alone() {
    let mut inq = Inquiry::new(10);
    inq.start();
    assert_eq!(inq.feed(&[0; 4]), Err(ParseError::Truncated));
    assert!(inq.found().is_empty());
}

#[test]
fn a_typed_key_record_starts_with_the_magic_and_the_kind_byte() {
    let key = SecretKey::from_bytes(&[7u8; 16]).unwrap();
    let record = encode(KeyKind::Symmetric, &key);
    assert_eq!(&record[..4], b"BFK1");
    assert_eq!(record[4], 1);
    assert_eq!(record.len(), 5 + 16);
}

#[test]
fn a_typed_key_reads_back_with_its_kind_and_bytes() {
    let key = SecretKey::from_bytes(&[9u8; 32]).unwrap();
    let stored = decode(&encode(KeyKind::Authentication, &key)).unwrap();
    assert_eq!(stored.kind, KeyKind::Authentication);
    assert_eq!(stored.key, key);
}

#[test]
fn a_secret_without_the_typed_key_magic_is_not_read_as_a_key() {
    assert_eq!(decode(b"plain password"), Err(RecordError::NotAKey));
}

#[test]
fn a_record_with_an_unknown_kind_is_refused() {
    let key = SecretKey::from_bytes(&[1u8; 24]).unwrap();
    let mut record = encode(KeyKind::Symmetric, &key);
    record[4] = 42;
    assert_eq!(decode(&record), Err(RecordError::UnknownKind(42)));
}

#[test]
fn a_record_with_a_key_of_the_wrong_length_is_refused() {
    let key = SecretKey::from_bytes(&[1u8; 16]).unwrap();
    let mut record = encode(KeyKind::Symmetric, &key);
    record.truncate(5 + 10);
    assert_eq!(decode(&record), Err(RecordError::InvalidKey(KeyError::InvalidSize)));
}

#[test]
fn a_keychain_refusal_is_passed_on_as_a_store_error() {
    let reply = Reply { request: 1, result: Err(keychain::Error::ItemNotFound) };
    assert_eq!(decode_reply(reply), Err(StoreError::Keychain(keychain::Error::ItemNotFound)));
}

#[test]
fn a_keychain_reply_with_a_typed_key_decodes_to_that_key() {
    let key = SecretKey::from_bytes(&[3u8; 16]).unwrap();
    let reply = Reply { request: 2, result: Ok(encode(KeyKind::Symmetric, &key)) };
    let stored = decode_reply(reply).unwrap();
    assert_eq!(stored.kind, KeyKind::Symmetric);
    assert_eq!(stored.key, key);
}

const RFC_CHALLENGE: &str = "Digest realm=\"testrealm@host.com\", qop=\"auth,auth-int\", \
    nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\", opaque=\"5ccc069c403ebaf9f0171e9517f40e41\"";

fn rfc_answer() -> DigestAnswer<'static> {
    DigestAnswer { user: "Mufasa", password: "Circle Of Life", method: "GET", uri: "/dir/index.html", cnonce: "0a4f113b", nonce_count: 1 }
}

#[test]
fn a_digest_challenge_yields_its_realm_nonce_opaque_and_auth_offer() {
    let challenge = parse_digest_challenge(RFC_CHALLENGE).unwrap();
    assert_eq!(challenge.realm, "testrealm@host.com");
    assert_eq!(challenge.nonce, "dcd98b7102dd2f0e8b11d0f600bfb0c093");
    assert_eq!(challenge.opaque.as_deref(), Some("5ccc069c403ebaf9f0171e9517f40e41"));
    assert!(challenge.qop_auth);
}

#[test]
fn the_digest_response_matches_the_worked_example_of_rfc_2617() {
    let challenge = parse_digest_challenge(RFC_CHALLENGE).unwrap();
    assert_eq!(response_digest(&challenge, &rfc_answer()), "6629fae49393a05397450978507c4ef1");
}

#[test]
fn the_authorization_header_carries_every_field_of_the_answer() {
    let challenge = parse_digest_challenge(RFC_CHALLENGE).unwrap();
    let header = authorization(&challenge, &rfc_answer());
    assert!(header.starts_with("Digest username=\"Mufasa\", realm=\"testrealm@host.com\""));
    assert!(header.contains("uri=\"/dir/index.html\""));
    assert!(header.contains("response=\"6629fae49393a05397450978507c4ef1\""));
    assert!(header.contains("qop=auth, nc=00000001, cnonce=\"0a4f113b\""));
    assert!(header.ends_with("opaque=\"5ccc069c403ebaf9f0171e9517f40e41\""));
}

#[test]
fn a_new_nonce_count_changes_the_response_when_auth_is_used() {
    let challenge = parse_digest_challenge(RFC_CHALLENGE).unwrap();
    let mut second = rfc_answer();
    second.nonce_count = 2;
    assert_ne!(response_digest(&challenge, &rfc_answer()), response_digest(&challenge, &second));
}

#[test]
fn a_challenge_without_qop_gives_a_32_digit_hex_response() {
    let challenge = parse_digest_challenge("Digest realm=\"r\", nonce=\"n\"").unwrap();
    assert!(!challenge.qop_auth);
    let response = response_digest(&challenge, &rfc_answer());
    assert_eq!(response.len(), 32);
    assert!(response.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    assert!(!authorization(&challenge, &rfc_answer()).contains("qop="));
}

#[test]
fn a_challenge_for_another_scheme_is_not_a_digest_challenge() {
    assert_eq!(parse_digest_challenge("Basic realm=\"r\""), None);
    assert_eq!(parse_digest_challenge("Diges realm=\"r\", nonce=\"n\""), None);
}

#[test]
fn a_digest_challenge_with_another_algorithm_is_refused() {
    assert_eq!(parse_digest_challenge("Digest realm=\"r\", nonce=\"n\", algorithm=SHA-256"), None);
}

#[test]
fn a_digest_challenge_offering_only_auth_int_is_refused() {
    assert_eq!(parse_digest_challenge("Digest realm=\"r\", nonce=\"n\", qop=\"auth-int\""), None);
}

#[test]
fn a_digest_challenge_without_a_nonce_is_refused() {
    assert_eq!(parse_digest_challenge("Digest realm=\"r\""), None);
}
