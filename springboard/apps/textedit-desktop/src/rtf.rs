//! Document files: rich text as RTF, and plain UTF-8 text.
//!
//! The RTF writer emits the subset the model needs: the font table, character formatting (bold, italic, underline,
//! size, font), paragraph alignment, paragraph breaks, tabs and Unicode escapes. The reader accepts that subset and
//! the common forms other writers produce (hex escapes, `\uN` with its fallback character, `\plain`, `\pard`), and
//! skips the destinations it does not use (colour and style tables, info, pictures, `\*` extensions).

use crate::text::{Align, Cell, Style, DEFAULT_SIZE, FAMILIES};

/// Whether `bytes` are an RTF file (the header every RTF file starts with).
pub fn is_rtf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"{\\rtf")
}

/// Destinations whose text is not document text.
const SKIPPED_DESTINATIONS: &[&str] = &[
    "colortbl", "stylesheet", "info", "pict", "header", "headerl", "headerr", "footer", "footerl", "footerr", "footnote",
    "generator", "listtable", "listoverridetable", "rsidtbl", "xmlnstbl", "latentstyles", "themedata",
    "colorschememapping", "datastore", "fldinst", "object", "mmathPr", "filetbl", "revtbl",
];

/// Writes the cells as RTF.
pub fn write(cells: &[Cell]) -> Vec<u8> {
    let mut out = String::from("{\\rtf1\\ansi\\deff0{\\fonttbl");
    for (i, family) in FAMILIES.iter().enumerate() {
        out += &format!("{{\\f{i} {family};}}");
    }
    out += "}\n";
    let mut para_start = true;
    let mut last: Option<Style> = None;
    for c in cells {
        if para_start {
            out += &format!("\\pard\\q{} ", align_code(c.style.align));
            para_start = false;
        }
        if c.ch == '\n' {
            out += "\\par\n";
            para_start = true;
            continue;
        }
        if !last.is_some_and(|l| same_character(&l, &c.style)) {
            out += &character_words(&c.style);
            last = Some(c.style);
        }
        out += &escape(c.ch);
    }
    out += "}";
    out.into_bytes()
}

/// Reads an RTF file. Text the reader does not understand is dropped, never shown as garbage.
pub fn read(bytes: &[u8]) -> Vec<Cell> {
    let mut reader = Reader::default();
    reader.run(bytes);
    reader.cells
}

/// Reads plain text: UTF-8 (invalid bytes become the replacement character), with CR LF as one line break.
pub fn read_plain(bytes: &[u8]) -> Vec<Cell> {
    let text = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
    text.chars().filter(|&c| c != '\r').map(|ch| Cell { ch, style: Style::default() }).collect()
}

/// Writes the text without any formatting, as UTF-8.
pub fn write_plain(cells: &[Cell]) -> Vec<u8> {
    cells.iter().map(|c| c.ch).collect::<String>().into_bytes()
}

fn align_code(align: Align) -> char {
    match align {
        Align::Left => 'l',
        Align::Center => 'c',
        Align::Right => 'r',
        Align::Justified => 'j',
    }
}

fn same_character(a: &Style, b: &Style) -> bool {
    a.bold == b.bold && a.italic == b.italic && a.underline == b.underline && a.size == b.size && a.font == b.font
}

/// The control words that set a character's formatting from nothing: `\plain` first, so nothing is inherited.
fn character_words(style: &Style) -> String {
    let mut words = String::from("\\plain ");
    words += &format!("\\f{} \\fs{} ", style.font, (style.size * 2.0).round() as i32);
    if style.bold {
        words += "\\b ";
    }
    if style.italic {
        words += "\\i ";
    }
    if style.underline {
        words += "\\ul ";
    }
    words
}

fn escape(ch: char) -> String {
    match ch {
        '\\' => "\\\\".into(),
        '{' => "\\{".into(),
        '}' => "\\}".into(),
        '\t' => "\\tab ".into(),
        ' '..='~' => ch.to_string(),
        c if (c as u32) < 0x20 => String::new(),
        c => {
            // one `\uN?` per UTF-16 unit; N is signed, and `?` is the fallback for readers that lack Unicode
            let mut s = String::new();
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units).iter() {
                s += &format!("\\u{}?", *unit as i16);
            }
            s
        }
    }
}

/// The state of the reader: the current formatting and where the group stack and skipped text are.
#[derive(Default)]
struct Reader {
    cells: Vec<Cell>,
    cur: Style,
    stack: Vec<Style>,
    depth: usize,
    /// the depth of a destination being skipped; everything inside it is ignored
    skip: Option<usize>,
    /// the depth of the font table group, while it is read
    table: Option<usize>,
    /// the font number of the table entry being read, and its name so far
    table_font: Option<u32>,
    table_name: String,
    /// number -> index into `FAMILIES`
    fonts: Vec<(u32, u8)>,
    /// `\uc`: how many characters after a `\u` escape are its fallback
    uc: usize,
    uc_skip: usize,
    /// set by `{`, cleared by the first token of the group
    group_start: bool,
    /// the high surrogate of a `\u` pair, until its low half arrives
    high: Option<u32>,
}

impl Reader {
    fn run(&mut self, bytes: &[u8]) {
        self.uc = 1;
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            i += 1;
            match b {
                b'{' => {
                    self.depth += 1;
                    self.stack.push(self.cur);
                    self.group_start = true;
                }
                b'}' => {
                    let closing = self.depth;
                    self.depth = self.depth.saturating_sub(1);
                    self.cur = self.stack.pop().unwrap_or_default();
                    if self.skip == Some(closing) {
                        self.skip = None;
                    }
                    if self.table == Some(closing) {
                        self.table = None;
                    }
                    self.group_start = false;
                }
                b'\\' => {
                    i = self.control(bytes, i);
                }
                b'\r' | b'\n' => {}
                _ => {
                    self.take_group_start();
                    if self.skip.is_some() {
                        continue;
                    }
                    if self.table.is_some() {
                        self.table_char(b as char);
                    } else {
                        self.literal(b as char);
                    }
                }
            }
        }
    }

    /// Whether the token being read is the first of its group, clearing the flag.
    fn take_group_start(&mut self) -> bool {
        std::mem::replace(&mut self.group_start, false)
    }

    /// Reads the control after a backslash at `i`; returns the index after it.
    fn control(&mut self, bytes: &[u8], mut i: usize) -> usize {
        let Some(&c) = bytes.get(i) else { return i };
        if c.is_ascii_alphabetic() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                i += 1;
            }
            let name = String::from_utf8_lossy(&bytes[start..i]).into_owned();
            let mut param: Option<i64> = None;
            let negative = bytes.get(i) == Some(&b'-');
            let digits_at = if negative { i + 1 } else { i };
            let mut end = digits_at;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > digits_at {
                let value: i64 = std::str::from_utf8(&bytes[digits_at..end]).ok().and_then(|s| s.parse().ok()).unwrap_or(0);
                param = Some(if negative { -value } else { value });
                i = end;
            }
            if bytes.get(i) == Some(&b' ') {
                i += 1;
            }
            let first = self.take_group_start();
            if self.skip.is_some() {
                return i;
            }
            if first && name == "fonttbl" {
                self.table = Some(self.depth);
                return i;
            }
            if first && SKIPPED_DESTINATIONS.contains(&name.as_str()) {
                self.skip = Some(self.depth);
                return i;
            }
            self.apply(&name, param);
            i
        } else {
            i += 1;
            match c {
                b'*' => {
                    if self.take_group_start() {
                        self.skip = Some(self.depth);
                    }
                }
                b'\'' => {
                    let hex = bytes.get(i..i + 2).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                    self.take_group_start();
                    if let Some(byte) = hex {
                        if self.skip.is_none() {
                            self.literal(byte as char);
                        }
                        i += 2;
                    }
                }
                b'\\' | b'{' | b'}' => {
                    self.take_group_start();
                    if self.skip.is_none() {
                        self.literal(c as char);
                    }
                }
                b'~' => {
                    self.take_group_start();
                    if self.skip.is_none() {
                        self.literal('\u{a0}');
                    }
                }
                _ => {
                    self.take_group_start();
                }
            }
            i
        }
    }

    fn apply(&mut self, name: &str, param: Option<i64>) {
        let on = param != Some(0);
        match name {
            "b" => self.cur.bold = on,
            "i" => self.cur.italic = on,
            "ul" => self.cur.underline = on,
            "ulnone" => self.cur.underline = false,
            "plain" => {
                self.cur.bold = false;
                self.cur.italic = false;
                self.cur.underline = false;
                self.cur.size = DEFAULT_SIZE;
                self.cur.font = 0;
            }
            "fs" => {
                if let Some(half_points) = param.filter(|&v| v > 0) {
                    self.cur.size = (half_points as f32 / 2.0).clamp(crate::text::MIN_SIZE, crate::text::MAX_SIZE);
                }
            }
            "f" => {
                if let Some(number) = param {
                    if self.table.is_some() {
                        self.table_font = Some(number as u32);
                        self.table_name.clear();
                    } else {
                        self.cur.font = self.font_index(number as u32);
                    }
                }
            }
            "ql" => self.cur.align = Align::Left,
            "qc" => self.cur.align = Align::Center,
            "qr" => self.cur.align = Align::Right,
            "qj" => self.cur.align = Align::Justified,
            "pard" => self.cur.align = Align::Left,
            "par" | "line" => self.push('\n'),
            "tab" => self.push('\t'),
            "uc" => self.uc = param.unwrap_or(1).max(0) as usize,
            "u" => {
                if let Some(v) = param {
                    let unit = (if v < 0 { v + 65536 } else { v }) as u32;
                    // a surrogate pair is two escapes: the high one waits for its low half
                    let ch = if (0xD800..0xDC00).contains(&unit) {
                        self.high = Some(unit);
                        None
                    } else if (0xDC00..0xE000).contains(&unit) {
                        self.high.take().and_then(|h| char::from_u32(0x10000 + ((h - 0xD800) << 10) + (unit - 0xDC00)))
                    } else {
                        char::from_u32(unit)
                    };
                    if let Some(ch) = ch.filter(|_| self.skip.is_none() && self.table.is_none()) {
                        self.push(ch);
                    }
                    self.uc_skip = self.uc;
                }
            }
            _ => {}
        }
    }

    fn font_index(&self, number: u32) -> u8 {
        self.fonts.iter().find(|(n, _)| *n == number).map_or(0, |(_, index)| *index)
    }

    /// A character from the body text, unless it is the fallback of a `\u` escape.
    fn literal(&mut self, ch: char) {
        if self.uc_skip > 0 {
            self.uc_skip -= 1;
            return;
        }
        self.push(ch);
    }

    fn push(&mut self, ch: char) {
        self.cells.push(Cell { ch, style: self.cur });
    }

    /// Text inside the font table: `\fN Name;` entries.
    fn table_char(&mut self, ch: char) {
        if ch == ';' {
            if let Some(number) = self.table_font.take() {
                let name = self.table_name.trim();
                // "Courier New" is the Courier family; a name we do not carry falls back to the default
                let lower = name.to_ascii_lowercase();
                let index = FAMILIES.iter().position(|f| lower.starts_with(&f.to_ascii_lowercase())).unwrap_or(0) as u8;
                self.fonts.push((number, index));
            }
            self.table_name.clear();
        } else if self.table_font.is_some() {
            self.table_name.push(ch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(ch: char, style: Style) -> Cell {
        Cell { ch, style }
    }

    fn text_of(cells: &[Cell]) -> String {
        cells.iter().map(|c| c.ch).collect()
    }

    #[test]
    fn round_trips_formatting_alignment_and_breaks() {
        let bold = Style { bold: true, ..Style::default() };
        let centred = Style { size: 18.0, underline: true, align: Align::Center, ..Style::default() };
        let cells = vec![
            cell('a', bold),
            cell('b', Style::default()),
            cell('\n', Style::default()),
            cell('c', centred),
            cell('\n', centred),
        ];
        let back = read(&write(&cells));
        assert_eq!(back, cells);
    }

    #[test]
    fn writes_braces_and_backslashes_escaped() {
        let cells: Vec<Cell> = "{\\}".chars().map(|ch| cell(ch, Style::default())).collect();
        let back = read(&write(&cells));
        assert_eq!(text_of(&back), "{\\}");
    }

    #[test]
    fn non_ascii_survives_the_round_trip() {
        let cells: Vec<Cell> = "é€😀".chars().map(|ch| cell(ch, Style::default())).collect();
        let back = read(&write(&cells));
        assert_eq!(text_of(&back), "é€😀");
    }

    #[test]
    fn reads_hex_and_unicode_escapes_with_their_fallback() {
        // \'e9 is a hex escape, \u233? is a Unicode escape with `?` as its fallback, and the last two escapes are a
        // surrogate pair for U+1F600
        let back = read(b"{\\rtf1 caf\\'e9\\u233?x\\u-10179?\\u-8704?}");
        assert_eq!(text_of(&back), "cafééx\u{1F600}");
    }

    #[test]
    fn maps_font_numbers_through_the_font_table() {
        let back = read(b"{\\rtf1{\\fonttbl{\\f0 Times;}{\\f1 Courier New;}}\\f1 x\\f0 y}");
        assert_eq!(back[0].style.font, 2, "Courier maps to the third family");
        assert_eq!(back[1].style.font, 1, "Times maps to the second family");
        assert_eq!(text_of(&back), "xy");
    }

    #[test]
    fn skips_destinations_it_does_not_use() {
        let back = read(b"{\\rtf1{\\info{\\title hidden}}{\\*\\generator Other;}{\\colortbl;\\red0;}visible}");
        assert_eq!(text_of(&back), "visible");
    }

    #[test]
    fn plain_text_round_trips_and_keeps_breaks() {
        let cells = read_plain(b"one\r\ntwo");
        assert_eq!(text_of(&cells), "one\ntwo");
        assert_eq!(write_plain(&cells), b"one\ntwo".to_vec());
        assert!(!is_rtf(b"one"));
        assert!(is_rtf(b"{\\rtf1 x}"));
    }
}
