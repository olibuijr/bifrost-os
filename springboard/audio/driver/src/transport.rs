//! Device transports and the composite driver that serves devices from several providers.
//!
//! A *transport* is how a device is attached: on the PCI bus, over USB, HDMI, Bluetooth, a network, or not at all
//! (a virtual or aggregate device). Drivers describe it in `DeviceDesc::transport`; [`Transport`] gives those names a
//! type, with the properties the server decides on (can the device go away, is it wireless).
//!
//! A *provider* is any [`Driver`] that serves devices of one or more transports: the ALSA driver for the hardware
//! the kernel knows, the plug-in driver for virtual devices, a network driver. [`CompositeDriver`] is the pluggable
//! transport layer between the server and its providers: it is itself a `Driver`, enumerates every provider, sends
//! `open`, `controls` and `set_control` to the provider that owns the uid, and merges their events. A uid belongs to
//! the first provider that publishes it; a second provider publishing the same uid is recorded in
//! [`CompositeDriver::conflicts`] and its device is not offered.

use crate::{ControlDesc, ControlId, ControlValue, DeviceDesc, DeviceIo, Driver, DriverError, DriverEvent, IoConfig};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Transport {
    BuiltIn,
    Pci,
    Usb,
    Hdmi,
    Bluetooth,
    Network,
    Virtual,
    Aggregate,
    /// A transport name this server does not know.
    Other(String),
}

impl Transport {
    pub fn from_name(name: &str) -> Transport {
        match name {
            "builtin" => Transport::BuiltIn,
            "pci" => Transport::Pci,
            "usb" => Transport::Usb,
            "hdmi" => Transport::Hdmi,
            "bluetooth" => Transport::Bluetooth,
            "network" => Transport::Network,
            "virtual" => Transport::Virtual,
            "aggregate" => Transport::Aggregate,
            other => Transport::Other(other.to_string()),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Transport::BuiltIn => "builtin",
            Transport::Pci => "pci",
            Transport::Usb => "usb",
            Transport::Hdmi => "hdmi",
            Transport::Bluetooth => "bluetooth",
            Transport::Network => "network",
            Transport::Virtual => "virtual",
            Transport::Aggregate => "aggregate",
            Transport::Other(n) => n,
        }
    }

    /// Whether devices on this transport can be plugged in and removed while the server runs.
    pub fn is_hot_pluggable(&self) -> bool {
        matches!(self, Transport::Usb | Transport::Hdmi | Transport::Bluetooth | Transport::Network | Transport::Virtual | Transport::Aggregate)
    }

    /// Whether the device is reached over a radio or a network, so its latency can vary.
    pub fn is_wireless(&self) -> bool {
        matches!(self, Transport::Bluetooth | Transport::Network)
    }

    /// Whether the device is a real piece of hardware.
    pub fn is_hardware(&self) -> bool {
        !matches!(self, Transport::Virtual | Transport::Aggregate | Transport::Other(_))
    }
}

struct Provider {
    name: String,
    driver: Box<dyn Driver>,
}

#[derive(Default)]
pub struct CompositeDriver {
    providers: Vec<Provider>,
    /// uid -> provider index
    owners: HashMap<String, usize>,
    transports: HashMap<String, Transport>,
    offered: Vec<DeviceDesc>,
    conflicts: Vec<String>,
}

impl CompositeDriver {
    pub fn new() -> CompositeDriver {
        CompositeDriver::default()
    }

    /// Adds a provider. Providers are asked in the order they were added, and win a uid in that order.
    pub fn add(&mut self, name: &str, driver: Box<dyn Driver>) {
        self.providers.push(Provider { name: name.to_string(), driver });
    }

    pub fn provider_names(&self) -> Vec<&str> {
        self.providers.iter().map(|p| p.name.as_str()).collect()
    }

    /// The provider that serves `uid`, as of the last enumeration or event.
    pub fn provider_of(&self, uid: &str) -> Option<&str> {
        self.owners.get(uid).map(|i| self.providers[*i].name.as_str())
    }

    pub fn transport_of(&self, uid: &str) -> Option<&Transport> {
        self.transports.get(uid)
    }

    /// The uids on one transport, sorted.
    pub fn uids_on(&self, transport: &Transport) -> Vec<&str> {
        let mut v: Vec<&str> = self.transports.iter().filter(|(_, t)| *t == transport).map(|(u, _)| u.as_str()).collect();
        v.sort_unstable();
        v
    }

    /// uids that more than one provider published, as `"<uid> (<provider>)"` for the provider that lost it.
    pub fn conflicts(&self) -> &[String] {
        &self.conflicts
    }

    fn refresh(&mut self) {
        self.owners.clear();
        self.transports.clear();
        self.offered.clear();
        self.conflicts.clear();
        for (i, p) in self.providers.iter_mut().enumerate() {
            let mut descs = Vec::new();
            p.driver.enumerate(&mut descs);
            for d in descs {
                if self.owners.contains_key(&d.uid) {
                    self.conflicts.push(format!("{} ({})", d.uid, p.name));
                    continue;
                }
                self.owners.insert(d.uid.clone(), i);
                self.transports.insert(d.uid.clone(), Transport::from_name(&d.transport));
                self.offered.push(d);
            }
        }
    }
}

impl Driver for CompositeDriver {
    fn name(&self) -> &str {
        "composite"
    }

    fn enumerate(&mut self, out: &mut Vec<DeviceDesc>) {
        self.refresh();
        out.extend(self.offered.iter().cloned());
    }

    fn open(&mut self, uid: &str, cfg: &IoConfig) -> Result<Box<dyn DeviceIo>, DriverError> {
        if !self.owners.contains_key(uid) {
            // a device may have appeared since the last enumeration
            self.refresh();
        }
        match self.owners.get(uid).copied() {
            Some(i) => self.providers[i].driver.open(uid, cfg),
            None => Err(DriverError::NotFound),
        }
    }

    fn poll_events(&mut self, out: &mut Vec<DriverEvent>) {
        for (i, p) in self.providers.iter_mut().enumerate() {
            let mut events = Vec::new();
            p.driver.poll_events(&mut events);
            for e in events {
                match &e {
                    DriverEvent::DeviceAdded(d) => match self.owners.get(&d.uid) {
                        // already offered by this provider (it was in the last enumeration)
                        Some(owner) if *owner == i => continue,
                        Some(_) => {
                            self.conflicts.push(format!("{} ({})", d.uid, p.name));
                            continue;
                        }
                        None => {
                            self.owners.insert(d.uid.clone(), i);
                            self.transports.insert(d.uid.clone(), Transport::from_name(&d.transport));
                            self.offered.push(d.clone());
                        }
                    },
                    DriverEvent::DeviceRemoved { uid } => {
                        if self.owners.get(uid) != Some(&i) {
                            continue;
                        }
                        self.owners.remove(uid);
                        self.transports.remove(uid);
                        self.offered.retain(|d| &d.uid != uid);
                    }
                    DriverEvent::JackChanged { uid, .. } | DriverEvent::RateChanged { uid, .. } | DriverEvent::ControlChanged { uid, .. } => {
                        if self.owners.get(uid) != Some(&i) {
                            continue;
                        }
                    }
                }
                out.push(e);
            }
        }
    }

    fn controls(&mut self, uid: &str) -> Vec<ControlDesc> {
        match self.owners.get(uid).copied() {
            Some(i) => self.providers[i].driver.controls(uid),
            None => Vec::new(),
        }
    }

    fn set_control(&mut self, uid: &str, id: ControlId, v: ControlValue) -> Result<(), DriverError> {
        match self.owners.get(uid).copied() {
            Some(i) => self.providers[i].driver.set_control(uid, id, v),
            None => Err(DriverError::NotFound),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopback::LoopbackDriver;
    use crate::null::ManualClock;
    use crate::plugin::{PluginDriver, VirtualSpec};
    use std::sync::{Arc, Mutex};

    fn desc(uid: &str, transport: &str) -> DeviceDesc {
        DeviceDesc {
            uid: uid.into(),
            name: uid.into(),
            manufacturer: "test".into(),
            transport: transport.into(),
            input_channels: 0,
            output_channels: 2,
            rates: vec![(48_000.0, 48_000.0)],
            buffer_frames_range: (16, 4096),
            clock_domain: 9,
        }
    }

    /// A provider whose devices and events the test controls.
    struct Stub {
        devices: Arc<Mutex<Vec<DeviceDesc>>>,
        events: Arc<Mutex<Vec<DriverEvent>>>,
    }

    impl Driver for Stub {
        fn name(&self) -> &str {
            "stub"
        }
        fn enumerate(&mut self, out: &mut Vec<DeviceDesc>) {
            out.extend(self.devices.lock().unwrap().iter().cloned());
        }
        fn open(&mut self, _uid: &str, _cfg: &IoConfig) -> Result<Box<dyn DeviceIo>, DriverError> {
            Err(DriverError::Busy)
        }
        fn poll_events(&mut self, out: &mut Vec<DriverEvent>) {
            out.append(&mut self.events.lock().unwrap());
        }
        fn controls(&mut self, uid: &str) -> Vec<ControlDesc> {
            vec![ControlDesc { id: ControlId(1), name: uid.into(), kind: "volume".into(), output: true, db_range: (-60.0, 0.0) }]
        }
        fn set_control(&mut self, _uid: &str, _id: ControlId, _v: ControlValue) -> Result<(), DriverError> {
            Ok(())
        }
    }

    fn stub(devices: Vec<DeviceDesc>) -> (Stub, Arc<Mutex<Vec<DeviceDesc>>>, Arc<Mutex<Vec<DriverEvent>>>) {
        let d = Arc::new(Mutex::new(devices));
        let e = Arc::new(Mutex::new(Vec::new()));
        (Stub { devices: d.clone(), events: e.clone() }, d, e)
    }

    fn cfg() -> IoConfig {
        IoConfig { rate: 48_000.0, quantum: 4, input: true, output: true }
    }

    #[test]
    fn transport_names_map_both_ways_and_have_properties() {
        for n in ["builtin", "pci", "usb", "hdmi", "bluetooth", "network", "virtual", "aggregate"] {
            assert_eq!(Transport::from_name(n).name(), n);
        }
        assert_eq!(Transport::from_name("firewire"), Transport::Other("firewire".into()));
        assert_eq!(Transport::from_name("firewire").name(), "firewire");
        assert!(Transport::Usb.is_hot_pluggable() && !Transport::Pci.is_hot_pluggable());
        assert!(Transport::Bluetooth.is_wireless() && !Transport::Usb.is_wireless());
        assert!(Transport::Hdmi.is_hardware() && !Transport::Virtual.is_hardware());
    }

    #[test]
    fn providers_are_enumerated_together_and_open_goes_to_the_owner() {
        let mut plugin = PluginDriver::new(ManualClock::new().as_fn());
        plugin.publish(VirtualSpec { uid: "bus1".into(), name: "Bus".into(), channels: 2, sample_rate: 48_000.0 }).unwrap();
        let (hw, _, _) = stub(vec![desc("pci:0", "pci"), desc("usb:1", "usb")]);
        let mut c = CompositeDriver::new();
        c.add("alsa", Box::new(hw));
        c.add("plugin", Box::new(plugin));
        c.add("loopback", Box::new(LoopbackDriver::with_clock(ManualClock::new().as_fn())));
        let mut all = Vec::new();
        c.enumerate(&mut all);
        let uids: Vec<&str> = all.iter().map(|d| d.uid.as_str()).collect();
        assert_eq!(uids, vec!["pci:0", "usb:1", "bus1:out", "bus1:in", "loopback:out", "loopback:in"]);
        assert_eq!(c.provider_of("bus1:in"), Some("plugin"));
        assert_eq!(c.transport_of("usb:1"), Some(&Transport::Usb));
        assert_eq!(c.uids_on(&Transport::Virtual), vec!["bus1:in", "bus1:out", "loopback:in", "loopback:out"]);
        assert!(c.open("loopback:in", &cfg()).is_ok());
        assert!(c.open("bus1:out", &cfg()).is_ok());
        assert_eq!(c.open("pci:0", &cfg()).err(), Some(DriverError::Busy), "the owner's own answer comes back");
        assert_eq!(c.open("nothing", &cfg()).err(), Some(DriverError::NotFound));
        assert_eq!(c.controls("pci:0").len(), 1);
        assert!(c.controls("nothing").is_empty());
        assert_eq!(c.set_control("nothing", ControlId(1), ControlValue::Bool(true)), Err(DriverError::NotFound));
    }

    #[test]
    fn a_uid_published_twice_belongs_to_the_first_provider() {
        let (a, _, _) = stub(vec![desc("dup", "usb")]);
        let (b, _, _) = stub(vec![desc("dup", "network"), desc("only-b", "network")]);
        let mut c = CompositeDriver::new();
        c.add("first", Box::new(a));
        c.add("second", Box::new(b));
        let mut all = Vec::new();
        c.enumerate(&mut all);
        assert_eq!(all.len(), 2);
        assert_eq!(c.provider_of("dup"), Some("first"));
        assert_eq!(c.transport_of("dup"), Some(&Transport::Usb));
        assert_eq!(c.conflicts(), &["dup (second)".to_string()]);
    }

    #[test]
    fn events_are_merged_and_keep_the_ownership_current() {
        let (hw, devices, events) = stub(vec![desc("usb:1", "usb")]);
        let mut c = CompositeDriver::new();
        c.add("alsa", Box::new(hw));
        let mut all = Vec::new();
        c.enumerate(&mut all);
        // a device is plugged in: the provider lists it and says so
        devices.lock().unwrap().push(desc("bt:9", "bluetooth"));
        events.lock().unwrap().push(DriverEvent::DeviceAdded(desc("bt:9", "bluetooth")));
        // an event about a device that is already offered is not repeated
        events.lock().unwrap().push(DriverEvent::DeviceAdded(desc("usb:1", "usb")));
        let mut ev = Vec::new();
        c.poll_events(&mut ev);
        assert_eq!(ev.len(), 1);
        assert_eq!(c.provider_of("bt:9"), Some("alsa"));
        assert_eq!(c.transport_of("bt:9"), Some(&Transport::Bluetooth));
        // and unplugged again
        events.lock().unwrap().push(DriverEvent::DeviceRemoved { uid: "bt:9".into() });
        ev.clear();
        c.poll_events(&mut ev);
        assert_eq!(ev, vec![DriverEvent::DeviceRemoved { uid: "bt:9".into() }]);
        assert_eq!(c.provider_of("bt:9"), None);
        // an event about a device nobody offered is dropped
        events.lock().unwrap().push(DriverEvent::RateChanged { uid: "ghost".into(), rate: 44_100.0 });
        ev.clear();
        c.poll_events(&mut ev);
        assert!(ev.is_empty());
    }

    #[test]
    fn open_finds_a_device_that_appeared_after_the_last_enumeration() {
        let (hw, devices, _) = stub(vec![desc("usb:1", "usb")]);
        let mut c = CompositeDriver::new();
        c.add("alsa", Box::new(hw));
        let mut all = Vec::new();
        c.enumerate(&mut all);
        devices.lock().unwrap().push(desc("usb:2", "usb"));
        assert_eq!(c.open("usb:2", &cfg()).err(), Some(DriverError::Busy), "found by a fresh enumeration, then handed to its owner");
    }
}
