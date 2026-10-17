//! Copy Special: the selection on the pasteboard as plain text, and as RTF with its colours (Text with Colors) or
//! its foreground colours only (Without Background Colour). The plain text is the first item, the RTF the second.
use super::*;

/// The UTType ids the pasteboard items carry.
pub const PLAIN: &str = "public.utf8-plain-text";
pub const RTF: &str = "public.rtf";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Edit > Copy: plain text only.
    Plain,
    /// Edit > Copy Special > Text with Colors.
    Colors,
    /// Edit > Copy Special > Without Background Colour.
    ColorsNoBackground,
}

/// The pasteboard items for a copy of `text` in `mode`: plain text, plus RTF for the colour modes when `rtf` is
/// given. Pure, so the item list is testable without the shell.
pub fn items(mode: Mode, text: &str, rtf: Option<String>) -> Vec<(String, Vec<u8>)> {
    let mut out = vec![(PLAIN.to_string(), text.as_bytes().to_vec())];
    if mode != Mode::Plain {
        if let Some(rtf) = rtf {
            out.push((RTF.to_string(), rtf.into_bytes()));
        }
    }
    out
}

impl Shell {
    /// Copies the selection in `mode` and says what went on the pasteboard.
    pub(super) fn terminal_copy_special(&mut self, mode: Mode) {
        let font_pt = termcore::zoom::SIZES[self.terminal_ui.settings.zoom] as f32;
        let Some(term) = self.term.as_ref() else {
            self.terminal_ui.message = "Select terminal text first".into();
            return;
        };
        let Some(text) = term.selected_text() else {
            self.terminal_ui.message = "Select terminal text first".into();
            return;
        };
        let rtf = match mode {
            Mode::Plain => None,
            Mode::Colors => term.selected_rtf(true, font_pt),
            Mode::ColorsNoBackground => term.selected_rtf(false, font_pt),
        };
        let items = items(mode, &text, rtf.clone());
        // the shell's pasteboard keeps every item; its plain text is what the shared clipboard carries
        let plain = super::super::pasteboard::set(items.clone()).unwrap_or_else(|| text.clone());
        clipboard::set_text(&plain);
        super::super::pasteboard::offer_wayland(&items);
        self.terminal_ui.message = match (mode, rtf.is_some()) {
            (Mode::Plain, _) => "Selection copied".into(),
            (Mode::Colors, true) => "Selection copied with colors".into(),
            (Mode::ColorsNoBackground, true) => "Selection copied without background colour".into(),
            _ => "Selection copied as plain text".into(),
        };
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_copies_carry_plain_text_then_rtf() {
        let plain = items(Mode::Plain, "hi", Some("{\\rtf1}".into()));
        assert_eq!(plain, vec![(PLAIN.to_string(), b"hi".to_vec())], "plain copy never carries RTF");
        let styled = items(Mode::Colors, "hi", Some("{\\rtf1}".into()));
        assert_eq!(styled.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), [PLAIN, RTF]);
        assert_eq!(styled[1].1, b"{\\rtf1}".to_vec());
        let none = items(Mode::ColorsNoBackground, "hi", None);
        assert_eq!(none.len(), 1, "no RTF when the selection has none");
    }
}
