//! Default audio devices: the device a system uses for input, for output and for sound effects, chosen by the user
//! or left automatic. A chosen device stays chosen while it is away, so it returns as the default when it comes
//! back. While it is away, or when nothing was chosen, the default is the first device in list order that can take
//! the role. Hidden devices never take a role. Pure: no device is opened and no sound is routed here.

/// What a default device is used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Input,
    Output,
    SoundEffects,
}

/// One audio device as the system lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub can_input: bool,
    pub can_output: bool,
    pub can_sound_effects: bool,
    /// A hidden device is not in the normal list and cannot be a default.
    pub hidden: bool,
}

impl Device {
    /// A visible device that can take every role. Narrow it with the fields for a partial device.
    pub fn new(id: &str, name: &str) -> Self {
        Device {
            id: id.into(),
            name: name.into(),
            can_input: true,
            can_output: true,
            can_sound_effects: true,
            hidden: false,
        }
    }

    /// Whether this device may take `role`.
    pub fn can_take(&self, role: Role) -> bool {
        !self.hidden
            && match role {
                Role::Input => self.can_input,
                Role::Output => self.can_output,
                Role::SoundEffects => self.can_sound_effects,
            }
    }
}

/// Why a device list change was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceError {
    /// The identifier is empty.
    Empty,
    /// A device with this identifier is already listed.
    Duplicate,
    /// No device with this identifier is listed.
    NotFound,
    /// The device exists but cannot take the role (wrong direction, or hidden).
    NotEligible,
}

fn slot(role: Role) -> usize {
    match role {
        Role::Input => 0,
        Role::Output => 1,
        Role::SoundEffects => 2,
    }
}

/// The device list and the chosen default for each role.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Defaults {
    devices: Vec<Device>,
    chosen: [Option<String>; 3],
}

impl Defaults {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// Lists a device. Refuses an empty or already listed identifier.
    pub fn add(&mut self, device: Device) -> Result<(), DeviceError> {
        if device.id.is_empty() {
            return Err(DeviceError::Empty);
        }
        if self.devices.iter().any(|d| d.id == device.id) {
            return Err(DeviceError::Duplicate);
        }
        self.devices.push(device);
        Ok(())
    }

    /// Unlists a device. The choice for each role is kept, so the device is the default again when re-added.
    pub fn remove(&mut self, id: &str) -> Result<Device, DeviceError> {
        let at = self.devices.iter().position(|d| d.id == id).ok_or(DeviceError::NotFound)?;
        Ok(self.devices.remove(at))
    }

    /// Chooses `id` as the default for `role`.
    pub fn set_default(&mut self, role: Role, id: &str) -> Result<(), DeviceError> {
        let device = self.devices.iter().find(|d| d.id == id).ok_or(DeviceError::NotFound)?;
        if !device.can_take(role) {
            return Err(DeviceError::NotEligible);
        }
        self.chosen[slot(role)] = Some(id.into());
        Ok(())
    }

    /// Returns `role` to automatic choice.
    pub fn clear_default(&mut self, role: Role) {
        self.chosen[slot(role)] = None;
    }

    /// The default for `role`: the chosen device when it is listed and may take the role, else the first device in
    /// list order that may take it. None when no device can take the role.
    pub fn default_for(&self, role: Role) -> Option<&Device> {
        let chosen = self.chosen[slot(role)].as_deref();
        chosen
            .and_then(|id| self.devices.iter().find(|d| d.id == id && d.can_take(role)))
            .or_else(|| self.devices.iter().find(|d| d.can_take(role)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speakers() -> Device {
        Device::new("spk", "Speakers")
    }

    #[test]
    fn add_refuses_empty_and_duplicate_ids() {
        let mut d = Defaults::new();
        assert_eq!(d.add(Device::new("", "Nameless")), Err(DeviceError::Empty));
        assert_eq!(d.add(speakers()), Ok(()));
        assert_eq!(d.add(Device::new("spk", "Again")), Err(DeviceError::Duplicate));
        assert_eq!(d.devices().len(), 1);
    }

    #[test]
    fn automatic_default_is_the_first_eligible_device() {
        let mut d = Defaults::new();
        let mut mic = Device::new("mic", "Microphone");
        mic.can_output = false;
        let mut hidden = Device::new("hid", "Hidden");
        hidden.hidden = true;
        d.add(mic).unwrap();
        d.add(hidden).unwrap();
        d.add(speakers()).unwrap();
        assert_eq!(d.default_for(Role::Output).map(|x| x.id.as_str()), Some("spk"));
        assert_eq!(d.default_for(Role::Input).map(|x| x.id.as_str()), Some("mic"));
    }

    #[test]
    fn set_default_refuses_hidden_wrong_role_and_unknown_devices() {
        let mut d = Defaults::new();
        let mut mic = Device::new("mic", "Microphone");
        mic.can_output = false;
        let mut hidden = Device::new("hid", "Hidden");
        hidden.hidden = true;
        d.add(mic).unwrap();
        d.add(hidden).unwrap();
        assert_eq!(d.set_default(Role::Output, "mic"), Err(DeviceError::NotEligible));
        assert_eq!(d.set_default(Role::Output, "hid"), Err(DeviceError::NotEligible));
        assert_eq!(d.set_default(Role::Output, "nope"), Err(DeviceError::NotFound));
        assert_eq!(d.default_for(Role::Output), None);
    }

    #[test]
    fn chosen_default_survives_removal_and_returns_when_re_added() {
        let mut d = Defaults::new();
        d.add(speakers()).unwrap();
        d.add(Device::new("usb", "USB headset")).unwrap();
        d.set_default(Role::Output, "usb").unwrap();
        assert_eq!(d.default_for(Role::Output).map(|x| x.id.as_str()), Some("usb"));
        d.remove("usb").unwrap();
        assert_eq!(d.default_for(Role::Output).map(|x| x.id.as_str()), Some("spk"));
        d.add(Device::new("usb", "USB headset")).unwrap();
        assert_eq!(d.default_for(Role::Output).map(|x| x.id.as_str()), Some("usb"));
    }

    #[test]
    fn clear_default_returns_to_automatic_and_roles_are_independent() {
        let mut d = Defaults::new();
        d.add(speakers()).unwrap();
        d.add(Device::new("usb", "USB headset")).unwrap();
        d.set_default(Role::Output, "usb").unwrap();
        d.set_default(Role::SoundEffects, "spk").unwrap();
        assert_eq!(d.default_for(Role::Input).map(|x| x.id.as_str()), Some("spk"));
        assert_eq!(d.default_for(Role::SoundEffects).map(|x| x.id.as_str()), Some("spk"));
        d.clear_default(Role::Output);
        assert_eq!(d.default_for(Role::Output).map(|x| x.id.as_str()), Some("spk"));
        assert_eq!(d.remove("ghost"), Err(DeviceError::NotFound));
    }
}
