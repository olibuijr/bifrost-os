//! Notes settings: the default text size, the dark note background, what a new note starts with and the tile size of
//! the attachment browser. They are saved beside the notes as key=value lines. A missing, unknown or damaged value
//! falls back to its default, so an older or edited file still loads.

use std::io;
use std::path::Path;

const SETTINGS_FILE: &str = ".springboard-notes-settings";

/// The default text size of a note's body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl TextSize {
    /// The next size in the Settings pane (Small, Medium, Large, then Small again).
    pub fn next(self) -> Self {
        match self {
            TextSize::Small => TextSize::Medium,
            TextSize::Medium => TextSize::Large,
            TextSize::Large => TextSize::Small,
        }
    }

    /// The name shown in the Settings pane.
    pub fn label(self) -> &'static str {
        match self {
            TextSize::Small => "Small",
            TextSize::Medium => "Medium",
            TextSize::Large => "Large",
        }
    }

    /// Body size in points.
    pub fn body_pt(self) -> f32 {
        match self {
            TextSize::Small => 13.0,
            TextSize::Medium => 15.0,
            TextSize::Large => 18.0,
        }
    }

    fn name(self) -> &'static str {
        match self {
            TextSize::Small => "small",
            TextSize::Medium => "medium",
            TextSize::Large => "large",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [TextSize::Small, TextSize::Medium, TextSize::Large].into_iter().find(|s| s.name() == text)
    }
}

/// The first line a new note is started with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NewNoteStart {
    #[default]
    Title,
    Heading,
    Body,
    Checklist,
}

impl NewNoteStart {
    /// The next choice in the Settings pane (Title, Heading, Body, then Title again). Checklist is not offered there: a
    /// checklist start is set by the Format menu, and a Checklist value from the file cycles to Title.
    pub fn next(self) -> Self {
        match self {
            NewNoteStart::Title => NewNoteStart::Heading,
            NewNoteStart::Heading => NewNoteStart::Body,
            NewNoteStart::Body | NewNoteStart::Checklist => NewNoteStart::Title,
        }
    }

    /// The name shown in the Settings pane.
    pub fn label(self) -> &'static str {
        match self {
            NewNoteStart::Title => "Title",
            NewNoteStart::Heading => "Heading",
            NewNoteStart::Body | NewNoteStart::Checklist => "Body",
        }
    }

    /// The Markdown prefix of the first line of a new note.
    pub fn prefix(self) -> &'static str {
        match self {
            NewNoteStart::Title => "# ",
            NewNoteStart::Heading => "## ",
            NewNoteStart::Body => "",
            NewNoteStart::Checklist => "- [ ] ",
        }
    }

    fn name(self) -> &'static str {
        match self {
            NewNoteStart::Title => "title",
            NewNoteStart::Heading => "heading",
            NewNoteStart::Body => "body",
            NewNoteStart::Checklist => "checklist",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [NewNoteStart::Title, NewNoteStart::Heading, NewNoteStart::Body, NewNoteStart::Checklist].into_iter().find(|s| s.name() == text)
    }
}

/// Tile size of the attachment browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AttachmentSize {
    #[default]
    Small,
    Large,
}

impl AttachmentSize {
    /// Tile width in points.
    pub fn tile_pt(self) -> f32 {
        match self {
            AttachmentSize::Small => 96.0,
            AttachmentSize::Large => 160.0,
        }
    }

    /// How many tiles fit across `width_pt` with `gap_pt` between them; never fewer than one.
    pub fn columns(self, width_pt: f32, gap_pt: f32) -> usize {
        let per = ((width_pt + gap_pt) / (self.tile_pt() + gap_pt)).floor();
        if per.is_finite() && per >= 1.0 {
            per as usize
        } else {
            1
        }
    }

    fn name(self) -> &'static str {
        match self {
            AttachmentSize::Small => "small",
            AttachmentSize::Large => "large",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [AttachmentSize::Small, AttachmentSize::Large].into_iter().find(|s| s.name() == text)
    }
}

/// Every setting of the Notes settings window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Settings {
    pub text_size: TextSize,
    /// Notes are drawn on a dark background (the paper colour follows the appearance when false).
    pub dark_background: bool,
    pub new_note_start: NewNoteStart,
    pub attachment_size: AttachmentSize,
}

/// Reads settings from `text`; unknown keys and values keep their defaults.
pub fn parse(text: &str) -> Settings {
    let mut s = Settings::default();
    for line in text.lines() {
        match line.split_once('=') {
            Some(("text_size", v)) => s.text_size = TextSize::parse(v.trim()).unwrap_or(s.text_size),
            Some(("dark_background", v)) => s.dark_background = v.trim() == "1",
            Some(("new_note_start", v)) => s.new_note_start = NewNoteStart::parse(v.trim()).unwrap_or(s.new_note_start),
            Some(("attachment_size", v)) => s.attachment_size = AttachmentSize::parse(v.trim()).unwrap_or(s.attachment_size),
            _ => {}
        }
    }
    s
}

/// The key=value text of `s`, one setting per line.
pub fn render(s: &Settings) -> String {
    format!(
        "text_size={}\ndark_background={}\nnew_note_start={}\nattachment_size={}\n",
        s.text_size.name(),
        s.dark_background as u8,
        s.new_note_start.name(),
        s.attachment_size.name()
    )
}

/// The settings saved in `dir`, or the defaults when there are none.
pub fn load(dir: &Path) -> Settings {
    std::fs::read_to_string(dir.join(SETTINGS_FILE)).map(|t| parse(&t)).unwrap_or_default()
}

/// Saves `s` in `dir`, atomically.
pub fn save(dir: &Path, s: Settings) -> io::Result<()> {
    springboard::shell::fsutil::write_atomic(&dir.join(SETTINGS_FILE), render(&s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_size_cycles_through_the_three_sizes_and_back() {
        assert_eq!(TextSize::Small.next(), TextSize::Medium);
        assert_eq!(TextSize::Medium.next(), TextSize::Large);
        assert_eq!(TextSize::Large.next(), TextSize::Small);
        assert_eq!(TextSize::Medium.label(), "Medium");
    }

    #[test]
    fn a_new_note_start_cycles_title_heading_body_and_a_checklist_goes_to_title() {
        assert_eq!(NewNoteStart::Title.next(), NewNoteStart::Heading);
        assert_eq!(NewNoteStart::Heading.next(), NewNoteStart::Body);
        assert_eq!(NewNoteStart::Body.next(), NewNoteStart::Title);
        assert_eq!(NewNoteStart::Checklist.next(), NewNoteStart::Title);
        assert_eq!(NewNoteStart::Checklist.label(), "Body");
    }

    #[test]
    fn a_saved_settings_file_reads_back_the_same() {
        let s = Settings { text_size: TextSize::Large, dark_background: true, new_note_start: NewNoteStart::Heading, attachment_size: AttachmentSize::Large };
        assert_eq!(parse(&render(&s)), s);
    }
}
