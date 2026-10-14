//! The reader's shelf: named collections, Want to Read and Finished marks, daily reading minutes against a goal
//! (with the streak), and the reader display settings (font family, line spacing, margins, justification, highlight
//! colour). Books are named by their library key (the file name). Pure std; each piece has a line-based text form so
//! `store.rs` can keep it beside the library.
#![allow(dead_code)] // the sidebar collections, the Finished filter, the goal card and the Display panel wire these in

use std::collections::BTreeMap;

/// A mark on a book.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    WantToRead,
    Finished,
}

/// Collections and marks. Text form: `c<TAB>collection<TAB>book`, `w<TAB>book`, `f<TAB>book`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shelf {
    collections: BTreeMap<String, Vec<String>>,
    want: Vec<String>,
    finished: Vec<String>,
}

impl Shelf {
    pub fn collections(&self) -> Vec<&str> {
        self.collections.keys().map(String::as_str).collect()
    }

    /// Creates an empty collection; false for an empty or taken name.
    pub fn create(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || name.contains('\t') || self.collections.contains_key(name) {
            return false;
        }
        self.collections.insert(name.to_string(), vec![]);
        true
    }

    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        let to = to.trim();
        if to.is_empty() || to.contains('\t') || self.collections.contains_key(to) {
            return false;
        }
        match self.collections.remove(from) {
            Some(books) => {
                self.collections.insert(to.to_string(), books);
                true
            }
            None => false,
        }
    }

    pub fn delete(&mut self, name: &str) -> bool {
        self.collections.remove(name).is_some()
    }

    /// Adds a book to a collection (once); false when the collection is missing or already holds it.
    pub fn add(&mut self, name: &str, book: &str) -> bool {
        match self.collections.get_mut(name) {
            Some(list) if !list.iter().any(|b| b == book) => {
                list.push(book.to_string());
                true
            }
            _ => false,
        }
    }

    pub fn remove(&mut self, name: &str, book: &str) -> bool {
        match self.collections.get_mut(name) {
            Some(list) => {
                let before = list.len();
                list.retain(|b| b != book);
                list.len() != before
            }
            None => false,
        }
    }

    pub fn books(&self, name: &str) -> &[String] {
        self.collections.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The collections that hold a book.
    pub fn collections_of(&self, book: &str) -> Vec<&str> {
        self.collections.iter().filter(|(_, l)| l.iter().any(|b| b == book)).map(|(n, _)| n.as_str()).collect()
    }

    fn list(&mut self, mark: Mark) -> &mut Vec<String> {
        match mark {
            Mark::WantToRead => &mut self.want,
            Mark::Finished => &mut self.finished,
        }
    }

    pub fn has(&self, mark: Mark, book: &str) -> bool {
        match mark {
            Mark::WantToRead => self.want.iter().any(|b| b == book),
            Mark::Finished => self.finished.iter().any(|b| b == book),
        }
    }

    /// Sets a mark. Finishing a book takes it off Want to Read.
    pub fn set(&mut self, mark: Mark, book: &str, on: bool) {
        let list = self.list(mark);
        list.retain(|b| b != book);
        if on {
            list.push(book.to_string());
            if mark == Mark::Finished {
                self.want.retain(|b| b != book);
            }
        }
    }

    pub fn marked(&self, mark: Mark) -> &[String] {
        match mark {
            Mark::WantToRead => &self.want,
            Mark::Finished => &self.finished,
        }
    }

    /// A book was renamed or moved in the library: every reference follows it.
    pub fn rename_book(&mut self, from: &str, to: &str) {
        let fix = |list: &mut Vec<String>| {
            for b in list.iter_mut() {
                if b == from {
                    *b = to.to_string();
                }
            }
        };
        for list in self.collections.values_mut() {
            fix(list);
        }
        fix(&mut self.want);
        fix(&mut self.finished);
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (name, books) in &self.collections {
            if books.is_empty() {
                out.push_str(&format!("c\t{name}\t\n"));
            }
            for b in books {
                out.push_str(&format!("c\t{name}\t{b}\n"));
            }
        }
        for b in &self.want {
            out.push_str(&format!("w\t{b}\n"));
        }
        for b in &self.finished {
            out.push_str(&format!("f\t{b}\n"));
        }
        out
    }

    pub fn from_text(text: &str) -> Shelf {
        let mut s = Shelf::default();
        for line in text.lines() {
            let p: Vec<&str> = line.splitn(3, '\t').collect();
            match p.as_slice() {
                ["c", name, book] => {
                    s.create(name);
                    if !book.is_empty() {
                        s.add(name, book);
                    }
                }
                ["w", book] => s.set(Mark::WantToRead, book, true),
                ["f", book] => s.set(Mark::Finished, book, true),
                _ => {}
            }
        }
        s
    }
}

/// Minutes read per local day (day numbers since 1970-01-01) and the daily goal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Goal {
    pub daily_minutes: u32,
    days: BTreeMap<i64, u32>,
}

impl Goal {
    pub fn new(daily_minutes: u32) -> Goal {
        Goal { daily_minutes, days: BTreeMap::new() }
    }

    pub fn add(&mut self, day: i64, minutes: u32) {
        *self.days.entry(day).or_insert(0) += minutes;
    }

    pub fn minutes(&self, day: i64) -> u32 {
        self.days.get(&day).copied().unwrap_or(0)
    }

    pub fn met(&self, day: i64) -> bool {
        self.daily_minutes > 0 && self.minutes(day) >= self.daily_minutes
    }

    /// Progress today as 0.0 to 1.0.
    pub fn progress(&self, day: i64) -> f32 {
        if self.daily_minutes == 0 {
            return 0.0;
        }
        (self.minutes(day) as f32 / self.daily_minutes as f32).min(1.0)
    }

    /// Consecutive days the goal was met, ending today (or yesterday while today is still open).
    pub fn streak(&self, today: i64) -> u32 {
        let mut day = if self.met(today) { today } else { today - 1 };
        let mut n = 0;
        while self.met(day) {
            n += 1;
            day -= 1;
        }
        n
    }

    /// Total minutes over the last `n` days ending today.
    pub fn total(&self, today: i64, n: i64) -> u32 {
        (0..n).map(|i| self.minutes(today - i)).sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontFamily {
    Serif,
    Sans,
    Mono,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HighlightColour {
    Yellow,
    Green,
    Blue,
    Pink,
    Purple,
    Underline,
}

impl HighlightColour {
    pub const ALL: [HighlightColour; 6] = [Self::Yellow, Self::Green, Self::Blue, Self::Pink, Self::Purple, Self::Underline];

    pub fn index(self) -> u8 {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0) as u8
    }

    pub fn from_index(i: u8) -> HighlightColour {
        Self::ALL.get(usize::from(i)).copied().unwrap_or(Self::Yellow)
    }
}

/// How the reader lays out a page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Display {
    pub family: FontFamily,
    /// line height as a multiple of the font size, 1.0 to 2.0
    pub line_spacing: f32,
    /// side margin as a fraction of the page width, 0.02 to 0.20
    pub margin: f32,
    pub justify: bool,
    pub continuous: bool,
}

impl Default for Display {
    fn default() -> Self {
        Display { family: FontFamily::Serif, line_spacing: 1.4, margin: 0.08, justify: false, continuous: false }
    }
}

impl Display {
    pub fn clamped(mut self) -> Display {
        self.line_spacing = self.line_spacing.clamp(1.0, 2.0);
        self.margin = self.margin.clamp(0.02, 0.20);
        self
    }

    pub fn to_text(&self) -> String {
        let f = match self.family {
            FontFamily::Serif => "serif",
            FontFamily::Sans => "sans",
            FontFamily::Mono => "mono",
        };
        format!("family={f}\nspacing={}\nmargin={}\njustify={}\ncontinuous={}\n", self.line_spacing, self.margin, self.justify as u8, self.continuous as u8)
    }

    pub fn from_text(text: &str) -> Display {
        let mut d = Display::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k {
                "family" => {
                    d.family = match v {
                        "sans" => FontFamily::Sans,
                        "mono" => FontFamily::Mono,
                        _ => FontFamily::Serif,
                    }
                }
                "spacing" => d.line_spacing = v.parse().unwrap_or(d.line_spacing),
                "margin" => d.margin = v.parse().unwrap_or(d.margin),
                "justify" => d.justify = v == "1",
                "continuous" => d.continuous = v == "1",
                _ => {}
            }
        }
        d.clamped()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collections_hold_books_once() {
        let mut s = Shelf::default();
        assert!(s.create("Sci-fi"));
        assert!(!s.create("Sci-fi"));
        assert!(!s.create("  "));
        assert!(s.add("Sci-fi", "dune.epub"));
        assert!(!s.add("Sci-fi", "dune.epub"));
        assert!(!s.add("Nope", "dune.epub"));
        assert_eq!(s.collections_of("dune.epub"), vec!["Sci-fi"]);
        assert!(s.rename("Sci-fi", "SF"));
        assert_eq!(s.books("SF"), ["dune.epub".to_string()]);
        assert!(s.remove("SF", "dune.epub"));
        assert!(s.delete("SF"));
        assert!(s.collections().is_empty());
    }

    #[test]
    fn finishing_clears_want_to_read() {
        let mut s = Shelf::default();
        s.set(Mark::WantToRead, "a.epub", true);
        assert!(s.has(Mark::WantToRead, "a.epub"));
        s.set(Mark::Finished, "a.epub", true);
        assert!(!s.has(Mark::WantToRead, "a.epub"));
        assert_eq!(s.marked(Mark::Finished), ["a.epub".to_string()]);
        s.set(Mark::Finished, "a.epub", false);
        assert!(s.marked(Mark::Finished).is_empty());
    }

    #[test]
    fn text_round_trip_keeps_empty_collections() {
        let mut s = Shelf::default();
        s.create("Empty");
        s.create("Full");
        s.add("Full", "x.epub");
        s.set(Mark::WantToRead, "y.pdf", true);
        s.set(Mark::Finished, "z.epub", true);
        assert_eq!(Shelf::from_text(&s.to_text()), s);
    }

    #[test]
    fn goal_progress_and_streak() {
        let mut g = Goal::new(20);
        g.add(100, 25);
        g.add(99, 10);
        g.add(99, 10);
        g.add(98, 5);
        assert!(g.met(100) && g.met(99) && !g.met(98));
        assert_eq!(g.streak(100), 2);
        assert_eq!(g.streak(101), 2); // today still open, yesterday counts
        assert_eq!(g.streak(102), 0);
        assert_eq!(g.progress(99), 1.0);
        assert_eq!(g.progress(98), 0.25);
        assert_eq!(g.total(100, 3), 25 + 20 + 5);
        assert_eq!(Goal::new(0).progress(1), 0.0);
    }

    #[test]
    fn display_round_trips_and_clamps() {
        let d = Display { family: FontFamily::Sans, line_spacing: 1.6, margin: 0.1, justify: true, continuous: true };
        assert_eq!(Display::from_text(&d.to_text()), d);
        let wild = Display::from_text("spacing=9\nmargin=0\nfamily=mono");
        assert_eq!(wild.line_spacing, 2.0);
        assert_eq!(wild.margin, 0.02);
        assert_eq!(wild.family, FontFamily::Mono);
        assert_eq!(HighlightColour::from_index(HighlightColour::Pink.index()), HighlightColour::Pink);
        assert_eq!(HighlightColour::from_index(99), HighlightColour::Yellow);
    }
}
