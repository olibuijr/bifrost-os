//! The Messages window: the model in [`crate::model`] wired to the Matrix jobs ([`crate::sync`]), the saved accounts in
//! Passwords ([`crate::accounts`]) and the notifications. This file holds the flow and the input; drawing and hit-testing
//! are in [`crate::draw`].

use crate::accounts::{self, Account};
use crate::api::{Api, UPLOAD_LIMIT};
use crate::draw::{inside, Hit, Layout};
use crate::model::{Messages, SignIn};
use crate::protocol;
use crate::sync::{self, Event};
use bifrost_app::kit::{Scroller, TextCache, Ui};
use bifrost_app::passwords::{self, Error as KeyError};
use bifrost_app::proto::{DocMode, CHORD_CTRL, CHORD_META};
use bifrost_app::{Ctx, DocumentEvent, KeyInput, TouchKind};
use bifrost_core::json::Json;
use springboard::deildu::http::{Tls, Transport};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{channel, Receiver, Sender};
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
/// The picture types the app sends.
const PICTURE_TYPES: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

/// What the window is waiting for from Passwords (through `service_reply`).
enum Pending {
    /// The list of saved Matrix logins.
    Accounts,
    /// The password of a saved login, read to sign in.
    Password(Account),
    /// A login saved after a typed sign-in: added, or replaced when an item already exists.
    Save { account: Account, password: String },
    Replace,
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

pub struct MessagesWindow {
    pub model: Messages,
    transport: Arc<dyn Transport>,
    /// The signed-in session, once a sign-in succeeded.
    api: Option<Arc<Api>>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    wake_rx: Option<UnixStream>,
    wake_tx: Option<UnixStream>,
    pending: HashMap<u32, Pending>,
    /// The picture the user is choosing, with the room it is for.
    picking: Option<(u32, String)>,
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
        let (tx, rx) = channel();
        let (wake_rx, wake_tx) = match wake_pair() {
            Some((rx, tx)) => (Some(rx), Some(tx)),
            None => (None, None),
        };
        let mut window = MessagesWindow {
            model: Messages::default(),
            transport,
            api: None,
            tx,
            rx,
            wake_rx,
            wake_tx,
            pending: HashMap::new(),
            picking: None,
            w: 0,
            h: 0,
            room_scroll: Scroller::new(),
            timeline_scroll: Scroller::new(),
            stick_bottom: true,
            press: None,
            labels: TextCache::default(),
            dirty: true,
        };
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
        while let Ok(event) = self.rx.try_recv() {
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
            Event::Synced(Ok(sync)) => {
                let notices = self.model.apply_sync(&sync);
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
        }
        self.dirty = true;
    }

    fn signed_in(&mut self, ctx: &Ctx, api: Arc<Api>, homeserver: String, user: String, password: String, typed: bool) {
        self.model.user_id = api.user_id.clone();
        self.model.sign_in = None;
        self.model.status.clear();
        sync::run_sync(api.clone(), self.tx.clone(), self.wake());
        self.api = Some(api);
        if typed {
            let request = passwords::save(ctx, &accounts::service(&homeserver), &user, &password);
            self.pending.insert(request, Pending::Save { account: Account { homeserver, user }, password });
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
                Ok(password) => self.start_sign_in(account.homeserver, account.user, password, false),
                Err(e) => {
                    self.model.status = match e {
                        KeyError::ItemNotFound => "No password is saved for this account".into(),
                        other => format!("Passwords could not return the password: {other:?}"),
                    };
                    self.model.sign_in = Some(SignIn::prefilled(&account.homeserver, &account.user, ""));
                }
            },
            Pending::Save { account, password } => match passwords::decode_password(result) {
                Ok(_) => {}
                Err(KeyError::DuplicateItem) => {
                    let request = passwords::update(ctx, &accounts::service(&account.homeserver), &account.user, &password);
                    self.pending.insert(request, Pending::Replace);
                }
                Err(e) => self.model.status = format!("The password could not be saved in Passwords: {e:?}"),
            },
            Pending::Replace => {
                if let Err(e) = passwords::decode_password(result) {
                    self.model.status = format!("The password could not be saved in Passwords: {e:?}");
                }
            }
        }
        self.dirty = true;
        true
    }

    /// Signs in on a job thread. A typed password is saved on success (see `signed_in`).
    fn start_sign_in(&mut self, homeserver: String, user: String, password: String, typed: bool) {
        if let Some(sign) = self.model.sign_in.as_mut() {
            sign.busy = true;
        }
        self.model.status = "Signing in...".into();
        let transport = self.transport.clone();
        let tx = self.tx.clone();
        let wake = self.wake();
        sync::spawn(tx, wake, move || sync::sign_in(transport, homeserver, user, password, typed));
    }

    /// Checks the sign-in sheet and starts the sign-in when every field is right.
    fn submit_sign_in(&mut self) {
        let Some(sign) = self.model.sign_in.as_mut() else { return };
        if sign.busy {
            return;
        }
        match sign.checked() {
            Ok((homeserver, user, password)) => {
                sign.error.clear();
                self.start_sign_in(homeserver, user, password, true);
            }
            Err(e) => sign.error = e,
        }
    }

    /// Puts a room on screen and marks its newest message read.
    fn open_room(&mut self, id: &str) {
        let marker = self.model.select(id);
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

    /// Sends the composer's text to the room on screen.
    fn send(&mut self) {
        let Some(room) = self.model.selected.clone() else {
            self.model.status = "Choose a conversation first".into();
            return;
        };
        let Some(api) = self.api.clone() else {
            self.model.status = "Sign in first".into();
            return;
        };
        let Some(text) = self.model.take_composer() else { return };
        self.model.status = "Sending...".into();
        self.stick_bottom = true;
        sync::spawn(self.tx.clone(), self.wake(), move || sync::send_text(api, room, text));
    }

    /// Asks for a picture to send to the room on screen. The answer arrives in [`MessagesWindow::document`].
    fn pick_image(&mut self, ctx: &Ctx) {
        let Some(room) = self.model.selected.clone() else {
            self.model.status = "Choose a conversation first".into();
            return;
        };
        if self.api.is_none() {
            self.model.status = "Sign in first".into();
            return;
        }
        let request = ctx.open_document(DocMode::Read, "Send Image", &PICTURE_TYPES);
        self.picking = Some((request, room));
    }

    /// The file the user chose in the open panel: read it, then send it to the room it was picked for.
    pub fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::Opened(mut doc) if self.picking.as_ref().is_some_and(|(r, _)| *r == doc.request) => {
                let Some((_, room)) = self.picking.take() else { return false };
                let mime = protocol::image_mime(&doc.name);
                let bytes = read_picture(&mut doc.file, doc.size);
                ctx.release_document(doc.grant);
                match (mime, bytes) {
                    (Some(mime), Ok(bytes)) => {
                        if let Some(api) = self.api.clone() {
                            self.model.status = "Sending picture...".into();
                            self.stick_bottom = true;
                            let name = doc.name.clone();
                            sync::spawn(self.tx.clone(), self.wake(), move || sync::send_image(api, room, name, mime, bytes));
                        }
                    }
                    (None, _) => self.model.status = "Only PNG, JPEG, GIF and WebP pictures can be sent".into(),
                    (_, Err(e)) => self.model.status = format!("The picture could not be read: {e}"),
                }
                self.dirty = true;
                true
            }
            _ => false,
        }
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
        match self.hit(&ctx.ui, x, y) {
            Hit::Room(id) => self.open_room(&id),
            Hit::Send => self.send(),
            Hit::Image => self.pick_image(ctx),
            Hit::Field(i) => {
                if let Some(sign) = self.model.sign_in.as_mut() {
                    sign.focus = i;
                }
            }
            Hit::SignIn => self.submit_sign_in(),
            Hit::Panel | Hit::Nothing => return false,
        }
        self.dirty = true;
        true
    }

    fn region(&self, ui: &Ui, x: f32, y: f32) -> Region {
        if self.model.sign_in.is_some() {
            return Region::Other;
        }
        let lay = Layout::new(ui, self.w as f32, self.h as f32);
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

    /// Typed text: into the focused sign-in field while the sheet is open, otherwise into the composer.
    fn type_text(&mut self, text: &str) {
        match self.model.sign_in.as_mut() {
            Some(sign) => sign.type_text(text),
            None => self.model.composer.push_str(text),
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
                    self.submit_sign_in();
                } else {
                    self.send();
                }
            }
            KeyInput::Code(KEY_TAB) => {
                if let Some(sign) = self.model.sign_in.as_mut() {
                    sign.next_field();
                }
            }
            KeyInput::Code(KEY_BACKSPACE) => match self.model.sign_in.as_mut() {
                Some(sign) => sign.backspace(),
                None => {
                    self.model.composer.pop();
                }
            },
            KeyInput::Code(KEY_ESCAPE) => return false,
            KeyInput::Code(_) => return false,
        }
        self.dirty = true;
        true
    }

    pub fn paste(&mut self, ctx: &Ctx) -> bool {
        let Some(text) = ctx.pasteboard_string() else { return false };
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        self.type_text(&text);
        self.dirty = true;
        true
    }
}

/// The bytes of a chosen picture. A picture past the upload limit is refused before it is read.
fn read_picture(file: &mut File, size: u64) -> Result<Vec<u8>, String> {
    let too_big = || format!("The picture is larger than {} MB", UPLOAD_LIMIT / (1024 * 1024));
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
    fn a_picture_past_the_limit_is_refused_before_it_is_read() {
        let dir = std::env::temp_dir().join(format!("messages-picture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.png");
        std::fs::write(&path, b"png bytes").unwrap();
        let mut file = File::open(&path).unwrap();
        assert_eq!(read_picture(&mut file, 9).unwrap(), b"png bytes".to_vec());
        let mut again = File::open(&path).unwrap();
        assert!(read_picture(&mut again, UPLOAD_LIMIT as u64 + 1).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_picture_types_are_the_ones_the_protocol_sends() {
        for ext in PICTURE_TYPES {
            assert!(protocol::image_mime(&format!("x.{ext}")).is_some(), "{ext}");
        }
    }
}
