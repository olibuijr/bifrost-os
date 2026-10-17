//! File provider domains in Files. An installed app that declares a file provider extension
//! (`[[extension]] point = "is.bifrost.file-provider"`) shows up as a row under Locations; opening the row browses the
//! provider's items, and opening a file downloads it first.
//!
//! The extension is started and called by the shared extension host (`springboard::shell::extension_host`): the first
//! listing starts the package's binary in extension mode and the host stops it again after it has been idle.
//! Each provider item lives at a stand-in path, `/.provider/<domain>/<folder names>/<file name>`, so the path bar, Back
//! and Up work on names. [`Providers`] keeps the item behind every listed path (an item's identifier is what the
//! provider knows it by) and talks to the extension with `bifrost_app::extension_service::wire`
//! (`provider.enumerate`, `provider.fetch`).
//!
//! A file is a placeholder until it is opened: the listing has its name, size and date, and `provider.fetch` writes its
//! bytes into the cache (`~/.cache/bifrost/providers/<domain>/<item>/<name>`) when the user opens it. A file already
//! fetched is opened from the cache while it is not older than the provider's modification date.
//!
//! Gaps: a provider folder is read-only in Files (no create, rename, trash or paste, no drag out), the listing does not
//! refresh when the provider signals a change, placeholders have no cloud mark in the views, and the domain list is read
//! once, so a newly installed provider appears after Files restarts.
use crate::*;
use bifrost_app::extension::FILE_PROVIDER_POINT;
use bifrost_app::extension_service::wire;
use bifrost_app::file_provider::{Domain, DomainRegistry, ItemCapabilities, ItemIdentifier, ProvidedItem};
use bifrost_core::json::Json;
use springboard::shell::extension_host::{Host, HostError};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The stand-in folder every provider domain lives under.
pub(crate) const ROOT: &str = "/.provider";
/// How long a listing waits for its extension.
const LIST_WAIT: Duration = Duration::from_secs(15);
/// How long a download waits: a large file takes a while.
const FETCH_WAIT: Duration = Duration::from_secs(120);
/// How often idle extensions are looked for.
const REAP_EVERY: Duration = Duration::from_secs(5);

/// True inside a provider domain (the domain's root and everything under it).
pub(crate) fn is_path(path: &Path) -> bool {
    domain_of(path).is_some()
}

/// The stand-in folder of one domain.
pub(crate) fn root_path(domain: &str) -> PathBuf {
    Path::new(ROOT).join(domain)
}

/// The identifier of the domain a stand-in path is inside.
fn domain_of(path: &Path) -> Option<String> {
    path.strip_prefix(ROOT).ok()?.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned())
}

/// `s` with every byte but letters, digits, `-` and `_` written as `%XX`, so it is one safe folder name.
fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// What the person reads when the host or the provider fails.
fn explain(e: &HostError) -> String {
    match e {
        HostError::Remote(code) => match code.as_str() {
            "noSuchItem" => "That item no longer exists".into(),
            "notAuthenticated" => "Sign in to use this location".into(),
            "serverUnreachable" | "cannotSynchronize" | "providerDomainTemporarilyUnavailable" => "This location can't be reached right now".into(),
            "insufficientQuota" => "There is not enough storage left in this location".into(),
            other => other.to_string(),
        },
        other => other.to_string(),
    }
}

fn entry_of(path: PathBuf, item: &ProvidedItem) -> Entry {
    let is_dir = wire::is_folder(item);
    Entry {
        name: item.filename().to_string(),
        path,
        is_dir,
        size: item.document_size.unwrap_or(0),
        mtime: item.content_modification_date.map_or(0, |m| m as i64),
        kind: fs::kind_of(item.filename(), is_dir),
        items: None,
    }
}

/// The provider domains and the items listed from them.
pub(crate) struct Providers {
    host: Arc<Mutex<Host>>,
    domains: DomainRegistry,
    /// the item behind each listed stand-in path
    items: Mutex<HashMap<PathBuf, ProvidedItem>>,
    cache: PathBuf,
}

fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Providers {
    /// The domains are the enabled file provider extensions of `host`; downloads go under `cache`.
    pub(crate) fn new(host: Host, cache: PathBuf) -> Providers {
        let mut domains = DomainRegistry::default();
        for e in host.discover(FILE_PROVIDER_POINT) {
            if let Ok(domain) = Domain::new(&e.bundle_id, &e.name) {
                let _ = domains.add(domain);
            }
        }
        Providers { host: Arc::new(Mutex::new(host)), domains, items: Mutex::new(HashMap::new()), cache }
    }

    /// The sidebar rows: each visible, enabled domain with the stand-in folder it opens.
    pub(crate) fn rows(&self) -> Vec<(String, PathBuf)> {
        self.domains.visible().filter(|d| d.user_enabled).map(|d| (d.display_name.clone(), root_path(&d.identifier))).collect()
    }

    fn display_name(&self, domain: &str) -> String {
        self.domains.get(domain).map_or_else(|| domain.to_string(), |d| d.display_name.clone())
    }

    /// The window title: the domain's name at its root, else the folder's name.
    pub(crate) fn title(&self, path: &Path) -> String {
        match (domain_of(path), path.file_name()) {
            (Some(domain), _) if path == root_path(&domain) => self.display_name(&domain),
            (_, Some(name)) => name.to_string_lossy().into_owned(),
            _ => "Locations".to_string(),
        }
    }

    /// The path bar: the domain, then each folder down to `path`.
    pub(crate) fn path_items(&self, path: &Path) -> Vec<(String, &'static str, PathBuf)> {
        let Some(domain) = domain_of(path) else { return Vec::new() };
        let root = root_path(&domain);
        let mut at = root.clone();
        let mut out = vec![(self.display_name(&domain), "externaldrive", root.clone())];
        for c in path.strip_prefix(&root).into_iter().flat_map(|rest| rest.components()) {
            at = at.join(c);
            out.push((c.as_os_str().to_string_lossy().into_owned(), "folder", at.clone()));
        }
        out
    }

    /// One call to the extension of `domain`. Blocking: run it off the UI thread.
    fn call(&self, domain: &str, method: &str, params: Json, wait: Duration) -> Result<Json, String> {
        let lease = locked(&self.host).begin(domain, FILE_PROVIDER_POINT, Instant::now()).map_err(|e| explain(&e))?;
        let result = lease.exchange(method, params, wait);
        locked(&self.host).end(lease, Instant::now());
        result.map_err(|e| explain(&e))
    }

    /// The identifier of the folder at `path`: the domain's root container, or a folder listed earlier.
    fn container(&self, domain: &str, path: &Path) -> Result<ItemIdentifier, String> {
        if path == root_path(domain) {
            return Ok(ItemIdentifier::root_container());
        }
        let items = locked(&self.items);
        match items.get(path) {
            Some(item) if wire::is_folder(item) => Ok(item.identifier.clone()),
            _ => Err("This folder is no longer available".to_string()),
        }
    }

    /// The rows of the folder at `path`, from the provider. Blocking: run it off the UI thread.
    pub(crate) fn list(&self, path: &Path) -> Result<Vec<Entry>, String> {
        let domain = domain_of(path).ok_or_else(|| "This location is not available".to_string())?;
        let container = self.container(&domain, path)?;
        let reply = self.call(&domain, wire::ENUMERATE, wire::enumerate_params(&container), LIST_WAIT)?;
        let items = wire::items_from(&reply).ok_or_else(|| "The listing from this location could not be read".to_string())?;
        let mut known = locked(&self.items);
        known.retain(|p, _| p.parent() != Some(path));
        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            let child = path.join(item.filename());
            rows.push(entry_of(child.clone(), &item));
            known.insert(child, item);
        }
        Ok(rows)
    }

    /// The local file for the item at `path`, fetched from the provider when it is not in the cache or is older than
    /// the provider's copy. Blocking: run it off the UI thread.
    pub(crate) fn fetch(&self, path: &Path) -> Result<PathBuf, String> {
        let domain = domain_of(path).ok_or_else(|| "This location is not available".to_string())?;
        let item = locked(&self.items).get(path).cloned().ok_or_else(|| "This item is no longer available".to_string())?;
        if wire::is_folder(&item) {
            return Err("This is a folder".to_string());
        }
        if !item.can(ItemCapabilities::READING) {
            return Err(format!("\u{201C}{}\u{201D} can't be opened from this location", item.filename()));
        }
        let dest = self.cache.join(encode(&domain)).join(encode(item.identifier.as_str())).join(item.filename());
        if let Some(changed) = item.content_modification_date {
            let have = std::fs::metadata(&dest).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs());
            if have.is_some_and(|have| have >= changed) {
                return Ok(dest);
            }
        }
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("The download folder can't be made: {e}"))?;
        }
        let dest_text = dest.to_str().ok_or_else(|| "The download path is not text".to_string())?;
        self.call(&domain, wire::FETCH, wire::fetch_params(&item.identifier, dest_text), FETCH_WAIT)?;
        if !dest.is_file() {
            return Err("The provider did not deliver the file".to_string());
        }
        if let Some(it) = locked(&self.items).get_mut(path) {
            let _ = it.request_download();
            let _ = it.finish_download();
        }
        Ok(dest)
    }
}

/// The providers of this Files process. The first use reads the installed packages and starts a thread that stops idle
/// extensions.
pub(crate) fn shared() -> Arc<Providers> {
    static SHARED: OnceLock<Arc<Providers>> = OnceLock::new();
    Arc::clone(SHARED.get_or_init(|| {
        let cache = fs::home().join(".cache/bifrost/providers");
        let providers = Arc::new(Providers::new(Host::installed(), cache));
        let host = Arc::clone(&providers.host);
        std::thread::spawn(move || loop {
            std::thread::sleep(REAP_EVERY);
            locked(&host).reap(Instant::now());
        });
        providers
    }))
}

/// Messages from provider threads for the footer: text to show, or empty text to clear it.
static NOTICES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Queues a footer message from a worker thread and wakes the app loop.
pub(crate) fn notify(message: String) {
    locked(&NOTICES).push(message);
    DispatchQueue::main().async_(|| {});
}

/// The next queued footer message. `Some("")` clears the footer.
pub(crate) fn take_notice() -> Option<String> {
    let mut queue = locked(&NOTICES);
    (!queue.is_empty()).then(|| queue.remove(0))
}

impl Files {
    /// Opens an item of a provider: a folder is entered; a file is downloaded to the cache (a placeholder's bytes
    /// arrive now) and then opened with its app. The download runs on a worker thread; its failure shows in the footer.
    pub(crate) fn open_provider_item(&mut self, e: Entry) {
        if e.is_dir {
            self.navigate(e.path, true);
            return;
        }
        self.notice = Some(format!("Downloading \u{201C}{}\u{201D}\u{2026}", e.name));
        self.revision += 1;
        let providers = shared();
        DispatchQueue::global(Qos::Utility).async_(move || match providers.fetch(&e.path) {
            Ok(local) => {
                notify(String::new());
                let _ = std::process::Command::new("/usr/bin/bifrost-apps").arg("--open").arg(local).status();
            }
            Err(message) => notify(message),
        });
    }

    /// Takes the footer message a provider thread left. True when the footer changed.
    pub(crate) fn take_provider_notice(&mut self) -> bool {
        let Some(message) = take_notice() else { return false };
        self.notice = (!message.is_empty()).then_some(message);
        self.revision += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::extension::{scan_manifests, Scan};
    use bifrost_app::extension_service::{serve_at, FileProviderService, Provider};
    use bifrost_app::file_provider::{ErrorCode, ProviderError};
    use springboard::shell::extension_host::{Entry as HostEntry, Instance, Launcher, Package, Sockets};
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const DRIVE: &str = "com.example.drive";
    const MANIFEST: &str = "[[extension]]\npoint = \"is.bifrost.file-provider\"\nname = \"Example Drive\"\n";

    /// Root: a folder "Reports" (holds Q3.txt), a folder "Gone" the provider no longer knows, and a placeholder
    /// "Plan.txt" with a modification date; "Locked.txt" cannot be read.
    struct Memory {
        fetches: Arc<AtomicUsize>,
    }

    fn id(s: &str) -> ItemIdentifier {
        ItemIdentifier::new(s).unwrap()
    }

    fn doc(item: &str, parent: &ItemIdentifier, name: &str, bytes: u64) -> ProvidedItem {
        ProvidedItem::new(id(item), parent.clone(), name).unwrap().with_document_size(bytes).with_capabilities(ItemCapabilities::READING).as_placeholder()
    }

    impl Provider for Memory {
        fn enumerate(&mut self, container: &ItemIdentifier) -> Result<Vec<ProvidedItem>, ProviderError> {
            let root = ItemIdentifier::root_container();
            match container.as_str() {
                "root" => {
                    let mut plan = doc("d1", &root, "Plan.txt", 5);
                    plan.content_modification_date = Some(1);
                    let no_read = ProvidedItem::new(id("d9"), root.clone(), "Locked.txt").unwrap().as_placeholder();
                    Ok(vec![
                        wire::as_folder(ProvidedItem::new(id("f1"), root.clone(), "Reports").unwrap()),
                        wire::as_folder(ProvidedItem::new(id("f2"), root.clone(), "Gone").unwrap()),
                        plan,
                        no_read,
                    ])
                }
                "f1" => Ok(vec![doc("d2", &id("f1"), "Q3.txt", 7)]),
                other => Err(ProviderError::for_item(ErrorCode::NoSuchItem, id(other))),
            }
        }
        fn fetch(&mut self, item: &ItemIdentifier, dest: &Path) -> Result<(), ProviderError> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            let bytes: &[u8] = match item.as_str() {
                "d1" => b"hello",
                "d2" => b"quarter",
                other => return Err(ProviderError::for_item(ErrorCode::NoSuchItem, id(other))),
            };
            std::fs::write(dest, bytes).map_err(|_| ProviderError::new(ErrorCode::CannotSynchronize))
        }
    }

    /// Runs the real SDK service in a thread on the socket the host names.
    struct InProcess {
        socket: PathBuf,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Instance for InProcess {
        fn alive(&mut self) -> bool {
            self.thread.as_ref().is_some_and(|t| !t.is_finished())
        }
        fn stop(&mut self) {
            if let Ok(mut s) = UnixStream::connect(&self.socket) {
                let _ = s.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"extension.shutdown\",\"params\":{}}\n");
                let _ = s.shutdown(std::net::Shutdown::Write);
                let _ = std::io::Read::read_to_end(&mut s, &mut Vec::new());
            }
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    struct Launches(Arc<AtomicUsize>, Arc<AtomicUsize>);

    impl Launcher for Launches {
        fn launch(&self, _entry: &HostEntry, socket: &Path) -> std::io::Result<Box<dyn Instance>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            let (path, fetches) = (socket.to_path_buf(), Arc::clone(&self.1));
            let thread = std::thread::spawn(move || {
                let _ = serve_at(&path, &mut FileProviderService(Memory { fetches }), false);
            });
            Ok(Box::new(InProcess { socket: socket.to_path_buf(), thread: Some(thread) }))
        }
    }

    struct Fixture {
        providers: Providers,
        launches: Arc<AtomicUsize>,
        fetches: Arc<AtomicUsize>,
        dir: PathBuf,
    }

    fn fixture(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("files-provider-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (launches, fetches) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let scan: Scan = scan_manifests([(DRIVE, MANIFEST)]);
        let packages = vec![Package { app_id: DRIVE.into(), exec: "/bin/true".into(), data_dir: dir.join("data") }];
        let mut host = Host::new(scan, packages, Box::new(Launches(Arc::clone(&launches), Arc::clone(&fetches))), Arc::new(Sockets), dir.join("sock"));
        host.set_start_wait(Duration::from_secs(3));
        Fixture { providers: Providers::new(host, dir.join("cache")), launches, fetches, dir }
    }

    fn at(rest: &str) -> PathBuf {
        root_path(DRIVE).join(rest)
    }

    #[test]
    fn the_installed_provider_is_a_sidebar_row_and_nothing_runs_until_it_is_opened() {
        let f = fixture("rows");
        assert_eq!(f.providers.rows(), vec![("Example Drive".to_string(), root_path(DRIVE))]);
        assert_eq!(f.launches.load(Ordering::SeqCst), 0);
        assert!(is_path(&root_path(DRIVE)) && is_path(&at("Reports/Q3.txt")));
        assert!(!is_path(Path::new(ROOT)) && !is_path(Path::new("/home/me")));
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn the_root_and_a_folder_list_through_the_extension() {
        let f = fixture("list");
        let rows = f.providers.list(&root_path(DRIVE)).unwrap();
        let names: Vec<&str> = rows.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Reports", "Gone", "Plan.txt", "Locked.txt"]);
        assert!(rows[0].is_dir && rows[0].kind == Kind::Folder);
        assert_eq!((rows[2].is_dir, rows[2].size, rows[2].path.clone()), (false, 5, at("Plan.txt")));

        let inner = f.providers.list(&at("Reports")).unwrap();
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0].path, at("Reports/Q3.txt"));
        assert_eq!(f.launches.load(Ordering::SeqCst), 1, "one process serves both listings");
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn a_folder_that_was_never_listed_or_that_the_provider_lost_gives_a_message() {
        let f = fixture("errors");
        assert_eq!(f.providers.list(&at("Reports")), Err("This folder is no longer available".to_string()));
        f.providers.list(&root_path(DRIVE)).unwrap();
        assert_eq!(f.providers.list(&at("Gone")), Err("That item no longer exists".to_string()));
        assert_eq!(f.providers.list(&at("Plan.txt")), Err("This folder is no longer available".to_string()), "a file is not a folder");
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn opening_a_placeholder_downloads_it_once_while_the_cache_is_fresh() {
        let f = fixture("fetch");
        f.providers.list(&root_path(DRIVE)).unwrap();
        let local = f.providers.fetch(&at("Plan.txt")).unwrap();
        assert!(local.starts_with(f.dir.join("cache")));
        assert_eq!(local.file_name().unwrap(), "Plan.txt");
        assert_eq!(std::fs::read(&local).unwrap(), b"hello");
        assert_eq!(f.fetches.load(Ordering::SeqCst), 1);
        assert_eq!(f.providers.fetch(&at("Plan.txt")).unwrap(), local);
        assert_eq!(f.fetches.load(Ordering::SeqCst), 1, "the cached copy is not older than the provider's");

        f.providers.list(&at("Reports")).unwrap();
        let q3 = f.providers.fetch(&at("Reports/Q3.txt")).unwrap();
        assert_eq!(std::fs::read(q3).unwrap(), b"quarter");
        assert_eq!(f.fetches.load(Ordering::SeqCst), 2, "an item with no date is always fetched");
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn items_that_cannot_be_read_or_are_unknown_are_refused_without_a_download() {
        let f = fixture("refuse");
        f.providers.list(&root_path(DRIVE)).unwrap();
        let message = f.providers.fetch(&at("Locked.txt")).unwrap_err();
        assert!(message.contains("Locked.txt") && message.contains("can't be opened"), "{message}");
        assert_eq!(f.providers.fetch(&at("Reports")), Err("This is a folder".to_string()));
        assert_eq!(f.providers.fetch(&at("Missing.txt")), Err("This item is no longer available".to_string()));
        assert_eq!(f.fetches.load(Ordering::SeqCst), 0);
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn titles_and_the_path_bar_use_the_names() {
        let f = fixture("names");
        assert_eq!(f.providers.title(&root_path(DRIVE)), "Example Drive");
        assert_eq!(f.providers.title(&at("Reports")), "Reports");
        let items = f.providers.path_items(&at("Reports/Q3"));
        let labels: Vec<&str> = items.iter().map(|(l, _, _)| l.as_str()).collect();
        assert_eq!(labels, ["Example Drive", "Reports", "Q3"]);
        assert_eq!(items[1].2, at("Reports"));
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn item_identifiers_make_safe_folder_names() {
        assert_eq!(encode("doc-7_a"), "doc-7_a");
        assert_eq!(encode("../etc/x"), "%2E%2E%2Fetc%2Fx");
    }

    #[test]
    fn provider_threads_leave_footer_messages_for_the_loop() {
        // drain anything another test queued first
        while take_notice().is_some() {}
        notify("Sign in to use this location".to_string());
        notify(String::new());
        assert_eq!(take_notice().as_deref(), Some("Sign in to use this location"));
        assert_eq!(take_notice().as_deref(), Some(""));
        assert_eq!(take_notice(), None);
    }
}
