//! Input Sources > Show Keyboard Viewer: a floating board of the active source's keys. Each key shows what it types plain
//! and with Shift, read from the same keymap tables the typing uses (`installer::keymaps::key_levels`), so the viewer
//! shows the layout the hardware keyboard really types. The key being pressed is lit until it is released. The viewer
//! takes touches only on its Done button; everything else reaches the desktop, so it can stay up while typing.
//!
//! Not yet: AltGr and Shift levels are not drawn (the viewer shows plain and Shift), and the board is the ANSI shape
//! for every source.

use super::ui::{Ui, Weight, CARD_BG, LABEL_2, LINK, MAC_CAPTION1_PT, MAC_TITLE3_PT, MAC_BODY_PT, SEPARATOR, WHITE, FILL};
use super::{Drawn, Effect, Event, Shell};
use crate::gfx::Surface;
use crate::installer::keymaps::key_levels;

/// The board, row by row: evdev key code and width in key units. The first four rows are 15 units wide, as the ANSI
/// board is; the bottom row is the modifiers and the space bar.
pub const BOARD: [&[(u16, f32)]; 5] = [
    &[(41, 1.0), (2, 1.0), (3, 1.0), (4, 1.0), (5, 1.0), (6, 1.0), (7, 1.0), (8, 1.0), (9, 1.0), (10, 1.0), (11, 1.0), (12, 1.0), (13, 1.0), (14, 2.0)],
    &[(15, 1.5), (16, 1.0), (17, 1.0), (18, 1.0), (19, 1.0), (20, 1.0), (21, 1.0), (22, 1.0), (23, 1.0), (24, 1.0), (25, 1.0), (26, 1.0), (27, 1.0), (43, 1.5)],
    &[(58, 1.75), (30, 1.0), (31, 1.0), (32, 1.0), (33, 1.0), (34, 1.0), (35, 1.0), (36, 1.0), (37, 1.0), (38, 1.0), (39, 1.0), (40, 1.0), (28, 2.25)],
    &[(42, 2.25), (44, 1.0), (45, 1.0), (46, 1.0), (47, 1.0), (48, 1.0), (49, 1.0), (50, 1.0), (51, 1.0), (52, 1.0), (53, 1.0), (54, 2.75)],
    &[(29, 1.25), (125, 1.25), (56, 1.25), (57, 6.25), (100, 1.25)],
];

/// Width of the board's first four rows, in key units.
pub const UNITS: f32 = 15.0;

/// What a key face says: a key's name, its plain and Shift characters, or nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Legend {
    Name(&'static str),
    Chars { plain: char, shift: char },
    Blank,
}

/// The legend of a key: modifier and editing keys carry their name; a key that types a character shows it, with the
/// Shift character when the layout has one. `levels` is the layout's four characters for the key, when it has any.
pub fn legend(code: u16, levels: Option<[char; 4]>) -> Legend {
    let name = match code {
        14 => Some("delete"),
        15 => Some("tab"),
        28 => Some("return"),
        58 => Some("caps lock"),
        42 | 54 => Some("shift"),
        29 => Some("control"),
        56 | 100 => Some("option"),
        125 => Some("command"),
        _ => None,
    };
    if let Some(name) = name {
        return Legend::Name(name);
    }
    match levels {
        Some([plain, shift, _, _]) if plain != '\0' && !plain.is_whitespace() => Legend::Chars { plain, shift },
        _ => Legend::Blank,
    }
}

const PAD_PT: f32 = 16.0;
const TITLE_PT: f32 = 40.0;
const KEY_H_PT: f32 = 36.0;
const KEY_GAP_PT: f32 = 4.0;
const MAX_W_PT: f32 = 760.0;
const RADIUS_PT: f32 = 13.0;
const KEY_RADIUS_PT: f32 = 6.0;
const DONE_W_PT: f32 = 56.0;

/// x, y, w, h in px.
type Rect = (f32, f32, f32, f32);

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

/// The viewer: the key held down, if any.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyboardViewer {
    held: Option<u16>,
}

impl KeyboardViewer {
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub fn new() -> Self {
        Self::default()
    }

    /// The key lit on the board, if one is held.
    pub fn held(&self) -> Option<u16> {
        self.held
    }

    /// A key went down or up. Returns true when the lit key changed, so the caller repaints.
    pub fn key(&mut self, code: u16, down: bool) -> bool {
        let before = self.held;
        if down {
            if BOARD.iter().any(|row| row.iter().any(|k| k.0 == code)) {
                self.held = Some(code);
            }
        } else if self.held == Some(code) {
            self.held = None;
        }
        self.held != before
    }

    fn panel(ui: &Ui, w: i32, h: i32) -> Rect {
        let pw = (w as f32 * 0.9).min(ui.pt(MAX_W_PT));
        let key = ui.pt(KEY_H_PT) + ui.pt(KEY_GAP_PT);
        let ph = ui.pt(TITLE_PT) + ui.pt(PAD_PT) + BOARD.len() as f32 * key + ui.pt(PAD_PT);
        ((w as f32 - pw) / 2.0, (h as f32 - ph) / 2.0, pw, ph)
    }

    fn unit(ui: &Ui, w: i32, h: i32) -> f32 {
        let (_, _, pw, _) = Self::panel(ui, w, h);
        (pw - 2.0 * ui.pt(PAD_PT)) / UNITS
    }

    /// The rectangle of the key at row `r`, position `k` in the row (the gap is inside the key's width).
    fn key_rect(ui: &Ui, w: i32, h: i32, r: usize, k: usize) -> Rect {
        let (x, y, _, _) = Self::panel(ui, w, h);
        let unit = Self::unit(ui, w, h);
        let gap = ui.pt(KEY_GAP_PT);
        let before: f32 = BOARD[r][..k].iter().map(|key| key.1).sum();
        let units = BOARD[r][k].1;
        let key = ui.pt(KEY_H_PT) + gap;
        (x + ui.pt(PAD_PT) + before * unit, y + ui.pt(TITLE_PT) + ui.pt(PAD_PT) / 2.0 + r as f32 * key, units * unit - gap, ui.pt(KEY_H_PT))
    }

    fn done_rect(ui: &Ui, w: i32, h: i32) -> Rect {
        let (x, y, pw, _) = Self::panel(ui, w, h);
        (x + pw - ui.pt(PAD_PT + DONE_W_PT), y, ui.pt(DONE_W_PT), ui.pt(TITLE_PT))
    }

    /// True when a touch at (x, y) lands on the Done button.
    pub fn hit_done(ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> bool {
        inside(Self::done_rect(ui, w, h), x, y)
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32, layout: usize) {
        let (x, y, pw, ph) = Self::panel(ui, w, h);
        s.fill_rrect(x, y, pw, ph, ui.pt(RADIUS_PT), CARD_BG.now());
        s.stroke_rrect(x, y, pw, ph, ui.pt(RADIUS_PT), 1.0, SEPARATOR);

        let title = ui.text("Keyboard Viewer", MAC_TITLE3_PT, Weight::Semibold, WHITE);
        s.blit_colored(&title, (x + ui.pt(PAD_PT)) as i32, (y + (ui.pt(TITLE_PT) - title.h as f32) / 2.0) as i32, super::ui::LABEL.now(), 255);
        let (dx, dy, dw, dh) = Self::done_rect(ui, w, h);
        let done = ui.text("Done", MAC_BODY_PT, Weight::Semibold, WHITE);
        s.blit_colored(&done, (dx + (dw - done.w as f32) / 2.0) as i32, (dy + (dh - done.h as f32) / 2.0) as i32, LINK, 255);

        for (r, row) in BOARD.iter().enumerate() {
            for (k, &(code, _)) in row.iter().enumerate() {
                let (kx, ky, kw, kh) = Self::key_rect(ui, w, h, r, k);
                let lit = self.held() == Some(code);
                s.fill_rrect(kx, ky, kw, kh, ui.pt(KEY_RADIUS_PT), if lit { LINK } else { FILL });
                match legend(code, key_levels(layout, code)) {
                    Legend::Name(name) => {
                        let img = ui.text(name, MAC_CAPTION1_PT, Weight::Regular, WHITE);
                        s.blit_colored(&img, (kx + (kw - img.w as f32) / 2.0) as i32, (ky + (kh - img.h as f32) / 2.0) as i32, WHITE, 255);
                    }
                    Legend::Chars { plain, shift } => {
                        let pad = ui.pt(4.0);
                        if shift != '\0' && shift != plain {
                            let img = ui.text(&shift.to_string(), MAC_CAPTION1_PT, Weight::Regular, WHITE);
                            s.blit_colored(&img, (kx + pad) as i32, (ky + pad) as i32, LABEL_2.now(), 255);
                        }
                        let img = ui.text(&plain.to_string(), MAC_BODY_PT, Weight::Regular, WHITE);
                        s.blit_colored(&img, (kx + kw - pad - img.w as f32) as i32, (ky + kh - pad - img.h as f32) as i32, WHITE, 255);
                    }
                    Legend::Blank => {}
                }
            }
        }
    }
}

impl Shell {
    /// Input Sources > Show Keyboard Viewer.
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub(super) fn keyboard_viewer_open(&mut self) {
        self.keyboard_viewer = Some(KeyboardViewer::new());
        self.drawn = Drawn::Nothing;
        self.dirty = true;
    }

    /// Every key event from the hardware keyboard, lit on the viewer while it is up.
    pub fn keyboard_viewer_key(&mut self, code: u16, down: bool) {
        if let Some(v) = self.keyboard_viewer.as_mut() {
            if v.key(code, down) {
                self.dirty = true;
            }
        }
    }

    /// A touch while the viewer is up: only its Done button is taken. None lets the touch reach the desktop.
    pub(super) fn keyboard_viewer_touch(&mut self, ev: Event) -> Option<Vec<Effect>> {
        let Event::Up(x, y) = ev else { return None };
        if self.keyboard_viewer.is_none() || !KeyboardViewer::hit_done(&self.ui, self.w, self.h, x, y) {
            return None;
        }
        self.keyboard_viewer = None;
        self.drawn = Drawn::Nothing;
        Some(vec![])
    }

    /// The layout the viewer draws: the active source's keymap.
    pub(super) fn keyboard_viewer_layout(&self) -> usize {
        crate::installer::keymaps::layout_index(super::kb_layouts::active().keymap()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_four_rows_are_fifteen_units_wide() {
        for row in BOARD.iter().take(4) {
            let total: f32 = row.iter().map(|k| k.1).sum();
            assert!((total - UNITS).abs() < 0.01, "a row is {total} units");
        }
    }

    #[test]
    fn every_key_appears_once_on_the_board() {
        let mut codes: Vec<u16> = BOARD.iter().flat_map(|row| row.iter().map(|k| k.0)).collect();
        let n = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), n);
    }

    #[test]
    fn modifiers_carry_their_names_and_letters_show_both_levels() {
        assert_eq!(legend(42, None), Legend::Name("shift"));
        assert_eq!(legend(14, None), Legend::Name("delete"));
        assert_eq!(legend(57, Some([' ', ' ', '\0', '\0'])), Legend::Blank, "the space bar has no legend");
        assert_eq!(legend(30, Some(['a', 'A', '\0', '\0'])), Legend::Chars { plain: 'a', shift: 'A' });
        assert_eq!(legend(2, Some(['1', '!', '\0', '\0'])), Legend::Chars { plain: '1', shift: '!' });
    }

    #[test]
    fn the_layout_tables_give_the_letters_of_the_active_source() {
        // the Icelandic board has ð where the US board has the bracket: key 26 is the same key on both
        let is = crate::installer::keymaps::layout_index("is").unwrap();
        let us = crate::installer::keymaps::layout_index("us").unwrap();
        assert_eq!(key_levels(is, 30).map(|l| l[0]), Some('a'));
        assert_eq!(key_levels(us, 26).map(|l| l[0]), Some('['));
        assert_eq!(key_levels(is, 26).map(|l| l[0]), Some('ð'));
        assert_eq!(key_levels(usize::MAX, 30), None, "an unknown layout has no legends");
    }

    #[test]
    fn a_held_key_is_lit_until_it_is_released() {
        let mut v = KeyboardViewer::new();
        assert!(v.key(30, true));
        assert_eq!(v.held(), Some(30));
        assert!(!v.key(30, true), "a repeat changes nothing");
        assert!(!v.key(31, false), "a release of another key changes nothing");
        assert!(v.key(30, false));
        assert_eq!(v.held(), None);
        assert!(!v.key(999, true), "keys off the board are not lit");
    }
}
