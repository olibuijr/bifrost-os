//! The radio detail of a Wi-Fi interface, as `iw dev <name> info` reports it: the mode (managed, AP, ad-hoc), the
//! channel with its frequency and width, and the transmit power. [`parse`] reads the text; [`read`] runs `iw`.

use std::process::Command;

/// What `iw dev <name> info` says about a radio. A field the output does not carry is `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Phy {
    /// The interface type: `managed`, `AP`, `IBSS` (ad-hoc) and so on, as iw names it.
    pub mode: Option<String>,
    pub channel: Option<u16>,
    pub frequency_mhz: Option<u32>,
    /// The channel width in MHz (20, 40, 80, 160, 320).
    pub width_mhz: Option<u16>,
    /// The transmit power in dBm.
    pub tx_power_dbm: Option<f64>,
}

/// Reads the `iw dev <name> info` text. Lines it does not know are skipped.
pub fn parse(text: &str) -> Phy {
    let mut phy = Phy::default();
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("type ") {
            phy.mode = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("channel ") {
            phy.channel = first_token(value).and_then(|t| t.parse().ok());
            // `channel 36 (5180 MHz), width: 80 MHz, center1: 5210 MHz`
            phy.frequency_mhz = value
                .split_once('(')
                .and_then(|(_, rest)| rest.split_once(" MHz"))
                .and_then(|(num, _)| num.trim().parse().ok());
            phy.width_mhz = value
                .split_once("width:")
                .and_then(|(_, rest)| first_token(rest))
                .and_then(|t| t.parse().ok());
        } else if let Some(value) = line.strip_prefix("txpower ") {
            phy.tx_power_dbm = first_token(value).and_then(|t| t.parse().ok());
        }
    }
    phy
}

fn first_token(text: &str) -> Option<&str> {
    text.split_whitespace().next()
}

/// Whether `name` can be an interface name: 1 to 15 bytes of letters, digits, `-`, `_` or `.`.
pub fn valid_interface(name: &str) -> bool {
    !name.is_empty() && name.len() <= 15 && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Runs `iw dev <name> info` (with a timeout) and parses it. None when the name is not an interface name or iw
/// fails.
pub fn read(name: &str) -> Option<Phy> {
    if !valid_interface(name) {
        return None;
    }
    let out = Command::new("timeout").args(["5", "iw", "dev", name, "info"]).output().ok()?;
    out.status.success().then(|| parse(&String::from_utf8_lossy(&out.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: &str = "Interface wlan0\n\tifindex 3\n\twdev 0x1\n\taddr aa:bb:cc:dd:ee:ff\n\tssid Hus\n\ttype managed\n\twiphy 0\n\tchannel 36 (5180 MHz), width: 80 MHz, center1: 5210 MHz\n\ttxpower 20.00 dBm\n";

    #[test]
    fn a_managed_radio_reports_channel_frequency_width_and_power() {
        let phy = parse(WIDE);
        assert_eq!(phy.mode.as_deref(), Some("managed"));
        assert_eq!(phy.channel, Some(36));
        assert_eq!(phy.frequency_mhz, Some(5180));
        assert_eq!(phy.width_mhz, Some(80));
        assert_eq!(phy.tx_power_dbm, Some(20.0));
    }

    #[test]
    fn a_2_4_ghz_channel_without_ht_reads_its_width() {
        let phy = parse("\ttype AP\n\tchannel 6 (2437 MHz), width: 20 MHz (no HT), center1: 2437 MHz\n");
        assert_eq!(phy.mode.as_deref(), Some("AP"));
        assert_eq!(phy.channel, Some(6));
        assert_eq!(phy.frequency_mhz, Some(2437));
        assert_eq!(phy.width_mhz, Some(20));
    }

    #[test]
    fn a_radio_not_on_a_channel_has_no_channel_detail() {
        let phy = parse("\ttype managed\n\ttxpower 0.00 dBm\n");
        assert_eq!(phy.channel, None);
        assert_eq!(phy.frequency_mhz, None);
        assert_eq!(phy.width_mhz, None);
        assert_eq!(phy.tx_power_dbm, Some(0.0));
    }

    #[test]
    fn only_plain_interface_names_are_read() {
        assert!(valid_interface("wlan0"));
        assert!(valid_interface("wl-p.1"));
        assert!(!valid_interface(""));
        assert!(!valid_interface("wlan0; reboot"));
        assert!(!valid_interface("../wlan0"));
        assert!(!valid_interface("wlan0000000000000000"));
        assert_eq!(read("wlan0; reboot"), None);
    }
}
