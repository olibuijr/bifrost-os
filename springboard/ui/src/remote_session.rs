//! Remote media sessions: a Now Playing session whose playback runs on another device. The session holds the attributes a
//! remote control shows (title and the app that owns the session), the devices it may use and the device it is
//! connected to. This model checks connecting and disconnecting; the transport that moves the media is outside it.

/// A device a remote session can play on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaDevice {
    pub id: String,
    pub name: String,
}

/// Why a remote session step was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteSessionError {
    /// A session needs a title and an app identifier.
    Incomplete,
    /// The device is not in the session's device list.
    UnknownDevice,
    /// There is no connected device to disconnect.
    NotConnected,
}

/// A session whose playback can move between devices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteMediaSession {
    pub app_id: String,
    pub title: String,
    devices: Vec<MediaDevice>,
    connected: Option<String>,
}

impl RemoteMediaSession {
    /// Starts a session with no devices. The app identifier and the title are trimmed and must not be empty.
    pub fn new(app_id: &str, title: &str) -> Result<Self, RemoteSessionError> {
        let (app_id, title) = (app_id.trim(), title.trim());
        if app_id.is_empty() || title.is_empty() {
            return Err(RemoteSessionError::Incomplete);
        }
        Ok(RemoteMediaSession { app_id: app_id.into(), title: title.into(), devices: Vec::new(), connected: None })
    }

    /// Replaces the device list. A device with an empty or repeated identifier is dropped (the first one wins). When
    /// the connected device is no longer listed, the session is disconnected.
    pub fn set_devices(&mut self, devices: Vec<MediaDevice>) {
        let mut unique: Vec<MediaDevice> = Vec::new();
        for device in devices {
            if !device.id.is_empty() && !unique.iter().any(|u| u.id == device.id) {
                unique.push(device);
            }
        }
        if let Some(id) = &self.connected {
            if !unique.iter().any(|d| &d.id == id) {
                self.connected = None;
            }
        }
        self.devices = unique;
    }

    pub fn devices(&self) -> &[MediaDevice] {
        &self.devices
    }

    /// Connects playback to a listed device. A session plays on one device at a time, so this moves it from the device
    /// it was on.
    pub fn connect(&mut self, id: &str) -> Result<&MediaDevice, RemoteSessionError> {
        let index = self.devices.iter().position(|d| d.id == id).ok_or(RemoteSessionError::UnknownDevice)?;
        self.connected = Some(self.devices[index].id.clone());
        Ok(&self.devices[index])
    }

    /// Ends the connection and returns the device it was on.
    pub fn disconnect(&mut self) -> Result<MediaDevice, RemoteSessionError> {
        let id = self.connected.take().ok_or(RemoteSessionError::NotConnected)?;
        self.devices.iter().find(|d| d.id == id).cloned().ok_or(RemoteSessionError::NotConnected)
    }

    /// The device playback is connected to, if any.
    pub fn connected(&self) -> Option<&MediaDevice> {
        let id = self.connected.as_deref()?;
        self.devices.iter().find(|d| d.id == id)
    }
}
