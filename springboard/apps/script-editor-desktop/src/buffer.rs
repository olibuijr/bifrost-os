//! The text of a script and where the caret is. The text is kept as one `String` per line, so an edit touches only
//! the line it changes. Columns count characters, not bytes, so multi-byte text edits and moves correctly.

/// What a Tab key press inserts: four spaces, so the editor never has to draw a tab stop.
pub const TAB: &str = "    ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Buffer {
    lines: Vec<String>,
    row: usize,
    col: usize,
}

/// The byte offset of the character at `col` (the line length in bytes when `col` is past the end).
fn byte_at(line: &str, col: usize) -> usize {
    line.char_indices().nth(col).map_or(line.len(), |(i, _)| i)
}

impl Buffer {
    /// An empty buffer with the caret at the start.
    pub fn new() -> Self {
        Buffer { lines: vec![String::new()], row: 0, col: 0 }
    }

    /// A buffer holding `text`, with the caret at the start. Line endings are normalised to `\n`.
    pub fn from_text(text: &str) -> Self {
        let normal = text.replace("\r\n", "\n").replace('\r', "\n");
        Buffer { lines: normal.split('\n').map(str::to_string).collect(), row: 0, col: 0 }
    }

    /// The whole text with `\n` between lines.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// The caret's line, counted from 0.
    pub fn row(&self) -> usize {
        self.row
    }

    /// The caret's column in characters, counted from 0.
    pub fn col(&self) -> usize {
        self.col
    }

    /// The length of line `row` in characters (0 for a line that does not exist).
    pub fn line_len(&self, row: usize) -> usize {
        self.lines.get(row).map_or(0, |l| l.chars().count())
    }

    /// Moves the caret to `row` and `col`, clamped to the text.
    pub fn place(&mut self, row: usize, col: usize) {
        self.row = row.min(self.lines.len() - 1);
        self.col = col.min(self.line_len(self.row));
    }

    /// Types `s` at the caret. Newlines split the line, tabs become spaces, carriage returns and other control
    /// characters are dropped.
    pub fn insert(&mut self, s: &str) {
        let mut utf8 = [0u8; 4];
        for ch in s.chars() {
            match ch {
                '\n' => self.newline(),
                '\t' => self.insert_text(TAB),
                c if c.is_control() => {}
                c => self.insert_text(c.encode_utf8(&mut utf8)),
            }
        }
    }

    fn insert_text(&mut self, s: &str) {
        let line = &mut self.lines[self.row];
        let at = byte_at(line, self.col);
        line.insert_str(at, s);
        self.col += s.chars().count();
    }

    /// Splits the line at the caret. The new line starts with the indentation of the old one.
    pub fn newline(&mut self) {
        let indent: String = self.lines[self.row].chars().take_while(|c| *c == ' ').collect();
        let line = &mut self.lines[self.row];
        let at = byte_at(line, self.col);
        let rest = line.split_off(at);
        self.row += 1;
        self.col = indent.chars().count();
        self.lines.insert(self.row, format!("{indent}{rest}"));
    }

    /// Deletes the character before the caret, or joins the line with the one above at the start of a line.
    pub fn backspace(&mut self) {
        if self.col > 0 {
            let line = &mut self.lines[self.row];
            let start = byte_at(line, self.col - 1);
            let end = byte_at(line, self.col);
            line.replace_range(start..end, "");
            self.col -= 1;
        } else if self.row > 0 {
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.line_len(self.row);
            self.lines[self.row].push_str(&line);
        }
    }

    /// Deletes the character at the caret, or joins the next line at the end of a line.
    pub fn delete(&mut self) {
        if self.col < self.line_len(self.row) {
            let line = &mut self.lines[self.row];
            let start = byte_at(line, self.col);
            let end = byte_at(line, self.col + 1);
            line.replace_range(start..end, "");
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    /// Left arrow: one character back, or to the end of the line above.
    pub fn left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.line_len(self.row);
        }
    }

    /// Right arrow: one character on, or to the start of the next line.
    pub fn right(&mut self) {
        if self.col < self.line_len(self.row) {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    /// Up arrow: the same column on the line above, clamped to its length.
    pub fn up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            self.col = self.col.min(self.line_len(self.row));
        }
    }

    /// Down arrow: the same column on the line below, clamped to its length.
    pub fn down(&mut self) {
        if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = self.col.min(self.line_len(self.row));
        }
    }

    pub fn home(&mut self) {
        self.col = 0;
    }

    pub fn end(&mut self) {
        self.col = self.line_len(self.row);
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips_including_a_trailing_newline() {
        for text in ["", "echo hi", "a\nb\n", "\n\n", "one\r\ntwo\r\n"] {
            let b = Buffer::from_text(text);
            assert_eq!(b.text(), text.replace("\r\n", "\n"));
        }
        assert_eq!(Buffer::from_text("a\nb\n").line_count(), 3);
    }

    #[test]
    fn typing_inserts_at_the_caret_and_moves_it() {
        let mut b = Buffer::new();
        b.insert("ls -l");
        assert_eq!((b.row(), b.col()), (0, 5));
        b.place(0, 3);
        b.insert("-a ");
        assert_eq!(b.text(), "ls -a -l");
        assert_eq!(b.col(), 6);
    }

    #[test]
    fn tabs_become_spaces_and_control_characters_are_dropped() {
        let mut b = Buffer::new();
        b.insert("\tx\r\u{7}");
        assert_eq!(b.text(), format!("{TAB}x"));
    }

    #[test]
    fn newline_keeps_the_indentation_of_the_line_it_splits() {
        let mut b = Buffer::from_text("  if true; then echo yes");
        b.place(0, 23);
        b.newline();
        assert_eq!(b.lines(), &["  if true; then echo yes".to_string(), "  ".to_string()]);
        assert_eq!((b.row(), b.col()), (1, 2));
    }

    #[test]
    fn backspace_joins_lines_at_the_start() {
        let mut b = Buffer::from_text("ab\ncd");
        b.place(1, 0);
        b.backspace();
        assert_eq!(b.text(), "abcd");
        assert_eq!((b.row(), b.col()), (0, 2));
        b.backspace();
        assert_eq!(b.text(), "acd");
    }

    #[test]
    fn delete_removes_the_character_at_the_caret_or_joins_the_next_line() {
        let mut b = Buffer::from_text("abc\ndef");
        b.place(0, 1);
        b.delete();
        assert_eq!(b.text(), "ac\ndef");
        b.end();
        b.delete();
        assert_eq!(b.text(), "acdef");
    }

    #[test]
    fn multibyte_characters_are_one_column_each() {
        let mut b = Buffer::from_text("héllo wörld");
        b.place(0, 2);
        assert_eq!(b.line_len(0), 11);
        b.backspace();
        assert_eq!(b.text(), "hllo wörld");
        b.right();
        b.delete();
        assert_eq!(b.text(), "hlo wörld");
    }

    #[test]
    fn arrows_wrap_across_lines_and_clamp_the_column() {
        let mut b = Buffer::from_text("long line\nxy");
        b.place(0, 8);
        b.down();
        assert_eq!((b.row(), b.col()), (1, 2));
        b.right();
        assert_eq!((b.row(), b.col()), (1, 2));
        b.up();
        assert_eq!((b.row(), b.col()), (0, 2));
        b.place(0, 9);
        b.left();
        assert_eq!(b.col(), 8);
        b.home();
        b.left();
        assert_eq!((b.row(), b.col()), (0, 0));
    }

    #[test]
    fn place_clamps_to_the_text() {
        let mut b = Buffer::from_text("a\nbc");
        b.place(9, 9);
        assert_eq!((b.row(), b.col()), (1, 2));
    }
}
