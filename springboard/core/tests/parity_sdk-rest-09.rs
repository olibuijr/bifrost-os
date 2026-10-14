//! Parity sdk-rest-09: headless tests for controller initialization (fabric, identifiers and the credential record). No
//! test touches the disk, a network or a key generator; the root key is a fixed test value.

use bifrost_core::matter_fabric::{
    compressed_fabric_id, Controller, Fabric, FabricError, COMPRESSED_FABRIC_ID_LEN, FABRIC_ID_MAX, NODE_ID_MAX,
    ROOT_KEY_LEN,
};

/// A fixed uncompressed key: the 0x04 marker, then 64 bytes of test coordinates.
fn key(fill: u8) -> Vec<u8> {
    let mut k = vec![0x04];
    k.extend(std::iter::repeat(fill).take(ROOT_KEY_LEN - 1));
    k
}

#[test]
fn a_fabric_refuses_the_reserved_identifiers() {
    assert_eq!(Fabric::new(0, &key(0xab)), Err(FabricError::FabricIdOutOfRange(0)));
    assert_eq!(Fabric::new(FABRIC_ID_MAX + 1, &key(0xab)), Err(FabricError::FabricIdOutOfRange(FABRIC_ID_MAX + 1)));
    assert!(Fabric::new(FABRIC_ID_MAX, &key(0xab)).is_ok(), "the top of the range is a valid fabric identifier");
}

#[test]
fn a_root_key_must_be_an_uncompressed_point() {
    assert_eq!(Fabric::new(7, &key(0xab)[..64]), Err(FabricError::RootKeyLength(64)));
    let mut compressed = key(0xab);
    compressed[0] = 0x02;
    assert_eq!(Fabric::new(7, &compressed), Err(FabricError::RootKeyNotUncompressed));
}

#[test]
fn the_compressed_fabric_identifier_is_eight_bytes_and_repeats_for_the_same_inputs() {
    let fabric = Fabric::new(42, &key(0x11)).unwrap();
    let again = Fabric::new(42, &key(0x11)).unwrap();
    assert_eq!(fabric.compressed_id().len(), COMPRESSED_FABRIC_ID_LEN);
    assert_eq!(fabric.compressed_id(), again.compressed_id());
}

#[test]
fn the_compressed_fabric_identifier_depends_on_both_the_fabric_and_the_key() {
    let base = Fabric::new(42, &key(0x11)).unwrap().compressed_id();
    assert_ne!(base, Fabric::new(43, &key(0x11)).unwrap().compressed_id(), "another fabric identifier");
    assert_ne!(base, Fabric::new(42, &key(0x12)).unwrap().compressed_id(), "another root key");
}

#[test]
fn the_compressed_identifier_uses_the_key_without_its_marker() {
    let k = key(0x33);
    let direct = compressed_fabric_id(9, &k[1..]).unwrap();
    assert_eq!(Fabric::new(9, &k).unwrap().compressed_id(), direct);
}

#[test]
fn a_new_controller_numbers_nodes_from_one_and_never_repeats_one() {
    let mut controller = Controller::initialize(None, 7, &key(0x22)).unwrap();
    assert_eq!(controller.fabric().fabric_id(), 7);
    assert_eq!(controller.next_node_id(), 1);
    assert_eq!(controller.allocate_node_id(), Ok(1));
    assert_eq!(controller.allocate_node_id(), Ok(2));
    assert_eq!(controller.next_node_id(), 3);
}

#[test]
fn a_stored_record_for_the_same_fabric_is_reused_with_its_node_count() {
    let mut first = Controller::initialize(None, 7, &key(0x22)).unwrap();
    first.allocate_node_id().unwrap();
    let again = Controller::initialize(Some(first.clone()), 7, &key(0x22)).unwrap();
    assert_eq!(again, first);
    assert_eq!(again.next_node_id(), 2, "the next device gets the next free identifier");
}

#[test]
fn a_stored_record_for_another_fabric_is_refused_and_not_replaced() {
    let stored = Controller::initialize(None, 7, &key(0x22)).unwrap();
    assert_eq!(
        Controller::initialize(Some(stored), 8, &key(0x22)),
        Err(FabricError::FabricMismatch { stored: 7, requested: 8 })
    );
}

#[test]
fn the_record_round_trips_through_its_text_form() {
    let mut controller = Controller::initialize(None, 0xabcdef, &key(0x44)).unwrap();
    controller.allocate_node_id().unwrap();
    let text = controller.encode();
    assert!(text.starts_with("fabric_id=0000000000abcdef\n"), "numbers are written as 16 hexadecimal digits");
    assert_eq!(Controller::decode(&text).unwrap(), controller);
}

#[test]
fn a_record_missing_a_field_is_refused_with_the_field_named() {
    let text = Controller::initialize(None, 7, &key(0x22)).unwrap().encode();
    let without_fabric: String = text.lines().filter(|l| !l.starts_with("fabric_id")).map(|l| format!("{l}\n")).collect();
    assert_eq!(Controller::decode(&without_fabric), Err(FabricError::Corrupt("missing fabric_id".into())));
}

#[test]
fn a_record_with_a_key_that_is_not_hexadecimal_is_refused() {
    let text = Controller::initialize(None, 7, &key(0x22)).unwrap().encode();
    let broken = text.replace("root_public_key=04", "root_public_key=0g");
    assert!(matches!(Controller::decode(&broken), Err(FabricError::Corrupt(_))));
}

#[test]
fn a_record_whose_node_count_is_zero_is_refused() {
    let text = Controller::initialize(None, 7, &key(0x22)).unwrap().encode();
    let zero = text.replace("next_node_id=0000000000000001", "next_node_id=0000000000000000");
    assert_eq!(Controller::decode(&zero), Err(FabricError::NodeIdOutOfRange(0)));
}

#[test]
fn node_identifiers_run_out_after_the_last_operational_one() {
    let text = Controller::initialize(None, 7, &key(0x22)).unwrap().encode();
    let last = text.replace("next_node_id=0000000000000001", &format!("next_node_id={:016x}", NODE_ID_MAX));
    let mut controller = Controller::decode(&last).unwrap();
    assert_eq!(controller.allocate_node_id(), Ok(NODE_ID_MAX));
    assert_eq!(controller.allocate_node_id(), Err(FabricError::NodeIdsExhausted));
}
