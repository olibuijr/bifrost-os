//! The shell's Search field runs the shared compose method (`bifrost_ui::compose_method`): a typed apostrophe and then a
//! letter open the candidate list over the field, and the letter shows as marked text. Digits 1 to 9 pick a candidate and
//! commit it; any other key ends the composition and keeps the letter. With the Japanese (Romaji) source on, the same
//! field composes kana (`bifrost_ui::kana_method`): romaji becomes marked hiragana as it is typed, Space opens the
//! hiragana and katakana list, and a digit picks from it. A held letter opens its accent list (`accent_hold`). The window
//! is painted last in `Shell::draw`, above the windows. Only the painting and the mapping onto the Search field are here:
//! the state and the typing flow are the ones every SDK text field uses.
use super::kb_layouts::{self, Layout};
use super::keyboard::Out;
use super::ui::{Ui, Weight, LABEL, MAC_BODY_PT, MENU_HIGHLIGHT, MENU_HIGHLIGHT_TEXT, MENU_SOLID, WHITE};
use super::{Effect, Shell};
use crate::gfx::Surface;
use bifrost_proto::ToApp;
use bifrost_ui::compose_method::{self, ComposeTarget, Composer};
use bifrost_ui::imk_host::KeyResult;
use bifrost_ui::prefs_layout::m;

/// Row pitch, padding and width of the candidate window, in points.
const ROW_PT: f32 = 24.0;
const PAD_PT: f32 = 6.0;
const WIDTH_PT: f32 = 180.0;
/// Top of the window as a share of the screen height (the Search card sits at the top).
const TOP_SHARE: f32 = 0.3;

/// The Search field's compose state: the shared composer (the armed apostrophe, the kana and the open list).
#[derive(Default)]
pub struct ComposeWindow {
    composer: Composer,
}

impl ComposeWindow {
    /// True while a candidate list is open over the Search field.
    pub fn is_open(&self) -> bool {
        self.composer.is_open()
    }

    /// The candidates on screen (empty when no list is open).
    pub fn candidates(&self) -> &[String] {
        self.composer.candidates()
    }
}

/// The Search field as the compose method's target: its marked text, and plain typing through the search, so a typed
/// letter filters the results like any other key.
struct SearchField<'a> {
    sh: &'a mut Shell,
    t: f64,
    fx: Vec<Effect>,
}

impl ComposeTarget for SearchField<'_> {
    fn set_marked(&mut self, text: &str) {
        self.sh.quicksearch.field_mut().set_marked(&self.sh.ui, text, text.len());
        self.sh.dirty = true;
    }

    fn commit_marked(&mut self) {
        self.sh.quicksearch.field_mut().unmark(&self.sh.ui, true);
        self.sh.dirty = true;
    }

    fn insert(&mut self, text: &str) {
        let fx = self.sh.quicksearch_key(Out::Text(text.to_string()), self.t);
        self.fx.extend(fx);
    }

    fn discard_marked(&mut self) {
        self.sh.quicksearch.field_mut().unmark(&self.sh.ui, false);
        self.sh.dirty = true;
    }

    fn delete_back(&mut self) {
        let fx = self.sh.quicksearch_key(Out::Backspace, self.t);
        self.fx.extend(fx);
    }
}

impl Shell {
    /// Keys typed while the Search field is open go through the compose method first.
    pub(super) fn compose_search_key(&mut self, out: Out, t: f64) -> Vec<Effect> {
        // an installed input method that is the selected source takes the key first
        let routed = self.input_providers.search_key(&out, &self.ui, self.quicksearch.field_mut());
        if routed == KeyResult::Consumed {
            self.dirty = true;
            return vec![];
        }
        let japanese = kb_layouts::active() == Layout::Japanese;
        let mut composer = std::mem::take(&mut self.compose_ime.composer);
        let fx = {
            let mut field = SearchField { sh: &mut *self, t, fx: Vec::new() };
            match out {
                Out::Text(text) => compose_method::type_text_as(&mut composer, &mut field, &text, japanese),
                other => {
                    // Backspace removes one letter of an open kana composition; any other control key ends the
                    // composition (the letter stays) and gives back a held apostrophe
                    let kept = matches!(other, Out::Backspace) && compose_method::backspace(&mut composer, &mut field, japanese);
                    if !kept {
                        compose_method::end_for_control(&mut composer, &mut field);
                        let more = field.sh.quicksearch_key(other, t);
                        field.fx.extend(more);
                    }
                }
            }
            field.fx
        };
        self.compose_ime.composer = composer;
        fx
    }

    /// Whether a held letter may open its accent list: the Search field or a focused text field of the app in front is
    /// open on a Latin source, and no list is up.
    pub fn accent_popup_ready(&self) -> bool {
        let field = self.quicksearch.is_shown() || self.ext_text_focused();
        field && kb_layouts::active() != Layout::Japanese && !self.compose_ime.is_open()
    }

    /// A letter held down (`KeyRepeat::accent_hold`): the letter typed at the press is taken back and shown as marked
    /// text over its accents, with digits to pick one. With no Search field open, the focused field of the app in front
    /// takes the hold (`ToApp::AccentHold`) and composes the accents in its own compose method.
    pub fn accent_hold(&mut self, letter: char, t: f64) -> Vec<Effect> {
        if !self.quicksearch.is_shown() {
            self.ext_fg_send(&ToApp::AccentHold { letter });
            return vec![];
        }
        let mut composer = std::mem::take(&mut self.compose_ime.composer);
        let fx = {
            let mut field = SearchField { sh: &mut *self, t, fx: Vec::new() };
            compose_method::accent_hold(&mut composer, &mut field, letter);
            field.fx
        };
        self.compose_ime.composer = composer;
        fx
    }

    /// Paints the candidate window over the Search field: a numbered list, one row per candidate.
    pub(super) fn draw_compose(&self, s: &mut Surface<'_>, ui: &Ui, w: i32, h: i32) {
        // the candidates of the selected input method, else those of the built-in compose method
        let view = self.input_providers.candidate_view();
        let (items, selected): (&[String], Option<usize>) = match &view {
            Some(v) => (v.items.as_slice(), v.selected),
            None => (self.compose_ime.candidates(), None),
        };
        if items.is_empty() {
            return;
        }
        let (row, pad, width) = (ui.pt(ROW_PT), ui.pt(PAD_PT), ui.pt(WIDTH_PT));
        let height = pad * 2.0 + row * items.len() as f32;
        let x = (w as f32 - width) / 2.0;
        let y = h as f32 * TOP_SHARE;
        s.fill_rrect(x, y, width, height, ui.pt(m::CARD_RADIUS), MENU_SOLID.now());
        for (i, item) in items.iter().enumerate() {
            if selected == Some(i) {
                s.fill_rrect(x + pad / 2.0, y + pad + row * i as f32, width - pad, row, ui.pt(m::SELECTION_RADIUS), MENU_HIGHLIGHT);
            }
            let label = format!("{}   {}", i + 1, item);
            let img = ui.text(&label, MAC_BODY_PT, Weight::Regular, WHITE);
            let ty = y + pad + row * i as f32 + (row - img.h as f32) / 2.0;
            let ink = if selected == Some(i) { MENU_HIGHLIGHT_TEXT } else { LABEL.now() };
            s.blit_colored(&img, (x + pad * 2.0) as i32, ty as i32, ink, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_apostrophe_and_a_letter_open_the_list_and_a_digit_commits_the_pick() {
        let mut sh = crate::shell::tests::shell();
        assert!(sh.compose_search_key(Out::Text("'".into()), 0.0).is_empty());
        assert!(sh.compose_search_key(Out::Text("e".into()), 0.0).is_empty());
        assert!(sh.compose_ime.is_open());
        assert_eq!(sh.compose_ime.candidates(), &["é".to_string(), "è".to_string(), "ê".to_string(), "ë".to_string()]);
        // the letter shows as marked text in the field, over the empty range at 0
        assert_eq!(sh.quicksearch.field_mut().text(), "e");
        assert_eq!(sh.quicksearch.field_mut().marked_range(), Some((0, 1)));
        // "2" picks the second candidate: it replaces the marked letter and is kept as plain text
        assert!(sh.compose_search_key(Out::Text("2".into()), 0.0).is_empty());
        assert!(!sh.compose_ime.is_open());
        assert_eq!(sh.quicksearch.field_mut().text(), "è");
        assert_eq!(sh.quicksearch.field_mut().marked_range(), None);
    }

    #[test]
    fn a_control_key_ends_the_list_and_keeps_the_letter() {
        let mut sh = crate::shell::tests::shell();
        sh.compose_search_key(Out::Text("'".into()), 0.0);
        sh.compose_search_key(Out::Text("o".into()), 0.0);
        assert!(sh.compose_ime.is_open());
        sh.compose_search_key(Out::Named("Escape"), 0.0);
        assert!(!sh.compose_ime.is_open());
        assert_eq!(sh.quicksearch.field_mut().text(), "o");
        assert_eq!(sh.quicksearch.field_mut().marked_range(), None);
    }

    #[test]
    fn an_apostrophe_before_a_plain_letter_is_not_lost() {
        let mut sh = crate::shell::tests::shell();
        // the Search card is not shown here, so inserted text is not painted, but the composition state is what
        // this checks: a held apostrophe is given back as its own key and no list opens
        sh.compose_search_key(Out::Text("'".into()), 0.0);
        sh.compose_search_key(Out::Named("Backspace"), 0.0);
        assert!(!sh.compose_ime.is_open());
        assert!(!sh.compose_ime.composer.is_armed());
    }

    #[test]
    fn a_held_letter_opens_its_accents_and_a_digit_picks_one() {
        let mut sh = crate::shell::tests::shell();
        crate::shell::tests::unlock(&mut sh);
        // a held letter only opens accents over a shown Search field (or a focused app field, see `accent_popup_ready`)
        sh.open_quicksearch_hw(0.0);
        // the reveal animates; a tick past its end shows the field
        let h = sh.h as i32;
        sh.quicksearch.tick(&sh.ui, h, 2000.0);
        assert!(sh.quicksearch.is_shown());
        assert!(sh.compose_search_key(Out::Text("e".into()), 0.0).is_empty());
        sh.accent_hold('e', 0.0);
        assert!(sh.compose_ime.is_open());
        assert_eq!(sh.quicksearch.field_mut().text(), "e");
        assert_eq!(sh.quicksearch.field_mut().marked_range(), Some((0, 1)));
        sh.compose_search_key(Out::Text("2".into()), 0.0);
        assert!(!sh.compose_ime.is_open());
        assert_eq!(sh.quicksearch.field_mut().text(), "è");
    }

    #[test]
    fn romaji_composes_kana_in_the_field_on_the_japanese_source() {
        let mut sh = crate::shell::tests::shell();
        // the source is enabled and switched to for this test only, and put back after it
        kb_layouts::toggle(Layout::Japanese);
        kb_layouts::set_active(Layout::Japanese);
        sh.compose_search_key(Out::Text("k".into()), 0.0);
        sh.compose_search_key(Out::Text("a".into()), 0.0);
        assert_eq!(sh.quicksearch.field_mut().text(), "か");
        assert_eq!(sh.quicksearch.field_mut().marked_range(), Some((0, 1)));
        // Space opens the hiragana and katakana list; digit 2 picks the katakana
        sh.compose_search_key(Out::Text(" ".into()), 0.0);
        assert!(sh.compose_ime.is_open());
        assert_eq!(sh.compose_ime.candidates(), &["か".to_string(), "カ".to_string()]);
        sh.compose_search_key(Out::Text("2".into()), 0.0);
        assert!(!sh.compose_ime.is_open());
        assert_eq!(sh.quicksearch.field_mut().text(), "カ");
        assert_eq!(sh.quicksearch.field_mut().marked_range(), None);
        kb_layouts::set_active(Layout::Icelandic);
        kb_layouts::toggle(Layout::Japanese);
    }

    #[test]
    fn an_installed_input_method_composes_kana_in_the_search_field() {
        let mut sh = crate::shell::tests::shell();
        let dir = std::env::temp_dir().join(format!("bifrost-compose-ime-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bundle = dir.join("Kana.imk");
        std::fs::create_dir_all(&bundle).unwrap();
        let info = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>org.example.kana</string>
<key>BifrostEngine</key><string>kana</string>
<key>IMKControllerClass</key><string>KanaController</string>
<key>IMKModeDictionary</key><dict><key>romaji</key><dict><key>label</key><string>Romaji</string></dict></dict>
</dict></plist>"#;
        std::fs::write(bundle.join("Info.plist"), info).unwrap();
        assert_eq!(sh.input_providers.load_dir(&dir), 1);
        let _ = std::fs::remove_dir_all(&dir);
        // with no method selected a letter is plain typing: the method does not see it
        assert_eq!(sh.input_providers.search_key(&Out::Text("k".into()), &sh.ui, sh.quicksearch.field_mut()), KeyResult::PassThrough);
        sh.input_providers.select("org.example.kana.romaji");
        sh.compose_search_key(Out::Text("k".into()), 0.0);
        sh.compose_search_key(Out::Text("a".into()), 0.0);
        assert_eq!(sh.quicksearch.field_mut().text(), "か");
        assert_eq!(sh.quicksearch.field_mut().marked_range(), Some((0, 1)));
        let view = sh.input_providers.candidate_view().expect("a candidate window is offered");
        assert_eq!(view.items, vec!["か".to_string(), "カ".to_string()]);
        // a digit picks a candidate: it replaces the marked kana and the window closes
        sh.compose_search_key(Out::Text("2".into()), 0.0);
        assert_eq!(sh.quicksearch.field_mut().text(), "カ");
        assert_eq!(sh.quicksearch.field_mut().marked_range(), None);
        assert!(sh.input_providers.candidate_view().is_none());
        // choosing a keyboard layout ends the method: a letter is typed by the field again
        sh.input_layout_selected(Layout::English);
        assert!(!sh.input_providers.is_method_active());
    }

    #[test]
    fn a_new_window_is_closed_and_has_no_candidates() {
        let c = ComposeWindow::default();
        assert!(!c.is_open());
        assert!(c.candidates().is_empty());
    }
}
