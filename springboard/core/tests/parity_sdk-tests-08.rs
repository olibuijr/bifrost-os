//! Parity tests (sdk-tests-08) for the authentication context: one evaluation at a time, the biometric failure count that
//! leads to lockout, passcode results that leave that count alone, and the refusals. Pure host tests with concrete values:
//! no sensor, no prompt, no secure storage, no clock.

use bifrost_core::auth_context::{AuthContext, ContextError, Phase};
use bifrost_core::biometry::{BiometryType, Device, Failure, Method, Policy};

fn phone() -> Device {
    Device {
        biometry: BiometryType::Face,
        enrolled_sets: vec!["owner".to_string()],
        passcode_set: true,
        failures: 0,
        max_failures: 3,
    }
}

#[test]
fn context_begin_runs_one_evaluation_at_a_time() {
    let mut ctx = AuthContext::new(Policy::DeviceOwner, phone());
    assert_eq!(ctx.begin(), Ok(Method::Biometry));
    assert_eq!(ctx.phase(), Phase::Evaluating(Method::Biometry));
    assert_eq!(ctx.begin(), Err(ContextError::Busy));
}

#[test]
fn context_biometric_success_clears_the_failure_count() {
    let mut d = phone();
    d.failures = 2;
    let mut ctx = AuthContext::new(Policy::DeviceOwner, d);
    ctx.begin().unwrap();
    assert_eq!(ctx.finish(Ok(())), Ok(Phase::Succeeded(Method::Biometry)));
    assert_eq!(ctx.device().failures, 0);
}

#[test]
fn context_biometric_failure_adds_one_to_the_count() {
    let mut ctx = AuthContext::new(Policy::DeviceOwner, phone());
    ctx.begin().unwrap();
    assert_eq!(
        ctx.finish(Err(Failure::BiometryUnavailable)),
        Ok(Phase::Failed(Failure::BiometryUnavailable))
    );
    assert_eq!(ctx.device().failures, 1);
}

#[test]
fn context_three_biometric_failures_leave_only_the_passcode() {
    let mut ctx = AuthContext::new(Policy::BiometricsOnly, phone());
    for _ in 0..3 {
        ctx.begin().unwrap();
        ctx.finish(Err(Failure::BiometryUnavailable)).unwrap();
    }
    assert_eq!(ctx.device().failures, 3);
    assert_eq!(ctx.can_evaluate(), Err(Failure::BiometryLockout));
    assert_eq!(
        AuthContext::new(Policy::DeviceOwner, ctx.device().clone()).can_evaluate(),
        Ok(Method::Passcode)
    );
}

#[test]
fn context_passcode_result_does_not_touch_the_biometric_count() {
    let mut d = phone();
    d.failures = 3;
    let mut ctx = AuthContext::new(Policy::DeviceOwner, d);
    assert_eq!(ctx.begin(), Ok(Method::Passcode));
    ctx.finish(Ok(())).unwrap();
    assert_eq!(ctx.phase(), Phase::Succeeded(Method::Passcode));
    assert_eq!(ctx.device().failures, 3);
}

#[test]
fn context_without_enrolled_identities_falls_back_to_the_passcode() {
    let d = Device { enrolled_sets: Vec::new(), ..phone() };
    let mut ctx = AuthContext::new(Policy::DeviceOwner, d);
    assert_eq!(ctx.begin(), Ok(Method::Passcode));
}

#[test]
fn context_unavailable_policy_refuses_to_start_and_keeps_the_phase() {
    let mut d = phone();
    d.failures = 3;
    d.passcode_set = false;
    let mut ctx = AuthContext::new(Policy::DeviceOwner, d);
    assert_eq!(ctx.begin(), Err(ContextError::Unavailable(Failure::PasscodeNotSet)));
    assert_eq!(ctx.phase(), Phase::Ready);
}

#[test]
fn context_invalidated_context_refuses_every_call() {
    let mut ctx = AuthContext::new(Policy::DeviceOwner, phone());
    ctx.invalidate();
    assert_eq!(ctx.phase(), Phase::Invalidated);
    assert_eq!(ctx.begin(), Err(ContextError::Invalidated));
    assert_eq!(ctx.finish(Ok(())), Err(ContextError::Invalidated));
}

#[test]
fn context_finish_without_a_running_evaluation_is_an_error() {
    let mut ctx = AuthContext::new(Policy::DeviceOwner, phone());
    assert_eq!(ctx.finish(Ok(())), Err(ContextError::NotEvaluating));
    assert_eq!(ctx.phase(), Phase::Ready);
}

/// Guest device model (`guest_vm`): network, input, serial, memory balloon and storage configuration. Each device is
/// checked as plain data through `GuestConfig::validate`; nothing is started and no disk, network or display is touched.
mod guest_devices {
    use bifrost_core::disk_image::{create_raw, BlockSize};
    use bifrost_core::guest_vm::{
        CachingMode, ConfigError, Device, GuestConfig, InputKind, MemoryBalloon, NetworkAttachment, NetworkDevice, SerialAttachment,
        SerialPort, StorageAttachment, SyncMode, MAX_NETWORK_DEVICES, MAX_SERIAL_PORTS, MIB,
    };

    const HOST_CPUS: u32 = 4;

    fn config(devices: Vec<Device>) -> GuestConfig {
        GuestConfig { cpu_count: 2, memory_bytes: 512 * MIB, devices }
    }

    fn nat(mac: Option<[u8; 6]>) -> Device {
        Device::Network(NetworkDevice { attachment: NetworkAttachment::Nat, mac })
    }

    fn serial_file(path: &str) -> Device {
        Device::Serial(SerialPort { attachment: SerialAttachment::File(path.to_string()) })
    }

    #[test]
    fn a_network_interface_without_an_address_is_valid() {
        assert_eq!(config(vec![nat(None)]).validate(HOST_CPUS), Ok(()));
        let host_only = Device::Network(NetworkDevice { attachment: NetworkAttachment::HostOnly, mac: None });
        assert_eq!(config(vec![host_only]).validate(HOST_CPUS), Ok(()));
    }

    #[test]
    fn a_network_address_must_be_unicast_and_not_zero() {
        let locally_administered = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
        assert_eq!(config(vec![nat(Some(locally_administered))]).validate(HOST_CPUS), Ok(()));

        let multicast = [0x01, 0x11, 0x22, 0x33, 0x44, 0x55];
        assert_eq!(config(vec![nat(Some(multicast))]).validate(HOST_CPUS), Err(ConfigError::BadMacAddress));
        assert_eq!(config(vec![nat(Some([0; 6]))]).validate(HOST_CPUS), Err(ConfigError::BadMacAddress));
    }

    #[test]
    fn two_network_interfaces_may_not_share_an_address() {
        let mac = [0x02, 0, 0, 0, 0, 1];
        assert_eq!(config(vec![nat(Some(mac)), nat(Some(mac))]).validate(HOST_CPUS), Err(ConfigError::DuplicateMacAddress));
        assert_eq!(config(vec![nat(Some(mac)), nat(Some([0x02, 0, 0, 0, 0, 2]))]).validate(HOST_CPUS), Ok(()));
        assert_eq!(config(vec![nat(None), nat(None)]).validate(HOST_CPUS), Ok(()));
    }

    #[test]
    fn a_guest_may_have_up_to_the_network_interface_limit() {
        assert_eq!(MAX_NETWORK_DEVICES, 4);
        let at_limit: Vec<Device> = (0..MAX_NETWORK_DEVICES as u8).map(|i| nat(Some([0x02, 0, 0, 0, 0, i]))).collect();
        assert_eq!(config(at_limit.clone()).validate(HOST_CPUS), Ok(()));

        let mut over = at_limit;
        over.push(nat(None));
        assert_eq!(config(over).validate(HOST_CPUS), Err(ConfigError::TooManyNetworkDevices));
    }

    #[test]
    fn a_guest_may_have_one_keyboard_and_one_pointing_device() {
        let both = config(vec![Device::Input(InputKind::Keyboard), Device::Input(InputKind::Pointing)]);
        assert_eq!(both.validate(HOST_CPUS), Ok(()));
    }

    #[test]
    fn a_second_keyboard_or_pointing_device_is_refused_by_kind() {
        let two_keyboards = config(vec![Device::Input(InputKind::Keyboard), Device::Input(InputKind::Keyboard)]);
        assert_eq!(two_keyboards.validate(HOST_CPUS), Err(ConfigError::DuplicateDevice("keyboard")));

        let two_pointers = config(vec![Device::Input(InputKind::Pointing), Device::Input(InputKind::Pointing)]);
        assert_eq!(two_pointers.validate(HOST_CPUS), Err(ConfigError::DuplicateDevice("pointing")));
    }

    #[test]
    fn input_kinds_are_compared_by_their_kind() {
        assert_ne!(InputKind::Keyboard, InputKind::Pointing);
        assert_eq!(Device::Input(InputKind::Pointing), Device::Input(InputKind::Pointing));
    }

    #[test]
    fn a_serial_port_writes_to_an_absolute_file_or_nowhere() {
        assert_eq!(config(vec![serial_file("/var/log/guest-serial.log")]).validate(HOST_CPUS), Ok(()));
        let discard = Device::Serial(SerialPort { attachment: SerialAttachment::Discard });
        assert_eq!(config(vec![discard]).validate(HOST_CPUS), Ok(()));
    }

    #[test]
    fn a_serial_file_must_be_an_absolute_path_to_a_file_name() {
        assert_eq!(config(vec![serial_file("guest.log")]).validate(HOST_CPUS), Err(ConfigError::BadSerialPath));
        assert_eq!(config(vec![serial_file("/")]).validate(HOST_CPUS), Err(ConfigError::BadSerialPath));
        assert_eq!(config(vec![serial_file("/var/log/")]).validate(HOST_CPUS), Err(ConfigError::BadSerialPath));
        assert_eq!(config(vec![serial_file("/var/\0log")]).validate(HOST_CPUS), Err(ConfigError::BadSerialPath));
    }

    #[test]
    fn a_guest_may_have_up_to_the_serial_port_limit() {
        assert_eq!(MAX_SERIAL_PORTS, 4);
        let at_limit: Vec<Device> = (0..MAX_SERIAL_PORTS).map(|i| serial_file(&format!("/var/log/port{i}.log"))).collect();
        assert_eq!(config(at_limit.clone()).validate(HOST_CPUS), Ok(()));

        let mut over = at_limit;
        over.push(serial_file("/var/log/port4.log"));
        assert_eq!(config(over).validate(HOST_CPUS), Err(ConfigError::TooManySerialPorts));
    }

    #[test]
    fn a_guest_may_have_one_memory_balloon_with_or_without_page_reporting() {
        let reporting = Device::MemoryBalloon(MemoryBalloon { free_page_reporting: true });
        let plain = Device::MemoryBalloon(MemoryBalloon { free_page_reporting: false });
        assert_eq!(config(vec![reporting.clone()]).validate(HOST_CPUS), Ok(()));
        assert_eq!(config(vec![plain]).validate(HOST_CPUS), Ok(()));
        assert_ne!(reporting, Device::MemoryBalloon(MemoryBalloon { free_page_reporting: false }));
    }

    #[test]
    fn a_second_memory_balloon_is_refused() {
        let balloon = Device::MemoryBalloon(MemoryBalloon { free_page_reporting: true });
        assert_eq!(config(vec![balloon.clone(), balloon]).validate(HOST_CPUS), Err(ConfigError::DuplicateDevice("memory balloon")));
    }

    #[test]
    fn a_raw_disk_image_of_four_mebibytes_has_8192_512_byte_blocks() {
        let image = create_raw(4 * MIB, BlockSize::Bytes512).unwrap();
        assert_eq!(image.block_count, 8192);
        assert!(!image.read_only);
    }

    #[test]
    fn a_storage_attachment_keeps_its_caching_and_sync_modes() {
        let image = create_raw(MIB, BlockSize::Bytes4096).unwrap();
        assert_eq!(image.block_count, 256);
        let attachment = StorageAttachment::from_image(image, CachingMode::Uncached, SyncMode::Fsync);
        assert_eq!(attachment.caching, CachingMode::Uncached);
        assert_eq!(attachment.sync, SyncMode::Fsync);
        assert!(!attachment.read_only());
    }

    #[test]
    fn a_read_only_image_attaches_read_only() {
        let mut image = create_raw(MIB, BlockSize::Bytes512).unwrap();
        image.read_only = true;
        let attachment = StorageAttachment::from_image(image, CachingMode::Automatic, SyncMode::Unsynced);
        assert!(attachment.read_only());
    }

    #[test]
    fn a_guest_may_have_several_disk_images_and_they_are_not_counted_as_duplicates() {
        let system = create_raw(MIB, BlockSize::Bytes512).unwrap();
        let data = create_raw(2 * MIB, BlockSize::Bytes512).unwrap();
        let disks = vec![
            Device::Storage(StorageAttachment::from_image(system, CachingMode::Cached, SyncMode::Full)),
            Device::Storage(StorageAttachment::from_image(data, CachingMode::Automatic, SyncMode::Fsync)),
        ];
        assert_eq!(config(disks).validate(HOST_CPUS), Ok(()));
    }

    #[test]
    fn a_full_device_set_validates_as_one_configuration() {
        let system = create_raw(MIB, BlockSize::Bytes512).unwrap();
        let devices = vec![
            Device::Entropy,
            Device::Socket,
            Device::Storage(StorageAttachment::from_image(system, CachingMode::Cached, SyncMode::Full)),
            nat(Some([0x02, 0x42, 0xac, 0x11, 0x00, 0x02])),
            Device::Input(InputKind::Keyboard),
            Device::Input(InputKind::Pointing),
            Device::MemoryBalloon(MemoryBalloon { free_page_reporting: true }),
            serial_file("/var/log/guest-serial.log"),
        ];
        assert_eq!(config(devices).validate(HOST_CPUS), Ok(()));
    }
}
