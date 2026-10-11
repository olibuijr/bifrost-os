//! Shared look: fonts, text styles, bifrost dark-mode colours and widgets.

use crate::font::{Font, Style, Var};
use crate::gfx::{pack, rgba, Image, Surface};
use crate::symbols::Symbols;

pub const WHITE: u32 = pack(255, 255, 255, 255);
pub const BLACK: u32 = pack(0, 0, 0, 255);
pub const SECONDARY: u32 = pack(142, 142, 147, 255); // systemGray
pub const DESKTOP_BAR_TINT:u32=pack(28,28,30,220);
pub const DESKTOP_MENU_TINT:u32=pack(28,28,30,240);
/// Menu and bar fill when Reduce Transparency is on: the same grey, opaque, with no blur behind it.
pub const DESKTOP_MENU_SOLID:u32=pack(28,28,30,255);
pub const CELL: u32 = pack(28, 28, 30, 255); // secondarySystemGroupedBackground
/// The selected row of the bifrost Settings sidebar (measured on bifrost-ref/screens/settings.png).
/// The full-height sidebar column (Golden Gate shades the whole column, not just its rows).
pub const SIDEBAR_FILL: u32 = pack(20, 20, 22, 255);
// ---- Scheme-following colours (dark, light). Light values are measured from the Settings reference; call `.now()`. ----
use crate::appearance::Pair;
/// Menu bar and menu materials: the tint over the blurred backdrop (straight alpha, as `glass::tint` takes it).
pub const BAR_TINT: Pair = Pair::new(pack(28, 28, 30, 220), pack(246, 246, 246, 205));
pub const MENU_TINT: Pair = Pair::new(pack(28, 28, 30, 240), pack(246, 246, 246, 238));
/// Golden Gate menu bar: transparent over the wallpaper; titles and status items are white with a soft dark shadow.
pub const BAR_TEXT: u32 = WHITE;
pub const BAR_TEXT_SHADOW: u32 = pack(0, 0, 0, 255);
/// Alpha of the bar text shadow (0..255).
pub const BAR_TEXT_SHADOW_ALPHA: u32 = 90;
/// Over a light wallpaper the bar's titles and status items are near-black (85%), with a faint light halo.
pub const BAR_TEXT_ON_LIGHT: u32 = pack(0, 0, 0, 255);
pub const BAR_TEXT_ON_LIGHT_ALPHA: u32 = 217;
pub const BAR_TEXT_HALO: u32 = WHITE;
pub const BAR_TEXT_HALO_ALPHA: u32 = 60;
/// The open menu title's capsule over a light wallpaper (black at ~12%, premultiplied).
pub const BAR_OPEN_CAPSULE_ON_LIGHT: u32 = pack(0, 0, 0, 31);
/// The open menu title's light glass capsule (white at ~25%, premultiplied).
pub const BAR_OPEN_CAPSULE: u32 = pack(64, 64, 64, 64);
/// Menu item highlight capsule (accent, systemBlue) and the text on it.
pub const MENU_HIGHLIGHT: u32 = TINT;
pub const MENU_HIGHLIGHT_TEXT: u32 = WHITE;
/// Menu shortcut glyphs: secondary label.
pub const MENU_SHORTCUT: u32 = SECONDARY;
/// The same with Reduce Transparency on: opaque.
pub const MENU_SOLID: Pair = Pair::new(pack(28, 28, 30, 255), pack(246, 246, 246, 255));
/// The dock's glass tint (premultiplied, as `rgba` makes it).
pub const DOCK_TINT: Pair = Pair::new(crate::gfx::pack(15, 15, 17, 150), crate::gfx::pack(120, 120, 121, 150));
/// The sidebar column.
pub const SIDEBAR_BG: Pair = Pair::new(pack(20, 20, 22, 255), pack(235, 235, 235, 255));
/// The detail pane behind the cards.
pub const DETAIL_BG: Pair = Pair::new(pack(0, 0, 0, 255), pack(255, 255, 255, 255));
/// A grouped card (secondarySystemGroupedBackground).
pub const CARD_BG: Pair = Pair::new(pack(28, 28, 30, 255), pack(247, 247, 247, 255));
/// Hairline between rows.
pub const SEPARATOR_C: Pair = Pair::new(pack(56, 56, 58, 255), pack(232, 232, 232, 255));
/// Primary text.
pub const LABEL: Pair = Pair::new(pack(255, 255, 255, 255), pack(38, 38, 38, 255));
/// Secondary text and chevrons.
pub const LABEL_2: Pair = Pair::new(pack(142, 142, 147, 255), pack(142, 142, 147, 255));
/// The selected sidebar row.
pub const SELECTION: Pair = Pair::new(pack(14, 100, 196, 255), pack(0, 112, 245, 255));
/// A pressed row.
pub const ROW_PRESSED: Pair = Pair::new(pack(58, 58, 60, 255), pack(226, 226, 229, 255));
/// A search or text field plate.
pub const FIELD_BG: Pair = Pair::new(pack(58, 58, 60, 255), pack(226, 226, 228, 255));
/// Tertiary text: disabled controls and placeholder glyphs (`tertiaryLabelColor`).
pub const LABEL_3: Pair = Pair::new(pack(99, 99, 102, 255), pack(184, 184, 188, 255));
/// Golden Gate sidebar selection: a neutral plate under a bold label (Files, Notes, Reminders references).
pub const SIDEBAR_PLATE: Pair = Pair::new(pack(66, 66, 70, 255), pack(220, 220, 222, 255));
/// Toolbar capsules (Golden Gate glass button groups) and their one-pixel edge.
pub const CAPSULE_BG: Pair = Pair::new(pack(50, 50, 53, 255), pack(255, 255, 255, 255));
pub const CAPSULE_EDGE: Pair = Pair::new(pack(72, 72, 76, 255), pack(224, 224, 227, 255));
/// The soft ring under a capsule, premultiplied black.
pub const CAPSULE_SHADOW: Pair = Pair::new(pack(0, 0, 0, 70), pack(0, 0, 0, 16));
/// The selected segment of a segmented toolbar group (Files's current view).
pub const SEGMENT_ON: Pair = Pair::new(pack(86, 86, 90, 255), pack(232, 232, 235, 255));
/// A pressed or open push or pop-up button.
pub const CONTROL_PRESSED: Pair = Pair::new(pack(84, 84, 88, 255), pack(206, 206, 210, 255));
/// Control outlines in the current scheme (Show Borders, menus).
pub const HAIRLINE_C: Pair = Pair::new(pack(84, 84, 88, 255), pack(200, 200, 204, 255));
/// System materials, straight alpha (premultiply at use): sidebar, title bar, popover, HUD tints.
pub const MATERIAL_SIDEBAR: Pair = Pair::new(pack(24, 24, 27, 190), pack(236, 236, 236, 200));
pub const MATERIAL_TITLEBAR: Pair = Pair::new(pack(28, 28, 30, 200), pack(246, 246, 246, 205));
pub const MATERIAL_POPOVER: Pair = Pair::new(pack(28, 28, 30, 236), pack(246, 246, 246, 238));
pub const MATERIAL_HUD: Pair = Pair::new(pack(12, 12, 14, 170), pack(12, 12, 14, 170));
/// `.windowBackground`, `.contentBackground`, `.underWindowBackground`, `.underPageBackground`: nearly opaque.
pub const MATERIAL_WINDOW: Pair = Pair::new(pack(30, 30, 32, 228), pack(240, 240, 240, 228));
/// `.selection`: the accent at material strength (straight alpha, as the other `MATERIAL_*`).
pub const MATERIAL_SELECTION: Pair = Pair::new(pack(14, 100, 196, 200), pack(0, 112, 245, 200));
/// The opaque colours of the materials when Reduce Transparency is on.
pub const MATERIAL_SIDEBAR_SOLID: Pair = Pair::new(pack(20, 20, 22, 255), pack(235, 235, 235, 255));
pub const MATERIAL_TITLEBAR_SOLID: Pair = Pair::new(pack(28, 28, 30, 255), pack(246, 246, 246, 255));
pub const MATERIAL_HUD_SOLID: Pair = Pair::new(pack(12, 12, 14, 255), pack(12, 12, 14, 255));
/// Window frame: focused and background title areas.
pub const WINDOW_FRAME_FOCUSED: Pair = Pair::new(pack(52, 52, 55, 255), pack(236, 236, 236, 255));
pub const WINDOW_FRAME_BLURRED: Pair = Pair::new(pack(40, 40, 43, 255), pack(246, 246, 246, 255));
pub const SIDEBAR_SELECTED: u32 = pack(14, 100, 196, 255);
pub const AVATAR_BG: u32 = pack(112, 112, 116, 255);
pub const AVATAR_FG: u32 = pack(40, 40, 43, 255);
pub const AVATAR_PT: f32 = 60.0;
pub const SEPARATOR: u32 = pack(56, 56, 58, 255);
pub const TINT: u32 = pack(10, 132, 255, 255); // systemBlue
pub const MINUS_FILL: u32 = pack(209, 209, 214, 240); // home edit mode minus badge
pub const SYSTEM_RED: u32 = pack(255, 69, 58, 255);
pub const SYSTEM_ORANGE: u32 = pack(255, 159, 10, 255);
pub const SYSTEM_YELLOW: u32 = pack(255, 204, 0, 255);
pub const SYSTEM_GREEN: u32 = pack(48, 209, 88, 255);
pub const SYSTEM_PURPLE: u32 = pack(191, 90, 242, 255);
pub const TOAST_FILL: u32 = pack(44, 44, 46, 255); // tertiarySystemBackground
pub const TRACK: u32 = pack(58, 58, 60, 255);
/// the empty part of a switch or slider track, in the current scheme
pub const TRACK_C: Pair = Pair::new(pack(58, 58, 60, 255), pack(209, 209, 214, 255));
pub const SYSTEM_BLUE: u32 = pack(10, 132, 255, 255); // tint
pub const FILL: u32 = pack(44, 44, 46, 255); // tertiarySystemFill plate
pub const FILL_PRESSED: u32 = pack(72, 72, 74, 255);
pub const SYSTEM_TEAL: u32 = pack(100, 210, 255, 255);
pub const FILL_GRAY: u32 = pack(44, 44, 46, 255); // tertiarySystemBackground / segmented track
pub const FILL_GRAY2: u32 = pack(72, 72, 74, 255); // unopened board cells
/// Corner radii follow the Golden Gate reference: window 17 pt, cards 13, selection plates 9, icon tiles 5, and a search
/// field is a capsule (half its height). Older small controls keep their 6 pt tokens below until they are restyled.
/// Today story card backgrounds (App Store purple first).
pub const STORY_HUES: [u32; 5] = [pack(124, 32, 208, 255), pack(30, 90, 200, 255), pack(0, 120, 128, 255), pack(190, 84, 20, 255), pack(180, 40, 110, 255)];
pub const CELL_RADIUS_PT: f32 = 6.0;
pub const CARD_RADIUS_PT: f32 = 6.0;
pub const CAPSULE_RADIUS: f32 = 6.0;
pub const FILL_TERTIARY: u32 = pack(44, 44, 46, 255); // tertiarySystemBackground bands/groups
/// open folder panel (premultiplied white-ish over the blurred home) and the folder icon plate
pub const FOLDER_FILL: u32 = pack(52, 52, 55, 110);
pub const FOLDER_ICON_FILL: u32 = pack(60, 60, 62, 90);
/// backdrop of the App Library icon in the dock (bifrost-ref/screens/home.png)
pub const DOCK_LIBRARY_FILL: u32 = pack(22, 24, 34, 255);
pub const GRAY_DIM: u32 = pack(99, 99, 102, 255); // systemGray2, disabled glyphs
pub const CALC_NUMERIC: u32 = pack(51, 51, 51, 255);
pub const CALC_UTILITY: u32 = pack(165, 165, 165, 255);
pub const CALC_SCIENTIFIC: u32 = pack(38, 38, 40, 255);
pub const CALC_NUMERIC_DOWN: u32 = pack(115, 115, 115, 255);
pub const CALC_UTILITY_DOWN: u32 = pack(210, 210, 210, 255);
pub const CALC_OPERATOR_DOWN: u32 = pack(255, 191, 93, 255);
pub const CALC_SCIENTIFIC_DOWN: u32 = pack(90, 90, 92, 255);
/// tertiaryLabel-ish grey for disclosure chevrons.
pub const CHEVRON: u32 = pack(99, 99, 102, 255);
/// Opaque separator on elevated materials (menus, alerts, fields' outlines).
pub const HAIRLINE: u32 = pack(84, 84, 88, 255);
/// Reading themes (Books): each page keeps its own colours whatever the system appearance is, so these are fixed
/// tokens, not `Pair`s. White: ink on a white page.
pub const READER_WHITE_INK: u32 = pack(28, 28, 30, 255);
pub const READER_WHITE_DIM: u32 = pack(120, 120, 126, 255);
pub const READER_WHITE_SEP: u32 = pack(216, 216, 220, 255);
pub const READER_WHITE_RAISED: u32 = pack(242, 242, 247, 255);
pub const READER_WHITE_ACCENT: u32 = pack(0, 100, 220, 255);
/// Sepia: brown ink on a paper page.
pub const READER_SEPIA_BG: u32 = pack(244, 236, 216, 255);
pub const READER_SEPIA_INK: u32 = pack(74, 58, 44, 255);
pub const READER_SEPIA_DIM: u32 = pack(128, 108, 88, 255);
pub const READER_SEPIA_SEP: u32 = pack(220, 208, 182, 255);
pub const READER_SEPIA_RAISED: u32 = pack(234, 223, 198, 255);
pub const READER_SEPIA_ACCENT: u32 = pack(168, 88, 12, 255);
/// Dark: light grey ink on a near-black page.
pub const READER_DARK_BG: u32 = pack(22, 22, 24, 255);
pub const READER_DARK_INK: u32 = pack(208, 208, 212, 255);
pub const READER_DARK_DIM: u32 = pack(128, 128, 134, 255);
pub const READER_DARK_SEP: u32 = pack(50, 50, 54, 255);
/// Pressed-row overlay: white at 22/255, premultiplied.
pub const PRESSED: u32 = pack(22, 22, 22, 22);
pub const TERTIARY: u32 = pack(99, 99, 102, 255); // systemGray2, disabled text
pub const SYSTEM_PINK: u32 = pack(255, 55, 95, 255);
pub const SYSTEM_BROWN: u32 = pack(172, 142, 104, 255);
/// Golden Gate text styles in points, from the HIG Golden Gate table (Typography, "Golden Gate built-in text styles").
/// The PC desktop uses these. Use these names instead of literal sizes.
pub const MAC_LARGE_TITLE_PT: f32 = 26.0;
pub const MAC_TITLE1_PT: f32 = 22.0;
pub const MAC_TITLE2_PT: f32 = 17.0;
pub const MAC_TITLE3_PT: f32 = 15.0;
pub const MAC_HEADLINE_PT: f32 = 13.0;
pub const MAC_BODY_PT: f32 = 13.0;
pub const MAC_CALLOUT_PT: f32 = 12.0;
pub const MAC_SUBHEAD_PT: f32 = 11.0;
pub const MAC_FOOTNOTE_PT: f32 = 10.0;
pub const MAC_CAPTION1_PT: f32 = 10.0;
pub const MAC_CAPTION2_PT: f32 = 10.0;
/// Golden Gate group headers and footers use the footnote style.
pub const MAC_GROUP_HEADER_PT: f32 = MAC_FOOTNOTE_PT;
/// Spacing scale in points (4-pt grid): padding, gaps and offsets. Use `ui.pt(SPACE_L)`, never `ui.pt(16.0)`.
/// One place to change the density of every app.
pub const SPACE_XXS: f32 = 2.0;
pub const SPACE_XS: f32 = 4.0;
pub const SPACE_S: f32 = 8.0;
pub const SPACE_M: f32 = 12.0;
pub const SPACE_L: f32 = 16.0;
pub const SPACE_XL: f32 = 20.0;
pub const SPACE_XXL: f32 = 24.0;
pub const SPACE_XXXL: f32 = 32.0;
pub const SPACE_HUGE: f32 = 40.0;
/// Display numerals (clock face, temperature, calculator readout). Not text styles: large figures
/// that scale with the window like any other size.
pub const DISPLAY_SMALL_PT: f32 = 36.0;
pub const DISPLAY_MEDIUM_PT: f32 = 64.0;
pub const DISPLAY_LARGE_PT: f32 = 96.0;
/// Text links and bar buttons (systemBlue, the tint).
pub const LINK: u32 = TINT;
/// Destructive actions only (systemRed). Never a navigation link.
pub const DESTRUCTIVE: u32 = SYSTEM_RED;
/// Desktop window chrome: focused and background title bars, inactive traffic lights.
pub const WINDOW_BAR_FOCUSED: u32 = pack(52, 52, 55, 255);
pub const WINDOW_BAR_BLURRED: u32 = pack(40, 40, 43, 255);
pub const TRAFFIC_INACTIVE: u32 = pack(88, 88, 92, 255);
/// an unfocused window's traffic lights in the current scheme
pub const TRAFFIC_INACTIVE_C: Pair = Pair::new(pack(88, 88, 92, 255), pack(205, 205, 208, 255));
/// Glass rim on a focused traffic light (Golden Gate: background windows drop it).
pub const TRAFFIC_RIM: u32 = pack(0, 0, 0, 60);
/// The close, minimise and zoom glyphs shown while the pointer is over the traffic lights.
pub const TRAFFIC_GLYPH: u32 = pack(0, 0, 0, 190);
/// Desktop window corner radius, in pt.
pub const WINDOW_RADIUS_PT: f32 = 10.0;
/// Desktop menu and popup corner radius, in pt.
pub const MENU_RADIUS_PT: f32 = 14.0;
pub const FIELD_FILL: u32 = pack(58, 58, 60, 255); // search/address field on dark bars
/// Disabled nav-bar button text (bifrost 15 Clock's greyed Edit), measured on bifrost-ref.
pub const NAV_DISABLED: u32 = pack(64, 64, 64, 255);
/// Empty-state title in Clock (No Alarms), measured on bifrost-ref.
pub const EMPTY_TITLE_DIM: u32 = pack(89, 89, 89, 255);
/// Empty-state text and outline in Photos (No Photos or Videos), measured on bifrost-ref.
pub const EMPTY_STATE_GRAY: u32 = pack(102, 102, 102, 255);
pub const WEB_BROWSER_BAR: u32 = pack(18, 18, 18, 255); // WebBrowser toolbar and selected tab (bifrost 15 capture)
pub const WEB_BROWSER_FIELD: u32 = pack(61, 61, 61, 255); // WebBrowser address field
pub const TERTIARY_LABEL: u32 = pack(88, 88, 92, 255); // disabled glyphs

/// Dynamic Type: the smallest and largest text scale Settings offers.
pub const TEXT_SCALE_MIN: f32 = 0.85;
pub const TEXT_SCALE_MAX: f32 = 1.3;

pub struct Ui {
    pub font: Font<'static>,
    /// px per bifrost point
    pub s: f32,
    /// Dynamic Type factor applied to every `text` size (not to geometry)
    pub text_scale: f32,
    /// Bold Text: every weight renders one step heavier
    pub bold: bool,
    /// symbol fonts for icons, loaded from the data dir (may be empty)
    pub symbols: Symbols,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Weight {
    /// SF Pro Thin (the lock screen clock)
    Thin,
    Light,
    Regular,
    Medium,
    Semibold,
    Bold,
}

impl Weight {
    /// The `wght` axis value of SF Pro's named instance.
    pub fn wght(self) -> f32 {
        match self {
            Weight::Thin => 200.0,
            Weight::Light => 300.0,
            Weight::Regular => 400.0,
            Weight::Medium => 510.0,
            Weight::Semibold => 590.0,
            Weight::Bold => 700.0,
        }
    }

    /// One step heavier, as the Bold Text accessibility setting renders it.
    pub fn heavier(self) -> Weight {
        match self {
            Weight::Thin => Weight::Light,
            Weight::Light => Weight::Regular,
            Weight::Regular => Weight::Semibold,
            Weight::Medium => Weight::Semibold,
            Weight::Semibold => Weight::Bold,
            Weight::Bold => Weight::Bold,
        }
    }
}

/// The variation settings for text at `pt` points: the real weight and the optical size (the point size; the
/// renderer clamps it to the font's own `opsz` range, so a text cut is used for small sizes and a display cut above).
pub fn vars(pt: f32, weight: Weight) -> [Var; 2] {
    [(*b"wght", weight.wght()), (*b"opsz", pt)]
}

impl Ui {
    /// A UI at `s` px per point with default text settings.
    pub fn new(font: Font<'static>, s: f32) -> Self {
        Ui { font, s, text_scale: 1.0, bold: false, symbols: Symbols::default() }
    }

    pub fn pt(&self, v: f32) -> f32 {
        v * self.s
    }

    /// The point size `pt` after Dynamic Type.
    fn scaled(&self, pt: f32) -> f32 {
        pt * self.text_scale
    }

    fn weight(&self, w: Weight) -> Weight {
        if self.bold {
            w.heavier()
        } else {
            w
        }
    }

    /// Changes whenever the same string would render differently: UI scale, Dynamic Type, Bold Text or
    /// Increase Contrast. Text caches clear themselves when it moves.
    pub fn text_epoch(&self) -> u64 {
        (self.pt(1.0).to_bits() as u64) << 32 | (self.text_scale.to_bits() as u64 & 0xffff_fff0) | (self.bold as u64) << 1 | crate::appearance::increase_contrast() as u64
    }

    /// Renders text at `pt` points in the given weight.
    pub fn text(&self, text: &str, pt: f32, weight: Weight, color: u32) -> Image {
        self.styled(text, pt, weight, color, false)
    }

    pub fn styled(&self, text: &str, pt: f32, weight: Weight, color: u32, shadow: bool) -> Image {
        let (pt, weight) = (self.scaled(pt), self.weight(weight));
        let color = crate::appearance::contrast(color);
        self.font.render_var(text, &Style { px: self.pt(pt), embolden: 0.0, color, shadow }, &vars(pt, weight))
    }

    /// System chrome text (status bar, lock screen clock): never scaled by
    /// Dynamic Type, as on bifrost.
    pub fn chrome(&self, text: &str, pt: f32, weight: Weight, color: u32) -> Image {
        let color = crate::appearance::contrast(color);
        self.font.render_var(text, &Style { px: self.pt(pt), embolden: 0.0, color, shadow: false }, &vars(pt, weight))
    }

    /// Width of `text` in px, as `text` would draw it.
    pub fn measure(&self, text: &str, pt: f32, weight: Weight) -> f32 {
        let (pt, weight) = (self.scaled(pt), self.weight(weight));
        self.font.measure_var(text, self.pt(pt), &vars(pt, weight))
    }

    pub fn fit(&self, text: &str, pt: f32, max_px: f32) -> String {
        let (pt, weight) = (self.scaled(pt), self.weight(Weight::Regular));
        self.font.fit_var(text, self.pt(pt), max_px, &vars(pt, weight))
    }

    /// Greedy word wrap of `text` into lines no wider than `max_px`.
    pub fn wrap(&self, text: &str, pt: f32, weight: Weight, max_px: f32) -> Vec<String> {
        let mut lines = vec![];
        let mut line = String::new();
        for word in text.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && self.measure(&candidate, pt, weight) > max_px {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            } else {
                line = candidate;
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }
}

/// Blits `img` horizontally centred on `cx`.
pub fn centred(s: &mut Surface, img: &Image, cx: i32, y: i32) {
    s.blit(img, cx - img.w as i32 / 2, y, 255, 255);
}

/// A footer line under a list or a screen: footnote size (10 pt) secondary, centred on `cx`, top at `y` (px).
pub fn footer(s: &mut Surface, ui: &Ui, text: &str, cx: i32, y: i32) {
    let img = ui.text(text, MAC_FOOTNOTE_PT, Weight::Regular, SECONDARY);
    centred(s, &img, cx, y);
}

/// A large navigation title (Golden Gate large title, 26 pt bold, white) left-aligned at `x` (px), top at `y` (px).
pub fn large_title(s: &mut Surface, ui: &Ui, text: &str, x: i32, y: i32) {
    let img = ui.text(text, MAC_LARGE_TITLE_PT, Weight::Bold, WHITE);
    s.blit(&img, x, y, 255, 255);
}

const ALERT_W_PT: f32 = 259.0;

/// An alert card (`NativeAlert` in the Golden Gate style): left-aligned title and message and a row of capsule buttons,
/// centred on a dimmed screen. Shared by the Settings confirmations and the Low Battery alert.
pub struct AlertCard {
    title: Image,
    message: Vec<Image>,
    buttons: Vec<Image>,
    /// `showsSuppressionButton`-style checkbox above the buttons: its label and whether it is checked
    check: Option<(Image, bool)>,
}

/// Inner margin, gap between buttons and button height of an alert, in pt (measured from the reference).
const ALERT_PAD_PT: f32 = 16.0;
const ALERT_GAP_PT: f32 = 8.0;
const ALERT_BTN_H_PT: f32 = 28.0;
const ALERT_RADIUS_PT: f32 = 28.0;
/// The checkbox row: its height, the gap above the buttons and the box size, in pt.
const ALERT_CHECK_H_PT: f32 = 18.0;
const ALERT_CHECK_GAP_PT: f32 = 10.0;
const ALERT_CHECK_BOX_PT: f32 = 14.0;

impl AlertCard {
    /// `buttons` are the rendered labels, left to right. Title and message are white images drawn in the label colour.
    pub fn new(ui: &Ui, title: &str, message: &str, buttons: Vec<Image>) -> Self {
        AlertCard {
            title: ui.text(title, MAC_TITLE3_PT, Weight::Semibold, WHITE),
            message: ui.wrap(message, MAC_BODY_PT, Weight::Regular, ui.pt(ALERT_W_PT - 2.0 * ALERT_PAD_PT)).iter().map(|l| ui.text(l, MAC_BODY_PT, Weight::Regular, WHITE)).collect(),
            buttons,
            check: None,
        }
    }

    /// Adds a checkbox row above the buttons (`NativeAlert.showsSuppressionButton`, "Reopen windows when logging back in").
    pub fn with_checkbox(mut self, ui: &Ui, label: &str, checked: bool) -> Self {
        self.check = Some((ui.text(label, MAC_BODY_PT, Weight::Regular, WHITE), checked));
        self
    }

    /// The checkbox state, `None` for a card without one.
    pub fn checked(&self) -> Option<bool> {
        self.check.as_ref().map(|c| c.1)
    }

    pub fn set_checked(&mut self, on: bool) {
        if let Some(c) = self.check.as_mut() {
            c.1 = on;
        }
    }

    /// The checkbox row (x, y, w, h) inside a card at (x, y, w, h).
    fn check_rect(&self, ui: &Ui, (x, y, cw, ch): (f32, f32, f32, f32)) -> Option<(f32, f32, f32, f32)> {
        self.check.as_ref()?;
        let pad = ui.pt(ALERT_PAD_PT);
        let row_h = ui.pt(ALERT_CHECK_H_PT);
        Some((x + pad, y + ch - pad - ui.pt(ALERT_BTN_H_PT) - ui.pt(ALERT_CHECK_GAP_PT) - row_h, cw - 2.0 * pad, row_h))
    }

    /// True when (x, y) is on the checkbox row.
    pub fn hit_check(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> bool {
        self.check_rect(ui, self.card(ui, w, h)).is_some_and(|(rx, ry, rw, rh)| x >= rx && x < rx + rw && y >= ry && y < ry + rh)
    }

    /// Centre of the checkbox row, for tests.
    pub fn check_centre(&self, ui: &Ui, w: i32, h: i32) -> Option<(f32, f32)> {
        self.check_rect(ui, self.card(ui, w, h)).map(|(rx, ry, rw, rh)| (rx + rw / 2.0, ry + rh / 2.0))
    }

    /// The card (x, y, w, h), centred on screen.
    fn card(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let cw = ui.pt(ALERT_W_PT);
        let ch = ui.pt(ALERT_PAD_PT + 2.0) + self.title.h as f32 + ui.pt(8.0) + ui.pt(17.0) * self.message.len() as f32 + ui.pt(ALERT_PAD_PT) + ui.pt(ALERT_BTN_H_PT) + ui.pt(ALERT_PAD_PT)
            + if self.check.is_some() { ui.pt(ALERT_CHECK_H_PT + ALERT_CHECK_GAP_PT) } else { 0.0 };
        (((w as f32 - cw) / 2.0).round(), ((h as f32 - ch) / 2.0).round(), cw, ch)
    }

    /// Button `i` as (x, y, w, h) inside a card at (x, y, w, h).
    fn button_rect(&self, ui: &Ui, (x, y, cw, ch): (f32, f32, f32, f32), i: usize) -> (f32, f32, f32, f32) {
        let n = self.buttons.len().max(1) as f32;
        let (pad, gap) = (ui.pt(ALERT_PAD_PT), ui.pt(ALERT_GAP_PT));
        let bw = (cw - 2.0 * pad - gap * (n - 1.0)) / n;
        (x + pad + (bw + gap) * i as f32, y + ch - pad - ui.pt(ALERT_BTN_H_PT), bw, ui.pt(ALERT_BTN_H_PT))
    }

    /// Index of the button under (x, y), if any.
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<usize> {
        let card = self.card(ui, w, h);
        (0..self.buttons.len()).find(|&i| {
            let (bx, by, bw, bh) = self.button_rect(ui, card, i);
            x >= bx && x < bx + bw && y >= by && y < by + bh
        })
    }

    /// Centre of button `i`, for tests.
    pub fn button_centre(&self, ui: &Ui, w: i32, h: i32, i: usize) -> (f32, f32) {
        let (bx, by, bw, bh) = self.button_rect(ui, self.card(ui, w, h), i);
        (bx + bw / 2.0, by + bh / 2.0)
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, rgba(0, 0, 0, 110));
        let card = self.card(ui, w, h);
        let (x, y, cw, ch) = card;
        s.fill_rrect(x, y, cw, ch, ui.pt(ALERT_RADIUS_PT), CARD_BG.now());
        let (lx, mut ty) = (x + ui.pt(ALERT_PAD_PT), y + ui.pt(ALERT_PAD_PT + 2.0));
        s.blit_colored(&self.title, lx as i32, ty as i32, LABEL.now(), 255);
        ty += self.title.h as f32 + ui.pt(8.0);
        for line in &self.message {
            s.blit_colored(line, lx as i32, ty as i32, LABEL.now(), 255);
            ty += ui.pt(17.0);
        }
        if let (Some((label, on)), Some((rx, ry, _, rh))) = (&self.check, self.check_rect(ui, card)) {
            let b = ui.pt(ALERT_CHECK_BOX_PT);
            let by = ry + (rh - b) / 2.0;
            if *on {
                s.fill_rrect(rx, by, b, b, ui.pt(3.0), TINT);
                // the tick: a short stroke down to the right, then a long one up to the right, in white dots
                let t = ui.pt(1.8).max(1.0);
                let mut dot = |px: f32, py: f32| s.fill_rrect(rx + px * b - t / 2.0, by + py * b - t / 2.0, t, t, t / 2.0, rgba(255, 255, 255, 255));
                for k in 0..=8 {
                    let f = k as f32 / 8.0;
                    dot(0.22 + 0.20 * f, 0.52 + 0.20 * f);
                }
                for k in 1..=14 {
                    let f = k as f32 / 14.0;
                    dot(0.42 + 0.36 * f, 0.72 - 0.42 * f);
                }
            } else {
                s.stroke_rrect(rx, by, b, b, ui.pt(3.0), ui.pt(1.0).max(1.0), SECONDARY);
            }
            s.blit_colored(label, (rx + b + ui.pt(8.0)) as i32, (ry + (rh - label.h as f32) / 2.0) as i32, LABEL.now(), 255);
        }
        for (i, label) in self.buttons.iter().enumerate() {
            let (bx, by, bw, bh) = self.button_rect(ui, card, i);
            s.fill_rrect(bx, by, bw, bh, bh / 2.0, FIELD_BG.now());
            button_plate(s, ui, (bx, by, bw, bh), label.w as f32);
            centred(s, label, (bx + bw / 2.0) as i32, (by + (bh - label.h as f32) / 2.0) as i32);
        }
    }
}

/// Button Shapes: a rounded plate (radius under 6 pt, no pill) behind the
/// tappable text centred in `(x, y, w, h)`, `label_w` px wide. Draws nothing
/// while the setting is off, so default frames are untouched.
pub fn button_plate(s: &mut Surface, ui: &Ui, (x, y, w, h): (f32, f32, f32, f32), label_w: f32) {
    let Some(r) = crate::appearance::button_shape_radius(ui.s) else { return };
    let (pw, ph) = ((label_w + ui.pt(20.0)).min(w), (h - ui.pt(10.0)).max(ui.pt(24.0)).min(h));
    let (px, py) = (x + (w - pw) / 2.0, y + (h - ph) / 2.0);
    s.fill_rrect(px, py, pw, ph, r, rgba(10, 132, 255, 46));
    s.stroke_rrect(px, py, pw, ph, r, ui.pt(1.0).max(1.0), rgba(10, 132, 255, 140));
}

/// The bifrost home indicator: a short white bar at the bottom edge.
pub fn home_indicator(s: &mut Surface, ui: &Ui, w: i32, h: i32, alpha: u8) {
    let (bw, bh) = (ui.pt(134.0), ui.pt(5.0));
    s.fill_rrect((w as f32 - bw) / 2.0, h as f32 - ui.pt(8.0) - bh, bw, bh, bh / 2.0, rgba(255, 255, 255, alpha));
}

/// Height of the strip at the bottom that starts a go-home swipe.
pub fn home_zone(ui: &Ui) -> f32 {
    ui.pt(28.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> Ui {
        crate::tests::ui()
    }

    #[test]
    fn text_scale_and_bold_change_rendering_but_not_chrome() {
        let mut ui = ui();
        let plain = ui.text("Settings", 17.0, Weight::Regular, WHITE);
        let chrome = ui.chrome("9:41", 12.0, Weight::Semibold, WHITE);
        ui.text_scale = TEXT_SCALE_MAX;
        let big = ui.text("Settings", 17.0, Weight::Regular, WHITE);
        assert!(big.w > plain.w && big.h > plain.h);
        assert!(ui.measure("Settings", 17.0, Weight::Regular) > plain.w as f32 * 1.1);
        assert_eq!(ui.chrome("9:41", 12.0, Weight::Semibold, WHITE).w, chrome.w, "the status bar never scales");
        ui.text_scale = 1.0;
        ui.bold = true;
        let bold = ui.text("Settings", 17.0, Weight::Regular, WHITE);
        let ink = |i: &Image| i.px.iter().filter(|&&p| p >> 24 > 128).count();
        assert!(ink(&bold) > ink(&plain), "Bold Text renders heavier");
        assert_eq!(Weight::Bold.heavier(), Weight::Bold);
        assert_eq!(Weight::Medium.heavier(), Weight::Semibold);
    }

    #[test]
    fn wrap_breaks_on_words_and_keeps_long_words() {
        let ui = ui();
        let text = "Known networks will be joined automatically. If no known networks are available, you will be notified.";
        let lines = ui.wrap(text, 13.0, Weight::Regular, ui.pt(300.0));
        assert!(lines.len() >= 3, "{lines:?}");
        assert!(lines.iter().all(|l| ui.measure(l, 13.0, Weight::Regular) <= ui.pt(300.0)));
        assert_eq!(lines.join(" "), text);
        assert_eq!(ui.wrap("supercalifragilistic", 13.0, Weight::Regular, 10.0), vec!["supercalifragilistic"]);
        assert!(ui.wrap("   ", 13.0, Weight::Regular, 100.0).is_empty());
    }

    #[test]
    fn kit_roles_and_type_scale_hold() {
        // links are the tint, destructive is red only, footers are the group-header size
        assert_eq!(LINK, TINT);
        assert_eq!(DESTRUCTIVE, SYSTEM_RED);
    }

    #[test]
    fn mac_type_scale_matches_the_hig_golden_gate_table() {
        // HIG Golden Gate built-in text styles (point sizes). Changing these needs a new HIG check.
        let table = [
            (MAC_LARGE_TITLE_PT, 26.0),
            (MAC_TITLE1_PT, 22.0),
            (MAC_TITLE2_PT, 17.0),
            (MAC_TITLE3_PT, 15.0),
            (MAC_HEADLINE_PT, 13.0),
            (MAC_BODY_PT, 13.0),
            (MAC_CALLOUT_PT, 12.0),
            (MAC_SUBHEAD_PT, 11.0),
            (MAC_FOOTNOTE_PT, 10.0),
            (MAC_CAPTION1_PT, 10.0),
            (MAC_CAPTION2_PT, 10.0),
        ];
        for (actual, hig) in table {
            assert_eq!(actual, hig);
        }
        assert_eq!(MAC_GROUP_HEADER_PT, MAC_FOOTNOTE_PT);
        // the Golden Gate scale is non-increasing (caption 1 and 2 share a size)
        let scale = [MAC_LARGE_TITLE_PT, MAC_TITLE1_PT, MAC_TITLE2_PT, MAC_TITLE3_PT, MAC_HEADLINE_PT, MAC_CALLOUT_PT, MAC_SUBHEAD_PT, MAC_FOOTNOTE_PT, MAC_CAPTION1_PT];
        assert!(scale.windows(2).all(|w| w[0] >= w[1]));
    }
}

/// the native toolkit `NativeColor` semantic colours as dynamic `Pair`s (docs/bifrost-ui.md, "NativeColor semantic colours").
/// Resolve with `.now()` at draw time; they follow Light/Dark like `NativeColor` follows `NativeAppearance`.
/// Plain constants such as `BLACK`, `CELL` or `WHITE` never follow the scheme: use these for window and content colours.
pub mod system {
    use super::*;
    pub const WINDOW_BACKGROUND: Pair = DETAIL_BG;
    pub const CONTROL_BACKGROUND: Pair = DETAIL_BG;
    pub const UNDER_PAGE_BACKGROUND: Pair = SIDEBAR_BG;
    pub const TEXT_BACKGROUND: Pair = DETAIL_BG;
    pub const LABEL: Pair = super::LABEL;
    pub const SECONDARY_LABEL: Pair = LABEL_2;
    pub const TERTIARY_LABEL: Pair = super::LABEL_3;
    pub const QUATERNARY_LABEL: Pair = Pair::new(pack(72, 72, 76, 255), pack(212, 212, 216, 255));
    pub const TEXT: Pair = super::LABEL;
    pub const PLACEHOLDER_TEXT: Pair = super::LABEL_3;
    pub const SEPARATOR: Pair = SEPARATOR_C;
    pub const GRID: Pair = Pair::new(pack(56, 56, 58, 255), pack(214, 214, 218, 255));
    pub const SELECTED_CONTENT_BACKGROUND: Pair = SELECTION;
    pub const UNEMPHASIZED_SELECTED_CONTENT_BACKGROUND: Pair = SIDEBAR_PLATE;
    pub const CONTROL_ACCENT: Pair = Pair::new(pack(10, 132, 255, 255), pack(0, 122, 255, 255));
    pub const LINK: Pair = CONTROL_ACCENT;
    pub const SYSTEM_RED: Pair = Pair::new(pack(255, 69, 58, 255), pack(255, 59, 48, 255));
    pub const SYSTEM_ORANGE: Pair = Pair::new(pack(255, 159, 10, 255), pack(255, 149, 0, 255));
    pub const SYSTEM_YELLOW: Pair = Pair::new(pack(255, 214, 10, 255), pack(255, 204, 0, 255));
    pub const SYSTEM_GREEN: Pair = Pair::new(pack(48, 209, 88, 255), pack(52, 199, 89, 255));
    pub const SYSTEM_MINT: Pair = Pair::new(pack(99, 230, 226, 255), pack(0, 199, 190, 255));
    pub const SYSTEM_TEAL: Pair = Pair::new(pack(64, 200, 224, 255), pack(48, 176, 199, 255));
    pub const SYSTEM_CYAN: Pair = Pair::new(pack(100, 210, 255, 255), pack(50, 173, 230, 255));
    pub const SYSTEM_BLUE: Pair = CONTROL_ACCENT;
    pub const SYSTEM_INDIGO: Pair = Pair::new(pack(94, 92, 230, 255), pack(88, 86, 214, 255));
    pub const SYSTEM_PURPLE: Pair = Pair::new(pack(191, 90, 242, 255), pack(175, 82, 222, 255));
    pub const SYSTEM_PINK: Pair = Pair::new(pack(255, 55, 95, 255), pack(255, 45, 85, 255));
    pub const SYSTEM_BROWN: Pair = Pair::new(pack(172, 142, 104, 255), pack(162, 132, 94, 255));
    pub const SYSTEM_GRAY: Pair = Pair::new(pack(142, 142, 147, 255), pack(142, 142, 147, 255));
}

// ---- Settings > Appearance preferences that every app can read ----
static HIGHLIGHT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static SIDEBAR_ICON: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
static SCROLL_BARS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Highlight colour (text selection): 0 follows the accent colour, n is `Accent::ALL[n]` (Blue to Graphite).
pub fn set_highlight(index: u32) {
    HIGHLIGHT.store(index.min(crate::appearance::Accent::ALL.len() as u32 - 1), std::sync::atomic::Ordering::Relaxed);
}

/// The text selection colour now: the chosen highlight, or the accent colour.
pub fn highlight_now() -> u32 {
    match HIGHLIGHT.load(std::sync::atomic::Ordering::Relaxed) as usize {
        0 => crate::appearance::accent_now(),
        k => {
            let (dark, light) = crate::appearance::Accent::ALL[k.min(crate::appearance::Accent::ALL.len() - 1)].colours();
            Pair::new(dark, light).now()
        }
    }
}

/// Sidebar icon size: 0 small, 1 medium, 2 large.
pub fn set_sidebar_icon_size(index: u32) {
    SIDEBAR_ICON.store(index.min(2), std::sync::atomic::Ordering::Relaxed);
}

/// The factor a source list scales its row glyphs and row height by (medium is 1).
pub fn sidebar_icon_scale() -> f32 {
    [0.85, 1.0, 1.2][SIDEBAR_ICON.load(std::sync::atomic::Ordering::Relaxed).min(2) as usize]
}

/// Show scroll bars: 0 automatically, 1 when scrolling, 2 always.
pub fn set_scroll_bars(mode: u32) {
    SCROLL_BARS.store(mode.min(2), std::sync::atomic::Ordering::Relaxed);
}

pub fn scroll_bars_always() -> bool {
    SCROLL_BARS.load(std::sync::atomic::Ordering::Relaxed) == 2
}

/// Scroll bars fade out when idle (every mode but Always).
pub fn scroll_bars_fade() -> bool {
    !scroll_bars_always()
}

#[cfg(test)]
mod appearance_guard {
    use std::path::{Path, PathBuf};

    fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { rs_files(&p, out) } else if p.extension().is_some_and(|x| x == "rs") { out.push(p) }
        }
    }

    /// Files that may fill with the dark-only constants, each with the reason. Everything else must use a `Pair` + `.now()`.
    /// "legacy" entries are known gaps (full-screen dark bifrost-era surfaces) tracked in docs/bifrost-ui.md.
    const ALLOW: &[(&str, &str)] = &[
        ("shell/apphost.rs", "letterbox behind a running app's pixels"),
        ("shell/rotate.rs", "letterbox bars"),
        ("shell/mod.rs", "backdrop shown before an app's first frame"),
        ("shell/desk.rs", "desktop backdrop and dock placeholder tiles"),
        ("shell/desktop_bar.rs", "blur base under translucent materials"),
        ("shell/camera/", "camera viewfinder is always black"),
        ("shell/photos/", "full-screen photo viewer and camera tile are black, as in Photos"),
        ("apps/clock-desktop/src/clock/", "translucent scrims (night shade, sheet dimming) over the clock faces"),
        ("shell/magnifier.rs", "magnifier viewfinder"),
        ("shell/jiggle.rs", "badge glyph ink"),
        ("shell/textarea.rs", "checkbox tick ink"),
        ("shell/textfield.rs", "pill ink"),
        ("shell/markup.rs", "annotation canvas and colour swatches"),
        ("shell/widgets/deildu.rs", "poster placeholder"),
        ("shell/browser/", "legacy: Browser-style dark chrome, gap"),
        ("shell/clock/", "legacy: full-screen clock, gap"),
        ("shell/power_menu.rs", "legacy: full-screen power alert, gap"),
        ("apps/remote/", "legacy: dark remote, gap"),
    ];

    fn has_word(line: &str, word: &str) -> bool {
        let cleaned = line.replace("Color::BLACK", "").replace("Rgba::BLACK", "");
        let b = cleaned.as_bytes();
        cleaned.match_indices(word).any(|(i, _)| {
            let before = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
            let after = i + word.len() >= b.len() || !(b[i + word.len()].is_ascii_alphanumeric() || b[i + word.len()] == b'_');
            before && after
        })
    }

    #[test]
    fn window_and_content_fills_follow_light_dark() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files = Vec::new();
        rs_files(&root.join("src/shell"), &mut files);
        rs_files(&root.join("apps"), &mut files);
        let mut bad = Vec::new();
        for f in files {
            let rel = f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
            let key = rel.strip_prefix("src/").unwrap_or(&rel).to_string();
            if rel.ends_with("tests.rs") || rel.contains("/tests/") || ALLOW.iter().any(|(p, _)| key.starts_with(p)) { continue }
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            for (n, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("//") || !(line.contains("fill_rect(") || line.contains("fill_rrect(")) { continue }
                if has_word(line, "BLACK") || has_word(line, "CELL") {
                    bad.push(format!("{rel}:{}: {}", n + 1, line.trim()));
                }
            }
        }
        assert!(bad.is_empty(), "dark-only BLACK/CELL fill; use ui::system / DETAIL_BG / CARD_BG with .now() (or allowlist with a reason):\n{}", bad.join("\n"));
    }
}
