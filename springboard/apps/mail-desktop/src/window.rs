//! The Mail window: the state in [`crate::state`] wired to the network jobs ([`crate::sync`]), the shared Internet
//! Accounts list and Passwords for the logins, and the notifications for new mail. This file holds the flow and the
//! input; drawing and hit-testing are in [`crate::draw`].

use crate::accounts;
use crate::attachments;
use crate::flags::Flag;
use crate::message;
use crate::migrate;
use crate::search::Query;
use crate::state::{Mail, Notice, Sheet, ARCHIVE_NAMES, INBOX, TRASH_NAMES};
use crate::sync::{self, Change, Event};
use crate::transport;
use bifrost_app::controls::internet_accounts::Account;
use bifrost_app::keychain::{self, Error as LegacyError, Reply as KeyReply};
use bifrost_app::kit::{clipboard, Scroller, TextCache, Ui};
use bifrost_app::passwords::{self, Error as KeyError};
use bifrost_app::usernotif::{self, Request};
use bifrost_app::{AppMenu, AppMenuItem, Ctx, KeyInput, TouchKind};
use bifrost_app::proto::{CHORD_CTRL, CHORD_META};
use bifrost_core::json::Json;
use springboard::deildu::http::Transport;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

pub const MENU_NEW: u32 = 1;
pub const MENU_REPLY: u32 = 2;
pub const MENU_SEND: u32 = 3;
pub const MENU_REFRESH: u32 = 4;
pub const MENU_ADD: u32 = 5;
pub const MENU_FLAG: u32 = 6;
pub const MENU_UNREAD: u32 = 7;
pub const MENU_ARCHIVE: u32 = 8;
pub const MENU_TRASH: u32 = 9;
pub const MENU_FIND: u32 = 10;
pub const MENU_SHOW_ALL: u32 = 11;
pub const MENU_THREADS: u32 = 12;
pub const MENU_SAVE: u32 = 13;
pub const MENU_PRINT: u32 = 14;

/// How often the inbox of every account is checked for new mail, in milliseconds.
pub const POLL_MS: f64 = 60_000.0;
/// An account whose inbox is watched over IDLE is listed by the watcher when mail arrives; the poll still checks it
/// once every this many poll rounds (10 minutes) in case the watcher missed something.
pub const IDLE_CHECK_ROUNDS: u32 = 10;
/// Movement, in points, before a press counts as a drag.
const DRAG_PT: f32 = 6.0;
/// Key codes the shell sends for the editing keys (as the other apps read them).
const KEY_ESCAPE: u32 = 1;
const KEY_BACKSPACE: u32 = 8;
const KEY_TAB: u32 = 9;
const KEY_RETURN: u32 = 13;
/// evdev V, for the paste chord.
const KEY_V: u32 = 47;

/// What the window is waiting for from Passwords (the passwords group, through `service_reply`).
enum Pending {
    /// The password of an account, read when the app starts. The id is the account's.
    Load(String),
    /// A new account's password, being added. `Replace` follows when an item already exists.
    Add { account: Account, password: String },
    Replace { account: Account, password: String },
    /// An account moved from the 0.3.0 file: its password is read from the old keychain item `legacy`.
    Migrate { account: Account, legacy: String },
    /// A password read from the old keychain item `legacy` is being saved in Passwords.
    Moved { account: Account, legacy: String },
    /// The old keychain item is being deleted. Nothing follows its answer.
    Discard,
}

impl Pending {
    /// The keychain answers: the window tells them apart from Passwords' by the request id.
    fn from_keychain(&self) -> bool {
        matches!(self, Pending::Migrate { .. } | Pending::Discard)
    }
}

/// Where a press started. Only the two panes scroll; the rest is chrome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    List,
    Reader,
    Chrome,
}

#[derive(Clone, Copy)]
struct Press {
    x: f32,
    y: f32,
    moved: bool,
    region: Region,
}

pub struct MailWindow {
    pub mail: Mail,
    transport: Arc<dyn Transport>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    wake_rx: Option<UnixStream>,
    wake_tx: Option<UnixStream>,
    pending: HashMap<u32, Pending>,
    /// Jobs started and not yet answered.
    jobs: usize,
    /// Counter for Message-IDs of the messages sent in this run.
    seq: u64,
    next_poll: f64,
    /// Poll rounds run so far (see [`IDLE_CHECK_ROUNDS`]).
    poll_round: u32,
    /// Accounts with an inbox watcher started. A watcher runs for the rest of the run, or until its server has no IDLE.
    watching: HashSet<String>,
    /// Accounts whose watcher is connected and waiting in IDLE.
    live: HashSet<String>,
    pub w: u32,
    pub h: u32,
    pub list_scroll: Scroller,
    pub reader_scroll: Scroller,
    press: Option<Press>,
    pub labels: TextCache,
    /// The wrapped text of the open message, for the width it was laid out at.
    pub laid: Option<crate::draw::Laid>,
    dirty: bool,
}

/// A connected pair of sockets: the job threads write to one, the loop reads the other (`wake_fd`).
fn wake_pair() -> Option<(UnixStream, UnixStream)> {
    let (rx, tx) = UnixStream::pair().ok()?;
    rx.set_nonblocking(true).ok()?;
    tx.set_nonblocking(true).ok()?;
    Some((rx, tx))
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

impl MailWindow {
    /// Reads the mail accounts of the shared list and asks Passwords for each password. Mail is fetched as the passwords
    /// arrive.
    pub fn new(ctx: &Ctx) -> MailWindow {
        // the first run after 0.3.0 moves its accounts into the shared list, before the list is read
        let (imports, import_error) = match migrate::run(&ctx.data_dir) {
            Ok(imported) => (imported, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let (mail_accounts, load_error) = match accounts::load_mail() {
            Ok(list) => (list, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let (tx, rx) = channel();
        let (wake_rx, wake_tx) = match wake_pair() {
            Some((rx, tx)) => (Some(rx), Some(tx)),
            None => (None, None),
        };
        let mut mail = Mail::new(mail_accounts);
        if let Some(e) = load_error.or(import_error) {
            mail.status = e;
        }
        let legacy: HashMap<String, String> = imports.into_iter().map(|i| (i.account.id, i.legacy_id)).collect();
        let mut window = MailWindow {
            mail,
            transport: transport::shared(),
            tx,
            rx,
            wake_rx,
            wake_tx,
            pending: HashMap::new(),
            jobs: 0,
            seq: 0,
            next_poll: 0.0,
            poll_round: 0,
            watching: HashSet::new(),
            live: HashSet::new(),
            w: 0,
            h: 0,
            list_scroll: Scroller::new(),
            reader_scroll: Scroller::new(),
            press: None,
            labels: TextCache::default(),
            laid: None,
            dirty: true,
        };
        for account in &window.mail.accounts {
            match legacy.get(&account.id) {
                // a 0.3.0 password is read from its old keychain item; Passwords gets it from there
                Some(legacy_id) => {
                    let request = keychain::get(ctx, migrate::LEGACY_SERVICE, &migrate::legacy_name(legacy_id));
                    window.pending.insert(request, Pending::Migrate { account: account.clone(), legacy: legacy_id.clone() });
                }
                None => {
                    let request = passwords::fill(ctx, &account.password_site(), &account.username);
                    window.pending.insert(request, Pending::Load(account.id.clone()));
                }
            }
        }
        window
    }

    /// True while a listing, a message or a send is on its way.
    pub fn busy(&self) -> bool {
        self.jobs > 0
    }

    pub fn set_size(&mut self, w: u32, h: u32) {
        self.w = w;
        self.h = h;
        self.dirty = true;
    }

    pub fn wake_fd(&self) -> Option<i32> {
        self.wake_rx.as_ref().map(AsRawFd::as_raw_fd)
    }

    /// The next time the loop should come back: the next poll, or a frame while a scroll settles.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.list_scroll.animating() || self.reader_scroll.animating() {
            Some(t + 16.0)
        } else {
            Some(self.next_poll)
        }
    }

    /// Applies what the job threads and the poll timer produced. True when the screen changed.
    pub fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        if let Some(mut r) = self.wake_rx.as_ref() {
            let mut buf = [0u8; 64];
            while r.read(&mut buf).is_ok_and(|n| n > 0) {}
        }
        while let Ok(event) = self.rx.try_recv() {
            if event.is_job() {
                self.jobs = self.jobs.saturating_sub(1);
            }
            self.event(ctx, event);
            self.dirty = true;
        }
        if t >= self.next_poll {
            self.next_poll = t + POLL_MS;
            self.poll();
        }
        let scrolled = self.list_scroll.tick(t) | self.reader_scroll.tick(t);
        std::mem::take(&mut self.dirty) || scrolled
    }

    /// The answer to one of the requests the window made to Passwords. False when the request was not the window's.
    pub fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.pending.get(&request).is_some_and(Pending::from_keychain) {
            return false;
        }
        let Some(pending) = self.pending.remove(&request) else { return false };
        match pending {
            Pending::Migrate { .. } | Pending::Discard => {}
            Pending::Moved { account, legacy } => match passwords::decode_password(result) {
                Ok(_) => {
                    let request = keychain::delete(ctx, migrate::LEGACY_SERVICE, &migrate::legacy_name(&legacy));
                    self.pending.insert(request, Pending::Discard);
                }
                Err(KeyError::DuplicateItem) => {
                    self.mail.status = format!("Passwords already has a password for {}. The old one is kept", account.name);
                }
                Err(e) => self.mail.status = format!("The password could not be saved in Passwords: {e:?}"),
            },
            Pending::Load(id) => match passwords::decode_password(result) {
                Ok(password) => {
                    self.mail.passwords.insert(id.clone(), password);
                    self.refresh_account(&id);
                }
                Err(KeyError::ItemNotFound) => {
                    let name = self.account_name(&id);
                    self.mail.status = format!("No password is saved for {name} in Passwords. Add the account again");
                }
                Err(e) => self.mail.status = format!("Passwords could not return the password: {e:?}"),
            },
            Pending::Add { account, password } => match passwords::decode_password(result) {
                Ok(_) => self.account_saved(account, password),
                Err(KeyError::DuplicateItem) => {
                    let request = passwords::update(ctx, &account.password_site(), &account.username, &password);
                    self.pending.insert(request, Pending::Replace { account, password });
                }
                Err(e) => self.mail.status = format!("The password could not be saved in Passwords: {e:?}"),
            },
            Pending::Replace { account, password } => match passwords::decode_password(result) {
                Ok(_) => self.account_saved(account, password),
                Err(e) => self.mail.status = format!("The password could not be saved in Passwords: {e:?}"),
            },
        }
        self.dirty = true;
        true
    }

    /// The answer to one of the keychain calls for the 0.3.0 passwords. False when the request was not the window's.
    pub fn keychain(&mut self, ctx: &Ctx, reply: KeyReply) -> bool {
        if !self.pending.get(&reply.request).is_some_and(Pending::from_keychain) {
            return false;
        }
        let Some(pending) = self.pending.remove(&reply.request) else { return false };
        match pending {
            Pending::Migrate { account, legacy } => match reply.result {
                Ok(secret) => match String::from_utf8(secret) {
                    Ok(password) => self.moved(ctx, account, legacy, password),
                    Err(_) => self.mail.status = format!("The saved password of {} is not text", account.name),
                },
                // nothing in the old store: Passwords may hold it already (saved by a later run), so ask there
                Err(LegacyError::ItemNotFound) => {
                    let request = passwords::fill(ctx, &account.password_site(), &account.username);
                    self.pending.insert(request, Pending::Load(account.id.clone()));
                }
                Err(e) => self.mail.status = format!("The password of {} could not be read: {e:?}", account.name),
            },
            // the old item is gone or was refused; nothing else depends on it
            _ => {}
        }
        self.dirty = true;
        true
    }

    /// A password read from the 0.3.0 store: it is saved in Passwords under the account, and the account is listed
    /// with it now.
    fn moved(&mut self, ctx: &Ctx, account: Account, legacy: String, password: String) {
        let request = passwords::save(ctx, &account.password_site(), &account.username, &password);
        self.pending.insert(request, Pending::Moved { account: account.clone(), legacy });
        self.mail.passwords.insert(account.id.clone(), password);
        self.refresh_account(&account.id);
    }

    pub fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32, t: f64) -> bool {
        match kind {
            TouchKind::Down => {
                let region = self.region(&ctx.ui, x, y);
                if let Some(scroll) = self.scroller(region) {
                    scroll.down(y, t);
                }
                self.press = Some(Press { x, y, moved: false, region });
                false
            }
            TouchKind::Move => {
                let Some(press) = self.press else { return false };
                let far = (x - press.x).hypot(y - press.y) > ctx.ui.pt(DRAG_PT);
                if !(press.moved || far) {
                    return false;
                }
                let Some(scroll) = self.scroller(press.region) else { return false };
                scroll.moved(y, t);
                self.press = Some(Press { moved: true, ..press });
                true
            }
            TouchKind::Up | TouchKind::Cancel => {
                let Some(press) = self.press.take() else { return false };
                if press.moved {
                    if let Some(scroll) = self.scroller(press.region) {
                        scroll.up(y, t);
                    }
                    return true;
                }
                self.tap(ctx, x, y)
            }
        }
    }

    pub fn key(&mut self, ctx: &Ctx, k: KeyInput) -> bool {
        if let Some((key, mods)) = k.as_chord() {
            return key == KEY_V && mods & (CHORD_CTRL | CHORD_META) != 0 && self.paste();
        }
        if self.mail.sheet.is_none() {
            return false;
        }
        let multiline = self.focus_is_message();
        match k {
            KeyInput::Text(s) => {
                let text: String = s.chars().filter(|c| !c.is_control() || (*c == '\n' && multiline)).collect();
                if let Some(form) = self.form_mut() {
                    form.type_text(&text);
                }
            }
            KeyInput::Code(KEY_RETURN) if multiline => {
                if let Some(form) = self.form_mut() {
                    form.type_text("\n");
                }
            }
            KeyInput::Code(KEY_RETURN) => self.submit(ctx),
            KeyInput::Code(KEY_TAB) => {
                if let Some(form) = self.form_mut() {
                    form.next_field();
                }
            }
            KeyInput::Code(KEY_BACKSPACE) => {
                if let Some(form) = self.form_mut() {
                    form.backspace();
                }
            }
            KeyInput::Code(KEY_ESCAPE) => self.mail.close_sheet(),
            KeyInput::Code(_) => return false,
        }
        self.dirty = true;
        true
    }

    pub fn paste(&mut self) -> bool {
        let Some(text) = clipboard::text() else { return false };
        let multiline = self.focus_is_message();
        let text: String = text.chars().filter(|c| !c.is_control() || (*c == '\n' && multiline)).collect();
        match self.form_mut() {
            Some(form) => form.type_text(&text),
            None => return false,
        }
        self.dirty = true;
        true
    }

    pub fn menu_items(&self) -> Vec<AppMenu> {
        let compose_open = matches!(self.mail.sheet, Some(Sheet::Compose(_)));
        vec![
            AppMenu {
                title: "Message".into(),
                items: vec![
                    AppMenuItem::new(MENU_NEW, "New Message").key("cmd+n"),
                    AppMenuItem::new(MENU_REPLY, "Reply").key("cmd+shift+r").enabled(self.mail.open.is_some()),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_SEND, "Send").key("cmd+shift+d").enabled(compose_open),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_FLAG, "Flag").key("cmd+shift+l").enabled(self.mail.open.is_some()),
                    AppMenuItem::new(MENU_UNREAD, "Mark as Unread").enabled(self.mail.open.is_some()),
                    AppMenuItem::new(MENU_ARCHIVE, "Archive").key("cmd+shift+a").enabled(self.mail.open.is_some()),
                    AppMenuItem::new(MENU_TRASH, "Move to Trash").enabled(self.mail.open.is_some()),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_SAVE, "Save Attachments...").key("cmd+shift+s").enabled(self.open_has_files()),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_PRINT, "Print...").key("cmd+p").enabled(self.mail.open.is_some()),
                ],
            },
            AppMenu {
                title: "Mailbox".into(),
                items: vec![
                    AppMenuItem::new(MENU_REFRESH, "Refresh").key("cmd+r"),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_FIND, "Find...").key("cmd+f"),
                    AppMenuItem::new(MENU_SHOW_ALL, "Show All Messages").enabled(!self.mail.search.is_empty()),
                    AppMenuItem::new(MENU_THREADS, if self.mail.threaded { "Hide Threads" } else { "Show Threads" }).key("cmd+shift+t"),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_ADD, "Add Account..."),
                ],
            },
        ]
    }

    pub fn menu(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_NEW => self.compose(),
            MENU_REPLY => self.reply(),
            MENU_SEND => self.submit(ctx),
            MENU_REFRESH => self.refresh_selected(),
            MENU_ADD => self.mail.open_setup(),
            MENU_FLAG => self.toggle_flag(Flag::Flagged),
            MENU_UNREAD => self.toggle_flag(Flag::Seen),
            MENU_ARCHIVE => self.move_open(&ARCHIVE_NAMES, "Archive"),
            MENU_TRASH => self.move_open(&TRASH_NAMES, "Trash"),
            MENU_FIND => self.mail.open_search(),
            MENU_SAVE => self.save_attachments(ctx),
            MENU_SHOW_ALL => self.mail.apply_search(""),
            MENU_THREADS => {
                self.mail.threaded = !self.mail.threaded;
                self.list_scroll = Scroller::new();
            }
            MENU_PRINT => self.print_open(),
            _ => return false,
        }
        self.dirty = true;
        true
    }

    // flow -------------------------------------------------------------------------------------------------------

    fn event(&mut self, ctx: &Ctx, event: Event) {
        match event {
            Event::Listed { account, mailbox, result } => match result {
                Ok(listing) => {
                    let notices = self.mail.apply_listing(&account, &mailbox, listing.mailboxes, listing.messages);
                    for notice in notices {
                        self.notify(ctx, &notice);
                    }
                    if self.mail.current().is_some_and(|a| a.id == account) {
                        self.mail.status.clear();
                    }
                }
                Err(e) => self.mail.status = format!("{}: {e}", self.account_name(&account)),
            },
            Event::Changed { account, result, .. } => {
                // the change is shown at once; a refusal brings the listing back from the server
                if let Err(e) = result {
                    self.mail.status = format!("{}: {e}", self.account_name(&account));
                    self.refresh_account(&account);
                }
            }
            Event::Body { account, mailbox, uid, result } => match result {
                Ok(text) => self.mail.body_arrived(&account, &mailbox, uid, text),
                Err(e) => self.mail.status = format!("The message could not be loaded: {e}"),
            },
            Event::Found { account, mailbox, query, result } => match result {
                Ok(messages) => self.mail.apply_found(&account, &mailbox, &query, messages),
                Err(e) => self.mail.status = format!("{}: {e}", self.account_name(&account)),
            },
            Event::Watch { account, live } => {
                if live {
                    self.live.insert(account);
                } else {
                    self.live.remove(&account);
                }
            }
            Event::Arrived { account } => self.refresh_account(&account),
            Event::Sent { result, .. } => match result {
                Ok(()) => {
                    if matches!(self.mail.sheet, Some(Sheet::Compose(_))) {
                        self.mail.close_sheet();
                    }
                    self.mail.status = "Sent".into();
                }
                Err(e) => self.mail.status = format!("The message was not sent: {e}"),
            },
        }
    }

    fn notify(&self, ctx: &Ctx, notice: &Notice) {
        let who = notice.from.split('<').next().unwrap_or("").trim().trim_matches('"');
        let who = if who.is_empty() { notice.from.as_str() } else { who };
        let id = format!("mail-{}-{}", notice.account, notice.uid);
        let request = Request::new(&id, "New Mail").subtitle(who).body(&notice.subject).thread(&notice.account);
        let _ = usernotif::add(ctx, request);
    }

    /// The password is in Passwords: the account goes into the shared list and becomes the shown account.
    fn account_saved(&mut self, account: Account, password: String) {
        match accounts::add(account.clone()) {
            Ok(()) => {
                self.mail.add_account(account, password);
                self.mail.close_sheet();
                self.refresh_selected();
            }
            Err(e) => self.mail.status = e,
        }
    }

    fn account_name(&self, id: &str) -> String {
        self.mail.accounts.iter().find(|a| a.id == id).map_or_else(|| id.to_string(), |a| a.name.clone())
    }

    /// Starts a listing of `mailbox` for the account at `index`. Nothing happens until its password is known.
    fn refresh(&mut self, index: usize, mailbox: &str) {
        let Some(account) = self.mail.accounts.get(index).cloned() else { return };
        let Some(password) = self.mail.password(&account).map(str::to_string) else { return };
        if self.watching.insert(account.id.clone()) {
            sync::watch(self.transport.clone(), account.clone(), password.clone(), self.tx.clone(), self.wake_clone());
        }
        let transport = self.transport.clone();
        let mailbox = mailbox.to_string();
        self.jobs += 1;
        sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::list(transport.as_ref(), &account, &password, &mailbox));
    }

    fn refresh_account(&mut self, id: &str) {
        let Some(index) = self.mail.accounts.iter().position(|a| a.id == id) else { return };
        self.refresh(index, INBOX);
        if index == self.mail.account && self.mail.mailbox != INBOX {
            let mailbox = self.mail.mailbox.clone();
            self.refresh(index, &mailbox);
        }
    }

    fn refresh_selected(&mut self) {
        let index = self.mail.account;
        self.refresh(index, INBOX);
        let mailbox = self.mail.mailbox.clone();
        if mailbox != INBOX {
            self.refresh(index, &mailbox);
        }
    }

    /// The periodic check: the inbox of every account (for notices) and the shown mailbox. An account whose watcher is
    /// connected is left to the watcher, and checked here only every [`IDLE_CHECK_ROUNDS`] rounds.
    fn poll(&mut self) {
        self.poll_round = self.poll_round.wrapping_add(1);
        for index in 0..self.mail.accounts.len() {
            let live = self.live.contains(&self.mail.accounts[index].id);
            if live && self.poll_round % IDLE_CHECK_ROUNDS != 0 {
                continue;
            }
            self.refresh(index, INBOX);
        }
        let mailbox = self.mail.mailbox.clone();
        if mailbox != INBOX {
            let index = self.mail.account;
            self.refresh(index, &mailbox);
        }
    }

    /// Sets or clears a flag on the open message. The row changes at once; the server is told in the background.
    fn toggle_flag(&mut self, flag: Flag) {
        let Some(uid) = self.mail.open else { return };
        let Some(flags) = self.mail.flags_of(uid) else { return };
        let on = !flags.contains(flag);
        self.mail.set_flag_locally(uid, flag, on);
        self.change_message(uid, Change::Flag { flag, on });
    }

    /// Moves the open message to the account's Archive or Trash mailbox. The message leaves the list at once.
    fn move_open(&mut self, candidates: &[&str], label: &str) {
        let Some(uid) = self.mail.open else { return };
        let Some(to) = self.mail.find_mailbox(candidates) else {
            self.mail.status = format!("This account has no {label} mailbox");
            return;
        };
        self.mail.remove_row(uid);
        self.change_message(uid, Change::Move { to });
    }

    /// True when the open message carries files.
    fn open_has_files(&self) -> bool {
        self.mail.open.and_then(|uid| self.mail.attachments.get(&uid)).is_some_and(|files| !files.is_empty())
    }

    /// Writes the open message's files into the app's data directory, one folder per message.
    fn save_attachments(&mut self, ctx: &Ctx) {
        let Some(uid) = self.mail.open else { return };
        let dir = ctx.data_dir.join("attachments").join(uid.to_string());
        let result = match self.mail.attachments.get(&uid) {
            Some(files) if !files.is_empty() => attachments::save_all(&dir, files),
            _ => {
                self.mail.status = "This message has no attachments".into();
                return;
            }
        };
        self.mail.status = match result {
            Ok(n) => format!("Saved {n} attachment{} to {}", if n == 1 { "" } else { "s" }, dir.display()),
            Err(e) => format!("The attachments could not be saved: {e}"),
        };
    }

    /// Runs one change to a message of the shown mailbox on the server.
    fn change_message(&mut self, uid: u32, action: Change) {
        let Some(account) = self.mail.current().cloned() else { return };
        let Some(password) = self.mail.password(&account).map(str::to_string) else {
            self.mail.status = format!("No password is saved for {}", account.name);
            return;
        };
        let mailbox = self.mail.mailbox.clone();
        let transport = self.transport.clone();
        self.jobs += 1;
        sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::change(transport.as_ref(), &account, &password, &mailbox, uid, &action));
    }

    fn open_message(&mut self, uid: u32) {
        self.reader_scroll = Scroller::new();
        if !self.mail.open_message(uid) {
            return;
        }
        let Some(account) = self.mail.current().cloned() else { return };
        let Some(password) = self.mail.password(&account).map(str::to_string) else {
            self.mail.status = format!("No password is saved for {}", account.name);
            return;
        };
        let mailbox = self.mail.mailbox.clone();
        let transport = self.transport.clone();
        self.jobs += 1;
        sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::body(transport.as_ref(), &account, &password, &mailbox, uid));
    }

    fn compose(&mut self) {
        if self.mail.accounts.is_empty() {
            self.mail.open_setup();
        } else {
            self.mail.open_compose(None);
        }
    }

    /// Message > Print (Cmd+P): the open message's headers and decoded body as plain text. The job goes to CUPS on its own
    /// thread; nothing prints until the body has arrived.
    fn print_open(&self) {
        let Some(uid) = self.mail.open else { return };
        let Some(row) = self.mail.row(uid) else { return };
        let Some(raw) = self.mail.bodies.get(&uid) else { return };
        let s = &row.summary;
        let text = format!("From: {}\nTo: {}\nDate: {}\nSubject: {}\n\n{}", s.from, s.to, s.date, s.subject, crate::body::readable(raw));
        let title = if s.subject.is_empty() { "Message".to_string() } else { s.subject.clone() };
        let job = bifrost_app::print::PrintOperation::text(title, text);
        std::thread::spawn(move || {
            if let Err(e) = job.run() {
                eprintln!("Mail print: {e:?}");
            }
        });
    }

    fn reply(&mut self) {
        let Some(uid) = self.mail.open else { return };
        let original = self.mail.row(uid).map(|r| r.summary.clone());
        if let Some(original) = original {
            self.mail.open_compose(Some(&original));
        }
    }

    /// Runs the sheet's primary action: add the account, or send the message.
    fn submit(&mut self, ctx: &Ctx) {
        match self.mail.sheet.clone() {
            Some(Sheet::Setup(form)) => {
                let taken = match accounts::load_all() {
                    Ok(all) => all,
                    Err(e) => {
                        self.mail.status = e;
                        return;
                    }
                };
                match self.mail.account_from_form(&form, &taken) {
                    Ok((account, password)) => {
                        let request = passwords::save(ctx, &account.password_site(), &account.username, &password);
                        self.pending.insert(request, Pending::Add { account, password });
                        self.mail.status = "Saving the password".into();
                    }
                    Err(e) => self.mail.status = e,
                }
            }
            Some(Sheet::Compose(compose)) => self.send(&compose),
            Some(Sheet::Search(form)) => {
                let text = form.value("Search").to_string();
                self.mail.apply_search(&text);
                self.mail.close_sheet();
                self.search_server();
            }
            None => {}
        }
    }

    /// Asks the server to search the shown mailbox for the applied search, so messages beyond the loaded ones match too.
    /// The answer replaces the list when it arrives (see `Mail::apply_found`).
    fn search_server(&mut self) {
        let query = self.mail.search.clone();
        let Some(criteria) = Query::parse(&query).imap_criteria() else { return };
        let Some(account) = self.mail.current().cloned() else { return };
        let Some(password) = self.mail.password(&account).map(str::to_string) else { return };
        let mailbox = self.mail.mailbox.clone();
        let transport = self.transport.clone();
        self.jobs += 1;
        sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::search(transport.as_ref(), &account, &password, &mailbox, &query, &criteria));
    }

    fn send(&mut self, compose: &crate::state::Compose) {
        let draft = match self.mail.draft_from(compose) {
            Ok(draft) => draft,
            Err(e) => {
                self.mail.status = e;
                return;
            }
        };
        let Some(account) = self.mail.current().cloned() else {
            self.mail.status = "Add an account first".into();
            return;
        };
        let Some(password) = self.mail.password(&account).map(str::to_string) else {
            self.mail.status = format!("No password is saved for {}", account.name);
            return;
        };
        let unix = now_unix();
        let domain = account.email.rsplit_once('@').map_or("localhost", |(_, d)| d).to_string();
        self.seq += 1;
        let id = message::new_message_id(&domain, unix, self.seq);
        let text = match draft.render(&id, unix) {
            Ok(text) => text,
            Err(e) => {
                self.mail.status = e;
                return;
            }
        };
        self.mail.status = "Sending".into();
        self.jobs += 1;
        let transport = self.transport.clone();
        sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::send(transport, account, password, draft, text));
    }

    fn wake_clone(&self) -> Option<UnixStream> {
        self.wake_tx.as_ref().and_then(|w| w.try_clone().ok())
    }

    fn focus_is_message(&self) -> bool {
        match &self.mail.sheet {
            Some(Sheet::Compose(c)) => c.form.fields.get(c.form.focus).is_some_and(|f| f.label == "Message"),
            _ => false,
        }
    }

    fn form_mut(&mut self) -> Option<&mut crate::state::Form> {
        match self.mail.sheet.as_mut()? {
            Sheet::Setup(form) | Sheet::Search(form) => Some(form),
            Sheet::Compose(compose) => Some(&mut compose.form),
        }
    }

    /// The pane a point is in. With a sheet open everything is chrome, so presses never scroll the panes behind it.
    pub fn region(&self, ui: &Ui, x: f32, y: f32) -> Region {
        if self.mail.sheet.is_some() {
            return Region::Chrome;
        }
        crate::draw::pane_at(crate::draw::columns(ui, self.w as f32), ui.pt(crate::draw::TOOLBAR_PT), x, y)
    }

    fn scroller(&mut self, region: Region) -> Option<&mut Scroller> {
        match region {
            Region::List => Some(&mut self.list_scroll),
            Region::Reader => Some(&mut self.reader_scroll),
            Region::Chrome => None,
        }
    }

    fn tap(&mut self, ctx: &Ctx, x: f32, y: f32) -> bool {
        let hit = self.hit(&ctx.ui, x, y);
        match hit {
            crate::draw::Hit::Button(button) => match button {
                crate::draw::Button::Refresh => self.refresh_selected(),
                crate::draw::Button::Compose => self.compose(),
                crate::draw::Button::Reply => self.reply(),
                crate::draw::Button::Archive => self.move_open(&ARCHIVE_NAMES, "Archive"),
                crate::draw::Button::Trash => self.move_open(&TRASH_NAMES, "Trash"),
            },
            crate::draw::Hit::Side(side) => match side {
                crate::draw::Side::Account(index) => {
                    self.mail.show_account(index);
                    self.refresh_selected();
                }
                crate::draw::Side::Mailbox(name) => {
                    self.mail.show_mailbox(&name);
                    self.list_scroll = Scroller::new();
                    self.refresh_selected();
                }
                crate::draw::Side::Add => self.mail.open_setup(),
            },
            crate::draw::Hit::Message(uid) => self.open_message(uid),
            crate::draw::Hit::Field(index) => {
                if let Some(form) = self.form_mut() {
                    form.focus_on(index);
                }
            }
            crate::draw::Hit::Cancel => self.mail.close_sheet(),
            crate::draw::Hit::Primary => self.submit(ctx),
            crate::draw::Hit::None => return false,
        }
        self.dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_send_needs_the_compose_sheet_and_a_recipient() {
        let mut mail = Mail::new(Vec::new());
        mail.open_compose(None);
        assert!(matches!(mail.sheet, Some(Sheet::Compose(_))));
        assert_eq!(mail.draft_from(&match &mail.sheet {
            Some(Sheet::Compose(c)) => c.clone(),
            _ => unreachable!(),
        }), Err("Add an account first".to_string()));
    }
}
