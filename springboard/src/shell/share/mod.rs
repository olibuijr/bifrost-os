//! The share sheet: a popover with a preview of what is shared, a row of
//! nearby devices and the actions Copy, Save to Files and Send Nearby.
//! `Sheet` is the model (layout, hit testing, results); `Sheet::draw` paints
//! it. Apps open it with a `Item`; the shell acts on the returned `Outcome`.

use crate::gfx::Surface;
use crate::net::share::Peer;
use crate::shell::ui::{Ui, Weight};
use std::path::PathBuf;

pub mod glue;
pub mod prompt;

pub const WIDTH_PT: f32 = 340.0;
const RADIUS_PT: f32 = 6.0;
const PAD_PT: f32 = 16.0;
const PREVIEW_PT: f32 = 76.0;
const PEER_PT: f32 = 76.0;
const ROW_PT: f32 = 48.0;

/// What is being shared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    File(PathBuf),
    /// shared as a .txt (Notes)
    Text { title: String, body: String },
    /// shared as a .vcf (Contacts)
    Card { name: String, vcard: String },
}

impl Item {
    /// A contact shared as a vCard.
    pub fn from_contact(c: &crate::shell::contactstore::Contact) -> Item {
        Item::Card { name: c.display_name().to_string(), vcard: vcard(&c.name, &c.phone, &c.email, &c.notes) }
    }

    pub fn title(&self) -> String {
        match self {
            Item::File(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            Item::Text { title, .. } => title.clone(),
            Item::Card { name, .. } => name.clone(),
        }
    }

    pub fn subtitle(&self) -> String {
        match self {
            Item::File(p) => p.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_else(|| "File".into()),
            Item::Text { body, .. } => body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(60).collect(),
            Item::Card { .. } => "Contact card".into(),
        }
    }

    /// The name the receiver sees.
    pub fn file_name(&self) -> String {
        match self {
            Item::File(_) => self.title(),
            Item::Text { title, .. } => format!("{}.txt", base(title, "Note")),
            Item::Card { name, .. } => format!("{}.vcf", base(name, "Contact")),
        }
    }

    /// The bytes to send or save; files are streamed from disk instead.
    pub fn bytes(&self) -> Option<Vec<u8>> {
        match self {
            Item::File(_) => None,
            Item::Text { title, body } => Some(format!("{title}\n\n{body}").into_bytes()),
            Item::Card { vcard, .. } => Some(vcard.clone().into_bytes()),
        }
    }

    /// Text for the clipboard (a file copies its path).
    pub fn clip_text(&self) -> String {
        match self {
            Item::File(p) => p.to_string_lossy().into_owned(),
            Item::Text { title, body } => format!("{title}\n\n{body}"),
            Item::Card { vcard, .. } => vcard.clone(),
        }
    }
}

fn base(s: &str, fallback: &str) -> String {
    let t: String = s.chars().filter(|c| !c.is_control() && !"/\\".contains(*c)).take(40).collect();
    let t = t.trim().trim_start_matches('.');
    if t.is_empty() { fallback.into() } else { t.into() }
}

/// A vCard 3.0 for a contact; fields are escaped per RFC 6350 and empty ones left out.
pub fn vcard(name: &str, phone: &str, email: &str, notes: &str) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('\n', "\\n").replace(',', "\\,").replace(';', "\\;");
    let mut v = format!("BEGIN:VCARD\r\nVERSION:3.0\r\nFN:{}\r\nN:{};;;;\r\n", esc(name), esc(name));
    for (key, val) in [("TEL", phone), ("EMAIL", email), ("NOTE", notes)] {
        if !val.trim().is_empty() {
            v.push_str(&format!("{key}:{}\r\n", esc(val.trim())));
        }
    }
    v.push_str("END:VCARD\r\n");
    v
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Copy,
    SaveToFiles,
}

impl Action {
    pub const ALL: [Action; 2] = [Action::Copy, Action::SaveToFiles];
    pub fn label(self) -> &'static str {
        match self {
            Action::Copy => "Copy",
            Action::SaveToFiles => "Save to Files",
        }
    }
}

/// What a tap asks the shell to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    None,
    Close,
    Copy(Item),
    SaveToFiles(Item),
    /// send to this peer (the shell runs `net::share::send` off the UI thread)
    SendTo(Item, Peer),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Peer(usize),
    Action(Action),
    Outside,
    Inside,
}

pub struct Sheet {
    pub item: Item,
    pub peers: Vec<Peer>,
    /// where the share button was, in px; the popover opens below it
    pub anchor: (f32, f32),
}

impl Sheet {
    pub fn new(item: Item, anchor: (f32, f32)) -> Self {
        Sheet { item, peers: vec![], anchor }
    }

    /// Replaces the nearby list (the shell polls `Service::peers`); true when it changed.
    pub fn set_peers(&mut self, peers: Vec<Peer>) -> bool {
        let changed = self.peers != peers;
        self.peers = peers;
        changed
    }

    fn height_pt(&self) -> f32 {
        PAD_PT * 3.0 + PREVIEW_PT + PEER_PT + ROW_PT * Action::ALL.len() as f32
    }

    /// The popover's box (x, y, w, h) in px, pulled inside a `w` x `h` px screen.
    pub fn rect(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let (pw, ph, m) = (ui.pt(WIDTH_PT), ui.pt(self.height_pt()), ui.pt(PAD_PT));
        let x = (self.anchor.0 - pw / 2.0).min(w as f32 - m - pw).max(m);
        let y = (self.anchor.1 + ui.pt(8.0)).min(h as f32 - m - ph).max(m);
        (x, y, pw, ph)
    }

    fn peer_rect(&self, ui: &Ui, w: i32, h: i32, i: usize) -> (f32, f32, f32, f32) {
        let (x, y, ..) = self.rect(ui, w, h);
        let pad = ui.pt(PAD_PT);
        let cell = ui.pt(PEER_PT);
        (x + pad + cell * i as f32, y + pad * 2.0 + ui.pt(PREVIEW_PT), cell, cell)
    }

    fn action_rect(&self, ui: &Ui, w: i32, h: i32, i: usize) -> (f32, f32, f32, f32) {
        let (x, y, pw, _) = self.rect(ui, w, h);
        let top = y + ui.pt(PAD_PT * 3.0 + PREVIEW_PT + PEER_PT);
        (x, top + ui.pt(ROW_PT) * i as f32, pw, ui.pt(ROW_PT))
    }

    pub fn hit(&self, ui: &Ui, w: i32, h: i32, px: f32, py: f32) -> Hit {
        let inside = |(x, y, rw, rh): (f32, f32, f32, f32)| px >= x && px < x + rw && py >= y && py < y + rh;
        if !inside(self.rect(ui, w, h)) {
            return Hit::Outside;
        }
        if let Some(i) = (0..self.peers.len()).find(|&i| inside(self.peer_rect(ui, w, h, i))) {
            return Hit::Peer(i);
        }
        match Action::ALL.iter().enumerate().find(|(i, _)| inside(self.action_rect(ui, w, h, *i))) {
            Some((_, &a)) => Hit::Action(a),
            None => Hit::Inside,
        }
    }

    pub fn tap(&self, ui: &Ui, w: i32, h: i32, px: f32, py: f32) -> Outcome {
        match self.hit(ui, w, h, px, py) {
            Hit::Outside => Outcome::Close,
            Hit::Inside => Outcome::None,
            Hit::Peer(i) => Outcome::SendTo(self.item.clone(), self.peers[i].clone()),
            Hit::Action(Action::Copy) => Outcome::Copy(self.item.clone()),
            Hit::Action(Action::SaveToFiles) => Outcome::SaveToFiles(self.item.clone()),
        }
    }

    pub fn draw(&self, s: &mut Surface<'_>, ui: &Ui, w: i32, h: i32) {
        let (x, y, pw, ph) = self.rect(ui, w, h);
        let r = ui.pt(RADIUS_PT);
        s.fill_rrect(x, y, pw, ph, r, 0xFF2C2C2E);
        let pad = ui.pt(PAD_PT);
        // preview
        let icon = ui.pt(PREVIEW_PT - PAD_PT);
        s.fill_rrect(x + pad, y + pad, icon, icon, r, 0xFF48484A);
        let tx = x + pad * 2.0 + icon;
        let max = pw - icon - pad * 3.0;
        let t = ui.text(&ui.fit(&self.item.title(), 17.0, max), 17.0, Weight::Semibold, 0xFFFFFFFF);
        s.blit(&t, tx as i32, (y + pad) as i32, 255, 255);
        let sub = ui.text(&ui.fit(&self.item.subtitle(), 13.0, max), 13.0, Weight::Regular, 0xFF98989D);
        s.blit(&sub, tx as i32, (y + pad + ui.pt(24.0)) as i32, 255, 255);
        // nearby devices
        if self.peers.is_empty() {
            let t = ui.text("Looking for nearby devices…", 13.0, Weight::Regular, 0xFF98989D);
            s.blit(&t, (x + pad) as i32, (y + pad * 2.0 + ui.pt(PREVIEW_PT) + ui.pt(24.0)) as i32, 255, 255);
        }
        for (i, p) in self.peers.iter().enumerate() {
            let (cx, cy, cw, _) = self.peer_rect(ui, w, h, i);
            let d = cw * 0.6;
            s.fill_rrect(cx + (cw - d) / 2.0, cy, d, d, r, 0xFF0A84FF);
            let n = ui.text(&ui.fit(&p.name, 11.0, cw), 11.0, Weight::Regular, 0xFFFFFFFF);
            s.blit(&n, (cx + (cw - n.w as f32) / 2.0) as i32, (cy + d + ui.pt(4.0)) as i32, 255, 255);
        }
        // actions
        for (i, a) in Action::ALL.iter().enumerate() {
            let (rx, ry, rw, rh) = self.action_rect(ui, w, h, i);
            s.fill_rect(rx as usize + pad as usize, ry as usize, (rw - pad * 2.0) as usize, 1, 0xFF48484A);
            let t = ui.text(a.label(), 17.0, Weight::Regular, 0xFFFFFFFF);
            s.blit(&t, (rx + pad) as i32, (ry + (rh - t.h as f32) / 2.0) as i32, 255, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(n: &str) -> Peer {
        Peer { name: n.into(), addr: "10.0.0.2:5000".parse().unwrap() }
    }

    #[test]
    fn items_name_themselves() {
        let note = Item::Text { title: "Shopping/list".into(), body: "milk".into() };
        assert_eq!(note.file_name(), "Shoppinglist.txt");
        assert_eq!(note.bytes().unwrap(), b"Shopping/list\n\nmilk");
        let card = Item::Card { name: "".into(), vcard: "x".into() };
        assert_eq!(card.file_name(), "Contact.vcf");
        let file = Item::File("/a/b/pic.png".into());
        assert_eq!((file.file_name().as_str(), file.subtitle().as_str()), ("pic.png", "PNG"));
        assert!(file.bytes().is_none());
    }

    #[test]
    fn vcard_escapes_and_skips_empty_fields() {
        let v = vcard("Doe, Jane", "+354 555", "", "a;b\nc");
        assert!(v.starts_with("BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Doe\\, Jane\r\n"));
        assert!(v.contains("TEL:+354 555\r\n") && !v.contains("EMAIL"));
        assert!(v.contains("NOTE:a\\;b\\nc\r\n") && v.ends_with("END:VCARD\r\n"));
    }

    #[test]
    fn set_peers_reports_whether_the_list_changed() {
        let mut sh = Sheet::new(Item::Text { title: "T".into(), body: "b".into() }, (400.0, 100.0));
        assert!(!sh.set_peers(vec![]), "still empty");
        assert!(sh.set_peers(vec![peer("Titan")]));
        assert!(!sh.set_peers(vec![peer("Titan")]), "same list");
        assert!(sh.set_peers(vec![]));
    }

    #[test]
    fn taps_map_to_outcomes() {
        let ui = crate::shell::tests::ui();
        let mut sh = Sheet::new(Item::Text { title: "T".into(), body: "b".into() }, (400.0, 100.0));
        sh.set_peers(vec![peer("Titan")]);
        let (w, h) = (1200, 1920);
        let (px, py, cw, ch) = sh.peer_rect(&ui, w, h, 0);
        assert!(matches!(sh.tap(&ui, w, h, px + cw / 2.0, py + ch / 2.0), Outcome::SendTo(_, p) if p.name == "Titan"));
        let (ax, ay, aw, ah) = sh.action_rect(&ui, w, h, 1);
        assert!(matches!(sh.tap(&ui, w, h, ax + aw / 2.0, ay + ah / 2.0), Outcome::SaveToFiles(_)));
        assert_eq!(sh.tap(&ui, w, h, 1.0, 1900.0), Outcome::Close);
    }
}
