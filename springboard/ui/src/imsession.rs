//! A text input session on the provider side: one controller per client input session. It keeps the raw keystrokes
//! (`original`), the converted text that the client draws as marked text (`composed`), the candidate list offered for
//! the composition, and the commit, cancel and close rules. The model draws nothing and talks to no client; the shell
//! reads the strings and the candidate window and sends them on.

use crate::candidates::{Candidates, PanelType};

/// One client's input session. It is opened when the client starts typing and closed when the client leaves.
#[derive(Clone, Debug)]
pub struct InputSession {
    client: u32,
    original: String,
    composed: String,
    candidates: Candidates,
    open: bool,
}

impl InputSession {
    pub fn new(client: u32, panel: PanelType) -> Self {
        InputSession {
            client,
            original: String::new(),
            composed: String::new(),
            candidates: Candidates::new(panel),
            open: true,
        }
    }

    pub fn client(&self) -> u32 {
        self.client
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The raw keystrokes of the current composition (`originalString`).
    pub fn original_string(&self) -> &str {
        &self.original
    }

    /// The converted text that the client shows as marked text (`composedString`).
    pub fn composed_string(&self) -> &str {
        &self.composed
    }

    /// True while the composition holds text.
    pub fn is_composing(&self) -> bool {
        !self.composed.is_empty()
    }

    /// `updateComposition`: the engine changed the raw and the converted text. Returns false once the session is closed.
    pub fn update(&mut self, original: &str, composed: &str) -> bool {
        if !self.open {
            return false;
        }
        self.original = original.to_string();
        self.composed = composed.to_string();
        true
    }

    /// The candidates offered for the composition (`candidates`).
    pub fn candidates(&self) -> &[String] {
        self.candidates.items()
    }

    /// Replaces the candidate list. A new list clears the candidate selection.
    pub fn set_candidates(&mut self, items: Vec<String>) {
        self.candidates.set_items(items);
    }

    /// The candidate window model, for the shell to draw and page through.
    pub fn candidate_window(&self) -> &Candidates {
        &self.candidates
    }

    pub fn candidate_window_mut(&mut self) -> &mut Candidates {
        &mut self.candidates
    }

    /// `candidateSelected`: candidate `index` replaces the converted text. Returns false when the index is out of range
    /// or the session is closed; the text then stays as it was.
    pub fn select_candidate(&mut self, index: usize) -> bool {
        if !self.open || !self.candidates.select(index) {
            return false;
        }
        let text = self.candidates.selected_string().map(|s| s.to_string());
        match text {
            Some(t) => {
                self.composed = t;
                true
            }
            None => false,
        }
    }

    /// `commitComposition`: ends the composition and returns the converted text to insert. None when nothing is composed.
    pub fn commit(&mut self) -> Option<String> {
        if !self.is_composing() {
            return None;
        }
        self.original.clear();
        self.candidates.set_items(Vec::new());
        Some(std::mem::take(&mut self.composed))
    }

    /// `cancelComposition`: ends the composition and returns the raw keystrokes that replace the marked text.
    /// None when nothing is composed.
    pub fn cancel(&mut self) -> Option<String> {
        if !self.is_composing() {
            return None;
        }
        self.composed.clear();
        self.candidates.set_items(Vec::new());
        Some(std::mem::take(&mut self.original))
    }

    /// `inputControllerWillClose`: the session ends. Returns the converted text that was still composed, so the caller
    /// can commit or drop it. Later updates are refused.
    pub fn close(&mut self) -> Option<String> {
        let pending = if self.is_composing() { Some(std::mem::take(&mut self.composed)) } else { None };
        self.original.clear();
        self.composed.clear();
        self.candidates.set_items(Vec::new());
        self.candidates.hide();
        self.open = false;
        pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_is_open_and_empty() {
        let s = InputSession::new(7, PanelType::SingleColumnScrolling);
        assert_eq!(s.client(), 7);
        assert!(s.is_open());
        assert!(!s.is_composing());
        assert_eq!(s.original_string(), "");
        assert_eq!(s.composed_string(), "");
        assert!(s.candidates().is_empty());
    }

    #[test]
    fn update_then_commit_returns_converted_text_once() {
        let mut s = InputSession::new(1, PanelType::SingleColumnScrolling);
        assert!(s.update("nihao", "你好"));
        assert!(s.is_composing());
        assert_eq!(s.original_string(), "nihao");
        assert_eq!(s.commit(), Some("你好".to_string()));
        assert!(!s.is_composing());
        assert_eq!(s.original_string(), "");
        assert_eq!(s.commit(), None);
    }

    #[test]
    fn cancel_returns_the_raw_keystrokes() {
        let mut s = InputSession::new(1, PanelType::SingleColumnScrolling);
        s.update("nihao", "你好");
        assert_eq!(s.cancel(), Some("nihao".to_string()));
        assert!(!s.is_composing());
        assert_eq!(s.cancel(), None);
    }

    #[test]
    fn selecting_a_candidate_replaces_the_converted_text() {
        let mut s = InputSession::new(1, PanelType::ScrollingGrid);
        s.update("nihao", "你好");
        s.set_candidates(vec!["你好".to_string(), "泥好".to_string()]);
        assert_eq!(s.candidates().len(), 2);
        assert!(s.select_candidate(1));
        assert_eq!(s.composed_string(), "泥好");
        assert!(!s.select_candidate(5));
        assert_eq!(s.composed_string(), "泥好");
        assert_eq!(s.commit(), Some("泥好".to_string()));
    }

    #[test]
    fn new_candidate_list_clears_the_selection() {
        let mut s = InputSession::new(1, PanelType::SingleRowStepping);
        s.update("ka", "咔");
        s.set_candidates(vec!["咔".to_string()]);
        assert!(s.select_candidate(0));
        s.set_candidates(vec!["卡".to_string()]);
        assert_eq!(s.candidate_window().selected(), None);
        assert_eq!(s.candidates(), &["卡".to_string()][..]);
    }

    #[test]
    fn closing_returns_pending_text_and_refuses_updates() {
        let mut s = InputSession::new(2, PanelType::SingleColumnScrolling);
        s.update("nihao", "你好");
        assert_eq!(s.close(), Some("你好".to_string()));
        assert!(!s.is_open());
        assert!(!s.update("a", "a"));
        assert!(!s.is_composing());
        assert!(!s.select_candidate(0));
        assert_eq!(s.close(), None);
    }
}
