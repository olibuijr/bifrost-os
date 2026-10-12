//! Communication notifications: how a message from a person is shown as a direct communication. The sender's name is
//! the title, the conversation's name is the subtitle when the conversation is a group, the conversation identifier is
//! the thread so its messages group together, and the sender's initials stand in for an avatar. The model keeps the
//! conversation and the participants as plain values and works out what the card shows. It does not read contacts,
//! load images or post anything; the shell draws the presentation.

/// A person in a conversation: the name shown on the card and a stable handle that identifies the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub handle: String,
}

/// A conversation: a direct chat with one other person, or a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversation {
    pub id: String,
    pub name: String,
    pub group: bool,
}

/// One incoming message with the conversation it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub conversation: Conversation,
    pub sender: Person,
    pub text: String,
}

/// The parts of the notification the shell draws for a communication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Presentation {
    /// the sender's name
    pub title: String,
    /// the group's name in a group conversation, empty for a direct one
    pub subtitle: String,
    /// the message text
    pub body: String,
    /// `conversation:<id>`; every message of one conversation has the same thread
    pub thread: String,
    /// up to two initials of the sender's name, upper case
    pub avatar: String,
}

/// Why a message cannot be shown as a communication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    EmptyConversation,
    EmptySender,
}

/// The initials of a name: the first character of its first two words, upper case. A name with no words gives "".
pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .flat_map(|c| c.to_uppercase())
        .collect()
}

/// The line the card shows for several messages from one sender: "1 message from X", "3 messages from X". Zero
/// messages give "".
pub fn summary_text(sender: &str, count: u32) -> String {
    match count {
        0 => String::new(),
        1 => format!("1 message from {}", sender),
        n => format!("{} messages from {}", n, sender),
    }
}

impl Message {
    /// Checks that the conversation has an identifier and the sender has a name.
    pub fn validate(&self) -> Result<(), Error> {
        if self.conversation.id.is_empty() {
            return Err(Error::EmptyConversation);
        }
        if self.sender.name.trim().is_empty() {
            return Err(Error::EmptySender);
        }
        Ok(())
    }

    /// What the card shows. The subtitle is the conversation's name only for a group; the avatar is always the sender's.
    pub fn presentation(&self) -> Presentation {
        let subtitle = if self.conversation.group { self.conversation.name.clone() } else { String::new() };
        Presentation {
            title: self.sender.name.clone(),
            subtitle,
            body: self.text.clone(),
            thread: format!("conversation:{}", self.conversation.id),
            avatar: initials(&self.sender.name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str) -> Person {
        Person { name: name.into(), handle: format!("h-{}", name) }
    }

    fn direct() -> Conversation {
        Conversation { id: "c1".into(), name: "Anna".into(), group: false }
    }

    fn group() -> Conversation {
        Conversation { id: "g7".into(), name: "Climbing club".into(), group: true }
    }

    #[test]
    fn direct_message_has_the_sender_as_title_and_no_subtitle() {
        let m = Message { conversation: direct(), sender: person("Anna Berg"), text: "see you at six".into() };
        let p = m.presentation();
        assert_eq!(p.title, "Anna Berg");
        assert_eq!(p.subtitle, "");
        assert_eq!(p.body, "see you at six");
        assert_eq!(p.thread, "conversation:c1");
        assert_eq!(p.avatar, "AB");
    }

    #[test]
    fn group_message_names_the_group_as_subtitle() {
        let m = Message { conversation: group(), sender: person("Jón"), text: "rope?".into() };
        let p = m.presentation();
        assert_eq!(p.title, "Jón");
        assert_eq!(p.subtitle, "Climbing club");
        assert_eq!(p.thread, "conversation:g7");
        assert_eq!(p.avatar, "J");
    }

    #[test]
    fn messages_of_one_conversation_share_a_thread_and_others_do_not() {
        let a = Message { conversation: direct(), sender: person("Anna"), text: "1".into() };
        let b = Message { conversation: direct(), sender: person("Anna"), text: "2".into() };
        let other = Message { conversation: group(), sender: person("Anna"), text: "3".into() };
        assert_eq!(a.presentation().thread, b.presentation().thread);
        assert_ne!(a.presentation().thread, other.presentation().thread);
    }

    #[test]
    fn initials_take_two_words_upper_cased_and_keep_accents() {
        assert_eq!(initials("anna jóhanna berg"), "AJ");
        assert_eq!(initials("x"), "X");
        assert_eq!(initials("ólafur búi"), "ÓB");
        assert_eq!(initials("   "), "");
        assert_eq!(initials(""), "");
    }

    #[test]
    fn summary_text_counts_singular_and_plural() {
        assert_eq!(summary_text("Anna", 0), "");
        assert_eq!(summary_text("Anna", 1), "1 message from Anna");
        assert_eq!(summary_text("Anna", 3), "3 messages from Anna");
    }

    #[test]
    fn validate_refuses_an_empty_conversation_id_or_sender_name() {
        let mut m = Message { conversation: direct(), sender: person("Anna"), text: "hi".into() };
        assert_eq!(m.validate(), Ok(()));
        m.conversation.id.clear();
        assert_eq!(m.validate(), Err(Error::EmptyConversation));
        m.conversation.id = "c1".into();
        m.sender.name = "   ".into();
        assert_eq!(m.validate(), Err(Error::EmptySender));
    }
}
