//! Audio hardware objects for apps: the system, the devices the kernel exposes and the streams each device has
//! (system, device and stream parity for audio hardware). The system reads the card list
//! from the kernel's PCM listing (`/proc/asound/pcm`) and gives every object a stable id, so an app can name a device
//! by id and ask for its streams. Controls and clocks are not modelled: the mixer controls belong to the audio driver
//! layer (`bifrost_audio_alsa::ctl`) and no app reads them yet.
//!
//! Rules of this model:
//! - The system is object 1. Each device, then each of its streams, takes the next id in listing order.
//! - A device has an output stream when its listing names playback substreams, and an input stream when it names
//!   capture substreams. A device with neither is kept and has no streams.
//! - A listing line that does not match the format is skipped.

/// An object id, unique within one [`System`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(pub u32);

/// The id of the system object.
pub const SYSTEM: ObjectId = ObjectId(1);

/// What an object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    System,
    Device,
    Stream,
}

/// The direction of a stream: an output stream plays, an input stream records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Input,
    Output,
}

/// One stream of a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stream {
    pub id: ObjectId,
    pub scope: Scope,
    /// Substreams of the listing, as the kernel counts them.
    pub substreams: u32,
}

/// One device, as the listing names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: ObjectId,
    pub card: u32,
    pub device: u32,
    /// The driver's identifier for the device.
    pub identifier: String,
    /// The name the listing gives the device.
    pub name: String,
    pub streams: Vec<Stream>,
}

impl Device {
    /// The device's address in the card layer, for example `hw:0,3`.
    pub fn uid(&self) -> String {
        format!("hw:{},{}", self.card, self.device)
    }

    /// The stream of one direction, if the device has one.
    pub fn stream(&self, scope: Scope) -> Option<&Stream> {
        self.streams.iter().find(|s| s.scope == scope)
    }
}

/// The audio system: its devices in listing order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct System {
    devices: Vec<Device>,
}

/// One parsed listing line, before ids are assigned.
struct Listed {
    card: u32,
    device: u32,
    identifier: String,
    name: String,
    playback: u32,
    capture: u32,
}

/// Parses one line of the form `00-03: identifier : name : playback 1 : capture 1`. Fields other than the two
/// substream counts are ignored.
fn parse_line(line: &str) -> Option<Listed> {
    let mut fields = line.split(" : ");
    let (address, identifier) = fields.next()?.split_once(": ")?;
    let (card, device) = address.split_once('-')?;
    let name = fields.next()?;
    let (mut playback, mut capture) = (0, 0);
    for field in fields {
        if let Some(count) = field.strip_prefix("playback ") {
            playback = count.trim().parse().ok()?;
        } else if let Some(count) = field.strip_prefix("capture ") {
            capture = count.trim().parse().ok()?;
        }
    }
    Some(Listed {
        card: card.parse().ok()?,
        device: device.parse().ok()?,
        identifier: identifier.to_string(),
        name: name.to_string(),
        playback,
        capture,
    })
}

impl System {
    /// Builds the system from the text of a PCM listing. Malformed lines are skipped.
    pub fn from_listing(text: &str) -> System {
        let mut next = SYSTEM.0 + 1;
        let mut take_id = || {
            let id = ObjectId(next);
            next += 1;
            id
        };
        let mut devices = Vec::new();
        for listed in text.lines().filter_map(parse_line) {
            let id = take_id();
            let mut streams = Vec::new();
            if listed.playback > 0 {
                streams.push(Stream { id: take_id(), scope: Scope::Output, substreams: listed.playback });
            }
            if listed.capture > 0 {
                streams.push(Stream { id: take_id(), scope: Scope::Input, substreams: listed.capture });
            }
            devices.push(Device {
                id,
                card: listed.card,
                device: listed.device,
                identifier: listed.identifier,
                name: listed.name,
                streams,
            });
        }
        System { devices }
    }

    /// The devices, in listing order.
    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// The device with this id.
    pub fn device(&self, id: ObjectId) -> Option<&Device> {
        self.devices.iter().find(|d| d.id == id)
    }

    /// The first device with a stream in this direction: the default for that direction.
    pub fn default_device(&self, scope: Scope) -> Option<&Device> {
        self.devices.iter().find(|d| d.stream(scope).is_some())
    }

    /// The device that owns a stream.
    pub fn owner_of(&self, stream: ObjectId) -> Option<ObjectId> {
        self.devices.iter().find(|d| d.streams.iter().any(|s| s.id == stream)).map(|d| d.id)
    }

    /// What the object with this id is, or `None` when the system has no such object.
    pub fn kind_of(&self, id: ObjectId) -> Option<ObjectKind> {
        if id == SYSTEM {
            return Some(ObjectKind::System);
        }
        if self.device(id).is_some() {
            return Some(ObjectKind::Device);
        }
        self.devices.iter().any(|d| d.streams.iter().any(|s| s.id == id)).then_some(ObjectKind::Stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = "00-00: ALC Analog : ALC Analog : playback 1 : capture 1\n\
                           00-03: HDMI 0 : HDMI 0 : playback 1\n\
                           01-00: USB Audio : USB Audio : capture 2\n\
                           not a listing line\n\
                           00-07: Odd : Odd\n";

    #[test]
    fn devices_and_streams_take_ids_in_listing_order() {
        let system = System::from_listing(LISTING);
        let devices = system.devices();
        assert_eq!(devices.len(), 4, "the malformed line is skipped, the device without streams is kept");
        assert_eq!(devices[0].id, ObjectId(2));
        assert_eq!(devices[0].stream(Scope::Output).map(|s| s.id), Some(ObjectId(3)));
        assert_eq!(devices[0].stream(Scope::Input).map(|s| s.id), Some(ObjectId(4)));
        assert_eq!(devices[1].id, ObjectId(5));
        assert_eq!(devices[2].stream(Scope::Input).map(|s| s.substreams), Some(2));
        assert!(devices[3].streams.is_empty());
    }

    #[test]
    fn a_device_is_addressed_by_its_card_and_device_numbers() {
        let system = System::from_listing(LISTING);
        assert_eq!(system.device(ObjectId(5)).map(Device::uid), Some("hw:0,3".to_string()));
        assert_eq!(system.device(ObjectId(7)).map(|d| d.name.as_str()), Some("USB Audio"));
        assert!(system.device(ObjectId(99)).is_none());
    }

    #[test]
    fn the_default_device_is_the_first_with_a_stream_in_that_direction() {
        let system = System::from_listing(LISTING);
        assert_eq!(system.default_device(Scope::Output).map(|d| d.id), Some(ObjectId(2)));
        let capture_only = System::from_listing("01-00: USB Audio : USB Audio : capture 1\n");
        assert_eq!(capture_only.default_device(Scope::Input).map(|d| d.id), Some(ObjectId(2)));
        assert!(capture_only.default_device(Scope::Output).is_none());
    }

    #[test]
    fn objects_know_their_kind_and_a_stream_knows_its_owner() {
        let system = System::from_listing(LISTING);
        assert_eq!(system.kind_of(SYSTEM), Some(ObjectKind::System));
        assert_eq!(system.kind_of(ObjectId(5)), Some(ObjectKind::Device));
        assert_eq!(system.kind_of(ObjectId(6)), Some(ObjectKind::Stream));
        assert_eq!(system.kind_of(ObjectId(99)), None);
        assert_eq!(system.owner_of(ObjectId(6)), Some(ObjectId(5)));
        assert_eq!(system.owner_of(ObjectId(5)), None);
    }

    #[test]
    fn an_empty_listing_is_an_empty_system() {
        let system = System::from_listing("");
        assert!(system.devices().is_empty());
        assert_eq!(system.kind_of(SYSTEM), Some(ObjectKind::System));
    }
}
