//! The Mail window: the state in [`crate::state`] wired to the network jobs ([`crate::sync`]), the shared Internet
//! Accounts list and Passwords for the logins, and the notifications for new mail. This file holds the flow and the
//! input; drawing and hit-testing are in [`crate::draw`].

use crate::accounts;
use crate::attachments;
use crate::body::Attachment;
use crate::category::Category;
use crate::draw::Rect;
use crate::findbar::{self, FindBar};
use crate::flags::{Flag, FlagColor};
use crate::mbox;
use crate::message::{self, Priority, MAX_ATTACHMENT_BYTES};
use crate::migrate;
use crate::mime;
use crate::people::{self, Person};
use crate::prefs;
use crate::rules::{self, Action};
use crate::schedule::{self, Queued, Reminder};
use crate::search::Query;
use crate::state::{is_multiline, Compose, Filter, Mail, Notice, Prompt, PromptKind, Sheet, SortKey, ARCHIVE_NAMES, INBOX, JUNK_NAMES, TRASH_NAMES};
use crate::sync::{self, Change, Event};
use crate::transport;
use bifrost_app::contacts::{self, Key as ContactKey, SaveRequest};
use bifrost_app::controls::internet_accounts::Account;
use bifrost_app::keychain::{self, Error as LegacyError, Reply as KeyReply};
use bifrost_app::kit::{clipboard, Scroller, TextCache, Ui};
use bifrost_app::passwords::{self, Error as KeyError};
use bifrost_app::usernotif::{self, Request};
use bifrost_app::{AppMenu, AppMenuItem, Ctx, DocumentEvent, KeyInput, OpenOptions, TouchKind};
use bifrost_app::proto::{DocErrorCode, DocMode, CHORD_CTRL, CHORD_META};
use bifrost_core::json::Json;
use springboard::deildu::http::Transport;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
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
pub const MENU_REPLY_ALL: u32 = 15;
pub const MENU_FORWARD: u32 = 16;
pub const MENU_JUNK: u32 = 17;
pub const MENU_MOVE_AGAIN: u32 = 18;
pub const MENU_EXPAND_ALL: u32 = 19;
pub const MENU_COLLAPSE_ALL: u32 = 20;
pub const MENU_TOGGLE_CONVERSATION: u32 = 21;
pub const MENU_SORT_ASC: u32 = 22;
pub const MENU_SORT_DESC: u32 = 23;
pub const MENU_NEW_MAILBOX: u32 = 24;
pub const MENU_RENAME_MAILBOX: u32 = 25;
pub const MENU_DELETE_MAILBOX: u32 = 26;
pub const MENU_ERASE_TRASH: u32 = 27;
pub const MENU_ERASE_JUNK: u32 = 28;
pub const MENU_PASTE_QUOTE: u32 = 29;
pub const MENU_SHOW_CC: u32 = 30;
pub const MENU_SHOW_BCC: u32 = 31;
pub const MENU_SHOW_REPLY_TO: u32 = 32;
pub const MENU_MUTE: u32 = 33;
pub const MENU_SHOW_SIDEBAR: u32 = 34;
pub const MENU_RAW_SOURCE: u32 = 35;
pub const MENU_SAVE_AS: u32 = 36;
pub const MENU_QUICKLY: u32 = 37;
pub const MENU_CLEAR_FLAG: u32 = 38;
pub const MENU_ATTACH: u32 = 39;
pub const MENU_REMOVE_ATTACHMENTS: u32 = 40;
pub const MENU_ADD_TO_CONTACTS: u32 = 41;
pub const MENU_FORWARD_ATTACHMENT: u32 = 42;
pub const MENU_REDIRECT: u32 = 43;
pub const MENU_SEND_LATER: u32 = 44;
pub const MENU_UNDO_SEND: u32 = 45;
pub const MENU_REMIND: u32 = 46;
pub const MENU_FIND_IN_MESSAGE: u32 = 47;
pub const MENU_FIND_NEXT: u32 = 48;
pub const MENU_FIND_PREVIOUS: u32 = 49;
pub const MENU_IMPORT_MBOX: u32 = 50;
pub const MENU_EXPORT_MBOX: u32 = 51;
pub const MENU_NEW_SMART: u32 = 52;
pub const MENU_DELETE_SMART: u32 = 53;
pub const MENU_NEW_RULE: u32 = 54;
pub const MENU_APPLY_RULES: u32 = 55;
pub const MENU_SETTINGS: u32 = 56;
pub const MENU_ACCOUNT_SETTINGS: u32 = 57;
pub const MENU_REMOVE_ACCOUNT: u32 = 58;
pub const MENU_FAVORITE: u32 = 59;
pub const MENU_SHOW_FAVORITES: u32 = 60;
pub const MENU_SEND_QUEUED: u32 = 61;
pub const MENU_ALL_CATEGORIES: u32 = 62;
/// Menu ids that run from a base: the nth entry of a list (mailboxes, sort keys, filters, priorities) is `base + n`.
pub const MENU_MOVE_BASE: u32 = 100;
pub const MENU_COPY_BASE: u32 = 200;
pub const MENU_SORT_BASE: u32 = 300;
pub const MENU_FILTER_BASE: u32 = 400;
pub const MENU_PRIORITY_BASE: u32 = 500;
pub const MENU_COLOR_BASE: u32 = 600;
pub const MENU_CATEGORY_BASE: u32 = 700;
pub const MENU_RULE_BASE: u32 = 800;
/// The room each id range has.
pub const MENU_TARGET_LIMIT: usize = 99;
/// Most mailboxes a Move To or Copy To submenu lists. An app menu holds 48 items, and the Move menu has two such
/// submenus besides its own entries.
pub const MENU_TARGETS_SHOWN: usize = 22;
/// Most rules Mail keeps: each is listed in the Rules submenu so it can be deleted.
pub const MAX_RULES: usize = 12;
/// Largest mailbox file Import Mailbox reads, in bytes.
pub const IMPORT_LIMIT: usize = 64 * 1024 * 1024;

/// The position of menu id `id` in the list that starts at `base`, when it falls inside the range.
pub fn menu_index(id: u32, base: u32) -> Option<usize> {
    id.checked_sub(base).map(|i| i as usize).filter(|i| *i < MENU_TARGET_LIMIT)
}

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
    /// A new account's password, being added. `Replace` follows when an item already exists. `old` is the account being
    /// edited, when this is Account Settings and not a new account.
    Add { account: Account, password: String, old: Option<Account> },
    Replace { account: Account, password: String, old: Option<Account> },
    /// Add Sender to Contacts: the address book is asked whether it has the address, and then given the person.
    ContactCheck(Person),
    ContactSave(Person),
    /// A password item being deleted. Nothing follows its answer.
    Forget,
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

/// What a file panel was opened for.
enum DocTask {
    /// Message > Attach Files: the files chosen go on the message being written.
    Attach,
    /// A save panel: the bytes are written to the place chosen. The name is for the status line.
    Save { name: String, bytes: Vec<u8> },
    /// Mailbox > Import Mailbox: the file chosen is read as a mailbox file.
    ImportMbox,
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
    /// Where the muted conversations are kept (see [`crate::mute`]).
    muted_path: std::path::PathBuf,
    /// Where the settings and the outbox are kept (see [`crate::prefs`] and [`crate::schedule`]).
    prefs_path: std::path::PathBuf,
    queue_path: std::path::PathBuf,
    /// The data directory, for the files Mail saves when the file panel is not available.
    data_dir: std::path::PathBuf,
    /// File panels that are open, by request.
    docs: HashMap<u32, DocTask>,
    /// Files still waiting for their save panel: Save Attachments asks for each file's place in turn.
    save_queue: VecDeque<(String, Vec<u8>)>,
    /// The last message sent with an undo-send delay: its place in the outbox and what the sheet held.
    undo: Option<u64>,
    drafts: HashMap<u64, Compose>,
    /// Queued messages out for sending.
    in_flight: HashMap<u64, Queued>,
    /// The files named in the open message, where they were drawn last frame, by place in the message.
    pub attach_rects: Vec<(Rect, usize)>,
    /// The Previous, Next and Done buttons of the find bar where they were drawn last frame.
    pub find_hits: Option<[Rect; 3]>,
    /// Where the open message's text starts in the reader's scrolled content, the height of a line, and the height of the
    /// reader, as drawn last frame (for scrolling to a match).
    pub reader_metrics: (f32, f32, f32),
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
        let muted_path = ctx.data_dir.join(crate::mute::FILE);
        mail.muted = crate::mute::load(&muted_path);
        let prefs_path = ctx.data_dir.join(prefs::FILE);
        let queue_path = ctx.data_dir.join(schedule::FILE);
        mail.use_prefs(prefs::load(&prefs_path));
        mail.queue = schedule::load(&queue_path);
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
            muted_path,
            prefs_path,
            queue_path,
            data_dir: ctx.data_dir.clone(),
            docs: HashMap::new(),
            save_queue: VecDeque::new(),
            undo: None,
            drafts: HashMap::new(),
            in_flight: HashMap::new(),
            attach_rects: Vec::new(),
            find_hits: None,
            reader_metrics: (0.0, 0.0, 0.0),
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

    /// The next time the loop should come back: the next poll or the next queued message or reminder, or a frame while a
    /// scroll settles.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.list_scroll.animating() || self.reader_scroll.animating() {
            return Some(t + 16.0);
        }
        let mut wake = self.next_poll;
        if let Some(due) = self.mail.queue.next_due() {
            wake = wake.min(t + (due - now_unix()).max(0) as f64 * 1000.0);
        }
        Some(wake)
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
        self.run_queue(ctx);
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
            Pending::Add { account, password, old } => match passwords::decode_password(result) {
                Ok(_) => self.account_saved(ctx, account, Some(password), old),
                Err(KeyError::DuplicateItem) => {
                    let request = passwords::update(ctx, &account.password_site(), &account.username, &password);
                    self.pending.insert(request, Pending::Replace { account, password, old });
                }
                Err(e) => self.mail.status = format!("The password could not be saved in Passwords: {e:?}"),
            },
            Pending::Replace { account, password, old } => match passwords::decode_password(result) {
                Ok(_) => self.account_saved(ctx, account, Some(password), old),
                Err(e) => self.mail.status = format!("The password could not be saved in Passwords: {e:?}"),
            },
            Pending::Forget => {}
            Pending::ContactCheck(person) => match result {
                Ok(body) => {
                    let known = contacts::decode_contacts(body).unwrap_or_default().iter().any(|record| people::has_address(record, &person.address));
                    if known {
                        self.mail.status = format!("{} is already in Contacts", person.address);
                    } else {
                        let request = contacts::save(ctx, &SaveRequest { add: vec![person.record()], ..SaveRequest::default() });
                        self.pending.insert(request, Pending::ContactSave(person));
                    }
                }
                Err(e) => self.mail.status = format!("Contacts: {e}"),
            },
            Pending::ContactSave(person) => {
                self.mail.status = match result {
                    Ok(_) => format!("Added {} to Contacts", person.address),
                    Err(e) => format!("The contact could not be saved: {e}"),
                };
            }
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
            return self.find_key(k);
        }
        let multiline = self.focus_is_multiline();
        let picker = self.focus_is_picker();
        match k {
            KeyInput::Text(s) => {
                let text: String = s.chars().filter(|c| !c.is_control() || (*c == '\n' && multiline)).collect();
                if let Some(form) = self.form_mut().filter(|_| !picker) {
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
                if let Some(form) = self.form_mut().filter(|_| !picker) {
                    form.backspace();
                }
            }
            KeyInput::Code(KEY_ESCAPE) => self.mail.close_sheet(),
            KeyInput::Code(_) => return false,
        }
        self.dirty = true;
        true
    }

    /// Keys for the find bar over the reader, when no sheet is open: text goes to the search, Return steps to the next
    /// match, Escape closes the bar. False when the bar is not open (the key is not Mail's).
    fn find_key(&mut self, k: KeyInput) -> bool {
        if self.mail.find.is_none() || self.mail.open.is_none() {
            return false;
        }
        match k {
            KeyInput::Text(s) => {
                let text: String = s.chars().filter(|c| !c.is_control()).collect();
                if let Some(bar) = self.mail.find.as_mut() {
                    bar.type_text(&text);
                }
                self.scroll_to_match();
            }
            KeyInput::Code(KEY_BACKSPACE) => {
                if let Some(bar) = self.mail.find.as_mut() {
                    bar.backspace();
                }
                self.scroll_to_match();
            }
            KeyInput::Code(KEY_RETURN) => self.step_match(true),
            KeyInput::Code(KEY_ESCAPE) => self.mail.find = None,
            KeyInput::Code(_) => return false,
        }
        self.dirty = true;
        true
    }

    /// Every match of the find bar's text in the open message as drawn.
    fn find_matches(&self) -> Vec<findbar::Span> {
        match (&self.mail.find, &self.laid) {
            (Some(bar), Some(laid)) if self.mail.open == Some(laid.uid) => findbar::find_all(&laid.lines, &bar.text),
            _ => Vec::new(),
        }
    }

    /// Edit > Find Next and Find Previous, and the bar's buttons.
    fn step_match(&mut self, forward: bool) {
        let total = self.find_matches().len();
        if let Some(bar) = self.mail.find.as_mut() {
            if forward {
                bar.next(total);
            } else {
                bar.previous(total);
            }
        }
        self.scroll_to_match();
    }

    /// Scrolls the reader so the current match of the find bar is in view, a third of the way down.
    fn scroll_to_match(&mut self) {
        let matches = self.find_matches();
        let Some(bar) = &self.mail.find else { return };
        let Some(span) = matches.get(bar.current.min(matches.len().saturating_sub(1))) else { return };
        let (origin, line_h, area) = self.reader_metrics;
        let position = origin + span.line as f32 * line_h;
        self.reader_scroll.jump_to((position - area / 3.0).max(0.0));
    }

    pub fn paste(&mut self) -> bool {
        let Some(text) = clipboard::text() else { return false };
        if self.mail.sheet.is_none() {
            // pasting into the find bar
            let Some(bar) = self.mail.find.as_mut().filter(|_| self.mail.open.is_some()) else { return false };
            let text: String = text.chars().filter(|c| !c.is_control()).collect();
            bar.type_text(&text);
            self.scroll_to_match();
            self.dirty = true;
            return true;
        }
        let multiline = self.focus_is_multiline();
        let text: String = text.chars().filter(|c| !c.is_control() || (*c == '\n' && multiline)).collect();
        if self.focus_is_picker() {
            return false;
        }
        match self.form_mut() {
            Some(form) => form.type_text(&text),
            None => return false,
        }
        self.dirty = true;
        true
    }

    pub fn menu_items(&self) -> Vec<AppMenu> {
        let mail = &self.mail;
        let compose_open = matches!(mail.sheet, Some(Sheet::Compose(_)));
        let open = mail.open.is_some();
        let loaded = open && self.open_body_loaded();
        let targets = mail.move_targets();
        let shown_targets = &targets[..targets.len().min(MENU_TARGETS_SHOWN)];
        let mut message = vec![
            AppMenuItem::new(MENU_NEW, "New Message").key("cmd+n"),
            AppMenuItem::new(MENU_REPLY, "Reply").key("cmd+shift+r").enabled(open),
            AppMenuItem::new(MENU_REPLY_ALL, "Reply All").key("cmd+alt+r").enabled(open),
            AppMenuItem::new(MENU_FORWARD, "Forward").key("cmd+shift+f").enabled(loaded),
            AppMenuItem::new(MENU_FORWARD_ATTACHMENT, "Forward as Attachment").enabled(loaded),
            AppMenuItem::new(MENU_REDIRECT, "Redirect...").enabled(loaded),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_SEND, "Send").key("cmd+shift+d").enabled(compose_open),
            AppMenuItem::new(MENU_SEND_LATER, "Send Later").enabled(compose_open).checked(mail.send_later_shown()),
            AppMenuItem::new(MENU_UNDO_SEND, "Undo Send").enabled(self.undo_pending()),
            AppMenuItem::submenu("Set Priority").enabled(compose_open),
        ];
        let current = match &mail.sheet {
            Some(Sheet::Compose(c)) => Some(c.priority),
            _ => None,
        };
        for (i, priority) in Priority::ALL.into_iter().enumerate() {
            message.push(AppMenuItem::new(MENU_PRIORITY_BASE + i as u32, priority.label()).depth(1).checked(current == Some(priority)));
        }
        message.extend([
            AppMenuItem::new(MENU_ATTACH, "Attach Files...").key("cmd+shift+e").enabled(compose_open),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_FLAG, "Flag").key("cmd+shift+l").enabled(open),
            AppMenuItem::submenu("Flag Color").enabled(open),
        ]);
        let flagged = mail.open.and_then(|uid| mail.flags_of(uid)).and_then(|f| f.color());
        for (i, color) in FlagColor::ALL.into_iter().enumerate() {
            message.push(AppMenuItem::new(MENU_COLOR_BASE + i as u32, color.label()).depth(1).checked(flagged == Some(color)).enabled(open));
        }
        message.push(AppMenuItem::new(MENU_CLEAR_FLAG, "Clear Flag").depth(1).enabled(open && flagged.is_some()));
        let muted = mail.open.is_some_and(|uid| mail.is_muted(uid));
        message.extend([
            AppMenuItem::new(MENU_UNREAD, "Mark as Unread").enabled(open),
            AppMenuItem::new(MENU_MUTE, if muted { "Unmute" } else { "Mute" }).enabled(open),
            AppMenuItem::new(MENU_REMIND, "Remind Me...").enabled(open),
            AppMenuItem::new(MENU_ARCHIVE, "Archive").key("cmd+shift+a").enabled(open),
            AppMenuItem::new(MENU_TRASH, "Move to Trash").enabled(open),
            AppMenuItem::new(MENU_JUNK, "Move to Junk").key("cmd+shift+j").enabled(open),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_ADD_TO_CONTACTS, "Add Sender to Contacts").enabled(open),
            AppMenuItem::new(MENU_REMOVE_ATTACHMENTS, "Remove Attachments").enabled(self.open_has_files()),
            AppMenuItem::new(MENU_SAVE, "Save Attachments...").key("cmd+alt+a").enabled(self.open_has_files()),
            AppMenuItem::new(MENU_SAVE_AS, "Save As...").key("cmd+shift+s").enabled(loaded),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_PRINT, "Print...").key("cmd+p").enabled(open),
        ]);

        // Move To and Copy To list the account's mailboxes, so they have a menu of their own: one menu holds 48 items
        let mut moves = vec![AppMenuItem::submenu("Move To").enabled(open)];
        for (i, name) in shown_targets.iter().enumerate() {
            moves.push(AppMenuItem::new(MENU_MOVE_BASE + i as u32, name).depth(1));
        }
        moves.push(AppMenuItem::submenu("Copy To").enabled(open));
        for (i, name) in shown_targets.iter().enumerate() {
            moves.push(AppMenuItem::new(MENU_COPY_BASE + i as u32, name).depth(1));
        }
        let again = mail.last_target.as_ref().filter(|t| targets.contains(t));
        let again_title = again.map_or_else(|| "Move Again".to_string(), |t| format!("Move Again to {t}"));
        moves.push(AppMenuItem::new(MENU_MOVE_AGAIN, &again_title).enabled(open && again.is_some()));

        let editable = !mail.mailbox.eq_ignore_ascii_case(INBOX);
        let account_id = mail.current().map(|a| a.id.as_str()).unwrap_or("");
        let pinned = mail.prefs.is_favorite(account_id, &mail.mailbox);
        let mut mailbox = vec![
            AppMenuItem::new(MENU_REFRESH, "Refresh").key("cmd+r"),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_FIND, "Find...").key("cmd+f"),
            AppMenuItem::new(MENU_SHOW_ALL, "Show All Messages").enabled(!mail.search.is_empty()),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_QUICKLY, "Open Quickly...").key("cmd+shift+o").enabled(!mail.accounts.is_empty()),
            AppMenuItem::new(MENU_FAVORITE, if pinned { "Remove from Favorites Bar" } else { "Add to Favorites Bar" }).enabled(!mail.accounts.is_empty()),
            AppMenuItem::new(MENU_NEW_SMART, "New Smart Mailbox...").enabled(!mail.accounts.is_empty()),
            AppMenuItem::new(MENU_DELETE_SMART, "Delete Smart Mailbox").enabled(mail.smart_active.is_some()),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_NEW_MAILBOX, "New Mailbox...").enabled(!mail.accounts.is_empty()),
            AppMenuItem::new(MENU_RENAME_MAILBOX, "Rename Mailbox...").enabled(editable),
            AppMenuItem::new(MENU_DELETE_MAILBOX, "Delete Mailbox").enabled(editable),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_ERASE_TRASH, "Erase Deleted Items").enabled(mail.find_mailbox(&TRASH_NAMES).is_some()),
            AppMenuItem::new(MENU_ERASE_JUNK, "Erase Junk Mail").enabled(mail.find_mailbox(&JUNK_NAMES).is_some()),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_IMPORT_MBOX, "Import Mailbox...").enabled(!mail.accounts.is_empty()),
            AppMenuItem::new(MENU_EXPORT_MBOX, "Export Mailbox...").enabled(!mail.accounts.is_empty()),
            AppMenuItem::separator(),
            AppMenuItem::submenu("Rules"),
            AppMenuItem::new(MENU_NEW_RULE, "New Rule...").depth(1).enabled(!mail.accounts.is_empty() && mail.prefs.rules.len() < MAX_RULES),
            AppMenuItem::new(MENU_APPLY_RULES, "Apply Rules").depth(1).enabled(!mail.prefs.rules.is_empty()),
        ];
        for (i, rule) in mail.prefs.rules.iter().enumerate().take(MAX_RULES) {
            mailbox.push(AppMenuItem::new(MENU_RULE_BASE + i as u32, &format!("Delete Rule: {}", rule.name)).depth(1));
        }
        mailbox.extend([
            AppMenuItem::new(MENU_SEND_QUEUED, "Send Queued Messages Now").enabled(!mail.queue.outbox.is_empty()),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_SETTINGS, "Mail Settings...").enabled(!mail.accounts.is_empty()),
            AppMenuItem::new(MENU_ACCOUNT_SETTINGS, "Account Settings...").enabled(!mail.accounts.is_empty()),
            AppMenuItem::new(MENU_REMOVE_ACCOUNT, "Remove Account...").enabled(!mail.accounts.is_empty()),
            AppMenuItem::new(MENU_ADD, "Add Account..."),
        ]);

        let mut view = vec![
            AppMenuItem::new(MENU_SHOW_SIDEBAR, if mail.show_sidebar { "Hide Sidebar" } else { "Show Sidebar" }).key("cmd+ctrl+s"),
            AppMenuItem::new(MENU_SHOW_FAVORITES, if mail.prefs.show_favorites { "Hide Favorites Bar" } else { "Show Favorites Bar" }),
            AppMenuItem::new(MENU_RAW_SOURCE, "Raw Source").key("cmd+alt+u").checked(mail.show_raw).enabled(open),
            AppMenuItem::separator(),
            AppMenuItem::submenu("Sort By"),
        ];
        for (i, key) in SortKey::ALL.into_iter().enumerate() {
            view.push(AppMenuItem::new(MENU_SORT_BASE + i as u32, key.label()).depth(1).checked(mail.sort == key));
        }
        view.push(AppMenuItem::new(MENU_SORT_ASC, "Ascending").depth(1).checked(mail.ascending));
        view.push(AppMenuItem::new(MENU_SORT_DESC, "Descending").depth(1).checked(!mail.ascending));
        view.push(AppMenuItem::submenu("Filter"));
        for (i, filter) in Filter::ALL.into_iter().enumerate() {
            view.push(AppMenuItem::new(MENU_FILTER_BASE + i as u32, filter.label()).depth(1).checked(mail.filter == filter));
        }
        view.push(AppMenuItem::submenu("Category"));
        view.push(AppMenuItem::new(MENU_ALL_CATEGORIES, "All Categories").depth(1).checked(mail.category.is_none()));
        for (i, category) in Category::ALL.into_iter().enumerate() {
            view.push(AppMenuItem::new(MENU_CATEGORY_BASE + i as u32, category.label()).depth(1).checked(mail.category == Some(category)));
        }
        let fold_title = if mail.threaded && mail.open.is_some_and(|uid| mail.is_folded(uid)) { "Expand Conversation" } else { "Collapse Conversation" };
        view.extend([
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_THREADS, if mail.threaded { "Hide Threads" } else { "Show Threads" }).key("cmd+shift+t"),
            AppMenuItem::new(MENU_TOGGLE_CONVERSATION, fold_title).enabled(mail.threaded && open),
            AppMenuItem::new(MENU_EXPAND_ALL, "Expand All Conversations").enabled(mail.threaded),
            AppMenuItem::new(MENU_COLLAPSE_ALL, "Collapse All Conversations").enabled(mail.threaded),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_SHOW_CC, "Cc Address Field").checked(mail.show_cc),
            AppMenuItem::new(MENU_SHOW_BCC, "Bcc Address Field").checked(mail.show_bcc),
            AppMenuItem::new(MENU_SHOW_REPLY_TO, "Reply-To Address Field").checked(mail.show_reply_to),
        ]);

        let edit = vec![
            AppMenuItem::new(MENU_PASTE_QUOTE, "Paste as Quotation").key("cmd+alt+v").enabled(compose_open),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_FIND_IN_MESSAGE, "Find in Message...").key("cmd+alt+f").enabled(open),
            AppMenuItem::new(MENU_FIND_NEXT, "Find Next").key("cmd+g").enabled(mail.find.is_some()),
            AppMenuItem::new(MENU_FIND_PREVIOUS, "Find Previous").key("cmd+shift+g").enabled(mail.find.is_some()),
        ];

        vec![
            AppMenu { title: "Message".into(), items: message },
            AppMenu { title: "Move".into(), items: moves },
            AppMenu { title: "Mailbox".into(), items: mailbox },
            AppMenu { title: "View".into(), items: view },
            AppMenu { title: "Edit".into(), items: edit },
        ]
    }

    pub fn menu(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_NEW => self.compose(),
            MENU_REPLY => self.reply(),
            MENU_REPLY_ALL => self.reply_all(),
            MENU_FORWARD => self.forward(),
            MENU_FORWARD_ATTACHMENT => self.forward_attachment(),
            MENU_REDIRECT => self.redirect(),
            MENU_SEND => self.submit(ctx),
            MENU_SEND_LATER => {
                let shown = self.mail.send_later_shown();
                self.mail.set_send_later(!shown);
            }
            MENU_UNDO_SEND => self.undo_send(),
            MENU_ATTACH => self.pick_files(ctx),
            MENU_REFRESH => self.refresh_selected(),
            MENU_ADD => self.mail.open_setup(),
            MENU_FLAG => self.toggle_flag(Flag::Flagged),
            MENU_UNREAD => self.toggle_flag(Flag::Seen),
            MENU_ARCHIVE => self.move_open(&ARCHIVE_NAMES, "Archive"),
            MENU_TRASH => self.move_open(&TRASH_NAMES, "Trash"),
            MENU_JUNK => self.junk_open(),
            MENU_MOVE_AGAIN => {
                if let Some(target) = self.mail.last_target.clone() {
                    self.move_open_to(&target);
                }
            }
            MENU_FIND => self.mail.open_search(),
            MENU_SAVE => self.save_attachments(ctx),
            MENU_SHOW_ALL => self.mail.apply_search(""),
            MENU_THREADS => {
                self.mail.threaded = !self.mail.threaded;
                self.list_scroll = Scroller::new();
            }
            MENU_PRINT => self.print_open(),
            MENU_NEW_MAILBOX => self.mail.open_new_mailbox(),
            MENU_RENAME_MAILBOX => {
                let _ = self.mail.open_rename_mailbox();
            }
            MENU_DELETE_MAILBOX => self.delete_mailbox(),
            MENU_ERASE_TRASH => self.erase(&TRASH_NAMES, "Deleted Items"),
            MENU_ERASE_JUNK => self.erase(&JUNK_NAMES, "Junk"),
            MENU_SORT_ASC => self.mail.set_ascending(true),
            MENU_SORT_DESC => self.mail.set_ascending(false),
            MENU_TOGGLE_CONVERSATION => {
                if let Some(uid) = self.mail.open {
                    self.mail.toggle_conversation(uid);
                }
            }
            MENU_EXPAND_ALL => self.mail.expand_all(),
            MENU_COLLAPSE_ALL => self.mail.collapse_all(),
            MENU_SHOW_CC => self.mail.set_address_field("Cc", !self.mail.show_cc),
            MENU_SHOW_BCC => self.mail.set_address_field("Bcc", !self.mail.show_bcc),
            MENU_SHOW_REPLY_TO => self.mail.set_address_field("Reply-To", !self.mail.show_reply_to),
            MENU_PASTE_QUOTE => return self.paste_quoted(),
            MENU_MUTE => self.toggle_mute(),
            MENU_SHOW_SIDEBAR => self.mail.show_sidebar = !self.mail.show_sidebar,
            MENU_RAW_SOURCE => {
                self.mail.show_raw = !self.mail.show_raw;
                self.laid = None;
                self.reader_scroll = Scroller::new();
            }
            MENU_SAVE_AS => self.save_open_as(ctx),
            MENU_QUICKLY => self.mail.open_quickly(),
            MENU_CLEAR_FLAG => self.set_color_open(None),
            MENU_REMOVE_ATTACHMENTS => self.remove_attachments(),
            MENU_ADD_TO_CONTACTS => self.add_sender_to_contacts(ctx),
            MENU_REMIND => {
                if self.sheet_free() {
                    if let Err(e) = self.mail.open_remind() {
                        self.mail.status = e;
                    }
                }
            }
            MENU_FIND_IN_MESSAGE => {
                if self.mail.open.is_some() {
                    if self.mail.find.is_none() {
                        self.mail.find = Some(FindBar::default());
                    }
                } else {
                    self.mail.status = "Open a message to find in it".into();
                }
            }
            MENU_FIND_NEXT => self.step_match(true),
            MENU_FIND_PREVIOUS => self.step_match(false),
            MENU_IMPORT_MBOX => self.import_mailbox(ctx),
            MENU_EXPORT_MBOX => self.export_mailbox(),
            MENU_NEW_SMART => {
                if self.sheet_free() {
                    self.mail.open_smart_mailbox();
                }
            }
            MENU_DELETE_SMART => self.delete_smart(),
            MENU_NEW_RULE => {
                if self.sheet_free() {
                    self.mail.open_rule();
                }
            }
            MENU_APPLY_RULES => self.apply_rules_to_shown(),
            MENU_SETTINGS => {
                if self.sheet_free() {
                    if let Err(e) = self.mail.open_settings() {
                        self.mail.status = e;
                    }
                }
            }
            MENU_ACCOUNT_SETTINGS => {
                if self.sheet_free() {
                    if let Err(e) = self.mail.open_edit_account() {
                        self.mail.status = e;
                    }
                }
            }
            MENU_REMOVE_ACCOUNT => {
                if self.sheet_free() {
                    if let Err(e) = self.mail.open_remove_account() {
                        self.mail.status = e;
                    }
                }
            }
            MENU_FAVORITE => self.toggle_favorite(),
            MENU_SHOW_FAVORITES => {
                self.mail.prefs.show_favorites = !self.mail.prefs.show_favorites;
                self.save_prefs();
            }
            MENU_SEND_QUEUED => self.send_queued_now(),
            MENU_ALL_CATEGORIES => self.mail.category = None,
            _ => {
                if let Some(i) = menu_index(id, MENU_MOVE_BASE) {
                    let target = self.mail.move_targets().get(i).cloned();
                    match target {
                        Some(target) => self.move_open_to(&target),
                        None => return false,
                    }
                } else if let Some(i) = menu_index(id, MENU_COPY_BASE) {
                    let target = self.mail.move_targets().get(i).cloned();
                    match target {
                        Some(target) => self.copy_open_to(&target),
                        None => return false,
                    }
                } else if let Some(i) = menu_index(id, MENU_SORT_BASE) {
                    match SortKey::ALL.get(i) {
                        Some(key) => self.mail.sort_by(*key),
                        None => return false,
                    }
                } else if let Some(i) = menu_index(id, MENU_FILTER_BASE) {
                    match Filter::ALL.get(i) {
                        Some(filter) => self.mail.filter = *filter,
                        None => return false,
                    }
                } else if let Some(i) = menu_index(id, MENU_COLOR_BASE) {
                    match FlagColor::ALL.get(i) {
                        Some(color) => self.set_color_open(Some(*color)),
                        None => return false,
                    }
                } else if let Some(i) = menu_index(id, MENU_CATEGORY_BASE) {
                    match Category::ALL.get(i) {
                        Some(category) => self.mail.category = Some(*category),
                        None => return false,
                    }
                } else if let Some(i) = menu_index(id, MENU_RULE_BASE) {
                    self.delete_rule(i);
                } else if let Some(i) = menu_index(id, MENU_PRIORITY_BASE) {
                    match Priority::ALL.get(i) {
                        Some(priority) => self.mail.set_priority(*priority),
                        None => return false,
                    }
                } else {
                    return false;
                }
            }
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
                    // a message a rule moves or reads raises no notification
                    let quiet = self.apply_rules_to_arrivals();
                    for notice in notices {
                        if !quiet.contains(&(notice.account.clone(), notice.uid)) {
                            self.notify(ctx, &notice);
                        }
                    }
                    if self.mail.current().is_some_and(|a| a.id == account) {
                        self.mail.status.clear();
                    }
                }
                Err(e) => self.mail.status = format!("{}: {e}", self.account_name(&account)),
            },
            Event::Changed { account, uid, result, .. } => {
                // the change is shown at once; a refusal brings the listing back from the server. A change to a
                // whole mailbox (UID 0) is listed again when it is done, which also reloads the mailbox names.
                match result {
                    Err(e) => {
                        self.mail.status = format!("{}: {e}", self.account_name(&account));
                        self.refresh_account(&account);
                    }
                    Ok(()) if uid == 0 => {
                        self.mail.status.clear();
                        self.refresh_account(&account);
                    }
                    Ok(()) => {}
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
            Event::Sent { id, result, .. } => self.sent(id, result),
            Event::Exported { mailbox, result, .. } => match result {
                Ok(messages) => {
                    let count = messages.len();
                    let file = mbox::write(&messages, now_unix());
                    self.queue_save(ctx, mbox::file_name(&mailbox), file.into_bytes());
                    self.mail.status = format!("Read {count} message{} of {mailbox}. Choose where to save them", if count == 1 { "" } else { "s" });
                }
                Err(e) => self.mail.status = format!("{mailbox} could not be exported: {e}"),
            },
        }
    }

    /// A message went out, or failed. One sent at once (`id` 0) closes the compose sheet; a queued one is taken off the
    /// books, and a failed one comes back as the sheet it was written in, or waits for another try.
    fn sent(&mut self, id: u64, result: Result<(), String>) {
        let flight = self.in_flight.remove(&id);
        match result {
            Ok(()) => {
                if id == 0 {
                    if matches!(self.mail.sheet, Some(Sheet::Compose(_))) {
                        self.mail.close_sheet();
                    }
                } else {
                    self.drafts.remove(&id);
                    if self.undo == Some(id) {
                        self.undo = None;
                    }
                }
                self.mail.status = "Sent".into();
            }
            Err(e) if id == 0 => self.mail.status = format!("The message was not sent: {e}"),
            Err(e) => {
                if let Some(compose) = self.drafts.remove(&id).filter(|_| self.mail.sheet_is_free()) {
                    self.mail.sheet = Some(Sheet::Compose(compose));
                    self.mail.status = format!("The message was not sent: {e}");
                } else if let Some(mut queued) = flight {
                    queued.attempts += 1;
                    if queued.attempts >= schedule::MAX_ATTEMPTS {
                        self.mail.status = format!("A queued message could not be sent and was dropped: {e}");
                    } else {
                        queued.due = now_unix() + schedule::RETRY_SECS;
                        self.mail.queue.outbox.push(queued);
                        self.mail.status = format!("A queued message was not sent ({e}). Mail will try again in a few minutes");
                    }
                    self.save_queue_file();
                }
            }
        }
    }

    fn notify(&self, ctx: &Ctx, notice: &Notice) {
        let who = notice.from.split('<').next().unwrap_or("").trim().trim_matches('"');
        let who = if who.is_empty() { notice.from.as_str() } else { who };
        let id = format!("mail-{}-{}", notice.account, notice.uid);
        let request = Request::new(&id, "New Mail").subtitle(who).body(&notice.subject).thread(&notice.account);
        let _ = usernotif::add(ctx, request);
    }

    /// The password is in Passwords (or, for Account Settings with no new password, nothing needed saving). A new account
    /// goes into the shared list and becomes the shown account; an edited one replaces its old self in the list, and the
    /// password item of its old login goes when the login changed.
    fn account_saved(&mut self, ctx: &Ctx, account: Account, password: Option<String>, old: Option<Account>) {
        match old {
            None => {
                let Some(password) = password else { return };
                match accounts::add(account.clone()) {
                    Ok(()) => {
                        self.mail.add_account(account, password);
                        self.mail.close_sheet();
                        self.refresh_selected();
                    }
                    Err(e) => self.mail.status = e,
                }
            }
            Some(old) => match accounts::update(account.clone()) {
                Ok(()) => {
                    if account.password_site() != old.password_site() || account.username != old.username {
                        let request = passwords::remove(ctx, &old.password_site(), &old.username);
                        self.pending.insert(request, Pending::Forget);
                    }
                    let id = account.id.clone();
                    self.mail.replace_account(account, password);
                    self.mail.close_sheet();
                    self.mail.status = "Account saved".into();
                    self.refresh_account(&id);
                }
                Err(e) => self.mail.status = e,
            },
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
        if flag == Flag::Flagged && !on {
            // clearing the flag also clears its colour keywords on the server
            self.change_message(uid, Change::Color { color: None });
        } else {
            self.change_message(uid, Change::Flag { flag, on });
        }
    }

    /// Message > Flag Color and Clear Flag: the open message is flagged in `color` (or loses its flag). The row changes
    /// at once; the server is told in the background.
    fn set_color_open(&mut self, color: Option<FlagColor>) {
        let Some(uid) = self.mail.open else { return };
        self.mail.set_color_locally(uid, color);
        self.change_message(uid, Change::Color { color });
    }

    /// Message > Mute: the open message's conversation stops raising notifications, or starts again.
    fn toggle_mute(&mut self) {
        let Some(uid) = self.mail.open else { return };
        let Some(muted) = self.mail.toggle_mute(uid) else { return };
        self.mail.status = match crate::mute::save(&self.muted_path, &self.mail.muted) {
            Ok(()) if muted => "Conversation muted".into(),
            Ok(()) => "Conversation unmuted".into(),
            Err(e) => format!("The muted conversations could not be saved: {e}"),
        };
    }

    /// File > Save As: the raw source of the open message, as an `.eml` file in the place the save panel chooses.
    fn save_open_as(&mut self, ctx: &Ctx) {
        let Some(uid) = self.mail.open else { return };
        let Some(raw) = self.mail.bodies.get(&uid) else { return };
        let bytes = crate::export::crlf(raw).into_bytes();
        let subject = self.mail.row(uid).map(|r| r.summary.subject.clone()).unwrap_or_default();
        self.queue_save(ctx, crate::export::eml_name(&subject), bytes);
    }

    /// Asks for a place to save `bytes` as `name`. Files wait their turn: the save panel serves one at a time.
    fn queue_save(&mut self, ctx: &Ctx, name: String, bytes: Vec<u8>) {
        self.save_queue.push_back((name, bytes));
        self.start_next_save(ctx);
    }

    /// Opens the save panel for the next file in line, unless one is open already.
    fn start_next_save(&mut self, ctx: &Ctx) {
        if self.docs.values().any(|task| matches!(task, DocTask::Save { .. })) {
            return;
        }
        let Some((name, bytes)) = self.save_queue.pop_front() else { return };
        let request = ctx.save_document(&attachments::safe_name(&name), &[]);
        self.docs.insert(request, DocTask::Save { name, bytes });
    }

    /// The save panel is not available to the app (the shell lacks the feature): the file goes in the app's data directory.
    fn save_in_data_dir(&mut self, name: &str, bytes: &[u8]) {
        let dir = self.data_dir.join("saved-files");
        let file = Attachment { name: name.to_string(), mime: String::new(), bytes: bytes.to_vec() };
        self.mail.status = match attachments::save_all(&dir, &[file]) {
            Ok(_) => format!("Saved {name} in {}", dir.display()),
            Err(e) => format!("{name} could not be saved: {e}"),
        };
    }

    /// Message > Attach Files: the open panel, for as many files as the message may carry.
    fn pick_files(&mut self, ctx: &Ctx) {
        if !matches!(self.mail.sheet, Some(Sheet::Compose(_))) {
            self.mail.status = "Start a message first".into();
            return;
        }
        let options = OpenOptions { multiple: true, title: "Attach Files".into(), ..OpenOptions::default() };
        let request = ctx.open_panel(&options);
        self.docs.insert(request, DocTask::Attach);
    }

    /// Mailbox > Import Mailbox: the open panel for an mbox file.
    fn import_mailbox(&mut self, ctx: &Ctx) {
        let request = ctx.open_document(DocMode::Read, "Import Mailbox", &[]);
        self.docs.insert(request, DocTask::ImportMbox);
    }

    /// The answer to a request for a file panel. False when the request was not the window's.
    pub fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::OpenedMany { request, docs } => match self.docs.remove(&request) {
                Some(DocTask::Attach) => self.attach_documents(ctx, docs),
                Some(other) => {
                    self.docs.insert(request, other);
                    return false;
                }
                None => return false,
            },
            DocumentEvent::Opened(doc) => match self.docs.remove(&doc.request) {
                Some(DocTask::Attach) => self.attach_documents(ctx, vec![doc]),
                Some(DocTask::ImportMbox) => self.import_document(ctx, doc),
                Some(other) => {
                    self.docs.insert(doc.request, other);
                    return false;
                }
                None => return false,
            },
            DocumentEvent::SaveTo(mut doc) => match self.docs.remove(&doc.request) {
                Some(DocTask::Save { name, bytes }) => match doc.file.write_all(&bytes) {
                    Ok(()) => ctx.commit_document(doc.request),
                    Err(e) => {
                        self.mail.status = format!("{name} could not be saved: {e}");
                        ctx.abort_document(doc.request);
                        self.start_next_save(ctx);
                    }
                },
                Some(other) => {
                    self.docs.insert(doc.request, other);
                    return false;
                }
                None => return false,
            },
            DocumentEvent::Saved(doc) => {
                ctx.release_document(doc.grant);
                self.mail.status = format!("Saved {}", doc.name);
                self.start_next_save(ctx);
            }
            DocumentEvent::Failed { request, code } => match self.docs.remove(&request) {
                Some(DocTask::Save { name, bytes }) if code == DocErrorCode::Unsupported => {
                    self.save_in_data_dir(&name, &bytes);
                    self.start_next_save(ctx);
                }
                Some(DocTask::Save { .. }) => {
                    // the user cancelled one file: the ones still waiting are dropped with it
                    self.save_queue.clear();
                    if code != DocErrorCode::Cancelled {
                        self.mail.status = format!("The file could not be saved ({code:?})");
                    }
                }
                Some(_) => {
                    if code != DocErrorCode::Cancelled {
                        self.mail.status = format!("The file could not be opened ({code:?})");
                    }
                }
                None => return false,
            },
        }
        self.dirty = true;
        true
    }

    /// The files chosen in Attach Files go on the message being written, each read whole.
    fn attach_documents(&mut self, ctx: &Ctx, docs: Vec<bifrost_app::Document>) {
        for mut doc in docs {
            let name = doc.name.clone();
            let bytes = read_file(&mut doc.file, doc.size, MAX_ATTACHMENT_BYTES);
            ctx.release_document(doc.grant);
            let result = bytes.and_then(|bytes| match &mut self.mail.sheet {
                Some(Sheet::Compose(compose)) => compose.attach(Attachment { mime: attachments::mime_for(&name), name: name.clone(), bytes }),
                _ => Err("The message is no longer open".to_string()),
            });
            if let Err(e) = result {
                self.mail.status = format!("{name}: {e}");
            }
        }
    }

    /// The file chosen in Import Mailbox: its messages are appended to the mailbox named after the file, which is created
    /// when the account lacks it.
    fn import_document(&mut self, ctx: &Ctx, mut doc: bifrost_app::Document) {
        let name = doc.name.clone();
        let bytes = read_file(&mut doc.file, doc.size, IMPORT_LIMIT);
        ctx.release_document(doc.grant);
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(e) => {
                self.mail.status = format!("{name}: {e}");
                return;
            }
        };
        let messages = mbox::read(&String::from_utf8_lossy(&bytes));
        if messages.is_empty() {
            self.mail.status = format!("{name} holds no messages");
            return;
        }
        let target = mbox::mailbox_name(&name);
        self.mail.status = format!("Importing {} message{} into {target}", messages.len(), if messages.len() == 1 { "" } else { "s" });
        self.change_mailbox(&target, Change::Import { messages });
    }

    /// Mailbox > Export Mailbox: reads the shown mailbox; the save panel opens when the messages have arrived.
    fn export_mailbox(&mut self) {
        let Some(account) = self.mail.current().cloned() else { return };
        let Some(password) = self.mail.password(&account).map(str::to_string) else {
            self.mail.status = format!("No password is saved for {}", account.name);
            return;
        };
        let mailbox = self.mail.mailbox.clone();
        self.mail.status = format!("Reading {mailbox}");
        let transport = self.transport.clone();
        self.jobs += 1;
        sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::export(transport.as_ref(), &account, &password, &mailbox));
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

    /// Message > Move To and Move Again: moves the open message to `to` and remembers it for Move Again. A message leaving
    /// the Junk mailbox loses its junk mark.
    fn move_open_to(&mut self, to: &str) {
        let Some(uid) = self.mail.open else { return };
        self.mail.last_target = Some(to.to_string());
        let leaving_junk = JUNK_NAMES.iter().any(|name| name.eq_ignore_ascii_case(&self.mail.mailbox)) && !JUNK_NAMES.iter().any(|name| name.eq_ignore_ascii_case(to));
        self.mail.remove_row(uid);
        let action = if leaving_junk { Change::NotJunk { to: to.to_string() } } else { Change::Move { to: to.to_string() } };
        self.change_message(uid, action);
    }

    /// Message > Move to Junk: the message moves to the account's Junk mailbox marked `$Junk`.
    fn junk_open(&mut self) {
        let Some(uid) = self.mail.open else { return };
        let Some(to) = self.mail.find_mailbox(&JUNK_NAMES) else {
            self.mail.status = "This account has no Junk mailbox".into();
            return;
        };
        self.mail.remove_row(uid);
        self.change_message(uid, Change::Junk { to });
    }

    /// Message > Copy To: the open message stays where it is and a copy goes to `to`.
    fn copy_open_to(&mut self, to: &str) {
        let Some(uid) = self.mail.open else { return };
        self.change_message(uid, Change::Copy { to: to.to_string() });
        self.mail.status = format!("Copying to {to}");
    }

    /// Runs a mailbox-level change (no message) for `mailbox` of the shown account. The listing is asked for again when
    /// the server answers (see `Event::Changed` with UID 0).
    fn change_mailbox(&mut self, mailbox: &str, action: Change) {
        let Some(account) = self.mail.current().cloned() else { return };
        let Some(password) = self.mail.password(&account).map(str::to_string) else {
            self.mail.status = format!("No password is saved for {}", account.name);
            return;
        };
        let mailbox = mailbox.to_string();
        let transport = self.transport.clone();
        self.jobs += 1;
        sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::change(transport.as_ref(), &account, &password, &mailbox, 0, &action));
    }

    /// Mailbox > Delete Mailbox: the shown mailbox goes, and the inbox is shown. The inbox itself cannot be deleted.
    fn delete_mailbox(&mut self) {
        let name = self.mail.mailbox.clone();
        if name.eq_ignore_ascii_case(INBOX) {
            self.mail.status = "The inbox cannot be deleted".into();
            return;
        }
        self.change_mailbox(&name, Change::Delete);
        self.mail.mailboxes.retain(|m| *m != name);
        self.mail.show_mailbox(INBOX);
        self.list_scroll = Scroller::new();
    }

    /// Mailbox > Erase Deleted Items and Erase Junk Mail: every message of the mailbox is expunged.
    fn erase(&mut self, candidates: &[&str], label: &str) {
        let Some(name) = self.mail.find_mailbox(candidates) else {
            self.mail.status = format!("This account has no {label} mailbox");
            return;
        };
        self.change_mailbox(&name, Change::EraseAll);
        self.mail.status = format!("Erasing {label}");
    }

    /// True when the open message's text has arrived (Forward quotes it).
    fn open_body_loaded(&self) -> bool {
        self.mail.open.is_some_and(|uid| self.mail.bodies.contains_key(&uid))
    }

    /// Edit > Paste as Quotation: the clipboard text goes into the focused compose field with `> ` before each line.
    fn paste_quoted(&mut self) -> bool {
        let Some(text) = clipboard::text() else { return false };
        let multiline = self.focus_is_multiline();
        let quoted = message::quote_text(&text);
        let quoted: String = quoted.chars().filter(|c| !c.is_control() || (*c == '\n' && multiline)).collect();
        match self.form_mut() {
            Some(form) => form.type_text(&quoted),
            None => return false,
        }
        self.dirty = true;
        true
    }

    fn reply_all(&mut self) {
        let Some(uid) = self.mail.open else { return };
        if let Some(original) = self.mail.row(uid).map(|r| r.summary.clone()) {
            self.mail.open_reply_all(&original);
        }
    }

    /// Message > Forward: the decoded text of the open message under a `Fwd:` subject. Nothing happens until the text
    /// has arrived.
    fn forward(&mut self) {
        let Some(uid) = self.mail.open else { return };
        let Some(original) = self.mail.row(uid).map(|r| r.summary.clone()) else { return };
        let Some(raw) = self.mail.bodies.get(&uid) else { return };
        let text = crate::body::readable(raw);
        self.mail.open_forward(&original, &text);
    }

    /// True when the open message carries files.
    fn open_has_files(&self) -> bool {
        self.mail.open.and_then(|uid| self.mail.attachments.get(&uid)).is_some_and(|files| !files.is_empty())
    }

    /// Message > Save Attachments: each file of the open message gets its own save panel, one after the other.
    fn save_attachments(&mut self, ctx: &Ctx) {
        let Some(uid) = self.mail.open else { return };
        let files: Vec<(String, Vec<u8>)> = match self.mail.attachments.get(&uid) {
            Some(files) if !files.is_empty() => files.iter().map(|a| (a.name.clone(), a.bytes.clone())).collect(),
            _ => {
                self.mail.status = "This message has no attachments".into();
                return;
            }
        };
        self.save_queue.extend(files);
        self.start_next_save(ctx);
    }

    /// A tap on a file named in the open message: the save panel for that file.
    fn save_attachment(&mut self, ctx: &Ctx, index: usize) {
        let Some(uid) = self.mail.open else { return };
        let Some(file) = self.mail.attachments.get(&uid).and_then(|files| files.get(index)).map(|a| (a.name.clone(), a.bytes.clone())) else { return };
        self.queue_save(ctx, file.0, file.1);
    }

    /// Message > Remove Attachments: the message is written again without its files and the old one is deleted. The copy
    /// has a new UID, so the list is read again when the server has done it.
    fn remove_attachments(&mut self) {
        let Some(uid) = self.mail.open else { return };
        let Some(raw) = self.mail.bodies.get(&uid) else {
            self.mail.status = "Wait for the message to load".into();
            return;
        };
        let stripped = mime::strip_attachments(raw);
        if stripped.removed == 0 {
            self.mail.status = "This message has no attachments".into();
            return;
        }
        let flags = self.mail.flags_of(uid).unwrap_or_default();
        self.mail.status = format!("Removing {} attachment{}", stripped.removed, if stripped.removed == 1 { "" } else { "s" });
        self.mail.remove_row(uid);
        self.change_message(0, Change::Replace { uid, raw: stripped.raw, flags });
    }

    /// Message > Add Sender to Contacts: asks the address book whether it has the address, and saves the person if not.
    fn add_sender_to_contacts(&mut self, ctx: &Ctx) {
        let Some(uid) = self.mail.open else { return };
        let Some(from) = self.mail.row(uid).map(|r| r.summary.from.clone()) else { return };
        let Some(person) = people::person_from(&from) else {
            self.mail.status = "The sender has no email address".into();
            return;
        };
        let request = contacts::fetch(ctx, &[ContactKey::Emails], Some(person.address.as_str()));
        self.pending.insert(request, Pending::ContactCheck(person));
        self.mail.status = "Checking Contacts".into();
    }

    /// Runs one change to a message of the shown mailbox on the server.
    fn change_message(&mut self, uid: u32, action: Change) {
        let Some(account) = self.mail.current().map(|a| a.id.clone()) else { return };
        let mailbox = self.mail.mailbox.clone();
        self.change_in(&account, &mailbox, uid, action);
    }

    /// Runs one change to a message of `mailbox` of the account with this id. A UID of 0 asks for the mailbox to be listed
    /// again afterwards.
    fn change_in(&mut self, account_id: &str, mailbox: &str, uid: u32, action: Change) {
        let Some(account) = self.mail.accounts.iter().find(|a| a.id == account_id).cloned() else { return };
        let Some(password) = self.mail.password(&account).map(str::to_string) else {
            self.mail.status = format!("No password is saved for {}", account.name);
            return;
        };
        let mailbox = mailbox.to_string();
        let transport = self.transport.clone();
        self.jobs += 1;
        sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::change(transport.as_ref(), &account, &password, &mailbox, uid, &action));
    }

    fn open_message(&mut self, uid: u32) {
        self.reader_scroll = Scroller::new();
        if let Some(bar) = self.mail.find.as_mut() {
            bar.current = 0;
        }
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
                        self.pending.insert(request, Pending::Add { account, password, old: None });
                        self.mail.status = "Saving the password".into();
                    }
                    Err(e) => self.mail.status = e,
                }
            }
            Some(Sheet::Compose(compose)) => self.send(&compose),
            Some(Sheet::Mailbox(sheet)) => match self.mail.new_mailbox_name(&sheet.form, sheet.rename_from.as_deref()) {
                Ok(name) => {
                    match &sheet.rename_from {
                        Some(old) => {
                            self.change_mailbox(old, Change::Rename { to: name.clone() });
                            self.mail.show_mailbox(&name);
                        }
                        None => self.change_mailbox(&name, Change::Create { name: name.clone() }),
                    }
                    self.mail.close_sheet();
                }
                Err(e) => self.mail.status = e,
            },
            Some(Sheet::Quickly(form)) => match self.mail.quickly_matches(form.value("Go to mailbox")).into_iter().next() {
                Some(name) => {
                    self.mail.close_sheet();
                    self.mail.show_mailbox(&name);
                    self.list_scroll = Scroller::new();
                    self.refresh_selected();
                }
                None => self.mail.status = "No mailbox matches".into(),
            },
            Some(Sheet::Search(form)) => {
                let text = form.value("Search").to_string();
                self.mail.apply_search(&text);
                self.mail.close_sheet();
                self.search_server();
            }
            Some(Sheet::Prompt(prompt)) => self.submit_prompt(ctx, prompt),
            None => {}
        }
    }

    /// The primary action of a small sheet (see [`PromptKind`]).
    fn submit_prompt(&mut self, ctx: &Ctx, prompt: Prompt) {
        match prompt.kind {
            PromptKind::Remind => self.submit_remind(&prompt.form),
            PromptKind::Redirect => self.submit_redirect(&prompt.form),
            PromptKind::SmartMailbox => match self.mail.smart_from_form(&prompt.form) {
                Ok(smart) => {
                    self.mail.prefs.smart.push(smart);
                    let index = self.mail.prefs.smart.len() - 1;
                    self.save_prefs();
                    self.mail.close_sheet();
                    self.mail.apply_smart(index);
                    self.search_server();
                    self.list_scroll = Scroller::new();
                }
                Err(e) => self.mail.status = e,
            },
            PromptKind::Rule => {
                if self.mail.prefs.rules.len() >= MAX_RULES {
                    self.mail.status = format!("Mail keeps at most {MAX_RULES} rules. Delete one first");
                    return;
                }
                match self.mail.rule_from_form(&prompt.form) {
                    Ok(rule) => {
                        self.mail.status = format!("Rule {} added. Mailbox > Rules > Apply Rules runs it on the shown mailbox", rule.name);
                        self.mail.prefs.rules.push(rule);
                        self.save_prefs();
                        self.mail.close_sheet();
                    }
                    Err(e) => self.mail.status = e,
                }
            }
            PromptKind::Settings => match self.mail.apply_settings(&prompt.form) {
                Ok(()) => {
                    self.save_prefs();
                    self.mail.close_sheet();
                    self.mail.status = "Settings saved".into();
                }
                Err(e) => self.mail.status = e,
            },
            PromptKind::EditAccount(id) => self.submit_edit_account(ctx, &id, &prompt.form),
            PromptKind::RemoveAccount(id) => self.submit_remove_account(ctx, &id, &prompt.form),
        }
    }

    /// Remind Me: the message is noted with the time it should come back.
    fn submit_remind(&mut self, form: &crate::state::Form) {
        let seconds = match schedule::parse_delay(form.value("Remind me in")) {
            Ok(seconds) => seconds,
            Err(e) => {
                self.mail.status = e;
                return;
            }
        };
        let Some(uid) = self.mail.open else { return };
        let Some(account) = self.mail.current().map(|a| a.id.clone()) else { return };
        let Some((message_id, subject)) = self.mail.row(uid).and_then(|r| r.summary.message_id.clone().map(|id| (id, r.summary.subject.clone()))) else {
            self.mail.status = "This message has no Message-ID, so it cannot be found again".into();
            return;
        };
        let mailbox = self.mail.mailbox.clone();
        self.mail.queue.reminders.push(Reminder { account, mailbox, message_id, subject, due: now_unix() + seconds });
        self.save_queue_file();
        self.mail.close_sheet();
        self.mail.status = format!("The message will come back in {}", wait_text(seconds));
    }

    /// Redirect: the original message goes to the new recipients with Resent headers in front of it.
    fn submit_redirect(&mut self, form: &crate::state::Form) {
        let to: Vec<String> = message::split_addresses(form.value("To")).iter().map(|a| message::address_of(a).to_string()).collect();
        if to.is_empty() {
            self.mail.status = "Add at least one recipient".into();
            return;
        }
        if let Some(bad) = to.iter().find(|a| !bifrost_app::controls::internet_accounts::valid_address(a)) {
            self.mail.status = format!("{bad} is not a valid address");
            return;
        }
        let Some(uid) = self.mail.open else { return };
        let Some(raw) = self.mail.bodies.get(&uid).cloned() else {
            self.mail.status = "Wait for the message to load".into();
            return;
        };
        let Some(account) = self.mail.current().cloned() else { return };
        let Some(password) = self.mail.password(&account).map(str::to_string) else {
            self.mail.status = format!("No password is saved for {}", account.name);
            return;
        };
        let unix = now_unix();
        let domain = account.email.rsplit_once('@').map_or("localhost", |(_, d)| d).to_string();
        self.seq += 1;
        let id = message::new_message_id(&domain, unix, self.seq);
        let text = message::redirect_message(&raw, &account.email, &to, &id, unix);
        let from = account.email.clone();
        self.mail.close_sheet();
        self.mail.status = "Redirecting".into();
        self.jobs += 1;
        let transport = self.transport.clone();
        sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::send(transport, account, password, 0, from, to, text));
    }

    /// Account Settings: the edited account replaces the old one. A new password is saved in Passwords first.
    fn submit_edit_account(&mut self, ctx: &Ctx, id: &str, form: &crate::state::Form) {
        let Some(old) = self.mail.accounts.iter().find(|a| a.id == id).cloned() else {
            self.mail.status = "That account is gone".into();
            return;
        };
        match self.mail.edited_account(form, &old) {
            Ok((account, Some(password))) => {
                let request = passwords::save(ctx, &account.password_site(), &account.username, &password);
                self.pending.insert(request, Pending::Add { account, password, old: Some(old) });
                self.mail.status = "Saving the password".into();
            }
            Ok((account, None)) => self.account_saved(ctx, account, None, Some(old)),
            Err(e) => self.mail.status = e,
        }
    }

    /// Remove Account: the account must be named. It leaves the shared list, its password leaves Passwords, and what Mail
    /// kept for it goes.
    fn submit_remove_account(&mut self, ctx: &Ctx, id: &str, form: &crate::state::Form) {
        let Some(account) = self.mail.accounts.iter().find(|a| a.id == id).cloned() else {
            self.mail.status = "That account is gone".into();
            return;
        };
        if !form.value("Account name").trim().eq_ignore_ascii_case(account.name.trim()) {
            self.mail.status = format!("Type {} to remove the account", account.name);
            return;
        }
        if let Err(e) = accounts::remove(id) {
            self.mail.status = e;
            return;
        }
        let request = passwords::remove(ctx, &account.password_site(), &account.username);
        self.pending.insert(request, Pending::Forget);
        self.mail.forget_account(id);
        self.watching.remove(id);
        self.live.remove(id);
        self.mail.queue.outbox.retain(|q| q.account != id);
        self.mail.queue.reminders.retain(|r| r.account != id);
        self.save_queue_file();
        self.save_prefs();
        self.mail.close_sheet();
        self.list_scroll = Scroller::new();
        self.mail.status = format!("Removed {}", account.name);
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

    /// Sends the message now, or queues it: Send Later (the "Send in" field) waits as long as it says, and the undo-send
    /// delay of the settings holds every message back for a few seconds. A queued message goes out from `run_queue`.
    fn send(&mut self, compose: &Compose) {
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
        let wait = match self.send_delay(compose) {
            Ok(wait) => wait,
            Err(e) => {
                self.mail.status = e;
                return;
            }
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
        let recipients: Vec<String> = draft.envelope_recipients().into_iter().map(str::to_string).collect();
        if let Some((seconds, undoable)) = wait {
            let queued_id = self.mail.queue.new_id();
            self.mail.queue.outbox.push(Queued { id: queued_id, due: unix + seconds, account: account.id.clone(), from: draft.from.clone(), recipients, text, attempts: 0 });
            self.save_queue_file();
            if undoable {
                self.undo = Some(queued_id);
                self.drafts.insert(queued_id, compose.clone());
                self.mail.status = format!("Sending in {seconds} s. Message > Undo Send takes it back");
            } else {
                self.mail.status = format!("The message will be sent in {}", wait_text(seconds));
            }
            self.mail.close_sheet();
            return;
        }
        let Some(password) = self.mail.password(&account).map(str::to_string) else {
            self.mail.status = format!("No password is saved for {}", account.name);
            return;
        };
        self.mail.status = "Sending".into();
        self.jobs += 1;
        let transport = self.transport.clone();
        let from = draft.from.clone();
        sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::send(transport, account, password, 0, from, recipients, text));
    }

    /// How long a message waits before it is sent: the "Send in" field when the message has one, else the undo-send delay
    /// of the settings. None sends at once. The flag tells whether Undo Send can still take the message back.
    fn send_delay(&self, compose: &Compose) -> Result<Option<(i64, bool)>, String> {
        if compose.form.fields.iter().any(|f| f.label == "Send in") {
            let text = compose.form.value("Send in").trim();
            if text.is_empty() {
                return Err("Enter when to send the message (30m, 2h, 1d), or turn Message > Send Later off".into());
            }
            return schedule::parse_delay(text).map(|seconds| Some((seconds, false)));
        }
        let delay = self.mail.prefs.undo_delay;
        Ok((delay > 0).then_some((i64::from(delay), true)))
    }

    /// True while the message sent last is still held in the outbox, so Undo Send can bring it back.
    fn undo_pending(&self) -> bool {
        self.undo.is_some_and(|id| self.mail.queue.outbox.iter().any(|q| q.id == id))
    }

    /// Message > Undo Send: the held message leaves the outbox and opens again as it was written.
    fn undo_send(&mut self) {
        if !self.sheet_free() {
            return;
        }
        let Some(id) = self.undo.take() else { return };
        if !self.mail.queue.cancel(id) {
            self.mail.status = "The message has gone already".into();
            return;
        }
        self.save_queue_file();
        match self.drafts.remove(&id) {
            Some(compose) => {
                self.mail.sheet = Some(Sheet::Compose(compose));
                self.mail.status = "Send undone".into();
            }
            None => self.mail.status = "Send undone, but the message could not be opened again".into(),
        }
    }

    /// Mailbox > Send Queued Messages Now: everything waiting in the outbox is due at once.
    fn send_queued_now(&mut self) {
        let now = now_unix();
        for queued in &mut self.mail.queue.outbox {
            queued.due = now;
        }
        self.save_queue_file();
        self.mail.status = "Sending the queued messages".into();
    }

    /// Sends the queued messages that are due and brings back the messages whose reminders are due. Called on every tick.
    fn run_queue(&mut self, ctx: &Ctx) {
        let now = now_unix();
        if self.mail.queue.next_due().map_or(true, |due| due > now) {
            return;
        }
        for queued in self.mail.queue.take_due_messages(now) {
            let Some(account) = self.mail.accounts.iter().find(|a| a.id == queued.account).cloned() else {
                self.mail.status = "A queued message was dropped: its account is gone".into();
                continue;
            };
            let Some(password) = self.mail.password(&account).map(str::to_string) else {
                // the password is still being read from Passwords; try again in a moment
                self.mail.queue.outbox.push(Queued { due: now + 10, ..queued });
                continue;
            };
            self.in_flight.insert(queued.id, queued.clone());
            self.jobs += 1;
            let transport = self.transport.clone();
            let Queued { id, from, recipients, text, .. } = queued;
            sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::send(transport, account, password, id, from, recipients, text));
        }
        for reminder in self.mail.queue.take_due_reminders(now) {
            if !self.mail.accounts.iter().any(|a| a.id == reminder.account) {
                continue;
            }
            if self.mail.passwords.get(&reminder.account).is_none() {
                self.mail.queue.reminders.push(Reminder { due: now + 10, ..reminder });
                continue;
            }
            self.change_in(&reminder.account, &reminder.mailbox, 0, Change::Remind { message_id: reminder.message_id.clone() });
            let id = format!("mail-reminder-{}-{}", reminder.account, reminder.due);
            let request = Request::new(&id, "Mail Reminder").subtitle(&reminder.subject).body("This message is back in your inbox").thread(&reminder.account);
            let _ = usernotif::add(ctx, request);
        }
        self.save_queue_file();
        self.dirty = true;
    }

    fn save_queue_file(&mut self) {
        if let Err(e) = schedule::save(&self.queue_path, &self.mail.queue) {
            self.mail.status = format!("The outbox could not be saved: {e}");
        }
    }

    fn save_prefs(&mut self) {
        if let Err(e) = prefs::save(&self.prefs_path, self.mail.prefs_to_save()) {
            self.mail.status = format!("The settings could not be saved: {e}");
        }
    }

    /// True when a new sheet may open; otherwise says why not.
    fn sheet_free(&mut self) -> bool {
        if self.mail.sheet_is_free() {
            true
        } else {
            self.mail.status = "Send or cancel the message first".into();
            false
        }
    }

    // forward, redirect, rules and the sidebar --------------------------------------------------------------------

    /// Message > Forward as Attachment: the original goes along as a file.
    fn forward_attachment(&mut self) {
        let Some(uid) = self.mail.open else { return };
        let Some(original) = self.mail.row(uid).map(|r| r.summary.clone()) else { return };
        let Some(raw) = self.mail.bodies.get(&uid) else { return };
        let own = self.mail.current().map(|a| a.email.clone()).unwrap_or_default();
        let draft = message::forward_attachment_draft(&original, &own, raw);
        self.mail.open_draft(draft);
    }

    /// Message > Redirect: asks who should get the open message.
    fn redirect(&mut self) {
        if self.open_body_loaded() && self.sheet_free() {
            self.mail.open_redirect();
        }
    }

    /// Runs the rules over the messages that just arrived in an inbox. Returns the (account, UID) of the messages a rule
    /// moved or read, which raise no notification.
    fn apply_rules_to_arrivals(&mut self) -> HashSet<(String, u32)> {
        let mut quiet = HashSet::new();
        if self.mail.prefs.rules.is_empty() {
            return quiet;
        }
        for (account, row) in self.mail.arrived.clone() {
            let actions = rules::outcome(&self.mail.prefs.rules, &row.summary);
            if actions.iter().any(Action::is_quiet) {
                quiet.insert((account.clone(), row.uid));
            }
            for action in &actions {
                self.run_action(&account, INBOX, row.uid, action);
            }
        }
        quiet
    }

    /// Mailbox > Rules > Apply Rules: the rules run over the messages loaded in the shown mailbox.
    fn apply_rules_to_shown(&mut self) {
        let Some(account) = self.mail.current().map(|a| a.id.clone()) else { return };
        let mailbox = self.mail.mailbox.clone();
        let mut touched = 0;
        for row in self.mail.rows.clone() {
            let actions = rules::outcome(&self.mail.prefs.rules, &row.summary);
            if actions.is_empty() {
                continue;
            }
            touched += 1;
            for action in &actions {
                self.run_action(&account, &mailbox, row.uid, action);
            }
        }
        self.mail.status = format!("Rules ran on {touched} message{}", if touched == 1 { "" } else { "s" });
    }

    /// Does what a rule says to one message. The shown list changes at once; the server is told in the background.
    fn run_action(&mut self, account_id: &str, mailbox: &str, uid: u32, action: &Action) {
        let shown = self.mail.current().is_some_and(|a| a.id == account_id) && self.mail.mailbox == mailbox;
        match action {
            Action::MarkRead => {
                if shown {
                    self.mail.set_flag_locally(uid, Flag::Seen, true);
                }
                self.change_in(account_id, mailbox, uid, Change::Flag { flag: Flag::Seen, on: true });
            }
            Action::Flag(color) => {
                if shown {
                    self.mail.set_color_locally(uid, Some(*color));
                }
                self.change_in(account_id, mailbox, uid, Change::Color { color: Some(*color) });
            }
            Action::MoveTo(target) => {
                if shown {
                    self.mail.remove_row(uid);
                }
                self.change_in(account_id, mailbox, uid, Change::Move { to: target.clone() });
            }
        }
    }

    /// Mailbox > Rules > Delete Rule.
    fn delete_rule(&mut self, index: usize) {
        if index < self.mail.prefs.rules.len() {
            let rule = self.mail.prefs.rules.remove(index);
            self.save_prefs();
            self.mail.status = format!("Deleted rule {}", rule.name);
        }
    }

    /// Mailbox > Delete Smart Mailbox: the smart mailbox on screen is deleted and the full list comes back.
    fn delete_smart(&mut self) {
        let Some(index) = self.mail.smart_active else { return };
        if index < self.mail.prefs.smart.len() {
            let smart = self.mail.prefs.smart.remove(index);
            self.mail.status = format!("Deleted smart mailbox {}", smart.name);
        }
        self.mail.apply_search("");
        self.save_prefs();
    }

    /// Mailbox > Add to Favorites Bar (or Remove): pins the shown mailbox, and shows the bar when something is pinned.
    fn toggle_favorite(&mut self) {
        let Some(account) = self.mail.current().map(|a| a.id.clone()) else { return };
        let mailbox = self.mail.mailbox.clone();
        if !self.mail.prefs.toggle_favorite(&account, &mailbox) {
            self.mail.status = "The favorites bar is full".into();
            return;
        }
        if self.mail.prefs.is_favorite(&account, &mailbox) {
            self.mail.prefs.show_favorites = true;
        }
        self.save_prefs();
    }

    /// A tap on the favourites bar: the mailbox of that account is shown.
    fn open_favorite(&mut self, index: usize) {
        let Some(favorite) = self.mail.prefs.favorites.get(index).cloned() else { return };
        let Some(at) = self.mail.accounts.iter().position(|a| a.id == favorite.account) else { return };
        if at != self.mail.account {
            self.mail.show_account(at);
        }
        self.mail.show_mailbox(&favorite.mailbox);
        self.list_scroll = Scroller::new();
        self.refresh_selected();
    }

    fn wake_clone(&self) -> Option<UnixStream> {
        self.wake_tx.as_ref().and_then(|w| w.try_clone().ok())
    }

    /// True when the field that takes typed text holds several lines (the message, the signature).
    fn focus_is_multiline(&self) -> bool {
        self.mail.sheet.as_ref().map(|sheet| sheet.form()).is_some_and(|form| form.fields.get(form.focus).is_some_and(|f| is_multiline(f.label)))
    }

    /// True when the field that takes typed text is the From picker: a tap changes it, typing does not.
    fn focus_is_picker(&self) -> bool {
        match &self.mail.sheet {
            Some(Sheet::Compose(c)) => c.form.fields.get(c.form.focus).is_some_and(|f| f.label == "From"),
            _ => false,
        }
    }

    fn form_mut(&mut self) -> Option<&mut crate::state::Form> {
        self.mail.sheet.as_mut().map(Sheet::form_mut)
    }

    /// The pane a point is in. With a sheet open everything is chrome, so presses never scroll the panes behind it.
    pub fn region(&self, ui: &Ui, x: f32, y: f32) -> Region {
        if self.mail.sheet.is_some() {
            return Region::Chrome;
        }
        crate::draw::pane_at(crate::draw::columns(ui, self.w as f32, self.mail.show_sidebar), crate::draw::chrome_height(ui, &self.mail), x, y)
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
                crate::draw::Button::ReplyAll => self.reply_all(),
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
                crate::draw::Side::Smart(index) => {
                    if self.mail.apply_smart(index) {
                        self.list_scroll = Scroller::new();
                        self.search_server();
                    }
                }
                crate::draw::Side::Heading(_) => return false,
                crate::draw::Side::Add => self.mail.open_setup(),
            },
            crate::draw::Hit::Message(uid) => self.open_message(uid),
            crate::draw::Hit::Favorite(index) => self.open_favorite(index),
            crate::draw::Hit::Attachment(index) => self.save_attachment(ctx, index),
            crate::draw::Hit::Attach => self.pick_files(ctx),
            crate::draw::Hit::RemoveAttachment(index) => {
                if let Some(Sheet::Compose(compose)) = &mut self.mail.sheet {
                    compose.detach(index);
                }
            }
            crate::draw::Hit::FindPrevious => self.step_match(false),
            crate::draw::Hit::FindNext => self.step_match(true),
            crate::draw::Hit::FindDone => self.mail.find = None,
            crate::draw::Hit::Field(index) => {
                let from_picker = self.mail.sheet.as_ref().map(Sheet::form).and_then(|form| form.fields.get(index)).is_some_and(|f| f.label == "From");
                if let Some(form) = self.form_mut() {
                    form.focus_on(index);
                }
                if from_picker {
                    let choices = self.mail.from_choices();
                    if let Some(Sheet::Compose(compose)) = &mut self.mail.sheet {
                        compose.cycle_from(&choices);
                    }
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

/// How long a wait of `seconds` is, in words: "2 hours", "1 day", "45 minutes".
fn wait_text(seconds: i64) -> String {
    let (amount, unit) = if seconds >= 86_400 {
        (seconds / 86_400, "day")
    } else if seconds >= 3600 {
        (seconds / 3600, "hour")
    } else if seconds >= 60 {
        (seconds / 60, "minute")
    } else {
        (seconds.max(1), "second")
    };
    format!("{amount} {unit}{}", if amount == 1 { "" } else { "s" })
}

/// Reads a file the user chose, whole, refusing one over `limit` bytes without reading it.
fn read_file(file: &mut std::fs::File, size: u64, limit: usize) -> Result<Vec<u8>, String> {
    let too_big = || format!("The file is larger than {} MB", limit / (1024 * 1024));
    if size > limit as u64 {
        return Err(too_big());
    }
    let mut bytes = Vec::new();
    (&mut *file).take(limit as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err(too_big());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_menu_ids_map_to_their_position_inside_the_range_only() {
        assert_eq!(menu_index(MENU_MOVE_BASE, MENU_MOVE_BASE), Some(0));
        assert_eq!(menu_index(MENU_MOVE_BASE + 98, MENU_MOVE_BASE), Some(98));
        assert_eq!(menu_index(MENU_MOVE_BASE + 99, MENU_MOVE_BASE), None);
        assert_eq!(menu_index(MENU_REPLY, MENU_MOVE_BASE), None);
        assert_eq!(menu_index(MENU_COPY_BASE + 3, MENU_MOVE_BASE), None, "the next range is not this one");
    }

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
