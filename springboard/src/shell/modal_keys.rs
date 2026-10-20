//! Keys for the system alerts. An alert is modal over the whole screen and, on the desktop, a surface of its own that
//! holds the keyboard (`desktop_chrome_frames`, id 15), so the keys come to the shell and must not reach the app in
//! front: Esc answers the way the alert's cancel button does, Return presses its bold button unless that would grant a permission (a key never grants), every other key is used up.
//! The alerts handle touches in `Shell::modal_event`; a key is turned into the touch on the button it stands for, so
//! there is one path for answering an alert and a key never does what a tap could not.

use super::{Effect, Event, Shell};

/// The alerts a key answers, topmost first in `Shell::modal_event`'s order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Card {
    Notify,
    Crash,
    Launch,
    Malware,
    Guardian,
    Permission,
    LowBattery,
}

impl Shell {
    /// The card alert on top, or None when none is up or another modal sits above it (that one takes its own keys).
    pub(super) fn top_card_alert(&self) -> Option<Card> {
        // in `Shell::modal_event`'s order: these come before the notification alert and read their keys elsewhere (`hw_type`)
        let above_notify = self.auth_showing()
            || self.volume_showing()
            || self.access_showing()
            || self.package_showing()
            || self.net_auth_showing()
            || self.usb_showing()
            || self.fw_showing()
            || self.share.is_some()
            || self.share_prompt.is_some();
        if above_notify {
            return None;
        }
        if self.notify.ask.is_some() {
            return Some(Card::Notify);
        }
        // these come after the notification alert and before the other cards
        let above_cards = self.compose_showing()
            || self.power_menu.is_some()
            || self.keyboard_viewer.is_some()
            || self.help_viewer.is_some()
            || self.char_viewer.is_some()
            || self.a11y_panel.is_some()
            || self.print_showing();
        if above_cards {
            return None;
        }
        if self.crash_alert.is_some() {
            Some(Card::Crash)
        } else if self.launch_alert.is_some() {
            Some(Card::Launch)
        } else if self.pick_sheet.is_some() {
            None
        } else if self.malware_alert.is_some() {
            Some(Card::Malware)
        } else if self.photo_pick_sheet.is_some() || self.trust_sheet.is_some() {
            None
        } else if self.guardian_sheet.is_some() {
            Some(Card::Guardian)
        } else if self.permission_sheet.is_some() {
            Some(Card::Permission)
        } else if self.low_alert.is_some() {
            Some(Card::LowBattery)
        } else {
            None
        }
    }

    /// True while a card alert is up that takes the keys (the app behind it gets none).
    pub(super) fn card_alert_up(&self) -> bool {
        self.top_card_alert().is_some()
    }

    /// The centre of the button a key stands for on the card alert on top: Esc is its cancel button (`cancel`), Return its
    /// bold one. None for an alert no key answers (the guardian's sheet has no default).
    pub(super) fn card_alert_key_point(&self, cancel: bool) -> Option<(f32, f32)> {
        let (ui, w, h) = (&self.ui, self.w, self.h);
        match self.top_card_alert()? {
            Card::Notify => self.notify.ask.as_ref()?.key_button(ui, w, h, cancel),
            Card::Crash => self.crash_alert.as_ref()?.key_button(ui, w, h, cancel),
            Card::Launch => self.launch_alert.as_ref()?.key_button(ui, w, h, cancel),
            Card::Malware => self.malware_alert.as_ref()?.key_button(ui, w, h, cancel),
            Card::Permission => self.permission_sheet.as_ref()?.key_button(ui, w, h, cancel),
            Card::LowBattery => self.low_alert.as_ref()?.key_button(ui, w, h, cancel),
            Card::Guardian => None,
        }
    }

    /// True once, after a key answered a card alert: the key loop then ends the repeat of the held key.
    pub fn take_key_repeat_cancel(&mut self) -> bool {
        std::mem::take(&mut self.key_repeat_cancel)
    }

    /// Esc (`cancel`) or Return answers the card alert on top as a tap on its button would; Return answers only an alert
    /// whose bold button a key may press (`key_button`: never a permission). Always consumes the key.
    pub(super) fn card_alert_key(&mut self, cancel: bool, t: f64) -> Vec<Effect> {
        let Some((x, y)) = self.card_alert_key_point(cancel) else { return vec![] };
        self.last_input = t;
        self.key_repeat_cancel = true;
        let mut fx = self.modal_event(Event::Down(x, y)).unwrap_or_default();
        fx.extend(self.modal_event(Event::Up(x, y)).unwrap_or_default());
        self.dirty = true;
        fx
    }
}

#[cfg(test)]
mod tests {
    use crate::shell::hwkeys::Shortcut;
    use crate::shell::keyboard::Out;
    use crate::shell::notify::permission::Sheet;
    use crate::shell::{AppId, Screen, Shell};

    /// A desktop shell with the first-time notification alert up, as Mail's request showed it.
    fn shell_with_alert() -> Shell {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        sh.notify.ask = Sheet::new(&sh.ui, AppId::Files, "Mail", 0);
        assert!(sh.notify.ask.is_some());
        sh
    }

    #[test]
    fn escape_answers_the_alert_with_dont_allow_and_return_never_grants() {
        let mut sh = shell_with_alert();
        assert!(sh.card_alert_up() && sh.shell_takes_typing());
        sh.hw_shortcut(Shortcut::Back, 1.0);
        assert!(sh.notify.ask.is_none(), "Escape answers the alert");
        assert!(sh.notify.store().is_asked(AppId::Files) && sh.notify.store().is_muted(AppId::Files), "as Don't Allow");
        assert!(!sh.card_alert_up());
        assert!(sh.take_key_repeat_cancel(), "a held key stops repeating into the app that comes back");
        assert!(!sh.take_key_repeat_cancel(), "once");

        // a Return meant for the app behind must not give the permission: it is used up and the alert stays
        let mut sh = shell_with_alert();
        assert!(sh.hw_type(Out::Enter, 1.0).is_empty());
        assert!(sh.notify.ask.is_some(), "Return does not answer the alert");
        assert!(!sh.notify.store().is_asked(AppId::Files) && !sh.take_key_repeat_cancel());
    }

    #[test]
    fn a_terminal_chord_and_paste_are_used_up_and_escape_answers_under_secure_entry() {
        use crate::shell::hwkeys::TermKey;
        let mut sh = shell_with_alert();
        sh.screen = Screen::App(AppId::Terminal);
        sh.terminal_ui.secure = true;
        assert!(sh.hw_shortcut(Shortcut::Term(TermKey::Copy), 1.0).is_empty());
        assert!(sh.hw_shortcut(Shortcut::Paste, 1.0).is_empty());
        assert!(sh.notify.ask.is_some(), "neither answers the alert");
        sh.hw_shortcut(Shortcut::Back, 2.0);
        assert!(sh.notify.ask.is_none(), "Escape reaches the alert although the Terminal has Secure Keyboard Entry on");
    }

    #[test]
    fn the_notification_alert_is_on_top_of_the_other_modals_as_in_modal_event() {
        let mut sh = shell_with_alert();
        sh.power_menu = Some(crate::shell::power_menu::PowerMenu::new(&sh.ui));
        assert_eq!(sh.top_card_alert(), Some(super::Card::Notify), "modal_event asks the notification alert before the power menu");
    }

    #[test]
    fn every_other_key_is_used_up_while_the_alert_is_up() {
        let mut sh = shell_with_alert();
        for out in [Out::Text("a".into()), Out::Named("Tab"), Out::Backspace, Out::Named("Escape")] {
            assert!(sh.hw_type(out, 1.0).is_empty());
            assert!(sh.notify.ask.is_some(), "the alert waits for Escape or Return");
        }
        assert!(!sh.hw_chord(30, bifrost_proto::CHORD_META), "a chord does not reach the app behind the alert");
        assert!(!sh.hw_key(30, true, 1.0));
        assert!(!sh.notify.store().is_asked(AppId::Files));
    }

    /// NP1: the notification alert is a surface above the windows with the keyboard, listed only while it is up.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn the_notification_alert_is_an_overlay_above_the_windows_only_while_it_is_up() {
        use crate::gfx::Surface;
        use crate::shell::desktop_bar::MODAL_CHROME_ID;
        let _lock = crate::test_support::global_lock();
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut px = vec![0u32; w * h];
        sh.dirty = true;
        sh.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 500.0);
        assert!(sh.desktop_chrome_frames(&px, w, h).iter().all(|f| f.id != MODAL_CHROME_ID), "no alert, no overlay");
        sh.notify.ask = Sheet::new(&sh.ui, AppId::Files, "Mail", 0);
        sh.dirty = true;
        sh.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 1000.0);
        let frames = sh.desktop_chrome_frames(&px, w, h);
        let f = frames.iter().find(|f| f.id == MODAL_CHROME_ID).expect("the alert is a surface above the windows");
        assert_eq!((f.x, f.y, f.width, f.height), (0, 0, w, h), "it spans the screen, veil included, so it is modal over the apps");
        assert_eq!(f.input, None, "it takes the pointer over its whole surface: a click outside the card goes nowhere");
        let (bx, by) = sh.notify.ask.as_ref().unwrap().key_button(&sh.ui, sh.w, sh.h, true).unwrap();
        assert!(bx >= 0.0 && by >= 0.0 && (bx as usize) < f.width && (by as usize) < f.height, "the card is inside the surface");
        assert!(f.pixels[0] >> 24 != 0, "the veil covers the corner");
        assert!(f.pixels[by as usize * w + bx as usize] != f.pixels[0], "and the card is on it");
        // the wheel over the veil is used up, not given to the app under it
        assert!(sh.pointer_wheel(100.0, 100.0, 0.0, 5.0, 1.0));
        // a click on the card's button reaches the existing hit test (screen coordinates) and answers it
        sh.event(crate::shell::Event::Down(bx, by), 1.1);
        sh.event(crate::shell::Event::Up(bx, by), 1.2);
        assert!(sh.notify.ask.is_none());
        sh.dirty = true;
        sh.draw(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 2000.0);
        assert!(sh.desktop_chrome_frames(&px, w, h).iter().all(|f| f.id != MODAL_CHROME_ID), "answered: the overlay is dropped");
    }
}
