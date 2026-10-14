//! MIDI Capability Inquiry (MIDI-CI) messages and the discovery manager, for apps that find and configure devices.
//!
//! A MIDI-CI message is a Universal System Exclusive message: `F0 7E <device id> 0D <sub-id 2> <version>
//! <source MUID> <destination MUID> <payload> F7`. A MUID is a 28-bit id sent as four 7-bit bytes, least
//! significant first. `Message::encode` and `Message::decode` convert between bytes and values; a decoded message
//! never carries a byte above 0x7F.
//!
//! `DiscoveryManager` mirrors `MIDICIDiscoveryManager`: it builds the broadcast inquiry, answers inquiries about
//! the local device, and keeps a `Device` (`MIDICIDevice`) with its `Profile`s (`MIDICIProfile`) for every reply it
//! sees. Nothing here opens a MIDI port; the caller moves the bytes.

/// The destination MUID that addresses every device.
pub const BROADCAST_MUID: Muid = Muid(0x0FFF_FFFF);
/// The device id that addresses the whole function block.
pub const WHOLE_BLOCK: u8 = 0x7F;
/// The MIDI-CI version this module speaks (1.1).
pub const VERSION: u8 = 0x01;

pub(crate) const SYSEX_START: u8 = 0xF0;
pub(crate) const SYSEX_END: u8 = 0xF7;
pub(crate) const UNIVERSAL_NON_REALTIME: u8 = 0x7E;
pub(crate) const SUB_ID_1: u8 = 0x0D;
const HEADER_LEN: usize = 13;

const DISCOVERY_INQUIRY: u8 = 0x70;
const DISCOVERY_REPLY: u8 = 0x71;
const INVALIDATE_MUID: u8 = 0x7E;
const PROFILE_INQUIRY: u8 = 0x20;
const PROFILE_INQUIRY_REPLY: u8 = 0x21;
const PROFILE_SET_ON: u8 = 0x22;
const PROFILE_ENABLED_REPORT: u8 = 0x24;

/// A MIDI-CI unique id, 28 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Muid(pub u32);

impl Muid {
    /// The four 7-bit bytes, least significant first.
    pub fn to_bytes(self) -> [u8; 4] {
        let v = self.0 & 0x0FFF_FFFF;
        [(v & 0x7F) as u8, ((v >> 7) & 0x7F) as u8, ((v >> 14) & 0x7F) as u8, ((v >> 21) & 0x7F) as u8]
    }

    /// Reads four 7-bit bytes; `None` when one has the high bit set or fewer than four are given.
    pub fn from_bytes(b: &[u8]) -> Option<Muid> {
        if b.len() < 4 || b[..4].iter().any(|x| x & 0x80 != 0) {
            return None;
        }
        Some(Muid(b[0] as u32 | (b[1] as u32) << 7 | (b[2] as u32) << 14 | (b[3] as u32) << 21))
    }
}

/// A profile id: five bytes, the first `0x7E` for a standard profile or a manufacturer id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProfileId(pub [u8; 5]);

/// What a device says about itself in a discovery message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub manufacturer: [u8; 3],
    pub family: u16,
    pub model: u16,
    pub software_revision: [u8; 4],
    /// Bit 1 profile configuration, bit 2 property exchange, bit 3 process inquiry.
    pub capability_category: u8,
    pub max_sysex_size: u32,
}

/// The payload of a message; the sub-id 2 byte picks the variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    DiscoveryInquiry(DeviceInfo),
    DiscoveryReply(DeviceInfo),
    InvalidateMuid { target: Muid },
    ProfileInquiry,
    ProfileInquiryReply { enabled: Vec<ProfileId>, disabled: Vec<ProfileId> },
    ProfileSetOn { profile: ProfileId, channels: u16 },
    ProfileEnabledReport { profile: ProfileId, channels: u16 },
}

/// A whole MIDI-CI message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub device_id: u8,
    pub source: Muid,
    pub destination: Muid,
    pub body: Body,
}

/// Why bytes are not a MIDI-CI message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NotMidiCi,
    Truncated,
    BadByte,
    UnknownMessage(u8),
    UnsupportedVersion(u8),
}

pub(crate) fn put14(out: &mut Vec<u8>, v: u16) {
    out.push((v & 0x7F) as u8);
    out.push(((v >> 7) & 0x7F) as u8);
}

pub(crate) fn get14(b: &[u8]) -> u16 {
    b[0] as u16 | (b[1] as u16) << 7
}

fn put28(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&Muid(v).to_bytes());
}

impl DeviceInfo {
    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.manufacturer);
        put14(out, self.family);
        put14(out, self.model);
        out.extend_from_slice(&self.software_revision);
        out.push(self.capability_category);
        put28(out, self.max_sysex_size);
    }

    fn decode(b: &[u8]) -> Result<DeviceInfo, Error> {
        if b.len() < 16 {
            return Err(Error::Truncated);
        }
        Ok(DeviceInfo {
            manufacturer: [b[0], b[1], b[2]],
            family: get14(&b[3..5]),
            model: get14(&b[5..7]),
            software_revision: [b[7], b[8], b[9], b[10]],
            capability_category: b[11],
            max_sysex_size: Muid::from_bytes(&b[12..16]).ok_or(Error::BadByte)?.0,
        })
    }
}

fn profiles(b: &[u8]) -> Result<(Vec<ProfileId>, &[u8]), Error> {
    if b.len() < 2 {
        return Err(Error::Truncated);
    }
    let n = get14(b) as usize;
    let rest = &b[2..];
    if rest.len() < n * 5 {
        return Err(Error::Truncated);
    }
    let list = rest[..n * 5].chunks(5).map(|c| ProfileId([c[0], c[1], c[2], c[3], c[4]])).collect();
    Ok((list, &rest[n * 5..]))
}

fn put_profiles(out: &mut Vec<u8>, list: &[ProfileId]) {
    put14(out, list.len() as u16);
    for p in list {
        out.extend_from_slice(&p.0);
    }
}

fn profile_and_channels(p: &[u8]) -> Result<(ProfileId, u16), Error> {
    if p.len() < 7 {
        return Err(Error::Truncated);
    }
    Ok((ProfileId([p[0], p[1], p[2], p[3], p[4]]), get14(&p[5..7])))
}

impl Message {
    fn sub_id(&self) -> u8 {
        match self.body {
            Body::DiscoveryInquiry(_) => DISCOVERY_INQUIRY,
            Body::DiscoveryReply(_) => DISCOVERY_REPLY,
            Body::InvalidateMuid { .. } => INVALIDATE_MUID,
            Body::ProfileInquiry => PROFILE_INQUIRY,
            Body::ProfileInquiryReply { .. } => PROFILE_INQUIRY_REPLY,
            Body::ProfileSetOn { .. } => PROFILE_SET_ON,
            Body::ProfileEnabledReport { .. } => PROFILE_ENABLED_REPORT,
        }
    }

    /// The SysEx bytes, `F0` through `F7`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![SYSEX_START, UNIVERSAL_NON_REALTIME, self.device_id & 0x7F, SUB_ID_1, self.sub_id(), VERSION];
        put28(&mut out, self.source.0);
        put28(&mut out, self.destination.0);
        match &self.body {
            Body::DiscoveryInquiry(info) | Body::DiscoveryReply(info) => info.encode(&mut out),
            Body::InvalidateMuid { target } => put28(&mut out, target.0),
            Body::ProfileInquiry => {}
            Body::ProfileInquiryReply { enabled, disabled } => {
                put_profiles(&mut out, enabled);
                put_profiles(&mut out, disabled);
            }
            Body::ProfileSetOn { profile, channels } | Body::ProfileEnabledReport { profile, channels } => {
                out.extend_from_slice(&profile.0);
                put14(&mut out, *channels);
            }
        }
        out.push(SYSEX_END);
        out
    }

    /// Parses one complete message.
    pub fn decode(bytes: &[u8]) -> Result<Message, Error> {
        if bytes.len() < 5 || bytes[0] != SYSEX_START || bytes[1] != UNIVERSAL_NON_REALTIME || bytes[3] != SUB_ID_1 {
            return Err(Error::NotMidiCi);
        }
        if bytes.len() < HEADER_LEN + 2 || bytes[bytes.len() - 1] != SYSEX_END {
            return Err(Error::Truncated);
        }
        if bytes[1..bytes.len() - 1].iter().any(|b| b & 0x80 != 0) {
            return Err(Error::BadByte);
        }
        if bytes[5] != VERSION {
            return Err(Error::UnsupportedVersion(bytes[5]));
        }
        let source = Muid::from_bytes(&bytes[6..10]).ok_or(Error::BadByte)?;
        let destination = Muid::from_bytes(&bytes[10..14]).ok_or(Error::BadByte)?;
        let p = &bytes[14..bytes.len() - 1];
        let body = match bytes[4] {
            DISCOVERY_INQUIRY => Body::DiscoveryInquiry(DeviceInfo::decode(p)?),
            DISCOVERY_REPLY => Body::DiscoveryReply(DeviceInfo::decode(p)?),
            INVALIDATE_MUID => Body::InvalidateMuid { target: Muid::from_bytes(p).ok_or(Error::Truncated)? },
            PROFILE_INQUIRY => Body::ProfileInquiry,
            PROFILE_INQUIRY_REPLY => {
                let (enabled, rest) = profiles(p)?;
                let (disabled, _) = profiles(rest)?;
                Body::ProfileInquiryReply { enabled, disabled }
            }
            PROFILE_SET_ON => {
                let (profile, channels) = profile_and_channels(p)?;
                Body::ProfileSetOn { profile, channels }
            }
            PROFILE_ENABLED_REPORT => {
                let (profile, channels) = profile_and_channels(p)?;
                Body::ProfileEnabledReport { profile, channels }
            }
            other => return Err(Error::UnknownMessage(other)),
        };
        Ok(Message { device_id: bytes[2], source, destination, body })
    }
}

/// One profile of a discovered device (`MIDICIProfile`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    pub id: ProfileId,
    pub enabled: bool,
}

/// A device that answered discovery (`MIDICIDevice`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub muid: Muid,
    pub info: DeviceInfo,
    pub profiles: Vec<Profile>,
}

/// Finds MIDI-CI devices (`MIDICIDiscoveryManager`).
#[derive(Debug)]
pub struct DiscoveryManager {
    local_muid: Muid,
    local_info: DeviceInfo,
    devices: Vec<Device>,
}

impl DiscoveryManager {
    pub fn new(local_muid: Muid, local_info: DeviceInfo) -> Self {
        DiscoveryManager { local_muid, local_info, devices: Vec::new() }
    }

    /// The devices found so far, in the order they replied.
    pub fn discovered_nodes(&self) -> &[Device] {
        &self.devices
    }

    /// The discovery inquiry to send to every device.
    pub fn discovery_inquiry(&self) -> Vec<u8> {
        Message {
            device_id: WHOLE_BLOCK,
            source: self.local_muid,
            destination: BROADCAST_MUID,
            body: Body::DiscoveryInquiry(self.local_info),
        }
        .encode()
    }

    /// The profile inquiry for a discovered device.
    pub fn profile_inquiry(&self, target: Muid) -> Vec<u8> {
        Message { device_id: WHOLE_BLOCK, source: self.local_muid, destination: target, body: Body::ProfileInquiry }
            .encode()
    }

    /// Takes one incoming message and returns the bytes to send back, if any. A discovery inquiry addressed to us
    /// or to everyone is answered with our reply; a reply adds a device; an invalidate removes it; a profile
    /// inquiry reply or enabled report updates the sender. Messages for another MUID and bytes that are not
    /// MIDI-CI are ignored.
    pub fn handle(&mut self, bytes: &[u8]) -> Option<Vec<u8>> {
        let message = Message::decode(bytes).ok()?;
        if message.destination != self.local_muid && message.destination != BROADCAST_MUID {
            return None;
        }
        let from = message.source;
        match message.body {
            Body::DiscoveryInquiry(_) => {
                return Some(
                    Message {
                        device_id: message.device_id,
                        source: self.local_muid,
                        destination: from,
                        body: Body::DiscoveryReply(self.local_info),
                    }
                    .encode(),
                );
            }
            Body::DiscoveryReply(info) => {
                if let Some(d) = self.devices.iter_mut().find(|d| d.muid == from) {
                    d.info = info;
                } else {
                    self.devices.push(Device { muid: from, info, profiles: Vec::new() });
                }
            }
            Body::InvalidateMuid { target } => self.devices.retain(|d| d.muid != target),
            Body::ProfileInquiryReply { enabled, disabled } => {
                if let Some(d) = self.devices.iter_mut().find(|d| d.muid == from) {
                    d.profiles = enabled
                        .into_iter()
                        .map(|id| Profile { id, enabled: true })
                        .chain(disabled.into_iter().map(|id| Profile { id, enabled: false }))
                        .collect();
                }
            }
            Body::ProfileEnabledReport { profile, .. } => {
                if let Some(p) = self
                    .devices
                    .iter_mut()
                    .find(|d| d.muid == from)
                    .and_then(|d| d.profiles.iter_mut().find(|p| p.id == profile))
                {
                    p.enabled = true;
                }
            }
            Body::ProfileInquiry | Body::ProfileSetOn { .. } => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> DeviceInfo {
        DeviceInfo {
            manufacturer: [0x7D, 0x00, 0x00],
            family: 0x0102,
            model: 0x0304,
            software_revision: [1, 2, 3, 4],
            capability_category: 0x06,
            max_sysex_size: 512,
        }
    }

    const ME: Muid = Muid(0x0123_4567);
    const THEM: Muid = Muid(0x0001_0002);

    #[test]
    fn muid_is_four_seven_bit_bytes_least_significant_first() {
        assert_eq!(BROADCAST_MUID.to_bytes(), [0x7F, 0x7F, 0x7F, 0x7F]);
        assert_eq!(Muid(0x81).to_bytes(), [0x01, 0x01, 0x00, 0x00]);
        assert_eq!(Muid::from_bytes(&[0x01, 0x01, 0, 0]), Some(Muid(0x81)));
        assert_eq!(Muid::from_bytes(&[0x80, 0, 0, 0]), None);
        assert_eq!(Muid::from_bytes(&[0, 0, 0]), None);
    }

    #[test]
    fn discovery_inquiry_matches_the_spec_byte_layout() {
        let m = Message {
            device_id: WHOLE_BLOCK,
            source: Muid(0x81),
            destination: BROADCAST_MUID,
            body: Body::DiscoveryInquiry(info()),
        };
        let expected = [
            0xF0, 0x7E, 0x7F, 0x0D, 0x70, 0x01, // header, sub-id 0x70, version 1.1
            0x01, 0x01, 0x00, 0x00, // source MUID
            0x7F, 0x7F, 0x7F, 0x7F, // broadcast
            0x7D, 0x00, 0x00, // manufacturer
            0x02, 0x02, // family 0x0102 as 7-bit pair
            0x04, 0x06, // model 0x0304
            1, 2, 3, 4, // software revision
            0x06, // capability categories
            0x00, 0x04, 0x00, 0x00, // max size 512
            0xF7,
        ];
        assert_eq!(m.encode(), expected);
        assert_eq!(Message::decode(&expected), Ok(m));
    }

    #[test]
    fn every_message_round_trips() {
        let a = ProfileId([0x7E, 0, 0, 1, 1]);
        let b = ProfileId([0x7E, 0, 0, 2, 1]);
        let bodies = vec![
            Body::DiscoveryReply(info()),
            Body::InvalidateMuid { target: THEM },
            Body::ProfileInquiry,
            Body::ProfileInquiryReply { enabled: vec![a], disabled: vec![b, a] },
            Body::ProfileInquiryReply { enabled: vec![], disabled: vec![] },
            Body::ProfileSetOn { profile: a, channels: 1 },
            Body::ProfileEnabledReport { profile: b, channels: 300 },
        ];
        for body in bodies {
            let m = Message { device_id: 0x02, source: ME, destination: THEM, body };
            let bytes = m.encode();
            assert!(bytes[1..bytes.len() - 1].iter().all(|x| x & 0x80 == 0));
            assert_eq!(Message::decode(&bytes), Ok(m));
        }
    }

    #[test]
    fn profile_inquiry_reply_lists_enabled_then_disabled() {
        let a = ProfileId([0x7E, 0, 0, 1, 1]);
        let m = Message {
            device_id: 0,
            source: ME,
            destination: THEM,
            body: Body::ProfileInquiryReply { enabled: vec![a], disabled: vec![] },
        };
        let bytes = m.encode();
        assert_eq!(&bytes[14..], &[1, 0, 0x7E, 0, 0, 1, 1, 0, 0, 0xF7]);
    }

    #[test]
    fn bad_input_is_refused() {
        assert_eq!(Message::decode(&[0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]), Err(Error::NotMidiCi));
        assert_eq!(Message::decode(&[0xF0, 0x7E, 0x7F, 0x0D, 0x70, 0x01, 0xF7]), Err(Error::Truncated));
        let mut ok = Message { device_id: 0, source: ME, destination: THEM, body: Body::ProfileInquiry }.encode();
        ok[5] = 0x02;
        assert_eq!(Message::decode(&ok), Err(Error::UnsupportedVersion(2)));
        ok[5] = VERSION;
        ok[4] = 0x55;
        assert_eq!(Message::decode(&ok), Err(Error::UnknownMessage(0x55)));
        ok[4] = PROFILE_INQUIRY;
        ok[7] = 0x90;
        assert_eq!(Message::decode(&ok), Err(Error::BadByte));
        // a discovery body one byte short
        let mut short = Message { device_id: 0, source: ME, destination: THEM, body: Body::DiscoveryReply(info()) }.encode();
        short.remove(short.len() - 2);
        assert_eq!(Message::decode(&short), Err(Error::Truncated));
    }

    #[test]
    fn manager_answers_discovery_and_records_replies() {
        let mut ours = DiscoveryManager::new(ME, info());
        let mut theirs = DiscoveryManager::new(THEM, DeviceInfo { model: 9, ..info() });
        let reply = theirs.handle(&ours.discovery_inquiry()).expect("a reply");
        assert!(ours.handle(&reply).is_none());
        let found = ours.discovered_nodes();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].muid, THEM);
        assert_eq!(found[0].info.model, 9);
        // a repeated reply updates, not duplicates
        ours.handle(&reply);
        assert_eq!(ours.discovered_nodes().len(), 1);
    }

    #[test]
    fn manager_tracks_profiles_and_invalidation() {
        let mut m = DiscoveryManager::new(ME, info());
        let hello = Message { device_id: 0x7F, source: THEM, destination: ME, body: Body::DiscoveryReply(info()) };
        m.handle(&hello.encode());
        let a = ProfileId([0x7E, 0, 0, 1, 1]);
        let b = ProfileId([0x7E, 0, 0, 2, 1]);
        let reply = Message {
            device_id: 0x7F,
            source: THEM,
            destination: ME,
            body: Body::ProfileInquiryReply { enabled: vec![a], disabled: vec![b] },
        };
        m.handle(&reply.encode());
        assert_eq!(
            m.discovered_nodes()[0].profiles,
            vec![Profile { id: a, enabled: true }, Profile { id: b, enabled: false }]
        );
        let on = Message {
            device_id: 0x7F,
            source: THEM,
            destination: ME,
            body: Body::ProfileEnabledReport { profile: b, channels: 1 },
        };
        m.handle(&on.encode());
        assert!(m.discovered_nodes()[0].profiles[1].enabled);
        let bye = Message {
            device_id: 0x7F,
            source: THEM,
            destination: BROADCAST_MUID,
            body: Body::InvalidateMuid { target: THEM },
        };
        m.handle(&bye.encode());
        assert!(m.discovered_nodes().is_empty());
    }

    #[test]
    fn messages_for_another_muid_are_ignored() {
        let mut m = DiscoveryManager::new(ME, info());
        let other = Message { device_id: 0, source: THEM, destination: Muid(5), body: Body::DiscoveryReply(info()) };
        assert!(m.handle(&other.encode()).is_none());
        assert!(m.discovered_nodes().is_empty());
        assert!(m.handle(&[0x90, 0x3C, 0x40]).is_none());
    }
}
