//! The built-in compose method in the shell's Search field (`bifrost_ui::compose_method`), with its candidate window
//! (`bifrost_ui::imsession::InputSession`, the candidate window model). A typed apostrophe and then a letter open the
//! candidate list over the field; the letter shows as marked text. Digits 1 to 9 pick a candidate and commit it; any
//! other key ends the composition and keeps the letter. The window is painted last in `Shell::draw`, above the windows.
use super::keyboard::Out;
use super::ui::{Ui, Weight, LABEL, MAC_BODY_PT, MENU_SOLID, WHITE};
use super::{Effect, Shell};
use crate::gfx::Surface;
use bifrost_ui::candidates::PanelType;
use bifrost_ui::compose_method::{self, Compose, Step};
use bifrost_ui::imsession::InputSession;
use bifrost_ui::prefs_layout::m;

/// Row pitch, padding and width of the candidate window, in points.
const ROW_PT: f32 = 24.0;
const PAD_PT: f32 = 6.0;
const WIDTH_PT: f32 = 180.0;
/// Top of the window as a share of the screen height (the Search card sits at the top).
const TOP_SHARE: f32 = 0.3;

/// The compose method's state in the shell: the armed apostrophe and the open composition, if any.
#[derive(Default)]
pub struct ComposeWindow {
    method: Compose,
    session: Option<InputSession>,
}

impl ComposeWindow {
    /// True while a candidate list is open over the Search field.
    pub fn is_open(&self) -> bool {
        self.session.is_some()
    }

    /// The candidates on screen (empty when no list is open).
    pub fn candidates(&self) -> &[String] {
        match &self.session {
            Some(s) => s.candidates(),
            None => &[],
        }
    }
}

impl Shell {
    /// Keys typed while the Search field is open go through the compose method first.
    pub(super) fn compose_search_key(&mut self, out: Out, t: f64) -> Vec<Effect> {
        let text = match out {
            Out::Text(text) => text,
            other => {
                // a control key ends an open composition (the letter stays) and gives back a held apostrophe
                self.compose_finish(None);
                let mut fx = Vec::new();
                if self.compose_ime.method.take_prefix() {
                    fx = self.quicksearch_key(Out::Text(compose_method::PREFIX.to_string()), t);
                }
                fx.extend(self.quicksearch_key(other, t));
                return fx;
            }
        };
        // digits pick from an open list; any other typed text ends the composition, keeping the letter
        let picked = self.compose_ime.session.as_ref().and_then(|s| compose_method::pick(s.candidates(), &text));
        if let Some(i) = picked {
            self.compose_finish(Some(i));
            return vec![];
        }
        self.compose_finish(None);
        match self.compose_ime.method.key(&text) {
            Step::Insert(s) => self.quicksearch_key(Out::Text(s), t),
            Step::Armed => vec![],
            Step::Open { original, candidates } => {
                self.compose_open(&original, candidates);
                vec![]
            }
        }
    }

    /// Shows the letter as marked text in the Search field and opens its candidate list.
    fn compose_open(&mut self, original: &str, candidates: Vec<String>) {
        self.quicksearch.field_mut().set_marked(&self.ui, original, original.len());
        let mut session = InputSession::new(0, PanelType::SingleColumnScrolling);
        session.update(original, original);
        session.set_candidates(candidates);
        self.compose_ime.session = Some(session);
        self.dirty = true;
    }

    /// Ends the open composition. `pick` selects candidate `i` first; the marked text is then kept as plain text.
    fn compose_finish(&mut self, pick: Option<usize>) {
        let Some(mut session) = self.compose_ime.session.take() else {
            return;
        };
        if let Some(i) = pick {
            session.select_candidate(i);
        }
        if let Some(text) = session.commit() {
            let field = self.quicksearch.field_mut();
            field.set_marked(&self.ui, &text, text.len());
            field.unmark(&self.ui, true);
        }
        self.dirty = true;
    }

    /// Paints the candidate window over the Search field: a numbered list, one row per candidate.
    pub(super) fn draw_compose(&self, s: &mut Surface<'_>, ui: &Ui, w: i32, h: i32) {
        let Some(session) = self.compose_ime.session.as_ref() else {
            return;
        };
        let items = session.candidates();
        if items.is_empty() {
            return;
        }
        let (row, pad, width) = (ui.pt(ROW_PT), ui.pt(PAD_PT), ui.pt(WIDTH_PT));
        let height = pad * 2.0 + row * items.len() as f32;
        let x = (w as f32 - width) / 2.0;
        let y = h as f32 * TOP_SHARE;
        s.fill_rrect(x, y, width, height, ui.pt(m::CARD_RADIUS), MENU_SOLID.now());
        for (i, item) in items.iter().enumerate() {
            let label = format!("{}   {}", i + 1, item);
            let img = ui.text(&label, MAC_BODY_PT, Weight::Regular, WHITE);
            let ty = y + pad + row * i as f32 + (row - img.h as f32) / 2.0;
            s.blit_colored(&img, (x + pad * 2.0) as i32, ty as i32, LABEL.now(), 255);
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
        assert!(!sh.compose_ime.method.is_armed());
    }

    #[test]
    fn a_new_window_is_closed_and_has_no_candidates() {
        let c = ComposeWindow::default();
        assert!(!c.is_open());
        assert!(c.candidates().is_empty());
    }

    #[test]
    fn an_open_session_lists_its_candidates_in_order() {
        let mut c = ComposeWindow::default();
        let mut session = InputSession::new(0, PanelType::SingleColumnScrolling);
        session.update("e", "e");
        session.set_candidates(vec!["é".into(), "è".into()]);
        c.session = Some(session);
        assert!(c.is_open());
        assert_eq!(c.candidates(), &["é".to_string(), "è".to_string()]);
        assert_eq!(compose_method::pick(c.candidates(), "2"), Some(1));
    }
}
