//! Wi-Fi connection status for apps: the state of the connection, the network it joined, its address and signal, the
//! saved networks in priority order and the problem notes the system recorded for a network.
//!
//! The system layer owns the radio and the supplicant. It reports a state code (0 off, 1 starting, 2 searching,
//! 3 connecting, 4 connected) with the fields below. This module turns those values into an app-facing snapshot.
//! It makes no service call and holds no socket, so an app builds a [`WifiStatus`] from what it was given.

/// The state of the Wi-Fi connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WifiState {
    Off,
    Starting,
    Searching,
    Connecting,
    Connected,
    /// A state code this build does not know. The raw code is kept so a newer system can still be read.
    Unknown(u8),
}

impl WifiState {
    /// The state for a state code reported by the system layer.
    pub fn from_code(code: u8) -> WifiState {
        match code {
            0 => WifiState::Off,
            1 => WifiState::Starting,
            2 => WifiState::Searching,
            3 => WifiState::Connecting,
            4 => WifiState::Connected,
            other => WifiState::Unknown(other),
        }
    }

    /// The state code for this state.
    pub fn code(self) -> u8 {
        match self {
            WifiState::Off => 0,
            WifiState::Starting => 1,
            WifiState::Searching => 2,
            WifiState::Connecting => 3,
            WifiState::Connected => 4,
            WifiState::Unknown(code) => code,
        }
    }

    /// A short English label for the state.
    pub fn label(self) -> &'static str {
        match self {
            WifiState::Off => "Off",
            WifiState::Starting => "Starting",
            WifiState::Searching => "Searching",
            WifiState::Connecting => "Connecting",
            WifiState::Connected => "Connected",
            WifiState::Unknown(_) => "Unknown",
        }
    }
}

/// The number of signal bars (0 to 4) for a received signal strength in dBm.
///
/// `None` (no reading) gives 0 bars. The bars start at -55, -65, -75 and -85 dBm.
pub fn signal_bars(rssi: Option<i32>) -> u8 {
    match rssi {
        Some(r) if r >= -55 => 4,
        Some(r) if r >= -65 => 3,
        Some(r) if r >= -75 => 2,
        Some(r) if r >= -85 => 1,
        _ => 0,
    }
}

/// A snapshot of the Wi-Fi connection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WifiStatus {
    pub state: Option<WifiState>,
    /// The network name the system is joined to or joining.
    pub ssid: Option<String>,
    /// The IPv4 address, when DHCP has given one.
    pub ip: Option<String>,
    /// The default gateway, when known.
    pub router: Option<String>,
    /// The received signal strength in dBm, when there is a reading.
    pub rssi: Option<i32>,
    /// Saved network names, highest priority first.
    pub saved: Vec<String>,
    /// Problem notes as (network name, text), for example ("Home", "Incorrect Password").
    pub problems: Vec<(String, String)>,
}

impl WifiStatus {
    /// The state, or `Off` when the system has not reported one.
    pub fn state_or_off(&self) -> WifiState {
        self.state.unwrap_or(WifiState::Off)
    }

    /// Whether the system reports the connected state.
    pub fn is_connected(&self) -> bool {
        self.state_or_off() == WifiState::Connected
    }

    /// Whether an address has been assigned.
    pub fn has_address(&self) -> bool {
        self.ip.is_some()
    }

    /// The signal bars for the current reading, from 0 to 4.
    pub fn signal_bars(&self) -> u8 {
        signal_bars(self.rssi)
    }

    /// Whether a network with this name is saved.
    pub fn is_saved(&self, ssid: &str) -> bool {
        self.saved.iter().any(|s| s == ssid)
    }

    /// The first problem note recorded for a network, if any.
    pub fn problem_for(&self, ssid: &str) -> Option<&str> {
        self.problems.iter().find(|(name, _)| name == ssid).map(|(_, text)| text.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_codes_round_trip_and_unknown_codes_are_kept() {
        assert_eq!(WifiState::from_code(4), WifiState::Connected);
        assert_eq!(WifiState::from_code(0), WifiState::Off);
        assert_eq!(WifiState::from_code(9), WifiState::Unknown(9));
        assert_eq!(WifiState::Unknown(9).code(), 9);
        for code in 0..=4u8 {
            assert_eq!(WifiState::from_code(code).code(), code);
        }
        assert_eq!(WifiState::from_code(3).label(), "Connecting");
        assert_eq!(WifiState::Unknown(7).label(), "Unknown");
    }

    #[test]
    fn signal_bars_follow_the_four_cut_offs() {
        assert_eq!(signal_bars(Some(-40)), 4);
        assert_eq!(signal_bars(Some(-55)), 4);
        assert_eq!(signal_bars(Some(-56)), 3);
        assert_eq!(signal_bars(Some(-65)), 3);
        assert_eq!(signal_bars(Some(-66)), 2);
        assert_eq!(signal_bars(Some(-75)), 2);
        assert_eq!(signal_bars(Some(-76)), 1);
        assert_eq!(signal_bars(Some(-85)), 1);
        assert_eq!(signal_bars(Some(-86)), 0);
        assert_eq!(signal_bars(None), 0);
    }

    #[test]
    fn missing_state_reads_as_off_and_connected_needs_the_connected_code() {
        let empty = WifiStatus::default();
        assert_eq!(empty.state_or_off(), WifiState::Off);
        assert!(!empty.is_connected());
        let connecting = WifiStatus { state: Some(WifiState::from_code(3)), ..WifiStatus::default() };
        assert!(!connecting.is_connected());
        let up = WifiStatus {
            state: Some(WifiState::from_code(4)),
            ssid: Some("Home".to_string()),
            ip: Some("192.168.1.20".to_string()),
            ..WifiStatus::default()
        };
        assert!(up.is_connected());
        assert!(up.has_address());
    }

    #[test]
    fn saved_networks_and_problem_notes_are_looked_up_by_name() {
        let status = WifiStatus {
            saved: vec!["Office".to_string(), "Home".to_string()],
            problems: vec![("Home".to_string(), "Incorrect Password".to_string())],
            ..WifiStatus::default()
        };
        assert!(status.is_saved("Home"));
        assert!(!status.is_saved("Cafe"));
        assert_eq!(status.problem_for("Home"), Some("Incorrect Password"));
        assert_eq!(status.problem_for("Office"), None);
    }

    #[test]
    fn snapshot_reports_bars_from_its_reading() {
        let status = WifiStatus { rssi: Some(-70), ..WifiStatus::default() };
        assert_eq!(status.signal_bars(), 2);
        assert_eq!(WifiStatus::default().signal_bars(), 0);
    }
}
