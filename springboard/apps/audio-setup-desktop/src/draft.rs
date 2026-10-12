//! An aggregate or multi-output device being edited: its name, its kind, its members in order, and its clock source.
//! `validate` says what the server would refuse, and `encode` and `decode` carry the draft across the property the
//! server reads and writes (`COMPOSITION`, see below).
//!
//! Contract addition (task 12, for task 11 to implement on the server): `COMPOSITION` (`agcm`) is read on an
//! aggregate or multi-output device as `PropertyValue::String` in the encoded form below, and set on the system
//! object with the same form to create (first line empty) or replace (first line is the device's UID) a device.
//! `REMOVE` (`agrm`) is set on a made device with `PropertyValue::Bool(true)` to destroy it.

use crate::model::{DeviceInfo, Kind};
use bifrost_audio_types::{FourCC, PropertyAddress, Scope};

/// The composition property (see the module comment).
pub const COMPOSITION: FourCC = FourCC::new(b"agcm");
/// The removal property (see the module comment).
pub const REMOVE: FourCC = FourCC::new(b"agrm");

/// The address of the composition property on a device (global scope, main element).
pub const fn composition_address() -> PropertyAddress {
    PropertyAddress::scoped(COMPOSITION, Scope::Global)
}

/// The address of the removal property on a device.
pub const fn remove_address() -> PropertyAddress {
    PropertyAddress::scoped(REMOVE, Scope::Global)
}

/// What the server would refuse, in words the window shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftError {
    NoName,
    NoMembers,
    /// A member is not in the device list (it was unplugged while the draft was open).
    UnknownMember(String),
    /// A member has no output, so it cannot take part in a multi-output device.
    NoOutput(String),
    /// Members run at different rates, so one mix cannot go to all of them.
    RateMismatch,
}

impl DraftError {
    /// The sentence the window shows under the form.
    pub fn message(&self) -> String {
        match self {
            DraftError::NoName => "Give the device a name.".to_string(),
            DraftError::NoMembers => "Choose at least one device.".to_string(),
            DraftError::UnknownMember(uid) => format!("The device {uid} is no longer connected."),
            DraftError::NoOutput(uid) => format!("The device {uid} has no output for a multi-output device."),
            DraftError::RateMismatch => "The devices run at different rates. A multi-output device needs one rate.".to_string(),
        }
    }
}

/// The form state. `uid` is `Some` when the draft edits an existing made device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub uid: Option<String>,
    pub name: String,
    /// `Kind::Aggregate` or `Kind::MultiOutput`.
    pub kind: Kind,
    /// Member device identifiers in order; the first one is the clock when no clock source is chosen.
    pub members: Vec<String>,
    /// The clock source, a member UID.
    pub clock_source: Option<String>,
}

impl Draft {
    /// An empty draft of `kind` (a hardware kind is treated as an aggregate).
    pub fn new(kind: Kind, name: &str) -> Draft {
        let kind = if kind == Kind::MultiOutput { Kind::MultiOutput } else { Kind::Aggregate };
        Draft { uid: None, name: name.to_string(), kind, members: Vec::new(), clock_source: None }
    }

    pub fn is_member(&self, uid: &str) -> bool {
        self.members.iter().any(|m| m == uid)
    }

    /// Adds the device when it is not a member, removes it when it is. Removing the clock makes the first member the clock.
    pub fn toggle(&mut self, uid: &str) {
        if let Some(pos) = self.members.iter().position(|m| m == uid) {
            self.members.remove(pos);
            if self.clock_source.as_deref() == Some(uid) {
                self.clock_source = None;
            }
        } else {
            self.members.push(uid.to_string());
        }
    }

    /// Makes `uid` the clock source. Only a member can be the clock; anything else is refused and `false` returned.
    pub fn set_clock_source(&mut self, uid: &str) -> bool {
        if !self.is_member(uid) {
            return false;
        }
        self.clock_source = Some(uid.to_string());
        true
    }

    /// The clock source the device will use: the chosen clock_source, or the first member.
    pub fn clock(&self) -> Option<&str> {
        match &self.clock_source {
            Some(m) if self.is_member(m) => Some(m.as_str()),
            _ => self.members.first().map(|s| s.as_str()),
        }
    }

    /// Checks the draft against the devices the server reported now.
    pub fn validate(&self, devices: &[DeviceInfo]) -> Result<(), DraftError> {
        if self.name.trim().is_empty() {
            return Err(DraftError::NoName);
        }
        if self.members.is_empty() {
            return Err(DraftError::NoMembers);
        }
        let mut rate: Option<f64> = None;
        for uid in &self.members {
            let Some(dev) = devices.iter().find(|d| &d.uid == uid && d.alive) else {
                return Err(DraftError::UnknownMember(uid.clone()));
            };
            if self.kind == Kind::MultiOutput {
                if !dev.has_output() {
                    return Err(DraftError::NoOutput(uid.clone()));
                }
                match rate {
                    None => rate = Some(dev.rate),
                    Some(r) if (r - dev.rate).abs() > 0.5 => return Err(DraftError::RateMismatch),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// The composition text: line 1 the UID (empty for a new device), line 2 the transport, line 3 the clock UID
    /// (empty when none), line 4 the name, then one member UID per line. A name cannot hold a line break.
    pub fn encode(&self) -> String {
        let name: String = self.name.trim().chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
        let mut out = String::new();
        out.push_str(self.uid.as_deref().unwrap_or(""));
        out.push('\n');
        out.push_str(self.kind.transport());
        out.push('\n');
        out.push_str(self.clock().unwrap_or(""));
        out.push('\n');
        out.push_str(&name);
        for m in &self.members {
            out.push('\n');
            out.push_str(m);
        }
        out
    }

    /// The draft `encode` wrote, or `None` when the text does not have that shape.
    pub fn decode(text: &str) -> Option<Draft> {
        let mut lines = text.split('\n');
        let uid = lines.next()?;
        let kind = Kind::from_transport(lines.next()?);
        if !kind.is_made() {
            return None;
        }
        let clock_source = lines.next()?;
        let name = lines.next()?;
        let members: Vec<String> = lines.filter(|l| !l.is_empty()).map(str::to_string).collect();
        Some(Draft {
            uid: (!uid.is_empty()).then(|| uid.to_string()),
            name: name.to_string(),
            kind,
            members,
            clock_source: (!clock_source.is_empty()).then(|| clock_source.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hw(uid: &str, rate: f64, outputs: u32) -> DeviceInfo {
        let mut d = DeviceInfo::new(10, uid, uid);
        d.rate = rate;
        d.output_channels = outputs;
        d
    }

    #[test]
    fn toggling_members_keeps_their_order_and_the_clock() {
        let mut d = Draft::new(Kind::Aggregate, "Stage");
        d.toggle("analog");
        d.toggle("hdmi3");
        assert_eq!(d.members, vec!["analog", "hdmi3"]);
        assert_eq!(d.clock(), Some("analog"));
        assert!(d.set_clock_source("hdmi3"));
        assert_eq!(d.clock(), Some("hdmi3"));
        // removing the clock drops the clock_source: the first remaining member becomes the clock
        d.toggle("hdmi3");
        assert_eq!(d.clock_source, None);
        assert_eq!(d.clock(), Some("analog"));
        // a device that is not a member cannot be the clock
        assert!(!d.set_clock_source("hdmi7"));
    }

    #[test]
    fn validation_follows_the_kind() {
        let devices = vec![hw("analog", 48000.0, 2), hw("hdmi3", 48000.0, 2), hw("mic", 44100.0, 0)];
        let mut d = Draft::new(Kind::Aggregate, "Stage");
        assert_eq!(d.validate(&devices), Err(DraftError::NoMembers));
        d.members = vec!["analog".into(), "mic".into()];
        // an aggregate may mix rates (drift compensation) and may have an input-only member
        assert_eq!(d.validate(&devices), Ok(()));
        d.name = "  ".into();
        assert_eq!(d.validate(&devices), Err(DraftError::NoName));
        d.name = "Stage".into();
        d.members.push("gone".into());
        assert_eq!(d.validate(&devices), Err(DraftError::UnknownMember("gone".into())));
    }

    #[test]
    fn multi_output_needs_outputs_and_one_rate() {
        let devices = vec![hw("analog", 48000.0, 2), hw("hdmi3", 48000.0, 2), hw("mic", 48000.0, 0), hw("cd", 44100.0, 2)];
        let mut d = Draft::new(Kind::MultiOutput, "Both");
        d.members = vec!["analog".into(), "mic".into()];
        assert_eq!(d.validate(&devices), Err(DraftError::NoOutput("mic".into())));
        d.members = vec!["analog".into(), "cd".into()];
        assert_eq!(d.validate(&devices), Err(DraftError::RateMismatch));
        d.members = vec!["analog".into(), "hdmi3".into()];
        assert_eq!(d.validate(&devices), Ok(()));
    }

    #[test]
    fn the_composition_round_trips() {
        let mut d = Draft::new(Kind::Aggregate, "Rack\nB");
        d.members = vec!["analog".into(), "hdmi3".into()];
        d.clock_source = Some("hdmi3".into());
        let text = d.encode();
        assert_eq!(text, "\naggregate\nhdmi3\nRack B\nanalog\nhdmi3");
        let back = Draft::decode(&text).expect("decodes");
        assert_eq!(back.uid, None);
        assert_eq!(back.kind, Kind::Aggregate);
        assert_eq!(back.name, "Rack B");
        assert_eq!(back.members, d.members);
        assert_eq!(back.clock_source.as_deref(), Some("hdmi3"));

        // an edited device keeps its UID on the first line
        let edited = Draft { uid: Some("agg-1".into()), kind: Kind::MultiOutput, members: vec!["analog".into()], ..Draft::new(Kind::MultiOutput, "Rack") };
        let text = edited.encode();
        assert!(text.starts_with("agg-1\nmulti-output\nanalog\n"));
        assert_eq!(Draft::decode(&text).unwrap().uid.as_deref(), Some("agg-1"));
    }

    #[test]
    fn decode_refuses_hardware_and_short_text() {
        assert_eq!(Draft::decode("agg\npci\nx\nname\na"), None);
        assert_eq!(Draft::decode("agg\naggregate"), None);
        assert_eq!(Draft::decode(""), None);
    }

    #[test]
    fn the_addresses_are_global() {
        assert_eq!(composition_address().scope, Scope::Global);
        assert_eq!(composition_address().selector, COMPOSITION);
        assert_eq!(remove_address().selector, REMOVE);
    }
}
