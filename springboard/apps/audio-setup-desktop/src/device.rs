//! The server side of the window: connects through the HAL client, reads every device's properties into `DeviceInfo`,
//! and sends the create, edit and remove requests. Nothing here draws; a failure becomes a sentence for the window.

use crate::draft::{composition_address, remove_address, Draft};
use crate::model::{DeviceInfo, Kind};
use bifrost_audio_hal::{Device, System};
use bifrost_audio_types::props::{
    BUFFER_FRAMES, BUFFER_FRAMES_RANGE, DEVICE_UID, IS_ALIVE, LATENCY, MANUFACTURER, NAME,
    NOMINAL_RATE, SAFETY_OFFSET, TRANSPORT, VIRTUAL_FORMAT, VOLUME_SCALAR,
};
use bifrost_audio_types::{AudioError, FourCC, ObjectId, PropertyAddress, PropertyValue, Scope, StreamFormat, SYSTEM_OBJECT};

/// The address of a device's scoped property (main element).
fn scoped(selector: FourCC, scope: Scope) -> PropertyAddress {
    PropertyAddress::scoped(selector, scope)
}

/// The server connection and the last list it gave. `None` means the server is not reachable (yet).
pub struct Session {
    system: Option<System>,
}

impl Session {
    pub fn new() -> Session {
        Session { system: None }
    }

    /// Connects when there is no connection yet. The app id is the name the server shows in its client list.
    pub fn connect(&mut self) -> Result<(), String> {
        if self.system.is_some() {
            return Ok(());
        }
        match System::connect("is.olibuijr.audiosetup.desktop") {
            Ok(system) => {
                self.system = Some(system);
                Ok(())
            }
            Err(e) => {
                self.system = None;
                Err(describe(e))
            }
        }
    }

    /// Every device the server knows, read into `DeviceInfo`. A property the server does not answer keeps its default.
    pub fn devices(&mut self) -> Result<Vec<DeviceInfo>, String> {
        self.connect()?;
        let Some(system) = self.system.as_ref() else { return Err(describe(AudioError::NoServer)) };
        let list = system.devices().map_err(describe)?;
        Ok(list.iter().map(read_device).collect())
    }

    /// The members and clock of an aggregate, read from its composition property.
    pub fn composition(&mut self, object: u32) -> Result<Draft, String> {
        self.connect()?;
        let system = self.system.as_ref().ok_or_else(|| describe(AudioError::NoServer))?;
        let dev = Device { id: ObjectId(object), system: system.clone() };
        match dev.get(composition_address()).map_err(describe)? {
            PropertyValue::String(text) => Draft::decode(&text).ok_or_else(|| "The device's composition could not be read.".to_string()),
            _ => Err("The device's composition could not be read.".to_string()),
        }
    }

    /// Creates a device (a draft without UID) or replaces the one the draft names.
    pub fn save(&mut self, draft: &Draft) -> Result<(), String> {
        self.connect()?;
        let system = self.system.as_ref().ok_or_else(|| describe(AudioError::NoServer))?;
        system
            .set(SYSTEM_OBJECT, composition_address(), PropertyValue::String(draft.encode()))
            .map_err(describe)
    }

    /// Removes a made device.
    pub fn remove(&mut self, object: u32) -> Result<(), String> {
        self.connect()?;
        let system = self.system.as_ref().ok_or_else(|| describe(AudioError::NoServer))?;
        let dev = Device { id: ObjectId(object), system: system.clone() };
        dev.set(remove_address(), PropertyValue::Bool(true)).map_err(describe)
    }

    /// Forgets the connection after the server went away, so the next call connects again.
    pub fn drop_connection(&mut self) {
        self.system = None;
    }
}

/// Reads one device's properties. Each read that fails leaves its field at the default.
fn read_device(dev: &Device) -> DeviceInfo {
    let object = dev.id.0;
    let uid = text(dev, PropertyAddress::global(DEVICE_UID));
    let name = text(dev, PropertyAddress::global(NAME));
    let mut info = DeviceInfo::new(object, &uid, if name.is_empty() { &uid } else { &name });
    info.manufacturer = text(dev, PropertyAddress::global(MANUFACTURER));
    info.transport = text(dev, PropertyAddress::global(TRANSPORT));
    info.kind = Kind::from_transport(&info.transport);
    info.alive = matches!(dev.get(PropertyAddress::global(IS_ALIVE)), Ok(PropertyValue::Bool(true)));
    info.rate = match dev.get(PropertyAddress::global(NOMINAL_RATE)) {
        Ok(PropertyValue::F64(r)) => r,
        Ok(PropertyValue::F32(r)) => f64::from(r),
        _ => 0.0,
    };
    info.output_format = format(dev, Scope::Output);
    info.output_channels = info.output_format.map_or(0, |f| f.channels);
    info.input_channels = format(dev, Scope::Input).map_or(0, |f| f.channels);
    info.buffer_frames = match dev.get(PropertyAddress::global(BUFFER_FRAMES)) {
        Ok(PropertyValue::U32(n)) => n,
        _ => 0,
    };
    info.buffer_range = match dev.get(PropertyAddress::global(BUFFER_FRAMES_RANGE)) {
        Ok(PropertyValue::RangeU32(lo, hi)) => (lo, hi),
        _ => (0, 0),
    };
    info.latency_frames = u32_of(dev, scoped(LATENCY, Scope::Output));
    info.safety_frames = u32_of(dev, scoped(SAFETY_OFFSET, Scope::Output));
    info.volume = match dev.get(scoped(VOLUME_SCALAR, Scope::Output)) {
        Ok(PropertyValue::F32(v)) => Some(v),
        _ => None,
    };
    info
}

fn text(dev: &Device, address: PropertyAddress) -> String {
    match dev.get(address) {
        Ok(PropertyValue::String(s)) => s,
        _ => String::new(),
    }
}

fn u32_of(dev: &Device, address: PropertyAddress) -> u32 {
    match dev.get(address) {
        Ok(PropertyValue::U32(n)) => n,
        _ => 0,
    }
}

fn format(dev: &Device, scope: Scope) -> Option<StreamFormat> {
    match dev.get(scoped(VIRTUAL_FORMAT, scope)) {
        Ok(PropertyValue::Format(f)) => Some(f),
        _ => None,
    }
}

/// The sentence the window shows for a failed request.
pub fn describe(e: AudioError) -> String {
    match e {
        AudioError::NoServer => "The audio server is not running.".to_string(),
        AudioError::UnknownObject | AudioError::UnknownProperty => "That device is no longer available.".to_string(),
        AudioError::NotSettable => "That device cannot be changed.".to_string(),
        AudioError::BadValueType | AudioError::BadValue => "The server refused those settings.".to_string(),
        AudioError::Hogged => "Another app has this device.".to_string(),
        AudioError::PermissionDenied => "This app is not allowed to change audio devices.".to_string(),
        AudioError::DeviceGone => "The device was unplugged.".to_string(),
        AudioError::Protocol => "The audio server sent something this app does not understand.".to_string(),
        AudioError::Io => "The audio device failed.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_read_as_sentences() {
        assert_eq!(describe(AudioError::NoServer), "The audio server is not running.");
        assert_eq!(describe(AudioError::Hogged), "Another app has this device.");
        assert_eq!(describe(AudioError::Io), "The audio device failed.");
    }

    #[test]
    fn the_request_selectors_are_the_contract_ones() {
        assert_eq!(composition_address().selector.0, *b"agcm");
        assert_eq!(remove_address().selector.0, *b"agrm");
    }
}
