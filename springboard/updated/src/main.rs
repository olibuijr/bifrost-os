//! The update agent: background OS and app update checks, system package installs and the OS
//! download, kept out of the shell. The shell tells it which version it runs and what the device is
//! doing (`Conditions`), and shows the `State` it pushes (badges, Software Update page).

use bifrost_proto::svc;
use bifrost_proto::svc::update::{Cmd, Evt, Offer, Phase, State};
use springboard::install_log;
use springboard::shell::ota::{self, Ota};
use springboard::shell::update::{self, State as UpdState};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ID: &str = "is.olibuijr.update";
const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Core {
    ota: Ota,
    dir: PathBuf,
    version: String,
    git: String,
    auto_download: bool,
    wifi: bool,
    low_power: bool,
    charging: bool,
    started: Instant,
    subscribers: Vec<UnixStream>,
    /// the last failure written to the install log, so one failure is recorded once
    last_failure: Option<String>,
}

fn offer(m: &update::Manifest) -> Offer {
    Offer { version: m.version.clone(), build: m.build.clone(), sha256: m.sha256.clone(), size: m.size, url: m.url.clone(), notes: m.notes.clone() }
}

impl Core {
    fn state(&self) -> State {
        let (phase, off, error) = match &self.ota.updater.state {
            UpdState::Idle => (Phase::Idle, self.ota.os.as_ref(), None),
            UpdState::Checking => (Phase::Checking, self.ota.os.as_ref(), None),
            UpdState::UpToDate => (Phase::UpToDate, None, None),
            UpdState::Available(m) => (Phase::Available, Some(m), None),
            UpdState::Downloading => (Phase::Downloading, self.ota.os.as_ref(), None),
            UpdState::Staged(m) => (Phase::Staged, Some(m), None),
            UpdState::Failed(e) => (Phase::Failed, self.ota.os.as_ref(), Some(e.clone())),
        };
        let phase = if phase == Phase::Idle && off.is_some() { Phase::Available } else { phase };
        State { phase, offer: off.map(offer), fraction: self.ota.updater.fraction(), apps: self.ota.apps.clone(), system: self.ota.system.clone(), error }
    }

    fn broadcast(&mut self, evt: &Evt) {
        let bytes = evt.encode();
        self.subscribers.retain_mut(|s| svc::write_frame(s, &bytes).is_ok());
    }

    fn apply(&mut self, cmd: Cmd) -> Option<Evt> {
        match cmd {
            Cmd::Ping => return Some(Evt::Pong { id: ID.into(), version: VERSION.into() }),
            Cmd::Subscribe => return Some(Evt::State(self.state())),
            Cmd::Configure { version, git, auto_download, url } => {
                if url.as_deref().filter(|u| !u.is_empty()).unwrap_or(update::DEFAULT_URL) != self.ota.updater.url() {
                    self.ota = Ota::new(self.dir.clone(), url.as_deref());
                }
                self.version = version;
                self.git = git;
                self.auto_download = auto_download;
            }
            Cmd::Conditions { wifi, low_power, charging } => {
                self.wifi = wifi;
                self.low_power = low_power;
                self.charging = charging;
                return None;
            }
            Cmd::Check => self.ota.updater.check(),
            Cmd::Download => self.ota.updater.start_download(),
            Cmd::Schedule { automatic } => {
                // the switch is a file in the data directory, so the shell's own checker honours it too
                if let Err(e) = ota::set_schedule(&self.dir, automatic) {
                    eprintln!("updated: schedule: {e}");
                }
            }
        }
        let evt = Evt::State(self.state());
        self.broadcast(&evt);
        None
    }

    /// Writes a system update failure to the install log once; a failure that clears re-arms the log.
    fn record_failure(&mut self) {
        match &self.ota.updater.state {
            UpdState::Failed(e) => {
                if self.last_failure.as_deref() != Some(e.as_str()) {
                    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
                    if let Err(err) = install_log::append(&self.dir, unix, "system update", e) {
                        eprintln!("updated: install log: {err}");
                    }
                    self.last_failure = Some(e.clone());
                }
            }
            _ => self.last_failure = None,
        }
    }

    /// Advances the checker and pushes what changed; once a second.
    fn tick(&mut self) {
        if self.version.is_empty() {
            return;
        }
        let now = self.started.elapsed().as_secs_f64();
        let changed = self.ota.tick(now, self.wifi, self.low_power, self.charging, self.auto_download, &self.version.clone(), &self.git.clone());
        self.record_failure();
        let installed = self.ota.take_installed();
        if !installed.is_empty() {
            self.broadcast(&Evt::SystemInstalled(installed.iter().map(|a| a.id.clone()).collect()));
        }
        if changed || matches!(self.ota.updater.state, UpdState::Downloading) {
            let evt = Evt::State(self.state());
            self.broadcast(&evt);
        }
    }
}

fn serve(core: Arc<Mutex<Core>>, mut stream: UnixStream) {
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    let Ok(writer) = stream.try_clone() else { return };
    let mut writer = Some(writer);
    while let Ok(frame) = svc::read_frame(&mut stream) {
        let Ok(cmd) = Cmd::decode(&frame) else { continue };
        let subscribe = cmd == Cmd::Subscribe;
        let mut c = core.lock().unwrap();
        let reply = c.apply(cmd);
        if subscribe {
            if let Some(w) = writer.take() {
                c.subscribers.push(w);
            }
        }
        if let Some(r) = reply {
            if svc::write_frame(&mut stream, &r.encode()).is_err() {
                break;
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    // the shell's data directory by user: /var/lib/springboard for root, else the user's
    // (where Updater checks pacman instead of the OTA server)
    let root = std::fs::metadata("/proc/self").is_ok_and(|m| std::os::unix::fs::MetadataExt::uid(&m) == 0);
    let default_dir = springboard::bare::platform::data_dir(root, std::env::var("XDG_DATA_HOME").ok().as_deref(), std::env::var("HOME").ok().as_deref());
    let dir = PathBuf::from(std::env::var("BIFROST_SHELL_DIR").unwrap_or(default_dir));
    let sock = std::env::var("BIFROST_SERVICE_SOCK").unwrap_or_else(|_| format!("{}/run/{ID}.sock", dir.display()));
    let core = Arc::new(Mutex::new(Core {
        ota: Ota::new(dir.clone(), None),
        dir,
        version: String::new(),
        git: String::new(),
        auto_download: false,
        wifi: false,
        low_power: false,
        charging: false,
        started: Instant::now(),
        subscribers: Vec::new(),
        last_failure: None,
    }));
    if let Some(parent) = std::path::Path::new(&sock).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // socket activation: bifrostd binds the socket and starts this process on the first connection
    let listener = svc::listen(std::path::Path::new(&sock))?;
    eprintln!("updated {VERSION} listening on {sock}");

    let ticker = core.clone();
    std::thread::spawn(move || loop {
        let busy = {
            let mut c = ticker.lock().unwrap();
            c.tick();
            // a download needs a progress push every second; everything else is deadline work on the scale of minutes
            matches!(c.ota.updater.state, UpdState::Downloading | UpdState::Checking)
        };
        std::thread::sleep(Duration::from_secs(if busy { 1 } else { 15 }));
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

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("updated-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn core_in(dir: PathBuf) -> Core {
        Core { ota: Ota::new(dir.clone(), None), dir, version: String::new(), git: String::new(), auto_download: false, wifi: false, low_power: false, charging: false, started: Instant::now(), subscribers: Vec::new(), last_failure: None }
    }

    fn core(name: &str) -> Core {
        core_in(tmp(name))
    }

    fn manifest(v: &str) -> update::Manifest {
        update::Manifest { version: v.into(), build: Some("abc".into()), sha256: "00".into(), size: 7, url: "http://x/springboard".into(), notes: None, changelog: vec![] }
    }

    #[test]
    fn ping_answers_with_id_and_version() {
        let mut c = core("ping");
        match c.apply(Cmd::Ping) {
            Some(Evt::Pong { id, version }) => {
                assert_eq!(id, ID);
                assert_eq!(version, VERSION);
            }
            _ => panic!("expected pong"),
        }
    }

    #[test]
    fn fresh_core_has_no_error() {
        let s = core("idle").state();
        assert!(s.phase == Phase::Idle || s.phase == Phase::Available);
        assert!(s.error.is_none());
    }

    #[test]
    fn state_maps_every_updater_phase() {
        let mut c = core("phases");
        c.ota.os = None;
        c.ota.updater.state = UpdState::UpToDate;
        assert!(c.state().phase == Phase::UpToDate && c.state().offer.is_none());
        c.ota.updater.state = UpdState::Checking;
        assert!(c.state().phase == Phase::Checking);
        c.ota.updater.state = UpdState::Available(manifest("9.9.9"));
        let s = c.state();
        assert!(s.phase == Phase::Available);
        assert_eq!(s.offer.unwrap().version, "9.9.9");
        c.ota.updater.state = UpdState::Staged(manifest("9.9.9"));
        assert!(c.state().phase == Phase::Staged);
        c.ota.updater.state = UpdState::Downloading;
        assert!(c.state().phase == Phase::Downloading);
    }

    #[test]
    fn failed_state_carries_the_error_and_keeps_the_known_offer() {
        let mut c = core("failed");
        c.ota.os = Some(manifest("1.2.3"));
        c.ota.updater.state = UpdState::Failed("hash mismatch".into());
        let s = c.state();
        assert!(s.phase == Phase::Failed);
        assert_eq!(s.error.as_deref(), Some("hash mismatch"));
        assert_eq!(s.offer.unwrap().version, "1.2.3");
    }

    #[test]
    fn idle_with_a_known_os_offer_reports_available() {
        let mut c = core("idle-offer");
        c.ota.updater.state = UpdState::Idle;
        c.ota.os = Some(manifest("2.0.0"));
        assert!(c.state().phase == Phase::Available);
    }

    #[test]
    fn up_to_date_hides_a_stale_offer() {
        let mut c = core("uptodate");
        c.ota.os = Some(manifest("2.0.0"));
        c.ota.updater.state = UpdState::UpToDate;
        assert!(c.state().offer.is_none());
    }

    #[test]
    fn conditions_are_stored_and_need_no_reply() {
        let mut c = core("cond");
        assert!(c.apply(Cmd::Conditions { wifi: true, low_power: true, charging: true }).is_none());
        assert!(c.wifi && c.low_power && c.charging);
        assert!(c.apply(Cmd::Conditions { wifi: false, low_power: false, charging: false }).is_none());
        assert!(!c.wifi && !c.low_power && !c.charging);
    }

    #[test]
    fn configure_sets_version_and_is_idempotent() {
        let mut c = core("configure");
        let cfg = || Cmd::Configure { version: "0.5.4".into(), git: "deadbeef".into(), auto_download: true, url: None };
        assert!(c.apply(cfg()).is_none());
        let url = c.ota.updater.url().to_string();
        assert!(c.apply(cfg()).is_none());
        assert_eq!((c.version.as_str(), c.git.as_str(), c.auto_download), ("0.5.4", "deadbeef", true));
        assert_eq!(c.ota.updater.url(), url, "same config must not rebuild the updater");
        assert_eq!(url, update::DEFAULT_URL);
    }

    #[test]
    fn configure_empty_url_means_default_and_new_url_replaces() {
        let mut c = core("url");
        c.apply(Cmd::Configure { version: "1".into(), git: String::new(), auto_download: false, url: Some(String::new()) });
        assert_eq!(c.ota.updater.url(), update::DEFAULT_URL);
        c.apply(Cmd::Configure { version: "1".into(), git: String::new(), auto_download: false, url: Some("http://127.0.0.1:1/ota".into()) });
        assert_eq!(c.ota.updater.url(), "http://127.0.0.1:1/ota");
    }

    #[test]
    fn tick_without_a_version_does_nothing() {
        let mut c = core("tick");
        c.tick();
        assert!(c.version.is_empty());
        assert!(matches!(c.ota.updater.state, UpdState::Idle | UpdState::Staged(_)));
    }

    #[test]
    fn a_binary_staged_earlier_counts_as_staged_after_restart() {
        let dir = tmp("staged");
        std::fs::write(dir.join("springboard.new"), b"bin").unwrap();
        let c = core_in(dir);
        assert!(matches!(c.ota.updater.state, UpdState::Staged(_)));
        assert!(c.state().phase == Phase::Staged);
    }

    #[test]
    fn broadcast_drops_closed_subscribers_and_keeps_live_ones() {
        let mut c = core("bcast");
        let (a, mut a_peer) = UnixStream::pair().unwrap();
        let (b, b_peer) = UnixStream::pair().unwrap();
        drop(b_peer);
        c.subscribers.push(a);
        c.subscribers.push(b);
        for _ in 0..3 {
            c.broadcast(&Evt::Pong { id: "x".into(), version: "1".into() });
        }
        assert_eq!(c.subscribers.len(), 1);
        assert!(svc::read_frame(&mut a_peer).is_ok());
    }

    #[test]
    fn schedule_command_switches_the_background_checks_by_file() {
        let mut c = core("schedule");
        assert!(ota::schedule_on(&c.dir));
        c.apply(Cmd::Schedule { automatic: false });
        assert!(!ota::schedule_on(&c.dir));
        c.apply(Cmd::Schedule { automatic: true });
        assert!(ota::schedule_on(&c.dir));
    }

    #[test]
    fn offer_copies_the_manifest_fields() {
        let o = offer(&manifest("3.1.4"));
        assert_eq!((o.version.as_str(), o.size, o.sha256.as_str()), ("3.1.4", 7, "00"));
        assert_eq!(o.build.as_deref(), Some("abc"));
    }
}
