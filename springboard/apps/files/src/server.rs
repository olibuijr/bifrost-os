//! Go > Connect to Server: an ftp:// address sheet with the recent servers, and a browser sheet that lists a remote
//! folder. The listing runs on its own thread over a control connection and a passive data connection (RFC 959). The
//! model in `bifrost_app::ftp` parses the URL, the replies and the listing; this file only moves the bytes. Remote files
//! cannot be opened or copied yet; the browser lists folders and moves between them.
use crate::*;
use bifrost_app::ftp::{self, Command, FtpUrl};
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

/// A remote folder shown by the browser sheet.
pub(crate) struct Browse {
    pub(crate) url: FtpUrl,
    pub(crate) entries: Vec<ftp::Entry>,
    pub(crate) selected: usize,
    pub(crate) message: Option<String>,
}

/// A listing running on its own thread: the address the user typed (empty for a folder opened inside the browser),
/// the parsed URL, and the channel its answer comes back on.
struct Job {
    addr: String,
    url: FtpUrl,
    rx: Receiver<Result<Vec<ftp::Entry>, String>>,
}

/// The Connect to Server state: at most one of the two sheets is open.
#[derive(Default)]
pub(crate) struct ServerState {
    pub(crate) address: Option<AddressSheet>,
    pub(crate) browse: Option<Browse>,
    /// Recent server addresses, newest first, as the user typed them (with `ftp://`).
    pub(crate) recent: Vec<String>,
    job: Option<Job>,
}

/// Adds `ftp://` to an address typed without a scheme. An address with a scheme keeps its own spelling.
pub(crate) fn normalize(typed: &str) -> String {
    let t = typed.trim();
    if t.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("ftp://")) {
        t.to_string()
    } else {
        format!("ftp://{t}")
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
pub(crate) fn sorted(mut entries: Vec<ftp::Entry>) -> Vec<ftp::Entry> {
    entries.retain(|e| e.name != "." && e.name != "..");
    entries.sort_by(|a, b| is_folder(b).cmp(&is_folder(a)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    entries
}

fn is_folder(e: &ftp::Entry) -> bool {
    matches!(e.kind, ftp::Kind::Directory)
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
    Go(FtpUrl),
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
    /// Starts a listing for a typed address. The sheet stays open with "Connecting…" until the answer comes back.
    fn start_connect(&mut self, typed: &str) {
        if self.fx.server.job.is_some() {
            return;
        }
        let addr = normalize(typed);
        match FtpUrl::parse(&addr) {
            Ok(url) => self.spawn_list(addr, url),
            Err(e) => self.set_server_message(e.to_string()),
        }
    }
    fn spawn_list(&mut self, addr: String, url: FtpUrl) {
        let (tx, rx) = mpsc::channel();
        let for_thread = url.clone();
        DispatchQueue::global(Qos::Utility).async_(move || {
            let _ = tx.send(fetch(&for_thread));
            DispatchQueue::main().async_(|| {});
        });
        self.fx.server.job = Some(Job { addr, url, rx });
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
                Err(TryRecvError::Disconnected) => Err("the connection stopped".to_string()),
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
                self.fx.server.browse = Some(Browse { url: job.url, entries: sorted(entries), selected: 0, message: None });
            }
            Err(msg) => self.set_server_message(msg),
        }
        self.revision += 1;
        true
    }
    /// Keys and clicks for the Connect to Server sheets while one is open. Clicks are swallowed so they do not reach
    /// the folder behind. Returns false when no sheet is open.
    pub(crate) fn server_input(&mut self, input: &Input) -> bool {
        let handled = if self.fx.server.browse.is_some() {
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
                    27 => AddressAct::Close,
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
                        Some(e) if is_folder(e) => {
                            let mut url = b.url.clone();
                            url.path = remote_join(&url.path, &e.name);
                            BrowseAct::Go(url)
                        }
                        Some(_) => {
                            b.message = Some("Files can't open remote files yet.".to_string());
                            BrowseAct::Nothing
                        }
                        None => BrowseAct::Nothing,
                    },
                    8 => {
                        let mut url = b.url.clone();
                        url.path = remote_parent(&url.path);
                        BrowseAct::Go(url)
                    }
                    27 => BrowseAct::Close,
                    _ => BrowseAct::Nothing,
                },
                _ => BrowseAct::Nothing,
            }
        };
        match act {
            BrowseAct::Go(url) => {
                if self.fx.server.job.is_none() {
                    self.spawn_list(String::new(), url);
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
    }
    fn paint_address(&self, ui: &Ui, s: &mut Surface, sheet: &AddressSheet) {
        let w = (self.w - self.side() - 32.).clamp(240., 440.);
        let (x, y) = (self.side() + (self.w - self.side() - w) / 2., 72.);
        let recent = &self.fx.server.recent;
        let h = if recent.is_empty() { 120. } else { 124. + recent.len() as f32 * 22. };
        rect(s, ui, x, y, w, h, CARD_BG.now());
        text(s, ui, "Server Address:", MAC_TITLE3_PT, LABEL.now(), x + 20., y + 16., w - 40.);
        bifrost_app::widgets::text_field(s, ui, &sheet.field, Rect { x: x + 20., y: y + 48., w: w - 40., h: 24. }, "ftp://example.com/folder", sheet.picked.is_none());
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
        let title = format!("{}:{}{}", b.url.host, b.url.port, b.url.path);
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
            let detail = match &e.kind {
                ftp::Kind::Directory => "Folder".to_string(),
                ftp::Kind::File => format!("{} bytes", e.size),
                ftp::Kind::Symlink(target) => format!("Link to {target}"),
                ftp::Kind::Other => String::new(),
            };
            text(s, ui, &e.name, bifrost_app::kit::ui::MAC_SUBHEAD_PT, colour, x + 20., y, w * 0.6);
            text(s, ui, &detail, bifrost_app::kit::ui::MAC_SUBHEAD_PT, if picked { WHITE } else { LABEL_2.now() }, x + w * 0.6, y, w * 0.4 - 20.);
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
        let names: Vec<String> = sorted(ftp::parse_listing(text)).into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["pub", "a.txt"]);
    }
}
