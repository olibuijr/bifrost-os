//! An bifrost 15 grouped-inset list page: navigation bar with back button,
//! large title that collapses into the bar on scroll, inset groups with
//! headers and footers, icon tiles, values, chevrons, switches, checkmarks,
//! sliders and the battery graph. Scrolls with a fling and rubber band.

use super::battery::Graph;
use super::icons::{self, Icon, BLUE, RED};
use super::pages::{Action, PageId};
use super::ui::{home_zone, Ui, Weight, AVATAR_BG, AVATAR_FG, CARD_BG, DETAIL_BG, LABEL, LABEL_2, ROW_PRESSED, SECONDARY, SEPARATOR_C, WHITE, MAC_BODY_PT, MAC_HEADLINE_PT, MAC_TITLE3_PT, MATERIAL_MENU, MENU_HIGHLIGHT, MENU_RADIUS_PT};
use super::controls::{self, draw_slider, draw_switch, switch_progress, KNOB_PT, SWITCH_H_PT, SWITCH_MS, SWITCH_W_PT};
use crate::deildu::scroll::Scroller;
use crate::gfx::{pack, Image, Surface};
use crate::scene::STATUS_PT;
use crate::status::draw_wifi_bars;

pub const MARGIN_PT: f32 = 40.0;
pub const ROW_PT: f32 = 49.0;
pub const SLIDER_ROW_PT: f32 = 56.0;
pub const GRAPH_ROW_PT: f32 = 236.0;
pub const BAR_ROW_PT: f32 = 76.0;
pub const CHART_ROW_PT: f32 = 230.0;
/// Space above a group; a header sits inside a taller one.
pub const GROUP_GAP_PT: f32 = 11.0;
const HEADER_PT: f32 = 26.0;
/// The room a row's leading check (the joined network) takes before its label.
const LEADING_CHECK_PT: f32 = 26.0;
/// A row label wraps onto at most this many lines before its last one ends with an ellipsis.
const LABEL_LINES: usize = 2;
const FOOTER_LINE_PT: f32 = 18.0;
const FOOTER_GAP_PT: f32 = 8.0;
pub const RADIUS_PT: f32 = 13.0;
/// The navigation bar below the status bar.
pub const NAV_PT: f32 = 44.0;
/// The tablet navigation bar is taller than the phone's.
pub const BIFROST_NAV_PT: f32 = 50.0;
/// bifrost Settings sidebar: width, side margin of its groups, the local account card.
pub const SIDEBAR_PT: f32 = bifrost_ui::prefs_layout::m::SIDEBAR_W;
pub const SIDEBAR_MARGIN_PT: f32 = bifrost_ui::prefs_layout::m::SIDEBAR_INSET;
/// side margin of the detail pane's cards
pub const DETAIL_MARGIN_PT: f32 = bifrost_ui::prefs_layout::m::DETAIL_MARGIN;
const SIDEBAR_CARD_PT: f32 = bifrost_ui::prefs_layout::m::ACCOUNT_H;
const SIDEBAR_ROW_PT: f32 = bifrost_ui::prefs_layout::m::SIDEBAR_ROW_PITCH;
const SIDEBAR_SEL_H_PT: f32 = bifrost_ui::prefs_layout::m::SIDEBAR_ROW_H;
const SIDEBAR_GROUP_GAP_PT: f32 = bifrost_ui::prefs_layout::m::SIDEBAR_GROUP_GAP;
/// gap between the account block and the first sidebar row
const SIDEBAR_ACCOUNT_GAP_PT: f32 = 17.0;
/// Height of the large-title area under the bar.
pub const LARGE_TITLE_PT: f32 = 52.0;
pub const HIGHLIGHT: u32 = pack(58, 58, 60, 255);
const TEXT_INSET_PT: f32 = 16.0;

/// What sits at the right edge of a row.
#[derive(Clone, Debug, PartialEq)]
pub enum Accessory {
    None,
    Chevron,
    Switch(bool),
    Check(bool),
    /// the ⓘ detail button
    Info,
    /// a Wi-Fi network's padlock and signal fan
    Wifi { secure: bool, bars: u8 },
    /// a trailing pop-up button: the chosen label and up/down chevrons (the row's `popup` holds the choices)
    Popup,
    /// a segmented control across the row (the row's `segments` holds the choices); the row has no label of its own
    Segmented,
}

/// A trailing pop-up button's choices and the one chosen. Tapping the row opens a menu of the choices under the row;
/// choosing one sends the row's action with the choice's index (see `pages::chosen`).
#[derive(Clone, Debug, PartialEq)]
pub struct Popup {
    pub choices: Vec<String>,
    pub selected: usize,
}

/// The ends of a slider.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ends {
    /// Keyboard > Key Repeat > Delay Until Repeat: Long and Short
    Delay,
    /// Keyboard > Key Repeat > Key Repeat: Slow and Fast
    Rate,
    Brightness,
    Volume,
    TextSize,
    Warmth,
    /// Liquid Glass: an outlined pane (clear) and a filled one (tinted)
    Glass,
    /// Mouse and Trackpad tracking and scrolling speed: slower and faster
    Speed,
    /// Accessibility > Cursor Size: Normal and Large
    Cursor,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Row,
    Slider { ends: Ends, value: f32 },
    Graph(Graph),
    /// the bifrost Storage usage bar
    Bar(super::storage_page::Bar),
    /// the Screen Time usage chart
    Chart(super::screentime::chart::Chart),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tint {
    Plain,
    Blue,
    Red,
    /// greyed out, not tappable
    Dim,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub icon: Option<Icon>,
    /// a drawn tile for detail rows (the sidebar uses `icon`)
    pub glyph: Option<super::row_glyphs::Glyph>,
    pub label: String,
    pub value: String,
    pub accessory: Accessory,
    pub body: Body,
    pub action: Action,
    pub tint: Tint,
    /// a blue ✓ before the label (the joined network)
    pub leading_check: bool,
    /// bifrost red badge before the chevron (0 = none)
    pub badge: usize,
    /// the leading app icon: an installed app's id, drawn at the icon token (blank for a built-in app)
    pub app_icon: Option<String>,
    /// the pop-up button's choices (set with `popup`, drawn by `Accessory::Popup`)
    pub popup: Option<Popup>,
    /// the segmented control's choices (set with `segmented`, drawn by `Accessory::Segmented`)
    pub segments: Option<Popup>,
}

impl Item {
    /// A plain row with a label.
    pub fn new(label: impl Into<String>) -> Self {
        Item { icon: None, glyph: None, label: label.into(), value: String::new(), accessory: Accessory::None, body: Body::Row, action: Action::None, tint: Tint::Plain, leading_check: false, badge: 0, app_icon: None, popup: None, segments: None }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn glyph(mut self, glyph: super::row_glyphs::Glyph) -> Self {
        self.glyph = Some(glyph);
        self
    }

    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }

    /// A row that pushes `page`, with a chevron.
    pub fn link(mut self, page: PageId) -> Self {
        self.accessory = Accessory::Chevron;
        self.action = Action::Push(page);
        self
    }

    pub fn switch(mut self, on: bool, action: Action) -> Self {
        self.accessory = Accessory::Switch(on);
        self.action = action;
        self
    }

    pub fn check(mut self, on: bool, action: Action) -> Self {
        self.accessory = Accessory::Check(on);
        self.action = action;
        self
    }

    /// A leading app icon for the installed app `app_id`.
    pub fn app_icon(mut self, app_id: impl Into<String>) -> Self {
        self.app_icon = Some(app_id.into());
        self
    }

    /// A trailing pop-up button showing `choices[selected]`. The row's action is sent with the chosen index.
    pub fn popup(mut self, choices: Vec<String>, selected: usize, action: Action) -> Self {
        self.accessory = Accessory::Popup;
        self.popup = Some(Popup { choices, selected });
        self.action = action;
        self
    }

    /// A segmented control across the row for a view switch (Day or Week): `choices[selected]` is filled. The row's action is
    /// sent with the tapped segment's index (see `pages::chosen`). The row's label is for the screen reader only.
    pub fn segmented(mut self, choices: Vec<String>, selected: usize, action: Action) -> Self {
        self.accessory = Accessory::Segmented;
        self.segments = Some(Popup { choices, selected });
        self.action = action;
        self
    }

    pub fn badge(mut self, n: usize) -> Self {
        self.badge = n;
        self
    }

    pub fn action(mut self, action: Action) -> Self {
        self.action = action;
        self
    }

    pub fn tint(mut self, tint: Tint) -> Self {
        self.tint = tint;
        self
    }

    pub fn slider(ends: Ends, value: f32, action: Action) -> Self {
        Item { body: Body::Slider { ends, value }, action, ..Item::new("") }
    }

    pub fn chart(chart: super::screentime::chart::Chart) -> Self {
        Item { body: Body::Chart(chart), ..Item::new("") }
    }

    pub fn graph(graph: Graph) -> Self {
        Item { body: Body::Graph(graph), ..Item::new("") }
    }

    /// Whether a tap does something (and the row highlights).
    pub fn tappable(&self) -> bool {
        self.action != Action::None && self.tint != Tint::Dim
    }

    fn height_pt(&self) -> f32 {
        match self.body {
            Body::Row => ROW_PT,
            Body::Slider { .. } => SLIDER_ROW_PT,
            Body::Graph(_) => GRAPH_ROW_PT,
            Body::Bar(_) => BAR_ROW_PT,
            Body::Chart(_) => CHART_ROW_PT,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Group {
    pub header: Option<String>,
    pub footer: Option<String>,
    pub items: Vec<Item>,
}

impl Group {
    pub fn new(items: Vec<Item>) -> Self {
        Group { header: None, footer: None, items }
    }

    pub fn header(mut self, h: &str) -> Self {
        self.header = Some(h.into());
        self
    }

    pub fn footer(mut self, f: &str) -> Self {
        self.footer = Some(f.into());
        self
    }
}

/// Everything a page shows, before rendering.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub id: PageId,
    pub title: String,
    /// a large title that collapses on scroll (the root) or an inline one
    pub large: bool,
    pub groups: Vec<Group>,
}

/// What a touch landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hit {
    Back,
    /// group, item
    Row(usize, usize),
    Slider(usize, usize),
    /// a choice in the open pop-up menu (its index)
    Choice(usize),
    /// a segment of a segmented row: group, item, segment
    Segment(usize, usize, usize),
    /// a touch outside the open pop-up menu: it closes
    Dismiss,
}

struct RowGeom {
    g: usize,
    i: usize,
    /// top in content px (unscrolled, from the screen top)
    y: f32,
    h: f32,
}

struct RowImages {
    /// the label's lines: one, or a wrapped second line when it would run under the trailing control
    label: Vec<Image>,
    value: Option<Image>,
    /// the leading app icon at the icon token, when the app has one installed
    icon: Option<Image>,
    /// a pop-up button's chosen label (white, drawn in the label colour)
    popup: Option<Image>,
}

/// The pop-up menu open on a row: the row, its menu, and the menu's label images and width in px.
struct OpenPopup {
    g: usize,
    i: usize,
    menu: bifrost_ui::menu::Menu,
    labels: Vec<Image>,
    w: f32,
}

/// Menu rows, separators and padding in pt (the desktop menus' metrics), the text inset and the width of a menu.
const POPUP_ROW_PT: f32 = 24.0;
const POPUP_SEP_PT: f32 = 11.0;
const POPUP_PAD_PT: f32 = 5.0;
const POPUP_MIN_W_PT: f32 = 160.0;
/// the label's inset from a menu's left edge (the checkmark sits in front of it), and the space after the widest label
const POPUP_TEXT_PT: f32 = 28.0;
const POPUP_TAIL_PT: f32 = 16.0;
/// a pop-up button's chevron block at the row's right edge, and the gap between the label and it
const POPUP_CHEV_PT: f32 = 10.0;
const POPUP_GAP_PT: f32 = 6.0;

/// Whether a row has something before its label: an icon, a symbol tile or an app icon.
fn has_leading(item: &Item) -> bool {
    item.icon.is_some() || item.glyph.is_some() || item.app_icon.is_some()
}

/// A row's leading slot in px: the icon's left edge and the label's left edge (the icon's slot plus its gap when the
/// row has one, else the plain text inset).
fn leading_x(ui: &Ui, margin: f32, item: &Item, sidebar: bool) -> (f32, f32) {
    let (icon_pt, icon_x, gap_pt) = row_icon(sidebar);
    if has_leading(item) {
        (margin + ui.pt(icon_x), margin + ui.pt(icon_x + icon_pt + gap_pt))
    } else {
        (margin + ui.pt(TEXT_INSET_PT), margin + ui.pt(TEXT_INSET_PT))
    }
}

/// A pop-up button's geometry in px for a row whose right edge is `right` and whose label is `label_w` wide: the
/// chevrons' centre and the label's left edge. The chevron block is the row's right edge; the label sits before it.
fn popup_layout(ui: &Ui, right: f32, label_w: f32) -> (f32, f32) {
    let label_left = right - ui.pt(POPUP_CHEV_PT + POPUP_GAP_PT) - label_w;
    (right - ui.pt(POPUP_CHEV_PT / 2.0), label_left)
}

/// A group header in headline case. Specs write headers in capitals ("DOCK POSITION"); the page shows them as title-case
/// headlines ("Dock Position"). A word with a lower-case letter in it (a device name, "v1.2") is left as written, and so
/// are the acronyms in `ACRONYMS`. A small word (article, conjunction, preposition) stays low unless it opens or ends the
/// header; verbs such as "is" are capitalised.
pub fn headline_case(text: &str) -> String {
    const ACRONYMS: [&str; 28] = ["TV", "OS", "ID", "IP", "UI", "PC", "USB", "HID", "DNS", "VPN", "DHCP", "URL", "SSID", "VLAN", "LAN", "GPU", "CPU", "RAM", "SSD", "HDD", "PDF", "API", "TLS", "SSH", "NTP", "HDR", "LED", "PIN"];
    const SMALL: [&str; 15] = ["A", "AN", "AND", "AS", "AT", "BUT", "BY", "FOR", "FROM", "IN", "OF", "ON", "OR", "THE", "TO"];
    let mut out = String::with_capacity(text.len());
    let last = text.split(' ').count() - 1;
    for (n, word) in text.split(' ').enumerate() {
        if n > 0 {
            out.push(' ');
        }
        let letters = word.chars().filter(|c| c.is_alphabetic()).count();
        if letters == 0 || word.chars().any(char::is_lowercase) {
            out.push_str(word);
        } else if word == "IPV4" || word == "IPV6" {
            out.push_str(&word.replace("IPV", "IPv"));
        } else if ACRONYMS.contains(&word) {
            out.push_str(word);
        } else if n > 0 && n < last && SMALL.contains(&word) {
            out.push_str(&word.to_lowercase());
        } else {
            // each part of a hyphenated word starts with a capital: "Double-Click"
            let mut start = true;
            for c in word.chars() {
                if start {
                    out.extend(c.to_uppercase());
                } else {
                    out.extend(c.to_lowercase());
                }
                start = c == '-';
            }
        }
    }
    out
}

/// The lines a row label takes in `width` px: the label wrapped onto at most `LABEL_LINES` lines, the last ending in an
/// ellipsis when the text is still longer. A label never runs under the trailing control. `wrap` off (a sidebar row) keeps
/// it to one line.
fn label_lines(ui: &Ui, text: &str, width: f32, wrap: bool) -> Vec<String> {
    let width = width.max(0.0);
    let mut lines = if wrap { ui.wrap(text, MAC_BODY_PT, Weight::Regular, width) } else { vec![text.to_string()] };
    let limit = if wrap { LABEL_LINES } else { 1 };
    if lines.len() > limit {
        // the rest of the text joins the last line, which `fit` then ends with an ellipsis
        let tail = lines.split_off(limit - 1).join(" ");
        lines.push(tail);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines.iter().map(|l| ui.fit(l, MAC_BODY_PT, width)).collect()
}

/// The width in px a row's trailing control takes from its label: the chevron, switch, check, detail button or network
/// icons, and the badge, with the gap before them. A pop-up button is counted by its own label (`popup_layout`) and a
/// segmented row has no label.
fn accessory_room(ui: &Ui, item: &Item, chevron_w: f32, check_w: f32, badge_w: f32) -> f32 {
    let control = match item.accessory {
        Accessory::None | Accessory::Popup | Accessory::Segmented => 0.0,
        Accessory::Chevron => chevron_w + ui.pt(6.0),
        Accessory::Switch(_) => ui.pt(SWITCH_W_PT + 12.0),
        Accessory::Check(_) => check_w + ui.pt(8.0),
        Accessory::Info => ui.pt(22.0 + 16.0),
        Accessory::Wifi { secure, .. } => ui.pt(12.0) * 1.42 + ui.pt(10.0) + if secure { ui.pt(9.0 + 10.0) } else { 0.0 },
    };
    let badge = if item.badge > 0 { badge_w + ui.pt(8.0) } else { 0.0 };
    control + badge + if control + badge > 0.0 { ui.pt(8.0) } else { 0.0 }
}

pub struct ListPage {
    spec: Spec,
    w: i32,
    h: i32,
    rows: Vec<RowGeom>,
    /// (group top, group height) in content px
    group_rects: Vec<(f32, f32)>,
    headers: Vec<Option<Image>>,
    footers: Vec<Vec<Image>>,
    images: Vec<RowImages>,
    /// when each row's switch last flipped (same order as `rows`)
    flipped: Vec<f64>,
    title_large: Option<Image>,
    nav: NavBar,
    chevron: Image,
    check: Image,
    scroll: Scroller,
    /// content y of the search capsule the root page carries
    search_y: Option<f32>,
    /// side margin and nav bar height of this pane, in px and pt
    margin: f32,
    nav_pt: f32,
    /// height above the nav bar (the phone's status bar; 0 in a desktop window)
    status_pt: f32,
    /// the header card of a first-level detail page
    header: Option<Header>,
    /// the Settings sidebar: no chevrons, the local account card, the selected row filled
    sidebar: Option<Sidebar>,
    /// changes whenever the content (rows, values, switches, sliders) changes, never on scroll
    gen: u64,
    /// the pop-up menu open on a row, if any
    open: Option<OpenPopup>,
}

/// Where a page sits: the phone's full-width column or an bifrost split-view pane.
#[derive(Clone, Copy, Debug)]
pub struct Pane {
    pub margin_pt: f32,
    pub nav_pt: f32,
    pub sidebar: bool,
    /// space above the nav bar: the phone's status bar, nothing on the desktop
    pub status_pt: f32,
    /// a first-level detail page opens with its header card (icon, title, description)
    pub header: bool,
}

impl Pane {
    pub const PHONE: Pane = Pane { margin_pt: MARGIN_PT, nav_pt: NAV_PT, sidebar: false, status_pt: STATUS_PT, header: false };
}

/// The card at the top of a detail page: a large icon, the page's name and a line about it.
struct Header {
    /// one row (icon at the left, title and description beside it) instead of the centred hero
    compact: bool,
    icon: Icon,
    title: Image,
    lines: Vec<Image>,
    /// content y of the card's top and its height, in px
    y: f32,
    h: f32,
}

/// What a first-level page opens with, and how. Only General has the large centred hero (a category page: its rows are
/// all links). Notifications and Screen Time open with one compact card (icon, title and description in one row); every
/// other pane starts at its first group under the toolbar title, as the reference panes do. The bool is `compact`.
fn header_for(id: PageId) -> Option<(Icon, &'static str, bool)> {
    Some(match id {
        PageId::General => (Icon::General, "Manage your overall setup and preferences, such as software updates, language and more.", false),
        PageId::Notifications => (Icon::Notifications, "Choose how and when apps can notify you.", true),
        PageId::ScreenTime => (Icon::ScreenTime, "See how you use your computer and set limits.", true),
        _ => return None,
    })
}

struct Sidebar {
    selected: PageId,
    card_y: f32,
    card_title: Image,
    card_lines: Vec<Image>,
}

/// Fills one row of an inset group, rounding only the group's outer corners.
/// `rect` is (x, y, w, h) in px.
pub fn fill_row(s: &mut Surface, ui: &Ui, (x, y, w, h): (f32, f32, f32, f32), first: bool, last: bool, color: u32) {
    s.fill_rrect(x, y, w, h, ui.pt(RADIUS_PT), color);
    if !first {
        s.fill_rrect(x, y, w, h / 2.0, 0.0, color);
    }
    if !last {
        s.fill_rrect(x, y + h / 2.0, w, h / 2.0, 0.0, color);
    }
}

/// A fresh content generation number.
pub fn next_gen() -> u64 {
    static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Bottom of the navigation bar in px.
pub fn nav_bottom(ui: &Ui) -> f32 {
    ui.pt(STATUS_PT + NAV_PT)
}

/// Blurs the top `rows` of the frame in place: the bar's blur material.
/// Skipped on a band surface that does not hold the whole strip.
///
/// The blur runs on a 1/4 (or 1/2) size copy and is scaled back up bilinearly, which for a
/// material blur is indistinguishable from the full-size box blur but costs ~1/16 of it.
fn blur_strip(s: &mut Surface, rows: usize, radius: usize) {
    if s.y0 != 0 || rows == 0 || s.h < rows {
        return;
    }
    let (w, stride) = (s.w, s.stride);
    blur_rows(&mut s.px[..(rows - 1) * stride + w], w, rows, stride, radius);
}

/// Blurs the `w` x `h` pixels at the start of `px` (rows `stride` apart) in place.
fn blur_rows(px: &mut [u32], w: usize, h: usize, stride: usize, radius: usize) {
    let f = match radius {
        0 => return,
        1..=3 => 1,
        4..=7 => 2,
        _ => 4,
    };
    if f == 1 || w < 2 * f || h < 2 * f {
        let mut img = Image::new(w, h);
        for y in 0..h {
            img.px[y * w..(y + 1) * w].copy_from_slice(&px[y * stride..y * stride + w]);
        }
        img.blur(radius);
        for y in 0..h {
            px[y * stride..y * stride + w].copy_from_slice(&img.px[y * w..(y + 1) * w]);
        }
        return;
    }
    // box-average down by f
    let (sw, sh) = (w.div_ceil(f), h.div_ceil(f));
    let mut small = Image::new(sw, sh);
    for sy in 0..sh {
        for sx in 0..sw {
            let (x0, y0) = (sx * f, sy * f);
            let (x1, y1) = ((x0 + f).min(w), (y0 + f).min(h));
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                for &p in &px[y * stride + x0..y * stride + x1] {
                    for (ch, c) in sum.iter_mut().enumerate() {
                        *c += (p >> (ch * 8)) & 255;
                    }
                }
            }
            let n = ((x1 - x0) * (y1 - y0)) as u32;
            small.px[sy * sw + sx] = (0..4).fold(0, |c, ch| c | ((sum[ch] + n / 2) / n) << (ch * 8));
        }
    }
    small.blur((radius / f).max(1));
    // bilinear back up; sample positions per column and row computed once
    let axis = |len: usize, small_len: usize| -> Vec<(usize, usize, u32)> {
        (0..len)
            .map(|i| {
                let p = ((i as f32 + 0.5) / f as f32 - 0.5).clamp(0.0, (small_len - 1) as f32);
                let i0 = p as usize;
                (i0, (i0 + 1).min(small_len - 1), ((p - i0 as f32) * 256.0) as u32)
            })
            .collect()
    };
    let (cols, rws) = (axis(w, sw), axis(h, sh));
    for (y, &(r0, r1, fy)) in rws.iter().enumerate() {
        let (top, bot) = (&small.px[r0 * sw..(r0 + 1) * sw], &small.px[r1 * sw..(r1 + 1) * sw]);
        for (d, &(c0, c1, fx)) in px[y * stride..y * stride + w].iter_mut().zip(&cols) {
            *d = lerp8(lerp8(top[c0], top[c1], fx), lerp8(bot[c0], bot[c1], fx), fy);
        }
    }
}

/// `a + (b - a) * t / 256` per channel, two channels per multiply.
#[inline]
fn lerp8(a: u32, b: u32, t: u32) -> u32 {
    const M: u32 = 0x00ff_00ff;
    let rb = (((a & M) * (256 - t) + (b & M) * t) >> 8) & M;
    let ag = ((((a >> 8) & M) * (256 - t) + ((b >> 8) & M) * t) & 0xff00_ff00) as u32;
    rb | ag
}

/// The navigation bar: an inline title and, below the root, "‹ Previous".
pub struct NavBar {
    title: Image,
    /// the title as text, to draw it again in the toolbar's own style
    title_text: String,
    back: Option<(Image, Image)>,
    nav_pt: f32,
    status_pt: f32,
    /// desktop toolbar: back and forward in one pill instead of a labelled button
    pill: bool,
}

impl NavBar {
    pub fn new(ui: &Ui, title: &str, back: Option<&str>) -> Self {
        NavBar {
            title: ui.text(title, 17.0, Weight::Semibold, WHITE),
            title_text: title.to_string(),
            back: back.map(|b| (ui.text("‹", 30.0, Weight::Medium, BLUE), ui.text(&ui.fit(b, 17.0, ui.pt(160.0)), 17.0, Weight::Regular, BLUE))),
            nav_pt: NAV_PT,
            status_pt: STATUS_PT,
            pill: false,
        }
    }

    /// Draws back and forward as one pill (the desktop toolbar). The title then sits left-aligned beside the pill in the
    /// title 3 style, not centred in the bar.
    pub fn pill(mut self, ui: &Ui, on: bool) -> Self {
        self.pill = on;
        if on {
            self.title = ui.text(&self.title_text, MAC_TITLE3_PT, Weight::Semibold, WHITE);
        }
        self
    }

    /// Space above the bar (0 on the desktop).
    pub fn status(mut self, pt: f32) -> Self {
        self.status_pt = pt;
        self
    }

    /// The bar's height below the status bar (the bifrost's is taller than the phone's).
    pub fn height(mut self, pt: f32) -> Self {
        self.nav_pt = pt;
        self
    }

    fn bottom(&self, ui: &Ui) -> f32 {
        ui.pt(self.status_pt + self.nav_pt)
    }

    /// Whether (x, y) is on the back button.
    pub fn hit_back(&self, ui: &Ui, x: f32, y: f32) -> bool {
        if self.pill {
            // the left half of the pill
            use bifrost_ui::prefs_layout::m;
            return self.back.is_some() && x >= ui.pt(m::TOOLBAR_X) && x < ui.pt(m::TOOLBAR_X + m::TOOLBAR_PILL_W / 2.0) && y >= ui.pt(m::TOOLBAR_Y) && y < ui.pt(m::TOOLBAR_Y + m::TOOLBAR_PILL_H);
        }
        self.back.is_some() && y < self.bottom(ui) && x < ui.pt(150.0)
    }

    /// Draws the bar `alpha` (0..=1) solid over the content, with the title
    /// shown at `title_alpha`.
    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, alpha: f32, title_alpha: f32) {
        let bottom = self.bottom(ui);
        if alpha > 0.0 {
            s.fill_rrect(0.0, 0.0, w as f32, bottom, 0.0, crate::gfx::scale(DETAIL_BG.now(), (alpha * 200.0) as u32));
            s.fill_rrect(0.0, bottom - 1.0, w as f32, 1.0, 0.0, crate::gfx::scale(super::ui::SEPARATOR_C.now(), (alpha * 255.0) as u32));
        }
        let mid = ui.pt(self.status_pt + self.nav_pt / 2.0);
        if self.pill {
            use bifrost_ui::prefs_layout::m;
            let (px, py, pw, ph) = (ui.pt(m::TOOLBAR_X), ui.pt(m::TOOLBAR_Y), ui.pt(m::TOOLBAR_PILL_W), ui.pt(m::TOOLBAR_PILL_H));
            s.fill_rrect(px - 1.0, py - 1.0, pw + 2.0, ph + 2.0, ph / 2.0 + 1.0, super::ui::SEPARATOR_C.now());
            s.fill_rrect(px, py, pw, ph, ph / 2.0, CARD_BG.now());
            s.fill_rrect(px + pw / 2.0 - 0.5, py + ph * 0.28, 1.0, ph * 0.44, 0.0, super::ui::SEPARATOR_C.now());
            let (cy, a, th) = (py + ph / 2.0, ui.pt(5.5), ui.pt(1.8));
            let col = |on: bool| if on { LABEL.now() } else { LABEL_2.now() };
            let bx = px + pw * 0.27;
            icons::line(s, (bx + a * 0.5, cy - a), (bx - a * 0.5, cy), th, col(self.back.is_some()));
            icons::line(s, (bx - a * 0.5, cy), (bx + a * 0.5, cy + a), th, col(self.back.is_some()));
            // there is no forward history yet: the forward half stays dimmed
            let fx = px + pw * 0.73;
            icons::line(s, (fx - a * 0.5, cy - a), (fx + a * 0.5, cy), th, col(false));
            icons::line(s, (fx + a * 0.5, cy), (fx - a * 0.5, cy + a), th, col(false));
        } else if let Some((chev, label)) = &self.back {
            let x = ui.pt(12.0);
            let lw = chev.w as f32 + ui.pt(2.0) + label.w as f32;
            super::ui::button_plate(s, ui, (x - ui.pt(6.0), mid - ui.pt(16.0), lw + ui.pt(12.0), ui.pt(32.0)), lw);
            s.blit(chev, x as i32, (mid - chev.h as f32 / 2.0 - ui.pt(2.0)) as i32, 255, 255);
            s.blit(label, (x + chev.w as f32 + ui.pt(2.0)) as i32, (mid - label.h as f32 / 2.0) as i32, 255, 255);
        }
        let ta = (title_alpha * 255.0) as u32;
        if ta > 0 {
            let x = if self.pill { ui.pt(bifrost_ui::prefs_layout::toolbar_title_x()) as i32 } else { w / 2 - self.title.w as i32 / 2 };
            s.blit_colored(&self.title, x, (mid - self.title.h as f32 / 2.0) as i32, LABEL.now(), ta);
        }
    }
}

/// (icon side, icon x from the row's left edge, gap to the label) in pt for a sidebar or a detail card row.
fn row_icon(sidebar: bool) -> (f32, f32, f32) {
    use bifrost_ui::prefs_layout::m;
    if sidebar {
        (m::SIDEBAR_ICON, m::SIDEBAR_ICON_X - m::SIDEBAR_INSET, m::SIDEBAR_LABEL_X - m::SIDEBAR_ICON_X - m::SIDEBAR_ICON)
    } else {
        (m::ROW_ICON, m::ROW_ICON_X, m::ROW_LABEL_X - m::ROW_ICON_X - m::ROW_ICON)
    }
}

impl ListPage {
    /// Lays out and renders `spec` for a `w` x `h` screen; `back` is the
    /// previous page's title for the back button.
    pub fn new(ui: &Ui, spec: Spec, w: i32, h: i32, back: Option<&str>) -> Self {
        Self::in_pane(ui, spec, w, h, back, Pane::PHONE)
    }

    /// `new` for a page in `pane`.
    pub fn in_pane(ui: &Ui, spec: Spec, w: i32, h: i32, back: Option<&str>, pane: Pane) -> Self {
        let m = ui.pt(pane.margin_pt);
        let nav_bottom = ui.pt(pane.status_pt + pane.nav_pt);
        let cw = w as f32 - 2.0 * m;
        let value_w = cw * 0.5;
        let mut rows = vec![];
        let mut group_rects = vec![];
        let mut headers = vec![];
        let mut footers = vec![];
        let mut images = vec![];
        let chevron = controls::chevron(ui);
        // rendered white and tinted with the accent colour when drawn, so it follows the user's accent
        let check = ui.text("✓", 17.0, Weight::Semibold, WHITE);
        let mut y = nav_bottom + if spec.large && !pane.sidebar { ui.pt(LARGE_TITLE_PT) } else { 0.0 };
        // the root page's capsule under its large title; in the sidebar the field sits under the traffic lights
        let search_y = if pane.sidebar { Some(ui.pt(bifrost_ui::prefs_layout::m::SEARCH_Y)) } else { (spec.large && spec.id == PageId::Root).then_some(y) };
        if search_y.is_some() && !pane.sidebar {
            y += ui.pt(crate::shell::settings_search::FIELD_PT);
        }
        let sidebar = pane.sidebar.then(|| {
            let card_y = ui.pt(bifrost_ui::prefs_layout::m::ACCOUNT_Y);
            y = card_y + ui.pt(SIDEBAR_CARD_PT + SIDEBAR_ACCOUNT_GAP_PT);
            let account = crate::shell::account::load();
            Sidebar {
                selected: PageId::Root,
                card_y,
                // white text, drawn with the scheme's label colour so it fades with the scheme
                card_title: ui.text(&account.full_name, super::ui::MAC_TITLE3_PT, Weight::Semibold, WHITE),
                card_lines: ["Local account".to_string(), account.username].iter().map(|l| ui.text(l, super::ui::MAC_CALLOUT_PT, Weight::Regular, SECONDARY)).collect(),
            }
        });
        let header = (pane.header && !pane.sidebar).then(|| header_for(spec.id)).flatten().map(|(icon, desc, compact)| {
            use bifrost_ui::prefs_layout::m as pm;
            let hy = y + ui.pt(12.0);
            if compact {
                let text_w = cw - ui.pt(pm::HEADER_COMPACT_TEXT_X + 2.0 * pm::HEADER_COMPACT_PAD);
                let title = ui.text(&spec.title, MAC_BODY_PT, Weight::Regular, WHITE);
                let lines: Vec<Image> = ui.wrap(desc, MAC_BODY_PT, Weight::Regular, text_w).iter().map(|l| ui.text(l, MAC_BODY_PT, Weight::Regular, SECONDARY)).collect();
                let text_h = title.h as f32 + lines.iter().map(|l| l.h as f32).sum::<f32>();
                let h = (text_h + ui.pt(2.0 * pm::HEADER_COMPACT_PAD)).max(ui.pt(pm::HEADER_COMPACT_ICON + 2.0 * pm::HEADER_COMPACT_PAD));
                y = hy + h;
                return Header { compact, icon, title, lines, y: hy, h };
            }
            let h = ui.pt(pm::HEADER_H);
            y = hy + h;
            Header { compact, icon, title: ui.text(&spec.title, super::ui::MAC_TITLE1_PT, Weight::Bold, WHITE), lines: ui.wrap(desc, super::ui::MAC_BODY_PT, Weight::Regular, cw - ui.pt(40.0)).iter().map(|l| ui.text(l, super::ui::MAC_BODY_PT, Weight::Regular, SECONDARY)).collect(), y: hy, h }
        });
        for (g, group) in spec.groups.iter().enumerate() {
            y += ui.pt(if !pane.sidebar { GROUP_GAP_PT } else if g == 0 { 0.0 } else { SIDEBAR_GROUP_GAP_PT });
            // a headline above its card: title case, headline weight, in the label colour (drawn with `blit_colored`)
            headers.push(group.header.as_ref().map(|hd| ui.text(&headline_case(hd), MAC_HEADLINE_PT, Weight::Semibold, WHITE)));
            if group.header.is_some() {
                y += ui.pt(HEADER_PT);
            }
            let top = y;
            for (i, item) in group.items.iter().enumerate() {
                let rh = ui.pt(if pane.sidebar { SIDEBAR_ROW_PT } else { item.height_pt() });
                rows.push(RowGeom { g, i, y, h: rh });
                let color = match item.tint {
                    Tint::Plain => WHITE,
                    Tint::Blue => BLUE,
                    Tint::Red => RED,
                    Tint::Dim => SECONDARY,
                };
                // the value keeps its natural width (up to half the row); the label gets the rest
                let value_img = (!item.value.is_empty() && !pane.sidebar).then(|| ui.text(&ui.fit(&item.value, MAC_BODY_PT, value_w), MAC_BODY_PT, Weight::Regular, SECONDARY));
                let popup_img = item.popup.as_ref().filter(|_| !pane.sidebar).map(|p| {
                    let chosen = p.choices.get(p.selected).map_or("", String::as_str);
                    ui.text(&ui.fit(chosen, MAC_BODY_PT, value_w), MAC_BODY_PT, Weight::Regular, WHITE)
                });
                let value_room = value_img.as_ref().map_or(0.0, |v| v.w as f32 + ui.pt(8.0)) + popup_img.as_ref().map_or(0.0, |p| p.w as f32 + ui.pt(POPUP_CHEV_PT + POPUP_GAP_PT));
                let badge_w = if item.badge > 0 { super::notify::list::badge_image(ui, item.badge).w as f32 } else { 0.0 };
                let accessory = if pane.sidebar { 0.0 } else { accessory_room(ui, item, chevron.w as f32, check.w as f32, badge_w) };
                let (icon_pt, _, gap_pt) = row_icon(pane.sidebar);
                // the label stops before the value and the trailing control and wraps instead of running under them
                let label_w = cw - ui.pt(2.0 * TEXT_INSET_PT) - if has_leading(item) { ui.pt(icon_pt + gap_pt) } else { 0.0 } - if item.leading_check { ui.pt(LEADING_CHECK_PT) } else { 0.0 } - value_room - accessory;
                // the app's icon at the icon token; a built-in app has none and leaves the slot blank
                let icon_img = item.app_icon.as_ref().filter(|_| has_leading(item)).and_then(|id| { let px = ui.pt(icon_pt).round() as usize; super::apphost::icon_image(id, px).or_else(|| super::apps::builtin_icon(id, px)) });
                images.push(RowImages {
                    label: label_lines(ui, &item.label, label_w, !pane.sidebar).iter().map(|l| ui.text(l, MAC_BODY_PT, Weight::Regular, color)).collect(),
                    value: value_img,
                    icon: icon_img,
                    popup: popup_img,
                });
                y += rh;
            }
            group_rects.push((top, y - top));
            let lines = group.footer.as_ref().map(|f| ui.wrap(f, 13.0, Weight::Regular, cw - ui.pt(2.0 * TEXT_INSET_PT))).unwrap_or_default();
            if !lines.is_empty() {
                y += ui.pt(FOOTER_GAP_PT) + ui.pt(FOOTER_LINE_PT) * lines.len() as f32;
            }
            footers.push(lines.iter().map(|l| ui.text(l, 13.0, Weight::Regular, SECONDARY)).collect());
        }
        let content_h = y + ui.pt(40.0) + home_zone(ui);
        let mut scroll = Scroller::new();
        scroll.set_max(content_h - h as f32);
        let flipped = vec![-1e9; rows.len()];
        ListPage {
            title_large: (spec.large && !pane.sidebar).then(|| ui.text(&spec.title, super::ui::MAC_LARGE_TITLE_PT, Weight::Bold, WHITE)),
            nav: NavBar::new(ui, &spec.title, back).height(pane.nav_pt).status(pane.status_pt).pill(ui, pane.status_pt == 0.0 && !pane.sidebar),
            chevron,
            check,
            spec,
            w,
            h,
            rows,
            group_rects,
            headers,
            footers,
            images,
            flipped,
            scroll,
            open: None,
            search_y,
            margin: m,
            nav_pt: pane.nav_pt,
            status_pt: pane.status_pt,
            header,
            sidebar,
            gen: next_gen(),
        }
    }

    fn nav_bottom(&self, ui: &Ui) -> f32 {
        ui.pt(self.status_pt + self.nav_pt)
    }

    /// Marks the sidebar row that opens `page` as selected.
    pub fn select(&mut self, page: PageId) {
        if let Some(sb) = &mut self.sidebar {
            sb.selected = page;
        }
    }

    /// The search capsule's rect on screen, when it is on the page.
    pub fn search_frame(&self, ui: &Ui) -> Option<(f32, f32, f32, f32)> {
        let y = self.search_y? - self.draw_offset();
        let m = self.margin;
        if self.sidebar.is_some() {
            return Some((m, y, self.w as f32 - 2.0 * m, ui.pt(bifrost_ui::prefs_layout::m::SEARCH_H)));
        }
        (y >= self.nav_bottom(ui)).then(|| (m, y, self.w as f32 - 2.0 * m, ui.pt(crate::shell::settings_search::FIELD_PT)))
    }

    pub fn spec(&self) -> &Spec {
        &self.spec
    }

    pub fn id(&self) -> PageId {
        self.spec.id
    }

    pub fn item(&self, g: usize, i: usize) -> &Item {
        &self.spec.groups[g].items[i]
    }

    fn row_index(&self, g: usize, i: usize) -> usize {
        self.rows.iter().position(|r| r.g == g && r.i == i).expect("row exists")
    }

    /// Flips a switch row at `t`, animating from its old state.
    pub fn set_switch(&mut self, g: usize, i: usize, on: bool, t: f64) {
        self.gen = next_gen();
        let k = self.row_index(g, i);
        if self.spec.groups[g].items[i].accessory != Accessory::Switch(on) {
            self.spec.groups[g].items[i].accessory = Accessory::Switch(on);
            self.flipped[k] = t;
        }
    }

    /// Starts the switch animation on a row whose state was already set
    /// (after a rebuild).
    pub fn flipped_at(&mut self, g: usize, i: usize, t: f64) {
        self.gen = next_gen();
        let k = self.row_index(g, i);
        self.flipped[k] = t;
    }

    /// Scrolls straight to `offset`, for a page rebuilt in place.
    pub fn jump_to(&mut self, offset: f32) {
        self.scroll.jump_to(offset);
    }

    /// Moves the checkmark in group `g` to item `i`.
    pub fn set_check(&mut self, g: usize, i: usize) {
        self.gen = next_gen();
        for (j, item) in self.spec.groups[g].items.iter_mut().enumerate() {
            if let Accessory::Check(_) = item.accessory {
                item.accessory = Accessory::Check(j == i);
            }
        }
    }

    /// Shows `k` as the chosen one of row (g, i)'s pop-up button or segmented control, after the choice was made.
    pub fn set_choice(&mut self, ui: &Ui, g: usize, i: usize, k: usize) {
        self.gen = next_gen();
        let row = self.row_index(g, i);
        let item = &mut self.spec.groups[g].items[i];
        if let Some(seg) = &mut item.segments {
            seg.selected = k.min(seg.choices.len().saturating_sub(1));
        }
        if let Some(pop) = &mut item.popup {
            pop.selected = k.min(pop.choices.len().saturating_sub(1));
            let chosen = pop.choices.get(pop.selected).cloned().unwrap_or_default();
            let value_w = (self.w as f32 - 2.0 * self.margin) * 0.5;
            if self.images[row].popup.is_some() {
                self.images[row].popup = Some(ui.text(&ui.fit(&chosen, MAC_BODY_PT, value_w), MAC_BODY_PT, Weight::Regular, WHITE));
            }
        }
    }

    /// Re-renders a row's value text.
    pub fn set_value(&mut self, ui: &Ui, g: usize, i: usize, value: &str) {
        self.gen = next_gen();
        let k = self.row_index(g, i);
        let value_w = (self.w as f32 - 2.0 * self.margin) * 0.5;
        self.spec.groups[g].items[i].value = value.into();
        self.images[k].value = (!value.is_empty()).then(|| ui.text(&ui.fit(value, MAC_BODY_PT, value_w), MAC_BODY_PT, Weight::Regular, SECONDARY));
    }

    /// Replaces the page's progress bar in place (a download ticking), leaving every other row alone.
    /// False when the page has none.
    pub fn set_bar(&mut self, bar: super::storage_page::Bar) -> bool {
        let Some(item) = self.spec.groups.iter_mut().flat_map(|g| g.items.iter_mut()).find(|it| matches!(it.body, Body::Bar(_))) else { return false };
        item.body = Body::Bar(bar);
        self.gen = next_gen();
        true
    }

    pub fn set_slider(&mut self, g: usize, i: usize, v: f32) {
        self.gen = next_gen();
        if let Body::Slider { value, .. } = &mut self.spec.groups[g].items[i].body {
            *value = v.clamp(0.0, 1.0);
        }
    }

    /// The row's rect (x, y, w, h) on screen at the current scroll offset.
    pub fn row_rect(&self, _ui: &Ui, g: usize, i: usize) -> (f32, f32, f32, f32) {
        let r = &self.rows[self.row_index(g, i)];
        let m = self.margin;
        (m, r.y - self.draw_offset(), self.w as f32 - 2.0 * m, r.h)
    }

    /// Centre of the row on screen, for tests and previews.
    pub fn row_centre(&self, ui: &Ui, g: usize, i: usize) -> (f32, f32) {
        let (x, y, w, h) = self.row_rect(ui, g, i);
        (x + w / 2.0, y + h / 2.0)
    }

    /// The (group, item) of the first row whose label is `label`.
    pub fn find(&self, label: &str) -> Option<(usize, usize)> {
        self.rows.iter().find(|r| self.item(r.g, r.i).label == label).map(|r| (r.g, r.i))
    }

    /// Slider track (x, y, w) on screen for a slider row.
    pub fn track(&self, ui: &Ui, g: usize, i: usize) -> (f32, f32, f32) {
        let (x, y, w, h) = self.row_rect(ui, g, i);
        let inset = ui.pt(52.0);
        (x + inset, y + h / 2.0, w - 2.0 * inset)
    }

    /// Opens the pop-up menu of row (g, i): its choices, the selected one checked. False when the row has no pop-up.
    pub fn open_popup(&mut self, ui: &Ui, g: usize, i: usize) -> bool {
        let Some(pop) = self.spec.groups[g].items[i].popup.clone() else { return false };
        let items = pop.choices.iter().enumerate().map(|(k, c)| bifrost_ui::menu::MenuItem::new(k as u32, c)).collect();
        let labels: Vec<Image> = pop.choices.iter().map(|c| ui.text(&ui.fit(c, MAC_BODY_PT, self.w as f32 * 0.6), MAC_BODY_PT, Weight::Regular, WHITE)).collect();
        let widest = labels.iter().map(|l| l.w as f32).fold(0.0, f32::max);
        let w = (widest + ui.pt(POPUP_TEXT_PT + POPUP_TAIL_PT)).max(ui.pt(POPUP_MIN_W_PT));
        self.open = Some(OpenPopup { g, i, menu: bifrost_ui::menu::Menu::popup(items, pop.selected), labels, w });
        self.gen = next_gen();
        true
    }

    /// Closes the open pop-up menu; the row it was opened from.
    pub fn close_popup(&mut self) -> Option<(usize, usize)> {
        self.gen = next_gen();
        self.open.take().map(|p| (p.g, p.i))
    }

    /// Whether a pop-up menu is open.
    pub fn popup_open(&self) -> bool {
        self.open.is_some()
    }

    /// The open menu's rect (x, y, w, h) in px on screen: the menu sits over its button so the chosen item covers it,
    /// and stays inside the list below the bar (`bifrost_ui::menu::popup_origin`).
    pub fn popup_rect(&self, ui: &Ui) -> Option<(f32, f32, f32, f32)> {
        let p = self.open.as_ref()?;
        let (spans, total) = p.menu.rows(POPUP_ROW_PT, POPUP_SEP_PT, POPUP_PAD_PT);
        let (rx, ry, rw, rh) = self.row_rect(ui, p.g, p.i);
        let right = rx + rw - ui.pt(TEXT_INSET_PT);
        let button = bifrost_ui::Rect { x: right - p.w, y: ry, w: p.w, h: rh };
        let sel_top = spans.get(p.menu.selected().unwrap_or(0)).map_or(0.0, |s| s.0);
        let nav = self.nav_bottom(ui);
        let bounds = bifrost_ui::Rect { x: 0.0, y: nav, w: self.w as f32, h: self.h as f32 - nav };
        let (x, y) = bifrost_ui::menu::popup_origin(button, p.w, ui.pt(total), ui.pt(sel_top), bounds);
        Some((x, y, p.w, ui.pt(total)))
    }

    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Hit> {
        if y < self.nav_bottom(ui) {
            return self.nav.hit_back(ui, x, y).then_some(Hit::Back);
        }
        // an open menu takes the touch: a row of it chooses, its rim does nothing, anything else closes it
        if let Some((mx, my, mw, mh)) = self.popup_rect(ui) {
            if x < mx || x >= mx + mw || y < my || y >= my + mh {
                return Some(Hit::Dismiss);
            }
            let p = self.open.as_ref()?;
            return p.menu.hit((y - my) / ui.pt(1.0), POPUP_ROW_PT, POPUP_SEP_PT, POPUP_PAD_PT).map(Hit::Choice);
        }
        let m = self.margin;
        if x < m || x >= self.w as f32 - m {
            return None;
        }
        let cy = y + self.scroll.offset();
        let r = self.rows.iter().find(|r| cy >= r.y && cy < r.y + r.h)?;
        let item = self.item(r.g, r.i);
        if let Some(seg) = item.segments.as_ref().filter(|_| item.accessory == Accessory::Segmented) {
            // a touch on the segmented row chooses the segment under it; the track's rim goes to the nearest one
            let (tx, _, tw, _) = self.segment_track(ui, (m, 0.0, self.w as f32 - 2.0 * m, 0.0));
            let k = (((x - tx) / (tw / seg.choices.len().max(1) as f32)).floor().max(0.0) as usize).min(seg.choices.len().saturating_sub(1));
            return Some(Hit::Segment(r.g, r.i, k));
        }
        Some(match item.body {
            Body::Slider { .. } => Hit::Slider(r.g, r.i),
            _ => Hit::Row(r.g, r.i),
        })
    }

    // -- scrolling --

    pub fn scroll_down(&mut self, y: f32, t: f64) {
        self.scroll.down(y, t);
    }

    pub fn scroll_moved(&mut self, y: f32, t: f64) {
        self.scroll.moved(y, t);
    }

    /// Ends the touch; true when it was a tap rather than a drag.
    pub fn scroll_up(&mut self, y: f32, t: f64) -> bool {
        self.scroll.up(y, t)
    }

    pub fn dragging(&self) -> bool {
        self.scroll.dragging()
    }

    /// The scroll offset the page is drawn at: whole pixels, so a scrolled frame is the
    /// previous one shifted by an integer number of rows.
    pub fn draw_offset(&self) -> f32 {
        self.scroll.offset().round()
    }

    /// Content generation (see `gen`).
    pub fn gen(&self) -> u64 {
        self.gen
    }

    /// Whether a frame at `t` is the previous one shifted by the scroll offset `off` px alone:
    /// no switch mid-flip, and (large-title pages) the title and search capsule scrolled away.
    pub fn scroll_only(&self, ui: &Ui, t: f64, off: i32) -> bool {
        !self.flipped.iter().any(|&f| t - f < SWITCH_MS) && (!self.spec.large || off as f32 >= ui.pt(LARGE_TITLE_PT) + 1.0)
    }

    pub fn offset(&self) -> f32 {
        self.scroll.offset()
    }

    /// Advances the fling and spring; true while a frame is still needed.
    pub fn tick(&mut self, t: f64) -> bool {
        let scrolling = self.scroll.tick(t);
        scrolling || self.animating(t)
    }

    /// Whether anything on the page is mid-animation at `t`.
    pub fn animating(&self, t: f64) -> bool {
        self.scroll.animating() || self.flipped.iter().any(|&f| t - f < SWITCH_MS)
    }

    /// How far the bar has faded in: 0 transparent over the content, 1 solid.
    fn bar_alpha(&self, ui: &Ui) -> f32 {
        let o = self.draw_offset();
        if let Some(h) = self.header.as_ref().filter(|h| !h.compact) {
            // the hero scrolls under the toolbar; the bar and its inline title fade in as it leaves
            return ((o - (h.y + h.h - ui.pt(40.0))) / ui.pt(16.0)).clamp(0.0, 1.0);
        }
        if self.spec.large {
            ((o - ui.pt(LARGE_TITLE_PT - 16.0)) / ui.pt(16.0)).clamp(0.0, 1.0)
        } else {
            (o / ui.pt(12.0)).clamp(0.0, 1.0)
        }
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, pressed: Option<Hit>, t: f64) {
        let m = self.margin;
        let cw = self.w as f32 - 2.0 * m;
        let dy = -self.draw_offset();
        let text_x = m + ui.pt(TEXT_INSET_PT);
        for (g, group) in self.spec.groups.iter().enumerate() {
            let (gy, gh) = self.group_rects[g];
            let gy = gy + dy;
            if let Some(hd) = &self.headers[g] {
                s.blit_colored(hd, text_x as i32, (gy - ui.pt(HEADER_PT) + ui.pt(6.0)) as i32, LABEL.now(), 255);
            }
            if gy + gh >= 0.0 && gy < self.h as f32 {
                if self.sidebar.is_none() {
                    s.fill_rrect(m, gy, cw, gh, ui.pt(RADIUS_PT), CARD_BG.now());
                }
                for (i, item) in group.items.iter().enumerate() {
                    self.draw_row(s, ui, g, i, item, pressed, t);
                }
            }
            let mut fy = gy + gh + ui.pt(FOOTER_GAP_PT);
            for line in &self.footers[g] {
                s.blit(line, text_x as i32, fy as i32, 255, 255);
                fy += ui.pt(FOOTER_LINE_PT);
            }
        }
        if let Some(h) = &self.header {
            let hy = h.y + dy;
            if hy + h.h >= 0.0 && hy < self.h as f32 {
                use bifrost_ui::prefs_layout::m as pm;
                s.fill_rrect(m, hy, cw, h.h, ui.pt(RADIUS_PT), CARD_BG.now());
                if h.compact {
                    // icon at the left, the title and the description in one column beside it
                    let pad = ui.pt(pm::HEADER_COMPACT_PAD);
                    icons::tile_sized(s, ui, h.icon, m + ui.pt(pm::HEADER_COMPACT_ICON_X), hy + pad, pm::HEADER_COMPACT_ICON);
                    let tx = (m + ui.pt(pm::HEADER_COMPACT_TEXT_X)) as i32;
                    let mut ty = hy + pad;
                    s.blit_colored(&h.title, tx, ty as i32, LABEL.now(), 255);
                    ty += h.title.h as f32;
                    for line in &h.lines {
                        s.blit(line, tx, ty as i32, 255, 255);
                        ty += line.h as f32;
                    }
                } else {
                    let side = ui.pt(pm::HEADER_ICON);
                    icons::tile_sized(s, ui, h.icon, m + (cw - side) / 2.0, hy + ui.pt(24.0), pm::HEADER_ICON);
                    let mut ty = hy + ui.pt(24.0) + side + ui.pt(8.0);
                    s.blit_colored(&h.title, (m + (cw - h.title.w as f32) / 2.0) as i32, ty as i32, LABEL.now(), 255);
                    ty += h.title.h as f32 + ui.pt(2.0);
                    for line in &h.lines {
                        s.blit(line, (m + (cw - line.w as f32) / 2.0) as i32, ty as i32, 255, 255);
                        ty += line.h as f32;
                    }
                }
            }
        }
        if let Some(sb) = &self.sidebar {
            self.draw_card(s, ui, sb, dy);
        }
        self.draw_popup(s, ui, pressed);
        if let (Some(title), None) = (&self.title_large, &self.sidebar) {
            // fades out as it slides under the bar, while the inline title fades in
            let fade = 1.0 - (self.draw_offset().max(0.0) / ui.pt(LARGE_TITLE_PT - 8.0)).clamp(0.0, 1.0);
            s.blit_colored(title, (m - ui.pt(2.0)) as i32, (self.nav_bottom(ui) + dy) as i32, LABEL.now(), (fade * 255.0) as u32);
        }
        if self.sidebar.is_some() {
            return;
        }
        let alpha = self.bar_alpha(ui);
        if alpha > 0.0 {
            blur_strip(s, self.nav_bottom(ui) as usize, ui.pt(10.0) as usize);
        }
        self.nav.draw(s, ui, self.w, alpha, if self.spec.large || self.header.as_ref().is_some_and(|h| !h.compact) { alpha } else { 1.0 });
    }

    /// The open pop-up menu: a translucent rounded card, a checkmark on the chosen item and the highlighted item filled.
    fn draw_popup(&self, s: &mut Surface, ui: &Ui, pressed: Option<Hit>) {
        let (Some(p), Some((x, y, w, _))) = (&self.open, self.popup_rect(ui)) else { return };
        let (spans, total) = p.menu.rows(POPUP_ROW_PT, POPUP_SEP_PT, POPUP_PAD_PT);
        s.fill_rrect(x, y, w, ui.pt(total), ui.pt(MENU_RADIUS_PT), MATERIAL_MENU.now());
        for (k, item) in p.menu.items.iter().enumerate() {
            let (top, h) = spans[k];
            let ty = y + ui.pt(top);
            if item.separator {
                s.fill_rrect(x + ui.pt(10.0), ty + ui.pt(h) / 2.0 - 0.5, w - ui.pt(20.0), 1.0, 0.0, SEPARATOR_C.now());
                continue;
            }
            let mid = ty + ui.pt(h) / 2.0;
            let picked = pressed == Some(Hit::Choice(k));
            if picked {
                s.fill_rrect(x + ui.pt(4.0), ty, w - ui.pt(8.0), ui.pt(h), ui.pt(5.0), MENU_HIGHLIGHT);
            }
            if item.checked {
                s.blit_colored(&self.check, (x + ui.pt(8.0)) as i32, (mid - self.check.h as f32 / 2.0) as i32, bifrost_render::appearance::accent_now(), 255);
            }
            let label = &p.labels[k];
            if picked {
                s.blit(label, (x + ui.pt(POPUP_TEXT_PT)) as i32, (mid - label.h as f32 / 2.0) as i32, 255, 255);
            } else {
                s.blit_colored(label, (x + ui.pt(POPUP_TEXT_PT)) as i32, (mid - label.h as f32 / 2.0) as i32, LABEL.now(), 255);
            }
        }
    }

    /// The account block: avatar, the local user's name and a two-line caption (no cloud sign-in). It has no plate; the
    /// sidebar's own fill shows behind it.
    fn draw_card(&self, s: &mut Surface, ui: &Ui, sb: &Sidebar, dy: f32) {
        use bifrost_ui::prefs_layout::m as pm;
        let (x, y, w, h) = (self.margin, sb.card_y + dy, self.w as f32 - 2.0 * self.margin, ui.pt(SIDEBAR_CARD_PT));
        if y + h < 0.0 || y > self.h as f32 {
            return;
        }
        let _ = w;
        let d = ui.pt(pm::ACCOUNT_AVATAR);
        let (ax, ay) = (x + ui.pt(6.0), y + (h - d) / 2.0);
        s.fill_rrect(ax, ay, d, d, d / 2.0, AVATAR_BG);
        let head = d * 0.32;
        s.fill_rrect(ax + (d - head) / 2.0, ay + d * 0.22, head, head, head / 2.0, AVATAR_FG);
        s.fill_rrect(ax + (d - d * 0.54) / 2.0, ay + d * 0.62, d * 0.54, d * 0.27, d * 0.17, AVATAR_FG);
        let tx = ax + d + ui.pt(8.0);
        let name = &sb.card_title;
        let lines_h = name.h as f32 + sb.card_lines.first().map_or(0.0, |l| l.h as f32);
        let ty = y + (h - lines_h) / 2.0;
        s.blit_colored(name, tx as i32, ty as i32, LABEL.now(), 255);
        if let Some(l) = sb.card_lines.first() {
            s.blit(l, tx as i32, (ty + name.h as f32) as i32, 255, 255);
        }
    }

    fn draw_row(&self, s: &mut Surface, ui: &Ui, g: usize, i: usize, item: &Item, pressed: Option<Hit>, t: f64) {
        let k = self.row_index(g, i);
        let r = &self.rows[k];
        let m = self.margin;
        let cw = self.w as f32 - 2.0 * m;
        let (ry, rh) = (r.y - self.draw_offset(), r.h);
        if ry + rh < 0.0 || ry > self.h as f32 {
            return;
        }
        let n = self.spec.groups[g].items.len();
        let selected = matches!((&self.sidebar, &item.action), (Some(sb), Action::Push(id)) if *id == sb.selected);
        let is_pressed = item.tappable() && pressed == Some(Hit::Row(g, i));
        if self.sidebar.is_some() {
            // a sidebar row is its own rounded plate (30 pt) inside a 32 pt pitch, not part of a card
            let plate = (m, ry + ui.pt((SIDEBAR_ROW_PT - SIDEBAR_SEL_H_PT) / 2.0), cw, ui.pt(SIDEBAR_SEL_H_PT));
            if selected {
                s.fill_rrect(plate.0, plate.1, plate.2, plate.3, ui.pt(bifrost_ui::prefs_layout::m::SELECTION_RADIUS), bifrost_render::appearance::accent_now());
            } else if is_pressed {
                s.fill_rrect(plate.0, plate.1, plate.2, plate.3, ui.pt(bifrost_ui::prefs_layout::m::SELECTION_RADIUS), ROW_PRESSED.now());
            }
        } else if is_pressed {
            fill_row(s, ui, (m, ry, cw, rh), i == 0, i + 1 == n, ROW_PRESSED.now());
        }
        let mid = ry + rh / 2.0;
        let mut lx = m + ui.pt(TEXT_INSET_PT);
        match &item.body {
            Body::Slider { ends, value } => {
                let (tx, ty, tw) = self.track(ui, g, i);
                self.draw_ends(s, ui, *ends, m, tx, tw, ty);
                draw_slider(s, ui, tx, ty, tw, *value);
            }
            Body::Graph(graph) => graph.draw(s, ui, (m + ui.pt(TEXT_INSET_PT), ry + ui.pt(12.0), cw - ui.pt(2.0 * TEXT_INSET_PT), rh - ui.pt(24.0))),
            Body::Bar(bar) => bar.draw(s, ui, (m + ui.pt(TEXT_INSET_PT), ry, cw - ui.pt(2.0 * TEXT_INSET_PT), rh)),
            Body::Chart(chart) => chart.draw(s, ui, (m + ui.pt(TEXT_INSET_PT), ry + ui.pt(12.0), cw - ui.pt(2.0 * TEXT_INSET_PT), rh - ui.pt(24.0))),
            Body::Row => {
                let (icon_left, text_left) = leading_x(ui, m, item, self.sidebar.is_some());
                lx = text_left;
                if has_leading(item) {
                    let icon_pt = row_icon(self.sidebar.is_some()).0;
                    match (item.icon, item.glyph) {
                        (Some(icon), _) => icons::tile_sized(s, ui, icon, icon_left, mid - ui.pt(icon_pt) / 2.0, icon_pt),
                        (None, Some(glyph)) => super::row_glyphs::tile(s, ui, glyph, icon_left, mid - ui.pt(icon_pt) / 2.0, icon_pt),
                        (None, None) => {
                            if let Some(img) = &self.images[k].icon {
                                s.blit(img, icon_left as i32, (mid - img.h as f32 / 2.0) as i32, 255, 255);
                            }
                        }
                    }
                }
                if item.leading_check {
                    s.blit_colored(&self.check, lx as i32, (mid - self.check.h as f32 / 2.0) as i32, bifrost_render::appearance::accent_now(), 255);
                    lx += ui.pt(LEADING_CHECK_PT);
                }
                // a label of two lines is centred on the row as a block
                let labels = &self.images[k].label;
                let block: f32 = labels.iter().map(|l| l.h as f32).sum();
                let mut ly = mid - block / 2.0;
                for label in labels.iter().filter(|_| item.accessory != Accessory::Segmented) {
                    // plain labels are white images drawn in the scheme's label colour; the selected sidebar row stays white
                    if item.tint == Tint::Plain && !selected {
                        s.blit_colored(label, lx as i32, ly as i32, LABEL.now(), 255);
                    } else {
                        s.blit(label, lx as i32, ly as i32, 255, 255);
                    }
                    ly += label.h as f32;
                }
                let mut right = m + cw - ui.pt(TEXT_INSET_PT);
                match item.accessory {
                    Accessory::None => {}
                    Accessory::Chevron if self.sidebar.is_some() => {}
                    Accessory::Chevron => {
                        let ch = &self.chevron;
                        s.blit(ch, (right - ch.w as f32 + ui.pt(2.0)) as i32, (mid - ch.h as f32 / 2.0) as i32, 255, 255);
                        right -= ch.w as f32 + ui.pt(6.0);
                    }
                    Accessory::Switch(on) => {
                        let p = switch_progress(on, self.flipped[k], t);
                        draw_switch(s, ui, right - ui.pt(SWITCH_W_PT), mid - ui.pt(SWITCH_H_PT) / 2.0, p);
                        right -= ui.pt(SWITCH_W_PT + 12.0);
                    }
                    Accessory::Check(on) => {
                        if on {
                            s.blit_colored(&self.check, (right - self.check.w as f32) as i32, (mid - self.check.h as f32 / 2.0) as i32, bifrost_render::appearance::accent_now(), 255);
                        }
                        right -= self.check.w as f32 + ui.pt(8.0);
                    }
                    Accessory::Popup => {
                        // the chosen label, then up and down chevrons at the right edge
                        let label_w = self.images[k].popup.as_ref().map_or(0.0, |v| v.w as f32);
                        let (cx, label_left) = popup_layout(ui, right, label_w);
                        let (a, g, th, col) = (ui.pt(2.5), ui.pt(2.0), ui.pt(1.5), LABEL_2.now());
                        icons::line(s, (cx - a, mid - g + a), (cx, mid - g), th, col);
                        icons::line(s, (cx, mid - g), (cx + a, mid - g + a), th, col);
                        icons::line(s, (cx - a, mid + g - a), (cx, mid + g), th, col);
                        icons::line(s, (cx, mid + g), (cx + a, mid + g - a), th, col);
                        if let Some(v) = &self.images[k].popup {
                            s.blit_colored(v, label_left as i32, (mid - v.h as f32 / 2.0) as i32, LABEL.now(), 255);
                        }
                        right = label_left;
                    }
                    Accessory::Segmented => {
                        if let Some(seg) = &item.segments {
                            self.draw_segments(s, ui, (m, ry, cw, rh), seg);
                        }
                    }
                    Accessory::Info => {
                        icons::info(s, ui, right - ui.pt(11.0), mid);
                        right -= ui.pt(22.0 + 16.0);
                    }
                    Accessory::Wifi { secure, bars } => {
                        let size = ui.pt(12.0);
                        draw_wifi_bars(s, right - 2.0 * size + size * 0.29, mid - size * 0.55, size, bars, WHITE);
                        right -= size * 1.42 + ui.pt(10.0);
                        if secure {
                            icons::lock(s, ui, right - ui.pt(9.0), mid - ui.pt(6.5), WHITE);
                            right -= ui.pt(9.0 + 10.0);
                        }
                    }
                }
                if item.badge > 0 {
                    let b = super::notify::list::badge_image(ui, item.badge);
                    s.blit(&b, (right - b.w as f32) as i32, (mid - b.h as f32 / 2.0) as i32, 255, 255);
                    right -= b.w as f32 + ui.pt(8.0);
                }
                // a sidebar row is a destination: its status value lives on the detail page
                if let (None, Some(v)) = (&self.sidebar, &self.images[k].value) {
                    s.blit(v, (right - v.w as f32) as i32, (mid - v.h as f32 / 2.0) as i32, 255, 255);
                }
            }
        }
        if i + 1 < n && self.sidebar.is_none() {
            let sep_x = if matches!(item.body, Body::Row) { lx } else { m + ui.pt(TEXT_INSET_PT) };
            s.fill_rrect(sep_x, ry + rh - 1.0, m + cw - sep_x, 1.0, 0.0, SEPARATOR_C.now());
        }
    }

    /// A segmented row's track in px: the card's width less the inset on each side, `SEGMENT_H` tall, centred on the row.
    fn segment_track(&self, ui: &Ui, row: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
        use bifrost_ui::prefs_layout::m as pm;
        let (x, y, w, h) = row;
        let (inset, th) = (ui.pt(pm::SEGMENT_INSET), ui.pt(pm::SEGMENT_H));
        (x + inset, y + (h - th) / 2.0, (w - 2.0 * inset).max(0.0), th)
    }

    /// The segmented control: a grey track with the chosen segment filled in the accent colour, a label in each.
    fn draw_segments(&self, s: &mut Surface, ui: &Ui, row: (f32, f32, f32, f32), seg: &Popup) {
        let (tx, ty, tw, th) = self.segment_track(ui, row);
        let n = seg.choices.len().max(1);
        let sw = tw / n as f32;
        s.fill_rrect(tx, ty, tw, th, ui.pt(7.0), ROW_PRESSED.now());
        for (k, choice) in seg.choices.iter().enumerate() {
            let on = k == seg.selected;
            let x = tx + sw * k as f32;
            if on {
                s.fill_rrect(x, ty, sw, th, ui.pt(7.0), bifrost_render::appearance::accent_now());
            }
            let img = ui.text(&ui.fit(choice, MAC_BODY_PT, sw - ui.pt(8.0)), MAC_BODY_PT, Weight::Regular, WHITE);
            let (ix, iy) = ((x + (sw - img.w as f32) / 2.0) as i32, (ty + (th - img.h as f32) / 2.0) as i32);
            if on {
                s.blit(&img, ix, iy, 255, 255);
            } else {
                s.blit_colored(&img, ix, iy, LABEL.now(), 255);
            }
        }
    }

    /// The glyphs at either end of a slider's track.
    fn draw_ends(&self, s: &mut Surface, ui: &Ui, ends: Ends, m: f32, tx: f32, tw: f32, ty: f32) {
        let (lx, rx) = (m + ui.pt(26.0), tx + tw + ui.pt(26.0));
        match ends {
            Ends::Brightness => {
                for (cx, sym, d) in [(lx, crate::symbols::Symbol::SunMin, 11.0), (rx, crate::symbols::Symbol::SunMax, 18.0)] {
                    match ui.symbols.render_fit(sym, ui.pt(d), ui.pt(d), SECONDARY) {
                        Some(img) => s.blit(&img, (cx - img.w as f32 / 2.0) as i32, (ty - img.h as f32 / 2.0) as i32, 255, 255),
                        None => icons::disc(s, cx, ty, ui.pt(d) / 2.0 - ui.pt(2.0), SECONDARY),
                    }
                }
            }
            Ends::Volume => {
                for (cx, waves) in [(lx, 0), (rx, 2)] {
                    let p = |px: f32, py: f32| (cx - ui.pt(7.0) + ui.pt(px), ty - ui.pt(7.0) + ui.pt(py));
                    icons::polygon(s, &[p(0.0, 4.5), p(3.5, 4.5), p(8.0, 1.0), p(8.0, 13.0), p(3.5, 9.5), p(0.0, 9.5)], SECONDARY);
                    let q = std::f32::consts::FRAC_PI_4;
                    for w in 0..waves {
                        let (ax, ay) = p(8.5, 7.0);
                        icons::arc(s, ax, ay, ui.pt(3.5 + 3.5 * w as f32), ui.pt(1.5), -q, q, SECONDARY);
                    }
                }
            }
            Ends::Glass => {
                let d = ui.pt(14.0);
                for (cx, filled) in [(lx, false), (rx, true)] {
                    if filled {
                        s.fill_rrect(cx - d / 2.0, ty - d / 2.0, d, d, ui.pt(4.0), SECONDARY);
                    } else {
                        s.stroke_rrect(cx - d / 2.0, ty - d / 2.0, d, d, ui.pt(4.0), ui.pt(1.5), SECONDARY);
                    }
                }
            }
            Ends::Delay | Ends::Rate => {
                let (left, right) = if ends == Ends::Delay { ("Long", "Short") } else { ("Slow", "Fast") };
                for (cx, label) in [(lx, left), (rx, right)] {
                    let a = ui.chrome(label, 11.0, Weight::Regular, SECONDARY);
                    let x = if cx < tx { m } else { tx + tw + ui.pt(26.0) * 2.0 - a.w as f32 };
                    s.blit(&a, x as i32, (ty + ui.pt(KNOB_PT) / 2.0 + ui.pt(4.0)) as i32, 255, 255);
                }
            }
            Ends::Speed => {
                for (cx, label) in [(lx, "Slower"), (rx, "Faster")] {
                    let a = ui.chrome(label, 11.0, Weight::Regular, SECONDARY);
                    let x = if cx < tx { m } else { tx + tw + ui.pt(26.0) * 2.0 - a.w as f32 };
                    s.blit(&a, x as i32, (ty + ui.pt(KNOB_PT) / 2.0 + ui.pt(4.0)) as i32, 255, 255);
                }
            }
            Ends::Cursor => {
                for (cx, label) in [(lx, "Normal"), (rx, "Large")] {
                    let a = ui.chrome(label, 11.0, Weight::Regular, SECONDARY);
                    let x = if cx < tx { m } else { tx + tw + ui.pt(26.0) * 2.0 - a.w as f32 };
                    s.blit(&a, x as i32, (ty + ui.pt(KNOB_PT) / 2.0 + ui.pt(4.0)) as i32, 255, 255);
                }
            }
            Ends::Warmth => {
                for (cx, label) in [(lx, "Less Warm"), (rx, "More Warm")] {
                    let a = ui.chrome(label, 11.0, Weight::Regular, SECONDARY);
                    let x = if cx < tx { m } else { tx + tw + ui.pt(26.0) * 2.0 - a.w as f32 };
                    s.blit(&a, x as i32, (ty + ui.pt(KNOB_PT) / 2.0 + ui.pt(4.0)) as i32, 255, 255);
                }
            }
            Ends::TextSize => {
                for (cx, pt) in [(lx, 12.0), (rx, 20.0)] {
                    let a = ui.chrome("A", pt, Weight::Regular, SECONDARY);
                    s.blit(&a, (cx - a.w as f32 / 2.0) as i32, (ty - a.h as f32 / 2.0) as i32, 255, 255);
                }
                // the notches under the track
                let n = super::options::TEXT_SIZES.len();
                for k in 0..n {
                    let x = tx + tw * k as f32 / (n - 1) as f32;
                    s.fill_rrect(x - 0.5, ty + ui.pt(KNOB_PT) / 2.0 + ui.pt(3.0), 1.0, ui.pt(4.0), 0.0, SECONDARY);
                }
            }
        }
    }

}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::pages::Toggle;

    #[test]
    fn downscaled_blur_stays_close_to_the_full_size_blur() {
        let (w, h, radius) = (300usize, 100usize, 12usize);
        // a soft gradient with a few hard-edged "rows" like list content under the bar
        let mut px: Vec<u32> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let g = if (y / 20) % 2 == 0 && x > 40 && x < 260 { 220 } else { (x * 255 / w) as u32 };
                0xff00_0000 | g << 16 | (255 - g) << 8 | (y as u32 * 2)
            })
            .collect();
        let mut reference = Image::new(w, h);
        reference.px.copy_from_slice(&px);
        reference.blur(radius);
        blur_rows(&mut px, w, h, w, radius);
        let (mut err, mut worst) = (0u64, 0u32);
        for (a, b) in px.iter().zip(&reference.px) {
            for ch in 0..3 {
                let d = (((a >> (ch * 8)) & 255) as i32 - ((b >> (ch * 8)) & 255) as i32).unsigned_abs();
                err += d as u64;
                worst = worst.max(d);
            }
            assert_eq!(a >> 24, 255);
        }
        let mean = err as f64 / (w * h * 3) as f64;
        assert!(mean < 3.0, "mean error {mean}");
        assert!(worst < 60, "worst error {worst}");
    }

    fn ui() -> Ui {
        crate::shell::tests::ui()
    }

    fn spec(large: bool, n: usize) -> Spec {
        let items = |k: usize| (0..k).map(|i| Item::new(format!("Row {i}")).value("v").link(PageId::General)).collect();
        Spec {
            id: PageId::Root,
            title: "Settings".into(),
            large,
            groups: vec![
                Group::new(vec![Item::new("Airplane Mode").icon(Icon::Airplane).switch(false, Action::Toggle(Toggle::Airplane)), Item::new("Wi-Fi").icon(Icon::Wifi).value("Home").link(PageId::Wifi)]),
                Group::new(items(n)).header("MORE").footer("A footer that explains the group in a sentence or two of grey text."),
                Group::new(vec![Item::slider(Ends::Brightness, 0.5, Action::Slider(crate::shell::pages::Slider::Brightness))]),
            ],
        }
    }

    fn buf() -> Vec<u32> {
        vec![0u32; 1200 * 1920]
    }

    fn draw(page: &ListPage, ui: &Ui, buf: &mut [u32], pressed: Option<Hit>, t: f64) {
        page.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: buf, y0: 0 }, ui, pressed, t);
    }

    #[test]
    fn rows_stack_with_gaps_headers_and_footers() {
        let ui = ui();
        let page = ListPage::new(&ui, spec(true, 3), 1200, 1920, None);
        let (x, y0, w, h) = page.row_rect(&ui, 0, 0);
        assert_eq!(x, ui.pt(MARGIN_PT));
        assert_eq!(w, 1200.0 - 2.0 * ui.pt(MARGIN_PT));
        assert_eq!(y0, ui.pt(STATUS_PT + NAV_PT + LARGE_TITLE_PT + crate::shell::settings_search::FIELD_PT + GROUP_GAP_PT));
        assert_eq!(h, ui.pt(ROW_PT));
        assert_eq!(page.row_rect(&ui, 0, 1).1, y0 + h);
        let (_, y1, _, _) = page.row_rect(&ui, 1, 0);
        assert_eq!(y1, y0 + 2.0 * h + ui.pt(GROUP_GAP_PT + HEADER_PT), "a header makes the gap taller");
        let (_, y2, _, h2) = page.row_rect(&ui, 2, 0);
        assert!(y2 >= y1 + 3.0 * h + ui.pt(GROUP_GAP_PT + FOOTER_GAP_PT + FOOTER_LINE_PT), "the footer pushes the next group down");
        let no_footer = ListPage::new(&ui, Spec { groups: spec(true, 3).groups.into_iter().map(|g| Group { footer: None, ..g }).collect(), ..spec(true, 3) }, 1200, 1920, None);
        assert_eq!(no_footer.row_rect(&ui, 2, 0).1, y1 + 3.0 * h + ui.pt(GROUP_GAP_PT));
        assert_eq!(h2, ui.pt(SLIDER_ROW_PT));
        assert_eq!(page.find("Wi-Fi"), Some((0, 1)));
        assert_eq!(page.find("Nope"), None);
        let inline = ListPage::new(&ui, spec(false, 1), 1200, 1920, Some("Settings"));
        assert_eq!(inline.row_rect(&ui, 0, 0).1, ui.pt(STATUS_PT + NAV_PT + GROUP_GAP_PT));
    }

    #[test]
    fn hits_resolve_rows_sliders_and_back() {
        let ui = ui();
        let page = ListPage::new(&ui, spec(false, 2), 1200, 1920, Some("Settings"));
        let (cx, cy) = page.row_centre(&ui, 0, 1);
        assert_eq!(page.hit(&ui, cx, cy), Some(Hit::Row(0, 1)));
        assert_eq!(page.hit(&ui, 5.0, cy), None, "outside the margins");
        let (sx, sy) = page.row_centre(&ui, 2, 0);
        assert_eq!(page.hit(&ui, sx, sy), Some(Hit::Slider(2, 0)));
        assert_eq!(page.hit(&ui, ui.pt(40.0), ui.pt(42.0)), Some(Hit::Back));
        assert_eq!(page.hit(&ui, 600.0, ui.pt(42.0)), None, "the title is not a button");
        let root = ListPage::new(&ui, spec(true, 2), 1200, 1920, None);
        assert_eq!(root.hit(&ui, ui.pt(40.0), ui.pt(42.0)), None, "no back on the root");
        let (gx, gy) = root.row_centre(&ui, 1, 0);
        assert_eq!(root.hit(&ui, gx, gy - ui.pt(ROW_PT)), None, "the header is not a row");
    }

    #[test]
    fn long_pages_scroll_and_hits_follow_the_offset() {
        let ui = ui();
        let mut page = ListPage::new(&ui, spec(true, 40), 1200, 1920, None);
        let before = page.row_rect(&ui, 1, 10).1;
        page.scroll_down(1500.0, 0.0);
        for k in 1..=10 {
            page.scroll_moved(1500.0 - k as f32 * 50.0, k as f64 * 40.0);
        }
        assert!(page.dragging());
        assert!(!page.scroll_up(1000.0, 1000.0), "a drag is not a tap");
        let mut t = 1000.0;
        while page.tick(t) {
            t += 16.0;
        }
        // the drag follows the finger from where it left the slop, so the content never jumps
        assert!((page.offset() - 488.0).abs() < 1.0, "a paused release stops where it is: {}", page.offset());
        assert_eq!(page.row_rect(&ui, 1, 10).1, before - page.offset());
        let (cx, cy) = page.row_centre(&ui, 1, 10);
        assert_eq!(page.hit(&ui, cx, cy), Some(Hit::Row(1, 10)));
        // a short page never scrolls
        let mut short = ListPage::new(&ui, spec(true, 2), 1200, 1920, None);
        short.scroll_down(1500.0, 0.0);
        short.scroll_moved(1000.0, 100.0);
        assert!(short.offset() < 0.0 || short.offset() > 0.0, "rubber band while dragging");
        short.scroll_up(1000.0, 1000.0);
        let mut t = 1000.0;
        while short.tick(t) {
            t += 16.0;
        }
        assert_eq!(short.offset(), 0.0);
        // a tap without movement
        short.scroll_down(800.0, 2000.0);
        assert!(short.scroll_up(802.0, 2050.0));
    }

    #[test]
    fn large_title_collapses_into_the_bar() {
        let ui = ui();
        let mut page = ListPage::new(&ui, spec(true, 40), 1200, 1920, None);
        let mut top = buf();
        draw(&page, &ui, &mut top, None, 0.0);
        let bar_px = |b: &[u32]| b[(ui.pt(STATUS_PT + 10.0) as usize) * 1200 + 600];
        assert_eq!(bar_px(&top), 0, "no bar over the large title");
        let title_y = ui.pt(STATUS_PT + NAV_PT + 20.0) as usize;
        assert!(top[title_y * 1200..(title_y + 1) * 1200].iter().take(300).any(|&p| p != top[title_y * 1200 + 1100]), "the large title");
        page.scroll_down(1500.0, 0.0);
        page.scroll_moved(1000.0, 50.0);
        page.scroll_moved(900.0, 100.0);
        let mut scrolled = buf();
        draw(&page, &ui, &mut scrolled, None, 100.0);
        assert_ne!(bar_px(&scrolled), 0, "a tinted blur bar once scrolled");
        let mid = ui.pt(STATUS_PT + NAV_PT / 2.0) as usize;
        assert!(scrolled[mid * 1200 + 500..mid * 1200 + 700].iter().any(|&p| p != scrolled[mid * 1200 + 100]), "the small title in the bar");
    }

    #[test]
    fn switches_animate_and_values_rerender() {
        let ui = ui();
        let mut page = ListPage::new(&ui, spec(false, 1), 1200, 1920, Some("Settings"));
        assert!(!page.animating(0.0));
        page.set_switch(0, 0, true, 1000.0);
        assert!(page.animating(1050.0));
        assert!(!page.animating(1000.0 + SWITCH_MS));
        assert_eq!(page.item(0, 0).accessory, Accessory::Switch(true));
        let (rx, ry, rw, rh) = page.row_rect(&ui, 0, 0);
        let knob_x = |t: f64| {
            let mut b = buf();
            draw(&page, &ui, &mut b, None, t);
            let row = (ry + rh / 2.0) as usize;
            (0..1200).filter(|&x| x > (rx + rw - ui.pt(70.0)) as usize && b[row * 1200 + x] == WHITE).min().unwrap()
        };
        assert!(knob_x(1000.0) < knob_x(1300.0), "the knob slides right");
        page.set_switch(0, 0, true, 2000.0);
        assert!(!page.animating(2050.0), "setting the same state does not animate");
        page.set_value(&ui, 0, 1, "Cafe");
        assert_eq!(page.item(0, 1).value, "Cafe");
        page.set_slider(2, 0, 1.5);
        assert_eq!(page.item(2, 0).body, Body::Slider { ends: Ends::Brightness, value: 1.0 });
        let mut pressed = buf();
        draw(&page, &ui, &mut pressed, Some(Hit::Row(0, 1)), 0.0);
        let (_, y, _, h) = page.row_rect(&ui, 0, 1);
        assert_eq!(pressed[(y + h / 2.0) as usize * 1200 + (rx + 4.0) as usize], ROW_PRESSED.now());
    }

    #[test]
    fn a_leading_app_icon_takes_the_icon_slot_and_pushes_the_label() {
        let ui = ui();
        let plain = Item::new("Camera").value("v");
        let app = Item::new("Camera").app_icon("is.example.camera");
        let m = ui.pt(MARGIN_PT);
        let (icon_x, label_x) = leading_x(&ui, m, &app, false);
        assert_eq!(icon_x, m + ui.pt(bifrost_ui::prefs_layout::m::ROW_ICON_X), "the icon at the detail row's icon token");
        assert!((label_x - (m + ui.pt(bifrost_ui::prefs_layout::m::ROW_LABEL_X))).abs() < 0.01, "the label after the icon and its gap");
        let (plain_icon, plain_label) = leading_x(&ui, m, &plain, false);
        assert_eq!(plain_label, m + ui.pt(TEXT_INSET_PT), "a plain row keeps the text inset");
        assert_eq!(plain_icon, plain_label, "no icon slot without a leading accessory");
        let (sidebar_icon, sidebar_label) = leading_x(&ui, m, &app, true);
        assert!(sidebar_label > sidebar_icon, "the label follows the sidebar icon");
        // a built-in app has no installed package, so its icon is blank, but the label still moves in
        let page = ListPage::new(&ui, Spec { id: PageId::Root, title: "T".into(), large: false, groups: vec![Group::new(vec![app.clone()])] }, 1200, 1920, None);
        assert!(page.images[0].icon.is_none());
        assert_eq!(leading_x(&ui, page.margin, page.item(0, 0), false).1, label_x);
    }

    #[test]
    fn a_popup_button_sits_at_the_row_edge_with_its_chevrons_and_label_before_them() {
        let ui = ui();
        let right = 1000.0;
        let (chev, label_left) = popup_layout(&ui, right, 120.0);
        assert_eq!(chev, right - ui.pt(POPUP_CHEV_PT / 2.0), "the chevron pair is the row's last slot");
        assert_eq!(label_left, right - ui.pt(POPUP_CHEV_PT + POPUP_GAP_PT) - 120.0, "the label ends a gap before the chevrons");
        assert!(label_left < chev, "the label is left of the chevrons");
        assert_eq!(popup_layout(&ui, right, 0.0).1, right - ui.pt(POPUP_CHEV_PT + POPUP_GAP_PT));
    }

    #[test]
    fn a_pop_up_row_opens_a_menu_over_its_button_and_keeps_it_inside_the_list() {
        let ui = ui();
        let choices = vec!["Never".to_string(), "Ask Next Time".to_string(), "While Using the App".to_string()];
        let access = Item::new("Maps").app_icon("is.example.maps").popup(choices.clone(), 1, Action::Pick(crate::shell::pages::Pick::AutoLock(0)));
        let spec = Spec { id: PageId::AutoLock, title: "Privacy".into(), large: false, groups: vec![Group::new(vec![Item::new("Before"), access, Item::new("After")])] };
        let mut page = ListPage::new(&ui, spec, 1200, 1920, Some("Privacy"));
        assert!(!page.popup_open());
        assert!(!page.open_popup(&ui, 0, 0), "a plain row has no pop-up");
        assert!(page.open_popup(&ui, 0, 1));
        assert!(page.popup_open());
        assert_eq!(page.hit(&ui, 5.0, 300.0), Some(Hit::Dismiss), "a touch outside the menu closes it");
        // the selected choice sits over the button: the menu's top is the row's top less the selected choice's offset
        let (mx, my, mw, mh) = page.popup_rect(&ui).expect("open");
        let (_, ry, _, _) = page.row_rect(&ui, 0, 1);
        let menu = bifrost_ui::menu::Menu::popup((0..3).map(|k| bifrost_ui::menu::MenuItem::new(k, &choices[k as usize])).collect(), 1);
        let (spans, total) = menu.rows(POPUP_ROW_PT, POPUP_SEP_PT, POPUP_PAD_PT);
        assert!((my - (ry - ui.pt(spans[1].0))).abs() < 1.0, "the selected choice covers the row");
        assert!((mh - ui.pt(total)).abs() < 0.5 && mw >= ui.pt(POPUP_MIN_W_PT));
        // each choice is hit by its own centre; the top padding is not
        for (k, (top, h)) in spans.iter().enumerate().take(3) {
            let (cx, cy) = (mx + mw / 2.0, my + ui.pt(top + h / 2.0));
            assert_eq!(page.hit(&ui, cx, cy), Some(Hit::Choice(k)), "choice {k}");
        }
        assert_eq!(page.hit(&ui, mx + mw / 2.0, my + 0.5), None, "the top padding does nothing");
        assert_eq!(page.close_popup(), Some((0, 1)));
        assert!(!page.popup_open());
        let (rx, rcy) = page.row_centre(&ui, 0, 1);
        assert_eq!(page.hit(&ui, rx, rcy), Some(Hit::Row(0, 1)), "closed, the row is tappable again");
    }

    #[test]
    fn checkmarks_move_within_their_group() {
        let ui = ui();
        let items = (0..3).map(|i| Item::new(format!("Choice {i}")).check(i == 0, Action::Pick(crate::shell::pages::Pick::AutoLock(i)))).collect();
        let mut page = ListPage::new(&ui, Spec { id: PageId::AutoLock, title: "Auto-Lock".into(), large: false, groups: vec![Group::new(items)] }, 1200, 1920, Some("Display"));
        page.set_check(0, 2);
        assert_eq!(page.item(0, 0).accessory, Accessory::Check(false));
        assert_eq!(page.item(0, 2).accessory, Accessory::Check(true));
        let mut b = buf();
        draw(&page, &ui, &mut b, None, 0.0);
        let (rx, ry, rw, rh) = page.row_rect(&ui, 0, 2);
        let row = (ry + rh / 2.0) as usize;
        assert!(b[row * 1200 + (rx + rw - ui.pt(30.0)) as usize..row * 1200 + (rx + rw) as usize].iter().any(|&p| p == bifrost_render::appearance::accent_now()), "a check in the accent colour on the chosen row");
    }
    #[test]
    fn group_headers_are_title_case_headlines() {
        assert_eq!(headline_case("DOCK POSITION"), "Dock Position");
        assert_eq!(headline_case("NOTIFICATION STYLE"), "Notification Style");
        assert_eq!(headline_case("ALLOW IN THE BACKGROUND"), "Allow in the Background", "a small word inside a header stays low");
        assert_eq!(headline_case("PUT THE COMPUTER TO SLEEP WHEN THE DISPLAY IS OFF"), "Put the Computer to Sleep When the Display Is Off", "verbs are capitalised");
        assert_eq!(headline_case("START THIS COMPUTER FROM"), "Start This Computer From", "a final preposition is capitalised");
        assert_eq!(headline_case("ON"), "On", "the first word always starts with a capital");
        assert_eq!(headline_case("DOUBLE-CLICK SPEED"), "Double-Click Speed");
        assert_eq!(headline_case("WHAT'S NEW IN 1.2"), "What's New in 1.2");
        assert_eq!(headline_case("USB DEVICES"), "USB Devices");
        assert_eq!(headline_case("IPV4"), "IPv4");
        assert_eq!(headline_case("is.example.chat"), "is.example.chat", "a name with lower-case letters is left alone");
        assert_eq!(headline_case("My iPhone"), "My iPhone");
        assert_eq!(headline_case(""), "");
    }

    #[test]
    fn a_row_label_wraps_or_ends_in_an_ellipsis_before_the_trailing_control() {
        let ui = ui();
        assert_eq!(label_lines(&ui, "Short", 400.0, true), ["Short"]);
        assert_eq!(label_lines(&ui, "", 400.0, true), [""]);
        let long = "word ".repeat(60);
        let wrapped = label_lines(&ui, &long, 300.0, true);
        assert_eq!(wrapped.len(), LABEL_LINES, "never more lines than the row has room for");
        assert!(wrapped[1].ends_with('\u{2026}'), "the last line ends with an ellipsis when the text is still longer");
        assert_eq!(label_lines(&ui, &long, 300.0, false).len(), 1, "a sidebar label keeps to one line");
        // the example from the E2E run: a label that fits on two lines wraps and loses nothing
        let two = label_lines(&ui, "When Switching to an Application, Switch to a Desktop with Open Windows", ui.measure("When Switching to an Application, Switch to a", MAC_BODY_PT, Weight::Regular) + 4.0, true);
        assert_eq!(two.len(), 2);
        assert!(!two[1].ends_with('\u{2026}'));
        assert_eq!(two.join(" "), "When Switching to an Application, Switch to a Desktop with Open Windows");
    }

    #[test]
    fn a_long_label_stops_before_its_switch() {
        let ui = ui();
        let label = "When Switching to an Application, Switch to a Desktop with Open Windows";
        let spec = Spec { id: PageId::Root, title: "T".into(), large: false, groups: vec![Group::new(vec![Item::new(label).switch(true, Action::Toggle(Toggle::Airplane)), Item::new("Short").switch(true, Action::Toggle(Toggle::Airplane))])] };
        let page = ListPage::new(&ui, spec, 700, 1920, Some("Back"));
        // what the row leaves for the label: the card less both insets, the switch and the gap before it
        let room = 700.0 - 2.0 * ui.pt(MARGIN_PT) - 2.0 * ui.pt(TEXT_INSET_PT) - ui.pt(SWITCH_W_PT + 12.0) - ui.pt(8.0);
        assert_eq!(page.images[0].label.len(), 2, "the long label wraps onto a second line");
        for img in &page.images[0].label {
            assert!(img.w as f32 <= room + 2.0, "a line {} px wide runs under the switch ({room} px left)", img.w);
        }
        assert_eq!(page.images[1].label.len(), 1, "a short label stays on one line");
        // both lines are inside the 49 pt row
        let total: f32 = page.images[0].label.iter().map(|l| l.h as f32).sum();
        assert!(total <= ui.pt(ROW_PT), "two lines of {total} px fit the {} px row", ui.pt(ROW_PT));
    }

    #[test]
    fn a_segmented_row_chooses_the_segment_under_the_touch() {
        let ui = ui();
        let row = Item::new("Range").segmented(vec!["Day".into(), "Week".into()], 0, Action::Pick(crate::shell::pages::Pick::AutoLock(0)));
        let spec = Spec { id: PageId::AutoLock, title: "T".into(), large: false, groups: vec![Group::new(vec![row])] };
        let mut page = ListPage::new(&ui, spec, 1200, 1920, Some("Back"));
        let (x, y, w, h) = page.row_rect(&ui, 0, 0);
        assert_eq!(page.hit(&ui, x + w * 0.25, y + h / 2.0), Some(Hit::Segment(0, 0, 0)));
        assert_eq!(page.hit(&ui, x + w * 0.75, y + h / 2.0), Some(Hit::Segment(0, 0, 1)));
        assert_eq!(page.hit(&ui, x + 1.0, y + h / 2.0), Some(Hit::Segment(0, 0, 0)), "the rim goes to the nearest segment");
        assert_eq!(page.hit(&ui, x + w - 1.0, y + h / 2.0), Some(Hit::Segment(0, 0, 1)));
        let accent = bifrost_render::appearance::accent_now();
        let filled = |page: &ListPage, left: bool| {
            let mut b = buf();
            draw(page, &ui, &mut b, None, 0.0);
            let row = (y + h / 2.0) as usize;
            let (lo, hi) = if left { (x + w * 0.1, x + w * 0.4) } else { (x + w * 0.6, x + w * 0.9) };
            (lo as usize..hi as usize).any(|px| b[row * 1200 + px] == accent)
        };
        assert!(filled(&page, true) && !filled(&page, false), "the chosen segment is filled in the accent");
        page.set_choice(&ui, 0, 0, 1);
        assert_eq!(page.item(0, 0).segments.as_ref().map(|p| p.selected), Some(1));
        assert!(filled(&page, false) && !filled(&page, true), "the fill follows the choice");
    }

    #[test]
    fn a_choice_updates_the_pop_up_button_on_its_row() {
        let ui = ui();
        let choices = vec!["Never".to_string(), "Ask".to_string(), "Always".to_string()];
        let spec = Spec { id: PageId::AutoLock, title: "T".into(), large: false, groups: vec![Group::new(vec![Item::new("Access").popup(choices, 0, Action::Pick(crate::shell::pages::Pick::AutoLock(0)))])] };
        let mut page = ListPage::new(&ui, spec, 1200, 1920, Some("Back"));
        let before = page.images[0].popup.as_ref().map(|p| p.w);
        page.set_choice(&ui, 0, 0, 2);
        assert_eq!(page.item(0, 0).popup.as_ref().map(|p| p.selected), Some(2));
        assert_ne!(page.images[0].popup.as_ref().map(|p| p.w), before, "\"Always\" is wider than \"Never\"");
        page.set_choice(&ui, 0, 0, 9);
        assert_eq!(page.item(0, 0).popup.as_ref().map(|p| p.selected), Some(2), "an index past the list stays on the last choice");
    }

    #[test]
    fn only_general_opens_with_the_hero_and_two_panes_with_the_compact_card() {
        use crate::shell::pages::PageId as P;
        assert!(matches!(header_for(P::General), Some((_, _, false))), "General: the large centred hero");
        assert!(matches!(header_for(P::Notifications), Some((_, _, true))), "Notifications: one compact card");
        assert!(matches!(header_for(P::ScreenTime), Some((_, _, true))));
        for id in [P::Display, P::Sounds, P::Focus, P::ControlCentre, P::Sharing, P::LockScreen, P::HomeScreen, P::Wifi] {
            assert!(header_for(id).is_none(), "{id:?} starts at its first group");
        }
        let ui = ui();
        let desk = Pane { margin_pt: bifrost_ui::prefs_layout::m::DETAIL_MARGIN, nav_pt: 52.0, sidebar: false, status_pt: 0.0, header: true };
        let page_of = |id: PageId| {
            let spec = Spec { id, title: "Pane".into(), large: false, groups: vec![Group::new(vec![Item::new("Row")])] };
            ListPage::in_pane(&ui, spec, 1200, 1920, None, desk)
        };
        let (hero, compact) = (page_of(P::General), page_of(P::Notifications));
        assert!(!hero.header.as_ref().expect("hero").compact);
        let card = compact.header.as_ref().expect("card");
        assert!(card.compact && card.h < hero.header.as_ref().unwrap().h, "the compact card is shorter than the hero");
        assert!(card.h >= ui.pt(bifrost_ui::prefs_layout::m::HEADER_COMPACT_ICON), "and holds its icon");
        assert!(page_of(P::Display).header.is_none());
    }

    #[test]
    fn the_pane_title_sits_left_beside_the_pill_not_in_the_middle() {
        let ui = ui();
        let desk = Pane { margin_pt: bifrost_ui::prefs_layout::m::DETAIL_MARGIN, nav_pt: 52.0, sidebar: false, status_pt: 0.0, header: false };
        let spec = Spec { id: PageId::Focus, title: "Focus".into(), large: false, groups: vec![Group::new(vec![Item::new("Row")])] };
        let page = ListPage::in_pane(&ui, spec, 1200, 1920, None, desk);
        let mut b = buf();
        draw(&page, &ui, &mut b, None, 0.0);
        let mid = ui.pt(52.0 / 2.0) as usize;
        let inked = |from: usize, to: usize| (mid - 6..mid + 6).any(|row| b[row * 1200 + from..row * 1200 + to].iter().any(|&p| p != 0));
        let x = ui.pt(bifrost_ui::prefs_layout::toolbar_title_x()) as usize;
        assert!(inked(x, x + ui.pt(40.0) as usize), "the title starts right of the pill");
        assert!(!inked(560, 640), "nothing in the middle of the bar");
    }

    #[test]
    fn group_headers_draw_in_headline_weight_above_their_card() {
        let ui = ui();
        let spec = Spec { id: PageId::Root, title: "T".into(), large: false, groups: vec![Group::new(vec![Item::new("Row")]).header("DOCK POSITION")] };
        let page = ListPage::new(&ui, spec, 1200, 1920, Some("Back"));
        assert_eq!(page.spec().groups[0].header.as_deref(), Some("DOCK POSITION"), "the spec keeps what it wrote");
        let hd = page.headers[0].as_ref().expect("header image");
        assert_eq!(hd.w as f32, ui.text("Dock Position", MAC_HEADLINE_PT, Weight::Semibold, WHITE).w as f32, "rendered as a title-case headline");
    }
}
