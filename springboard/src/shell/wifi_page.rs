//! Settings › Wi-Fi: what tapping a network does, the "Enter Password"
//! sheet with its keyboard, and the "Other Network" sheet that joins a
//! hidden, WPA3 or 802.1X network by hand. The network list itself is a
//! `list` page built by `pages::spec`.

use crate::layout::in_rect as within;
use super::keyboard::{self, Keyboard, Out};
use super::list::{RADIUS_PT, ROW_PT};
use super::textfield::{Frame, TextField};
use super::ui::{centred, home_zone, Ui, Weight, CARD_BG, LABEL, SECONDARY, SEPARATOR_C};
use super::{Network, WifiAction};
use crate::gfx::{pack, rgba, Image, Surface};
use crate::net::wifi_config::{Eap, NetworkConfig, Security};

pub const BLUE: u32 = pack(10, 132, 255, 255);
const FIELD: u32 = pack(44, 44, 46, 255);
const PSK_MIN: usize = 8;
const PSK_MAX: usize = 63;

pub enum Choice {
    Act(WifiAction),
    /// a secure network that is not saved yet
    Password(String),
}

/// What tapping a network in the list does.
pub fn choose(n: &Network) -> Choice {
    if n.saved {
        Choice::Act(WifiAction::Select(n.ssid.clone()))
    } else if !n.secure {
        Choice::Act(WifiAction::Join { ssid: n.ssid.clone(), psk: None })
    } else {
        Choice::Password(n.ssid.clone())
    }
}

fn centre((x, y, w, h): (f32, f32, f32, f32)) -> (f32, f32) {
    (x + w / 2.0, y + h / 2.0)
}

const CARD_W_PT: f32 = 540.0;
const CARD_H_PT: f32 = 164.0;
const BAR_PT: f32 = 76.0;
const BUTTON_W_PT: f32 = 120.0;
/// Where the password text starts inside the field, after the "Password" label.
const TEXT_INSET_PT: f32 = 126.0;

/// Longest password the Other Network sheet takes (SAE allows long ones).
const OTHER_MAX: usize = 128;
const NAME_MAX: usize = 32;
const ACCOUNT_MAX: usize = 128;
/// Gap between the title bar and the rows, and below the last row.
const GAP_PT: f32 = 12.0;
const NOTICE_LINE_PT: f32 = 16.0;
/// What the user must accept before joining an 802.1X network.
const NOTICE: &str = "Don’t Validate Server: this network’s certificate is not checked, so a look-alike access point could capture your account name and password. Turn it on only for networks you trust.";

/// A row of the Other Network sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Name,
    Security,
    Eap,
    Username,
    Anonymous,
    Password,
    /// the explicit "don't validate the server" acknowledgement
    Validate,
}

/// The rows the sheet shows for a security type, top to bottom.
pub fn rows_for(security: Security) -> &'static [Row] {
    match security {
        Security::Open => &[Row::Name, Row::Security],
        Security::Wep | Security::Wpa | Security::Wpa2 | Security::Wpa3 | Security::Wpa2Wpa3 => &[Row::Name, Row::Security, Row::Password],
        Security::Enterprise(_) => &[Row::Name, Row::Security, Row::Eap, Row::Username, Row::Anonymous, Row::Password, Row::Validate],
    }
}

/// The state and pictures of the Other Network sheet.
struct Other {
    name: TextField,
    identity: TextField,
    anonymous: TextField,
    security: Security,
    eap: Eap,
    acknowledged: bool,
    /// a name typed in by hand is probed for with directed scans
    hidden: bool,
    focus: Row,
    title: Image,
    labels: Vec<(Row, Image)>,
    notice: Vec<Image>,
    security_img: Image,
    eap_img: Image,
    validate_img: Image,
}

impl Other {
    fn new(ui: &Ui, preset: Option<&str>) -> Self {
        let mut name = TextField::new(ui, "Network Name", 17.0, false, NAME_MAX);
        let mut identity = TextField::new(ui, "Required", 17.0, false, ACCOUNT_MAX);
        let mut anonymous = TextField::new(ui, "Optional", 17.0, false, ACCOUNT_MAX);
        // a network from the scan list has its name; start on the account
        let (focus, hidden) = match preset {
            Some(ssid) => {
                name.set_text(ui, ssid);
                (Row::Username, false)
            }
            None => (Row::Name, true),
        };
        name.focused = focus == Row::Name;
        identity.focused = focus == Row::Username;
        anonymous.focused = false;
        let label = |row, text: &str| (row, ui.text(text, 17.0, Weight::Regular, LABEL.now()));
        let notice = ui.wrap(NOTICE, 13.0, Weight::Regular, ui.pt(CARD_W_PT - 40.0)).iter().map(|l| ui.text(l, 13.0, Weight::Regular, SECONDARY)).collect();
        let mut o = Other {
            name,
            identity,
            anonymous,
            security: if preset.is_some() { Security::Enterprise(Eap::Peap) } else { Security::Wpa2Wpa3 },
            eap: Eap::Peap,
            acknowledged: false,
            hidden,
            focus,
            title: ui.text("Other Network", 17.0, Weight::Semibold, LABEL.now()),
            labels: vec![
                label(Row::Name, "Name"),
                label(Row::Security, "Security"),
                label(Row::Eap, "EAP Method"),
                label(Row::Username, "Username"),
                label(Row::Anonymous, "Anonymous"),
                label(Row::Password, "Password"),
                label(Row::Validate, "Don’t Validate Server"),
            ],
            notice,
            security_img: Image::new(1, 1),
            eap_img: Image::new(1, 1),
            validate_img: Image::new(1, 1),
        };
        o.refresh_values(ui);
        o
    }

    /// Re-renders the right-hand values after a tap changed one.
    fn refresh_values(&mut self, ui: &Ui) {
        self.security_img = ui.text(self.security.label(), 17.0, Weight::Regular, SECONDARY);
        self.eap_img = ui.text(self.eap.label(), 17.0, Weight::Regular, SECONDARY);
        self.validate_img = ui.text(if self.acknowledged { "On" } else { "Off" }, 17.0, Weight::Regular, if self.acknowledged { BLUE } else { SECONDARY });
    }

    fn rows(&self) -> &'static [Row] {
        rows_for(self.security)
    }

    fn label(&self, row: Row) -> Option<&Image> {
        self.labels.iter().find(|(r, _)| *r == row).map(|(_, i)| i)
    }

    fn shows_notice(&self) -> bool {
        self.security.is_enterprise()
    }
}

/// What a touch on the sheet landed on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SheetHit {
    Keyboard,
    Cancel,
    Join,
    Field,
}

/// The "Enter Password" sheet for a secure network.
pub struct Sheet {
    pub ssid: String,
    pub field: TextField,
    pub keyboard: Keyboard,
    prompt: Image,
    title: Image,
    cancel: Image,
    join: Image,
    join_off: Image,
    label: Image,
    /// set for the Other Network sheet
    other: Option<Other>,
    /// set for a one-field prompt from another Settings pane (`forms`)
    form: Option<super::forms::Purpose>,
}

impl Sheet {
    /// The Other Network sheet: name, security and credentials. `preset` is
    /// the name of a scanned 802.1X network, which starts on its account fields.
    pub fn other(ui: &Ui, preset: Option<String>, w: i32, t: f64) -> Self {
        let mut sh = Sheet::new(ui, preset.clone().unwrap_or_default(), w, t);
        sh.field = TextField::new(ui, "Required", 17.0, true, OTHER_MAX);
        sh.field.focused = false;
        let other = Other::new(ui, preset.as_deref());
        sh.other = Some(other);
        sh
    }

    /// Opens at `t` with the keyboard sliding in.
    pub fn new(ui: &Ui, ssid: String, w: i32, t: f64) -> Self {
        let prompt = ui.fit(&format!("Enter the password for “{ssid}”"), 13.0, ui.pt(CARD_W_PT - 40.0));
        let mut keyboard = Keyboard::new(w as f32, ui.s, false);
        keyboard.appear(t);
        Sheet {
            field: TextField::new(ui, "", 17.0, true, PSK_MAX),
            keyboard,
            prompt: ui.text(&prompt, 13.0, Weight::Semibold, LABEL.now()),
            title: ui.text("Enter Password", 17.0, Weight::Semibold, LABEL.now()),
            cancel: ui.text("Cancel", 17.0, Weight::Regular, BLUE),
            join: ui.text("Join", 17.0, Weight::Semibold, BLUE),
            join_off: ui.text("Join", 17.0, Weight::Semibold, rgba(235, 235, 245, 77)),
            label: ui.text("Password", 17.0, Weight::Regular, LABEL.now()),
            other: None,
            form: None,
            ssid,
        }
    }

    /// A one-field prompt (`forms::Prompt`): its title, message, field label and button replace the password sheet's.
    pub fn form(ui: &Ui, w: i32, t: f64, p: super::forms::Prompt) -> Self {
        let mut sh = Sheet::new(ui, String::new(), w, t);
        let message = ui.fit(&p.message, 13.0, ui.pt(CARD_W_PT - 40.0));
        sh.field = TextField::new(ui, "", 17.0, p.secure, p.max);
        sh.field.set_text(ui, &p.initial);
        sh.prompt = ui.text(&message, 13.0, Weight::Semibold, LABEL.now());
        sh.title = ui.text(&p.title, 17.0, Weight::Semibold, LABEL.now());
        sh.join = ui.text(p.ok, 17.0, Weight::Semibold, BLUE);
        sh.join_off = ui.text(p.ok, 17.0, Weight::Semibold, rgba(235, 235, 245, 77));
        sh.label = ui.text(p.label, 17.0, Weight::Regular, LABEL.now());
        sh.form = Some(p.purpose);
        sh
    }

    /// What a prompt sheet collected, once its text is acceptable.
    pub fn form_result(&self) -> Option<(super::forms::Purpose, String)> {
        let purpose = self.form.clone()?;
        super::forms::acceptable(&purpose, self.field.text()).then(|| (purpose, self.field.text().to_string()))
    }

    pub fn password(&self) -> &str {
        self.field.text()
    }

    /// The network typed into the Other Network sheet.
    pub fn config(&self) -> Option<NetworkConfig> {
        let o = self.other.as_ref()?;
        let security = match o.security {
            Security::Enterprise(_) => Security::Enterprise(o.eap),
            s => s,
        };
        Some(NetworkConfig {
            ssid: o.name.text().to_string(),
            security,
            password: self.field.text().to_string(),
            hidden: o.hidden,
            identity: o.identity.text().to_string(),
            anonymous: o.anonymous.text().to_string(),
            auto_join: true,
        })
    }

    pub fn can_join(&self) -> bool {
        if let Some(p) = &self.form {
            return super::forms::acceptable(p, self.field.text());
        }
        match (&self.other, self.config()) {
            (Some(o), Some(cfg)) => cfg.problem().is_none() && (!o.security.is_enterprise() || o.acknowledged),
            _ => self.field.len() >= PSK_MIN,
        }
    }

    /// What joining does: a saved-network password, or the typed-in network.
    pub fn action(&self) -> WifiAction {
        match self.config() {
            Some(cfg) => WifiAction::JoinOther(cfg),
            None => WifiAction::Join { ssid: self.ssid.clone(), psk: Some(self.password().to_string()) },
        }
    }

    /// The row whose field the keyboard types into.
    fn focus_row(&self) -> Row {
        self.other.as_ref().map_or(Row::Password, |o| o.focus)
    }

    fn field_mut(&mut self, row: Row) -> Option<&mut TextField> {
        match (row, &mut self.other) {
            (Row::Password, _) => Some(&mut self.field),
            (Row::Name, Some(o)) => Some(&mut o.name),
            (Row::Username, Some(o)) => Some(&mut o.identity),
            (Row::Anonymous, Some(o)) => Some(&mut o.anonymous),
            _ => None,
        }
    }

    /// Every text field with its row.
    fn fields_mut(&mut self) -> Vec<(Row, &mut TextField)> {
        let mut v = vec![(Row::Password, &mut self.field)];
        if let Some(o) = &mut self.other {
            v.push((Row::Name, &mut o.name));
            v.push((Row::Username, &mut o.identity));
            v.push((Row::Anonymous, &mut o.anonymous));
        }
        v
    }

    /// Moves the caret to `row`'s field; only that field blinks.
    fn set_focus(&mut self, row: Row) {
        for (r, f) in self.fields_mut() {
            f.focused = r == row;
        }
        if let Some(o) = &mut self.other {
            o.focus = row;
        }
    }

    /// The text row after the focused one, if any.
    fn next_text_row(&self) -> Option<Row> {
        let o = self.other.as_ref()?;
        let rows = o.rows();
        let at = rows.iter().position(|r| *r == o.focus)?;
        rows[at + 1..].iter().copied().find(|r| matches!(r, Row::Name | Row::Username | Row::Anonymous | Row::Password))
    }

    /// Applies a key; true when the user asked to join. In the Other
    /// Network sheet Tab and Return step to the next text field first.
    pub fn edit(&mut self, ui: &Ui, out: Out, t: f64) -> bool {
        if out == Out::Hide {
            self.keyboard.set_shown(false, t);
            for (_, f) in self.fields_mut() {
                f.focused = false;
            }
            return false;
        }
        if self.other.is_some() && matches!(out, Out::Named("Tab") | Out::Enter) {
            return match self.next_text_row() {
                Some(next) => {
                    self.set_focus(next);
                    false
                }
                None => out == Out::Enter,
            };
        }
        let row = self.focus_row();
        self.field_mut(row).is_some_and(|f| f.edit(ui, &out, t))
    }

    /// A tap on a row of the Other Network sheet outside a text box (or
    /// anywhere on a row that is not a field): focuses the field, cycles the
    /// security or EAP method, or flips the server-validation switch.
    pub fn press(&mut self, ui: &Ui, w: i32, h: i32, x: f32, y: f32, t: f64) {
        let Some(row) = self.row_at(ui, w, h, x, y) else {
            if self.other.is_some() {
                return;
            }
            // the password sheet's single field
            let tx = self.text_x(ui, w, h);
            self.focus(ui, x - tx, t);
            return;
        };
        match row {
            Row::Security | Row::Eap | Row::Validate => {
                if let Some(o) = &mut self.other {
                    match row {
                        Row::Security => {
                            o.security = o.security.next();
                            if !o.rows().contains(&o.focus) {
                                o.focus = Row::Name;
                            }
                        }
                        Row::Eap => o.eap = if o.eap == Eap::Peap { Eap::Ttls } else { Eap::Peap },
                        _ => o.acknowledged = !o.acknowledged,
                    }
                    o.refresh_values(ui);
                }
                // a change of rows can leave the focus on a field that is gone
                let focus = self.focus_row();
                self.set_focus(focus);
            }
            _ => {
                let left = self.row_text_left(ui, w, h, row);
                self.set_focus(row);
                self.keyboard.set_shown(true, t);
                if let Some(f) = self.field_mut(row) {
                    f.tap(ui, x - left, t);
                }
            }
        }
    }

    /// Brings the keyboard back and focuses the field, placing the caret at
    /// `x` px into the text.
    pub fn focus(&mut self, ui: &Ui, x: f32, t: f64) {
        self.keyboard.set_shown(true, t);
        self.field.tap(ui, x, t);
    }

    /// Top of the docked keyboard while it is shown.
    pub fn keyboard_top(&self, ui: &Ui, h: i32) -> Option<f32> {
        self.keyboard.shown().then(|| h as f32 - home_zone(ui) - self.keyboard.h())
    }

    /// Px from the field's left edge to where the password text starts.
    pub fn text_x(&self, ui: &Ui, w: i32, h: i32) -> f32 {
        self.field_rect(ui, w, h).0 + ui.pt(TEXT_INSET_PT)
    }

    /// The box the password text is drawn in.
    fn text_frame(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let (fx, fy, fw, fh) = self.field_rect(ui, w, h);
        let tx = fx + ui.pt(TEXT_INSET_PT);
        (tx, fy, fx + fw - ui.pt(16.0) - tx, fh)
    }

    /// Offers a touch to the text fields (their text, handles and editing
    /// menu); true when a field took it. A tap or long press that focuses
    /// a field brings the keyboard back.
    pub fn field_touch(&mut self, ui: &Ui, w: i32, h: i32, ev: super::Event, t: f64) -> bool {
        for (row, frame) in self.text_frames(ui, w, h) {
            let Some(field) = self.field_mut(row) else { continue };
            let took = field.touch(ui, ev, frame, w as f32, t);
            if took && field.focused {
                self.set_focus(row);
                self.keyboard.set_shown(true, t);
            }
            if took {
                return true;
            }
        }
        false
    }

    /// Repeats held keys, fires a long press on a field and finishes the
    /// keyboard slide; true when a frame is needed.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> (Vec<Out>, bool) {
        let outs = self.keyboard.tick(t);
        let row = self.focus_row();
        for out in &outs {
            if let Some(f) = self.field_mut(row) {
                f.edit(ui, out, t);
            }
        }
        let mut pressed = None;
        for (row, f) in self.fields_mut() {
            if f.tick(ui, t) && f.focused {
                pressed = Some(row);
            }
        }
        if let Some(row) = pressed {
            self.set_focus(row);
            self.keyboard.set_shown(true, t);
        }
        let blinking = self.field_wake(t).is_some_and(|due| due <= t + 1.0);
        let frame = !outs.is_empty() || pressed.is_some() || blinking || self.keyboard.animating();
        (outs, frame)
    }

    /// The earliest caret blink or long press among the fields.
    fn field_wake(&self, t: f64) -> Option<f64> {
        let mut wake: Option<f64> = self.field.next_wake(t);
        if let Some(o) = &self.other {
            for f in [&o.name, &o.identity, &o.anonymous] {
                wake = match (wake, f.next_wake(t)) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
            }
        }
        wake
    }

    /// When the next frame is due for the caret blink, a long press or the slide.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.keyboard.animating() {
            return Some(t);
        }
        self.field_wake(t)
    }

    /// The card's height: fixed for the password sheet, as tall as its rows
    /// (and the 802.1X notice) for the Other Network sheet.
    fn card_h(&self, ui: &Ui) -> f32 {
        match &self.other {
            None => ui.pt(CARD_H_PT),
            Some(o) => {
                let notice = if o.shows_notice() { ui.pt(8.0) + ui.pt(NOTICE_LINE_PT) * o.notice.len() as f32 } else { 0.0 };
                ui.pt(BAR_PT + 2.0 * GAP_PT) + ui.pt(ROW_PT) * o.rows().len() as f32 + notice
            }
        }
    }

    /// The card, centred in the space above the keyboard. When the sheet
    /// is taller than that, it rises until the focused row clears the keyboard.
    fn card(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let (cw, ch) = (ui.pt(CARD_W_PT), self.card_h(ui));
        let (top, bottom) = (ui.pt(24.0), self.keyboard_top(ui, h).unwrap_or(h as f32));
        let mut y = (top + (bottom - top - ch) / 2.0).round();
        if let Some(o) = &self.other {
            y = y.max(top);
            let at = o.rows().iter().position(|r| *r == o.focus).unwrap_or(0) as f32;
            let focus_bottom = y + ui.pt(BAR_PT + GAP_PT + ROW_PT) + ui.pt(ROW_PT) * at;
            let limit = bottom - ui.pt(8.0);
            if self.keyboard_top(ui, h).is_some() && focus_bottom > limit {
                y -= focus_bottom - limit;
            }
        }
        ((w as f32 - cw) / 2.0, y, cw, ch)
    }

    /// The Other Network sheet's row `i`: (x, y, w, h).
    fn row_rect(&self, ui: &Ui, w: i32, h: i32, i: usize) -> Frame {
        let (x, y, cw, _) = self.card(ui, w, h);
        (x + ui.pt(16.0), y + ui.pt(BAR_PT + GAP_PT) + ui.pt(ROW_PT) * i as f32, cw - ui.pt(32.0), ui.pt(ROW_PT))
    }

    /// The row under a touch in the Other Network sheet.
    fn row_at(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Row> {
        let o = self.other.as_ref()?;
        (0..o.rows().len()).find(|&i| within(x, y, self.row_rect(ui, w, h, i))).map(|i| o.rows()[i])
    }

    /// Px where a text row's text starts, after its label.
    fn row_text_left(&self, ui: &Ui, w: i32, h: i32, row: Row) -> f32 {
        let at = self.other.as_ref().and_then(|o| o.rows().iter().position(|r| *r == row)).unwrap_or(0);
        self.row_rect(ui, w, h, at).0 + ui.pt(TEXT_INSET_PT)
    }

    /// The boxes the text fields draw into, one per visible text row.
    fn text_frames(&self, ui: &Ui, w: i32, h: i32) -> Vec<(Row, Frame)> {
        match &self.other {
            None => vec![(Row::Password, self.text_frame(ui, w, h))],
            Some(o) => o
                .rows()
                .iter()
                .enumerate()
                .filter(|(_, r)| matches!(r, Row::Name | Row::Username | Row::Anonymous | Row::Password))
                .map(|(i, r)| {
                    let (rx, ry, rw, rh) = self.row_rect(ui, w, h, i);
                    let tx = rx + ui.pt(TEXT_INSET_PT);
                    (*r, (tx, ry, rx + rw - ui.pt(16.0) - tx, rh))
                })
                .collect(),
        }
    }

    fn cancel_rect(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let (x, y, _, _) = self.card(ui, w, h);
        (x, y, ui.pt(BUTTON_W_PT), ui.pt(BAR_PT))
    }

    fn join_rect(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let (x, y, cw, _) = self.card(ui, w, h);
        (x + cw - ui.pt(BUTTON_W_PT), y, ui.pt(BUTTON_W_PT), ui.pt(BAR_PT))
    }

    fn field_rect(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let (x, y, cw, _) = self.card(ui, w, h);
        (x + ui.pt(16.0), y + ui.pt(BAR_PT + 16.0), cw - ui.pt(32.0), ui.pt(ROW_PT))
    }

    pub fn cancel_button(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32) {
        centre(self.cancel_rect(ui, w, h))
    }

    pub fn join_button(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32) {
        centre(self.join_rect(ui, w, h))
    }

    pub fn field(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32) {
        centre(self.field_rect(ui, w, h))
    }

    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<SheetHit> {
        if self.keyboard_top(ui, h).is_some_and(|top| y >= top) {
            Some(SheetHit::Keyboard)
        } else if within(x, y, self.cancel_rect(ui, w, h)) {
            Some(SheetHit::Cancel)
        } else if within(x, y, self.join_rect(ui, w, h)) {
            Some(SheetHit::Join)
        } else if self.other.is_some() && self.row_at(ui, w, h, x, y).is_some() {
            Some(SheetHit::Field)
        } else if self.other.is_none() && within(x, y, self.field_rect(ui, w, h)) {
            Some(SheetHit::Field)
        } else {
            None
        }
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, w: i32, h: i32, t: f64) {
        s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, rgba(0, 0, 0, 128));
        if self.other.is_some() {
            self.draw_rows(s, ui, w, h, t);
        } else {
            self.draw_card(s, ui, w, h, t);
        }
        if self.keyboard.shown() || self.keyboard.animating() {
            let top = h as f32 - home_zone(ui) - self.keyboard.h();
            self.keyboard.draw(s, ui, top, t);
            let below = top + self.keyboard.h() + self.keyboard.offset(t);
            s.fill_rrect(0.0, below, w as f32, h as f32 - below, 0.0, keyboard::BG);
        }
    }

    /// The Other Network sheet: title bar, one grouped block of rows, and
    /// the notice an 802.1X network needs accepted.
    fn draw_rows(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32, t: f64) {
        let Some(o) = &self.other else { return };
        let (x, y, cw, ch) = self.card(ui, w, h);
        let radius = ui.pt(RADIUS_PT).min(6.0);
        s.fill_rrect(x, y, cw, ch, radius, CARD_BG.now());
        let cx = (x + cw / 2.0) as i32;
        let bar = y + ui.pt(BAR_PT) / 2.0;
        let mid = |img: &Image| (bar - img.h as f32 / 2.0) as i32;
        centred(s, &o.title, cx, mid(&o.title));
        s.blit(&self.cancel, (x + ui.pt(16.0)) as i32, mid(&self.cancel), 255, 255);
        let join = if self.can_join() { &self.join } else { &self.join_off };
        s.blit(join, (x + cw - ui.pt(16.0) - join.w as f32) as i32, mid(join), 255, 255);
        s.fill_rrect(x, y + ui.pt(BAR_PT) - 1.0, cw, 1.0, 0.0, SEPARATOR_C.now());

        let rows = o.rows();
        let (bx, by, bw, _) = self.row_rect(ui, w, h, 0);
        s.fill_rrect(bx, by, bw, ui.pt(ROW_PT) * rows.len() as f32, radius, FIELD);
        for (i, row) in rows.iter().enumerate() {
            let (rx, ry, rw, rh) = self.row_rect(ui, w, h, i);
            let ymid = ry + rh / 2.0;
            if i > 0 {
                s.fill_rrect(rx + ui.pt(16.0), ry, rw - ui.pt(16.0), 1.0, 0.0, SEPARATOR_C.now());
            }
            if let Some(label) = o.label(*row) {
                s.blit(label, (rx + ui.pt(16.0)) as i32, (ymid - label.h as f32 / 2.0) as i32, 255, 255);
            }
            let value = match row {
                Row::Security => Some(&o.security_img),
                Row::Eap => Some(&o.eap_img),
                Row::Validate => Some(&o.validate_img),
                _ => None,
            };
            if let Some(v) = value {
                s.blit(v, (rx + rw - ui.pt(16.0) - v.w as f32) as i32, (ymid - v.h as f32 / 2.0) as i32, 255, 255);
            }
        }
        for (row, frame) in self.text_frames(ui, w, h) {
            let field = match row {
                Row::Name => &o.name,
                Row::Username => &o.identity,
                Row::Anonymous => &o.anonymous,
                _ => &self.field,
            };
            field.draw(s, ui, frame, t);
        }
        if o.shows_notice() {
            let ny = by + ui.pt(ROW_PT) * rows.len() as f32 + ui.pt(8.0);
            for (i, line) in o.notice.iter().enumerate() {
                s.blit(line, (bx + ui.pt(4.0)) as i32, (ny + ui.pt(NOTICE_LINE_PT) * i as f32) as i32, 255, 255);
            }
        }
    }

    fn draw_card(&mut self, s: &mut Surface, ui: &Ui, w: i32, h: i32, t: f64) {
        let (x, y, cw, ch) = self.card(ui, w, h);
        s.fill_rrect(x, y, cw, ch, ui.pt(13.0), CARD_BG.now());
        let cx = (x + cw / 2.0) as i32;
        centred(s, &self.prompt, cx, (y + ui.pt(20.0) - self.prompt.h as f32 / 2.0) as i32);
        let bar = y + ui.pt(52.0);
        let mid = |img: &Image| (bar - img.h as f32 / 2.0) as i32;
        centred(s, &self.title, cx, mid(&self.title));
        s.blit(&self.cancel, (x + ui.pt(16.0)) as i32, mid(&self.cancel), 255, 255);
        let join = if self.can_join() { &self.join } else { &self.join_off };
        s.blit(join, (x + cw - ui.pt(16.0) - join.w as f32) as i32, mid(join), 255, 255);
        s.fill_rrect(x, y + ui.pt(BAR_PT) - 1.0, cw, 1.0, 0.0, SEPARATOR_C.now());

        let (fx, fy, fw, fh) = self.field_rect(ui, w, h);
        s.fill_rrect(fx, fy, fw, fh, ui.pt(RADIUS_PT), FIELD);
        let fmid = fy + fh / 2.0;
        s.blit(&self.label, (fx + ui.pt(16.0)) as i32, (fmid - self.label.h as f32 / 2.0) as i32, 255, 255);
        self.field.draw(s, ui, self.text_frame(ui, w, h), t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> Ui {
        super::super::tests::ui()
    }

    #[test]
    fn choice_depends_on_saved_and_secure() {
        let n = |secure, saved| Network { ssid: "N".into(), bars: 1, secure, saved };
        assert!(matches!(choose(&n(true, true)), Choice::Act(WifiAction::Select(s)) if s == "N"));
        assert!(matches!(choose(&n(false, false)), Choice::Act(WifiAction::Join { psk: None, .. })));
        assert!(matches!(choose(&n(true, false)), Choice::Password(s) if s == "N"));
    }

    #[test]
    fn password_editing() {
        let ui = ui();
        let mut sh = Sheet::new(&ui, "Net".into(), 1200, 0.0);
        assert!(!sh.edit(&ui, Out::Text("abc".into()), 1.0));
        sh.edit(&ui, Out::Backspace, 2.0);
        assert_eq!(sh.password(), "ab");
        sh.edit(&ui, Out::Named("Tab"), 3.0);
        assert_eq!(sh.password(), "ab");
        assert!(!sh.can_join());
        sh.edit(&ui, Out::Text("cdéfgh".into()), 4.0);
        assert!(sh.can_join());
        assert!(sh.edit(&ui, Out::Enter, 5.0));
        sh.edit(&ui, Out::Text("x".repeat(100)), 6.0);
        assert_eq!(sh.password().chars().count(), PSK_MAX, "WPA passphrases stop at 63");
        sh.edit(&ui, Out::Hide, 7.0);
        assert!(!sh.keyboard.shown() && !sh.field.focused);
    }

    #[test]
    fn sheet_sits_above_the_keyboard_and_recentres_without_it() {
        let ui = ui();
        let mut sh = Sheet::new(&ui, "Net".into(), 1200, 0.0);
        let top = sh.keyboard_top(&ui, 1920).unwrap();
        let (_, y, _, ch) = sh.card(&ui, 1200, 1920);
        assert!(y + ch < top && y > ui.pt(24.0));
        assert_eq!(sh.hit(&ui, 1200, 1920, 600.0, top + 5.0), Some(SheetHit::Keyboard));
        sh.edit(&ui, Out::Hide, 1.0);
        let (_, y2, _, _) = sh.card(&ui, 1200, 1920);
        assert!(y2 > y);
        assert_eq!(sh.hit(&ui, 1200, 1920, 600.0, top + 5.0), None);
        let (fx, fy) = sh.field(&ui, 1200, 1920);
        sh.focus(&ui, fx - sh.text_x(&ui, 1200, 1920), 2.0);
        assert!(sh.keyboard.shown() && sh.field.focused);
        assert!(sh.next_wake(2.0).is_some(), "sliding keyboard wants frames");
        let mut buf = vec![0u32; 1200 * 1920];
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, 1920, 100.0);
        assert!(fy > 0.0);
    }

    // Auto-repeat of a held key belongs to the keyboard app: `holding_backspace_repeats_after_a_delay` in apps/keyboard.

    fn row_centre(sh: &Sheet, ui: &Ui, row: Row) -> (f32, f32) {
        let at = sh.other.as_ref().unwrap().rows().iter().position(|r| *r == row).unwrap();
        centre(sh.row_rect(ui, 1200, 1920, at))
    }

    fn tap_row(sh: &mut Sheet, ui: &Ui, row: Row, t: f64) {
        let (x, y) = row_centre(sh, ui, row);
        assert_eq!(sh.hit(ui, 1200, 1920, x, y), Some(SheetHit::Field));
        sh.press(ui, 1200, 1920, x, y, t);
    }

    #[test]
    fn rows_follow_the_security_type() {
        assert_eq!(rows_for(Security::Open), [Row::Name, Row::Security]);
        assert_eq!(rows_for(Security::Wpa3), [Row::Name, Row::Security, Row::Password]);
        let ent = rows_for(Security::Enterprise(Eap::Ttls));
        assert_eq!(ent, [Row::Name, Row::Security, Row::Eap, Row::Username, Row::Anonymous, Row::Password, Row::Validate]);
    }

    #[test]
    fn other_network_joins_a_hidden_wpa_network() {
        let ui = ui();
        let mut sh = Sheet::other(&ui, None, 1200, 0.0);
        assert!(!sh.can_join(), "no name yet");
        sh.edit(&ui, Out::Text("Ghost".into()), 1.0);
        assert!(!sh.can_join(), "WPA2/WPA3 needs a password");
        sh.edit(&ui, Out::Named("Tab"), 2.0);
        sh.edit(&ui, Out::Text("secret12".into()), 3.0);
        assert!(sh.can_join());
        let cfg = sh.config().unwrap();
        assert_eq!((cfg.ssid.as_str(), cfg.security, cfg.password.as_str(), cfg.hidden), ("Ghost", Security::Wpa2Wpa3, "secret12", true));
        assert!(matches!(sh.action(), WifiAction::JoinOther(c) if c == cfg));
        assert!(sh.edit(&ui, Out::Enter, 4.0), "return on the last field submits");
    }

    #[test]
    fn security_row_cycles_and_open_needs_only_a_name() {
        let ui = ui();
        let mut sh = Sheet::other(&ui, None, 1200, 0.0);
        sh.edit(&ui, Out::Text("Cafe".into()), 1.0);
        tap_row(&mut sh, &ui, Row::Security, 2.0);
        assert_eq!(sh.other.as_ref().unwrap().security, Security::Enterprise(Eap::Peap));
        tap_row(&mut sh, &ui, Row::Security, 3.0);
        assert_eq!(sh.other.as_ref().unwrap().security, Security::Open);
        assert!(sh.can_join());
        assert_eq!(sh.config().unwrap().problem(), None);
    }

    #[test]
    fn enterprise_needs_an_account_and_the_acknowledgement() {
        let ui = ui();
        let mut sh = Sheet::other(&ui, None, 1200, 0.0);
        sh.edit(&ui, Out::Text("Corp".into()), 1.0);
        tap_row(&mut sh, &ui, Row::Security, 2.0);
        assert!(sh.other.as_ref().unwrap().security.is_enterprise());
        tap_row(&mut sh, &ui, Row::Username, 3.0);
        sh.edit(&ui, Out::Text("alice".into()), 4.0);
        tap_row(&mut sh, &ui, Row::Password, 5.0);
        sh.edit(&ui, Out::Text("hunter22".into()), 6.0);
        assert!(!sh.can_join(), "the server is not validated until the user says so");
        tap_row(&mut sh, &ui, Row::Eap, 7.0);
        tap_row(&mut sh, &ui, Row::Validate, 8.0);
        assert!(sh.can_join());
        let cfg = sh.config().unwrap();
        assert_eq!(cfg.security, Security::Enterprise(Eap::Ttls));
        assert_eq!((cfg.identity.as_str(), cfg.password.as_str(), cfg.anonymous.as_str()), ("alice", "hunter22", ""));
        tap_row(&mut sh, &ui, Row::Validate, 9.0);
        assert!(!sh.can_join(), "switching it off again blocks joining");
    }

    #[test]
    fn a_scanned_enterprise_network_starts_on_the_username() {
        let ui = ui();
        let mut sh = Sheet::other(&ui, Some("Corp".into()), 1200, 0.0);
        sh.edit(&ui, Out::Text("bob".into()), 1.0);
        let cfg = sh.config().unwrap();
        assert_eq!((cfg.ssid.as_str(), cfg.identity.as_str(), cfg.hidden), ("Corp", "bob", false));
        assert!(cfg.security.is_enterprise());
    }

    #[test]
    fn the_tall_sheet_keeps_the_focused_row_above_the_keyboard() {
        let ui = ui();
        let mut sh = Sheet::other(&ui, Some("Corp".into()), 1200, 0.0);
        tap_row(&mut sh, &ui, Row::Password, 1.0);
        let top = sh.keyboard_top(&ui, 900).unwrap();
        let at = sh.other.as_ref().unwrap().rows().iter().position(|r| *r == Row::Password).unwrap();
        let (_, ry, _, rh) = sh.row_rect(&ui, 1200, 900, at);
        assert!(ry + rh <= top, "the password row is not hidden by the keyboard");
        let mut buf = vec![0u32; 1200 * 900];
        sh.draw(&mut Surface { w: 1200, h: 900, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, 900, 10.0);
    }
}
