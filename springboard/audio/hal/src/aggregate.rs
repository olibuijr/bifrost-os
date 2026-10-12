//! Aggregate devices (the reference's `AudioHardwareAggregateDevice`): an ordered list of sub-devices that the
//! client sees as one device, with one clock source. This module is the pure description: it validates the list,
//! resolves the master and clock sub-devices and maps the aggregate's channels onto the sub-devices. It opens no
//! device and runs no IO; the audio server owns the clocks and the drift compensation.
//!
//! Channels are numbered from 0 per direction. Sub-devices keep their order: the first sub-device's channels come
//! first, then the second's, and so on. The master defaults to the first sub-device and the clock to the master,
//! as the reference does when the creation dictionary leaves them out.

use std::ops::Range;

/// The direction of a channel group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Input,
    Output,
}

/// One member of an aggregate: its UID and how many input and output channels it contributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubDevice {
    pub uid: String,
    pub input_channels: u32,
    pub output_channels: u32,
}

impl SubDevice {
    /// The number of channels this sub-device has in `direction`.
    pub fn channels(&self, direction: Direction) -> u32 {
        match direction {
            Direction::Input => self.input_channels,
            Direction::Output => self.output_channels,
        }
    }
}

/// The creation request, as the reference's dictionary keys name it. `master` and `clock` are optional.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateSpec {
    pub uid: String,
    pub name: String,
    pub sub_devices: Vec<SubDevice>,
    pub master: Option<String>,
    pub clock: Option<String>,
    /// A private aggregate belongs to the client that created it and is not listed to others.
    pub private: bool,
    /// A stacked aggregate is a multi-output device (outputs only, duplicated to every sub-device).
    pub stacked: bool,
}

/// Why an aggregate description was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AggregateError {
    /// The aggregate has no UID.
    EmptyUid,
    /// A sub-device has an empty UID.
    EmptySubDeviceUid,
    /// The sub-device list is empty.
    NoSubDevices,
    /// The same sub-device UID appears twice.
    DuplicateSubDevice(String),
    /// The named master is not in the sub-device list.
    UnknownMaster(String),
    /// The named clock is not in the sub-device list.
    UnknownClock(String),
}

/// A validated aggregate: the spec with the master and clock resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Aggregate {
    uid: String,
    name: String,
    sub_devices: Vec<SubDevice>,
    master: String,
    clock: String,
    private: bool,
    stacked: bool,
}

impl Aggregate {
    /// Validates `spec` and resolves the defaults: master is the first sub-device, clock is the master.
    pub fn new(spec: AggregateSpec) -> Result<Aggregate, AggregateError> {
        if spec.uid.is_empty() {
            return Err(AggregateError::EmptyUid);
        }
        if spec.sub_devices.is_empty() {
            return Err(AggregateError::NoSubDevices);
        }
        for (i, sub) in spec.sub_devices.iter().enumerate() {
            if sub.uid.is_empty() {
                return Err(AggregateError::EmptySubDeviceUid);
            }
            if spec.sub_devices[..i].iter().any(|other| other.uid == sub.uid) {
                return Err(AggregateError::DuplicateSubDevice(sub.uid.clone()));
            }
        }
        let first = spec.sub_devices[0].uid.clone();
        let master = match spec.master {
            None => first,
            Some(m) => {
                if !spec.sub_devices.iter().any(|s| s.uid == m) {
                    return Err(AggregateError::UnknownMaster(m));
                }
                m
            }
        };
        let clock = match spec.clock {
            None => master.clone(),
            Some(c) => {
                if !spec.sub_devices.iter().any(|s| s.uid == c) {
                    return Err(AggregateError::UnknownClock(c));
                }
                c
            }
        };
        Ok(Aggregate {
            uid: spec.uid,
            name: spec.name,
            sub_devices: spec.sub_devices,
            master,
            clock,
            private: spec.private,
            stacked: spec.stacked,
        })
    }

    pub fn uid(&self) -> &str {
        &self.uid
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The sub-devices in their order.
    pub fn sub_devices(&self) -> &[SubDevice] {
        &self.sub_devices
    }

    /// The UID of the master sub-device (the one whose format the aggregate reports).
    pub fn master(&self) -> &str {
        &self.master
    }

    /// The UID of the clock source sub-device.
    pub fn clock(&self) -> &str {
        &self.clock
    }

    pub fn is_private(&self) -> bool {
        self.private
    }

    pub fn is_stacked(&self) -> bool {
        self.stacked
    }

    /// The total channels in `direction`, the sum over the sub-devices (saturating).
    pub fn channel_count(&self, direction: Direction) -> u32 {
        self.sub_devices
            .iter()
            .fold(0u32, |total, sub| total.saturating_add(sub.channels(direction)))
    }

    /// The aggregate channels that sub-device `index` occupies in `direction`, or None when there is no such
    /// sub-device.
    pub fn sub_device_channels(&self, direction: Direction, index: usize) -> Option<Range<u32>> {
        let sub = self.sub_devices.get(index)?;
        let start = self.sub_devices[..index]
            .iter()
            .fold(0u32, |total, s| total.saturating_add(s.channels(direction)));
        Some(start..start.saturating_add(sub.channels(direction)))
    }

    /// Maps an aggregate channel to (sub-device index, channel within that sub-device). None when the channel is
    /// past the end of the direction.
    pub fn channel_map(&self, direction: Direction, channel: u32) -> Option<(usize, u32)> {
        let mut base = 0u32;
        for (i, sub) in self.sub_devices.iter().enumerate() {
            let n = sub.channels(direction);
            if let Some(local) = channel.checked_sub(base) {
                if local < n {
                    return Some((i, local));
                }
            }
            base = base.saturating_add(n);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(uid: &str, input: u32, output: u32) -> SubDevice {
        SubDevice { uid: uid.to_string(), input_channels: input, output_channels: output }
    }

    fn spec(subs: Vec<SubDevice>) -> AggregateSpec {
        AggregateSpec {
            uid: "agg-1".to_string(),
            name: "Studio".to_string(),
            sub_devices: subs,
            master: None,
            clock: None,
            private: false,
            stacked: false,
        }
    }

    #[test]
    fn master_and_clock_default_to_the_first_sub_device() {
        let a = Aggregate::new(spec(vec![sub("usb", 2, 2), sub("hdmi", 0, 8)])).unwrap();
        assert_eq!(a.master(), "usb");
        assert_eq!(a.clock(), "usb");
        assert_eq!(a.uid(), "agg-1");
        assert_eq!(a.name(), "Studio");
        assert!(!a.is_private());
        assert!(!a.is_stacked());
    }

    #[test]
    fn explicit_master_sets_the_default_clock() {
        let mut s = spec(vec![sub("usb", 2, 2), sub("hdmi", 0, 8)]);
        s.master = Some("hdmi".to_string());
        let a = Aggregate::new(s).unwrap();
        assert_eq!(a.master(), "hdmi");
        assert_eq!(a.clock(), "hdmi");
    }

    #[test]
    fn explicit_clock_can_differ_from_master() {
        let mut s = spec(vec![sub("usb", 2, 2), sub("hdmi", 0, 8)]);
        s.master = Some("usb".to_string());
        s.clock = Some("hdmi".to_string());
        let a = Aggregate::new(s).unwrap();
        assert_eq!(a.master(), "usb");
        assert_eq!(a.clock(), "hdmi");
    }

    #[test]
    fn refuses_bad_descriptions() {
        let mut no_uid = spec(vec![sub("usb", 2, 2)]);
        no_uid.uid = String::new();
        assert_eq!(Aggregate::new(no_uid), Err(AggregateError::EmptyUid));

        assert_eq!(Aggregate::new(spec(vec![])), Err(AggregateError::NoSubDevices));
        assert_eq!(
            Aggregate::new(spec(vec![sub("", 1, 1)])),
            Err(AggregateError::EmptySubDeviceUid)
        );
        assert_eq!(
            Aggregate::new(spec(vec![sub("usb", 1, 1), sub("usb", 2, 2)])),
            Err(AggregateError::DuplicateSubDevice("usb".to_string()))
        );

        let mut bad_master = spec(vec![sub("usb", 1, 1)]);
        bad_master.master = Some("nope".to_string());
        assert_eq!(
            Aggregate::new(bad_master),
            Err(AggregateError::UnknownMaster("nope".to_string()))
        );

        let mut bad_clock = spec(vec![sub("usb", 1, 1)]);
        bad_clock.clock = Some("nope".to_string());
        assert_eq!(
            Aggregate::new(bad_clock),
            Err(AggregateError::UnknownClock("nope".to_string()))
        );
    }

    #[test]
    fn channel_counts_sum_over_sub_devices() {
        let a = Aggregate::new(spec(vec![sub("usb", 2, 2), sub("hdmi", 0, 8)])).unwrap();
        assert_eq!(a.channel_count(Direction::Input), 2);
        assert_eq!(a.channel_count(Direction::Output), 10);
    }

    #[test]
    fn channel_map_walks_the_sub_devices_in_order() {
        let a = Aggregate::new(spec(vec![sub("usb", 2, 2), sub("hdmi", 0, 8)])).unwrap();
        assert_eq!(a.channel_map(Direction::Output, 0), Some((0, 0)));
        assert_eq!(a.channel_map(Direction::Output, 1), Some((0, 1)));
        assert_eq!(a.channel_map(Direction::Output, 2), Some((1, 0)));
        assert_eq!(a.channel_map(Direction::Output, 9), Some((1, 7)));
        assert_eq!(a.channel_map(Direction::Output, 10), None);
        assert_eq!(a.channel_map(Direction::Input, 1), Some((0, 1)));
        assert_eq!(a.channel_map(Direction::Input, 2), None);
    }

    #[test]
    fn sub_device_ranges_cover_the_direction_without_gaps() {
        let a = Aggregate::new(spec(vec![sub("usb", 2, 2), sub("mic", 1, 0), sub("hdmi", 0, 8)])).unwrap();
        assert_eq!(a.sub_device_channels(Direction::Input, 0), Some(0..2));
        assert_eq!(a.sub_device_channels(Direction::Input, 1), Some(2..3));
        assert_eq!(a.sub_device_channels(Direction::Input, 2), Some(3..3));
        assert_eq!(a.sub_device_channels(Direction::Output, 2), Some(2..10));
        assert_eq!(a.sub_device_channels(Direction::Output, 3), None);
    }

    #[test]
    fn flags_are_kept() {
        let mut s = spec(vec![sub("usb", 2, 2)]);
        s.private = true;
        s.stacked = true;
        let a = Aggregate::new(s).unwrap();
        assert!(a.is_private());
        assert!(a.is_stacked());
        assert_eq!(a.sub_devices().len(), 1);
    }
}
