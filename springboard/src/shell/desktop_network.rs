//! Desktop networking and power state. All D-Bus/CLI work runs off the UI thread.
use super::{Network, SavedNetwork, WifiAction};
use std::{
    process::Command,
    sync::{mpsc, Arc, Mutex, OnceLock},
    time::Duration,
};
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
}
pub enum Request {
    Wifi(WifiAction),
    Power(String),
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
            loop {
                let mut value = read();
                value.error = error.clone();
                if let Ok(mut s) = out.lock() {
                    if *s != value { *s = value; let _=notify.send(&[1]); quiet = 0; } else { quiet = (quiet + 1).min(3); }
                }
                match rx.recv() {
                    Ok(WorkerMessage::Request(request)) => { error = apply(request).err(); }
                    Ok(WorkerMessage::Changed) => {
                        std::thread::sleep(Duration::from_millis(1000u64 << quiet));
                        queued.store(false,std::sync::atomic::Ordering::Release);
                    }
                    Err(_) => break,
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
                        s.saved.push(SavedNetwork {
                            ssid,
                            auto_join: f[2] == "yes",
                            note: String::new(),
                        });
                    }
                }
            }
        }
        for n in &mut s.networks {
            n.saved = s.saved.iter().any(|v| v.ssid == n.ssid);
        }
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
