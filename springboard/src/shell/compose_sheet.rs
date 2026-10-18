//! The compose post sheet: the share menu's "Compose Post" target opens it with the shared items as the draft.
//!
//! The draft is the framework model `bifrost_ui::compose::ComposeSheet` (`docs/parity/social.md`): text parts become the
//! post text, files become attachments, web links become links. Anything else (file links, other schemes) is not part of
//! a post. The sheet shows the character count against [`CHARACTER_LIMIT`] and the attachment and link counts. Posting
//! has no network behind it, so "Post" hands the post text, attachments and links to the clipboard and reports `done`
//! to the sender; "Cancel" reports `cancelled`.
//!
//! Typing edits the post text: the keyboard's text goes to the sheet (`Shell::compose_type`), which rebuilds its draft
//! with the new text and keeps the attachments and links. Gaps: the draft is not posted through
//! `social::draft_request` (no social account is signed in), so nothing is sent to a social network.
//!
//! Like `power_menu`, this is pure state plus drawing; the shell routes touches to it and performs the hand-off.

use bifrost_proto::Payload;
use bifrost_ui::compose::{ComposeItem, ComposeResult, ComposeSheet};

use super::ui::{Ui, Weight, CONTENT_BG, FIELD_BG, LABEL, LABEL_2, LABEL_3, LINK, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_TITLE2_PT, WHITE};
use super::keyboard::Out;
use super::Event;
use crate::gfx::{rgba, Image, Surface};

/// Characters allowed in a post. The framework documents no limit for the compose sheet; this is BifrOSt's choice, the
/// length of a short network post.
pub const CHARACTER_LIMIT: usize = 280;
/// How much of the text the sheet previews.
const PREVIEW_CHARS: usize = 60;

const WIDTH_PT: f32 = 420.0;
const HEIGHT_PT: f32 = 260.0;
const PAD_PT: f32 = 20.0;
const GAP_PT: f32 = 12.0;
const BUTTON_H_PT: f32 = 40.0;
const RADIUS_PT: f32 = 13.0;
const BUTTON_RADIUS_PT: f32 = 9.0;

/// Characters in `text`.
pub fn characters(text: &str) -> usize {
    text.chars().count()
}

/// Characters left for `text`; negative when the text is over the limit.
pub fn remaining(text: &str) -> i64 {
    CHARACTER_LIMIT as i64 - characters(text) as i64
}

/// The sheet for a share: text parts join with newlines, files are attachments, and web links are links.
pub fn sheet_for(items: &[Payload]) -> ComposeSheet {
    let mut sheet = ComposeSheet::new();
    let text: Vec<&str> = items
        .iter()
        .filter_map(|i| match i {
            Payload::Text(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    // a fresh sheet is editing, so these cannot be refused; the model's own caps drop extras
    let _ = sheet.set_initial_text(&text.join("\n"));
    for item in items {
        let _ = match item {
            Payload::File { name, .. } => sheet.add_image(name),
            Payload::Url(u) => sheet.add_url(u),
            Payload::Text(_) => Ok(()),
        };
    }
    sheet.present();
    sheet
}

/// The post as handed to the clipboard: text, attachment names and links, one per line, in the model's order.
pub fn hand_off_text(sheet: &ComposeSheet) -> String {
    sheet
        .draft()
        .items()
        .into_iter()
        .map(|i| match i {
            ComposeItem::Text(s) | ComposeItem::Url(s) => s,
            ComposeItem::Image(name) => name,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The first line of `text`, cut to `PREVIEW_CHARS` with an ellipsis.
fn preview(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    if characters(first) > PREVIEW_CHARS {
        let cut: String = first.chars().take(PREVIEW_CHARS).collect();
        format!("{cut}…")
    } else {
        first.to_string()
    }
}

/// What the sheet reports after a touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Idle,
    Redraw,
    /// The user closed the sheet: `true` for Post, `false` for Cancel.
    Finish(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Cancel,
    Post,
}

type Rect = (f32, f32, f32, f32);

fn inside((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

fn centre((x, y, w, h): Rect) -> (f32, f32) {
    (x + w / 2.0, y + h / 2.0)
}

/// Where everything sits for a `w` x `h` screen.
struct Layout {
    sheet: Rect,
    cancel: Rect,
    post: Rect,
}

fn layout(ui: &Ui, w: i32, h: i32) -> Layout {
    let sw = ui.pt(WIDTH_PT).min(w as f32 - ui.pt(32.0));
    let sh = ui.pt(HEIGHT_PT).min(h as f32 - ui.pt(32.0));
    let sx = ((w as f32 - sw) / 2.0).round();
    let sy = ((h as f32 - sh) / 2.0).round();
    let pad = ui.pt(PAD_PT);
    let bh = ui.pt(BUTTON_H_PT);
    let by = sy + sh - pad - bh;
    let half = (sw - 2.0 * pad - ui.pt(GAP_PT)) / 2.0;
    Layout { sheet: (sx, sy, sw, sh), cancel: (sx + pad, by, half, bh), post: (sx + sw - pad - half, by, half, bh) }
}

pub struct ComposeOverlay {
    /// the app that shared the items, and its request id (for `ToApp::ShareDone`)
    pub slot: u8,
    pub request: u32,
    /// the shared items as they came in, rebuilt into the draft when the text changes
    items: Vec<Payload>,
    sheet: ComposeSheet,
    pressed: Option<Button>,
    title: Image,
    preview: Image,
    attachments: Image,
    counter: Image,
    counter_over: Image,
    cancel: Image,
    post: Image,
    post_off: Image,
}

impl ComposeOverlay {
    pub fn new(ui: &Ui, slot: u8, request: u32, items: &[Payload]) -> Self {
        let sheet = sheet_for(items);
        let draft = sheet.draft();
        let text = draft.text();
        let (images, links) = (draft.images().len(), draft.urls().len());
        let attachments = format!(
            "{} attachment{} · {} link{}",
            images,
            if images == 1 { "" } else { "s" },
            links,
            if links == 1 { "" } else { "s" }
        );
        let left = remaining(text);
        let first_line = preview(text);
        ComposeOverlay {
            slot,
            request,
            items: items.to_vec(),
            pressed: None,
            title: ui.text("Compose Post", MAC_TITLE2_PT, Weight::Semibold, LABEL.now()),
            preview: ui.text(&first_line, MAC_BODY_PT, Weight::Regular, LABEL.now()),
            attachments: ui.text(&attachments, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now()),
            counter: ui.text(&format!("{left} characters left"), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now()),
            counter_over: ui.text(&format!("{} over the limit", -left), MAC_FOOTNOTE_PT, Weight::Semibold, LABEL.now()),
            cancel: ui.text("Cancel", MAC_BODY_PT, Weight::Semibold, LINK),
            post: ui.text("Post", MAC_BODY_PT, Weight::Semibold, WHITE),
            post_off: ui.text("Post", MAC_BODY_PT, Weight::Semibold, LABEL_3.now()),
            sheet,
        }
    }

    /// The post text as typed so far.
    pub fn text(&self) -> &str {
        self.sheet.draft().text()
    }

    /// Replaces the post text and rebuilds the sheet from the same items, so the attachments and links stay.
    pub fn set_text(&mut self, ui: &Ui, text: &str) {
        let items = with_text(&self.items, text);
        let (slot, request) = (self.slot, self.request);
        *self = ComposeOverlay::new(ui, slot, request, &items);
    }

    /// Post is allowed for a non-empty post within the limit.
    pub fn can_post(&self) -> bool {
        let text = self.sheet.draft().text();
        !self.sheet.draft().is_empty() && remaining(text) >= 0
    }

    /// Closes the sheet with the model's result: `Some(Done)` for a sent post with content.
    pub fn finish(&mut self, send: bool) -> Option<ComposeResult> {
        self.sheet.finish(send)
    }

    fn enabled(&self, b: Button) -> bool {
        match b {
            Button::Cancel => true,
            Button::Post => self.can_post(),
        }
    }

    fn button_at(l: &Layout, x: f32, y: f32) -> Option<Button> {
        [(Button::Cancel, l.cancel), (Button::Post, l.post)].into_iter().find(|(_, r)| inside(*r, x, y)).map(|(b, _)| b)
    }

    /// Feeds a touch (screen px). Only the two buttons do anything; a disabled Post ignores taps.
    pub fn event(&mut self, ui: &Ui, w: i32, h: i32, ev: Event) -> Outcome {
        let l = layout(ui, w, h);
        match ev {
            Event::Down(x, y) => {
                let pressed = Self::button_at(&l, x, y).filter(|b| self.enabled(*b));
                self.pressed = pressed;
                Outcome::Redraw
            }
            Event::Up(x, y) => {
                let pressed = self.pressed.take();
                match pressed.filter(|b| Self::button_at(&l, x, y) == Some(*b)) {
                    Some(Button::Cancel) => Outcome::Finish(false),
                    Some(Button::Post) => Outcome::Finish(true),
                    None => Outcome::Redraw,
                }
            }
            _ => Outcome::Idle,
        }
    }

    /// The post text, attachments and links for the clipboard on Post.
    pub fn hand_off(&self) -> String {
        hand_off_text(&self.sheet)
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, rgba(0, 0, 0, 120));
        let l = layout(ui, w, h);
        let (sx, sy, sw, sh) = l.sheet;
        let pad = ui.pt(PAD_PT);
        let gap = ui.pt(GAP_PT) / 2.0;
        s.fill_rrect(sx, sy, sw, sh, ui.pt(RADIUS_PT), CONTENT_BG.now());
        let mut y = sy + pad;
        s.blit(&self.title, (sx + pad) as i32, y as i32, 255, 255);
        y += self.title.h as f32 + gap * 2.0;
        s.blit(&self.preview, (sx + pad) as i32, y as i32, 255, 255);
        y += self.preview.h as f32 + gap;
        s.blit(&self.attachments, (sx + pad) as i32, y as i32, 255, 255);
        y += self.attachments.h as f32 + gap;
        let left = remaining(self.sheet.draft().text());
        let counter = if left < 0 { &self.counter_over } else { &self.counter };
        s.blit(counter, (sx + pad) as i32, y as i32, 255, 255);

        let (cx, cy, cw, ch) = l.cancel;
        let cancel_fill = if self.pressed == Some(Button::Cancel) { LABEL_3.now() } else { FIELD_BG.now() };
        s.fill_rrect(cx, cy, cw, ch, ui.pt(BUTTON_RADIUS_PT), cancel_fill);
        centre_label(s, &self.cancel, centre(l.cancel));

        let (px, py, pw, ph) = l.post;
        let on = self.can_post();
        let post_fill = if !on {
            FIELD_BG.now()
        } else if self.pressed == Some(Button::Post) {
            LABEL_3.now()
        } else {
            LINK
        };
        s.fill_rrect(px, py, pw, ph, ui.pt(BUTTON_RADIUS_PT), post_fill);
        centre_label(s, if on { &self.post } else { &self.post_off }, centre(l.post));
    }
}

fn centre_label(s: &mut Surface, img: &Image, (cx, cy): (f32, f32)) {
    s.blit(img, (cx - img.w as f32 / 2.0) as i32, (cy - img.h as f32 / 2.0) as i32, 255, 255);
}

/// The shared items with their text replaced by `text` (first, as the post text). Attachments and links keep their
/// order; an empty `text` removes the text.
pub fn with_text(items: &[Payload], text: &str) -> Vec<Payload> {
    let mut out: Vec<Payload> = items.iter().filter(|p| !matches!(p, Payload::Text(_))).cloned().collect();
    if !text.is_empty() {
        out.insert(0, Payload::Text(text.to_string()));
    }
    out
}

/// Shell side: typing while the compose sheet is open. Text is appended, Backspace removes the last character;
/// other keys do nothing here.
impl super::Shell {
    pub(super) fn compose_type(&mut self, out: Out) {
        let Some(overlay) = self.compose.as_mut() else { return };
        let mut text = overlay.text().to_string();
        match out {
            Out::Text(typed) => text.push_str(&typed),
            Out::Backspace => {
                text.pop();
            }
            _ => return,
        }
        overlay.set_text(&self.ui, &text);
        self.drawn = super::Drawn::Nothing;
        self.dirty = true;
    }
}

/// Shell side: the user closed the compose sheet. Post copies the post to the clipboard and reports `compose-post`
/// to the sender as done; Cancel reports an empty target.
impl super::Shell {
    pub(super) fn compose_done(&mut self, send: bool) {
        let Some(mut overlay) = self.compose.take() else { return };
        let done = overlay.finish(send) == Some(ComposeResult::Done);
        if done {
            super::clipboard::set_text(&overlay.hand_off());
        }
        let target = if done { super::sharing::COMPOSE } else { "" };
        self.share_finish(overlay.slot, overlay.request, target);
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Payload {
        Payload::Text(s.into())
    }

    #[test]
    fn typing_replaces_the_text_and_keeps_the_attachments_and_links() {
        let items = vec![text("old"), Payload::Url("https://ruv.is".into()), text("second")];
        assert_eq!(with_text(&items, "new post"), vec![Payload::Text("new post".into()), Payload::Url("https://ruv.is".into())]);
        assert_eq!(with_text(&items, ""), vec![Payload::Url("https://ruv.is".into())]);
    }

    #[test]
    fn text_parts_join_files_attach_and_web_links_only_are_kept() {
        let items = [
            text("Hello"),
            text("world"),
            Payload::File { grant: [1; 16], name: "a.png".into() },
            Payload::Url("https://example.com".into()),
            Payload::Url("file:///home/a/b.png".into()),
        ];
        let sheet = sheet_for(&items);
        assert_eq!(sheet.draft().text(), "Hello\nworld");
        assert_eq!(sheet.draft().images(), ["a.png"]);
        assert_eq!(sheet.draft().urls(), ["https://example.com"]);
        assert_eq!(sheet.state(), bifrost_ui::compose::SheetState::Presented);
    }

    #[test]
    fn the_limit_counts_characters_not_bytes() {
        assert_eq!(characters("Hello\nworld"), 11);
        assert_eq!(remaining("Hello\nworld"), 269);
        assert_eq!(characters("日本語"), 3);
        assert_eq!(remaining(&"a".repeat(CHARACTER_LIMIT)), 0);
        assert_eq!(remaining(&"a".repeat(CHARACTER_LIMIT + 1)), -1);
    }

    #[test]
    fn the_hand_off_is_text_then_attachments_then_links() {
        let items = [text("Hello"), text("world"), Payload::File { grant: [1; 16], name: "a.png".into() }, Payload::Url("https://example.com".into())];
        assert_eq!(hand_off_text(&sheet_for(&items)), "Hello\nworld\na.png\nhttps://example.com");
        assert_eq!(hand_off_text(&sheet_for(&[])), "");
    }

    #[test]
    fn previews_are_cut_to_the_first_line() {
        assert_eq!(preview("one\ntwo"), "one");
        assert_eq!(preview(&"b".repeat(61)), format!("{}…", "b".repeat(60)));
        assert_eq!(preview(&"b".repeat(60)), "b".repeat(60));
    }

    #[test]
    fn an_empty_post_cannot_be_posted_and_over_the_limit_cannot_either() {
        let ui = crate::shell::tests::ui();
        let empty = ComposeOverlay::new(&ui, 3, 9, &[]);
        assert!(!empty.can_post());
        let over = ComposeOverlay::new(&ui, 3, 9, &[text(&"a".repeat(CHARACTER_LIMIT + 1))]);
        assert!(!over.can_post());
        let ok = ComposeOverlay::new(&ui, 3, 9, &[text("hi")]);
        assert!(ok.can_post());
    }

    #[test]
    fn post_is_refused_when_empty_and_cancel_closes_with_false() {
        let ui = crate::shell::tests::ui();
        let (w, h) = (1200, 768);
        let mut o = ComposeOverlay::new(&ui, 3, 9, &[]);
        let l = layout(&ui, w, h);
        let (px, py) = centre(l.post);
        assert_eq!(o.event(&ui, w, h, Event::Down(px, py)), Outcome::Redraw);
        assert_eq!(o.event(&ui, w, h, Event::Up(px, py)), Outcome::Redraw);
        let (cx, cy) = centre(l.cancel);
        assert_eq!(o.event(&ui, w, h, Event::Down(cx, cy)), Outcome::Redraw);
        assert_eq!(o.event(&ui, w, h, Event::Up(cx, cy)), Outcome::Finish(false));
    }

    #[test]
    fn post_with_text_finishes_with_true_and_reports_done() {
        let ui = crate::shell::tests::ui();
        let (w, h) = (1200, 768);
        let mut o = ComposeOverlay::new(&ui, 3, 9, &[text("hi")]);
        let (px, py) = centre(layout(&ui, w, h).post);
        o.event(&ui, w, h, Event::Down(px, py));
        assert_eq!(o.event(&ui, w, h, Event::Up(px, py)), Outcome::Finish(true));
        assert_eq!(o.finish(true), Some(ComposeResult::Done));
        assert_eq!(o.hand_off(), "hi");
    }
}
