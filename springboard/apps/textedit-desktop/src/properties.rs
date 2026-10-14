//! Document properties: the title, author, subject, keywords and copyright a document carries (File > Show
//! Properties). They are read and written by the file formats (`rtf.rs` for the RTF info group, `docx.rs` and `odt.rs`
//! for the package metadata) and edited in a small panel over the page, whose geometry and editing rules are here.
//! Pure: no shell types besides `Rect`.

use bifrost_app::controls::Rect;

/// The longest value a field takes, in characters.
pub const MAX_LEN: usize = 256;
/// The field names, in the order the panel lists them and `Properties::get` indexes them.
pub const LABELS: [&str; 5] = ["Title", "Author", "Subject", "Keywords", "Copyright"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Properties {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
    pub copyright: String,
}

impl Properties {
    pub fn is_empty(&self) -> bool {
        (0..LABELS.len()).all(|i| self.get(i).is_empty())
    }

    /// The field at `index` in `LABELS` order (an index past the end gives the last field).
    pub fn get(&self, index: usize) -> &str {
        match index {
            0 => &self.title,
            1 => &self.author,
            2 => &self.subject,
            3 => &self.keywords,
            _ => &self.copyright,
        }
    }

    pub fn get_mut(&mut self, index: usize) -> &mut String {
        match index {
            0 => &mut self.title,
            1 => &mut self.author,
            2 => &mut self.subject,
            3 => &mut self.keywords,
            _ => &mut self.copyright,
        }
    }

    /// Sets a field by the name a file format uses for it (RTF info words, package metadata elements). Unknown names
    /// are left alone; the value is cleaned the way the panel cleans typed text.
    pub fn set_named(&mut self, name: &str, value: &str) {
        let index = match name {
            "title" | "dc:title" => 0,
            "author" | "creator" | "dc:creator" | "meta:initial-creator" => 1,
            "subject" | "dc:subject" => 2,
            "keywords" | "keyword" | "cp:keywords" | "meta:keyword" => 3,
            "copyright" => 4,
            _ => return,
        };
        let field = self.get_mut(index);
        if field.is_empty() {
            *field = clean(value);
        } else if index == 3 && name == "meta:keyword" {
            // an OpenDocument file has one keyword element per keyword
            field.push_str(", ");
            field.push_str(&clean(value));
        }
    }
}

/// The properties in a metadata part of a package (`docProps/core.xml`, `meta.xml`): the text of the elements
/// `set_named` knows, by their prefixed names.
pub fn read_xml(xml: &str) -> Properties {
    use crate::xml::Token;
    let mut props = Properties::default();
    let mut open: Option<String> = None;
    for token in crate::xml::tokens(xml) {
        match token {
            Token::Open { name, empty: false, .. } => open = Some(name),
            Token::Text(text) => {
                if let Some(name) = open.take() {
                    props.set_named(&name, &text);
                }
            }
            Token::Close(_) | Token::Open { .. } => open = None,
        }
    }
    props
}

/// Text for a field: one line, no control characters, at most `MAX_LEN` characters, no outer blanks.
pub fn clean(value: &str) -> String {
    let one_line: String = value.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    one_line.trim().chars().take(MAX_LEN).collect()
}

/// The panel while it is open: a copy of the properties being edited, and the field with the keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panel {
    pub draft: Properties,
    pub focus: usize,
}

impl Panel {
    pub fn new(current: &Properties) -> Self {
        Panel { draft: current.clone(), focus: 0 }
    }

    /// Typed text goes to the focused field (control characters are dropped, the length is capped).
    pub fn type_text(&mut self, s: &str) {
        let field = self.draft.get_mut(self.focus);
        for ch in s.chars().filter(|c| !c.is_control()) {
            if field.chars().count() >= MAX_LEN {
                break;
            }
            field.push(ch);
        }
    }

    pub fn backspace(&mut self) {
        self.draft.get_mut(self.focus).pop();
    }

    /// Tab: the next field, wrapping to the first.
    pub fn next_field(&mut self) {
        self.focus = (self.focus + 1) % LABELS.len();
    }

    /// The draft as it is stored: each field cleaned.
    pub fn result(&self) -> Properties {
        let mut out = Properties::default();
        for i in 0..LABELS.len() {
            *out.get_mut(i) = clean(self.draft.get(i));
        }
        out
    }
}

/// Where the panel's parts are, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelLayout {
    pub frame: Rect,
    pub title_y: f32,
    pub labels_x: f32,
    pub fields: [Rect; 5],
    pub buttons_y: f32,
    pub right: f32,
}

const PAD: f32 = 16.0;
const TITLE_H: f32 = 30.0;
const ROW_H: f32 = 34.0;
const FIELD_H: f32 = 26.0;
const LABEL_W: f32 = 84.0;
const BUTTONS_H: f32 = 48.0;
const MAX_W: f32 = 440.0;
/// The gap between the page's top edge (where the panel hangs from) and the panel.
const TOP_GAP: f32 = 12.0;

/// The properties panel for a window `width` points wide whose page starts at `body_top`: centred, never wider than the
/// window.
pub fn layout(width: f32, body_top: f32) -> PanelLayout {
    layout_rows(width, body_top, LABELS.len())
}

/// A panel with `rows` labelled fields (at most five), laid out like the properties panel: the link panel has one.
pub fn layout_rows(width: f32, body_top: f32, rows: usize) -> PanelLayout {
    let rows = rows.min(5);
    let w = (width - 2.0 * PAD).clamp(0.0, MAX_W);
    let x = ((width - w) / 2.0).max(0.0);
    let y = body_top + TOP_GAP;
    let h = PAD + TITLE_H + ROW_H * rows as f32 + BUTTONS_H;
    let field_x = x + PAD + LABEL_W;
    let field_w = (w - 2.0 * PAD - LABEL_W).max(0.0);
    let first = y + PAD + TITLE_H;
    let mut fields = [Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }; 5];
    for (i, f) in fields.iter_mut().enumerate().take(rows) {
        *f = Rect { x: field_x, y: first + ROW_H * i as f32 + (ROW_H - FIELD_H) / 2.0, w: field_w, h: FIELD_H };
    }
    PanelLayout {
        frame: Rect { x, y, w, h },
        title_y: y + PAD,
        labels_x: x + PAD,
        fields,
        buttons_y: first + ROW_H * rows as f32 + (BUTTONS_H - FIELD_H) / 2.0,
        right: x + w - PAD,
    }
}

/// The field under a point, in points.
pub fn field_at(layout: &PanelLayout, x: f32, y: f32) -> Option<usize> {
    layout.fields.iter().position(|f| f.contains(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_goes_to_the_focused_field_and_tab_steps_through_them() {
        let mut panel = Panel::new(&Properties::default());
        panel.type_text("Notes");
        panel.next_field();
        panel.type_text("Ada");
        assert_eq!(panel.draft.title, "Notes");
        assert_eq!(panel.draft.author, "Ada");
        for _ in 0..4 {
            panel.next_field();
        }
        assert_eq!(panel.focus, 0, "Tab wraps after the last field");
        panel.backspace();
        assert_eq!(panel.draft.title, "Note");
    }

    #[test]
    fn a_field_is_one_clean_line_of_bounded_length() {
        let mut panel = Panel::new(&Properties::default());
        panel.type_text("one\ntwo\u{7}");
        assert_eq!(panel.draft.title, "onetwo", "control characters are not typed");
        assert_eq!(clean("  a\tb\nc  "), "a b c");
        panel.type_text(&"x".repeat(1000));
        assert_eq!(panel.draft.title.chars().count(), MAX_LEN);
        assert_eq!(panel.result().title.chars().count(), MAX_LEN);
    }

    #[test]
    fn named_fields_come_from_the_names_file_formats_use() {
        let mut p = Properties::default();
        p.set_named("title", " Report ");
        p.set_named("dc:creator", "Ada");
        p.set_named("cp:keywords", "a, b");
        p.set_named("copyright", "2026");
        p.set_named("nonsense", "ignored");
        assert_eq!((p.title.as_str(), p.author.as_str(), p.keywords.as_str(), p.copyright.as_str()), ("Report", "Ada", "a, b", "2026"));
        assert!(!p.is_empty());
        assert!(Properties::default().is_empty());
        // an OpenDocument file lists one keyword element each
        let mut q = Properties::default();
        q.set_named("meta:keyword", "red");
        q.set_named("meta:keyword", "blue");
        assert_eq!(q.keywords, "red, blue");
        // a second title does not replace the first
        p.set_named("title", "Other");
        assert_eq!(p.title, "Report");
    }

    #[test]
    fn package_metadata_is_read_from_its_elements() {
        let core = "<?xml version=\"1.0\"?><cp:coreProperties><dc:title>Plan &amp; notes</dc:title><dc:creator>Ada</dc:creator>\
<cp:keywords>a, b</cp:keywords><dc:description>ignored</dc:description><dc:subject/></cp:coreProperties>";
        let p = read_xml(core);
        assert_eq!((p.title.as_str(), p.author.as_str(), p.keywords.as_str(), p.subject.as_str()), ("Plan & notes", "Ada", "a, b", ""));
        let meta = "<office:meta><meta:initial-creator>Bob</meta:initial-creator><dc:creator>Eve</dc:creator><meta:keyword>x</meta:keyword><meta:keyword>y</meta:keyword></office:meta>";
        let q = read_xml(meta);
        assert_eq!((q.author.as_str(), q.keywords.as_str()), ("Bob", "x, y"), "the first author wins and keywords are joined");
    }

    #[test]
    fn the_panel_is_centred_and_fits_a_narrow_window() {
        let wide = layout(1200.0, 80.0);
        assert!((wide.frame.x * 2.0 + wide.frame.w - 1200.0).abs() < 0.01);
        assert_eq!(wide.frame.w, MAX_W);
        let narrow = layout(300.0, 80.0);
        assert_eq!(narrow.frame.w, 300.0 - 2.0 * PAD);
        for f in narrow.fields {
            assert!(f.x >= narrow.frame.x && f.x + f.w <= narrow.frame.x + narrow.frame.w);
        }
        assert!(wide.fields[0].y > wide.title_y, "the first field is under the title");
        assert!(wide.buttons_y > wide.fields[4].y + FIELD_H);
    }

    #[test]
    fn a_panel_with_one_field_is_shorter_and_only_that_field_is_touchable() {
        let one = layout_rows(800.0, 80.0, 1);
        let five = layout(800.0, 80.0);
        assert!(one.frame.h < five.frame.h);
        assert_eq!(one.frame.y, five.frame.y);
        assert_eq!(one.fields[0], five.fields[0]);
        assert_eq!(field_at(&one, one.fields[0].x + 2.0, one.fields[0].y + 2.0), Some(0));
        assert_eq!(field_at(&one, one.fields[1].x + 2.0, one.fields[1].y + 2.0), None, "unused rows have no field");
        assert!(one.buttons_y < five.buttons_y);
        assert!(one.buttons_y + FIELD_H <= one.frame.y + one.frame.h);
    }

    #[test]
    fn a_touch_finds_its_field() {
        let l = layout(800.0, 80.0);
        let f = l.fields[3];
        assert_eq!(field_at(&l, f.x + 2.0, f.y + 2.0), Some(3));
        assert_eq!(field_at(&l, l.frame.x + 1.0, l.frame.y + 1.0), None);
    }
}
