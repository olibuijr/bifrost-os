//! The Mail window: the state in [`crate::state`] wired to the network jobs ([`crate::sync`]), the keychain for the
//! passwords and the notifications for new mail. This file holds the flow and the input; drawing and hit-testing are in
//! [`crate::draw`].

use crate::account::{Account, KEYCHAIN_SERVICE};
use crate::message;
use crate::state::{Mail, Notice, Sheet, INBOX};
use crate::store;
use crate::sync::{self, Event};
use crate::transport;
use bifrost_app::kit::{clipboard, Scroller, TextCache, Ui};
use bifrost_app::keychain::{self, Error as KeyError, Reply as KeyReply};
use bifrost_app::usernotif::{self, Request};
use bifrost_app::{AppMenu, AppMenuItem, Ctx, KeyInput, TouchKind};
use bifrost_app::proto::{CHORD_CTRL, CHORD_META};
use springboard::deildu::http::Transport;
use std::collections::HashMap;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

pub const MENU_NEW: u32 = 1;
pub const MENU_REPLY: u32 = 2;
pub const MENU_SEND: u32 = 3;
pub const MENU_REFRESH: u32 = 4;
pub const MENU_ADD: u32 = 5;

/// How often the inbox of every account is checked for new mail, in milliseconds.
pub const POLL_MS: f64 = 60_000.0;
/// Movement, in points, before a press counts as a drag.
const DRAG_PT: f32 = 6.0;
/// Key codes the shell sends for the editing keys (as the other apps read them).
const KEY_ESCAPE: u32 = 1;
const KEY_BACKSPACE: u32 = 8;
const KEY_TAB: u32 = 9;
const KEY_RETURN: u32 = 13;
/// evdev V, for the paste chord.
const KEY_V: u32 = 47;

/// What the window is waiting for from the keychain.
enum Pending {
    /// The password of an account, read when the app starts.
    Load(String),
    /// A new account's password, being added. `Replace` follows when an item already exists.
    Add { account: Account, password: String },
    Replace { account: Account, password: String },
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
    dir: PathBuf,
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
    /// Reads the accounts and asks the keychain for each password. Mail is fetched as the passwords arrive.
    pub fn new(ctx: &Ctx) -> MailWindow {
        let dir = ctx.data_dir.clone();
        let (accounts, load_error) = match store::load(&dir) {
            Ok(accounts) => (accounts, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let (tx, rx) = channel();
        let (wake_rx, wake_tx) = match wake_pair() {
            Some((rx, tx)) => (Some(rx), Some(tx)),
            None => (None, None),
        };
        let mut mail = Mail::new(accounts);
        if let Some(e) = load_error {
            mail.status = e;
        }
        let mut window = MailWindow {
            dir,
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
            let request = keychain::get(ctx, KEYCHAIN_SERVICE, &account.keychain_account());
            window.pending.insert(request, Pending::Load(account.id.clone()));
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
            self.jobs = self.jobs.saturating_sub(1);
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

    /// The keychain's answer to one of the requests the window made.
    pub fn keychain(&mut self, ctx: &Ctx, reply: KeyReply) -> bool {
        let Some(pending) = self.pending.remove(&reply.request) else { return false };
        match pending {
            Pending::Load(id) => match reply.result {
                Ok(bytes) => {
                    self.mail.passwords.insert(id.clone(), String::from_utf8_lossy(&bytes).into_owned());
                    self.refresh_account(&id);
                }
                Err(KeyError::ItemNotFound) => {
                    let name = self.account_name(&id);
                    self.mail.status = format!("No password is saved for {name}. Add the account again");
                }
                Err(e) => self.mail.status = format!("The keychain could not return the password: {e:?}"),
            },
            Pending::Add { account, password } => match reply.result {
                Ok(_) => self.account_saved(account, password),
                Err(KeyError::DuplicateItem) => {
                    let request = keychain::update(ctx, KEYCHAIN_SERVICE, &account.keychain_account(), password.as_bytes());
                    self.pending.insert(request, Pending::Replace { account, password });
                }
                Err(e) => self.mail.status = format!("The password could not be saved: {e:?}"),
            },
            Pending::Replace { account, password } => match reply.result {
                Ok(_) => self.account_saved(account, password),
                Err(e) => self.mail.status = format!("The password could not be saved: {e:?}"),
            },
        }
        self.dirty = true;
        true
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
                ],
            },
            AppMenu {
                title: "Mailbox".into(),
                items: vec![AppMenuItem::new(MENU_REFRESH, "Refresh").key("cmd+r"), AppMenuItem::separator(), AppMenuItem::new(MENU_ADD, "Add Account...")],
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
            Event::Body { account, mailbox, uid, result } => match result {
                Ok(text) => self.mail.body_arrived(&account, &mailbox, uid, text),
                Err(e) => self.mail.status = format!("The message could not be loaded: {e}"),
            },
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

    fn account_saved(&mut self, account: Account, password: String) {
        self.mail.add_account(account, password);
        self.save_accounts();
        self.mail.close_sheet();
        self.refresh_selected();
    }

    fn save_accounts(&mut self) {
        if let Err(e) = store::save(&self.dir, &self.mail.accounts) {
            self.mail.status = format!("The accounts file could not be saved: {e}");
        }
    }

    fn account_name(&self, id: &str) -> String {
        self.mail.accounts.iter().find(|a| a.id == id).map_or_else(|| id.to_string(), |a| a.name.clone())
    }

    /// Starts a listing of `mailbox` for the account at `index`. Nothing happens until its password is known.
    fn refresh(&mut self, index: usize, mailbox: &str) {
        let Some(account) = self.mail.accounts.get(index).cloned() else { return };
        let Some(password) = self.mail.password(&account).map(str::to_string) else { return };
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

    /// The periodic check: the inbox of every account (for notices) and the shown mailbox.
    fn poll(&mut self) {
        for index in 0..self.mail.accounts.len() {
            self.refresh(index, INBOX);
        }
        let mailbox = self.mail.mailbox.clone();
        if mailbox != INBOX {
            let index = self.mail.account;
            self.refresh(index, &mailbox);
        }
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

    fn reply(&mut self) {
        let Some(uid) = self.mail.open else { return };
        let original = self.mail.rows.iter().find(|r| r.uid == uid).map(|r| r.summary.clone());
        if let Some(original) = original {
            self.mail.open_compose(Some(&original));
        }
    }

    /// Runs the sheet's primary action: add the account, or send the message.
    fn submit(&mut self, ctx: &Ctx) {
        match self.mail.sheet.clone() {
            Some(Sheet::Setup(form)) => match self.mail.account_from_form(&form) {
                Ok((account, password)) => {
                    let request = keychain::add(ctx, KEYCHAIN_SERVICE, &account.keychain_account(), password.as_bytes());
                    self.pending.insert(request, Pending::Add { account, password });
                    self.mail.status = "Saving the password".into();
                }
                Err(e) => self.mail.status = e,
            },
            Some(Sheet::Compose(compose)) => self.send(&compose),
            None => {}
        }
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
            Sheet::Setup(form) => Some(form),
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
