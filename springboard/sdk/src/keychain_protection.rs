//! Data protection classes for keychain items (`kSecAttrAccessible*`): when an item can be read, and whether it may
//! leave this device in a backup or a sync.
//!
//! The class is a pure value: [`Protection::readable`] answers whether an item in that class may be read given the
//! device state, and [`Protection::code`] / [`Protection::from_code`] give the one-byte wire value. The shell does not
//! lock its store yet, so the class is not enforced at rest; the keychain queries in [`crate::keychain_query`] do not
//! carry it yet. Until they do, an app that depends on a class must not rely on the store refusing reads.

/// Device state a read is checked against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceState {
    /// The session is unlocked now.
    pub unlocked: bool,
    /// The session has been unlocked at least once since boot.
    pub unlocked_since_boot: bool,
}

/// The protection class of a keychain item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Protection {
    /// Readable only while the session is unlocked.
    WhenUnlocked,
    /// Readable after the first unlock since boot, including while locked. The default.
    AfterFirstUnlock,
    /// Readable at any time, including before the first unlock.
    Always,
    /// As `WhenUnlocked`, and never copied to a backup or another device.
    WhenUnlockedThisDeviceOnly,
    /// As `AfterFirstUnlock`, and never copied to a backup or another device.
    AfterFirstUnlockThisDeviceOnly,
    /// As `Always`, and never copied to a backup or another device.
    AlwaysThisDeviceOnly,
}

impl Default for Protection {
    fn default() -> Self {
        Protection::AfterFirstUnlock
    }
}

impl Protection {
    /// The one-byte wire value stored with an item.
    pub fn code(self) -> u8 {
        match self {
            Protection::WhenUnlocked => 1,
            Protection::AfterFirstUnlock => 2,
            Protection::Always => 3,
            Protection::WhenUnlockedThisDeviceOnly => 4,
            Protection::AfterFirstUnlockThisDeviceOnly => 5,
            Protection::AlwaysThisDeviceOnly => 6,
        }
    }

    /// The class for a wire value, or `None` for an unknown value.
    pub fn from_code(code: u8) -> Option<Protection> {
        match code {
            1 => Some(Protection::WhenUnlocked),
            2 => Some(Protection::AfterFirstUnlock),
            3 => Some(Protection::Always),
            4 => Some(Protection::WhenUnlockedThisDeviceOnly),
            5 => Some(Protection::AfterFirstUnlockThisDeviceOnly),
            6 => Some(Protection::AlwaysThisDeviceOnly),
            _ => None,
        }
    }

    /// True when the item may be copied to a backup or another device.
    pub fn migrates(self) -> bool {
        !matches!(
            self,
            Protection::WhenUnlockedThisDeviceOnly
                | Protection::AfterFirstUnlockThisDeviceOnly
                | Protection::AlwaysThisDeviceOnly
        )
    }

    /// True when an item in this class can be read in `state`.
    pub fn readable(self, state: DeviceState) -> bool {
        match self {
            Protection::WhenUnlocked | Protection::WhenUnlockedThisDeviceOnly => state.unlocked,
            Protection::AfterFirstUnlock | Protection::AfterFirstUnlockThisDeviceOnly => state.unlocked_since_boot,
            Protection::Always | Protection::AlwaysThisDeviceOnly => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Protection; 6] = [
        Protection::WhenUnlocked,
        Protection::AfterFirstUnlock,
        Protection::Always,
        Protection::WhenUnlockedThisDeviceOnly,
        Protection::AfterFirstUnlockThisDeviceOnly,
        Protection::AlwaysThisDeviceOnly,
    ];

    #[test]
    fn codes_round_trip_and_unknown_codes_are_refused() {
        for class in ALL {
            assert_eq!(Protection::from_code(class.code()), Some(class));
        }
        assert_eq!(Protection::from_code(0), None);
        assert_eq!(Protection::from_code(7), None);
    }

    #[test]
    fn the_default_is_available_after_first_unlock() {
        assert_eq!(Protection::default(), Protection::AfterFirstUnlock);
    }

    #[test]
    fn reads_follow_the_device_state() {
        let locked_after_boot = DeviceState { unlocked: false, unlocked_since_boot: true };
        let locked_before_unlock = DeviceState { unlocked: false, unlocked_since_boot: false };
        let open = DeviceState { unlocked: true, unlocked_since_boot: true };
        assert!(!Protection::WhenUnlocked.readable(locked_after_boot));
        assert!(Protection::WhenUnlocked.readable(open));
        assert!(Protection::AfterFirstUnlock.readable(locked_after_boot));
        assert!(!Protection::AfterFirstUnlock.readable(locked_before_unlock));
        assert!(Protection::Always.readable(locked_before_unlock));
        assert!(!Protection::AfterFirstUnlockThisDeviceOnly.readable(locked_before_unlock));
        assert!(Protection::AlwaysThisDeviceOnly.readable(locked_before_unlock));
    }

    #[test]
    fn device_only_classes_do_not_migrate() {
        assert!(Protection::AfterFirstUnlock.migrates());
        assert!(Protection::Always.migrates());
        assert!(!Protection::WhenUnlockedThisDeviceOnly.migrates());
        assert!(!Protection::AfterFirstUnlockThisDeviceOnly.migrates());
        assert!(!Protection::AlwaysThisDeviceOnly.migrates());
    }
}
