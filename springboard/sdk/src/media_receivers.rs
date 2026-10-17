//! Streaming receivers on the local network (AirPlay style): the model an app reads to list them and to see what each one
//! advertises.
//!
//! The receivers come from a DNS-SD browse of `_airplay._tcp` (see `service_discovery::ServiceBrowser`). [`receivers`]
//! reads the TXT record of each instance into a [`Receiver`]: its name, host, port, device id, model and the feature bits
//! it advertises. [`advertisement_txt`] builds the same record for a receiver to publish. This module never connects to a
//! receiver and never streams: the sender and receiver services that do that are not built yet.

use crate::service_discovery::ServiceInstance;
use bifrost_core::txt_record::{Presence, TxtError, TxtRecord};

/// The DNS-SD service type of streaming receivers.
pub const SERVICE_TYPE: &str = "_airplay._tcp";

/// One receiver, as its advertisement describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receiver {
    pub name: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub device_id: String,
    pub model: String,
    pub features: u64,
}

impl Receiver {
    /// Whether the receiver advertises feature bit `bit` (0 to 63). A bit past 63 is never advertised.
    pub fn supports(&self, bit: u32) -> bool {
        bit < 64 && self.features & (1u64 << bit) != 0
    }
}

/// Parses a `features` TXT value: two 32-bit hexadecimal words, low then high, such as `0x1C,0x0`. A single word is the
/// low half alone. The value is `high << 32 | low`. None when there are more than two words or a word is not hexadecimal.
pub fn parse_features(text: &str) -> Option<u64> {
    let mut words = text.split(',');
    let low = parse_word(words.next()?)?;
    let high = match words.next() {
        Some(word) => parse_word(word)?,
        None => 0,
    };
    if words.next().is_some() {
        return None;
    }
    Some((u64::from(high) << 32) | u64::from(low))
}

/// The TXT value for a feature set: the low word, then the high word, each as `0x` and upper-case hex.
pub fn format_features(features: u64) -> String {
    format!("0x{:X},0x{:X}", features & 0xFFFF_FFFF, features >> 32)
}

fn parse_word(word: &str) -> Option<u32> {
    let word = word.trim();
    let digits = word.strip_prefix("0x").or_else(|| word.strip_prefix("0X")).unwrap_or(word);
    u32::from_str_radix(digits, 16).ok()
}

/// The receiver a browse instance describes. None when the record lacks a `deviceid` or `features` value, or the features
/// are malformed. A missing `model` reads as an empty string.
pub fn receiver_from_instance(instance: &ServiceInstance) -> Option<Receiver> {
    let text = |key: &str| match instance.txt.get(key) {
        Presence::Value(value) => std::str::from_utf8(value).ok().map(str::to_string),
        _ => None,
    };
    let features = parse_features(&text("features")?)?;
    let device_id = text("deviceid")?;
    Some(Receiver {
        name: instance.name.clone(),
        host: instance.host.clone(),
        port: instance.port,
        device_id,
        model: text("model").unwrap_or_default(),
        features,
    })
}

/// The receivers among browse results, sorted by name. Only instances of [`SERVICE_TYPE`] count, and when two instances
/// carry the same device id the first one found is kept.
pub fn receivers(instances: &[ServiceInstance]) -> Vec<Receiver> {
    let mut out: Vec<Receiver> = Vec::new();
    for found in instances.iter().filter(|i| is_receiver_service(&i.service)) {
        let Some(receiver) = receiver_from_instance(found) else { continue };
        if !out.iter().any(|o| o.device_id == receiver.device_id) {
            out.push(receiver);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Whether a browsed service type is the receiver type, in any domain (`_airplay._tcp` or `_airplay._tcp.local`).
fn is_receiver_service(service: &str) -> bool {
    service.strip_prefix(SERVICE_TYPE).is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

/// The TXT record a receiver publishes: its device id, its model and its feature bits.
pub fn advertisement_txt(device_id: &str, model: &str, features: u64) -> Result<TxtRecord, TxtError> {
    let mut txt = TxtRecord::new();
    txt.set("deviceid", Some(device_id.as_bytes()))?;
    txt.set("features", Some(format_features(features).as_bytes()))?;
    txt.set("model", Some(model.as_bytes()))?;
    Ok(txt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(name: &str, service: &str, txt: TxtRecord) -> ServiceInstance {
        ServiceInstance {
            name: name.into(),
            full_name: format!("{name}.{service}"),
            service: service.into(),
            host: Some("den.local".into()),
            port: Some(7000),
            addresses: Vec::new(),
            txt,
        }
    }

    fn txt(device_id: &str, features: &str) -> TxtRecord {
        let mut txt = TxtRecord::new();
        txt.set("deviceid", Some(device_id.as_bytes())).unwrap();
        txt.set("features", Some(features.as_bytes())).unwrap();
        txt.set("model", Some("Den1,1".as_bytes())).unwrap();
        txt
    }

    #[test]
    fn features_read_as_low_then_high_hex_words() {
        assert_eq!(parse_features("0x1C,0x0"), Some(0x1C));
        assert_eq!(parse_features("0x0,0x1"), Some(1 << 32));
        assert_eq!(parse_features("0XFF"), Some(0xFF), "a single word is the low half");
        assert_eq!(parse_features(" 0x4 , 0x0 "), Some(4), "spaces around a word are ignored");
        assert_eq!(parse_features("0xZZ,0x0"), None);
        assert_eq!(parse_features("0x1,0x2,0x3"), None, "three words are not a feature set");
        assert_eq!(parse_features(""), None);
        assert_eq!(parse_features("0x"), None, "a prefix with no digits");
    }

    #[test]
    fn a_feature_set_round_trips_through_its_text() {
        let features = (5u64 << 32) | 0xDEAD_BEEF;
        assert_eq!(format_features(features), "0xDEADBEEF,0x5");
        assert_eq!(parse_features(&format_features(features)), Some(features));
    }

    #[test]
    fn a_receiver_reports_the_bits_it_advertises() {
        let receiver = Receiver { name: "Den".into(), host: None, port: None, device_id: "a".into(), model: String::new(), features: 0b101 };
        assert!(receiver.supports(0));
        assert!(!receiver.supports(1));
        assert!(receiver.supports(2));
        assert!(!receiver.supports(64), "bits past 63 are never advertised");
    }

    #[test]
    fn receivers_are_read_sorted_and_deduplicated_by_device_id() {
        let found = vec![
            instance("Living room", "_airplay._tcp.local", txt("bb", "0x4,0x0")),
            instance("Kitchen", "_airplay._tcp.local", txt("aa", "0x1,0x0")),
            instance("Kitchen again", "_airplay._tcp.local", txt("aa", "0x9,0x0")),
            instance("Printer", "_ipp._tcp.local", txt("cc", "0x1,0x0")),
            instance("No features", "_airplay._tcp.local", {
                let mut bare = TxtRecord::new();
                bare.set("deviceid", Some(b"dd".as_slice())).unwrap();
                bare
            }),
        ];
        let got = receivers(&found);
        let names: Vec<&str> = got.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Kitchen", "Living room"]);
        assert_eq!(got[0].features, 1, "the first instance with a device id is kept");
        assert_eq!(got[0].host.as_deref(), Some("den.local"));
        assert_eq!(got[0].port, Some(7000));
        assert_eq!(got[0].model, "Den1,1");
    }

    #[test]
    fn a_service_type_must_match_the_whole_label() {
        assert!(is_receiver_service("_airplay._tcp"));
        assert!(is_receiver_service("_airplay._tcp.local"));
        assert!(!is_receiver_service("_airplayx._tcp.local"));
        assert!(!is_receiver_service("_airplay._udp.local"));
    }

    #[test]
    fn an_advertisement_reads_back_as_the_same_receiver() {
        let txt = advertisement_txt("ab:cd", "Den1,1", (1u64 << 32) | 6).unwrap();
        let found = vec![instance("Den", "_airplay._tcp.local", txt)];
        let got = receivers(&found);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].device_id, "ab:cd");
        assert_eq!(got[0].features, (1u64 << 32) | 6);
        assert!(got[0].supports(1) && got[0].supports(2));
    }
}
