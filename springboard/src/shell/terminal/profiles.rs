//! Profiles: the colour theme and text size (Term Basic, Pro, Homebrew ... seam).
use super::*;

impl Shell {
    /// Terminal's default profile follows the system scheme (Golden Gate "Basic": black on white in Light, white on black in Dark)
    /// until the user picks a theme, which is then kept in `terminal.theme`.
    pub(crate) fn terminal_follow_scheme(&mut self) {
        if self.prefs.get("terminal.theme").is_some() {
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
    pub(super) fn terminal_appearance(&mut self) {
        let settings = self.terminal_ui.settings;
        for term in self.term.iter_mut() {
            term.appearance(
                settings.theme,
                self.ui.pt(termcore::zoom::SIZES[settings.zoom] as f32),
            );
        }
        self.term_touch = termtouch::TermTouch::default();
        let follows = self.prefs.get("terminal.theme").is_none()
            && settings.theme == usize::from(bifrost_render::appearance::target_is_light());
        let theme_saved = if follows { Ok(()) } else { self.prefs.set("terminal.theme", &settings.theme.to_string()) };
        if let Err(e) = theme_saved.and_then(|_| self.prefs.set("terminal.zoom", &settings.zoom.to_string()))
        {
            self.terminal_ui.message = format!("Could not save appearance: {e}");
        }
    }
}
