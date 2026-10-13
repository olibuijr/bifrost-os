//! Typed drag and drop between app windows (`NativeDraggingSource`, `NativeDraggingDestination`, `registerForDraggedTypes`).
//!
//! A source app sends `ToShell::DragBegin` (session id with `sharing::INBOUND` set, the operations it allows, the items with
//! the UTTypes they are offered as, and a small preview image). The shell then owns the pointer: it hit-tests the app
//! windows, and for a window whose registered types (`ToShell::DragRegister`) conform to a type of an item it sends
//! `ToApp::DragDest` `Entered`, `Updated` and `Exited`. The destination answers `Entered` and `Updated` with
//! `ToShell::DragReply` (the single `dragop` bit it would perform, 0 for none). On release over a destination that replied
//! with an operation the shell hands over the files (fresh read-only grants, `DocGranted` with request = session) and sends
//! `Perform`; the destination answers `DragPerformed`, and the source hears `ToApp::DragEnded` with the operation (0 when
//! nothing took the drop).
//!
//! `Session` is the pure state machine; `Shell::dnd_*` feed it from the sockets and the pointer.

use super::sharing::INBOUND;
use super::apps::apps;
use super::AppId;
use bifrost_proto::{dragop, DocMode, DragItem, DragPhase, DragPreview, GrantId, Payload, ToApp, ToShell};
use std::collections::HashMap;
use std::path::Path;

/// Where a drop that no app window takes goes: a Dock icon, or the desktop.
#[derive(Clone, Debug, PartialEq)]
pub enum Zone {
    /// the Dock icon of the app with this icon key: the files open in it, as Open With does
    Dock(String),
    /// the desktop: the files are copied into the account's Desktop folder
    Desktop,
}

/// The window under the pointer, with the pointer in that window's coordinates (the same space as `Touch`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub slot: u8,
    pub x: f32,
    pub y: f32,
}

/// What the shell must do after a pointer step.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    /// send `DragDest` to `slot` (items are attached for `Entered` and `Perform`)
    Dest { slot: u8, phase: DragPhase, x: f32, y: f32 },
    /// tell the source the session ended with `op`
    Ended { op: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Over {
    slot: u8,
    x: f32,
    y: f32,
    /// the operation the destination last replied with
    op: u8,
}

pub struct Session {
    pub id: u32,
    pub source: u8,
    pub source_app: String,
    pub allowed: u8,
    pub items: Vec<DragItem>,
    pub preview: DragPreview,
    /// pointer in screen px, once it moved
    pub pos: Option<(f32, f32)>,
    over: Option<Over>,
    /// the destination `Perform` went to; the session ends when it answers
    pub performing: Option<u8>,
}

/// True when a window that registered `registered` takes at least one of `items` (`NativeDraggingInfo` types match): some type
/// of some item conforms to a registered type.
pub fn accepts(registered: &[String], items: &[DragItem]) -> bool {
    items.iter().any(|i| i.types.iter().any(|t| registered.iter().any(|r| bifrost_ui::uttype::conforms(t, r))))
}

/// The operation a reply selects: the destination's bit if the source allows it, else none.
pub fn negotiate(allowed: u8, reply: u8) -> u8 {
    if reply.count_ones() == 1 && allowed & reply == reply {
        reply
    } else {
        0
    }
}

impl Session {
    pub fn new(id: u32, source: u8, source_app: &str, allowed: u8, items: Vec<DragItem>, preview: DragPreview) -> Self {
        Session { id, source, source_app: source_app.into(), allowed, items, preview, pos: None, over: None, performing: None }
    }

    /// The destination window the session is over.
    pub fn over(&self) -> Option<u8> {
        self.over.map(|o| o.slot)
    }

    /// The operation currently agreed (0 until the destination replies).
    pub fn operation(&self) -> u8 {
        self.over.map_or(0, |o| o.op)
    }

    /// The pointer moved to `target` (None: not over an app window). `takes(slot)` says whether that window registered for
    /// the items. Returns the messages to send.
    pub fn moved(&mut self, target: Option<Target>, takes: &dyn Fn(u8) -> bool) -> Vec<Out> {
        let target = target.filter(|t| takes(t.slot));
        let mut out = Vec::new();
        match (self.over, target) {
            (None, None) => {}
            (None, Some(t)) => {
                self.over = Some(Over { slot: t.slot, x: t.x, y: t.y, op: 0 });
                out.push(Out::Dest { slot: t.slot, phase: DragPhase::Entered, x: t.x, y: t.y });
            }
            (Some(o), Some(t)) if o.slot == t.slot => {
                self.over = Some(Over { x: t.x, y: t.y, ..o });
                out.push(Out::Dest { slot: t.slot, phase: DragPhase::Updated, x: t.x, y: t.y });
            }
            (Some(o), Some(t)) => {
                out.push(Out::Dest { slot: o.slot, phase: DragPhase::Exited, x: o.x, y: o.y });
                self.over = Some(Over { slot: t.slot, x: t.x, y: t.y, op: 0 });
                out.push(Out::Dest { slot: t.slot, phase: DragPhase::Entered, x: t.x, y: t.y });
            }
            (Some(o), None) => {
                out.push(Out::Dest { slot: o.slot, phase: DragPhase::Exited, x: o.x, y: o.y });
                self.over = None;
            }
        }
        out
    }

    /// The destination `slot` replied to `Entered` / `Updated` with `op`. Stale replies (another window) are ignored.
    pub fn reply(&mut self, slot: u8, op: u8) {
        let allowed = self.allowed;
        if let Some(o) = self.over.as_mut().filter(|o| o.slot == slot) {
            o.op = negotiate(allowed, op);
        }
    }

    /// The pointer was released. Over a destination that agreed to an operation this is `Perform` (the session then waits
    /// for `performed`); otherwise the destination, if any, hears `Exited` and the session ends with nothing.
    pub fn release(&mut self) -> Vec<Out> {
        match self.over {
            Some(o) if o.op != 0 => {
                self.performing = Some(o.slot);
                vec![Out::Dest { slot: o.slot, phase: DragPhase::Perform, x: o.x, y: o.y }]
            }
            Some(o) => {
                self.over = None;
                vec![Out::Dest { slot: o.slot, phase: DragPhase::Exited, x: o.x, y: o.y }, Out::Ended { op: 0 }]
            }
            None => vec![Out::Ended { op: 0 }],
        }
    }

    /// `performDragOperation` answered.
    pub fn performed(&mut self, accepted: bool) -> Vec<Out> {
        let op = if accepted { self.operation() } else { 0 };
        self.performing = None;
        self.over = None;
        vec![Out::Ended { op }]
    }

    /// The session is abandoned (the source cancelled, or an app involved went away).
    pub fn cancel(&mut self) -> Vec<Out> {
        let mut out = Vec::new();
        if let Some(o) = self.over.take() {
            out.push(Out::Dest { slot: o.slot, phase: DragPhase::Exited, x: o.x, y: o.y });
        }
        out.push(Out::Ended { op: 0 });
        out
    }
}

/// The preview as a premultiplied image for the compositor (straight RGBA in, `gfx::rgba` out).
pub fn preview_image(p: &DragPreview) -> Option<bifrost_render::gfx::Image> {
    if !p.valid() || p.w == 0 || p.h == 0 {
        return None;
    }
    let px = p.rgba.chunks_exact(4).map(|c| bifrost_render::gfx::rgba(c[0], c[1], c[2], c[3])).collect();
    Some(bifrost_render::gfx::Image { w: p.w as usize, h: p.h as usize, px })
}

/// The name a copy takes in a folder that already holds `taken`: `name`, else `name 2`, `name 3`... (before the extension).
pub fn free_name(taken: &[String], name: &str) -> String {
    if !taken.iter().any(|t| t == name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    (2..).map(|n| format!("{stem} {n}{ext}")).find(|c| !taken.iter().any(|t| t == c)).unwrap_or_else(|| name.to_string())
}

/// Checks the items of a new session: at least one, every item offered as at least one type, and only files the caller can
/// name. (The broker checks the grants themselves when the files are handed over.)
pub fn valid_items(items: &[DragItem]) -> bool {
    !items.is_empty() && items.iter().all(|i| !i.types.is_empty() && !matches!(&i.payload, Payload::Text(t) | Payload::Url(t) if t.is_empty()))
}

#[derive(Default)]
pub struct State {
    pub session: Option<Session>,
    /// `registerForDraggedTypes` per window slot
    pub registered: HashMap<u8, Vec<String>>,
}

impl State {
    pub fn takes(&self, slot: u8, items: &[DragItem]) -> bool {
        self.registered.get(&slot).is_some_and(|r| accepts(r, items))
    }
}

/// Shell side.
impl super::Shell {
    /// A drag message from the app in `slot`.
    pub(super) fn dnd_msg(&mut self, slot: u8, msg: ToShell) {
        match msg {
            ToShell::DragRegister { types } => {
                if types.is_empty() {
                    self.dnd.registered.remove(&slot);
                } else {
                    self.dnd.registered.insert(slot, types);
                }
            }
            ToShell::DragBegin { session, allowed, items, preview } => {
                let source_app = self.ext.get(slot as usize).and_then(Option::as_ref).map(|h| h.id.clone());
                let ok = self.dnd.session.is_none() && self.ext_fg == Some(slot) && session & INBOUND != 0 && allowed != 0 && valid_items(&items);
                match (ok, source_app) {
                    (true, Some(app)) => {
                        let preview = if preview.valid() { preview } else { DragPreview::default() };
                        self.dnd.session = Some(Session::new(session, slot, &app, allowed, items, preview));
                        self.dirty = true;
                    }
                    // refused: the source hears that nothing took it
                    _ => self.ext_send(slot as usize, &ToApp::DragEnded { session, op: 0 }),
                }
            }
            ToShell::DragReply { session, op } => {
                if let Some(s) = self.dnd.session.as_mut().filter(|s| s.id == session) {
                    s.reply(slot, op);
                }
            }
            ToShell::DragPerformed { session, accepted } => {
                let mine = self.dnd.session.as_ref().is_some_and(|s| s.id == session && s.performing == Some(slot));
                if mine {
                    let outs = self.dnd.session.as_mut().map(|s| s.performed(accepted)).unwrap_or_default();
                    self.dnd_apply(outs);
                }
            }
            ToShell::DragCancel { session } => {
                if self.dnd.session.as_ref().is_some_and(|s| s.id == session && s.source == slot) {
                    let outs = self.dnd.session.as_mut().map(|s| s.cancel()).unwrap_or_default();
                    self.dnd_apply(outs);
                }
            }
            _ => {}
        }
    }

    /// An app exited: its registration goes, and a session it takes part in is abandoned.
    pub(super) fn dnd_app_gone(&mut self, slot: u8) {
        self.dnd.registered.remove(&slot);
        let involved = self.dnd.session.as_ref().is_some_and(|s| s.source == slot || s.over() == Some(slot) || s.performing == Some(slot));
        if involved {
            let source_gone = self.dnd.session.as_ref().is_some_and(|s| s.source == slot);
            let outs = self.dnd.session.as_mut().map(|s| s.cancel()).unwrap_or_default();
            if source_gone {
                // nobody to tell; just drop the session and the grants it made
                if let Some(s) = self.dnd.session.take() {
                    self.dnd_release_sources(&s);
                }
                self.dirty = true;
            } else {
                self.dnd_apply(outs);
            }
        }
    }

    fn dnd_target(&self, x: f32, y: f32) -> Option<Target> {
        let d = self.desk.as_ref()?;
        let m = self.desk_metrics();
        let (i, hit) = d.pick(&m, x, y)?;
        let w = d.wins[i];
        let super::AppId::External(slot) = w.app else { return None };
        if hit != super::desk::Hit::Content {
            return None;
        }
        let b = w.body(&m);
        Some(Target { slot, x: x - b.x as f32, y: y - b.y as f32 })
    }

    /// Pointer events while a session is live. None when there is no session (the event goes on).
    pub(super) fn dnd_event(&mut self, ev: bifrost_ui::Event, t: f64) -> Option<Vec<super::Effect>> {
        use bifrost_ui::Event;
        self.dnd.session.as_ref()?;
        if self.dnd.session.as_ref().is_some_and(|s| s.performing.is_some()) {
            return Some(Vec::new());
        }
        let (x, y, up) = match ev {
            Event::Move(x, y) => (x, y, false),
            Event::Up(x, y) => (x, y, true),
            Event::Down(..) => return Some(Vec::new()),
            _ => return None,
        };
        let target = self.dnd_target(x, y);
        let mut sess = self.dnd.session.take()?;
        let registered = &self.dnd.registered;
        let items = sess.items.clone();
        let takes = |slot: u8| registered.get(&slot).is_some_and(|r| accepts(r, &items));
        let mut outs = sess.moved(target, &takes);
        sess.pos = Some((x, y));
        if up {
            if sess.over().is_none() {
                if let Some(zone) = self.dnd_zone(x, y) {
                    // the Dock or the desktop takes the files; the session ends here
                    let op = self.dnd_zone_drop(&sess, zone, t);
                    self.dnd_release_sources(&sess);
                    self.ext_send(sess.source as usize, &ToApp::DragEnded { session: sess.id, op });
                    self.dirty = true;
                    return Some(Vec::new());
                }
            }
            outs.extend(sess.release());
        }
        self.dnd.session = Some(sess);
        self.dnd_apply(outs);
        self.dirty = true;
        Some(Vec::new())
    }

    /// The Dock icon or the desktop under the pointer, for a drop no app window took. None over windows, the menu bar or
    /// nothing at all.
    fn dnd_zone(&self, x: f32, y: f32) -> Option<Zone> {
        if !self.desk_active() {
            return None;
        }
        let m = self.desk_metrics();
        if self.dock_is_visible() {
            let (list, _, rects) = self.desk_dock(&m);
            for (k, r) in rects.iter().enumerate() {
                if x >= r.x as f32 && x < (r.x + r.w) as f32 && y >= r.y as f32 && y < (r.y + r.h) as f32 {
                    return list.get(k).and_then(|&e| apps().get(e)).map(|a| Zone::Dock(a.icon.to_string()));
                }
            }
        }
        let over_window = self.desk.as_ref().and_then(|d| d.pick(&m, x, y)).is_some();
        (!over_window && y as i32 > m.menubar).then_some(Zone::Desktop)
    }

    /// Hands the session's files to `zone`; returns the operation the source hears (0 when nothing took them).
    fn dnd_zone_drop(&mut self, sess: &Session, zone: Zone, t: f64) -> u8 {
        let grants: Vec<GrantId> = sess.items.iter().filter_map(|i| if let Payload::File { grant, .. } = &i.payload { Some(*grant) } else { None }).collect();
        if grants.is_empty() || sess.allowed & dragop::COPY == 0 {
            return 0;
        }
        match zone {
            Zone::Dock(key) => {
                let Some(app) = apps().iter().find(|a| a.icon == key).map(|a| a.id) else { return 0 };
                let app = match app {
                    AppId::Files => super::apphost::slot_of("is.olibuijr.files.desktop").map_or(app, |s| AppId::External(s as u8)),
                    other => other,
                };
                let AppId::External(to) = app else { return 0 };
                self.desk_open_or_start(app, t);
                let request = self.app_share.next_inbound();
                self.inbound_files(&sess.source_app, &key, to, request, &grants);
                dragop::COPY
            }
            Zone::Desktop => {
                if self.dnd_copy_to_desktop(&sess.source_app, &grants) {
                    dragop::COPY
                } else {
                    0
                }
            }
        }
    }

    /// Copies the files of `grants` (made for `source_app`) into the account's Desktop folder, made if missing. A name
    /// already there gets a number. Folders are skipped. True when at least one file was copied.
    fn dnd_copy_to_desktop(&mut self, source_app: &str, grants: &[GrantId]) -> bool {
        let Some(b) = self.doc_broker() else { return false };
        let dir = Path::new("Desktop");
        let _ = b.opener.make_dir(Path::new(""), "Desktop");
        let mut taken: Vec<String> = b.opener.list(dir).unwrap_or_default().into_iter().map(|(n, _)| n).collect();
        let mut copied = false;
        for g in grants {
            let Ok(grant) = b.grants.get(source_app, g) else { continue };
            let Some(file_name) = grant.path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            let Ok(mut src) = b.opener.open_any(&grant.path, DocMode::Read) else { continue };
            if !src.metadata().is_ok_and(|m| m.is_file()) {
                continue;
            }
            let name = free_name(&taken, &file_name);
            let Ok((pending, mut out)) = b.opener.begin_save(dir, &name) else { continue };
            if std::io::copy(&mut src, &mut out).is_ok() {
                if pending.commit(&b.opener).is_ok() {
                    taken.push(name);
                    copied = true;
                }
            } else {
                pending.abort(&b.opener);
            }
        }
        copied
    }

    /// The grants a drag made for its file items go when the drag ends. The destinations' own grants stay.
    fn dnd_release_sources(&mut self, sess: &Session) {
        let source = sess.source_app.clone();
        let Some(b) = self.doc_broker() else { return };
        for i in &sess.items {
            if let Payload::File { grant, .. } = &i.payload {
                let _ = b.grants.release(&source, grant);
            }
        }
    }

    /// Sends what the state machine asked for; a finished session is dropped.
    fn dnd_apply(&mut self, outs: Vec<Out>) {
        for out in outs {
            match out {
                Out::Dest { slot, phase, x, y } => {
                    let Some(s) = self.dnd.session.as_ref() else { continue };
                    let (id, allowed, source_app, source) = (s.id, s.allowed, s.source_app.clone(), s.source);
                    let mut items = Vec::new();
                    if phase == DragPhase::Entered {
                        items = s.items.clone();
                    }
                    if phase == DragPhase::Perform {
                        items = self.dnd_hand_over(id, &source_app, slot);
                        if items.is_empty() {
                            // nothing could be handed over: the drop fails
                            let outs = self.dnd.session.as_mut().map(|s| s.performed(false)).unwrap_or_default();
                            self.ext_send(slot as usize, &ToApp::DragDest { session: id, phase: DragPhase::Exited, x, y, allowed, items: Vec::new() });
                            self.dnd_apply(outs);
                            continue;
                        }
                    }
                    let _ = source;
                    self.ext_send(slot as usize, &ToApp::DragDest { session: id, phase, x, y, allowed, items });
                }
                Out::Ended { op } => {
                    if let Some(s) = self.dnd.session.take() {
                        self.dnd_release_sources(&s);
                        self.ext_send(s.source as usize, &ToApp::DragEnded { session: s.id, op });
                    }
                    self.dirty = true;
                }
            }
        }
    }

    /// The items as the destination receives them in `Perform`: text and URLs unchanged, each file replaced by a grant made
    /// for the destination (the files arrive just before, as `DocGranted` with request = session).
    fn dnd_hand_over(&mut self, id: u32, source_app: &str, dest_slot: u8) -> Vec<DragItem> {
        let items = self.dnd.session.as_ref().map(|s| s.items.clone()).unwrap_or_default();
        let registered = super::apphost::installed();
        let Some(dest_app) = registered.get(dest_slot as usize).map(|a| a.manifest.id.clone()) else { return Vec::new() };
        // a window of the source app already holds the grants: it gets them as they are
        if dest_app == source_app {
            return items;
        }
        // Without the desktop broker there is no grant mapping, so `map_files` drops every file item.
        #[cfg(feature = "desktop-wayland")]
        let mapping = {
            let grants: Vec<_> = items.iter().filter_map(|i| if let Payload::File { grant, .. } = &i.payload { Some(*grant) } else { None }).collect();
            if grants.is_empty() { Vec::new() } else { self.inbound_files(source_app, &dest_app, dest_slot, id, &grants) }
        };
        #[cfg(not(feature = "desktop-wayland"))]
        let mapping: Vec<(bifrost_proto::GrantId, bifrost_proto::GrantId)> = {
            let _ = (id, source_app, &dest_app);
            Vec::new()
        };
        map_files(items, &mapping)
    }

    /// Draws the drag image at the pointer while a session is live.
    pub(super) fn dnd_draw(&self, s: &mut bifrost_render::gfx::Surface) {
        let Some(sess) = &self.dnd.session else { return };
        let (Some((x, y)), Some(img)) = (sess.pos, preview_image(&sess.preview)) else { return };
        let k = self.layout.s.max(1.0);
        s.blit(&img, (x - img.w as f32 * k / 2.0) as i32, (y - img.h as f32 * k / 2.0) as i32, 255, 200);
    }
}

/// Replaces each file's grant by the one made for the destination; a file that could not be handed over is dropped (and an
/// item with it).
pub fn map_files(items: Vec<DragItem>, mapping: &[(bifrost_proto::GrantId, bifrost_proto::GrantId)]) -> Vec<DragItem> {
    items
        .into_iter()
        .filter_map(|mut i| {
            if let Payload::File { grant, .. } = &mut i.payload {
                *grant = mapping.iter().find(|(old, _)| old == grant)?.1;
            }
            Some(i)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_proto::dragop;

    fn png() -> DragItem {
        DragItem::new(Payload::File { grant: [1; 16], name: "a.png".into() })
    }
    fn txt() -> DragItem {
        DragItem::new(Payload::Text("hello".into()))
    }
    fn session(allowed: u8, items: Vec<DragItem>) -> Session {
        Session::new(INBOUND | 1, 0, "src", allowed, items, DragPreview::default())
    }
    fn at(slot: u8, x: f32, y: f32) -> Option<Target> {
        Some(Target { slot, x, y })
    }
    fn dest(slot: u8, phase: DragPhase, x: f32, y: f32) -> Out {
        Out::Dest { slot, phase, x, y }
    }
    fn all(_: u8) -> bool {
        true
    }
    const ALL: &dyn Fn(u8) -> bool = &all;

    #[test]
    fn registration_matches_by_conformance() {
        let img = vec!["public.image".to_string()];
        assert!(accepts(&img, &[png()]));
        assert!(!accepts(&img, &[txt()]));
        assert!(accepts(&["public.text".to_string()], &[txt()]));
        assert!(accepts(&["public.data".to_string()], &[png()]), "an ancestor type takes everything below it");
        // one matching item is enough
        assert!(accepts(&img, &[txt(), png()]));
        assert!(!accepts(&[], &[png()]));
        // an item offered as several types matches on any
        let multi = DragItem { types: vec!["public.utf8-plain-text".into(), "public.url".into()], payload: Payload::Url("https://x".into()) };
        assert!(accepts(&["public.url".to_string()], &[multi]));
    }

    #[test]
    fn negotiation_keeps_one_allowed_bit() {
        assert_eq!(negotiate(dragop::ALL, dragop::MOVE), dragop::MOVE);
        assert_eq!(negotiate(dragop::COPY, dragop::MOVE), 0, "not allowed by the source");
        assert_eq!(negotiate(dragop::ALL, dragop::COPY | dragop::MOVE), 0, "two bits is not an operation");
        assert_eq!(negotiate(dragop::ALL, 0), 0);
    }

    #[test]
    fn entered_updated_exited_follow_the_pointer() {
        let mut s = session(dragop::COPY, vec![png()]);
        assert!(s.moved(None, ALL).is_empty());
        assert_eq!(s.moved(at(2, 5.0, 6.0), ALL), vec![dest(2, DragPhase::Entered, 5.0, 6.0)]);
        assert_eq!(s.moved(at(2, 7.0, 8.0), ALL), vec![dest(2, DragPhase::Updated, 7.0, 8.0)]);
        // another window: exit the old one at its last point, enter the new
        assert_eq!(s.moved(at(3, 1.0, 1.0), ALL), vec![dest(2, DragPhase::Exited, 7.0, 8.0), dest(3, DragPhase::Entered, 1.0, 1.0)]);
        assert_eq!(s.over(), Some(3));
        assert_eq!(s.moved(None, ALL), vec![dest(3, DragPhase::Exited, 1.0, 1.0)]);
        assert_eq!(s.over(), None);
    }

    #[test]
    fn windows_that_did_not_register_are_skipped() {
        let mut s = session(dragop::COPY, vec![png()]);
        let only_two: &dyn Fn(u8) -> bool = &|slot| slot == 2;
        assert!(s.moved(at(1, 0.0, 0.0), only_two).is_empty());
        assert_eq!(s.moved(at(2, 0.0, 0.0), only_two).len(), 1);
        // moving onto a window that does not take it counts as leaving
        assert_eq!(s.moved(at(1, 0.0, 0.0), only_two), vec![dest(2, DragPhase::Exited, 0.0, 0.0)]);
    }

    #[test]
    fn a_reply_selects_the_operation_and_a_drop_performs_it() {
        let mut s = session(dragop::COPY | dragop::MOVE, vec![png()]);
        s.moved(at(2, 3.0, 4.0), ALL);
        assert_eq!(s.operation(), 0);
        s.reply(9, dragop::COPY);
        assert_eq!(s.operation(), 0, "a reply from another window is stale");
        s.reply(2, dragop::LINK);
        assert_eq!(s.operation(), 0, "the source did not allow link");
        s.reply(2, dragop::MOVE);
        assert_eq!(s.operation(), dragop::MOVE);
        assert_eq!(s.release(), vec![dest(2, DragPhase::Perform, 3.0, 4.0)]);
        assert_eq!(s.performing, Some(2));
        assert_eq!(s.performed(true), vec![Out::Ended { op: dragop::MOVE }]);
    }

    #[test]
    fn a_refused_perform_ends_with_no_operation() {
        let mut s = session(dragop::COPY, vec![png()]);
        s.moved(at(2, 0.0, 0.0), ALL);
        s.reply(2, dragop::COPY);
        s.release();
        assert_eq!(s.performed(false), vec![Out::Ended { op: 0 }]);
    }

    #[test]
    fn releasing_where_nothing_agreed_exits_and_ends() {
        let mut s = session(dragop::COPY, vec![png()]);
        s.moved(at(2, 1.0, 2.0), ALL);
        assert_eq!(s.release(), vec![dest(2, DragPhase::Exited, 1.0, 2.0), Out::Ended { op: 0 }]);
        let mut s = session(dragop::COPY, vec![png()]);
        assert_eq!(s.release(), vec![Out::Ended { op: 0 }]);
    }

    #[test]
    fn cancel_leaves_the_window_and_ends() {
        let mut s = session(dragop::COPY, vec![png()]);
        s.moved(at(4, 9.0, 9.0), ALL);
        assert_eq!(s.cancel(), vec![dest(4, DragPhase::Exited, 9.0, 9.0), Out::Ended { op: 0 }]);
    }

    #[test]
    fn a_new_window_starts_without_an_operation() {
        let mut s = session(dragop::COPY, vec![png()]);
        s.moved(at(2, 0.0, 0.0), ALL);
        s.reply(2, dragop::COPY);
        s.moved(at(3, 0.0, 0.0), ALL);
        assert_eq!(s.operation(), 0, "the old window's answer does not carry over");
    }

    #[test]
    fn item_validation() {
        assert!(valid_items(&[png(), txt()]));
        assert!(!valid_items(&[]));
        assert!(!valid_items(&[DragItem { types: vec![], payload: Payload::Text("x".into()) }]));
        assert!(!valid_items(&[DragItem::new(Payload::Text(String::new()))]));
    }

    #[test]
    fn files_are_remapped_to_the_destination_grants() {
        let url = DragItem::new(Payload::Url("https://x".into()));
        let one = DragItem::new(Payload::File { grant: [1; 16], name: "a.png".into() });
        let two = DragItem::new(Payload::File { grant: [2; 16], name: "b.png".into() });
        let out = map_files(vec![url.clone(), one, two], &[([1; 16], [9; 16])]);
        assert_eq!(out.len(), 2, "the file with no new grant is dropped");
        assert_eq!(out[0], url);
        assert!(matches!(&out[1].payload, Payload::File { grant, name } if *grant == [9; 16] && name == "a.png"));
    }

    #[test]
    fn the_preview_becomes_a_premultiplied_image() {
        let p = DragPreview { w: 1, h: 2, rgba: vec![255, 0, 0, 255, 255, 255, 255, 0] };
        let img = preview_image(&p).unwrap();
        assert_eq!((img.w, img.h), (1, 2));
        assert_eq!(img.px[0], bifrost_render::gfx::rgba(255, 0, 0, 255));
        assert_eq!(img.px[1] >> 24, 0);
        assert!(preview_image(&DragPreview::default()).is_none());
        assert!(preview_image(&DragPreview { w: 4, h: 4, rgba: vec![0; 3] }).is_none());
    }

    #[test]
    fn copies_take_a_free_name_before_the_extension() {
        let taken = vec!["a.png".to_string(), "a 2.png".to_string(), "notes".to_string()];
        assert_eq!(free_name(&taken, "b.png"), "b.png");
        assert_eq!(free_name(&taken, "a.png"), "a 3.png");
        assert_eq!(free_name(&taken, "notes"), "notes 2");
        assert_eq!(free_name(&[".hidden".to_string()], ".hidden"), ".hidden 2", "a leading dot is not an extension");
    }

    #[test]
    fn registration_state_decides_who_takes_the_items() {
        let mut st = State::default();
        st.registered.insert(1, vec!["public.image".into()]);
        assert!(st.takes(1, &[png()]));
        assert!(!st.takes(1, &[txt()]));
        assert!(!st.takes(2, &[png()]));
    }
}
