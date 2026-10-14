//! Aggregate devices (`AudioHardwareCreateAggregateDevice`) and the hardware clock domain
//! (`kAudioDevicePropertyClockDomain`), as a pure model. The description serialises with the same keys as the reference platform's
//! `kAudioAggregateDevice*Key` dictionary. The model also computes the drift correction of each flagged sub-device
//! (`drift_corrections`) and the sample position of a clock at a host time (`SampleClock`). Not implemented: the audio
//! path does not yet resample by those ratios, and the audiod command that would create the device does not exist.
use bifrost_core::json::Json;

/// `kAudioDevicePropertyClockDomain` ('clkd').
pub const PROPERTY_CLOCK_DOMAIN: u32 = u32::from_be_bytes(*b"clkd");

/// A member of an aggregate device (`kAudioSubDeviceUIDKey`, `kAudioSubDeviceDriftCompensationKey`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubDevice {
    pub uid: String,
    pub drift_compensation: bool,
}

/// The description passed to `AudioHardwareCreateAggregateDevice`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AggregateDescription {
    /// `kAudioAggregateDeviceNameKey`
    pub name: String,
    /// `kAudioAggregateDeviceUIDKey`
    pub uid: String,
    /// `kAudioAggregateDeviceSubDeviceListKey`
    pub sub_devices: Vec<SubDevice>,
    /// `kAudioAggregateDeviceMainSubDeviceKey` (the clock source sub-device uid)
    pub main_sub_device: Option<String>,
    /// `kAudioAggregateDeviceClockDeviceKey`
    pub clock_device: Option<String>,
    /// `kAudioAggregateDeviceIsPrivateKey`
    pub private: bool,
    /// `kAudioAggregateDeviceIsStackedKey`
    pub stacked: bool,
}

impl AggregateDescription {
    /// Checks what the HAL checks before it creates the device.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("the aggregate device has no name".into());
        }
        if self.uid.is_empty() {
            return Err("the aggregate device has no uid".into());
        }
        if self.sub_devices.is_empty() {
            return Err("the aggregate device has no sub-devices".into());
        }
        for (i, s) in self.sub_devices.iter().enumerate() {
            if s.uid.is_empty() {
                return Err("a sub-device has no uid".into());
            }
            if self.sub_devices[..i].iter().any(|o| o.uid == s.uid) {
                return Err(format!("sub-device {} is listed twice", s.uid));
            }
        }
        if let Some(m) = &self.main_sub_device {
            if !self.sub_devices.iter().any(|s| &s.uid == m) {
                return Err(format!("main sub-device {m} is not in the sub-device list"));
            }
        }
        Ok(())
    }

    /// The uid whose clock drives the aggregate: the clock device, else the main sub-device, else the first one.
    pub fn clock_source(&self) -> Option<&str> {
        self.clock_device
            .as_deref()
            .or(self.main_sub_device.as_deref())
            .or(self.sub_devices.first().map(|s| s.uid.as_str()))
    }

    /// The dictionary the reference platform's keys describe.
    pub fn to_json(&self) -> Json {
        let subs = self
            .sub_devices
            .iter()
            .map(|s| obj(vec![("uid", Json::Str(s.uid.clone())), ("drift", Json::Bool(s.drift_compensation))]))
            .collect();
        let mut pairs = vec![
            ("name", Json::Str(self.name.clone())),
            ("uid", Json::Str(self.uid.clone())),
            ("subdevices", Json::Arr(subs)),
            ("private", Json::Bool(self.private)),
            ("stacked", Json::Bool(self.stacked)),
        ];
        if let Some(m) = &self.main_sub_device {
            pairs.push(("master", Json::Str(m.clone())));
        }
        if let Some(c) = &self.clock_device {
            pairs.push(("clock", Json::Str(c.clone())));
        }
        obj(pairs)
    }

    /// Reads a dictionary back; the result is validated.
    pub fn from_json(v: &Json) -> Result<Self, String> {
        let subs = v.get("subdevices").and_then(|d| d.as_array()).ok_or_else(|| "no subdevices".to_string())?;
        let sub_devices = subs
            .iter()
            .map(|s| {
                Ok(SubDevice {
                    uid: s.str_field("uid").ok_or_else(|| "a sub-device has no uid".to_string())?,
                    drift_compensation: matches!(s.get("drift"), Some(Json::Bool(true))),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let flag = |k: &str| matches!(v.get(k), Some(Json::Bool(true)));
        let d = AggregateDescription {
            name: v.str_field("name").unwrap_or_default(),
            uid: v.str_field("uid").unwrap_or_default(),
            sub_devices,
            main_sub_device: v.str_field("master"),
            clock_device: v.str_field("clock"),
            private: flag("private"),
            stacked: flag("stacked"),
        };
        d.validate()?;
        Ok(d)
    }
}

/// A hardware clock domain: devices with the same non-zero domain share a clock (zero means unknown).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockDomain(pub u32);

impl ClockDomain {
    /// Whether `other` runs from the same hardware clock.
    pub fn is_synchronized_with(self, other: ClockDomain) -> bool {
        self.0 != 0 && self.0 == other.0
    }
}

/// Which sub-devices need drift compensation against the clock source: those in another domain than it. The clock
/// source itself never does. `domains` maps sub-device uid to its domain.
pub fn drift_compensation_needed(desc: &AggregateDescription, domains: &[(String, ClockDomain)]) -> Vec<String> {
    let Some(source) = desc.clock_source() else { return Vec::new() };
    let domain_of = |uid: &str| domains.iter().find(|(u, _)| u == uid).map(|(_, d)| *d).unwrap_or(ClockDomain(0));
    let base = domain_of(source);
    desc.sub_devices
        .iter()
        .filter(|s| s.uid != source && !domain_of(&s.uid).is_synchronized_with(base))
        .map(|s| s.uid.clone())
        .collect()
}

/// The clock facts of one sub-device: its hardware domain and the nominal sample rate it runs at.
#[derive(Clone, Debug, PartialEq)]
pub struct SubDeviceClock {
    pub uid: String,
    pub domain: ClockDomain,
    pub sample_rate: f64,
}

/// A sample-rate conversion that brings a flagged sub-device onto the clock source's timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct DriftCorrection {
    pub uid: String,
    /// Source rate over sub-device rate: the factor a stream from this sub-device is resampled by.
    pub ratio: f64,
}

/// The sample-rate conversion factor from `sub_rate` to `source_rate`. Both rates must be positive and finite.
pub fn resample_ratio(source_rate: f64, sub_rate: f64) -> Result<f64, String> {
    if !is_positive_rate(source_rate) {
        return Err(format!("source rate {source_rate} is not a positive rate"));
    }
    if !is_positive_rate(sub_rate) {
        return Err(format!("sub-device rate {sub_rate} is not a positive rate"));
    }
    Ok(source_rate / sub_rate)
}

/// The drift corrections an aggregate needs: each sub-device flagged for drift compensation whose clock is in another
/// domain than the clock source. The clock source never gets one. A flagged sub-device without a known clock is an error,
/// because its ratio cannot be computed.
pub fn drift_corrections(desc: &AggregateDescription, clocks: &[SubDeviceClock]) -> Result<Vec<DriftCorrection>, String> {
    desc.validate()?;
    let Some(source_uid) = desc.clock_source() else { return Ok(Vec::new()) };
    let domains: Vec<(String, ClockDomain)> = clocks.iter().map(|c| (c.uid.clone(), c.domain)).collect();
    let flagged = |uid: &str| desc.sub_devices.iter().any(|s| s.uid == uid && s.drift_compensation);
    let clock_of = |uid: &str| {
        clocks.iter().find(|c| c.uid == uid).ok_or_else(|| format!("no clock is known for sub-device {uid}"))
    };
    let mut out = Vec::new();
    for uid in drift_compensation_needed(desc, &domains).iter().filter(|uid| flagged(uid)) {
        let source = clock_of(source_uid)?;
        let sub = clock_of(uid)?;
        out.push(DriftCorrection { uid: uid.clone(), ratio: resample_ratio(source.sample_rate, sub.sample_rate)? });
    }
    Ok(out)
}

fn is_positive_rate(rate: f64) -> bool {
    rate.is_finite() && rate > 0.0
}

/// A sample-time anchor: at `host_ticks` on the host time base, the clock was at sample frame `sample`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockAnchor {
    pub host_ticks: u64,
    pub sample: u64,
}

/// A hardware sample clock: an anchor and the nominal rate it runs at. Two clocks that run at different rates drift apart;
/// `skew` measures by how many frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SampleClock {
    pub anchor: ClockAnchor,
    pub sample_rate: f64,
}

impl SampleClock {
    /// A clock anchored at `anchor` and running at `sample_rate` frames per second.
    pub fn new(anchor: ClockAnchor, sample_rate: f64) -> Result<Self, String> {
        if !is_positive_rate(sample_rate) {
            return Err(format!("sample rate {sample_rate} is not a positive rate"));
        }
        Ok(SampleClock { anchor, sample_rate })
    }

    /// The (fractional) sample frame reached at `host_ticks`; `ticks_per_second` is the host time base frequency.
    pub fn sample_at(&self, host_ticks: u64, ticks_per_second: f64) -> f64 {
        let elapsed = host_ticks as f64 - self.anchor.host_ticks as f64;
        self.anchor.sample as f64 + elapsed * self.sample_rate / ticks_per_second
    }

    /// The host time at which this clock reaches `sample`, clamped at host time zero.
    pub fn host_at(&self, sample: u64, ticks_per_second: f64) -> u64 {
        let elapsed = (sample as f64 - self.anchor.sample as f64) * ticks_per_second / self.sample_rate;
        (self.anchor.host_ticks as f64 + elapsed).max(0.0).round() as u64
    }

    /// Frames by which `other` is ahead of this clock at `host_ticks`. A positive value means `other` runs fast.
    pub fn skew(&self, other: &SampleClock, host_ticks: u64, ticks_per_second: f64) -> f64 {
        other.sample_at(host_ticks, ticks_per_second) - self.sample_at(host_ticks, ticks_per_second)
    }
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(uid: &str, drift: bool) -> SubDevice {
        SubDevice { uid: uid.into(), drift_compensation: drift }
    }

    fn desc() -> AggregateDescription {
        AggregateDescription {
            name: "Call".into(),
            uid: "agg-1".into(),
            sub_devices: vec![sub("mic", true), sub("speaker", false)],
            main_sub_device: Some("speaker".into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_valid_description_passes() {
        assert!(desc().validate().is_ok());
    }

    #[test]
    fn invalid_descriptions_are_refused() {
        let mut d = desc();
        d.sub_devices.clear();
        assert!(d.validate().is_err());
        let mut d = desc();
        d.main_sub_device = Some("nope".into());
        assert!(d.validate().is_err());
        let mut d = desc();
        d.sub_devices.push(sub("mic", false));
        assert!(d.validate().is_err());
        let mut d = desc();
        d.name.clear();
        assert!(d.validate().is_err());
    }

    #[test]
    fn it_serialises_with_reference_keys_and_round_trips() {
        let d = desc();
        let j = d.to_json();
        assert_eq!(j.str_field("master").as_deref(), Some("speaker"));
        assert_eq!(j.get("subdevices").and_then(|s| s.as_array()).map(|a| a.len()), Some(2));
        assert_eq!(AggregateDescription::from_json(&j).unwrap(), d);
    }

    #[test]
    fn clock_source_prefers_clock_device_then_main_then_first() {
        let mut d = desc();
        assert_eq!(d.clock_source(), Some("speaker"));
        d.clock_device = Some("mic".into());
        assert_eq!(d.clock_source(), Some("mic"));
        d.clock_device = None;
        d.main_sub_device = None;
        assert_eq!(d.clock_source(), Some("mic"));
    }

    #[test]
    fn drift_is_needed_only_across_clock_domains() {
        let d = desc();
        let same = [("mic".to_string(), ClockDomain(7)), ("speaker".to_string(), ClockDomain(7))];
        assert!(drift_compensation_needed(&d, &same).is_empty());
        let apart = [("mic".to_string(), ClockDomain(3)), ("speaker".to_string(), ClockDomain(7))];
        assert_eq!(drift_compensation_needed(&d, &apart), vec!["mic".to_string()]);
        assert!(!ClockDomain(0).is_synchronized_with(ClockDomain(0)));
        assert_eq!(PROPERTY_CLOCK_DOMAIN, 0x636c_6b64);
    }
}
