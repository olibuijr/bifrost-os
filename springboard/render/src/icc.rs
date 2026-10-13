//! Reads ICC colour profiles and keeps the profile each device uses.
//!
//! `Profile::parse` checks the 128-byte header, the `acsp` magic and the tag table, and rejects any tag whose data
//! would run past the declared size of the profile. `Profile::description` returns the text of the description tag in
//! its version 2 form (`desc`) or its version 4 form (`mluc`, UTF-16BE). `Devices` records the profile of each display,
//! printer, scanner and camera, with an optional custom profile that overrides the default. Profiles are read here,
//! never converted: transforms, rendering intents and black point compensation live in `colorspace`.

use std::collections::BTreeMap;

/// The fixed part of the header, in bytes.
const HEADER_LEN: usize = 128;
/// Where the tag count sits: right after the header.
const TABLE_START: usize = 132;
/// One tag table entry: signature, offset and size, four bytes each.
const TAG_ENTRY_LEN: usize = 12;

/// Why a profile was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IccError {
    /// The data is shorter than the header and tag table it declares.
    Truncated,
    /// The `acsp` magic is missing, so the data is not an ICC profile.
    NotAProfile,
    /// A tag's data runs past the declared size of the profile.
    TagOutOfBounds(String),
}

/// The fields of the profile header that describe what the profile is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// The declared size of the profile in bytes.
    pub size: u32,
    /// Major and minor version: version 2.1 is `(2, 1)`.
    pub version: (u8, u8),
    /// Four-character device class, for example `mntr` for a display or `prtr` for a printer.
    pub device_class: String,
    /// Four-character colour space of the data, for example `RGB ` or `GRAY`.
    pub color_space: String,
    /// Four-character profile connection space, `XYZ ` or `Lab `.
    pub connection_space: String,
}

/// One entry of the tag table. The offset is measured from the start of the profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub signature: String,
    pub offset: u32,
    pub size: u32,
}

/// A parsed profile header and tag table. The tag data stays in the caller's bytes; `description` takes them again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub header: Header,
    pub tags: Vec<Tag>,
}

impl Profile {
    /// Parses the header and the tag table of a profile held in `data`.
    pub fn parse(data: &[u8]) -> Result<Profile, IccError> {
        if data.len() < TABLE_START {
            return Err(IccError::Truncated);
        }
        let size_field = be_u32(data, 0).ok_or(IccError::Truncated)?;
        let size = size_field as usize;
        if size < TABLE_START || size > data.len() {
            return Err(IccError::Truncated);
        }
        if signature(data, 36) != "acsp" {
            return Err(IccError::NotAProfile);
        }
        let count = be_u32(data, HEADER_LEN).ok_or(IccError::Truncated)? as usize;
        if TABLE_START + TAG_ENTRY_LEN * count > size {
            return Err(IccError::Truncated);
        }
        let mut tags = Vec::with_capacity(count);
        for i in 0..count {
            let at = TABLE_START + TAG_ENTRY_LEN * i;
            let sig = signature(data, at);
            let offset = be_u32(data, at + 4).ok_or(IccError::Truncated)?;
            let len = be_u32(data, at + 8).ok_or(IccError::Truncated)?;
            if (offset as usize) + (len as usize) > size {
                return Err(IccError::TagOutOfBounds(sig));
            }
            tags.push(Tag { signature: sig, offset, size: len });
        }
        let header = Header {
            size: size_field,
            version: (data[8], data[9] >> 4),
            device_class: signature(data, 12),
            color_space: signature(data, 16),
            connection_space: signature(data, 20),
        };
        Ok(Profile { header, tags })
    }

    /// Whether the tag table has an entry with this signature.
    pub fn has_tag(&self, sig: &str) -> bool {
        self.tags.iter().any(|t| t.signature == sig)
    }

    /// The text of the description tag, or `None` when the profile has none or its data is malformed. `data` must be
    /// the bytes the profile was parsed from.
    pub fn description(&self, data: &[u8]) -> Option<String> {
        let tag = self.tags.iter().find(|t| t.signature == "desc")?;
        let start = tag.offset as usize;
        let body = data.get(start..start + tag.size as usize)?;
        let kind = body.get(0..4)?;
        if kind == b"desc" {
            // Version 2: count (bytes, including the terminating NUL) at +8, the ASCII text from +12.
            let count = be_u32(body, 8)? as usize;
            let text = body.get(12..12 + count)?;
            Some(String::from_utf8_lossy(text).trim_end_matches('\0').to_string())
        } else if kind == b"mluc" {
            // Version 4: record count at +8, then 12-byte records (language, country, length, offset) from +16.
            // The first record gives the text; its offset is measured from the start of the tag.
            if be_u32(body, 8)? == 0 {
                return None;
            }
            let len = be_u32(body, 20)? as usize;
            let off = be_u32(body, 24)? as usize;
            let bytes = body.get(off..off + len)?;
            let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            Some(char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect())
        } else {
            None
        }
    }
}

fn be_u32(data: &[u8], at: usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

fn signature(data: &[u8], at: usize) -> String {
    data.get(at..at + 4).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default()
}

/// The profile a display uses when none is registered for it.
pub const FALLBACK_DISPLAY_PROFILE: &str = "sRGB IEC61966-2.1";

/// The kinds of device that have profiles. The order is the order `Devices::iterate` reports them in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeviceClass {
    Display,
    Printer,
    Scanner,
    Camera,
}

/// Why a device change was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceError {
    /// No device with that class and identifier is registered.
    NotRegistered,
    /// The profile name is empty.
    EmptyProfile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Device {
    default: String,
    custom: Option<String>,
}

/// The registered devices and the profile each one uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Devices {
    by_id: BTreeMap<(DeviceClass, String), Device>,
}

impl Devices {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a device with its default profile. Registering a device again replaces its default and keeps its
    /// custom profile.
    pub fn register(&mut self, class: DeviceClass, id: &str, default_profile: &str) -> Result<(), DeviceError> {
        if default_profile.is_empty() {
            return Err(DeviceError::EmptyProfile);
        }
        let entry = self
            .by_id
            .entry((class, id.to_string()))
            .or_insert_with(|| Device { default: String::new(), custom: None });
        entry.default = default_profile.to_string();
        Ok(())
    }

    /// Removes a device. Returns whether it was registered.
    pub fn unregister(&mut self, class: DeviceClass, id: &str) -> bool {
        self.by_id.remove(&(class, id.to_string())).is_some()
    }

    /// Sets the custom profile of a registered device, or clears it with `None` so the default applies again.
    pub fn set_custom(&mut self, class: DeviceClass, id: &str, profile: Option<&str>) -> Result<(), DeviceError> {
        if profile.is_some_and(str::is_empty) {
            return Err(DeviceError::EmptyProfile);
        }
        let device = self.by_id.get_mut(&(class, id.to_string())).ok_or(DeviceError::NotRegistered)?;
        device.custom = profile.map(str::to_string);
        Ok(())
    }

    /// The profile in use for a device: its custom profile when one is set, otherwise its default.
    pub fn profile_for(&self, class: DeviceClass, id: &str) -> Option<&str> {
        let device = self.by_id.get(&(class, id.to_string()))?;
        Some(device.custom.as_deref().unwrap_or(device.default.as_str()))
    }

    /// The profile a display uses, falling back to the sRGB profile when the display is not registered.
    pub fn display_profile(&self, id: &str) -> &str {
        self.profile_for(DeviceClass::Display, id).unwrap_or(FALLBACK_DISPLAY_PROFILE)
    }

    /// Every registered device with the profile in use, ordered by class and then by identifier.
    pub fn iterate(&self) -> Vec<(DeviceClass, &str, &str)> {
        self.by_id
            .iter()
            .map(|((class, id), d)| (*class, id.as_str(), d.custom.as_deref().unwrap_or(d.default.as_str())))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a profile with the given class, colour space and tags. The tag data follows the table, each tag padded
    /// to a four-byte boundary. The profile connection space is `XYZ `.
    fn build(class: &str, color_space: &str, tags: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let table_end = TABLE_START + TAG_ENTRY_LEN * tags.len();
        let mut body = Vec::new();
        let mut offsets = Vec::new();
        for (_, bytes) in tags {
            offsets.push(table_end + body.len());
            body.extend_from_slice(bytes);
            while body.len() % 4 != 0 {
                body.push(0);
            }
        }
        let total = table_end + body.len();
        let mut data = vec![0u8; table_end];
        data[0..4].copy_from_slice(&(total as u32).to_be_bytes());
        data[8] = 2;
        data[9] = 0x10;
        data[12..16].copy_from_slice(class.as_bytes());
        data[16..20].copy_from_slice(color_space.as_bytes());
        data[20..24].copy_from_slice(b"XYZ ");
        data[36..40].copy_from_slice(b"acsp");
        data[HEADER_LEN..TABLE_START].copy_from_slice(&(tags.len() as u32).to_be_bytes());
        for (i, (sig, bytes)) in tags.iter().enumerate() {
            let at = TABLE_START + TAG_ENTRY_LEN * i;
            data[at..at + 4].copy_from_slice(sig.as_bytes());
            data[at + 4..at + 8].copy_from_slice(&(offsets[i] as u32).to_be_bytes());
            data[at + 8..at + 12].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
        }
        data.extend_from_slice(&body);
        data
    }

    /// A version 2 description tag holding `text`.
    fn desc_tag(text: &str) -> Vec<u8> {
        let mut v = b"desc".to_vec();
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&((text.len() + 1) as u32).to_be_bytes());
        v.extend_from_slice(text.as_bytes());
        v.push(0);
        v
    }

    /// A version 4 multi-localized tag holding one en-US record with `text` in UTF-16BE.
    fn mluc_tag(text: &str) -> Vec<u8> {
        let units: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        let mut v = b"mluc".to_vec();
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&1u32.to_be_bytes());
        v.extend_from_slice(&12u32.to_be_bytes());
        v.extend_from_slice(b"en");
        v.extend_from_slice(b"US");
        v.extend_from_slice(&(units.len() as u32).to_be_bytes());
        v.extend_from_slice(&28u32.to_be_bytes());
        v.extend_from_slice(&units);
        v
    }

    #[test]
    fn parse_reads_the_header_and_the_first_tag() {
        let data = build("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
        let profile = Profile::parse(&data).unwrap();
        assert_eq!(profile.header.device_class, "mntr");
        assert_eq!(profile.header.color_space, "RGB ");
        assert_eq!(profile.header.connection_space, "XYZ ");
        assert_eq!(profile.header.version, (2, 1));
        assert_eq!(profile.header.size as usize, data.len());
        // The table is 144 bytes long, so the one tag starts at 144 and its text is 18 bytes (4 + 4 + 4 + 5 + NUL).
        assert_eq!(profile.tags, vec![Tag { signature: "desc".into(), offset: 144, size: 18 }]);
    }

    #[test]
    fn description_reads_a_version_two_desc_tag() {
        let data = build("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
        let profile = Profile::parse(&data).unwrap();
        assert_eq!(profile.description(&data), Some("Panel".to_string()));
    }

    #[test]
    fn description_reads_a_version_four_mluc_tag_as_utf16() {
        let data = build("mntr", "RGB ", &[("desc", mluc_tag("Skjár"))]);
        let profile = Profile::parse(&data).unwrap();
        assert_eq!(profile.description(&data), Some("Skjár".to_string()));
    }

    #[test]
    fn description_is_none_without_a_desc_tag() {
        let data = build("prtr", "GRAY", &[("wtpt", vec![0; 20])]);
        let profile = Profile::parse(&data).unwrap();
        assert!(profile.has_tag("wtpt"));
        assert!(!profile.has_tag("desc"));
        assert_eq!(profile.description(&data), None);
    }

    #[test]
    fn data_without_the_acsp_magic_is_not_a_profile() {
        let mut data = build("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
        data[36..40].copy_from_slice(b"xxxx");
        assert_eq!(Profile::parse(&data), Err(IccError::NotAProfile));
    }

    #[test]
    fn a_tag_that_runs_past_the_declared_size_is_refused() {
        let mut data = build("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
        // The first table entry's size field sits at 132 + 8.
        data[140..144].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        assert_eq!(Profile::parse(&data), Err(IccError::TagOutOfBounds("desc".into())));
    }

    #[test]
    fn short_data_and_an_overlong_declared_size_are_truncated() {
        let data = build("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
        assert_eq!(Profile::parse(&data[..100]), Err(IccError::Truncated));
        let mut long = data.clone();
        long[0..4].copy_from_slice(&10_000u32.to_be_bytes());
        assert_eq!(Profile::parse(&long), Err(IccError::Truncated));
    }

    #[test]
    fn a_registered_device_reports_its_default_until_a_custom_profile_is_set() {
        let mut devices = Devices::new();
        devices.register(DeviceClass::Display, "1", "Panel A").unwrap();
        assert_eq!(devices.profile_for(DeviceClass::Display, "1"), Some("Panel A"));
        devices.set_custom(DeviceClass::Display, "1", Some("Calibrated")).unwrap();
        assert_eq!(devices.profile_for(DeviceClass::Display, "1"), Some("Calibrated"));
        devices.set_custom(DeviceClass::Display, "1", None).unwrap();
        assert_eq!(devices.profile_for(DeviceClass::Display, "1"), Some("Panel A"));
    }

    #[test]
    fn re_registering_a_device_keeps_its_custom_profile() {
        let mut devices = Devices::new();
        devices.register(DeviceClass::Printer, "lp0", "Paper v1").unwrap();
        devices.set_custom(DeviceClass::Printer, "lp0", Some("Glossy")).unwrap();
        devices.register(DeviceClass::Printer, "lp0", "Paper v2").unwrap();
        assert_eq!(devices.profile_for(DeviceClass::Printer, "lp0"), Some("Glossy"));
        assert!(devices.unregister(DeviceClass::Printer, "lp0"));
        assert!(!devices.unregister(DeviceClass::Printer, "lp0"));
    }

    #[test]
    fn an_unregistered_display_uses_the_fallback_profile() {
        let devices = Devices::new();
        assert_eq!(devices.profile_for(DeviceClass::Display, "9"), None);
        assert_eq!(devices.display_profile("9"), FALLBACK_DISPLAY_PROFILE);
    }

    #[test]
    fn custom_profiles_on_unknown_devices_and_empty_names_are_refused() {
        let mut devices = Devices::new();
        assert_eq!(devices.set_custom(DeviceClass::Camera, "cam", Some("X")), Err(DeviceError::NotRegistered));
        devices.register(DeviceClass::Camera, "cam", "Sensor").unwrap();
        assert_eq!(devices.set_custom(DeviceClass::Camera, "cam", Some("")), Err(DeviceError::EmptyProfile));
        assert_eq!(devices.register(DeviceClass::Camera, "cam2", ""), Err(DeviceError::EmptyProfile));
    }

    #[test]
    fn iterate_orders_by_class_then_identifier() {
        let mut devices = Devices::new();
        devices.register(DeviceClass::Camera, "b", "Cam").unwrap();
        devices.register(DeviceClass::Display, "z", "Zed").unwrap();
        devices.register(DeviceClass::Display, "a", "Aye").unwrap();
        let listed = devices.iterate();
        assert_eq!(
            listed,
            vec![
                (DeviceClass::Display, "a", "Aye"),
                (DeviceClass::Display, "z", "Zed"),
                (DeviceClass::Camera, "b", "Cam"),
            ]
        );
    }
}
