//! Client side of hog mode: take and release a device through `HOG_PID`. Owner: task 10.

use crate::Device;
use bifrost_audio_types::props::HOG_PID;
use bifrost_audio_types::{AudioError, PropertyAddress, PropertyValue};

fn addr() -> PropertyAddress {
    PropertyAddress::global(HOG_PID)
}

/// The pid hogging `device`, or None.
pub fn hog_owner(device: &Device) -> Result<Option<i32>, AudioError> {
    match device.get(addr())? {
        PropertyValue::I32(-1) => Ok(None),
        PropertyValue::I32(p) => Ok(Some(p)),
        _ => Err(AudioError::BadValueType),
    }
}

/// Takes the device exclusively (the server asks the policy process; `PermissionDenied` or `Hogged` otherwise).
pub fn hog(device: &Device) -> Result<(), AudioError> {
    device.set(addr(), PropertyValue::I32(std::process::id() as i32))
}

/// Releases the device (it is also released when this process exits).
pub fn release(device: &Device) -> Result<(), AudioError> {
    device.set(addr(), PropertyValue::I32(-1))
}

/// Should a client move to another device because `owner` hogs its device?
pub fn must_move(owner: Option<i32>, my_pid: i32) -> bool {
    owner.is_some_and(|p| p != my_pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn move_rule() {
        assert!(!must_move(None, 5));
        assert!(!must_move(Some(5), 5));
        assert!(must_move(Some(6), 5));
    }
}
