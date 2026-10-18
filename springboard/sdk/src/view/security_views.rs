//! Security panels an app can show: the certificate panel with a details disclosure, the trust settings editor and the
//! choose-identity panel. Each is a [`View`] drawn with the kit's colours, type and row heights over the models in
//! `bifrost_app::framework_kits::security_interface` (`CertificatePanel`, `CertificateTrustPanel`, `IdentityChooser`).
//! The model decides (validity, effective trust, which identity is chosen); the view shows it and turns a click or a key
//! into a change of the model plus one action string for the responder chain, like every other control.
//!
//! Actions (sent from the view; read the model back with `Window::view`):
//! - [`CertificatePanelView`]: `certificate.select` after a different certificate of the chain was selected,
//!   `certificate.disclosure` after Details was opened or closed.
//! - [`TrustEditorView`]: `trust.setting` after one entry's trust setting changed, `trust.reset` after Reset to Defaults.
//! - [`IdentityChooserView`]: `identity.select` after another identity was selected, `identity.choose` for the default
//!   button (the chosen identity is `chooser().confirm()`), `identity.cancel` for the alternate button.
//!
//! Layout is a column of kit rows ([`ROW_PT`] high) inside the frame, so the panels scale with the window and the text
//! size; text that does not fit its row is cut with an ellipsis. Only a name and a validity word are drawn; nothing
//! here reads a certificate encoding or a key. A status is never shown by colour alone: the dot beside a certificate
//! has its word on the same row, and the accessibility label names it.
use super::controls::{draw_text, focus_ring, is_activate, node};
use super::{View, ViewBase, ViewEvent};
use crate::kit::ui::{Weight, CAPSULE_BG, CONTROL_PRESSED, FIELD_BG, LABEL, LABEL_2, LABEL_3, SELECTION, SEGMENT_ON, SYSTEM_GREEN, SYSTEM_ORANGE, SYSTEM_RED, WHITE};
use crate::kit::{Event, Ui};
use crate::layout::ROW_PT;
use crate::proto::KeyInput;
use bifrost_render::canvas::{Canvas, RectF};
use bifrost_render::ui::{MAC_BODY_PT, MAC_CALLOUT_PT, SPACE_L, SPACE_M, SPACE_S, SPACE_XS};
use bifrost_ui::certificate_panel::{CertificatePanel, PanelError, Validity};
use bifrost_ui::certificate_trust::{CertificateTrustPanel, TrustSetting};
use bifrost_ui::identity_chooser::{ChooserError, IdentityChooser};
use bifrost_ui::{Node, Rect, Role};

/// Actions of the certificate panel.
pub const ACTION_CERTIFICATE_SELECT: &str = "certificate.select";
pub const ACTION_CERTIFICATE_DISCLOSURE: &str = "certificate.disclosure";
/// Actions of the trust settings editor.
pub const ACTION_TRUST_SETTING: &str = "trust.setting";
pub const ACTION_TRUST_RESET: &str = "trust.reset";
/// Actions of the choose-identity panel.
pub const ACTION_IDENTITY_SELECT: &str = "identity.select";
pub const ACTION_IDENTITY_CHOOSE: &str = "identity.choose";
pub const ACTION_IDENTITY_CANCEL: &str = "identity.cancel";

/// Corner radius of a selected row and of a capsule, in points.
const PLATE_RADIUS_PT: f32 = 6.0;
/// Validity dot diameter and the chevron box edge, in points.
const DOT_PT: f32 = 8.0;
const CHEVRON_PT: f32 = 8.0;
/// Share of the width the label column of the details rows takes.
const LABEL_COLUMN: f32 = 0.34;
/// Share of the width the three-way control of a trust row takes.
const SEGMENTS_COLUMN: f32 = 0.5;
/// The widest a button of the identity panel is, in points.
const BUTTON_W_PT: f32 = 88.0;

// Arrow keys, space and return as the SDK delivers them.
const KEY_RETURN: u32 = 13;
const KEY_ESCAPE: u32 = 27;
const KEY_LEFT: u32 = 37;
const KEY_UP: u32 = 38;
const KEY_RIGHT: u32 = 39;
const KEY_DOWN: u32 = 40;

fn px(ui: &Ui, r: Rect) -> RectF {
    RectF::new(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h))
}

/// The colour of a validity dot.
fn validity_color(v: Validity) -> u32 {
    match v {
        Validity::Valid => SYSTEM_GREEN,
        Validity::NotYetValid => SYSTEM_ORANGE,
        Validity::Expired => SYSTEM_RED,
    }
}

fn dot(s: &mut dyn Canvas, ui: &Ui, row: Rect, x: f32, color: u32) {
    let r = Rect { x, y: row.y + (row.h - DOT_PT) / 2.0, w: DOT_PT, h: DOT_PT };
    s.fill_rrect(px(ui, r), ui.pt(DOT_PT / 2.0), color);
}

/// A disclosure triangle with its top left at `(x, y)`: right-pointing when closed, down-pointing when open.
fn chevron(s: &mut dyn Canvas, ui: &Ui, x: f32, y: f32, open: bool, color: u32) {
    let half = CHEVRON_PT / 2.0;
    for k in 0..(half as usize) {
        let k = k as f32;
        let r = if open {
            Rect { x: x + k, y: y + k + half / 2.0, w: CHEVRON_PT - 2.0 * k, h: 1.0 }
        } else {
            Rect { x: x + k + half / 2.0, y: y + k, w: 1.0, h: CHEVRON_PT - 2.0 * k }
        };
        s.fill_rect(px(ui, r), color);
    }
}

/// `text` against the trailing edge of `row`, in the secondary colour.
fn draw_trailing(s: &mut dyn Canvas, ui: &Ui, row: Rect, text: &str, color: u32) {
    let w = ui.measure(text, MAC_CALLOUT_PT, Weight::Regular) / ui.pt(1.0) + 1.0;
    let r = Rect { x: row.x + row.w - w - SPACE_M, y: row.y, w, h: row.h };
    draw_text(s, ui, r, text, MAC_CALLOUT_PT, Weight::Regular, color, false);
}

/// A capsule button plate with its title.
fn draw_button(s: &mut dyn Canvas, ui: &Ui, r: Rect, title: &str, enabled: bool, pressed: bool) {
    let fill = if pressed && enabled { CONTROL_PRESSED } else { CAPSULE_BG };
    s.fill_rrect(px(ui, r), ui.pt(r.h / 2.0), fill.now());
    let c = if enabled { LABEL } else { LABEL_3 };
    draw_text(s, ui, r, title, MAC_BODY_PT, Weight::Regular, c.now(), true);
}

// ---- Certificate panel ----

/// What a point can land on in the certificate panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CertHit {
    Chain(usize),
    Disclosure,
}

/// The certificate panel (`SFCertificatePanel` / `SFCertificateView`): the chain, leaf first, with one certificate
/// selected, and a Details disclosure under it that opens the fields of the selected certificate.
pub struct CertificatePanelView {
    base: ViewBase,
    panel: CertificatePanel,
    /// The time validity is judged at, Unix seconds. The view reads no clock.
    now: i64,
    disclosed: bool,
    pressed: Option<CertHit>,
}

impl CertificatePanelView {
    /// A panel over `panel`, judging validity at `now`. Details start hidden.
    pub fn new(panel: CertificatePanel, now: i64) -> Self {
        CertificatePanelView { base: ViewBase::default(), panel, now, disclosed: false, pressed: None }
    }

    pub fn panel(&self) -> &CertificatePanel {
        &self.panel
    }

    pub fn now(&self) -> i64 {
        self.now
    }

    /// Moves the judging time (the app's clock moved on).
    pub fn set_now(&mut self, now: i64) {
        if self.now != now {
            self.now = now;
            self.base.dirty = true;
        }
    }

    pub fn is_disclosed(&self) -> bool {
        self.disclosed
    }

    /// Opens or closes Details. True only when the state changed.
    pub fn set_disclosed(&mut self, on: bool) -> bool {
        let changed = self.disclosed != on;
        self.disclosed = on;
        if changed {
            self.base.dirty = true;
        }
        changed
    }

    /// Selects a certificate of the chain; no action is sent. A bad index is refused and changes nothing.
    pub fn select(&mut self, index: usize) -> Result<(), PanelError> {
        let before = self.panel.selected();
        self.panel.select(index)?;
        if before != index {
            self.base.dirty = true;
        }
        Ok(())
    }

    /// The height the panel needs in points: the chain, a gap, the Details row, and the detail rows when open.
    pub fn content_height(&self) -> f32 {
        let details = if self.disclosed { self.detail_count() } else { 0 };
        ROW_PT * (self.panel.chain().len() + 1 + details) as f32 + SPACE_S
    }

    fn detail_count(&self) -> usize {
        self.panel.rows(self.panel.selected(), self.now).map_or(0, |r| r.len())
    }

    fn chain_row(&self, i: usize) -> Rect {
        let f = self.base.frame;
        Rect { x: f.x, y: f.y + ROW_PT * i as f32, w: f.w, h: ROW_PT }
    }

    fn disclosure_row(&self) -> Rect {
        let f = self.base.frame;
        Rect { x: f.x, y: f.y + ROW_PT * self.panel.chain().len() as f32 + SPACE_S, w: f.w, h: ROW_PT }
    }

    fn detail_row(&self, k: usize) -> Rect {
        let d = self.disclosure_row();
        Rect { x: d.x, y: d.y + ROW_PT * (k + 1) as f32, w: d.w, h: ROW_PT }
    }

    fn hit(&self, x: f32, y: f32) -> Option<CertHit> {
        if !self.base.frame.contains(x, y) {
            return None;
        }
        if self.disclosure_row().contains(x, y) {
            return Some(CertHit::Disclosure);
        }
        (0..self.panel.chain().len()).find(|&i| self.chain_row(i).contains(x, y)).map(CertHit::Chain)
    }

    fn act(&mut self, hit: CertHit) {
        match hit {
            CertHit::Chain(i) => {
                if self.panel.selected() != i && self.panel.select(i).is_ok() {
                    self.base.dirty = true;
                    self.base.send_action(ACTION_CERTIFICATE_SELECT);
                }
            }
            CertHit::Disclosure => self.toggle(),
        }
    }

    fn toggle(&mut self) {
        let on = !self.disclosed;
        self.set_disclosed(on);
        self.base.send_action(ACTION_CERTIFICATE_DISCLOSURE);
    }

    fn step(&mut self, delta: isize) -> bool {
        let n = self.panel.chain().len() as isize;
        let next = (self.panel.selected() as isize + delta).clamp(0, n - 1) as usize;
        self.act(CertHit::Chain(next));
        true
    }
}

impl View for CertificatePanelView {
    crate::view_base!();

    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        (0.0, self.content_height())
    }

    fn accepts_first_responder(&self) -> bool {
        true
    }

    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        for (i, cert) in self.panel.chain().iter().enumerate() {
            let row = self.chain_row(i);
            let selected = i == self.panel.selected();
            if selected {
                s.fill_rrect(px(ui, row), ui.pt(PLATE_RADIUS_PT), SELECTION.now());
            }
            let validity = cert.validity(self.now);
            let indent = SPACE_M + SPACE_M * i as f32;
            dot(s, ui, row, row.x + indent, validity_color(validity));
            let text = Rect { x: row.x + indent + DOT_PT + SPACE_S, y: row.y, w: (row.w - indent - DOT_PT - 2.0 * SPACE_S - SPACE_L * 4.0).max(0.0), h: row.h };
            let (main, side) = if selected { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
            draw_text(s, ui, text, &cert.subject, MAC_BODY_PT, Weight::Regular, main, false);
            draw_trailing(s, ui, row, validity.text(), side);
        }
        let d = self.disclosure_row();
        chevron(s, ui, d.x + SPACE_M, d.y + (d.h - CHEVRON_PT) / 2.0, self.disclosed, LABEL_2.now());
        let title = Rect { x: d.x + SPACE_M + CHEVRON_PT + SPACE_S, ..d };
        draw_text(s, ui, title, "Details", MAC_BODY_PT, Weight::Regular, LABEL.now(), false);
        if self.disclosed {
            if let Ok(rows) = self.panel.rows(self.panel.selected(), self.now) {
                for (k, (label, value)) in rows.iter().enumerate() {
                    let row = self.detail_row(k);
                    let split = row.w * LABEL_COLUMN;
                    let l = Rect { x: row.x + SPACE_M, w: split - SPACE_M - SPACE_S, ..row };
                    let v = Rect { x: row.x + split, w: row.w - split - SPACE_M, ..row };
                    draw_text(s, ui, l, label, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), false);
                    draw_text(s, ui, v, value, MAC_CALLOUT_PT, Weight::Regular, LABEL.now(), false);
                }
            }
        }
        if self.base.focused {
            let f = self.base.frame;
            focus_ring(s, ui, Rect { h: self.content_height().min(f.h.max(ROW_PT)), ..f }, PLATE_RADIUS_PT);
        }
    }

    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        match ev {
            ViewEvent::Pointer(Event::Down(x, y)) => {
                self.pressed = self.hit(*x, *y);
                self.pressed.is_some()
            }
            ViewEvent::Pointer(Event::Move(..)) => self.pressed.is_some(),
            ViewEvent::Pointer(Event::Up(x, y)) => {
                let pressed = self.pressed.take();
                if let (Some(p), Some(h)) = (pressed, self.hit(*x, *y)) {
                    if p == h {
                        self.act(h);
                    }
                }
                pressed.is_some()
            }
            ViewEvent::Cancel => self.pressed.take().is_some(),
            ViewEvent::Key(KeyInput::Code(KEY_UP)) => self.step(-1),
            ViewEvent::Key(KeyInput::Code(KEY_DOWN)) => self.step(1),
            ViewEvent::Key(KeyInput::Code(KEY_RIGHT)) if !self.disclosed => {
                self.toggle();
                true
            }
            ViewEvent::Key(KeyInput::Code(KEY_LEFT)) if self.disclosed => {
                self.toggle();
                true
            }
            k if is_activate(k) => {
                self.toggle();
                true
            }
            _ => false,
        }
    }

    fn ax_node(&self) -> Option<Node> {
        let i = self.panel.selected();
        let cert = &self.panel.chain()[i];
        let value = format!("{}, {}, certificate {} of {}", cert.subject, cert.validity(self.now).text(), i + 1, self.panel.chain().len());
        node(self, Role::List, "Certificate", &value)
    }
}

// ---- Trust settings editor ----

/// What a point can land on in the trust editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrustHit {
    /// Entry index and the setting of the segment.
    Segment(usize, TrustSetting),
    Reset,
}

const TRUST_SETTINGS: [(TrustSetting, &str); 3] =
    [(TrustSetting::UseDefault, "Default"), (TrustSetting::AlwaysTrust, "Always"), (TrustSetting::NeverTrust, "Never")];

/// The trust settings editor (`SFCertificateTrustPanel`): a message, then one row for each certificate of the chain
/// with a Default, Always or Never control, the verdict of the whole chain, and Reset to Defaults.
pub struct TrustEditorView {
    base: ViewBase,
    panel: CertificateTrustPanel,
    /// The row the keys act on.
    cursor: usize,
    pressed: Option<TrustHit>,
}

impl TrustEditorView {
    pub fn new(panel: CertificateTrustPanel) -> Self {
        TrustEditorView { base: ViewBase::default(), panel, cursor: 0, pressed: None }
    }

    pub fn panel(&self) -> &CertificateTrustPanel {
        &self.panel
    }

    /// The model, for the app to change the message or an entry. The view repaints.
    pub fn panel_mut(&mut self) -> &mut CertificateTrustPanel {
        self.base.dirty = true;
        &mut self.panel
    }

    /// The entry the keys act on.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Rows above the entries: the message, and the informative text when there is one.
    fn header_rows(&self) -> usize {
        1 + usize::from(self.panel.informative_text().is_some())
    }

    /// The height the editor needs in points.
    pub fn content_height(&self) -> f32 {
        ROW_PT * (self.header_rows() + self.panel.chain().len() + 2) as f32 + SPACE_S
    }

    fn entry_row(&self, i: usize) -> Rect {
        let f = self.base.frame;
        Rect { x: f.x, y: f.y + ROW_PT * (self.header_rows() + i) as f32, w: f.w, h: ROW_PT }
    }

    fn summary_row(&self) -> Rect {
        let last = self.entry_row(self.panel.chain().len());
        Rect { y: last.y + SPACE_S, ..last }
    }

    fn reset_rect(&self) -> Rect {
        let r = self.summary_row();
        let w = BUTTON_W_PT * 1.5;
        Rect { x: r.x + r.w - SPACE_M - w, y: r.y + ROW_PT, w, h: ROW_PT }
    }

    /// The three-way control of entry `i`.
    fn segments_rect(&self, i: usize) -> Rect {
        let r = self.entry_row(i);
        let w = r.w * SEGMENTS_COLUMN;
        Rect { x: r.x + r.w - SPACE_M - w, y: r.y + SPACE_XS, w, h: r.h - 2.0 * SPACE_XS }
    }

    fn hit(&self, x: f32, y: f32) -> Option<TrustHit> {
        if self.reset_rect().contains(x, y) {
            return Some(TrustHit::Reset);
        }
        for i in 0..self.panel.chain().len() {
            let seg = self.segments_rect(i);
            if seg.contains(x, y) {
                let k = (((x - seg.x) / seg.w * 3.0) as usize).min(2);
                return Some(TrustHit::Segment(i, TRUST_SETTINGS[k].0));
            }
        }
        None
    }

    fn act(&mut self, hit: TrustHit) {
        match hit {
            TrustHit::Segment(i, setting) => {
                self.cursor = i;
                let current = self.panel.chain().get(i).map(|e| e.setting);
                if current != Some(setting) && self.panel.set_setting(i, setting).is_ok() {
                    self.base.dirty = true;
                    self.base.send_action(ACTION_TRUST_SETTING);
                }
            }
            TrustHit::Reset => {
                let changed = self.panel.chain().iter().any(|e| e.setting != TrustSetting::UseDefault);
                self.panel.reset_all();
                if changed {
                    self.base.dirty = true;
                    self.base.send_action(ACTION_TRUST_RESET);
                }
            }
        }
    }

    fn nudge(&mut self, delta: isize) -> bool {
        let Some(entry) = self.panel.chain().get(self.cursor) else { return false };
        let at = TRUST_SETTINGS.iter().position(|(s, _)| *s == entry.setting).unwrap_or(0) as isize;
        let next = at + delta;
        if !(0..3).contains(&next) {
            return true;
        }
        self.act(TrustHit::Segment(self.cursor, TRUST_SETTINGS[next as usize].0));
        true
    }
}

impl View for TrustEditorView {
    crate::view_base!();

    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        (0.0, self.content_height())
    }

    fn accepts_first_responder(&self) -> bool {
        true
    }

    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        let head = Rect { x: f.x + SPACE_M, y: f.y, w: f.w - 2.0 * SPACE_M, h: ROW_PT };
        draw_text(s, ui, head, self.panel.message(), MAC_BODY_PT, Weight::Semibold, LABEL.now(), false);
        if let Some(info) = self.panel.informative_text() {
            draw_text(s, ui, Rect { y: head.y + ROW_PT, ..head }, info, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), false);
        }
        for (i, entry) in self.panel.chain().iter().enumerate() {
            let row = self.entry_row(i);
            let trusted = self.panel.effective_trust(i).unwrap_or(false);
            dot(s, ui, row, row.x + SPACE_M, if trusted { SYSTEM_GREEN } else { SYSTEM_RED });
            let seg = self.segments_rect(i);
            let label = Rect { x: row.x + SPACE_M + DOT_PT + SPACE_S, y: row.y, w: (seg.x - row.x - SPACE_M - DOT_PT - 2.0 * SPACE_S).max(0.0), h: row.h };
            draw_text(s, ui, label, &entry.label, MAC_BODY_PT, Weight::Regular, LABEL.now(), false);
            s.fill_rrect(px(ui, seg), ui.pt(seg.h / 2.0), FIELD_BG.now());
            for (k, (setting, title)) in TRUST_SETTINGS.iter().enumerate() {
                let cell = Rect { x: seg.x + seg.w * k as f32 / 3.0, y: seg.y, w: seg.w / 3.0, h: seg.h };
                if *setting == entry.setting {
                    s.fill_rrect(px(ui, cell), ui.pt(cell.h / 2.0), SEGMENT_ON.now());
                }
                draw_text(s, ui, cell, title, MAC_CALLOUT_PT, Weight::Regular, LABEL.now(), true);
            }
            if self.base.focused && i == self.cursor {
                focus_ring(s, ui, seg, seg.h / 2.0);
            }
        }
        let sum = self.summary_row();
        let trusted = self.panel.chain_trusted();
        dot(s, ui, sum, sum.x + SPACE_M, if trusted { SYSTEM_GREEN } else { SYSTEM_RED });
        let text = if trusted { "This certificate chain is trusted" } else { "This certificate chain is not trusted" };
        draw_text(s, ui, Rect { x: sum.x + SPACE_M + DOT_PT + SPACE_S, w: sum.w - 2.0 * SPACE_M - DOT_PT - SPACE_S, ..sum }, text, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), false);
        let pressed = self.pressed == Some(TrustHit::Reset);
        let any = self.panel.chain().iter().any(|e| e.setting != TrustSetting::UseDefault);
        draw_button(s, ui, self.reset_rect(), "Reset to Defaults", any, pressed);
    }

    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        match ev {
            ViewEvent::Pointer(Event::Down(x, y)) => {
                self.pressed = self.hit(*x, *y);
                if self.pressed.is_some() {
                    self.base.dirty = true;
                }
                self.pressed.is_some()
            }
            ViewEvent::Pointer(Event::Move(..)) => self.pressed.is_some(),
            ViewEvent::Pointer(Event::Up(x, y)) => {
                let pressed = self.pressed.take();
                if let (Some(p), Some(h)) = (pressed, self.hit(*x, *y)) {
                    if p == h {
                        self.act(h);
                    }
                }
                if pressed.is_some() {
                    self.base.dirty = true;
                }
                pressed.is_some()
            }
            ViewEvent::Cancel => {
                let was = self.pressed.take().is_some();
                self.base.dirty |= was;
                was
            }
            ViewEvent::Key(KeyInput::Code(KEY_UP)) => {
                self.cursor = self.cursor.saturating_sub(1);
                self.base.dirty = true;
                true
            }
            ViewEvent::Key(KeyInput::Code(KEY_DOWN)) => {
                self.cursor = (self.cursor + 1).min(self.panel.chain().len() - 1);
                self.base.dirty = true;
                true
            }
            ViewEvent::Key(KeyInput::Code(KEY_LEFT)) => self.nudge(-1),
            ViewEvent::Key(KeyInput::Code(KEY_RIGHT)) => self.nudge(1),
            _ => false,
        }
    }

    fn ax_node(&self) -> Option<Node> {
        let verdict = if self.panel.chain_trusted() { "chain trusted" } else { "chain not trusted" };
        node(self, Role::List, self.panel.message(), verdict)
    }
}

// ---- Choose identity panel ----

/// What a point can land on in the identity panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdentityHit {
    Row(usize),
    Default,
    Alternate,
}

/// The choose-identity panel (`SFChooseIdentityPanel`): the informative text, the identities that passed the policy
/// with one selected, and the alternate and default buttons. Choose sends nothing while no identity is listed.
pub struct IdentityChooserView {
    base: ViewBase,
    chooser: IdentityChooser,
    /// The time validity is judged at, Unix seconds.
    now: i64,
    pressed: Option<IdentityHit>,
}

impl IdentityChooserView {
    pub fn new(chooser: IdentityChooser, now: i64) -> Self {
        IdentityChooserView { base: ViewBase::default(), chooser, now, pressed: None }
    }

    pub fn chooser(&self) -> &IdentityChooser {
        &self.chooser
    }

    /// The model, for the app to change the button titles or the text. The view repaints.
    pub fn chooser_mut(&mut self) -> &mut IdentityChooser {
        self.base.dirty = true;
        &mut self.chooser
    }

    /// Selects a listed identity; no action is sent.
    pub fn select(&mut self, index: usize) -> Result<(), ChooserError> {
        self.chooser.select(index)?;
        self.base.dirty = true;
        Ok(())
    }

    fn header_rows(&self) -> usize {
        usize::from(self.chooser.informative_text().is_some())
    }

    /// The height the panel needs in points. An empty list still has one row for its message.
    pub fn content_height(&self) -> f32 {
        ROW_PT * (self.header_rows() + self.chooser.listed().len().max(1) + 1) as f32 + 2.0 * SPACE_S
    }

    fn list_row(&self, i: usize) -> Rect {
        let f = self.base.frame;
        Rect { x: f.x, y: f.y + ROW_PT * (self.header_rows() + i) as f32, w: f.w, h: ROW_PT }
    }

    fn buttons_y(&self) -> f32 {
        let rows = self.chooser.listed().len().max(1);
        self.base.frame.y + ROW_PT * (self.header_rows() + rows) as f32 + SPACE_S
    }

    fn default_rect(&self) -> Rect {
        let f = self.base.frame;
        Rect { x: f.x + f.w - SPACE_M - BUTTON_W_PT, y: self.buttons_y(), w: BUTTON_W_PT, h: ROW_PT }
    }

    fn alternate_rect(&self) -> Rect {
        let d = self.default_rect();
        Rect { x: d.x - SPACE_S - BUTTON_W_PT, ..d }
    }

    fn can_choose(&self) -> bool {
        self.chooser.confirm().is_ok()
    }

    fn hit(&self, x: f32, y: f32) -> Option<IdentityHit> {
        if self.default_rect().contains(x, y) {
            return Some(IdentityHit::Default);
        }
        if self.alternate_rect().contains(x, y) {
            return Some(IdentityHit::Alternate);
        }
        (0..self.chooser.listed().len()).find(|&i| self.list_row(i).contains(x, y)).map(IdentityHit::Row)
    }

    fn act(&mut self, hit: IdentityHit) {
        match hit {
            IdentityHit::Row(i) => {
                if self.chooser.selected() != Some(i) && self.chooser.select(i).is_ok() {
                    self.base.dirty = true;
                    self.base.send_action(ACTION_IDENTITY_SELECT);
                }
            }
            IdentityHit::Default => {
                if self.can_choose() {
                    self.base.send_action(ACTION_IDENTITY_CHOOSE);
                }
            }
            IdentityHit::Alternate => self.base.send_action(ACTION_IDENTITY_CANCEL),
        }
    }

    fn step(&mut self, delta: isize) -> bool {
        let n = self.chooser.listed().len() as isize;
        if n == 0 {
            return true;
        }
        let at = self.chooser.selected().unwrap_or(0) as isize;
        self.act(IdentityHit::Row((at + delta).clamp(0, n - 1) as usize));
        true
    }
}

impl View for IdentityChooserView {
    crate::view_base!();

    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        (0.0, self.content_height())
    }

    fn accepts_first_responder(&self) -> bool {
        true
    }

    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        if let Some(info) = self.chooser.informative_text() {
            let head = Rect { x: f.x + SPACE_M, y: f.y, w: f.w - 2.0 * SPACE_M, h: ROW_PT };
            draw_text(s, ui, head, info, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), false);
        }
        if self.chooser.listed().is_empty() {
            let row = self.list_row(0);
            draw_text(s, ui, Rect { x: row.x + SPACE_M, w: row.w - 2.0 * SPACE_M, ..row }, "No identity is available", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), false);
        }
        for (i, id) in self.chooser.listed().iter().enumerate() {
            let row = self.list_row(i);
            let selected = self.chooser.selected() == Some(i);
            if selected {
                s.fill_rrect(px(ui, row), ui.pt(PLATE_RADIUS_PT), SELECTION.now());
            }
            let validity = id.certificate.validity(self.now);
            dot(s, ui, row, row.x + SPACE_M, validity_color(validity));
            let (main, side) = if selected { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
            let text = Rect { x: row.x + SPACE_M + DOT_PT + SPACE_S, y: row.y, w: (row.w - DOT_PT - 2.0 * SPACE_S - SPACE_L * 6.0).max(0.0), h: row.h };
            draw_text(s, ui, text, &id.certificate.subject, MAC_BODY_PT, Weight::Regular, main, false);
            let note = if id.has_private_key { validity.text() } else { "No private key" };
            draw_trailing(s, ui, row, note, side);
        }
        let pressed = self.pressed;
        draw_button(s, ui, self.alternate_rect(), self.chooser.alternate_button_title(), true, pressed == Some(IdentityHit::Alternate));
        draw_button(s, ui, self.default_rect(), self.chooser.default_button_title(), self.can_choose(), pressed == Some(IdentityHit::Default));
        if self.base.focused {
            let r = Rect { h: self.content_height().min(f.h.max(ROW_PT)), ..f };
            focus_ring(s, ui, r, PLATE_RADIUS_PT);
        }
    }

    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        match ev {
            ViewEvent::Pointer(Event::Down(x, y)) => {
                self.pressed = self.hit(*x, *y);
                if self.pressed.is_some() {
                    self.base.dirty = true;
                }
                self.pressed.is_some()
            }
            ViewEvent::Pointer(Event::Move(..)) => self.pressed.is_some(),
            ViewEvent::Pointer(Event::Up(x, y)) => {
                let pressed = self.pressed.take();
                if let (Some(p), Some(h)) = (pressed, self.hit(*x, *y)) {
                    if p == h {
                        self.act(h);
                    }
                }
                if pressed.is_some() {
                    self.base.dirty = true;
                }
                pressed.is_some()
            }
            ViewEvent::Cancel => {
                let was = self.pressed.take().is_some();
                self.base.dirty |= was;
                was
            }
            ViewEvent::Key(KeyInput::Code(KEY_UP)) => self.step(-1),
            ViewEvent::Key(KeyInput::Code(KEY_DOWN)) => self.step(1),
            ViewEvent::Key(KeyInput::Code(KEY_RETURN)) => {
                self.act(IdentityHit::Default);
                true
            }
            ViewEvent::Key(KeyInput::Code(KEY_ESCAPE)) => {
                self.act(IdentityHit::Alternate);
                true
            }
            _ => false,
        }
    }

    fn ax_node(&self) -> Option<Node> {
        let value = match self.chooser.confirm() {
            Ok(id) => format!("{} selected", id.certificate.subject),
            Err(_) => "no identity available".to_string(),
        };
        node(self, Role::List, "Choose an identity", &value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{Font, Surface};
    use bifrost_ui::certificate_panel::CertificateInfo;
    use bifrost_ui::certificate_trust::ChainEntry;
    use bifrost_ui::identity_chooser::{Identity, IdentityPolicy};
    use bifrost_ui::responder::Responder;

    use super::super::Window;

    const NOW: i64 = 1_000;

    fn ui() -> Ui {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(Font::parse(data).unwrap(), 1.0)
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[derive(Default)]
    struct Model {
        log: Vec<String>,
    }
    impl Responder for Model {
        fn perform(&mut self, action: &str) -> bool {
            self.log.push(action.into());
            true
        }
    }

    fn cert(subject: &str, not_before: i64, not_after: i64) -> CertificateInfo {
        CertificateInfo { subject: subject.into(), issuer: "CN=Root".into(), serial: "01AF".into(), not_before, not_after }
    }

    fn click(w: &mut Window<Model>, m: &mut Model, x: f32, y: f32) {
        w.event(&ViewEvent::Pointer(Event::Down(x, y)), m);
        w.event(&ViewEvent::Pointer(Event::Up(x, y)), m);
    }

    fn show<V: View>(v: V) -> (Window<Model>, u64) {
        let id = v.id();
        let mut w: Window<Model> = Window::new(v, 1);
        w.layout(&ui(), rect(0.0, 0.0, 400.0, 300.0));
        (w, id)
    }

    fn pixels(w: &mut Window<Model>) -> Vec<u32> {
        let mut px = vec![0u32; 400 * 300];
        let mut s = Surface { w: 400, h: 300, stride: 400, px: &mut px, y0: 0 };
        w.draw(&mut s, &ui());
        px
    }

    fn chain_panel() -> CertificatePanel {
        CertificatePanel::new(vec![cert("CN=leaf.example", 0, 2_000), cert("CN=Intermediate", 0, 500), cert("CN=Root", 0, 9_000)]).unwrap()
    }

    // ---- certificate panel ----

    #[test]
    fn a_click_on_a_chain_row_selects_that_certificate_and_sends_one_action() {
        let (mut w, id) = show(CertificatePanelView::new(chain_panel(), NOW));
        let mut m = Model::default();
        click(&mut w, &mut m, 100.0, ROW_PT * 1.5);
        assert_eq!(w.view::<CertificatePanelView>(id).unwrap().panel().selected(), 1);
        assert_eq!(m.log, [ACTION_CERTIFICATE_SELECT]);
        click(&mut w, &mut m, 100.0, ROW_PT * 1.5);
        assert_eq!(m.log.len(), 1, "selecting the selected certificate again changes nothing");
    }

    #[test]
    fn the_details_disclosure_opens_the_rows_of_the_selected_certificate() {
        let (mut w, id) = show(CertificatePanelView::new(chain_panel(), NOW));
        let mut m = Model::default();
        let v = w.view::<CertificatePanelView>(id).unwrap();
        assert!(!v.is_disclosed());
        let closed = v.content_height();
        let d = v.disclosure_row();
        click(&mut w, &mut m, d.x + 20.0, d.y + d.h / 2.0);
        let v = w.view::<CertificatePanelView>(id).unwrap();
        assert!(v.is_disclosed());
        assert_eq!(v.content_height(), closed + 4.0 * ROW_PT, "subject, issuer, serial number and status");
        assert_eq!(m.log, [ACTION_CERTIFICATE_DISCLOSURE]);
        click(&mut w, &mut m, d.x + 20.0, d.y + d.h / 2.0);
        assert!(!w.view::<CertificatePanelView>(id).unwrap().is_disclosed());
    }

    #[test]
    fn arrow_keys_move_the_selection_and_stop_at_the_ends() {
        let (mut w, id) = show(CertificatePanelView::new(chain_panel(), NOW));
        let mut m = Model::default();
        w.make_first_responder(Some(id));
        for _ in 0..5 {
            w.event(&ViewEvent::Key(KeyInput::Code(KEY_DOWN)), &mut m);
        }
        assert_eq!(w.view::<CertificatePanelView>(id).unwrap().panel().selected(), 2);
        w.event(&ViewEvent::Key(KeyInput::Code(KEY_UP)), &mut m);
        assert_eq!(w.view::<CertificatePanelView>(id).unwrap().panel().selected(), 1);
        assert_eq!(m.log.len(), 3, "two moves down, one up; the presses at the end send nothing");
        w.event(&ViewEvent::Key(KeyInput::Code(KEY_RIGHT)), &mut m);
        assert!(w.view::<CertificatePanelView>(id).unwrap().is_disclosed());
        w.event(&ViewEvent::Key(KeyInput::Code(KEY_LEFT)), &mut m);
        assert!(!w.view::<CertificatePanelView>(id).unwrap().is_disclosed());
    }

    #[test]
    fn the_dot_beside_each_certificate_shows_its_validity() {
        let (mut w, _) = show(CertificatePanelView::new(chain_panel(), NOW));
        let px = pixels(&mut w);
        let at = |row: usize, indent: f32| px[(ROW_PT * row as f32 + ROW_PT / 2.0) as usize * 400 + (indent + DOT_PT / 2.0) as usize];
        assert_eq!(at(0, SPACE_M), SYSTEM_GREEN, "the leaf is valid");
        assert_eq!(at(1, 2.0 * SPACE_M), SYSTEM_RED, "the intermediate expired at 500");
    }

    #[test]
    fn disclosed_details_draw_without_panicking_and_the_view_reports_its_state_to_assistive_technology() {
        let mut v = CertificatePanelView::new(chain_panel(), NOW);
        v.set_disclosed(true);
        let (mut w, id) = show(v);
        pixels(&mut w);
        let n = w.view::<CertificatePanelView>(id).unwrap().ax_node().unwrap();
        assert!(n.value.contains("CN=leaf.example") && n.value.contains("Valid") && n.value.contains("1 of 3"), "{}", n.value);
    }

    #[test]
    fn a_bad_index_is_refused_and_a_time_change_repaints() {
        let mut v = CertificatePanelView::new(chain_panel(), NOW);
        assert_eq!(v.select(7), Err(PanelError::BadIndex(7)));
        assert_eq!(v.panel().selected(), 0);
        v.base.dirty = false;
        v.set_now(NOW);
        assert!(!v.needs_display());
        v.set_now(NOW + 1);
        assert!(v.needs_display());
    }

    // ---- trust editor ----

    fn trust_panel(default_trusted: bool) -> CertificateTrustPanel {
        let entry = |label: &str| ChainEntry { label: label.into(), default_trusted, setting: TrustSetting::UseDefault };
        CertificateTrustPanel::new(vec![entry("leaf.example"), entry("Root")], "Trust leaf.example?").unwrap()
    }

    fn segment_centre(v: &TrustEditorView, entry: usize, k: usize) -> (f32, f32) {
        let seg = v.segments_rect(entry);
        (seg.x + seg.w * (k as f32 + 0.5) / 3.0, seg.y + seg.h / 2.0)
    }

    #[test]
    fn a_click_on_a_segment_sets_that_entrys_trust_and_the_chain_verdict_follows() {
        let (mut w, id) = show(TrustEditorView::new(trust_panel(false)));
        let mut m = Model::default();
        assert!(!w.view::<TrustEditorView>(id).unwrap().panel().chain_trusted());
        for entry in 0..2 {
            let (x, y) = segment_centre(w.view::<TrustEditorView>(id).unwrap(), entry, 1);
            click(&mut w, &mut m, x, y);
        }
        let v = w.view::<TrustEditorView>(id).unwrap();
        assert!(v.panel().chain_trusted(), "Always trust on both rescues an untrusted chain");
        assert_eq!(m.log, [ACTION_TRUST_SETTING, ACTION_TRUST_SETTING]);
        let (x, y) = segment_centre(v, 1, 2);
        click(&mut w, &mut m, x, y);
        let v = w.view::<TrustEditorView>(id).unwrap();
        assert_eq!(v.panel().chain()[1].setting, TrustSetting::NeverTrust);
        assert!(!v.panel().chain_trusted());
    }

    #[test]
    fn reset_returns_every_entry_to_its_default_and_is_silent_when_nothing_changed() {
        let (mut w, id) = show(TrustEditorView::new(trust_panel(true)));
        let mut m = Model::default();
        let r = w.view::<TrustEditorView>(id).unwrap().reset_rect();
        click(&mut w, &mut m, r.x + 5.0, r.y + r.h / 2.0);
        assert!(m.log.is_empty(), "nothing to reset");
        let (x, y) = segment_centre(w.view::<TrustEditorView>(id).unwrap(), 0, 2);
        click(&mut w, &mut m, x, y);
        click(&mut w, &mut m, r.x + 5.0, r.y + r.h / 2.0);
        let v = w.view::<TrustEditorView>(id).unwrap();
        assert!(v.panel().chain().iter().all(|e| e.setting == TrustSetting::UseDefault));
        assert_eq!(m.log, [ACTION_TRUST_SETTING, ACTION_TRUST_RESET]);
    }

    #[test]
    fn left_and_right_change_the_setting_of_the_cursor_row_and_up_down_move_the_cursor() {
        let (mut w, id) = show(TrustEditorView::new(trust_panel(true)));
        let mut m = Model::default();
        w.make_first_responder(Some(id));
        w.event(&ViewEvent::Key(KeyInput::Code(KEY_DOWN)), &mut m);
        w.event(&ViewEvent::Key(KeyInput::Code(KEY_RIGHT)), &mut m);
        let v = w.view::<TrustEditorView>(id).unwrap();
        assert_eq!((v.cursor(), v.panel().chain()[1].setting), (1, TrustSetting::AlwaysTrust));
        w.event(&ViewEvent::Key(KeyInput::Code(KEY_RIGHT)), &mut m);
        w.event(&ViewEvent::Key(KeyInput::Code(KEY_RIGHT)), &mut m);
        assert_eq!(w.view::<TrustEditorView>(id).unwrap().panel().chain()[1].setting, TrustSetting::NeverTrust, "stops at the last setting");
        assert_eq!(m.log.len(), 2);
    }

    #[test]
    fn the_editor_draws_the_message_the_rows_and_the_verdict() {
        let mut p = trust_panel(false);
        p.set_informative_text(Some("The server did not prove who it is."));
        let (mut w, id) = show(TrustEditorView::new(p));
        let px = pixels(&mut w);
        assert!(px.iter().any(|&p| p != 0), "something was drawn");
        let v = w.view::<TrustEditorView>(id).unwrap();
        assert_eq!(v.content_height(), ROW_PT * 6.0 + SPACE_S);
        assert_eq!(v.ax_node().unwrap().value, "chain not trusted");
    }

    // ---- identity chooser ----

    fn identity(subject: &str, key: bool, not_after: i64) -> Identity {
        Identity { certificate: cert(subject, 0, not_after), has_private_key: key }
    }

    fn chooser() -> IdentityChooser {
        IdentityChooser::new(
            vec![identity("CN=Zoe", true, 5_000), identity("CN=Ana", true, 5_000), identity("CN=Old", true, 10), identity("CN=NoKey", false, 5_000)],
            IdentityPolicy { require_private_key: true, allow_expired: false },
            NOW,
        )
    }

    #[test]
    fn only_the_identities_that_pass_the_policy_are_listed_and_the_first_is_selected() {
        let (w, id) = show(IdentityChooserView::new(chooser(), NOW));
        let v = w.view::<IdentityChooserView>(id).unwrap();
        let subjects: Vec<&str> = v.chooser().listed().iter().map(|i| i.certificate.subject.as_str()).collect();
        assert_eq!(subjects, ["CN=Ana", "CN=Zoe"]);
        assert_eq!(v.chooser().selected(), Some(0));
    }

    #[test]
    fn selecting_a_row_and_pressing_choose_sends_the_actions_and_the_model_names_the_identity() {
        let (mut w, id) = show(IdentityChooserView::new(chooser(), NOW));
        let mut m = Model::default();
        click(&mut w, &mut m, 100.0, ROW_PT * 1.5);
        let v = w.view::<IdentityChooserView>(id).unwrap();
        assert_eq!(v.chooser().confirm().unwrap().certificate.subject, "CN=Zoe");
        let d = v.default_rect();
        click(&mut w, &mut m, d.x + d.w / 2.0, d.y + d.h / 2.0);
        assert_eq!(m.log, [ACTION_IDENTITY_SELECT, ACTION_IDENTITY_CHOOSE]);
    }

    #[test]
    fn cancel_and_escape_send_cancel_and_return_sends_choose() {
        let (mut w, id) = show(IdentityChooserView::new(chooser(), NOW));
        let mut m = Model::default();
        let a = w.view::<IdentityChooserView>(id).unwrap().alternate_rect();
        click(&mut w, &mut m, a.x + a.w / 2.0, a.y + a.h / 2.0);
        w.make_first_responder(Some(id));
        w.event(&ViewEvent::Key(KeyInput::Code(KEY_ESCAPE)), &mut m);
        w.event(&ViewEvent::Key(KeyInput::Code(KEY_RETURN)), &mut m);
        assert_eq!(m.log, [ACTION_IDENTITY_CANCEL, ACTION_IDENTITY_CANCEL, ACTION_IDENTITY_CHOOSE]);
    }

    #[test]
    fn with_nothing_listed_choose_does_nothing_and_cancel_still_works() {
        let none = IdentityChooser::new(vec![identity("CN=Old", true, 10)], IdentityPolicy { require_private_key: true, allow_expired: false }, NOW);
        let (mut w, id) = show(IdentityChooserView::new(none, NOW));
        let mut m = Model::default();
        w.make_first_responder(Some(id));
        w.event(&ViewEvent::Key(KeyInput::Code(KEY_RETURN)), &mut m);
        w.event(&ViewEvent::Key(KeyInput::Code(KEY_DOWN)), &mut m);
        assert!(m.log.is_empty());
        let v = w.view::<IdentityChooserView>(id).unwrap();
        assert_eq!(v.ax_node().unwrap().value, "no identity available");
        let a = v.alternate_rect();
        click(&mut w, &mut m, a.x + 2.0, a.y + 2.0);
        assert_eq!(m.log, [ACTION_IDENTITY_CANCEL]);
        pixels(&mut w);
    }

    #[test]
    fn arrow_keys_move_the_identity_selection() {
        let (mut w, id) = show(IdentityChooserView::new(chooser(), NOW));
        let mut m = Model::default();
        w.make_first_responder(Some(id));
        w.event(&ViewEvent::Key(KeyInput::Code(KEY_DOWN)), &mut m);
        assert_eq!(w.view::<IdentityChooserView>(id).unwrap().chooser().selected(), Some(1));
        w.event(&ViewEvent::Key(KeyInput::Code(KEY_DOWN)), &mut m);
        assert_eq!(w.view::<IdentityChooserView>(id).unwrap().chooser().selected(), Some(1), "stops at the last row");
        assert_eq!(m.log, [ACTION_IDENTITY_SELECT]);
    }
}
