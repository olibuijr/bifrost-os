//! The Setup Assistant's window state: the screen on show, what the user has chosen so far, and the layout. Every
//! button, chip and switch is an `Item` with its rectangle in points, so what is painted (`draw.rs`) is what a tap
//! hits (`act_at`). The model (steps, choices, the state file) is `springboard::setup`.

use bifrost_app::kit::TextCache;
use bifrost_app::kit::appearance::{Accent, Mode};
use springboard::installer::app::{layout_for_locale, LANGUAGE_LAYOUTS};
use springboard::installer::keymaps;
use springboard::setup::{self, Choices, Cloud, Step, TIMEZONES};
use springboard::shell::prefs::Prefs;
use springboard::shell::settings::{ACCENT_PREF, APPEARANCE_PREF};

/// The margin of every page, in points.
pub const PAD_PT: f32 = 40.0;
/// The height of the footer buttons and of a chip, in points.
pub const BTN_PT: f32 = 36.0;
/// Chip width and the gap between chips and rows, in points.
const CHIP_W: f32 = 168.0;
pub const GAP_PT: f32 = 12.0;
/// Where the step's own content starts, below the title and body text.
pub const CONTENT_Y: f32 = 214.0;
/// A switch row's height.
pub const ROW_PT: f32 = 56.0;

/// A rectangle in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// What a control does when it is tapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Next,
    Back,
    /// An index into `Setup::zones`.
    Zone(usize),
    /// An index into `LANGUAGE_LAYOUTS`.
    Layout(usize),
    Switch(Switch),
    Look(Mode),
    Tint(Accent),
    Cloud(Cloud),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    Shortcut,
    Location,
    Analytics,
}

/// How a control is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Back: a quiet button.
    Plain,
    /// Continue and Get Started: the one tinted button on a page.
    Primary,
    /// A choice among several; `on` marks the chosen one.
    Chip,
    /// A row with a label and a switch at its right; `on` is the switch.
    Switch,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub r: Rect,
    pub act: Act,
    pub kind: Kind,
    pub label: String,
    pub on: bool,
    /// The accent colour a chip shows as a dot (appearance page), in the window's scheme.
    pub swatch: Option<u32>,
}

/// The whole window.
pub struct Setup {
    /// The window size in points.
    pub size: (f32, f32),
    /// True in the Dark scheme; swatches follow it.
    pub dark: bool,
    pub step: Step,
    pub choices: Choices,
    /// The zone the system uses when the window opened (`/etc/localtime`), if it can be read.
    pub zone_now: Option<String>,
    /// The system language (`is_IS.UTF-8`), for the Language page.
    pub locale: String,
    pub online: bool,
    /// The installed account: full name and user name.
    pub account: (String, String),
    /// A line shown above the footer when finishing failed.
    pub error: Option<String>,
    /// The control a pointer went down on; it fires only when the pointer comes up on it.
    pub press: Option<Act>,
    pub labels: TextCache,
    pub now: f64,
}

impl Setup {
    /// The window as the installer left the machine: the install-time look and keyboard are pre-filled, so the
    /// assistant only asks what the installer did not.
    pub fn start() -> Setup {
        let shell_dir = setup::shell_data_dir();
        let prefs = Prefs::load(shell_dir.join("settings.ini"));
        let locale = std::fs::read_to_string("/etc/locale.conf").ok().and_then(|t| setup::locale_from(&t)).unwrap_or_default();
        let mut choices = Choices::default();
        if let Some(id) = layout_for_locale(&locale) {
            choices.keyboard = id.to_string();
        }
        if let Some(m) = prefs.get(APPEARANCE_PREF) {
            choices.appearance = Mode::parse(m);
        }
        if let Some(a) = prefs.get(ACCENT_PREF) {
            choices.accent = Accent::parse(a);
        }
        let account = springboard::shell::account::load();
        Setup {
            size: (0.0, 0.0),
            dark: true,
            step: Step::Welcome,
            choices,
            zone_now: setup::current_timezone(std::path::Path::new("/etc/localtime")),
            locale,
            online: setup::online(&setup::interface_states(std::path::Path::new("/sys/class/net"))),
            account: (account.full_name, account.username),
            error: None,
            press: None,
            labels: TextCache::default(),
            now: 0.0,
        }
    }

    /// The zones the Region page offers: the current zone first when it is not in the standard list.
    pub fn zones(&self) -> Vec<String> {
        let mut v: Vec<String> = TIMEZONES.iter().map(|z| z.to_string()).collect();
        if let Some(now) = &self.zone_now {
            if !v.contains(now) {
                v.insert(0, now.clone());
            }
        }
        v
    }

    /// The zone the Region page shows as chosen: the user's pick, else the system's current zone.
    pub fn zone_selected(&self) -> Option<&str> {
        if self.choices.timezone.is_empty() {
            self.zone_now.as_deref()
        } else {
            Some(self.choices.timezone.as_str())
        }
    }

    /// The width of the content column in points.
    fn content_w(&self) -> f32 {
        (self.size.0 - 2.0 * PAD_PT).max(0.0)
    }

    /// Chips in a grid from `y`, as many per row as fit the content width.
    fn grid(&self, n: usize, y: f32, chip_w: f32) -> Vec<Rect> {
        let cols = ((self.content_w() + GAP_PT) / (chip_w + GAP_PT)).floor().max(1.0) as usize;
        (0..n)
            .map(|i| {
                let (col, row) = (i % cols, i / cols);
                Rect { x: PAD_PT + col as f32 * (chip_w + GAP_PT), y: y + row as f32 * (BTN_PT + GAP_PT), w: chip_w, h: BTN_PT }
            })
            .collect()
    }

    /// Every control on the current screen, footer included.
    pub fn items(&self) -> Vec<Item> {
        let mut out = Vec::new();
        let (w, h) = self.size;
        let footer_y = h - PAD_PT - BTN_PT;
        if self.step != Step::Welcome {
            out.push(Item { r: Rect { x: PAD_PT, y: footer_y, w: 110.0, h: BTN_PT }, act: Act::Back, kind: Kind::Plain, label: "Back".into(), on: false, swatch: None });
        }
        let next = if self.step == Step::Done { "Get Started" } else { "Continue" };
        out.push(Item { r: Rect { x: w - PAD_PT - 160.0, y: footer_y, w: 160.0, h: BTN_PT }, act: Act::Next, kind: Kind::Primary, label: next.into(), on: false, swatch: None });

        match self.step {
            Step::Region => {
                let zones = self.zones();
                for (i, (r, z)) in self.grid(zones.len(), CONTENT_Y, 260.0).into_iter().zip(&zones).enumerate() {
                    let on = self.zone_selected() == Some(z.as_str());
                    out.push(Item { r, act: Act::Zone(i), kind: Kind::Chip, label: z.clone(), on, swatch: None });
                }
            }
            Step::Keyboard => {
                // LANGUAGE_LAYOUTS pairs (locale, keyboard layout id); the chip is the layout id
                for (i, (r, (_, id))) in self.grid(LANGUAGE_LAYOUTS.len(), CONTENT_Y, CHIP_W).into_iter().zip(LANGUAGE_LAYOUTS).enumerate() {
                    let name = keymaps::layouts().find(|l| l.id == *id).map_or(*id, |l| l.name);
                    out.push(Item { r, act: Act::Layout(i), kind: Kind::Chip, label: name.to_string(), on: self.choices.keyboard == *id, swatch: None });
                }
            }
            Step::Accessibility => {
                out.push(self.switch_row(CONTENT_Y, "Accessibility shortcut", Act::Switch(Switch::Shortcut), self.choices.accessibility_shortcut));
            }
            Step::Appearance => {
                let modes = [(Mode::Light, "Light"), (Mode::Dark, "Dark"), (Mode::Auto, "Auto")];
                for (r, (m, label)) in self.grid(modes.len(), CONTENT_Y, CHIP_W).into_iter().zip(modes) {
                    out.push(Item { r, act: Act::Look(m), kind: Kind::Chip, label: label.into(), on: self.choices.appearance == m, swatch: None });
                }
                let accent_y = CONTENT_Y + BTN_PT + GAP_PT + 40.0;
                for (r, a) in self.grid(Accent::ALL.len(), accent_y, CHIP_W).into_iter().zip(Accent::ALL) {
                    let (dark, light) = a.colours();
                    let swatch = if self.dark { dark } else { light };
                    out.push(Item { r, act: Act::Tint(a), kind: Kind::Chip, label: a.label().into(), on: self.choices.accent == a, swatch: Some(swatch) });
                }
            }
            Step::Privacy => {
                out.push(self.switch_row(CONTENT_Y, "Location Services", Act::Switch(Switch::Location), self.choices.location));
                out.push(self.switch_row(CONTENT_Y + ROW_PT + GAP_PT, "Share Analytics & Diagnostics", Act::Switch(Switch::Analytics), self.choices.analytics));
            }
            Step::Cloud => {
                let options = [(Cloud::SignIn, "Sign In"), (Cloud::Later, "Later")];
                for (r, (c, label)) in self.grid(options.len(), CONTENT_Y, CHIP_W).into_iter().zip(options) {
                    out.push(Item { r, act: Act::Cloud(c), kind: Kind::Chip, label: label.into(), on: self.choices.cloud == c, swatch: None });
                }
            }
            _ => {}
        }
        out
    }

    fn switch_row(&self, y: f32, label: &str, act: Act, on: bool) -> Item {
        Item { r: Rect { x: PAD_PT, y, w: self.content_w(), h: ROW_PT }, act, kind: Kind::Switch, label: label.into(), on, swatch: None }
    }

    /// The control under a point, if any.
    pub fn act_at(&self, x: f32, y: f32) -> Option<Act> {
        self.items().into_iter().find(|i| i.r.contains(x, y)).map(|i| i.act)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(step: Step) -> Setup {
        Setup {
            size: (1200.0, 780.0),
            dark: true,
            step,
            choices: Choices::default(),
            zone_now: None,
            locale: String::new(),
            online: false,
            account: ("Ada".into(), "ada".into()),
            error: None,
            press: None,
            labels: TextCache::default(),
            now: 0.0,
        }
    }

    #[test]
    fn welcome_has_no_back_button() {
        let acts: Vec<Act> = window(Step::Welcome).items().iter().map(|i| i.act).collect();
        assert_eq!(acts, [Act::Next]);
        assert!(window(Step::Language).items().iter().any(|i| i.act == Act::Back));
    }

    #[test]
    fn the_footer_button_is_get_started_on_the_last_screen() {
        // the footer button is the Next item; option chips (such as Cloud's Sign In / Later) come after it in the list
        let footer = |step| window(step).items().into_iter().find(|i| i.act == Act::Next).map(|i| i.label);
        assert_eq!(footer(Step::Done).as_deref(), Some("Get Started"));
        assert_eq!(footer(Step::Cloud).as_deref(), Some("Continue"));
    }

    #[test]
    fn the_current_zone_leads_the_list_when_it_is_not_standard() {
        let mut w = window(Step::Region);
        w.zone_now = Some("Atlantic/Azores".into());
        assert_eq!(w.zones()[0], "Atlantic/Azores");
        assert_eq!(w.zones().len(), TIMEZONES.len() + 1);
        w.zone_now = Some("UTC".into());
        assert_eq!(w.zones().len(), TIMEZONES.len());
    }

    #[test]
    fn a_tap_on_a_zone_chip_picks_that_zone() {
        let w = window(Step::Region);
        let zones = w.zones();
        let chip = w.items().into_iter().find(|i| i.label == "Europe/London").unwrap();
        let act = w.act_at(chip.r.x + 5.0, chip.r.y + 5.0).unwrap();
        assert_eq!(act, Act::Zone(zones.iter().position(|z| z == "Europe/London").unwrap()));
    }

    #[test]
    fn appearance_offers_three_modes_and_every_accent() {
        let items = window(Step::Appearance).items();
        assert_eq!(items.iter().filter(|i| matches!(i.act, Act::Look(_))).count(), 3);
        let tints: Vec<&Item> = items.iter().filter(|i| matches!(i.act, Act::Tint(_))).collect();
        assert_eq!(tints.len(), Accent::ALL.len());
        assert!(tints.iter().all(|i| i.swatch.is_some()));
    }

    #[test]
    fn switches_and_chips_show_the_current_choice() {
        let mut w = window(Step::Privacy);
        w.choices.location = true;
        let items = w.items();
        let location = items.iter().find(|i| i.act == Act::Switch(Switch::Location)).unwrap();
        let analytics = items.iter().find(|i| i.act == Act::Switch(Switch::Analytics)).unwrap();
        assert!(location.on && !analytics.on);
        let mut c = window(Step::Cloud);
        c.choices.cloud = Cloud::SignIn;
        assert!(c.items().iter().any(|i| i.act == Act::Cloud(Cloud::SignIn) && i.on));
    }

    #[test]
    fn keyboard_chips_cover_the_install_layouts() {
        let items = window(Step::Keyboard).items();
        assert_eq!(items.iter().filter(|i| matches!(i.act, Act::Layout(_))).count(), LANGUAGE_LAYOUTS.len());
        assert_eq!(items.iter().filter(|i| i.on).count(), 1, "the default layout is chosen");
    }

    #[test]
    fn controls_stay_on_the_window_at_a_narrow_width() {
        let mut w = window(Step::Appearance);
        w.size = (420.0, 780.0);
        for i in w.items() {
            assert!(i.r.x >= 0.0 && i.r.x + i.r.w <= w.size.0 + 0.5, "{:?}", i.label);
        }
    }
}
