//! The share sheet (`NativeSharingServicePicker`). An app sends `ToShell::Share` with items (text, a URL, a file it holds a
//! document grant for) and the rect it was invoked from. The shell lists the installed apps that declare they open every
//! item (`handles` UTTypes and `schemes` in the manifest, matched by conformance with `uttype`) plus Copy, as a menu
//! under the rect. The chosen app receives the items: a URL as `ToApp::Open`, text as an `Open` with a
//! `data:text/plain` URL (`text_from_url` reads it back), a file as a fresh read-only document grant
//! (`DocGranted`, request `INBOUND | n`) the shell makes from the sender's grant. The sender hears `ToApp::ShareDone`.

use bifrost_proto::{ContextItem, Payload};

/// The target id of Copy in `ShareDone`.
pub const COPY: &str = "copy";
/// Context-menu ids of the share menu start here (`ToApp::ContextAction` is never sent for them).
pub const MENU_BASE: u32 = 0x5348_0000;
/// Document requests with this bit set were not asked for by the receiving app (shared or dropped items).
pub const INBOUND: u32 = 0x8000_0000;

/// One app that could receive the items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// the app id, or `COPY`
    pub id: String,
    pub name: String,
}

/// What the picker needs to know about an installed app.
pub struct Candidate<'a> {
    pub id: &'a str,
    pub name: &'a str,
    /// manifest `handles`: `type` or `type:rank`
    pub handles: &'a [String],
    pub schemes: &'a [String],
}

/// The types an item can be opened as.
fn open_types(p: &Payload) -> Vec<String> {
    match p {
        Payload::Text(_) => vec!["public.utf8-plain-text".into()],
        Payload::Url(u) => match u.strip_prefix("file://") {
            Some(path) => bifrost_ui::uttype::type_for_extension(path).map(|t| vec![t.to_string()]).unwrap_or_default(),
            None => Vec::new(),
        },
        Payload::File { .. } => vec![p.uti()],
    }
}

fn scheme(url: &str) -> Option<&str> {
    let (s, _) = url.split_once(':')?;
    (!s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))).then_some(s)
}

/// True when the app opens `item`: one of its handled types is an ancestor of the item's type, or (for a web URL) it
/// handles the URL scheme.
pub fn handles(c: &Candidate, item: &Payload) -> bool {
    if let Payload::Url(u) = item {
        if scheme(u).is_some_and(|s| c.schemes.iter().any(|x| x.eq_ignore_ascii_case(s))) {
            return true;
        }
    }
    let types = open_types(item);
    c.handles.iter().map(|h| h.split(':').next().unwrap_or("")).any(|h| types.iter().any(|t| bifrost_ui::uttype::conforms(t, h)))
}

/// The share menu's targets: apps (other than the sender) that handle every item, by name, then Copy.
pub fn targets(cands: &[Candidate], sender: &str, items: &[Payload]) -> Vec<Target> {
    let mut out: Vec<Target> = if items.is_empty() {
        Vec::new()
    } else {
        cands.iter().filter(|c| c.id != sender && items.iter().all(|i| handles(c, i))).map(|c| Target { id: c.id.into(), name: c.name.into() }).collect()
    };
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    out.dedup_by(|a, b| a.id == b.id);
    out.push(Target { id: COPY.into(), name: "Copy".into() });
    out
}

/// The menu for `targets`, ids `MENU_BASE + index`.
pub fn menu(targets: &[Target]) -> Vec<ContextItem> {
    targets.iter().enumerate().map(|(i, t)| ContextItem { id: MENU_BASE + i as u32, label: t.name.clone(), enabled: true }).collect()
}

/// The text Copy puts on the pasteboard: text and URLs, one per line; a file is copied as its name.
pub fn copy_text(items: &[Payload]) -> String {
    items
        .iter()
        .map(|i| match i {
            Payload::Text(s) | Payload::Url(s) => s.clone(),
            Payload::File { name, .. } => name.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A `data:` URL carrying `text`, which is how shared and dropped text reaches an app as `Input::Open`.
pub fn text_url(text: &str) -> String {
    let mut s = String::from("data:text/plain;charset=utf-8,");
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// The text of a `text_url`; `None` for any other URL.
pub fn text_from_url(url: &str) -> Option<String> {
    let body = url.strip_prefix("data:text/plain;charset=utf-8,")?;
    let bytes = body.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A share waiting for the user's choice.
pub struct Pending {
    pub slot: u8,
    pub request: u32,
    pub items: Vec<Payload>,
    pub targets: Vec<Target>,
}

#[derive(Default)]
pub struct State {
    pub pending: Option<Pending>,
    /// last `INBOUND` request handed out
    pub seq: u32,
}

impl State {
    /// A fresh request id for an inbound document (`INBOUND | n`).
    pub fn next_inbound(&mut self) -> u32 {
        self.seq = self.seq.wrapping_add(1) & !INBOUND;
        INBOUND | self.seq
    }
}

/// Shell side.
impl super::Shell {
    /// A `ToShell::Share` from the app in `slot`: shows the target menu under `anchor`.
    pub(super) fn share_begin(&mut self, slot: u8, request: u32, items: Vec<Payload>, anchor: bifrost_proto::UiRect) {
        // a share still open is cancelled first
        self.share_cancel();
        let registered = super::apphost::installed();
        let Some(sender) = registered.get(slot as usize).map(|a| a.manifest.id.clone()) else { return };
        let cands: Vec<Candidate> = registered
            .iter()
            .filter(|a| !a.manifest.is_system())
            .map(|a| Candidate { id: &a.manifest.id, name: &a.manifest.name, handles: &a.manifest.handles, schemes: &a.manifest.schemes })
            .collect();
        let list = targets(&cands, &sender, &items);
        let body = self.desk.as_ref().and_then(|d| d.index_of(super::AppId::External(slot)).map(|i| d.wins[i].body(&self.desk_metrics())));
        let Some(body) = body else { return self.share_finish(slot, request, "") };
        let (x, y) = (anchor.x + body.x as f32, anchor.y + anchor.h + body.y as f32);
        self.desktop_context = Some(super::desktop_bar::ContextMenu { slot, x, y, items: menu(&list) });
        self.desktop_menu = Some(super::desktop_bar::Menu::Context);
        self.app_share.pending = Some(Pending { slot, request, items, targets: list });
        self.dirty = true;
    }

    fn share_finish(&mut self, slot: u8, request: u32, target: &str) {
        self.ext_send(slot as usize, &bifrost_proto::ToApp::ShareDone { request, target: target.into() });
    }

    /// The share menu went away without a choice.
    fn share_cancel(&mut self) {
        if let Some(p) = self.app_share.pending.take() {
            self.share_finish(p.slot, p.request, "");
        }
    }

    /// Called every tick: a closed menu with no choice is a dismissal.
    pub(super) fn share_tick(&mut self) {
        if self.app_share.pending.is_some() && self.desktop_context.is_none() {
            self.share_cancel();
        }
    }

    /// The user chose menu item `id` (a `MENU_BASE` id). False when no share is open.
    pub(super) fn share_choose(&mut self, id: u32, t: f64) -> bool {
        let Some(p) = self.app_share.pending.take() else { return false };
        let Some(target) = id.checked_sub(MENU_BASE).and_then(|i| p.targets.get(i as usize)).cloned() else {
            self.share_finish(p.slot, p.request, "");
            return true;
        };
        let delivered = if target.id == COPY { self.share_copy(&p) } else { self.share_deliver(&p, &target.id, t) };
        self.share_finish(p.slot, p.request, if delivered { &target.id } else { "" });
        true
    }

    fn share_copy(&mut self, p: &Pending) -> bool {
        super::clipboard::set_text(&copy_text(&p.items));
        true
    }

    fn share_deliver(&mut self, p: &Pending, app: &str, t: f64) -> bool {
        let Some(to) = super::apphost::slot_of(app) else { return false };
        let sender = self.ext.get(p.slot as usize).and_then(Option::as_ref).map(|h| h.id.clone());
        if self.desk_active() {
            self.desk_open(super::AppId::External(to as u8), t);
        } else {
            self.ext_open(to);
        }
        let mut files = Vec::new();
        for item in &p.items {
            match item {
                Payload::Url(u) => self.ext_send(to, &bifrost_proto::ToApp::Open { url: u.clone() }),
                Payload::Text(s) => self.ext_send(to, &bifrost_proto::ToApp::Open { url: text_url(s) }),
                Payload::File { grant, .. } => files.push(*grant),
            }
        }
        if let (Some(sender), false) = (sender, files.is_empty()) {
            let request = self.app_share.next_inbound();
            #[cfg(feature = "desktop-wayland")]
            self.inbound_files(&sender, app, to as u8, request, &files);
            #[cfg(not(feature = "desktop-wayland"))]
            let _ = (sender, request);
        }
        self.dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn cands<'a>(table: &'a [(&'a str, &'a str, Vec<String>, Vec<String>)]) -> Vec<Candidate<'a>> {
        table.iter().map(|(id, name, handles, schemes)| Candidate { id, name, handles, schemes }).collect()
    }

    fn table() -> Vec<(&'static str, &'static str, Vec<String>, Vec<String>)> {
        vec![
            ("notes", "Notes", s(&["public.plain-text:default", "net.daringfireball.markdown"]), vec![]),
            ("photos", "Photos", s(&["public.png:owner", "public.jpeg"]), vec![]),
            ("browser", "Browser", s(&["public.html:owner"]), s(&["http", "https"])),
            ("files", "Files", s(&["public.folder:owner", "public.data:alternate"]), vec![]),
        ]
    }

    fn names(t: &[Target]) -> Vec<&str> {
        t.iter().map(|t| t.name.as_str()).collect()
    }

    #[test]
    fn text_goes_to_text_apps_and_copy_is_last() {
        let table = table();
        let t = targets(&cands(&table), "other", &[Payload::Text("hi".into())]);
        assert_eq!(names(&t), ["Files", "Notes", "Copy"]);
        assert_eq!(t.last().unwrap().id, COPY);
    }

    #[test]
    fn files_match_by_conformance() {
        let table = table();
        let png = Payload::File { grant: [1; 16], name: "a.png".into() };
        assert_eq!(names(&targets(&cands(&table), "other", &[png.clone()])), ["Files", "Photos", "Copy"]);
        // every item must be accepted
        let txt = Payload::File { grant: [2; 16], name: "a.txt".into() };
        assert_eq!(names(&targets(&cands(&table), "other", &[png, txt])), ["Files", "Copy"]);
    }

    #[test]
    fn web_urls_match_by_scheme_and_the_sender_is_left_out() {
        let table = table();
        let url = Payload::Url("https://example.com/x".into());
        assert_eq!(names(&targets(&cands(&table), "other", &[url.clone()])), ["Browser", "Copy"]);
        assert_eq!(names(&targets(&cands(&table), "browser", &[url])), ["Copy"]);
        // a file URL is matched by its extension, not its scheme
        let f = Payload::Url("file:///home/a/b.png".into());
        assert_eq!(names(&targets(&cands(&table), "other", &[f])), ["Files", "Photos", "Copy"]);
    }

    #[test]
    fn nothing_to_share_offers_nothing_but_copy() {
        let table = table();
        assert_eq!(names(&targets(&cands(&table), "other", &[])), ["Copy"]);
    }

    #[test]
    fn menu_ids_index_the_targets() {
        let t = vec![Target { id: "a".into(), name: "A".into() }, Target { id: COPY.into(), name: "Copy".into() }];
        let m = menu(&t);
        assert_eq!((m[0].id, m[1].id), (MENU_BASE, MENU_BASE + 1));
        assert!(m.iter().all(|i| i.enabled));
    }

    #[test]
    fn copy_text_joins_items() {
        let items = [Payload::Text("a".into()), Payload::Url("https://x".into()), Payload::File { grant: [0; 16], name: "f.png".into() }];
        assert_eq!(copy_text(&items), "a\nhttps://x\nf.png");
    }

    #[test]
    fn text_urls_round_trip() {
        for t in ["plain", "héllo wörld\n100% done & more", "", "日本語"] {
            assert_eq!(text_from_url(&text_url(t)).as_deref(), Some(t));
        }
        assert_eq!(text_from_url("https://example.com"), None);
        assert_eq!(text_from_url("data:text/plain;charset=utf-8,%ZZ"), None);
        assert_eq!(text_from_url("data:text/plain;charset=utf-8,%4"), None);
    }

    #[test]
    fn inbound_requests_have_the_marker_bit_and_advance() {
        let mut st = State::default();
        let (a, b) = (st.next_inbound(), st.next_inbound());
        assert!(a & INBOUND != 0 && b & INBOUND != 0 && a != b);
    }
}
