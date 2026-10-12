//! Wireless LAN scan data for apps: the channel, band and PHY mode of an access point, the security class
//! its flags advertise, and the merge of access points that share a network name.
//!
//! Pure functions over what the system layer already reads from the supplicant's scan (frequency in MHz,
//! signal in dBm, the bracketed flag text, the access point's BSSID and SSID). No service call is made here.
//! Channel width is not in the scan flags, so it is not reported.

/// The frequency band of an access point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    TwoPointFour,
    Five,
    Six,
}

impl Band {
    /// The band of a centre frequency in MHz, or `None` outside the three bands.
    pub fn from_freq(mhz: u32) -> Option<Band> {
        match mhz {
            2400..=2495 => Some(Band::TwoPointFour),
            5150..=5924 => Some(Band::Five),
            5925..=7125 => Some(Band::Six),
            _ => None,
        }
    }
}

/// The channel number for a centre frequency in MHz: 1 to 14 in the 2.4 GHz band, 36 to 177 at 5 GHz,
/// 1 to 233 at 6 GHz. `None` for a frequency that is not on a channel raster.
pub fn channel_for_freq(mhz: u32) -> Option<u16> {
    match mhz {
        2412..=2472 if (mhz - 2407) % 5 == 0 => Some(((mhz - 2407) / 5) as u16),
        2484 => Some(14),
        5150..=5924 if mhz % 5 == 0 => Some(((mhz - 5000) / 5) as u16),
        5955..=7115 if (mhz - 5950) % 5 == 0 => Some(((mhz - 5950) / 5) as u16),
        _ => None,
    }
}

/// The 802.11 PHY generation an access point advertises.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phy {
    /// 802.11a (5 GHz, no HT)
    A,
    /// 802.11b or 802.11g; the scan flags do not separate the two
    G,
    /// 802.11n (HT)
    N,
    /// 802.11ac (VHT)
    Ac,
    /// 802.11ax (HE)
    Ax,
}

impl Phy {
    pub fn label(self) -> &'static str {
        match self {
            Phy::A => "802.11a",
            Phy::G => "802.11b/g",
            Phy::N => "802.11n",
            Phy::Ac => "802.11ac",
            Phy::Ax => "802.11ax",
        }
    }

    /// The highest generation the flags name: `[HE]` is ax, `[VHT]` is ac, `[HT]` is n. Without any of these,
    /// a 5 or 6 GHz access point is a, and a 2.4 GHz one is b/g.
    pub fn from_flags(flags: &str, band: Option<Band>) -> Phy {
        if flags.contains("[HE") {
            Phy::Ax
        } else if flags.contains("[VHT") {
            Phy::Ac
        } else if flags.contains("[HT") {
            Phy::N
        } else if matches!(band, Some(Band::Five) | Some(Band::Six)) {
            Phy::A
        } else {
            Phy::G
        }
    }
}

/// The security an access point advertises, as the classes an app offers a password prompt for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityClass {
    Open,
    Wep,
    /// WPA (TKIP) personal, without WPA2
    WpaPersonal,
    Wpa2Personal,
    Wpa3Personal,
    /// WPA2 personal and WPA3 personal offered together (transition mode)
    Wpa2Wpa3Personal,
    /// 802.1X: needs a user name and password, not one shared key
    Enterprise,
}

impl SecurityClass {
    /// Classifies the bracketed flags of a scan line, for example `[WPA2-PSK-CCMP][ESS]`. Enterprise (EAP) is
    /// checked first, then SAE (WPA3) with or without PSK, then WPA2, WPA, WEP; anything else is open.
    pub fn from_flags(flags: &str) -> SecurityClass {
        let sae = flags.contains("SAE");
        let psk = flags.contains("PSK");
        if flags.contains("EAP") {
            SecurityClass::Enterprise
        } else if sae && psk {
            SecurityClass::Wpa2Wpa3Personal
        } else if sae {
            SecurityClass::Wpa3Personal
        } else if flags.contains("WPA2-PSK") {
            SecurityClass::Wpa2Personal
        } else if flags.contains("WPA-PSK") {
            SecurityClass::WpaPersonal
        } else if flags.contains("WEP") {
            SecurityClass::Wep
        } else {
            SecurityClass::Open
        }
    }

    /// True when the class takes one shared password.
    pub fn is_personal(self) -> bool {
        !matches!(self, SecurityClass::Open | SecurityClass::Enterprise)
    }
}

/// One access point from a scan.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub bssid: String,
    /// centre frequency, MHz
    pub freq: u32,
    /// received signal, dBm
    pub rssi: i32,
    /// the bracketed flag text of the scan line
    pub flags: String,
    pub ssid: String,
}

/// A network: all access points that share one SSID.
#[derive(Clone, Debug, PartialEq)]
pub struct Network {
    pub ssid: String,
    /// the access points' BSSIDs, strongest first
    pub bssids: Vec<String>,
    /// the strongest access point's signal, dBm
    pub rssi: i32,
    /// the strongest access point's channel
    pub channel: Option<u16>,
    pub band: Option<Band>,
    pub phy: Phy,
    /// the strongest access point's security; a network with mixed security reports this one
    pub security: SecurityClass,
}

/// Merges scan samples by SSID. Samples with an empty SSID (hidden networks) are dropped. The result is
/// strongest first: a network's position is that of its strongest access point, and ties keep the scan order.
pub fn merge(samples: &[Sample]) -> Vec<Network> {
    let mut order: Vec<&Sample> = samples.iter().filter(|s| !s.ssid.is_empty()).collect();
    order.sort_by(|a, b| b.rssi.cmp(&a.rssi));
    let mut out: Vec<Network> = Vec::new();
    for s in order {
        if let Some(i) = out.iter().position(|n| n.ssid == s.ssid) {
            out[i].bssids.push(s.bssid.clone());
        } else {
            let band = Band::from_freq(s.freq);
            out.push(Network {
                ssid: s.ssid.clone(),
                bssids: vec![s.bssid.clone()],
                rssi: s.rssi,
                channel: channel_for_freq(s.freq),
                band,
                phy: Phy::from_flags(&s.flags, band),
                security: SecurityClass::from_flags(&s.flags),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(bssid: &str, freq: u32, rssi: i32, flags: &str, ssid: &str) -> Sample {
        Sample { bssid: bssid.into(), freq, rssi, flags: flags.into(), ssid: ssid.into() }
    }

    #[test]
    fn channels_follow_the_raster() {
        assert_eq!(channel_for_freq(2412), Some(1));
        assert_eq!(channel_for_freq(2437), Some(6));
        assert_eq!(channel_for_freq(2472), Some(13));
        assert_eq!(channel_for_freq(2484), Some(14));
        assert_eq!(channel_for_freq(2415), None);
        assert_eq!(channel_for_freq(5180), Some(36));
        assert_eq!(channel_for_freq(5825), Some(165));
        assert_eq!(channel_for_freq(5955), Some(1));
        assert_eq!(channel_for_freq(7115), Some(233));
        assert_eq!(channel_for_freq(5927), None);
        assert_eq!(channel_for_freq(3000), None);
    }

    #[test]
    fn bands_split_at_the_edges() {
        assert_eq!(Band::from_freq(2437), Some(Band::TwoPointFour));
        assert_eq!(Band::from_freq(2495), Some(Band::TwoPointFour));
        assert_eq!(Band::from_freq(2500), None);
        assert_eq!(Band::from_freq(5500), Some(Band::Five));
        assert_eq!(Band::from_freq(6115), Some(Band::Six));
        assert_eq!(Band::from_freq(3000), None);
    }

    #[test]
    fn phy_takes_the_highest_flag() {
        assert_eq!(Phy::from_flags("[WPA2-PSK-CCMP][ESS][HT][VHT]", Some(Band::Five)), Phy::Ac);
        assert_eq!(Phy::from_flags("[ESS][HE][HT]", Some(Band::Six)), Phy::Ax);
        assert_eq!(Phy::from_flags("[WPA2-PSK-CCMP][HT40+]", Some(Band::Five)), Phy::N);
        assert_eq!(Phy::from_flags("[ESS]", Some(Band::Five)), Phy::A);
        assert_eq!(Phy::from_flags("[ESS]", Some(Band::TwoPointFour)), Phy::G);
        assert_eq!(Phy::from_flags("[ESS]", None), Phy::G);
        assert_eq!(Phy::Ac.label(), "802.11ac");
    }

    #[test]
    fn security_classes_from_flags() {
        assert_eq!(SecurityClass::from_flags("[WPA2-PSK-CCMP][ESS]"), SecurityClass::Wpa2Personal);
        assert_eq!(SecurityClass::from_flags("[WPA-PSK-TKIP][WPA2-PSK-CCMP]"), SecurityClass::Wpa2Personal);
        assert_eq!(SecurityClass::from_flags("[WPA-PSK-TKIP][ESS]"), SecurityClass::WpaPersonal);
        assert_eq!(SecurityClass::from_flags("[SAE][ESS]"), SecurityClass::Wpa3Personal);
        assert_eq!(SecurityClass::from_flags("[WPA2-PSK+SAE-CCMP][ESS]"), SecurityClass::Wpa2Wpa3Personal);
        assert_eq!(SecurityClass::from_flags("[WEP][ESS]"), SecurityClass::Wep);
        assert_eq!(SecurityClass::from_flags("[WPA2-EAP-CCMP][ESS]"), SecurityClass::Enterprise);
        assert_eq!(SecurityClass::from_flags("[ESS]"), SecurityClass::Open);
        assert!(SecurityClass::Wep.is_personal() && !SecurityClass::Open.is_personal());
        assert!(!SecurityClass::Enterprise.is_personal());
    }

    #[test]
    fn merge_groups_by_name_strongest_first() {
        let scan = [
            sample("02:00:00:00:00:01", 2412, -70, "[WPA2-PSK-CCMP][ESS]", "WIFI"),
            sample("02:00:00:00:00:02", 5180, -48, "[WPA2-PSK-CCMP][ESS][HT][VHT]", "WIFI"),
            sample("02:00:00:00:00:03", 2437, -80, "[ESS]", "Cafe"),
            sample("02:00:00:00:00:04", 2462, -55, "[WPA2-PSK-CCMP]", ""),
            sample("02:00:00:00:00:05", 2462, -66, "[WPA2-EAP-CCMP][ESS]", "Corp"),
        ];
        let nets = merge(&scan);
        assert_eq!(nets.len(), 3);
        let names: Vec<&str> = nets.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(names, ["WIFI", "Corp", "Cafe"]);
        let wifi = &nets[0];
        assert_eq!(wifi.bssids, ["02:00:00:00:00:02", "02:00:00:00:00:01"]);
        assert_eq!(wifi.rssi, -48);
        assert_eq!(wifi.channel, Some(36));
        assert_eq!(wifi.band, Some(Band::Five));
        assert_eq!(wifi.phy, Phy::Ac);
        assert_eq!(wifi.security, SecurityClass::Wpa2Personal);
        assert_eq!(nets[1].channel, Some(11));
        assert_eq!(nets[1].phy, Phy::G);
        assert_eq!(nets[1].security, SecurityClass::Enterprise);
        assert_eq!(nets[2].security, SecurityClass::Open);
    }

    #[test]
    fn merge_keeps_scan_order_on_equal_signal() {
        let scan = [
            sample("02:00:00:00:00:0a", 2412, -60, "[ESS]", "B"),
            sample("02:00:00:00:00:0b", 2412, -60, "[ESS]", "A"),
            sample("02:00:00:00:00:0c", 2412, -60, "[ESS]", "B"),
        ];
        let nets = merge(&scan);
        assert_eq!(nets.iter().map(|n| n.ssid.as_str()).collect::<Vec<_>>(), ["B", "A"]);
        assert_eq!(nets[0].bssids, ["02:00:00:00:00:0a", "02:00:00:00:00:0c"]);
        assert!(merge(&[]).is_empty());
    }
}
