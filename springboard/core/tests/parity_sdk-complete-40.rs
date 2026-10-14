//! Parity sdk-complete-40: headless tests of the Bluetooth link addressing (L2CAP PSMs and CIDs) and the controller
//! packets (HCI commands and events) through the public API. No socket, no radio.

use bifrost_core::hci::{self, Command, CommandComplete, Event, HciStatus, Opcode};
use bifrost_core::l2cap::{self, Endpoint, PsmError, Transport};

#[test]
fn a_psm_must_be_odd_with_an_even_upper_octet() {
    assert_eq!(l2cap::validate_psm(l2cap::PSM_RFCOMM), Ok(0x0003));
    assert_eq!(l2cap::validate_psm(0x0004), Err(PsmError::EvenLowOctet));
    assert_eq!(l2cap::validate_psm(0x0201), Err(PsmError::OddHighOctet));
}

#[test]
fn a_dynamic_psm_is_one_the_device_assigns_at_run_time() {
    assert!(!l2cap::is_dynamic(l2cap::PSM_SDP));
    assert!(l2cap::is_dynamic(0x1003));
}

#[test]
fn the_keep_alive_link_uses_psm_one_or_the_attribute_channel() {
    assert_eq!(l2cap::keep_alive_endpoint(Transport::BrEdr), Endpoint::Psm(1));
    assert_eq!(l2cap::keep_alive_endpoint(Transport::LowEnergy), Endpoint::FixedCid(l2cap::CID_ATT));
}

#[test]
fn the_reset_command_is_the_three_bytes_the_specification_gives() {
    let reset = Command::new(hci::OP_RESET, Vec::new()).expect("a command without parameters");
    assert_eq!(reset.encode(), vec![0x03, 0x0C, 0x00]);
}

#[test]
fn a_disconnect_command_carries_its_handle_and_reason() {
    let disconnect = Command::new(hci::OP_DISCONNECT, vec![0x40, 0x00, 0x13]).expect("parameters fit");
    assert_eq!(disconnect.opcode.raw(), 0x0406);
    assert_eq!(disconnect.encode()[2], 3, "the header counts the three parameter bytes");
}

#[test]
fn an_opcode_too_wide_for_its_group_is_refused_before_a_command_is_built() {
    assert!(Opcode::new(64, 0).is_err());
    assert!(Command::new((0x01, 1024), Vec::new()).is_err());
}

#[test]
fn a_command_complete_event_reports_success_for_the_read_address_command() {
    let bytes = [hci::EVT_COMMAND_COMPLETE, 0x0A, 0x01, 0x09, 0x10, 0x00, 0x00, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
    let event = Event::parse(&bytes).expect("a complete event");
    let done = CommandComplete::parse(&event).expect("a command complete");
    assert_eq!(done.opcode, Opcode::new(0x04, 0x0009).unwrap());
    assert_eq!(done.status, HciStatus::Success);
    assert_eq!(done.return_parameters.len(), 6, "the six address bytes follow the status");
}

#[test]
fn a_truncated_event_is_refused_rather_than_read_past_its_end() {
    assert!(Event::parse(&[hci::EVT_INQUIRY_COMPLETE, 0x01]).is_err());
}
