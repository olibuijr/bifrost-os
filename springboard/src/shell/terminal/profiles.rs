//! Profiles: the colour theme and text size (Term Basic, Pro, Homebrew ... seam).
use super::profile_table::PROFILES;
use super::*;
use termcore::palette::{Rgb, Theme};

/// The colours of profile `i` as a theme: its text and background over the dark or the light set of ANSI colours,
/// by how light the background is. The profile's opacity is not drawn (the grid is opaque). An index past the profiles
/// is the user's own `custom` colours.
pub fn profile_theme(i: usize, custom: Option<(Rgb, Rgb)>) -> Theme {
    let (name, fg, bg) = match PROFILES.get(i) {
        Some(p) => (p.name, p.fg, p.bg),
        None => {
            let (fg, bg) = custom.unwrap_or((THEMES[0].fg, THEMES[0].bg));
            ("Custom", fg, bg)
        }
    };
    let light = (299 * u32::from(bg.0) + 587 * u32::from(bg.1) + 114 * u32::from(bg.2)) / 1000 >= 128;
    Theme { name, fg, bg, ansi: THEMES[usize::from(light)].ansi }
}

/// The step after the current colours in the Color theme cycle: the `themes` themes in order, then the profiles after
/// Basic (which is the pair of themes that follow the system scheme), then the first theme again.
pub fn next_scheme(theme: usize, profile: Option<usize>, themes: usize, profiles: usize) -> (usize, Option<usize>) {
    match profile {
        None if theme + 1 < themes => (theme + 1, None),
        None => (theme, Some(1)),
        Some(p) if p + 1 < profiles => (theme, Some(p + 1)),
        Some(_) => (0, None),
    }
}

/// The name shown for the current colours: the profile's, else the theme's.
pub fn scheme_name(theme: usize, profile: Option<usize>) -> &'static str {
    match profile {
        Some(p) => PROFILES.get(p).map_or("Custom", |profile| profile.name),
        None => THEMES[theme.min(THEMES.len() - 1)].name,
    }
}

/// The pref that holds the profile a Terminal starts with: a profile number, or empty for the numbered theme.
pub const DEFAULT_KEY: &str = "terminal.default_profile";

/// The profile a Terminal starts with. The default the user set wins (empty: the numbered theme, no profile), else the
/// profile in use when the Terminal last ran.
pub fn startup_profile(default: Option<&str>, saved: Option<usize>, known: usize) -> Option<usize> {
    match default {
        Some("") => None,
        Some(text) => text.trim().parse::<usize>().ok().filter(|p| (1..known).contains(p)).or(saved),
        None => saved,
    }
}

/// One row of the Profiles panel: a numbered theme (Basic) or a profile after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Theme(usize),
    Profile(usize),
}

/// The rows of the Profiles panel: the themes, then the profiles after Basic, in the order of the Color theme cycle.
/// `profiles` counts every profile, the user's own colours included.
pub fn choices(themes: usize, profiles: usize) -> Vec<Choice> {
    (0..themes).map(Choice::Theme).chain((1..profiles).map(Choice::Profile)).collect()
}

/// The name of a row: the theme's, the profile's, or Custom past the profiles.
pub fn choice_name(choice: Choice) -> &'static str {
    match choice {
        Choice::Theme(t) => THEMES[t.min(THEMES.len() - 1)].name,
        Choice::Profile(p) => PROFILES.get(p).map_or("Custom", |profile| profile.name),
    }
}

/// Whether a row is the one in use: a theme row when no profile is chosen, a profile row when it is.
pub fn is_current(choice: Choice, theme: usize, profile: Option<usize>) -> bool {
    match choice {
        Choice::Theme(t) => profile.is_none() && t == theme,
        Choice::Profile(p) => profile == Some(p),
    }
}

impl Shell {
    /// The colours the tabs draw with when a profile is chosen; None draws the numbered theme.
    pub(crate) fn terminal_scheme(&self) -> Option<Theme> {
        let custom = self.terminal_ui.custom_colors;
        self.terminal_ui.profile.map(|p| profile_theme(p, custom))
    }

    /// The text and background colours the tabs draw with now.
    pub(crate) fn terminal_colors(&self) -> (Rgb, Rgb) {
        match self.terminal_scheme() {
            Some(theme) => (theme.fg, theme.bg),
            None => {
                let base = &THEMES[self.terminal_ui.settings.theme.min(THEMES.len() - 1)];
                (base.fg, base.bg)
            }
        }
    }

    /// The pixel behind and around the grid, for the strips the canvas does not cover.
    pub(crate) fn terminal_background(&self) -> u32 {
        match self.terminal_ui.profile {
            Some(p) => crate::term::color_pixel(profile_theme(p, self.terminal_ui.custom_colors).bg),
            None => crate::term::theme_background(self.terminal_ui.settings.theme),
        }
    }

    /// Terminal's default profile follows the system scheme (Golden Gate "Basic": black on white in Light, white on black in Dark)
    /// until the user picks a theme, which is then kept in `terminal.theme`.
    pub(crate) fn terminal_follow_scheme(&mut self) {
        if self.prefs.get("terminal.theme").is_some() || self.terminal_ui.profile.is_some() {
            return;
        }
        let want = usize::from(bifrost_render::appearance::target_is_light());
        if self.terminal_ui.settings.theme == want {
            return;
        }
        self.terminal_ui.settings.theme = want;
        let settings = self.terminal_ui.settings;
        for term in self.term.iter_mut() {
            term.appearance(want, self.ui.pt(termcore::zoom::SIZES[settings.zoom] as f32));
        }
        self.dirty = true;
    }
    /// Turns Secure Keyboard Entry on or off and saves it. While on, the shell's shortcuts (switcher, search,
    /// Home, Back, window keys) stay out of the Terminal's keys (see `hwkeys`).
    pub(super) fn terminal_toggle_secure(&mut self) {
        self.terminal_ui.secure = !self.terminal_ui.secure;
        let secure = self.terminal_ui.secure;
        self.terminal_ui.message = match self.prefs.set("terminal.secure_input", if secure { "1" } else { "0" }) {
            Ok(()) if secure => "Secure keyboard entry on: shortcuts stay in the terminal".into(),
            Ok(()) => "Secure keyboard entry off".into(),
            Err(e) => format!("Could not save keyboard entry: {e}"),
        };
        self.dirty = true;
    }

    /// Turns mouse reporting to programs on or off for every tab and saves it.
    pub(super) fn terminal_toggle_mouse(&mut self) {
        self.terminal_ui.mouse_reporting = !self.terminal_ui.mouse_reporting;
        let on = self.terminal_ui.mouse_reporting;
        for term in self.term.iter_mut() {
            term.set_mouse_reporting(on);
        }
        self.terminal_ui.message = match self.prefs.set("terminal.mouse", if on { "1" } else { "0" }) {
            Ok(()) if on => "Programs can receive mouse clicks".into(),
            Ok(()) => "Mouse clicks stay with the terminal".into(),
            Err(e) => format!("Could not save mouse reporting: {e}"),
        };
        self.dirty = true;
    }

    /// Applies the profile caret (style and blink) to every tab and saves it.
    pub(super) fn terminal_cursor_changed(&mut self) {
        let (shape, blink) = (self.terminal_ui.cursor.shape(), self.terminal_ui.blink);
        for term in self.term.iter_mut() {
            term.set_cursor(Some(shape), blink);
        }
        let style_saved = self.prefs.set("terminal.cursor", self.terminal_ui.cursor.key());
        let saved = style_saved.and_then(|_| self.prefs.set("terminal.blink", if blink { "1" } else { "0" }));
        if let Err(e) = saved {
            self.terminal_ui.message = format!("Could not save cursor: {e}");
        }
        self.dirty = true;
    }
    pub(super) fn terminal_appearance(&mut self) {
        let settings = self.terminal_ui.settings;
        let scheme = self.terminal_ui.profile;
        let custom = self.terminal_ui.custom_colors;
        for term in self.term.iter_mut() {
            term.appearance_with(
                settings.theme,
                scheme.map(|p| profile_theme(p, custom)),
                self.ui.pt(termcore::zoom::SIZES[settings.zoom] as f32),
            );
        }
        self.term_touch = termtouch::TermTouch::default();
        let follows = self.prefs.get("terminal.theme").is_none()
            && settings.theme == usize::from(bifrost_render::appearance::target_is_light());
        let theme_saved = if follows { Ok(()) } else { self.prefs.set("terminal.theme", &settings.theme.to_string()) };
        let profile_text = scheme.map(|p| p.to_string()).unwrap_or_default();
        if let Err(e) = theme_saved
            .and_then(|_| self.prefs.set("terminal.profile", &profile_text))
            .and_then(|_| self.prefs.set("terminal.zoom", &settings.zoom.to_string()))
        {
            self.terminal_ui.message = format!("Could not save appearance: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cycle_walks_the_themes_then_the_profiles_after_basic_and_wraps() {
        let (themes, profiles) = (3, 9);
        let mut at = (0, None);
        let mut seen = vec![at];
        for _ in 0..(themes - 1 + profiles - 1) {
            at = next_scheme(at.0, at.1, themes, profiles);
            seen.push(at);
        }
        assert_eq!(&seen[1..4], &[(1, None), (2, None), (2, Some(1))]);
        assert_eq!(seen.last(), Some(&(2, Some(8))));
        assert_eq!(next_scheme(2, Some(8), themes, profiles), (0, None), "back to the first theme");
    }

    #[test]
    fn the_default_profile_is_the_one_set_or_else_the_one_in_use() {
        assert_eq!(startup_profile(None, Some(3), 9), Some(3), "no default: the saved profile");
        assert_eq!(startup_profile(Some(""), Some(3), 9), None, "the default is the numbered theme");
        assert_eq!(startup_profile(Some("5"), Some(3), 9), Some(5));
        assert_eq!(startup_profile(Some("0"), Some(3), 9), Some(3), "Basic is a theme, not a profile: the saved one stays");
        assert_eq!(startup_profile(Some("12"), None, 9), None, "a profile that is gone");
        assert_eq!(startup_profile(Some("junk"), Some(2), 9), Some(2));
    }

    #[test]
    fn the_profiles_panel_lists_each_theme_then_each_profile_once() {
        let rows = choices(3, 9);
        assert_eq!(rows.len(), 3 + 8);
        assert_eq!(rows[0], Choice::Theme(0));
        assert_eq!(rows[3], Choice::Profile(1));
        assert_eq!(rows.last(), Some(&Choice::Profile(8)));
        assert_eq!(choice_name(Choice::Theme(2)), "Solarized Dark");
        assert_eq!(choice_name(Choice::Profile(9)), "Custom", "past the profiles is the user's own");
        assert!(is_current(Choice::Theme(1), 1, None));
        assert!(!is_current(Choice::Theme(1), 1, Some(4)), "a profile in use is not its theme");
        assert!(is_current(Choice::Profile(4), 0, Some(4)));
    }

    #[test]
    fn a_profile_brings_its_colours_over_the_ansi_set_that_suits_its_background() {
        let pro = profile_theme(1, None);
        assert_eq!((pro.name, pro.fg, pro.bg), ("Pro", (242, 242, 242), (0, 0, 0)));
        assert_eq!(pro.ansi, THEMES[0].ansi, "dark background, dark set");
        assert_eq!(profile_theme(4, None).ansi, THEMES[1].ansi, "a light background gets the light set");
        let own = profile_theme(PROFILES.len(), Some(((250, 250, 250), (20, 20, 20))));
        assert_eq!((own.name, own.fg, own.bg), ("Custom", (250, 250, 250), (20, 20, 20)));
        assert_eq!(own.ansi, THEMES[0].ansi);
        assert_eq!(profile_theme(PROFILES.len(), None).bg, THEMES[0].bg, "no custom colours: the first theme's");
        assert_eq!(scheme_name(2, None), "Solarized Dark");
        assert_eq!(scheme_name(0, Some(3)), "Ocean");
        assert_eq!(scheme_name(0, Some(PROFILES.len())), "Custom");
    }
}
