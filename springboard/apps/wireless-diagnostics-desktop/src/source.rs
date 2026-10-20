//! Where the readings come from: `/proc/net/wireless` and the `iw` tool. Commands run on background threads so the
//! window never waits for them. A poller sends one `Reading` per interval; a scan sends its answer once.

use crate::model::{self, Iface, Link};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::wlan::Sample;
use std::collections::VecDeque;
use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

const PROC_WIRELESS: &str = "/proc/net/wireless";

/// How long a link read may take before `iw` is killed, and how long a scan may take.
const LINK_LIMIT: Duration = Duration::from_secs(5);
const SCAN_LIMIT: Duration = Duration::from_secs(30);
/// The most output read from `iw`.
const MAX_OUTPUT: u64 = 1 << 20;

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

/// Reads a pipe on its own thread, so a full pipe never stops the child and a child that never exits cannot hold the
/// caller.
fn drain_pipe<R: Read + Send + 'static>(pipe: Option<R>) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = channel();
    // compliance-allow: RES-01 a pool worker waiting on another pool job can starve at the width cap; a stalled tool must not hold one
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(pipe) = pipe {
            let _ = pipe.take(MAX_OUTPUT).read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
    rx
}

/// Runs `program` with `args` and returns its standard output, or a message for the user. A program that runs past
/// `limit` is killed.
fn run_limited(program: &str, args: &[&str], limit: Duration) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program} could not run: {e}"))?;
    let out = drain_pipe(child.stdout.take());
    let err = drain_pipe(child.stderr.take());
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} did not answer in time"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("{program} could not run: {e}")),
        }
    };
    let grace = Duration::from_secs(1);
    let stdout = out.recv_timeout(grace).unwrap_or_default();
    if status.success() {
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    } else {
        let stderr = err.recv_timeout(grace).unwrap_or_default();
        let text = String::from_utf8_lossy(&stderr).trim().to_string();
        Err(if text.is_empty() { format!("{program} exited with {status}") } else { text })
    }
}

/// Runs `iw` with `args` and returns its standard output, or a message for the user. Killed after `limit`.
pub fn run_iw(args: &[&str], limit: Duration) -> Result<String, String> {
    run_limited("iw", args, limit)
}

/// Reads the interfaces and the link of the first wireless interface once.
pub fn read_once() -> Reading {
    let ifaces = fs::read_to_string(PROC_WIRELESS).map(|text| model::parse_proc_wireless(&text)).unwrap_or_default();
    let iface = ifaces.first().map_or_else(|| FALLBACK_IFACE.to_string(), |i| i.name.clone());
    match run_iw(&["dev", &iface, "link"], LINK_LIMIT) {
        Ok(text) => Reading { link: model::parse_link(&text), ifaces, iface, note: None },
        Err(note) => Reading { link: Link::default(), ifaces, iface, note: Some(note) },
    }
}

/// Whether the poller's chain of reads is running, and whether it was asked to stop.
#[derive(Default)]
struct PollState {
    paused: bool,
    alive: bool,
}

/// The poller: one `read_once` every `every`, until the receiver is dropped. It can be paused (the window is hidden or
/// minimised): the chain then ends at its next turn and no `iw` is started until `resume`.
pub struct Poller {
    every: Duration,
    readings: Weak<Mutex<VecDeque<Reading>>>,
    state: Mutex<PollState>,
}

impl Poller {
    /// A poller that has not started.
    fn new(every: Duration, readings: &Arc<Mutex<VecDeque<Reading>>>) -> Arc<Poller> {
        Arc::new(Poller { every, readings: Arc::downgrade(readings), state: Mutex::new(PollState::default()) })
    }

    /// Starts the poller.
    pub fn start(every: Duration, readings: &Arc<Mutex<VecDeque<Reading>>>) -> Arc<Poller> {
        let poller = Poller::new(every, readings);
        poller.begin();
        poller
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PollState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts the chain unless it is running or paused.
    fn begin(self: &Arc<Self>) {
        {
            let mut st = self.lock();
            if st.alive || st.paused {
                return;
            }
            st.alive = true;
        }
        Poller::poll(self.clone());
    }

    /// Stops reading at the next turn (the window is not visible).
    pub fn pause(&self) {
        self.lock().paused = true;
    }

    /// Reads again: at once when the chain had ended, else at its next turn.
    pub fn resume(self: &Arc<Self>) {
        self.lock().paused = false;
        self.begin();
    }

    /// True (and the chain is over) when the poller is paused.
    fn should_stop(&self) -> bool {
        let mut st = self.lock();
        if st.paused {
            st.alive = false;
        }
        st.paused
    }

    fn poll(this: Arc<Poller>) {
        DispatchQueue::global(Qos::Utility).async_(move || {
            if this.should_stop() {
                return;
            }
            let reading = read_once();
            DispatchQueue::main().async_(move || {
                let Some(queue) = this.readings.upgrade() else {
                    this.lock().alive = false;
                    return;
                };
                if let Ok(mut queue) = queue.lock() {
                    queue.push_back(reading);
                }
                let every = this.every;
                DispatchQueue::global(Qos::Utility).async_after(every, move || Poller::poll(this));
            });
        });
    }
}

/// Scans `iface` on a background thread. A fresh scan usually needs the network privilege; when it is refused the
/// cached results of the last scan are used and marked as cached.
pub fn spawn_scan(iface: String, result: Arc<Mutex<Option<Result<ScanOutcome, String>>>>) {
    DispatchQueue::global(Qos::Utility).async_(move || {
        let outcome = match run_iw(&["dev", &iface, "scan"], SCAN_LIMIT) {
            Ok(text) => Ok(ScanOutcome { samples: model::parse_scan(&text), cached: false }),
            Err(fresh_err) => match run_iw(&["dev", &iface, "scan", "dump"], LINK_LIMIT) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_that_finishes_gives_its_output_and_one_that_fails_gives_its_message() {
        assert_eq!(run_limited("echo", &["hello"], Duration::from_secs(5)), Ok("hello\n".to_string()));
        let err = run_limited("sh", &["-c", "echo nope >&2; exit 3"], Duration::from_secs(5)).unwrap_err();
        assert_eq!(err, "nope");
    }

    #[test]
    fn a_program_that_runs_past_the_limit_is_killed() {
        let started = Instant::now();
        let err = run_limited("sleep", &["30"], Duration::from_millis(200)).unwrap_err();
        assert!(err.contains("did not answer in time"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_program_that_writes_a_lot_to_stderr_does_not_stall() {
        // 400 KB on stderr is far more than a pipe holds; the reader thread keeps it moving
        let out = run_limited("sh", &["-c", "head -c 400000 /dev/zero >&2; echo done"], Duration::from_secs(10));
        assert_eq!(out, Ok("done\n".to_string()));
    }

    #[test]
    fn a_paused_poller_ends_its_chain_and_a_resumed_one_may_start_again() {
        let readings = Arc::new(Mutex::new(VecDeque::new()));
        let poller = Poller::new(Duration::from_secs(1), &readings);
        poller.lock().alive = true;
        assert!(!poller.should_stop());
        poller.pause();
        assert!(poller.should_stop());
        assert!(!poller.lock().alive, "the chain ended");
        // a pause while the chain is waiting for its next turn leaves it running until that turn
        poller.lock().paused = false;
        poller.lock().alive = true;
        poller.pause();
        assert!(poller.lock().alive);
    }
}
