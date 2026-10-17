//! The window's controls: the Share My Location switch on the Me page, the join button for a device without the account
//! key, the approval sheet that lists devices waiting for approval (Approve and Deny), and the lines of positions shared
//! with the account on the People page. Every control is a rectangle in points: `draw_controls` paints them and `tap`
//! hit-tests the same rectangles, so the two cannot drift apart.

use crate::sync::unix_now;
use crate::window::{fill, LocateWindow, INSET_PT, LINE_PT, SIDEBAR_PT, TOP_PT};
use bifrost_app::entropy;
use bifrost_app::kit::ui::{CARD_BG, LABEL, LINK, MAC_BODY_PT, SEPARATOR_C};
use bifrost_app::kit::{Surface, Ui};
use bifrost_app::Ctx;
use bifrost_locate::cloud::Job;
use bifrost_locate::pairing::{self, Enrollment};
use bifrost_locate::panel::Section;
use bifrost_locate::sealing::{self, Sealed};

const SWITCH_W: f32 = 38.0;
const SWITCH_H: f32 = 22.0;
/// The Me page's first line sits at `TOP_PT + 2 * LINE_PT`; the switch is centred on it.
const SWITCH_TOP: f32 = TOP_PT + LINE_PT * 2.0 - 2.0;
/// The join button, under the Me page's two lines.
const JOIN_TOP: f32 = TOP_PT + LINE_PT * 6.0;
const JOIN_W: f32 = 300.0;
const JOIN_H: f32 = 28.0;
/// The approval sheet: its heading, then one row of name and buttons per device.
const SHEET_TOP: f32 = TOP_PT + LINE_PT * 8.0;
const ROW_PT: f32 = 60.0;
const BUTTON_H: f32 = 26.0;
const APPROVE_W: f32 = 96.0;
const DENY_W: f32 = 72.0;
const DENY_DX: f32 = 104.0;

/// A rectangle in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box2 {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Box2 {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

fn row_name_top(i: usize) -> f32 {
    SHEET_TOP + LINE_PT * 1.5 + i as f32 * ROW_PT
}

fn approve_box(i: usize) -> Box2 {
    Box2 { x: SIDEBAR_PT + INSET_PT, y: row_name_top(i) + LINE_PT, w: APPROVE_W, h: BUTTON_H }
}

fn deny_box(i: usize) -> Box2 {
    Box2 { x: SIDEBAR_PT + INSET_PT + DENY_DX, y: row_name_top(i) + LINE_PT, w: DENY_W, h: BUTTON_H }
}

/// How long ago a position was taken, as the People page says it.
pub fn age_text(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        "just now".into()
    } else if secs < 3_600 {
        format!("{} min ago", secs / 60)
    } else {
        format!("{} h ago", secs / 3_600)
    }
}

impl LocateWindow {
    /// The approval sheet shows while a device waits and this device holds the account key, which approving needs.
    pub fn sheet_shown(&self) -> bool {
        self.approvals.sheet_open() && self.keys.private().is_some()
    }

    /// The join button shows on the Me page for a device that has the cloud, no account key, and no request in flight.
    fn join_shown(&self) -> bool {
        self.link.is_some() && self.keys.private().is_none() && self.device_key.is_none() && !matches!(self.enrollment, Enrollment::Waiting { .. })
    }

    fn switch_box(&self) -> Box2 {
        Box2 { x: self.width_pt - INSET_PT - SWITCH_W, y: SWITCH_TOP, w: SWITCH_W, h: SWITCH_H }
    }

    fn join_box(&self) -> Box2 {
        Box2 { x: SIDEBAR_PT + INSET_PT, y: JOIN_TOP, w: JOIN_W, h: JOIN_H }
    }

    /// Paints the controls of the current page and the approval sheet.
    pub fn draw_controls(&mut self, ui: &Ui, s: &mut Surface) {
        if self.panel.section == Section::Me {
            self.draw_switch(ui, s);
            if self.join_shown() {
                self.draw_join(ui, s);
            }
        }
        if self.sheet_shown() {
            self.draw_sheet(ui, s);
        }
    }

    fn draw_switch(&mut self, ui: &Ui, s: &mut Surface) {
        let b = self.switch_box();
        let on = self.panel.sharing();
        fill(s, ui, b.x, b.y, b.w, b.h, if on { LINK.now() } else { SEPARATOR_C.now() });
        let knob = b.h - 4.0;
        let kx = if on { b.x + b.w - knob - 2.0 } else { b.x + 2.0 };
        fill(s, ui, kx, b.y + 2.0, knob, knob, CARD_BG.now());
    }

    fn draw_join(&mut self, ui: &Ui, s: &mut Surface) {
        let b = self.join_box();
        fill(s, ui, b.x, b.y, b.w, b.h, LINK.now());
        self.line(ui, s, "Join my account from this computer", MAC_BODY_PT, CARD_BG.now(), b.x + 10.0, b.y + 5.0, b.w - 20.0);
    }

    fn draw_sheet(&mut self, ui: &Ui, s: &mut Surface) {
        let x = SIDEBAR_PT + INSET_PT;
        let w = (self.width_pt - x - INSET_PT).max(0.0);
        let rows: Vec<String> = self.approvals.pending().iter().map(|p| p.name.clone()).collect();
        let h = LINE_PT * 1.5 + ROW_PT * rows.len() as f32 + LINE_PT;
        fill(s, ui, x - 12.0, SHEET_TOP - 12.0, w + 24.0, h + 24.0, CARD_BG.now());
        self.line(ui, s, "Approve new devices", MAC_BODY_PT, LABEL.now(), x, SHEET_TOP, w);
        for (i, name) in rows.iter().enumerate() {
            self.line(ui, s, &format!("{name} is asking to join your account."), MAC_BODY_PT, LABEL.now(), x, row_name_top(i), w);
            let a = approve_box(i);
            fill(s, ui, a.x, a.y, a.w, a.h, LINK.now());
            self.line(ui, s, "Approve", MAC_BODY_PT, CARD_BG.now(), a.x + 12.0, a.y + 5.0, a.w - 24.0);
            let d = deny_box(i);
            fill(s, ui, d.x, d.y, d.w, d.h, SEPARATOR_C.now());
            self.line(ui, s, "Deny", MAC_BODY_PT, LABEL.now(), d.x + 12.0, d.y + 5.0, d.w - 24.0);
        }
    }

    /// A tap in points. Returns whether a control took it. The sheet is checked first: it covers the page.
    pub fn tap(&mut self, ctx: &Ctx, x: f32, y: f32) -> bool {
        if self.sheet_shown() {
            let ids: Vec<String> = self.approvals.pending().iter().map(|p| p.id.clone()).collect();
            for (i, id) in ids.iter().enumerate() {
                if approve_box(i).contains(x, y) {
                    self.approve_pending(ctx, id);
                    return true;
                }
                if deny_box(i).contains(x, y) {
                    self.deny_pending(id);
                    return true;
                }
            }
        }
        if self.panel.section == Section::Me {
            if self.switch_box().contains(x, y) {
                self.toggle_sharing(ctx);
                return true;
            }
            if self.join_shown() && self.join_box().contains(x, y) {
                self.start_enrollment(ctx);
                return true;
            }
        }
        false
    }

    /// Turns Share My Location on or off. The first device to turn it on creates the account key.
    pub fn toggle_sharing(&mut self, ctx: &Ctx) {
        let on = !self.panel.sharing();
        self.panel.set_sharing(on);
        if on {
            self.ensure_key(ctx);
        }
    }

    /// The person approved a pending device: its key is sealed to it and the request leaves the sheet. Nothing is
    /// approved without this call.
    pub fn approve_pending(&mut self, ctx: &Ctx, id: &str) {
        let (Some(private), Some(pending)) = (self.keys.private().copied(), self.approvals.pending().iter().find(|p| p.id == id).cloned()) else {
            return;
        };
        let Some(link) = self.link.as_ref() else { return };
        let Ok(random) = entropy::bytes(32) else { return };
        self.approvals.confirm(id);
        self.handled_pending.push(id.to_string());
        match pairing::seal_for(&pending, &private, &random) {
            Ok(blob) => link.send(Job::Approve { id: pending.id.clone(), sealed_key: sealing::pack(&blob) }),
            Err(e) => ctx.log(&format!("locate: could not seal the key for {}: {e:?}", pending.name)),
        }
    }

    /// The person denied a pending device: it leaves the sheet and the cloud request is deleted.
    pub fn deny_pending(&mut self, id: &str) {
        self.approvals.deny(id);
        self.handled_pending.push(id.to_string());
        if let Some(link) = self.link.as_ref() {
            link.send(Job::Deny { id: id.to_string() });
        }
    }

    /// The positions shared with the account, opened with the account key: one line each, with the owner's name, the
    /// position and its age. A position that does not open says so.
    pub fn shared_lines(&self) -> Vec<String> {
        let now = unix_now();
        let private = self.keys.private();
        self.shared
            .iter()
            .map(|entry| {
                let blob = Sealed { sealed: entry.blob.sealed.clone(), enc: entry.blob.enc.clone() };
                match private.and_then(|k| sealing::open_report(k, &entry.share_id, &blob).ok()) {
                    Some(r) => format!("{} · {:.4}, {:.4} · {}", entry.owner_name, r.lat, r.lon, age_text(now - r.at_unix)),
                    None => format!("{} · position unavailable", entry.owner_name),
                }
            })
            .collect()
    }
}
