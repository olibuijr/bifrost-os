//! Remember the input source for each app (Settings > Keyboard > Keyboards, "Remember Input Source for Each App"). The
//! shell only knows apps, not their documents, so the choice is kept per app: when an app comes to the front it gets
//! back the source it had when it was last in front, and the source it leaves is kept. The desktop, the Switcher and
//! the lock screen are no app, so they keep the source in use and remember nothing.

use super::kb_layouts::{self, Layout};
use super::{AppId, Screen, Shell};
use std::collections::HashMap;

#[derive(Default)]
pub struct InputMemory {
    by_app: HashMap<AppId, Layout>,
    /// the app in front, `None` when no app is
    front: Option<AppId>,
}

impl InputMemory {
    /// The app in front is now `app` while `current` is the source in use. Returns the source the coming app gets back,
    /// when it has one and the option is on. With the option off nothing is kept.
    pub fn focus(&mut self, app: Option<AppId>, current: Layout, remember: bool) -> Option<Layout> {
        if app == self.front {
            return None;
        }
        let left = self.front.take();
        self.front = app;
        if !remember {
            self.by_app.clear();
            return None;
        }
        if let Some(left) = left {
            self.by_app.insert(left, current);
        }
        app.and_then(|a| self.by_app.get(&a).copied())
    }
}

impl Shell {
    /// Called every tick: a change of the app in front swaps the input source to the one that app remembers.
    pub(super) fn input_memory_tick(&mut self) {
        let app = match self.screen {
            Screen::App(a) => Some(a),
            _ => None,
        };
        if let Some(l) = self.input_memory.focus(app, kb_layouts::active(), kb_layouts::remembers()) {
            if l != kb_layouts::active() {
                kb_layouts::set_active(l);
                self.dirty = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_gets_back_the_source_it_left() {
        let mut m = InputMemory::default();
        assert_eq!(m.focus(Some(AppId::Terminal), Layout::Icelandic, true), None, "nothing remembered yet");
        assert_eq!(m.focus(Some(AppId::Files), Layout::Icelandic, true), None, "Files has none, the source stays");
        // Terminal was left on Icelandic and Files on English (switched while Files was in front)
        assert_eq!(m.focus(Some(AppId::Terminal), Layout::English, true), Some(Layout::Icelandic));
        assert_eq!(m.focus(Some(AppId::Files), Layout::Icelandic, true), Some(Layout::English));
    }

    #[test]
    fn the_desktop_and_the_switcher_keep_the_source_in_use_and_remember_nothing() {
        let mut m = InputMemory::default();
        m.focus(Some(AppId::Terminal), Layout::English, true);
        assert_eq!(m.focus(None, Layout::English, true), None, "leaving Terminal for the desktop restores nothing");
        assert_eq!(m.focus(None, Layout::Japanese, true), None, "the desktop itself is not remembered");
        assert_eq!(m.focus(Some(AppId::Terminal), Layout::Japanese, true), Some(Layout::English), "Terminal kept the source it left");
    }

    #[test]
    fn with_the_option_off_nothing_is_restored_and_nothing_is_kept() {
        let mut m = InputMemory::default();
        m.focus(Some(AppId::Terminal), Layout::English, false);
        assert_eq!(m.focus(Some(AppId::Files), Layout::Icelandic, false), None);
        assert_eq!(m.focus(Some(AppId::Terminal), Layout::Icelandic, false), None);
        // turned on later: nothing from the time it was off is restored
        assert_eq!(m.focus(Some(AppId::Files), Layout::Icelandic, true), None);
    }

    #[test]
    fn the_same_app_in_front_changes_nothing() {
        let mut m = InputMemory::default();
        m.focus(Some(AppId::Terminal), Layout::English, true);
        assert_eq!(m.focus(Some(AppId::Terminal), Layout::Japanese, true), None);
    }
}
