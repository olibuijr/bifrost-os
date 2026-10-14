//! Parity sdk-os-05: headless tests for the shared memory files that back cross-process buffers. Each test creates
//! its own file and needs no display or device.

use bifrost_proto::sys::{shm_create, shm_len, shm_resize};

#[test]
fn headless_a_shared_memory_file_has_the_size_it_was_created_with() {
    let fd = shm_create("bifrost-sdk-os-05-created", 8192).unwrap();
    assert_eq!(shm_len(&fd).unwrap(), 8192);
}

#[test]
fn headless_a_shared_memory_file_grows_and_shrinks_to_the_size_asked_for() {
    let fd = shm_create("bifrost-sdk-os-05-resized", 4096).unwrap();
    shm_resize(&fd, 16_384).unwrap();
    assert_eq!(shm_len(&fd).unwrap(), 16_384);
    shm_resize(&fd, 4096).unwrap();
    assert_eq!(shm_len(&fd).unwrap(), 4096);
}
