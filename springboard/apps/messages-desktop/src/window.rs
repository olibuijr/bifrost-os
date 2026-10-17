//! The Messages window: the model in [`crate::model`] wired to the Matrix jobs ([`crate::sync`]), the saved accounts in
//! Passwords ([`crate::accounts`]) and the notifications. This file holds the flow and the input; drawing and hit-testing
//! are in [`crate::draw`].

use crate::accounts::{self, Account};
use crate::crypto;
use crate::state_file;
use bifrost_app::keychain;
use crate::api::{Api, UPLOAD_LIMIT};
use crate::draw::{inside, ChipHit, Hit, OverlayAction, Rect};
use crate::model::{InfoKind, Messages, Outgoing, Overlay, Prompt, PromptKind, SignIn, REACTIONS};
use crate::protocol;
use crate::sync::{self, Event};
use bifrost_app::kit::{Scroller, TextCache, Ui};
use bifrost_app::passwords::{self, Error as KeyError};
use bifrost_app::proto::{DocMode, CHORD_CTRL, CHORD_META};
use bifrost_app::dnd::{self, DragPhase, Payload};
use bifrost_app::proto::ContextItem;
use bifrost_app::{AppMenu, AppMenuItem, Ctx, DocumentEvent, KeyInput, TouchKind};
use bifrost_core::json::Json;
use springboard::deildu::http::{Tls, Transport};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;

/// Key codes the shell sends for the editing keys (as the other apps read them).
const KEY_ESCAPE: u32 = 1;
const KEY_BACKSPACE: u32 = 8;
const KEY_TAB: u32 = 9;
const KEY_RETURN: u32 = 13;
/// evdev V, for the paste chord.
const KEY_V: u32 = 47;
/// Movement, in points, before a press counts as a drag.
const DRAG_PT: f32 = 6.0;
/// The picture types the app sends as pictures; any other file goes as a file message.
#[cfg(test)]
const PICTURE_TYPES: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

// Main-menu items (the Conversation menu and the others), also the ids of the room context menu.
pub const MENU_NEW: u32 = 1;
pub const MENU_PIN: u32 = 2;
pub const MENU_MUTE: u32 = 3;
pub const MENU_UNREAD: u32 = 4;
pub const MENU_DETAILS: u32 = 5;
pub const MENU_DELETE: u32 = 6;
pub const MENU_ATTACH: u32 = 7;
pub const MENU_EMOJI: u32 = 8;
pub const MENU_SETTINGS: u32 = 9;
pub const MENU_FIND: u32 = 10;
pub const MENU_PIN_UP: u32 = 11;
pub const MENU_PIN_DOWN: u32 = 12;
// Message context menu.
pub const CTX_REPLY: u32 = 20;
pub const CTX_EDIT: u32 = 21;
pub const CTX_UNSEND: u32 = 22;
pub const CTX_COPY: u32 = 23;
pub const CTX_REACTIONS: u32 = 24;
pub const CTX_HISTORY: u32 = 25;
/// The first of the tapback ids: `CTX_REACT + i` reacts with `REACTIONS[i]`.
pub const CTX_REACT: u32 = 30;

/// What the window is waiting for from Passwords (through `service_reply`).
enum Pending {
    /// The list of saved Matrix logins.
    Accounts,
    /// The password of a saved login, read to sign in.
    Password(Account),
    /// A login saved after a typed sign-in, or moved to the server's full user id: added, or replaced when an item already
    /// exists. `legacy` is the bare name the login was saved under before, removed once the full id is saved.
    Save { account: Account, password: String, legacy: Option<String> },
    Replace { legacy: Option<String> },
    /// The saved login removed at sign-out, or a login under a bare name removed after it moved (its answer is not needed).
    Forget,
    /// One of the crypto items read before a sign-in (see [`Lookup`]).
    Lookup(Item),
    /// A crypto item written after a change. `added` is true once the item was added after an update found none. `moving`
    /// is true while the items are moved from a bare name to the full user id (see [`Migration`]).
    Store { service: String, account: String, value: String, added: bool, moving: bool },
    /// A keychain item removed after a move; the answer is not needed.
    Discard,
}

/// A sign-in under a name other than the stored full user id, or one that found the sessions in the keychain. The crypto
/// items under the typed name are copied to the stored name and then removed, and the sessions keychain item is removed
/// once the sessions file holds them.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Migration {
    /// The name the crypto items were read under at this sign-in.
    typed: String,
    /// True while the sessions keychain item of an earlier version is still to be removed.
    sessions_item: bool,
    /// True once the device and key items were copied to the stored name and removed under `typed`.
    items_moved: bool,
    /// The copies still to be confirmed (the device and the key) before `typed` is removed.
    stores_left: u32,
}

/// The keychain items of the end-to-end encryption state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Device,
    Key,
    Sessions,
}

/// The crypto items read for one sign-in: the device the account had, its pickle key and its sessions. They wait for
/// the three keychain answers, then the sign-in starts with the device id as a hint, and `restore` holds them for the
/// sign-in to install.
#[derive(Clone, Debug, Default)]
struct Lookup {
    homeserver: String,
    user: String,
    password: String,
    typed: bool,
    waiting: u32,
    device: Option<String>,
    key: Option<String>,
    sessions: Option<String>,
    /// True when `sessions` came from the keychain item of an earlier version (see [`crypto::SESSIONS_ITEM`]).
    sessions_legacy: bool,
}

/// What the open context menu is about.
#[derive(Clone, Debug, PartialEq, Eq)]
enum CtxTarget {
    Room(String),
    Message(String),
}

/// Which part of the window a press started in. Only the two lists scroll; the rest is chrome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Rooms,
    Timeline,
    Other,
}

#[derive(Clone, Copy)]
struct Press {
    x: f32,
    y: f32,
    moved: bool,
    region: Region,
}

/// A file drag over the window, or one just dropped on it. Files that arrive before the drop wait in `files`; after the
/// drop they go to `room`, the conversation the drop landed on.
struct FileDrop {
    session: u32,
    /// True when the drag carries files, as the items of its first event and of its drop say.
    offered: bool,
    room: Option<String>,
    files: Vec<(String, Vec<u8>)>,
}

impl FileDrop {
    fn new(session: u32) -> FileDrop {
        FileDrop { session, offered: false, room: None, files: Vec::new() }
    }
}

pub struct MessagesWindow {
    pub model: Messages,
    transport: Arc<dyn Transport>,
    /// The signed-in session, once a sign-in succeeded.
    api: Option<Arc<Api>>,
    tx: sync::EventQueue,
    wake_rx: Option<UnixStream>,
    wake_tx: Option<UnixStream>,
    pending: HashMap<u32, Pending>,
    /// The crypto items being read for a sign-in that has not started yet.
    lookup: Option<Lookup>,
    /// The crypto items the sign-in in progress restores from.
    restore: Option<Lookup>,
    /// The move of a login under a bare name, or the removal of the sessions keychain item (see [`Migration`]).
    migration: Option<Migration>,
    /// The file the user is choosing, with the room it is for.
    picking: Option<(u32, String)>,
    /// The file drag in progress, if any.
    file_drop: Option<FileDrop>,
    /// The signed-in login, to forget it at sign-out.
    account: Option<Account>,
    /// Where the per-conversation choices are saved.
    data_dir: PathBuf,
    /// What the context menu that is open acts on.
    ctx_target: Option<CtxTarget>,
    /// The bubbles and reaction chips of the last frame, for taps and the context menu.
    pub bubbles: Vec<(Rect, String)>,
    pub chips: Vec<ChipHit>,
    /// The quoted lines of replies in the last frame, with the id of the message each one quotes.
    pub quotes: Vec<(Rect, String)>,
    pub w: u32,
    pub h: u32,
    pub room_scroll: Scroller,
    pub timeline_scroll: Scroller,
    /// Keeps the newest message in view until the user scrolls the timeline by hand.
    pub stick_bottom: bool,
    press: Option<Press>,
    pub labels: TextCache,
    dirty: bool,
}

/// A connected pair of sockets: the job threads write to one, the loop reads the other (`wake_fd`).
fn wake_pair() -> Option<(UnixStream, UnixStream)> {
    let (rx, tx) = UnixStream::pair().ok()?;
    rx.set_nonblocking(true).ok()?;
    tx.set_nonblocking(true).ok()?;
    Some((rx, tx))
}

impl MessagesWindow {
    /// Asks Passwords for the saved Matrix logins. The first one signs in as the answer arrives.
    pub fn new(ctx: &Ctx) -> MessagesWindow {
        let transport: Arc<dyn Transport> = Arc::new(Tls::new());
        let tx = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
        let (wake_rx, wake_tx) = match wake_pair() {
            Some((rx, tx)) => (Some(rx), Some(tx)),
            None => (None, None),
        };
        let mut model = Messages::default();
        model.prefs = crate::store::load(&ctx.data_dir);
        let mut window = MessagesWindow {
            model,
            transport,
            api: None,
            tx,
            wake_rx,
            wake_tx,
            pending: HashMap::new(),
            lookup: None,
            restore: None,
            migration: None,
            picking: None,
            file_drop: None,
            account: None,
            data_dir: ctx.data_dir.clone(),
            ctx_target: None,
            bubbles: Vec::new(),
            chips: Vec::new(),
            quotes: Vec::new(),
            w: 0,
            h: 0,
            room_scroll: Scroller::new(),
            timeline_scroll: Scroller::new(),
            stick_bottom: true,
            press: None,
            labels: TextCache::default(),
            dirty: true,
        };
        dnd::register_types(ctx, &["public.data"]);
        let request = passwords::list(ctx);
        window.pending.insert(request, Pending::Accounts);
        window.model.status = "Loading saved accounts...".into();
        window
    }

    pub fn set_size(&mut self, w: u32, h: u32) {
        self.w = w;
        self.h = h;
        self.dirty = true;
    }

    pub fn wake_fd(&self) -> Option<i32> {
        self.wake_rx.as_ref().map(AsRawFd::as_raw_fd)
    }

    /// The next time the loop should come back: a frame while a scroll settles, otherwise only when a job answers.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.room_scroll.animating() || self.timeline_scroll.animating() {
            Some(t + 16.0)
        } else {
            None
        }
    }

    /// A clone of the wake pipe's writing end, for a job thread.
    fn wake(&self) -> Option<UnixStream> {
        self.wake_tx.as_ref().and_then(|w| w.try_clone().ok())
    }

    /// Applies what the job threads produced. True when the screen changed.
    pub fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        if let Some(mut r) = self.wake_rx.as_ref() {
            let mut buf = [0u8; 64];
            while r.read(&mut buf).is_ok_and(|n| n > 0) {}
        }
        let events = self.tx.lock().map(|mut queue| queue.drain(..).collect::<Vec<_>>()).unwrap_or_default();
        for event in events {
            self.event(ctx, event);
            self.dirty = true;
        }
        let scrolled = self.room_scroll.tick(t) | self.timeline_scroll.tick(t);
        std::mem::take(&mut self.dirty) || scrolled
    }

    fn event(&mut self, ctx: &Ctx, event: Event) {
        match event {
            Event::SignedIn { homeserver, user, password, typed, result } => match result {
                Ok(api) => self.signed_in(ctx, api, homeserver, user, password, typed),
                Err(e) => self.model.sign_in = Some(SignIn::prefilled(&homeserver, &user, &e)),
            },
            Event::Synced(Ok(_)) if self.api.is_none() => {}
            Event::Synced(Ok(sync)) => {
                self.save_crypto(ctx);
                let marks = self.model.prefs.unread_marks.clone();
                let notices = self.model.apply_sync(&sync);
                if self.model.prefs.unread_marks != marks {
                    self.save_prefs();
                }
                for notice in &notices {
                    ctx.notify(&notice.title, &notice.body);
                }
                if self.model.status.starts_with("Sync problem") {
                    self.model.status.clear();
                }
                self.badge(ctx);
                let marker = self.model.take_read_marker();
                self.send_read(marker);
            }
            Event::Synced(Err(e)) => self.model.status = format!("Sync problem: {e}"),
            Event::Sent { room: _, result } => match result {
                Ok(()) => self.model.status.clear(),
                Err(e) => self.model.status = format!("Not sent: {e}"),
            },
            Event::Read { result } => {
                if let Err(e) = result {
                    self.model.status = format!("Read receipt failed: {e}");
                }
            }
            Event::Changed { what, result } => {
                if let Err(e) = result {
                    self.model.status = format!("{what} failed: {e}");
                }
            }
            Event::Created { result } => match result {
                Ok(id) => {
                    self.model.overlay = Overlay::None;
                    self.model.show_new_room(&id);
                    self.open_room(&id);
                }
                Err(e) => {
                    if let Overlay::Prompt(p) = &mut self.model.overlay {
                        p.busy = false;
                        p.error = e;
                    } else {
                        self.model.status = format!("New message failed: {e}");
                    }
                }
            },
            Event::Left { room, result } => match result {
                Ok(()) => {
                    self.model.remove_room(&room);
                    self.save_prefs();
                    self.badge(ctx);
                }
                Err(e) => self.model.status = format!("Could not leave the conversation: {e}"),
            },
            Event::Renamed { room, name, result } => match result {
                Ok(()) => {
                    if let Some(r) = self.model.rooms.iter_mut().find(|r| r.id == room) {
                        r.name = name;
                    }
                }
                Err(e) => self.model.status = format!("Rename failed: {e}"),
            },
            Event::Decrypted(patches) => {
                self.model.apply_decrypted(&patches);
                self.save_crypto(ctx);
            }
            Event::LoggedOut { result: _ } => {}
        }
        self.dirty = true;
    }

    fn signed_in(&mut self, ctx: &Ctx, api: Arc<Api>, homeserver: String, typed_user: String, password: String, typed: bool) {
        // The login is stored under the full user id the server returned, whatever was typed (see `accounts::stored_user`).
        let user = accounts::stored_user(&typed_user, &api.user_id);
        let moved = accounts::moved_from(&typed_user, &user);
        self.model.user_id = api.user_id.clone();
        self.model.homeserver = homeserver.clone();
        self.account = Some(Account { homeserver: homeserver.clone(), user: user.clone() });
        self.model.sign_in = None;
        self.model.status.clear();
        let restore = self.restore.take().unwrap_or_default();
        if moved.is_some() || restore.sessions_legacy {
            self.migration = Some(Migration {
                typed: restore.user.clone(),
                sessions_item: restore.sessions_legacy,
                items_moved: moved.is_none(),
                stores_left: 0,
            });
        }
        let key = restore.key.as_deref().and_then(crypto::parse_key_item);
        let stored = restore.device.as_deref().and_then(crypto::parse_device_item);
        if let Err(e) = api.install_crypto(key, stored, restore.sessions.as_deref()) {
            self.model.status = format!("Encryption could not start: {e}");
        }
        sync::run_sync(api.clone(), self.tx.clone(), self.wake());
        self.api = Some(api);
        // A login under a bare name is saved again under the full id, and the bare item is removed once that is stored.
        if typed || moved.is_some() {
            let request = passwords::save(ctx, &accounts::service(&homeserver), &user, &password);
            self.pending.insert(request, Pending::Save { account: Account { homeserver, user }, password, legacy: moved });
        }
    }

    /// The answer to one of the window's requests to Passwords. False when the request was not the window's.
    pub fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        let Some(pending) = self.pending.remove(&request) else { return false };
        match pending {
            Pending::Accounts => match passwords::decode_list(result) {
                Some(rows) => match accounts::from_rows(&rows).into_iter().next() {
                    Some(account) => {
                        let request = passwords::fill(ctx, &accounts::service(&account.homeserver), &account.user);
                        self.pending.insert(request, Pending::Password(account));
                    }
                    None => self.model.sign_in = Some(SignIn::default()),
                },
                None => {
                    self.model.status = "Passwords could not list the saved accounts".into();
                    self.model.sign_in = Some(SignIn::default());
                }
            },
            Pending::Password(account) => match passwords::decode_password(result) {
                Ok(password) => self.start_sign_in(ctx, account.homeserver, account.user, password, false),
                Err(e) => {
                    self.model.status = match e {
                        KeyError::ItemNotFound => "No password is saved for this account".into(),
                        other => format!("Passwords could not return the password: {other:?}"),
                    };
                    self.model.sign_in = Some(SignIn::prefilled(&account.homeserver, &account.user, ""));
                }
            },
            Pending::Save { account, password, legacy } => match passwords::decode_password(result) {
                Ok(_) => self.forget_bare_login(ctx, &account.homeserver, legacy),
                Err(KeyError::DuplicateItem) => {
                    let request = passwords::update(ctx, &accounts::service(&account.homeserver), &account.user, &password);
                    self.pending.insert(request, Pending::Replace { legacy });
                }
                Err(e) => self.model.status = format!("The password could not be saved in Passwords: {e:?}"),
            },
            Pending::Replace { legacy } => match passwords::decode_password(result) {
                Ok(_) => {
                    let homeserver = self.model_homeserver();
                    self.forget_bare_login(ctx, &homeserver, legacy);
                }
                Err(e) => self.model.status = format!("The password could not be saved in Passwords: {e:?}"),
            },
            Pending::Forget | Pending::Discard => {}
        }
        self.dirty = true;
        true
    }

    /// Starts a sign-in. First the account's crypto items are read from the keychain (the device, its pickle key and
    /// its sessions); the sign-in job starts when the three answers are in (see [`MessagesWindow::keychain`]). A typed
    /// password is saved on success (see `signed_in`).
    fn start_sign_in(&mut self, ctx: &Ctx, homeserver: String, user: String, password: String, typed: bool) {
        if let Some(sign) = self.model.sign_in.as_mut() {
            sign.busy = true;
        }
        self.model.status = "Signing in...".into();
        for (prefix, item) in [(crypto::DEVICE_ITEM, Item::Device), (crypto::KEY_ITEM, Item::Key), (crypto::SESSIONS_ITEM, Item::Sessions)] {
            let request = keychain::get(ctx, &crypto::item_service(prefix, &homeserver), &user);
            self.pending.insert(request, Pending::Lookup(item));
        }
        self.lookup = Some(Lookup { homeserver, user, password, typed, waiting: 3, ..Lookup::default() });
    }

    /// Starts the sign-in job once the crypto items are read. The stored device id is the hint the login asks for.
    fn launch_sign_in(&mut self) {
        let Some(mut lookup) = self.lookup.take() else { return };
        // The sessions file wins over the keychain item of an earlier version, which is removed after the file is written.
        if let Some(key) = lookup.key.as_deref().and_then(crypto::parse_key_item) {
            match state_file::load(&self.data_dir, &lookup.homeserver, &lookup.user, &key) {
                Ok(Some(blob)) => lookup.sessions = Some(blob),
                Ok(None) => {}
                Err(e) => self.model.status = format!("The saved sessions could not be read: {e}"),
            }
        }
        let device_hint = lookup.device.as_deref().and_then(crypto::parse_device_item).map(|(device, _)| device.to_string());
        let (homeserver, user, password, typed) =
            (lookup.homeserver.clone(), lookup.user.clone(), lookup.password.clone(), lookup.typed);
        self.restore = Some(lookup);
        let transport = self.transport.clone();
        let tx = self.tx.clone();
        let wake = self.wake();
        sync::spawn(tx, wake, move || sync::sign_in(transport, homeserver, user, password, typed, device_hint));
    }

    /// Checks the sign-in sheet and starts the sign-in when every field is right.
    fn submit_sign_in(&mut self, ctx: &Ctx) {
        let Some(sign) = self.model.sign_in.as_mut() else { return };
        if sign.busy {
            return;
        }
        match sign.checked() {
            Ok((homeserver, user, password)) => {
                sign.error.clear();
                self.start_sign_in(ctx, homeserver, user, password, true);
            }
            Err(e) => sign.error = e,
        }
    }

    /// Writes the crypto state to the keychain when it changed (see [`crypto::Machine::snapshot`]). A write is an update;
    /// an item that does not exist yet is added (see [`MessagesWindow::keychain`]).
    fn save_crypto(&mut self, ctx: &Ctx) {
        let Some(api) = self.api.clone() else { return };
        let Some(snap) = api.take_snapshot() else { return };
        let Some(account) = self.account.clone() else { return };
        // The sessions are too large for an item: they go to the sessions file, sealed with the pickle key.
        if let (Some(sessions), Some(key)) = (snap.sessions.as_deref(), crypto::parse_key_item(&snap.key)) {
            match state_file::save(&self.data_dir, &account.homeserver, &account.user, &key, sessions) {
                Ok(()) => self.retire_sessions_item(ctx),
                Err(e) => self.model.status = format!("The encryption sessions could not be saved: {e}"),
            }
        }
        let moving = match self.migration.as_mut() {
            Some(m) if !m.items_moved && m.typed != account.user => {
                m.stores_left = 2;
                true
            }
            _ => false,
        };
        for (prefix, value) in [(crypto::KEY_ITEM, snap.key), (crypto::DEVICE_ITEM, snap.device)] {
            let service = crypto::item_service(prefix, &account.homeserver);
            let request = keychain::update(ctx, &service, &account.user, value.as_bytes());
            self.pending.insert(request, Pending::Store { service, account: account.user.clone(), value, added: false, moving });
        }
    }

    /// Removes the sessions keychain item of an earlier version, once the sessions file holds its sessions.
    fn retire_sessions_item(&mut self, ctx: &Ctx) {
        let Some(m) = self.migration.as_mut() else { return };
        if !m.sessions_item {
            return;
        }
        m.sessions_item = false;
        let typed = m.typed.clone();
        let homeserver = self.model_homeserver();
        let request = keychain::delete(ctx, &crypto::item_service(crypto::SESSIONS_ITEM, &homeserver), &typed);
        self.pending.insert(request, Pending::Discard);
        self.finish_migration();
    }

    /// One copy of the device or the key item to the stored name is confirmed. After both, the items under the bare name
    /// are removed.
    fn copy_confirmed(&mut self, ctx: &Ctx) {
        let Some(m) = self.migration.as_mut() else { return };
        if m.items_moved {
            return;
        }
        m.stores_left = m.stores_left.saturating_sub(1);
        if m.stores_left > 0 {
            return;
        }
        m.items_moved = true;
        let typed = m.typed.clone();
        let homeserver = self.model_homeserver();
        for prefix in [crypto::DEVICE_ITEM, crypto::KEY_ITEM] {
            let request = keychain::delete(ctx, &crypto::item_service(prefix, &homeserver), &typed);
            self.pending.insert(request, Pending::Discard);
        }
        self.finish_migration();
    }

    /// Ends the migration once nothing under the bare name is left to remove.
    fn finish_migration(&mut self) {
        if self.migration.as_ref().is_some_and(|m| m.items_moved && !m.sessions_item) {
            self.migration = None;
        }
    }

    /// Removes the Passwords login under a bare name, once the full id holds the password.
    fn forget_bare_login(&mut self, ctx: &Ctx, homeserver: &str, legacy: Option<String>) {
        if let Some(bare) = legacy {
            let request = passwords::remove(ctx, &accounts::service(homeserver), &bare);
            self.pending.insert(request, Pending::Forget);
        }
    }

    /// The homeserver of the signed-in login (empty before a sign-in).
    fn model_homeserver(&self) -> String {
        self.account.as_ref().map(|a| a.homeserver.clone()).unwrap_or_default()
    }

    /// The devices of the members of the selected room, as (user, device id), for the room info. All are unverified.
    pub fn devices_for_room(&self) -> Vec<(String, String)> {
        let Some(room) = self.model.selected_room() else { return Vec::new() };
        let users: Vec<String> = room.members.iter().map(|m| m.user_id.clone()).collect();
        self.api.as_ref().map(|api| api.device_list(&users)).unwrap_or_default()
    }

    /// The answer to one of the window's keychain requests: a crypto item read for a sign-in, or a crypto item written.
    /// False when the request was not the window's.
    pub fn keychain(&mut self, ctx: &Ctx, reply: keychain::Reply) -> bool {
        let Some(pending) = self.pending.remove(&reply.request) else { return false };
        match pending {
            Pending::Lookup(item) => {
                let value = reply.result.ok().and_then(|bytes| String::from_utf8(bytes).ok());
                let ready = match self.lookup.as_mut() {
                    Some(lookup) => {
                        match item {
                            Item::Device => lookup.device = value,
                            Item::Key => lookup.key = value,
                            Item::Sessions => {
                                lookup.sessions_legacy = value.is_some();
                                lookup.sessions = value;
                            }
                        }
                        lookup.waiting = lookup.waiting.saturating_sub(1);
                        lookup.waiting == 0
                    }
                    None => false,
                };
                if ready {
                    self.launch_sign_in();
                }
            }
            Pending::Store { service, account, value, added, moving } => match reply.result {
                Ok(_) => {
                    if moving {
                        self.copy_confirmed(ctx);
                    }
                }
                Err(keychain::Error::ItemNotFound) if !added => {
                    let request = keychain::add(ctx, &service, &account, value.as_bytes());
                    self.pending.insert(request, Pending::Store { service, account, value, added: true, moving });
                }
                Err(e) => self.model.status = format!("The encryption keys could not be saved in the keychain: {e:?}"),
            },
            Pending::Discard => {}
            other => {
                self.pending.insert(reply.request, other);
                return false;
            }
        }
        self.dirty = true;
        true
    }

    /// Puts a room on screen and marks its newest message read.
    fn open_room(&mut self, id: &str) {
        let was_marked = self.model.is_marked_unread(id);
        let marker = self.model.select(id);
        if was_marked {
            self.save_prefs();
            self.store_marked_unread(id, false);
        }
        self.timeline_scroll = Scroller::new();
        self.stick_bottom = true;
        self.send_read(marker);
    }

    /// Sends the read receipt for a marker from the model, on a job thread.
    fn send_read(&mut self, marker: Option<(String, String)>) {
        let (Some((room, event)), Some(api)) = (marker, self.api.clone()) else { return };
        sync::spawn(self.tx.clone(), self.wake(), move || sync::read(api, room, event));
    }

    /// The dock badge: the unread count of every room, or no badge at all when nothing is unread.
    fn badge(&self, ctx: &Ctx) {
        let total = self.model.total_unread();
        ctx.set_badge(&if total == 0 { String::new() } else { total.to_string() });
    }

    /// Sends the composer to the room on screen: a text, a reply, or the edit of one of the user's messages.
    fn send(&mut self) {
        let Some(room) = self.model.selected.clone() else {
            self.model.status = "Choose a conversation first".into();
            return;
        };
        let Some(api) = self.api.clone() else {
            self.model.status = "Sign in first".into();
            return;
        };
        let Some(outgoing) = self.model.take_outgoing() else { return };
        self.model.status = "Sending...".into();
        self.stick_bottom = true;
        let (tx, wake) = (self.tx.clone(), self.wake());
        match outgoing {
            Outgoing::Text(text) => sync::spawn(tx, wake, move || sync::send_text(api, room, text)),
            Outgoing::Reply { text, to } => sync::spawn(tx, wake, move || sync::send_reply(api, room, text, to)),
            Outgoing::Edit { text, target } => sync::spawn(tx, wake, move || sync::send_edit(api, room, target, text)),
        }
    }

    /// Asks for a file to send to the room on screen. The answer arrives in [`MessagesWindow::document`].
    fn pick_file(&mut self, ctx: &Ctx) {
        let Some(room) = self.model.selected.clone() else {
            self.model.status = "Choose a conversation first".into();
            return;
        };
        if self.api.is_none() {
            self.model.status = "Sign in first".into();
            return;
        }
        let request = ctx.open_document(DocMode::Read, "Attach File", &[]);
        self.picking = Some((request, room));
    }

    /// The file the user chose in the open panel: read it, then send it to the room it was picked for. A picture goes
    /// as a picture, anything else as a file.
    pub fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::Opened(mut doc) if self.picking.as_ref().is_some_and(|(r, _)| *r == doc.request) => {
                let Some((_, room)) = self.picking.take() else { return false };
                let bytes = read_file(&mut doc.file, doc.size);
                ctx.release_document(doc.grant);
                self.send_read_file(room, doc.name, bytes);
                self.dirty = true;
                true
            }
            DocumentEvent::Opened(mut doc) if self.is_dropping(doc.request) => {
                let bytes = read_file(&mut doc.file, doc.size);
                ctx.release_document(doc.grant);
                self.drop_arrived(doc.request, doc.name, bytes);
                self.dirty = true;
                true
            }
            DocumentEvent::OpenedMany { request, docs } if self.is_dropping(request) => {
                for mut doc in docs {
                    let bytes = read_file(&mut doc.file, doc.size);
                    ctx.release_document(doc.grant);
                    self.drop_arrived(request, doc.name, bytes);
                }
                self.dirty = true;
                true
            }
            _ => false,
        }
    }

    /// Sends a file that was read from disk to a room, or reports why it could not be read.
    fn send_read_file(&mut self, room: String, name: String, bytes: Result<Vec<u8>, String>) {
        match bytes {
            Ok(bytes) => self.send_bytes(room, name, bytes),
            Err(e) => self.model.status = format!("The file could not be read: {e}"),
        }
    }

    /// Sends the bytes of a file to a room: a picture goes as a picture, anything else as a file.
    fn send_bytes(&mut self, room: String, name: String, bytes: Vec<u8>) {
        let Some(api) = self.api.clone() else { return };
        self.stick_bottom = true;
        match protocol::image_mime(&name) {
            Some(mime) => {
                self.model.status = "Sending picture...".into();
                sync::spawn(self.tx.clone(), self.wake(), move || sync::send_image(api, room, name, mime, bytes));
            }
            None => {
                self.model.status = "Sending file...".into();
                let mime = protocol::file_mime(&name);
                sync::spawn(self.tx.clone(), self.wake(), move || sync::send_file(api, room, name, mime, bytes));
            }
        }
    }

    /// True when `request` is the session of the file drag this window is taking in.
    fn is_dropping(&self, request: u32) -> bool {
        self.file_drop.as_ref().is_some_and(|d| d.session == request)
    }

    /// The state of the drag `session`. A drag that replaces another starts fresh.
    fn drop_state(&mut self, session: u32) -> &mut FileDrop {
        let slot = self.file_drop.get_or_insert_with(|| FileDrop::new(session));
        if slot.session != session {
            *slot = FileDrop::new(session);
        }
        slot
    }

    /// A file of the drag arrives. Before the drop it waits; after the drop it goes straight to the conversation the
    /// drop landed on.
    fn drop_arrived(&mut self, session: u32, name: String, bytes: Result<Vec<u8>, String>) {
        let Some(state) = self.file_drop.as_mut().filter(|d| d.session == session) else { return };
        let Some(room) = state.room.clone() else {
            match bytes {
                Ok(bytes) => state.files.push((name, bytes)),
                Err(e) => self.model.status = format!("The file could not be read: {e}"),
            }
            return;
        };
        self.send_read_file(room, name, bytes);
    }

    /// A drag over the window or a drop on it. Files dropped while a conversation is on screen are sent to it; any other
    /// data is refused, as the drag offers no files.
    pub fn drag(&mut self, ctx: &Ctx, ev: dnd::Event) -> bool {
        let dnd::Event::Dest { session, phase, items, .. } = ev else { return false };
        let state = self.drop_state(session);
        if !items.is_empty() {
            state.offered = items.iter().any(|item| matches!(item.payload, Payload::File { .. }));
        }
        let offered = state.offered;
        match phase {
            DragPhase::Entered | DragPhase::Updated => {
                let op = if offered && self.model.accepts_drop() { dnd::dragop::COPY } else { 0 };
                dnd::reply(ctx, session, op);
            }
            DragPhase::Exited => {
                if self.is_dropping(session) {
                    self.file_drop = None;
                }
            }
            DragPhase::Perform => {
                let target = (offered && self.model.accepts_drop()).then(|| self.model.selected.clone()).flatten();
                let state = self.drop_state(session);
                state.room = target.clone();
                let held = std::mem::take(&mut state.files);
                dnd::performed(ctx, session, target.is_some());
                if let Some(room) = target {
                    for (name, bytes) in held {
                        self.send_bytes(room.clone(), name, bytes);
                    }
                }
            }
        }
        self.dirty = true;
        true
    }

    // conversation commands ---------------------------------------------------------------------------------------

    /// Saves the pinned, hidden-alert and unread choices.
    fn save_prefs(&mut self) {
        if let Err(e) = crate::store::save(&self.data_dir, &self.model.prefs) {
            self.model.status = format!("Conversation choices were not saved: {e}");
        }
    }

    fn new_message(&mut self) {
        if !self.model.signed_in() {
            return;
        }
        self.model.search_focus = false;
        self.model.overlay = Overlay::Prompt(Prompt::new_message());
    }

    fn pin(&mut self, ctx: &Ctx, id: &str) {
        if let Err(e) = self.model.toggle_pin(id) {
            self.model.status = e;
            return;
        }
        self.save_prefs();
        self.badge(ctx);
    }

    /// Moves a pinned conversation one place up or down among the pins.
    fn move_pin(&mut self, id: &str, up: bool) {
        if self.model.move_pin(id, up) {
            self.save_prefs();
        }
    }

    fn mute(&mut self, ctx: &Ctx, id: &str) {
        self.model.toggle_mute(id);
        self.save_prefs();
        self.badge(ctx);
    }

    fn mark_unread(&mut self, ctx: &Ctx, id: &str) {
        let marked = self.model.toggle_unread_mark(id);
        self.save_prefs();
        self.badge(ctx);
        self.store_marked_unread(id, marked);
    }

    /// Stores the unread flag of a conversation on the account, so the other devices show it too.
    fn store_marked_unread(&mut self, id: &str, marked: bool) {
        let Some(api) = self.api.clone() else { return };
        let room = id.to_string();
        sync::spawn(self.tx.clone(), self.wake(), move || sync::set_marked_unread(api, room, marked));
    }

    /// Scrolls the timeline to the message `id`, the one a reply quotes.
    fn jump_to_message(&mut self, ui: &Ui, id: &str) {
        let Some(offset) = self.message_offset(ui, id) else { return };
        self.stick_bottom = false;
        self.timeline_scroll.jump_to(offset);
        self.model.highlight_message(id);
    }

    fn open_emoji(&mut self, message: Option<String>) {
        if self.model.selected.is_some() {
            self.model.open_emoji(message);
        }
    }

    /// Reacts to `msg` with `key`, or takes the user's own reaction back when it is already there.
    fn react(&mut self, msg: &str, key: &str) {
        let (Some(room), Some(api)) = (self.model.selected.clone(), self.api.clone()) else { return };
        let own = self
            .model
            .selected_room()
            .and_then(|r| r.timeline.iter().find(|m| m.id == msg))
            .and_then(|m| m.reaction_of(&self.model.user_id, key).map(str::to_string));
        let (tx, wake) = (self.tx.clone(), self.wake());
        match own {
            Some(event) => sync::spawn(tx, wake, move || sync::redact(api, room, event, "Undo Reaction")),
            None => {
                let (msg, key) = (msg.to_string(), key.to_string());
                sync::spawn(tx, wake, move || sync::send_reaction(api, room, msg, key));
            }
        }
    }

    /// Takes back one of the user's own messages.
    fn unsend(&mut self, msg: &str) {
        let (Some(room), Some(api)) = (self.model.selected.clone(), self.api.clone()) else { return };
        let mine = self
            .model
            .selected_room()
            .and_then(|r| r.timeline.iter().find(|m| m.id == msg))
            .is_some_and(|m| m.sender == self.model.user_id);
        if !mine {
            return;
        }
        let msg = msg.to_string();
        sync::spawn(self.tx.clone(), self.wake(), move || sync::redact(api, room, msg, "Undo Send"));
    }

    /// A main-menu item, or a context-menu item on a room. True when a frame is needed.
    fn run_command(&mut self, ctx: &Ctx, id: u32) -> bool {
        if self.model.sign_in.is_some() {
            return false;
        }
        match id {
            MENU_NEW => self.new_message(),
            MENU_ATTACH => self.pick_file(ctx),
            MENU_EMOJI => self.open_emoji(None),
            MENU_SETTINGS => self.model.overlay = Overlay::Settings,
            MENU_FIND => self.model.search_focus = true,
            MENU_PIN | MENU_PIN_UP | MENU_PIN_DOWN | MENU_MUTE | MENU_UNREAD | MENU_DELETE | MENU_DETAILS => {
                let from_menu = match &self.ctx_target {
                    Some(CtxTarget::Room(room)) => Some(room.clone()),
                    _ => None,
                };
                let Some(room) = from_menu.or_else(|| self.model.selected.clone()) else { return false };
                match id {
                    MENU_PIN => self.pin(ctx, &room),
                    MENU_PIN_UP => self.move_pin(&room, true),
                    MENU_PIN_DOWN => self.move_pin(&room, false),
                    MENU_MUTE => self.mute(ctx, &room),
                    MENU_UNREAD => self.mark_unread(ctx, &room),
                    MENU_DELETE => self.model.overlay = Overlay::Confirm(room),
                    _ => self.model.overlay = Overlay::Details,
                }
            }
            _ => return false,
        }
        self.ctx_target = None;
        self.dirty = true;
        true
    }

    /// The File, Message and Conversation menus with their key equivalents. Recomputed after every event.
    pub fn main_menu(&self) -> Vec<AppMenu> {
        let signed_in = self.model.signed_in() && self.model.sign_in.is_none();
        let room = self.model.selected.as_deref();
        let has_room = room.is_some() && signed_in;
        let pinned = room.is_some_and(|id| self.model.is_pinned(id));
        let muted = room.is_some_and(|id| self.model.is_muted(id));
        let marked = room.is_some_and(|id| self.model.is_marked_unread(id));
        vec![
            AppMenu {
                title: "File".into(),
                items: vec![
                    AppMenuItem::new(MENU_NEW, "New Message").key("cmd+n").enabled(signed_in),
                    AppMenuItem::new(MENU_ATTACH, "Attach File...").key("cmd+o").enabled(has_room),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_SETTINGS, "Settings...").key("cmd+,").enabled(signed_in),
                ],
            },
            AppMenu {
                title: "Message".into(),
                items: vec![
                    AppMenuItem::new(MENU_FIND, "Find").key("cmd+f").enabled(signed_in),
                    AppMenuItem::new(MENU_EMOJI, "Emoji & Symbols").key("ctrl+cmd+space").enabled(has_room),
                ],
            },
            AppMenu {
                title: "Conversation".into(),
                items: vec![
                    AppMenuItem::new(MENU_PIN, if pinned { "Unpin" } else { "Pin" }).key("cmd+shift+p").enabled(has_room),
                    AppMenuItem::new(MENU_PIN_UP, "Move Pin Up").enabled(has_room && room.is_some_and(|id| self.model.can_move_pin(id, true))),
                    AppMenuItem::new(MENU_PIN_DOWN, "Move Pin Down").enabled(has_room && room.is_some_and(|id| self.model.can_move_pin(id, false))),
                    AppMenuItem::new(MENU_MUTE, if muted { "Show Alerts" } else { "Hide Alerts" }).key("cmd+shift+d").enabled(has_room),
                    AppMenuItem::new(MENU_UNREAD, if marked { "Mark as Read" } else { "Mark as Unread" }).key("cmd+shift+u").enabled(has_room),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_DETAILS, "Details...").key("cmd+i").enabled(has_room),
                    AppMenuItem::new(MENU_DELETE, "Delete Conversation...").key("cmd+backspace").enabled(has_room),
                ],
            },
        ]
    }

    /// A choice from the main menu. True when the window needs a frame.
    pub fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        self.ctx_target = None;
        self.run_command(ctx, id)
    }

    /// Opens the context menu for what is under (`x`, `y`): a conversation row or a message bubble.
    pub fn context(&mut self, ctx: &Ctx, x: f32, y: f32) -> bool {
        if self.model.sign_in.is_some() || self.model.overlay != Overlay::None {
            return false;
        }
        let item = |id: u32, label: &str, enabled: bool| ContextItem { id, label: label.to_string(), enabled };
        if let Some(room) = self.room_at(&ctx.ui, x, y) {
            let mut items = vec![item(MENU_PIN, if self.model.is_pinned(&room) { "Unpin" } else { "Pin" }, true)];
            if self.model.is_pinned(&room) {
                items.push(item(MENU_PIN_UP, "Move Pin Up", self.model.can_move_pin(&room, true)));
                items.push(item(MENU_PIN_DOWN, "Move Pin Down", self.model.can_move_pin(&room, false)));
            }
            items.extend([
                item(MENU_MUTE, if self.model.is_muted(&room) { "Show Alerts" } else { "Hide Alerts" }, true),
                item(MENU_UNREAD, if self.model.is_marked_unread(&room) { "Mark as Read" } else { "Mark as Unread" }, true),
                item(MENU_DELETE, "Delete Conversation...", true),
            ]);
            self.ctx_target = Some(CtxTarget::Room(room));
            ctx.context_menu(x, y, items);
            return false;
        }
        if let Some(id) = self.message_at(x, y) {
            let own = self
                .model
                .selected_room()
                .and_then(|r| r.timeline.iter().find(|m| m.id == id))
                .is_some_and(|m| m.sender == self.model.user_id);
            let text = self.message_text(&id).is_some();
            let (reacted, edited) = self
                .model
                .selected_room()
                .and_then(|r| r.timeline.iter().find(|m| m.id == id))
                .map_or((false, false), |m| (!m.reactions.is_empty(), m.edited));
            let mut items = vec![item(CTX_REPLY, "Reply", true)];
            for (i, (label, _)) in REACTIONS.iter().enumerate() {
                items.push(item(CTX_REACT + i as u32, label, true));
            }
            if reacted {
                items.push(item(CTX_REACTIONS, "Show Reactions...", true));
            }
            if edited {
                items.push(item(CTX_HISTORY, "Edit History...", true));
            }
            items.push(item(CTX_COPY, "Copy", text));
            if own {
                items.push(item(CTX_EDIT, "Edit", text));
                items.push(item(CTX_UNSEND, "Undo Send", true));
            }
            self.ctx_target = Some(CtxTarget::Message(id));
            ctx.context_menu(x, y, items);
        }
        false
    }

    /// The words of a text message of the room on screen.
    fn message_text(&self, id: &str) -> Option<String> {
        let room = self.model.selected_room()?;
        match &room.timeline.iter().find(|m| m.id == id)?.body {
            protocol::Body::Text(t) | protocol::Body::Notice(t) | protocol::Body::Emote(t) => Some(t.clone()),
            _ => None,
        }
    }

    /// A choice from the context menu. True when the window needs a frame.
    pub fn context_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let target = self.ctx_target.clone();
        match (id, target) {
            (MENU_PIN | MENU_PIN_UP | MENU_PIN_DOWN | MENU_MUTE | MENU_UNREAD | MENU_DELETE, Some(CtxTarget::Room(_))) => {
                return self.run_command(ctx, id)
            }
            (CTX_REPLY, Some(CtxTarget::Message(m))) => {
                self.model.begin_reply(&m);
            }
            (CTX_EDIT, Some(CtxTarget::Message(m))) => {
                self.model.begin_edit(&m);
            }
            (CTX_UNSEND, Some(CtxTarget::Message(m))) => self.unsend(&m),
            (CTX_REACTIONS, Some(CtxTarget::Message(m))) => self.model.overlay = Overlay::Info(InfoKind::Reactions, m),
            (CTX_HISTORY, Some(CtxTarget::Message(m))) => self.model.overlay = Overlay::Info(InfoKind::Edits, m),
            (CTX_COPY, Some(CtxTarget::Message(m))) => {
                if let Some(text) = self.message_text(&m) {
                    bifrost_app::kit::clipboard::set_text(&text);
                }
            }
            (i, Some(CtxTarget::Message(m))) if (CTX_REACT..CTX_REACT + REACTIONS.len() as u32).contains(&i) => {
                let key = REACTIONS[(i - CTX_REACT) as usize].1;
                self.react(&m, key);
            }
            _ => return false,
        }
        self.ctx_target = None;
        self.dirty = true;
        true
    }

    /// Signs out: tells the homeserver, forgets the saved login and every conversation.
    fn sign_out(&mut self, ctx: &Ctx) {
        if let Some(api) = self.api.take() {
            sync::spawn(self.tx.clone(), self.wake(), move || sync::logout(api));
        }
        let account = self.account.take();
        if let Some(account) = &account {
            let request = passwords::remove(ctx, &accounts::service(&account.homeserver), &account.user);
            self.pending.insert(request, Pending::Forget);
        }
        self.model.sign_out();
        self.model.sign_in = Some(match &account {
            Some(a) => SignIn::prefilled(&a.homeserver, &a.user, ""),
            None => SignIn::default(),
        });
        self.badge(ctx);
    }

    /// A button of the open sheet or panel.
    fn overlay_action(&mut self, ctx: &Ctx, action: OverlayAction) {
        match (action, self.model.overlay.clone()) {
            (OverlayAction::Cancel, _) => self.model.overlay = Overlay::None,
            (OverlayAction::Ok, Overlay::Prompt(p)) => self.submit_prompt(p),
            (OverlayAction::Ok, _) => self.model.overlay = Overlay::None,
            (OverlayAction::Rename, Overlay::Details) => {
                if let Some(room) = self.model.selected_room() {
                    let prompt = Prompt::rename(&room.id, &room.title(&self.model.user_id));
                    self.model.overlay = Overlay::Prompt(prompt);
                }
            }
            (OverlayAction::Delete, Overlay::Confirm(id)) => {
                self.model.overlay = Overlay::None;
                if let Some(api) = self.api.clone() {
                    self.model.status = "Leaving...".into();
                    sync::spawn(self.tx.clone(), self.wake(), move || sync::leave(api, id));
                }
            }
            (OverlayAction::SignOut, Overlay::Settings) => {
                self.model.overlay = Overlay::None;
                self.sign_out(ctx);
            }
            (OverlayAction::EmojiGroup(group), Overlay::Emoji(_)) => self.model.emoji_group = group,
            (OverlayAction::Pick(i), Overlay::Emoji(target)) => {
                self.model.overlay = Overlay::None;
                if let Some(emoji) = self.model.emoji_choices().get(i).cloned() {
                    self.model.note_emoji_used(&emoji);
                    self.save_prefs();
                    match target {
                        Some(msg) => self.react(&msg, &emoji),
                        None => self.model.composer.push_str(&emoji),
                    }
                }
            }
            _ => {}
        }
    }

    /// Checks the one-line sheet and starts what it asks for.
    fn submit_prompt(&mut self, prompt: Prompt) {
        if prompt.busy {
            return;
        }
        let value = match prompt.checked() {
            Ok(v) => v,
            Err(e) => {
                if let Overlay::Prompt(p) = &mut self.model.overlay {
                    p.error = e;
                }
                return;
            }
        };
        let Some(api) = self.api.clone() else { return };
        match prompt.kind {
            PromptKind::NewMessage => {
                if let Overlay::Prompt(p) = &mut self.model.overlay {
                    p.busy = true;
                    p.error.clear();
                }
                let users = Prompt::users(&value);
                sync::spawn(self.tx.clone(), self.wake(), move || sync::create_conversation(api, users));
            }
            PromptKind::Rename(room) => {
                self.model.overlay = Overlay::None;
                sync::spawn(self.tx.clone(), self.wake(), move || sync::rename(api, room, value));
            }
        }
    }

    pub fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32, t: f64) -> bool {
        match kind {
            TouchKind::Down => {
                self.model.clear_highlight();
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
                if press.region == Region::Timeline {
                    self.stick_bottom = false;
                }
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

    fn tap(&mut self, ctx: &Ctx, x: f32, y: f32) -> bool {
        let hit = self.hit(&ctx.ui, x, y);
        let had_overlay = self.model.overlay != Overlay::None;
        if !matches!(hit, Hit::Search) {
            self.model.search_focus = false;
        }
        match hit {
            Hit::Room(id) => self.open_room(&id),
            Hit::Send => self.send(),
            Hit::Image => self.pick_file(ctx),
            Hit::Search => self.model.search_focus = true,
            Hit::NewMessage => self.new_message(),
            Hit::Info => self.model.overlay = Overlay::Details,
            Hit::EmojiButton => self.open_emoji(None),
            Hit::CancelCompose => self.model.cancel_compose(),
            Hit::Chip { msg, key } => self.react(&msg, &key),
            Hit::Quote(target) => self.jump_to_message(&ctx.ui, &target),
            Hit::Overlay(action) => self.overlay_action(ctx, action),
            Hit::Field(i) => {
                if let Some(sign) = self.model.sign_in.as_mut() {
                    sign.focus = i;
                }
            }
            Hit::SignIn => self.submit_sign_in(ctx),
            Hit::Nothing if had_overlay => {
                // A tap outside a panel closes it, except a sheet that is busy.
                let busy = matches!(&self.model.overlay, Overlay::Prompt(p) if p.busy);
                if !busy {
                    self.model.overlay = Overlay::None;
                }
            }
            Hit::Panel | Hit::Nothing => return false,
        }
        self.dirty = true;
        true
    }

    fn region(&self, ui: &Ui, x: f32, y: f32) -> Region {
        if self.model.sign_in.is_some() || self.model.overlay != Overlay::None {
            return Region::Other;
        }
        let lay = self.layout(ui);
        if inside(lay.sidebar(), x, y) {
            Region::Rooms
        } else if inside(lay.timeline(), x, y) {
            Region::Timeline
        } else {
            Region::Other
        }
    }

    fn scroller(&mut self, region: Region) -> Option<&mut Scroller> {
        match region {
            Region::Rooms => Some(&mut self.room_scroll),
            Region::Timeline => Some(&mut self.timeline_scroll),
            Region::Other => None,
        }
    }

    /// Typed text: into the focused sign-in field while the sheet is open, then the open sheet's field, then the search
    /// field when it has the keyboard, otherwise into the composer.
    fn type_text(&mut self, text: &str) {
        if let Some(sign) = self.model.sign_in.as_mut() {
            sign.type_text(text);
            return;
        }
        match &mut self.model.overlay {
            Overlay::Prompt(p) => {
                p.value.push_str(text);
                p.error.clear();
            }
            Overlay::Emoji(_) => self.model.emoji_query.push_str(text),
            Overlay::None => {
                if self.model.search_focus {
                    self.model.search.push_str(text);
                    self.room_scroll = Scroller::new();
                } else {
                    self.model.composer.push_str(text);
                }
            }
            _ => {}
        }
    }

    fn backspace(&mut self) {
        if let Some(sign) = self.model.sign_in.as_mut() {
            sign.backspace();
            return;
        }
        match &mut self.model.overlay {
            Overlay::Prompt(p) => {
                p.value.pop();
            }
            Overlay::Emoji(_) => {
                self.model.emoji_query.pop();
            }
            Overlay::None => {
                if self.model.search_focus {
                    self.model.search.pop();
                    self.room_scroll = Scroller::new();
                } else {
                    self.model.composer.pop();
                }
            }
            _ => {}
        }
    }

    pub fn key(&mut self, ctx: &Ctx, k: KeyInput) -> bool {
        if let Some((key, mods)) = k.as_chord() {
            return key == KEY_V && mods & (CHORD_CTRL | CHORD_META) != 0 && self.paste(ctx);
        }
        match k {
            KeyInput::Text(s) => {
                let text: String = s.chars().filter(|c| !c.is_control()).collect();
                self.type_text(&text);
            }
            KeyInput::Code(KEY_RETURN) => {
                if self.model.sign_in.is_some() {
                    self.submit_sign_in(ctx);
                } else if let Overlay::Prompt(p) = self.model.overlay.clone() {
                    self.submit_prompt(p);
                } else if matches!(self.model.overlay, Overlay::Emoji(_)) {
                    // Return picks the first emoji the search shows.
                    if self.model.emoji_choices().is_empty() {
                        return false;
                    }
                    self.overlay_action(ctx, OverlayAction::Pick(0));
                } else if self.model.overlay != Overlay::None {
                    return false;
                } else if self.model.search_focus {
                    // Return in the search field hands the keyboard back to the composer.
                    self.model.search_focus = false;
                } else {
                    self.send();
                }
            }
            KeyInput::Code(KEY_TAB) => {
                if let Some(sign) = self.model.sign_in.as_mut() {
                    sign.next_field();
                }
            }
            KeyInput::Code(KEY_BACKSPACE) => self.backspace(),
            KeyInput::Code(KEY_ESCAPE) => {
                if self.model.sign_in.is_some() {
                    return false;
                }
                if self.model.overlay != Overlay::None {
                    let busy = matches!(&self.model.overlay, Overlay::Prompt(p) if p.busy);
                    if !busy {
                        self.model.overlay = Overlay::None;
                    }
                } else if self.model.search_focus || !self.model.search.is_empty() {
                    self.model.search.clear();
                    self.model.search_focus = false;
                } else if self.model.compose.is_some() {
                    self.model.cancel_compose();
                } else {
                    return false;
                }
            }
            KeyInput::Code(_) => return false,
        }
        self.dirty = true;
        true
    }

    pub fn paste(&mut self, ctx: &Ctx) -> bool {
        if !matches!(self.model.overlay, Overlay::None | Overlay::Prompt(_) | Overlay::Emoji(_)) {
            return false;
        }
        let Some(text) = ctx.pasteboard_string() else { return false };
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        self.type_text(&text);
        self.dirty = true;
        true
    }
}

/// The bytes of a chosen file. A file past the upload limit is refused before it is read.
fn read_file(file: &mut File, size: u64) -> Result<Vec<u8>, String> {
    let too_big = || format!("The file is larger than {} MB", UPLOAD_LIMIT / (1024 * 1024));
    if size > UPLOAD_LIMIT as u64 {
        return Err(too_big());
    }
    let mut bytes = Vec::new();
    (&mut *file).take(UPLOAD_LIMIT as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > UPLOAD_LIMIT {
        return Err(too_big());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn a_wake_pair_is_connected_and_nonblocking() {
        let (mut rx, mut tx) = wake_pair().expect("a socket pair");
        assert_eq!(tx.write(&[1]).unwrap(), 1);
        let mut buf = [0u8; 4];
        assert_eq!(rx.read(&mut buf).unwrap(), 1);
        assert!(rx.read(&mut buf).is_err(), "no byte left, and the read does not block");
    }

    #[test]
    fn a_file_past_the_limit_is_refused_before_it_is_read() {
        let dir = std::env::temp_dir().join(format!("messages-picture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.png");
        std::fs::write(&path, b"png bytes").unwrap();
        let mut file = File::open(&path).unwrap();
        assert_eq!(read_file(&mut file, 9).unwrap(), b"png bytes".to_vec());
        let mut again = File::open(&path).unwrap();
        assert!(read_file(&mut again, UPLOAD_LIMIT as u64 + 1).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_picture_types_are_the_ones_the_protocol_sends() {
        for ext in PICTURE_TYPES {
            assert!(protocol::image_mime(&format!("x.{ext}")).is_some(), "{ext}");
        }
    }
}
