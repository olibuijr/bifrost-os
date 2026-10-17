//! Desktop networking and power state. All D-Bus/CLI work runs off the UI thread.
use super::{Network, SavedNetwork, WifiAction};
use crate::net::wifi_config::{Eap, Security};
use bifrost_core::tunnel::{VpnProfile, VpnProtocol};
use std::{
    process::Command,
    sync::{mpsc, Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

/// The host every DNS check looks up: a generic, always-on name (the same family as the captive probe).
const DNS_CHECK_HOST: &str = "connectivitycheck.gstatic.com";
/// How long a DNS answer is kept: a lookup can wait a long time on a dead server, so it is not asked per read.
const DNS_TTL: Duration = Duration::from_secs(30);

/// Why a connected Wi-Fi link does not reach the internet yet, as the menu bar names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkProblem {
    /// associated, but DHCP gave no IPv4 address
    NoAddress,
    /// an address, but no DNS server was given or the check name does not resolve
    NoDns,
}

impl LinkProblem {
    pub fn label(self) -> &'static str {
        match self {
            LinkProblem::NoAddress => "No IP Address",
            LinkProblem::NoDns => "DNS Not Responding",
        }
    }
}

/// The problem with `link`, if it is connected but not usable. `resolves` is asked only once an address
/// and a DNS server are there, so a link without either never waits on a lookup.
pub fn link_problem(link: Option<&Interface>, resolves: impl FnOnce() -> bool) -> Option<LinkProblem> {
    let link = link.filter(|i| i.connected())?;
    if link.ipv4.is_empty() {
        return Some(LinkProblem::NoAddress);
    }
    if link.dns.is_empty() || !resolves() {
        return Some(LinkProblem::NoDns);
    }
    None
}

/// Whether the check host resolves, asked at most once per `DNS_TTL`.
fn dns_resolves() -> bool {
    static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    use std::net::ToSocketAddrs;
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, ok)) = *cache {
        if at.elapsed() < DNS_TTL {
            return ok;
        }
    }
    let ok = (DNS_CHECK_HOST, 80).to_socket_addrs().is_ok_and(|mut a| a.next().is_some());
    *cache = Some((Instant::now(), ok));
    ok
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Interface {
    pub name: String,
    pub kind: String,
    pub status: String,
    pub connection: String,
    pub uuid: String,
    pub mac: String,
    pub ipv4: Vec<String>,
    pub ipv6: Vec<String>,
    pub gateway: String,
    pub dns: Vec<String>,
    pub method: String,
}
impl Interface {
    pub fn connected(&self) -> bool {
        self.status.starts_with("100")
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub interfaces: Vec<Interface>,
    pub wifi_available: bool,
    pub wifi_on: bool,
    pub ssid: Option<String>,
    pub networks: Vec<Network>,
    pub saved: Vec<SavedNetwork>,
    pub power_profiles: Vec<String>,
    pub power: String,
    pub error: Option<String>,
    /// the Wi-Fi link is up but not usable (no address, no DNS); None when it is fine or not connected
    pub problem: Option<LinkProblem>,
    /// the VPN and WireGuard connections NetworkManager holds, with which ones are up
    pub vpns: Vec<VpnConnection>,
}

/// A VPN configuration NetworkManager holds: a VPN plug-in connection (`vpn`) or a WireGuard one (`wireguard`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VpnConnection {
    pub name: String,
    pub kind: String,
    /// whether the tunnel is up now
    pub active: bool,
}

impl VpnConnection {
    /// What the VPN page shows beside the name.
    pub fn status_label(&self) -> &'static str {
        if self.active {
            "Connected"
        } else {
            "Not Connected"
        }
    }
}

/// The VPN connections in `nmcli -t -f NAME,TYPE,ACTIVE connection show`: lines of name, type and `yes` or `no`, with
/// colons in a name escaped. Connections of other types (Wi-Fi, Ethernet, bridges) are left out.
pub fn parse_vpn_connections(text: &str) -> Vec<VpnConnection> {
    text.lines()
        .filter_map(|line| {
            let f = fields(line);
            if f.len() < 3 {
                return None;
            }
            let (active, kind) = (f[f.len() - 1].as_str(), f[f.len() - 2].as_str());
            if !matches!(kind, "vpn" | "wireguard") {
                return None;
            }
            let name = f[..f.len() - 2].join(":");
            (!name.trim().is_empty()).then(|| VpnConnection { name, kind: kind.to_string(), active: active == "yes" })
        })
        .collect()
}

/// The `nmcli` arguments that bring the VPN connection `name` up or down. Refuses a blank name or one with a control
/// character (the name is a single argument, so it cannot add options).
pub fn vpn_args(name: &str, up: bool) -> Result<Vec<String>, String> {
    if name.trim().is_empty() || name.chars().any(char::is_control) {
        return Err("That VPN name cannot be used".into());
    }
    let args: &[&str] = if up { &["--wait", "6", "connection", "up", "id"] } else { &["connection", "down", "id"] };
    Ok(args.iter().map(|a| a.to_string()).chain([name.to_string()]).collect())
}
pub enum Request {
    Wifi(WifiAction),
    Power(String),
    /// Offers a network from this machine: an access point or an ad-hoc network.
    Offer(Offer),
    /// Takes down the network offered by [`Request::Offer`].
    StopOffer,
    /// Brings the VPN connection `name` up (`up`) or down.
    Vpn { name: String, up: bool },
}

/// A network this machine offers on one wireless interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Offer {
    /// An access point (a hotspot). `psk` is the WPA passphrase or hex key; None leaves it open.
    AccessPoint { ifname: String, ssid: String, psk: Option<String> },
    /// An ad-hoc (IBSS) network, open.
    AdHoc { ifname: String, ssid: String },
}

/// The NetworkManager connection names this machine uses for the networks it offers, so a stop finds them.
pub const ACCESS_POINT_CON: &str = "bifrost-hotspot";
pub const ADHOC_CON: &str = "bifrost-adhoc";

/// The nmcli steps that bring an offered network up.
fn start_offer(offer: &Offer) -> Result<String, String> {
    match offer {
        Offer::AccessPoint { ifname, ssid, psk } => {
            let mut args: Vec<&str> = vec!["--wait", "8", "device", "wifi", "hotspot", "ifname", ifname.as_str(), "con-name", ACCESS_POINT_CON, "ssid", ssid.as_str()];
            if let Some(password) = psk {
                args.extend(["password", password.as_str()]);
            }
            output("nmcli", &args)
        }
        Offer::AdHoc { ifname, ssid } => {
            // a leftover profile from an earlier ad-hoc network is replaced, not reused
            let _ = output("nmcli", &["connection", "delete", "id", ADHOC_CON]);
            output("nmcli", &["connection", "add", "type", "wifi", "ifname", ifname.as_str(), "con-name", ADHOC_CON, "ssid", ssid.as_str(), "mode", "adhoc"])?;
            output("nmcli", &["--wait", "8", "connection", "up", "id", ADHOC_CON])
        }
    }
}

/// Takes down whichever offered network is up. Either profile may be absent; the error is the last one tried.
fn stop_offer() -> Result<String, String> {
    output("nmcli", &["connection", "down", "id", ACCESS_POINT_CON]).or_else(|_| output("nmcli", &["connection", "down", "id", ADHOC_CON]))
}
enum WorkerMessage { Request(Request), Changed }
struct Service {
    state: Arc<Mutex<Snapshot>>,
    changed: std::os::unix::net::UnixDatagram,
    tx: mpsc::Sender<WorkerMessage>,
}
fn service() -> &'static Service {
    static SERVICE: OnceLock<Service> = OnceLock::new();
    SERVICE.get_or_init(|| {
        let state = Arc::new(Mutex::new(Snapshot::default()));
        let out = state.clone();
        let (changed,notify)=std::os::unix::net::UnixDatagram::pair().expect("network notification socket");
        changed.set_nonblocking(true).expect("nonblocking network socket");
        notify.set_nonblocking(true).expect("nonblocking network notifier");
        let (tx, rx) = mpsc::channel();
        let queued=Arc::new(std::sync::atomic::AtomicBool::new(false));
        let signal_tx=tx.clone(); let signal_queued=queued.clone();
        crate::bare::sys::spawn_named("nm-monitor", move || {
            use std::io::{BufRead,BufReader};
            // Signal-driven only. Not matched on purpose: AccessPoint (Strength flaps on every scan), the Statistics and
            // Wireless interfaces (LastScan, byte counters): they woke dbus-broker and this process many times a second at idle.
            loop {
                let mut child=match Command::new("dbus-monitor").args(["--system",
                    "type='signal',sender='org.freedesktop.NetworkManager',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged',path_namespace='/org/freedesktop/NetworkManager/Devices',arg0='org.freedesktop.NetworkManager.Device'",
                    "type='signal',sender='org.freedesktop.NetworkManager',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged',path_namespace='/org/freedesktop/NetworkManager/ActiveConnection',arg0='org.freedesktop.NetworkManager.Connection.Active'",
                    "type='signal',interface='org.freedesktop.DBus.Properties',path='/org/freedesktop/UPower/PowerProfiles'",
                    "type='signal',interface='org.freedesktop.DBus.Properties',path='/net/hadess/PowerProfiles'"])
                    .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn() {
                    Ok(c)=>c, Err(_)=>{std::thread::sleep(Duration::from_secs(5));continue}
                };
                for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
                    if line.starts_with("signal ") && !signal_queued.swap(true,std::sync::atomic::Ordering::AcqRel)
                        && signal_tx.send(WorkerMessage::Changed).is_err() {return;}
                }
                let _=child.wait();
                std::thread::sleep(Duration::from_secs(5));
            }
        });
        crate::bare::sys::spawn_named("nm-worker", move || {
            let mut error = None;
            // consecutive signal-driven reads that found nothing new: the debounce grows (1 s up to 8 s)
            let mut quiet: u32 = 0;
            // when the debounced read is due; until then requests are still served at once
            let mut due: Option<Instant> = None;
            loop {
                let mut value = read();
                value.error = error.clone();
                if let Ok(mut s) = out.lock() {
                    if *s != value { *s = value; let _=notify.send(&[1]); quiet = 0; } else { quiet = (quiet + 1).min(3); }
                }
                let next = match due {
                    Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
                    None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
                };
                match next {
                    Ok(WorkerMessage::Request(request)) => { error = apply(request).err(); }
                    Ok(WorkerMessage::Changed) => {
                        due.get_or_insert_with(|| Instant::now() + Duration::from_millis(1000u64 << quiet));
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        due = None;
                        queued.store(false,std::sync::atomic::Ordering::Release);
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Service { state, tx, changed }
    })
}
pub fn fd() -> std::os::fd::RawFd {
    use std::os::fd::AsRawFd;service().changed.as_raw_fd()
}
pub fn take_changed() -> bool {
    let mut changed=false;let mut bytes=[0;64];
    while service().changed.recv(&mut bytes).is_ok() {changed=true;}
    changed
}
pub fn snapshot() -> Snapshot {
    service()
        .state
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default()
}
pub fn send(request: Request) {
    let _ = service().tx.send(WorkerMessage::Request(request));
}
fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let result = Command::new("timeout")
        .arg("8")
        .arg(program)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}
/// nmcli escapes colons/backslashes in terse output; SSIDs may contain either.
fn fields(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut escape = false;
    for c in line.chars() {
        if escape {
            out.last_mut().unwrap().push(c);
            escape = false;
        } else if c == '\\' {
            escape = true;
        } else if c == ':' {
            out.push(String::new());
        } else {
            out.last_mut().unwrap().push(c);
        }
    }
    out
}
fn read() -> Snapshot {
    let mut s = Snapshot::default();
    if let Ok(text)=output("nmcli",&["--escape","no","-t","-f","GENERAL.DEVICE,GENERAL.TYPE,GENERAL.STATE,GENERAL.CONNECTION,GENERAL.CON-UUID,GENERAL.HWADDR,IP4.ADDRESS,IP4.GATEWAY,IP4.DNS,IP6.ADDRESS","device","show"]){
        for line in text.lines(){let Some((key,value))=line.split_once(':') else{continue};
            if key=="GENERAL.DEVICE" {s.interfaces.push(Interface{name:value.into(),..Interface::default()});continue;}
            let Some(i)=s.interfaces.last_mut() else{continue};
            match key {
                "GENERAL.TYPE"=>i.kind=value.into(),"GENERAL.STATE"=>i.status=value.into(),
                "GENERAL.CONNECTION"=>i.connection=value.into(),"GENERAL.CON-UUID"=>i.uuid=value.into(),
                "GENERAL.HWADDR"=>i.mac=value.into(),"IP4.GATEWAY"=>i.gateway=value.into(),
                k if k.starts_with("IP4.ADDRESS")=>i.ipv4.push(value.into()),
                k if k.starts_with("IP6.ADDRESS")=>i.ipv6.push(value.into()),
                k if k.starts_with("IP4.DNS")=>i.dns.push(value.into()),_=>{}
            }
        }
    }
    s.interfaces
        .retain(|i| matches!(i.kind.as_str(), "ethernet" | "wifi"));
    for i in &mut s.interfaces {
        if !i.uuid.is_empty() && i.uuid != "--" {
            i.method = output(
                "nmcli",
                &["-g", "ipv4.method", "connection", "show", &i.uuid],
            )
            .unwrap_or_default()
            .trim()
            .into();
        }
    }
    s.wifi_available = s.interfaces.iter().any(|i| i.kind == "wifi");
    let wifi_link = s.interfaces.iter().find(|i| i.kind == "wifi");
    s.problem = link_problem(wifi_link, dns_resolves);
    if s.wifi_available {
        s.wifi_on = output("nmcli", &["radio", "wifi"]).is_ok_and(|v| v.trim() == "enabled");
        if let Ok(text) = output(
            "nmcli",
            &[
                "-t",
                "-f",
                "IN-USE,SSID,SIGNAL,SECURITY",
                "device",
                "wifi",
                "list",
                "--rescan",
                "no",
            ],
        ) {
            for line in text.lines() {
                let f = fields(line);
                if f.len() < 4 || f[1].is_empty() {
                    continue;
                }
                if f[0] == "*" {
                    s.ssid = Some(f[1].clone());
                }
                if !s.networks.iter().any(|n| n.ssid == f[1]) {
                    let signal = f[2].parse::<u8>().unwrap_or(0);
                    s.networks.push(Network {
                        ssid: f[1].clone(),
                        bars: if signal > 66 {
                            3
                        } else if signal > 33 {
                            2
                        } else {
                            1
                        },
                        secure: f[3] != "--" && !f[3].is_empty(),
                        saved: false,
                    });
                }
            }
        }
        if let Ok(text) = output(
            "nmcli",
            &["-t", "-f", "UUID,TYPE,AUTOCONNECT", "connection", "show"],
        ) {
            for line in text.lines() {
                let f = fields(line);
                if f.len() < 3 || f[1] != "802-11-wireless" {
                    continue;
                }
                if let Ok(ssid) = output(
                    "nmcli",
                    &["-g", "802-11-wireless.ssid", "connection", "show", &f[0]],
                ) {
                    let ssid = ssid.trim().to_owned();
                    if !ssid.is_empty() {
                        // an open network has no key management: nmcli prints an empty value, which reads as Open
                        let security = output(
                            "nmcli",
                            &["-g", "802-11-wireless-security.key-mgmt", "connection", "show", &f[0]],
                        )
                        .ok()
                        .and_then(|v| security_of(v.trim()));
                        s.saved.push(SavedNetwork {
                            ssid,
                            auto_join: f[2] == "yes",
                            note: String::new(),
                            security,
                        });
                    }
                }
            }
        }
        for n in &mut s.networks {
            n.saved = s.saved.iter().any(|v| v.ssid == n.ssid);
        }
    }
    if let Ok(text) = output("nmcli", &["-t", "-f", "NAME,TYPE,ACTIVE", "connection", "show"]) {
        s.vpns = parse_vpn_connections(&text);
    }
    if let Ok(text) = output("powerprofilesctl", &["list"]) {
        for line in text.lines() {
            let name = line
                .trim()
                .trim_start_matches('*')
                .trim()
                .trim_end_matches(':');
            if matches!(name, "performance" | "balanced" | "power-saver") {
                s.power_profiles.push(name.into());
            }
        }
    }
    s.power = output("powerprofilesctl", &["get"])
        .unwrap_or_default()
        .trim()
        .into();
    s
}
/// The security type of a saved network, from NetworkManager's `802-11-wireless-security.key-mgmt`
/// (a list such as `wpa-psk`, `sae`, `wpa-eap`; empty or `none` for an open network). None for a value
/// the Wi-Fi pages do not name, so the row shows no type rather than a wrong one.
pub fn security_of(key_mgmt: &str) -> Option<Security> {
    let words: Vec<&str> = key_mgmt
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |w: &str| words.iter().any(|x| *x == w);
    if words.is_empty() || has("none") || has("owe") {
        return Some(Security::Open);
    }
    if has("wpa-eap") || has("ieee8021x") {
        // the EAP method is not reported by the key list; the label does not show it either
        return Some(Security::Enterprise(Eap::Peap));
    }
    match (has("sae"), has("wpa-psk")) {
        (true, true) => Some(Security::Wpa2Wpa3),
        (true, false) => Some(Security::Wpa3),
        (false, true) => Some(Security::Wpa2),
        (false, false) => None,
    }
}

fn saved_uuid(ssid: &str) -> Result<String, String> {
    let text = output("nmcli", &["-t", "-f", "UUID,TYPE", "connection", "show"])?;
    for line in text.lines() {
        let f = fields(line);
        if f.len() == 2
            && f[1] == "802-11-wireless"
            && output(
                "nmcli",
                &["-g", "802-11-wireless.ssid", "connection", "show", &f[0]],
            )
            .is_ok_and(|v| v.trim() == ssid)
        {
            return Ok(f[0].clone());
        }
    }
    Err("Saved network not found".into())
}
fn apply(request: Request) -> Result<(), String> {
    let result = match request {
        Request::Power(name) => output("powerprofilesctl", &["set", &name]),
        Request::Offer(offer) => start_offer(&offer),
        Request::StopOffer => stop_offer(),
        Request::Vpn { name, up } => {
            let args = vpn_args(&name, up)?;
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            output("nmcli", &args)
        }
        Request::Wifi(action) => match action {
            WifiAction::Scan => output("nmcli", &["--wait", "5", "device", "wifi", "rescan"]),
            WifiAction::Enable(on) => {
                output("nmcli", &["radio", "wifi", if on { "on" } else { "off" }])
            }
            WifiAction::Select(ssid) => {
                let uuid = saved_uuid(&ssid)?;
                output("nmcli", &["--wait", "5", "connection", "up", "uuid", &uuid])
            }
            WifiAction::Join { ssid, psk } => match psk {
                Some(password) => output(
                    "nmcli",
                    &[
                        "--wait", "5", "device", "wifi", "connect", &ssid, "password", &password,
                    ],
                ),
                None => output(
                    "nmcli",
                    &["--wait", "5", "device", "wifi", "connect", &ssid],
                ),
            },
            WifiAction::Forget(ssid) => {
                let uuid = saved_uuid(&ssid)?;
                output("nmcli", &["connection", "delete", "uuid", &uuid])
            }
            WifiAction::SetAutoJoin { ssid, on } => {
                let uuid = saved_uuid(&ssid)?;
                output(
                    "nmcli",
                    &[
                        "connection",
                        "modify",
                        "uuid",
                        &uuid,
                        "connection.autoconnect",
                        if on { "yes" } else { "no" },
                    ],
                )
            }
            _ => return Err("This desktop network option is not supported yet".into()),
        },
    };
    result.map(|_| ())
}

/// The VPN profiles the SDK's `VpnManager` model keeps for apps. The Settings VPN page lists the connections
/// NetworkManager holds instead (`Snapshot::vpns`); no profile is stored through the model yet, so this list is empty.
pub fn vpn_profiles() -> Vec<VpnProfile> {
    Vec::new()
}

/// One line for a VPN profile in the list: the protocol, then the server and port.
pub fn vpn_summary(profile: &VpnProfile) -> String {
    let protocol = match profile.protocol {
        VpnProtocol::Ikev2 => "IKEv2",
        VpnProtocol::IpsecIkev1 => "IPsec",
        VpnProtocol::Tunnel => "Tunnel",
    };
    format!("{protocol}, {}:{}", profile.server, profile.port)
}

#[cfg(test)]
mod tests {
    use super::{parse_vpn_connections, security_of, vpn_args, vpn_profiles, vpn_summary, VpnConnection};
    use crate::net::wifi_config::{Eap, Security};
    use bifrost_core::tunnel::{VpnProfile, VpnProtocol};

    #[test]
    fn vpn_connections_are_read_from_the_terse_list_and_other_types_are_left_out() {
        let text = "Home Wi-Fi:802-11-wireless:yes\nOffice VPN:vpn:no\nwg0:wireguard:yes\nlo:loopback:yes\nCafe\\: guest:vpn:yes\n:vpn:no\nbad\n";
        let vpns = parse_vpn_connections(text);
        assert_eq!(
            vpns,
            vec![
                VpnConnection { name: "Office VPN".into(), kind: "vpn".into(), active: false },
                VpnConnection { name: "wg0".into(), kind: "wireguard".into(), active: true },
                VpnConnection { name: "Cafe: guest".into(), kind: "vpn".into(), active: true },
            ],
            "an escaped colon stays in the name, a blank name and a short line are skipped"
        );
        assert_eq!(vpns[0].status_label(), "Not Connected");
        assert_eq!(vpns[1].status_label(), "Connected");
        assert!(parse_vpn_connections("").is_empty());
    }

    #[test]
    fn the_vpn_request_is_one_nmcli_command_with_the_name_as_a_single_argument() {
        assert_eq!(vpn_args("Office VPN", true).unwrap(), ["--wait", "6", "connection", "up", "id", "Office VPN"]);
        assert_eq!(vpn_args("wg0", false).unwrap(), ["connection", "down", "id", "wg0"]);
        assert_eq!(vpn_args("--help", true).unwrap().last().map(String::as_str), Some("--help"), "the name follows id, so it is a value and not an option");
        assert!(vpn_args("  ", true).is_err());
        assert!(vpn_args("a\nb", true).is_err());
    }

    #[test]
    fn vpn_summary_names_protocol_server_and_default_port() {
        let office = VpnProfile::new("Office", " VPN.Example.com ", VpnProtocol::Ikev2);
        assert_eq!(vpn_summary(&office), "IKEv2, vpn.example.com:500", "the server is trimmed and lower-cased, port 500 by default");
        let tunnel = VpnProfile::new("Tun", "h.example", VpnProtocol::Tunnel);
        assert_eq!(vpn_summary(&tunnel), "Tunnel, h.example:443");
    }

    #[test]
    fn a_link_problem_names_the_missing_address_or_dns() {
        use crate::shell::desktop_network::{link_problem, Interface, LinkProblem};
        let up = |ipv4: &[&str], dns: &[&str]| Interface { status: "100 (connected)".into(), ipv4: ipv4.iter().map(|s| s.to_string()).collect(), dns: dns.iter().map(|s| s.to_string()).collect(), ..Interface::default() };
        let never = || -> bool { panic!("no lookup without an address and a server") };
        assert_eq!(link_problem(None, never), None, "no Wi-Fi link");
        assert_eq!(link_problem(Some(&Interface { status: "30 (disconnected)".into(), ..Interface::default() }), never), None, "not connected is not a problem here");
        assert_eq!(link_problem(Some(&up(&[], &["1.1.1.1"])), never), Some(LinkProblem::NoAddress));
        assert_eq!(link_problem(Some(&up(&["10.0.0.5/24"], &[])), never), Some(LinkProblem::NoDns));
        assert_eq!(link_problem(Some(&up(&["10.0.0.5/24"], &["10.0.0.1"])), || false), Some(LinkProblem::NoDns), "the check name does not resolve");
        assert_eq!(link_problem(Some(&up(&["10.0.0.5/24"], &["10.0.0.1"])), || true), None);
        assert_eq!(LinkProblem::NoDns.label(), "DNS Not Responding");
    }

    #[test]
    fn no_vpn_profiles_are_stored_yet() {
        assert!(vpn_profiles().is_empty(), "the VPN page shows its empty row until profiles are stored");
    }

    #[test]
    fn key_management_maps_to_the_security_the_pages_name() {
        assert_eq!(security_of(""), Some(Security::Open), "an open network prints an empty key list");
        assert_eq!(security_of("none"), Some(Security::Open));
        assert_eq!(security_of("wpa-psk"), Some(Security::Wpa2));
        assert_eq!(security_of("sae"), Some(Security::Wpa3));
        assert_eq!(security_of("wpa-psk,sae"), Some(Security::Wpa2Wpa3));
        assert_eq!(security_of("wpa-psk sae"), Some(Security::Wpa2Wpa3), "a space-separated list reads the same");
        assert_eq!(security_of("wpa-eap"), Some(Security::Enterprise(Eap::Peap)));
        assert_eq!(security_of("wpa-eap-suite-b-192"), None, "a suite name the shell does not name shows no type");
    }
}
