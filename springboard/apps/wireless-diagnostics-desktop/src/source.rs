//! Where the readings come from: `/proc/net/wireless` and the `iw` tool. Commands run on background threads so the
//! window never waits for them. A poller sends one `Reading` per interval; a scan sends its answer once.

use crate::model::{self, Iface, Link};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::wlan::Sample;
use std::fs;
use std::process::Command;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

const PROC_WIRELESS: &str = "/proc/net/wireless";

/// The interface used when the kernel lists none. A missing interface is reported in the footer, not hidden.
const FALLBACK_IFACE: &str = "wlan0";

/// What the radio looked like at one poll.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    pub ifaces: Vec<Iface>,
    /// The interface the link was read from.
    pub iface: String,
    pub link: Link,
    /// Why the link could not be read (for example `iw` is not installed).
    pub note: Option<String>,
}

/// The answer to a scan: the access points, and whether they are the cached results of an earlier scan.
#[derive(Clone, Debug, PartialEq)]
pub struct ScanOutcome {
    pub samples: Vec<Sample>,
    pub cached: bool,
}

/// The readings a pinned (test) run shows instead of the machine's radio, oldest first: one connected link whose signal
/// drifts a little, so the chart and the connection card are the same on every run and on every host.
pub fn fixture_readings() -> Vec<Reading> {
    const SIGNALS: [i32; 8] = [-60, -58, -57, -57, -55, -54, -54, -53];
    SIGNALS
        .iter()
        .map(|&dbm| Reading {
            ifaces: vec![Iface { name: "wlan0".into(), quality: Some(60), level_dbm: Some(dbm), noise_dbm: Some(-92) }],
            iface: "wlan0".into(),
            link: Link {
                connected: true,
                ssid: "Example Network".into(),
                bssid: "02:00:00:00:00:01".into(),
                freq_mhz: Some(5180),
                signal_dbm: Some(dbm),
                rx_mbps: Some(866.7),
                tx_mbps: Some(780.0),
            },
            note: None,
        })
        .collect()
}

/// Runs `iw` with `args` and returns its standard output, or a message for the user.
pub fn run_iw(args: &[&str]) -> Result<String, String> {
    let out = Command::new("iw").args(args).output().map_err(|e| format!("iw could not run: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { format!("iw exited with {}", out.status) } else { err })
    }
}

/// Reads the interfaces and the link of the first wireless interface once.
pub fn read_once() -> Reading {
    let ifaces = fs::read_to_string(PROC_WIRELESS).map(|text| model::parse_proc_wireless(&text)).unwrap_or_default();
    let iface = ifaces.first().map_or_else(|| FALLBACK_IFACE.to_string(), |i| i.name.clone());
    match run_iw(&["dev", &iface, "link"]) {
        Ok(text) => Reading { link: model::parse_link(&text), ifaces, iface, note: None },
        Err(note) => Reading { link: Link::default(), ifaces, iface, note: Some(note) },
    }
}

/// Starts the poller: one `read_once` every `every`, until the receiver is dropped.
pub fn spawn_poller(every: Duration, readings: &Arc<Mutex<VecDeque<Reading>>>) {
    fn schedule(every: Duration, readings: Weak<Mutex<VecDeque<Reading>>>) {
        DispatchQueue::global(Qos::Utility).async_after(every, move || poll(every, readings));
    }
    fn poll(every: Duration, readings: Weak<Mutex<VecDeque<Reading>>>) {
        DispatchQueue::global(Qos::Utility).async_(move || {
            let reading = read_once();
            DispatchQueue::main().async_(move || {
                let Some(queue) = readings.upgrade() else { return };
                if let Ok(mut queue) = queue.lock() {
                    queue.push_back(reading);
                }
                schedule(every, readings);
            });
        });
    }
    poll(every, Arc::downgrade(readings));
}

/// Scans `iface` on a background thread. A fresh scan usually needs the network privilege; when it is refused the
/// cached results of the last scan are used and marked as cached.
pub fn spawn_scan(iface: String, result: Arc<Mutex<Option<Result<ScanOutcome, String>>>>) {
    DispatchQueue::global(Qos::Utility).async_(move || {
        let outcome = match run_iw(&["dev", &iface, "scan"]) {
            Ok(text) => Ok(ScanOutcome { samples: model::parse_scan(&text), cached: false }),
            Err(fresh_err) => match run_iw(&["dev", &iface, "scan", "dump"]) {
                Ok(text) => Ok(ScanOutcome { samples: model::parse_scan(&text), cached: true }),
                Err(_) => Err(format!("Scan failed: {fresh_err}")),
            },
        };
        DispatchQueue::main().async_(move || {
            if let Ok(mut slot) = result.lock() {
                *slot = Some(outcome);
            }
        });
    });
}
