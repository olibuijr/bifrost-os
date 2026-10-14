//! The pure model of Wireless Diagnostics: parsing of `/proc/net/wireless`, `iw dev <if> link` and `iw dev <if> scan`
//! output, the signal quality rating, the signal history for the chart, and the plain-text diagnostics report.
//! Nothing here touches the system; `source.rs` reads the text and hands it in.

use bifrost_app::wlan::{self, Band, Network, SecurityClass, Sample};
use std::collections::VecDeque;

/// How many signal readings the chart keeps (one per poll, about two minutes).
pub const HISTORY_LEN: usize = 120;

/// The kernel's marker for a value it does not have (wireless extensions report -256 dBm).
const NO_VALUE_DBM: i32 = -256;

/// One wireless interface as the kernel reports it in `/proc/net/wireless`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Iface {
    pub name: String,
    /// The link quality the driver reports (0 to 70 for most drivers).
    pub quality: Option<i32>,
    /// The received signal, dBm.
    pub level_dbm: Option<i32>,
    /// The noise floor, dBm.
    pub noise_dbm: Option<i32>,
}

/// Parses `/proc/net/wireless`. The two header lines carry no colon, so they are skipped without a special case.
/// Each interface line reads `name: status link level noise ...`, and the numbers may carry a trailing dot.
pub fn parse_proc_wireless(text: &str) -> Vec<Iface> {
    text.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let fields: Vec<&str> = rest.split_whitespace().collect();
            if fields.len() < 4 {
                return None;
            }
            Some(Iface {
                name: name.trim().to_string(),
                quality: kernel_number(fields[1]),
                level_dbm: dbm(fields[2]),
                noise_dbm: dbm(fields[3]),
            })
        })
        .collect()
}

/// A kernel number such as `-40.` or `70.`, without its trailing dot.
fn kernel_number(text: &str) -> Option<i32> {
    text.trim_end_matches('.').parse::<i32>().ok()
}

/// A dBm reading, or `None` for the kernel's "no value" marker.
fn dbm(text: &str) -> Option<i32> {
    kernel_number(text).filter(|v| *v > NO_VALUE_DBM)
}

/// The state of the link to an access point, from `iw dev <if> link`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Link {
    pub connected: bool,
    pub ssid: String,
    pub bssid: String,
    pub freq_mhz: Option<u32>,
    pub signal_dbm: Option<i32>,
    /// The receive rate in megabits per second.
    pub rx_mbps: Option<f32>,
    /// The transmit rate in megabits per second.
    pub tx_mbps: Option<f32>,
}

impl Link {
    /// The channel number of the link's frequency.
    pub fn channel(&self) -> Option<u16> {
        self.freq_mhz.and_then(wlan::channel_for_freq)
    }

    /// The band of the link's frequency.
    pub fn band(&self) -> Option<Band> {
        self.freq_mhz.and_then(Band::from_freq)
    }
}

/// Parses the output of `iw dev <if> link`. "Not connected." leaves the link disconnected.
pub fn parse_link(text: &str) -> Link {
    let mut link = Link::default();
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("Connected to ") {
            link.connected = true;
            link.bssid = rest.split_whitespace().next().unwrap_or("").to_string();
        } else if let Some(value) = line.strip_prefix("SSID:") {
            link.ssid = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("freq:") {
            link.freq_mhz = whole_number(value);
        } else if let Some(value) = line.strip_prefix("signal:") {
            link.signal_dbm = decimal_dbm(value);
        } else if let Some(value) = line.strip_prefix("rx bitrate:") {
            link.rx_mbps = bitrate_mbps(value);
        } else if let Some(value) = line.strip_prefix("tx bitrate:") {
            link.tx_mbps = bitrate_mbps(value);
        }
    }
    link
}

/// The integer part of a number such as `5180` or `5180.0`.
fn whole_number(text: &str) -> Option<u32> {
    text.trim().split('.').next()?.parse().ok()
}

/// A signal such as `-52 dBm` or `-41.00 dBm`, rounded to whole dBm.
fn decimal_dbm(text: &str) -> Option<i32> {
    let first = text.split_whitespace().next()?;
    first.parse::<f32>().ok().map(|v| v.round() as i32)
}

/// A rate such as `866.7 MBit/s VHT-MCS 9 80MHz`, in megabits per second.
fn bitrate_mbps(text: &str) -> Option<f32> {
    let mut parts = text.split_whitespace();
    let value: f32 = parts.next()?.parse().ok()?;
    (parts.next()? == "MBit/s").then_some(value)
}

/// An 802.1X (enterprise) suite. The scan names it either as EAP or by its standard number.
fn is_enterprise(suite: &str) -> bool {
    suite.contains("EAP") || suite.contains("802.1x")
}

/// One access point of a scan, before it is merged by network name.
struct Bss {
    bssid: String,
    freq: Option<u32>,
    signal: Option<i32>,
    ssid: String,
    privacy: bool,
    /// The authentication suites of the RSN (WPA2 or WPA3) block, when the access point has one.
    rsn: Option<Vec<String>>,
    /// The authentication suites of the legacy WPA block, when the access point has one.
    wpa: Option<Vec<String>>,
    /// Which block the current indented lines belong to.
    in_rsn: bool,
    in_wpa: bool,
}

impl Bss {
    fn new(header: &str) -> Bss {
        let bssid = header.split(|c: char| c == '(' || c.is_whitespace()).next().unwrap_or("").to_string();
        Bss { bssid, freq: None, signal: None, ssid: String::new(), privacy: false, rsn: None, wpa: None, in_rsn: false, in_wpa: false }
    }

    fn line(&mut self, line: &str) {
        if let Some(rest) = line.strip_prefix("* Authentication suites:") {
            let suites = rest.split_whitespace().map(str::to_string);
            if self.in_rsn {
                self.rsn.get_or_insert_with(Vec::new).extend(suites);
            } else if self.in_wpa {
                self.wpa.get_or_insert_with(Vec::new).extend(suites);
            }
            return;
        }
        if line.starts_with("* ") {
            // another field of the block that is open
            return;
        }
        self.in_rsn = line.starts_with("RSN:");
        self.in_wpa = line.starts_with("WPA:");
        if self.in_rsn {
            self.rsn.get_or_insert_with(Vec::new);
        }
        if self.in_wpa {
            self.wpa.get_or_insert_with(Vec::new);
        }
        if let Some(v) = line.strip_prefix("freq:") {
            self.freq = whole_number(v);
        } else if let Some(v) = line.strip_prefix("signal:") {
            self.signal = decimal_dbm(v);
        } else if let Some(v) = line.strip_prefix("SSID:") {
            self.ssid = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("capability:") {
            self.privacy = v.split_whitespace().any(|w| w == "Privacy");
        }
    }

    /// The bracketed flag text the security classifier reads, as the supplicant writes it.
    fn flags(&self) -> String {
        if let Some(suites) = &self.rsn {
            let eap = suites.iter().any(|s| is_enterprise(s));
            let psk = suites.iter().any(|s| s.contains("PSK"));
            let sae = suites.iter().any(|s| s.contains("SAE"));
            return if eap {
                "[WPA2-EAP]".into()
            } else if psk && sae {
                "[WPA2-PSK][SAE]".into()
            } else if sae {
                "[SAE]".into()
            } else {
                // WPA2 personal, and any suite this parser does not name
                "[WPA2-PSK]".into()
            };
        }
        if let Some(suites) = &self.wpa {
            return if suites.iter().any(|s| is_enterprise(s)) { "[WPA-EAP]".into() } else { "[WPA-PSK]".into() };
        }
        if self.privacy {
            "[WEP]".into()
        } else {
            String::new()
        }
    }

    fn finish(self) -> Option<Sample> {
        let freq = self.freq?;
        let rssi = self.signal?;
        let flags = self.flags();
        Some(Sample { bssid: self.bssid, freq, rssi, flags, ssid: self.ssid })
    }
}

/// Parses the output of `iw dev <if> scan` (or `scan dump`): one `BSS <address>` block per access point.
/// Access points without a frequency or a signal are dropped; hidden networks keep an empty name for the merge to skip.
pub fn parse_scan(text: &str) -> Vec<Sample> {
    let mut out = Vec::new();
    let mut current: Option<Bss> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(header) = line.strip_prefix("BSS ") {
            if let Some(done) = current.take().and_then(Bss::finish) {
                out.push(done);
            }
            current = Some(Bss::new(header));
        } else if let Some(bss) = current.as_mut() {
            bss.line(line);
        }
    }
    if let Some(done) = current.and_then(Bss::finish) {
        out.push(done);
    }
    out
}

/// The signal quality rating shown on the Connection card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rating {
    Excellent,
    Good,
    Fair,
    Poor,
    Unknown,
}

impl Rating {
    pub fn label(self) -> &'static str {
        match self {
            Rating::Excellent => "Excellent",
            Rating::Good => "Good",
            Rating::Fair => "Fair",
            Rating::Poor => "Poor",
            Rating::Unknown => "Unknown",
        }
    }
}

/// The signal-to-noise ratio in dB, when both readings are there.
pub fn snr_db(signal: Option<i32>, noise: Option<i32>) -> Option<i32> {
    Some(signal? - noise?)
}

/// Rates the link. With a noise floor the rating follows the signal-to-noise ratio (40 dB or more is excellent, 25 good,
/// 15 fair). Without one it follows the signal alone (-55 dBm or stronger is excellent, -67 good, -75 fair).
pub fn rate_link(signal: Option<i32>, noise: Option<i32>) -> Rating {
    match (snr_db(signal, noise), signal) {
        (Some(snr), _) => match snr {
            s if s >= 40 => Rating::Excellent,
            s if s >= 25 => Rating::Good,
            s if s >= 15 => Rating::Fair,
            _ => Rating::Poor,
        },
        (None, Some(s)) => match s {
            s if s >= -55 => Rating::Excellent,
            s if s >= -67 => Rating::Good,
            s if s >= -75 => Rating::Fair,
            _ => Rating::Poor,
        },
        (None, None) => Rating::Unknown,
    }
}

/// The signal readings of the last minutes, oldest first. A poll that found no signal keeps a gap.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    values: VecDeque<Option<i32>>,
}

impl History {
    pub fn push(&mut self, dbm: Option<i32>) {
        if self.values.len() == HISTORY_LEN {
            self.values.pop_front();
        }
        self.values.push_back(dbm);
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The readings, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = Option<i32>> + '_ {
        self.values.iter().copied()
    }

    /// The most recent reading that is there.
    pub fn latest(&self) -> Option<i32> {
        self.values.iter().rev().find_map(|v| *v)
    }

    /// The lowest, highest and mean of the readings that are there.
    pub fn summary(&self) -> Option<(i32, i32, f32)> {
        let known: Vec<i32> = self.values.iter().filter_map(|v| *v).collect();
        if known.is_empty() {
            return None;
        }
        let min = *known.iter().min()?;
        let max = *known.iter().max()?;
        let mean = known.iter().map(|v| *v as f32).sum::<f32>() / known.len() as f32;
        Some((min, max, mean))
    }
}

/// Where a signal sits on the chart: -100 dBm is the floor and -30 dBm the top, clamped to 0 to 1.
pub fn chart_fraction(dbm: i32) -> f32 {
    ((dbm + 100) as f32 / 70.0).clamp(0.0, 1.0)
}

/// The band as a name for people, such as `5 GHz`.
pub fn band_label(band: Option<Band>) -> &'static str {
    match band {
        Some(Band::TwoPointFour) => "2.4 GHz",
        Some(Band::Five) => "5 GHz",
        Some(Band::Six) => "6 GHz",
        None => "Unknown",
    }
}

/// The security of a network as a name for people.
pub fn security_label(security: SecurityClass) -> &'static str {
    match security {
        SecurityClass::Open => "None",
        SecurityClass::Wep => "WEP",
        SecurityClass::WpaPersonal => "WPA",
        SecurityClass::Wpa2Personal => "WPA2 Personal",
        SecurityClass::Wpa3Personal => "WPA3 Personal",
        SecurityClass::Wpa2Wpa3Personal => "WPA2/WPA3 Personal",
        SecurityClass::Enterprise => "Enterprise",
    }
}

/// `2026-10-13 09:30:00 UTC` for a Unix time in seconds (proleptic Gregorian calendar).
pub fn utc_stamp(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let rem = unix % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        year,
        month,
        day,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// The file name of a saved report for a Unix time, such as `wireless-diagnostics-1781342400.txt`.
pub fn report_file_name(unix: u64) -> String {
    format!("wireless-diagnostics-{unix}.txt")
}

fn value_or_none<T: std::fmt::Display>(v: Option<T>, unit: &str) -> String {
    match v {
        Some(v) if unit.is_empty() => v.to_string(),
        Some(v) => format!("{v} {unit}"),
        None => "Not available".to_string(),
    }
}

/// The plain-text diagnostics report: the link, its quality, the signal history and the nearby networks.
pub fn report(unix: u64, iface: &str, link: &Link, noise: Option<i32>, networks: &[Network], history: &History) -> String {
    let mut out = String::new();
    out.push_str("Wireless Diagnostics report\n");
    out.push_str(&format!("Generated: {} ({unix})\n", utc_stamp(unix)));
    out.push_str(&format!("Interface: {}\n\n", if iface.is_empty() { "none" } else { iface }));

    out.push_str("Connection\n");
    if link.connected {
        out.push_str(&format!("  Network: {}\n", if link.ssid.is_empty() { "Hidden network" } else { link.ssid.as_str() }));
        out.push_str(&format!("  BSSID: {}\n", link.bssid));
    } else {
        out.push_str("  Network: Not connected\n");
    }
    out.push_str(&format!("  Channel: {}\n", value_or_none(link.channel(), "")));
    out.push_str(&format!("  Band: {}\n", band_label(link.band())));
    out.push_str(&format!("  Signal: {}\n", value_or_none(link.signal_dbm, "dBm")));
    out.push_str(&format!("  Noise: {}\n", value_or_none(noise, "dBm")));
    let snr = snr_db(link.signal_dbm, noise);
    out.push_str(&format!("  Signal-to-noise: {}\n", value_or_none(snr, "dB")));
    out.push_str(&format!("  Quality: {}\n", rate_link(link.signal_dbm, noise).label()));
    out.push_str(&format!("  Receive rate: {}\n", value_or_none(link.rx_mbps.map(|v| format!("{v:.1}")), "Mbit/s")));
    out.push_str(&format!("  Transmit rate: {}\n\n", value_or_none(link.tx_mbps.map(|v| format!("{v:.1}")), "Mbit/s")));

    out.push_str("Signal history\n");
    match history.summary() {
        Some((min, max, mean)) => out.push_str(&format!(
            "  {} readings: lowest {min} dBm, highest {max} dBm, mean {mean:.1} dBm\n\n",
            history.len()
        )),
        None => out.push_str("  No readings\n\n"),
    }

    out.push_str("Nearby networks\n");
    if networks.is_empty() {
        out.push_str("  No scan results. Run a scan to list networks.\n");
    }
    for n in networks {
        out.push_str(&format!(
            "  {}: {} dBm, channel {}, {}, {}, {} access point(s), {}\n",
            n.ssid,
            n.rssi,
            n.channel.map_or("unknown".to_string(), |c| c.to_string()),
            band_label(n.band),
            n.phy.label(),
            n.bssids.len(),
            security_label(n.security)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROC: &str = "Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE\n\
 face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22\n\
  wlan0: 0000   70.  -40.  -256        0      0      0      0      0        0\n\
  wlp3s0: 0000   45.  -62.  -91        3      0      0      0      0        0\n";

    #[test]
    fn proc_wireless_reads_each_interface() {
        let ifaces = parse_proc_wireless(PROC);
        assert_eq!(ifaces.len(), 2);
        assert_eq!(ifaces[0].name, "wlan0");
        assert_eq!(ifaces[0].quality, Some(70));
        assert_eq!(ifaces[0].level_dbm, Some(-40));
        // -256 is the kernel's "no value" marker
        assert_eq!(ifaces[0].noise_dbm, None);
        assert_eq!(ifaces[1].noise_dbm, Some(-91));
    }

    #[test]
    fn proc_wireless_without_interfaces_is_empty() {
        assert!(parse_proc_wireless("Inter-| sta-|\n face | tus |\n").is_empty());
    }

    const LINK: &str = "Connected to 3c:52:82:aa:bb:cc (on wlan0)\n\
\tSSID: Home Net\n\
\tfreq: 5180\n\
\tRX: 123456 bytes (789 packets)\n\
\tTX: 654321 bytes (987 packets)\n\
\tsignal: -52 dBm\n\
\trx bitrate: 866.7 MBit/s VHT-MCS 9 80MHz short GI VHT-NSS 2\n\
\ttx bitrate: 400.2 MBit/s 40MHz\n\
\tbss flags:\tshort-slot-time\n";

    #[test]
    fn link_reads_the_connected_access_point() {
        let link = parse_link(LINK);
        assert!(link.connected);
        assert_eq!(link.bssid, "3c:52:82:aa:bb:cc");
        assert_eq!(link.ssid, "Home Net");
        assert_eq!(link.freq_mhz, Some(5180));
        assert_eq!(link.channel(), Some(36));
        assert_eq!(link.band(), Some(Band::Five));
        assert_eq!(link.signal_dbm, Some(-52));
        assert_eq!(link.rx_mbps, Some(866.7));
        assert_eq!(link.tx_mbps, Some(400.2));
    }

    #[test]
    fn link_not_connected_reads_empty() {
        let link = parse_link("Not connected.\n");
        assert!(!link.connected);
        assert_eq!(link.channel(), None);
        assert_eq!(link.signal_dbm, None);
    }

    #[test]
    fn link_signal_average_line_is_not_the_signal() {
        let link = parse_link("\tsignal avg: -60 dBm\n");
        assert_eq!(link.signal_dbm, None);
    }

    #[test]
    fn scan_reads_security_and_channels() {
        let text = "BSS 00:11:22:33:44:55(on wlan0) -- associated\n\
\tfreq: 2437.0\n\
\tcapability: ESS Privacy ShortSlotTime (0x0411)\n\
\tsignal: -41.00 dBm\n\
\tSSID: Cafe\n\
\tRSN:\t * Version: 1\n\
\t\t * Authentication suites: PSK\n\
BSS 00:11:22:33:44:66(on wlan0)\n\
\tfreq: 5500.0\n\
\tsignal: -70.00 dBm\n\
\tSSID: Office\n\
\tRSN:\t * Version: 1\n\
\t\t * Authentication suites: SAE\n\
BSS 00:11:22:33:44:77(on wlan0)\n\
\tfreq: 2412.0\n\
\tsignal: -80.00 dBm\n\
\tSSID: Old Router\n\
\tcapability: ESS Privacy (0x0011)\n\
BSS 00:11:22:33:44:88(on wlan0)\n\
\tfreq: 2412.0\n\
\tsignal: -85.00 dBm\n\
\tSSID: Guest\n\
\tWPA:\t * Version: 1\n\
\t\t * Authentication suites: PSK\n";
        let samples = parse_scan(text);
        assert_eq!(samples.len(), 4);
        assert_eq!(samples[0].bssid, "00:11:22:33:44:55");
        assert_eq!(samples[0].freq, 2437);
        assert_eq!(samples[0].rssi, -41);
        assert_eq!(samples[0].flags, "[WPA2-PSK]");
        assert_eq!(SecurityClass::from_flags(&samples[1].flags), SecurityClass::Wpa3Personal);
        assert_eq!(SecurityClass::from_flags(&samples[2].flags), SecurityClass::Wep);
        assert_eq!(SecurityClass::from_flags(&samples[3].flags), SecurityClass::WpaPersonal);
    }

    #[test]
    fn scan_drops_blocks_without_signal() {
        let text = "BSS 00:11:22:33:44:99(on wlan0)\n\tfreq: 2412.0\n\tSSID: NoSignal\n";
        assert!(parse_scan(text).is_empty());
    }

    #[test]
    fn scan_mixed_wpa2_and_wpa3_is_transition_mode() {
        let text = "BSS 00:11:22:33:44:aa(on wlan0)\n\tfreq: 5745.0\n\tsignal: -50.00 dBm\n\tSSID: Mixed\n\
\tRSN:\t * Version: 1\n\t\t * Authentication suites: PSK SAE\n";
        let samples = parse_scan(text);
        assert_eq!(SecurityClass::from_flags(&samples[0].flags), SecurityClass::Wpa2Wpa3Personal);
    }

    #[test]
    fn scan_enterprise_is_detected() {
        let text = "BSS 00:11:22:33:44:bb(on wlan0)\n\tfreq: 2462.0\n\tsignal: -60.00 dBm\n\tSSID: Corp\n\
\tRSN:\t * Version: 1\n\t\t * Authentication suites: 802.1x\n";
        let samples = parse_scan(text);
        assert_eq!(SecurityClass::from_flags(&samples[0].flags), SecurityClass::Enterprise);
    }

    #[test]
    fn rating_follows_snr_then_signal() {
        assert_eq!(rate_link(Some(-40), Some(-95)), Rating::Excellent);
        assert_eq!(rate_link(Some(-60), Some(-90)), Rating::Good);
        assert_eq!(rate_link(Some(-75), Some(-92)), Rating::Fair);
        assert_eq!(rate_link(Some(-85), Some(-90)), Rating::Poor);
        assert_eq!(rate_link(Some(-50), None), Rating::Excellent);
        assert_eq!(rate_link(Some(-70), None), Rating::Good);
        assert_eq!(rate_link(None, Some(-90)), Rating::Unknown);
        assert_eq!(snr_db(Some(-40), Some(-95)), Some(55));
        assert_eq!(snr_db(Some(-40), None), None);
    }

    #[test]
    fn history_keeps_the_latest_window() {
        let mut h = History::default();
        assert!(h.is_empty());
        for i in 0..(HISTORY_LEN + 5) {
            h.push(Some(-(i as i32)));
        }
        assert_eq!(h.len(), HISTORY_LEN);
        assert_eq!(h.iter().next(), Some(Some(-5)));
        assert_eq!(h.latest(), Some(-(HISTORY_LEN as i32 + 4)));
    }

    #[test]
    fn history_summary_skips_gaps() {
        let mut h = History::default();
        h.push(Some(-40));
        h.push(None);
        h.push(Some(-60));
        assert_eq!(h.summary(), Some((-60, -40, -50.0)));
        assert_eq!(h.latest(), Some(-60));
        let mut empty = History::default();
        empty.push(None);
        assert_eq!(empty.summary(), None);
        assert_eq!(empty.latest(), None);
    }

    #[test]
    fn chart_fraction_is_clamped() {
        assert_eq!(chart_fraction(-100), 0.0);
        assert_eq!(chart_fraction(-30), 1.0);
        assert_eq!(chart_fraction(-120), 0.0);
        assert_eq!(chart_fraction(0), 1.0);
        assert!((chart_fraction(-65) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn utc_stamp_matches_known_times() {
        assert_eq!(utc_stamp(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc_stamp(1_700_000_000), "2023-11-14 22:13:20 UTC");
        assert_eq!(utc_stamp(951_782_400), "2000-02-29 00:00:00 UTC");
        assert_eq!(report_file_name(42), "wireless-diagnostics-42.txt");
    }

    #[test]
    fn report_lists_link_history_and_networks() {
        let link = parse_link(LINK);
        let mut history = History::default();
        history.push(Some(-52));
        history.push(Some(-54));
        let networks = wlan::merge(&[Sample {
            bssid: "00:11:22:33:44:55".into(),
            freq: 2437,
            rssi: -41,
            flags: "[WPA2-PSK]".into(),
            ssid: "Cafe".into(),
        }]);
        let text = report(1_700_000_000, "wlan0", &link, Some(-95), &networks, &history);
        assert!(text.starts_with("Wireless Diagnostics report\n"));
        assert!(text.contains("Generated: 2023-11-14 22:13:20 UTC (1700000000)"));
        assert!(text.contains("  Network: Home Net\n"));
        assert!(text.contains("  Channel: 36\n"));
        assert!(text.contains("  Band: 5 GHz\n"));
        assert!(text.contains("  Signal-to-noise: 43 dB\n"));
        assert!(text.contains("  Quality: Excellent\n"));
        assert!(text.contains("  Receive rate: 866.7 Mbit/s\n"));
        assert!(text.contains("2 readings: lowest -54 dBm, highest -52 dBm"));
        assert!(text.contains("  Cafe: -41 dBm, channel 6, 2.4 GHz"));
        assert!(text.contains("WPA2 Personal"));
    }

    #[test]
    fn report_handles_a_disconnected_radio() {
        let text = report(0, "", &Link::default(), None, &[], &History::default());
        assert!(text.contains("Interface: none"));
        assert!(text.contains("  Network: Not connected"));
        assert!(text.contains("  Signal: Not available"));
        assert!(text.contains("No readings"));
        assert!(text.contains("No scan results"));
    }
}
