//! The sticky notes board as plain data: the notes, their colour, frame, collapse and text style, editing,
//! import and export as text, and the JSON file the app keeps in its data directory. No drawing here.

use bifrost_core::json::{self, Json};

/// The title bar height of a note, in points. A collapsed note is only its title bar.
pub const TITLE_H: f32 = 28.0;
/// Width of the disclosure (collapse) control at the left of the title bar, in points.
pub const DISCLOSURE_PT: f32 = 24.0;
/// The resize grip square at the bottom right corner of an expanded note, in points.
pub const GRIP_PT: f32 = 16.0;
/// Inner padding of a note's body, in points.
pub const PAD_PT: f32 = 10.0;
pub const MIN_W: f32 = 180.0;
pub const MIN_H: f32 = 120.0;
pub const DEFAULT_W: f32 = 220.0;
pub const DEFAULT_H: f32 = 200.0;
/// Separates the notes in the exported text file of all notes.
pub const EXPORT_SEPARATOR: &str = "\n\n---\n\n";
/// Largest file the app reads on import (bytes).
pub const IMPORT_MAX: u64 = 1 << 20;

/// The six sticky colours, in menu order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colour {
    Yellow,
    Orange,
    Green,
    Teal,
    Pink,
    Purple,
}

impl Colour {
    pub const ALL: [Colour; 6] = [Colour::Yellow, Colour::Orange, Colour::Green, Colour::Teal, Colour::Pink, Colour::Purple];

    pub fn index(self) -> usize {
        Colour::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    /// The colour at `i`, wrapping, so a stored index past the end still picks a colour.
    pub fn from_index(i: usize) -> Colour {
        Colour::ALL[i % Colour::ALL.len()]
    }

    pub fn next(self) -> Colour {
        Colour::from_index(self.index() + 1)
    }

    /// The name stored in the file and shown in the menu.
    pub fn name(self) -> &'static str {
        match self {
            Colour::Yellow => "Yellow",
            Colour::Orange => "Orange",
            Colour::Green => "Green",
            Colour::Teal => "Teal",
            Colour::Pink => "Pink",
            Colour::Purple => "Purple",
        }
    }

    pub fn from_name(name: &str) -> Option<Colour> {
        Colour::ALL.iter().copied().find(|c| c.name() == name)
    }
}

/// Text size of a note's body. The drawing maps each to a macOS text style token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSize {
    Small,
    Medium,
    Large,
}

impl TextSize {
    pub fn name(self) -> &'static str {
        match self {
            TextSize::Small => "small",
            TextSize::Medium => "medium",
            TextSize::Large => "large",
        }
    }

    pub fn from_name(name: &str) -> TextSize {
        match name {
            "small" => TextSize::Small,
            "large" => TextSize::Large,
            _ => TextSize::Medium,
        }
    }
}

/// Whole-note formatting: Bold and Italic apply to the entire body (per-range formatting is not kept yet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub size: TextSize,
}

impl Default for Style {
    fn default() -> Self {
        Style { bold: false, italic: false, size: TextSize::Medium }
    }
}

/// The hit areas of a note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Disclosure,
    Title,
    Body,
    Grip,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub id: u64,
    pub colour: Colour,
    /// Frame in points, board space.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub collapsed: bool,
    pub style: Style,
    pub text: String,
    /// Unix seconds.
    pub created: u64,
    pub modified: u64,
}

impl Note {
    pub fn new(id: u64, x: f32, y: f32, now: u64) -> Note {
        Note {
            id,
            colour: Colour::Yellow,
            x,
            y,
            w: DEFAULT_W,
            h: DEFAULT_H,
            collapsed: false,
            style: Style::default(),
            text: String::new(),
            created: now,
            modified: now,
        }
    }

    /// The height the note occupies on the board: its title bar when collapsed.
    pub fn frame_h(&self) -> f32 {
        if self.collapsed {
            TITLE_H
        } else {
            self.h
        }
    }

    /// The first non-blank line, which names the note in the title bar; empty for a blank note.
    pub fn title(&self) -> String {
        self.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").chars().take(60).collect()
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.frame_h()
    }

    /// Which part of the note a board point (points) lands on.
    pub fn part_at(&self, px: f32, py: f32) -> Option<Part> {
        if !self.contains(px, py) {
            return None;
        }
        if py < self.y + TITLE_H {
            return Some(if px < self.x + DISCLOSURE_PT { Part::Disclosure } else { Part::Title });
        }
        if !self.collapsed && px >= self.x + self.w - GRIP_PT && py >= self.y + self.h - GRIP_PT {
            return Some(Part::Grip);
        }
        Some(Part::Body)
    }

    /// Set the frame, keeping it at least the minimum size.
    pub fn resize(&mut self, w: f32, h: f32) {
        self.w = w.max(MIN_W);
        self.h = h.max(MIN_H);
    }

    /// Typed text goes to the end of the body: the board has no caret position yet.
    pub fn type_text(&mut self, text: &str, now: u64) {
        self.text.push_str(text);
        self.modified = now;
    }

    pub fn newline(&mut self, now: u64) {
        self.text.push('\n');
        self.modified = now;
    }

    pub fn backspace(&mut self, now: u64) -> bool {
        let removed = self.text.pop().is_some();
        if removed {
            self.modified = now;
        }
        removed
    }
}

/// All notes, in stacking order: the last one is in front.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Board {
    pub notes: Vec<Note>,
    pub next_id: u64,
    /// The note that takes typing and draws active.
    pub focus: Option<u64>,
}

impl Board {
    pub fn find(&self, id: u64) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    pub fn find_mut(&mut self, id: u64) -> Option<&mut Note> {
        self.notes.iter_mut().find(|n| n.id == id)
    }

    pub fn focused(&self) -> Option<&Note> {
        self.focus.and_then(|id| self.find(id))
    }

    pub fn focused_mut(&mut self) -> Option<&mut Note> {
        let id = self.focus?;
        self.find_mut(id)
    }

    /// Adds an empty note at `(x, y)`, in front, and focuses it.
    pub fn new_note(&mut self, x: f32, y: f32, now: u64) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.notes.push(Note::new(id, x, y, now));
        self.focus = Some(id);
        id
    }

    /// Adds a note holding `text`, placed at `(x, y)`.
    pub fn add_text(&mut self, text: &str, x: f32, y: f32, now: u64) -> u64 {
        let id = self.new_note(x, y, now);
        if let Some(n) = self.find_mut(id) {
            n.text = text.to_string();
        }
        id
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        if self.focus == Some(id) {
            self.focus = self.notes.last().map(|n| n.id);
        }
        self.notes.len() != before
    }

    /// The front-most note under the point, if any.
    pub fn hit(&self, px: f32, py: f32) -> Option<u64> {
        self.notes.iter().rev().find(|n| n.contains(px, py)).map(|n| n.id)
    }

    /// Moves the note to the front and focuses it.
    pub fn raise(&mut self, id: u64) {
        if let Some(pos) = self.notes.iter().position(|n| n.id == id) {
            let n = self.notes.remove(pos);
            self.notes.push(n);
            self.focus = Some(id);
        }
    }

    /// Plain text of one note, or of all of them, separated by `EXPORT_SEPARATOR`.
    pub fn export_text(&self, id: Option<u64>) -> String {
        match id {
            Some(id) => self.find(id).map(|n| n.text.clone()).unwrap_or_default(),
            None => self.notes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>().join(EXPORT_SEPARATOR),
        }
    }

    /// Imports a text file as one note, or as several when it holds `EXPORT_SEPARATOR` lines. Returns the new ids.
    pub fn import_text(&mut self, text: &str, now: u64) -> Vec<u64> {
        let parts: Vec<&str> = if text.contains("\n---\n") {
            text.split("\n---\n").map(|p| p.trim_matches('\n')).collect()
        } else {
            vec![text]
        };
        let mut ids = Vec::new();
        for (i, part) in parts.iter().filter(|p| !p.trim().is_empty()).enumerate() {
            let offset = (i as f32) * 24.0;
            ids.push(self.add_text(part, 40.0 + offset, 60.0 + offset, now));
        }
        ids
    }
}

/// Encodes the board as the app's JSON file.
pub fn encode(board: &Board) -> String {
    let notes = board
        .notes
        .iter()
        .map(|n| {
            Json::Obj(vec![
                ("id".into(), Json::Num(n.id as f64)),
                ("colour".into(), Json::Str(n.colour.name().into())),
                ("x".into(), Json::Num(n.x as f64)),
                ("y".into(), Json::Num(n.y as f64)),
                ("w".into(), Json::Num(n.w as f64)),
                ("h".into(), Json::Num(n.h as f64)),
                ("collapsed".into(), Json::Bool(n.collapsed)),
                ("bold".into(), Json::Bool(n.style.bold)),
                ("italic".into(), Json::Bool(n.style.italic)),
                ("size".into(), Json::Str(n.style.size.name().into())),
                ("text".into(), Json::Str(n.text.clone())),
                ("created".into(), Json::Num(n.created as f64)),
                ("modified".into(), Json::Num(n.modified as f64)),
            ])
        })
        .collect();
    Json::Obj(vec![
        ("version".into(), Json::Num(1.0)),
        ("next_id".into(), Json::Num(board.next_id as f64)),
        ("focus".into(), board.focus.map_or(Json::Null, |id| Json::Num(id as f64))),
        ("notes".into(), Json::Arr(notes)),
    ])
    .encode()
}

/// Decodes the app's JSON file. Unknown or malformed notes are skipped rather than failing the whole board.
pub fn decode(text: &str) -> Result<Board, String> {
    let root = json::parse(text)?;
    let mut board = Board::default();
    let notes = root.get("notes").and_then(Json::as_array).ok_or("no notes array")?;
    for item in notes {
        let Some(id) = item.get("id").and_then(Json::number) else { continue };
        let num = |k: &str, d: f32| item.get(k).and_then(Json::number).map_or(d, |v| v as f32);
        let mut note = Note::new(id as u64, num("x", 40.0), num("y", 60.0), 0);
        note.colour = item.get("colour").and_then(Json::as_str).and_then(Colour::from_name).unwrap_or(Colour::Yellow);
        note.resize(num("w", DEFAULT_W), num("h", DEFAULT_H));
        note.collapsed = matches!(item.get("collapsed"), Some(Json::Bool(true)));
        note.style = Style {
            bold: matches!(item.get("bold"), Some(Json::Bool(true))),
            italic: matches!(item.get("italic"), Some(Json::Bool(true))),
            size: TextSize::from_name(&item.str_field("size").unwrap_or_default()),
        };
        note.text = item.str_field("text").unwrap_or_default();
        note.created = item.get("created").and_then(Json::number).unwrap_or(0.0) as u64;
        note.modified = item.get("modified").and_then(Json::number).unwrap_or(0.0) as u64;
        board.notes.push(note);
    }
    board.next_id = root.get("next_id").and_then(Json::number).map_or(0, |v| v as u64);
    board.next_id = board.next_id.max(board.notes.iter().map(|n| n.id).max().unwrap_or(0));
    board.focus = root.get("focus").and_then(Json::number).map(|v| v as u64).filter(|id| board.find(*id).is_some());
    Ok(board)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_wrap_and_round_trip_by_name() {
        assert_eq!(Colour::from_index(6), Colour::Yellow);
        assert_eq!(Colour::Purple.next(), Colour::Yellow);
        for c in Colour::ALL {
            assert_eq!(Colour::from_name(c.name()), Some(c));
        }
        assert_eq!(Colour::from_name("Lime"), None);
    }

    #[test]
    fn collapsed_note_is_only_its_title_bar() {
        let mut n = Note::new(1, 10.0, 10.0, 0);
        assert!(n.contains(20.0, 150.0));
        n.collapsed = true;
        assert!(!n.contains(20.0, 150.0));
        assert!(n.contains(20.0, 20.0));
    }

    #[test]
    fn parts_of_an_expanded_and_a_collapsed_note() {
        let mut n = Note::new(1, 100.0, 100.0, 0);
        assert_eq!(n.part_at(101.0, 101.0), Some(Part::Disclosure));
        assert_eq!(n.part_at(150.0, 101.0), Some(Part::Title));
        assert_eq!(n.part_at(150.0, 150.0), Some(Part::Body));
        assert_eq!(n.part_at(100.0 + n.w - 1.0, 100.0 + n.h - 1.0), Some(Part::Grip));
        assert_eq!(n.part_at(50.0, 50.0), None);
        n.collapsed = true;
        assert_eq!(n.part_at(150.0, 150.0), None);
        assert_eq!(n.part_at(150.0, 101.0), Some(Part::Title));
    }

    #[test]
    fn resize_keeps_the_minimum_size() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.resize(10.0, 10.0);
        assert_eq!((n.w, n.h), (MIN_W, MIN_H));
    }

    #[test]
    fn typing_and_backspace_update_the_text_and_time() {
        let mut n = Note::new(1, 0.0, 0.0, 5);
        n.type_text("héllo", 9);
        n.newline(10);
        n.type_text("x", 11);
        assert_eq!(n.text, "héllo\nx");
        assert!(n.backspace(12));
        assert!(n.backspace(13));
        assert_eq!(n.text, "héllo\n");
        assert_eq!(n.modified, 13);
        assert_eq!(n.created, 5);
        let mut empty = Note::new(2, 0.0, 0.0, 0);
        assert!(!empty.backspace(4));
        assert_eq!(empty.modified, 0);
    }

    #[test]
    fn title_is_the_first_non_blank_line() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        n.text = "\n   \n  Groceries  \nmilk".into();
        assert_eq!(n.title(), "Groceries");
    }

    #[test]
    fn hit_returns_the_front_note_and_raise_focuses_it() {
        let mut b = Board::default();
        let a = b.add_text("a", 0.0, 0.0, 0);
        let c = b.add_text("c", 0.0, 0.0, 0);
        assert_eq!(b.hit(10.0, 10.0), Some(c));
        b.raise(a);
        assert_eq!(b.hit(10.0, 10.0), Some(a));
        assert_eq!(b.focus, Some(a));
    }

    #[test]
    fn removing_the_focused_note_moves_focus_to_the_front_one() {
        let mut b = Board::default();
        let a = b.add_text("a", 0.0, 0.0, 0);
        let c = b.add_text("c", 0.0, 0.0, 0);
        assert!(b.remove(c));
        assert_eq!(b.focus, Some(a));
        assert!(!b.remove(c));
        assert!(b.remove(a));
        assert_eq!(b.focus, None);
    }

    #[test]
    fn ids_keep_growing_after_removal() {
        let mut b = Board::default();
        let a = b.new_note(0.0, 0.0, 0);
        b.remove(a);
        assert!(b.new_note(0.0, 0.0, 0) > a);
    }

    #[test]
    fn export_all_joins_notes_and_import_splits_them_back() {
        let mut b = Board::default();
        b.add_text("first", 0.0, 0.0, 0);
        b.add_text("second", 0.0, 0.0, 0);
        let all = b.export_text(None);
        assert_eq!(all, format!("first{EXPORT_SEPARATOR}second"));
        let mut other = Board::default();
        let ids = other.import_text(&all, 7);
        assert_eq!(ids.len(), 2);
        assert_eq!(other.find(ids[1]).map(|n| n.text.as_str()), Some("second"));
        assert_eq!(other.find(ids[0]).map(|n| n.created), Some(7));
    }

    #[test]
    fn import_of_blank_text_adds_nothing() {
        let mut b = Board::default();
        assert!(b.import_text("  \n ", 1).is_empty());
        assert!(b.notes.is_empty());
    }

    #[test]
    fn export_of_one_note_and_of_a_missing_note() {
        let mut b = Board::default();
        let a = b.add_text("only", 0.0, 0.0, 0);
        assert_eq!(b.export_text(Some(a)), "only");
        assert_eq!(b.export_text(Some(99)), "");
    }

    #[test]
    fn json_round_trip_keeps_every_field() {
        let mut b = Board::default();
        let a = b.add_text("Hello \"world\"\nline two", 12.5, 40.0, 100);
        {
            let n = b.find_mut(a).unwrap();
            n.colour = Colour::Pink;
            n.collapsed = true;
            n.style = Style { bold: true, italic: true, size: TextSize::Large };
            n.resize(300.0, 240.0);
            n.modified = 150;
        }
        b.add_text("second", 0.0, 0.0, 101);
        let decoded = decode(&encode(&b)).unwrap();
        assert_eq!(decoded, b);
    }

    #[test]
    fn decode_skips_broken_notes_and_drops_a_stale_focus() {
        let text = r#"{"version":1,"next_id":3,"focus":9,"notes":[{"id":2,"colour":"Nope","text":"ok"},{"text":"no id"}]}"#;
        let b = decode(text).unwrap();
        assert_eq!(b.notes.len(), 1);
        assert_eq!(b.notes[0].colour, Colour::Yellow);
        assert_eq!(b.notes[0].text, "ok");
        assert_eq!(b.focus, None);
        assert_eq!(b.next_id, 3);
    }

    #[test]
    fn decode_rejects_text_that_is_not_a_board() {
        assert!(decode("[]").is_err());
        assert!(decode("{").is_err());
    }
}
