//! `PluginDriver`: the loader for virtual audio devices. The reference platform lets a user-space plug-in add devices
//! to the audio server; here a plug-in is a description file, and this driver publishes what the files describe.
//!
//! A description is `key=value` lines: `uid`, `name`, `channels` and `sample_rate` (blank lines and `#` comments are
//! skipped; a key that is missing, repeated or unknown refuses the file). Each description becomes a loopback pair:
//! a playback device `<uid>:out` and a capture device `<uid>:in`, with the description's name, channel count and
//! rate, and what the server mixes into the first comes out of the second. `load_dir` reads every `*.vdev` file of a
//! folder in name order; a file that is refused is listed in [`PluginDriver::refused`] with the reason and does not
//! stop the others. Devices can be published and removed while the server runs; the driver reports each as a
//! [`DriverEvent`] for the server to pick up with `poll_events`.

use crate::loopback::LoopbackDriver;
use crate::null::ClockFn;
use crate::{ControlDesc, ControlId, ControlValue, DeviceDesc, DeviceIo, Driver, DriverError, DriverEvent, IoConfig};
use std::path::Path;

/// Where description files are looked for by default.
pub const PLUGIN_DIR: &str = "/etc/bifrost/audio/plugins";
/// File extension of a description.
pub const EXTENSION: &str = "vdev";
/// Most channels one virtual device may carry.
pub const MAX_CHANNELS: u32 = 64;
/// The sample rates a virtual device may run at, in hertz.
pub const SUPPORTED_RATES: [f64; 11] = [
    8_000.0, 11_025.0, 16_000.0, 22_050.0, 32_000.0, 44_100.0, 48_000.0, 88_200.0, 96_000.0, 176_400.0, 192_000.0,
];

/// A device a description file asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualSpec {
    pub uid: String,
    pub name: String,
    pub channels: u32,
    pub sample_rate: f64,
}

impl VirtualSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.uid.is_empty() || self.uid.chars().any(|c| c.is_whitespace() || c == ':') {
            return Err("the uid is empty or has a space or a colon".into());
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

    pub fn out_uid(&self) -> String {
        format!("{}:out", self.uid)
    }

    pub fn in_uid(&self) -> String {
        format!("{}:in", self.uid)
    }
}

/// Reads one description.
pub fn parse_descriptor(text: &str) -> Result<VirtualSpec, String> {
    let (mut uid, mut name, mut channels, mut rate) = (None, None, None, None);
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let (key, value) = line.split_once('=').ok_or_else(|| format!("'{line}' is not a key=value line"))?;
        let (key, value) = (key.trim(), value.trim());
        let slot = match key {
            "uid" => &mut uid,
            "name" => &mut name,
            "channels" => &mut channels,
            "sample_rate" => &mut rate,
            other => return Err(format!("unknown key '{other}'")),
        };
        if slot.is_some() {
            return Err(format!("key '{key}' is given twice"));
        }
        *slot = Some(value.to_string());
    }
    let missing = |key: &str| format!("no '{key}'");
    let spec = VirtualSpec {
        uid: uid.ok_or_else(|| missing("uid"))?,
        name: name.ok_or_else(|| missing("name"))?,
        channels: channels.ok_or_else(|| missing("channels"))?.parse().map_err(|_| "channels is not a whole number".to_string())?,
        sample_rate: rate.ok_or_else(|| missing("sample_rate"))?.parse().map_err(|_| "sample_rate is not a number".to_string())?,
    };
    spec.validate()?;
    Ok(spec)
}

struct Published {
    spec: VirtualSpec,
    pair: LoopbackDriver,
}

pub struct PluginDriver {
    clock: ClockFn,
    devices: Vec<Published>,
    events: Vec<DriverEvent>,
    refused: Vec<(String, String)>,
}

impl PluginDriver {
    pub fn new(clock: ClockFn) -> PluginDriver {
        PluginDriver { clock, devices: Vec::new(), events: Vec::new(), refused: Vec::new() }
    }

    /// A driver on the process's monotonic clock.
    pub fn realtime() -> PluginDriver {
        static BASE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let base = *BASE.get_or_init(std::time::Instant::now);
        PluginDriver::new(std::sync::Arc::new(move || base.elapsed().as_nanos() as u64))
    }

    /// Publishes a device. Refused when the description is invalid or its uid is already published.
    pub fn publish(&mut self, spec: VirtualSpec) -> Result<(), String> {
        spec.validate()?;
        if self.devices.iter().any(|d| d.spec.uid == spec.uid) {
            return Err(format!("virtual device {} is already published", spec.uid));
        }
        let pair = LoopbackDriver::named(self.clock.clone(), &spec.out_uid(), &spec.in_uid(), &spec.name, spec.channels as usize);
        let mut published = Published { spec, pair };
        let mut added = Vec::new();
        Self::enumerate_one(&mut published, &mut added);
        self.events.extend(added.into_iter().map(DriverEvent::DeviceAdded));
        self.devices.push(published);
        Ok(())
    }

    fn enumerate_one(p: &mut Published, out: &mut Vec<DeviceDesc>) {
        let start = out.len();
        p.pair.enumerate(out);
        for d in &mut out[start..] {
            d.rates = vec![(p.spec.sample_rate, p.spec.sample_rate)];
        }
    }

    /// Removes a published device. True when there was one.
    pub fn unpublish(&mut self, uid: &str) -> bool {
        let Some(i) = self.devices.iter().position(|d| d.spec.uid == uid) else { return false };
        let gone = self.devices.remove(i);
        self.events.push(DriverEvent::DeviceRemoved { uid: gone.spec.out_uid() });
        self.events.push(DriverEvent::DeviceRemoved { uid: gone.spec.in_uid() });
        true
    }

    /// Reads every `*.vdev` file of `dir` in name order and publishes it. Returns how many were published; the
    /// others are in [`PluginDriver::refused`]. A folder that cannot be read publishes nothing.
    pub fn load_dir(&mut self, dir: &Path) -> usize {
        let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
        let mut files: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == EXTENSION)).collect();
        files.sort();
        let mut published = 0;
        for path in files {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let result = std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| parse_descriptor(&t)).and_then(|s| self.publish(s));
            match result {
                Ok(()) => published += 1,
                Err(why) => self.refused.push((name, why)),
            }
        }
        published
    }

    /// Files (or devices) that were refused, with the reason.
    pub fn refused(&self) -> &[(String, String)] {
        &self.refused
    }

    pub fn published(&self) -> Vec<&VirtualSpec> {
        self.devices.iter().map(|d| &d.spec).collect()
    }

    fn owner(&mut self, uid: &str) -> Option<&mut Published> {
        self.devices.iter_mut().find(|d| d.spec.out_uid() == uid || d.spec.in_uid() == uid)
    }
}

impl Driver for PluginDriver {
    fn name(&self) -> &str {
        "plugin"
    }

    fn enumerate(&mut self, out: &mut Vec<DeviceDesc>) {
        for p in &mut self.devices {
            Self::enumerate_one(p, out);
        }
    }

    fn open(&mut self, uid: &str, cfg: &IoConfig) -> Result<Box<dyn DeviceIo>, DriverError> {
        match self.owner(uid) {
            Some(p) => p.pair.open(uid, cfg),
            None => Err(DriverError::NotFound),
        }
    }

    fn poll_events(&mut self, out: &mut Vec<DriverEvent>) {
        out.append(&mut self.events);
    }

    fn controls(&mut self, _uid: &str) -> Vec<ControlDesc> {
        Vec::new()
    }

    fn set_control(&mut self, _uid: &str, _id: ControlId, _v: ControlValue) -> Result<(), DriverError> {
        Err(DriverError::Unsupported("a virtual device has no controls".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::null::ManualClock;
    use crate::Xrun;
    use bifrost_audio_types::{Buffer, BufferList};

    fn spec(uid: &str, channels: u32) -> VirtualSpec {
        VirtualSpec { uid: uid.into(), name: format!("Virtual {uid}"), channels, sample_rate: 48_000.0 }
    }

    fn cfg() -> IoConfig {
        IoConfig { rate: 48_000.0, quantum: 4, input: true, output: true }
    }

    fn bl(planes: &[&[f32]]) -> BufferList {
        BufferList { buffers: planes.iter().map(|p| Buffer { channels: 1, data: p.to_vec() }).collect() }
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-plugin-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn descriptors_are_parsed_and_checked() {
        let ok = parse_descriptor("# a bus\nuid=bus1\nname=Bus One\nchannels=4\nsample_rate=96000\n").unwrap();
        assert_eq!((ok.uid.as_str(), ok.name.as_str(), ok.channels, ok.sample_rate), ("bus1", "Bus One", 4, 96_000.0));
        assert!(parse_descriptor("uid=a\nname=b\nchannels=2\n").is_err());
        assert!(parse_descriptor("uid=a\nuid=b\nname=b\nchannels=2\nsample_rate=48000\n").is_err());
        assert!(parse_descriptor("uid=a\nname=b\nchannels=2\nsample_rate=48000\nvolume=3\n").is_err());
        assert!(parse_descriptor("uid=a\nname=b\nchannels=0\nsample_rate=48000\n").is_err());
        assert!(parse_descriptor("uid=a\nname=b\nchannels=2\nsample_rate=12345\n").is_err());
        assert!(parse_descriptor("uid=a:b\nname=b\nchannels=2\nsample_rate=48000\n").is_err());
    }

    #[test]
    fn a_published_device_is_a_playback_and_a_capture_device_with_its_own_shape() {
        let mut d = PluginDriver::new(ManualClock::new().as_fn());
        d.publish(spec("bus1", 4)).unwrap();
        let mut devs = Vec::new();
        d.enumerate(&mut devs);
        assert_eq!(devs.len(), 2);
        assert_eq!((devs[0].uid.as_str(), devs[0].output_channels, devs[0].input_channels), ("bus1:out", 4, 0));
        assert_eq!((devs[1].uid.as_str(), devs[1].output_channels, devs[1].input_channels), ("bus1:in", 0, 4));
        assert_eq!(devs[0].rates, vec![(48_000.0, 48_000.0)]);
        assert_eq!(devs[0].name, "Virtual bus1 (playback)");
        assert_eq!(devs[0].transport, "virtual");
    }

    #[test]
    fn what_is_played_to_the_device_comes_out_of_its_capture_side() {
        let mut d = PluginDriver::new(ManualClock::new().as_fn());
        d.publish(spec("bus1", 2)).unwrap();
        let mut out = d.open("bus1:out", &cfg()).unwrap();
        let mut inp = d.open("bus1:in", &cfg()).unwrap();
        out.write_mix(4, &bl(&[&[1.0, 2.0, 3.0, 4.0], &[5.0, 6.0, 7.0, 8.0]])).unwrap();
        let mut dst = bl(&[&[9.0; 4], &[9.0; 4]]);
        inp.read_input(4, &mut dst).unwrap();
        inp.read_input(4, &mut dst).unwrap();
        assert_eq!(dst.buffers[0].data, vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(dst.buffers[1].data, vec![5.0, 6.0, 7.0, 8.0]);
        assert_eq!(inp.read_input(4, &mut dst), Err(Xrun { frames_lost: 4 }));
        assert_eq!(d.open("nope:in", &cfg()).err(), Some(DriverError::NotFound));
    }

    #[test]
    fn two_devices_do_not_share_audio() {
        let mut d = PluginDriver::new(ManualClock::new().as_fn());
        d.publish(spec("a", 1)).unwrap();
        d.publish(spec("b", 1)).unwrap();
        let mut out_a = d.open("a:out", &cfg()).unwrap();
        let mut in_b = d.open("b:in", &cfg()).unwrap();
        out_a.write_mix(4, &bl(&[&[1.0; 4]])).unwrap();
        let mut dst = bl(&[&[9.0; 4]]);
        assert_eq!(in_b.read_input(4, &mut dst), Err(Xrun { frames_lost: 4 }), "b's capture side heard nothing");
    }

    #[test]
    fn publishing_and_removing_are_reported_as_events() {
        let mut d = PluginDriver::new(ManualClock::new().as_fn());
        d.publish(spec("bus1", 2)).unwrap();
        assert!(d.publish(spec("bus1", 2)).is_err(), "a uid is published once");
        assert!(d.publish(spec("bad uid", 2)).is_err());
        let mut ev = Vec::new();
        d.poll_events(&mut ev);
        assert_eq!(ev.len(), 2);
        assert!(matches!(&ev[0], DriverEvent::DeviceAdded(x) if x.uid == "bus1:out"));
        assert!(d.unpublish("bus1"));
        assert!(!d.unpublish("bus1"));
        ev.clear();
        d.poll_events(&mut ev);
        assert_eq!(ev, vec![DriverEvent::DeviceRemoved { uid: "bus1:out".into() }, DriverEvent::DeviceRemoved { uid: "bus1:in".into() }]);
        assert_eq!(d.open("bus1:out", &cfg()).err(), Some(DriverError::NotFound));
        assert!(d.published().is_empty());
    }

    #[test]
    fn a_folder_of_descriptions_is_loaded_and_bad_files_are_listed() {
        let dir = tmp("dir");
        std::fs::write(dir.join("10-main.vdev"), "uid=main\nname=Main Bus\nchannels=2\nsample_rate=48000\n").unwrap();
        std::fs::write(dir.join("20-broken.vdev"), "uid=broken\nname=Broken\n").unwrap();
        std::fs::write(dir.join("30-dup.vdev"), "uid=main\nname=Again\nchannels=2\nsample_rate=48000\n").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a description").unwrap();
        let mut d = PluginDriver::new(ManualClock::new().as_fn());
        assert_eq!(d.load_dir(&dir), 1);
        assert_eq!(d.published().len(), 1);
        let refused: Vec<&str> = d.refused().iter().map(|(f, _)| f.as_str()).collect();
        assert_eq!(refused, vec!["20-broken.vdev", "30-dup.vdev"]);
        assert_eq!(d.load_dir(&dir.join("missing")), 0);
    }
}
