//! Wi-Fi: the wireless interface is found in /sys/class/net/*/wireless, and the shell talks to
//! the system's own wpa_supplicant over its control socket (/run/wpa_supplicant/<iface>, the
//! same protocol `Ctrl` already speaks). That is the simplest robust option: it is the one stable
//! interface every Arch setup with a standalone supplicant (wpa_supplicant@, netctl, dhcpcd,
//! networkd) exposes, with scanning, joining and saved networks all handled by commands we
//! already have. NetworkManager and iwd keep their supplicant behind D-Bus, so without that
//! socket the page is read-only: link state and address come from sysfs. No kernel module is
//! loaded, no supplicant started or killed. Saved networks are mirrored under `<data dir>/wifi`.
//! DHCP is the host's when it runs a client (NetworkManager, networkd, dhcpcd...), else ours. With
//! no Wi-Fi at all the wired link (carrier, address) is published as `Status::ethernet`.

use super::wifi_config::{self, NetworkConfig};
use super::wifi_event;
use super::{dhcp, netlink, wpa};
use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub const IFACE: &str = "wlan0";
const IDLE_KICK: Duration = Duration::from_secs(8);
/// Associated without a DHCP lease for this long means no internet.
const DHCP_WAIT: Duration = Duration::from_secs(15);
/// Pause before asking DHCP again after that.
const DHCP_RETRY: Duration = Duration::from_secs(10);
/// Where a desktop's standalone wpa_supplicant keeps its control sockets.
const DESKTOP_CTRL_DIR: &str = "/run/wpa_supplicant";
/// How often a desktop looks again for a missing adapter or supplicant.
const PROBE_EVERY: Duration = Duration::from_secs(5);
/// How often the wired link is read.
const ETH_EVERY: Duration = Duration::from_secs(2);
/// The wall clock runs through a suspend and the monotonic clock does not: a gap this much larger than
/// the tick between two readings is a sleep.
const SLEEP_GAP: Duration = Duration::from_secs(5);
/// Down this long after a network we had joined: ask the supplicant to reconnect, then again at this pace.
const RECONNECT_AFTER: Duration = Duration::from_secs(10);
const RECONNECT_EVERY: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum State {
    #[default]
    Off,
    Starting,
    Searching,
    Connecting,
    Connected,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub state: State,
    pub ssid: Option<String>,
    pub ip: Option<Ipv4Addr>,
    pub router: Option<Ipv4Addr>,
    /// signal of the current network, dBm
    pub rssi: Option<i32>,
    pub scan: Vec<wpa::ScanResult>,
    /// saved networks' names, highest priority first
    pub saved: Vec<String>,
    /// the saved networks with their settings, highest priority first
    pub known: Vec<Known>,
    /// what went wrong per network name: "Incorrect Password", "No Internet Connection"
    pub notes: HashMap<String, String>,
    pub error: Option<String>,
    /// desktop: the wired link, shown as the connection when there is no Wi-Fi
    pub ethernet: Option<Ethernet>,
    /// desktop: the host's network manager owns the adapter; status is shown, nothing can be changed
    pub read_only: bool,
}

/// The wired link of a desktop.
#[derive(Clone, Debug, PartialEq)]
pub struct Ethernet {
    pub iface: String,
    pub ip: Option<Ipv4Addr>,
}

/// A saved network as the Settings pages show it.
#[derive(Clone, Debug, PartialEq)]
pub struct Known {
    pub ssid: String,
    pub auto_join: bool,
    pub hidden: bool,
    pub enterprise: bool,
}

#[derive(Debug)]
pub enum Cmd {
    Scan,
    Join {
        ssid: String,
        psk: Option<String>,
    },
    /// connects to an already saved network
    Select(String),
    /// joins a network typed in by hand (hidden, enterprise, WPA3...)
    JoinOther(NetworkConfig),
    Forget(String),
    SetAutoJoin {
        ssid: String,
        on: bool,
    },
    /// one step up or down the priority list
    Move {
        ssid: String,
        up: bool,
    },
    Enable(bool),
}

pub struct Wifi {
    pub status: Arc<Mutex<Status>>,
    tx: Sender<Cmd>,
}

impl Wifi {
    /// Starts the Wi-Fi thread; `dir` holds wpa_supplicant.conf, the MAC
    /// file and the control socket directory.
    pub fn start(dir: &str) -> Self {
        let status = Arc::new(Mutex::new(Status { state: State::Starting, ..Status::default() }));
        let (tx, rx) = std::sync::mpsc::channel();
        let (st, d) = (status.clone(), dir.to_string());
        crate::bare::sys::spawn_named("wifi", move || Manager::new(d, st).run(rx));
        Wifi { status, tx }
    }

    pub fn send(&self, c: Cmd) {
        if let Ok(mut s) = self.status.lock() {
            anticipate(&mut s, &c);
        }
        let _ = self.tx.send(c);
    }

    /// A sender for commands from other threads (the takeover guard).
    pub fn sender(&self) -> Sender<Cmd> {
        self.tx.clone()
    }

    pub fn snapshot(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

/// Shows a switch flip at once; the manager thread may take seconds to act.
fn anticipate(s: &mut Status, c: &Cmd) {
    match c {
        Cmd::Enable(true) if s.state == State::Off => s.state = State::Starting,
        Cmd::Enable(false) => *s = Status { ethernet: s.ethernet.take(), ..Status::default() },
        _ => {}
    }
}

/// A config written before WoWLAN support, with the trigger added.
pub fn with_wowlan(conf: &str) -> Option<String> {
    (!conf.contains("wowlan_triggers=")).then(|| format!("wowlan_triggers=any\n{conf}"))
}

struct Manager {
    dir: String,
    status: Arc<Mutex<Status>>,
    ctrl: Option<wpa::Ctrl>,
    lease: Option<(dhcp::Lease, Instant)>,
    enabled: bool,
    last_scan: Option<Instant>,
    /// since when the supplicant has sat idle (DISCONNECTED/INACTIVE)
    idle_since: Option<Instant>,
    /// the saved networks, highest priority first; springboard.conf mirrors them
    nets: Vec<NetworkConfig>,
    /// the supplicant's unsolicited events (authentication failures)
    events: Option<wpa::Ctrl>,
    /// the network the user last asked to join, for events that do not name one
    attempt: Option<String>,
    /// a manual join in progress: its auto-join setting does not apply until it connects
    manual: Option<String>,
    /// networks with auto-join off that are connected now; disabled once they drop
    pending_off: HashSet<String>,
    /// no DHCP request before this, after one went unanswered
    next_dhcp: Option<Instant>,
    /// the wireless interface, discovered at start-up
    iface: String,
    /// the host runs a DHCP client, so we only read the address
    host_ip: bool,
    next_probe: Option<Instant>,
    next_eth: Option<Instant>,
    /// the network the link last reached, and since when it has been down
    connected_to: Option<String>,
    lost_since: Option<Instant>,
    last_reconnect: Option<Instant>,
    /// a DHCP renewal is due now (after a sleep), whatever the lease's age
    renew_due: bool,
    /// the clocks at the last desktop tick, to tell a sleep from a slow tick
    last_wall: SystemTime,
    last_mono: Instant,
}

impl Manager {
    fn new(dir: String, status: Arc<Mutex<Status>>) -> Self {
        Manager { iface: IFACE.to_string(), host_ip: false, next_probe: None, next_eth: None, dir, status, ctrl: None, lease: None, enabled: true, last_scan: None, idle_since: None, nets: Vec::new(), events: None, attempt: None, manual: None, pending_off: HashSet::new(), next_dhcp: None, connected_to: None, lost_since: None, last_reconnect: None, renew_due: false, last_wall: SystemTime::now(), last_mono: Instant::now() }
    }

    fn set(&self, f: impl FnOnce(&mut Status)) {
        if let Ok(mut s) = self.status.lock() {
            f(&mut s);
        }
    }

    fn conf_path(&self) -> String {
        format!("{}/wifi/springboard.conf", self.dir)
    }

    /// The supplicant's control socket directory (`SPRINGBOARD_WPA_CTRL` overrides it).
    fn ctrl_dir(&self) -> String {
        std::env::var("SPRINGBOARD_WPA_CTRL").unwrap_or_else(|_| DESKTOP_CTRL_DIR.to_string())
    }

    /// Where our own end of the control sockets lives on a desktop (/run/wpa_supplicant is root's).
    fn local_dir(&self) -> String {
        format!("{}/wifi/run", self.dir)
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        if let Err(e) = self.bring_up_desktop() {
            self.set(|s| {
                s.state = State::Off;
                s.error = Some(e.to_string());
            });
        }
        let mut wait = Duration::ZERO;
        loop {
            // a command wakes the thread at once; otherwise it ticks at the pace `tick_interval` gives
            let first = rx.recv_timeout(wait);
            if matches!(&first, Err(std::sync::mpsc::RecvTimeoutError::Disconnected)) {
                std::thread::sleep(wait);
            }
            let mut cmds: Vec<Cmd> = first.into_iter().collect();
            cmds.extend(rx.try_iter());
            for cmd in cmds {
                if let Err(e) = self.command(cmd) {
                    self.set(|s| s.error = Some(e.to_string()));
                }
            }
            self.desktop_tick();
            self.publish_state();
            let state = self.status.lock().map(|s| s.state).unwrap_or_default();
            let transient = self.last_scan.is_some() || self.attempt.is_some() || self.manual.is_some() || self.next_dhcp.is_some();
            wait = tick_interval(state, transient, self.ctrl.is_some());
        }
    }

    /// Publishes the link's address, router and network name into the system configuration store.
    fn publish_state(&self) {
        let Ok(s) = self.status.lock() else { return };
        let desired = super::state_publish::desired(&self.iface, s.ip, s.router, s.ssid.as_deref());
        drop(s);
        if let Ok(mut store) = super::state_publish::store().lock() {
            super::state_publish::apply(&mut store, &desired);
        }
    }

    fn ctrl(&self) -> std::io::Result<&wpa::Ctrl> {
        self.ctrl.as_ref().ok_or_else(|| std::io::Error::other(self.no_ctrl_reason()))
    }

    fn no_ctrl_reason(&self) -> &'static str {
        match (self.iface.is_empty(), self.status.lock().is_ok_and(|s| s.read_only)) {
            (true, _) => "No Wi-Fi adapter",
            (false, true) => "Wi-Fi is managed by the system",
            _ => "wifi off",
        }
    }

    fn command(&mut self, cmd: Cmd) -> std::io::Result<()> {
        match cmd {
            Cmd::Scan => {
                self.ctrl()?.ok("SCAN")?;
                self.last_scan = Some(Instant::now());
            }
            Cmd::Join { ssid, psk } => self.join(NetworkConfig::personal(&ssid, psk.as_deref()))?,
            Cmd::JoinOther(cfg) => {
                if let Some(why) = cfg.problem() {
                    return Err(std::io::Error::other(why));
                }
                self.join(cfg)?;
            }
            Cmd::Select(ssid) => {
                let c = self.ctrl()?;
                let list = wpa::parse_networks(&c.request("LIST_NETWORKS")?);
                let (id, _) = list.iter().find(|(_, s)| *s == ssid).ok_or_else(|| std::io::Error::other(format!("{ssid} is not saved")))?;
                c.ok(&format!("SELECT_NETWORK {id}"))?;
                self.start_attempt(&ssid);
            }
            Cmd::Forget(ssid) => {
                for id in self.ids_of(&ssid)? {
                    self.ctrl()?.ok(&format!("REMOVE_NETWORK {id}"))?;
                }
                self.nets.retain(|n| n.ssid != ssid);
                self.pending_off.remove(&ssid);
                self.set(|s| {
                    s.notes.remove(&ssid);
                });
                self.persist()?;
                self.publish_known();
            }
            Cmd::SetAutoJoin { ssid, on } => {
                let Some(net) = self.nets.iter_mut().find(|n| n.ssid == ssid) else { return Ok(()) };
                net.auto_join = on;
                self.persist()?;
                self.publish_known();
                let current = self.status.lock().ok().and_then(|s| s.ssid.clone());
                for id in self.ids_of(&ssid)? {
                    if on {
                        self.pending_off.remove(&ssid);
                        self.ctrl()?.ok(&format!("ENABLE_NETWORK {id}"))?;
                    } else if current.as_deref() == Some(ssid.as_str()) {
                        // switching the joined network off must not drop the connection
                        self.pending_off.insert(ssid.clone());
                    } else {
                        self.ctrl()?.ok(&format!("DISABLE_NETWORK {id}"))?;
                    }
                }
            }
            Cmd::Move { ssid, up } => {
                if wifi_config::move_network(&mut self.nets, &ssid, up) {
                    self.persist()?;
                    self.publish_known();
                    self.reprioritise()?;
                }
            }
            Cmd::Enable(on) => {
                // the host's supplicant keeps running; we only ask it to (dis)connect
                self.enabled = on;
                if on {
                    self.desktop_attach();
                    if let Some(c) = &self.ctrl {
                        let _ = c.ok("RECONNECT");
                    }
                } else {
                    self.drop_lease();
                    if let Some(c) = &self.ctrl {
                        let _ = c.ok("DISCONNECT");
                    }
                    self.set(|s| *s = Status { ethernet: s.ethernet.take(), read_only: s.read_only, ..Status::default() });
                }
            }
        }
        Ok(())
    }

    fn poll(&mut self) -> std::io::Result<()> {
        self.read_events();
        let c = self.ctrl.as_ref().ok_or_else(|| std::io::Error::other(self.no_ctrl_reason()))?;
        let st = c.request("STATUS")?;
        let state = wpa::field(&st, "wpa_state").unwrap_or("").to_string();
        let ssid = wpa::field(&st, "ssid").map(wpa::unescape);
        if self.last_scan.is_some_and(|t| t.elapsed() > Duration::from_secs(3)) {
            let scan = wpa::parse_scan(&c.request("SCAN_RESULTS")?);
            self.set(|s| s.scan = scan);
            self.last_scan = None;
        }
        // The vendor supplicant can sit in DISCONNECTED after start or a
        // failed attempt without retrying; nudge it.
        if matches!(state.as_str(), "DISCONNECTED" | "INACTIVE") {
            let since = *self.idle_since.get_or_insert_with(Instant::now);
            if since.elapsed() > IDLE_KICK {
                c.ok("REASSOCIATE")?;
                self.idle_since = None;
            }
        } else {
            self.idle_since = None;
        }
        if state != "COMPLETED" {
            // a network we had joined dropped (signal loss, a roam): ask for it back, not more often than RECONNECT_EVERY
            let down = self.lost_since.get_or_insert_with(Instant::now).elapsed();
            if self.connected_to.is_some() && should_reconnect(down, self.last_reconnect.map(|t| t.elapsed())) {
                c.ok("RECONNECT")?;
                self.last_reconnect = Some(Instant::now());
            }
            if self.lease.is_some() {
                self.drop_lease();
            }
            self.next_dhcp = None;
            if self.manual.is_none() {
                let off: Vec<String> = self.pending_off.drain().collect();
                for ssid in off {
                    self.disable_live(&ssid);
                }
            }
            let next =
                if matches!(state.as_str(), "ASSOCIATING" | "ASSOCIATED" | "4WAY_HANDSHAKE" | "GROUP_HANDSHAKE" | "AUTHENTICATING") { State::Connecting } else { State::Searching };
            self.set(|s| {
                s.state = next;
                s.ssid = None;
                s.ip = None;
            });
            return Ok(());
        }
        self.manual = None;
        self.lost_since = None;
        self.connected_to = ssid.clone();
        if let Some(n) = ssid.as_ref().and_then(|s| self.nets.iter().find(|n| &n.ssid == s)) {
            if !n.auto_join {
                self.pending_off.insert(n.ssid.clone());
            }
        }
        let rssi = c.request("SIGNAL_POLL").ok().and_then(|r| wpa::field(&r, "RSSI").and_then(|v| v.trim().parse().ok()));
        self.set(|s| s.rssi = rssi);
        if self.host_ip {
            // the host's DHCP client owns the address: read it, never set one
            let ip = iface_ipv4(&self.iface);
            let router = default_gateway(&std::fs::read_to_string("/proc/net/route").unwrap_or_default()).filter(|(i, _)| *i == self.iface).map(|(_, g)| g);
            self.set(|s| {
                // a momentarily unreadable address on the same network is not a drop: keep Connected
                let same = s.state == State::Connected && s.ssid == ssid;
                s.state = if ip.is_some() || same { State::Connected } else { State::Connecting };
                s.ssid = ssid.clone();
                s.ip = ip;
                s.router = router;
            });
            return Ok(());
        }
        let renew = self.renew_due || self.lease.as_ref().is_none_or(|(l, at)| at.elapsed() > Duration::from_secs(l.seconds as u64 / 2));
        if renew {
            if self.next_dhcp.is_some_and(|t| Instant::now() < t) {
                return Ok(());
            }
            self.set(|s| {
                if s.ip.is_none() {
                    s.state = State::Connecting;
                }
                s.ssid = ssid.clone();
            });
            let prefer = self.lease.as_ref().map(|l| l.0.ip).or_else(|| self.status.lock().ok().and_then(|s| s.ip));
            let mac = netlink::parse_mac(&std::fs::read_to_string(format!("/sys/class/net/{}/address", self.iface)).unwrap_or_default()).unwrap_or([0; 6]);
            let lease = match dhcp::acquire(&self.iface, mac, prefer, "springboard", DHCP_WAIT) {
                Ok(l) => l,
                // associated, but nobody answered: say so instead of retrying silently
                Err(_) if self.lease.is_none() => {
                    if let Some(ssid) = &ssid {
                        self.note(ssid, wifi_event::NO_INTERNET);
                    }
                    self.next_dhcp = Some(Instant::now() + DHCP_RETRY);
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            self.apply(&lease)?;
            self.next_dhcp = None;
            self.renew_due = false;
            self.attempt = None;
            self.set(|s| {
                s.state = State::Connected;
                s.ssid = ssid.clone();
                s.ip = Some(lease.ip);
                s.router = lease.router;
                s.error = None;
                if let Some(ssid) = &ssid {
                    s.notes.remove(ssid);
                }
            });
            self.lease = Some((lease, Instant::now()));
        }
        Ok(())
    }

    /// Publishes the saved networks and their order for the Settings pages.
    fn publish_known(&self) {
        let known: Vec<Known> = self.nets.iter().map(|n| Known { ssid: n.ssid.clone(), auto_join: n.auto_join, hidden: n.hidden, enterprise: n.security.is_enterprise() }).collect();
        self.set(|s| {
            s.saved = known.iter().map(|k| k.ssid.clone()).collect();
            s.known = known;
        });
    }

    /// Writes the saved networks to springboard.conf (replacing it in one step).
    fn persist(&self) -> std::io::Result<()> {
        let tmp = format!("{}.tmp", self.conf_path());
        std::fs::write(&tmp, wifi_config::conf(&self.ctrl_dir(), &self.nets))?;
        std::fs::rename(tmp, self.conf_path())
    }

    /// The supplicant's ids of the networks named `ssid`.
    fn ids_of(&self, ssid: &str) -> std::io::Result<Vec<String>> {
        let list = wpa::parse_networks(&self.ctrl()?.request("LIST_NETWORKS")?);
        Ok(list.into_iter().filter(|(_, s)| s == ssid).map(|(id, _)| id).collect())
    }

    /// Gives every network in the supplicant the priority of its place in `nets`.
    fn reprioritise(&self) -> std::io::Result<()> {
        let c = self.ctrl()?;
        for (id, ssid) in wpa::parse_networks(&c.request("LIST_NETWORKS")?) {
            if let Some(i) = self.nets.iter().position(|n| n.ssid == ssid) {
                c.ok(&format!("SET_NETWORK {id} priority {}", wifi_config::priority_of(i, self.nets.len())))?;
            }
        }
        Ok(())
    }

    /// Adds `cfg` to the supplicant and returns its id; nothing is left behind on failure.
    fn add_network(&self, cfg: &NetworkConfig, priority: i32) -> std::io::Result<String> {
        let c = self.ctrl()?;
        let id = c.request("ADD_NETWORK")?.trim().to_string();
        for (key, value) in cfg.fields(priority) {
            if let Err(e) = c.ok(&format!("SET_NETWORK {id} {key} {value}")) {
                let _ = c.ok(&format!("REMOVE_NETWORK {id}"));
                return Err(e);
            }
        }
        Ok(id)
    }

    /// Saves `cfg` as the first network, replacing one of the same name, and connects to it.
    fn join(&mut self, cfg: NetworkConfig) -> std::io::Result<()> {
        self.ctrl()?;
        let ssid = cfg.ssid.clone();
        let before = self.nets.clone();
        wifi_config::upsert(&mut self.nets, cfg);
        let added = self.replace_live(&ssid);
        let id = match added {
            Ok(id) => id,
            Err(e) => {
                self.nets = before;
                return Err(e);
            }
        };
        self.reprioritise()?;
        self.ctrl()?.ok(&format!("SELECT_NETWORK {id}"))?;
        self.persist()?;
        self.publish_known();
        self.start_attempt(&ssid);
        Ok(())
    }

    /// Swaps the supplicant's entries for `ssid` with the one in `nets`.
    fn replace_live(&self, ssid: &str) -> std::io::Result<String> {
        let (i, cfg) = self.nets.iter().enumerate().find(|(_, n)| n.ssid == ssid).ok_or_else(|| std::io::Error::other("network missing"))?;
        let id = self.add_network(cfg, wifi_config::priority_of(i, self.nets.len()))?;
        for old in self.ids_of(ssid)?.into_iter().filter(|o| *o != id) {
            self.ctrl()?.ok(&format!("REMOVE_NETWORK {old}"))?;
        }
        Ok(id)
    }

    /// A connection attempt begins: forget the last failure for it.
    fn start_attempt(&mut self, ssid: &str) {
        self.drop_lease();
        self.attempt = Some(ssid.to_string());
        self.manual = Some(ssid.to_string());
        self.next_dhcp = None;
        self.set(|s| {
            s.state = State::Connecting;
            s.notes.remove(ssid);
        });
    }

    fn note(&self, ssid: &str, text: &str) {
        self.set(|s| {
            s.notes.insert(ssid.to_string(), text.to_string());
        });
    }

    /// Turns authentication failures reported by the supplicant into notes.
    fn read_events(&self) {
        let lines = self.events.as_ref().map(|e| e.events()).unwrap_or_default();
        for ev in lines.iter().filter_map(|l| wifi_event::parse(l)) {
            if let Some(ssid) = ev.ssid.or_else(|| self.attempt.clone()) {
                self.note(&ssid, ev.failure.note());
            }
        }
    }

    /// Stops the supplicant joining `ssid` by itself.
    fn disable_live(&self, ssid: &str) {
        for id in self.ids_of(ssid).unwrap_or_default() {
            let _ = self.ctrl().and_then(|c| c.ok(&format!("DISABLE_NETWORK {id}")));
        }
    }

    fn apply(&self, l: &dhcp::Lease) -> std::io::Result<()> {
        let idx = netlink::ifindex(&self.iface).ok_or_else(|| std::io::Error::other("wlan0 missing"))?;
        let mut nl = netlink::Netlink::open()?;
        if let Some((old, _)) = &self.lease {
            if old.ip != l.ip {
                let _ = nl.run(netlink::del_addr(idx, old.ip, old.prefix));
            }
        }
        nl.run(netlink::add_addr(idx, l.ip, l.prefix))?;
        let _ = nl.run(netlink::subnet_route(idx, dhcp::network(l.ip, l.prefix), l.prefix));
        if let Some(gw) = l.router {
            nl.run(netlink::default_route(idx, gw, true))?;
        }
        let _ = nl.run(netlink::main_rule(true));
        if std::env::var("SPRINGBOARD_NATIVE_DNS").as_deref() == Ok("1") {
            write_native_dns(std::path::Path::new("/run/a8-wifi/resolv.conf"), &l.dns)?;
        }
        Ok(())
    }

    /// Desktop start: load the mirrored saved networks and look for the adapter.
    fn bring_up_desktop(&mut self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.local_dir())?;
        self.nets = wifi_config::parse_conf(&std::fs::read_to_string(self.conf_path()).unwrap_or_default());
        self.desktop_attach();
        Ok(())
    }

    /// Finds the wireless interface and connects to its supplicant; without one the page is
    /// read-only (the host's manager owns the adapter) or says there is no adapter.
    fn desktop_attach(&mut self) {
        self.ctrl = None;
        self.events = None;
        self.iface = wireless_iface(std::path::Path::new("/sys/class/net")).unwrap_or_default();
        if self.iface.is_empty() {
            self.set(|s| *s = Status { state: State::Off, ethernet: s.ethernet.take(), ..Status::default() });
            return;
        }
        let (local, dir) = (self.local_dir(), self.ctrl_dir());
        let Some(c) = wpa::Ctrl::connect_in(&local, &dir, &self.iface).ok().filter(|c| c.request("PING").is_ok_and(|r| r.starts_with("PONG"))) else {
            self.set(|s| s.read_only = true);
            return;
        };
        self.events = wpa::Ctrl::connect_events_in(&local, &dir, &self.iface).ok();
        self.ctrl = Some(c);
        self.host_ip = host_runs_dhcp();
        // networks saved here but unknown to the supplicant go back in (its config may be volatile)
        let live = self.ctrl().and_then(|c| c.request("LIST_NETWORKS")).map(|l| wpa::parse_networks(&l)).unwrap_or_default();
        for (i, n) in self.nets.iter().enumerate() {
            if !live.iter().any(|(_, s)| *s == n.ssid) {
                let _ = self.add_network(n, wifi_config::priority_of(i, self.nets.len()));
            }
        }
        self.publish_known();
        self.set(|s| {
            s.read_only = false;
            s.state = State::Searching;
            s.error = None;
        });
    }

    /// One desktop pass: reattach when the adapter or supplicant appears, poll it, read the wired link.
    fn desktop_tick(&mut self) {
        let (wall, mono) = (SystemTime::now(), Instant::now());
        let asleep = slept(wall.duration_since(self.last_wall).unwrap_or_default(), mono.duration_since(self.last_mono));
        (self.last_wall, self.last_mono) = (wall, mono);
        if asleep && self.enabled {
            self.resumed();
        }
        if self.next_eth.is_none_or(|t| Instant::now() >= t) {
            self.next_eth = Some(Instant::now() + ETH_EVERY);
            let eth = ethernet(std::path::Path::new("/sys/class/net"));
            self.set(|s| s.ethernet = eth);
        }
        if !self.enabled {
            return;
        }
        if self.ctrl.is_none() {
            if self.next_probe.is_none_or(|t| Instant::now() >= t) {
                self.next_probe = Some(Instant::now() + PROBE_EVERY);
                self.desktop_attach();
                if self.ctrl.is_none() && !self.iface.is_empty() {
                    self.read_only_status();
                }
            }
            return;
        }
        if let Err(e) = self.poll() {
            // the supplicant went away: drop the sockets and probe again
            self.ctrl = None;
            self.events = None;
            self.set(|s| s.error = Some(e.to_string()));
        }
    }

    /// The machine woke from a sleep: the lease may be stale and the supplicant may sit idle, so ask for both now.
    fn resumed(&mut self) {
        let Some(c) = self.ctrl.as_ref() else { return };
        let _ = c.ok("RECONNECT");
        self.renew_due = true;
        self.next_dhcp = None;
        self.idle_since = None;
        self.last_reconnect = Some(Instant::now());
    }

    /// Adapter present but not ours to drive: show link and address from sysfs.
    fn read_only_status(&self) {
        let up = std::fs::read_to_string(format!("/sys/class/net/{}/operstate", self.iface)).is_ok_and(|s| s.trim() == "up");
        let ip = if up { iface_ipv4(&self.iface) } else { None };
        let router = default_gateway(&std::fs::read_to_string("/proc/net/route").unwrap_or_default()).filter(|(i, _)| *i == self.iface).map(|(_, g)| g);
        self.set(|s| {
            s.read_only = true;
            s.state = match (up, ip) {
                (true, Some(_)) => State::Connected,
                (true, None) => State::Connecting,
                _ => State::Searching,
            };
            s.ip = ip;
            s.router = router;
        });
    }

    fn drop_lease(&mut self) {
        if let (Some((l, _)), Some(idx)) = (self.lease.take(), netlink::ifindex(&self.iface)) {
            if let Ok(mut nl) = netlink::Netlink::open() {
                let _ = nl.run(netlink::del_addr(idx, l.ip, l.prefix));
            }
        }
        if std::env::var("SPRINGBOARD_NATIVE_DNS").as_deref() == Ok("1") {
            let _ = write_native_dns(std::path::Path::new("/run/a8-wifi/resolv.conf"), &[]);
        }
    }
}

/// Whether the gap between two readings of the wall clock and of the monotonic clock is a sleep.
pub fn slept(wall: Duration, mono: Duration) -> bool {
    wall.saturating_sub(mono) > SLEEP_GAP
}

/// Whether to ask the supplicant to reconnect: the link has been down for `RECONNECT_AFTER`, and the last ask
/// (if any) was at least `RECONNECT_EVERY` ago.
pub fn should_reconnect(down: Duration, since_last: Option<Duration>) -> bool {
    down >= RECONNECT_AFTER && since_last.is_none_or(|s| s >= RECONNECT_EVERY)
}

/// Pause between desktop ticks: quick while something is changing (a scan, an attempt, a lease),
/// slow once connected and settled, and at the probe pace while there is no supplicant yet.
fn tick_interval(state: State, transient: bool, has_ctrl: bool) -> Duration {
    match (has_ctrl, transient, matches!(state, State::Connected)) {
        (true, true, _) => Duration::from_millis(700),
        (true, false, true) => Duration::from_secs(10),
        // settled but not connected (searching, idle): nothing is changing, so tick at the probe pace
        (true, false, false) => PROBE_EVERY,
        (false, _, _) => PROBE_EVERY,
    }
}

/// Preserve the inode: native init bind-mounts this RAM file over /etc/resolv.conf.
fn write_native_dns(path: &std::path::Path, servers: &[Ipv4Addr]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut text = String::from("# Native DHCP resolver\n");
    let mut seen = HashSet::new();
    for server in servers.iter().filter(|ip| !ip.is_unspecified() && !ip.is_multicast() && !ip.is_broadcast()) {
        if seen.insert(*server) {
            text.push_str(&format!("nameserver {server}\n"));
            if seen.len() == 3 { break; }
        }
    }
    // O_NOFOLLOW; require init's existing file instead of creating a new inode.
    let mut file = std::fs::OpenOptions::new().write(true).truncate(true).custom_flags(0x20000).open(path)?;
    file.write_all(text.as_bytes())
}

/// The first wireless interface under `sys_net` (/sys/class/net): one with a `wireless` or `phy80211` entry.
pub fn wireless_iface(sys_net: &std::path::Path) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(sys_net)
        .ok()?
        .flatten()
        .filter(|e| e.path().join("wireless").exists() || e.path().join("phy80211").exists())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| !name.starts_with("p2p"))
        .collect();
    names.sort();
    if names.iter().any(|name| name == "wlan0") {
        return Some("wlan0".into());
    }
    names.into_iter().next()
}

/// The wired link: a physical (has a `device`), non-wireless Ethernet interface, preferring one
/// with carrier. None when there is no such interface or none has a link.
pub fn ethernet(sys_net: &std::path::Path) -> Option<Ethernet> {
    let mut names: Vec<String> = std::fs::read_dir(sys_net)
        .ok()?
        .flatten()
        .filter(|e| {
            let p = e.path();
            let ether = std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "1");
            ether && p.join("device").exists() && !p.join("wireless").exists() && !p.join("phy80211").exists()
        })
        .filter(|e| std::fs::read_to_string(e.path().join("carrier")).is_ok_and(|c| c.trim() == "1"))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    let iface = names.into_iter().next()?;
    let ip = iface_ipv4(&iface);
    Some(Ethernet { iface, ip })
}

/// The default route's interface and gateway from /proc/net/route.
pub fn default_gateway(route: &str) -> Option<(String, Ipv4Addr)> {
    route.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 4 || f[1] != "00000000" || u32::from_str_radix(f[3], 16).ok()? & 2 == 0 {
            return None;
        }
        // the kernel prints addresses as little-endian hex
        let gw = u32::from_str_radix(f[2], 16).ok()?;
        Some((f[0].to_string(), Ipv4Addr::from(gw.swap_bytes())))
    })
}

/// The IPv4 address of `iface` (SIOCGIFADDR), if it has one.
pub fn iface_ipv4(iface: &str) -> Option<Ipv4Addr> {
    use std::os::fd::AsRawFd;
    extern "C" {
        fn ioctl(fd: i32, req: std::ffi::c_ulong, ...) -> i32;
    }
    const SIOCGIFADDR: std::ffi::c_ulong = 0x8915;
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    // struct ifreq: 16 bytes of name, then a sockaddr_in (family, port, address)
    let mut req = [0u8; 40];
    let name = iface.as_bytes();
    if name.len() >= 16 {
        return None;
    }
    req[..name.len()].copy_from_slice(name);
    // SAFETY: SIOCGIFADDR reads the name and fills the 40-byte ifreq we own.
    let r = unsafe { ioctl(sock.as_raw_fd(), SIOCGIFADDR, req.as_mut_ptr()) };
    (r == 0).then(|| Ipv4Addr::new(req[20], req[21], req[22], req[23])).filter(|a| !a.is_unspecified())
}

/// Whether the host runs something that does DHCP for its interfaces.
fn host_runs_dhcp() -> bool {
    const CLIENTS: [&str; 6] = ["NetworkManager", "systemd-network", "dhcpcd", "dhclient", "udhcpc", "connmand"];
    std::fs::read_dir("/proc").is_ok_and(|d| d.flatten().any(|e| std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| CLIENTS.contains(&c.trim()))))
}

/// PIDs whose command line contains `needle` as one of its arguments' substrings.
pub fn processes_with_arg(needle: &str) -> Vec<i32> {
    let Ok(dir) = std::fs::read_dir("/proc") else { return vec![] };
    dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|c| c.split(|&b| b == 0).any(|a| String::from_utf8_lossy(a).contains(needle))))
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_sleep_is_a_wall_clock_gap_the_monotonic_clock_did_not_see() {
        assert!(!slept(Duration::from_secs(10), Duration::from_secs(10)), "a normal slow tick");
        assert!(!slept(Duration::from_secs(14), Duration::from_secs(10)), "under the gap");
        assert!(slept(Duration::from_secs(120), Duration::from_secs(10)), "suspended for about two minutes");
        assert!(!slept(Duration::from_secs(3), Duration::from_secs(10)), "the monotonic clock ran ahead");
    }

    #[test]
    fn a_lost_link_is_asked_back_after_a_pause_and_not_too_often() {
        assert!(!should_reconnect(Duration::from_secs(9), None), "a short dip is the supplicant's to fix");
        assert!(should_reconnect(Duration::from_secs(10), None));
        assert!(!should_reconnect(Duration::from_secs(30), Some(Duration::from_secs(5))), "asked a moment ago");
        assert!(should_reconnect(Duration::from_secs(30), Some(Duration::from_secs(20))));
    }

    #[test]
    fn tick_interval_backs_off_when_settled() {
        assert_eq!(tick_interval(State::Connected, false, true), Duration::from_secs(10));
        assert_eq!(tick_interval(State::Connected, true, true), Duration::from_millis(700));
        assert_eq!(tick_interval(State::Searching, false, true), PROBE_EVERY);
        assert_eq!(tick_interval(State::Searching, true, true), Duration::from_millis(700));
        assert_eq!(tick_interval(State::Off, false, false), PROBE_EVERY);
    }
    use super::*;

    #[test]
    fn native_dns_updates_the_bound_inode_and_clears_old_lease() {
        use std::os::unix::fs::MetadataExt;
        let root = std::env::temp_dir().join(format!("sb-dns-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("resolv.conf");
        std::fs::write(&path, "old resolver").unwrap();
        let inode = std::fs::metadata(&path).unwrap().ino();
        let a = Ipv4Addr::new(192, 0, 2, 1);
        write_native_dns(&path, &[Ipv4Addr::UNSPECIFIED, a, a, Ipv4Addr::BROADCAST]).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Native DHCP resolver\nnameserver 192.0.2.1\n");
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
        write_native_dns(&path, &[]).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("nameserver"));
        let link = root.join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(write_native_dns(&link, &[a]).is_err());
        assert!(write_native_dns(&root.join("missing"), &[a]).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn older_conf_gains_wowlan_triggers_once() {
        let old = "ctrl_interface=/d/wpa\nupdate_config=1\n\nnetwork={\n\tssid=\"A\"\n}\n";
        let new = with_wowlan(old).unwrap();
        assert_eq!(new, format!("wowlan_triggers=any\n{old}"));
        assert_eq!(with_wowlan(&new), None);
    }

    #[test]
    fn switch_flips_show_before_the_manager_acts() {
        let mut s = Status::default();
        anticipate(&mut s, &Cmd::Enable(true));
        assert_eq!(s.state, State::Starting);
        s = Status { state: State::Connected, ssid: Some("WIFI".into()), saved: vec!["WIFI".into()], ..Status::default() };
        anticipate(&mut s, &Cmd::Scan);
        assert_eq!(s.state, State::Connected);
        anticipate(&mut s, &Cmd::Enable(true));
        assert_eq!(s.state, State::Connected, "already on");
        anticipate(&mut s, &Cmd::Enable(false));
        assert_eq!(s, Status::default());
    }

    #[test]
    fn gateway_is_read_from_the_default_route() {
        let t = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
            eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\n\
            eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\n";
        assert_eq!(default_gateway(t), Some(("eth0".into(), Ipv4Addr::new(192, 168, 1, 1))));
        assert_eq!(default_gateway("Iface\tDestination\tGateway\tFlags\n"), None);
    }

    #[test]
    fn finds_wireless_and_wired_interfaces() {
        let root = std::env::temp_dir().join(format!("sb-net-{}", std::process::id()));
        let mk = |n: &str, files: &[(&str, &str)], dirs: &[&str]| {
            let d = root.join(n);
            std::fs::create_dir_all(&d).unwrap();
            for (f, c) in files {
                std::fs::write(d.join(f), c).unwrap();
            }
            for x in dirs {
                std::fs::create_dir_all(d.join(x)).unwrap();
            }
        };
        mk("lo", &[("type", "772\n"), ("carrier", "1\n")], &[]);
        mk("wlp2s0", &[("type", "1\n"), ("carrier", "0\n")], &["wireless", "device"]);
        mk("enp1s0", &[("type", "1\n"), ("carrier", "1\n")], &["device"]);
        mk("veth0", &[("type", "1\n"), ("carrier", "1\n")], &[]);
        assert_eq!(wireless_iface(&root), Some("wlp2s0".into()));
        mk("p2p0", &[], &["wireless"]);
        assert_eq!(wireless_iface(&root), Some("wlp2s0".into()));
        mk("wlan1", &[], &["wireless"]);
        mk("wlan0", &[], &["wireless"]);
        assert_eq!(wireless_iface(&root), Some("wlan0".into()));
        assert_eq!(ethernet(&root).map(|e| e.iface), Some("enp1s0".into()));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(wireless_iface(&root), None);
    }
}
