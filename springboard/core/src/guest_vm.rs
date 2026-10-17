//! Linux guest model: configuration checks, the runtime state machine, and the device list.
//!
//! Pure: nothing is started here. A launcher outside this module takes a configuration that passed `validate`, boots the
//! guest on a hypervisor, and reports every step through `Machine::transition`. The state names, and the rule for which
//! step may follow which, mirror the states in the official virtualization framework documentation. The limits below
//! marked "BifrOSt" are this project's own choices, not values the official documentation gives.

use crate::disk_image::ImageProperties;

pub const MIB: u64 = 1024 * 1024;

/// BifrOSt minimum guest memory. This is a project choice, not a documented framework limit.
pub const MIN_MEMORY_BYTES: u64 = 64 * MIB;

/// BifrOSt longest shared-directory tag. This is a project choice, not a documented framework limit.
pub const MAX_SHARE_TAG_LEN: usize = 64;

/// Runtime states of a guest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Stopped,
    Starting,
    Running,
    Pausing,
    Paused,
    Resuming,
    Stopping,
    Saving,
    Restoring,
    Error,
}

/// Whether a runtime step may move a guest from `from` to `to`.
pub fn allowed(from: State, to: State) -> bool {
    use State::*;
    matches!(
        (from, to),
        (Stopped, Starting)
            | (Stopped, Restoring)
            | (Starting, Running)
            | (Starting, Error)
            | (Running, Pausing)
            | (Running, Stopping)
            | (Running, Error)
            | (Pausing, Paused)
            | (Pausing, Error)
            | (Paused, Resuming)
            | (Paused, Stopping)
            | (Paused, Saving)
            | (Paused, Error)
            | (Resuming, Running)
            | (Resuming, Error)
            | (Stopping, Stopped)
            | (Stopping, Error)
            | (Saving, Paused)
            | (Saving, Error)
            | (Restoring, Paused)
            | (Restoring, Error)
            | (Error, Stopping)
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmError {
    /// The step is not allowed from the current state. The state is left unchanged.
    InvalidTransition { from: State, to: State },
    /// A stop request is only meaningful while the guest is running.
    NotRunning,
}

/// Runtime state of one guest. It starts stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Machine {
    state: State,
}

impl Default for Machine {
    fn default() -> Self {
        Machine { state: State::Stopped }
    }
}

impl Machine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// Moves to `to` if the rule allows it. A refused step leaves the state as it was.
    pub fn transition(&mut self, to: State) -> Result<State, VmError> {
        if !allowed(self.state, to) {
            return Err(VmError::InvalidTransition { from: self.state, to });
        }
        self.state = to;
        Ok(self.state)
    }

    pub fn can_start(&self) -> bool {
        allowed(self.state, State::Starting)
    }

    pub fn can_pause(&self) -> bool {
        allowed(self.state, State::Pausing)
    }

    pub fn can_resume(&self) -> bool {
        allowed(self.state, State::Resuming)
    }

    pub fn can_stop(&self) -> bool {
        allowed(self.state, State::Stopping)
    }

    pub fn can_request_stop(&self) -> bool {
        self.state == State::Running
    }

    /// Asks the guest to stop itself. The state does not change; the guest reports the stop later.
    pub fn request_stop(&self) -> Result<(), VmError> {
        if self.can_request_stop() {
            Ok(())
        } else {
            Err(VmError::NotRunning)
        }
    }
}

/// How the host caches and syncs a disk image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachingMode {
    Automatic,
    Uncached,
    Cached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncMode {
    /// Every write is flushed to the device before it is reported done.
    Full,
    /// Only explicit flush requests from the guest are honoured.
    Fsync,
    /// The host does not synchronise the image.
    Unsynced,
}

/// A disk image attached to a guest. The read-only flag comes from the image itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageAttachment {
    pub image: ImageProperties,
    pub caching: CachingMode,
    pub sync: SyncMode,
}

impl StorageAttachment {
    pub fn from_image(image: ImageProperties, caching: CachingMode, sync: SyncMode) -> Self {
        StorageAttachment { image, caching, sync }
    }

    pub fn read_only(&self) -> bool {
        self.image.read_only
    }
}

/// A host directory shared into the guest under a tag the guest mounts by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryShare {
    pub tag: String,
    pub read_only: bool,
}

/// BifrOSt most scanouts on one graphics device. This is a project choice, not a documented framework limit.
pub const MAX_SCANOUTS: usize = 4;

/// BifrOSt largest scanout side in pixels. This is a project choice, not a documented framework limit.
pub const MAX_SCANOUT_PIXELS: u32 = 8192;

/// One display a guest's graphics device drives, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scanout {
    pub width: u32,
    pub height: u32,
}

/// A paravirtualized graphics device. The guest sees each scanout as a display, and the host shows it in a view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphicsDevice {
    pub scanouts: Vec<Scanout>,
}

/// Bytes per pixel of a scanout framebuffer: 32-bit colour.
const FRAMEBUFFER_BYTES_PER_PIXEL: u64 = 4;

impl GraphicsDevice {
    /// Bytes of the framebuffer behind scanout `index`, or `None` when there is no such scanout.
    pub fn framebuffer_bytes(&self, index: usize) -> Option<u64> {
        self.scanouts
            .get(index)
            .map(|s| u64::from(s.width) * u64::from(s.height) * FRAMEBUFFER_BYTES_PER_PIXEL)
    }

    /// Bytes of the framebuffers of every scanout together.
    pub fn total_framebuffer_bytes(&self) -> u64 {
        self.scanouts
            .iter()
            .map(|s| u64::from(s.width) * u64::from(s.height) * FRAMEBUFFER_BYTES_PER_PIXEL)
            .sum()
    }
}

/// BifrOSt most network interfaces on one guest. This is a project choice, not a documented framework limit.
pub const MAX_NETWORK_DEVICES: usize = 4;

/// BifrOSt most serial ports on one guest. This is a project choice, not a documented framework limit.
pub const MAX_SERIAL_PORTS: usize = 4;

/// How a guest's network interface reaches the network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkAttachment {
    /// The guest shares the host's address and is translated through network address translation.
    Nat,
    /// The guest reaches the host and other guests on this host only.
    HostOnly,
}

/// A network interface. `mac` is a unicast, non-zero station address; `None` lets the launcher choose one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkDevice {
    pub attachment: NetworkAttachment,
    pub mac: Option<[u8; 6]>,
}

/// The two input devices a guest may have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Keyboard,
    Pointing,
}

/// Where a serial port's output goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SerialAttachment {
    /// The output is dropped.
    Discard,
    /// The output is appended to the file at this absolute path.
    File(String),
}

/// A serial port (a console the guest writes to).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerialPort {
    pub attachment: SerialAttachment,
}

/// A memory balloon. With free page reporting the guest tells the host which pages it no longer uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryBalloon {
    pub free_page_reporting: bool,
}

/// Why a balloon target is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BalloonError {
    /// The guest configuration has no memory balloon.
    NoBalloon,
    /// The target is below `MIN_MEMORY_BYTES`.
    TargetTooSmall,
    /// The target is above the memory the guest was configured with.
    TargetAboveMemory,
    /// The target is not a whole number of MiB.
    TargetNotWholeMiB,
}

/// One device in a guest configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Device {
    /// Entropy source for the guest. A guest has at most one.
    Entropy,
    /// Host-to-guest socket channel. A guest has at most one.
    Socket,
    /// Graphics for the guest's displays. A guest has at most one.
    Graphics(GraphicsDevice),
    /// A shared host directory. Tags must be unique.
    DirectoryShare(DirectoryShare),
    /// A disk image. A guest may have several.
    Storage(StorageAttachment),
    /// A network interface. A guest may have up to `MAX_NETWORK_DEVICES`.
    Network(NetworkDevice),
    /// A keyboard or a pointing device. A guest has at most one of each.
    Input(InputKind),
    /// A serial port. A guest may have up to `MAX_SERIAL_PORTS`.
    Serial(SerialPort),
    /// A memory balloon. A guest has at most one.
    MemoryBalloon(MemoryBalloon),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// The CPU count is zero or more than the host has.
    BadCpuCount,
    /// Memory is below `MIN_MEMORY_BYTES`.
    MemoryTooSmall,
    /// Memory is not a whole number of MiB.
    MemoryNotWholeMiB,
    /// A device that may appear only once appears twice. Carries the kind name.
    DuplicateDevice(&'static str),
    /// A share tag is empty, too long, or has a character outside ASCII letters, digits, `-` and `_`.
    BadShareTag,
    /// Two shares use the same tag.
    DuplicateShareTag,
    /// A graphics device has no scanout.
    NoScanout,
    /// A graphics device has more than `MAX_SCANOUTS` scanouts.
    TooManyScanouts,
    /// A scanout has a zero side, or a side above `MAX_SCANOUT_PIXELS`.
    BadScanoutSize,
    /// The guest has more than `MAX_NETWORK_DEVICES` network interfaces.
    TooManyNetworkDevices,
    /// A network address is multicast, all zero, or shared with another interface.
    BadMacAddress,
    /// Two network interfaces use the same address.
    DuplicateMacAddress,
    /// The guest has more than `MAX_SERIAL_PORTS` serial ports.
    TooManySerialPorts,
    /// A serial port's file is not an absolute path to a file name.
    BadSerialPath,
}

/// The complete guest description a launcher receives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestConfig {
    pub cpu_count: u32,
    pub memory_bytes: u64,
    pub devices: Vec<Device>,
}

fn share_tag_ok(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= MAX_SHARE_TAG_LEN
        && tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl GuestConfig {
    /// The memory a balloon would hand back to the host for a guest that should use `target_bytes`: the configured
    /// memory minus the target. Only a configuration with a balloon device accepts a target, and the target must be a
    /// whole number of MiB between `MIN_MEMORY_BYTES` and the configured memory. The guest is not resized here; a
    /// launcher applies the returned reclaim to the running guest.
    pub fn balloon_reclaim(&self, target_bytes: u64) -> Result<u64, BalloonError> {
        if !self.devices.iter().any(|d| matches!(d, Device::MemoryBalloon(_))) {
            return Err(BalloonError::NoBalloon);
        }
        if target_bytes < MIN_MEMORY_BYTES {
            return Err(BalloonError::TargetTooSmall);
        }
        if target_bytes > self.memory_bytes {
            return Err(BalloonError::TargetAboveMemory);
        }
        if !target_bytes.is_multiple_of(MIB) {
            return Err(BalloonError::TargetNotWholeMiB);
        }
        Ok(self.memory_bytes - target_bytes)
    }

    /// The graphics device, if the configuration has one.
    pub fn graphics(&self) -> Option<&GraphicsDevice> {
        self.devices.iter().find_map(|d| match d {
            Device::Graphics(g) => Some(g),
            _ => None,
        })
    }

    /// Checks the configuration against the host's CPU count. Returns the first problem found.
    pub fn validate(&self, host_cpus: u32) -> Result<(), ConfigError> {
        if self.cpu_count == 0 || self.cpu_count > host_cpus {
            return Err(ConfigError::BadCpuCount);
        }
        if self.memory_bytes < MIN_MEMORY_BYTES {
            return Err(ConfigError::MemoryTooSmall);
        }
        if !self.memory_bytes.is_multiple_of(MIB) {
            return Err(ConfigError::MemoryNotWholeMiB);
        }

        let entropy = self.devices.iter().filter(|d| matches!(d, Device::Entropy)).count();
        if entropy > 1 {
            return Err(ConfigError::DuplicateDevice("entropy"));
        }
        let sockets = self.devices.iter().filter(|d| matches!(d, Device::Socket)).count();
        if sockets > 1 {
            return Err(ConfigError::DuplicateDevice("socket"));
        }
        let graphics: Vec<&GraphicsDevice> = self
            .devices
            .iter()
            .filter_map(|d| match d {
                Device::Graphics(g) => Some(g),
                _ => None,
            })
            .collect();
        if graphics.len() > 1 {
            return Err(ConfigError::DuplicateDevice("graphics"));
        }
        for device in graphics {
            if device.scanouts.is_empty() {
                return Err(ConfigError::NoScanout);
            }
            if device.scanouts.len() > MAX_SCANOUTS {
                return Err(ConfigError::TooManyScanouts);
            }
            let size_ok = |px: u32| (1..=MAX_SCANOUT_PIXELS).contains(&px);
            if device.scanouts.iter().any(|s| !size_ok(s.width) || !size_ok(s.height)) {
                return Err(ConfigError::BadScanoutSize);
            }
        }

        let mut tags: Vec<&str> = Vec::new();
        for device in &self.devices {
            if let Device::DirectoryShare(share) = device {
                if !share_tag_ok(&share.tag) {
                    return Err(ConfigError::BadShareTag);
                }
                if tags.contains(&share.tag.as_str()) {
                    return Err(ConfigError::DuplicateShareTag);
                }
                tags.push(share.tag.as_str());
            }
        }
        self.validate_inputs_and_balloon()?;
        self.validate_networks()?;
        self.validate_serial_ports()
    }

    fn count(&self, pick: impl Fn(&Device) -> bool) -> usize {
        self.devices.iter().filter(|d| pick(*d)).count()
    }

    fn validate_inputs_and_balloon(&self) -> Result<(), ConfigError> {
        let once: [(&'static str, fn(&Device) -> bool); 3] = [
            ("keyboard", |d| matches!(d, Device::Input(InputKind::Keyboard))),
            ("pointing", |d| matches!(d, Device::Input(InputKind::Pointing))),
            ("memory balloon", |d| matches!(d, Device::MemoryBalloon(_))),
        ];
        for (name, pick) in once {
            if self.count(pick) > 1 {
                return Err(ConfigError::DuplicateDevice(name));
            }
        }
        Ok(())
    }

    fn validate_networks(&self) -> Result<(), ConfigError> {
        let mut macs: Vec<[u8; 6]> = Vec::new();
        let mut interfaces = 0;
        for device in &self.devices {
            if let Device::Network(nic) = device {
                interfaces += 1;
                if let Some(mac) = nic.mac {
                    if !mac_ok(mac) {
                        return Err(ConfigError::BadMacAddress);
                    }
                    if macs.contains(&mac) {
                        return Err(ConfigError::DuplicateMacAddress);
                    }
                    macs.push(mac);
                }
            }
        }
        if interfaces > MAX_NETWORK_DEVICES {
            return Err(ConfigError::TooManyNetworkDevices);
        }
        Ok(())
    }

    fn validate_serial_ports(&self) -> Result<(), ConfigError> {
        if self.count(|d| matches!(d, Device::Serial(_))) > MAX_SERIAL_PORTS {
            return Err(ConfigError::TooManySerialPorts);
        }
        for device in &self.devices {
            if let Device::Serial(SerialPort { attachment: SerialAttachment::File(path) }) = device {
                if !serial_path_ok(path) {
                    return Err(ConfigError::BadSerialPath);
                }
            }
        }
        Ok(())
    }
}

/// A station address is unicast (the low bit of the first octet is clear) and not all zero.
fn mac_ok(mac: [u8; 6]) -> bool {
    mac != [0; 6] && mac[0] & 1 == 0
}

/// A serial file is an absolute path that names a file, not a directory.
fn serial_path_ok(path: &str) -> bool {
    path.starts_with('/') && path.len() > 1 && !path.ends_with('/') && !path.contains('\0')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk_image::{create_raw, BlockSize};

    fn config(cpus: u32, memory: u64, devices: Vec<Device>) -> GuestConfig {
        GuestConfig { cpu_count: cpus, memory_bytes: memory, devices }
    }

    fn share(tag: &str) -> Device {
        Device::DirectoryShare(DirectoryShare { tag: tag.to_string(), read_only: false })
    }

    #[test]
    fn allowed_follows_the_lifecycle_edges() {
        assert!(allowed(State::Stopped, State::Starting));
        assert!(allowed(State::Running, State::Pausing));
        assert!(allowed(State::Paused, State::Stopping));
        assert!(!allowed(State::Stopped, State::Running));
        assert!(!allowed(State::Starting, State::Stopped));
        assert!(!allowed(State::Running, State::Paused));
    }

    #[test]
    fn full_pause_and_resume_cycle() {
        let mut m = Machine::new();
        assert_eq!(m.state(), State::Stopped);
        m.transition(State::Starting).unwrap();
        m.transition(State::Running).unwrap();
        m.transition(State::Pausing).unwrap();
        m.transition(State::Paused).unwrap();
        m.transition(State::Resuming).unwrap();
        assert_eq!(m.transition(State::Running), Ok(State::Running));
    }

    #[test]
    fn refused_step_leaves_the_state_unchanged() {
        let mut m = Machine::new();
        m.transition(State::Starting).unwrap();
        m.transition(State::Running).unwrap();
        assert_eq!(
            m.transition(State::Paused),
            Err(VmError::InvalidTransition { from: State::Running, to: State::Paused })
        );
        assert_eq!(m.state(), State::Running);
    }

    #[test]
    fn can_helpers_match_the_paused_state() {
        let mut m = Machine::new();
        m.transition(State::Starting).unwrap();
        m.transition(State::Running).unwrap();
        m.transition(State::Pausing).unwrap();
        m.transition(State::Paused).unwrap();
        assert!(m.can_resume());
        assert!(m.can_stop());
        assert!(!m.can_pause());
        assert!(!m.can_start());
        assert!(!m.can_request_stop());
    }

    #[test]
    fn request_stop_only_while_running_and_keeps_the_state() {
        let stopped = Machine::new();
        assert_eq!(stopped.request_stop(), Err(VmError::NotRunning));

        let mut running = Machine::new();
        running.transition(State::Starting).unwrap();
        running.transition(State::Running).unwrap();
        assert_eq!(running.request_stop(), Ok(()));
        assert_eq!(running.state(), State::Running);
    }

    #[test]
    fn a_balloon_target_reclaims_the_difference_from_the_configured_memory() {
        let balloon = Device::MemoryBalloon(MemoryBalloon { free_page_reporting: true });
        let guest = config(2, 2048 * MIB, vec![balloon]);
        assert_eq!(guest.balloon_reclaim(1024 * MIB), Ok(1024 * MIB));
        assert_eq!(guest.balloon_reclaim(2048 * MIB), Ok(0));
    }

    #[test]
    fn a_balloon_target_is_refused_without_a_balloon_or_outside_the_memory_range() {
        let balloon = Device::MemoryBalloon(MemoryBalloon { free_page_reporting: false });
        let with = config(2, 2048 * MIB, vec![balloon]);
        let without = config(2, 2048 * MIB, vec![]);
        assert_eq!(without.balloon_reclaim(1024 * MIB), Err(BalloonError::NoBalloon));
        assert_eq!(with.balloon_reclaim(32 * MIB), Err(BalloonError::TargetTooSmall));
        assert_eq!(with.balloon_reclaim(4096 * MIB), Err(BalloonError::TargetAboveMemory));
        assert_eq!(with.balloon_reclaim(1024 * MIB + 1), Err(BalloonError::TargetNotWholeMiB));
    }

    #[test]
    fn validate_checks_cpus_and_memory() {
        let ok = config(8, 2 * 1024 * MIB, vec![]);
        assert_eq!(ok.validate(8), Ok(()));
        assert_eq!(config(0, 2 * 1024 * MIB, vec![]).validate(8), Err(ConfigError::BadCpuCount));
        assert_eq!(config(9, 2 * 1024 * MIB, vec![]).validate(8), Err(ConfigError::BadCpuCount));
        assert_eq!(config(2, 63 * MIB, vec![]).validate(8), Err(ConfigError::MemoryTooSmall));
        assert_eq!(config(2, 64 * MIB + 1, vec![]).validate(8), Err(ConfigError::MemoryNotWholeMiB));
    }

    #[test]
    fn validate_rejects_a_second_entropy_device() {
        let cfg = config(2, 512 * MIB, vec![Device::Entropy, Device::Entropy]);
        assert_eq!(cfg.validate(4), Err(ConfigError::DuplicateDevice("entropy")));
    }

    #[test]
    fn share_tags_must_be_valid_and_unique() {
        assert_eq!(config(2, 512 * MIB, vec![share("home share")]).validate(4), Err(ConfigError::BadShareTag));
        assert_eq!(config(2, 512 * MIB, vec![share("")]).validate(4), Err(ConfigError::BadShareTag));
        assert_eq!(
            config(2, 512 * MIB, vec![share("projects"), share("projects")]).validate(4),
            Err(ConfigError::DuplicateShareTag)
        );
        assert_eq!(config(2, 512 * MIB, vec![share("home-1_x")]).validate(4), Ok(()));
    }

    #[test]
    fn storage_attachment_takes_read_only_from_the_image() {
        let image = create_raw(1 * MIB, BlockSize::Bytes512).unwrap();
        assert_eq!(image.block_count, 2048);
        let att = StorageAttachment::from_image(image, CachingMode::Cached, SyncMode::Full);
        assert!(!att.read_only());
        assert_eq!(att.caching, CachingMode::Cached);
        assert_eq!(att.sync, SyncMode::Full);

        let mut ro = image;
        ro.read_only = true;
        assert!(StorageAttachment::from_image(ro, CachingMode::Automatic, SyncMode::Unsynced).read_only());
    }
}
