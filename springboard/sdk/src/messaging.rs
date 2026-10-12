//! Communication notifications and messaging requests: the people in a conversation, the notice an incoming
//! message arrives as, and the send and search requests an app can receive.
//!
//! Mirrors the parts of `INPerson`, `INSendMessageIntent`, `INSearchForMessagesIntent` and the communication
//! notification content that BifrOSt can carry without a messaging service:
//!
//! - [`Person`]: a participant, with a name, a handle (phone number, address or account id) and whether it is the
//!   current user. [`Person::same_as`] compares by handle when both have one, else by name.
//! - [`Conversation`]: the thread a message belongs to. A conversation with more than one other participant is a
//!   group; its title is the given title, else the names of the others ([`Conversation::display_title`]).
//! - [`Notice`]: an incoming message. [`Notice::request`] builds the `usernotif` request for it; notices of one
//!   conversation share the conversation id as their thread, so the shell stacks them.
//! - [`SendRequest`]: a send-message intent. [`SendRequest::check`] applies the rules before the app sends.
//! - [`SearchRequest`] and [`search`]: a search for messages by text, sender and conversation, newest first.
//!
//! Gaps: no shell listing of these notices (the shell still draws its own banners), no contact lookup, no
//! attachments, reactions or edits, and no Focus or relevance ranking. The limits below are BifrOSt's own.

use crate::usernotif::Request;

/// The most recipients one send request may name (a BifrOSt limit).
pub const MAX_RECIPIENTS: usize = 50;
/// The result limit a search uses when its request does not set one.
pub const DEFAULT_LIMIT: usize = 25;
/// The largest result limit a search may ask for.
pub const MAX_LIMIT: usize = 100;

/// A participant of a conversation (`INPerson`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    /// phone number, address or account id; empty when unknown
    pub handle: String,
    /// true for the current user, who is a recipient of every incoming message
    pub me: bool,
}

impl Person {
    pub fn new(name: &str, handle: &str) -> Self {
        Person { name: name.into(), handle: handle.into(), me: false }
    }

    /// Marks this person as the current user.
    pub fn current_user(mut self) -> Self {
        self.me = true;
        self
    }

    /// The name shown for the person: the name, else the handle, else "Unknown".
    pub fn display_name(&self) -> String {
        let name = self.name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        let handle = self.handle.trim();
        if !handle.is_empty() {
            return handle.to_string();
        }
        "Unknown".to_string()
    }

    /// Up to two initials for an avatar: the first letters of the first and the last word, upper case.
    pub fn initials(&self) -> String {
        let words: Vec<&str> = self.name.split_whitespace().collect();
        let picks: Vec<&str> = match words.len() {
            0 => return String::new(),
            1 => vec![words[0]],
            n => vec![words[0], words[n - 1]],
        };
        picks
            .iter()
            .filter_map(|w| w.chars().next())
            .flat_map(|c| c.to_uppercase())
            .collect()
    }

    /// Whether the two entries name the same person: by handle when both have one, else by name, ignoring case.
    pub fn same_as(&self, other: &Person) -> bool {
        let (a, b) = (self.handle.trim(), other.handle.trim());
        if !a.is_empty() && !b.is_empty() {
            return a.eq_ignore_ascii_case(b);
        }
        let (x, y) = (self.name.trim(), other.name.trim());
        !x.is_empty() && x.eq_ignore_ascii_case(y)
    }
}

/// The thread a message belongs to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conversation {
    pub id: String,
    /// the group's name; empty for a direct conversation or an unnamed group
    pub title: String,
    pub participants: Vec<Person>,
}

impl Conversation {
    pub fn new(id: &str, title: &str, participants: Vec<Person>) -> Self {
        Conversation { id: id.into(), title: title.into(), participants }
    }

    /// The participants other than the current user.
    pub fn others(&self) -> Vec<&Person> {
        self.participants.iter().filter(|p| !p.me).collect()
    }

    /// A group has more than one other participant.
    pub fn is_group(&self) -> bool {
        self.others().len() > 1
    }

    /// The title shown for the conversation: its own title, else the names of the others. Three names are listed
    /// at most, the rest counted: "Ada and Grace", "Ada, Grace, Linus and 1 more".
    pub fn display_title(&self) -> String {
        let title = self.title.trim();
        if !title.is_empty() {
            return title.to_string();
        }
        let others = self.others();
        let more = others.len().saturating_sub(3);
        let names: Vec<String> = others.iter().take(3).map(|p| p.display_name()).collect();
        if names.is_empty() {
            return "Unknown".to_string();
        }
        if more > 0 {
            return format!("{} and {} more", names.join(", "), more);
        }
        let last = names.len() - 1;
        if last == 0 {
            return names[0].clone();
        }
        format!("{} and {}", names[..last].join(", "), names[last])
    }
}

/// An incoming message, shown as a communication notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub id: String,
    pub conversation: Conversation,
    pub sender: Person,
    pub text: String,
    /// when the message was sent, in milliseconds since the Unix epoch
    pub sent_ms: u64,
}

impl Notice {
    /// The sender's name; in a group, "Grace in Ada and Grace" names the group as well.
    pub fn title(&self) -> String {
        let sender = self.sender.display_name();
        if self.conversation.is_group() {
            format!("{} in {}", sender, self.conversation.display_title())
        } else {
            sender
        }
    }

    /// The text trimmed; "New message" when the message has no text.
    pub fn body(&self) -> String {
        let text = self.text.trim();
        if text.is_empty() {
            "New message".to_string()
        } else {
            text.to_string()
        }
    }

    /// The notification request for this message, on the conversation's thread.
    pub fn request(&self) -> Request {
        Request::new(&self.id, &self.title()).body(&self.body()).thread(&self.conversation.id)
    }
}

/// Why a send request is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageError {
    NoRecipients,
    TooManyRecipients,
    /// the current user cannot be a recipient of a message they send
    CurrentUserIsRecipient,
    EmptyText,
}

impl std::fmt::Display for MessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            MessageError::NoRecipients => "the message has no recipients",
            MessageError::TooManyRecipients => "the message has too many recipients",
            MessageError::CurrentUserIsRecipient => "the current user cannot be a recipient",
            MessageError::EmptyText => "the message text is empty",
        };
        f.write_str(text)
    }
}

impl std::error::Error for MessageError {}

/// A send-message intent: the recipients and the text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SendRequest {
    pub recipients: Vec<Person>,
    pub text: String,
}

impl SendRequest {
    /// Applies the rules of a send request, in this order: at least one recipient, at most [`MAX_RECIPIENTS`], no
    /// current user among them, and text that is not blank.
    pub fn check(&self) -> Result<(), MessageError> {
        if self.recipients.is_empty() {
            return Err(MessageError::NoRecipients);
        }
        if self.recipients.len() > MAX_RECIPIENTS {
            return Err(MessageError::TooManyRecipients);
        }
        if self.recipients.iter().any(|p| p.me) {
            return Err(MessageError::CurrentUserIsRecipient);
        }
        if self.text.trim().is_empty() {
            return Err(MessageError::EmptyText);
        }
        Ok(())
    }

    /// The recipients with repeats removed: a person named twice is sent to once.
    pub fn unique_recipients(&self) -> Vec<&Person> {
        let mut out: Vec<&Person> = Vec::new();
        for p in &self.recipients {
            if !out.iter().any(|q| q.same_as(p)) {
                out.push(p);
            }
        }
        out
    }
}

/// A search for messages (`INSearchForMessagesIntent`): every criterion that is set must match.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchRequest {
    /// the words to find; empty finds every message that passes the other criteria
    pub text: String,
    pub sender: Option<Person>,
    pub conversation_id: Option<String>,
    /// the most results to return; 0 means [`DEFAULT_LIMIT`]
    pub limit: usize,
}

impl SearchRequest {
    /// The text in lower case with its words separated by single spaces.
    pub fn query(&self) -> String {
        self.text.split_whitespace().collect::<Vec<&str>>().join(" ").to_lowercase()
    }

    /// The limit in use: [`DEFAULT_LIMIT`] when unset, at most [`MAX_LIMIT`].
    pub fn effective_limit(&self) -> usize {
        match self.limit {
            0 => DEFAULT_LIMIT,
            n => n.min(MAX_LIMIT),
        }
    }

    /// Whether a notice passes every criterion that is set.
    pub fn matches(&self, notice: &Notice) -> bool {
        let query = self.query();
        if !query.is_empty() && !notice.text.to_lowercase().contains(query.as_str()) {
            return false;
        }
        if let Some(sender) = &self.sender {
            if !sender.same_as(&notice.sender) {
                return false;
            }
        }
        if let Some(id) = &self.conversation_id {
            if *id != notice.conversation.id {
                return false;
            }
        }
        true
    }
}

/// The notices a search finds, newest first, at most [`SearchRequest::effective_limit`] of them.
pub fn search<'a>(request: &SearchRequest, notices: &'a [Notice]) -> Vec<&'a Notice> {
    let mut found: Vec<&Notice> = notices.iter().filter(|n| request.matches(n)).collect();
    found.sort_by(|a, b| b.sent_ms.cmp(&a.sent_ms));
    found.truncate(request.effective_limit());
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Person {
        Person::new("Ólafur", "").current_user()
    }

    fn direct(name: &str) -> Conversation {
        Conversation::new(&format!("c-{}", name), "", vec![me(), Person::new(name, "")])
    }

    fn notice(id: &str, conv: Conversation, sender: Person, text: &str, sent_ms: u64) -> Notice {
        Notice { id: id.into(), conversation: conv, sender, text: text.into(), sent_ms }
    }

    #[test]
    fn initials_use_first_and_last_word() {
        assert_eq!(Person::new("ada lovelace", "").initials(), "AL");
        assert_eq!(Person::new("Mary Ann Evans", "").initials(), "ME");
        assert_eq!(Person::new("plato", "").initials(), "P");
        assert_eq!(Person::new("Ólafur Búi Ólafsson", "").initials(), "ÓÓ");
        assert_eq!(Person::new("   ", "+3541").initials(), "");
    }

    #[test]
    fn display_name_falls_back_to_handle_then_unknown() {
        assert_eq!(Person::new("Ada ", "").display_name(), "Ada");
        assert_eq!(Person::new("  ", "+3541234").display_name(), "+3541234");
        assert_eq!(Person::new("", "").display_name(), "Unknown");
    }

    #[test]
    fn same_as_compares_handles_when_both_have_one() {
        assert!(Person::new("Ada", "ada@x.is").same_as(&Person::new("Lady", "ADA@x.is")));
        assert!(!Person::new("Ada", "+3541").same_as(&Person::new("Ada", "+3542")));
        assert!(Person::new("ADA", "").same_as(&Person::new("ada", "")));
        assert!(!Person::new("", "").same_as(&Person::new("", "")));
    }

    #[test]
    fn group_is_more_than_one_other_and_title_lists_names() {
        let two = Conversation::new("g1", "", vec![me(), Person::new("Ada", ""), Person::new("Grace", "")]);
        assert!(two.is_group());
        assert_eq!(two.display_title(), "Ada and Grace");
        assert!(!direct("Ada").is_group());
        assert_eq!(direct("Ada").display_title(), "Ada");

        let four = Conversation::new(
            "g2",
            "",
            vec![
                me(),
                Person::new("Ada", ""),
                Person::new("Grace", ""),
                Person::new("Linus", ""),
                Person::new("Alan", ""),
            ],
        );
        assert_eq!(four.display_title(), "Ada, Grace, Linus and 1 more");
        let named = Conversation::new("g3", " Team ", vec![me(), Person::new("Ada", ""), Person::new("Grace", "")]);
        assert_eq!(named.display_title(), "Team");
    }

    #[test]
    fn notice_title_and_body_follow_the_group_rule() {
        let group = Conversation::new("g1", "", vec![me(), Person::new("Ada", ""), Person::new("Grace", "")]);
        let n = notice("n1", group, Person::new("Grace", ""), "  hi  ", 5);
        assert_eq!(n.title(), "Grace in Ada and Grace");
        assert_eq!(n.body(), "hi");

        let d = notice("n2", direct("Ada"), Person::new("Ada", ""), "   ", 6);
        assert_eq!(d.title(), "Ada");
        assert_eq!(d.body(), "New message");
    }

    #[test]
    fn send_request_rules_apply_in_order() {
        let empty = SendRequest { recipients: vec![], text: "x".into() };
        assert_eq!(empty.check(), Err(MessageError::NoRecipients));

        let to_me = SendRequest { recipients: vec![me()], text: "x".into() };
        assert_eq!(to_me.check(), Err(MessageError::CurrentUserIsRecipient));

        let blank = SendRequest { recipients: vec![Person::new("Ada", "")], text: "  ".into() };
        assert_eq!(blank.check(), Err(MessageError::EmptyText));

        let many: Vec<Person> = (0..=MAX_RECIPIENTS).map(|i| Person::new(&format!("P{}", i), "")).collect();
        let too_many = SendRequest { recipients: many, text: "x".into() };
        assert_eq!(too_many.check(), Err(MessageError::TooManyRecipients));

        let ok = SendRequest { recipients: vec![Person::new("Ada", "")], text: "x".into() };
        assert_eq!(ok.check(), Ok(()));
    }

    #[test]
    fn unique_recipients_drops_repeats() {
        let req = SendRequest {
            recipients: vec![
                Person::new("Ada", "+1"),
                Person::new("Grace", ""),
                Person::new("Lady", "+1"),
                Person::new("grace", ""),
            ],
            text: "x".into(),
        };
        let names: Vec<String> = req.unique_recipients().iter().map(|p| p.name.clone()).collect();
        assert_eq!(names, vec!["Ada", "Grace"]);
    }

    #[test]
    fn query_collapses_spaces_and_limit_is_clamped() {
        let q = SearchRequest { text: "  Hello   WORLD ".into(), ..Default::default() };
        assert_eq!(q.query(), "hello world");
        assert_eq!(q.effective_limit(), DEFAULT_LIMIT);
        let big = SearchRequest { limit: 500, ..Default::default() };
        assert_eq!(big.effective_limit(), MAX_LIMIT);
        let seven = SearchRequest { limit: 7, ..Default::default() };
        assert_eq!(seven.effective_limit(), 7);
    }

    #[test]
    fn search_filters_and_sorts_newest_first() {
        let notices = vec![
            notice("a", direct("Ada"), Person::new("Ada", ""), "Lunch at noon", 100),
            notice("b", direct("Grace"), Person::new("Grace", ""), "lunch?", 300),
            notice("c", direct("Ada"), Person::new("Ada", ""), "see you", 200),
        ];

        let by_text = SearchRequest { text: "LUNCH".into(), ..Default::default() };
        let found = search(&by_text, &notices);
        let ids: Vec<&str> = found.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "a"]);

        let by_sender = SearchRequest { sender: Some(Person::new("ada", "")), ..Default::default() };
        let found = search(&by_sender, &notices);
        let ids: Vec<&str> = found.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["c", "a"]);

        let limited = SearchRequest { limit: 1, ..Default::default() };
        let found = search(&limited, &notices);
        let ids: Vec<&str> = found.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["b"]);

        let in_conv = SearchRequest { conversation_id: Some("c-Grace".into()), ..Default::default() };
        let found = search(&in_conv, &notices);
        let ids: Vec<&str> = found.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["b"]);
    }
}
