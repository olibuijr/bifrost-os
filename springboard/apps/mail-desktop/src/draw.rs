//! The Mail window's drawing and hit-testing. Every size is in points and goes through `ui.pt`; the three panes come from
//! [`crate::layout`], the message list and the reader scroll, and the sheets are panels centred over the window. The
//! layout functions here are shared by the painter and the hit test, so a tap lands on what is drawn.

use crate::attachments;
use crate::findbar;
use crate::flags::{Flag, FlagColor};
use crate::layout;
use crate::message::Priority;
use crate::prefs::Favorite;
use crate::richtext::{self, Style, TextColor};
use crate::state::{is_multiline, Compose, Filter, Mail, Sheet, ARCHIVE_NAMES, INBOX, JUNK_NAMES, MAX_CHOICES, MAX_SUGGESTIONS, TRASH_NAMES};
use crate::window::{MailWindow, Region};
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::nav::{Toolbar, ToolbarItem};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, FIELD_BG, LABEL, LABEL_2, LABEL_3, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_TITLE1_PT, SELECTION, SEPARATOR_C, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXS, WHITE};
use bifrost_app::kit::ui::system::{FIND_HIGHLIGHT, SYSTEM_ORANGE};
use bifrost_app::kit::{colors, Image, Surface, TextCache, Ui, Weight};
use bifrost_app::layout::traffic_lights_trailing;
use bifrost_app::widgets;

pub const TOOLBAR_PT: f32 = 52.0;
const BUTTON_H_PT: f32 = 28.0;
const BUTTON_W_PT: f32 = 96.0;
const SIDE_HEAD_PT: f32 = 30.0;
const SIDE_ROW_PT: f32 = 26.0;
const ROW_PT: f32 = 58.0;
pub(crate) const READER_PAD_PT: f32 = 20.0;
pub(crate) const BODY_LINE_PT: f32 = 18.0;
pub(crate) const FOOT_LINE_PT: f32 = 15.0;
pub(crate) const TITLE_LINE_PT: f32 = 28.0;
const SHEET_W_PT: f32 = 440.0;
const SHEET_HEAD_PT: f32 = 44.0;
const SHEET_FOOT_PT: f32 = 60.0;
const SHEET_PAD_PT: f32 = 16.0;
const FIELD_PT: f32 = 46.0;
const MESSAGE_PT: f32 = 170.0;
const SIGNATURE_PT: f32 = 110.0;
const ACTIVITY_PT: f32 = 260.0;
/// The favourites bar under the toolbar, the find bar over the reader, and the rows of the compose sheet's files.
const FAVORITES_PT: f32 = 30.0;
const FIND_BAR_PT: f32 = 36.0;
const ATTACH_ROW_PT: f32 = 26.0;
const NOTE_PT: f32 = 22.0;
const REMOVE_W_PT: f32 = 64.0;
const DOT_PT: f32 = 8.0;
/// The indent of one level of a conversation in the threaded list, and how many levels are indented (deeper replies
/// stay at the last).
const INDENT_PT: f32 = 14.0;
const MAX_INDENT_LEVELS: usize = 6;
const RADIUS_PT: f32 = m::ICON_RADIUS;

/// Where a message row's sender and subject start in a list column at `x0`: past the unread dot's gutter (the dot, then
/// a space so it never touches the first letter) and the conversation indent of `depth`.
pub fn list_text_x(ui: &Ui, x0: f32, depth: usize) -> f32 {
    x0 + ui.pt(SPACE_M) + ui.pt(DOT_PT) + ui.pt(SPACE_S) + depth.min(MAX_INDENT_LEVELS) as f32 * ui.pt(INDENT_PT)
}
const PANEL_RADIUS_PT: f32 = m::CARD_RADIUS;

/// A rectangle in pixels: x, y, width, height.
pub type Rect = [f32; 4];

pub fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
}

/// The widths of the sidebar, the message list and the reader, in pixels. They fill the width of the window.
pub fn columns(ui: &Ui, w: f32, sidebar_shown: bool) -> [f32; 3] {
    let pt = ui.pt(1.0);
    let [a, b, c] = layout::panes_for(w / pt, sidebar_shown);
    [a * pt, b * pt, c * pt]
}

/// Which pane a point is in. The toolbar, and anything left of the list or right of it, is chrome.
pub fn pane_at(columns: [f32; 3], top: f32, x: f32, y: f32) -> Region {
    let [side, list, _] = columns;
    if y < top {
        Region::Chrome
    } else if x >= side && x < side + list {
        Region::List
    } else if x >= side + list {
        Region::Reader
    } else {
        Region::Chrome
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Button {
    Refresh,
    Compose,
    Reply,
    ReplyAll,
    Archive,
    Trash,
    /// The filter chip: shown only while a filter other than All is applied. A tap moves to the next filter.
    Filter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Side {
    Account(usize),
    Mailbox(String),
    /// A smart mailbox, by its place in the saved list.
    Smart(usize),
    /// A title over a group of rows. It takes no tap.
    Heading(&'static str),
    Add,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    Button(Button),
    Side(Side),
    Message(u32),
    Field(usize),
    /// A mailbox on the favourites bar, by its place in the saved list.
    Favorite(usize),
    /// Load Remote Content, under the privacy note of the open message.
    LoadRemote,
    /// A link of the "Found in Mail" banner over the message text.
    Found(crate::found::FoundHit),
    /// Don't Save on the question sheet: the message is discarded.
    Discard,
    /// A rule's name in the Rules pane: its editor opens.
    EditRule(usize),
    /// The disclosure triangle of a conversation: folds or unfolds it. The UID is its first message's.
    Fold(u32),
    /// A file named in the open message, by its place in the message.
    Attachment(usize),
    /// The Attach button of the compose sheet.
    Attach,
    /// The Remove button of a file on the compose sheet.
    RemoveAttachment(usize),
    FindPrevious,
    FindNext,
    FindDone,
    /// A mailbox the Open Quickly sheet lists under its field, by its place in the list.
    Choice(usize),
    Cancel,
    Primary,
    None,
}

/// The height of the favourites bar under the toolbar: 0 when it is hidden or has nothing on it.
fn favorites_height(ui: &Ui, mail: &Mail) -> f32 {
    if mail.prefs.show_favorites && !mail.prefs.favorites.is_empty() {
        ui.pt(FAVORITES_PT)
    } else {
        0.0
    }
}

/// The height of the toolbar and the favourites bar together: where the panes start.
pub fn chrome_height(ui: &Ui, mail: &Mail) -> f32 {
    ui.pt(TOOLBAR_PT) + favorites_height(ui, mail)
}

/// What a favourite shows: the mailbox, with its account's name when there is more than one account.
fn favorite_label(mail: &Mail, fav: &Favorite) -> String {
    let name = mailbox_title(&fav.mailbox);
    if mail.accounts.len() > 1 {
        let account = mail.accounts.iter().find(|a| a.id == fav.account).map_or("", |a| a.name.as_str());
        format!("{account}: {name}")
    } else {
        name.to_string()
    }
}

/// The favourites and their rectangles, left to right, in a bar that starts at `y` and is `h` high.
fn favorite_items(mail: &Mail, ui: &Ui, y: f32, h: f32) -> Vec<(usize, Rect)> {
    let pad = ui.pt(SPACE_S);
    let mut x = ui.pt(SPACE_M);
    let mut items = Vec::new();
    for (i, fav) in mail.prefs.favorites.iter().enumerate() {
        let width = ui.measure(&favorite_label(mail, fav), MAC_FOOTNOTE_PT, Weight::Regular) + 2.0 * pad;
        items.push((i, [x, y + ui.pt(3.0), width, (h - ui.pt(6.0)).max(0.0)]));
        x += width + ui.pt(SPACE_XS);
    }
    items
}

/// The toolbar buttons and their rectangles, from the right edge inwards: Compose, then Reply, Reply All (only when the
/// message has other recipients), Archive and Trash when a message is open, then the filter chip while a filter is
/// applied, then Refresh.
fn toolbar_items(open: bool, others: bool, filter: Filter) -> Vec<(Button, String)> {
    let mut order: Vec<(Button, String)> = vec![(Button::Compose, "Compose".to_string())];
    if open {
        order.push((Button::Reply, "Reply".to_string()));
        if others {
            order.push((Button::ReplyAll, "Reply All".to_string()));
        }
        order.push((Button::Archive, "Archive".to_string()));
        order.push((Button::Trash, "Trash".to_string()));
    }
    if filter != Filter::All {
        order.push((Button::Filter, filter.label().to_string()));
    }
    order.push((Button::Refresh, "Refresh".to_string()));
    order
}

/// Where the toolbar's items go, in points: the unified bar, starting clear of the traffic lights the shell draws over
/// the window's top left corner.
fn toolbar_bounds(ui: &Ui, w: f32) -> bifrost_app::controls::Rect {
    let x = traffic_lights_trailing() + widgets::TOOLBAR_GAP_PT;
    bifrost_app::controls::Rect { x, y: 0.0, w: (w / ui.pt(1.0) - x - widgets::TOOLBAR_GAP_PT).max(0.0), h: TOOLBAR_PT }
}

/// The glyph a toolbar button wears and its name (the accessibility label), or `None` for the filter chip, which shows
/// the filter's name.
fn toolbar_glyph(button: &Button) -> Option<(&'static str, &'static str)> {
    Some(match button {
        Button::Refresh => ("arrow.clockwise", "Get Mail"),
        Button::Compose => ("square.and.pencil", "New Message"),
        Button::Reply => ("arrowshape.turn.up.left", "Reply"),
        Button::ReplyAll => ("arrowshape.turn.up.left.2", "Reply All"),
        Button::Archive => ("archivebox", "Archive"),
        Button::Trash => ("trash", "Move to Trash"),
        Button::Filter => return None,
    })
}

fn toolbar_model(mail: &Mail) -> Toolbar {
    let mut items = vec![ToolbarItem::custom(90, "Mailbox title", 180.0), ToolbarItem::flexible_space()];
    let visible = toolbar_items(mail.open.is_some(), open_has_others(mail), mail.filter);
    for (button, label) in visible.into_iter().rev() {
        let id = match button {
            Button::Refresh => 1, Button::Compose => 2, Button::Reply => 3, Button::ReplyAll => 4,
            Button::Archive => 5, Button::Trash => 6, Button::Filter => 7,
        };
        // the message actions sit in two capsules: replying, then filing
        let group = match button {
            Button::Reply | Button::ReplyAll => 1,
            Button::Archive | Button::Trash => 2,
            _ => 0,
        };
        items.push(match toolbar_glyph(&button) {
            Some((symbol, name)) => ToolbarItem::icon(id, name, symbol).in_group(group),
            None => ToolbarItem::button(id, &label),
        });
    }
    Toolbar::new(items)
}

fn toolbar_button(id: u32) -> Option<Button> {
    Some(match id {
        1 => Button::Refresh, 2 => Button::Compose, 3 => Button::Reply, 4 => Button::ReplyAll,
        5 => Button::Archive, 6 => Button::Trash, 7 => Button::Filter, _ => return None,
    })
}

/// The names a provider gives the Sent and Drafts mailboxes, in order.
const SENT_NAMES: [&str; 4] = ["Sent", "Sent Items", "Sent Messages", "[Gmail]/Sent Mail"];
const DRAFTS_NAMES: [&str; 3] = ["Drafts", "Draft", "[Gmail]/Drafts"];

/// The glyph of a mailbox in the sidebar, by the role its name says (the standard mailboxes get their own).
fn mailbox_symbol(name: &str) -> &'static str {
    let is = |names: &[&str]| names.iter().any(|n| n.eq_ignore_ascii_case(name));
    if is(&[INBOX]) {
        "tray"
    } else if is(&DRAFTS_NAMES) {
        "doc"
    } else if is(&SENT_NAMES) {
        "paperplane"
    } else if is(&JUNK_NAMES) {
        "xmark.bin"
    } else if is(&TRASH_NAMES) {
        "trash"
    } else if is(&ARCHIVE_NAMES) {
        "archivebox"
    } else {
        "folder"
    }
}

/// The sidebar: Favorites (the standard mailboxes of the account on screen, greyed while it has none), the smart
/// mailboxes, then Mailboxes (each account, and the mailboxes of the one on screen). With no account every favourite is
/// greyed and Mailboxes says so, so the column never reads as broken. The second list maps a row's id to what a tap
/// does; greyed rows have no entry.
fn sidebar_model(mail: &Mail) -> (SourceList, Vec<(u32, Side)>) {
    let mut items = Vec::new();
    let mut actions: Vec<(u32, Side)> = Vec::new();
    let standard: [(&str, &[&str]); 5] =
        [("Inbox", &[INBOX]), ("Drafts", &DRAFTS_NAMES), ("Sent", &SENT_NAMES), ("Junk", &JUNK_NAMES), ("Trash", &TRASH_NAMES)];
    items.push(SidebarItem::header("Favorites"));
    for (title, names) in standard {
        let symbol = mailbox_symbol(names[0]);
        // `Mail::new` seeds the Inbox, so without an account nothing is found: the row stays greyed
        let found = if mail.accounts.is_empty() { None } else { mail.find_mailbox(names) };
        match found {
            Some(name) => {
                let id = actions.len() as u32 + 1;
                items.push(SidebarItem::row(id, title).with_symbol(symbol));
                actions.push((id, Side::Mailbox(name)));
            }
            // a row nothing answers to: its id never collides with an action's
            None => items.push(SidebarItem::row(u32::MAX - items.len() as u32, title).with_symbol(symbol).disabled()),
        }
    }
    if !mail.prefs.smart.is_empty() {
        items.push(SidebarItem::header("Smart Mailboxes"));
        for (index, smart) in mail.prefs.smart.iter().enumerate() {
            let id = actions.len() as u32 + 1;
            items.push(SidebarItem::row(id, &smart.name).with_symbol("folder"));
            actions.push((id, Side::Smart(index)));
        }
    }
    items.push(SidebarItem::header("Mailboxes"));
    if mail.accounts.is_empty() {
        items.push(SidebarItem::row(u32::MAX - items.len() as u32, "No Accounts").with_symbol("envelope").disabled());
    }
    for (index, account) in mail.accounts.iter().enumerate() {
        let id = actions.len() as u32 + 1;
        items.push(SidebarItem::row(id, &account.name).with_symbol("envelope"));
        actions.push((id, Side::Account(index)));
        if index == mail.account {
            for name in &mail.mailboxes {
                let id = actions.len() as u32 + 1;
                items.push(SidebarItem::row(id, mailbox_title(name)).with_symbol(mailbox_symbol(name)));
                actions.push((id, Side::Mailbox(name.clone())));
            }
        }
    }
    let id = actions.len() as u32 + 1;
    items.push(SidebarItem::row(id, "Add Account...").with_symbol("plus"));
    actions.push((id, Side::Add));
    (SourceList::new(items, SIDE_ROW_PT, SIDE_HEAD_PT), actions)
}

/// True when the open message has recipients besides its sender and the user (Reply All is offered).
pub fn open_has_others(mail: &Mail) -> bool {
    mail.open.and_then(|uid| mail.row(uid)).is_some_and(|row| mail.has_other_recipients(&row.summary))
}

/// The mailboxes the Open Quickly sheet lists under its field, the best matches first.
pub fn choices_of(mail: &Mail) -> Vec<String> {
    match &mail.sheet {
        Some(Sheet::Quickly(form)) => mail.quickly_matches(form.value("Go to mailbox")).into_iter().take(MAX_CHOICES).collect(),
        _ => Vec::new(),
    }
}

/// How many rows the open sheet lists under a field: the mailboxes Open Quickly matches, or the recipient suggestions of
/// the focused To, Cc or Bcc field.
pub fn choice_count(mail: &Mail) -> usize {
    match &mail.sheet {
        Some(Sheet::Compose(_)) => mail.recipient_suggestions().len().min(MAX_SUGGESTIONS),
        _ => choices_of(mail).len(),
    }
}

/// The header block of a message (every line before the first blank line), for All Headers.
pub fn header_block(raw: &str) -> String {
    let text = raw_text(raw);
    match text.find("\n\n") {
        Some(end) => text[..end].to_string(),
        None => text,
    }
}

/// The sheet's panel, field rectangles and buttons. Compose's message field is taller than the others, and its sheet also
/// lays out the signature note, one row for each attached file, and the Attach button.
pub struct SheetLayout {
    pub panel: Rect,
    pub fields: Vec<Rect>,
    pub cancel: Rect,
    pub primary: Rect,
    /// The question sheet's Don't Save button, left of Cancel.
    pub discard: Option<Rect>,
    /// Compose: the Attach button at the left of the footer.
    pub attach: Option<Rect>,
    /// Compose: the line that shows the signature the message will end with.
    pub note: Option<Rect>,
    /// Compose: the row of each attached file and its Remove button.
    pub files: Vec<(Rect, Rect)>,
    /// The row of each mailbox Open Quickly matches (below its field), or of each recipient suggestion (right under the
    /// focused To, Cc or Bcc field).
    pub choices: Vec<Rect>,
}

/// The height of a field: the message and the signature take several lines.
fn field_height(ui: &Ui, label: &str) -> f32 {
    match label {
        "Message" => ui.pt(MESSAGE_PT),
        "Signature" => ui.pt(SIGNATURE_PT),
        "Activity" => ui.pt(ACTIVITY_PT),
        _ => ui.pt(FIELD_PT),
    }
}

/// The sheet's layout. `choices` is how many matches the sheet lists under its fields (Open Quickly); it is at most
/// [`MAX_CHOICES`] and only the sheet that lists them takes any.
pub fn sheet_layout(ui: &Ui, w: f32, h: f32, sheet: &Sheet, choices: usize) -> SheetLayout {
    let form = sheet.form();
    // the question sheet has buttons only
    let shown: &[crate::state::Field] = if matches!(sheet, Sheet::Question(_)) { &[] } else { &form.fields };
    let pad = ui.pt(SHEET_PAD_PT);
    let head = ui.pt(SHEET_HEAD_PT);
    let foot = ui.pt(SHEET_FOOT_PT);
    let heights: Vec<f32> = shown.iter().map(|f| field_height(ui, f.label)).collect();
    let (note_h, files) = match sheet {
        Sheet::Rules(_, names) => (0.0, names.len()),
        Sheet::Compose(c) => (if c.signature.trim().is_empty() { 0.0 } else { ui.pt(NOTE_PT) }, c.attachments.len()),
        // Add Account: two lines for the progress of the lookup and what it found
        Sheet::Setup(_) => (2.0 * ui.pt(NOTE_PT), 0),
        _ => (0.0, 0),
    };
    let choices = match sheet {
        Sheet::Quickly(_) => choices.min(MAX_CHOICES),
        Sheet::Compose(_) => choices.min(MAX_SUGGESTIONS),
        _ => 0,
    };
    // the suggestions of a recipient field sit right under it; Open Quickly's list follows its only field
    let list_after = if matches!(sheet, Sheet::Compose(_)) { form.focus } else { shown.len().saturating_sub(1) };
    let row_h = ui.pt(ATTACH_ROW_PT);
    let body: f32 = heights.iter().sum::<f32>() + note_h + files as f32 * row_h + choices as f32 * row_h;
    let pw = ui.pt(SHEET_W_PT).min(w - 2.0 * ui.pt(SPACE_M)).max(0.0);
    let ph = head + body + foot;
    let px = ((w - pw) / 2.0).max(0.0);
    let py = ((h - ph) / 2.0).max(ui.pt(TOOLBAR_PT) + pad);
    let mut y = py + head;
    let mut fields = Vec::with_capacity(heights.len());
    let mut matched = Vec::with_capacity(choices);
    for (i, hh) in heights.into_iter().enumerate() {
        fields.push([px + pad, y, pw - 2.0 * pad, hh]);
        y += hh;
        if i == list_after {
            for _ in 0..choices {
                matched.push([px + pad, y, pw - 2.0 * pad, row_h]);
                y += row_h;
            }
        }
    }
    let note = (note_h > 0.0).then(|| [px + pad, y, pw - 2.0 * pad, note_h]);
    y += note_h;
    let remove_w = ui.pt(REMOVE_W_PT);
    let mut rows = Vec::with_capacity(files);
    for _ in 0..files {
        let row = [px + pad, y, pw - 2.0 * pad, row_h];
        rows.push((row, [row[0] + row[2] - remove_w, row[1], remove_w, row[3]]));
        y += row_h;
    }
    let bh = ui.pt(BUTTON_H_PT);
    let bw = ui.pt(BUTTON_W_PT);
    let by = py + ph - pad - bh;
    let primary = [px + pw - pad - bw, by, bw, bh];
    let cancel = [primary[0] - ui.pt(SPACE_S) - bw, by, bw, bh];
    let discard = matches!(sheet, Sheet::Question(_)).then(|| [cancel[0] - ui.pt(SPACE_S) - bw, by, bw, bh]);
    let attach = matches!(sheet, Sheet::Compose(_) | Sheet::Rules(..)).then(|| [px + pad, by, bw, bh]);
    SheetLayout { panel: [px, py, pw, ph], fields, cancel, primary, discard, attach, note, files: rows, choices: matched }
}

/// The wrapped text of the open message, kept while the message and the width stay the same.
pub struct Laid {
    pub uid: u32,
    pub width: f32,
    pub lines: Vec<String>,
}

/// Greedy word wrap at `width` pixels. Blank lines in the text stay as blank lines. A word wider than the line is
/// left whole on its own line.
pub fn wrap(ui: &Ui, text: &str, width: f32, pt: f32, weight: Weight) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = paragraph.trim_end_matches('\r');
        if paragraph.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if line.is_empty() || ui.measure(&candidate, pt, weight) <= width {
                line = candidate;
            } else {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        lines.push(line);
    }
    lines
}

/// `text` cut to fit `max` pixels, with `...` where it is cut.
pub(crate) fn fit(ui: &Ui, text: &str, pt: f32, weight: Weight, max: f32) -> String {
    if ui.measure(text, pt, weight) <= max {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "...";
        if ui.measure(&candidate, pt, weight) <= max {
            return candidate;
        }
    }
    String::new()
}

/// The name in a `Name <address>` value, or the address when there is no name.
/// The colour a flag is drawn in.
fn flag_rgb(color: FlagColor) -> u32 {
    let pair = match color {
        FlagColor::Red => colors::SYSTEM_RED,
        FlagColor::Orange => colors::SYSTEM_ORANGE,
        FlagColor::Yellow => colors::SYSTEM_YELLOW,
        FlagColor::Green => colors::SYSTEM_GREEN,
        FlagColor::Blue => colors::SYSTEM_BLUE,
        FlagColor::Purple => colors::SYSTEM_PURPLE,
        FlagColor::Gray => colors::SYSTEM_GRAY,
    };
    pair.now()
}

/// The message as the server holds it (View > Raw Source), for the reader: every header and the undecoded body, with
/// plain line ends and no control characters other than tabs.
pub fn raw_text(raw: &str) -> String {
    raw.replace("\r\n", "\n").chars().filter(|c| *c == '\n' || *c == '\t' || !c.is_control()).collect()
}

/// A disclosure triangle in a box `size` wide at (`x`, `y`): pointing right when `folded`, down when not.
fn disclosure(s: &mut Surface, x: f32, y: f32, size: f32, folded: bool, color: u32) {
    let n = size.max(2.0) as usize;
    let (x, y) = (px(x), px(y));
    // the triangle is drawn one stripe at a time: a column for the right-pointing one, a row for the other
    for i in 0..n / 2 + 1 {
        let len = n.saturating_sub(2 * i);
        if len == 0 {
            break;
        }
        if folded {
            s.fill_rect(x + i, y + i, 1, len, color);
        } else {
            s.fill_rect(x + i, y + i, len, 1, color);
        }
    }
}

/// The weight a style's text is drawn in.
fn style_weight(style: Style) -> Weight {
    if style.bold {
        Weight::Bold
    } else {
        Weight::Regular
    }
}

/// The colour a style's text is drawn in.
fn style_rgb(style: Style) -> u32 {
    let pair = match style.color {
        Some(TextColor::Red) => colors::SYSTEM_RED,
        Some(TextColor::Orange) => colors::SYSTEM_ORANGE,
        Some(TextColor::Green) => colors::SYSTEM_GREEN,
        Some(TextColor::Blue) => colors::SYSTEM_BLUE,
        Some(TextColor::Purple) => colors::SYSTEM_PURPLE,
        Some(TextColor::Gray) => colors::SYSTEM_GRAY,
        None => return LABEL.now(),
    };
    pair.now()
}

/// A copy of a text image leaning to the right: each row is moved over a little more than the row below it. Used for
/// italic text, because the font has no italic.
fn slanted(img: &Image) -> Image {
    let extra = img.h / 4;
    let w = img.w + extra;
    let mut px = vec![0u32; w * img.h];
    for y in 0..img.h {
        let shift = (img.h - 1 - y) * extra / img.h.max(1);
        px[y * w + shift..y * w + shift + img.w].copy_from_slice(&img.px[y * img.w..(y + 1) * img.w]);
    }
    Image { w, h: img.h, px }
}

fn display_name(from: &str) -> &str {
    let name = from.split('<').next().unwrap_or("").trim().trim_matches('"').trim();
    if name.is_empty() {
        from.trim()
    } else {
        name
    }
}

/// The words that end a list row's subject line: Muted for a muted conversation, and the sender's priority when it is
/// high or low. Empty when there is nothing to say.
pub fn row_tag(muted: bool, priority: Priority) -> String {
    let mut tags: Vec<&str> = Vec::new();
    if muted {
        tags.push("Muted");
    }
    match priority {
        Priority::High => tags.push("High priority"),
        Priority::Low => tags.push("Low priority"),
        Priority::Normal => {}
    }
    tags.join(", ")
}

/// The day and month of a Date header (`Tue, 13 Oct 2026 ...` gives `13 Oct`), or the header as it is.
fn short_date(date: &str) -> String {
    let parts: Vec<&str> = date.split_whitespace().collect();
    if parts.len() >= 3 {
        format!("{} {}", parts[1], parts[2])
    } else {
        date.to_string()
    }
}

fn mailbox_title(name: &str) -> &str {
    if name.eq_ignore_ascii_case("INBOX") {
        "Inbox"
    } else {
        name
    }
}

// text helpers --------------------------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(crate) fn put(s: &mut Surface, labels: &mut TextCache, ui: &Ui, text: &str, pt: f32, wt: Weight, color: u32, x: f32, y: f32) {
    if text.is_empty() {
        return;
    }
    let img = labels.get(ui, text, pt, wt, color);
    s.blit(img, x as i32, y as i32, 255, 255);
}

/// Text centred vertically on `cy`, its left edge at `x`.
#[allow(clippy::too_many_arguments)]
fn put_mid(s: &mut Surface, labels: &mut TextCache, ui: &Ui, text: &str, pt: f32, wt: Weight, color: u32, x: f32, cy: f32) {
    if text.is_empty() {
        return;
    }
    let img = labels.get(ui, text, pt, wt, color);
    let top = cy - img.h as f32 / 2.0;
    s.blit(img, x as i32, top as i32, 255, 255);
}

/// Text centred on (`cx`, `cy`).
#[allow(clippy::too_many_arguments)]
fn put_centred(s: &mut Surface, labels: &mut TextCache, ui: &Ui, text: &str, pt: f32, wt: Weight, color: u32, cx: f32, cy: f32) {
    if text.is_empty() {
        return;
    }
    let img = labels.get(ui, text, pt, wt, color);
    let (x, y) = (cx - img.w as f32 / 2.0, cy - img.h as f32 / 2.0);
    s.blit(img, x as i32, y as i32, 255, 255);
}

fn px(v: f32) -> usize {
    v.max(0.0) as usize
}

impl MailWindow {
    /// The control under a point, or [`Hit::None`]. While a sheet is open it takes every tap.
    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Hit {
        let (w, h) = (self.w as f32, self.h as f32);
        if let Some(sheet) = &self.mail.sheet {
            let l = sheet_layout(ui, w, h, sheet, choice_count(&self.mail));
            if inside(l.primary, x, y) {
                return Hit::Primary;
            }
            if l.discard.is_some_and(|r| inside(r, x, y)) {
                return Hit::Discard;
            }
            if inside(l.cancel, x, y) {
                return Hit::Cancel;
            }
            if l.attach.is_some_and(|r| inside(r, x, y)) {
                return Hit::Attach;
            }
            if let Some(index) = l.files.iter().position(|(_, remove)| inside(*remove, x, y)) {
                return Hit::RemoveAttachment(index);
            }
            if matches!(sheet, Sheet::Rules(..)) {
                if let Some(index) = l.files.iter().position(|(row, _)| inside(*row, x, y)) {
                    return Hit::EditRule(index);
                }
            }
            if let Some(index) = l.choices.iter().position(|r| inside(*r, x, y)) {
                return Hit::Choice(index);
            }
            return l.fields.iter().position(|r| inside(*r, x, y)).map_or(Hit::None, Hit::Field);
        }
        let bar = ui.pt(TOOLBAR_PT);
        let top = chrome_height(ui, &self.mail);
        if y < bar {
            let scale = ui.pt(1.0);
            let model = toolbar_model(&self.mail);
            let placed = widgets::toolbar_layout(ui, toolbar_bounds(ui, w), &model);
            return placed.hit(x / scale, y / scale).and_then(toolbar_button).map_or(Hit::None, Hit::Button);
        }
        if y < top {
            return favorite_items(&self.mail, ui, bar, top - bar).into_iter().find(|(_, r)| inside(*r, x, y)).map_or(Hit::None, |(i, _)| Hit::Favorite(i));
        }
        let [side, list, _] = columns(ui, w, self.mail.show_sidebar);
        if x < side {
            let scale = ui.pt(1.0);
            let (model, actions) = sidebar_model(&self.mail);
            let bounds = bifrost_app::controls::Rect {
                x: 0.0, y: (top + ui.pt(SPACE_S)) / scale, w: side / scale, h: (h - top) / scale,
            };
            return model.hit(bounds, 0.0, x / scale, y / scale)
                .and_then(|id| actions.iter().find(|(key, _)| *key == id).map(|(_, side)| side.clone()))
                .map_or(Hit::None, |side| Hit::Side(side));
        }
        if x < side + list {
            let row = ui.pt(ROW_PT);
            let index = ((y - top + self.list_scroll.offset()) / row).floor();
            let lines = self.mail.visible_lines();
            if index >= 0.0 && (index as usize) < lines.len() {
                let line = &lines[index as usize];
                if self.mail.threaded && line.head && line.thread > 1 && x - side < ui.pt(SPACE_M) {
                    return Hit::Fold(line.row.uid);
                }
                return Hit::Message(line.row.uid);
            }
            return Hit::None;
        }
        if let Some([previous, next, done]) = self.find_hits {
            if inside(previous, x, y) {
                return Hit::FindPrevious;
            }
            if inside(next, x, y) {
                return Hit::FindNext;
            }
            if inside(done, x, y) {
                return Hit::FindDone;
            }
        }
        if self.remote_rect.is_some_and(|r| inside(r, x, y)) {
            return Hit::LoadRemote;
        }
        if let Some((_, link)) = self.found_rects.iter().find(|(r, _)| inside(*r, x, y)) {
            return Hit::Found(*link);
        }
        if let Some((_, index)) = self.attach_rects.iter().find(|(r, _)| inside(*r, x, y)) {
            return Hit::Attachment(*index);
        }
        Hit::None
    }

    /// Paints the window: the sidebar, the message list, the reader, the toolbar, the favourites bar, and any sheet on top.
    pub fn draw(&mut self, s: &mut Surface, ui: &Ui) {
        let (w, h) = (s.w as f32, s.h as f32);
        let [side, list, view] = columns(ui, w, self.mail.show_sidebar);
        let bar = ui.pt(TOOLBAR_PT);
        let top = chrome_height(ui, &self.mail);
        let rule = (ui.pt(1.0) as usize).max(1);
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
        s.fill_rect(0, px(top), px(side), px(h - top), CARD_BG.now());
        if side > 0.0 {
            self.draw_sidebar(s, ui, side, top);
        }
        self.draw_list(s, ui, side, list, top, h);
        self.draw_reader(s, ui, side + list, view, top, h);
        s.fill_rect(px(side), px(top), rule, px(h - top), SEPARATOR_C.now());
        s.fill_rect(px(side + list), px(top), rule, px(h - top), SEPARATOR_C.now());
        self.draw_toolbar(s, ui, w, bar);
        if top > bar {
            self.draw_favorites(s, ui, bar, top - bar);
        }
        if let Some(sheet) = self.mail.sheet.clone() {
            self.draw_sheet(s, ui, &sheet, w, h);
        }
    }

    /// Paints the window without the compose sheet while that sheet belongs to a compose window (`compose_windows`).
    pub fn draw_hiding_compose(&mut self, s: &mut Surface, ui: &Ui, hide: bool) {
        let hidden = if hide && matches!(self.mail.sheet, Some(Sheet::Compose(_) | Sheet::Question(_))) { self.mail.sheet.take() } else { None };
        self.draw(s, ui);
        if hidden.is_some() {
            self.mail.sheet = hidden;
        }
    }

    /// Paints a compose window: its message sheet, centred, over the content background.
    pub fn draw_compose(&mut self, s: &mut Surface, ui: &Ui, sheet: &Sheet) {
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
        let (w, h) = (s.w as f32, s.h as f32);
        self.draw_sheet(s, ui, sheet, w, h);
    }

    /// The favourites bar: one chip for each pinned mailbox, the one on screen filled.
    fn draw_favorites(&mut self, s: &mut Surface, ui: &Ui, y: f32, h: f32) {
        s.fill_rect(0, px(y), s.w, px(h), CONTENT_BG.now());
        s.fill_rect(0, px(y + h) - 1, s.w, 1, SEPARATOR_C.now());
        let shown = self.mail.current().map(|a| a.id.clone());
        for (index, r) in favorite_items(&self.mail, ui, y, h) {
            let Some(fav) = self.mail.prefs.favorites.get(index).cloned() else { continue };
            let selected = shown.as_deref() == Some(fav.account.as_str()) && fav.mailbox == self.mail.mailbox;
            let (fill, color) = if selected { (colors::SYSTEM_BLUE.now(), WHITE) } else { (FIELD_BG.now(), LABEL.now()) };
            s.fill_rrect(r[0], r[1], r[2], r[3], ui.pt(RADIUS_PT), fill);
            let label = favorite_label(&self.mail, &fav);
            put_centred(s, &mut self.labels, ui, &label, MAC_FOOTNOTE_PT, Weight::Regular, color, r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        }
    }

    fn draw_toolbar(&mut self, s: &mut Surface, ui: &Ui, w: f32, _top: f32) {
        widgets::toolbar_band(s, ui, bifrost_app::controls::Rect { x: 0.0, y: 0.0, w: w / ui.pt(1.0), h: TOOLBAR_PT });
        let title = if self.mail.accounts.is_empty() { "Mail" } else { mailbox_title(&self.mail.mailbox) };
        let model = toolbar_model(&self.mail);
        let placed = widgets::toolbar(s, ui, toolbar_bounds(ui, w), &model, None);
        if let Some(r) = placed.rect_of(90) {
            put_mid(s, &mut self.labels, ui, title, MAC_BODY_PT, Weight::Semibold, LABEL.now(), ui.pt(r.x), ui.pt(r.y + r.h / 2.0));
        }
    }

    fn draw_sidebar(&mut self, s: &mut Surface, ui: &Ui, side: f32, top: f32) {
        let (mut model, actions) = sidebar_model(&self.mail);
        let selected = actions.iter().find_map(|(id, side)| match side {
            Side::Smart(index) if self.mail.smart_active == Some(*index) => Some(*id),
            Side::Mailbox(name) if name == &self.mail.mailbox && self.mail.smart_active.is_none() => Some(*id),
            _ => None,
        });
        model.set_selected(selected);
        let scale = ui.pt(1.0);
        let bounds = bifrost_app::controls::Rect {
            x: 0.0, y: (top + ui.pt(SPACE_S)) / scale, w: side / scale, h: (s.h as f32 - top) / scale,
        };
        widgets::source_list(s, ui, bounds, &model, 0.0, None);
    }

    fn draw_list(&mut self, s: &mut Surface, ui: &Ui, x0: f32, width: f32, top: f32, h: f32) {
        let row = ui.pt(ROW_PT);
        let pad = ui.pt(SPACE_M);
        let inset = ui.pt(SPACE_S);
        let dot = ui.pt(DOT_PT);
        let area = (h - top).max(0.0);
        let lines = self.mail.visible_lines();
        self.list_scroll.set_max((lines.len() as f32 * row - area).max(0.0));
        let offset = self.list_scroll.offset();
        s.fill_rect(px(x0), px(top), px(width), px(area), CONTENT_BG.now());
        let right = x0 + width - pad;
        let open = self.mail.open;
        for (i, line) in lines.iter().enumerate() {
            let r = &line.row;
            let y = top + i as f32 * row - offset;
            if y + row < top || y > h {
                continue;
            }
            let text_x = list_text_x(ui, x0, line.depth);
            let selected = open == Some(r.uid);
            if selected {
                s.fill_rrect(x0 + inset, y + inset / 2.0, width - 2.0 * inset, row - inset, ui.pt(RADIUS_PT), colors::SYSTEM_BLUE.now());
            } else if !r.flags.contains(Flag::Seen) {
                s.fill_rrect(x0 + pad, y + row * 0.3 - dot / 2.0, dot, dot, dot / 2.0, colors::SYSTEM_BLUE.now());
            }
            if self.mail.threaded && line.head && line.thread > 1 {
                let tint = if selected { WHITE } else { LABEL_2.now() };
                disclosure(s, x0 + ui.pt(3.0), y + row * 0.3 - ui.pt(3.0), ui.pt(7.0), self.mail.collapsed.contains(&r.uid), tint);
            }
            let (c_name, c_sub, c_date) = if selected { (WHITE, WHITE, WHITE) } else { (LABEL.now(), LABEL.now(), LABEL_2.now()) };
            let date = short_date(&r.summary.date);
            let date_w = ui.measure(&date, MAC_FOOTNOTE_PT, Weight::Regular);
            let name_max = (right - text_x - date_w - inset).max(0.0);
            let name_weight = if self.mail.prefs.bold_unread && !r.flags.contains(Flag::Seen) { Weight::Semibold } else { Weight::Regular };
            let name = fit(ui, display_name(&r.summary.from), MAC_BODY_PT, name_weight, name_max);
            put(s, &mut self.labels, ui, &name, MAC_BODY_PT, name_weight, c_name, text_x, y + row * 0.14);
            put(s, &mut self.labels, ui, &date, MAC_FOOTNOTE_PT, Weight::Regular, c_date, right - date_w, y + row * 0.16);
            let base = if r.summary.subject.is_empty() { "(no subject)" } else { r.summary.subject.as_str() };
            let label = if line.thread > 1 { format!("({}) {base}", line.thread) } else { base.to_string() };
            let flag_room = if r.flags.color().is_some() { dot + inset } else { 0.0 };
            let muted = self.mail.muted.contains(&crate::mute::key(&r.summary));
            let tag = row_tag(muted, r.summary.priority);
            let tag_w = if tag.is_empty() { 0.0 } else { ui.measure(&tag, MAC_FOOTNOTE_PT, Weight::Regular) + inset };
            let subject = fit(ui, &label, MAC_BODY_PT, Weight::Regular, (right - text_x - flag_room - tag_w).max(0.0));
            if !tag.is_empty() {
                let tag_x = right - flag_room - tag_w + inset;
                put(s, &mut self.labels, ui, &tag, MAC_FOOTNOTE_PT, Weight::Regular, if selected { WHITE } else { LABEL_2.now() }, tag_x, y + row * 0.5 + ui.pt(SPACE_XXS));
            }
            if let Some(color) = r.flags.color() {
                s.fill_rrect(right - dot, y + row * 0.64 - dot / 2.0, dot, dot, dot / 2.0, flag_rgb(color));
            }
            put(s, &mut self.labels, ui, &subject, MAC_BODY_PT, Weight::Regular, c_sub, text_x, y + row * 0.5);
            if !selected {
                s.fill_rect(px(text_x), px(y + row - 1.0), px(right - text_x), 1, SEPARATOR_C.now());
            }
        }

        let footer = ui.pt(FOOT_LINE_PT) + 2.0 * pad;
        let footer_top = top + area - footer;
        let centre_x = x0 + width / 2.0;
        if lines.is_empty() {
            let message = if self.mail.accounts.is_empty() {
                "Add an account to start"
            } else if !self.mail.search.is_empty() && self.busy() {
                "Searching..."
            } else if !self.mail.search.is_empty() {
                "No matching messages"
            } else if self.busy() {
                "Loading..."
            } else {
                "No messages"
            };
            put_centred(s, &mut self.labels, ui, message, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), centre_x, top + area / 2.0);
        }
        s.fill_rect(px(x0), px(footer_top), px(width), px(footer), CONTENT_BG.now());
        let note = if !self.mail.status.is_empty() {
            self.mail.status.clone()
        } else if self.busy() {
            "Checking mail...".to_string()
        } else {
            String::new()
        };
        let note = fit(ui, &note, MAC_FOOTNOTE_PT, Weight::Regular, (width - 2.0 * pad).max(0.0));
        put_mid(s, &mut self.labels, ui, &note, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x0 + pad, footer_top + footer / 2.0);
    }

    fn draw_reader(&mut self, s: &mut Surface, ui: &Ui, x0: f32, width: f32, top: f32, h: f32) {
        let pad = ui.pt(READER_PAD_PT);
        let area = (h - top).max(0.0);
        let text_w = (width - 2.0 * pad).max(0.0);
        self.attach_rects.clear();
        self.remote_rect = None;
        self.found_rects.clear();
        self.find_hits = None;
        let Some(uid) = self.mail.open else {
            self.reader_scroll.set_max(0.0);
            let message = if self.mail.accounts.is_empty() { "Add an account to read your mail" } else { "Select a message" };
            put_centred(s, &mut self.labels, ui, message, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x0 + width / 2.0, top + area / 2.0);
            return;
        };
        let Some(summary) = self.mail.rows.iter().find(|r| r.uid == uid).map(|r| r.summary.clone()) else {
            self.reader_scroll.set_max(0.0);
            return;
        };
        let loaded = self.mail.bodies.contains_key(&uid);
        if loaded {
            self.ensure_laid(ui, uid, text_w);
        }
        let lines: Vec<String> = match &self.laid {
            Some(laid) if laid.uid == uid => laid.lines.clone(),
            _ => Vec::new(),
        };
        let matches: Vec<findbar::Span> = match &self.mail.find {
            Some(bar) => findbar::find_all(&lines, &bar.text),
            None => Vec::new(),
        };
        let current = self.mail.find.as_ref().map_or(0, |bar| bar.current).min(matches.len().saturating_sub(1));
        let bar_h = if self.mail.find.is_some() { ui.pt(FIND_BAR_PT) } else { 0.0 };
        let offset = self.reader_scroll.offset();
        let x = x0 + pad;
        let mut y = top + bar_h + pad - offset;

        let subject = if summary.subject.is_empty() { "(no subject)" } else { summary.subject.as_str() };
        for line in wrap(ui, subject, text_w, MAC_TITLE1_PT, Weight::Semibold) {
            put(s, &mut self.labels, ui, &line, MAC_TITLE1_PT, Weight::Semibold, LABEL.now(), x, y);
            y += ui.pt(TITLE_LINE_PT);
        }
        y += ui.pt(SPACE_S);
        for (label, value) in [("From", summary.from.as_str()), ("To", summary.to.as_str()), ("Date", summary.date.as_str())] {
            if value.is_empty() {
                continue;
            }
            let text = fit(ui, &format!("{label}: {value}"), MAC_FOOTNOTE_PT, Weight::Regular, text_w);
            put(s, &mut self.labels, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x, y);
            y += ui.pt(FOOT_LINE_PT);
        }
        y += ui.pt(SPACE_M);
        s.fill_rect(px(x), px(y), px(text_w), (ui.pt(1.0) as usize).max(1), SEPARATOR_C.now());
        y += ui.pt(SPACE_M);

        // each file is its own line and a target: a tap saves it
        let names: Vec<String> = self
            .mail
            .attachments
            .get(&uid)
            .map(|files| files.iter().map(|a| format!("{} ({})", a.name, attachments::size_text(a.bytes.len()))).collect())
            .unwrap_or_default();
        for (index, name) in names.iter().enumerate() {
            let text = fit(ui, name, MAC_FOOTNOTE_PT, Weight::Regular, text_w);
            put(s, &mut self.labels, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, colors::SYSTEM_BLUE.now(), x, y);
            let wide = ui.measure(&text, MAC_FOOTNOTE_PT, Weight::Regular);
            self.attach_rects.push(([x, y, wide, ui.pt(FOOT_LINE_PT)], index));
            y += ui.pt(FOOT_LINE_PT);
        }
        if !names.is_empty() {
            y += ui.pt(SPACE_M);
        }

        // remote content stays blocked until the person loads it for this message or Mail Settings allows it
        if loaded && self.remote_memo.map(|(m, _)| m) != Some(uid) {
            let blocked = self.mail.bodies.get(&uid).map_or(0, |raw| crate::body::remote_images(raw));
            self.remote_memo = Some((uid, blocked));
        }
        let blocked = self.remote_memo.filter(|(m, _)| *m == uid).map_or(0, |(_, n)| n);
        if blocked > 0 && !self.mail.remote_allowed(uid) {
            let note = fit(ui, "Remote images are blocked to protect your privacy.", MAC_FOOTNOTE_PT, Weight::Regular, text_w);
            put(s, &mut self.labels, ui, &note, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x, y);
            y += ui.pt(FOOT_LINE_PT);
            let link = "Load Remote Content";
            put(s, &mut self.labels, ui, link, MAC_FOOTNOTE_PT, Weight::Regular, colors::SYSTEM_BLUE.now(), x, y);
            let wide = ui.measure(link, MAC_FOOTNOTE_PT, Weight::Regular);
            self.remote_rect = Some([x, y, wide, ui.pt(FOOT_LINE_PT)]);
            y += ui.pt(FOOT_LINE_PT) + ui.pt(SPACE_M);
        }

        y = self.draw_found_banner(s, ui, x, y, text_w);

        if !loaded {
            put(s, &mut self.labels, ui, "Loading...", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x, y);
            y += ui.pt(BODY_LINE_PT);
        }
        let line_h = ui.pt(BODY_LINE_PT);
        self.reader_metrics = (y + offset - top, line_h, area);
        for (i, line) in lines.iter().enumerate() {
            if y >= top - line_h && y <= h {
                for (k, span) in matches.iter().enumerate().filter(|(_, m)| m.line == i) {
                    let before = ui.measure(&line[..span.start], MAC_BODY_PT, Weight::Regular);
                    let wide = ui.measure(&line[span.start..span.end], MAC_BODY_PT, Weight::Regular);
                    let tint = if k == current { SYSTEM_ORANGE.now() } else { FIND_HIGHLIGHT.now() };
                    s.fill_rrect(x + before, y, wide, line_h, ui.pt(m::ICON_RADIUS), tint);
                }
                put(s, &mut self.labels, ui, line, MAC_BODY_PT, Weight::Regular, LABEL.now(), x, y);
            }
            y += line_h;
        }
        let content = y - (top + bar_h + pad - offset) + pad;
        self.reader_scroll.set_max((content + bar_h - area).max(0.0));
        if bar_h > 0.0 {
            self.draw_find_bar(s, ui, x0, width, top, bar_h, matches.len(), current);
        }
    }

    /// The "Found in Mail" banner over the message text: where the first suggestion of the open message was found, its
    /// title and detail, and the links that add it, turn it down or page to the next. Returns the y below the banner (`y`
    /// again when the message has nothing to offer). The links are recorded for `hit`.
    fn draw_found_banner(&mut self, s: &mut Surface, ui: &Ui, x: f32, y: f32, width: f32) -> f32 {
        let Some(found) = self.mail.found_current().cloned() else { return y };
        let label = self.mail.found_label();
        let pad = ui.pt(SPACE_S);
        let (small, body) = (ui.pt(FOOT_LINE_PT), ui.pt(BODY_LINE_PT));
        let rows = small + body + if found.subtitle.is_empty() { 0.0 } else { small } + small;
        let height = rows + 2.0 * pad;
        s.fill_rrect(x, y, width, height, ui.pt(RADIUS_PT), FIELD_BG.now());
        let (left, inner) = (x + pad, (width - 2.0 * pad).max(0.0));
        let mut cy = y + pad;
        let source = fit(ui, &found.found_in(), MAC_FOOTNOTE_PT, Weight::Regular, inner);
        put(s, &mut self.labels, ui, &source, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), left, cy);
        if let Some(label) = &label {
            let wide = ui.measure(label, MAC_FOOTNOTE_PT, Weight::Regular);
            put(s, &mut self.labels, ui, label, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), left + inner - wide, cy);
        }
        cy += small;
        let title = fit(ui, &found.title, MAC_BODY_PT, Weight::Semibold, inner);
        put(s, &mut self.labels, ui, &title, MAC_BODY_PT, Weight::Semibold, LABEL.now(), left, cy);
        cy += body;
        if !found.subtitle.is_empty() {
            let detail = fit(ui, &found.subtitle, MAC_FOOTNOTE_PT, Weight::Regular, inner);
            put(s, &mut self.labels, ui, &detail, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), left, cy);
            cy += small;
        }
        let gap = ui.pt(SPACE_M);
        let mut lx = left;
        let mut links: Vec<(&str, crate::found::FoundHit)> = Vec::new();
        if let Some(add) = crate::found::add_label(found.kind) {
            links.push((add, crate::found::FoundHit::Add));
        }
        links.push(("Dismiss", crate::found::FoundHit::Dismiss));
        for (text, link) in links {
            put(s, &mut self.labels, ui, text, MAC_FOOTNOTE_PT, Weight::Regular, colors::SYSTEM_BLUE.now(), lx, cy);
            let wide = ui.measure(text, MAC_FOOTNOTE_PT, Weight::Regular);
            self.found_rects.push(([lx, cy, wide, small], link));
            lx += wide + gap;
        }
        if label.is_some() {
            let mut rx = left + inner;
            for (text, link) in [("Next", crate::found::FoundHit::Next), ("Previous", crate::found::FoundHit::Previous)] {
                let wide = ui.measure(text, MAC_FOOTNOTE_PT, Weight::Regular);
                rx -= wide;
                put(s, &mut self.labels, ui, text, MAC_FOOTNOTE_PT, Weight::Regular, colors::SYSTEM_BLUE.now(), rx, cy);
                self.found_rects.push(([rx, cy, wide, small], link));
                rx -= gap;
            }
        }
        y + height + ui.pt(SPACE_M)
    }

    /// The find bar over the top of the reader: the text typed, which match is shown, and the Previous, Next and Done buttons.
    #[allow(clippy::too_many_arguments)]
    fn draw_find_bar(&mut self, s: &mut Surface, ui: &Ui, x0: f32, width: f32, top: f32, bar_h: f32, total: usize, current: usize) {
        let Some(bar) = self.mail.find.clone() else { return };
        s.fill_rect(px(x0), px(top), px(width), px(bar_h), CARD_BG.now());
        s.fill_rect(px(x0), px(top + bar_h) - 1, px(width), 1, SEPARATOR_C.now());
        let pad = ui.pt(SPACE_S);
        let button_h = ui.pt(BUTTON_H_PT - 4.0);
        let by = top + (bar_h - button_h) / 2.0;
        let mut right = x0 + width - ui.pt(SPACE_M);
        let mut rects = [[0.0f32; 4]; 3];
        for (slot, label) in [(2usize, "Done"), (1, "Next"), (0, "Previous")] {
            let bw = ui.measure(label, MAC_FOOTNOTE_PT, Weight::Regular) + 2.0 * pad;
            let r = [right - bw, by, bw, button_h];
            s.fill_rrect(r[0], r[1], r[2], r[3], ui.pt(RADIUS_PT), FIELD_BG.now());
            put_centred(s, &mut self.labels, ui, label, MAC_FOOTNOTE_PT, Weight::Regular, LABEL.now(), r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
            rects[slot] = r;
            right -= bw + ui.pt(SPACE_XS);
        }
        self.find_hits = Some(rects);
        let count = if bar.text.is_empty() {
            String::new()
        } else if total == 0 {
            "Not found".to_string()
        } else {
            format!("{} of {}", current + 1, total)
        };
        let count_w = ui.measure(&count, MAC_FOOTNOTE_PT, Weight::Regular);
        put_mid(s, &mut self.labels, ui, &count, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), right - count_w - pad, top + bar_h / 2.0);
        let field_x = x0 + ui.pt(SPACE_M);
        let field_w = (right - count_w - 2.0 * pad - field_x).max(0.0);
        s.fill_rrect(field_x, by, field_w, button_h, ui.pt(RADIUS_PT), FIELD_BG.now());
        let (shown, color) = if bar.text.is_empty() { ("Find in message".to_string(), LABEL_3.now()) } else { (bar.text.clone(), LABEL.now()) };
        let shown = fit(ui, &shown, MAC_BODY_PT, Weight::Regular, (field_w - 2.0 * pad).max(0.0));
        put_mid(s, &mut self.labels, ui, &shown, MAC_BODY_PT, Weight::Regular, color, field_x + pad, by + button_h / 2.0);
        let caret_x = field_x + pad + if bar.text.is_empty() { 0.0 } else { ui.measure(&shown, MAC_BODY_PT, Weight::Regular) };
        s.fill_rect(px(caret_x), px(by + ui.pt(5.0)), px(ui.pt(1.0).max(1.0)), px(button_h - ui.pt(10.0)), colors::SYSTEM_BLUE.now());
    }

    /// Lays out the open message's text at `width` unless the cache already holds it at that width.
    fn ensure_laid(&mut self, ui: &Ui, uid: u32, width: f32) {
        let fresh = matches!(&self.laid, Some(laid) if laid.uid == uid && (laid.width - width).abs() < 0.5);
        if fresh {
            return;
        }
        let Some(raw) = self.mail.bodies.get(&uid) else { return };
        let text = if self.mail.show_raw {
            raw_text(raw)
        } else if self.mail.prefs.all_headers {
            // View > All Headers: every header line above the text
            format!("{}\n\n{}", header_block(raw), crate::body::readable(raw))
        } else {
            crate::body::readable(raw)
        };
        let lines = wrap(ui, &text, width, MAC_BODY_PT, Weight::Regular);
        self.laid = Some(Laid { uid, width, lines });
    }

    /// The message field of the compose sheet: the text in its styles, a red line under each misspelled word (thicker under the
    /// one in view), and the caret after the last character.
    fn draw_styled_message(&mut self, s: &mut Surface, ui: &Ui, compose: &Compose, r: Rect, focused: bool) {
        let pad = ui.pt(SPACE_S);
        let tx = r[0] + pad;
        let value_y = r[1] + ui.pt(SPACE_XL);
        let line_h = ui.pt(BODY_LINE_PT);
        let rule = (ui.pt(1.0) as usize).max(1);
        let caret = ui.pt(1.0).max(1.0);
        let measure = |text: &str, style: Style| ui.measure(text, MAC_BODY_PT, style_weight(style));
        let lines = richtext::layout(compose.message(), &compose.styles, r[2] - 2.0 * pad, &measure);
        let room = ((r[1] + r[3] - value_y - ui.pt(SPACE_XS)) / line_h).floor().max(1.0) as usize;
        let first = lines.len().saturating_sub(room);
        let current = compose.misspelled.get(compose.misspelled_at);
        let (suggested, suggested_at) = compose.proofread_marks();
        let mut end_x = tx;
        let hilite = focused && compose.form.selected();
        for (k, line) in lines[first..].iter().enumerate() {
            let y = value_y + k as f32 * line_h;
            let mut x = tx;
            if hilite {
                let line_w: f32 = line.iter().map(|run| measure(&run.text, run.style)).sum();
                s.fill_rect(px(tx), px(y), px(line_w), px(line_h), SELECTION.now());
            }
            for run in line {
                let wide = measure(&run.text, run.style);
                let color = if hilite { WHITE } else { style_rgb(run.style) };
                let img = self.labels.get(ui, &run.text, MAC_BODY_PT, style_weight(run.style), color);
                if run.style.italic {
                    s.blit(&slanted(img), x as i32, y as i32, 255, 255);
                } else {
                    s.blit(img, x as i32, y as i32, 255, 255);
                }
                if run.style.underline {
                    s.fill_rect(px(x), px(y + ui.pt(MAC_BODY_PT) + ui.pt(1.0)), px(wide), rule, color);
                }
                let chars: Vec<char> = run.text.chars().collect();
                for mark in &compose.misspelled {
                    let (a, b) = (mark.start.max(run.start), mark.end.min(run.start + chars.len()));
                    if a < b {
                        let before: String = chars[..a - run.start].iter().collect();
                        let word: String = chars[a - run.start..b - run.start].iter().collect();
                        let thick = if current == Some(mark) { 2 * rule } else { rule };
                        s.fill_rect(px(x + measure(&before, run.style)), px(y + line_h - ui.pt(SPACE_XXS)), px(measure(&word, run.style)), thick, colors::SYSTEM_RED.now());
                    }
                }
                // the writing assistant's suggestions: a blue line, thicker under the one in view
                for (k, mark) in suggested.iter().enumerate() {
                    let (a, b) = (mark.start.max(run.start), mark.end.min(run.start + chars.len()));
                    if a < b {
                        let before: String = chars[..a - run.start].iter().collect();
                        let word: String = chars[a - run.start..b - run.start].iter().collect();
                        let thick = if k == suggested_at { 2 * rule } else { rule };
                        s.fill_rect(px(x + measure(&before, run.style)), px(y + line_h - ui.pt(SPACE_XXS)), px(measure(&word, run.style)), thick, colors::SYSTEM_BLUE.now());
                    }
                }
                x += wide;
            }
            end_x = x;
        }
        if focused {
            let cy = value_y + (lines.len() - first).saturating_sub(1) as f32 * line_h;
            s.fill_rect(px(end_x), px(cy), px(caret), px(ui.pt(MAC_BODY_PT)), colors::SYSTEM_BLUE.now());
        }
    }

    fn draw_sheet(&mut self, s: &mut Surface, ui: &Ui, sheet: &Sheet, w: f32, h: f32) {
        let matches = choices_of(&self.mail);
        let suggestions = if matches!(sheet, Sheet::Compose(_)) { self.mail.recipient_suggestions().to_vec() } else { Vec::new() };
        let highlighted = self.mail.recipient_field.highlighted();
        let l = sheet_layout(ui, w, h, sheet, if suggestions.is_empty() { matches.len() } else { suggestions.len() });
        let [px_, py, pw, ph] = l.panel;
        let rule = ui.pt(1.0).max(1.0);
        let radius = ui.pt(PANEL_RADIUS_PT);
        s.fill_rrect(px_ - rule, py - rule, pw + 2.0 * rule, ph + 2.0 * rule, radius + rule, SEPARATOR_C.now());
        s.fill_rrect(px_, py, pw, ph, radius, CARD_BG.now());

        let (title, action, form) = (sheet.title(), sheet.action(), sheet.form());
        put_centred(s, &mut self.labels, ui, title, MAC_BODY_PT, Weight::Semibold, LABEL.now(), px_ + pw / 2.0, py + ui.pt(SHEET_HEAD_PT) / 2.0);

        let pad = ui.pt(SPACE_S);
        let caret = ui.pt(1.0).max(1.0);
        let shown: &[crate::state::Field] = if matches!(sheet, Sheet::Question(_)) { &[] } else { &form.fields };
        for (i, field) in shown.iter().enumerate() {
            let r = l.fields[i];
            let focused = form.focus == i;
            if focused {
                s.fill_rrect(r[0] - rule, r[1] - rule, r[2] + 2.0 * rule, r[3] + 2.0 * rule, ui.pt(RADIUS_PT) + rule, colors::SYSTEM_BLUE.now());
            }
            s.fill_rrect(r[0], r[1], r[2], r[3], ui.pt(RADIUS_PT), FIELD_BG.now());
            let tx = r[0] + pad;
            put(s, &mut self.labels, ui, field.label, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), tx, r[1] + ui.pt(5.0));
            let value_y = r[1] + ui.pt(SPACE_XL);
            if field.label == "From" || field.choice {
                let hint = "Click to change";
                let hint_w = ui.measure(hint, MAC_FOOTNOTE_PT, Weight::Regular);
                put(s, &mut self.labels, ui, hint, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_3.now(), r[0] + r[2] - hint_w - pad, r[1] + ui.pt(5.0));
            }
            if is_multiline(field.label) {
                if let (true, Sheet::Compose(compose)) = (field.label == "Message", sheet) {
                    self.draw_styled_message(s, ui, compose, r, focused);
                    continue;
                }
                let text_w = r[2] - 2.0 * pad;
                let line_h = ui.pt(BODY_LINE_PT);
                if field.value.is_empty() {
                    put(s, &mut self.labels, ui, field.hint, MAC_BODY_PT, Weight::Regular, LABEL_3.now(), tx, value_y);
                    if focused {
                        s.fill_rect(px(tx), px(value_y), px(caret), px(ui.pt(MAC_BODY_PT)), colors::SYSTEM_BLUE.now());
                    }
                    continue;
                }
                let lines = wrap(ui, &field.value, text_w, MAC_BODY_PT, Weight::Regular);
                let room = ((r[1] + r[3] - value_y - ui.pt(SPACE_XS)) / line_h).floor().max(1.0) as usize;
                let start = lines.len().saturating_sub(room);
                let hilite = focused && form.selected();
                for (k, line) in lines[start..].iter().enumerate() {
                    let y = value_y + k as f32 * line_h;
                    if hilite {
                        s.fill_rect(px(tx), px(y), px(ui.measure(line, MAC_BODY_PT, Weight::Regular)), px(line_h), SELECTION.now());
                    }
                    put(s, &mut self.labels, ui, line, MAC_BODY_PT, Weight::Regular, if hilite { WHITE } else { LABEL.now() }, tx, y);
                }
                if focused {
                    let last = lines.last().map_or(0.0, |line| ui.measure(line, MAC_BODY_PT, Weight::Regular));
                    let cy = value_y + (lines.len() - start).saturating_sub(1) as f32 * line_h;
                    s.fill_rect(px(tx + last), px(cy), px(caret), px(ui.pt(MAC_BODY_PT)), colors::SYSTEM_BLUE.now());
                }
            } else {
                let (shown, color) = if field.value.is_empty() {
                    (field.hint.to_string(), LABEL_3.now())
                } else if field.secret {
                    ("*".repeat(field.value.chars().count()), LABEL.now())
                } else {
                    (field.value.clone(), LABEL.now())
                };
                let shown = fit(ui, &shown, MAC_BODY_PT, Weight::Regular, (r[2] - 2.0 * pad).max(0.0));
                // Select All: the whole text is on the selection colour, in white
                let hilite = focused && form.selected();
                if hilite {
                    s.fill_rect(px(tx), px(value_y), px(ui.measure(&shown, MAC_BODY_PT, Weight::Regular)), px(ui.pt(BODY_LINE_PT)), SELECTION.now());
                }
                put(s, &mut self.labels, ui, &shown, MAC_BODY_PT, Weight::Regular, if hilite { WHITE } else { color }, tx, value_y);
                if focused {
                    let end = if field.value.is_empty() { 0.0 } else { ui.measure(&shown, MAC_BODY_PT, Weight::Regular) };
                    s.fill_rect(px(tx + end), px(value_y), px(caret), px(ui.pt(MAC_BODY_PT)), colors::SYSTEM_BLUE.now());
                }
            }
        }

        if let (Sheet::Compose(compose), Some(note)) = (sheet, l.note) {
            let first = compose.signature.lines().next().unwrap_or("");
            let more = if compose.signature.lines().count() > 1 { " ..." } else { "" };
            let text = fit(ui, &format!("-- {first}{more}"), MAC_FOOTNOTE_PT, Weight::Regular, note[2]);
            put_mid(s, &mut self.labels, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_3.now(), note[0], note[1] + note[3] / 2.0);
        }
        if let (Sheet::Setup(_), Some(note)) = (sheet, l.note) {
            // the progress of the lookup or what it found, else what Add Account does with the address
            let (text, color) = if self.mail.status.is_empty() {
                ("Mail finds the servers from your address. Configure Manually lets you enter them.".to_string(), LABEL_3.now())
            } else {
                (self.mail.status.clone(), LABEL_2.now())
            };
            let line_h = note[3] / 2.0;
            for (k, line) in wrap(ui, &text, note[2], MAC_FOOTNOTE_PT, Weight::Regular).iter().take(2).enumerate() {
                put_mid(s, &mut self.labels, ui, line, MAC_FOOTNOTE_PT, Weight::Regular, color, note[0], note[1] + line_h * (k as f32 + 0.5));
            }
        }
        if let Sheet::Compose(compose) = sheet {
            for (file, (row, remove)) in compose.attachments.iter().zip(&l.files) {
                let name = format!("{} ({})", file.name, attachments::size_text(file.bytes.len()));
                let name = fit(ui, &name, MAC_FOOTNOTE_PT, Weight::Regular, (row[2] - remove[2]).max(0.0));
                put_mid(s, &mut self.labels, ui, &name, MAC_FOOTNOTE_PT, Weight::Regular, LABEL.now(), row[0], row[1] + row[3] / 2.0);
                put_centred(s, &mut self.labels, ui, "Remove", MAC_FOOTNOTE_PT, Weight::Regular, colors::SYSTEM_BLUE.now(), remove[0] + remove[2] / 2.0, remove[1] + remove[3] / 2.0);
            }
        }

        if let Sheet::Rules(_, names) = sheet {
            for (name, (row, remove)) in names.iter().zip(&l.files) {
                let text = fit(ui, name, MAC_FOOTNOTE_PT, Weight::Regular, (row[2] - remove[2]).max(0.0));
                put_mid(s, &mut self.labels, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL.now(), row[0], row[1] + row[3] / 2.0);
                put_centred(s, &mut self.labels, ui, "Remove", MAC_FOOTNOTE_PT, Weight::Regular, colors::SYSTEM_BLUE.now(), remove[0] + remove[2] / 2.0, remove[1] + remove[3] / 2.0);
            }
        }

        // Open Quickly: the mailboxes the field matches, the first one is the one Return opens
        for (index, (name, row)) in matches.iter().zip(&l.choices).enumerate() {
            if index == 0 {
                s.fill_rrect(row[0], row[1], row[2], row[3], ui.pt(RADIUS_PT), colors::SYSTEM_BLUE.now());
            }
            let color = if index == 0 { WHITE } else { LABEL.now() };
            let text = fit(ui, mailbox_title(name), MAC_BODY_PT, Weight::Regular, (row[2] - 2.0 * pad).max(0.0));
            put_mid(s, &mut self.labels, ui, &text, MAC_BODY_PT, Weight::Regular, color, row[0] + pad, row[1] + row[3] / 2.0);
        }

        // a recipient field: the people it could be, the one the arrow keys reached is highlighted
        for (index, (suggestion, row)) in suggestions.iter().zip(&l.choices).enumerate() {
            let on = highlighted == Some(index);
            if on {
                s.fill_rrect(row[0], row[1], row[2], row[3], ui.pt(RADIUS_PT), colors::SYSTEM_BLUE.now());
            }
            let (main, second) = if on { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
            let who = if suggestion.name == suggestion.address { suggestion.address.clone() } else { suggestion.name.clone() };
            let who = fit(ui, &who, MAC_BODY_PT, Weight::Regular, (row[2] / 2.0 - pad).max(0.0));
            put_mid(s, &mut self.labels, ui, &who, MAC_BODY_PT, Weight::Regular, main, row[0] + pad, row[1] + row[3] / 2.0);
            if suggestion.name != suggestion.address {
                let tag = match &suggestion.label {
                    Some(label) => format!("{label}  {}", suggestion.address),
                    None => suggestion.address.clone(),
                };
                let left = row[0] + row[2] / 2.0;
                let tag = fit(ui, &tag, MAC_FOOTNOTE_PT, Weight::Regular, (row[0] + row[2] - left - pad).max(0.0));
                put_mid(s, &mut self.labels, ui, &tag, MAC_FOOTNOTE_PT, Weight::Regular, second, left, row[1] + row[3] / 2.0);
            }
        }

        let (cr, pr) = (l.cancel, l.primary);
        if let Some(ar) = l.attach {
            let label = if matches!(sheet, Sheet::Rules(..)) { "Add Rule..." } else { "Attach..." };
            s.fill_rrect(ar[0], ar[1], ar[2], ar[3], ui.pt(RADIUS_PT), FIELD_BG.now());
            put_centred(s, &mut self.labels, ui, label, MAC_BODY_PT, Weight::Regular, LABEL.now(), ar[0] + ar[2] / 2.0, ar[1] + ar[3] / 2.0);
        }
        if let Some(dr) = l.discard {
            s.fill_rrect(dr[0], dr[1], dr[2], dr[3], ui.pt(RADIUS_PT), FIELD_BG.now());
            put_centred(s, &mut self.labels, ui, "Don't Save", MAC_BODY_PT, Weight::Regular, LABEL.now(), dr[0] + dr[2] / 2.0, dr[1] + dr[3] / 2.0);
        }
        s.fill_rrect(cr[0], cr[1], cr[2], cr[3], ui.pt(RADIUS_PT), FIELD_BG.now());
        put_centred(s, &mut self.labels, ui, "Cancel", MAC_BODY_PT, Weight::Regular, LABEL.now(), cr[0] + cr[2] / 2.0, cr[1] + cr[3] / 2.0);
        s.fill_rrect(pr[0], pr[1], pr[2], pr[3], ui.pt(RADIUS_PT), colors::SYSTEM_BLUE.now());
        put_centred(s, &mut self.labels, ui, action, MAC_BODY_PT, Weight::Semibold, WHITE, pr[0] + pr[2] / 2.0, pr[1] + pr[3] / 2.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unread_dot_has_a_gutter_before_the_sender() {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let ui = Ui::new(bifrost_app::kit::Font::parse(data).unwrap(), 2.0);
        // the dot is drawn at x0 + pad, DOT_PT wide (draw_list)
        let dot_right = 100.0 + ui.pt(SPACE_M) + ui.pt(DOT_PT);
        assert!(list_text_x(&ui, 100.0, 0) - dot_right >= ui.pt(SPACE_S), "a space between the dot and the first letter");
        assert_eq!(list_text_x(&ui, 100.0, 2) - list_text_x(&ui, 100.0, 0), 2.0 * ui.pt(INDENT_PT));
    }

    #[test]
    fn list_rows_say_muted_and_the_sender_s_priority() {
        assert_eq!(row_tag(false, Priority::Normal), "");
        assert_eq!(row_tag(true, Priority::Normal), "Muted");
        assert_eq!(row_tag(false, Priority::High), "High priority");
        assert_eq!(row_tag(true, Priority::Low), "Muted, Low priority");
    }

    #[test]
    fn all_headers_shows_the_header_block_only() {
        let raw = "From: Ana <ana@example.com>\r\nSubject: Fundur\r\n\r\nLine one\r\nLine two";
        assert_eq!(header_block(raw), "From: Ana <ana@example.com>\nSubject: Fundur");
        assert_eq!(header_block("Subject: no body"), "Subject: no body");
    }

    #[test]
    fn open_quickly_lists_the_best_matching_mailboxes_up_to_the_limit() {
        let mut mail = Mail::new(vec![]);
        mail.mailboxes = vec!["INBOX".into(), "Receipts".into(), "Work/Receipts".into(), "Sent".into()];
        assert!(choices_of(&mail).is_empty(), "no sheet, no list");
        mail.open_quickly();
        mail.sheet.as_mut().unwrap().form_mut().type_text("rec");
        assert_eq!(choices_of(&mail), ["Receipts", "Work/Receipts"]);
        mail.sheet.as_mut().unwrap().form_mut().type_text("zzz");
        assert!(choices_of(&mail).is_empty());
    }

    fn row_labels(list: &SourceList) -> Vec<(String, bool)> {
        list.items.iter().filter_map(|i| match i {
            SidebarItem::Row { label, enabled, .. } => Some((label.clone(), *enabled)),
            _ => None,
        }).collect()
    }

    #[test]
    fn the_sidebar_without_an_account_greys_the_standard_mailboxes_and_says_there_are_none() {
        let mail = Mail::new(vec![]);
        let (list, actions) = sidebar_model(&mail);
        let rows = row_labels(&list);
        for name in ["Inbox", "Drafts", "Sent", "Junk", "Trash"] {
            assert!(rows.contains(&(name.to_string(), false)), "{name} is greyed");
        }
        assert!(rows.contains(&("No Accounts".to_string(), false)));
        assert_eq!(actions.len(), 1, "only Add Account answers a tap");
        assert_eq!(actions[0].1, Side::Add);
        let headers: Vec<_> = list.items.iter().filter_map(|i| if let SidebarItem::Header(h) = i { Some(h.as_str()) } else { None }).collect();
        assert_eq!(headers, ["Favorites", "Mailboxes"]);
    }

    #[test]
    fn the_sidebar_enables_the_mailboxes_the_account_has() {
        let mut mail = Mail::new(vec![bifrost_app::controls::internet_accounts::Account::new(
            bifrost_app::controls::internet_accounts::Kind::Mail,
            "work",
            "Work",
            "user",
        )]);
        mail.mailboxes = vec!["INBOX".into(), "Sent Items".into(), "Deleted Items".into(), "Work".into()];
        let (list, actions) = sidebar_model(&mail);
        let rows = row_labels(&list);
        assert!(rows.contains(&("Inbox".to_string(), true)) && rows.contains(&("Sent".to_string(), true)) && rows.contains(&("Trash".to_string(), true)));
        assert!(rows.contains(&("Drafts".to_string(), false)) && rows.contains(&("Junk".to_string(), false)));
        assert!(actions.contains(&(1, Side::Mailbox("INBOX".into()))), "the first favourite opens the Inbox");
        assert!(actions.iter().any(|(_, s)| *s == Side::Mailbox("Sent Items".into())), "Sent opens the provider's name for it");
        let ids: Vec<u32> = list.items.iter().filter_map(|i| if let SidebarItem::Row { id, .. } = i { Some(*id) } else { None }).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(ids.len(), unique.len(), "every row has its own id");
    }

    #[test]
    fn mailboxes_wear_the_glyph_of_their_role() {
        assert_eq!(mailbox_symbol("INBOX"), "tray");
        assert_eq!(mailbox_symbol("sent items"), "paperplane");
        assert_eq!(mailbox_symbol("[Gmail]/Spam"), "xmark.bin");
        assert_eq!(mailbox_symbol("Deleted Items"), "trash");
        assert_eq!(mailbox_symbol("Work/Receipts"), "folder");
    }

    #[test]
    fn every_toolbar_button_but_the_filter_chip_has_a_glyph_the_kit_draws() {
        for button in [Button::Refresh, Button::Compose, Button::Reply, Button::ReplyAll, Button::Archive, Button::Trash] {
            let (symbol, _) = toolbar_glyph(&button).expect("a glyph");
            assert!(bifrost_app::glyphs::has(symbol), "{symbol}");
        }
        assert!(toolbar_glyph(&Button::Filter).is_none());
    }

    #[test]
    fn pane_regions_follow_the_columns() {
        let cols = [200.0, 300.0, 700.0];
        assert_eq!(pane_at(cols, 50.0, 10.0, 200.0), Region::Chrome);
        assert_eq!(pane_at(cols, 50.0, 210.0, 200.0), Region::List);
        assert_eq!(pane_at(cols, 50.0, 600.0, 200.0), Region::Reader);
        assert_eq!(pane_at(cols, 50.0, 210.0, 20.0), Region::Chrome, "the toolbar is chrome");
    }

    #[test]
    fn rectangles_include_their_left_and_top_edges_only() {
        assert!(inside([10.0, 10.0, 5.0, 5.0], 10.0, 10.0));
        assert!(!inside([10.0, 10.0, 5.0, 5.0], 15.0, 10.0));
        assert!(!inside([10.0, 10.0, 5.0, 5.0], 9.9, 12.0));
    }

    #[test]
    fn display_names_and_dates_are_short() {
        assert_eq!(display_name("Ana O <ana@example.com>"), "Ana O");
        assert_eq!(display_name("\"Bo\" <bo@example.com>"), "Bo");
        assert_eq!(display_name("bo@example.com"), "bo@example.com");
        assert_eq!(short_date("Tue, 13 Oct 2026 09:30:00 +0000"), "13 Oct");
        assert_eq!(short_date("13 Oct"), "13 Oct");
    }

    #[test]
    fn raw_source_keeps_the_headers_and_drops_carriage_returns() {
        assert_eq!(raw_text("Subject: x\r\nX-A:\t1\r\n\r\nbody\u{7}"), "Subject: x\nX-A:\t1\n\nbody");
    }

    #[test]
    fn mailbox_titles() {
        assert_eq!(mailbox_title("INBOX"), "Inbox");
        assert_eq!(mailbox_title("Sent"), "Sent");
    }
}
