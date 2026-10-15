//! Parity sdk-rest-10: headless tests for passing a shared buffer to another process over a connected channel.
//! Both ends of the channel are made in this process; no compositor, device or display is involved.

use bifrost_app::buffer_handoff::{receive_buffer, send_buffer, BufferError, SharedBuffer, MAX_BUFFER_BYTES};
use bifrost_app::proto::sys::{shm_create, socketpair_pair};

#[test]
fn a_buffer_sent_over_a_channel_shows_the_same_pixels_to_the_receiver() {
    let (sender, receiver) = socketpair_pair().unwrap();
    let buffer = SharedBuffer::create("handoff-test", 4096).unwrap();
    buffer.map().unwrap().pixels_mut()[0] = 0xFF11_2233;

    send_buffer(&sender, b"frame-1", &buffer).unwrap();
    let received = receive_buffer(&receiver, true).unwrap().expect("one packet is queued");

    assert_eq!(received.header, b"frame-1".to_vec());
    assert_eq!(received.buffer.byte_len(), 4096);
    assert_eq!(received.buffer.map().unwrap().pixels()[0], 0xFF11_2233);
}

#[test]
fn a_write_by_the_sender_after_sending_is_visible_to_the_receiver() {
    let (sender, receiver) = socketpair_pair().unwrap();
    let buffer = SharedBuffer::create("handoff-shared", 4096).unwrap();
    send_buffer(&sender, b"", &buffer).unwrap();
    let received = receive_buffer(&receiver, true).unwrap().expect("one packet is queued");

    buffer.map().unwrap().pixels_mut()[1] = 7;

    assert_eq!(received.buffer.map().unwrap().pixels()[1], 7);
}

#[test]
fn a_zero_length_buffer_is_refused_before_any_memory_is_made() {
    assert!(matches!(SharedBuffer::create("empty", 0), Err(BufferError::Empty)));
}

#[test]
fn a_buffer_above_the_size_limit_is_refused_with_its_requested_size() {
    let too_big = MAX_BUFFER_BYTES + 1;
    assert!(matches!(SharedBuffer::create("huge", too_big), Err(BufferError::TooLarge(n)) if n == too_big));
}

#[test]
fn a_packet_without_a_descriptor_carries_no_buffer() {
    let (sender, receiver) = socketpair_pair().unwrap();
    sender.send(b"plain", &[]).unwrap();
    assert!(matches!(receive_buffer(&receiver, true), Err(BufferError::MissingDescriptor)));
}

#[test]
fn a_packet_with_two_descriptors_is_refused_with_the_count() {
    let (sender, receiver) = socketpair_pair().unwrap();
    let first = shm_create("extra-one", 16).unwrap();
    let second = shm_create("extra-two", 16).unwrap();
    sender.send(b"two", &[first.0, second.0]).unwrap();
    assert!(matches!(receive_buffer(&receiver, true), Err(BufferError::ExtraDescriptors(2))));
}

#[test]
fn a_non_blocking_receive_on_an_empty_channel_returns_none() {
    let (_sender, receiver) = socketpair_pair().unwrap();
    assert!(receive_buffer(&receiver, false).unwrap().is_none());
}

#[test]
fn a_receive_after_the_peer_closed_reports_the_channel_closed() {
    let (sender, receiver) = socketpair_pair().unwrap();
    drop(sender);
    assert!(matches!(receive_buffer(&receiver, true), Err(BufferError::Closed)));
}

#[test]
fn a_send_to_a_closed_peer_reports_the_channel_closed() {
    let (sender, receiver) = socketpair_pair().unwrap();
    drop(receiver);
    let buffer = SharedBuffer::create("orphan", 64).unwrap();
    assert!(matches!(send_buffer(&sender, b"gone", &buffer), Err(BufferError::Closed)));
}
