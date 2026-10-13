//! Aggregate devices (`AudioHardwareCreateAggregateDevice`) and the hardware clock domain
//! (`kAudioDevicePropertyClockDomain`), as a pure model. The description serialises with the same keys as the reference platform's
//! `kAudioAggregateDevice*Key` dictionary. Not implemented: drift compensation resampling (the flag is carried and
//! reported, nothing corrects drift) and the audiod command that would create the device.
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
