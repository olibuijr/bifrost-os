//! wpa_supplicant control-socket client and reply parsers.

use std::io;
use std::os::unix::net::UnixDatagram;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct ScanResult {
    pub bssid: String,
    pub freq: u32,
    pub signal: i32,
    pub secure: bool,
    /// 802.1X (WPA-EAP): needs a username, not a shared password
    pub enterprise: bool,
    pub ssid: String,
}

impl ScanResult {
    /// 0..=3 bars like bifrost.
    pub fn bars(&self) -> u8 {
        match self.signal {
            s if s >= -60 => 3,
            s if s >= -72 => 2,
            s if s >= -84 => 1,
            _ => 0,
        }
    }
}

/// Decodes wpa_supplicant's printf-escaped SSIDs (\xNN, \\, \").
pub fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            if b[i + 1] == b'x' && i + 3 < b.len() {
                if let Ok(v) = u8::from_str_radix(&s[i + 2..i + 4], 16) {
                    out.push(v);
                    i += 4;
                    continue;
                }
            }
            out.push(b[i + 1]);
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parses SCAN_RESULTS: strongest entry per SSID, strongest first, hidden SSIDs dropped.
pub fn parse_scan(text: &str) -> Vec<ScanResult> {
    let mut v: Vec<ScanResult> = text
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.splitn(5, '\t').collect();
            let ssid = unescape(f.get(4)?);
            (!ssid.is_empty()).then(|| ScanResult {
                bssid: f[0].into(),
                freq: f[1].parse().unwrap_or(0),
                signal: f[2].parse().unwrap_or(-100),
                secure: f[3].contains("WPA") || f[3].contains("RSN") || f[3].contains("WEP") || f[3].contains("SAE"),
                enterprise: f[3].contains("EAP"),
                ssid,
            })
        })
        .collect();
    v.sort_by_key(|r| std::cmp::Reverse(r.signal));
    let mut seen = std::collections::HashSet::new();
    v.retain(|r| seen.insert(r.ssid.clone()));
    v
}

/// Parses LIST_NETWORKS into (network id, ssid) pairs.
pub fn parse_networks(text: &str) -> Vec<(String, String)> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let mut f = l.split('\t');
            Some((f.next()?.to_string(), unescape(f.next()?)))
        })
        .collect()
}

/// Looks up `key` in a key=value reply such as STATUS.
pub fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
}

/// Quotes a string for SET_NETWORK (ssid/psk) the way wpa_supplicant expects.
pub fn quoted(s: &str) -> String {
    if s.bytes().all(|c| (0x20..0x7f).contains(&c) && c != b'"') {
        format!("\"{s}\"")
    } else {
        s.bytes().map(|c| format!("{c:02x}")).collect()
    }
}

pub struct Ctrl {
    sock: UnixDatagram,
    local: String,
}

impl Ctrl {
    /// Connects to `<ctrl_dir>/<iface>`.
    pub fn connect(ctrl_dir: &str, iface: &str) -> io::Result<Self> {
        Self::connect_in(ctrl_dir, ctrl_dir, iface)
    }

    /// Connects to `<ctrl_dir>/<iface>` with our own socket in `local_dir`, for a system
    /// supplicant whose directory (/run/wpa_supplicant) an unprivileged shell cannot write to.
    pub fn connect_in(local_dir: &str, ctrl_dir: &str, iface: &str) -> io::Result<Self> {
        let local = format!("{local_dir}/sb_{}", std::process::id());
        let _ = std::fs::remove_file(&local);
        let sock = UnixDatagram::bind(&local)?;
        sock.connect(format!("{ctrl_dir}/{iface}"))?;
        sock.set_read_timeout(Some(Duration::from_secs(10)))?;
        Ok(Ctrl { sock, local })
    }

    pub fn request(&self, cmd: &str) -> io::Result<String> {
        self.sock.send(cmd.as_bytes())?;
        let mut buf = vec![0u8; 16384];
        loop {
            let n = self.sock.recv(&mut buf)?;
            let reply = String::from_utf8_lossy(&buf[..n]).into_owned();
            // unsolicited events start with "<level>"; skip them
            if !reply.starts_with('<') {
                return Ok(reply);
            }
        }
    }

    pub fn ok(&self, cmd: &str) -> io::Result<()> {
        let r = self.request(cmd)?;
        if r.trim() == "FAIL" {
            return Err(io::Error::other(format!("wpa: {} failed", verb(cmd))));
        }
        Ok(())
    }

    /// A second socket that receives the supplicant's unsolicited events
    /// (`request` skips them), polled with `events`.
    pub fn connect_events(ctrl_dir: &str, iface: &str) -> io::Result<Self> {
        Self::connect_events_in(ctrl_dir, ctrl_dir, iface)
    }

    /// `connect_events` with our socket in `local_dir` (see `connect_in`).
    pub fn connect_events_in(local_dir: &str, ctrl_dir: &str, iface: &str) -> io::Result<Self> {
        let local = format!("{local_dir}/sb_{}_ev", std::process::id());
        let _ = std::fs::remove_file(&local);
        let sock = UnixDatagram::bind(&local)?;
        sock.connect(format!("{ctrl_dir}/{iface}"))?;
        sock.set_read_timeout(Some(Duration::from_secs(10)))?;
        let c = Ctrl { sock, local };
        if !c.request("ATTACH")?.starts_with("OK") {
            return Err(io::Error::other("wpa: ATTACH failed"));
        }
        c.sock.set_nonblocking(true)?;
        Ok(c)
    }

    /// The events that arrived since the last call, without blocking.
    pub fn events(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut buf = vec![0u8; 4096];
        while let Ok(n) = self.sock.recv(&mut buf) {
            out.push(String::from_utf8_lossy(&buf[..n]).into_owned());
        }
        out
    }
}

/// The command name and network id of a control command, never its
/// arguments: SET_NETWORK carries passphrases that must not reach logs or
/// error texts.
pub fn verb(cmd: &str) -> String {
    cmd.split_whitespace().take(3).take_while(|w| !w.starts_with('"')).collect::<Vec<_>>().join(" ")
}

impl Drop for Ctrl {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.local);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCAN: &str = "bssid / frequency / signal level / flags / ssid\n\
        aa:bb:cc:dd:ee:01\t2412\t-70\t[WPA2-PSK-CCMP][ESS]\tWIFI\n\
        aa:bb:cc:dd:ee:02\t5180\t-48\t[WPA2-PSK-CCMP][ESS]\tWIFI\n\
        aa:bb:cc:dd:ee:03\t2437\t-80\t[ESS]\tCaf\\xc3\\xa9 Open\n\
        aa:bb:cc:dd:ee:05\t2462\t-90\t[WPA2-EAP-CCMP][ESS]\tCorp\n\
        aa:bb:cc:dd:ee:04\t2462\t-55\t[WPA2-PSK-CCMP][ESS]\t\n";

    #[test]
    fn scan_dedupes_sorts_and_decodes() {
        let r = parse_scan(SCAN);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].ssid, "WIFI");
        assert_eq!(r[0].signal, -48);
        assert_eq!(r[0].freq, 5180);
        assert!(r[0].secure);
        assert_eq!(r[1].ssid, "Café Open");
        assert!(!r[1].secure);
        assert_eq!(r[0].bars(), 3);
        assert_eq!(r[1].bars(), 1);
        assert!(r[2].enterprise && r[2].secure && !r[0].enterprise);
    }

    #[test]
    fn status_fields() {
        let s = "bssid=aa\nssid=WIFI\nwpa_state=COMPLETED\nip_address=192.168.1.171\n";
        assert_eq!(field(s, "wpa_state"), Some("COMPLETED"));
        assert_eq!(field(s, "ssid"), Some("WIFI"));
        assert_eq!(field(s, "missing"), None);
    }

    #[test]
    fn quoting_falls_back_to_hex() {
        assert_eq!(quoted("WIFI"), "\"WIFI\"");
        assert_eq!(quoted("a\"b"), "612262");
        assert_eq!(quoted("é"), "c3a9");
    }

    #[test]
    fn list_networks_gives_ids_and_ssids() {
        let list = "network id / ssid / bssid / flags\n0\tWIFI\tany\t[CURRENT]\n3\tCaf\\xc3\\xa9\tany\t[DISABLED]\n";
        assert_eq!(parse_networks(list), vec![("0".into(), "WIFI".into()), ("3".into(), "Café".into())]);
        assert!(parse_networks("network id / ssid / bssid / flags\n").is_empty());
    }

    #[test]
    fn errors_name_the_command_but_not_its_secret() {
        assert_eq!(verb("SET_NETWORK 3 psk \"hunter22\""), "SET_NETWORK 3 psk");
        assert_eq!(verb("SELECT_NETWORK 3"), "SELECT_NETWORK 3");
        assert_eq!(verb("SAVE_CONFIG"), "SAVE_CONFIG");
    }

    #[test]
    fn unescape_handles_backslashes() {
        assert_eq!(unescape("a\\\\b\\\"c"), "a\\b\"c");
    }
}
