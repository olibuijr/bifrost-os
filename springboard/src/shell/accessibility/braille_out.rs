//! Braille output: the screen reader's current line (the text it speaks for the element under its cursor, or the rotor name)
//! translated to six-dot cells for a Braille display. The translation and the display window come from `ui::braille`; this
//! module keeps the line the shell shows and the cells a display driver would send. No driver is attached yet, so the line
//! is ready but nothing is written to a device.

use bifrost_ui::braille::{self, Cell, Display};

/// The cells on a display when none is named: a common 40-cell line.
pub const DEFAULT_CELLS: usize = 40;

/// The line on the Braille display: the translated text and the window the display shows over it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrailleLine {
    display: Display,
    cells: Vec<Cell>,
}

impl Default for BrailleLine {
    fn default() -> Self {
        Self { display: Display::new(DEFAULT_CELLS), cells: Vec::new() }
    }
}

/// Translates `text` to cells. A character the translator does not cover becomes a blank cell rather than a wrong one, so
/// the line never shows a letter that was not spoken.
pub fn translate_lossy(text: &str) -> Vec<Cell> {
    braille::translate(text).unwrap_or_else(|_| {
        text.chars().flat_map(|c| braille::translate(&c.to_string()).unwrap_or_else(|_| vec![0])).collect()
    })
}

impl BrailleLine {
    /// Shows `text` from the start of the line.
    pub fn show(&mut self, text: &str) {
        self.cells = translate_lossy(text);
        self.display = Display::new(self.display.width());
    }

    /// The Unicode Braille characters the display shows now.
    pub fn window_text(&self) -> String {
        braille::to_unicode(self.display.window(&self.cells))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_shows_as_its_cells_from_the_start_of_the_line() {
        let mut line = BrailleLine::default();
        line.show("ab");
        assert_eq!(line.window_text(), braille::to_unicode(&braille::translate("ab").unwrap()));
    }

    #[test]
    fn a_character_outside_the_translator_becomes_a_blank_cell() {
        // the translator covers letters, digits and common punctuation; a Greek letter has no cell here
        let cells = translate_lossy("a\u{3b1}b");
        let plain = braille::translate("ab").unwrap();
        assert_eq!(cells, vec![plain[0], 0, plain[1]]);
    }

    #[test]
    fn an_empty_line_shows_nothing() {
        let mut line = BrailleLine::default();
        line.show("");
        assert_eq!(line.window_text(), "");
    }
}
