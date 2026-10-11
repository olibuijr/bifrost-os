//! Audio hardware object model: device containers (boxes), aggregate devices that combine several devices or taps
//! behind one clock, and the list of client processes that have audio running. These are pure models of the
//! objects a system audio service keeps. They hold identifiers and state; they do not open any device.

/// A container for devices, such as a dock or an external interface. Its devices are only visible while the box is
/// enabled, but they stay recorded while it is disabled, so enabling it again restores the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceBox {
    uid: String,
    pub has_audio: bool,
    pub has_midi: bool,
    pub has_video: bool,
    /// A protected box needs authentication before its devices can be used.
    pub protected: bool,
    enabled: bool,
    devices: Vec<String>,
}

impl DeviceBox {
    pub fn new(uid: &str) -> Self {
        DeviceBox {
            uid: uid.into(),
            has_audio: true,
            has_midi: false,
            has_video: false,
            protected: false,
            enabled: false,
            devices: Vec::new(),
        }
    }
    pub fn uid(&self) -> &str {
        &self.uid
    }
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
    /// Makes the contents available. Returns false when the box was already enabled.
    pub fn enable(&mut self) -> bool {
        !std::mem::replace(&mut self.enabled, true)
    }
    /// Hides the contents. Returns false when the box was already disabled.
    pub fn disable(&mut self) -> bool {
        std::mem::replace(&mut self.enabled, false)
    }
    /// Adds a device uid. Returns false for an empty or duplicate uid.
    pub fn add_device(&mut self, device_uid: &str) -> bool {
        if device_uid.is_empty() || self.devices.iter().any(|d| d == device_uid) {
            return false;
        }
        self.devices.push(device_uid.into());
        true
    }
    /// Removes a device uid. Returns false when it was not in the box.
    pub fn remove_device(&mut self, device_uid: &str) -> bool {
        let before = self.devices.len();
        self.devices.retain(|d| d != device_uid);
        self.devices.len() != before
    }
    /// The devices visible to the system: none while the box is disabled.
    pub fn devices(&self) -> &[String] {
        if self.enabled {
            &self.devices
        } else {
            &[]
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AggregateError {
    /// The uid is already a member of the aggregate, as a device or as a tap.
    Duplicate,
    /// The uid is not a member, so it cannot be the clock source.
    NotAMember,
}

/// A virtual device that joins several devices and taps. One member is the clock source, and the others follow it,
/// so their streams stay aligned. Members keep the order they were added in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateDevice {
    uid: String,
    pub name: String,
    subdevices: Vec<String>,
    subtaps: Vec<String>,
    clock_source: Option<String>,
}

impl AggregateDevice {
    pub fn new(uid: &str, name: &str) -> Self {
        AggregateDevice {
            uid: uid.into(),
            name: name.into(),
            subdevices: Vec::new(),
            subtaps: Vec::new(),
            clock_source: None,
        }
    }
    pub fn uid(&self) -> &str {
        &self.uid
    }
    pub fn subdevices(&self) -> &[String] {
        &self.subdevices
    }
    pub fn subtaps(&self) -> &[String] {
        &self.subtaps
    }
    pub fn clock_source(&self) -> Option<&str> {
        self.clock_source.as_deref()
    }

    fn is_member(&self, uid: &str) -> bool {
        self.subdevices.iter().any(|m| m == uid) || self.subtaps.iter().any(|m| m == uid)
    }

    /// Adds a device to the aggregate. A uid that is already a device or a tap is refused.
    pub fn add_subdevice(&mut self, uid: &str) -> Result<(), AggregateError> {
        if self.is_member(uid) {
            return Err(AggregateError::Duplicate);
        }
        self.subdevices.push(uid.into());
        Ok(())
    }

    /// Adds a tap to the aggregate. A uid that is already a device or a tap is refused.
    pub fn add_subtap(&mut self, uid: &str) -> Result<(), AggregateError> {
        if self.is_member(uid) {
            return Err(AggregateError::Duplicate);
        }
        self.subtaps.push(uid.into());
        Ok(())
    }

    /// Removes a device or tap. If it was the clock source, the clock source is cleared.
    pub fn remove_member(&mut self, uid: &str) -> bool {
        let before = self.subdevices.len() + self.subtaps.len();
        self.subdevices.retain(|m| m != uid);
        self.subtaps.retain(|m| m != uid);
        if self.clock_source.as_deref() == Some(uid) {
            self.clock_source = None;
        }
        self.subdevices.len() + self.subtaps.len() != before
    }

    /// Chooses the clock source. `None` clears it. Only a member can be the clock source.
    pub fn set_clock_source(&mut self, uid: Option<&str>) -> Result<(), AggregateError> {
        match uid {
            None => self.clock_source = None,
            Some(u) if self.is_member(u) => self.clock_source = Some(u.into()),
            Some(_) => return Err(AggregateError::NotAMember),
        }
        Ok(())
    }
}

/// A client process connected to the audio service, with the streams it is running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioProcess {
    pub pid: u32,
    pub bundle_id: String,
    running: bool,
    input_streams: u32,
    output_streams: u32,
    output_devices: Vec<String>,
}

impl AudioProcess {
    pub fn new(pid: u32, bundle_id: &str) -> Self {
        AudioProcess {
            pid,
            bundle_id: bundle_id.into(),
            running: false,
            input_streams: 0,
            output_streams: 0,
            output_devices: Vec::new(),
        }
    }
    /// Audio IO is in progress.
    pub fn is_running(&self) -> bool {
        self.running
    }
    /// IO is in progress and at least one input stream is active.
    pub fn is_running_input(&self) -> bool {
        self.running && self.input_streams > 0
    }
    /// IO is in progress and at least one output stream is active.
    pub fn is_running_output(&self) -> bool {
        self.running && self.output_streams > 0
    }
    /// Starts IO with the given stream counts.
    pub fn start_io(&mut self, input_streams: u32, output_streams: u32) {
        self.running = true;
        self.input_streams = input_streams;
        self.output_streams = output_streams;
    }
    /// Stops IO and drops the stream counts. Returns false when no IO was running.
    pub fn stop_io(&mut self) -> bool {
        let was = self.running;
        self.running = false;
        self.input_streams = 0;
        self.output_streams = 0;
        was
    }
    /// Records an output device uid. Returns false for an empty or duplicate uid.
    pub fn add_output_device(&mut self, device_uid: &str) -> bool {
        if device_uid.is_empty() || self.output_devices.iter().any(|d| d == device_uid) {
            return false;
        }
        self.output_devices.push(device_uid.into());
        true
    }
    /// Output devices this process uses. Reported only while it runs output.
    pub fn output_devices(&self) -> &[String] {
        if self.is_running_output() {
            &self.output_devices
        } else {
            &[]
        }
    }
}

/// The processes connected to the audio service, one entry per pid.
#[derive(Clone, Debug, Default)]
pub struct ProcessList {
    processes: Vec<AudioProcess>,
}

impl ProcessList {
    pub fn new() -> Self {
        Self::default()
    }
    /// Adds a process. Returns false when the pid is already present.
    pub fn add(&mut self, pid: u32, bundle_id: &str) -> bool {
        if self.get(pid).is_some() {
            return false;
        }
        self.processes.push(AudioProcess::new(pid, bundle_id));
        true
    }
    /// Removes a process when it disconnects. Returns false when the pid was unknown.
    pub fn remove(&mut self, pid: u32) -> bool {
        let before = self.processes.len();
        self.processes.retain(|p| p.pid != pid);
        self.processes.len() != before
    }
    pub fn get(&self, pid: u32) -> Option<&AudioProcess> {
        self.processes.iter().find(|p| p.pid == pid)
    }
    pub fn get_mut(&mut self, pid: u32) -> Option<&mut AudioProcess> {
        self.processes.iter_mut().find(|p| p.pid == pid)
    }
    /// Processes with IO in progress, in the order they were added.
    pub fn running(&self) -> Vec<&AudioProcess> {
        self.processes.iter().filter(|p| p.is_running()).collect()
    }
    /// Device uids used for output by running processes, each listed once.
    pub fn devices_in_use(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in self.processes.iter().filter(|p| p.is_running_output()) {
            for d in p.output_devices() {
                if !out.contains(d) {
                    out.push(d.clone());
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_hides_its_devices_while_disabled_and_keeps_them() {
        let mut b = DeviceBox::new("box.dock");
        assert!(b.add_device("dev.usb-1"));
        assert!(!b.add_device("dev.usb-1"), "duplicate device uid");
        assert!(b.devices().is_empty(), "disabled box shows nothing");
        assert!(b.enable());
        assert!(!b.enable());
        assert_eq!(b.devices(), &["dev.usb-1".to_string()]);
        assert!(b.disable());
        assert!(b.devices().is_empty());
        assert!(b.enable());
        assert_eq!(b.devices().len(), 1, "devices come back when enabled again");
    }

    #[test]
    fn box_removes_a_device_only_when_it_is_listed() {
        let mut b = DeviceBox::new("box.card");
        b.add_device("a");
        b.add_device("b");
        assert!(b.remove_device("a"));
        assert!(!b.remove_device("a"));
        b.enable();
        assert_eq!(b.devices(), &["b".to_string()]);
        assert!(!b.add_device(""), "empty uid is refused");
    }

    #[test]
    fn aggregate_refuses_a_member_twice_across_devices_and_taps() {
        let mut agg = AggregateDevice::new("agg.stage", "Stage");
        assert_eq!(agg.add_subdevice("dev.a"), Ok(()));
        assert_eq!(agg.add_subtap("tap.b"), Ok(()));
        assert_eq!(agg.add_subtap("dev.a"), Err(AggregateError::Duplicate));
        assert_eq!(agg.add_subdevice("tap.b"), Err(AggregateError::Duplicate));
        assert_eq!(agg.subdevices(), &["dev.a".to_string()]);
        assert_eq!(agg.subtaps(), &["tap.b".to_string()]);
    }

    #[test]
    fn clock_source_must_be_a_member_and_clears_when_the_member_leaves() {
        let mut agg = AggregateDevice::new("agg.stage", "Stage");
        agg.add_subdevice("dev.a").unwrap();
        agg.add_subtap("tap.b").unwrap();
        assert_eq!(agg.set_clock_source(Some("dev.zzz")), Err(AggregateError::NotAMember));
        assert_eq!(agg.clock_source(), None);
        agg.set_clock_source(Some("tap.b")).unwrap();
        assert_eq!(agg.clock_source(), Some("tap.b"));
        assert!(agg.remove_member("tap.b"));
        assert_eq!(agg.clock_source(), None, "removing the clock member clears the clock source");
        assert!(!agg.remove_member("tap.b"));
    }

    #[test]
    fn process_reports_input_and_output_only_while_io_runs() {
        let mut p = AudioProcess::new(4242, "is.olibuijr.music");
        assert!(!p.is_running() && !p.is_running_output());
        p.start_io(0, 2);
        assert!(p.is_running() && p.is_running_output() && !p.is_running_input());
        assert!(p.add_output_device("dev.speaker"));
        assert!(!p.add_output_device("dev.speaker"));
        assert_eq!(p.output_devices(), &["dev.speaker".to_string()]);
        assert!(p.stop_io());
        assert!(!p.stop_io());
        assert!(!p.is_running_output());
        assert!(p.output_devices().is_empty());
    }

    #[test]
    fn process_list_tracks_running_clients_and_the_devices_they_use() {
        let mut list = ProcessList::new();
        assert!(list.add(10, "app.one"));
        assert!(list.add(11, "app.two"));
        assert!(!list.add(10, "app.dup"), "one entry per pid");
        list.get_mut(10).unwrap().start_io(1, 0);
        list.get_mut(10).unwrap().add_output_device("dev.mic");
        list.get_mut(11).unwrap().start_io(0, 1);
        list.get_mut(11).unwrap().add_output_device("dev.speaker");
        list.get_mut(11).unwrap().add_output_device("dev.mic");
        let running: Vec<u32> = list.running().iter().map(|p| p.pid).collect();
        assert_eq!(running, vec![10, 11]);
        assert_eq!(list.devices_in_use(), vec!["dev.speaker".to_string(), "dev.mic".to_string()]);
        assert!(list.remove(11));
        assert!(!list.remove(11));
        assert!(list.get(11).is_none());
        assert!(list.devices_in_use().is_empty(), "app.one only has input running");
    }
}
