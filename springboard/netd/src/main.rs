//! The network service: the one process that owns Wi-Fi (supplicant, DHCP, routes). The shell and
//! apps talk to it over `BIFROST_SERVICE_SOCK` (frames from `bifrost_proto::svc::net`), so a
//! shell restart or update never drops the connection.

use bifrost_proto::svc;
use bifrost_proto::svc::net::{Ap, Cmd, Evt, Status};
use springboard::net::wifi::{self, State, Wifi};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ID: &str = "is.olibuijr.net";
const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Core {
    wifi: Wifi,
    /// Status listeners, each with the id its connection removes it by when the connection ends.
    subscribers: Vec<(u64, UnixStream)>,
    next_sub: u64,
}

fn status(s: wifi::Status) -> Status {
    let mut notes: Vec<(String, String)> = s.notes.into_iter().collect();
    notes.sort();
    Status {
        state: match s.state {
            State::Off => 0,
            State::Starting => 1,
            State::Searching => 2,
            State::Connecting => 3,
            State::Connected => 4,
        },
        ssid: s.ssid,
        ip: s.ip.map(|a| a.to_string()),
        router: s.router.map(|a| a.to_string()),
        rssi: s.rssi,
        scan: s.scan.into_iter().map(|r| Ap { ssid: r.ssid, bssid: r.bssid, signal: r.signal, secure: r.secure, enterprise: r.enterprise }).collect(),
        saved: s.saved,
        notes,
        error: s.error,
    }
}

impl Core {
    fn status(&self) -> Status {
        status(self.wifi.snapshot())
    }

    fn broadcast(&mut self, evt: &Evt) {
        let bytes = evt.encode();
        self.subscribers.retain_mut(|(_, s)| svc::write_frame(s, &bytes).is_ok());
    }

    fn apply(&mut self, cmd: Cmd) -> Option<Evt> {
        let c = match cmd {
            Cmd::Ping => return Some(Evt::Pong { id: ID.into(), version: VERSION.into() }),
            Cmd::Subscribe => return Some(Evt::Status(self.status())),
            Cmd::Scan => wifi::Cmd::Scan,
            Cmd::Join { ssid, psk } => wifi::Cmd::Join { ssid, psk },
            Cmd::Select(s) => wifi::Cmd::Select(s),
            Cmd::Forget(s) => wifi::Cmd::Forget(s),
            Cmd::SetAutoJoin { ssid, on } => wifi::Cmd::SetAutoJoin { ssid, on },
            Cmd::Move { ssid, up } => wifi::Cmd::Move { ssid, up },
            Cmd::Enable(on) => wifi::Cmd::Enable(on),
        };
        self.wifi.send(c);
        let evt = Evt::Status(self.status());
        self.broadcast(&evt);
        None
    }
}

/// Wakes the status ticker when the first subscriber arrives (it parks while nobody listens).
static SUBSCRIBED: std::sync::Condvar = std::sync::Condvar::new();

fn serve(core: Arc<Mutex<Core>>, mut stream: UnixStream) {
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    let Ok(writer) = stream.try_clone() else { return };
    let mut writer = Some(writer);
    let mut sub_id: Option<u64> = None;
    while let Ok(frame) = svc::read_frame(&mut stream) {
        let Ok(cmd) = Cmd::decode(&frame) else { continue };
        let subscribe = cmd == Cmd::Subscribe;
        let mut c = core.lock().unwrap();
        let reply = c.apply(cmd);
        if subscribe {
            if let Some(w) = writer.take() {
                let id = c.next_sub + 1;
                c.next_sub = id;
                sub_id = Some(id);
                c.subscribers.push((id, w));
                SUBSCRIBED.notify_all();
            }
        }
        if let Some(r) = reply {
            if svc::write_frame(&mut stream, &r.encode()).is_err() {
                break;
            }
        }
    }
    // the connection is gone: drop its status writer now rather than at the next status change
    if let Some(id) = sub_id {
        drop_subscriber(&mut core.lock().unwrap().subscribers, id);
    }
}

/// Removes the status listener `id`; its writer closes when the list lets go of it.
fn drop_subscriber(subscribers: &mut Vec<(u64, UnixStream)>, id: u64) {
    subscribers.retain(|(i, _)| *i != id);
}

fn main() -> std::io::Result<()> {
    let sock = std::env::var("BIFROST_SERVICE_SOCK").unwrap_or_else(|_| format!("/var/lib/springboard/run/{ID}.sock"));
    let dir = std::env::var("BIFROST_SHELL_DIR").unwrap_or_else(|_| "/var/lib/springboard".into());
    let core = Arc::new(Mutex::new(Core { wifi: Wifi::start(&dir), subscribers: Vec::new(), next_sub: 0 }));
    // socket activation: bifrostd binds the socket and starts this process on the first connection
    let listener = svc::listen(std::path::Path::new(&sock))?;
    eprintln!("netd {VERSION} listening on {sock}");

    // the Wi-Fi thread changes the status on its own (scans, DHCP, signal); push every change
    let ticker = core.clone();
    std::thread::spawn(move || {
        let mut last: Option<Status> = None;
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let mut c = ticker.lock().unwrap();
            while c.subscribers.is_empty() {
                c = SUBSCRIBED.wait(c).unwrap();
            }
            let now = c.status();
            if last.as_ref() != Some(&now) {
                last = Some(now.clone());
                c.broadcast(&Evt::Status(now));
            }
        }
    });

    for stream in listener.incoming().flatten() {
        let core = core.clone();
        std::thread::spawn(move || serve(core, stream));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_connection_removes_only_its_own_status_listener() {
        let mut subs: Vec<(u64, UnixStream)> = Vec::new();
        let (a, _a_peer) = UnixStream::pair().expect("pair");
        let (b, _b_peer) = UnixStream::pair().expect("pair");
        subs.push((1, a));
        subs.push((2, b));
        drop_subscriber(&mut subs, 1);
        assert_eq!(subs.iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![2]);
        drop_subscriber(&mut subs, 1);
        assert_eq!(subs.len(), 1, "an id that is gone changes nothing");
    }

    #[test]
    fn state_maps_to_wire_numbers() {
        for (s, n) in [(State::Off, 0), (State::Starting, 1), (State::Searching, 2), (State::Connecting, 3), (State::Connected, 4)] {
            let w = status(wifi::Status { state: s, ..Default::default() });
            assert_eq!(w.state, n);
        }
    }

    #[test]
    fn addresses_become_strings_and_notes_are_sorted() {
        let mut s = wifi::Status::default();
        s.ip = Some("192.168.20.140".parse().unwrap());
        s.router = Some("192.168.20.1".parse().unwrap());
        s.notes.insert("b".into(), "2".into());
        s.notes.insert("a".into(), "1".into());
        let w = status(s);
        assert_eq!(w.ip.as_deref(), Some("192.168.20.140"));
        assert_eq!(w.router.as_deref(), Some("192.168.20.1"));
        assert_eq!(w.notes, vec![("a".to_string(), "1".to_string()), ("b".to_string(), "2".to_string())]);
    }

    #[test]
    fn default_status_has_no_addresses() {
        let w = status(wifi::Status::default());
        assert!(w.ip.is_none() && w.router.is_none() && w.scan.is_empty());
    }
}
