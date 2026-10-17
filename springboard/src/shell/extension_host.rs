//! The app extension host: finds the extensions installed packages declare, starts one on demand, talks to it, and
//! stops it when it has been idle.
//!
//! - **Discovery.** [`Host::installed`] reads the `[[extension_point]]` and `[[extension]]` tables of every installed
//!   package's `manifest.toml` into a `bifrost_ui::extension::Registry` (`bifrost_ui::extension_manifest`), without
//!   starting anything. [`Host::discover`] lists the enabled extensions of a point. An extension the user switched off in
//!   Settings > Login Items & Extensions (`login.extensions.off`) is not offered.
//! - **Launch.** The first call to an extension starts the package's own app binary in extension mode: the binary gets
//!   `--extension <point>`, `BIFROST_EXTENSION_POINT` and `BIFROST_CONTROL` (the socket it must serve,
//!   `bifrost_app::extension_service`). The host waits until the extension answers `extension.ping`, and only then marks
//!   the process running in `bifrost_core::extension_process::ProcessTable`, which also counts the calls in flight.
//! - **Calls.** A call is one JSON-RPC request on the extension's control socket ([`Lease::exchange`]). [`Host::begin`]
//!   and [`Host::end`] bracket it so the host lock is not held while the extension works.
//! - **Stop.** [`Host::reap`] stops extensions with no call in flight for the idle time ([`IDLE`]): it asks for
//!   `extension.shutdown`, then kills a process that does not leave. A process that died is dropped and started again
//!   by the next call. Dropping the host stops everything it started.
//!
//! Process and socket go through the [`Launcher`] and [`Transport`] traits, so tests run a real socket server in a
//! thread instead of a binary. Gaps: the extension runs with the host's rights, not in the app sandbox
//! (`bare::app_policy`), because the sandbox does not bind the socket directory; and a scene is not presented
//! (`remote_view`), only data calls are made.

use super::apphost::Installed;
use super::login_items_page;
use bifrost_core::extension_process::{ConnectionId, Key, ProcessId, ProcessTable};
use bifrost_core::json::{self, Json};
use bifrost_ui::extension::Identity;
use bifrost_ui::extension_manifest::{self, Scan};
use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long an extension may sit with no call in flight before [`Host::reap`] stops it.
pub const IDLE: Duration = Duration::from_secs(30);
/// How long a launch waits for the extension to answer its first ping.
pub const START_WAIT: Duration = Duration::from_secs(5);
/// The environment variable that tells a binary which point it serves (`bifrost_app::extension_service::ENV_POINT`).
pub const ENV_POINT: &str = "BIFROST_EXTENSION_POINT";
/// The control socket the binary must serve (`bifrost_app::extension_service::ENV_CONTROL`).
pub const ENV_CONTROL: &str = "BIFROST_CONTROL";
const PING: &str = "extension.ping";
const SHUTDOWN: &str = "extension.shutdown";

/// Why a call to an extension did not produce a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    /// No enabled extension with this bundle identifier serves the point.
    NotAvailable,
    /// The binary could not be started.
    Launch(String),
    /// The extension started but did not answer on its socket.
    Unreachable(String),
    /// The extension answered with an error; the text is its message (a provider error code name for providers).
    Remote(String),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::NotAvailable => f.write_str("the extension is not available"),
            HostError::Launch(e) => write!(f, "the extension did not start: {e}"),
            HostError::Unreachable(e) => write!(f, "the extension did not answer: {e}"),
            HostError::Remote(e) => f.write_str(e),
        }
    }
}

/// The package an installed extension runs from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub app_id: String,
    /// the app binary
    pub exec: PathBuf,
    /// the app's data directory, which the extension shares
    pub data_dir: PathBuf,
}

/// One installed, enabled extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub bundle_id: String,
    pub point: String,
    pub name: String,
    pub package: Package,
}

/// A started process. Dropping it does not stop it; call [`Instance::stop`].
pub trait Instance: Send {
    /// False once the process has exited.
    fn alive(&mut self) -> bool;
    /// Waits briefly for the process to leave after `extension.shutdown`, then kills it.
    fn stop(&mut self);
}

/// Starts an extension process that serves `socket`.
pub trait Launcher: Send {
    fn launch(&self, entry: &Entry, socket: &Path) -> io::Result<Box<dyn Instance>>;
}

/// Sends one request line to a socket and returns the reply text.
pub trait Transport: Send + Sync {
    /// Connects (retrying for `connect_wait` while the extension starts), sends `line`, and waits `reply_wait` for the
    /// reply.
    fn exchange(&self, socket: &Path, line: &str, connect_wait: Duration, reply_wait: Duration) -> Result<String, String>;
}

/// The real transport: the extension's Unix control socket, one request per connection.
pub struct Sockets;

impl Transport for Sockets {
    fn exchange(&self, socket: &Path, line: &str, connect_wait: Duration, reply_wait: Duration) -> Result<String, String> {
        let deadline = Instant::now() + connect_wait;
        let mut stream = loop {
            match UnixStream::connect(socket) {
                Ok(s) => break s,
                Err(e) if Instant::now() >= deadline => return Err(format!("{}: {e}", socket.display())),
                Err(_) => std::thread::sleep(Duration::from_millis(25)),
            }
        };
        let _ = stream.set_read_timeout(Some(reply_wait));
        stream.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut reply = String::new();
        stream.read_to_string(&mut reply).map_err(|e| e.to_string())?;
        Ok(reply)
    }
}

/// The real launcher: the package's binary as a child process in extension mode.
pub struct ProcessLauncher;

struct ChildInstance(Child);

impl Instance for ChildInstance {
    fn alive(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(None))
    }

    fn stop(&mut self) {
        for _ in 0..25 {
            if !self.alive() {
                let _ = self.0.wait();
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Launcher for ProcessLauncher {
    fn launch(&self, entry: &Entry, socket: &Path) -> io::Result<Box<dyn Instance>> {
        let _ = std::fs::create_dir_all(&entry.package.data_dir);
        let child = Command::new(&entry.package.exec)
            .arg("--extension")
            .arg(&entry.point)
            .env(ENV_POINT, &entry.point)
            .env(ENV_CONTROL, socket)
            .env("BIFROST_DATA", &entry.package.data_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .current_dir(&entry.package.data_dir)
            .spawn()?;
        Ok(Box::new(ChildInstance(child)))
    }
}

/// The request line (with its newline) for `method` with `params`.
fn request_line(method: &str, params: Json) -> String {
    let request = Json::Obj(vec![
        ("jsonrpc".into(), Json::Str("2.0".into())),
        ("id".into(), Json::Num(1.0)),
        ("method".into(), Json::Str(method.into())),
        ("params".into(), params),
    ]);
    format!("{}\n", request.encode())
}

/// The `result` of a reply, or the message of its `error`.
fn parse_result(reply: &str) -> Result<Json, HostError> {
    let v = json::parse(reply.trim()).map_err(|e| HostError::Unreachable(format!("malformed reply: {e}")))?;
    if let Some(e) = v.get("error") {
        return Err(HostError::Remote(e.str_field("message").unwrap_or_else(|| "error".into())));
    }
    v.get("result").cloned().ok_or_else(|| HostError::Remote("the reply has no result".into()))
}

struct Running {
    key: Key,
    id: ProcessId,
    instance: Box<dyn Instance>,
    socket: PathBuf,
    last_used: Instant,
}

/// A started extension and a connection to it, for one call. Give it back with [`Host::end`].
pub struct Lease {
    process: ProcessId,
    conn: ConnectionId,
    socket: PathBuf,
    transport: Arc<dyn Transport>,
}

impl Lease {
    /// Calls `method` and waits up to `reply_wait` for the answer. Blocking: run it off the UI thread.
    pub fn exchange(&self, method: &str, params: Json, reply_wait: Duration) -> Result<Json, HostError> {
        let reply = self.transport.exchange(&self.socket, &request_line(method, params), Duration::from_secs(2), reply_wait).map_err(HostError::Unreachable)?;
        parse_result(&reply)
    }
}

/// The installed extensions and the processes started for them.
pub struct Host {
    scan: Scan,
    packages: Vec<Package>,
    /// every installed extension, enabled or not (the registry only lists the enabled ones)
    known: Vec<Identity>,
    table: ProcessTable,
    running: Vec<Running>,
    launcher: Box<dyn Launcher>,
    transport: Arc<dyn Transport>,
    sockets: PathBuf,
    idle: Duration,
    start_wait: Duration,
}

/// The packages of installed apps, with the manifest text of each read from disk.
fn read_installed(apps: &[Installed], root: &Path) -> (Scan, Vec<Package>) {
    let texts: Vec<(String, String)> = apps.iter().filter_map(|a| std::fs::read_to_string(a.dir.join("manifest.toml")).ok().map(|t| (a.manifest.id.clone(), t))).collect();
    let scan = extension_manifest::scan(texts.iter().map(|(id, text)| (id.as_str(), text.as_str())));
    let packages = apps.iter().map(|a| Package { app_id: a.manifest.id.clone(), exec: a.exec_path(), data_dir: root.join("appdata").join(&a.manifest.id) }).collect();
    (scan, packages)
}

/// Where extension sockets live: under the runtime directory, named for the host process so two hosts (the shell and
/// the Files app, say) never share a socket.
fn socket_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("bifrost/ext"),
        None => std::env::temp_dir().join(format!("bifrost-ext-{}", std::process::id())),
    }
}

impl Host {
    /// A host over an already built `scan` and the `packages` the extensions run from.
    pub fn new(scan: Scan, packages: Vec<Package>, launcher: Box<dyn Launcher>, transport: Arc<dyn Transport>, sockets: PathBuf) -> Host {
        let points: Vec<&str> = scan.owners.iter().map(|o| o.point.as_str()).collect();
        let known = scan.registry.matching(&points).into_iter().cloned().collect();
        Host { scan, packages, known, table: ProcessTable::new(), running: Vec::new(), launcher, transport, sockets, idle: IDLE, start_wait: START_WAIT }
    }

    /// A host over the installed apps (`apps`, with their data under `root/appdata`), starting real processes.
    pub fn from_apps(apps: &[Installed], root: &Path) -> Host {
        let (scan, packages) = read_installed(apps, root);
        Host::new(scan, packages, Box::new(ProcessLauncher), Arc::new(Sockets), socket_dir())
    }

    /// A host over the packages installed now. The shell reads its installed snapshot; another process (Files) has none
    /// and scans the apps directory. The extensions the user switched off in Settings are left out.
    pub fn installed() -> Host {
        let root = super::apphost::root();
        let snapshot = super::apphost::installed();
        let scanned;
        let apps: &[Installed] = if snapshot.is_empty() {
            scanned = super::apphost::scan(&root);
            &scanned
        } else {
            &snapshot
        };
        let mut host = Host::from_apps(apps, &root);
        let prefs = super::prefs::Prefs::load(root.join("settings.ini"));
        host.apply_off(&login_items_page::background_off_from_pref(prefs.get(login_items_page::EXTENSIONS_OFF_KEY)));
        host
    }

    /// How long an extension may idle before [`Host::reap`] stops it.
    pub fn set_idle(&mut self, idle: Duration) {
        self.idle = idle;
    }

    /// How long a launch waits for the first ping.
    pub fn set_start_wait(&mut self, wait: Duration) {
        self.start_wait = wait;
    }

    /// Switches extensions off and on to match the Login Items keys in `off` (`login.extensions.off`). Only the kinds
    /// Settings lists can be switched off; others stay on.
    pub fn apply_off(&mut self, off: &[String]) {
        for identity in self.known.clone() {
            let Some(owner) = self.scan.owner(&identity.bundle_id, &identity.point).map(str::to_string) else { continue };
            let Some(key) = off_key(&identity, &owner) else { continue };
            self.scan.registry.set_enabled(&identity.bundle_id, &identity.point, !off.contains(&key));
        }
    }

    /// The enabled extensions of `point`, by name.
    pub fn discover(&self, point: &str) -> Vec<Entry> {
        self.scan.registry.discover(point).into_iter().filter_map(|i| self.entry(i)).collect()
    }

    fn entry(&self, identity: &Identity) -> Option<Entry> {
        let owner = self.scan.owner(&identity.bundle_id, &identity.point)?;
        let package = self.packages.iter().find(|p| p.app_id == owner)?.clone();
        Some(Entry { bundle_id: identity.bundle_id.clone(), point: identity.point.clone(), name: identity.name.clone(), package })
    }

    /// True while a process runs for this extension.
    pub fn is_running(&self, bundle_id: &str, point: &str) -> bool {
        self.running.iter().any(|r| r.key.bundle_id == bundle_id && r.key.point == point)
    }

    /// How many extension processes this host has started and not stopped.
    pub fn live(&self) -> usize {
        self.table.live_processes()
    }

    /// Starts the extension if it is not running (or has died) and opens a connection for one call.
    pub fn begin(&mut self, bundle_id: &str, point: &str, now: Instant) -> Result<Lease, HostError> {
        let identity = self.scan.registry.discover(point).into_iter().find(|i| i.bundle_id == bundle_id).cloned().ok_or(HostError::NotAvailable)?;
        let entry = self.entry(&identity).ok_or(HostError::NotAvailable)?;
        let key = Key::new(bundle_id, point);
        if let Some(at) = self.running.iter().position(|r| r.key == key) {
            if self.running[at].instance.alive() {
                return self.lease(at, now);
            }
            self.drop_running(at);
        }
        let id = self.table.acquire(&key);
        let socket = self.sockets.join(format!("{}-{}.sock", std::process::id(), id.0));
        let mut instance = match self.launcher.launch(&entry, &socket) {
            Ok(i) => i,
            Err(e) => {
                let _ = self.table.exit(id);
                return Err(HostError::Launch(e.to_string()));
            }
        };
        if let Err(e) = self.transport.exchange(&socket, &request_line(PING, Json::Obj(Vec::new())), self.start_wait, self.start_wait) {
            instance.stop();
            let _ = self.table.exit(id);
            let _ = std::fs::remove_file(&socket);
            return Err(HostError::Unreachable(e));
        }
        let _ = self.table.running(id);
        self.running.push(Running { key, id, instance, socket, last_used: now });
        let at = self.running.len() - 1;
        self.lease(at, now)
    }

    fn lease(&mut self, at: usize, now: Instant) -> Result<Lease, HostError> {
        let r = &mut self.running[at];
        r.last_used = now;
        let conn = self.table.connect(r.id).map_err(|_| HostError::Unreachable("the process left the table".into()))?;
        Ok(Lease { process: r.id, conn, socket: r.socket.clone(), transport: Arc::clone(&self.transport) })
    }

    /// Ends the call: closes the connection and starts the idle time from `now`.
    pub fn end(&mut self, lease: Lease, now: Instant) {
        let _ = self.table.invalidate(lease.conn);
        if let Some(r) = self.running.iter_mut().find(|r| r.id == lease.process) {
            r.last_used = now;
        }
    }

    /// `begin`, one call, `end`, holding the host throughout. Blocking.
    pub fn call(&mut self, bundle_id: &str, point: &str, method: &str, params: Json, reply_wait: Duration, now: Instant) -> Result<Json, HostError> {
        let lease = self.begin(bundle_id, point, now)?;
        let result = lease.exchange(method, params, reply_wait);
        self.end(lease, now);
        result
    }

    /// Stops the extensions with no call in flight that have been idle for the idle time, and drops the ones that died.
    /// Returns the bundle identifiers stopped.
    pub fn reap(&mut self, now: Instant) -> Vec<String> {
        let mut stopped = Vec::new();
        let mut at = 0;
        while at < self.running.len() {
            let r = &mut self.running[at];
            let idle = self.table.connection_count(r.id) == 0 && now.saturating_duration_since(r.last_used) >= self.idle;
            if idle || !r.instance.alive() {
                stopped.push(r.key.bundle_id.clone());
                self.stop_running(at);
            } else {
                at += 1;
            }
        }
        stopped
    }

    /// Stops every extension this host started.
    pub fn stop_all(&mut self) {
        while !self.running.is_empty() {
            self.stop_running(self.running.len() - 1);
        }
    }

    /// Asks the extension to leave, then stops the process and forgets it.
    fn stop_running(&mut self, at: usize) {
        let _ = self.transport.exchange(&self.running[at].socket, &request_line(SHUTDOWN, Json::Obj(Vec::new())), Duration::ZERO, Duration::from_millis(300));
        self.drop_running(at);
    }

    fn drop_running(&mut self, at: usize) {
        let mut r = self.running.remove(at);
        r.instance.stop();
        let _ = self.table.exit(r.id);
        let _ = std::fs::remove_file(&r.socket);
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop_all();
    }
}

/// The Login Items key of an extension, when Settings lists its kind. The manifest reader names an extension without a
/// `name` after its bundle identifier; Settings labels it by kind, so the key uses the empty name there.
fn off_key(identity: &Identity, app_id: &str) -> Option<String> {
    let kind = login_items_page::kind_for_point(&identity.point)?;
    let named = if identity.name == identity.bundle_id { "" } else { identity.name.as_str() };
    let label = if named.is_empty() { kind.label().to_string() } else { named.to_string() };
    Some(login_items_page::extension_key(&login_items_page::Extension { app_id: app_id.to_string(), app_name: String::new(), kind, name: label }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::extension_manifest::FILE_PROVIDER_POINT;
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread::JoinHandle;

    const DRIVE_ID: &str = "com.example.drive";
    const DRIVE: &str = "[[extension]]\npoint = \"is.bifrost.file-provider\"\nname = \"Example Drive\"\n";

    /// An extension in a thread: a real socket server that answers ping, echo, fail and shutdown.
    struct Fake {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Instance for Fake {
        fn alive(&mut self) -> bool {
            !self.stop.load(Ordering::SeqCst)
        }
        fn stop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    #[derive(Clone, Default)]
    struct FakeLauncher {
        launches: Arc<AtomicUsize>,
        /// the last instance's stop flag, so a test can make it "crash"
        last_stop: Arc<std::sync::Mutex<Option<Arc<AtomicBool>>>>,
        fail: bool,
        silent: bool,
    }

    impl Launcher for FakeLauncher {
        fn launch(&self, _entry: &Entry, socket: &Path) -> io::Result<Box<dyn Instance>> {
            if self.fail {
                return Err(io::Error::other("no such binary"));
            }
            self.launches.fetch_add(1, Ordering::SeqCst);
            let stop = Arc::new(AtomicBool::new(false));
            *self.last_stop.lock().unwrap() = Some(Arc::clone(&stop));
            if self.silent {
                return Ok(Box::new(Fake { stop, thread: None }));
            }
            std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
            let _ = std::fs::remove_file(socket);
            let listener = UnixListener::bind(socket).unwrap();
            listener.set_nonblocking(true).unwrap();
            let (path, flag) = (socket.to_path_buf(), Arc::clone(&stop));
            let thread = std::thread::spawn(move || {
                while !flag.load(Ordering::SeqCst) {
                    let Ok((mut conn, _)) = listener.accept() else {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    conn.set_nonblocking(false).unwrap();
                    let mut text = String::new();
                    conn.read_to_string(&mut text).unwrap();
                    let req = json::parse(text.trim()).unwrap();
                    let params = req.get("params").cloned().unwrap_or(Json::Null);
                    let reply = match req.get("method").and_then(Json::as_str) {
                        Some("extension.shutdown") => {
                            flag.store(true, Ordering::SeqCst);
                            Json::Obj(vec![("result".into(), Json::Bool(true))])
                        }
                        Some("fail") => Json::Obj(vec![("error".into(), Json::Obj(vec![("message".into(), Json::Str("boom".into()))]))]),
                        Some("echo") => Json::Obj(vec![("result".into(), params)]),
                        _ => Json::Obj(vec![("result".into(), Json::Bool(true))]),
                    };
                    let _ = conn.write_all(reply.encode().as_bytes());
                }
                let _ = std::fs::remove_file(path);
            });
            Ok(Box::new(Fake { stop, thread: Some(thread) }))
        }
    }

    fn host_with(launcher: FakeLauncher, tag: &str) -> Host {
        let scan = extension_manifest::scan([(DRIVE_ID, DRIVE)]);
        let packages = vec![Package { app_id: DRIVE_ID.into(), exec: "/bin/true".into(), data_dir: PathBuf::from("/tmp") }];
        let sockets = std::env::temp_dir().join(format!("ext-host-{tag}-{}", std::process::id()));
        let mut host = Host::new(scan, packages, Box::new(launcher), Arc::new(Sockets), sockets);
        host.set_start_wait(Duration::from_millis(300));
        host
    }

    fn echo(host: &mut Host, now: Instant) -> Result<Json, HostError> {
        host.call(DRIVE_ID, FILE_PROVIDER_POINT, "echo", Json::Str("hi".into()), Duration::from_secs(2), now)
    }

    #[test]
    fn manifests_are_read_without_starting_anything() {
        let launcher = FakeLauncher::default();
        let host = host_with(launcher.clone(), "scan");
        let found = host.discover(FILE_PROVIDER_POINT);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].bundle_id.as_str(), found[0].name.as_str()), (DRIVE_ID, "Example Drive"));
        assert_eq!(found[0].package.exec, PathBuf::from("/bin/true"));
        assert_eq!(launcher.launches.load(Ordering::SeqCst), 0);
        assert_eq!(host.live(), 0);
    }

    #[test]
    fn installed_packages_are_read_from_their_manifest_files() {
        use super::super::apphost::Manifest;
        let root = std::env::temp_dir().join(format!("ext-host-root-{}", std::process::id()));
        let dir = root.join("apps/com.example.drive/current");
        std::fs::create_dir_all(&dir).unwrap();
        let text = format!("id = \"{DRIVE_ID}\"\nname = \"Drive\"\nexec = \"drive\"\n\n{DRIVE}");
        std::fs::write(dir.join("manifest.toml"), &text).unwrap();
        let apps = [Installed { manifest: Manifest::parse(&text).unwrap(), dir: dir.clone() }];
        let host = Host::from_apps(&apps, &root);
        let found = host.discover(FILE_PROVIDER_POINT);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].package.exec, dir.join("drive"));
        assert_eq!(found[0].package.data_dir, root.join("appdata").join(DRIVE_ID));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_first_call_launches_the_extension_and_later_calls_reuse_it() {
        let launcher = FakeLauncher::default();
        let mut host = host_with(launcher.clone(), "reuse");
        let now = Instant::now();
        assert!(!host.is_running(DRIVE_ID, FILE_PROVIDER_POINT));
        assert_eq!(echo(&mut host, now), Ok(Json::Str("hi".into())));
        assert_eq!(echo(&mut host, now), Ok(Json::Str("hi".into())));
        assert_eq!(launcher.launches.load(Ordering::SeqCst), 1);
        assert!(host.is_running(DRIVE_ID, FILE_PROVIDER_POINT));
        assert_eq!(host.live(), 1);
        host.stop_all();
        assert_eq!(host.live(), 0);
    }

    #[test]
    fn an_idle_extension_is_stopped_and_the_next_call_starts_it_again() {
        let launcher = FakeLauncher::default();
        let mut host = host_with(launcher.clone(), "idle");
        host.set_idle(Duration::from_secs(30));
        let t0 = Instant::now();
        echo(&mut host, t0).unwrap();
        assert!(host.reap(t0 + Duration::from_secs(29)).is_empty(), "not idle long enough");
        assert!(host.is_running(DRIVE_ID, FILE_PROVIDER_POINT));
        assert_eq!(host.reap(t0 + Duration::from_secs(31)), vec![DRIVE_ID.to_string()]);
        assert!(!host.is_running(DRIVE_ID, FILE_PROVIDER_POINT));
        assert_eq!(host.live(), 0);
        echo(&mut host, t0 + Duration::from_secs(40)).unwrap();
        assert_eq!(launcher.launches.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn an_extension_with_a_call_in_flight_is_not_reaped() {
        let mut host = host_with(FakeLauncher::default(), "busy");
        let t0 = Instant::now();
        let lease = host.begin(DRIVE_ID, FILE_PROVIDER_POINT, t0).unwrap();
        assert!(host.reap(t0 + Duration::from_secs(600)).is_empty());
        assert_eq!(lease.exchange("echo", Json::Num(7.0), Duration::from_secs(2)), Ok(Json::Num(7.0)));
        host.end(lease, t0 + Duration::from_secs(600));
        assert!(host.reap(t0 + Duration::from_secs(601)).is_empty(), "idle time starts when the call ends");
        assert_eq!(host.reap(t0 + Duration::from_secs(700)).len(), 1);
    }

    #[test]
    fn a_process_that_died_is_started_again_by_the_next_call() {
        let launcher = FakeLauncher::default();
        let mut host = host_with(launcher.clone(), "died");
        let now = Instant::now();
        echo(&mut host, now).unwrap();
        launcher.last_stop.lock().unwrap().as_ref().unwrap().store(true, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(echo(&mut host, now), Ok(Json::Str("hi".into())));
        assert_eq!(launcher.launches.load(Ordering::SeqCst), 2);
        assert_eq!(host.live(), 1);
    }

    #[test]
    fn an_error_reply_is_a_remote_error() {
        let mut host = host_with(FakeLauncher::default(), "remote");
        let r = host.call(DRIVE_ID, FILE_PROVIDER_POINT, "fail", Json::Null, Duration::from_secs(2), Instant::now());
        assert_eq!(r, Err(HostError::Remote("boom".into())));
    }

    #[test]
    fn an_unknown_or_switched_off_extension_is_not_available() {
        let mut host = host_with(FakeLauncher::default(), "off");
        let now = Instant::now();
        assert_eq!(host.begin("com.nobody.x", FILE_PROVIDER_POINT, now).err(), Some(HostError::NotAvailable));
        assert_eq!(host.begin(DRIVE_ID, "com.example.other", now).err(), Some(HostError::NotAvailable));

        let key = format!("{DRIVE_ID}/file-provider/Example Drive");
        host.apply_off(&[key]);
        assert!(host.discover(FILE_PROVIDER_POINT).is_empty());
        assert_eq!(host.begin(DRIVE_ID, FILE_PROVIDER_POINT, now).err(), Some(HostError::NotAvailable));
        host.apply_off(&[]);
        assert_eq!(host.discover(FILE_PROVIDER_POINT).len(), 1, "switching it back on offers it again");
    }

    #[test]
    fn a_launch_failure_and_a_silent_process_leave_nothing_behind() {
        let mut host = host_with(FakeLauncher { fail: true, ..Default::default() }, "fail");
        let r = host.begin(DRIVE_ID, FILE_PROVIDER_POINT, Instant::now());
        assert!(matches!(r, Err(HostError::Launch(_))));
        assert_eq!(host.live(), 0);

        let launcher = FakeLauncher { silent: true, ..Default::default() };
        let mut host = host_with(launcher.clone(), "silent");
        let r = host.begin(DRIVE_ID, FILE_PROVIDER_POINT, Instant::now());
        assert!(matches!(r, Err(HostError::Unreachable(_))));
        assert_eq!(host.live(), 0);
        assert!(launcher.last_stop.lock().unwrap().as_ref().unwrap().load(Ordering::SeqCst), "the silent process was stopped");
    }

    #[test]
    fn dropping_the_host_stops_what_it_started() {
        let launcher = FakeLauncher::default();
        let mut host = host_with(launcher.clone(), "drop");
        echo(&mut host, Instant::now()).unwrap();
        let stop = launcher.last_stop.lock().unwrap().clone().unwrap();
        drop(host);
        assert!(stop.load(Ordering::SeqCst));
    }
}
