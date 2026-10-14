//! Virtual audio devices (what an audio server driver plug-in would publish), as a pure model. The reference platform
//! loads a user-space plug-in that adds devices to the HAL. Here the sound server owns the hardware, so a virtual
//! device is only a description: its name, uid, channel count and sample rate, checked the way the HAL checks a
//! plug-in's device before it publishes it. Nothing in this module creates a device or moves audio. The driver
//! status records that no plug-in loader exists, so an app can tell a refused device from a missing loader.
use bifrost_audio_types::StreamFormat;

/// The sample rates a virtual device may run at, in hertz.
pub const SUPPORTED_RATES: [f64; 11] = [
    8_000.0, 11_025.0, 16_000.0, 22_050.0, 32_000.0, 44_100.0, 48_000.0, 88_200.0, 96_000.0, 176_400.0, 192_000.0,
];

/// Most channels one virtual device may carry.
pub const MAX_CHANNELS: u32 = 64;

/// A device a plug-in would publish.
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualDevice {
    pub uid: String,
    pub name: String,
    pub channels: u32,
    pub sample_rate: f64,
}

impl VirtualDevice {
    /// Checks the description the way the HAL checks a published device.
    pub fn validate(&self) -> Result<(), String> {
        if self.uid.is_empty() {
            return Err("the virtual device has no uid".into());
        }
        if self.name.is_empty() {
            return Err("the virtual device has no name".into());
        }
        if self.channels == 0 || self.channels > MAX_CHANNELS {
            return Err(format!("{} channels is outside 1 to {MAX_CHANNELS}", self.channels));
        }
        if !SUPPORTED_RATES.contains(&self.sample_rate) {
            return Err(format!("sample rate {} is not supported", self.sample_rate));
        }
        Ok(())
    }

    /// The stream format of the device: 32-bit float at the device rate and channel count, the server mix format.
    pub fn stream_format(&self) -> StreamFormat {
        StreamFormat::float32(self.sample_rate, self.channels)
    }

    /// Reads a plug-in's device description: one `key=value` line each for `uid`, `name`, `channels` and `sample_rate`.
    /// Blank lines and lines that start with `#` are skipped. A key that is missing, repeated or unknown is refused, and
    /// the values are checked by [`VirtualDevice::validate`].
    pub fn from_descriptor(text: &str) -> Result<Self, String> {
        let (mut uid, mut name, mut channels, mut sample_rate) = (None, None, None, None);
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let (key, value) = line.split_once('=').ok_or_else(|| format!("'{line}' is not a key=value line"))?;
            let (key, value) = (key.trim(), value.trim());
            let slot = match key {
                "uid" => &mut uid,
                "name" => &mut name,
                "channels" => &mut channels,
                "sample_rate" => &mut sample_rate,
                other => return Err(format!("unknown key '{other}' in a virtual device description")),
            };
            if slot.is_some() {
                return Err(format!("key '{key}' is given twice"));
            }
            *slot = Some(value.to_string());
        }
        let missing = |key: &str| format!("the virtual device description has no '{key}'");
        let device = VirtualDevice {
            uid: uid.ok_or_else(|| missing("uid"))?,
            name: name.ok_or_else(|| missing("name"))?,
            channels: channels.ok_or_else(|| missing("channels"))?.parse().map_err(|_| "channels is not a whole number".to_string())?,
            sample_rate: sample_rate.ok_or_else(|| missing("sample_rate"))?.parse().map_err(|_| "sample_rate is not a number".to_string())?,
        };
        device.validate()?;
        Ok(device)
    }

    /// The description that [`VirtualDevice::from_descriptor`] reads back to the same device.
    pub fn to_descriptor(&self) -> String {
        format!("uid={}\nname={}\nchannels={}\nsample_rate={}\n", self.uid, self.name, self.channels, self.sample_rate)
    }
}

/// A set of virtual devices a plug-in publishes together. Uids must be unique within the set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VirtualDeviceSet {
    devices: Vec<VirtualDevice>,
}

impl VirtualDeviceSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn devices(&self) -> &[VirtualDevice] {
        &self.devices
    }

    /// Adds a device after it validates and if its uid is not already in the set.
    pub fn publish(&mut self, device: VirtualDevice) -> Result<(), String> {
        device.validate()?;
        if self.devices.iter().any(|d| d.uid == device.uid) {
            return Err(format!("virtual device {} is already published", device.uid));
        }
        self.devices.push(device);
        Ok(())
    }

    /// Reads one description per device (see [`VirtualDevice::from_descriptor`]) and publishes them in order. The first
    /// refused description stops the load, and its message names its position.
    pub fn load(descriptions: &[&str]) -> Result<Self, String> {
        let mut set = VirtualDeviceSet::new();
        for (index, text) in descriptions.iter().enumerate() {
            let position = index + 1;
            let device = VirtualDevice::from_descriptor(text).map_err(|e| format!("device {position}: {e}"))?;
            set.publish(device).map_err(|e| format!("device {position}: {e}"))?;
        }
        Ok(set)
    }
}

/// The state of plug-in loading on this platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverStatus {
    /// No user-space plug-in loader is present, so nothing can publish devices.
    NoLoader,
}

impl DriverStatus {
    /// The status that applies here. The sound server owns the hardware and there is no loader.
    pub fn current() -> Self {
        DriverStatus::NoLoader
    }

    /// Whether a device set can reach the HAL. It cannot while no loader is present.
    pub fn can_publish(self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(uid: &str) -> VirtualDevice {
        VirtualDevice { uid: uid.into(), name: "Loopback".into(), channels: 2, sample_rate: 48_000.0 }
    }

    #[test]
    fn a_stereo_48k_device_validates() {
        assert!(device("loop-1").validate().is_ok());
    }

    #[test]
    fn invalid_descriptions_are_refused() {
        let mut d = device("loop-1");
        d.uid.clear();
        assert!(d.validate().is_err());
        let mut d = device("loop-1");
        d.name.clear();
        assert!(d.validate().is_err());
        let mut d = device("loop-1");
        d.channels = 0;
        assert!(d.validate().is_err());
        d.channels = MAX_CHANNELS + 1;
        assert!(d.validate().is_err());
        let mut d = device("loop-1");
        d.sample_rate = 47_000.0;
        assert!(d.validate().is_err());
    }

    #[test]
    fn the_stream_format_is_float32_at_the_device_rate_and_channels() {
        let f = device("loop-1").stream_format();
        assert!(f.is_float());
        assert_eq!(f.sample_rate, 48_000.0);
        assert_eq!(f.channels, 2);
    }

    #[test]
    fn a_set_refuses_duplicate_uids_and_keeps_order() {
        let mut s = VirtualDeviceSet::new();
        s.publish(device("a")).unwrap();
        s.publish(device("b")).unwrap();
        assert!(s.publish(device("a")).is_err());
        assert!(s.publish(VirtualDevice { sample_rate: 1.0, ..device("c") }).is_err());
        let uids: Vec<&str> = s.devices().iter().map(|d| d.uid.as_str()).collect();
        assert_eq!(uids, ["a", "b"]);
    }

    #[test]
    fn without_a_loader_nothing_can_publish() {
        assert_eq!(DriverStatus::current(), DriverStatus::NoLoader);
        assert!(!DriverStatus::current().can_publish());
    }
}
