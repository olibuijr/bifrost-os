//! The Color Meter's own menus: View (updating, colour space, value format, magnification, location, locks, aperture) and
//! Color (copy as text or as an image). `menus` builds them from the window state, so checks and enabled state are
//! recomputed after every event; `perform_menu` carries out the item the shell echoes back. The menu items and the
//! toolbar share the window's actions, so both reach the same state.

use crate::color::{CopyFormat, Space};
use crate::window::{ColorMeter, Outcome, APERTURE_MAX, APERTURE_MIN, ZOOMS};
use bifrost_app::{AppMenu, AppMenuItem};

pub const M_CONTINUOUS: u32 = 1;
pub const M_SHOW_LOCATION: u32 = 2;
pub const M_LOCK: u32 = 3;
pub const M_LOCK_X: u32 = 4;
pub const M_LOCK_Y: u32 = 5;
pub const M_APERTURE_UP: u32 = 6;
pub const M_APERTURE_DOWN: u32 = 7;
pub const M_COPY_TEXT: u32 = 8;
pub const M_COPY_IMAGE: u32 = 9;
/// The first id of the colour space items; the item for `Space::ALL[i]` is `M_SPACE + i`.
pub const M_SPACE: u32 = 100;
/// The first id of the value format items, in the order of `CopyFormat::ALL`.
pub const M_FORMAT: u32 = 200;
/// The first id of the magnification items, in the order of `ZOOMS`.
pub const M_ZOOM: u32 = 300;

/// The menu title of a value format.
fn format_title(f: CopyFormat) -> &'static str {
    match f {
        CopyFormat::Hex => "Hexadecimal",
        CopyFormat::Decimal => "Decimal (8-bit)",
        CopyFormat::Bit16 => "Decimal (16-bit)",
        CopyFormat::Percent => "Percent",
    }
}

/// The index of `id` in the run of `count` items that starts at `base`.
fn index_in(id: u32, base: u32, count: usize) -> Option<usize> {
    id.checked_sub(base).map(|i| i as usize).filter(|i| *i < count)
}

impl ColorMeter {
    /// The View and Color menus for the current state.
    pub fn menus(&self) -> Vec<AppMenu> {
        let reading = self.reading().is_some();
        let mut view = vec![AppMenuItem::new(M_CONTINUOUS, "Update Continuously").checked(self.continuous), AppMenuItem::separator()];
        view.push(AppMenuItem::submenu("Display In"));
        for (i, space) in Space::ALL.iter().enumerate() {
            view.push(AppMenuItem::new(M_SPACE + i as u32, space.name()).checked(self.space == *space).depth(1));
        }
        view.push(AppMenuItem::submenu("Display Values"));
        for (i, format) in CopyFormat::ALL.iter().enumerate() {
            view.push(AppMenuItem::new(M_FORMAT + i as u32, format_title(*format)).checked(self.format == *format).depth(1));
        }
        view.push(AppMenuItem::submenu("Magnification"));
        for (i, cells) in ZOOMS.iter().enumerate() {
            view.push(AppMenuItem::new(M_ZOOM + i as u32, &format!("{cells} x {cells} pixels")).checked(self.cells == *cells).depth(1));
        }
        view.push(AppMenuItem::separator());
        view.push(AppMenuItem::new(M_SHOW_LOCATION, "Show Mouse Location").checked(self.show_position));
        view.push(AppMenuItem::new(M_LOCK, "Lock Position").key("cmd+l").checked(self.pointer.locked()));
        view.push(AppMenuItem::new(M_LOCK_X, "Lock X Coordinate").key("cmd+x").checked(self.pointer.lock_x));
        view.push(AppMenuItem::new(M_LOCK_Y, "Lock Y Coordinate").key("cmd+y").checked(self.pointer.lock_y));
        view.push(AppMenuItem::separator());
        view.push(AppMenuItem::new(M_APERTURE_UP, "Increase Aperture Size").enabled(self.aperture < APERTURE_MAX));
        view.push(AppMenuItem::new(M_APERTURE_DOWN, "Decrease Aperture Size").enabled(self.aperture > APERTURE_MIN));
        vec![
            AppMenu { title: "View".into(), items: view },
            AppMenu {
                title: "Color".into(),
                items: vec![
                    AppMenuItem::new(M_COPY_TEXT, "Copy Color as Text").key("cmd+shift+c").enabled(reading),
                    AppMenuItem::new(M_COPY_IMAGE, "Copy Color as Image").key("cmd+alt+c").enabled(reading),
                ],
            },
        ]
    }

    /// Carries out menu item `id`. `Outcome::Nothing` for an id that is not ours.
    pub fn perform_menu(&mut self, id: u32) -> Outcome {
        match id {
            M_CONTINUOUS => self.set_continuous(!self.continuous),
            M_SHOW_LOCATION => self.show_position = !self.show_position,
            M_LOCK => self.pointer.toggle_lock(),
            M_LOCK_X => self.pointer.toggle_lock_x(),
            M_LOCK_Y => self.pointer.toggle_lock_y(),
            M_APERTURE_UP => {
                self.set_aperture(self.aperture.saturating_add(2));
            }
            M_APERTURE_DOWN => {
                self.set_aperture(self.aperture.saturating_sub(2));
            }
            M_COPY_TEXT => return self.copy(self.format),
            M_COPY_IMAGE => return self.copy_swatch(),
            _ => {
                if let Some(i) = index_in(id, M_SPACE, Space::ALL.len()) {
                    self.space = Space::ALL[i];
                } else if let Some(i) = index_in(id, M_FORMAT, CopyFormat::ALL.len()) {
                    self.format = CopyFormat::ALL[i];
                } else if let Some(i) = index_in(id, M_ZOOM, ZOOMS.len()) {
                    self.cells = ZOOMS[i];
                } else {
                    return Outcome::Nothing;
                }
            }
        }
        Outcome::Redraw
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgb8;
    use crate::source::{Fixture, ShellSource};
    use crate::window::{Target, B_COPY};

    fn meter() -> ColorMeter {
        let px: Vec<Rgb8> = (0..400u32).map(|i| Rgb8::new((i % 256) as u8, 40, 200)).collect();
        let mut f = Fixture::new(20, 20, px).unwrap();
        f.at = Some((3, 4));
        let mut m = ColorMeter::new(Box::new(f));
        m.refresh();
        m
    }

    fn item<'a>(menus: &'a [AppMenu], id: u32) -> &'a AppMenuItem {
        menus.iter().flat_map(|m| m.items.iter()).find(|i| i.id == id).expect("menu item")
    }

    #[test]
    fn the_menus_are_view_and_color() {
        let titles: Vec<String> = meter().menus().into_iter().map(|m| m.title).collect();
        assert_eq!(titles, ["View", "Color"]);
    }

    #[test]
    fn every_item_id_is_unique_and_within_the_shell_limits() {
        let menus = meter().menus();
        assert!(menus.len() <= bifrost_app::proto::MAX_APP_MENUS);
        let mut ids = Vec::new();
        for m in &menus {
            assert!(m.items.len() <= bifrost_app::proto::MAX_APP_MENU_ITEMS, "{}", m.title);
            for i in m.items.iter().filter(|i| i.id != 0) {
                assert!(i.title.chars().count() <= bifrost_app::proto::MAX_APP_MENU_TITLE);
                assert!(!ids.contains(&i.id), "duplicate id {}", i.id);
                ids.push(i.id);
            }
        }
    }

    #[test]
    fn every_key_equivalent_parses() {
        for m in meter().menus() {
            for i in m.items.iter().filter(|i| !i.key.is_empty()) {
                assert!(bifrost_app::controls::keys::KeyCombo::parse(&i.key).is_some(), "{}", i.key);
            }
        }
    }

    #[test]
    fn the_documented_shortcuts_are_bound() {
        let menus = meter().menus();
        assert_eq!(item(&menus, M_LOCK).key, "cmd+l");
        assert_eq!(item(&menus, M_LOCK_X).key, "cmd+x");
        assert_eq!(item(&menus, M_LOCK_Y).key, "cmd+y");
        assert_eq!(item(&menus, M_COPY_TEXT).key, "cmd+shift+c");
        assert_eq!(item(&menus, M_COPY_IMAGE).key, "cmd+alt+c");
    }

    #[test]
    fn the_checks_follow_the_state() {
        let mut m = meter();
        let menus = m.menus();
        assert!(item(&menus, M_CONTINUOUS).checked);
        assert!(item(&menus, M_SHOW_LOCATION).checked);
        assert!(!item(&menus, M_LOCK).checked);
        assert!(item(&menus, M_SPACE + 1).checked, "sRGB is the default space");
        assert!(item(&menus, M_FORMAT).checked, "hex is the default format");
        m.perform_menu(M_LOCK_X);
        m.perform_menu(M_CONTINUOUS);
        let menus = m.menus();
        assert!(item(&menus, M_LOCK_X).checked && !item(&menus, M_LOCK_Y).checked && !item(&menus, M_LOCK).checked);
        assert!(!item(&menus, M_CONTINUOUS).checked);
        m.perform_menu(M_LOCK_Y);
        assert!(item(&m.menus(), M_LOCK).checked);
    }

    #[test]
    fn the_space_items_pick_every_space() {
        let mut m = meter();
        for (i, space) in Space::ALL.iter().enumerate() {
            assert_eq!(m.perform_menu(M_SPACE + i as u32), Outcome::Redraw);
            assert_eq!(m.space, *space);
            assert!(item(&m.menus(), M_SPACE + i as u32).checked);
        }
        assert!(Space::ALL.contains(&Space::AdobeRgb));
    }

    #[test]
    fn the_format_items_pick_every_format() {
        let mut m = meter();
        for (i, format) in CopyFormat::ALL.iter().enumerate() {
            m.perform_menu(M_FORMAT + i as u32);
            assert_eq!(m.format, *format);
        }
        assert_eq!(m.format, CopyFormat::Percent);
    }

    #[test]
    fn the_magnification_items_set_the_loupe_size() {
        let mut m = meter();
        for (i, cells) in ZOOMS.iter().enumerate() {
            m.perform_menu(M_ZOOM + i as u32);
            assert_eq!(m.cells, *cells);
            assert!(item(&m.menus(), M_ZOOM + i as u32).checked);
        }
    }

    #[test]
    fn the_toggles_flip_their_state() {
        let mut m = meter();
        m.perform_menu(M_SHOW_LOCATION);
        assert!(!m.show_position);
        m.perform_menu(M_CONTINUOUS);
        assert!(!m.continuous);
        m.perform_menu(M_LOCK);
        assert!(m.pointer.locked());
        m.perform_menu(M_LOCK);
        assert!(!m.pointer.lock_x && !m.pointer.lock_y);
    }

    #[test]
    fn the_aperture_items_step_and_disable_at_the_ends() {
        let mut m = meter();
        assert!(!item(&m.menus(), M_APERTURE_DOWN).enabled);
        m.perform_menu(M_APERTURE_UP);
        assert_eq!(m.aperture, APERTURE_MIN + 2, "the menu steps between odd sizes");
        assert!(item(&m.menus(), M_APERTURE_DOWN).enabled);
        m.set_aperture(APERTURE_MAX);
        assert!(!item(&m.menus(), M_APERTURE_UP).enabled);
        m.perform_menu(M_APERTURE_UP);
        assert_eq!(m.aperture, APERTURE_MAX);
        m.perform_menu(M_APERTURE_DOWN);
        assert_eq!(m.aperture, APERTURE_MAX - 2);
    }

    #[test]
    fn the_copy_items_give_the_same_outcomes_as_the_toolbar() {
        let mut m = meter();
        let by_menu = m.perform_menu(M_COPY_TEXT);
        let by_toolbar = m.activate(Target::Toolbar(B_COPY));
        assert_eq!(by_menu, by_toolbar);
        // the pixel at (3, 4) of a 20-wide display is index 83: red 0x53
        assert_eq!(by_menu, Outcome::CopyText("#5328C8".into()));
        assert_eq!(m.perform_menu(M_COPY_IMAGE), Outcome::CopySwatch(Rgb8::new(3 + 4 * 20, 40, 200)));
    }

    #[test]
    fn the_copy_items_are_disabled_without_a_reading() {
        let m = ColorMeter::new(Box::new(ShellSource::default()));
        let menus = m.menus();
        assert!(!item(&menus, M_COPY_TEXT).enabled);
        assert!(!item(&menus, M_COPY_IMAGE).enabled);
        let mut m = m;
        assert_eq!(m.perform_menu(M_COPY_TEXT), Outcome::Nothing);
        assert_eq!(m.perform_menu(M_COPY_IMAGE), Outcome::Nothing);
    }

    #[test]
    fn an_unknown_id_does_nothing() {
        let mut m = meter();
        assert_eq!(m.perform_menu(0), Outcome::Nothing);
        assert_eq!(m.perform_menu(M_SPACE + Space::ALL.len() as u32), Outcome::Nothing);
        assert_eq!(m.perform_menu(9999), Outcome::Nothing);
    }

    #[test]
    fn the_submenu_children_follow_their_header_one_level_down() {
        let menus = meter().menus();
        let view = &menus[0].items;
        for (i, header) in view.iter().enumerate().filter(|(_, it)| it.submenu) {
            assert_eq!(header.depth, 0);
            assert!(view[i + 1].depth == 1, "{} has no children", header.title);
        }
        assert_eq!(view.iter().filter(|it| it.submenu).count(), 3);
    }
}
