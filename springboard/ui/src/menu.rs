//! Menus and pop-up buttons (`NativeMenu`, `NativePopUpButton`): items with checkmarks, separators and disabled rows,
//! keyboard navigation that skips what cannot be chosen, type-to-select, hit testing, and the pop-up rule that
//! the open menu is placed so the selected item sits over the button. Drawing stays in the renderer adapter.
use crate::Rect;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub id: u32,
    pub title: String,
    pub enabled: bool,
    pub checked: bool,
    pub separator: bool,
    /// Display text for a key equivalent, e.g. "⌘N".
    pub shortcut: Option<String>,
    /// A destructive action (Delete, Remove), drawn in systemRed (`NativeMenuElement.Attributes.destructive`; Golden Gate
    /// shows Delete items in red in Golden Gate context menus).
    pub destructive: bool,
    /// A colour swatch drawn before the title (0xAARRGGBB), as a calendar or a tag colour shows in its pop-up menu (the
    /// small round image `NativeMenuItem.image` carries there). When any item has one, every title moves over by the
    /// swatch's width so the titles stay aligned.
    pub swatch: Option<u32>,
}

impl MenuItem {
    pub fn new(id: u32, title: &str) -> Self {
        MenuItem { id, title: title.into(), enabled: true, checked: false, separator: false, shortcut: None, destructive: false, swatch: None }
    }
    pub fn separator() -> Self {
        MenuItem { id: 0, title: String::new(), enabled: false, checked: false, separator: true, shortcut: None, destructive: false, swatch: None }
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }
    pub fn shortcut(mut self, s: &str) -> Self {
        self.shortcut = Some(s.into());
        self
    }
    pub fn destructive(mut self, on: bool) -> Self {
        self.destructive = on;
        self
    }
    /// Draws a round swatch of `colour` before the title (see `swatch`).
    pub fn swatch(mut self, colour: u32) -> Self {
        self.swatch = Some(colour);
        self
    }
    fn selectable(&self) -> bool {
        self.enabled && !self.separator
    }
}

/// Type-to-select forgets what you typed after this long (seconds).
pub const TYPE_SELECT_SECS: f64 = 1.0;

#[derive(Clone, Debug, Default)]
pub struct Menu {
    pub items: Vec<MenuItem>,
    pub highlighted: Option<usize>,
    typed: String,
    typed_at: f64,
}

impl Menu {
    pub fn new(items: Vec<MenuItem>) -> Self {
        Menu { items, ..Menu::default() }
    }
    /// The pop-up button's menu: exactly one item is checked and highlighted to start with.
    pub fn popup(items: Vec<MenuItem>, selected: usize) -> Self {
        let mut m = Menu::new(items);
        for (i, it) in m.items.iter_mut().enumerate() {
            it.checked = i == selected;
        }
        m.highlighted = Some(selected).filter(|&i| m.items.get(i).is_some_and(|it| it.selectable()));
        m
    }
    pub fn selected(&self) -> Option<usize> {
        self.items.iter().position(|i| i.checked)
    }
    /// Arrow keys: the next selectable row in `dir` (+1 / -1), wrapping. With nothing highlighted, the first or last.
    pub fn step(&mut self, dir: i32) -> Option<usize> {
        let n = self.items.len() as i32;
        if n == 0 || !self.items.iter().any(MenuItem::selectable) {
            return None;
        }
        let mut i = match self.highlighted {
            Some(h) => h as i32,
            None => if dir > 0 { -1 } else { n },
        };
        loop {
            i = (i + dir.signum()).rem_euclid(n);
            if self.items[i as usize].selectable() {
                self.highlighted = Some(i as usize);
                return self.highlighted;
            }
        }
    }
    pub fn first(&mut self) -> Option<usize> {
        self.highlighted = None;
        self.step(1)
    }
    pub fn last(&mut self) -> Option<usize> {
        self.highlighted = None;
        self.step(-1)
    }
    /// Type-to-select: letters typed within `TYPE_SELECT_SECS` of each other extend one prefix; the first match at
    /// or after the highlighted row wins, and repeating one letter cycles through the rows that start with it.
    pub fn type_select(&mut self, ch: char, now: f64) -> Option<usize> {
        if now - self.typed_at > TYPE_SELECT_SECS {
            self.typed.clear();
        }
        self.typed_at = now;
        let cycling = self.typed.chars().all(|c| c.eq_ignore_ascii_case(&ch)) && !self.typed.is_empty();
        self.typed.extend(ch.to_lowercase());
        let prefix = if cycling { ch.to_lowercase().to_string() } else { self.typed.clone() };
        let n = self.items.len();
        let from = self.highlighted.map_or(0, |h| if cycling { h + 1 } else { h });
        let hit = (0..n).map(|k| (from + k) % n.max(1)).find(|&i| self.items[i].selectable() && self.items[i].title.to_lowercase().starts_with(&prefix));
        if hit.is_some() {
            self.highlighted = hit;
        }
        hit
    }
    /// Return / click: the id of the highlighted item, if it can be chosen.
    pub fn activate(&self) -> Option<u32> {
        self.highlighted.and_then(|i| self.items.get(i)).filter(|i| i.selectable()).map(|i| i.id)
    }
    /// Row tops and heights in points, from `y = 0`, and the total height.
    pub fn rows(&self, row_h: f32, separator_h: f32, padding: f32) -> (Vec<(f32, f32)>, f32) {
        let mut y = padding;
        let rows = self.items.iter().map(|it| {
            let h = if it.separator { separator_h } else { row_h };
            let r = (y, h);
            y += h;
            r
        }).collect();
        (rows, y + padding)
    }
    /// The selectable row at `y` (points from the menu's top), `None` over separators, disabled rows and padding.
    pub fn hit(&self, y: f32, row_h: f32, separator_h: f32, padding: f32) -> Option<usize> {
        let (rows, _) = self.rows(row_h, separator_h, padding);
        rows.iter().position(|&(top, h)| y >= top && y < top + h).filter(|&i| self.items[i].selectable())
    }
}

/// Where the open pop-up menu goes (`NativePopUpButton`): vertically so the selected row covers the button, then
/// slid to stay inside `bounds`. `selected_top` is that row's top inside the menu. Returns the menu's top-left.
pub fn popup_origin(button: Rect, menu_w: f32, menu_h: f32, selected_top: f32, bounds: Rect) -> (f32, f32) {
    let x = button.x.clamp(bounds.x, (bounds.x + bounds.w - menu_w).max(bounds.x));
    let y = (button.y - selected_top).clamp(bounds.y, (bounds.y + bounds.h - menu_h).max(bounds.y));
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_swatch_is_kept_and_leaves_the_item_selectable() {
        let it = MenuItem::new(3, "Work").swatch(0xFF34C759);
        assert_eq!(it.swatch, Some(0xFF34C759));
        assert!(it.selectable());
        assert_eq!(MenuItem::new(1, "Home").swatch, None);
        assert_eq!(MenuItem::separator().swatch, None);
    }
    #[test]
    fn a_destructive_item_is_still_selectable() {
        let it = MenuItem::new(9, "Delete").destructive(true);
        assert!(it.destructive && it.selectable());
        assert!(!MenuItem::new(1, "Open").destructive && !MenuItem::separator().destructive);
    }
    fn sample() -> Menu {
        Menu::new(vec![
            MenuItem::new(1, "Automatic"),
            MenuItem::new(2, "Intel HD Graphics"),
            MenuItem::separator(),
            MenuItem::new(3, "Integrated only").enabled(false),
            MenuItem::new(4, "Performance"),
        ])
    }
    #[test]
    fn arrows_skip_separators_and_disabled_rows_and_wrap() {
        let mut m = sample();
        assert_eq!(m.step(1), Some(0));
        assert_eq!(m.step(1), Some(1));
        assert_eq!(m.step(1), Some(4));
        assert_eq!(m.step(1), Some(0));
        assert_eq!(m.step(-1), Some(4));
        assert_eq!(m.last(), Some(4));
        assert_eq!(m.first(), Some(0));
    }
    #[test]
    fn nothing_selectable_gives_none() {
        let mut m = Menu::new(vec![MenuItem::separator(), MenuItem::new(1, "x").enabled(false)]);
        assert_eq!(m.step(1), None);
        assert_eq!(Menu::default().step(-1), None);
    }
    #[test]
    fn type_select_extends_a_prefix_and_cycles_one_letter() {
        let mut m = Menu::new(vec![MenuItem::new(1, "Apricot"), MenuItem::new(2, "Avocado"), MenuItem::new(3, "Banana"), MenuItem::new(4, "Avenue")]);
        assert_eq!(m.type_select('a', 0.0), Some(0));
        assert_eq!(m.type_select('a', 0.2), Some(1)); // same letter again: next row with that letter
        assert_eq!(m.type_select('a', 0.4), Some(3));
        assert_eq!(m.type_select('b', 5.0), Some(2)); // typing paused: a fresh prefix
        m.highlighted = None;
        assert_eq!(m.type_select('a', 10.0), Some(0));
        assert_eq!(m.type_select('v', 10.2), Some(1)); // "av" -> Avocado
        assert_eq!(m.type_select('e', 10.4), Some(3)); // "ave" -> Avenue
        assert_eq!(m.type_select('z', 10.5), None);
        assert_eq!(m.highlighted, Some(3));
    }
    #[test]
    fn activate_returns_the_item_id_only_for_a_choosable_row() {
        let mut m = sample();
        m.highlighted = Some(1);
        assert_eq!(m.activate(), Some(2));
        m.highlighted = Some(3);
        assert_eq!(m.activate(), None);
        m.highlighted = None;
        assert_eq!(m.activate(), None);
    }
    #[test]
    fn rows_and_hit_testing() {
        let m = sample();
        let (rows, total) = m.rows(22.0, 9.0, 5.0);
        assert_eq!(rows[0], (5.0, 22.0));
        assert_eq!(rows[2], (49.0, 9.0));
        assert_eq!(total, 5.0 + 22.0 * 4.0 + 9.0 + 5.0);
        assert_eq!(m.hit(10.0, 22.0, 9.0, 5.0), Some(0));
        assert_eq!(m.hit(52.0, 22.0, 9.0, 5.0), None); // separator
        assert_eq!(m.hit(70.0, 22.0, 9.0, 5.0), None); // disabled
        assert_eq!(m.hit(85.0, 22.0, 9.0, 5.0), Some(4));
        assert_eq!(m.hit(2.0, 22.0, 9.0, 5.0), None); // padding
    }
    #[test]
    fn popup_marks_one_item_and_covers_the_button_with_it() {
        let m = Menu::popup(vec![MenuItem::new(1, "A"), MenuItem::new(2, "B"), MenuItem::new(3, "C")], 1);
        assert_eq!(m.selected(), Some(1));
        assert_eq!(m.highlighted, Some(1));
        let screen = Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };
        let button = Rect { x: 100.0, y: 300.0, w: 160.0, h: 22.0 };
        assert_eq!(popup_origin(button, 160.0, 90.0, 27.0, screen), (100.0, 273.0));
        // near the bottom edge the menu slides up; near the right edge it slides left
        let low = Rect { x: 700.0, y: 590.0, w: 160.0, h: 22.0 };
        assert_eq!(popup_origin(low, 160.0, 90.0, 27.0, screen), (640.0, 510.0));
        assert_eq!(popup_origin(Rect { y: 5.0, ..button }, 160.0, 90.0, 27.0, screen).1, 0.0);
    }
}
