//! The mail window's state: the accounts and what is selected, the message rows of the open mailbox, the open message,
//! the sheets (account setup and compose) and the arrival baseline for new-mail notices. Nothing here touches the
//! network, the passwords or the window; `window.rs` runs the jobs and feeds their results in.

use crate::discover::Found as Discovered;
use crate::body::Attachment;
use crate::category::{self, Category};
use crate::findbar::FindBar;
use crate::flags::{Flag, FlagColor, Flags};
use crate::message::{self, Draft, Priority, Summary, MAX_ATTACHMENTS, MAX_ATTACHMENT_BYTES};
use crate::prefs::{self, Prefs, SmartMailbox};
use crate::richtext::{self, Style, TextColor};
use crate::rules::{self, Rule};
use crate::schedule::Queue;
use crate::search::{Doc, Query};
use crate::session::Listed;
use crate::threads::{self, Node};
use bifrost_app::controls::internet_accounts::{self as shared, Account, Kind, Security, Server};
use bifrost_app::controls::contact_book::Contact;
use bifrost_app::controls::found_in::{self, Action, Dismissed, Found, Pager};
use bifrost_app::controls::recipient_autocomplete::{Key as RecipientKey, RecipientField, Suggestion};
use bifrost_app::controls::proofread::Options as ProofreadOptions;
use bifrost_app::controls::proofread_panel::{self, Panel};
use bifrost_app::controls::recipient_recents::RecipientRecents;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

pub const INBOX: &str = "INBOX";
/// Most message texts kept at once. Past this the lowest UID values (the oldest mail) are dropped; a dropped text is
/// asked for again when its message is opened.
pub const MAX_CACHED_BODIES: usize = 64;

/// Most bytes the cached texts and their decoded attachments may hold together. The open message and the one that
/// just arrived stay even when they alone are larger.
pub const MAX_CACHED_BYTES: usize = 96 * 1024 * 1024;
/// One line of the list: a message, in the threaded view how many messages its conversation holds, and how far it is
/// indented under the conversation's first message (0 when flat).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub row: Row,
    pub thread: usize,
    pub depth: usize,
    /// True for the first line of a conversation (and for every line of the flat list): the one that folds the others.
    pub head: bool,
}

/// Most lines the activity list keeps.
pub const MAX_ACTIVITY: usize = 80;

/// How many recent recipients Mail keeps for suggestions (the same cap as the recents menu).
pub const MAX_RECIPIENT_RECENTS: usize = 50;

/// The mailbox names Archive and Trash are looked up under, in order (providers name them differently).
pub const ARCHIVE_NAMES: [&str; 4] = ["Archive", "Archives", "All Mail", "[Gmail]/All Mail"];
pub const TRASH_NAMES: [&str; 4] = ["Trash", "Deleted Items", "Deleted Messages", "[Gmail]/Trash"];
pub const JUNK_NAMES: [&str; 5] = ["Junk", "Junk E-mail", "Junk Email", "Spam", "[Gmail]/Spam"];

/// The compose fields in the order they are shown. From shows when the account has other addresses; Cc, Bcc and Reply-To
/// are optional (View menu), and so is Send in (Message > Send Later).
const COMPOSE_ORDER: [&str; 8] = ["From", "To", "Cc", "Bcc", "Reply-To", "Subject", "Send in", "Message"];

/// What the message list is ordered by (View > Sort By). Date is the order the messages arrived in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Date,
    From,
    To,
    Subject,
    /// Flagged messages together (first, when ascending).
    Flagged,
    /// Unread messages together (first, when ascending).
    Unread,
}

impl SortKey {
    pub const ALL: [SortKey; 6] = [SortKey::Date, SortKey::From, SortKey::To, SortKey::Subject, SortKey::Flagged, SortKey::Unread];

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Date => "Date Received",
            SortKey::From => "From",
            SortKey::To => "To",
            SortKey::Subject => "Subject",
            SortKey::Flagged => "Flag",
            SortKey::Unread => "Read Status",
        }
    }
}

/// Which messages the list shows (View > Filter).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Unread,
    Flagged,
    Replied,
    /// Messages that carry files: loaded ones by their files, the others by the type of the message.
    Attachments,
}

impl Filter {
    pub const ALL: [Filter; 5] = [Filter::All, Filter::Unread, Filter::Flagged, Filter::Replied, Filter::Attachments];

    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All Messages",
            Filter::Unread => "Unread",
            Filter::Flagged => "Flagged",
            Filter::Replied => "Replied",
            Filter::Attachments => "With Attachments",
        }
    }

    /// True when a row passes the filter. `loaded_files` is how many files the row's message is known to carry (0 when its
    /// text is not loaded).
    pub fn matches(self, row: &Row, loaded_files: usize) -> bool {
        match self {
            Filter::All => true,
            Filter::Unread => !row.flags.contains(Flag::Seen),
            Filter::Flagged => row.flags.contains(Flag::Flagged),
            Filter::Replied => row.flags.contains(Flag::Answered),
            Filter::Attachments => loaded_files > 0 || row.summary.may_have_files(),
        }
    }
}
/// The IMAP ports that start TLS at once; the other standard ports start with STARTTLS.
const IMAP_TLS_PORT: u16 = 993;
const SMTP_TLS_PORT: u16 = 465;
/// The yes or no field of Add Account that shows the server fields.
pub const MANUAL: &str = "Configure Manually";
/// The fields Configure Manually adds to Add Account, after it.
const MANUAL_LABELS: [&str; 5] = ["User name", "Incoming server", "Incoming port", "Outgoing server", "Outgoing port"];

/// The server fields of Add Account, filled from `found` when there is a result of discovery.
fn manual_fields(found: Option<&Discovered>) -> Vec<Field> {
    let (user, imap, smtp) = match found {
        Some(f) => (f.username.clone().unwrap_or_default(), accounts_server(&Some(f.imap.clone())), accounts_server(&Some(f.smtp.clone()))),
        None => (String::new(), (String::new(), IMAP_TLS_PORT.to_string()), (String::new(), "587".to_string())),
    };
    vec![
        Field::new("User name", "Same as the email when empty", &user),
        Field::new("Incoming server", "imap.<domain> when empty", &imap.0),
        Field::new("Incoming port", "", &imap.1),
        Field::new("Outgoing server", "smtp.<domain> when empty", &smtp.0),
        Field::new("Outgoing port", "", &smtp.1),
    ]
}

/// One message in the list: what the list and the reader need, from its header block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub uid: u32,
    pub flags: Flags,
    pub summary: Summary,
}

/// One line of a sheet. `hint` shows while the value is empty; `secret` hides the value. A `choice` field takes no typed
/// text: a click turns its value between `yes` and `no`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub label: &'static str,
    pub hint: &'static str,
    pub value: String,
    pub secret: bool,
    pub choice: bool,
}

impl Field {
    pub fn new(label: &'static str, hint: &'static str, value: &str) -> Field {
        Field { label, hint, value: value.to_string(), secret: false, choice: false }
    }

    /// A yes or no field, shown as `yes` or `no`.
    pub fn yes_no(label: &'static str, on: bool) -> Field {
        Field { choice: true, ..Field::new(label, "Click to change", if on { "yes" } else { "no" }) }
    }
}

/// The reading of a yes or no field: `yes` is true, anything else false.
pub fn is_yes(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("yes")
}

/// Most matches Open Quickly lists under its field.
pub const MAX_CHOICES: usize = 8;
/// Most suggestions the compose sheet lists under a To, Cc or Bcc field.
pub const MAX_SUGGESTIONS: usize = 5;

/// True for the compose fields that hold a list of recipients and offer suggestions.
pub fn is_recipient_field(label: &str) -> bool {
    matches!(label, "To" | "Cc" | "Bcc")
}

/// Splits the value of a recipient field where the address being typed starts: the part before it (the finished
/// recipients and their separator) and the typed text. A comma or semicolon inside quotes or angle brackets does not
/// separate recipients.
pub fn split_last_recipient(value: &str) -> (&str, &str) {
    let (mut quoted, mut angle) = (false, false);
    let mut cut = 0;
    for (i, c) in value.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ',' | ';' if !quoted && !angle => cut = i + c.len_utf8(),
            _ => {}
        }
    }
    value.split_at(cut)
}

/// The fields of a sheet and the one that takes typed text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    pub fields: Vec<Field>,
    pub focus: usize,
    /// The field whose whole text is selected (Edit > Select All); it counts only while that field has the focus.
    /// Typing, Backspace or Cut replaces or removes the text.
    selection: Option<usize>,
}

impl Form {
    pub fn new(fields: Vec<Field>) -> Form {
        Form { fields, focus: 0, selection: None }
    }

    /// True while the focused field's whole text is selected.
    pub fn selected(&self) -> bool {
        self.selection == Some(self.focus) && self.fields.get(self.focus).is_some_and(|f| !f.value.is_empty())
    }

    /// Edit > Select All: selects the whole text of the focused field. A field that takes no typed text has none to select.
    pub fn select_all(&mut self) -> bool {
        match self.fields.get(self.focus) {
            Some(field) if !field.choice && !field.value.is_empty() => {
                self.selection = Some(self.focus);
                true
            }
            _ => false,
        }
    }

    /// The selected text, for Copy; None when nothing is selected. A secret field (a password) never gives its text up.
    pub fn selected_text(&self) -> Option<&str> {
        self.selected().then(|| &self.fields[self.focus]).filter(|f| !f.secret).map(|f| f.value.as_str())
    }

    /// Edit > Cut: removes the selected text and returns it; None, and nothing is cut, when nothing is selected or the
    /// field is secret.
    pub fn cut(&mut self) -> Option<String> {
        if self.selected_text().is_none() {
            return None;
        }
        self.selection = None;
        Some(std::mem::take(&mut self.fields[self.focus].value))
    }

    /// A press in the field, or any other move of the caret, ends the selection.
    pub fn deselect(&mut self) {
        self.selection = None;
    }

    /// Typed or pasted text. It replaces the selection when there is one.
    pub fn type_text(&mut self, text: &str) {
        let replace = self.selected();
        self.selection = None;
        if let Some(field) = self.fields.get_mut(self.focus) {
            if replace {
                field.value.clear();
            }
            field.value.push_str(text);
        }
    }

    pub fn backspace(&mut self) {
        let replace = self.selected();
        self.selection = None;
        if let Some(field) = self.fields.get_mut(self.focus) {
            if replace {
                field.value.clear();
            } else {
                field.value.pop();
            }
        }
    }

    /// Option+Backspace: the word before the caret (the end of the text), with the blanks after it; the selection when there is one.
    pub fn delete_word(&mut self) {
        let replace = self.selected();
        self.selection = None;
        if let Some(field) = self.fields.get_mut(self.focus) {
            if replace {
                field.value.clear();
                return;
            }
            let kept = field.value.trim_end().trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').len();
            // punctuation stands for a word of its own
            let kept = if kept == field.value.trim_end().len() { field.value.trim_end().trim_end_matches(|c: char| !c.is_alphanumeric() && !c.is_whitespace()).len() } else { kept };
            field.value.truncate(kept);
        }
    }

    /// Cmd+Backspace: everything before the caret, which is all of the field's text (the caret stays at its end).
    pub fn delete_to_start(&mut self) {
        self.selection = None;
        if let Some(field) = self.fields.get_mut(self.focus) {
            field.value.clear();
        }
    }

    /// Tab: the next field, wrapping to the first.
    pub fn next_field(&mut self) {
        if !self.fields.is_empty() {
            self.focus = (self.focus + 1) % self.fields.len();
            self.selection = None;
        }
    }

    pub fn focus_on(&mut self, index: usize) {
        if index < self.fields.len() {
            if index != self.focus {
                self.selection = None;
            }
            self.focus = index;
        }
    }

    pub fn value(&self, label: &str) -> &str {
        self.fields.iter().find(|f| f.label == label).map_or("", |f| f.value.as_str())
    }

    /// Turns the yes or no field at `index` the other way. False when that field is not one.
    pub fn flip(&mut self, index: usize) -> bool {
        match self.fields.get_mut(index) {
            Some(field) if field.choice => {
                field.value = if is_yes(&field.value) { "no".to_string() } else { "yes".to_string() };
                true
            }
            _ => false,
        }
    }
}

/// A new message: the form and, for a reply, the threading headers of the message answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compose {
    pub form: Form,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub priority: Priority,
    /// Files to send with the message (Message > Attach Files).
    pub attachments: Vec<Attachment>,
    /// The signature of the sending account. It is added to the end of the message when the message is sent.
    pub signature: String,
    /// The style of each character of the message field, in step with its text (see [`crate::richtext`]).
    pub styles: Vec<Style>,
    /// The style of the text typed next (the Format menu).
    pub typing: Style,
    /// The misspelled words the last spelling check found, as ranges of characters of the message, and the one in view.
    pub misspelled: Vec<Range<usize>>,
    pub misspelled_at: usize,
    /// Writing > Proofread: the suggestions of the writing assistant for the message, while they are being gone through.
    pub proofread: Option<Panel>,
    /// The Message-ID of this message's draft in Drafts; empty until the first save.
    pub draft_id: String,
    /// How many lines the message field is scrolled up from its end (the scroll wheel, `draw::scroll_message`); 0
    /// shows the end of the message, where the caret is. Typing scrolls back to the end.
    pub scroll_back: usize,
    /// Why the last Send did not go out (an address that is not valid, no password saved, the server's answer). The
    /// compose sheet shows it over its buttons, in the window of the message it is about; an edit clears it.
    pub error: String,
}

impl Compose {
    /// A compose sheet around a form. The message starts in plain style.
    pub fn new(form: Form, in_reply_to: Option<String>, references: Vec<String>, priority: Priority, attachments: Vec<Attachment>, signature: String) -> Compose {
        let mut compose = Compose {
            form,
            in_reply_to,
            references,
            priority,
            attachments,
            signature,
            styles: Vec::new(),
            typing: Style::default(),
            misspelled: Vec::new(),
            misspelled_at: 0,
            proofread: None,
            draft_id: String::new(),
            scroll_back: 0,
            error: String::new(),
        };
        compose.styles = vec![Style::default(); compose.message().chars().count()];
        compose
    }

    /// The text of the message field.
    pub fn message(&self) -> &str {
        self.form.value("Message")
    }

    /// Runs an edit of the field that takes typed text. An edit of the message keeps the styles in step: text added gets the
    /// style being typed in, text taken away takes its styles with it. Any edit clears the spelling marks, which no longer
    /// fit the text.
    pub fn edit(&mut self, change: impl FnOnce(&mut Form)) {
        self.error.clear();
        let in_message = self.form.fields.get(self.form.focus).is_some_and(|f| f.label == "Message");
        if !in_message {
            change(&mut self.form);
            return;
        }
        let before = self.message().chars().count();
        self.styles.resize(before, Style::default());
        // a change over the selected message (typing, a paste) replaces all of it: none of the old runs' styles survive
        let replacing = self.form.selected();
        let old = replacing.then(|| self.message().to_string());
        change(&mut self.form);
        let after = self.message().chars().count();
        if old.is_some_and(|old| old != self.message()) {
            let typing = self.typing;
            self.styles.clear();
            self.styles.extend(std::iter::repeat(typing).take(after));
        } else if after > before {
            let typing = self.typing;
            self.styles.extend(std::iter::repeat(typing).take(after - before));
        } else {
            self.styles.truncate(after);
        }
        self.misspelled.clear();
        self.proofread = None;
    }

    /// Replaces the characters `start..end` of the message with `with`, in `style`. The styles of the rest stay. The
    /// spelling marks and the proofreading suggestions no longer fit the text and are dropped.
    fn splice_message(&mut self, start: usize, end: usize, with: &str, style: Style) {
        self.splice_raw(start, end, with, style);
        self.misspelled.clear();
        self.proofread = None;
    }

    /// `splice_message` without dropping the marks, for edits the caller keeps the marks in step with.
    fn splice_raw(&mut self, start: usize, end: usize, with: &str, style: Style) {
        let chars: Vec<char> = self.message().chars().collect();
        let start = start.min(chars.len());
        let end = end.clamp(start, chars.len());
        let mut text: String = chars[..start].iter().collect();
        text.push_str(with);
        text.extend(chars[end..].iter());
        if let Some(field) = self.form.fields.iter_mut().find(|f| f.label == "Message") {
            field.value = text;
        }
        self.styles.resize(chars.len(), Style::default());
        let added = vec![style; with.chars().count()];
        drop(self.styles.splice(start..end, added));
    }

    /// Format > Bold, Italic and Underline: turn the style of the text typed next on or off.
    pub fn toggle_bold(&mut self) {
        self.typing.bold = !self.typing.bold;
    }

    pub fn toggle_italic(&mut self) {
        self.typing.italic = !self.typing.italic;
    }

    pub fn toggle_underline(&mut self) {
        self.typing.underline = !self.typing.underline;
    }

    /// Format > Text Color: the colour of the text typed next (None is the ordinary colour).
    pub fn set_text_color(&mut self, color: Option<TextColor>) {
        self.typing.color = color;
    }

    /// Format > Clear Formatting: the whole message goes back to plain style, and so does the text typed next.
    pub fn clear_formatting(&mut self) {
        self.styles.iter_mut().for_each(|style| *style = Style::default());
        self.typing = Style::default();
    }

    /// Where the last line of the message starts, in characters.
    fn last_line_start(&self) -> usize {
        let text = self.message();
        let at = text.rfind('\n').map_or(0, |i| i + 1);
        text[..at].chars().count()
    }

    /// Format > Bulleted List: the last line of the message gets a bullet in front of it, or loses it.
    pub fn toggle_bullet(&mut self) {
        let start = self.last_line_start();
        let bulleted = self.message().chars().skip(start).take(2).collect::<String>() == "\u{2022} ";
        if bulleted {
            self.splice_message(start, start + 2, "", Style::default());
        } else {
            self.splice_message(start, start, "\u{2022} ", Style::default());
        }
    }

    /// Format > Increase Indent and Decrease Indent: four spaces more or fewer in front of the last line.
    pub fn indent(&mut self, more: bool) {
        let start = self.last_line_start();
        if more {
            self.splice_message(start, start, "    ", Style::default());
        } else {
            let spaces = self.message().chars().skip(start).take(4).take_while(|c| *c == ' ').count();
            self.splice_message(start, start + spaces, "", Style::default());
        }
    }

    /// Takes the result of a spelling check: the ranges of misspelled words, as characters of the message.
    pub fn set_misspelled(&mut self, ranges: Vec<Range<usize>>) {
        self.misspelled = ranges;
        self.misspelled_at = 0;
    }

    /// The misspelled word in view: its characters and the word.
    pub fn current_misspelling(&self) -> Option<(Range<usize>, String)> {
        let range = self.misspelled.get(self.misspelled_at)?.clone();
        let word = self.message().chars().skip(range.start).take(range.end - range.start).collect();
        Some((range, word))
    }

    /// Moves to the next misspelled word, wrapping at the end.
    pub fn next_misspelling(&mut self) {
        if !self.misspelled.is_empty() {
            self.misspelled_at = (self.misspelled_at + 1) % self.misspelled.len();
        }
    }

    /// Replaces the misspelled word in view with `word`, in the style of its first letter. The spelling marks are cleared
    /// (the text is checked again to find the rest). Returns false when no word is in view.
    pub fn correct_misspelling(&mut self, word: &str) -> bool {
        let Some((range, _)) = self.current_misspelling() else { return false };
        let style = self.styles.get(range.start).copied().unwrap_or_default();
        self.splice_message(range.start, range.end, word, style);
        true
    }

    /// Writing > Proofread: the writing assistant reads the whole message and keeps what it would change. Returns how many
    /// suggestions there are (0 when the message reads fine or is empty).
    pub fn start_proofread(&mut self) -> usize {
        let text = self.message().to_string();
        let options = ProofreadOptions { english: proofread_panel::english_by_letters(&text) };
        self.proofread = Panel::open(&text, 0..0, options).ok().filter(|panel| !panel.is_empty());
        self.proofread.as_ref().map_or(0, Panel::len)
    }

    /// The suggestions as ranges of characters of the message, and the index of the one in view.
    pub fn proofread_marks(&self) -> (Vec<Range<usize>>, usize) {
        let Some(panel) = &self.proofread else { return (Vec::new(), 0) };
        let text = self.message();
        let chars = |bytes: usize| text.get(..bytes).map_or(0, |head| head.chars().count());
        (panel.suggestions().iter().map(|s| chars(s.range.start)..chars(s.range.end)).collect(), panel.at())
    }

    /// The line for the status bar: which suggestion is in view and what it would change.
    pub fn proofread_line(&self) -> Option<String> {
        let panel = self.proofread.as_ref()?;
        let what = panel.describe(self.message(), panel.at())?;
        Some(format!("Suggestion {} of {}. {what}", panel.at() + 1, panel.len()))
    }

    /// Writing > Accept Suggestion (or Accept All Suggestions): the change is made in the message, keeping the style of
    /// the text it replaces, and the suggestions that are left move with it. Returns how many changes were made.
    pub fn accept_suggestion(&mut self, all: bool) -> usize {
        let Some(mut panel) = self.proofread.take() else { return 0 };
        let mut text = self.message().to_string();
        let steps = if all { panel.accept_all(&text) } else { panel.accept(&text).map(|(step, _)| vec![step]) };
        let Ok(steps) = steps else { return 0 };
        let made = steps.len();
        for step in steps {
            let (start, end) = (text[..step.range.start].chars().count(), text[..step.range.end].chars().count());
            let style = self.styles.get(start).copied().unwrap_or_default();
            self.splice_raw(start, end, &step.replacement, style);
            text = self.message().to_string();
        }
        self.misspelled.clear();
        if !panel.is_empty() {
            self.proofread = Some(panel);
        }
        made
    }

    /// Writing > Skip Suggestion: the one in view is left as it is and the next comes into view.
    pub fn skip_suggestion(&mut self) {
        if let Some(panel) = self.proofread.as_mut() {
            let _ = panel.skip();
            if panel.is_empty() {
                self.proofread = None;
            }
        }
    }

    /// Adds a file to send with the message. Refused past [`MAX_ATTACHMENTS`] files or [`MAX_ATTACHMENT_BYTES`] bytes.
    pub fn attach(&mut self, file: Attachment) -> Result<(), String> {
        if self.attachments.len() >= MAX_ATTACHMENTS {
            return Err(format!("A message carries at most {MAX_ATTACHMENTS} files"));
        }
        let total: usize = self.attachments.iter().map(|a| a.bytes.len()).sum::<usize>() + file.bytes.len();
        if total > MAX_ATTACHMENT_BYTES {
            return Err(format!("The files are over the {} MB a message may carry", MAX_ATTACHMENT_BYTES / (1024 * 1024)));
        }
        self.attachments.push(file);
        Ok(())
    }

    /// Takes the file at `index` off the message.
    pub fn detach(&mut self, index: usize) {
        if index < self.attachments.len() {
            self.attachments.remove(index);
        }
    }

    /// Moves the From field to the next address of `choices`, wrapping at the end. The first address is the account's own.
    pub fn cycle_from(&mut self, choices: &[String]) {
        let Some(field) = self.form.fields.iter_mut().find(|f| f.label == "From") else { return };
        let next = choices.iter().position(|c| c.eq_ignore_ascii_case(field.value.trim())).map_or(0, |at| (at + 1) % choices.len().max(1));
        if let Some(choice) = choices.get(next) {
            field.value = choice.clone();
        }
    }

    /// Shows or hides one of the optional address fields (Cc, Bcc, Reply-To), keeping the others and the typed text.
    pub fn set_field(&mut self, label: &'static str, hint: &'static str, shown: bool) {
        let at = self.form.fields.iter().position(|f| f.label == label);
        match (at, shown) {
            (None, true) => {
                let rank = COMPOSE_ORDER.iter().position(|l| *l == label).unwrap_or(0);
                let before = self
                    .form
                    .fields
                    .iter()
                    .take_while(|f| COMPOSE_ORDER.iter().position(|l| *l == f.label).is_some_and(|r| r < rank))
                    .count();
                self.form.fields.insert(before, Field::new(label, hint, ""));
                if self.form.focus >= before {
                    self.form.focus += 1;
                }
            }
            (Some(index), false) => {
                self.form.fields.remove(index);
                if self.form.focus > index {
                    self.form.focus -= 1;
                }
                self.form.focus = self.form.focus.min(self.form.fields.len().saturating_sub(1));
            }
            _ => {}
        }
    }
}

/// The sheet that names a mailbox: a new one, or `rename_from` under a new name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailboxSheet {
    pub form: Form,
    pub rename_from: Option<String>,
}

/// What a [`Prompt`] asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptKind {
    /// Remind Me: how long until the message comes back.
    Remind,
    /// Redirect: the new recipients.
    Redirect,
    /// New Smart Mailbox: a name and a search.
    SmartMailbox,
    /// New Rule.
    Rule,
    /// Edit Rule: the rule at this place in the list.
    EditRule(usize),
    /// Signature, other addresses and undo-send delay of the shown account.
    Settings,
    /// Account Settings: the setup form of the account with this id.
    EditAccount(String),
    /// Remove Account: the name of the account with this id must be typed.
    RemoveAccount(String),
    /// Show Activity: what Mail did lately, to read.
    Activity,
}

/// A small sheet with a few fields and one action (see [`PromptKind`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub kind: PromptKind,
    pub form: Form,
}

/// The sheet over the window, if one is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sheet {
    Setup(Form),
    Compose(Compose),
    /// The Find sheet: one field, the search text.
    Search(Form),
    /// New Mailbox and Rename Mailbox.
    Mailbox(MailboxSheet),
    /// Open Quickly: one field; Return opens the first mailbox whose name contains the text.
    Quickly(Form),
    Prompt(Prompt),
    /// Closing a message that has text: Save This Message, Don't Save, or Cancel (back to the message).
    Question(Compose),
    /// Mailbox > Rules: the form is empty; the names are the rules' lines, in order (see `rules::describe`).
    Rules(Form, Vec<String>),
}

impl Sheet {
    /// The fields the sheet shows.
    pub fn form(&self) -> &Form {
        match self {
            Sheet::Setup(form) | Sheet::Search(form) | Sheet::Quickly(form) => form,
            Sheet::Compose(compose) | Sheet::Question(compose) => &compose.form,
            Sheet::Mailbox(sheet) => &sheet.form,
            Sheet::Prompt(prompt) => &prompt.form,
            Sheet::Rules(form, _) => form,
        }
    }

    pub fn form_mut(&mut self) -> &mut Form {
        match self {
            Sheet::Setup(form) | Sheet::Search(form) | Sheet::Quickly(form) => form,
            Sheet::Compose(compose) | Sheet::Question(compose) => &mut compose.form,
            Sheet::Mailbox(sheet) => &mut sheet.form,
            Sheet::Prompt(prompt) => &mut prompt.form,
            Sheet::Rules(form, _) => form,
        }
    }

    /// The title at the top of the sheet.
    pub fn title(&self) -> &'static str {
        match self {
            Sheet::Search(_) => "Find",
            Sheet::Quickly(_) => "Open Quickly",
            Sheet::Setup(_) => "Add Account",
            Sheet::Compose(compose) if compose.in_reply_to.is_some() => "Reply",
            Sheet::Compose(_) => "New Message",
            Sheet::Question(_) => "Save This Message?",
            Sheet::Rules(..) => "Rules",
            Sheet::Mailbox(sheet) if sheet.rename_from.is_some() => "Rename Mailbox",
            Sheet::Mailbox(_) => "New Mailbox",
            Sheet::Prompt(prompt) => match prompt.kind {
                PromptKind::Remind => "Remind Me",
                PromptKind::Redirect => "Redirect",
                PromptKind::SmartMailbox => "New Smart Mailbox",
                PromptKind::Rule => "New Rule",
                PromptKind::EditRule(_) => "Edit Rule",
                PromptKind::Settings => "Mail Settings",
                PromptKind::EditAccount(_) => "Account Settings",
                PromptKind::RemoveAccount(_) => "Remove Account",
                PromptKind::Activity => "Activity",
            },
        }
    }

    /// The label of the primary button.
    pub fn action(&self) -> &'static str {
        match self {
            Sheet::Search(_) => "Search",
            Sheet::Quickly(_) => "Go",
            Sheet::Setup(_) => "Add Account",
            Sheet::Compose(_) => "Send",
            Sheet::Question(_) => "Save",
            Sheet::Rules(..) => "Done",
            Sheet::Mailbox(sheet) if sheet.rename_from.is_some() => "Rename",
            Sheet::Mailbox(_) => "Create",
            Sheet::Prompt(prompt) => match prompt.kind {
                PromptKind::Remind => "Remind Me",
                PromptKind::Redirect => "Redirect",
                PromptKind::SmartMailbox | PromptKind::Rule => "Add",
                PromptKind::Settings | PromptKind::EditAccount(_) | PromptKind::EditRule(_) => "Save",
                PromptKind::RemoveAccount(_) => "Remove",
                PromptKind::Activity => "Done",
            },
        }
    }
}

/// True for the fields that take several lines of text.
pub fn is_multiline(label: &str) -> bool {
    label == "Message" || label == "Signature" || label == "Activity"
}

/// A message that arrived since the last listing of the inbox, for a notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub account: String,
    pub uid: u32,
    pub from: String,
    pub subject: String,
}

/// The messages of a listing that are newer than `seen`, the highest UID already known. With nothing known yet (the
/// first listing) nothing is new: that listing only sets the baseline.
pub fn arrivals(seen: Option<u32>, listed: &[Listed]) -> Vec<&Listed> {
    match seen {
        None => Vec::new(),
        Some(seen) => listed.iter().filter(|m| m.uid > seen).collect(),
    }
}

/// What is known of an inbox between listings: the highest UID seen and the UIDVALIDITY it belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    pub uid: u32,
    pub validity: Option<u32>,
}

/// Takes one inbox listing against what was known (`old`). Returns the UID to compare arrivals with (None: nothing in
/// this listing is new) and the mark after the listing.
///
/// A listing with a different UIDVALIDITY than before starts the mark over without announcing anything: the server has
/// renumbered the mailbox, so the old UIDs say nothing. An empty inbox sets the mark to 0, so the first message that
/// arrives later is new.
pub fn advance(old: Option<Mark>, validity: Option<u32>, listed: &[Listed]) -> (Option<u32>, Mark) {
    let top = listed.iter().map(|m| m.uid).max().unwrap_or(0);
    let renumbered = old.is_some_and(|o| matches!((o.validity, validity), (Some(a), Some(b)) if a != b));
    match old {
        Some(o) if !renumbered => (Some(o.uid), Mark { uid: o.uid.max(top), validity: validity.or(o.validity) }),
        _ => (None, Mark { uid: top, validity }),
    }
}

/// Listed messages as rows, the newest (highest UID) first.
fn newest_first(messages: Vec<Listed>) -> Vec<Row> {
    let mut rows: Vec<Row> = messages.into_iter().map(|m| Row { uid: m.uid, flags: m.flags, summary: Summary::from_headers(&m.header) }).collect();
    rows.sort_by(|a, b| b.uid.cmp(&a.uid));
    rows
}

pub struct Mail {
    pub accounts: Vec<Account>,
    /// The index of the account shown in the sidebar.
    pub account: usize,
    pub mailboxes: Vec<String>,
    pub mailbox: String,
    /// The server's Drafts mailbox by special use, from the last listing; `None` until one is read or when none is flagged.
    pub drafts_mailbox: Option<String>,
    /// The messages of the shown mailbox, newest first.
    pub rows: Vec<Row>,
    /// The UID of the message open in the reader: the selected message, or the one clicked last into a selection of
    /// several (`selection`).
    pub open: Option<u32>,
    /// The messages selected together with Cmd-click and Shift-click (the reference table view's multiple selection), in the order
    /// they were selected. Empty while one message or none is selected: `open` is the selection then. Never one long.
    selection: Vec<u32>,
    /// The message a Shift-click or Shift-arrow extends the selection from: the last one clicked on its own.
    anchor: Option<u32>,
    /// Message text by UID, for the shown mailbox only.
    pub bodies: HashMap<u32, String>,
    /// Passwords read from the Passwords group for this run. They are never written to the shared account list.
    pub passwords: HashMap<String, String>,
    /// The highest inbox UID seen per account id. Absent until the first inbox listing of the account.
    pub baseline: HashMap<String, u32>,
    /// The UIDVALIDITY each baseline belongs to, for the accounts whose server gave one.
    pub validity: HashMap<String, u32>,
    pub sheet: Option<Sheet>,
    /// Set when a compose was made into the sheet slot (open_draft, a failed send, Undo Send). The app moves it into a
    /// compose window of its own (see `compose_windows`).
    pub fresh_compose: bool,
    /// Messages whose remote content the person allowed with Load Remote Content, by UID. Not kept between runs.
    pub remote_loaded: HashSet<u32>,
    /// The sheet slot was closed on purpose (sent, saved or discarded), so its compose window closes without a question.
    pub sheet_done: bool,
    pub status: String,
    /// Add Account is looking up the servers of the address: the sheet takes no second submit until the answer is here.
    pub discovering: bool,
    /// The search text as applied. Empty shows every message.
    pub search: String,
    /// The toolbar's search field as typed, with the spaces `search` trims; the field shows it while it still matches `search`.
    pub search_text: String,
    /// The toolbar's search field has the keyboard (a click on it, Edit > Find).
    pub search_focused: bool,
    /// The threaded view: the list groups conversations together.
    pub threaded: bool,
    /// The files of each message loaded for the shown mailbox, by UID.
    pub attachments: HashMap<u32, Vec<Attachment>>,
    /// The messages the server found for the applied search in the shown mailbox (`SEARCH` on the server), newest
    /// first. None until the server answers; the list then shows the loaded rows that match.
    pub found: Option<Vec<Row>>,
    /// The list order (View > Sort By): the key, and whether it runs ascending (oldest first for Date, A to Z otherwise).
    pub sort: SortKey,
    pub ascending: bool,
    /// Which messages the list shows (View > Filter).
    pub filter: Filter,
    /// Conversations folded to their first message in the threaded view, by the UID of that first message.
    pub collapsed: HashSet<u32>,
    /// The mailbox the last Move To went to, for Move Again.
    pub last_target: Option<String>,
    /// Which optional address fields a new message shows (View > Cc/Bcc/Reply-To Address Field).
    pub show_cc: bool,
    pub show_bcc: bool,
    pub show_reply_to: bool,
    /// Conversations whose new mail raises no notification (Message > Mute), by [`crate::mute::key`].
    pub muted: HashSet<String>,
    /// View > Show Sidebar.
    pub show_sidebar: bool,
    /// View > Raw Source: the reader shows the message as the server holds it, headers and all.
    pub show_raw: bool,
    /// What Mail remembers between runs (see [`crate::prefs`]).
    pub prefs: Prefs,
    /// Messages waiting to be sent and reminders (see [`crate::schedule`]).
    pub queue: Queue,
    /// View > Category: the list shows only this category of message.
    pub category: Option<Category>,
    /// The smart mailbox whose search is applied (an index into `prefs.smart`).
    pub smart_active: Option<usize>,
    /// The find bar over the reader (Edit > Find in Message), when it is open.
    pub find: Option<FindBar>,
    /// The messages that arrived in an inbox with the last listing, with their account id, for the rules.
    pub arrived: Vec<(String, Row)>,
    /// What Mail did lately, oldest first: each line starts with the time (Mailbox > Show Activity).
    pub activity: Vec<String>,
    /// The addresses Mail sent to, most recent first, offered again as suggestions when a recipient is typed. Kept for
    /// this run only.
    pub recipients: RecipientRecents,
    /// The people of the address book, as far as recipient suggestions need them (names and email addresses). Filled
    /// from the Contacts service once a compose sheet is open; empty when the person has not let Mail read Contacts.
    pub address_book: Vec<Contact>,
    /// The suggestions the focused To, Cc or Bcc field offers, with the one highlighted by the arrow keys.
    pub recipient_field: RecipientField,
    /// The messages whose people were donated to Contacts already (account, mailbox, UID): a message is donated once.
    pub donated: HashSet<(String, String, u32)>,
    /// The suggestions found in the open message (events, flights, contacts, addresses), paged one at a time, and the
    /// message they belong to. Filled once the message text is here.
    pub found_pager: Option<(u32, Pager)>,
    /// The suggestions the person turned down or took, for this run: they are not offered again from the same message.
    pub found_dismissed: Dismissed,
    /// The message selected in each mailbox the person left, by (account id, mailbox): coming back selects it again.
    pub remembered: HashMap<(String, String), u32>,
    /// The message to select again once the shown mailbox's listing arrives (from `remembered`).
    restore: Option<u32>,
    /// The listing brought back the message selected when the mailbox was left: the window opens it (and fetches its
    /// text) and takes this.
    pub reopen: Option<u32>,
    /// The message the reader last marked as read when it was shown. Showing it again (a selection change around it, a
    /// listing that keeps it open) does not mark it again, so Message > Mark as Unread on the open message holds until
    /// another message is shown and it is opened again.
    marked_seen: Option<u32>,
    /// The last rows of each mailbox not shown, by (account id, mailbox): a mailbox listed before shows them at once
    /// when it is shown again and its listing replaces them in place (only a mailbox never listed shows Loading).
    /// Kept for this run only.
    listings: HashMap<(String, String), Vec<Row>>,
}

impl Mail {
    pub fn new(accounts: Vec<Account>) -> Mail {
        Mail {
            accounts,
            account: 0,
            mailboxes: vec![INBOX.to_string()],
            mailbox: INBOX.to_string(),
            drafts_mailbox: None,
            rows: Vec::new(),
            open: None,
            selection: Vec::new(),
            anchor: None,
            bodies: HashMap::new(),
            passwords: HashMap::new(),
            baseline: HashMap::new(),
            validity: HashMap::new(),
            sheet: None,
            fresh_compose: false,
            remote_loaded: HashSet::new(),
            sheet_done: false,
            status: String::new(),
            discovering: false,
            search: String::new(),
            search_text: String::new(),
            search_focused: false,
            threaded: false,
            attachments: HashMap::new(),
            found: None,
            sort: SortKey::Date,
            ascending: false,
            filter: Filter::All,
            collapsed: HashSet::new(),
            last_target: None,
            show_cc: true,
            show_bcc: false,
            show_reply_to: false,
            muted: HashSet::new(),
            show_sidebar: true,
            show_raw: false,
            prefs: Prefs::default(),
            queue: Queue::default(),
            category: None,
            smart_active: None,
            find: None,
            arrived: Vec::new(),
            activity: Vec::new(),
            recipients: RecipientRecents::new(MAX_RECIPIENT_RECENTS),
            address_book: Vec::new(),
            recipient_field: RecipientField::new(),
            donated: HashSet::new(),
            found_pager: None,
            found_dismissed: Dismissed::default(),
            remembered: HashMap::new(),
            restore: None,
            reopen: None,
            marked_seen: None,
            listings: HashMap::new(),
        }
    }

    /// Adds a line to the activity list, dropping the oldest past [`MAX_ACTIVITY`].
    pub fn log(&mut self, line: String) {
        self.activity.push(line);
        if self.activity.len() > MAX_ACTIVITY {
            let extra = self.activity.len() - MAX_ACTIVITY;
            self.activity.drain(..extra);
        }
    }

    /// Mailbox > Show Activity: a sheet with the activity list, the newest line last.
    pub fn open_activity(&mut self) {
        let text = if self.activity.is_empty() { "Nothing has happened yet".to_string() } else { self.activity.join("\n") };
        self.open_prompt(PromptKind::Activity, vec![Field::new("Activity", "", &text)]);
    }

    /// Takes the saved settings: the optional address fields of new messages come from them too.
    pub fn use_prefs(&mut self, prefs: Prefs) {
        self.show_cc = prefs.show_cc;
        self.show_bcc = prefs.show_bcc;
        self.show_reply_to = prefs.show_reply_to;
        self.prefs = prefs;
    }

    /// The settings to save: the optional address fields are copied in first.
    pub fn prefs_to_save(&mut self) -> &Prefs {
        self.prefs.show_cc = self.show_cc;
        self.prefs.show_bcc = self.show_bcc;
        self.prefs.show_reply_to = self.show_reply_to;
        &self.prefs
    }

    pub fn current(&self) -> Option<&Account> {
        self.accounts.get(self.account)
    }

    pub fn password(&self, account: &Account) -> Option<&str> {
        self.passwords.get(&account.id).map(String::as_str)
    }

    /// Shows another account: its inbox is selected and the list starts empty until its listing arrives.
    pub fn show_account(&mut self, index: usize) {
        if index >= self.accounts.len() {
            return;
        }
        // the old account's selection and rows are kept under its own id before the account changes
        self.remember_selection();
        self.stash_rows();
        self.open = None;
        self.clear_selection();
        self.account = index;
        self.mailboxes = vec![INBOX.to_string()];
        self.enter_mailbox(INBOX);
    }

    /// Keeps the rows of the shown mailbox, for showing them at once when it is shown again. An empty list is not
    /// kept: it looks the same as a mailbox never listed.
    fn stash_rows(&mut self) {
        let Some(account) = self.current().map(|a| a.id.clone()) else { return };
        let rows = std::mem::take(&mut self.rows);
        if !rows.is_empty() {
            self.listings.insert((account, self.mailbox.clone()), rows);
        }
    }

    /// Keeps the selected message of the shown mailbox, for coming back to it.
    fn remember_selection(&mut self) {
        let Some(account) = self.current().map(|a| a.id.clone()) else { return };
        if let Some(uid) = self.open {
            self.remembered.insert((account, self.mailbox.clone()), uid);
        }
    }

    /// Shows mailbox `name` of the shown account. The selection and rows of the mailbox left are kept. A mailbox listed
    /// before shows its last rows at once, with the message it had selected, and the listing asked for next refreshes
    /// them in place (as the reference mail app keeps a list and a selection per mailbox).
    pub fn show_mailbox(&mut self, name: &str) {
        self.remember_selection();
        self.stash_rows();
        self.enter_mailbox(name);
    }

    /// Shows mailbox `name` of the shown account from what is kept of it, without keeping anything of the one left.
    fn enter_mailbox(&mut self, name: &str) {
        let account = self.current().map(|a| a.id.clone()).unwrap_or_default();
        let key = (account, name.to_string());
        let remembered = self.remembered.get(&key).copied();
        self.reopen = None;
        self.marked_seen = None;
        self.mailbox = name.to_string();
        self.rows = self.listings.remove(&key).unwrap_or_default();
        // the remembered message comes back now when the kept rows have it, else with the first listing
        self.restore = None;
        match remembered {
            Some(uid) if self.rows.iter().any(|r| r.uid == uid) => self.reopen = Some(uid),
            other => self.restore = other,
        }
        self.bodies.clear();
        self.attachments.clear();
        self.found = None;
        self.open = None;
        self.clear_selection();
        self.collapsed.clear();
    }

    /// Takes the result of a listing. Returns the notices for new inbox messages. The rows change only when the listing
    /// is for the shown account and mailbox.
    pub fn apply_listing(&mut self, account_id: &str, mailbox: &str, mailboxes: Vec<String>, messages: Vec<Listed>) -> Vec<Notice> {
        self.apply_listing_of(account_id, mailbox, None, mailboxes, messages)
    }

    /// Like [`Mail::apply_listing`], with the UIDVALIDITY the server gave for the mailbox.
    pub fn apply_listing_of(&mut self, account_id: &str, mailbox: &str, validity: Option<u32>, mailboxes: Vec<String>, messages: Vec<Listed>) -> Vec<Notice> {
        let mut notices = Vec::new();
        self.arrived.clear();
        if mailbox.eq_ignore_ascii_case(INBOX) {
            let old = self.baseline.get(account_id).map(|&uid| Mark { uid, validity: self.validity.get(account_id).copied() });
            let (seen, mark) = advance(old, validity, &messages);
            for m in arrivals(seen, &messages) {
                let summary = Summary::from_headers(&m.header);
                self.arrived.push((account_id.to_string(), Row { uid: m.uid, flags: m.flags, summary: summary.clone() }));
                if self.muted.contains(&crate::mute::key(&summary)) {
                    continue;
                }
                notices.push(Notice { account: account_id.to_string(), uid: m.uid, from: summary.from.clone(), subject: summary.subject.clone() });
            }
            self.baseline.insert(account_id.to_string(), mark.uid);
            match mark.validity {
                Some(v) => self.validity.insert(account_id.to_string(), v),
                None => self.validity.remove(account_id),
            };
        }
        let shown = self.current().is_some_and(|a| a.id == account_id) && self.mailbox == mailbox;
        if shown {
            if !mailboxes.is_empty() {
                self.mailboxes = mailboxes;
            }
            self.rows = newest_first(messages);
            // a selected message the server no longer lists leaves the selection
            let listed: HashSet<u32> = self.rows.iter().map(|r| r.uid).collect();
            if !self.selection.is_empty() {
                let kept: Vec<u32> = self.selection.iter().copied().filter(|uid| listed.contains(uid)).collect();
                let open = self.open.filter(|uid| kept.contains(uid)).or_else(|| kept.last().copied());
                self.set_selection(kept, open);
            }
            // the first listing after coming back selects the message the mailbox had, when it is still there and
            // nothing was clicked meanwhile
            if let Some(uid) = self.restore.take() {
                if self.open.is_none() && self.rows.iter().any(|r| r.uid == uid) {
                    self.reopen = Some(uid);
                }
            }
        } else if let Some(account) = self.accounts.iter().find(|a| a.id == account_id) {
            // a mailbox not shown keeps its listing for when it is shown
            self.listings.insert((account.id.clone(), mailbox.to_string()), newest_first(messages));
        }
        notices
    }

    /// Takes the server's answer to the applied search. It is kept only while the same search is applied to the same
    /// account and mailbox; an answer for an older search is dropped.
    pub fn apply_found(&mut self, account_id: &str, mailbox: &str, query: &str, messages: Vec<Listed>) {
        let current = self.current().is_some_and(|a| a.id == account_id) && self.mailbox == mailbox && self.search == query;
        if current {
            self.found = Some(newest_first(messages));
        }
    }

    /// The row of a message the list shows: from the server's answer when a search is applied, else from the listing.
    pub fn row(&self, uid: u32) -> Option<&Row> {
        self.found.iter().flatten().chain(self.rows.iter()).find(|r| r.uid == uid)
    }

    /// Opens a message in the reader, as the only one selected (a plain click, an arrow key). Returns true when its text
    /// is not loaded yet.
    pub fn open_message(&mut self, uid: u32) -> bool {
        self.clear_selection();
        self.anchor = Some(uid);
        self.show(uid)
    }

    /// Shows a message in the reader without changing which others are selected. True when its text is not loaded yet.
    fn show(&mut self, uid: u32) -> bool {
        self.open = Some(uid);
        if self.marked_seen != Some(uid) {
            self.marked_seen = Some(uid);
            self.mark_seen_locally(uid);
        }
        !self.bodies.contains_key(&uid)
    }

    fn clear_selection(&mut self) {
        self.selection.clear();
        self.anchor = None;
    }

    /// Makes `set` the selection with `open` in the reader. One message (or none) is the plain selection: `open` alone.
    fn set_selection(&mut self, mut set: Vec<u32>, open: Option<u32>) {
        let mut seen = HashSet::new();
        set.retain(|uid| seen.insert(*uid));
        if set.len() <= 1 {
            self.selection.clear();
            self.open = set.first().copied();
        } else {
            self.open = open.filter(|uid| set.contains(uid)).or_else(|| set.last().copied());
            self.selection = set;
        }
    }

    /// Every selected message: the selection of several, or the open message alone. The actions of the toolbar, the
    /// menus and the context menu (Archive, Move to Trash, Move To, Flag, Mark as Unread) act on all of them.
    pub fn selected(&self) -> Vec<u32> {
        if self.selection.len() > 1 {
            self.selection.clone()
        } else {
            self.open.into_iter().collect()
        }
    }

    /// True when message `uid` is selected (its row wears the selection plate).
    pub fn is_selected(&self, uid: u32) -> bool {
        if self.selection.len() > 1 {
            self.selection.contains(&uid)
        } else {
            self.open == Some(uid)
        }
    }

    /// How many messages are selected together; 0 or 1 is the plain selection. Above 1 the reader says so.
    pub fn selected_count(&self) -> usize {
        if self.selection.len() > 1 {
            self.selection.len()
        } else {
            usize::from(self.open.is_some())
        }
    }

    /// Cmd-click on message `uid`: it joins the selection, or leaves it when it was selected. A message clicked into the
    /// selection is shown in the reader and becomes the anchor of the next Shift-click; when the shown one leaves, the last
    /// one selected before it is shown, and the shown message is the anchor. Returns the message whose text must be
    /// fetched, if any.
    pub fn toggle_selected(&mut self, uid: u32) -> Option<u32> {
        let mut set = self.selected();
        let added = !set.contains(&uid);
        let open = if added {
            set.push(uid);
            Some(uid)
        } else {
            set.retain(|u| *u != uid);
            if self.open == Some(uid) { set.last().copied() } else { self.open }
        };
        self.set_selection(set, open);
        self.anchor = if added { Some(uid) } else { self.open };
        let shown = self.open?;
        self.show(shown).then_some(shown)
    }

    /// Shift-click on message `uid` (or Shift-arrow onto it): the selection becomes every line of the list from the
    /// anchor to `uid`, and `uid` is shown. The anchor stays, so the next Shift-click redraws the range from it. With no
    /// anchor in the list, `uid` alone is selected. Returns the message whose text must be fetched, if any.
    pub fn extend_selection(&mut self, uid: u32) -> Option<u32> {
        let lines: Vec<u32> = self.visible_lines().iter().map(|l| l.row.uid).collect();
        let Some(to) = lines.iter().position(|u| *u == uid) else { return None };
        let from = self.anchor.or(self.open).and_then(|a| lines.iter().position(|u| *u == a));
        let Some(from) = from else {
            return self.open_message(uid).then_some(uid);
        };
        let range = lines[from.min(to)..=from.max(to)].to_vec();
        let anchor = self.anchor.or(self.open);
        self.set_selection(range, Some(uid));
        self.anchor = anchor;
        self.show(uid).then_some(uid)
    }

    /// Edit > Select All in the message list: every line the list shows. The open message stays in the reader when it
    /// is listed. Returns the message whose text must be fetched, if any.
    pub fn select_all_messages(&mut self) -> Option<u32> {
        let lines: Vec<u32> = self.visible_lines().iter().map(|l| l.row.uid).collect();
        let open = self.open.filter(|uid| lines.contains(uid)).or_else(|| lines.first().copied());
        let anchor = self.anchor;
        self.set_selection(lines, open);
        self.anchor = anchor.or(self.open);
        let shown = self.open?;
        self.show(shown).then_some(shown)
    }

    /// Shift-Down Arrow (`down`) or Shift-Up Arrow: the selection grows or shrinks by the line below or above the shown
    /// message, from the anchor. None at either end of the list; otherwise the line's index and whether its text must be
    /// fetched.
    pub fn extend_by_arrow(&mut self, down: bool) -> Option<(usize, Option<u32>)> {
        if self.anchor.is_none() {
            self.anchor = self.open;
        }
        let (index, uid) = self.neighbour(down)?;
        Some((index, self.extend_selection(uid)))
    }

    /// The message Down Arrow (`down`) or Up Arrow selects in the list: the line below or above the selected one, and
    /// the first (Down) or the last (Up) line with nothing selected. None at either end, or for an empty list.
    pub fn neighbour(&self, down: bool) -> Option<(usize, u32)> {
        let lines = self.visible_lines();
        let at = self.open.and_then(|uid| lines.iter().position(|l| l.row.uid == uid));
        let next = match (at, down) {
            (None, true) => 0,
            (None, false) => lines.len().checked_sub(1)?,
            (Some(i), true) => i + 1,
            (Some(i), false) => i.checked_sub(1)?,
        };
        lines.get(next).map(|l| (next, l.row.uid))
    }

    pub fn mark_seen_locally(&mut self, uid: u32) {
        self.set_flag_locally(uid, Flag::Seen, true);
    }

    /// Sets a flag on a row at once, before the server answers.
    pub fn set_flag_locally(&mut self, uid: u32, flag: Flag, on: bool) {
        for row in self.rows.iter_mut().chain(self.found.iter_mut().flatten()).filter(|r| r.uid == uid) {
            row.flags = row.flags.with(flag, on);
            if flag == Flag::Flagged && !on {
                row.flags = row.flags.without_color();
            }
        }
    }

    /// Flags a row in `color` at once, or clears its flag and colour (`None`), before the server answers.
    pub fn set_color_locally(&mut self, uid: u32, color: Option<FlagColor>) {
        for row in self.rows.iter_mut().chain(self.found.iter_mut().flatten()).filter(|r| r.uid == uid) {
            row.flags = match color {
                Some(color) => row.flags.without_color().with_color(color),
                None => row.flags.with(Flag::Flagged, false).without_color(),
            };
        }
    }

    /// The flags of a row in the shown mailbox, when it is listed.
    pub fn flags_of(&self, uid: u32) -> Option<Flags> {
        self.row(uid).map(|r| r.flags)
    }

    /// Takes a message out of the shown list, because it moved to another mailbox. An open message is closed.
    pub fn remove_row(&mut self, uid: u32) {
        self.rows.retain(|r| r.uid != uid);
        if let Some(found) = &mut self.found {
            found.retain(|r| r.uid != uid);
        }
        self.bodies.remove(&uid);
        self.attachments.remove(&uid);
        if self.anchor == Some(uid) {
            self.anchor = None;
        }
        if self.selection.contains(&uid) {
            let kept: Vec<u32> = self.selection.iter().copied().filter(|u| *u != uid).collect();
            let open = self.open.filter(|u| *u != uid);
            self.set_selection(kept, open);
        } else if self.open == Some(uid) {
            self.open = None;
        }
    }

    /// The lines the list shows. Threaded, the matching messages are grouped by conversation (`threads::group`): each
    /// conversation is listed together, the newest conversation first, in tree order inside it (`threads::arrange`):
    /// a reply sits under the message it answers, indented by its depth.
    pub fn visible_lines(&self) -> Vec<Line> {
        if !self.threaded {
            return self.visible_rows().into_iter().map(|row| Line { row, thread: 1, depth: 0, head: true }).collect();
        }
        let mut lines = Vec::new();
        for conversation in self.conversations() {
            let folded = conversation.len() > 1 && conversation.first().is_some_and(|l| self.collapsed.contains(&l.row.uid));
            if folded {
                lines.extend(conversation.into_iter().take(1));
            } else {
                lines.extend(conversation);
            }
        }
        lines
    }

    /// The threaded view's conversations, each in tree order, whether folded or not.
    fn conversations(&self) -> Vec<Vec<Line>> {
        let rows = self.visible_rows();
        let nodes: Vec<Node> = rows
            .iter()
            .map(|r| Node {
                message_id: r.summary.message_id.clone(),
                in_reply_to: r.summary.in_reply_to.clone(),
                references: r.summary.references.clone(),
                subject: r.summary.subject.clone(),
            })
            .collect();
        let mut conversations = Vec::new();
        for group in threads::group(&nodes) {
            let lines: Vec<Line> = threads::arrange(&nodes, &group)
                .into_iter()
                .enumerate()
                .map(|(n, (i, depth))| Line { row: rows[i].clone(), thread: group.len(), depth, head: n == 0 })
                .collect();
            conversations.push(lines);
        }
        conversations
    }

    /// True when the conversation that holds message `uid` is folded to its first message.
    pub fn is_folded(&self, uid: u32) -> bool {
        self.conversations().iter().any(|c| c.len() > 1 && c.iter().any(|l| l.row.uid == uid) && c.first().is_some_and(|l| self.collapsed.contains(&l.row.uid)))
    }

    /// Folds or unfolds the conversation that holds message `uid` (View > Collapse or Expand Conversation). A message
    /// alone in its conversation has nothing to fold.
    pub fn toggle_conversation(&mut self, uid: u32) {
        let first = self
            .conversations()
            .into_iter()
            .find(|c| c.len() > 1 && c.iter().any(|l| l.row.uid == uid))
            .and_then(|c| c.first().map(|l| l.row.uid));
        if let Some(first) = first {
            if !self.collapsed.remove(&first) {
                self.collapsed.insert(first);
            }
        }
    }

    /// Folds every conversation of more than one message (View > Collapse All Conversations).
    pub fn collapse_all(&mut self) {
        self.collapsed = self.conversations().into_iter().filter(|c| c.len() > 1).filter_map(|c| c.first().map(|l| l.row.uid)).collect();
    }

    /// Unfolds every conversation (View > Expand All Conversations).
    pub fn expand_all(&mut self) {
        self.collapsed.clear();
    }

    /// Orders the list by `key`. Picking a new key starts newest first for Date and A to Z for the others; picking the
    /// key already in use keeps the direction.
    pub fn sort_by(&mut self, key: SortKey) {
        if self.sort != key {
            self.sort = key;
            self.ascending = key != SortKey::Date;
        }
    }

    pub fn set_ascending(&mut self, ascending: bool) {
        self.ascending = ascending;
    }

    /// The first of `candidates` the account has as a mailbox (names match without case), for Archive and Trash.
    pub fn find_mailbox(&self, candidates: &[&str]) -> Option<String> {
        candidates.iter().find_map(|c| self.mailboxes.iter().find(|m| m.eq_ignore_ascii_case(c)).cloned())
    }

    /// The text of a message arrived. It is kept only when it belongs to the shown account and mailbox.
    pub fn body_arrived(&mut self, account_id: &str, mailbox: &str, uid: u32, text: String) {
        if self.current().is_some_and(|a| a.id == account_id) && self.mailbox == mailbox {
            self.attachments.insert(uid, crate::body::attachments(&text));
            self.bodies.insert(uid, text);
            self.trim_bodies(uid);
        }
    }

    /// The bytes the cached texts and attachments hold.
    fn cached_bytes(&self) -> usize {
        let texts: usize = self.bodies.values().map(String::len).sum();
        let files: usize = self.attachments.values().flatten().map(|a| a.bytes.len()).sum();
        texts + files
    }

    /// Keeps at most [`MAX_CACHED_BODIES`] texts and [`MAX_CACHED_BYTES`] bytes: the open message and `keep` stay, the
    /// lowest UID values go first.
    fn trim_bodies(&mut self, keep: u32) {
        let mut bytes = self.cached_bytes();
        while self.bodies.len() > MAX_CACHED_BODIES || bytes > MAX_CACHED_BYTES {
            let victim = self.bodies.keys().copied().filter(|&u| u != keep && Some(u) != self.open).min();
            let Some(victim) = victim else { break };
            let text = self.bodies.remove(&victim).map_or(0, |t| t.len());
            let files: usize = self.attachments.remove(&victim).map_or(0, |list| list.iter().map(|a| a.bytes.len()).sum());
            bytes = bytes.saturating_sub(text + files);
        }
    }

    /// Add Account: a name, an address and a password. The servers are looked up from the address (see
    /// [`crate::discover`]); Configure Manually shows their fields for the person to fill in.
    pub fn open_setup(&mut self) {
        self.discovering = false;
        self.status.clear();
        self.sheet = Some(Sheet::Setup(Form::new(vec![
            Field::new("Name", "Shown in the sidebar", ""),
            Field::new("Email", "name@example.com", ""),
            Field { secret: true, ..Field::new("Password", "Stored in Passwords", "") },
            Field::yes_no(MANUAL, false),
        ])));
    }

    /// Makes the server fields of the Add Account sheet match its Configure Manually field: shown (empty) when it says
    /// yes, gone when it says no.
    pub fn sync_manual(&mut self) {
        let Some(Sheet::Setup(form)) = &mut self.sheet else { return };
        let on = is_yes(form.value(MANUAL));
        let shown = form.fields.iter().any(|f| f.label == "Incoming server");
        if on && !shown {
            form.fields.extend(manual_fields(None));
        } else if !on && shown {
            form.fields.retain(|f| !MANUAL_LABELS.contains(&f.label));
            form.focus = form.focus.min(form.fields.len().saturating_sub(1));
        }
    }

    /// Shows the server fields, filled from what discovery found (or empty).
    pub fn show_manual(&mut self, found: Option<&Discovered>) {
        let Some(Sheet::Setup(form)) = &mut self.sheet else { return };
        form.fields.retain(|f| !MANUAL_LABELS.contains(&f.label));
        form.fields.extend(manual_fields(found));
        if let Some(choice) = form.fields.iter_mut().find(|f| f.label == MANUAL) {
            choice.value = "yes".to_string();
        }
    }

    /// A compose sheet: empty for a new message, or the reply to `reply`, with the original's sender as recipient.
    /// The cursor goes to the recipient field when there is none yet, otherwise to the message body.
    pub fn open_compose(&mut self, reply: Option<&Summary>) {
        let draft = match reply {
            Some(original) => message::reply_draft(original, &self.sent_from(original)),
            None => Draft { from: self.current().map(|a| a.email.clone()).unwrap_or_default(), ..Draft::default() },
        };
        self.open_draft(draft);
    }

    /// Message > Reply: a reply to `original`. With `quoted` (the readable text of the original, when the setting asks
    /// for quotes) the body starts with an attribution line and the quoted text.
    pub fn open_reply(&mut self, original: &Summary, quoted: Option<&str>) {
        let mut draft = message::reply_draft(original, &self.sent_from(original));
        if let Some(text) = quoted {
            draft.body = format!("\n\nOn {}, {} wrote:\n{}", original.date, sender_name(&original.from), message::quote_text(text));
        }
        self.open_draft(draft);
    }

    /// The address a reply is sent from: the one the original was addressed to when the account has it (an alias), and
    /// the account's own address otherwise.
    pub fn sent_from(&self, original: &Summary) -> String {
        let own = self.current().map(|a| a.email.clone()).unwrap_or_default();
        let addressed: Vec<String> = message::split_addresses(&original.to)
            .into_iter()
            .chain(message::split_addresses(&original.cc))
            .map(|entry| message::address_of(&entry).to_ascii_lowercase())
            .collect();
        self.from_choices().into_iter().find(|choice| addressed.contains(&choice.to_ascii_lowercase())).unwrap_or(own)
    }

    /// True when the reply to `original` would reach more people than its sender: another address in To or Cc, other
    /// than the user's own. Reply All is only offered then.
    pub fn has_other_recipients(&self, original: &Summary) -> bool {
        let own = self.current().map(|a| a.email.clone()).unwrap_or_default();
        let all = message::reply_all_draft(original, &own);
        all.to.len() + all.cc.len() > message::reply_draft(original, &own).to.len()
    }

    /// Reply All: the sender and the other recipients of `original`, minus the user's own address.
    pub fn open_reply_all(&mut self, original: &Summary) {
        let from = self.sent_from(original);
        let draft = message::reply_all_draft(original, &from);
        self.open_draft(draft);
    }

    /// Forward: `text` (the decoded body of `original`) quoted under a `Fwd:` subject.
    pub fn open_forward(&mut self, original: &Summary, text: &str) {
        self.open_forward_with(original, text, Vec::new());
    }

    /// Forward with the files of the original (Message > Forward): they go along as attachments, within the limits of a
    /// message. A file that would pass a limit is left out.
    pub fn open_forward_with(&mut self, original: &Summary, text: &str, files: Vec<Attachment>) {
        let own = self.current().map(|a| a.email.clone()).unwrap_or_default();
        let mut draft = message::forward_draft(original, &own, text);
        draft.attachments = within_limits(files);
        self.open_draft(draft);
    }

    /// A compose sheet that starts from `draft`. Cc, Bcc and Reply-To are shown when the View menu asks for them or
    /// when the draft already has addresses there. The cursor goes to the recipient field when there is none yet,
    /// otherwise to the message body.
    pub fn open_draft(&mut self, draft: Draft) {
        let (account_id, own) = self.current().map(|a| (a.id.clone(), a.email.clone())).unwrap_or_default();
        let mut fields = Vec::new();
        if !self.prefs.aliases_of(&account_id).is_empty() {
            let shown = if draft.from.is_empty() { own } else { draft.from.clone() };
            fields.push(Field::new("From", "", &shown));
        }
        let to_at = fields.len();
        fields.push(Field::new("To", "name@example.com, other@example.com", &draft.to.join(", ")));
        if self.show_cc || !draft.cc.is_empty() {
            fields.push(Field::new("Cc", "", &draft.cc.join(", ")));
        }
        if self.show_bcc || !draft.bcc.is_empty() {
            fields.push(Field::new("Bcc", "", &draft.bcc.join(", ")));
        }
        if self.show_reply_to || !draft.reply_to.is_empty() {
            fields.push(Field::new("Reply-To", "", &draft.reply_to.join(", ")));
        }
        fields.push(Field::new("Subject", "", &draft.subject));
        fields.push(Field::new("Message", "", &draft.body));
        let last = fields.len() - 1;
        let mut form = Form::new(fields);
        form.focus_on(if draft.to.is_empty() { to_at } else { last });
        let signature = self.prefs.signature(&account_id).to_string();
        self.sheet = Some(Sheet::Compose(Compose::new(form, draft.in_reply_to, draft.references, draft.priority, draft.attachments, signature)));
        self.fresh_compose = true;
        self.sync_recipient_field();
    }

    /// Replaces the address book the recipient suggestions draw on.
    pub fn set_address_book(&mut self, book: Vec<Contact>) {
        self.address_book = book;
        self.sync_recipient_field();
    }

    /// Rebuilds the suggestions for the focused field: nothing unless it is a To, Cc or Bcc field of a compose sheet.
    /// The text after the last separator is what is being typed; the recipients before it are not offered again. An
    /// untouched field offers the addresses used last; once a recipient is in, an empty tail offers nothing.
    pub fn sync_recipient_field(&mut self) {
        let field = match &self.sheet {
            Some(Sheet::Compose(compose)) => compose.form.fields.get(compose.form.focus).filter(|f| is_recipient_field(f.label)).map(|f| {
                let (head, typed) = split_last_recipient(&f.value);
                let mut field = RecipientField::new();
                field.recipients = message::split_addresses(head).iter().map(|a| message::address_of(a).to_lowercase()).collect();
                field.set_text(typed.trim(), &self.address_book, &self.recipients, MAX_SUGGESTIONS);
                if typed.trim().is_empty() && !f.value.trim().is_empty() {
                    field.press(RecipientKey::Escape);
                }
                field
            }),
            _ => None,
        };
        self.recipient_field = field.unwrap_or_default();
    }

    /// The suggestions listed under the focused recipient field.
    pub fn recipient_suggestions(&self) -> &[Suggestion] {
        self.recipient_field.suggestions()
    }

    /// The arrow keys: moves the highlight in the suggestion list. False when no list is showing (the key is not ours).
    pub fn move_recipient_highlight(&mut self, down: bool) -> bool {
        if self.recipient_field.suggestions().is_empty() {
            return false;
        }
        self.recipient_field.press(if down { RecipientKey::Down } else { RecipientKey::Up });
        true
    }

    /// Escape: closes the suggestion list. False when no list is showing (Escape belongs to the sheet).
    pub fn close_recipient_list(&mut self) -> bool {
        if self.recipient_field.suggestions().is_empty() {
            return false;
        }
        self.recipient_field.press(RecipientKey::Escape);
        true
    }

    /// Return and Tab take the highlighted suggestion; a tap takes the one at `index`. The typed text is replaced by
    /// `Name <address>` and a separator, ready for the next recipient. False when there is nothing to take.
    pub fn accept_recipient(&mut self, index: Option<usize>) -> bool {
        let Some(at) = index.or(self.recipient_field.highlighted()) else { return false };
        let Some(suggestion) = self.recipient_field.suggestions().get(at).cloned() else { return false };
        // a name with list punctuation would split the recipient: the bare address is safe
        let token = if suggestion.name.contains([',', ';', '"', '<', '>']) { suggestion.address.clone() } else { suggestion.token() };
        let Some(Sheet::Compose(compose)) = self.sheet.as_mut() else { return false };
        let focus = compose.form.focus;
        let Some(field) = compose.form.fields.get_mut(focus).filter(|f| is_recipient_field(f.label)) else { return false };
        let (head, _) = split_last_recipient(&field.value);
        let head = head.trim_end();
        field.value = if head.is_empty() { format!("{token}, ") } else { format!("{head} {token}, ") };
        self.sync_recipient_field();
        true
    }

    /// The people of the open message to donate to Contacts, once per message and only when its text is here: the
    /// sender and the other recipients (see [`crate::people::seen_in`]). Nobody when Mail Settings turned donating off.
    pub fn people_to_donate(&mut self) -> Vec<crate::people::Seen> {
        if !self.prefs.donate_people {
            return Vec::new();
        }
        let Some(uid) = self.open.filter(|uid| self.bodies.contains_key(uid)) else { return Vec::new() };
        let Some(account) = self.current().map(|a| a.id.clone()) else { return Vec::new() };
        if !self.donated.insert((account, self.mailbox.clone(), uid)) {
            return Vec::new();
        }
        let Some(summary) = self.row(uid).map(|r| r.summary.clone()) else { return Vec::new() };
        crate::people::seen_in(&summary.from, &summary.to, &summary.cc, crate::people::is_bulk(&summary), &self.from_choices())
    }

    /// Reads the open message for things to offer (an event, a flight, an email address or phone number, a street
    /// address) once its text is here, minus the ones turned down. `region` is the region code whose phone rules turn a
    /// number into international form. Another message, or none, clears the banner.
    pub fn refresh_found_in(&mut self, region: &str) {
        let Some(uid) = self.open else {
            self.found_pager = None;
            return;
        };
        if self.found_pager.as_ref().is_some_and(|(shown, _)| *shown == uid) {
            return;
        }
        self.found_pager = None;
        let Some(raw) = self.bodies.get(&uid) else { return };
        let title = self.row(uid).map(|r| r.summary.subject.clone()).unwrap_or_default();
        let source = found_in::Source { app: "Mail".into(), title };
        let items = self.found_dismissed.filter(found_in::extract(&source, &crate::body::readable(raw), region));
        self.found_pager = Some((uid, Pager::new(items)));
    }

    /// The suggestion the banner shows, when the open message has any left.
    pub fn found_current(&self) -> Option<&Found> {
        let (uid, pager) = self.found_pager.as_ref()?;
        if Some(*uid) != self.open {
            return None;
        }
        pager.current()
    }

    /// "1 of 3" when the banner has more than one suggestion.
    pub fn found_label(&self) -> Option<String> {
        self.found_current()?;
        self.found_pager.as_ref()?.1.label()
    }

    /// Next and previous page of the banner. False at either end.
    pub fn found_step(&mut self, forward: bool) -> bool {
        match self.found_pager.as_mut() {
            Some((_, pager)) if forward => pager.next_page(),
            Some((_, pager)) => pager.previous_page(),
            None => false,
        }
    }

    /// Dismiss: the suggestion on show is turned down and not offered again from this message.
    pub fn found_dismiss(&mut self) -> bool {
        let Some(found) = self.found_pager.as_mut().and_then(|(_, pager)| pager.dismiss()) else { return false };
        self.found_dismissed.add(&found);
        true
    }

    /// Add: the suggestion on show leaves the banner (and is not offered again) and its action is returned for the
    /// window to run.
    pub fn found_accept(&mut self) -> Option<Action> {
        let (_, pager) = self.found_pager.as_mut()?;
        let found = pager.current().cloned()?;
        let action = pager.accept()?;
        self.found_dismissed.add(&found);
        Some(action)
    }

    /// The addresses the shown account sends from: its own first, then its other addresses.
    pub fn from_choices(&self) -> Vec<String> {
        let Some(account) = self.current() else { return Vec::new() };
        let mut choices = vec![account.email.clone()];
        choices.extend(self.prefs.aliases_of(&account.id).iter().cloned());
        choices
    }

    /// Message > Send Later: shows or hides the "Send in" field of the open compose sheet.
    pub fn set_send_later(&mut self, shown: bool) {
        if let Some(Sheet::Compose(compose)) = &mut self.sheet {
            compose.set_field("Send in", "30m, 2h, 1d", shown);
        }
    }

    /// True when the open compose sheet has its "Send in" field.
    pub fn send_later_shown(&self) -> bool {
        matches!(&self.sheet, Some(Sheet::Compose(c)) if c.form.fields.iter().any(|f| f.label == "Send in"))
    }

    /// Shows or hides an optional address field of new messages and of the compose sheet that is open.
    pub fn set_address_field(&mut self, label: &'static str, shown: bool) {
        match label {
            "Cc" => self.show_cc = shown,
            "Bcc" => self.show_bcc = shown,
            _ => self.show_reply_to = shown,
        }
        if let Some(Sheet::Compose(compose)) = &mut self.sheet {
            compose.set_field(label, "", shown);
        }
    }

    /// Sets the open message's priority (Message > Set Priority).
    pub fn set_priority(&mut self, priority: Priority) {
        if let Some(Sheet::Compose(compose)) = &mut self.sheet {
            compose.priority = priority;
        }
    }

    /// New Mailbox: a sheet that asks for the name.
    pub fn open_new_mailbox(&mut self) {
        self.sheet = Some(Sheet::Mailbox(MailboxSheet { form: Form::new(vec![Field::new("Name", "Work/Receipts", "")]), rename_from: None }));
    }

    /// Rename Mailbox: a sheet with the shown mailbox's name. The inbox cannot be renamed; None is returned then.
    pub fn open_rename_mailbox(&mut self) -> Option<()> {
        if self.mailbox.eq_ignore_ascii_case(INBOX) {
            return None;
        }
        let name = self.mailbox.clone();
        self.sheet = Some(Sheet::Mailbox(MailboxSheet { form: Form::new(vec![Field::new("Name", "", &name)]), rename_from: Some(name) }));
        Some(())
    }

    /// The mailboxes a message of the shown mailbox can move or copy to: every other mailbox of the account.
    pub fn move_targets(&self) -> Vec<String> {
        self.mailboxes.iter().filter(|m| **m != self.mailbox).cloned().collect()
    }

    /// The mailbox name a sheet's form asks for, checked: not empty, no line breaks, and not already a mailbox.
    pub fn new_mailbox_name(&self, form: &Form, rename_from: Option<&str>) -> Result<String, String> {
        let name = form.value("Name").trim().to_string();
        if name.is_empty() || name.contains(['\r', '\n']) {
            return Err("Enter a name for the mailbox".into());
        }
        if self.mailboxes.iter().any(|m| m.eq_ignore_ascii_case(&name) && rename_from != Some(m.as_str())) {
            return Err(format!("There is already a mailbox named {name}"));
        }
        Ok(name)
    }

    /// Message > Mute: mutes the conversation of message `uid`, or unmutes it. Returns whether it is muted now.
    pub fn toggle_mute(&mut self, uid: u32) -> Option<bool> {
        let key = crate::mute::key(&self.row(uid)?.summary);
        let muted = !self.muted.remove(&key);
        if muted {
            self.muted.insert(key);
        }
        Some(muted)
    }

    /// True when the conversation of message `uid` is muted.
    pub fn is_muted(&self, uid: u32) -> bool {
        self.row(uid).is_some_and(|r| self.muted.contains(&crate::mute::key(&r.summary)))
    }

    /// Open Quickly: a sheet that asks which mailbox to go to.
    pub fn open_quickly(&mut self) {
        self.sheet = Some(Sheet::Quickly(Form::new(vec![Field::new("Go to mailbox", "Type part of a mailbox name", "")])));
    }

    /// The mailboxes whose last name part contains `text` (without case), the ones that start with it first. An empty
    /// text matches every mailbox.
    pub fn quickly_matches(&self, text: &str) -> Vec<String> {
        let needle = text.trim().to_lowercase();
        let last = |m: &str| m.rsplit('/').next().unwrap_or(m).to_lowercase();
        let mut found: Vec<String> = self.mailboxes.iter().filter(|m| m.to_lowercase().contains(&needle)).cloned().collect();
        found.sort_by_key(|m| !last(m).starts_with(&needle));
        found
    }

    /// The Find sheet, with the search applied so far.
    pub fn open_search(&mut self) {
        let text = self.search.clone();
        self.sheet = Some(Sheet::Search(Form::new(vec![Field::new("Search", "from:ana subject:fundur \"exact phrase\"", &text)])));
    }

    /// What the toolbar's search field shows: the text as typed while it is the applied search, else the applied search
    /// (a smart mailbox or Show All changed it).
    pub fn search_shown(&self) -> &str {
        if self.search_text.trim() == self.search { &self.search_text } else { &self.search }
    }

    /// Types into the toolbar's search field: the list filters at once, as the text grows.
    pub fn type_search(&mut self, text: &str) {
        let mut raw = self.search_shown().to_string();
        raw.push_str(text);
        self.set_search_text(raw);
    }

    /// Deletes the last character of the toolbar's search field.
    pub fn backspace_search(&mut self) {
        let mut raw = self.search_shown().to_string();
        raw.pop();
        self.set_search_text(raw);
    }

    /// Sets the toolbar's search field to `raw` and applies it.
    pub fn set_search_text(&mut self, raw: String) {
        self.apply_search(&raw);
        self.search_text = raw;
    }

    /// Applies a search text. Empty (or blank) shows every message again.
    pub fn apply_search(&mut self, text: &str) {
        self.search = text.trim().to_string();
        self.found = None;
        self.smart_active = None;
    }

    /// Shows a smart mailbox: its saved search is applied to the shown mailbox. Returns false for an index past the list.
    pub fn apply_smart(&mut self, index: usize) -> bool {
        let Some(query) = self.prefs.smart.get(index).map(|s| s.query.clone()) else { return false };
        self.apply_search(&query);
        self.smart_active = Some(index);
        true
    }

    /// The rows the list shows. With a search applied and the server's answer in, that answer (the server searched
    /// every message of the mailbox). Otherwise the loaded rows that match the search, newest first: bodies are searched
    /// when they are loaded, and a message whose body is not loaded matches on its headers only.
    pub fn visible_rows(&self) -> Vec<Row> {
        let mut rows = self.matching_rows();
        rows.retain(|row| self.filter.matches(row, self.attachments.get(&row.uid).map_or(0, Vec::len)));
        if let Some(wanted) = self.category {
            rows.retain(|row| category::classify(&row.summary) == wanted);
        }
        let text = |row: &Row| match self.sort {
            SortKey::Date => String::new(),
            SortKey::From => row.summary.from.to_lowercase(),
            SortKey::To => row.summary.to.to_lowercase(),
            SortKey::Subject => row.summary.subject.to_lowercase(),
            SortKey::Flagged => (if row.flags.contains(Flag::Flagged) { "0" } else { "1" }).to_string(),
            SortKey::Unread => (if row.flags.contains(Flag::Seen) { "1" } else { "0" }).to_string(),
        };
        rows.sort_by(|a, b| {
            let order = text(a).cmp(&text(b)).then(a.uid.cmp(&b.uid));
            if self.ascending {
                order
            } else {
                order.reverse()
            }
        });
        rows
    }

    /// The rows the search keeps, before the filter and the sort.
    fn matching_rows(&self) -> Vec<Row> {
        if let Some(found) = &self.found {
            return found.clone();
        }
        let query = Query::parse(&self.search);
        self.rows
            .iter()
            .filter(|row| {
                let body = self.bodies.get(&row.uid).map_or("", String::as_str);
                let doc = Doc {
                    from: row.summary.from.as_str(),
                    to: row.summary.to.as_str(),
                    subject: row.summary.subject.as_str(),
                    body,
                };
                query.matches(&doc)
            })
            .cloned()
            .collect()
    }

    pub fn close_sheet(&mut self) {
        self.sheet = None;
    }

    /// True when a new sheet may replace what is on screen: not while a message is being written.
    /// True when remote content may load for message `uid`: Mail Settings allows it, or the person loaded it for this message.
    pub fn remote_allowed(&self, uid: u32) -> bool {
        self.prefs.remote_content || self.remote_loaded.contains(&uid)
    }

    /// True when the shown mailbox is the account's Drafts (its special-use mailbox, or the fixed name).
    pub fn is_drafts(&self) -> bool {
        self.mailbox == self.drafts_mailbox.as_deref().unwrap_or(crate::sync::DRAFTS)
    }

    /// Message > Edit Draft: the saved message `raw` opens as a compose sheet. Its Message-ID is kept, so saving the
    /// sheet again replaces that copy in Drafts instead of adding one.
    pub fn open_draft_copy(&mut self, raw: &str) {
        let id = Summary::from_headers(raw).message_id;
        self.open_draft(message::draft_of(raw));
        if let Some(Sheet::Compose(compose)) = self.sheet.as_mut() {
            compose.draft_id = id.unwrap_or_default();
        }
    }

    /// Load Remote Content: the remote content of message `uid` may load.
    pub fn allow_remote(&mut self, uid: u32) {
        self.remote_loaded.insert(uid);
    }

    /// True when the sheet slot can take a compose: it is empty, or it holds a compose (which keeps its own window).
    pub fn sheet_is_free_for_compose(&self) -> bool {
        matches!(self.sheet, None | Some(Sheet::Compose(_)))
    }

    pub fn sheet_is_free(&self) -> bool {
        !matches!(self.sheet, Some(Sheet::Compose(_)))
    }

    fn open_prompt(&mut self, kind: PromptKind, fields: Vec<Field>) {
        self.sheet = Some(Sheet::Prompt(Prompt { kind, form: Form::new(fields) }));
    }

    /// Message > Remind Me: asks when the open message should come back. The message needs a Message-ID, which is what
    /// finds it again.
    pub fn open_remind(&mut self) -> Result<(), String> {
        let uid = self.open.ok_or("Open a message first")?;
        let row = self.row(uid).ok_or("Open a message first")?;
        if row.summary.message_id.is_none() {
            return Err("This message has no Message-ID, so it cannot be found again".into());
        }
        self.open_prompt(PromptKind::Remind, vec![Field::new("Remind me in", "30m, 2h, 1d, 1w", "")]);
        Ok(())
    }

    /// True when message `uid` has a reminder waiting in the queue for the shown account.
    pub fn has_reminder(&self, uid: u32) -> bool {
        let Some(message_id) = self.row(uid).and_then(|r| r.summary.message_id.as_deref()) else { return false };
        let Some(account) = self.current().map(|a| a.id.as_str()) else { return false };
        self.queue.reminders.iter().any(|r| r.account == account && r.message_id == message_id)
    }

    /// Message > Redirect: asks who should get the open message.
    pub fn open_redirect(&mut self) {
        self.open_prompt(PromptKind::Redirect, vec![Field::new("To", "name@example.com, other@example.com", "")]);
    }

    /// Mailbox > New Smart Mailbox: a name and a search, which starts as the search applied now.
    pub fn open_smart_mailbox(&mut self) {
        let search = self.search.clone();
        self.open_prompt(
            PromptKind::SmartMailbox,
            vec![Field::new("Name", "Unread from Ana", ""), Field::new("Search", "from:ana subject:fundur \"exact phrase\"", &search)],
        );
    }

    /// Mailbox > Rules > New Rule.
    pub fn open_rule(&mut self) {
        self.open_prompt(
            PromptKind::Rule,
            vec![
                Field::new("Name", "Receipts", ""),
                Field::new("From contains", "shop.example", ""),
                Field::new("To contains", "", ""),
                Field::new("Subject contains", "receipt", ""),
                Field::new("Move to mailbox", "Receipts", ""),
                Field::new("Mark as read", "yes", ""),
                Field::new("Flag colour", "red, orange, yellow, green, blue, purple, gray", ""),
            ],
        );
    }

    /// Mailbox > Mail Settings: the signature, the other addresses and the undo-send delay of the shown account.
    pub fn open_settings(&mut self) -> Result<(), String> {
        let id = self.current().map(|a| a.id.clone()).ok_or("Add an account first")?;
        let signature = self.prefs.signature(&id).to_string();
        let aliases = self.prefs.aliases_of(&id).join(", ");
        let delay = if self.prefs.undo_delay == 0 { String::new() } else { self.prefs.undo_delay.to_string() };
        self.open_prompt(
            PromptKind::Settings,
            vec![
                Field::new("Signature", "Added to the end of new messages", &signature),
                Field::new("Other addresses", "info@example.com, ana@example.org", &aliases),
                Field::new("Undo send (seconds)", "0 sends at once", &delay),
                Field::yes_no("Quote the original in replies", self.prefs.quote_replies),
                Field::yes_no("Plain text messages only", self.prefs.plain_text),
                Field::yes_no("Bold unread messages", self.prefs.bold_unread),
                Field::yes_no("Load remote content", self.prefs.remote_content),
                Field::yes_no("Signature above the quoted text", self.prefs.signature_above_quote),
                Field::yes_no("Show in Search", self.prefs.show_in_search),
                Field::yes_no("Suggest people to Contacts", self.prefs.donate_people),
            ],
        );
        Ok(())
    }

    /// Mailbox > Account Settings: the setup form filled with the shown account. The password stays as it is when left
    /// empty.
    pub fn open_edit_account(&mut self) -> Result<(), String> {
        let account = self.current().cloned().ok_or("Add an account first")?;
        let (imap, smtp) = (accounts_server(&account.imap), accounts_server(&account.smtp));
        self.open_prompt(
            PromptKind::EditAccount(account.id.clone()),
            vec![
                Field::new("Name", "Shown in the sidebar", &account.name),
                Field::new("Email", "name@example.com", &account.email),
                Field::new("User name", "Same as the email when empty", &account.username),
                Field { secret: true, ..Field::new("Password", "Leave empty to keep the saved password", "") },
                Field::new("Incoming server", "", &imap.0),
                Field::new("Incoming port", "", &imap.1),
                Field::new("Outgoing server", "", &smtp.0),
                Field::new("Outgoing port", "", &smtp.1),
            ],
        );
        Ok(())
    }

    /// Mailbox > Remove Account: asks for the account's name before it goes.
    pub fn open_remove_account(&mut self) -> Result<(), String> {
        let account = self.current().cloned().ok_or("Add an account first")?;
        self.open_prompt(PromptKind::RemoveAccount(account.id), vec![Field::new("Account name", "Type the name of the account to remove it", "")]);
        Ok(())
    }

    /// The rule a New Rule form describes. The mailbox to move to must exist in the shown account.
    /// Mailbox > Rules: the rules as a list, each with Remove; a name opens its editor.
    pub fn open_rules_pane(&mut self) {
        let names = self.prefs.rules.iter().map(rules::describe).collect();
        self.sheet = Some(Sheet::Rules(Form::new(Vec::new()), names));
    }

    /// Edit Rule: the editor of the rule at `index`, filled in from the rule.
    pub fn open_rule_editor(&mut self, index: usize) {
        let Some(rule) = self.prefs.rules.get(index).cloned() else { return };
        let move_to = rule.actions.iter().find_map(|a| match a {
            rules::Action::MoveTo(name) => Some(name.clone()),
            _ => None,
        });
        let flag = rule.actions.iter().find_map(|a| match a {
            rules::Action::Flag(color) => Some(color.label().to_string()),
            _ => None,
        });
        let mark = if rule.actions.contains(&rules::Action::MarkRead) { "yes" } else { "no" };
        self.open_prompt(
            PromptKind::EditRule(index),
            vec![
                Field::new("Name", "Receipts", &rule.name),
                Field::new("From contains", "shop.example", &rule.from),
                Field::new("To contains", "", &rule.to),
                Field::new("Subject contains", "receipt", &rule.subject),
                Field::new("Move to mailbox", "Receipts", &move_to.unwrap_or_default()),
                Field::new("Mark as read", "yes", mark),
                Field::new("Flag colour", "red, orange, yellow, green, blue, purple, gray", &flag.unwrap_or_default()),
            ],
        );
    }

    pub fn rule_from_form(&self, form: &Form) -> Result<Rule, String> {
        self.rule_from_form_at(form, None)
    }

    /// The rule a form describes. `except` is the place of the rule being edited, which may keep its own name.
    pub fn rule_from_form_at(&self, form: &Form, except: Option<usize>) -> Result<Rule, String> {
        let name = form.value("Name").trim().to_string();
        if name.is_empty() {
            return Err("Name the rule".into());
        }
        let (from, to, subject) =
            (form.value("From contains").trim().to_string(), form.value("To contains").trim().to_string(), form.value("Subject contains").trim().to_string());
        if from.is_empty() && to.is_empty() && subject.is_empty() {
            return Err("Give the rule something to look for: a sender, a recipient or a subject".into());
        }
        let target = form.value("Move to mailbox").trim();
        let target = if target.is_empty() {
            String::new()
        } else {
            self.mailboxes.iter().find(|m| m.eq_ignore_ascii_case(target)).cloned().ok_or_else(|| format!("There is no mailbox named {target}"))?
        };
        let actions = rules::actions_from(&target, form.value("Mark as read"), form.value("Flag colour"))?;
        if self.prefs.rules.iter().enumerate().any(|(i, r)| Some(i) != except && r.name.eq_ignore_ascii_case(&name)) {
            return Err(format!("There is already a rule named {name}"));
        }
        Ok(Rule { name, from, to, subject, actions })
    }

    /// The smart mailbox a New Smart Mailbox form describes.
    pub fn smart_from_form(&self, form: &Form) -> Result<SmartMailbox, String> {
        let name = form.value("Name").trim().to_string();
        let query = form.value("Search").trim().to_string();
        if name.is_empty() {
            return Err("Name the smart mailbox".into());
        }
        if Query::parse(&query).terms.is_empty() {
            return Err("Enter what the smart mailbox searches for".into());
        }
        if self.prefs.smart.iter().any(|s| s.name.eq_ignore_ascii_case(&name)) {
            return Err(format!("There is already a smart mailbox named {name}"));
        }
        if self.prefs.smart.len() >= prefs::MAX_ENTRIES {
            return Err("There are too many smart mailboxes".into());
        }
        Ok(SmartMailbox { name, query })
    }

    /// Applies the Mail Settings form to the shown account and the shared undo-send delay.
    pub fn apply_settings(&mut self, form: &Form) -> Result<(), String> {
        let id = self.current().map(|a| a.id.clone()).ok_or("Add an account first")?;
        let mut aliases = Vec::new();
        for entry in message::split_addresses(form.value("Other addresses")) {
            let address = message::address_of(&entry).to_string();
            if !shared::valid_address(&address) {
                return Err(format!("{entry} is not an email address"));
            }
            if !aliases.iter().any(|a: &String| a.eq_ignore_ascii_case(&address)) {
                aliases.push(address);
            }
        }
        let delay = match form.value("Undo send (seconds)").trim() {
            "" => 0,
            text => text.parse::<u32>().map_err(|_| "Undo send takes a number of seconds".to_string())?,
        };
        if delay > prefs::MAX_UNDO_DELAY {
            return Err(format!("Undo send takes at most {} seconds", prefs::MAX_UNDO_DELAY));
        }
        self.prefs.set_signature(&id, form.value("Signature"));
        self.prefs.set_aliases(&id, aliases);
        self.prefs.undo_delay = delay;
        // a form from before the toggles (or one without them) keeps the settings as they are
        if form.fields.iter().any(|f| f.label == "Quote the original in replies") {
            self.prefs.quote_replies = is_yes(form.value("Quote the original in replies"));
        }
        if form.fields.iter().any(|f| f.label == "Plain text messages only") {
            self.prefs.plain_text = is_yes(form.value("Plain text messages only"));
        }
        if form.fields.iter().any(|f| f.label == "Bold unread messages") {
            self.prefs.bold_unread = is_yes(form.value("Bold unread messages"));
        }
        if form.fields.iter().any(|f| f.label == "Load remote content") {
            self.prefs.remote_content = is_yes(form.value("Load remote content"));
        }
        if form.fields.iter().any(|f| f.label == "Signature above the quoted text") {
            self.prefs.signature_above_quote = is_yes(form.value("Signature above the quoted text"));
        }
        if form.fields.iter().any(|f| f.label == "Show in Search") {
            self.prefs.show_in_search = is_yes(form.value("Show in Search"));
            self.prefs.donate_people = is_yes(form.value("Suggest people to Contacts"));
        }
        Ok(())
    }

    /// Checks the setup form and builds the mail account it describes, with its password. Hosts and ports fall back to
    /// the domain's `imap.` and `smtp.` names and the standard ports. `taken` is the whole shared list, so the new id
    /// is unique among every account.
    pub fn account_from_form(&self, form: &Form, taken: &[Account]) -> Result<(Account, String), String> {
        self.account_from_form_found(form, taken, None)
    }

    /// [`Self::account_from_form`] with the servers discovery found: they stand in for the `imap.` and `smtp.` guesses
    /// wherever the form leaves a server field empty.
    pub fn account_from_form_found(&self, form: &Form, taken: &[Account], found: Option<&Discovered>) -> Result<(Account, String), String> {
        let email = form.value("Email").trim().to_string();
        if !shared::valid_address(&email) {
            return Err("Enter an email address, for example name@example.com".into());
        }
        let password = form.value("Password").to_string();
        if password.is_empty() {
            return Err("Enter the password for this account".into());
        }
        let account = build_account(form, &account_id(&email, taken), found)?;
        Ok((account, password))
    }

    /// Checks the Account Settings form and builds the account `old` becomes: the same id and services, the new names and
    /// servers. The password is returned when one was typed; it is required when the user name or the incoming server
    /// changed, because Passwords keeps it under those.
    pub fn edited_account(&self, form: &Form, old: &Account) -> Result<(Account, Option<String>), String> {
        let mut account = build_account(form, &old.id, None)?;
        account.services = old.services.clone();
        let password = form.value("Password").to_string();
        let moved = account.password_site() != old.password_site() || account.username != old.username;
        if moved && password.is_empty() {
            return Err("Enter the password again: the user name or the incoming server changed".into());
        }
        Ok((account, (!password.is_empty()).then_some(password)))
    }

    /// Puts an edited account in the list in place of the one with its id, with its new password if there is one.
    pub fn replace_account(&mut self, account: Account, password: Option<String>) {
        if let Some(at) = self.accounts.iter().position(|a| a.id == account.id) {
            if let Some(password) = password {
                self.passwords.insert(account.id.clone(), password);
            }
            self.accounts[at] = account;
        }
    }

    /// Takes an account away with everything Mail kept for it: the password in memory, the new-mail baseline, the
    /// signature, the other addresses and the favourites. Another account becomes the shown one.
    pub fn forget_account(&mut self, id: &str) {
        let Some(index) = self.accounts.iter().position(|a| a.id == id) else { return };
        // what is shown is kept under its own account first: the indices shift and another account is shown next
        self.remember_selection();
        self.stash_rows();
        self.open = None;
        self.clear_selection();
        self.accounts.remove(index);
        self.listings.retain(|(account, _), _| account != id);
        self.remembered.retain(|(account, _), _| account != id);
        self.passwords.remove(id);
        self.baseline.remove(id);
        self.validity.remove(id);
        self.prefs.signatures.remove(id);
        self.prefs.aliases.remove(id);
        self.prefs.favorites.retain(|f| f.account != id);
        self.account = self.account.min(self.accounts.len().saturating_sub(1));
        self.mailboxes = vec![INBOX.to_string()];
        let shown = self.account;
        if self.accounts.is_empty() {
            self.show_mailbox(INBOX);
        } else {
            self.show_account(shown);
        }
    }

    /// The draft a compose sheet describes, from the shown account. Recipients are split on commas and semicolons.
    pub fn draft_from(&self, compose: &Compose) -> Result<Draft, String> {
        let mut from = self.current().map(|a| a.email.clone()).ok_or("Add an account first")?;
        let chosen = compose.form.value("From").trim();
        if !chosen.is_empty() {
            if !self.from_choices().iter().any(|c| c.eq_ignore_ascii_case(chosen)) {
                return Err(format!("{chosen} is not one of this account's addresses. Click the From field to choose another"));
            }
            from = chosen.to_string();
        }
        let to = message::split_addresses(compose.form.value("To"));
        if to.is_empty() {
            return Err("Add at least one recipient".into());
        }
        Ok(self.draft_fields(compose, from, to))
    }

    /// The draft of `compose` as it is kept in Drafts: the recipients may be empty.
    pub fn draft_for_save(&self, compose: &Compose) -> Draft {
        let from = self.current().map(|a| a.email.clone()).unwrap_or_default();
        let to = message::split_addresses(compose.form.value("To"));
        self.draft_fields(compose, from, to)
    }

    fn draft_fields(&self, compose: &Compose, from: String, to: Vec<String>) -> Draft {
        Draft {
            from,
            to,
            cc: message::split_addresses(compose.form.value("Cc")),
            bcc: message::split_addresses(compose.form.value("Bcc")),
            reply_to: message::split_addresses(compose.form.value("Reply-To")),
            priority: compose.priority,
            subject: compose.form.value("Subject").to_string(),
            body: if self.prefs.signature_above_quote {
                with_signature_above_quote(compose.form.value("Message"), &compose.signature)
            } else {
                with_signature(compose.form.value("Message"), &compose.signature)
            },
            in_reply_to: compose.in_reply_to.clone(),
            references: compose.references.clone(),
            attachments: compose.attachments.clone(),
            html: if self.prefs.plain_text { None } else { richtext::html_body(compose.message(), &compose.styles, &compose.signature) },
        }
    }

    /// Adds an account the user set up, once it is in the shared list and its password is saved. It becomes the shown
    /// account.
    pub fn add_account(&mut self, account: Account, password: String) {
        self.passwords.insert(account.id.clone(), password);
        self.accounts.push(account);
        self.show_account(self.accounts.len() - 1);
    }
}

/// The name in a `Name <address>` value, or the address when there is no name (the attribution line of a quoted reply).
fn sender_name(from: &str) -> String {
    let name = from.split('<').next().unwrap_or("").trim().trim_matches('"').trim();
    if name.is_empty() {
        message::address_of(from).to_string()
    } else {
        name.to_string()
    }
}

/// The files that fit a new message, in order: at most [`MAX_ATTACHMENTS`] of them and [`MAX_ATTACHMENT_BYTES`] in all.
/// A file that would pass either limit, and every file after it, is left out.
pub fn within_limits(files: Vec<Attachment>) -> Vec<Attachment> {
    let mut kept = Vec::new();
    let mut bytes = 0;
    for file in files {
        if kept.len() >= MAX_ATTACHMENTS || bytes + file.bytes.len() > MAX_ATTACHMENT_BYTES {
            break;
        }
        bytes += file.bytes.len();
        kept.push(file);
    }
    kept
}

/// Builds the mail account a setup form describes, with `id`. A server field the form leaves empty takes the server
/// `found` has, or else the domain's `imap.` or `smtp.` name and the standard port.
fn build_account(form: &Form, id: &str, found: Option<&Discovered>) -> Result<Account, String> {
    let email = form.value("Email").trim().to_string();
    if !shared::valid_address(&email) {
        return Err("Enter an email address, for example name@example.com".into());
    }
    let domain = email.rsplit_once('@').map_or("", |(_, d)| d).to_string();
    let name = match form.value("Name").trim() {
        "" => email.clone(),
        name => name.to_string(),
    };
    let username = match (form.value("User name").trim(), found.and_then(|f| f.username.as_deref())) {
        ("", None) => email.clone(),
        ("", Some(login)) => login.to_string(),
        (user, _) => user.to_string(),
    };
    let imap_default = found.map_or_else(|| Server { host: format!("imap.{domain}"), port: IMAP_TLS_PORT, security: Security::Tls }, |f| f.imap.clone());
    let smtp_default = found.map_or_else(|| Server { host: format!("smtp.{domain}"), port: 587, security: Security::StartTls }, |f| f.smtp.clone());
    let mut account = Account::new(Kind::Mail, id, &name, &username);
    account.email = email;
    account.imap = Some(server_of(form, "Incoming", imap_default, IMAP_TLS_PORT)?);
    account.smtp = Some(server_of(form, "Outgoing", smtp_default, SMTP_TLS_PORT)?);
    account.validate().map_err(|e| e.to_string())?;
    Ok(account)
}

/// The `which` ("Incoming" or "Outgoing") server of a setup form: what is typed, and `default` for what is not. A typed
/// port that differs from the default picks its encryption by the usual port (`tls_port` starts TLS at once, the others
/// start with STARTTLS).
fn server_of(form: &Form, which: &str, default: Server, tls_port: u16) -> Result<Server, String> {
    let typed = form.value(&format!("{which} port")).trim().to_string();
    let port = port(&typed, default.port)?;
    let host = host(form.value(&format!("{which} server")), &default.host);
    let security = if port == default.port { default.security } else { security(port, tls_port) };
    Ok(Server { host, port, security })
}

/// The message with the account's signature after it, set off by the `-- ` line mail programs recognise. No signature
/// leaves the text as it is.
pub fn with_signature(body: &str, signature: &str) -> String {
    let signature = signature.trim_end();
    if signature.trim().is_empty() {
        return body.to_string();
    }
    let body = body.trim_end();
    if body.is_empty() {
        format!("\n\n-- \n{signature}")
    } else {
        format!("{body}\n\n-- \n{signature}")
    }
}

/// The signature placed above the quoted text of a reply: it goes before the attribution line ("On ..., X wrote:"). A
/// message without one gets the signature at the end, as `with_signature` gives it.
pub fn with_signature_above_quote(body: &str, signature: &str) -> String {
    let signature = signature.trim_end();
    if signature.trim().is_empty() {
        return body.to_string();
    }
    let lines: Vec<&str> = body.split('\n').collect();
    let Some(at) = lines.iter().position(|line| line.starts_with("On ") && line.trim_end().ends_with("wrote:")) else {
        return with_signature(body, signature);
    };
    let above = lines[..at].join("\n");
    let top = above.trim_end();
    let below = lines[at..].join("\n");
    let block = format!("-- \n{signature}");
    if top.is_empty() {
        format!("{block}\n\n{below}")
    } else {
        format!("{top}\n\n{block}\n\n{below}")
    }
}

/// A server as the setup form shows it: its host and its port.
fn accounts_server(server: &Option<Server>) -> (String, String) {
    server.as_ref().map_or_else(|| (String::new(), String::new()), |s| (s.host.clone(), s.port.to_string()))
}

fn security(port: u16, tls_port: u16) -> Security {
    if port == tls_port {
        Security::Tls
    } else {
        Security::StartTls
    }
}

fn host(typed: &str, fallback: &str) -> String {
    match typed.trim() {
        "" => fallback.to_string(),
        h => h.to_string(),
    }
}

fn port(typed: &str, fallback: u16) -> Result<u16, String> {
    match typed.trim() {
        "" => Ok(fallback),
        p => p.parse::<u16>().ok().filter(|&n| n != 0).ok_or_else(|| format!("{p} is not a port number")),
    }
}

/// A stable account id from the address: lower-case letters, digits and dashes, unique among `taken`.
pub fn account_id(email: &str, taken: &[Account]) -> String {
    let mut base: String = email
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    base = base.trim_matches('-').to_string();
    if base.is_empty() {
        base = "account".to_string();
    }
    let mut id = base.clone();
    let mut n = 2;
    while taken.iter().any(|a| a.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

#[cfg(test)]
mod tests;
