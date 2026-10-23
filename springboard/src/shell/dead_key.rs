//! A hardware dead key shown while it waits. The keymap composes a dead key with the next letter (Icelandic ´ then a
//! types á, `installer::keymaps::Keyboard::press`); meanwhile the field being typed in shows the mark as marked text,
//! underlined, as the reference does (`setMarkedText:` with "´"). The next key takes the mark back first
//! (`UnmarkText { commit: false }`) and then types what it composed. A click, a focus, window or app change, a layout
//! switch, a shortcut, Esc and Backspace cancel the mark the same way, so it is never committed by a later click.
//!
//! The fields that show it: the key window of the SDK app in front (`WindowInput::SetMarkedText` with its window id), a
//! Wayland client's field (the input method's preedit) and the shell's Search field (its `TextField` marks text).
//! Shell fields without marked text (Settings search, the sheets) keep composing without showing the mark.
//! `bin/shell.rs` holds which target shows the mark (`DeadTarget`) next to the keyboard state.

use super::keyboard::Composition;
use super::{textinput, AppId, Screen, Shell};
use bifrost_proto::WindowInput;

/// Where a dead key's mark is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeadTarget {
    /// the shell's Search field
    Search,
    /// the key window `window` (0 is the main window) of the SDK app in slot `slot`
    App { slot: u8, window: u32 },
    /// the focused field of a Wayland client, through the input method
    Wayland,
}

impl Shell {
    /// The field the next typed key goes to, when it can show marked text. Mirrors `hw_type`: Search over whatever is
    /// behind it unless a sheet holds the keyboard; otherwise the app in front, when no shell layer takes the typing.
    pub fn dead_key_target(&mut self) -> Option<DeadTarget> {
        if self.quicksearch.is_shown() && !self.assistant.is_shown() {
            let sheet = self.auth_showing()
                || self.volume_showing()
                || self.package_showing()
                || self.net_auth_showing()
                || self.access_showing()
                || self.usb_showing()
                || self.fw_showing()
                || self.card_alert_up()
                || self.power_menu.is_some()
                || self.pick_sheet.is_some()
                || self.photo_pick_sheet.is_some()
                || self.trust_sheet.is_some()
                || self.a11y_panel.is_some()
                || self.overview.grid.visible()
                || self.compose_showing()
                || self.modal_layer.is_some();
            return (!sheet).then_some(DeadTarget::Search);
        }
        if self.shell_takes_typing() || self.help_viewer.is_some() || self.modal_layer.is_some() {
            return None;
        }
        if !matches!(self.screen, Screen::App(AppId::Terminal | AppId::External(_))) {
            return None;
        }
        if self.im.active {
            return Some(DeadTarget::Wayland);
        }
        if !matches!(self.screen, Screen::App(AppId::External(_))) {
            return None;
        }
        self.sync_key_windows();
        let slot = self.ext_fg?;
        let window = self.ext.get(slot as usize)?.as_ref()?.key_window;
        Some(DeadTarget::App { slot, window })
    }

    /// Shows a pending dead key's `mark` as marked text in `target`. False when it cannot be shown there: the Search
    /// field already composes something of its own, or the Wayland field lost focus.
    pub fn dead_key_show(&mut self, target: DeadTarget, mark: char) -> bool {
        let text = mark.to_string();
        let end = text.len() as u32;
        match target {
            DeadTarget::Search => {
                if self.compose_ime.is_open() || self.quicksearch.field_mut().marked_range().is_some() {
                    return false;
                }
                self.quicksearch.field_mut().set_marked(&self.ui, &text, text.len());
            }
            DeadTarget::Wayland => {
                if !self.im.active {
                    return false;
                }
                let out = textinput::composition(&mut self.im, &Composition::Preedit { text, sel_start: end, sel_len: 0 });
                self.im_out.extend(out);
            }
            DeadTarget::App { slot, window } => {
                let Some(Some(h)) = self.ext.get_mut(slot as usize) else { return false };
                h.input_to(window, WindowInput::SetMarkedText { text, sel_start: end, sel_len: 0 });
            }
        }
        self.dirty = true;
        true
    }

    /// Takes a dead key's mark back from `target` without typing it (`UnmarkText { commit: false }`).
    pub fn dead_key_clear(&mut self, target: DeadTarget) {
        match target {
            DeadTarget::Search => self.quicksearch.field_mut().unmark(&self.ui, false),
            DeadTarget::Wayland if self.im.active => {
                let out = textinput::composition(&mut self.im, &Composition::Unmark { commit: false });
                self.im_out.extend(out);
            }
            DeadTarget::Wayland => self.im.preedit.clear(),
            DeadTarget::App { slot, window } => {
                if let Some(Some(h)) = self.ext.get_mut(slot as usize) {
                    h.input_to(window, WindowInput::UnmarkText { commit: false });
                }
            }
        }
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::keyboard::Out;

    fn search_shell() -> Shell {
        let mut sh = crate::shell::tests::shell();
        crate::shell::tests::unlock(&mut sh);
        sh.open_quicksearch_hw(0.0);
        let h = sh.h as i32;
        sh.quicksearch.tick(&sh.ui, h, 2000.0);
        assert!(sh.quicksearch.is_shown());
        sh
    }

    #[test]
    fn the_search_field_shows_the_mark_underlined_and_the_composed_letter_replaces_it() {
        let mut sh = search_shell();
        assert_eq!(sh.dead_key_target(), Some(DeadTarget::Search));
        assert!(sh.dead_key_show(DeadTarget::Search, '´'));
        assert_eq!(sh.quicksearch.field_mut().text(), "´");
        assert_eq!(sh.quicksearch.field_mut().marked_range(), Some((0, 1)), "the mark is marked text (drawn underlined)");
        assert_eq!(sh.dead_key_target(), Some(DeadTarget::Search), "showing the mark does not move the target");
        // the next key: the mark goes, then the composed letter is typed
        sh.dead_key_clear(DeadTarget::Search);
        assert_eq!(sh.quicksearch.field_mut().text(), "");
        sh.compose_search_key(Out::Text("á".into()), 0.0);
        assert!(sh.quicksearch.field_mut().text().starts_with('á'), "{:?}", sh.quicksearch.field_mut().text());
        assert!(!sh.quicksearch.field_mut().text().contains('´'));
        assert_eq!(sh.quicksearch.field_mut().marked_range(), None);
    }

    #[test]
    fn a_cancelled_mark_leaves_the_field_as_it_was() {
        let mut sh = search_shell();
        assert!(sh.dead_key_show(DeadTarget::Search, '¨'));
        sh.dead_key_clear(DeadTarget::Search);
        assert_eq!(sh.quicksearch.field_mut().text(), "", "never committed");
        assert_eq!(sh.quicksearch.field_mut().marked_range(), None);
    }

    #[test]
    fn search_composing_its_own_text_does_not_take_the_mark() {
        let mut sh = search_shell();
        sh.compose_search_key(Out::Text("'".into()), 0.0);
        sh.compose_search_key(Out::Text("e".into()), 0.0);
        assert!(sh.compose_ime.is_open());
        assert!(!sh.dead_key_show(DeadTarget::Search, '´'));
        assert_eq!(sh.quicksearch.field_mut().text(), "e");
    }

    #[test]
    fn no_target_without_a_field_that_marks_text() {
        let mut sh = crate::shell::tests::shell();
        crate::shell::tests::unlock(&mut sh);
        assert_eq!(sh.dead_key_target(), None, "the home screen has no field");
    }

    #[test]
    fn a_wayland_field_gets_the_mark_as_preedit_and_loses_it_without_a_commit() {
        let mut sh = crate::shell::tests::shell();
        sh.im.active = true;
        assert!(sh.dead_key_show(DeadTarget::Wayland, '´'));
        assert_eq!(sh.im.preedit, "´");
        sh.dead_key_clear(DeadTarget::Wayland);
        assert_eq!(sh.im.preedit, "");
        assert!(!sh.im_out.iter().any(|o| matches!(o, textinput::ImOut::Commit(_))), "the mark is never committed");
        sh.im.active = false;
        assert!(!sh.dead_key_show(DeadTarget::Wayland, '´'));
    }
}
