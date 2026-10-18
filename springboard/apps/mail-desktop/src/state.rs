//! The mail window's state: the accounts and what is selected, the message rows of the open mailbox, the open message,
//! the sheets (account setup and compose) and the arrival baseline for new-mail notices. Nothing here touches the
//! network, the passwords or the window; `window.rs` runs the jobs and feeds their results in.

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
}

impl Form {
    pub fn new(fields: Vec<Field>) -> Form {
        Form { fields, focus: 0 }
    }

    pub fn type_text(&mut self, text: &str) {
        if let Some(field) = self.fields.get_mut(self.focus) {
            field.value.push_str(text);
        }
    }

    pub fn backspace(&mut self) {
        if let Some(field) = self.fields.get_mut(self.focus) {
            field.value.pop();
        }
    }

    /// Tab: the next field, wrapping to the first.
    pub fn next_field(&mut self) {
        if !self.fields.is_empty() {
            self.focus = (self.focus + 1) % self.fields.len();
        }
    }

    pub fn focus_on(&mut self, index: usize) {
        if index < self.fields.len() {
            self.focus = index;
        }
    }

    pub fn value(&self, label: &str) -> &str {
        self.fields.iter().find(|f| f.label == label).map_or("", |f| f.value.as_str())
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
        let in_message = self.form.fields.get(self.form.focus).is_some_and(|f| f.label == "Message");
        if !in_message {
            change(&mut self.form);
            return;
        }
        let before = self.message().chars().count();
        self.styles.resize(before, Style::default());
        change(&mut self.form);
        let after = self.message().chars().count();
        if after > before {
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
    /// The UID of the message open in the reader.
    pub open: Option<u32>,
    /// Message text by UID, for the shown mailbox only.
    pub bodies: HashMap<u32, String>,
    /// Passwords read from the Passwords group for this run. They are never written to the shared account list.
    pub passwords: HashMap<String, String>,
    /// The highest inbox UID seen per account id. Absent until the first inbox listing of the account.
    pub baseline: HashMap<String, u32>,
    pub sheet: Option<Sheet>,
    /// Set when a compose was made into the sheet slot (open_draft, a failed send, Undo Send). The app moves it into a
    /// compose window of its own (see `compose_windows`).
    pub fresh_compose: bool,
    /// Messages whose remote content the person allowed with Load Remote Content, by UID. Not kept between runs.
    pub remote_loaded: HashSet<u32>,
    /// The sheet slot was closed on purpose (sent, saved or discarded), so its compose window closes without a question.
    pub sheet_done: bool,
    pub status: String,
    /// The search text as applied. Empty shows every message.
    pub search: String,
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
            bodies: HashMap::new(),
            passwords: HashMap::new(),
            baseline: HashMap::new(),
            sheet: None,
            fresh_compose: false,
            remote_loaded: HashSet::new(),
            sheet_done: false,
            status: String::new(),
            search: String::new(),
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
        self.account = index;
        self.mailboxes = vec![INBOX.to_string()];
        self.show_mailbox(INBOX);
    }

    pub fn show_mailbox(&mut self, name: &str) {
        self.mailbox = name.to_string();
        self.rows.clear();
        self.bodies.clear();
        self.attachments.clear();
        self.found = None;
        self.open = None;
        self.collapsed.clear();
    }

    /// Takes the result of a listing. Returns the notices for new inbox messages. The rows change only when the listing
    /// is for the shown account and mailbox.
    pub fn apply_listing(&mut self, account_id: &str, mailbox: &str, mailboxes: Vec<String>, messages: Vec<Listed>) -> Vec<Notice> {
        let mut notices = Vec::new();
        self.arrived.clear();
        if mailbox.eq_ignore_ascii_case(INBOX) {
            let seen = self.baseline.get(account_id).copied();
            for m in arrivals(seen, &messages) {
                let summary = Summary::from_headers(&m.header);
                self.arrived.push((account_id.to_string(), Row { uid: m.uid, flags: m.flags, summary: summary.clone() }));
                if self.muted.contains(&crate::mute::key(&summary)) {
                    continue;
                }
                notices.push(Notice { account: account_id.to_string(), uid: m.uid, from: summary.from.clone(), subject: summary.subject.clone() });
            }
            if let Some(top) = messages.iter().map(|m| m.uid).max() {
                let entry = self.baseline.entry(account_id.to_string()).or_insert(top);
                *entry = (*entry).max(top);
            }
        }
        let shown = self.current().is_some_and(|a| a.id == account_id) && self.mailbox == mailbox;
        if shown {
            if !mailboxes.is_empty() {
                self.mailboxes = mailboxes;
            }
            self.rows = newest_first(messages);
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

    /// Opens a message in the reader. Returns true when its text is not loaded yet.
    pub fn open_message(&mut self, uid: u32) -> bool {
        self.open = Some(uid);
        self.mark_seen_locally(uid);
        !self.bodies.contains_key(&uid)
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
        if self.open == Some(uid) {
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

    /// Keeps at most [`MAX_CACHED_BODIES`] texts: the open message and `keep` stay, the lowest UID values go first.
    fn trim_bodies(&mut self, keep: u32) {
        while self.bodies.len() > MAX_CACHED_BODIES {
            let victim = self.bodies.keys().copied().filter(|&u| u != keep && Some(u) != self.open).min();
            let Some(victim) = victim else { break };
            self.bodies.remove(&victim);
            self.attachments.remove(&victim);
        }
    }

    pub fn open_setup(&mut self) {
        let hint_email = "name@example.com";
        self.sheet = Some(Sheet::Setup(Form::new(vec![
            Field::new("Name", "Shown in the sidebar", ""),
            Field::new("Email", hint_email, ""),
            Field::new("User name", "Same as the email when empty", ""),
            Field { secret: true, ..Field::new("Password", "Stored in Passwords", "") },
            Field::new("Incoming server", "imap.<domain> when empty", ""),
            Field::new("Incoming port", "", &IMAP_TLS_PORT.to_string()),
            Field::new("Outgoing server", "smtp.<domain> when empty", ""),
            Field::new("Outgoing port", "", "587"),
        ])));
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
        let email = form.value("Email").trim().to_string();
        if !shared::valid_address(&email) {
            return Err("Enter an email address, for example name@example.com".into());
        }
        let password = form.value("Password").to_string();
        if password.is_empty() {
            return Err("Enter the password for this account".into());
        }
        let account = build_account(form, &account_id(&email, taken))?;
        Ok((account, password))
    }

    /// Checks the Account Settings form and builds the account `old` becomes: the same id and services, the new names and
    /// servers. The password is returned when one was typed; it is required when the user name or the incoming server
    /// changed, because Passwords keeps it under those.
    pub fn edited_account(&self, form: &Form, old: &Account) -> Result<(Account, Option<String>), String> {
        let mut account = build_account(form, &old.id)?;
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
        self.accounts.remove(index);
        self.passwords.remove(id);
        self.baseline.remove(id);
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

/// Builds the mail account a setup form describes, with `id`. Hosts and ports fall back to the domain's `imap.` and `smtp.`
/// names and the standard ports.
fn build_account(form: &Form, id: &str) -> Result<Account, String> {
    let email = form.value("Email").trim().to_string();
    if !shared::valid_address(&email) {
        return Err("Enter an email address, for example name@example.com".into());
    }
    let domain = email.rsplit_once('@').map_or("", |(_, d)| d).to_string();
    let name = match form.value("Name").trim() {
        "" => email.clone(),
        name => name.to_string(),
    };
    let username = match form.value("User name").trim() {
        "" => email.clone(),
        user => user.to_string(),
    };
    let imap_port = port(form.value("Incoming port"), IMAP_TLS_PORT)?;
    let smtp_port = port(form.value("Outgoing port"), 587)?;
    let imap_host = host(form.value("Incoming server"), &format!("imap.{domain}"));
    let smtp_host = host(form.value("Outgoing server"), &format!("smtp.{domain}"));
    let mut account = Account::new(Kind::Mail, id, &name, &username);
    account.email = email;
    account.imap = Some(Server { host: imap_host, port: imap_port, security: security(imap_port, IMAP_TLS_PORT) });
    account.smtp = Some(Server { host: smtp_host, port: smtp_port, security: security(smtp_port, SMTP_TLS_PORT) });
    account.validate().map_err(|e| e.to_string())?;
    Ok(account)
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
mod tests {
    use super::*;

    fn account(id: &str) -> Account {
        let mut a = Account::new(Kind::Mail, id, "Work", "olibuijr");
        a.email = "olibuijr@example.com".into();
        a.imap = Some(Server { host: "imap.example.com".into(), port: 993, security: Security::Tls });
        a.smtp = Some(Server { host: "smtp.example.com".into(), port: 587, security: Security::StartTls });
        a
    }

    #[test]
    fn sent_addresses_are_suggested_most_recent_first() {
        let mut mail = Mail::new(vec![account("work")]);
        assert!(mail.recipients.is_empty());
        mail.recipients.record("Ana@Example.com", "");
        mail.recipients.record("bo@example.com", "");
        mail.recipients.record("ana@example.com", "");
        let got: Vec<String> = mail.recipients.suggest("", 10).into_iter().map(|r| r.address).collect();
        assert_eq!(got, vec!["ana@example.com", "bo@example.com"]);
        assert_eq!(mail.recipients.suggest("bo", 10).len(), 1);
    }

    fn listed(uid: u32, subject: &str) -> Listed {
        Listed { uid, flags: Flags::empty(), header: format!("Subject: {subject}\r\nFrom: Ana <ana@example.com>\r\n\r\n") }
    }

    fn setup(fields: &[(&str, &str)]) -> Form {
        let mut form = Mail::new(vec![]).sheet_form_for_test();
        for (label, value) in fields {
            let field = form.fields.iter_mut().find(|f| f.label == *label).unwrap();
            field.value = value.to_string();
        }
        form
    }

    impl Mail {
        fn sheet_form_for_test(mut self) -> Form {
            self.open_setup();
            match self.sheet {
                Some(Sheet::Setup(form)) => form,
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn the_first_listing_only_sets_the_baseline() {
        let mut mail = Mail::new(vec![account("work")]);
        let notices = mail.apply_listing("work", INBOX, vec![], vec![listed(4, "old"), listed(9, "older")]);
        assert!(notices.is_empty());
        assert_eq!(mail.baseline.get("work"), Some(&9));
    }

    #[test]
    fn later_listings_notify_for_newer_inbox_messages_only() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", INBOX, vec![], vec![listed(9, "old")]);
        let notices = mail.apply_listing("work", INBOX, vec![], vec![listed(9, "old"), listed(12, "Fundur")]);
        assert_eq!(notices, vec![Notice { account: "work".into(), uid: 12, from: "Ana <ana@example.com>".into(), subject: "Fundur".into() }]);
        assert_eq!(mail.baseline.get("work"), Some(&12));
    }

    #[test]
    fn rows_show_the_newest_first_for_the_shown_mailbox_only() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", "Sent", vec!["INBOX".into(), "Sent".into()], vec![listed(1, "sent")]);
        assert!(mail.rows.is_empty(), "another mailbox does not fill the list");
        mail.apply_listing("work", INBOX, vec!["INBOX".into(), "Sent".into()], vec![listed(2, "a"), listed(7, "b")]);
        let uids: Vec<u32> = mail.rows.iter().map(|r| r.uid).collect();
        assert_eq!(uids, vec![7, 2]);
        assert_eq!(mail.mailboxes, vec!["INBOX", "Sent"]);
    }

    #[test]
    fn opening_a_message_marks_it_read_and_asks_for_its_text_once() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", INBOX, vec![], vec![listed(3, "x")]);
        assert!(mail.open_message(3));
        assert!(mail.rows[0].flags.contains(Flag::Seen));
        mail.body_arrived("work", INBOX, 3, "text".into());
        assert!(!mail.open_message(3));
        mail.body_arrived("other", INBOX, 4, "not shown".into());
        assert!(!mail.bodies.contains_key(&4));
    }

    #[test]
    fn the_text_cache_stays_bounded_and_keeps_the_open_message() {
        let mut mail = Mail::new(vec![account("work")]);
        let rows: Vec<Listed> = (1..=(MAX_CACHED_BODIES as u32 + 40)).map(|u| listed(u, "x")).collect();
        mail.apply_listing("work", INBOX, vec![], rows);
        assert!(mail.open_message(1));
        for uid in 1..=(MAX_CACHED_BODIES as u32 + 40) {
            mail.body_arrived("work", INBOX, uid, format!("body {uid}"));
        }
        assert_eq!(mail.bodies.len(), MAX_CACHED_BODIES);
        assert_eq!(mail.attachments.len(), mail.bodies.len(), "attachments leave with their texts");
        assert!(mail.bodies.contains_key(&(MAX_CACHED_BODIES as u32 + 40)), "the newest text is kept");
        assert!(!mail.bodies.contains_key(&2), "the lowest UID went first");
        assert!(mail.open_message(2), "a dropped text is asked for again");
    }

    #[test]
    fn the_search_filters_the_rows_and_an_empty_search_shows_all() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", INBOX, vec![], vec![listed(3, "Fundur"), listed(4, "Reikningur")]);
        assert_eq!(mail.visible_rows().len(), 2);
        mail.apply_search("  subject:fundur ");
        assert_eq!(mail.search, "subject:fundur");
        assert_eq!(mail.visible_rows().iter().map(|r| r.uid).collect::<Vec<_>>(), vec![3]);
        mail.body_arrived("work", INBOX, 4, "Fundur um reikninginn".into());
        mail.apply_search("body:fundur");
        assert_eq!(mail.visible_rows().iter().map(|r| r.uid).collect::<Vec<_>>(), vec![4]);
        mail.apply_search("");
        assert_eq!(mail.visible_rows().len(), 2);
    }

    #[test]
    fn the_threaded_view_lists_each_conversation_together() {
        let mut mail = Mail::new(vec![account("work")]);
        let reply = "Message-ID: <b@x>\r\nIn-Reply-To: <a@x>\r\nSubject: Re: Plan\r\nFrom: Ana <ana@example.com>\r\n\r\n";
        let plan = "Message-ID: <a@x>\r\nSubject: Plan\r\nFrom: Ana <ana@example.com>\r\n\r\n";
        let lone = "Message-ID: <c@x>\r\nSubject: Lunch\r\nFrom: Bo <bo@example.com>\r\n\r\n";
        let listed_threaded = |uid, header: &str| Listed { uid, flags: Flags::empty(), header: header.to_string() };
        mail.apply_listing("work", INBOX, vec![], vec![listed_threaded(1, plan), listed_threaded(2, lone), listed_threaded(3, reply)]);
        assert_eq!(mail.visible_lines().iter().map(|l| l.row.uid).collect::<Vec<_>>(), vec![3, 2, 1], "flat, newest first");
        assert!(mail.visible_lines().iter().all(|l| l.thread == 1));
        mail.threaded = true;
        let lines = mail.visible_lines();
        assert_eq!(lines.iter().map(|l| l.row.uid).collect::<Vec<_>>(), vec![1, 3, 2], "the reply sits under its parent");
        assert_eq!(lines.iter().map(|l| l.thread).collect::<Vec<_>>(), vec![2, 2, 1]);
        assert_eq!(lines.iter().map(|l| l.depth).collect::<Vec<_>>(), vec![0, 1, 0]);
    }

    #[test]
    fn a_server_search_answer_replaces_the_rows_only_for_the_same_search() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", INBOX, vec![], vec![listed(3, "Fundur"), listed(4, "Reikningur")]);
        mail.apply_search("subject:reikn");
        assert_eq!(mail.visible_rows().iter().map(|r| r.uid).collect::<Vec<_>>(), vec![4], "the loaded rows until the server answers");
        mail.apply_found("work", INBOX, "subject:other", vec![listed(9, "x")]);
        assert_eq!(mail.found, None, "an answer for another search is dropped");
        mail.apply_found("work", INBOX, "subject:reikn", vec![listed(2, "old"), listed(9, "Reikningur 2")]);
        assert_eq!(mail.visible_rows().iter().map(|r| r.uid).collect::<Vec<_>>(), vec![9, 2]);
        assert!(mail.row(9).is_some() && mail.row(3).is_some());
        mail.apply_search("");
        assert_eq!(mail.visible_rows().len(), 2);
        assert_eq!(mail.found, None);
    }

    #[test]
    fn a_loaded_message_keeps_its_files_until_the_mailbox_changes() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", INBOX, vec![], vec![listed(3, "Skjöl")]);
        let raw = "Content-Type: multipart/mixed; boundary=B\r\n\r\n--B\r\nContent-Type: text/plain\r\n\r\nSee attached\r\n--B\r\nContent-Type: text/csv; name=\"tolur.csv\"\r\nContent-Disposition: attachment; filename=\"tolur.csv\"\r\n\r\na,b\r\n--B--\r\n";
        mail.body_arrived("work", INBOX, 3, raw.to_string());
        let files = mail.attachments.get(&3).expect("the files of the loaded message");
        assert_eq!(files.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), vec!["tolur.csv"]);
        mail.show_mailbox("Sent");
        assert!(mail.attachments.is_empty());
    }

    #[test]
    fn the_find_sheet_starts_from_the_applied_search() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_search("bo");
        mail.open_search();
        match &mail.sheet {
            Some(Sheet::Search(form)) => assert_eq!(form.value("Search"), "bo"),
            other => panic!("expected the Find sheet, got {other:?}"),
        }
    }

    #[test]
    fn muting_a_conversation_silences_its_notices_and_toggles_back() {
        let mut mail = Mail::new(vec![account("work")]);
        let root = "Message-ID: <a@x>\r\nSubject: Fundur\r\n\r\n";
        mail.apply_listing("work", INBOX, vec![], vec![Listed { uid: 1, flags: Flags::empty(), header: root.into() }]);
        assert_eq!(mail.toggle_mute(1), Some(true));
        assert!(mail.is_muted(1));
        let reply = "Message-ID: <b@x>\r\nReferences: <a@x>\r\nSubject: Re: Fundur\r\n\r\n";
        let other = "Message-ID: <z@x>\r\nSubject: Annad\r\n\r\n";
        let messages = vec![
            Listed { uid: 1, flags: Flags::empty(), header: root.into() },
            Listed { uid: 2, flags: Flags::empty(), header: reply.into() },
            Listed { uid: 3, flags: Flags::empty(), header: other.into() },
        ];
        let notices = mail.apply_listing("work", INBOX, vec![], messages);
        assert_eq!(notices.iter().map(|n| n.uid).collect::<Vec<_>>(), vec![3]);
        assert_eq!(mail.toggle_mute(1), Some(false));
        assert!(!mail.is_muted(1));
        assert_eq!(mail.toggle_mute(99), None, "an unlisted message cannot be muted");
    }

    #[test]
    fn open_quickly_lists_matching_mailboxes_best_first() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.mailboxes = vec!["INBOX".into(), "Archive/Receipts".into(), "Sent".into(), "Rec".into(), "Drafts".into()];
        assert_eq!(mail.quickly_matches("rec"), vec!["Archive/Receipts".to_string(), "Rec".to_string()]);
        mail.mailboxes = vec!["Old receipts".into(), "Receipts".into()];
        assert_eq!(mail.quickly_matches("receipts"), vec!["Receipts".to_string(), "Old receipts".to_string()], "a name that starts with the text first");
        assert_eq!(mail.quickly_matches("").len(), 2);
        assert!(mail.quickly_matches("zzz").is_empty());
        mail.open_quickly();
        assert!(matches!(mail.sheet, Some(Sheet::Quickly(_))));
    }

    #[test]
    fn a_flag_colour_shows_at_once_and_clearing_the_flag_clears_the_colour() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", INBOX, vec![], vec![listed(3, "x")]);
        mail.set_color_locally(3, Some(FlagColor::Purple));
        assert_eq!(mail.flags_of(3).and_then(Flags::color), Some(FlagColor::Purple));
        mail.set_color_locally(3, Some(FlagColor::Blue));
        assert_eq!(mail.flags_of(3).and_then(Flags::color), Some(FlagColor::Blue));
        mail.set_flag_locally(3, Flag::Flagged, false);
        assert_eq!(mail.flags_of(3), Some(Flags::empty()));
    }

    #[test]
    fn flags_change_at_once_and_a_moved_message_leaves_the_list() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", INBOX, vec![], vec![listed(3, "a"), listed(4, "b")]);
        mail.open_message(3);
        mail.set_flag_locally(3, Flag::Flagged, true);
        assert!(mail.flags_of(3).is_some_and(|f| f.contains(Flag::Flagged)));
        mail.remove_row(3);
        assert_eq!(mail.open, None, "the open message closes when it moves");
        assert_eq!(mail.rows.iter().map(|r| r.uid).collect::<Vec<_>>(), vec![4]);
        assert_eq!(mail.flags_of(3), None);
    }

    #[test]
    fn archive_and_trash_are_found_under_the_names_providers_use() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.mailboxes = vec!["INBOX".into(), "[Gmail]/All Mail".into(), "Trash".into()];
        assert_eq!(mail.find_mailbox(&ARCHIVE_NAMES).as_deref(), Some("[Gmail]/All Mail"));
        assert_eq!(mail.find_mailbox(&TRASH_NAMES).as_deref(), Some("Trash"));
        mail.mailboxes = vec!["INBOX".into()];
        assert_eq!(mail.find_mailbox(&ARCHIVE_NAMES), None);
        mail.mailboxes = vec!["archive".into()];
        assert_eq!(mail.find_mailbox(&ARCHIVE_NAMES).as_deref(), Some("archive"));
    }

    #[test]
    fn switching_mailboxes_clears_the_open_message() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.open_message(3);
        mail.show_mailbox("Sent");
        assert_eq!(mail.open, None);
        assert_eq!(mail.mailbox, "Sent");
    }

    #[test]
    fn a_setup_form_builds_an_account_with_defaults() {
        let mail = Mail::new(vec![]);
        let form = setup(&[("Email", "ana@example.com"), ("Password", "secret")]);
        let (account, password) = mail.account_from_form(&form, &[]).unwrap();
        assert_eq!(password, "secret");
        assert_eq!(account.kind, Kind::Mail);
        assert_eq!(account.id, "ana-example-com");
        assert_eq!(account.username, "ana@example.com");
        assert_eq!(account.imap, Some(Server { host: "imap.example.com".into(), port: 993, security: Security::Tls }));
        assert_eq!(account.smtp, Some(Server { host: "smtp.example.com".into(), port: 587, security: Security::StartTls }));
        assert_eq!(account.password_site(), "imap.example.com");
    }

    #[test]
    fn a_setup_form_needs_an_address_and_a_password() {
        let mail = Mail::new(vec![]);
        assert!(mail.account_from_form(&setup(&[("Email", "nope"), ("Password", "x")]), &[]).is_err());
        assert!(mail.account_from_form(&setup(&[("Email", "ana@example.com")]), &[]).is_err());
        assert!(mail.account_from_form(&setup(&[("Email", "ana@example.com"), ("Password", "x"), ("Incoming port", "0")]), &[]).is_err());
    }

    #[test]
    fn plain_imap_ports_start_with_starttls() {
        let mail = Mail::new(vec![]);
        let form = setup(&[("Email", "ana@example.com"), ("Password", "p"), ("Incoming port", "143")]);
        let (account, _) = mail.account_from_form(&form, &[]).unwrap();
        assert_eq!(account.imap.map(|s| s.security), Some(Security::StartTls));
    }

    #[test]
    fn a_new_account_id_avoids_every_account_in_the_shared_list() {
        let mail = Mail::new(vec![]);
        let shared_list = vec![account("ana-example-com")];
        let form = setup(&[("Email", "ana@example.com"), ("Password", "p")]);
        let (account, _) = mail.account_from_form(&form, &shared_list).unwrap();
        assert_eq!(account.id, "ana-example-com-2");
    }

    #[test]
    fn account_ids_are_unique() {
        let taken = vec![account("ana-example-com")];
        assert_eq!(account_id("ana@example.com", &taken), "ana-example-com-2");
        assert_eq!(account_id("Ana+x@Example.com", &[]), "ana-x-example-com");
    }

    #[test]
    fn a_compose_sheet_becomes_a_draft_with_its_recipients() {
        let mail = Mail::new(vec![account("work")]);
        let form = Form::new(vec![
            Field::new("To", "", "ana@example.com; bo@example.com,  "),
            Field::new("Subject", "", "Hi"),
            Field::new("Message", "", "Body"),
        ]);
        let draft = mail.draft_from(&Compose::new(form, None, vec![], Priority::Normal, vec![], String::new())).unwrap();
        assert_eq!(draft.to, vec!["ana@example.com", "bo@example.com"]);
        assert_eq!(draft.from, "olibuijr@example.com");
        let empty = Form::new(vec![Field::new("To", "", " ")]);
        assert!(mail.draft_from(&Compose::new(empty, None, vec![], Priority::Normal, vec![], String::new())).is_err());
    }

    fn person(given: &str, family: &str, address: &str) -> Contact {
        use bifrost_app::controls::contact_book::LabeledValue;
        Contact { given_name: given.into(), family_name: family.into(), email_addresses: vec![LabeledValue::new(None, address.to_string())], ..Contact::default() }
    }

    fn type_in_focus(mail: &mut Mail, text: &str) {
        mail.sheet.as_mut().unwrap().form_mut().type_text(text);
        mail.sync_recipient_field();
    }

    #[test]
    fn the_recipient_tail_is_what_is_being_typed() {
        assert_eq!(split_last_recipient(""), ("", ""));
        assert_eq!(split_last_recipient("an"), ("", "an"));
        assert_eq!(split_last_recipient("ana@example.com, bo"), ("ana@example.com,", " bo"));
        assert_eq!(split_last_recipient("Ana <ana@example.com>, "), ("Ana <ana@example.com>,", " "));
        assert_eq!(split_last_recipient("\"Smith, Ana\" <a@x.is>"), ("", "\"Smith, Ana\" <a@x.is>"), "a comma in quotes does not separate");
    }

    #[test]
    fn typing_in_to_suggests_contacts_and_recents_and_a_choice_fills_the_field() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.recipients.record("bob@example.com", "Bob Recent");
        mail.set_address_book(vec![person("Ana", "Lovelace", "ana@example.com"), person("Anders", "Holm", "anders@example.com")]);
        mail.open_compose(None);
        // an untouched To field offers the addresses used last
        assert_eq!(mail.recipient_suggestions().iter().map(|s| s.address.as_str()).collect::<Vec<_>>(), ["bob@example.com"]);
        type_in_focus(&mut mail, "an");
        let got: Vec<String> = mail.recipient_suggestions().iter().map(|s| s.address.clone()).collect();
        assert_eq!(got, ["anders@example.com", "ana@example.com"], "both contacts match at a word start, in name order");
        assert!(mail.move_recipient_highlight(true) && mail.move_recipient_highlight(true));
        assert_eq!(mail.recipient_field.highlighted(), Some(1));
        assert!(mail.accept_recipient(None));
        assert_eq!(compose_of(&mail).form.value("To"), "Ana Lovelace <ana@example.com>, ");
        assert!(mail.recipient_suggestions().is_empty(), "nothing is offered after a finished recipient");
        // the recipient is split off for the draft as an address with its name
        assert_eq!(message::split_addresses(compose_of(&mail).form.value("To")), ["Ana Lovelace <ana@example.com>"]);
        // the next recipient does not offer the one already in the field
        type_in_focus(&mut mail, "ana");
        assert!(mail.recipient_suggestions().is_empty());
        type_in_focus(&mut mail, "ders");
        assert_eq!(mail.recipient_suggestions().len(), 1);
        assert!(mail.accept_recipient(Some(0)));
        assert_eq!(compose_of(&mail).form.value("To"), "Ana Lovelace <ana@example.com>, Anders Holm <anders@example.com>, ");
    }

    #[test]
    fn the_message_field_and_a_closed_list_offer_nothing() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.set_address_book(vec![person("Ana", "Lovelace", "ana@example.com")]);
        mail.open_compose(None);
        type_in_focus(&mut mail, "ana");
        assert_eq!(mail.recipient_suggestions().len(), 1);
        assert!(mail.close_recipient_list(), "Escape closes the list first");
        assert!(mail.recipient_suggestions().is_empty());
        assert!(!mail.close_recipient_list(), "with no list showing Escape belongs to the sheet");
        assert!(!mail.accept_recipient(None), "nothing is highlighted");
        let sheet = mail.sheet.as_mut().unwrap().form_mut();
        sheet.focus_on(sheet.fields.len() - 1);
        mail.sync_recipient_field();
        assert!(mail.recipient_suggestions().is_empty(), "the message body has no suggestions");
    }

    #[test]
    fn a_name_with_a_comma_is_kept_out_of_the_token() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.set_address_book(vec![person("Lovelace, Dr", "Ada", "ada@example.com")]);
        mail.open_compose(None);
        type_in_focus(&mut mail, "ada");
        assert!(mail.move_recipient_highlight(true) && mail.accept_recipient(None));
        assert_eq!(compose_of(&mail).form.value("To"), "ada@example.com, ", "the bare address, so the list still splits cleanly");
    }

    fn read_message(mail: &mut Mail, uid: u32, from: &str, to: &str, body: &str) {
        let summary = Summary { subject: "Plans".into(), from: from.into(), to: to.into(), ..Summary::default() };
        mail.rows.push(Row { uid, flags: Flags::default(), summary });
        mail.open = Some(uid);
        mail.bodies.insert(uid, format!("From: {from}\r\nSubject: Plans\r\n\r\n{body}\r\n"));
    }

    #[test]
    fn a_read_message_donates_its_people_once_unless_the_setting_is_off() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.rows.push(Row { uid: 4, flags: Flags::default(), summary: Summary { from: "Bo <bo@example.com>".into(), to: "olibuijr@example.com, Cy <cy@example.com>".into(), ..Summary::default() } });
        mail.open = Some(4);
        assert!(mail.people_to_donate().is_empty(), "the text has not arrived");
        mail.bodies.insert(4, "Subject: x\r\n\r\nHi\r\n".into());
        let seen = mail.people_to_donate();
        let got: Vec<(&str, &str)> = seen.iter().map(|s| (s.name.as_str(), s.address.as_str())).collect();
        assert_eq!(got, [("Bo", "bo@example.com"), ("Cy", "cy@example.com")], "the person's own address is left out");
        assert!(mail.people_to_donate().is_empty(), "a message is donated once");
        mail.prefs.donate_people = false;
        mail.rows.push(Row { uid: 5, flags: Flags::default(), summary: Summary { from: "Di <di@example.com>".into(), ..Summary::default() } });
        mail.open = Some(5);
        mail.bodies.insert(5, "Subject: y\r\n\r\nHi\r\n".into());
        assert!(mail.people_to_donate().is_empty(), "donating is off");
    }

    #[test]
    fn the_open_message_offers_what_it_mentions_and_a_turned_down_one_stays_down() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.rows.push(Row { uid: 9, flags: Flags::default(), summary: Summary { subject: "Plans".into(), ..Summary::default() } });
        mail.open = Some(9);
        mail.refresh_found_in("IS");
        assert!(mail.found_current().is_none(), "no text, no banner");
        mail.bodies.insert(9, "Subject: Plans\r\n\r\nLunch tomorrow at 1pm\r\nWrite to ana@example.com\r\n".into());
        mail.refresh_found_in("IS");
        assert_eq!(mail.found_current().map(|f| f.kind), Some(found_in::SuggestionKind::Event));
        assert_eq!(mail.found_current().map(|f| f.found_in()).as_deref(), Some("Found in Mail"));
        assert_eq!(mail.found_label().as_deref(), Some("1 of 2"));
        assert!(mail.found_step(true) && !mail.found_step(true), "the second page is the last");
        assert_eq!(mail.found_current().map(|f| f.title.as_str()), Some("ana@example.com"));
        assert!(mail.found_step(false));
        assert!(mail.found_dismiss(), "the event is turned down");
        assert_eq!(mail.found_label(), None, "one left needs no pager");
        match mail.found_accept() {
            Some(Action::AddContact { email, phone }) => assert_eq!((email.as_deref(), phone), (Some("ana@example.com"), None)),
            other => panic!("{other:?}"),
        }
        assert!(mail.found_current().is_none());
        // the same message again: nothing that was answered comes back
        mail.open = None;
        mail.refresh_found_in("IS");
        mail.open = Some(9);
        mail.refresh_found_in("IS");
        assert!(mail.found_current().is_none());
        // another message clears the banner of the first
        read_message(&mut mail, 10, "a@example.com", "", "Call me on ana@example.org");
        mail.refresh_found_in("IS");
        assert_eq!(mail.found_current().map(|f| f.title.as_str()), Some("ana@example.org"));
        mail.open = Some(9);
        assert!(mail.found_current().is_none(), "a banner belongs to its message");
    }

    fn compose_of(mail: &Mail) -> &Compose {
        match &mail.sheet {
            Some(Sheet::Compose(c)) => c,
            other => panic!("expected a compose sheet, got {other:?}"),
        }
    }

    fn labels(compose: &Compose) -> Vec<&'static str> {
        compose.form.fields.iter().map(|f| f.label).collect()
    }

    #[test]
    fn cc_bcc_and_reply_to_fields_follow_the_view_menu_and_reach_the_draft() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.open_compose(None);
        assert_eq!(labels(compose_of(&mail)), vec!["To", "Cc", "Subject", "Message"]);
        mail.set_address_field("Bcc", true);
        mail.set_address_field("Reply-To", true);
        assert_eq!(labels(compose_of(&mail)), vec!["To", "Cc", "Bcc", "Reply-To", "Subject", "Message"]);
        if let Some(Sheet::Compose(c)) = &mut mail.sheet {
            c.form.focus_on(2);
            c.form.type_text("hidden@example.com");
            c.form.focus_on(0);
            c.form.type_text("ana@example.com");
            c.form.focus_on(3);
            c.form.type_text("back@example.com");
            c.priority = Priority::Low;
        }
        let draft = mail.draft_from(compose_of(&mail)).unwrap();
        assert_eq!((draft.bcc, draft.reply_to, draft.priority), (vec!["hidden@example.com".to_string()], vec!["back@example.com".to_string()], Priority::Low));
        mail.set_address_field("Cc", false);
        mail.set_address_field("Bcc", false);
        assert_eq!(labels(compose_of(&mail)), vec!["To", "Reply-To", "Subject", "Message"]);
        mail.open_compose(None);
        assert_eq!(labels(compose_of(&mail)), vec!["To", "Reply-To", "Subject", "Message"], "the choice is kept for the next message");
    }

    #[test]
    fn reply_all_and_forward_open_compose_sheets() {
        let mut mail = Mail::new(vec![account("work")]);
        let original = Summary::from_headers("Message-ID: <a@x>\r\nSubject: Plan\r\nFrom: Ana <ana@example.com>\r\nTo: olibuijr@example.com, bo@example.com\r\nCc: cy@example.com\r\n\r\n");
        mail.open_reply_all(&original);
        let c = compose_of(&mail);
        assert_eq!(c.form.value("To"), "ana@example.com, bo@example.com");
        assert_eq!(c.form.value("Cc"), "cy@example.com");
        assert_eq!(c.in_reply_to.as_deref(), Some("<a@x>"));
        mail.open_forward(&original, "Hello");
        let c = compose_of(&mail);
        assert_eq!(c.form.value("Subject"), "Fwd: Plan");
        assert_eq!(c.form.focus, 0, "a forward starts at the recipient");
        assert!(c.form.value("Message").ends_with("Hello"));
        assert_eq!(c.in_reply_to, None);
    }

    fn mail_with_mixed_rows() -> Mail {
        let mut mail = Mail::new(vec![account("work")]);
        let header = |subject: &str, from: &str| format!("Subject: {subject}\r\nFrom: {from}\r\n\r\n");
        let rows = vec![
            Listed { uid: 1, flags: Flags::empty().with(Flag::Seen, true), header: header("Banana", "Cy <cy@example.com>") },
            Listed { uid: 2, flags: Flags::empty().with(Flag::Flagged, true), header: header("avocado", "Bo <bo@example.com>") },
            Listed { uid: 3, flags: Flags::empty().with(Flag::Seen, true).with(Flag::Answered, true), header: header("Cherry", "Ana <ana@example.com>") },
        ];
        mail.apply_listing("work", INBOX, vec![], rows);
        mail
    }

    fn uids(mail: &Mail) -> Vec<u32> {
        mail.visible_rows().iter().map(|r| r.uid).collect()
    }

    #[test]
    fn the_list_sorts_by_key_and_direction() {
        let mut mail = mail_with_mixed_rows();
        assert_eq!(uids(&mail), vec![3, 2, 1], "newest first by default");
        mail.set_ascending(true);
        assert_eq!(uids(&mail), vec![1, 2, 3]);
        mail.sort_by(SortKey::Subject);
        assert_eq!(uids(&mail), vec![2, 1, 3], "A to Z without case");
        mail.set_ascending(false);
        assert_eq!(uids(&mail), vec![3, 1, 2]);
        mail.sort_by(SortKey::From);
        assert_eq!(uids(&mail), vec![3, 2, 1], "a new key starts A to Z: Ana, Bo, Cy");
        mail.set_ascending(false);
        assert_eq!(uids(&mail), vec![1, 2, 3]);
    }

    #[test]
    fn flagged_and_unread_messages_can_be_gathered_by_sorting() {
        let mut mail = mail_with_mixed_rows();
        mail.sort_by(SortKey::Flagged);
        assert_eq!(uids(&mail), vec![2, 1, 3], "flagged first, the rest in the order they were: oldest first when ascending");
        mail.set_ascending(false);
        assert_eq!(uids(&mail), vec![3, 1, 2], "flagged last");
        mail.sort_by(SortKey::Unread);
        mail.set_ascending(true);
        assert_eq!(uids(&mail), vec![2, 1, 3], "the unread message first");
        assert_eq!(SortKey::ALL.len(), 6);
        assert_eq!(SortKey::Unread.label(), "Read Status");
    }

    #[test]
    fn the_filter_keeps_unread_flagged_or_replied_messages() {
        let mut mail = mail_with_mixed_rows();
        mail.filter = Filter::Unread;
        assert_eq!(uids(&mail), vec![2]);
        mail.filter = Filter::Flagged;
        assert_eq!(uids(&mail), vec![2]);
        mail.filter = Filter::Replied;
        assert_eq!(uids(&mail), vec![3]);
        mail.filter = Filter::All;
        assert_eq!(uids(&mail).len(), 3);
    }

    #[test]
    fn conversations_fold_and_unfold() {
        let mut mail = Mail::new(vec![account("work")]);
        let plan = "Message-ID: <a@x>\r\nSubject: Plan\r\nFrom: Ana <ana@example.com>\r\n\r\n";
        let reply = "Message-ID: <b@x>\r\nIn-Reply-To: <a@x>\r\nSubject: Re: Plan\r\nFrom: Bo <bo@example.com>\r\n\r\n";
        let lone = "Message-ID: <c@x>\r\nSubject: Lunch\r\nFrom: Cy <cy@example.com>\r\n\r\n";
        let rows = vec![(1, plan), (2, lone), (3, reply)].into_iter().map(|(uid, h)| Listed { uid, flags: Flags::empty(), header: h.to_string() }).collect();
        mail.apply_listing("work", INBOX, vec![], rows);
        mail.threaded = true;
        let shown = |m: &Mail| m.visible_lines().iter().map(|l| l.row.uid).collect::<Vec<_>>();
        assert_eq!(shown(&mail), vec![1, 3, 2]);
        mail.toggle_conversation(3);
        assert_eq!(shown(&mail), vec![1, 2], "only the first message of the folded conversation");
        assert_eq!(mail.visible_lines()[0].thread, 2, "the count still says two");
        assert!(mail.is_folded(3) && !mail.is_folded(2));
        mail.toggle_conversation(2);
        assert_eq!(shown(&mail), vec![1, 2], "a message alone has nothing to fold");
        mail.expand_all();
        assert_eq!(shown(&mail), vec![1, 3, 2]);
        mail.collapse_all();
        assert_eq!(shown(&mail), vec![1, 2]);
    }

    #[test]
    fn mailbox_sheets_check_the_name() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.mailboxes = vec!["INBOX".into(), "Sent".into(), "Work".into()];
        mail.show_mailbox("Work");
        assert_eq!(mail.move_targets(), vec!["INBOX", "Sent"]);
        mail.open_new_mailbox();
        let Some(Sheet::Mailbox(sheet)) = mail.sheet.clone() else { panic!("expected the mailbox sheet") };
        assert!(mail.new_mailbox_name(&sheet.form, None).is_err(), "empty");
        let named = |name: &str| Form::new(vec![Field::new("Name", "", name)]);
        assert!(mail.new_mailbox_name(&named("sent"), None).is_err(), "taken, without case");
        assert_eq!(mail.new_mailbox_name(&named(" Bills "), None).unwrap(), "Bills");
        assert_eq!(mail.new_mailbox_name(&named("Work"), Some("Work")).unwrap(), "Work", "renaming to itself is allowed");
        assert!(mail.open_rename_mailbox().is_some());
        mail.show_mailbox(INBOX);
        assert!(mail.open_rename_mailbox().is_none(), "the inbox keeps its name");
    }

    #[test]
    fn a_reply_sheet_carries_the_threading_headers() {
        let mut mail = Mail::new(vec![account("work")]);
        let original = Summary::from_headers("Message-ID: <abc@x>\r\nSubject: Plan\r\nFrom: Ana <ana@example.com>\r\n\r\n");
        mail.open_compose(Some(&original));
        match &mail.sheet {
            Some(Sheet::Compose(c)) => {
                assert_eq!(c.form.value("To"), "ana@example.com");
                assert_eq!(c.form.value("Subject"), "Re: Plan");
                assert_eq!(c.in_reply_to.as_deref(), Some("<abc@x>"));
            }
            other => panic!("expected a compose sheet, got {other:?}"),
        }
    }

    #[test]
    fn arrivals_need_a_baseline() {
        let listed_now = vec![listed(5, "a")];
        assert!(arrivals(None, &listed_now).is_empty());
        assert_eq!(arrivals(Some(4), &listed_now).len(), 1);
        assert!(arrivals(Some(5), &listed_now).is_empty());
    }

    #[test]
    fn form_editing_moves_and_types() {
        let mut form = Form::new(vec![Field::new("A", "", ""), Field::new("B", "", "")]);
        form.type_text("hi");
        form.backspace();
        form.type_text("yo");
        form.next_field();
        form.type_text("x");
        assert_eq!(form.value("A"), "hyo");
        assert_eq!(form.value("B"), "x");
        form.next_field();
        assert_eq!(form.focus, 0);
    }

    fn file(name: &str, size: usize) -> Attachment {
        Attachment { name: name.into(), mime: "application/octet-stream".into(), bytes: vec![1; size] }
    }

    fn prompt_form(mail: &Mail) -> &Form {
        match &mail.sheet {
            Some(Sheet::Prompt(prompt)) => &prompt.form,
            other => panic!("expected a prompt, got {other:?}"),
        }
    }

    fn filled(mut form: Form, values: &[(&str, &str)]) -> Form {
        for (label, value) in values {
            form.fields.iter_mut().find(|f| f.label == *label).unwrap().value = value.to_string();
        }
        form
    }

    #[test]
    fn a_message_takes_a_limited_number_of_files_and_bytes() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.open_compose(None);
        let Some(Sheet::Compose(compose)) = &mut mail.sheet else { panic!("no compose sheet") };
        for n in 0..MAX_ATTACHMENTS {
            compose.attach(file(&format!("f{n}"), 10)).unwrap();
        }
        assert_eq!(compose.attach(file("one more", 10)).unwrap_err(), format!("A message carries at most {MAX_ATTACHMENTS} files"));
        compose.detach(0);
        compose.detach(99);
        assert_eq!(compose.attachments.len(), MAX_ATTACHMENTS - 1);
        compose.attachments.clear();
        assert!(compose.attach(file("big", MAX_ATTACHMENT_BYTES)).is_ok());
        assert!(compose.attach(file("too much", 1)).unwrap_err().contains("MB a message may carry"));
    }

    #[test]
    fn the_signature_follows_the_message_and_stays_out_of_the_text_field() {
        assert_eq!(with_signature("Hello", "Ana\nOlafsson"), "Hello\n\n-- \nAna\nOlafsson");
        assert_eq!(with_signature("Hello\n\n", "Ana\n"), "Hello\n\n-- \nAna");
        assert_eq!(with_signature("", "Ana"), "\n\n-- \nAna");
        assert_eq!(with_signature("Hello", "  \n"), "Hello");
        assert_eq!(with_signature_above_quote("Thanks\n\nOn Mon, Ana wrote:\n> hi", "Ana"), "Thanks\n\n-- \nAna\n\nOn Mon, Ana wrote:\n> hi");
        assert_eq!(with_signature_above_quote("Hello", "Ana"), "Hello\n\n-- \nAna", "no quote: the end");
        let mut mail = Mail::new(vec![account("work")]);
        mail.prefs.set_signature("work", "Ana");
        mail.open_compose(None);
        assert_eq!(compose_of(&mail).form.value("Message"), "", "the text field starts empty");
        let mut compose = compose_of(&mail).clone();
        compose.form.fields.iter_mut().find(|f| f.label == "To").unwrap().value = "bo@example.com".into();
        compose.form.fields.iter_mut().find(|f| f.label == "Message").unwrap().value = "Hi".into();
        assert_eq!(mail.draft_from(&compose).unwrap().body, "Hi\n\n-- \nAna");
    }

    #[test]
    fn other_addresses_add_a_from_field_that_must_name_one_of_them() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.prefs.set_aliases("work", vec!["info@example.com".into()]);
        mail.open_compose(None);
        assert_eq!(labels(compose_of(&mail))[..2], ["From", "To"]);
        assert_eq!(compose_of(&mail).form.value("From"), "olibuijr@example.com");
        assert_eq!(compose_of(&mail).form.focus, 1, "the cursor starts at the recipient");
        assert_eq!(mail.from_choices(), vec!["olibuijr@example.com", "info@example.com"]);
        let Some(Sheet::Compose(compose)) = &mut mail.sheet else { panic!("no compose sheet") };
        compose.form.fields.iter_mut().find(|f| f.label == "To").unwrap().value = "bo@example.com".into();
        let choices = vec!["olibuijr@example.com".to_string(), "info@example.com".to_string()];
        compose.cycle_from(&choices);
        assert_eq!(compose.form.value("From"), "info@example.com");
        compose.cycle_from(&choices);
        assert_eq!(compose.form.value("From"), "olibuijr@example.com", "the choices wrap");
        compose.cycle_from(&choices);
        let chosen = compose.clone();
        assert_eq!(mail.draft_from(&chosen).unwrap().from, "info@example.com");
        let mut other = chosen;
        other.form.fields.iter_mut().find(|f| f.label == "From").unwrap().value = "someone@else.example".into();
        assert!(mail.draft_from(&other).unwrap_err().contains("is not one of this account's addresses"));
    }

    #[test]
    fn send_later_adds_and_removes_its_field_without_losing_the_text() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.open_compose(None);
        mail.set_send_later(true);
        assert!(mail.send_later_shown());
        assert_eq!(labels(compose_of(&mail)), vec!["To", "Cc", "Subject", "Send in", "Message"]);
        if let Some(Sheet::Compose(c)) = &mut mail.sheet {
            c.form.focus_on(3);
            c.form.type_text("2h");
        }
        mail.set_send_later(false);
        assert!(!mail.send_later_shown());
        assert_eq!(labels(compose_of(&mail)), vec!["To", "Cc", "Subject", "Message"]);
    }

    #[test]
    fn a_new_sheet_may_not_replace_a_message_being_written() {
        let mut mail = Mail::new(vec![account("work")]);
        assert!(mail.sheet_is_free());
        mail.open_compose(None);
        assert!(!mail.sheet_is_free());
        mail.close_sheet();
        mail.open_search();
        assert!(mail.sheet_is_free());
    }

    #[test]
    fn the_attachments_filter_uses_loaded_files_and_the_message_type() {
        let mut mail = Mail::new(vec![account("work")]);
        let rows = vec![
            Listed { uid: 1, flags: Flags::empty(), header: "Subject: a\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n".into() },
            Listed { uid: 2, flags: Flags::empty(), header: "Subject: b\r\nContent-Type: text/plain\r\n\r\n".into() },
            Listed { uid: 3, flags: Flags::empty(), header: "Subject: c\r\n\r\n".into() },
        ];
        mail.apply_listing("work", INBOX, vec![], rows);
        mail.filter = Filter::Attachments;
        assert_eq!(uids(&mail), vec![1]);
        mail.attachments.insert(3, vec![file("late.pdf", 3)]);
        assert_eq!(uids(&mail), vec![3, 1], "a loaded message is judged by its files");
    }

    #[test]
    fn the_category_keeps_only_that_kind_of_mail() {
        let mut mail = Mail::new(vec![account("work")]);
        let rows = vec![
            Listed { uid: 1, flags: Flags::empty(), header: "Subject: Lunch?\r\nFrom: Ana <ana@example.com>\r\n\r\n".into() },
            Listed { uid: 2, flags: Flags::empty(), header: "Subject: Your receipt\r\nFrom: shop@store.example\r\n\r\n".into() },
            Listed { uid: 3, flags: Flags::empty(), header: "Subject: Hello\r\nFrom: news@store.example\r\nList-Unsubscribe: <mailto:u@store.example>\r\n\r\n".into() },
        ];
        mail.apply_listing("work", INBOX, vec![], rows);
        assert_eq!(uids(&mail).len(), 3);
        mail.category = Some(Category::Primary);
        assert_eq!(uids(&mail), vec![1]);
        mail.category = Some(Category::Transactions);
        assert_eq!(uids(&mail), vec![2]);
        mail.category = Some(Category::Promotions);
        assert_eq!(uids(&mail), vec![3]);
        mail.category = Some(Category::Updates);
        assert!(uids(&mail).is_empty());
    }

    #[test]
    fn a_smart_mailbox_applies_its_search_and_a_new_search_leaves_it() {
        let mut mail = mail_with_mixed_rows();
        mail.prefs.smart.push(SmartMailbox { name: "From Ana".into(), query: "from:ana".into() });
        assert!(mail.apply_smart(0));
        assert_eq!(mail.smart_active, Some(0));
        assert_eq!(uids(&mail), vec![3]);
        assert!(!mail.apply_smart(5));
        mail.apply_search("bo");
        assert_eq!(mail.smart_active, None);
    }

    #[test]
    fn the_listing_remembers_which_messages_just_arrived_for_the_rules() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_listing("work", INBOX, vec![], vec![listed(9, "old")]);
        assert!(mail.arrived.is_empty());
        mail.apply_listing("work", INBOX, vec![], vec![listed(9, "old"), listed(12, "Fundur"), listed(13, "Annað")]);
        let arrived: Vec<(&str, u32)> = mail.arrived.iter().map(|(a, r)| (a.as_str(), r.uid)).collect();
        assert_eq!(arrived, vec![("work", 12), ("work", 13)]);
        mail.apply_listing("work", INBOX, vec![], vec![listed(9, "old"), listed(12, "Fundur"), listed(13, "Annað")]);
        assert!(mail.arrived.is_empty(), "only the newest listing's arrivals are kept");
    }

    #[test]
    fn the_rule_form_checks_its_name_criteria_actions_and_mailbox() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.mailboxes = vec![INBOX.into(), "Receipts".into()];
        mail.open_rule();
        let form = prompt_form(&mail).clone();
        assert_eq!(mail.rule_from_form(&form).unwrap_err(), "Name the rule");
        let form = filled(form, &[("Name", "Shops")]);
        assert!(mail.rule_from_form(&form).unwrap_err().contains("something to look for"));
        let form = filled(form, &[("From contains", "shop.example")]);
        assert!(mail.rule_from_form(&form).unwrap_err().contains("Choose what the rule does"));
        let form = filled(form, &[("Move to mailbox", "receipts"), ("Mark as read", "yes")]);
        let rule = mail.rule_from_form(&form).unwrap();
        assert_eq!(rule.actions, vec![rules::Action::MoveTo("Receipts".into()), rules::Action::MarkRead], "the mailbox is spelled as the account has it");
        let missing = filled(form.clone(), &[("Move to mailbox", "Nowhere")]);
        assert_eq!(mail.rule_from_form(&missing).unwrap_err(), "There is no mailbox named Nowhere");
        mail.prefs.rules.push(rule);
        assert!(mail.rule_from_form(&form).unwrap_err().contains("already a rule named Shops"));
    }

    #[test]
    fn the_smart_mailbox_form_needs_a_name_and_a_search() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.apply_search("from:ana");
        mail.open_smart_mailbox();
        let form = prompt_form(&mail).clone();
        assert_eq!(form.value("Search"), "from:ana", "it starts from the search applied now");
        assert_eq!(mail.smart_from_form(&form).unwrap_err(), "Name the smart mailbox");
        let named = filled(form.clone(), &[("Name", "Ana")]);
        assert_eq!(mail.smart_from_form(&named).unwrap(), SmartMailbox { name: "Ana".into(), query: "from:ana".into() });
        assert!(mail.smart_from_form(&filled(named.clone(), &[("Search", "  ")])).unwrap_err().contains("searches for"));
        mail.prefs.smart.push(SmartMailbox { name: "ana".into(), query: "x".into() });
        assert!(mail.smart_from_form(&named).unwrap_err().contains("already a smart mailbox"));
    }

    #[test]
    fn mail_settings_set_the_signature_other_addresses_and_undo_delay() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.open_settings().unwrap();
        let form = prompt_form(&mail).clone();
        let set = filled(form.clone(), &[("Signature", "Ana\nOlafsson"), ("Other addresses", "Info <info@example.com>, info@example.com, ana@example.org"), ("Undo send (seconds)", "10")]);
        mail.apply_settings(&set).unwrap();
        assert_eq!(mail.prefs.signature("work"), "Ana\nOlafsson");
        assert_eq!(mail.prefs.aliases_of("work"), ["info@example.com", "ana@example.org"], "bare addresses, each once");
        assert_eq!(mail.prefs.undo_delay, 10);
        mail.open_settings().unwrap();
        assert_eq!(prompt_form(&mail).value("Other addresses"), "info@example.com, ana@example.org");
        assert_eq!(prompt_form(&mail).value("Undo send (seconds)"), "10");
        assert!(mail.apply_settings(&filled(form.clone(), &[("Other addresses", "not an address")])).is_err());
        assert!(mail.apply_settings(&filled(form.clone(), &[("Undo send (seconds)", "soon")])).unwrap_err().contains("number of seconds"));
        assert!(mail.apply_settings(&filled(form, &[("Undo send (seconds)", "31")])).unwrap_err().contains("at most 30"));
        assert!(Mail::new(vec![]).open_settings().is_err());
    }

    #[test]
    fn a_reply_comes_from_the_alias_it_was_addressed_to_and_quotes_the_original() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.prefs.set_aliases("work", vec!["info@example.com".into()]);
        let original = Summary { from: "Ana <ana@example.org>".into(), to: "Info <INFO@example.com>".into(), subject: "Hi".into(), ..Summary::default() };
        assert_eq!(mail.sent_from(&original), "info@example.com");
        let own = Summary { to: "olibuijr@example.com".into(), ..original.clone() };
        assert_eq!(mail.sent_from(&own), "olibuijr@example.com", "the account's own address when it was addressed");
        mail.open_reply(&original, Some("first line\nsecond"));
        let form = match &mail.sheet {
            Some(Sheet::Compose(compose)) => compose.form.clone(),
            _ => panic!("a compose sheet"),
        };
        assert_eq!(form.value("From"), "info@example.com");
        assert_eq!(form.value("To"), "ana@example.org");
        assert!(form.value("Message").contains("Ana wrote:"), "{}", form.value("Message"));
        assert!(form.value("Message").contains("> first line\n> second"));
    }

    #[test]
    fn reply_all_is_offered_only_when_someone_else_is_on_the_message() {
        let mail = Mail::new(vec![account("work")]);
        let alone = Summary { from: "Ana <ana@example.org>".into(), to: "olibuijr@example.com".into(), ..Summary::default() };
        assert!(!mail.has_other_recipients(&alone));
        let group = Summary { from: "Ana <ana@example.org>".into(), to: "olibuijr@example.com, Bo <bo@example.org>".into(), ..Summary::default() };
        assert!(mail.has_other_recipients(&group));
        let copied = Summary { from: "ana@example.org".into(), cc: "carl@example.org".into(), ..Summary::default() };
        assert!(mail.has_other_recipients(&copied));
    }

    #[test]
    fn forwarding_keeps_the_files_of_the_original_within_the_message_limits() {
        let mut mail = Mail::new(vec![account("work")]);
        let original = Summary { subject: "Plan".into(), from: "Ana <ana@example.org>".into(), ..Summary::default() };
        let file = |name: &str, size: usize| Attachment { name: name.into(), mime: "application/pdf".into(), bytes: vec![0; size] };
        let files: Vec<Attachment> = (0..10).map(|n| file(&format!("f{n}.pdf"), 1000)).collect();
        mail.open_forward_with(&original, "text", files);
        match &mail.sheet {
            Some(Sheet::Compose(compose)) => assert_eq!(compose.attachments.len(), MAX_ATTACHMENTS),
            _ => panic!("a compose sheet"),
        }
        let kept = within_limits(vec![file("big", MAX_ATTACHMENT_BYTES), file("more", 1)]);
        assert_eq!(kept.len(), 1, "the second file would pass the size limit");
        assert!(within_limits(Vec::new()).is_empty());
    }

    #[test]
    fn the_settings_toggle_quoting_and_plain_text() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.open_settings().unwrap();
        let form = prompt_form(&mail).clone();
        assert_eq!(form.value("Quote the original in replies"), "yes");
        assert_eq!(form.value("Plain text messages only"), "no");
        assert!(form.fields.iter().filter(|f| f.label.starts_with("Quote") || f.label.starts_with("Plain")).all(|f| f.choice));
        mail.apply_settings(&filled(form.clone(), &[("Quote the original in replies", "no"), ("Plain text messages only", "yes")])).unwrap();
        assert!(!mail.prefs.quote_replies && mail.prefs.plain_text);
        mail.open_settings().unwrap();
        assert_eq!(prompt_form(&mail).value("Plain text messages only"), "yes");
    }

    #[test]
    fn editing_an_account_keeps_its_id_and_asks_for_the_password_only_when_it_moves() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.open_edit_account().unwrap();
        let form = prompt_form(&mail).clone();
        assert_eq!(form.value("Incoming server"), "imap.example.com");
        assert_eq!(form.value("Outgoing port"), "587");
        let old = mail.accounts[0].clone();
        let renamed = filled(form.clone(), &[("Name", "Office"), ("Outgoing server", "smtp.office.example")]);
        let (edited, password) = mail.edited_account(&renamed, &old).unwrap();
        assert_eq!((edited.id.as_str(), edited.name.as_str(), password), ("work", "Office", None));
        assert_eq!(edited.smtp.as_ref().map(|s| s.host.as_str()), Some("smtp.office.example"));
        assert_eq!(edited.services, old.services);
        let moved = filled(form.clone(), &[("Incoming server", "imap.other.example")]);
        assert!(mail.edited_account(&moved, &old).unwrap_err().contains("Enter the password again"));
        let (_, password) = mail.edited_account(&filled(moved, &[("Password", "new")]), &old).unwrap();
        assert_eq!(password.as_deref(), Some("new"));
        mail.replace_account(edited, Some("kept".into()));
        assert_eq!(mail.accounts[0].name, "Office");
        assert_eq!(mail.passwords.get("work").map(String::as_str), Some("kept"));
    }

    #[test]
    fn removing_an_account_forgets_everything_kept_for_it() {
        let mut mail = Mail::new(vec![account("work"), account("home")]);
        mail.passwords.insert("work".into(), "pw".into());
        mail.baseline.insert("work".into(), 5);
        mail.prefs.set_signature("work", "Ana");
        mail.prefs.toggle_favorite("work", "INBOX");
        mail.prefs.toggle_favorite("home", "INBOX");
        mail.forget_account("work");
        assert_eq!(mail.accounts.len(), 1);
        assert_eq!(mail.accounts[0].id, "home");
        assert!(mail.passwords.is_empty() && mail.baseline.is_empty());
        assert_eq!(mail.prefs.signature("work"), "");
        assert_eq!(mail.prefs.favorites.len(), 1);
        assert_eq!(mail.account, 0);
        mail.forget_account("home");
        assert!(mail.accounts.is_empty());
        mail.forget_account("nobody");
    }

    #[test]
    fn remind_me_needs_an_open_message_with_an_id() {
        let mut mail = Mail::new(vec![account("work")]);
        assert_eq!(mail.open_remind().unwrap_err(), "Open a message first");
        let rows = vec![
            Listed { uid: 1, flags: Flags::empty(), header: "Message-ID: <a@x>\r\nSubject: with\r\n\r\n".into() },
            Listed { uid: 2, flags: Flags::empty(), header: "Subject: without\r\n\r\n".into() },
        ];
        mail.apply_listing("work", INBOX, vec![], rows);
        mail.open_message(2);
        assert!(mail.open_remind().unwrap_err().contains("no Message-ID"));
        mail.open_message(1);
        mail.open_remind().unwrap();
        assert_eq!(mail.sheet.as_ref().map(Sheet::title), Some("Remind Me"));
    }

    #[test]
    fn every_sheet_has_a_title_and_an_action() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.open_setup();
        assert_eq!((mail.sheet.as_ref().unwrap().title(), mail.sheet.as_ref().unwrap().action()), ("Add Account", "Add Account"));
        mail.open_compose(None);
        assert_eq!((mail.sheet.as_ref().unwrap().title(), mail.sheet.as_ref().unwrap().action()), ("New Message", "Send"));
        mail.open_redirect();
        assert_eq!((mail.sheet.as_ref().unwrap().title(), mail.sheet.as_ref().unwrap().action()), ("Redirect", "Redirect"));
        mail.open_remove_account().unwrap();
        assert_eq!((mail.sheet.as_ref().unwrap().title(), mail.sheet.as_ref().unwrap().action()), ("Remove Account", "Remove"));
        assert!(is_multiline("Message") && is_multiline("Signature") && !is_multiline("Subject"));
    }

    fn message_compose(text: &str) -> Compose {
        let form = Form::new(vec![Field::new("To", "", "a@example.com"), Field::new("Message", "", text)]);
        let mut compose = Compose::new(form, None, vec![], Priority::Normal, vec![], String::new());
        compose.form.focus_on(1);
        compose
    }

    #[test]
    fn typed_text_takes_the_style_in_force_and_a_deleted_character_takes_its_style_with_it() {
        let mut compose = message_compose("ab");
        assert_eq!(compose.styles, vec![Style::default(); 2]);
        compose.toggle_bold();
        compose.edit(|form| form.type_text("cd"));
        compose.toggle_bold();
        compose.toggle_italic();
        compose.edit(|form| form.type_text("e"));
        assert_eq!(compose.message(), "abcde");
        let looks: Vec<(bool, bool)> = compose.styles.iter().map(|s| (s.bold, s.italic)).collect();
        assert_eq!(looks, vec![(false, false), (false, false), (true, false), (true, false), (false, true)]);
        compose.edit(|form| {
            form.backspace();
            form.backspace();
        });
        assert_eq!(compose.styles.len(), 3);
        compose.edit(|form| form.type_text("é"));
        assert_eq!(compose.styles.len(), 4, "styles count characters, not bytes");
        assert!(!compose.styles[3].bold && compose.styles[3].italic);
    }

    #[test]
    fn an_edit_of_another_field_leaves_the_message_styles_alone() {
        let mut compose = message_compose("ab");
        compose.toggle_bold();
        compose.form.focus_on(0);
        compose.edit(|form| form.type_text("x"));
        assert_eq!(compose.form.value("To"), "a@example.comx");
        assert_eq!(compose.styles.len(), 2);
        assert!(compose.styles.iter().all(|s| s.is_plain()));
    }

    #[test]
    fn colour_and_underline_follow_the_typed_text_and_clearing_makes_it_plain_again() {
        let mut compose = message_compose("");
        compose.toggle_underline();
        compose.set_text_color(Some(TextColor::Blue));
        compose.edit(|form| form.type_text("link"));
        assert!(compose.styles.iter().all(|s| s.underline && s.color == Some(TextColor::Blue)));
        compose.clear_formatting();
        assert!(compose.styles.iter().all(|s| s.is_plain()) && compose.typing.is_plain());
        compose.set_text_color(None);
        assert!(compose.typing.is_plain());
    }

    #[test]
    fn a_bullet_and_an_indent_go_at_the_start_of_the_last_line_and_come_off_again() {
        let mut compose = message_compose("first\nsecond");
        compose.toggle_bullet();
        assert_eq!(compose.message(), "first\n\u{2022} second");
        assert_eq!(compose.styles.len(), compose.message().chars().count());
        compose.indent(true);
        assert_eq!(compose.message(), "first\n    \u{2022} second");
        compose.indent(false);
        compose.toggle_bullet();
        assert_eq!(compose.message(), "first\nsecond");
        compose.indent(false);
        assert_eq!(compose.message(), "first\nsecond", "nothing to take off");
        let mut single = message_compose("one");
        single.toggle_bullet();
        assert_eq!(single.message(), "\u{2022} one");
    }

    #[test]
    fn a_bulleted_line_keeps_the_styles_of_its_text() {
        let mut compose = message_compose("");
        compose.toggle_bold();
        compose.edit(|form| form.type_text("bold"));
        compose.toggle_bullet();
        assert_eq!(compose.message(), "\u{2022} bold");
        assert!(compose.styles[0].is_plain() && compose.styles[1].is_plain());
        assert!(compose.styles[2..].iter().all(|s| s.bold));
    }

    #[test]
    fn a_styled_message_gets_an_html_part_in_the_draft_and_a_plain_one_does_not() {
        let mail = Mail::new(vec![account("work")]);
        let mut compose = message_compose("");
        assert_eq!(mail.draft_from(&compose).unwrap().html, None);
        compose.toggle_bold();
        compose.edit(|form| form.type_text("hi"));
        let draft = mail.draft_from(&compose).unwrap();
        assert!(draft.html.as_deref().is_some_and(|html| html.contains("<span style=\"font-weight: bold\">hi</span>")));
        assert_eq!(draft.body, "hi");
    }

    #[test]
    fn spelling_marks_step_around_and_a_correction_replaces_the_word_in_its_style() {
        let mut compose = message_compose("");
        compose.toggle_bold();
        compose.edit(|form| form.type_text("hello wrold and teh end"));
        compose.set_misspelled(vec![6..11, 16..19]);
        assert_eq!(compose.current_misspelling(), Some((6..11, "wrold".to_string())));
        compose.next_misspelling();
        assert_eq!(compose.current_misspelling(), Some((16..19, "teh".to_string())));
        compose.next_misspelling();
        assert_eq!(compose.misspelled_at, 0, "the marks wrap");
        assert!(compose.correct_misspelling("world"));
        assert_eq!(compose.message(), "hello world and teh end");
        assert!(compose.misspelled.is_empty(), "the text changed, so the marks are cleared");
        assert_eq!(compose.styles.len(), compose.message().chars().count());
        assert!(compose.styles.iter().all(|s| s.bold));
        assert!(!compose.correct_misspelling("x"), "no word in view");
        compose.set_misspelled(vec![16..19]);
        compose.edit(|form| form.type_text("!"));
        assert!(compose.misspelled.is_empty(), "typing clears the marks");
    }

    #[test]
    fn proofreading_marks_the_slips_and_accepting_edits_the_message_in_its_style() {
        let mut compose = message_compose("");
        compose.toggle_bold();
        compose.edit(|form| form.type_text("Say it it again, red,green"));
        assert_eq!(compose.start_proofread(), 2);
        let (marks, at) = compose.proofread_marks();
        assert_eq!((marks.len(), at), (2, 0));
        assert!(compose.proofread_line().unwrap().starts_with("Suggestion 1 of 2."), "{:?}", compose.proofread_line());
        assert_eq!(compose.accept_suggestion(false), 1);
        assert_eq!(compose.message(), "Say it again, red,green");
        assert_eq!(compose.styles.len(), compose.message().chars().count());
        assert!(compose.styles.iter().all(|s| s.bold));
        assert_eq!(compose.proofread.as_ref().map(Panel::len), Some(1), "the other suggestion moved with the edit");
        compose.skip_suggestion();
        assert!(compose.proofread.is_none() && compose.proofread_line().is_none());
        // all at once, and typing drops the suggestions
        let mut again = message_compose("the the  cat red,green");
        assert_eq!(again.start_proofread(), 3);
        assert_eq!(again.accept_suggestion(true), 3);
        assert_eq!(again.message(), "the cat red, green");
        assert!(again.proofread.is_none());
        let mut typed = message_compose("so so");
        assert_eq!(typed.start_proofread(), 1);
        typed.edit(|form| form.type_text("!"));
        assert!(typed.proofread.is_none(), "the suggestions no longer fit the text");
        assert_eq!(message_compose("").start_proofread(), 0);
    }

    #[test]
    fn a_correction_longer_than_the_word_keeps_the_text_after_it_aligned_with_its_styles() {
        let mut compose = message_compose("ab cd");
        compose.styles[3].italic = true;
        compose.set_misspelled(vec![0..2]);
        assert!(compose.correct_misspelling("abcdef"));
        assert_eq!(compose.message(), "abcdef cd");
        assert_eq!(compose.styles.len(), 9);
        assert!(compose.styles[7].italic, "the italic 'c' moved to index 7");
        assert!(!compose.styles[3].italic);
    }

    #[test]
    fn the_activity_list_keeps_the_newest_lines_and_opens_as_a_sheet() {
        let mut mail = Mail::new(vec![account("work")]);
        mail.open_activity();
        assert_eq!(prompt_form(&mail).value("Activity"), "Nothing has happened yet");
        for n in 0..MAX_ACTIVITY + 5 {
            mail.log(format!("line {n}"));
        }
        assert_eq!(mail.activity.len(), MAX_ACTIVITY);
        assert_eq!(mail.activity[0], "line 5");
        mail.open_activity();
        let text = prompt_form(&mail).value("Activity").to_string();
        assert!(text.starts_with("line 5\n") && text.ends_with(&format!("line {}", MAX_ACTIVITY + 4)));
        assert_eq!((mail.sheet.as_ref().unwrap().title(), mail.sheet.as_ref().unwrap().action()), ("Activity", "Done"));
    }

    #[test]
    fn only_the_first_line_of_a_conversation_is_its_head() {
        let mut mail = Mail::new(vec![account("work")]);
        let rows = vec![
            Listed { uid: 1, flags: Flags::empty(), header: "Message-ID: <a@x>\r\nSubject: Plan\r\n\r\n".into() },
            Listed { uid: 2, flags: Flags::empty(), header: "Message-ID: <b@x>\r\nIn-Reply-To: <a@x>\r\nReferences: <a@x>\r\nSubject: Re: Plan\r\n\r\n".into() },
            Listed { uid: 3, flags: Flags::empty(), header: "Message-ID: <c@x>\r\nSubject: Other\r\n\r\n".into() },
        ];
        mail.apply_listing("work", INBOX, vec![], rows);
        assert!(mail.visible_lines().iter().all(|l| l.head && l.thread == 1), "the flat list has no conversations");
        mail.threaded = true;
        let lines = mail.visible_lines();
        let heads: Vec<(u32, bool)> = lines.iter().map(|l| (l.row.uid, l.head)).collect();
        let plan: Vec<&(u32, bool)> = heads.iter().filter(|(uid, _)| *uid != 3).collect();
        assert_eq!(plan.iter().filter(|(_, head)| *head).count(), 1, "one head in the two-message conversation");
        assert!(heads.contains(&(3, true)));
    }
}
