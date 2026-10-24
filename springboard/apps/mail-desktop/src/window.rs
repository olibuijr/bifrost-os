//! The Mail window: the state in [`crate::state`] wired to the network jobs ([`crate::sync`]), the shared Internet
//! Accounts list and Passwords for the logins, and the notifications for new mail. This file holds the flow and the
//! input; drawing and hit-testing are in [`crate::draw`].

use crate::accounts;
use crate::attachments;
use crate::body::Attachment;
use crate::draw::open_has_others;
use crate::pdf;
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
use crate::richtext::TextColor;
use crate::rules::{self, Action};
use crate::schedule::{self, Queued, Reminder};
use crate::search::Query;
use crate::spelling;
use crate::discover::Found;
use crate::state::{is_multiline, is_yes, Compose, Filter, Mail, Notice, Prompt, PromptKind, Sheet, SortKey, ARCHIVE_NAMES, INBOX, JUNK_NAMES, MANUAL, TRASH_NAMES};
use crate::sync::{self, Change, Event};
use crate::transport;
use bifrost_app::calendar;
use bifrost_app::contacts::{self, Key as ContactKey, SaveRequest};
use bifrost_app::controls::internet_accounts::Account;
use bifrost_app::controls::text::ClickCounter;
use bifrost_app::keychain::{self, Error as LegacyError, Reply as KeyReply};
use bifrost_app::kit::{clipboard, Scroller, TextCache, Ui};
use bifrost_app::passwords::{self, Error as KeyError};
use bifrost_app::text_services;
use bifrost_app::usernotif::{self, Request};
use bifrost_app::{AppMenu, AppMenuItem, ContextItem, Ctx, DocumentEvent, KeyInput, ModifierFlags, OpenOptions, TouchKind};
use bifrost_app::proto::{DocErrorCode, DocMode, CHORD_ALT, CHORD_CTRL, CHORD_META, CHORD_SHIFT};
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
/// Message > Edit Draft: the open message of the Drafts mailbox opens in a compose sheet.
pub const MENU_EDIT_DRAFT: u32 = 81;
pub const MENU_PROOFREAD: u32 = 82;
pub const MENU_ACCEPT_SUGGESTION: u32 = 83;
pub const MENU_SKIP_SUGGESTION: u32 = 84;
pub const MENU_ACCEPT_ALL_SUGGESTIONS: u32 = 85;
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
pub const MENU_BOLD: u32 = 63;
pub const MENU_ITALIC: u32 = 64;
pub const MENU_UNDERLINE: u32 = 65;
pub const MENU_BULLET: u32 = 66;
pub const MENU_INDENT: u32 = 67;
pub const MENU_OUTDENT: u32 = 68;
pub const MENU_CLEAR_FORMATTING: u32 = 69;
pub const MENU_CHECK_SPELLING: u32 = 70;
pub const MENU_NEXT_MISSPELLING: u32 = 71;
pub const MENU_CORRECT_SPELLING: u32 = 72;
pub const MENU_ACTIVITY: u32 = 73;
pub const MENU_CHECK_CONNECTION: u32 = 74;
pub const MENU_PASTE_MATCH: u32 = 75;
pub const MENU_EXPORT_PDF: u32 = 76;
pub const MENU_ALL_HEADERS: u32 = 77;
pub const MENU_IGNORE_FOLLOW_UP: u32 = 78;
pub const MENU_NEW_VIEWER: u32 = 79;
pub const MENU_RULES_PANE: u32 = 80;
/// Menu ids that run from a base: the nth entry of a list (mailboxes, sort keys, filters, priorities) is `base + n`.
pub const MENU_MOVE_BASE: u32 = 100;
pub const MENU_COPY_BASE: u32 = 200;
pub const MENU_SORT_BASE: u32 = 300;
pub const MENU_FILTER_BASE: u32 = 400;
pub const MENU_PRIORITY_BASE: u32 = 500;
pub const MENU_COLOR_BASE: u32 = 600;
pub const MENU_CATEGORY_BASE: u32 = 700;
pub const MENU_RULE_BASE: u32 = 800;
/// Format > Text Color: the first entry is the ordinary colour, the next ones are [`TextColor::ALL`].
pub const MENU_TEXT_COLOR_BASE: u32 = 900;
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
const KEY_UP: u32 = 38;
const KEY_DOWN: u32 = 40;
/// evdev V, for the paste chord.
const KEY_V: u32 = 47;
/// The evdev codes of the other keys the editing chords name: A, C, X and Backspace.
const KEY_A: u32 = 30;
const KEY_C: u32 = 46;
const KEY_X: u32 = 45;
const KEY_BACKSPACE_CHORD: u32 = 14;
/// The evdev codes of Up and Down Arrow, as they arrive in a chord (Shift-Up, Shift-Down; the manifest's `edit_keys`).
const KEY_UP_CHORD: u32 = 103;
const KEY_DOWN_CHORD: u32 = 108;

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
    /// A person donated to Contacts. Nothing follows its answer.
    Donated,
    /// "Add to Calendar" from the banner: the event being saved.
    EventSave,
    /// "Add to Contacts" from the banner for a phone number: the contact being saved.
    PhoneSave(String),
    /// Recipient suggestions: the answer to the question whether Mail may read the address book.
    BookAccess,
    /// Recipient suggestions: the names and email addresses of the address book.
    BookFetch,
    /// A password item being deleted. Nothing follows its answer.
    Forget,
    /// A question to the spelling checker about `text`, the message as it was asked.
    Spelling { text: String, step: SpellStep },
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

/// What a spelling question is for.
enum SpellStep {
    /// Which words are misspelled.
    Check,
    /// The suggestions for this misspelled word.
    Guesses(String),
    /// The best correction of the word in view.
    Correct,
}

/// What a file panel was opened for.
enum DocTask {
    /// Message > Attach Files: the files chosen go on the message being written.
    Attach,
    /// A save panel: the bytes are written to the place chosen. The name is for the status line.
    Save { name: String, bytes: Vec<u8> },
    /// Mailbox > Import Mailbox: the file chosen is read as a mailbox file.
    ImportMbox,
    /// Message > Save Attachments: the folder chosen gets every file of the message.
    SaveFolder { files: Vec<Attachment> },
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
    /// Accounts with an inbox watcher started, each with the flag that stops it. A watcher runs until its server has no
    /// IDLE, the account is removed or its settings change (see `stop_watcher`).
    watching: HashMap<String, Arc<std::sync::atomic::AtomicBool>>,
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
    /// The Load Remote Content link of the open message, where it was drawn last frame.
    pub remote_rect: Option<Rect>,
    /// The links of the "Found in Mail" banner, where they were drawn last frame.
    pub found_rects: Vec<(Rect, crate::found::FoundHit)>,
    /// How many remote images the open message has, as counted for it (message UID, count).
    pub remote_memo: Option<(u32, usize)>,
    /// An immediate send is on its way from the compose window it was sent from: that window stays the active one until
    /// the answer comes back.
    pub sending: bool,
    /// The sheet slot belongs to a compose window (`compose_windows`): its message fills that window, and `w` x `h` is
    /// that window's size, not the main window's.
    pub in_compose_window: bool,
    /// The Previous, Next and Done buttons of the find bar where they were drawn last frame.
    pub find_hits: Option<[Rect; 3]>,
    /// Where the open message's text starts in the reader's scrolled content, the height of a line, and the height of the
    /// reader, as drawn last frame (for scrolling to a match).
    pub reader_metrics: (f32, f32, f32),
    press: Option<Press>,
    pub labels: TextCache,
    /// The wrapped text of the open message, for the width it was laid out at.
    pub laid: Option<crate::draw::Laid>,
    /// Messages to open in viewer windows; the app takes them with `take_viewers`.
    viewer_requests: Vec<u32>,
    /// The clicks of the main window counted for a double-click (the kit's interval and slop, in milliseconds and
    /// points), and the message the last click landed on: a double-click on one message opens a viewer window.
    clicks: ClickCounter,
    last_tap: Option<u32>,
    /// The address book was asked for while the compose sheet is open (asked again for the next compose sheet).
    book_asked: bool,
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

/// True when the open message's text has arrived (Forward quotes it).
fn open_body_loaded(mail: &Mail) -> bool {
    mail.open.is_some_and(|uid| mail.bodies.contains_key(&uid))
}

/// True when the open message carries files.
fn open_has_files(mail: &Mail) -> bool {
    mail.open.and_then(|uid| mail.attachments.get(&uid)).is_some_and(|files| !files.is_empty())
}

/// What a right-click on a message offers, in the order of macOS Mail's message context menu: the Message and Move menu
/// items that act on one message (`None` is a separator line).
const CONTEXT_MENU: [Option<u32>; 21] = [
    Some(MENU_NEW_VIEWER),
    None,
    Some(MENU_REPLY),
    Some(MENU_REPLY_ALL),
    Some(MENU_FORWARD),
    Some(MENU_FORWARD_ATTACHMENT),
    Some(MENU_REDIRECT),
    None,
    Some(MENU_UNREAD),
    Some(MENU_FLAG),
    Some(MENU_MUTE),
    Some(MENU_REMIND),
    None,
    Some(MENU_ARCHIVE),
    Some(MENU_TRASH),
    Some(MENU_JUNK),
    Some(MENU_MOVE_AGAIN),
    None,
    Some(MENU_ADD_TO_CONTACTS),
    Some(MENU_SAVE_AS),
    Some(MENU_PRINT),
];

/// The context menu of the open message: the rows of [`CONTEXT_MENU`] with the titles and enabled state the menu bar
/// has for the same state (Mute or Unmute, Move Again to the last mailbox), so a choice runs the same menu action.
pub fn context_items(mail: &Mail, undo_pending: bool) -> Vec<ContextItem> {
    let menus = menus(mail, undo_pending);
    let item = |id: u32| menus.iter().flat_map(|m| &m.items).find(|i| i.id == id && !i.separator && !i.submenu);
    CONTEXT_MENU
        .iter()
        .filter_map(|row| match row {
            None => Some(ContextItem::separator()),
            Some(id) => item(*id).map(|i| {
                let title = if *id == MENU_NEW_VIEWER { "Open in New Window" } else { i.title.as_str() };
                ContextItem::new(*id, title).enabled(i.enabled)
            }),
        })
        .collect()
}

/// The menus of the window, for the state in `mail`. `undo_pending` is true while a sent message can still be taken back.
/// An app menu holds at most 48 items and an app has at most 8 menus (the tests keep to that).
pub fn menus(mail: &Mail, undo_pending: bool) -> Vec<AppMenu> {
    let compose_open = matches!(mail.sheet, Some(Sheet::Compose(_)));
    let open = mail.open.is_some();
    let loaded = open && open_body_loaded(mail);
    let targets = mail.move_targets();
    let shown_targets = &targets[..targets.len().min(MENU_TARGETS_SHOWN)];
    let mut message = vec![
        AppMenuItem::new(MENU_NEW, "New Message").key("cmd+n"),
        AppMenuItem::new(MENU_NEW_VIEWER, "New Viewer Window").enabled(open),
        AppMenuItem::new(MENU_REPLY, "Reply").key("cmd+shift+r").enabled(open),
        AppMenuItem::new(MENU_REPLY_ALL, "Reply All").key("cmd+alt+r").enabled(open_has_others(mail)),
        AppMenuItem::new(MENU_FORWARD, "Forward").key("cmd+shift+f").enabled(loaded),
        AppMenuItem::new(MENU_FORWARD_ATTACHMENT, "Forward as Attachment").enabled(loaded),
        AppMenuItem::new(MENU_REDIRECT, "Redirect...").enabled(loaded),
        AppMenuItem::new(MENU_EDIT_DRAFT, "Edit Draft").enabled(loaded && mail.is_drafts()),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_SEND, "Send").key("cmd+shift+d").enabled(compose_open),
        AppMenuItem::new(MENU_SEND_LATER, "Send Later").enabled(compose_open).checked(mail.send_later_shown()),
        AppMenuItem::new(MENU_UNDO_SEND, "Undo Send").enabled(undo_pending),
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
        AppMenuItem::new(MENU_IGNORE_FOLLOW_UP, "Ignore Follow-Up").enabled(mail.open.is_some_and(|uid| mail.has_reminder(uid))),
        AppMenuItem::new(MENU_ARCHIVE, "Archive").key("cmd+shift+a").enabled(open),
        AppMenuItem::new(MENU_TRASH, "Move to Trash").enabled(open),
        AppMenuItem::new(MENU_JUNK, "Move to Junk").key("cmd+shift+j").enabled(open),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_ADD_TO_CONTACTS, "Add Sender to Contacts").enabled(open),
        AppMenuItem::new(MENU_REMOVE_ATTACHMENTS, "Remove Attachments").enabled(open_has_files(mail)),
        AppMenuItem::new(MENU_SAVE, "Save Attachments...").key("cmd+alt+a").enabled(open_has_files(mail)),
        AppMenuItem::new(MENU_SAVE_AS, "Save As...").key("cmd+shift+s").enabled(loaded),
        AppMenuItem::new(MENU_EXPORT_PDF, "Export as PDF...").enabled(loaded),
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
        AppMenuItem::new(MENU_ACTIVITY, "Show Activity"),
        AppMenuItem::new(MENU_CHECK_CONNECTION, "Check Connection").enabled(!mail.accounts.is_empty()),
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
        AppMenuItem::new(MENU_RULES_PANE, "Rules...").depth(1).enabled(!mail.accounts.is_empty()),
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
        AppMenuItem::new(MENU_ALL_HEADERS, "All Headers").checked(mail.prefs.all_headers).enabled(open),
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

    let compose = match &mail.sheet {
        Some(Sheet::Compose(compose)) => Some(compose),
        _ => None,
    };
    let typing = compose.map(|c| c.typing).unwrap_or_default();
    let mut format = vec![
        AppMenuItem::new(MENU_BOLD, "Bold").key("cmd+b").checked(typing.bold).enabled(compose_open),
        AppMenuItem::new(MENU_ITALIC, "Italic").key("cmd+i").checked(typing.italic).enabled(compose_open),
        AppMenuItem::new(MENU_UNDERLINE, "Underline").key("cmd+u").checked(typing.underline).enabled(compose_open),
        AppMenuItem::submenu("Text Color").enabled(compose_open),
        AppMenuItem::new(MENU_TEXT_COLOR_BASE, "Default").depth(1).checked(typing.color.is_none()),
    ];
    for (i, color) in TextColor::ALL.into_iter().enumerate() {
        format.push(AppMenuItem::new(MENU_TEXT_COLOR_BASE + 1 + i as u32, color.label()).depth(1).checked(typing.color == Some(color)));
    }
    format.extend([
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_BULLET, "Bulleted List").enabled(compose_open),
        AppMenuItem::new(MENU_INDENT, "Increase Indent").key("cmd+]").enabled(compose_open),
        AppMenuItem::new(MENU_OUTDENT, "Decrease Indent").key("cmd+[").enabled(compose_open),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_CLEAR_FORMATTING, "Clear Formatting").enabled(compose_open),
    ]);
    let marked = compose.is_some_and(|c| !c.misspelled.is_empty());
    let proofing = compose.is_some_and(|c| c.proofread.is_some());
    let edit = vec![
        AppMenuItem::new(MENU_PASTE_QUOTE, "Paste as Quotation").key("cmd+alt+v").enabled(compose_open),
        AppMenuItem::new(MENU_PASTE_MATCH, "Paste and Match Style").enabled(compose_open),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_CHECK_SPELLING, "Check Spelling").enabled(compose_open),
        AppMenuItem::new(MENU_NEXT_MISSPELLING, "Next Misspelling").enabled(marked),
        AppMenuItem::new(MENU_CORRECT_SPELLING, "Correct Spelling").enabled(marked),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_PROOFREAD, "Proofread").enabled(compose_open),
        AppMenuItem::new(MENU_ACCEPT_SUGGESTION, "Accept Suggestion").enabled(proofing),
        AppMenuItem::new(MENU_SKIP_SUGGESTION, "Skip Suggestion").enabled(proofing),
        AppMenuItem::new(MENU_ACCEPT_ALL_SUGGESTIONS, "Accept All Suggestions").enabled(proofing),
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
        AppMenu { title: "Format".into(), items: format },
        AppMenu { title: "Writing".into(), items: edit },
    ]
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
        let (queue, queue_note) = schedule::load_checked(&queue_path);
        mail.queue = queue;
        if let Some(e) = load_error.or(import_error).or(queue_note) {
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
            watching: HashMap::new(),
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
            remote_rect: None,
            found_rects: Vec::new(),
            remote_memo: None,
            sending: false,
            in_compose_window: false,
            find_hits: None,
            reader_metrics: (0.0, 0.0, 0.0),
            press: None,
            labels: TextCache::default(),
            laid: None,
            viewer_requests: Vec::new(),
            clicks: ClickCounter::default(),
            last_tap: None,
            book_asked: false,
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
    /// compliance-allow: RES-02 The 60-second fallback checks IMAP IDLE health; workers also signal the wake fd.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.list_scroll.animating() || self.reader_scroll.animating() {
            return Some(t + 16.0);
        }
        let mut wake = self.next_poll;
        if let Some(due) = self.mail.queue.next_due() {
            // compliance-allow: LAY-03 Converts a queued reminder's seconds to the event loop's milliseconds.
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
        self.ensure_address_book(ctx);
        self.read_open_message(ctx);
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
                Err(KeyError::DuplicateItem) if self.password_shared(&account) => {
                    // Another account keeps its password in this item: replacing it would change that account's login too
                    self.mail.status = format!(
                        "Another account already uses {} on {}. Its saved password is kept; change it in that account's settings",
                        account.username,
                        account.password_site()
                    );
                }
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
            Pending::Spelling { text, step } => self.spelling_reply(ctx, text, step, result),
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
            Pending::Donated => {}
            Pending::EventSave => {
                self.mail.status = match result {
                    Ok(body) if calendar::decode_saved_id(body).is_some() => "Added to Calendar".to_string(),
                    Ok(_) => "The event could not be saved".to_string(),
                    Err(e) => format!("The event could not be saved: {e}"),
                };
            }
            Pending::PhoneSave(phone) => {
                self.mail.status = match result {
                    Ok(_) => format!("Added {phone} to Contacts"),
                    Err(e) => format!("The contact could not be saved: {e}"),
                };
            }
            Pending::BookAccess => {
                // Mail reads the address book only when the person let it: an undecided Mail has the access sheet queued
                // by the request, and the suggestions then come from the recent recipients alone
                let reads = result.as_ref().ok().and_then(contacts::decode_status).is_some_and(|status| matches!(status, contacts::Status::Authorized | contacts::Status::Limited));
                if reads {
                    let request = contacts::fetch(ctx, &[ContactKey::GivenName, ContactKey::FamilyName, ContactKey::Organization, ContactKey::Emails], None);
                    self.pending.insert(request, Pending::BookFetch);
                }
            }
            Pending::BookFetch => {
                if let Some(records) = result.as_ref().ok().and_then(contacts::decode_contacts) {
                    self.mail.set_address_book(records.iter().map(people::book_entry).collect());
                }
            }
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

    /// A press, drag or release at (`x`, `y`) with the modifier keys held for it: a click on a message with Cmd toggles it
    /// in the selection and with Shift extends the selection from the anchor (`NSEvent.modifierFlags`).
    pub fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32, t: f64, mods: ModifierFlags) -> bool {
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
                self.tap(ctx, x, y, t, mods)
            }
        }
    }

    /// `scrollWheel:` at (`x`, `y`): the wheel moved the content `dy` pixels (negative for a wheel turned down, which
    /// shows what is below). The pane under the pointer scrolls (the message list or the reader); with a compose sheet
    /// in the slot, its message scrolls. True when something moved.
    pub fn wheel(&mut self, ui: &Ui, x: f32, y: f32, dy: f32) -> bool {
        let (w, h) = (self.w as f32, self.h as f32);
        let moved = if self.mail.sheet.is_none() {
            let region = self.region(ui, x, y);
            self.scroller(region).is_some_and(|scroll| scroll.wheel(dy))
        } else if let Some(Sheet::Compose(compose)) = &mut self.mail.sheet {
            crate::draw::scroll_message(ui, compose, w, h, self.in_compose_window, dy)
        } else {
            false
        };
        self.dirty |= moved;
        moved
    }

    pub fn key(&mut self, ctx: &Ctx, k: KeyInput) -> bool {
        if let Some((key, mods)) = k.as_chord() {
            return self.chord(&ctx.ui, key, mods);
        }
        if self.mail.sheet.is_none() {
            // Up and Down Arrow move the selection in the message list (the find bar takes no arrows)
            if let KeyInput::Code(code @ (KEY_UP | KEY_DOWN)) = k {
                return self.step_selection(&ctx.ui, code == KEY_DOWN);
            }
            return self.find_key(k);
        }
        self.ensure_address_book(ctx);
        // the suggestions under a recipient field: the arrows move through them, Return and Tab take the highlighted
        // one, Escape closes the list
        match &k {
            KeyInput::Code(KEY_UP | KEY_DOWN) => {
                let moved = self.mail.move_recipient_highlight(matches!(k, KeyInput::Code(KEY_DOWN)));
                self.dirty |= moved;
                return moved;
            }
            KeyInput::Code(KEY_RETURN | KEY_TAB) if self.mail.recipient_field.highlighted().is_some() && self.mail.accept_recipient(None) => {
                self.dirty = true;
                return true;
            }
            KeyInput::Code(KEY_ESCAPE) if self.mail.close_recipient_list() => {
                self.dirty = true;
                return true;
            }
            _ => {}
        }
        let multiline = self.focus_is_multiline();
        let picker = self.focus_is_picker();
        match k {
            KeyInput::Text(s) => {
                let text: String = s.chars().filter(|c| !c.is_control() || (*c == '\n' && multiline)).collect();
                if !picker {
                    self.edit_form(|form| form.type_text(&text));
                } else if text == " " {
                    // Space turns a yes or no field, as a click does
                    if let Some(form) = self.form_mut() {
                        form.flip(form.focus);
                    }
                    self.mail.sync_manual();
                }
            }
            KeyInput::Code(KEY_RETURN) if multiline && !picker => self.edit_form(|form| form.type_text("\n")),
            KeyInput::Code(KEY_RETURN) => self.submit(ctx),
            KeyInput::Code(KEY_TAB) => {
                if let Some(form) = self.form_mut() {
                    form.next_field();
                }
            }
            KeyInput::Code(KEY_BACKSPACE) => {
                if !picker {
                    self.edit_form(|form| form.backspace());
                }
            }
            KeyInput::Code(KEY_ESCAPE) => self.cancel_sheet(),
            KeyInput::Code(_) => return false,
        }
        self.mail.sync_recipient_field();
        self.dirty = true;
        true
    }

    /// Up Arrow (`down` false) or Down Arrow in the message list: the message above or below the selected one is
    /// selected and opened, and the list scrolls to keep it in view. False at either end of the list.
    fn step_selection(&mut self, ui: &Ui, down: bool) -> bool {
        let Some((index, uid)) = self.mail.neighbour(down) else { return false };
        self.open_message(uid);
        self.scroll_list_to(ui, index);
        self.dirty = true;
        true
    }

    /// Shift-Up (`down` false) or Shift-Down Arrow: the selection grows or shrinks by one line from the anchor, and the
    /// list scrolls to keep the moving end in view. False at either end of the list.
    fn extend_by_arrow(&mut self, ui: &Ui, down: bool) -> bool {
        let Some((index, fetch)) = self.mail.extend_by_arrow(down) else { return false };
        self.shown_changed(fetch);
        self.scroll_list_to(ui, index);
        self.dirty = true;
        true
    }

    /// Scrolls the message list just far enough that line `index` is in view.
    fn scroll_list_to(&mut self, ui: &Ui, index: usize) {
        let offset = self.list_scroll.offset();
        let wanted = crate::draw::list_offset_showing(ui, &self.mail, self.h as f32, index, offset);
        if wanted != offset {
            self.list_scroll.jump_to(wanted);
        }
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

    /// A chord for the field of the open sheet: Cmd or Ctrl with V, C or X, Cmd+A (select all), Option or Ctrl with Backspace
    /// (the word before the caret) and Cmd+Backspace (everything before it). The shell sends Select All, Copy, Cut and Paste as
    /// commands when it can (`Mail::commands`); the chords are what arrives when it cannot.
    fn chord(&mut self, ui: &Ui, key: u32, mods: u32) -> bool {
        let primary = mods & (CHORD_CTRL | CHORD_META) != 0;
        if key == KEY_V && primary {
            return self.paste();
        }
        if self.mail.sheet.is_none() {
            // Shift-Up and Shift-Down Arrow extend the selection in the message list from its anchor
            if matches!(key, KEY_UP_CHORD | KEY_DOWN_CHORD) && mods == CHORD_SHIFT {
                return self.extend_by_arrow(ui, key == KEY_DOWN_CHORD);
            }
            return false;
        }
        if key == KEY_A && mods & CHORD_META != 0 {
            return self.select_all();
        }
        if key == KEY_C && primary {
            return self.copy();
        }
        if key == KEY_X && primary {
            return self.cut();
        }
        if key == KEY_BACKSPACE_CHORD && mods & (CHORD_ALT | CHORD_CTRL) != 0 {
            return self.edit_field(|form| form.delete_word());
        }
        if key == KEY_BACKSPACE_CHORD && mods & CHORD_META != 0 {
            return self.edit_field(|form| form.delete_to_start());
        }
        false
    }

    /// An edit of the sheet's focused field that is not typing; false for the From picker, which takes no text.
    fn edit_field(&mut self, change: impl FnOnce(&mut crate::state::Form)) -> bool {
        if self.mail.sheet.is_none() || self.focus_is_picker() {
            return false;
        }
        self.edit_form(change);
        self.mail.sync_recipient_field();
        self.dirty = true;
        true
    }

    /// Edit > Select All in the open sheet: the whole text of the focused field is selected, so typing, Backspace or Paste
    /// replaces it (an Add Account form retyped from the start). False when no field with text has the focus.
    pub fn select_all(&mut self) -> bool {
        // with no sheet, Select All selects every message the list shows
        if self.mail.sheet.is_none() {
            if self.mail.visible_lines().is_empty() {
                return false;
            }
            let fetch = self.mail.select_all_messages();
            self.shown_changed(fetch);
            self.dirty = true;
            return true;
        }
        if self.focus_is_picker() {
            return false;
        }
        let done = self.form_mut().is_some_and(|form| form.select_all());
        self.dirty |= done;
        done
    }

    /// Edit > Copy: the selected text of the focused field goes on the clipboard. A password is never copied. Nothing changes
    /// on screen, so the window needs no frame.
    pub fn copy(&mut self) -> bool {
        if let Some(text) = self.form_mut().and_then(|form| form.selected_text().map(str::to_string)) {
            clipboard::set_text(&text);
        }
        false
    }

    /// Edit > Cut: the selected text of the focused field goes on the clipboard and leaves the field.
    pub fn cut(&mut self) -> bool {
        if self.focus_is_picker() {
            return false;
        }
        let mut cut = None;
        self.edit_form(|form| cut = form.cut());
        let Some(text) = cut else { return false };
        clipboard::set_text(&text);
        self.mail.sync_recipient_field();
        self.dirty = true;
        true
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
        self.edit_form(|form| form.type_text(&text));
        self.mail.sync_recipient_field();
        self.dirty = true;
        true
    }

    /// Runs an edit on the field that takes typed text. In the compose sheet the styles of the message follow along.
    fn edit_form(&mut self, change: impl FnOnce(&mut crate::state::Form)) {
        match self.mail.sheet.as_mut() {
            Some(Sheet::Compose(compose)) => compose.edit(change),
            Some(sheet) => change(sheet.form_mut()),
            None => {}
        }
    }

    /// Runs a formatting change on the compose sheet that is open.
    fn format(&mut self, change: impl FnOnce(&mut Compose)) {
        if let Some(Sheet::Compose(compose)) = self.mail.sheet.as_mut() {
            change(compose);
        }
    }

    /// The text of the message being written.
    fn compose_message(&self) -> Option<String> {
        match &self.mail.sheet {
            Some(Sheet::Compose(compose)) => Some(compose.message().to_string()),
            _ => None,
        }
    }

    /// Edit > Check Spelling: the message goes to the system's spelling checker, and the words it finds are marked.
    fn check_spelling(&mut self, ctx: &Ctx) {
        let Some(text) = self.compose_message() else {
            self.mail.status = "Start a message first".into();
            return;
        };
        if text.trim().is_empty() {
            self.mail.status = "There is nothing to check yet".into();
            return;
        }
        let request = text_services::check(ctx, &text);
        self.pending.insert(request, Pending::Spelling { text, step: SpellStep::Check });
        self.mail.status = "Checking spelling".into();
    }

    /// Writing > Proofread: the writing assistant reads the message on the device and marks what it would change. The status
    /// line says what the suggestion in view does; Accept Suggestion makes the change, Skip Suggestion leaves it.
    fn proofread_message(&mut self) {
        let mut found = None;
        self.format(|compose| found = Some(compose.start_proofread()));
        match found {
            None => self.mail.status = "Start a message first".into(),
            Some(0) => self.mail.status = "No suggestions".into(),
            Some(_) => self.proofread_status(),
        }
    }

    /// Writing > Accept Suggestion and Accept All Suggestions.
    fn accept_suggestion(&mut self, all: bool) {
        let mut made = 0;
        self.format(|compose| made = compose.accept_suggestion(all));
        if made == 0 {
            self.mail.status = "No suggestion to accept".into();
        } else if self.proofread_left() {
            self.proofread_status();
        } else {
            self.mail.status = format!("{made} change{} made", if made == 1 { "" } else { "s" });
        }
    }

    fn proofread_left(&self) -> bool {
        matches!(&self.mail.sheet, Some(Sheet::Compose(compose)) if compose.proofread.is_some())
    }

    /// The status line for the suggestion in view, or the end of the list.
    fn proofread_status(&mut self) {
        let line = match &self.mail.sheet {
            Some(Sheet::Compose(compose)) => compose.proofread_line(),
            _ => None,
        };
        self.mail.status = line.unwrap_or_else(|| "No more suggestions".into());
    }

    /// Asks the checker for suggestions for the misspelled word in view, to show them in the status line.
    fn ask_suggestions(&mut self, ctx: &Ctx) {
        let Some((text, word, bytes)) = self.misspelling_in_view() else { return };
        let request = text_services::guesses(ctx, &text, bytes);
        self.pending.insert(request, Pending::Spelling { text, step: SpellStep::Guesses(word) });
    }

    /// Edit > Correct Spelling: asks the checker for the best correction of the word in view.
    fn correct_spelling(&mut self, ctx: &Ctx) {
        let Some((text, _, bytes)) = self.misspelling_in_view() else {
            self.mail.status = "Check the spelling first".into();
            return;
        };
        let request = text_services::correction(ctx, &text, bytes);
        self.pending.insert(request, Pending::Spelling { text, step: SpellStep::Correct });
    }

    /// The message, the misspelled word in view, and where the word is in the message in bytes.
    fn misspelling_in_view(&self) -> Option<(String, String, std::ops::Range<usize>)> {
        let Some(Sheet::Compose(compose)) = &self.mail.sheet else { return None };
        let (range, word) = compose.current_misspelling()?;
        let text = compose.message().to_string();
        let bytes = spelling::byte_range(&text, &range)?;
        Some((text, word, bytes))
    }

    /// The checker's answer. It counts only while the message is still the text it was asked about.
    fn spelling_reply(&mut self, ctx: &Ctx, text: String, step: SpellStep, result: &Result<Json, String>) {
        let reply = match result {
            Ok(reply) => reply,
            Err(e) => {
                self.mail.status = format!("Spelling: {e}");
                return;
            }
        };
        if self.compose_message().as_deref() != Some(text.as_str()) {
            self.mail.status = "The message changed. Check the spelling again".into();
            return;
        }
        match step {
            SpellStep::Check => {
                let ranges = spelling::char_ranges(&text, &spelling::byte_ranges(reply));
                let count = ranges.len();
                self.format(|compose| compose.set_misspelled(ranges));
                if count == 0 {
                    self.mail.status = "No spelling mistakes found".into();
                } else {
                    self.mail.status = format!("{count} possible misspelling{}. Edit > Next Misspelling steps through them", if count == 1 { "" } else { "s" });
                    self.ask_suggestions(ctx);
                }
            }
            SpellStep::Guesses(word) => {
                let guesses = spelling::words(reply);
                self.mail.status = if guesses.is_empty() { format!("{word}: no suggestions") } else { format!("{word}: {}", guesses.join(", ")) };
            }
            SpellStep::Correct => match reply.as_str() {
                Some(word) => {
                    let word = word.to_string();
                    let mut done = false;
                    self.format(|compose| done = compose.correct_misspelling(&word));
                    if done {
                        self.check_spelling(ctx);
                    }
                }
                None => self.mail.status = "No correction found for that word".into(),
            },
        }
    }

    /// A `mailto:` link handed to Mail (the system compose path, or Open With): its fields fill a new message. A link
    /// without a valid address is reported in the status line.
    pub fn open_handoff(&mut self, url: &str) {
        let Some(handoff) = bifrost_app::controls::compose_handoff::Compose::from_url(url) else {
            self.mail.status = "That link is not a mail address".to_string();
            return;
        };
        let from = self.mail.current().map(|a| a.email.clone()).unwrap_or_default();
        self.mail.open_draft(crate::message::Draft {
            from,
            to: handoff.to,
            cc: handoff.cc,
            bcc: handoff.bcc,
            subject: handoff.subject,
            body: handoff.body,
            ..crate::message::Draft::default()
        });
        self.dirty = true;
    }

    /// Cancel on a sheet: a question goes back to the message it asked about; any other sheet closes.
    fn cancel_sheet(&mut self) {
        match self.mail.sheet.take() {
            Some(Sheet::Question(compose)) => self.mail.sheet = Some(Sheet::Compose(compose)),
            _ => self.mail.close_sheet(),
        }
    }

    /// Save This Message: the message is saved to Drafts and its window closes.
    fn save_question(&mut self, compose: &Compose) {
        self.save_draft(compose);
        self.mail.sheet_done = true;
        self.mail.close_sheet();
    }

    /// Don't Save: the message and its saved copy in Drafts are dropped, and the window closes.
    fn discard_question(&mut self) {
        if let Some(Sheet::Question(compose)) = self.mail.sheet.clone() {
            self.discard_draft(&compose);
        }
        self.mail.sheet_done = true;
        self.mail.close_sheet();
    }

    /// A Message-ID for a new draft.
    pub fn new_draft_id(&mut self) -> String {
        self.seq += 1;
        let domain = self.mail.current().map_or("localhost", |a| a.email.rsplit_once('@').map_or("localhost", |(_, d)| d)).to_string();
        message::new_message_id(&domain, now_unix(), self.seq)
    }

    /// Saves `compose` to Drafts under its draft id: IMAP (the Drafts mailbox, replacing the older copy) when the account
    /// signs in, else a file in the data directory.
    pub fn save_draft(&mut self, compose: &Compose) {
        let key = compose.draft_id.clone();
        let raw = match self.mail.draft_for_save(compose).render(&key, now_unix()) {
            Ok(raw) => raw,
            Err(e) => {
                self.mail.status = e;
                return;
            }
        };
        let Some(account) = self.mail.current().cloned() else {
            self.save_draft_locally(&key, &raw);
            return;
        };
        let Some(password) = self.mail.password(&account).map(str::to_string) else {
            self.save_draft_locally(&key, &raw);
            return;
        };
        self.jobs += 1;
        let transport = self.transport.clone();
        let (job_key, job_raw) = (key.clone(), raw.clone());
        sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::draft_job(transport.as_ref(), &account, &password, &job_key, Some(&job_raw)));
    }

    /// Removes the saved copy of `compose` from Drafts (and the file kept here), when it has one.
    pub fn discard_draft(&mut self, compose: &Compose) {
        let key = compose.draft_id.clone();
        if key.is_empty() {
            return;
        }
        let _ = std::fs::remove_file(self.draft_file(&key));
        let Some(account) = self.mail.current().cloned() else { return };
        let Some(password) = self.mail.password(&account).map(str::to_string) else { return };
        self.jobs += 1;
        let transport = self.transport.clone();
        sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::draft_job(transport.as_ref(), &account, &password, &key, None));
    }

    /// The answer of a draft job: a saved copy is no longer needed here; a failed save keeps the text on this computer.
    fn draft_saved(&mut self, key: String, raw: Option<String>, result: Result<(), String>) {
        match (result, raw) {
            (Ok(()), _) => {
                let _ = std::fs::remove_file(self.draft_file(&key));
            }
            (Err(e), Some(raw)) => {
                self.note(format!("The draft could not be saved to Drafts ({e}); it is kept on this computer"));
                self.save_draft_locally(&key, &raw);
            }
            (Err(e), None) => self.note(format!("The draft could not be removed from Drafts: {e}")),
        }
    }

    fn save_draft_locally(&mut self, key: &str, raw: &str) {
        let dir = self.data_dir.join("drafts");
        match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(self.draft_file(key), raw)) {
            Ok(()) => self.note("A draft was saved on this computer".to_string()),
            Err(e) => self.mail.status = format!("The draft could not be saved: {e}"),
        }
    }

    fn draft_file(&self, key: &str) -> std::path::PathBuf {
        let name: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect();
        self.data_dir.join("drafts").join(format!("{name}.eml"))
    }

    /// Remove in the Rules pane: the rule goes, and the pane lists the rest.
    fn remove_rule(&mut self, index: usize) {
        if index < self.mail.prefs.rules.len() {
            self.mail.prefs.rules.remove(index);
            self.save_prefs();
            self.mail.open_rules_pane();
        }
    }

    /// Save on the rule editor of the rule at `index`: the rule takes the form's values.
    fn submit_edit_rule(&mut self, index: usize, form: &crate::state::Form) {
        match self.mail.rule_from_form_at(form, Some(index)) {
            Ok(rule) => {
                if let Some(slot) = self.mail.prefs.rules.get_mut(index) {
                    *slot = rule;
                }
                self.save_prefs();
                self.mail.close_sheet();
            }
            Err(e) => self.mail.status = e,
        }
    }

    /// Load Remote Content: the open message's remote content may load.
    fn load_remote(&mut self) {
        if let Some(uid) = self.mail.open {
            self.mail.allow_remote(uid);
            self.dirty = true;
        }
    }

    /// File > New Viewer Window: the open message goes into a viewer window of its own.
    fn new_viewer(&mut self) {
        if let Some(uid) = self.mail.open {
            self.viewer_requests.push(uid);
        }
    }

    /// The messages asked for viewer windows since the last call, copied for the viewers.
    pub fn take_viewers(&mut self) -> Vec<crate::viewer::Snapshot> {
        let uids = std::mem::take(&mut self.viewer_requests);
        uids.into_iter().map(|uid| self.viewer_snapshot(uid)).collect()
    }

    /// The copy of one message a viewer window shows. `loaded` is false until its body is fetched.
    pub fn viewer_snapshot(&self, uid: u32) -> crate::viewer::Snapshot {
        let summary = self.mail.rows.iter().find(|r| r.uid == uid).map(|r| r.summary.clone()).unwrap_or_default();
        let raw = self.mail.bodies.get(&uid);
        let body = raw.map(|raw| crate::body::readable(raw)).unwrap_or_default();
        let files = self
            .mail
            .attachments
            .get(&uid)
            .map(|files| files.iter().map(|a| format!("{} ({})", a.name, attachments::size_text(a.bytes.len()))).collect())
            .unwrap_or_default();
        crate::viewer::Snapshot {
            uid,
            subject: summary.subject,
            from: summary.from,
            to: summary.to,
            date: summary.date,
            body,
            files,
            loaded: raw.is_some(),
        }
    }

    /// A right-click at (`x`, `y`): on a message, it becomes the selection (as in macOS Mail) and its context menu opens
    /// there; the choice comes back as a menu action (`Input::ContextAction`). False anywhere else or under a sheet.
    pub fn context(&mut self, ctx: &Ctx, x: f32, y: f32) -> bool {
        if self.mail.sheet.is_some() {
            return false;
        }
        let crate::draw::Hit::Message(uid) = self.hit(&ctx.ui, x, y) else { return false };
        // a right-click inside a selection of several keeps it, so the menu acts on all of them
        if !self.mail.is_selected(uid) {
            self.open_message(uid);
        }
        ctx.context_menu(x, y, context_items(&self.mail, self.undo_pending()));
        self.dirty = true;
        true
    }

    pub fn menu_items(&self) -> Vec<AppMenu> {
        menus(&self.mail, self.undo_pending())
    }

    pub fn menu(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_NEW => self.compose(),
            MENU_REPLY => self.reply(),
            MENU_REPLY_ALL => self.reply_all(),
            MENU_FORWARD => self.forward(),
            MENU_FORWARD_ATTACHMENT => self.forward_attachment(),
            MENU_REDIRECT => self.redirect(),
            MENU_EDIT_DRAFT => self.edit_draft(),
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
            MENU_NEW_VIEWER => self.new_viewer(),
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
            MENU_SHOW_CC | MENU_SHOW_BCC | MENU_SHOW_REPLY_TO => {
                match id {
                    MENU_SHOW_CC => self.mail.set_address_field("Cc", !self.mail.show_cc),
                    MENU_SHOW_BCC => self.mail.set_address_field("Bcc", !self.mail.show_bcc),
                    _ => self.mail.set_address_field("Reply-To", !self.mail.show_reply_to),
                }
                // the choice is remembered for the next run
                self.save_prefs();
            }
            MENU_PASTE_QUOTE => return self.paste_quoted(),
            // a plain paste already takes the style of the text typed so far, which is what this command means
            MENU_PASTE_MATCH => return self.paste(),
            MENU_EXPORT_PDF => self.export_pdf(ctx),
            MENU_ALL_HEADERS => {
                self.mail.prefs.all_headers = !self.mail.prefs.all_headers;
                self.laid = None;
                self.reader_scroll = Scroller::new();
                self.save_prefs();
            }
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
            MENU_IGNORE_FOLLOW_UP => self.ignore_follow_up(),
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
            MENU_RULES_PANE => {
                if self.sheet_free() {
                    self.mail.open_rules_pane();
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
            MENU_ACTIVITY => {
                if self.sheet_free() {
                    self.mail.open_activity();
                }
            }
            MENU_CHECK_CONNECTION => self.check_connection(),
            MENU_ALL_CATEGORIES => self.mail.category = None,
            MENU_BOLD => self.format(Compose::toggle_bold),
            MENU_ITALIC => self.format(Compose::toggle_italic),
            MENU_UNDERLINE => self.format(Compose::toggle_underline),
            MENU_BULLET => self.format(Compose::toggle_bullet),
            MENU_INDENT => self.format(|compose| compose.indent(true)),
            MENU_OUTDENT => self.format(|compose| compose.indent(false)),
            MENU_CLEAR_FORMATTING => self.format(Compose::clear_formatting),
            MENU_CHECK_SPELLING => self.check_spelling(ctx),
            MENU_NEXT_MISSPELLING => {
                self.format(Compose::next_misspelling);
                self.ask_suggestions(ctx);
            }
            MENU_CORRECT_SPELLING => self.correct_spelling(ctx),
            MENU_PROOFREAD => self.proofread_message(),
            MENU_ACCEPT_SUGGESTION => self.accept_suggestion(false),
            MENU_ACCEPT_ALL_SUGGESTIONS => self.accept_suggestion(true),
            MENU_SKIP_SUGGESTION => {
                self.format(Compose::skip_suggestion);
                self.proofread_status();
            }
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
                } else if let Some(i) = menu_index(id, MENU_TEXT_COLOR_BASE) {
                    match i {
                        0 => self.format(|compose| compose.set_text_color(None)),
                        _ => match TextColor::ALL.get(i - 1) {
                            Some(color) => self.format(|compose| compose.set_text_color(Some(*color))),
                            None => return false,
                        },
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
                    let count = listing.messages.len();
                    self.note(format!("{}: {mailbox} read, {count} message{}", self.account_name(&account), if count == 1 { "" } else { "s" }));
                    self.mail.drafts_mailbox = listing.drafts.clone();
                    let notices = self.mail.apply_listing_of(&account, &mailbox, listing.uid_validity, listing.mailboxes, listing.messages);
                    // back in a mailbox: the message it had selected is selected and opened again
                    if let Some(uid) = self.mail.reopen.take() {
                        self.open_message(uid);
                        if let Some(index) = self.mail.visible_lines().iter().position(|l| l.row.uid == uid) {
                            self.scroll_list_to(&ctx.ui, index);
                        }
                    }
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
                Err(e) => {
                    self.note(format!("{}: {mailbox} could not be read: {e}", self.account_name(&account)));
                    self.mail.status = format!("{}: {e}", self.account_name(&account));
                }
            },
            Event::Changed { account, uid, result, .. } => {
                // the change is shown at once; a refusal brings the listing back from the server. A change to a
                // whole mailbox (UID 0) is listed again when it is done, which also reloads the mailbox names.
                match result {
                    Err(e) => {
                        self.note(format!("{}: a change was refused: {e}", self.account_name(&account)));
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
                let name = self.account_name(&account);
                if live {
                    self.note(format!("{name}: waiting for new mail"));
                    self.live.insert(account);
                } else {
                    self.note(format!("{name}: stopped waiting for new mail"));
                    self.live.remove(&account);
                }
            }
            Event::Arrived { account } => {
                self.note(format!("{}: new mail", self.account_name(&account)));
                self.refresh_account(&account);
            }
            Event::Diagnosed { account, lines } => {
                let name = self.account_name(&account);
                for line in lines {
                    self.note(format!("{name}: {line}"));
                }
                self.mail.status = "Connection checked. Mailbox > Show Activity has the details".into();
            }
            Event::Discovered { email, result } => self.discovered(ctx, email, result),
            Event::DraftSaved { key, raw, result, .. } => self.draft_saved(key, raw, result),
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
        if id == 0 {
            self.sending = false;
        }
        let flight = self.in_flight.remove(&id);
        match &result {
            Ok(()) => self.note("A message was sent".to_string()),
            Err(e) => self.note(format!("A message was not sent: {e}")),
        }
        match result {
            Ok(()) => {
                if id == 0 {
                    if matches!(self.mail.sheet, Some(Sheet::Compose(_))) {
                        self.mail.sheet_done = true;
                        self.mail.close_sheet();
                    }
                } else {
                    self.drafts.remove(&id);
                    if self.undo == Some(id) {
                        self.undo = None;
                    }
                    self.save_queue_file();
                }
                self.mail.status = "Sent".into();
            }
            Err(e) if id == 0 => self.compose_failed(format!("The message was not sent: {e}")),
            Err(e) => {
                if let Some(compose) = self.drafts.remove(&id).filter(|_| self.mail.sheet_is_free_for_compose()) {
                    self.mail.sheet = Some(Sheet::Compose(compose));
                    self.mail.fresh_compose = true;
                    self.mail.status = format!("The message was not sent: {e}");
                    self.save_queue_file();
                } else if let Some(mut queued) = flight {
                    queued.attempts += 1;
                    if crate::submit::is_unconfirmed(&e) {
                        // it may have been delivered: sending it again could reach the recipients twice
                        self.mail.status = format!("A queued message was taken off the outbox because delivery is unknown. {e}");
                    } else if queued.attempts >= schedule::MAX_ATTEMPTS {
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

    /// Adds a line to the activity list, with the time.
    fn note(&mut self, line: String) {
        self.mail.log(format!("{}  {line}", message::time_of_day(now_unix())));
    }

    /// Mailbox > Check Connection: signs in to both servers of the shown account and says what each answered.
    fn check_connection(&mut self) {
        let Some(account) = self.mail.current().cloned() else { return };
        let Some(password) = self.mail.password(&account).map(str::to_string) else {
            self.mail.status = format!("No password is saved for {}", account.name);
            return;
        };
        self.mail.status = format!("Checking the connection to {}", account.name);
        let transport = self.transport.clone();
        self.jobs += 1;
        sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::diagnose(transport.as_ref(), &account, &password));
    }

    fn notify(&self, ctx: &Ctx, notice: &Notice) {
        // the banner is the background check's too: the decoded sender and subject of the notice, never the raw header
        let b = crate::background::notification(notice);
        let request = Request::new(&b.id, &b.title).subtitle(&b.subtitle).body(&b.body).thread(&b.thread);
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
                    if (account.password_site() != old.password_site() || account.username != old.username) && !self.password_shared(&old) {
                        let request = passwords::remove(ctx, &old.password_site(), &old.username);
                        self.pending.insert(request, Pending::Forget);
                    }
                    let id = account.id.clone();
                    self.stop_watcher(&id);
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
        if !self.watching.contains_key(&account.id) {
            let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
            self.watching.insert(account.id.clone(), cancel.clone());
            sync::watch(self.transport.clone(), account.clone(), password.clone(), self.tx.clone(), self.wake_clone(), cancel);
        }
        let transport = self.transport.clone();
        let mailbox = mailbox.to_string();
        self.jobs += 1;
        sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::list(transport.as_ref(), &account, &password, &mailbox));
    }

    /// True when another account in the shared list uses the Passwords item of `account`. When the list cannot be
    /// read the answer is yes, so nothing is removed or replaced on a guess.
    fn password_shared(&self, account: &Account) -> bool {
        accounts::load_all().map_or(true, |all| accounts::shares_password(&all, account))
    }

    /// Stops the inbox watcher of an account, which holds the old server and password. The next refresh starts a new one.
    fn stop_watcher(&mut self, id: &str) {
        if let Some(cancel) = self.watching.remove(id) {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.live.remove(id);
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

    /// Sets or clears a flag on every selected message: the open one decides which (the menu item is titled after it),
    /// as Message > Flag and Mark as Unread do for a selection in the reference. The rows change at once; the server is
    /// told in the background.
    fn toggle_flag(&mut self, flag: Flag) {
        let Some(uid) = self.mail.open else { return };
        let Some(flags) = self.mail.flags_of(uid) else { return };
        let on = !flags.contains(flag);
        for uid in self.mail.selected() {
            self.mail.set_flag_locally(uid, flag, on);
            if flag == Flag::Flagged && !on {
                // clearing the flag also clears its colour keywords on the server
                self.change_message(uid, Change::Color { color: None });
            } else {
                self.change_message(uid, Change::Flag { flag, on });
            }
        }
    }

    /// Message > Flag Color and Clear Flag: every selected message is flagged in `color` (or loses its flag). The rows
    /// change at once; the server is told in the background.
    fn set_color_open(&mut self, color: Option<FlagColor>) {
        for uid in self.mail.selected() {
            self.mail.set_color_locally(uid, color);
            self.change_message(uid, Change::Color { color });
        }
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
                // a file another app hands over with a mailto: link (bifrost_app::mail_compose) goes on the open message
                None if bifrost_app::share::is_inbound(doc.request) => self.attach_documents(ctx, vec![doc]),
                Some(DocTask::ImportMbox) => self.import_document(ctx, doc),
                Some(DocTask::SaveFolder { files }) => self.save_folder(ctx, doc, files),
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
                Some(DocTask::SaveFolder { files }) if code == DocErrorCode::Unsupported => {
                    // no folder panel: each file goes to the data directory
                    for file in &files {
                        self.save_in_data_dir(&file.name, &file.bytes);
                    }
                }
                Some(DocTask::SaveFolder { .. }) => {
                    if code != DocErrorCode::Cancelled {
                        self.mail.status = format!("The folder could not be chosen ({code:?})");
                    }
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

    /// Moves the selected messages to the account's Archive or Trash mailbox. They leave the list at once.
    fn move_open(&mut self, candidates: &[&str], label: &str) {
        let selected = self.mail.selected();
        if selected.is_empty() {
            return;
        }
        let Some(to) = self.mail.find_mailbox(candidates) else {
            self.mail.status = format!("This account has no {label} mailbox");
            return;
        };
        for uid in selected {
            self.mail.remove_row(uid);
            self.change_message(uid, Change::Move { to: to.clone() });
        }
    }

    /// Message > Move To and Move Again: moves the open message to `to` and remembers it for Move Again. A message leaving
    /// the Junk mailbox loses its junk mark.
    fn move_open_to(&mut self, to: &str) {
        let selected = self.mail.selected();
        if selected.is_empty() {
            return;
        }
        self.mail.last_target = Some(to.to_string());
        let leaving_junk = JUNK_NAMES.iter().any(|name| name.eq_ignore_ascii_case(&self.mail.mailbox)) && !JUNK_NAMES.iter().any(|name| name.eq_ignore_ascii_case(to));
        for uid in selected {
            self.mail.remove_row(uid);
            let action = if leaving_junk { Change::NotJunk { to: to.to_string() } } else { Change::Move { to: to.to_string() } };
            self.change_message(uid, action);
        }
    }

    /// Message > Move to Junk: the message moves to the account's Junk mailbox marked `$Junk`.
    fn junk_open(&mut self) {
        let selected = self.mail.selected();
        if selected.is_empty() {
            return;
        }
        let Some(to) = self.mail.find_mailbox(&JUNK_NAMES) else {
            self.mail.status = "This account has no Junk mailbox".into();
            return;
        };
        for uid in selected {
            self.mail.remove_row(uid);
            self.change_message(uid, Change::Junk { to: to.clone() });
        }
    }

    /// Message > Copy To: the selected messages stay where they are and a copy of each goes to `to`.
    fn copy_open_to(&mut self, to: &str) {
        let selected = self.mail.selected();
        if selected.is_empty() {
            return;
        }
        for uid in selected {
            self.change_message(uid, Change::Copy { to: to.to_string() });
        }
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
        open_body_loaded(&self.mail)
    }

    /// Edit > Paste as Quotation: the clipboard text goes into the focused compose field with `> ` before each line.
    fn paste_quoted(&mut self) -> bool {
        let Some(text) = clipboard::text() else { return false };
        let multiline = self.focus_is_multiline();
        let quoted = message::quote_text(&text);
        let quoted: String = quoted.chars().filter(|c| !c.is_control() || (*c == '\n' && multiline)).collect();
        if self.mail.sheet.is_none() {
            return false;
        }
        self.edit_form(|form| form.type_text(&quoted));
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
        // the files of the original go along; the message being forwarded keeps its own copies
        let files = self.mail.attachments.get(&uid).cloned().unwrap_or_default();
        self.mail.open_forward_with(&original, &text, files);
    }

    /// True when the open message carries files.
    #[allow(dead_code)] // not wired yet
    fn open_has_files(&self) -> bool {
        open_has_files(&self.mail)
    }

    /// Message > Save Attachments: one folder panel for the files of the open message (a tap on one file saves it alone).
    fn save_attachments(&mut self, ctx: &Ctx) {
        let Some(uid) = self.mail.open else { return };
        let files: Vec<Attachment> = match self.mail.attachments.get(&uid) {
            Some(files) if !files.is_empty() => files.clone(),
            _ => {
                self.mail.status = "This message has no attachments".into();
                return;
            }
        };
        // one folder panel for every file of the message; the files keep their names
        let options = OpenOptions { files: false, directories: true, create_directories: true, title: "Save Attachments".into(), ..OpenOptions::default() };
        let request = ctx.open_panel(&options);
        self.docs.insert(request, DocTask::SaveFolder { files });
    }

    /// The folder the user chose for Save Attachments. The panel gives a handle to the folder, and its path is read back
    /// from that handle, so each file is written by name into the folder.
    fn save_folder(&mut self, ctx: &Ctx, doc: bifrost_app::Document, files: Vec<Attachment>) {
        let folder = std::fs::read_link(format!("/proc/self/fd/{}", doc.file.as_raw_fd()));
        self.mail.status = match folder {
            Ok(folder) => match attachments::save_all(&folder, &files) {
                Ok(count) => format!("Saved {count} file{} in {}", if count == 1 { "" } else { "s" }, folder.display()),
                Err(e) => format!("The files could not be saved: {e}"),
            },
            Err(e) => format!("The folder could not be used: {e}"),
        };
        ctx.release_document(doc.grant);
    }

    /// Message > Export as PDF: the subject and the readable text of the open message, saved through the save panel.
    fn export_pdf(&mut self, ctx: &Ctx) {
        let Some(uid) = self.mail.open else { return };
        let (Some(raw), Some(subject)) = (self.mail.bodies.get(&uid), self.mail.row(uid).map(|r| r.summary.subject.clone())) else {
            self.mail.status = "Wait for the message to load".into();
            return;
        };
        let text = crate::body::readable(raw);
        let title = if subject.trim().is_empty() { "Message".to_string() } else { subject.clone() };
        let bytes = pdf::render(&title, &text);
        self.queue_save(ctx, crate::export::pdf_name(&subject), bytes);
    }

    /// Open Quickly and the match list: shows the mailbox named.
    fn go_to_mailbox(&mut self, name: &str) {
        self.mail.close_sheet();
        self.mail.show_mailbox(name);
        self.list_scroll = Scroller::new();
        self.refresh_selected();
    }

    /// The filter chip in the toolbar: each tap moves to the next filter, and All takes the chip away.
    fn cycle_filter(&mut self) {
        let at = Filter::ALL.iter().position(|f| *f == self.mail.filter).unwrap_or(0);
        self.mail.filter = Filter::ALL[(at + 1) % Filter::ALL.len()];
        self.list_scroll = Scroller::new();
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

    /// Once the text of the open message is here: its people go to Contacts as suggestions (unless Mail Settings turned
    /// that off), and the banner reads it for events, flights, contacts and addresses to offer.
    fn read_open_message(&mut self, ctx: &Ctx) {
        for seen in self.mail.people_to_donate() {
            let request = contacts::suggested::donate(ctx, &seen.name, contacts::suggested::Handle::Email, &seen.address);
            self.pending.insert(request, Pending::Donated);
        }
        let shown = self.mail.found_pager.as_ref().map(|(uid, _)| *uid);
        let ready = self.mail.open.filter(|uid| self.mail.bodies.contains_key(uid));
        if shown != ready {
            let region = bifrost_app::locale::Locale::current().region_code().unwrap_or("IS").to_string();
            self.mail.refresh_found_in(&region);
            self.dirty = true;
        }
    }

    /// A tap on a link of the banner: page, turn the suggestion down, or take it. Taking it runs its action with the
    /// app that owns it: Calendar for an event, Contacts for an email address or phone number, Maps for an address.
    fn found_link(&mut self, ctx: &Ctx, link: crate::found::FoundHit) {
        use crate::found::FoundHit;
        use bifrost_app::controls::found_in::Action;
        match link {
            FoundHit::Next => {
                self.mail.found_step(true);
            }
            FoundHit::Previous => {
                self.mail.found_step(false);
            }
            FoundHit::Dismiss => {
                self.mail.found_dismiss();
            }
            FoundHit::Add => {
                let subject = self.mail.found_current().map(|f| f.source.title.trim().to_string()).unwrap_or_default();
                let Some(action) = self.mail.found_accept() else { return };
                match action {
                    Action::AddEvent(event) => {
                        let now = now_unix();
                        let offset = i64::from(bifrost_app::time_zone::TimeZone::current().seconds_from_gmt(bifrost_app::coding::Date(now as f64)));
                        let (start, end) = crate::found::event_times(&event, now, offset);
                        let title = if subject.is_empty() { "Event from Mail" } else { subject.as_str() };
                        let request = calendar::save_event(ctx, title, &start, &end, false, "");
                        self.pending.insert(request, Pending::EventSave);
                        self.mail.status = "Adding to Calendar".into();
                    }
                    Action::AddContact { email: Some(address), .. } => {
                        if let Some(person) = people::person_from(&address) {
                            let request = contacts::fetch(ctx, &[ContactKey::Emails], Some(person.address.as_str()));
                            self.pending.insert(request, Pending::ContactCheck(person));
                            self.mail.status = "Checking Contacts".into();
                        }
                    }
                    Action::AddContact { phone: Some(phone), .. } => {
                        let mut record = contacts::Record::new(0);
                        record.set(ContactKey::Phones, contacts::Value::List(vec![phone.clone()]));
                        let request = contacts::save(ctx, &SaveRequest { add: vec![record], ..SaveRequest::default() });
                        self.pending.insert(request, Pending::PhoneSave(phone));
                        self.mail.status = "Adding to Contacts".into();
                    }
                    Action::AddContact { .. } | Action::ShowFlight { .. } => {}
                    Action::OpenInMaps(address) => ctx.open_url(&crate::found::maps_link(&address)),
                }
            }
        }
    }

    /// While a compose sheet is open, asks for the address book once so the To, Cc and Bcc fields can suggest people from
    /// it. The sheet closing lets the next one ask again, which also refreshes the book.
    fn ensure_address_book(&mut self, ctx: &Ctx) {
        let composing = matches!(self.mail.sheet, Some(Sheet::Compose(_)));
        if composing && !self.book_asked {
            self.book_asked = true;
            let request = contacts::request(ctx);
            self.pending.insert(request, Pending::BookAccess);
        } else if !composing {
            self.book_asked = false;
        }
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
        let fetch = self.mail.open_message(uid).then_some(uid);
        self.shown_changed(fetch);
    }

    /// The reader shows another message (or none, or a selection of several): it starts from the top, and the text of
    /// `fetch` is asked for when it is not loaded yet.
    fn shown_changed(&mut self, fetch: Option<u32>) {
        self.reader_scroll = Scroller::new();
        if let Some(bar) = self.mail.find.as_mut() {
            bar.current = 0;
        }
        let Some(uid) = fetch else { return };
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
        bifrost_app::dispatch::DispatchQueue::global(bifrost_app::dispatch::Qos::Utility).async_(move || {
            if let Err(e) = job.run() {
                eprintln!("Mail print: {e:?}");
            }
        });
    }

    fn reply(&mut self) {
        let Some(uid) = self.mail.open else { return };
        let original = self.mail.row(uid).map(|r| r.summary.clone());
        if let Some(original) = original {
            // with the quoting setting on, the reply starts with the text of the original (once it has loaded)
            let quoted = if self.mail.prefs.quote_replies { self.mail.bodies.get(&uid).map(|raw| crate::body::readable(raw)) } else { None };
            self.mail.open_reply(&original, quoted.as_deref());
        }
    }

    /// Runs the sheet's primary action: add the account, or send the message.
    fn submit(&mut self, ctx: &Ctx) {
        match self.mail.sheet.clone() {
            Some(Sheet::Setup(form)) => self.submit_setup(ctx, &form, None),
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
                Some(name) => self.go_to_mailbox(&name),
                None => self.mail.status = "No mailbox matches".into(),
            },
            Some(Sheet::Search(form)) => {
                let text = form.value("Search").to_string();
                self.mail.apply_search(&text);
                self.mail.close_sheet();
                self.search_server();
            }
            Some(Sheet::Prompt(prompt)) => self.submit_prompt(ctx, prompt),
            Some(Sheet::Question(compose)) => self.save_question(&compose),
            Some(Sheet::Rules(..)) => self.mail.close_sheet(),
            None => {}
        }
    }

    /// Add Account's primary action. With Configure Manually off, the servers are looked up from the address first; the
    /// answer returns as `Event::Discovered` and calls this again with what was found. With it on, the form is used as
    /// it stands. Only the address goes out in the lookup; the password stays here.
    fn submit_setup(&mut self, ctx: &Ctx, form: &crate::state::Form, found: Option<&Found>) {
        if self.mail.discovering {
            return;
        }
        // what can be checked without the network is checked first
        if let Err(e) = self.mail.account_from_form_found(form, &[], found) {
            self.mail.status = e;
            return;
        }
        if found.is_none() && !is_yes(form.value(MANUAL)) {
            let email = form.value("Email").trim().to_string();
            let domain = email.rsplit_once('@').map_or("", |(_, d)| d).to_string();
            self.mail.discovering = true;
            self.mail.status = format!("Looking up the mail servers for {domain}...");
            self.jobs += 1;
            let transport = self.transport.clone();
            sync::spawn(self.tx.clone(), self.wake_clone(), move || sync::discover(transport, &email));
            return;
        }
        let taken = match accounts::load_all() {
            Ok(all) => all,
            Err(e) => {
                self.mail.status = e;
                return;
            }
        };
        match self.mail.account_from_form_found(form, &taken, found) {
            Ok((account, password)) => {
                let request = passwords::save(ctx, &account.password_site(), &account.username, &password);
                self.pending.insert(request, Pending::Add { account, password, old: None });
                self.mail.status = "Saving the password".into();
            }
            Err(e) => self.mail.status = e,
        }
    }

    /// The servers of the address typed in Add Account have been looked up. Servers the address's own domain vouches for
    /// are used at once; any others, and a failed lookup, show the server fields so the person decides.
    fn discovered(&mut self, ctx: &Ctx, email: String, result: Result<Found, String>) {
        self.mail.discovering = false;
        let Some(Sheet::Setup(form)) = self.mail.sheet.clone() else { return };
        if form.value("Email").trim() != email {
            self.mail.status.clear();
            return;
        }
        match result {
            Ok(found) if found.trusted => {
                self.note(format!("Add Account: found {} and {} for {email}", found.imap.host, found.smtp.host));
                self.submit_setup(ctx, &form, Some(&found));
            }
            Ok(found) => {
                let domain = email.rsplit_once('@').map_or("", |(_, d)| d);
                self.mail.status = format!("The servers found for {domain} are not on {domain}. Check them, then choose Add Account");
                self.mail.show_manual(Some(&found));
            }
            Err(e) => {
                self.mail.status = format!("{e}. Enter the servers below, or check the address");
                self.mail.show_manual(None);
            }
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
            PromptKind::Activity => self.mail.close_sheet(),
            PromptKind::EditRule(index) => self.submit_edit_rule(index, &prompt.form),
            PromptKind::EditAccount(id) => self.submit_edit_account(ctx, &id, &prompt.form),
            PromptKind::RemoveAccount(id) => self.submit_remove_account(ctx, &id, &prompt.form),
        }
    }

    /// Message > Ignore Follow-Up: the open message's reminders are dropped, so it does not come back on its own.
    fn ignore_follow_up(&mut self) {
        let Some(uid) = self.mail.open else { return };
        let Some(account) = self.mail.current().map(|a| a.id.clone()) else { return };
        let Some(message_id) = self.mail.row(uid).and_then(|r| r.summary.message_id.clone()) else { return };
        let dropped = self.mail.queue.drop_reminders(&account, &message_id);
        self.save_queue_file();
        self.mail.status = if dropped == 0 { "No reminder is set for this message".into() } else { "The follow-up is ignored".into() };
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
        // the item stays when another account (an alias, a calendar or contacts account) uses the same login
        if !self.password_shared(&account) {
            let request = passwords::remove(ctx, &account.password_site(), &account.username);
            self.pending.insert(request, Pending::Forget);
        }
        self.mail.forget_account(id);
        self.stop_watcher(id);
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
    /// A Send that did not go out: the reason goes on the compose sheet, so it shows in the window of the message it is
    /// about rather than in the main window's footer. With no compose sheet open it goes to the footer.
    fn compose_failed(&mut self, reason: String) {
        match &mut self.mail.sheet {
            Some(Sheet::Compose(compose)) => compose.error = reason,
            _ => self.mail.status = reason,
        }
    }

    fn send(&mut self, compose: &Compose) {
        if let Some(Sheet::Compose(open)) = &mut self.mail.sheet {
            open.error.clear();
        }
        let draft = match self.mail.draft_from(compose) {
            Ok(draft) => draft,
            Err(e) => {
                self.compose_failed(e);
                return;
            }
        };
        let Some(account) = self.mail.current().cloned() else {
            self.compose_failed("Add an account first".into());
            return;
        };
        let wait = match self.send_delay(compose) {
            Ok(wait) => wait,
            Err(e) => {
                self.compose_failed(e);
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
                self.compose_failed(e);
                return;
            }
        };
        // a message on its way is done with its draft: the saved copy leaves Drafts
        self.discard_draft(compose);
        let recipients: Vec<String> = draft.envelope_recipients().into_iter().map(str::to_string).collect();
        for address in &recipients {
            self.mail.recipients.record(address, "");
        }
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
            self.mail.sheet_done = true;
            self.mail.close_sheet();
            return;
        }
        let Some(password) = self.mail.password(&account).map(str::to_string) else {
            self.compose_failed(format!("No password is saved for {}", account.name));
            return;
        };
        self.mail.status = "Sending".into();
        self.sending = true;
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
                self.mail.fresh_compose = true;
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

    /// Writes the outbox and the reminders. A message that is out for sending stays in the file until the server has taken
    /// it, so a restart in the middle sends it again instead of losing it.
    fn save_queue_file(&mut self) {
        let mut saved = self.mail.queue.clone();
        saved.outbox.extend(self.in_flight.values().cloned());
        if let Err(e) = schedule::save(&self.queue_path, &saved) {
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
        if self.mail.sheet_is_free_for_compose() {
            true
        } else {
            self.mail.status = "Close the open sheet first".into();
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

    /// Message > Edit Draft: the open message (of the Drafts mailbox) goes back into a compose sheet.
    fn edit_draft(&mut self) {
        if !self.open_body_loaded() || !self.sheet_free() {
            return;
        }
        let Some(raw) = self.mail.open.and_then(|uid| self.mail.bodies.get(&uid)).cloned() else { return };
        self.mail.open_draft_copy(&raw);
        self.dirty = true;
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

    /// True when the field that takes typed text is not for typing: the From picker (a tap changes it) and the activity list.
    /// Which standard editing commands can act right now, for the Edit menu: see `edit_enabled`.
    pub fn edit_enabled(&self) -> (bool, bool, bool, bool) {
        edit_enabled(&self.mail)
    }

    fn focus_is_picker(&self) -> bool {
        picker_focused(&self.mail)
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

    fn tap(&mut self, ctx: &Ctx, x: f32, y: f32, t: f64, mods: ModifierFlags) -> bool {
        let hit = self.hit(&ctx.ui, x, y);
        // every click counts, so a click elsewhere in between breaks a double-click; `t` is milliseconds
        let scale = ctx.ui.pt(1.0).max(f32::EPSILON);
        let clicks = self.clicks.press(t, x / scale, y / scale);
        match hit {
            crate::draw::Hit::Button(button) => match button {
                crate::draw::Button::Refresh => self.refresh_selected(),
                crate::draw::Button::Compose => self.compose(),
                crate::draw::Button::Reply => self.reply(),
                crate::draw::Button::ReplyAll => self.reply_all(),
                crate::draw::Button::Archive => self.move_open(&ARCHIVE_NAMES, "Archive"),
                crate::draw::Button::Trash => self.move_open(&TRASH_NAMES, "Trash"),
                crate::draw::Button::Filter => self.cycle_filter(),
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
            // Cmd-click toggles the message in the selection, Shift-click selects the range from the anchor; neither counts
            // towards a double-click
            crate::draw::Hit::Message(uid) if mods.command() || mods.shift() => {
                self.last_tap = None;
                self.clicks = ClickCounter::default();
                let fetch = if mods.command() { self.mail.toggle_selected(uid) } else { self.mail.extend_selection(uid) };
                self.shown_changed(fetch);
            }
            crate::draw::Hit::Message(uid) => {
                // a double-click on a message opens it in a viewer window (the first click already selected it)
                let double = clicks >= 2 && self.last_tap == Some(uid);
                self.last_tap = Some(uid);
                if double {
                    self.clicks = ClickCounter::default();
                    self.viewer_requests.push(uid);
                } else {
                    self.open_message(uid);
                }
            }
            crate::draw::Hit::Favorite(index) => self.open_favorite(index),
            crate::draw::Hit::Fold(uid) => self.mail.toggle_conversation(uid),
            crate::draw::Hit::Attachment(index) => self.save_attachment(ctx, index),
            crate::draw::Hit::LoadRemote => self.load_remote(),
            crate::draw::Hit::Found(link) => self.found_link(ctx, link),
            crate::draw::Hit::Attach if matches!(self.mail.sheet, Some(Sheet::Rules(..))) => self.mail.open_rule(),
            crate::draw::Hit::Attach => self.pick_files(ctx),
            crate::draw::Hit::EditRule(index) => self.mail.open_rule_editor(index),
            crate::draw::Hit::RemoveAttachment(index) => {
                if let Some(Sheet::Compose(compose)) = &mut self.mail.sheet {
                    compose.detach(index);
                } else if matches!(self.mail.sheet, Some(Sheet::Rules(..))) {
                    self.remove_rule(index);
                }
            }
            crate::draw::Hit::FindPrevious => self.step_match(false),
            crate::draw::Hit::FindNext => self.step_match(true),
            crate::draw::Hit::FindDone => self.mail.find = None,
            crate::draw::Hit::Choice(index) if matches!(self.mail.sheet, Some(Sheet::Compose(_))) => {
                if !self.mail.accept_recipient(Some(index)) {
                    return false;
                }
            }
            crate::draw::Hit::Choice(index) => {
                let name = crate::draw::choices_of(&self.mail).get(index).cloned();
                match name {
                    Some(name) => self.go_to_mailbox(&name),
                    None => return false,
                }
            }
            crate::draw::Hit::Field(index) => {
                let (from_picker, choice) = self
                    .mail
                    .sheet
                    .as_ref()
                    .map(Sheet::form)
                    .and_then(|form| form.fields.get(index))
                    .map_or((false, false), |f| (f.label == "From", f.choice));
                if let Some(form) = self.form_mut() {
                    form.focus_on(index);
                    form.deselect();
                }
                self.mail.sync_recipient_field();
                if choice {
                    // a yes or no field flips with each click
                    if let Some(form) = self.form_mut() {
                        form.flip(index);
                    }
                    // Configure Manually shows or hides the server fields
                    self.mail.sync_manual();
                }
                if from_picker {
                    let choices = self.mail.from_choices();
                    if let Some(Sheet::Compose(compose)) = &mut self.mail.sheet {
                        compose.cycle_from(&choices);
                    }
                }
            }
            crate::draw::Hit::Cancel => self.cancel_sheet(),
            crate::draw::Hit::Discard => self.discard_question(),
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

/// True when the open sheet's focused field is a picker (From, Activity or a choice): it takes no typed text.
fn picker_focused(mail: &Mail) -> bool {
    mail.sheet.as_ref().map(Sheet::form).is_some_and(|form| form.fields.get(form.focus).is_some_and(|f| f.label == "From" || f.label == "Activity" || f.choice))
}

/// Which standard editing commands can act right now, for the Edit menu: `(select_all, copy, cut, paste)`. Select All needs
/// a sheet's text field with text in it; Copy and Cut need a selection in a field that is not secret; Paste needs a field
/// that takes typed text, or the find bar of an open message.
fn edit_enabled(mail: &Mail) -> (bool, bool, bool, bool) {
    let Some(form) = mail.sheet.as_ref().map(Sheet::form) else {
        // the message list takes Select All (every message it shows)
        return (!mail.visible_lines().is_empty(), false, false, mail.open.is_some() && mail.find.is_some());
    };
    let typed = !picker_focused(mail);
    let has_text = typed && form.fields.get(form.focus).is_some_and(|f| !f.value.is_empty());
    let selected = typed && form.selected_text().is_some();
    (has_text, selected, selected, typed)
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
    fn the_edit_commands_are_enabled_only_while_they_can_act() {
        fn form(mail: &mut Mail) -> &mut crate::state::Form {
            match mail.sheet.as_mut() {
                Some(Sheet::Setup(form)) => form,
                _ => panic!("no setup sheet"),
            }
        }
        let mut mail = Mail::new(Vec::new());
        assert_eq!(edit_enabled(&mail), (false, false, false, false), "no sheet: nothing to edit");
        mail.open_setup();
        form(&mut mail).focus_on(0);
        assert_eq!(edit_enabled(&mail), (false, false, false, true), "an empty field takes a paste and nothing else");
        form(&mut mail).type_text("Ana");
        assert_eq!(edit_enabled(&mail), (true, false, false, true), "text to select, nothing selected yet");
        form(&mut mail).select_all();
        assert_eq!(edit_enabled(&mail), (true, true, true, true), "a selection can be copied and cut");
        // a password is never copied or cut
        let secret = form(&mut mail).fields.iter().position(|f| f.secret).expect("a password field");
        form(&mut mail).focus_on(secret);
        form(&mut mail).type_text("hunter2");
        form(&mut mail).select_all();
        assert_eq!(edit_enabled(&mail), (true, false, false, true), "a secret field gives nothing up");
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

    use crate::flags::Flags;
    use crate::session::Listed;
    use bifrost_app::controls::internet_accounts::Kind;
    use bifrost_app::proto::{MAX_APP_MENUS, MAX_APP_MENU_DEPTH, MAX_APP_MENU_ITEMS, MAX_APP_MENU_TITLE};

    /// A mail with everything that makes a menu long: many mailboxes, every rule, a smart mailbox, an open message with a
    /// file, and (when asked) a message being written.
    #[test]
    fn edit_draft_reopens_a_saved_copy_under_its_message_id() {
        let mut mail = Mail::new(Vec::new());
        mail.mailbox = "Kladdur".into();
        mail.drafts_mailbox = Some("Kladdur".into());
        assert!(mail.is_drafts());
        mail.mailbox = INBOX.into();
        assert!(!mail.is_drafts());
        let raw = "Message-ID: <7.1@x>\r\nTo: bo@example.com\r\nSubject: Drog\r\n\r\nHi\r\n";
        mail.open_draft_copy(raw);
        match &mail.sheet {
            Some(Sheet::Compose(c)) => assert_eq!(c.draft_id, "<7.1@x>"),
            _ => panic!("no compose sheet"),
        }
    }

    fn busy_mail(compose: bool) -> Mail {
        let mut mail = Mail::new(vec![Account::new(Kind::Mail, "work", "Work", "me")]);
        mail.mailboxes = std::iter::once(INBOX.to_string()).chain((0..60).map(|n| format!("Box {n}"))).collect();
        for n in 0..MAX_RULES {
            mail.prefs.rules.push(rules::Rule { name: format!("Rule {n}"), from: "x".into(), actions: vec![Action::MarkRead], ..rules::Rule::default() });
        }
        mail.prefs.smart.push(prefs::SmartMailbox { name: "Ana".into(), query: "from:ana".into() });
        mail.smart_active = Some(0);
        mail.apply_listing("work", INBOX, vec![], vec![Listed { uid: 1, flags: Flags::empty(), header: "Subject: a\r\n\r\n".into() }]);
        mail.open_message(1);
        mail.body_arrived("work", INBOX, 1, "Subject: a\r\n\r\nbody".into());
        mail.attachments.insert(1, vec![Attachment { name: "a.txt".into(), mime: "text/plain".into(), bytes: vec![1] }]);
        mail.last_target = Some("Box 3".into());
        mail.threaded = true;
        mail.find = Some(FindBar::default());
        if compose {
            mail.open_compose(None);
        }
        mail
    }

    #[test]
    fn the_context_menu_of_a_message_offers_the_message_actions() {
        let items = context_items(&busy_mail(false), false);
        assert!(items.len() <= 32, "the shell shows 32 rows at most");
        let row = |id: u32| items.iter().find(|i| i.id == id).cloned().expect("a row");
        assert_eq!(row(MENU_NEW_VIEWER).title(), "Open in New Window");
        for id in [MENU_REPLY, MENU_FORWARD, MENU_UNREAD, MENU_FLAG, MENU_ARCHIVE, MENU_TRASH, MENU_JUNK, MENU_PRINT] {
            assert!(row(id).enabled, "{}", row(id).title());
        }
        assert_eq!(row(MENU_MOVE_AGAIN).title(), "Move Again to Box 3", "the menu bar's title");
        assert!(items.iter().any(ContextItem::is_separator));
        assert!(!items.first().unwrap().is_separator() && !items.last().unwrap().is_separator());
        // the actions that need a message are off without one
        let mut idle = busy_mail(false);
        idle.open = None;
        assert!(!context_items(&idle, false).iter().find(|i| i.id == MENU_REPLY).unwrap().enabled);
    }

    #[test]
    fn no_menu_is_longer_than_the_shell_takes() {
        for compose in [false, true] {
            let menus = menus(&busy_mail(compose), true);
            assert!(menus.len() <= MAX_APP_MENUS);
            for menu in &menus {
                assert!(menu.items.len() <= MAX_APP_MENU_ITEMS, "{} has {} items", menu.title, menu.items.len());
                for item in &menu.items {
                    assert!(item.title.chars().count() <= MAX_APP_MENU_TITLE, "{}", item.title);
                    assert!(item.depth <= MAX_APP_MENU_DEPTH);
                }
            }
        }
    }

    #[test]
    fn every_menu_item_has_its_own_id() {
        let menus = menus(&busy_mail(true), true);
        let mut seen = HashSet::new();
        for item in menus.iter().flat_map(|m| &m.items).filter(|i| !i.separator && !i.submenu) {
            assert_ne!(item.id, 0, "{}", item.title);
            assert!(seen.insert(item.id), "{} repeats id {}", item.title, item.id);
        }
        assert!(seen.contains(&MENU_ATTACH) && seen.contains(&MENU_FIND_IN_MESSAGE) && seen.contains(&(MENU_RULE_BASE + 11)));
    }

    #[test]
    fn move_and_copy_list_the_first_mailboxes_only() {
        let menus = menus(&busy_mail(false), false);
        let moves = menus.iter().find(|m| m.title == "Move").expect("a Move menu");
        let count = |base: u32| moves.items.iter().filter(|i| i.id >= base && i.id < base + MENU_TARGET_LIMIT as u32).count();
        assert_eq!(count(MENU_MOVE_BASE), MENU_TARGETS_SHOWN);
        assert_eq!(count(MENU_COPY_BASE), MENU_TARGETS_SHOWN);
        assert!(moves.items.iter().any(|i| i.id == MENU_MOVE_AGAIN && i.enabled && i.title == "Move Again to Box 3"));
    }

    #[test]
    fn menu_items_follow_the_state_they_act_on() {
        let item = |menus: &[AppMenu], id: u32| menus.iter().flat_map(|m| &m.items).find(|i| i.id == id).cloned().expect("item");
        let editing = menus(&busy_mail(true), true);
        assert!(item(&editing, MENU_UNDO_SEND).enabled);
        assert!(item(&editing, MENU_SEND).enabled && item(&editing, MENU_ATTACH).enabled && item(&editing, MENU_SEND_LATER).enabled);
        assert!(!item(&editing, MENU_SEND_LATER).checked);
        let reading = menus(&busy_mail(false), false);
        assert!(!item(&reading, MENU_UNDO_SEND).enabled);
        assert!(!item(&reading, MENU_SEND).enabled && !item(&reading, MENU_ATTACH).enabled);
        assert!(item(&reading, MENU_REMOVE_ATTACHMENTS).enabled && item(&reading, MENU_FORWARD_ATTACHMENT).enabled);
        assert!(item(&reading, MENU_FIND_NEXT).enabled, "the find bar is open");
        assert!(item(&reading, MENU_DELETE_SMART).enabled);
        let mut idle = busy_mail(false);
        idle.open = None;
        idle.find = None;
        idle.smart_active = None;
        let idle = menus(&idle, false);
        assert!(!item(&idle, MENU_REMOVE_ATTACHMENTS).enabled && !item(&idle, MENU_REMIND).enabled && !item(&idle, MENU_FIND_NEXT).enabled);
        assert!(!item(&idle, MENU_DELETE_SMART).enabled);
    }

    #[test]
    fn the_writing_menu_offers_proofread_while_writing_and_the_answers_while_suggestions_wait() {
        let item = |menus: &[AppMenu], id: u32| menus.iter().flat_map(|m| &m.items).find(|i| i.id == id).cloned().expect("item");
        let mut mail = busy_mail(true);
        let menus_now = menus(&mail, false);
        assert!(item(&menus_now, MENU_PROOFREAD).enabled);
        assert!(!item(&menus_now, MENU_ACCEPT_SUGGESTION).enabled && !item(&menus_now, MENU_SKIP_SUGGESTION).enabled && !item(&menus_now, MENU_ACCEPT_ALL_SUGGESTIONS).enabled);
        if let Some(Sheet::Compose(compose)) = mail.sheet.as_mut() {
            if let Some(field) = compose.form.fields.iter_mut().find(|f| f.label == "Message") {
                field.value = "Say it it again".into();
            }
            assert_eq!(compose.start_proofread(), 1);
        }
        let menus_now = menus(&mail, false);
        assert!(item(&menus_now, MENU_ACCEPT_SUGGESTION).enabled && item(&menus_now, MENU_SKIP_SUGGESTION).enabled && item(&menus_now, MENU_ACCEPT_ALL_SUGGESTIONS).enabled);
        assert!(!item(&menus(&busy_mail(false), false), MENU_PROOFREAD).enabled, "nothing to proofread outside a message");
    }

    #[test]
    fn a_wait_is_told_in_the_biggest_whole_unit() {
        assert_eq!(wait_text(0), "1 second");
        assert_eq!(wait_text(45), "45 seconds");
        assert_eq!(wait_text(60), "1 minute");
        assert_eq!(wait_text(7200), "2 hours");
        assert_eq!(wait_text(3 * 86_400), "3 days");
    }

    #[test]
    fn a_file_over_the_limit_is_refused_by_its_size_or_by_what_it_turns_out_to_hold() {
        let path = std::env::temp_dir().join(format!("bifrost-mail-read-{}", std::process::id()));
        std::fs::write(&path, b"0123456789").unwrap();
        let mut file = std::fs::File::open(&path).unwrap();
        assert_eq!(read_file(&mut file, 10, 100).unwrap(), b"0123456789".to_vec());
        let mut file = std::fs::File::open(&path).unwrap();
        assert!(read_file(&mut file, 10, 5).unwrap_err().contains("larger than"));
        let mut file = std::fs::File::open(&path).unwrap();
        assert!(read_file(&mut file, 0, 5).is_err(), "a size that was understated is caught while reading");
        let _ = std::fs::remove_file(&path);
    }
}
