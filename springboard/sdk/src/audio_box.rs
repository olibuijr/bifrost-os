//! Audio boxes (`AudioHardwareBox`): a physical enclosure that holds one or more audio devices, with the state an
//! app reads and asks for (enabled, acquired by a client). As a pure model: the sound server owns the hardware, so
//! `enable` and `disable` here only record the requested state and the rules that guard it. Hardware is not touched.
//! The registry refuses a change that the reference platform refuses: a box cannot be disabled while a client holds
//! it, and a client cannot acquire a disabled box or release one it does not hold.

/// One box and the device uids it contains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioBox {
    pub uid: String,
    pub name: String,
    pub manufacturer: String,
    pub enabled: bool,
    pub acquired: bool,
    pub devices: Vec<String>,
}

impl AudioBox {
    /// A new box. It starts enabled and not acquired, the state the hardware reports at attach.
    pub fn new(uid: &str, name: &str, manufacturer: &str) -> Self {
        AudioBox {
            uid: uid.into(),
            name: name.into(),
            manufacturer: manufacturer.into(),
            enabled: true,
            acquired: false,
            devices: Vec::new(),
        }
    }
}

/// The boxes known to the shell, in attach order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoxRegistry {
    boxes: Vec<AudioBox>,
}

impl BoxRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn boxes(&self) -> &[AudioBox] {
        &self.boxes
    }

    pub fn get(&self, uid: &str) -> Option<&AudioBox> {
        self.boxes.iter().find(|b| b.uid == uid)
    }

    /// Adds a box. The uid must not be empty and must not already be present.
    pub fn add(&mut self, b: AudioBox) -> Result<(), String> {
        if b.uid.is_empty() {
            return Err("the box has no uid".into());
        }
        if self.get(&b.uid).is_some() {
            return Err(format!("box {} is already registered", b.uid));
        }
        self.boxes.push(b);
        Ok(())
    }

    /// Puts a device under a box. A device belongs to at most one box.
    pub fn add_device(&mut self, box_uid: &str, device_uid: &str) -> Result<(), String> {
        if device_uid.is_empty() {
            return Err("the device has no uid".into());
        }
        if let Some(owner) = self.boxes.iter().find(|b| b.devices.iter().any(|d| d == device_uid)) {
            return Err(format!("device {device_uid} already belongs to box {}", owner.uid));
        }
        let b = self.find_mut(box_uid)?;
        b.devices.push(device_uid.into());
        Ok(())
    }

    /// Records whether the box is enabled. Disabling an acquired box is refused.
    pub fn set_enabled(&mut self, box_uid: &str, enabled: bool) -> Result<(), String> {
        let b = self.find_mut(box_uid)?;
        if !enabled && b.acquired {
            return Err(format!("box {box_uid} is acquired and cannot be disabled"));
        }
        b.enabled = enabled;
        Ok(())
    }

    /// Takes the box for a client. The box must be enabled and not already acquired.
    pub fn acquire(&mut self, box_uid: &str) -> Result<(), String> {
        let b = self.find_mut(box_uid)?;
        if !b.enabled {
            return Err(format!("box {box_uid} is disabled"));
        }
        if b.acquired {
            return Err(format!("box {box_uid} is already acquired"));
        }
        b.acquired = true;
        Ok(())
    }

    /// Gives the box back. Releasing a box that is not acquired is refused.
    pub fn release(&mut self, box_uid: &str) -> Result<(), String> {
        let b = self.find_mut(box_uid)?;
        if !b.acquired {
            return Err(format!("box {box_uid} is not acquired"));
        }
        b.acquired = false;
        Ok(())
    }

    fn find_mut(&mut self, uid: &str) -> Result<&mut AudioBox, String> {
        self.boxes
            .iter_mut()
            .find(|b| b.uid == uid)
            .ok_or_else(|| format!("no box {uid}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> BoxRegistry {
        let mut r = BoxRegistry::new();
        r.add(AudioBox::new("dock-1", "Dock", "Acme")).unwrap();
        r.add_device("dock-1", "dock-mic").unwrap();
        r.add_device("dock-1", "dock-out").unwrap();
        r
    }

    #[test]
    fn a_new_box_is_enabled_and_free() {
        let b = AudioBox::new("b", "Name", "Maker");
        assert!(b.enabled && !b.acquired && b.devices.is_empty());
    }

    #[test]
    fn boxes_and_devices_must_be_unique() {
        let mut r = registry();
        assert!(r.add(AudioBox::new("dock-1", "Again", "Acme")).is_err());
        assert!(r.add(AudioBox::new("", "No uid", "Acme")).is_err());
        assert!(r.add_device("dock-1", "dock-mic").is_err());
        assert!(r.add_device("dock-1", "").is_err());
        assert!(r.add_device("missing", "x").is_err());
        assert_eq!(r.get("dock-1").unwrap().devices.len(), 2);
    }

    #[test]
    fn an_acquired_box_cannot_be_disabled_until_released() {
        let mut r = registry();
        r.acquire("dock-1").unwrap();
        assert!(r.set_enabled("dock-1", false).is_err());
        assert!(r.get("dock-1").unwrap().enabled);
        r.release("dock-1").unwrap();
        r.set_enabled("dock-1", false).unwrap();
        assert!(!r.get("dock-1").unwrap().enabled);
    }

    #[test]
    fn a_disabled_box_cannot_be_acquired() {
        let mut r = registry();
        r.set_enabled("dock-1", false).unwrap();
        assert!(r.acquire("dock-1").is_err());
        r.set_enabled("dock-1", true).unwrap();
        r.acquire("dock-1").unwrap();
        assert!(r.acquire("dock-1").is_err());
    }

    #[test]
    fn release_needs_a_matching_acquire() {
        let mut r = registry();
        assert!(r.release("dock-1").is_err());
        r.acquire("dock-1").unwrap();
        r.release("dock-1").unwrap();
        assert!(!r.get("dock-1").unwrap().acquired);
    }

    #[test]
    fn unknown_boxes_are_reported() {
        let mut r = registry();
        assert!(r.acquire("nope").is_err());
        assert!(r.set_enabled("nope", true).is_err());
        assert!(r.get("nope").is_none());
    }
}
