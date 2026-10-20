//! The Help viewer: a floating window that reads the help books (`crate::shell::helpbook`). A sidebar holds the search
//! field and the current book's table of contents (its pages, then the headings of the open page); the main pane shows
//! the open page as Markdown (`md::classify` gives each line its block style). The toolbar has Back and Forward over a
//! history stack, the book popup and Done. `help://` links navigate inside the viewer; `http(s)` links open in the
//! browser. Help menu items call `Shell::open_help`. Like the character viewer it is an overlay that takes every touch
//! while it is up; typed text goes to its search field.
//!
//! Not yet: keyboard focus rules beyond "typing goes to the search field", and a scroll indicator. A link on a line
//! with several links follows the first one.

use std::path::Path;

use super::helpbook::{self, HelpIndex, Page};
use super::keyboard::Out;
use super::md::{self, Block};
use super::store::install::ROOT;
use super::ui::{Ui, Weight, CARD_BG, FIELD_BG, GRAY_DIM, LABEL, LABEL_2, LINK, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, MAC_TITLE1_PT, MAC_TITLE2_PT, SECONDARY, SEPARATOR, SIDEBAR_BG, SIDEBAR_SELECTED, WHITE};
use super::{Drawn, Effect, Event, Shell};
use crate::gfx::Surface;

/// The panel's size and top, in points and as a share of the screen height.
const PANEL_W_PT: f32 = 760.0;
const PANEL_H_PT: f32 = 540.0;
const TOP_SHARE: f32 = 0.12;
const MARGIN_PT: f32 = 16.0;
const RADIUS_PT: f32 = 13.0;
/// Toolbar height, the Back and Forward buttons, the Done label, and the gap between Back and Forward.
const TOOL_PT: f32 = 44.0;
const BTN_PT: f32 = 30.0;
const DONE_PT: f32 = 56.0;
const GAP_PT: f32 = 4.0;
/// Sidebar width, the side padding, the search field height, the sidebar row heights (one line and a search hit).
const SIDE_PT: f32 = 240.0;
const PAD_PT: f32 = 12.0;
const FIELD_PT: f32 = 28.0;
const ROW_PT: f32 = 26.0;
const HIT_ROW_PT: f32 = 44.0;
/// Indent of a heading row under its page, and of a quote in the page.
const INDENT_PT: f32 = 12.0;
const ROW_RADIUS_PT: f32 = 9.0;
/// The book popup's width.
const POPUP_PT: f32 = 220.0;
/// Most search hits the sidebar lists.
const RESULTS: usize = 40;
/// A touch that moves less than this is a tap, not a scroll.
const TAP_SLOP_PT: f32 = 8.0;

/// x, y, w, h in px, relative to the panel (or the screen where noted).
type Rect = (f32, f32, f32, f32);

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

/// A place in a book: the app, its page, and an optional heading anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub app: String,
    pub slug: String,
    pub anchor: Option<String>,
}

/// The history stack: the places visited, and the one on screen. Visiting a place drops the forward history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nav {
    stack: Vec<Location>,
    at: usize,
}

impl Nav {
    pub fn new(first: Location) -> Self {
        Self { stack: vec![first], at: 0 }
    }

    pub fn current(&self) -> &Location {
        &self.stack[self.at]
    }

    /// Shows `loc`. A visit to the place already on screen changes nothing.
    pub fn visit(&mut self, loc: Location) {
        if *self.current() == loc {
            return;
        }
        self.stack.truncate(self.at + 1);
        self.stack.push(loc);
        self.at = self.stack.len() - 1;
    }

    pub fn can_back(&self) -> bool {
        self.at > 0
    }

    pub fn can_forward(&self) -> bool {
        self.at + 1 < self.stack.len()
    }

    /// Steps back one place. False at the start of the history.
    pub fn back(&mut self) -> bool {
        if self.can_back() {
            self.at -= 1;
            true
        } else {
            false
        }
    }

    /// Steps forward one place. False at the end of the history.
    pub fn forward(&mut self) -> bool {
        if self.can_forward() {
            self.at += 1;
            true
        } else {
            false
        }
    }
}

/// One line of the table of contents: a page (indent 0) or a heading of the open page (indent 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TocItem {
    pub label: String,
    pub loc: Location,
    pub indent: u8,
}

/// The table of contents of `app`'s book: every page in file order, then the headings of the page `current_slug`.
pub fn toc(ix: &HelpIndex, app: &str, current_slug: &str) -> Vec<TocItem> {
    let pages = ix.pages_of(app);
    let mut out: Vec<TocItem> = pages
        .iter()
        .map(|p| TocItem { label: p.title.clone(), loc: Location { app: app.to_string(), slug: p.slug.clone(), anchor: None }, indent: 0 })
        .collect();
    if let Some(p) = pages.iter().find(|p| p.slug == current_slug) {
        out.extend(p.anchors.iter().map(|a| TocItem {
            label: a.title.clone(),
            loc: Location { app: app.to_string(), slug: p.slug.clone(), anchor: Some(a.id.clone()) },
            indent: 1,
        }));
    }
    out
}

/// Where a link goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// a `help://` link: navigates in the viewer
    Help(String),
    /// an `http://` or `https://` link: opens in the browser
    Web(String),
    /// anything else: not followed
    Other,
}

pub fn classify_link(url: &str) -> LinkKind {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    if lower.starts_with(helpbook::SCHEME) {
        return LinkKind::Help(u.to_string());
    }
    if (lower.starts_with("https://") || lower.starts_with("http://")) && !u.contains(char::is_whitespace) {
        return LinkKind::Web(u.to_string());
    }
    LinkKind::Other
}

/// Takes the `[text](url)` links out of a line: the text stays, the brackets and the url go. Returns the display text and
/// the first link's url, if any.
pub fn strip_links(line: &str) -> (String, Option<String>) {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut first = None;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            if let Some(close) = chars[i + 1..].iter().position(|&c| c == ']').map(|p| p + i + 1) {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = chars[close + 2..].iter().position(|&c| c == ')').map(|p| p + close + 2) {
                        out.extend(chars[i + 1..close].iter());
                        let url: String = chars[close + 2..end].iter().collect();
                        let url = url.trim();
                        if first.is_none() && !url.is_empty() {
                            first = Some(url.to_string());
                        }
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    (out, first)
}

/// The place a Help menu item or a link names: `help://app/#anchor` when the anchor exists, else the app's first page.
pub fn target_for(ix: &HelpIndex, app_id: &str, anchor: Option<&str>) -> Option<Location> {
    let wanted = anchor.filter(|a| !a.is_empty()).map(|a| format!("help://{app_id}/#{a}"));
    let found = wanted.and_then(|u| ix.resolve(&u)).or_else(|| ix.resolve(&format!("help://{app_id}")))?;
    Some(Location { app: found.app, slug: found.slug, anchor: found.anchor })
}

/// One line of a page as laid out: its position (content space, before scrolling), its text and style.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub y: f32,
    pub h: f32,
    pub x: f32,
    pub text: String,
    pub pt: f32,
    pub bold: bool,
    pub muted: bool,
    pub link: Option<String>,
}

/// Lays a page out in `width` px. Returns the lines and the height of the whole page.
pub fn layout(ui: &Ui, page: &Page, width: f32) -> (Vec<Placed>, f32) {
    let mut out = Vec::new();
    let mut y = 0.0f32;
    for line in page.body.lines() {
        let info = md::classify(line);
        let content: String = line.chars().skip(info.prefix).collect();
        let (text, link) = strip_links(&content);
        let text = text.replace("**", "").replace('`', "");
        let (mark, pt, bold, muted, x0) = match info.block {
            Block::Title => (String::new(), MAC_TITLE1_PT, true, false, 0.0),
            Block::Heading => (String::new(), MAC_TITLE2_PT, true, false, 0.0),
            Block::Subheading => (String::new(), MAC_HEADLINE_PT, true, false, 0.0),
            Block::Bullet => (String::from("\u{2022}  "), MAC_BODY_PT, false, false, 0.0),
            Block::Numbered(n) => (format!("{n}. "), MAC_BODY_PT, false, false, 0.0),
            Block::Check(done) => (String::from(if done { "[x]  " } else { "[ ]  " }), MAC_BODY_PT, false, false, 0.0),
            Block::Dashed => (String::from("\u{2013}  "), MAC_BODY_PT, false, false, 0.0),
            Block::Quote => (String::new(), MAC_BODY_PT, false, true, ui.pt(INDENT_PT)),
            Block::Body | Block::Mono => (String::new(), MAC_BODY_PT, false, false, 0.0),
        };
        if matches!(info.block, Block::Title | Block::Heading | Block::Subheading) && y > 0.0 {
            y += ui.pt(8.0);
        }
        let weight = if bold { Weight::Semibold } else { Weight::Regular };
        let full = format!("{mark}{text}");
        let lines = if full.trim().is_empty() { Vec::new() } else { ui.wrap(&full, pt, weight, (width - x0).max(1.0)) };
        if lines.is_empty() {
            // a blank line is a half-line of space
            y += ui.pt(MAC_BODY_PT * 0.75);
            continue;
        }
        let lh = ui.pt(pt) * 1.35;
        for seg in lines {
            out.push(Placed { y, h: lh, x: x0, text: seg, pt, bold, muted, link: link.clone() });
            y += lh;
        }
    }
    (out, y)
}

/// The viewer's state: the history, the search text, the book popup, the scroll, and a touch in progress.
#[derive(Debug)]
pub struct HelpViewer {
    index: HelpIndex,
    nav: Nav,
    query: String,
    popup: bool,
    scroll: f32,
    /// (y where the touch started, y of the last move, whether it has moved far enough to scroll)
    drag: Option<(f32, f32, bool)>,
}

/// What a touch on the panel lands on. Panel coordinates are not kept: a hit carries what it acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Hit {
    Done,
    Back,
    Forward,
    BookButton,
    Book(String),
    Side(Location),
    Field,
    Link(String),
    Content,
    /// inside the panel, where no control is
    Panel,
    Outside,
}

/// The panel's fixed measures in px, for the screen the panel is on.
struct Geo {
    pw: f32,
    ph: f32,
    tool: f32,
    btn: f32,
    pad: f32,
    side: f32,
    gap: f32,
    done_w: f32,
    field_h: f32,
    row_h: f32,
    hit_h: f32,
    popup_w: f32,
    radius: f32,
}

impl Geo {
    fn new(ui: &Ui, pw: f32, ph: f32) -> Self {
        Self {
            pw,
            ph,
            tool: ui.pt(TOOL_PT),
            btn: ui.pt(BTN_PT),
            pad: ui.pt(PAD_PT),
            side: ui.pt(SIDE_PT),
            gap: ui.pt(GAP_PT),
            done_w: ui.pt(DONE_PT),
            field_h: ui.pt(FIELD_PT),
            row_h: ui.pt(ROW_PT),
            hit_h: ui.pt(HIT_ROW_PT),
            popup_w: ui.pt(POPUP_PT),
            radius: ui.pt(RADIUS_PT),
        }
    }

    fn by(&self) -> f32 {
        (self.tool - self.btn) / 2.0
    }

    fn back(&self) -> Rect {
        (self.pad, self.by(), self.btn, self.btn)
    }

    fn forward(&self) -> Rect {
        (self.pad + self.btn + self.gap, self.by(), self.btn, self.btn)
    }

    fn book(&self) -> Rect {
        (self.side + self.pad, self.by(), self.popup_w, self.btn)
    }

    fn done(&self) -> Rect {
        (self.pw - self.pad - self.done_w, self.by(), self.done_w, self.btn)
    }

    fn field(&self) -> Rect {
        (self.pad, self.tool + self.pad, self.side - 2.0 * self.pad, self.field_h)
    }

    fn list_top(&self) -> f32 {
        self.tool + self.pad + self.field_h + self.pad
    }

    fn popup_row(&self, i: usize) -> Rect {
        (self.side + self.pad, self.tool + i as f32 * self.row_h, self.popup_w, self.row_h)
    }

    /// The main pane's text width.
    fn text_w(&self) -> f32 {
        self.pw - self.side - 2.0 * self.pad
    }
}

/// One row of the sidebar: a table-of-contents line, or a search hit (with its book's name under it).
struct SideRow {
    y: f32,
    h: f32,
    indent: f32,
    title: String,
    sub: Option<String>,
    loc: Location,
    selected: bool,
}

/// The panel: centred, below the top share of the screen, and as wide as the screen allows.
fn panel(ui: &Ui, w: i32, h: i32) -> Rect {
    let margin = ui.pt(MARGIN_PT);
    let pw = ui.pt(PANEL_W_PT).min(w as f32 - 2.0 * margin);
    let ph = ui.pt(PANEL_H_PT).min(h as f32 * (1.0 - TOP_SHARE) - margin);
    ((w as f32 - pw) / 2.0, h as f32 * TOP_SHARE, pw, ph)
}

/// Draws `text` vertically centred in `r`, centred or at the left edge.
fn put_in(s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, r: Rect, centred: bool) {
    let img = ui.text(text, pt, weight, WHITE);
    let x = if centred { r.0 + (r.2 - img.w as f32) / 2.0 } else { r.0 };
    let y = r.1 + (r.3 - img.h as f32) / 2.0;
    s.blit_colored(&img, x as i32, y as i32, color, 255);
}

impl HelpViewer {
    pub fn new(index: HelpIndex, first: Location) -> Self {
        Self { index, nav: Nav::new(first), query: String::new(), popup: false, scroll: 0.0, drag: None }
    }

    /// The page on screen.
    fn page(&self) -> Option<&Page> {
        let cur = self.nav.current();
        self.index.pages_of(&cur.app).into_iter().find(|p| p.slug == cur.slug)
    }

    /// Scrolls back to the top and closes the popup, after a navigation.
    fn reset_view(&mut self) {
        self.scroll = 0.0;
        self.popup = false;
    }

    /// The sidebar rows: search hits while there is a query, else the table of contents.
    fn sidebar(&self, ui: &Ui, g: &Geo) -> Vec<SideRow> {
        let mut rows = Vec::new();
        let mut y = g.list_top();
        let query = self.query.trim();
        if !query.is_empty() {
            for hit in self.index.search(query, RESULTS) {
                if y + g.hit_h > g.ph {
                    break;
                }
                let loc = Location { app: hit.app, slug: hit.slug, anchor: hit.anchor };
                rows.push(SideRow { y, h: g.hit_h, indent: 0.0, title: hit.title, sub: Some(hit.app_name), loc, selected: false });
                y += g.hit_h;
            }
        } else {
            let cur = self.nav.current();
            for item in toc(&self.index, &cur.app, &cur.slug) {
                if y + g.row_h > g.ph {
                    break;
                }
                let selected = item.loc == *cur;
                let indent = f32::from(item.indent) * ui.pt(INDENT_PT);
                rows.push(SideRow { y, h: g.row_h, indent, title: item.label, sub: None, loc: item.loc, selected });
                y += g.row_h;
            }
        }
        rows
    }

    /// The link under a touch in the main pane, if the line has one.
    fn link_at(&self, ui: &Ui, g: &Geo, x: f32, y: f32) -> Option<String> {
        let top = g.tool + g.pad - self.scroll;
        let placed = match self.page() {
            Some(p) => layout(ui, p, g.text_w()).0,
            None => return None,
        };
        placed
            .into_iter()
            .find(|p| {
                let yy = top + p.y;
                p.link.is_some() && x >= g.side + g.pad + p.x && x < g.pw - g.pad && y >= yy && y < yy + p.h
            })
            .and_then(|p| p.link)
    }

    /// The largest scroll that still shows the end of the page.
    fn max_scroll(&self, ui: &Ui, w: i32, h: i32) -> f32 {
        let (_, _, pw, ph) = panel(ui, w, h);
        let g = Geo::new(ui, pw, ph);
        let view = ph - g.tool - g.pad;
        match self.page() {
            Some(p) => (layout(ui, p, g.text_w()).1 - view).max(0.0),
            None => 0.0,
        }
    }

    fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Hit {
        let (px, py, pw, ph) = panel(ui, w, h);
        if !inside((px, py, pw, ph), x, y) {
            return Hit::Outside;
        }
        let (x, y) = (x - px, y - py);
        let g = Geo::new(ui, pw, ph);
        if inside(g.done(), x, y) {
            return Hit::Done;
        }
        if self.popup {
            for (i, (app, _)) in self.index.books().iter().enumerate() {
                if inside(g.popup_row(i), x, y) {
                    return Hit::Book(app.to_string());
                }
            }
        }
        if inside(g.back(), x, y) {
            return Hit::Back;
        }
        if inside(g.forward(), x, y) {
            return Hit::Forward;
        }
        if inside(g.book(), x, y) {
            return Hit::BookButton;
        }
        if x < g.side {
            if inside(g.field(), x, y) {
                return Hit::Field;
            }
            for row in self.sidebar(ui, &g) {
                if inside((0.0, row.y, g.side, row.h), x, y) {
                    return Hit::Side(row.loc);
                }
            }
            return Hit::Panel;
        }
        if y > g.tool {
            if let Some(url) = self.link_at(ui, &g, x, y) {
                return Hit::Link(url);
            }
            return Hit::Content;
        }
        Hit::Panel
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        let (x, y, pw, ph) = panel(ui, w, h);
        let g = Geo::new(ui, pw, ph);
        let at = |r: Rect| -> Rect { (x + r.0, y + r.1, r.2, r.3) };
        s.fill_rrect(x, y, pw, ph, g.radius, CARD_BG.now());
        s.stroke_rrect(x, y, pw, ph, g.radius, 1.0, SEPARATOR);

        // toolbar: Back, Forward, the book popup button and Done
        let cur = self.nav.current();
        let back = if self.nav.can_back() { LINK } else { GRAY_DIM };
        let forward = if self.nav.can_forward() { LINK } else { GRAY_DIM };
        put_in(s, ui, "<", MAC_BODY_PT, Weight::Semibold, back, at(g.back()), true);
        put_in(s, ui, ">", MAC_BODY_PT, Weight::Semibold, forward, at(g.forward()), true);
        let book_name = self.index.app_name(cur.app.as_str()).unwrap_or(cur.app.as_str());
        put_in(s, ui, book_name, MAC_BODY_PT, Weight::Semibold, LABEL.now(), at(g.book()), false);
        put_in(s, ui, "Done", MAC_BODY_PT, Weight::Semibold, LINK, at(g.done()), true);
        s.fill_rrect(x, y + g.tool, pw, 1.0, 0.0, SEPARATOR);

        // sidebar: the search field, then the rows
        s.fill_rrect(x, y + g.tool, g.side, ph - g.tool, 0.0, SIDEBAR_BG.now());
        s.fill_rrect(x + g.side, y + g.tool, 1.0, ph - g.tool, 0.0, SEPARATOR);
        let f = at(g.field());
        s.fill_rrect(f.0, f.1, f.2, f.3, f.3 / 2.0, FIELD_BG.now());
        let field_text = (f.0 + g.pad, f.1, f.2 - 2.0 * g.pad, f.3);
        if self.query.is_empty() {
            put_in(s, ui, "Search", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), field_text, false);
        } else {
            let shown = ui.fit(&self.query, MAC_BODY_PT, field_text.2);
            put_in(s, ui, &shown, MAC_BODY_PT, Weight::Regular, LABEL.now(), field_text, false);
        }
        for row in self.sidebar(ui, &g) {
            let r = at((0.0, row.y, g.side, row.h));
            let (title_c, sub_c) = if row.selected {
                s.fill_rrect(r.0 + g.pad / 2.0, r.1, r.2 - g.pad, r.3, ui.pt(ROW_RADIUS_PT), SIDEBAR_SELECTED);
                (WHITE, WHITE)
            } else {
                (LABEL.now(), LABEL_2.now())
            };
            let text_x = r.0 + g.pad + row.indent;
            let text_w = r.2 - 2.0 * g.pad - row.indent;
            match &row.sub {
                Some(sub) => {
                    let title = ui.fit(&row.title, MAC_BODY_PT, text_w);
                    let split = r.3 * 0.55;
                    put_in(s, ui, &title, MAC_BODY_PT, Weight::Semibold, title_c, (text_x, r.1, text_w, split), false);
                    put_in(s, ui, sub, MAC_FOOTNOTE_PT, Weight::Regular, sub_c, (text_x, r.1 + split, text_w, r.3 - split), false);
                }
                None => {
                    let title = ui.fit(&row.title, MAC_BODY_PT, text_w);
                    let weight = if row.indent == 0.0 { Weight::Semibold } else { Weight::Regular };
                    put_in(s, ui, &title, MAC_BODY_PT, weight, title_c, (text_x, r.1, text_w, r.3), false);
                }
            }
        }

        // the page
        if let Some(page) = self.page() {
            let top = g.tool + g.pad - self.scroll;
            for p in layout(ui, page, g.text_w()).0 {
                let yy = top + p.y;
                if yy < g.tool || yy + p.h > ph {
                    continue;
                }
                let color = if p.link.is_some() {
                    LINK
                } else if p.muted {
                    SECONDARY
                } else {
                    LABEL.now()
                };
                let weight = if p.bold { Weight::Semibold } else { Weight::Regular };
                let r = at((g.side + g.pad + p.x, yy, g.text_w() - p.x, p.h));
                put_in(s, ui, &p.text, p.pt, weight, color, r, false);
            }
        }

        // the book popup, over the page
        if self.popup {
            let books = self.index.books();
            let card = at((g.side + g.pad, g.tool, g.popup_w, books.len() as f32 * g.row_h));
            s.fill_rrect(card.0, card.1, card.2, card.3, ui.pt(ROW_RADIUS_PT), CARD_BG.now());
            s.stroke_rrect(card.0, card.1, card.2, card.3, ui.pt(ROW_RADIUS_PT), 1.0, SEPARATOR);
            for (i, (app, name)) in books.iter().enumerate() {
                let r = at(g.popup_row(i));
                let (c, on) = if *app == cur.app.as_str() {
                    s.fill_rrect(r.0 + 2.0, r.1, r.2 - 4.0, r.3, ui.pt(ROW_RADIUS_PT), SIDEBAR_SELECTED);
                    (WHITE, true)
                } else {
                    (LABEL.now(), false)
                };
                let weight = if on { Weight::Semibold } else { Weight::Regular };
                put_in(s, ui, name, MAC_BODY_PT, weight, c, (r.0 + g.pad, r.1, r.2 - 2.0 * g.pad, r.3), false);
            }
        }
    }
}

impl Shell {
    /// Opens the Help viewer on `app_id`'s book, at `anchor` when it has one, else on its first page. An app with no
    /// help book opens nothing.
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub(super) fn open_help(&mut self, app_id: &str, anchor: Option<&str>) {
        let opened = helpbook::with_index(Path::new(ROOT), |ix: &HelpIndex| -> Option<HelpViewer> {
            let first = target_for(ix, app_id, anchor)?;
            Some(HelpViewer::new(ix.clone(), first))
        });
        if let Some(v) = opened {
            self.help_viewer = Some(v);
            self.drawn = Drawn::Nothing;
            self.dirty = true;
        }
    }

    /// The app whose help the Help menu offers: the Settings book when Settings is on screen, or the book of the
    /// installed app in front. None when that app has no book.
    pub(super) fn help_book(&self) -> Option<(String, String)> {
        let id = match self.foreign_front_app_id() {
            Some(id) => id,
            None => match self.screen {
                super::Screen::App(super::AppId::Settings) => String::from("is.olibuijr.settings"),
                _ => return None,
            },
        };
        let name = helpbook::with_index(Path::new(ROOT), |ix| ix.app_name(&id).map(str::to_string))?;
        Some((id, name))
    }

    /// The manifest id of the installed app whose window is in front, if one is.
    #[cfg(feature = "desktop-wayland")]
    fn foreign_front_app_id(&self) -> Option<String> {
        let d = self.desk.as_ref()?;
        let win = d.native_windows.iter().find(|w| w.active)?;
        super::apphost::installed().iter().find(|a| a.manifest.wayland_app_id == win.app_id).map(|a| a.manifest.id.clone())
    }
    /// Without the desktop there are no foreign windows, so no installed app is in front.
    #[cfg(not(feature = "desktop-wayland"))]
    fn foreign_front_app_id(&self) -> Option<String> {
        None
    }

    /// A touch while the viewer is up. A release on a control acts; a drag in the page scrolls it.
    pub(super) fn help_viewer_event(&mut self, ev: Event) -> Vec<Effect> {
        let (w, h) = (self.w, self.h);
        let slop = self.ui.pt(TAP_SLOP_PT);
        let tap = {
            let Some(v) = self.help_viewer.as_mut() else { return vec![] };
            match ev {
                Event::Down(x, y) => {
                    let on_page = matches!(v.hit(&self.ui, w, h, x, y), Hit::Content | Hit::Link(_));
                    v.drag = on_page.then_some((y, y, false));
                    None
                }
                Event::Move(_, y) => {
                    if let Some((start, last, moved)) = v.drag {
                        let moved = moved || (y - start).abs() > slop;
                        if moved {
                            let max = v.max_scroll(&self.ui, w, h);
                            v.scroll = (v.scroll - (y - last)).clamp(0.0, max);
                        }
                        v.drag = Some((start, y, moved));
                    }
                    None
                }
                Event::Up(x, y) => {
                    let dragged = v.drag.take().is_some_and(|d| d.2);
                    if dragged {
                        None
                    } else {
                        Some(v.hit(&self.ui, w, h, x, y))
                    }
                }
                _ => None,
            }
        };
        self.drawn = Drawn::Nothing;
        self.dirty = true;
        if let Some(hit) = tap {
            self.help_viewer_hit(hit);
        }
        vec![]
    }

    fn help_viewer_hit(&mut self, hit: Hit) {
        if hit == Hit::Done {
            self.help_viewer = None;
            return;
        }
        // a web link leaves the viewer for the browser
        if let Hit::Link(url) = &hit {
            if let LinkKind::Web(u) = classify_link(url) {
                let t = self.last_input;
                if let Err(e) = self.desktop_open_url(&u, t) {
                    eprintln!("BifrOSt help link: {e}");
                }
                return;
            }
        }
        let Some(v) = self.help_viewer.as_mut() else { return };
        match hit {
            Hit::Back => {
                if v.nav.back() {
                    v.reset_view();
                }
            }
            Hit::Forward => {
                if v.nav.forward() {
                    v.reset_view();
                }
            }
            Hit::BookButton => v.popup = !v.popup,
            Hit::Book(app) => {
                let first = v.index.pages_of(&app).first().map(|p| p.slug.clone());
                if let Some(slug) = first {
                    v.nav.visit(Location { app, slug, anchor: None });
                    v.reset_view();
                }
                v.popup = false;
            }
            Hit::Side(loc) => {
                v.nav.visit(loc);
                v.query.clear();
                v.reset_view();
            }
            Hit::Link(url) => {
                if let LinkKind::Help(u) = classify_link(&url) {
                    if let Some(t) = v.index.resolve(&u) {
                        v.nav.visit(Location { app: t.app, slug: t.slug, anchor: t.anchor });
                        v.reset_view();
                    }
                }
            }
            Hit::Done | Hit::Field | Hit::Content | Hit::Panel | Hit::Outside => {}
        }
    }

    /// Typed text goes to the viewer's search field while the viewer is up (Hide still hides the keyboard).
    pub(super) fn help_viewer_key(&mut self, out: Out) {
        let Some(v) = self.help_viewer.as_mut() else { return };
        match out {
            Out::Text(s) => v.query.push_str(&s),
            Out::Backspace => {
                v.query.pop();
            }
            _ => return,
        }
        self.drawn = Drawn::Nothing;
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "is.olibuijr.files.desktop";

    fn page(app: &str, slug: &str, text: &str) -> Page {
        helpbook::parse_page(app, slug, text).unwrap()
    }

    fn index() -> HelpIndex {
        let mut ix = HelpIndex::new();
        ix.set_book(
            APP,
            "Files",
            vec![
                page(APP, "open", "---\ntitle: Open a file\n---\n## Open a file {#open}\nDouble click a file.\n## Save a copy {#copy}\nChoose [Duplicate](help://is.olibuijr.files.desktop/copy).\n"),
                page(APP, "tags", "---\ntitle: Tags\nkeywords: [label]\n---\n## Colour tags {#colour}\nGive a file a tag.\n"),
            ],
        );
        ix.set_book("is.olibuijr.textedit.desktop", "TextEdit", vec![page("is.olibuijr.textedit.desktop", "format", "# Format text\nUse the Format menu.\n")]);
        ix
    }

    fn loc(app: &str, slug: &str, anchor: Option<&str>) -> Location {
        Location { app: app.into(), slug: slug.into(), anchor: anchor.map(str::to_string) }
    }

    #[test]
    fn history_steps_back_and_forward_and_a_visit_drops_the_forward_part() {
        let a = loc(APP, "open", None);
        let b = loc(APP, "tags", None);
        let c = loc(APP, "open", Some("copy"));
        let mut nav = Nav::new(a.clone());
        assert!(!nav.can_back() && !nav.can_forward());
        assert!(!nav.back(), "nothing before the first place");
        nav.visit(b.clone());
        nav.visit(c.clone());
        assert_eq!(nav.current(), &c);
        assert!(nav.back());
        assert_eq!(nav.current(), &b);
        assert!(nav.can_forward());
        nav.visit(a.clone());
        assert!(!nav.can_forward(), "a new visit drops the forward history");
        assert!(nav.back());
        assert_eq!(nav.current(), &b);
        assert!(nav.forward());
        assert_eq!(nav.current(), &a);
        assert!(!nav.forward());
    }

    #[test]
    fn visiting_the_place_on_screen_changes_nothing() {
        let a = loc(APP, "open", None);
        let mut nav = Nav::new(a.clone());
        nav.visit(a.clone());
        assert!(!nav.can_back(), "no history entry for the same place");
        nav.visit(loc(APP, "tags", None));
        nav.visit(loc(APP, "tags", None));
        assert!(nav.back());
        assert!(!nav.back(), "one entry for the repeated visit");
    }

    #[test]
    fn toc_lists_pages_then_the_headings_of_the_open_page() {
        let ix = index();
        let items = toc(&ix, APP, "open");
        let labels: Vec<(&str, u8)> = items.iter().map(|i| (i.label.as_str(), i.indent)).collect();
        assert_eq!(labels, vec![("Open a file", 0), ("Tags", 0), ("Open a file", 1), ("Save a copy", 1)]);
        assert_eq!(items[2].loc, loc(APP, "open", Some("open")));
        assert_eq!(items[3].loc, loc(APP, "open", Some("copy")));
        assert_eq!(items[1].loc, loc(APP, "tags", None));
        assert_eq!(toc(&ix, APP, "tags").len(), 3, "the headings follow the open page only");
        assert!(toc(&ix, "nobody.app", "x").is_empty());
    }

    #[test]
    fn links_are_classified_by_scheme() {
        assert_eq!(classify_link("help://is.olibuijr.files.desktop/copy"), LinkKind::Help("help://is.olibuijr.files.desktop/copy".into()));
        assert_eq!(classify_link(" https://example.com/a "), LinkKind::Web("https://example.com/a".into()));
        assert_eq!(classify_link("HTTP://example.com"), LinkKind::Web("HTTP://example.com".into()));
        assert_eq!(classify_link("https://bad url"), LinkKind::Other, "a url with a space is not followed");
        assert_eq!(classify_link("file:///etc/passwd"), LinkKind::Other);
        assert_eq!(classify_link("javascript:alert(1)"), LinkKind::Other);
    }

    #[test]
    fn link_markup_keeps_its_text_and_yields_the_first_url() {
        let (text, url) = strip_links("Choose [Duplicate](help://a/copy) or [Share](https://x.y) now");
        assert_eq!(text, "Choose Duplicate or Share now");
        assert_eq!(url.as_deref(), Some("help://a/copy"));
        assert_eq!(strip_links("no links [here] (really)"), ("no links [here] (really)".to_string(), None));
        assert_eq!(strip_links("[open").0, "[open", "an unclosed bracket stays text");
        assert_eq!(strip_links("[empty]()").1, None, "an empty url is no link");
    }

    #[test]
    fn help_menu_target_resolves_an_anchor_and_falls_back_to_the_first_page() {
        let ix = index();
        assert_eq!(target_for(&ix, APP, Some("colour")), Some(loc(APP, "tags", Some("colour"))));
        assert_eq!(target_for(&ix, APP, Some("copy")), Some(loc(APP, "open", Some("copy"))));
        assert_eq!(target_for(&ix, APP, Some("missing")), Some(loc(APP, "open", None)), "an unknown anchor opens the first page");
        assert_eq!(target_for(&ix, APP, None), Some(loc(APP, "open", None)));
        assert_eq!(target_for(&ix, APP, Some("")), Some(loc(APP, "open", None)));
        assert_eq!(target_for(&ix, "nobody.app", Some("copy")), None, "an app without a book opens nothing");
    }

    #[test]
    fn the_book_list_names_every_installed_book() {
        let ix = index();
        assert_eq!(ix.books(), vec![("is.olibuijr.files.desktop", "Files"), ("is.olibuijr.textedit.desktop", "TextEdit")]);
    }
}
