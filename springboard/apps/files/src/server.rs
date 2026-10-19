//! Go > Connect to Server: an address sheet with the recent servers, and a browser sheet that lists a remote folder.
//! An ftp:// listing runs on its own thread over a control connection and a passive data connection (RFC 959); the
//! model in `bifrost_app::ftp` parses the URL, the replies and the listing, and this file only moves the bytes. An
//! smb:// address lists the server's shares, then a share's folders, through `bifrost_app::smb` (Samba's client does the
//! protocol). A server that wants a login gets a sheet with a name and a masked password field; the login is kept in
//! memory for that browser only and is never written to the recent-servers file or put in the address. Remote files
//! cannot be opened or copied yet; the browser lists folders and moves between them.
use crate::*;
use bifrost_app::ftp::{self, Command, FtpUrl};
use bifrost_app::smb::{self, Credentials, SmbUrl};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

/// How long a connect, a reply or a data transfer may wait before the sheet reports it.
const TIMEOUT: Duration = Duration::from_secs(15);
/// Recent servers kept, newest first.
const RECENT_MAX: usize = 8;
/// The user and password sent when an address names none (the anonymous login of RFC 1635).
const ANONYMOUS: &str = "anonymous";
const ANONYMOUS_PASS: &str = "anonymous@";

/// The address sheet: the field, the recent server the arrows picked, and the last message.
pub(crate) struct AddressSheet {
    pub(crate) field: TextField,
    /// The index of the recent server the arrows picked; `None` while the field has the focus.
    pub(crate) picked: Option<usize>,
    pub(crate) message: Option<String>,
}

/// One row of the remote browser, whichever protocol listed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) name: String,
    pub(crate) folder: bool,
    /// The grey text at the row's right: "Folder", a size, a link target.
    pub(crate) detail: String,
}

/// Where the browser is: a folder of an ftp:// server, or an smb:// server, share or folder with the login it was
/// opened with (none for a guest).
#[derive(Clone)]
pub(crate) enum Remote {
    Ftp(FtpUrl),
    Smb(SmbUrl, Credentials),
}

impl Remote {
    /// The heading of the browser.
    pub(crate) fn title(&self) -> String {
        match self {
            Remote::Ftp(u) => format!("{}:{}{}", u.host, u.port, u.path),
            Remote::Smb(u, _) => u.to_text(),
        }
    }

    /// The folder `name` inside this one. On an SMB server's share list the name is a share.
    pub(crate) fn child(&self, name: &str) -> Remote {
        match self {
            Remote::Ftp(u) => Remote::Ftp(FtpUrl { path: remote_join(&u.path, name), ..u.clone() }),
            Remote::Smb(u, c) => {
                let mut url = u.clone();
                if url.share.is_none() {
                    url.share = Some(name.to_string());
                    url.path.clear();
                } else if url.path.is_empty() {
                    url.path = name.to_string();
                } else {
                    url.path = format!("{}/{name}", url.path);
                }
                Remote::Smb(url, c.clone())
            }
        }
    }

    /// The folder above this one; a share's top goes up to the server's share list, and that stays.
    pub(crate) fn parent(&self) -> Remote {
        match self {
            Remote::Ftp(u) => Remote::Ftp(FtpUrl { path: remote_parent(&u.path), ..u.clone() }),
            Remote::Smb(u, c) => {
                let mut url = u.clone();
                match url.path.rfind('/') {
                    Some(i) => url.path.truncate(i),
                    None if !url.path.is_empty() => url.path.clear(),
                    None => url.share = None,
                }
                Remote::Smb(url, c.clone())
            }
        }
    }
}

/// A remote folder shown by the browser sheet.
pub(crate) struct Browse {
    pub(crate) remote: Remote,
    pub(crate) entries: Vec<Row>,
    pub(crate) selected: usize,
    pub(crate) message: Option<String>,
}

/// Why a listing failed, and whether a login would help.
struct Failure {
    text: String,
    needs_login: bool,
}

impl Failure {
    fn plain(text: String) -> Failure {
        Failure { text, needs_login: false }
    }
}

/// A listing running on its own thread: the address the user typed (empty for a folder opened inside the browser),
/// where it lists, and the channel its answer comes back on.
struct Job {
    addr: String,
    remote: Remote,
    rx: Receiver<Result<Vec<Row>, Failure>>,
}

/// The sheet that asks an SMB server's login: a name and a masked password. Tab moves between the fields, Return
/// connects and Escape cancels.
pub(crate) struct LoginSheet {
    /// The address as the user typed it, for the recent-servers list.
    addr: String,
    url: SmbUrl,
    pub(crate) user: TextField,
    pub(crate) password: TextField,
    pub(crate) on_password: bool,
    pub(crate) message: Option<String>,
}

/// What a key on the login sheet asks for.
enum LoginAct {
    Connect,
    Close,
    Nothing,
}

impl LoginSheet {
    fn new(addr: String, url: SmbUrl, message: Option<String>) -> LoginSheet {
        let mut user = TextField::new();
        if let Some(u) = &url.user {
            user.push_str(u);
        }
        // a name from the address leaves only the password to type
        let on_password = url.user.is_some();
        LoginSheet { addr, url, user, password: TextField::new(), on_password, message }
    }

    fn field(&mut self) -> &mut TextField {
        if self.on_password {
            &mut self.password
        } else {
            &mut self.user
        }
    }

    /// The bullets the password field shows instead of the password.
    pub(crate) fn masked(&self) -> TextField {
        let mut shown = TextField::new();
        shown.push_str(&"\u{2022}".repeat(self.password.chars().count()));
        shown
    }

    fn key(&mut self, input: &Input) -> LoginAct {
        match input {
            Input::Key(KeyInput::Text(s)) => {
                self.field().push_str(s);
                LoginAct::Nothing
            }
            Input::Key(KeyInput::Code(code)) => match *code {
                13 if self.user.is_empty() => {
                    self.message = Some("Type a name, or press Escape to cancel.".to_string());
                    self.on_password = false;
                    LoginAct::Nothing
                }
                13 => LoginAct::Connect,
                KEY_ESCAPE | KEY_ESCAPE_ASCII => LoginAct::Close,
                9 => {
                    self.on_password = !self.on_password;
                    LoginAct::Nothing
                }
                8 => {
                    self.field().pop();
                    LoginAct::Nothing
                }
                _ => LoginAct::Nothing,
            },
            _ => LoginAct::Nothing,
        }
    }

    /// Where to list now: the address with the typed name, and the login. An empty password is sent as none, so the
    /// client never stops to ask for one on a terminal it does not have.
    fn remote(&self) -> Remote {
        let user = self.user.to_string();
        let password = self.password.to_string();
        let mut url = self.url.clone();
        url.user = Some(user.clone());
        let creds = Credentials { user: Some(user), domain: url.domain.clone(), password: (!password.is_empty()).then_some(password) };
        Remote::Smb(url, creds)
    }
}

/// The Connect to Server state: at most one of the two sheets is open.
#[derive(Default)]
pub(crate) struct ServerState {
    pub(crate) address: Option<AddressSheet>,
    pub(crate) browse: Option<Browse>,
    /// Recent server addresses, newest first, as the user typed them (with `ftp://`).
    pub(crate) recent: Vec<String>,
    /// The login sheet, over whichever sheet asked for the login.
    pub(crate) login: Option<LoginSheet>,
    job: Option<Job>,
}

/// Whether `text` starts with `scheme` (written with its `://`), whatever the case.
fn has_scheme(text: &str, scheme: &str) -> bool {
    text.get(..scheme.len()).is_some_and(|p| p.eq_ignore_ascii_case(scheme))
}

/// Adds `ftp://` to an address typed without a scheme. An address with an ftp:// or smb:// scheme keeps its own
/// spelling.
pub(crate) fn normalize(typed: &str) -> String {
    let t = typed.trim();
    if has_scheme(t, "ftp://") || has_scheme(t, "smb://") {
        t.to_string()
    } else {
        format!("ftp://{t}")
    }
}

/// What a typed address asks for.
pub(crate) enum Plan {
    Ftp(FtpUrl),
    /// An SMB address. `ask` is set when the address names a user, whose password is then asked for before the first try;
    /// without a name the first try is a guest login, and the sheet appears only if the server refuses it.
    Smb { url: SmbUrl, ask: bool },
}

/// Reads a normalised address into a plan, or the reason it cannot be used.
pub(crate) fn plan_for(addr: &str) -> Result<Plan, String> {
    if has_scheme(addr, "smb://") {
        let url = SmbUrl::parse(addr).map_err(|e| e.to_string())?;
        let ask = url.user.is_some();
        Ok(Plan::Smb { url, ask })
    } else {
        FtpUrl::parse(addr).map(Plan::Ftp).map_err(|e| e.to_string())
    }
}

/// Moves `addr` to the front of the recent list, without a repeat, and keeps at most `RECENT_MAX` addresses.
pub(crate) fn remember(recent: &mut Vec<String>, addr: &str) {
    recent.retain(|r| r != addr);
    recent.insert(0, addr.to_string());
    recent.truncate(RECENT_MAX);
}

/// The recent list read from its file: one address a line, blanks and repeats dropped, at most `RECENT_MAX`.
pub(crate) fn parse_recent(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if out.len() < RECENT_MAX && !out.iter().any(|o| o == line) {
            out.push(line.to_string());
        }
    }
    out
}

/// The folder above a remote path: `/pub/docs/` is `/pub/`, `/pub` is `/`, and `/` stays `/`.
pub(crate) fn remote_parent(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(i) => format!("{}/", &trimmed[..i]),
        None => "/".to_string(),
    }
}

/// The folder `name` inside a remote path: `/pub/` and `docs` give `/pub/docs/`.
pub(crate) fn remote_join(path: &str, name: &str) -> String {
    format!("{}/{}/", path.trim_end_matches('/'), name)
}

/// Folders first, then the rest by name without case; the `.` and `..` entries are not shown.
pub(crate) fn sorted(mut entries: Vec<Row>) -> Vec<Row> {
    entries.retain(|e| e.name != "." && e.name != "..");
    entries.sort_by(|a, b| b.folder.cmp(&a.folder).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    entries
}

/// The rows of an ftp listing.
pub(crate) fn rows_from_ftp(entries: Vec<ftp::Entry>) -> Vec<Row> {
    entries
        .into_iter()
        .map(|e| {
            let folder = matches!(e.kind, ftp::Kind::Directory);
            let detail = match &e.kind {
                ftp::Kind::Directory => "Folder".to_string(),
                ftp::Kind::File => format!("{} bytes", e.size),
                ftp::Kind::Symlink(target) => format!("Link to {target}"),
                ftp::Kind::Other => String::new(),
            };
            Row { name: e.name, folder, detail }
        })
        .collect()
}

/// The rows of a server's share list: the folders it shares, not its printers or administrative shares.
pub(crate) fn rows_from_shares(shares: Vec<smb::Share>) -> Vec<Row> {
    shares
        .into_iter()
        .filter(|s| s.kind == smb::ShareKind::Disk && !s.is_hidden())
        .map(|s| Row { name: s.name, folder: true, detail: if s.comment.is_empty() { "Shared folder".to_string() } else { s.comment } })
        .collect()
}

/// The rows of a share's folder, without the hidden entries.
pub(crate) fn rows_from_smb(entries: Vec<smb::Entry>) -> Vec<Row> {
    entries
        .into_iter()
        .filter(|e| !e.is_hidden())
        .map(|e| {
            let detail = if e.is_dir { "Folder".to_string() } else { format!("{} bytes", e.size) };
            Row { name: e.name, folder: e.is_dir, detail }
        })
        .collect()
}

/// Lists an SMB server's shares, or a folder of one, with `creds`. A refused login is flagged so the sheet can ask for a
/// better one: always when the server refused the name, and for "access denied" when the try was a guest's.
fn fetch_smb(url: &SmbUrl, creds: &Credentials) -> Result<Vec<Row>, Failure> {
    let listed = match url.share {
        None => smb::list_shares(url, creds).map(rows_from_shares),
        Some(_) => smb::list_dir(url, creds).map(rows_from_smb),
    };
    listed.map_err(|e| Failure { needs_login: needs_login(&e, creds), text: e.to_string() })
}

/// Whether a failed SMB call is answered by asking for a login: the server refused the name, or it denied a guest.
fn needs_login(e: &smb::Error, creds: &Credentials) -> bool {
    matches!(e, smb::Error::AuthFailed) || (matches!(e, smb::Error::AccessDenied) && creds.user.is_none())
}

/// The failure a reply reports, as one line for the sheet: its code and first line of text.
fn failure(reply: &ftp::Reply) -> String {
    format!("{} {}", reply.code, reply.lines.first().map(String::as_str).unwrap_or(""))
}

/// Reads one reply: a single line, or a `NNN-` line through to the closing `NNN ` line.
pub(crate) fn read_reply(ctl: &mut impl BufRead) -> Result<ftp::Reply, String> {
    let mut text = String::new();
    let mut opened: Option<String> = None;
    loop {
        let mut line = String::new();
        if ctl.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Err("the server closed the connection".to_string());
        }
        let line = line.trim_end().to_string();
        text.push_str(&line);
        text.push('\n');
        let closing = match &opened {
            Some(code) => line.starts_with(code.as_str()) && line.as_bytes().get(3) == Some(&b' '),
            None => false,
        };
        if closing {
            break;
        }
        if opened.is_none() {
            if line.as_bytes().get(3) == Some(&b'-') {
                opened = line.get(..3).map(str::to_string);
            } else {
                break;
            }
        }
    }
    ftp::Reply::parse(&text).map_err(|e| e.to_string())
}

/// The reply when its class is the one wanted, or its failure.
fn expect(reply: ftp::Reply, class: ftp::Class) -> Result<ftp::Reply, String> {
    if reply.class() == Some(class) {
        Ok(reply)
    } else {
        Err(failure(&reply))
    }
}

fn send(ctl: &mut BufReader<TcpStream>, cmd: Command<'_>) -> Result<(), String> {
    let line = cmd.to_line().map_err(|e| e.to_string())?;
    ctl.get_mut().write_all(line.as_bytes()).map_err(|e| e.to_string())
}

/// Lists a remote folder: logs in, asks for a passive data connection, reads the LIST output and parses it.
pub(crate) fn fetch(url: &FtpUrl) -> Result<Vec<ftp::Entry>, String> {
    let addr = (url.host.as_str(), url.port)
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("the host has no address")?;
    let stream = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    let mut ctl = BufReader::new(stream);
    expect(read_reply(&mut ctl)?, ftp::Class::Completion)?;
    send(&mut ctl, Command::User(url.user.as_deref().unwrap_or(ANONYMOUS)))?;
    let login = read_reply(&mut ctl)?;
    if login.class() == Some(ftp::Class::Intermediate) {
        send(&mut ctl, Command::Pass(url.password.as_deref().unwrap_or(ANONYMOUS_PASS)))?;
        expect(read_reply(&mut ctl)?, ftp::Class::Completion)?;
    } else {
        expect(login, ftp::Class::Completion)?;
    }
    send(&mut ctl, Command::TypeAscii)?;
    expect(read_reply(&mut ctl)?, ftp::Class::Completion)?;
    send(&mut ctl, Command::Passive)?;
    let pasv = expect(read_reply(&mut ctl)?, ftp::Class::Completion)?;
    let data_addr = ftp::pasv_address(&pasv).map_err(|e| e.to_string())?;
    let mut data = TcpStream::connect_timeout(&SocketAddr::V4(data_addr), TIMEOUT).map_err(|e| e.to_string())?;
    data.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    let path = if url.path.is_empty() { "/" } else { url.path.as_str() };
    send(&mut ctl, Command::List(path))?;
    let opening = read_reply(&mut ctl)?;
    if !matches!(opening.class(), Some(ftp::Class::Preliminary | ftp::Class::Completion)) {
        return Err(failure(&opening));
    }
    let mut raw = Vec::new();
    data.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    drop(data);
    expect(read_reply(&mut ctl)?, ftp::Class::Completion)?;
    let _ = send(&mut ctl, Command::Quit);
    Ok(ftp::parse_listing(&String::from_utf8_lossy(&raw)))
}

fn recent_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"));
    base.join("bifrost").join("files-servers.txt")
}

/// The recent servers from their file; empty when there is no file or it cannot be read.
fn load_recent() -> Vec<String> {
    std::fs::read_to_string(recent_path()).map(|t| parse_recent(&t)).unwrap_or_default()
}

/// Writes the recent servers. A failed write only loses the list for the next launch.
fn save_recent(recent: &[String]) {
    let path = recent_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, recent.join("\n"));
}

enum AddressAct {
    Connect(String),
    Close,
    Nothing,
}

enum BrowseAct {
    Go(Remote),
    Close,
    Nothing,
}

impl Files {
    /// Go > Connect to Server (Cmd+K): opens the address sheet with the recent servers.
    pub(crate) fn open_connect(&mut self) {
        self.preview = None;
        self.info = None;
        self.fx.server.recent = load_recent();
        self.fx.server.address = Some(AddressSheet { field: TextField::new(), picked: None, message: None });
        self.revision += 1;
    }
    /// Starts a listing for a typed address. The sheet stays open with "Connecting…" until the answer comes back. An
    /// smb:// address that names a user asks for the password first.
    fn start_connect(&mut self, typed: &str) {
        if self.fx.server.job.is_some() {
            return;
        }
        let addr = normalize(typed);
        match plan_for(&addr) {
            Ok(Plan::Ftp(url)) => self.spawn_list(addr, Remote::Ftp(url)),
            Ok(Plan::Smb { url, ask: true }) => self.fx.server.login = Some(LoginSheet::new(addr, url, None)),
            Ok(Plan::Smb { url, ask: false }) => self.spawn_list(addr, Remote::Smb(url, Credentials::default())),
            Err(e) => self.set_server_message(e),
        }
    }
    fn spawn_list(&mut self, addr: String, remote: Remote) {
        let (tx, rx) = mpsc::channel();
        let for_thread = remote.clone();
        DispatchQueue::global(Qos::Utility).async_(move || {
            let answer = match &for_thread {
                Remote::Ftp(url) => fetch(url).map(rows_from_ftp).map_err(Failure::plain),
                Remote::Smb(url, creds) => fetch_smb(url, creds),
            };
            let _ = tx.send(answer);
            DispatchQueue::main().async_(|| {});
        });
        self.fx.server.job = Some(Job { addr, remote, rx });
        self.set_server_message("Connecting…".to_string());
    }
    /// The message of the sheet that is open.
    fn set_server_message(&mut self, msg: String) {
        if let Some(sheet) = self.fx.server.address.as_mut() {
            sheet.message = Some(msg);
        } else if let Some(b) = self.fx.server.browse.as_mut() {
            b.message = Some(msg);
        }
    }
    /// Takes the listing thread's answer. Returns true when the screen changed.
    pub(crate) fn poll_server(&mut self) -> bool {
        let answer = {
            let Some(job) = self.fx.server.job.as_ref() else {
                return false;
            };
            match job.rx.try_recv() {
                Ok(answer) => answer,
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => Err(Failure::plain("the connection stopped".to_string())),
            }
        };
        let Some(job) = self.fx.server.job.take() else {
            return false;
        };
        let from_sheet = self.fx.server.address.is_some();
        match answer {
            Ok(entries) => {
                if from_sheet {
                    remember(&mut self.fx.server.recent, &job.addr);
                    save_recent(&self.fx.server.recent);
                }
                self.fx.server.address = None;
                self.fx.server.browse = Some(Browse { remote: job.remote, entries: sorted(entries), selected: 0, message: None });
            }
            Err(f) => match (&job.remote, f.needs_login) {
                // the server wants a login: ask for it over whichever sheet was trying
                (Remote::Smb(url, _), true) => {
                    let note = "The server needs a name and password.".to_string();
                    self.fx.server.login = Some(LoginSheet::new(job.addr, url.clone(), Some(note)));
                }
                _ => self.set_server_message(f.text),
            },
        }
        self.revision += 1;
        true
    }
    /// Keys and clicks for the Connect to Server sheets while one is open. Clicks are swallowed so they do not reach
    /// the folder behind. Returns false when no sheet is open.
    pub(crate) fn server_input(&mut self, input: &Input) -> bool {
        let handled = if self.fx.server.login.is_some() {
            self.login_input(input)
        } else if self.fx.server.browse.is_some() {
            self.browse_input(input)
        } else if self.fx.server.address.is_some() {
            self.address_input(input)
        } else {
            return false;
        };
        if handled {
            self.revision += 1;
        }
        handled
    }
    fn login_input(&mut self, input: &Input) -> bool {
        let act = match self.fx.server.login.as_mut() {
            Some(sheet) => sheet.key(input),
            None => return false,
        };
        match act {
            LoginAct::Connect => {
                if let Some(sheet) = self.fx.server.login.take() {
                    let remote = sheet.remote();
                    self.spawn_list(sheet.addr.clone(), remote);
                }
            }
            LoginAct::Close => {
                self.fx.server.login = None;
                // the try that asked is over; the sheet underneath goes back to waiting for an address
                self.fx.server.job = None;
                self.set_server_message("Cancelled.".to_string());
            }
            LoginAct::Nothing => {}
        }
        true
    }
    fn address_input(&mut self, input: &Input) -> bool {
        let busy = self.fx.server.job.is_some();
        let act = {
            let recent = &self.fx.server.recent;
            let Some(sheet) = self.fx.server.address.as_mut() else {
                return false;
            };
            match input {
                Input::Touch { .. } | Input::Context { .. } => AddressAct::Nothing,
                Input::Key(KeyInput::Text(s)) => {
                    if !busy {
                        sheet.picked = None;
                        sheet.field.push_str(s);
                    }
                    AddressAct::Nothing
                }
                Input::Key(KeyInput::Code(code)) => match *code {
                    13 => match sheet.picked.and_then(|i| recent.get(i)) {
                        Some(addr) => AddressAct::Connect(addr.clone()),
                        None => AddressAct::Connect(sheet.field.to_string()),
                    },
                    KEY_ESCAPE | KEY_ESCAPE_ASCII => AddressAct::Close,
                    40 if !recent.is_empty() => {
                        sheet.picked = Some(sheet.picked.map_or(0, |i| (i + 1).min(recent.len() - 1)));
                        AddressAct::Nothing
                    }
                    38 => {
                        sheet.picked = sheet.picked.and_then(|i| i.checked_sub(1));
                        AddressAct::Nothing
                    }
                    8 => {
                        sheet.picked = None;
                        sheet.field.pop();
                        AddressAct::Nothing
                    }
                    46 => {
                        sheet.field.delete();
                        AddressAct::Nothing
                    }
                    37 | 39 => {
                        sheet.field.move_caret(*code == 39);
                        AddressAct::Nothing
                    }
                    36 => {
                        sheet.field.home();
                        AddressAct::Nothing
                    }
                    35 => {
                        sheet.field.end();
                        AddressAct::Nothing
                    }
                    _ => AddressAct::Nothing,
                },
                _ => AddressAct::Nothing,
            }
        };
        match act {
            AddressAct::Connect(addr) => self.start_connect(&addr),
            AddressAct::Close => {
                self.fx.server.address = None;
                self.fx.server.job = None;
            }
            AddressAct::Nothing => {}
        }
        true
    }
    fn browse_input(&mut self, input: &Input) -> bool {
        let act = {
            let Some(b) = self.fx.server.browse.as_mut() else {
                return false;
            };
            match input {
                Input::Touch { .. } | Input::Context { .. } => BrowseAct::Nothing,
                Input::Key(KeyInput::Code(code)) => match *code {
                    38 => {
                        b.selected = b.selected.saturating_sub(1);
                        BrowseAct::Nothing
                    }
                    40 => {
                        b.selected = (b.selected + 1).min(b.entries.len().saturating_sub(1));
                        BrowseAct::Nothing
                    }
                    13 => match b.entries.get(b.selected) {
                        Some(e) if e.folder => BrowseAct::Go(b.remote.child(&e.name)),
                        Some(_) => {
                            b.message = Some("Files can't open remote files yet.".to_string());
                            BrowseAct::Nothing
                        }
                        None => BrowseAct::Nothing,
                    },
                    8 => BrowseAct::Go(b.remote.parent()),
                    KEY_ESCAPE | KEY_ESCAPE_ASCII => BrowseAct::Close,
                    _ => BrowseAct::Nothing,
                },
                _ => BrowseAct::Nothing,
            }
        };
        match act {
            BrowseAct::Go(remote) => {
                if self.fx.server.job.is_none() {
                    self.spawn_list(String::new(), remote);
                }
            }
            BrowseAct::Close => {
                self.fx.server.browse = None;
                self.fx.server.job = None;
            }
            BrowseAct::Nothing => {}
        }
        true
    }
    /// The Connect to Server sheet, drawn over the content area in the same place as Go to Folder.
    pub(crate) fn paint_server(&self, ui: &Ui, s: &mut Surface) {
        if let Some(b) = &self.fx.server.browse {
            self.paint_browse(ui, s, b);
        } else if let Some(sheet) = &self.fx.server.address {
            self.paint_address(ui, s, sheet);
        }
        // the login sheet is drawn over whichever sheet asked for it
        if let Some(login) = &self.fx.server.login {
            self.paint_login(ui, s, login);
        }
    }
    fn paint_login(&self, ui: &Ui, s: &mut Surface, sheet: &LoginSheet) {
        let w = (self.w - self.side() - 32.).clamp(240., 440.);
        let (x, y, h) = (self.side() + (self.w - self.side() - w) / 2., 96., 176.);
        rect(s, ui, x, y, w, h, CARD_BG.now());
        let title = format!("Connect to {}", sheet.url.host);
        text(s, ui, &title, MAC_TITLE3_PT, LABEL.now(), x + 20., y + 16., w - 40.);
        let shown = sheet.masked();
        let fields: [(&str, &TextField, &str, bool); 2] = [("Name", &sheet.user, "Name", !sheet.on_password), ("Password", &shown, "Password", sheet.on_password)];
        for (row, (label, field, placeholder, focused)) in fields.into_iter().enumerate() {
            let fy = y + 52. + row as f32 * 36.;
            text(s, ui, label, bifrost_app::kit::ui::MAC_SUBHEAD_PT, LABEL_2.now(), x + 20., fy + 4., 76.);
            bifrost_app::widgets::text_field(s, ui, field, Rect { x: x + 100., y: fy, w: w - 120., h: 24. }, placeholder, focused);
        }
        let hint = sheet.message.as_deref().unwrap_or("Tab switches field. Return connects. Escape cancels.");
        text(s, ui, hint, bifrost_app::kit::ui::MAC_SUBHEAD_PT, LABEL_2.now(), x + 20., y + h - 28., w - 40.);
    }
    fn paint_address(&self, ui: &Ui, s: &mut Surface, sheet: &AddressSheet) {
        let w = (self.w - self.side() - 32.).clamp(240., 440.);
        let (x, y) = (self.side() + (self.w - self.side() - w) / 2., 72.);
        let recent = &self.fx.server.recent;
        let h = if recent.is_empty() { 120. } else { 124. + recent.len() as f32 * 22. };
        rect(s, ui, x, y, w, h, CARD_BG.now());
        text(s, ui, "Server Address:", MAC_TITLE3_PT, LABEL.now(), x + 20., y + 16., w - 40.);
        bifrost_app::widgets::text_field(s, ui, &sheet.field, Rect { x: x + 20., y: y + 48., w: w - 40., h: 24. }, "ftp://example.com or smb://server/share", sheet.picked.is_none());
        let mut row_y = y + 84.;
        if !recent.is_empty() {
            text(s, ui, "Recent Servers", bifrost_app::kit::ui::MAC_SUBHEAD_PT, LABEL_2.now(), x + 20., row_y, w - 40.);
            row_y += 20.;
            for (i, addr) in recent.iter().enumerate() {
                let picked = sheet.picked == Some(i);
                if picked {
                    rect(s, ui, x + 12., row_y - 2., w - 24., 22., SELECTION.now());
                }
                let colour = if picked { WHITE } else { LABEL.now() };
                text(s, ui, addr, bifrost_app::kit::ui::MAC_SUBHEAD_PT, colour, x + 20., row_y, w - 40.);
                row_y += 22.;
            }
        }
        let hint = sheet.message.as_deref().unwrap_or("Return connects. Down picks a recent server. Escape cancels.");
        text(s, ui, hint, bifrost_app::kit::ui::MAC_SUBHEAD_PT, LABEL_2.now(), x + 20., y + h - 26., w - 40.);
    }
    fn paint_browse(&self, ui: &Ui, s: &mut Surface, b: &Browse) {
        let x = self.side() + 16.;
        let (w, h) = ((self.w - x - 16.).max(120.), (self.h - 72. - 16.).max(120.));
        rect(s, ui, x, 72., w, h, CARD_BG.now());
        let title = b.remote.title();
        text(s, ui, &title, MAC_TITLE3_PT, LABEL.now(), x + 20., 88., w - 40.);
        let top = 124.;
        let footer = 40.;
        let visible = (((h - (top - 72.) - footer) / 22.).max(1.)) as usize;
        let first = if b.selected >= visible { b.selected + 1 - visible } else { 0 };
        for (row, e) in b.entries.iter().enumerate().skip(first).take(visible) {
            let y = top + (row - first) as f32 * 22.;
            let picked = row == b.selected;
            if picked {
                rect(s, ui, x + 12., y - 2., w - 24., 22., SELECTION.now());
            }
            let colour = if picked { WHITE } else { LABEL.now() };
            text(s, ui, &e.name, bifrost_app::kit::ui::MAC_SUBHEAD_PT, colour, x + 20., y, w * 0.6);
            text(s, ui, &e.detail, bifrost_app::kit::ui::MAC_SUBHEAD_PT, if picked { WHITE } else { LABEL_2.now() }, x + w * 0.6, y, w * 0.4 - 20.);
        }
        if b.entries.is_empty() {
            text(s, ui, "This folder is empty", MAC_CALLOUT_PT, LABEL_2.now(), x + 20., top, w - 40.);
        }
        let hint = b.message.as_deref().unwrap_or("Return opens a folder. Delete goes up one level. Escape disconnects.");
        text(s, ui, hint, bifrost_app::kit::ui::MAC_SUBHEAD_PT, LABEL_2.now(), x + 20., 72. + h - 30., w - 40.);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_adds_the_scheme_only_when_missing() {
        assert_eq!(normalize("  files.example.is/pub "), "ftp://files.example.is/pub");
        assert_eq!(normalize("FTP://files.example.is"), "FTP://files.example.is");
        assert_eq!(normalize(" SMB://nas.local/Photos "), "SMB://nas.local/Photos", "an smb address keeps its scheme");
        assert_eq!(normalize("ü"), "ftp://ü");
    }

    #[test]
    fn remember_moves_a_repeat_to_the_front_and_caps_the_list() {
        let mut recent = vec!["ftp://a".to_string(), "ftp://b".to_string()];
        remember(&mut recent, "ftp://b");
        assert_eq!(recent, ["ftp://b", "ftp://a"]);
        let mut full: Vec<String> = (0..RECENT_MAX).map(|i| format!("ftp://h{i}")).collect();
        remember(&mut full, "ftp://new");
        assert_eq!(full.len(), RECENT_MAX);
        assert_eq!(full[0], "ftp://new");
        assert_eq!(full[RECENT_MAX - 1], format!("ftp://h{}", RECENT_MAX - 2));
    }

    #[test]
    fn parse_recent_skips_blanks_and_repeats() {
        assert_eq!(parse_recent("ftp://a\n\n  ftp://b \nftp://a\n"), ["ftp://a", "ftp://b"]);
        assert_eq!(parse_recent(""), Vec::<String>::new());
    }

    #[test]
    fn remote_paths_move_between_folders() {
        assert_eq!(remote_parent("/pub/docs/"), "/pub/");
        assert_eq!(remote_parent("/pub/docs"), "/pub/");
        assert_eq!(remote_parent("/pub"), "/");
        assert_eq!(remote_parent("/"), "/");
        assert_eq!(remote_join("/", "pub"), "/pub/");
        assert_eq!(remote_join("/pub/", "docs"), "/pub/docs/");
    }

    #[test]
    fn a_single_line_reply_is_one_line() {
        let reply = read_reply(&mut std::io::Cursor::new(&b"550 No such file\r\n"[..])).unwrap();
        assert_eq!(reply.code, 550);
        assert_eq!(reply.class(), Some(ftp::Class::PermanentNegative));
    }

    #[test]
    fn a_multi_line_reply_reads_through_its_closing_line() {
        let text = b"220-Welcome\r\nNote\r\n220 Ready\r\nnot read\r\n";
        let reply = read_reply(&mut std::io::Cursor::new(&text[..])).unwrap();
        assert_eq!(reply.code, 220);
        assert_eq!(reply.lines, ["Welcome", "Note", "Ready"]);
    }

    #[test]
    fn a_closed_control_connection_is_an_error() {
        assert_eq!(read_reply(&mut std::io::Cursor::new(&b""[..])), Err("the server closed the connection".to_string()));
    }

    #[test]
    fn the_listing_shows_folders_first_without_dot_entries() {
        let text = "-rw-r--r--    1 ftp      ftp            12 Jan  1 00:00 a.txt\r\n\
drwxr-xr-x    2 ftp      ftp          4096 Jan  1 00:00 pub\r\n\
drwxr-xr-x    2 ftp      ftp          4096 Jan  1 00:00 .\r\n";
        let names: Vec<String> = sorted(rows_from_ftp(ftp::parse_listing(text))).into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["pub", "a.txt"]);
    }

    fn smb_remote(addr: &str) -> Remote {
        Remote::Smb(SmbUrl::parse(addr).unwrap(), Credentials::default())
    }

    #[test]
    fn an_smb_address_is_planned_as_a_guest_try_or_a_password_prompt() {
        assert!(matches!(plan_for("smb://nas.local"), Ok(Plan::Smb { ask: false, .. })), "no name: try as a guest first");
        assert!(matches!(plan_for("smb://ann@nas.local/Photos"), Ok(Plan::Smb { ask: true, .. })), "a name: ask for its password");
        assert!(matches!(plan_for("smb://ann:secret@nas.local"), Err(_)), "a password in the address is refused");
        assert!(matches!(plan_for("ftp://files.example.is"), Ok(Plan::Ftp(_))));
        assert!(plan_for("smb://").is_err());
    }

    #[test]
    fn the_smb_browser_walks_from_the_share_list_into_a_share_and_back() {
        let top = smb_remote("smb://nas.local");
        assert_eq!(top.title(), "smb://nas.local");
        let share = top.child("Photos");
        assert_eq!(share.title(), "smb://nas.local/Photos");
        let deeper = share.child("2024").child("summer trip");
        assert_eq!(deeper.title(), "smb://nas.local/Photos/2024/summer%20trip");
        assert_eq!(deeper.parent().title(), "smb://nas.local/Photos/2024");
        assert_eq!(deeper.parent().parent().title(), "smb://nas.local/Photos");
        assert_eq!(deeper.parent().parent().parent().title(), "smb://nas.local", "the top of a share goes up to the share list");
        assert_eq!(top.parent().title(), "smb://nas.local", "the share list stays");
    }

    #[test]
    fn the_ftp_browser_still_walks_by_path() {
        let top = Remote::Ftp(FtpUrl::parse("ftp://files.example.is").unwrap());
        let docs = top.child("pub").child("docs");
        assert_eq!(docs.title(), "files.example.is:21/pub/docs/");
        assert_eq!(docs.parent().title(), "files.example.is:21/pub/");
    }

    #[test]
    fn shares_and_folders_become_rows_without_the_hidden_ones() {
        let shares = smb::parse_shares("Disk|Photos|Family pictures\nDisk|Docs|\nDisk|C$|Default share\nPrinter|Laser|Office\nIPC|IPC$|IPC Service\n");
        let rows = sorted(rows_from_shares(shares));
        assert_eq!(rows, [
            Row { name: "Docs".into(), folder: true, detail: "Shared folder".into() },
            Row { name: "Photos".into(), folder: true, detail: "Family pictures".into() },
        ]);
        let listing = [
            "  .                                   D        0  Sun Mar  3 10:11:12 2024",
            "  ..                                  D        0  Sun Mar  3 10:11:12 2024",
            "  trip photos                         D        0  Sun Mar  3 10:11:12 2024",
            "  notes.txt                           A     2048  Sun Mar  3 10:11:12 2024",
            "  .hidden                             A       12  Sun Mar  3 10:11:12 2024",
        ]
        .join("\n");
        let rows = sorted(rows_from_smb(smb::parse_listing(&listing)));
        assert_eq!(rows, [
            Row { name: "trip photos".into(), folder: true, detail: "Folder".into() },
            Row { name: "notes.txt".into(), folder: false, detail: "2048 bytes".into() },
        ]);
    }

    fn type_into(sheet: &mut LoginSheet, text: &str) {
        sheet.key(&Input::Key(KeyInput::Text(text.to_string())));
    }

    fn press(sheet: &mut LoginSheet, code: u32) -> LoginAct {
        sheet.key(&Input::Key(KeyInput::Code(code)))
    }

    #[test]
    fn the_login_sheet_takes_a_name_then_a_masked_password_and_connects_with_both() {
        let mut sheet = LoginSheet::new("smb://nas.local/Photos".into(), SmbUrl::parse("smb://nas.local/Photos").unwrap(), Some("The server needs a name and password.".into()));
        assert!(!sheet.on_password, "no name in the address: the name field has the focus");
        assert!(matches!(press(&mut sheet, 13), LoginAct::Nothing), "Return without a name does not connect");
        assert!(sheet.message.as_deref().unwrap().contains("Type a name"));
        type_into(&mut sheet, "ann");
        assert!(matches!(press(&mut sheet, 9), LoginAct::Nothing));
        assert!(sheet.on_password);
        type_into(&mut sheet, "hunter2");
        assert_eq!(&*sheet.masked(), "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}", "the password is never drawn");
        press(&mut sheet, 8);
        assert_eq!(&*sheet.password, "hunter");
        assert!(matches!(press(&mut sheet, 13), LoginAct::Connect));
        let Remote::Smb(url, creds) = sheet.remote() else { panic!("an SMB remote") };
        assert_eq!(url.user.as_deref(), Some("ann"));
        assert_eq!(creds.user.as_deref(), Some("ann"));
        assert_eq!(creds.password.as_deref(), Some("hunter"));
        assert_eq!(url.to_text(), "smb://ann@nas.local/Photos", "the address never carries the password");
        assert!(!format!("{creds:?}").contains("hunter"), "the credentials hide the password in logs");
    }

    #[test]
    fn a_name_in_the_address_leaves_the_password_to_type_and_escape_cancels() {
        let mut sheet = LoginSheet::new("smb://ann@nas.local".into(), SmbUrl::parse("smb://ann@nas.local").unwrap(), None);
        assert!(sheet.on_password);
        assert_eq!(&*sheet.user, "ann");
        assert!(matches!(press(&mut sheet, KEY_ESCAPE), LoginAct::Close));
        // an empty password is sent as none: the client must not stop to ask on a terminal it does not have
        let Remote::Smb(_, creds) = sheet.remote() else { panic!("an SMB remote") };
        assert_eq!(creds.password, None);
    }

    #[test]
    fn a_refused_login_asks_and_an_access_denied_guest_asks_but_a_logged_in_one_does_not() {
        let flag = |e: smb::Error, creds: &Credentials| needs_login(&e, creds);
        let guest = Credentials::default();
        let named = Credentials { user: Some("ann".into()), ..Credentials::default() };
        assert!(flag(smb::Error::AuthFailed, &named));
        assert!(flag(smb::Error::AccessDenied, &guest));
        assert!(!flag(smb::Error::AccessDenied, &named));
        assert!(!flag(smb::Error::Unreachable, &guest));
    }
}
