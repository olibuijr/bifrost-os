//! The app's state: the signed-in user, the rooms with their timelines, the composer and the sign-in form. Pure: the
//! window feeds it sync answers and reads back what to draw and what to send. Nothing here touches a socket or the shell.

use crate::protocol::{self, Body, Event, RoomUpdate, Sync};

/// The most messages a room keeps in memory. Older ones are dropped from the front.
pub const TIMELINE_MAX: usize = 200;
/// The most characters a notification or a list preview shows.
const PREVIEW_CHARS: usize = 120;

/// A message as the window shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub sender: String,
    pub ts: u64,
    pub body: Body,
    /// The message this one answers, when it is a reply.
    pub reply_to: Option<String>,
    /// True once the sender replaced the words.
    pub edited: bool,
    /// The words this message had before each edit, oldest first.
    pub history: Vec<Body>,
    /// The reactions to this message, one entry per emoji in the order they first appeared.
    pub reactions: Vec<Reaction>,
}

/// The people who answered a message with one emoji. Each entry keeps the id of the reaction event, which taking the
/// reaction back removes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reaction {
    pub key: String,
    pub users: Vec<(String, String)>,
}

impl Message {
    /// The id of `user`'s reaction with `key` on this message, when there is one.
    pub fn reaction_of(&self, user: &str, key: &str) -> Option<&str> {
        let reaction = self.reactions.iter().find(|r| r.key == key)?;
        reaction.users.iter().find(|(u, _)| u == user).map(|(_, id)| id.as_str())
    }

    fn add_reaction(&mut self, key: &str, user: &str, id: &str) {
        let index = match self.reactions.iter().position(|r| r.key == key) {
            Some(index) => index,
            None => {
                self.reactions.push(Reaction { key: key.to_string(), users: Vec::new() });
                self.reactions.len() - 1
            }
        };
        let users = &mut self.reactions[index].users;
        if !users.iter().any(|(_, existing)| existing == id) {
            users.push((user.to_string(), id.to_string()));
        }
    }

    /// Drops the reaction event `id`. True when this message had it.
    fn remove_reaction(&mut self, id: &str) -> bool {
        let mut found = false;
        for reaction in &mut self.reactions {
            let before = reaction.users.len();
            reaction.users.retain(|(_, existing)| existing != id);
            found |= reaction.users.len() != before;
        }
        self.reactions.retain(|r| !r.users.is_empty());
        found
    }
}

/// A person in a room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub user_id: String,
    pub name: Option<String>,
    /// Invited and not yet joined.
    pub invited: bool,
}

impl Member {
    /// The name without the invitation note.
    fn label_plain(&self) -> String {
        self.name.clone().unwrap_or_else(|| protocol::display_name(&self.user_id).to_string())
    }

    /// The name to list: the display name, or the local part of the user id.
    pub fn label(&self) -> String {
        let name = self.name.clone().unwrap_or_else(|| protocol::display_name(&self.user_id).to_string());
        if self.invited {
            format!("{name} (invited)")
        } else {
            name
        }
    }
}

/// A joined room.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Room {
    pub id: String,
    /// The name to show: the room's name, its alias, or its id until a sync names it.
    pub name: String,
    pub timeline: Vec<Message>,
    pub unread: u64,
    pub highlights: u64,
    /// The newest event this client has marked read, from a receipt it sent or one another device sent.
    pub read_up_to: Option<String>,
    /// Server time of the newest message, for ordering the list.
    pub last_ts: u64,
    /// The other people typing in the room now, by user id.
    pub typing: Vec<String>,
    /// Who is in the room (joined or invited), as the syncs reported it.
    pub members: Vec<Member>,
}

impl Room {
    fn named(id: &str) -> Room {
        Room { id: id.to_string(), name: id.to_string(), ..Room::default() }
    }

    /// The name the list and the header show. A room nobody named is called after the other people in it (a direct
    /// conversation shows the person); until the members are known it keeps its id.
    pub fn title(&self, me: &str) -> String {
        if self.name != self.id {
            return self.name.clone();
        }
        let others: Vec<String> = self.members.iter().filter(|m| m.user_id != me).map(Member::label_plain).collect();
        if others.is_empty() {
            self.name.clone()
        } else {
            others.join(", ")
        }
    }

    /// The sender and one-line preview of the message `id`, for the quoted line above a reply.
    pub fn quote(&self, id: &str) -> Option<(String, String)> {
        let m = self.timeline.iter().find(|m| m.id == id)?;
        Some((protocol::display_name(&m.sender).to_string(), preview(&m.body)))
    }

    /// True when `query` (already lower case) appears in the room's title or in a loaded message.
    pub fn matches(&self, me: &str, query: &str) -> bool {
        if query.is_empty() || self.title(me).to_lowercase().contains(query) {
            return true;
        }
        self.timeline.iter().any(|m| match &m.body {
            Body::Text(t) | Body::Notice(t) | Body::Emote(t) => t.to_lowercase().contains(query),
            Body::Image { name, .. } => name.to_lowercase().contains(query),
            _ => false,
        })
    }

    /// The file names of the pictures in the loaded timeline, newest first. Removed messages are left out.
    pub fn shared_pictures(&self) -> Vec<String> {
        self.timeline
            .iter()
            .rev()
            .filter_map(|m| match &m.body {
                Body::Image { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    /// The one-line preview of the newest message, for the room list.
    pub fn preview(&self) -> String {
        match self.timeline.last() {
            Some(m) => preview(&m.body),
            None => String::new(),
        }
    }
}

/// A notification to show: the room it is in and the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// The sign-in sheet: homeserver, user name and password, and the one that has the keyboard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignIn {
    pub values: [String; 3],
    pub focus: usize,
    pub error: String,
    pub busy: bool,
}

/// The field labels of the sign-in sheet, in order.
pub const FIELD_LABELS: [&str; 3] = ["Homeserver", "User name", "Password"];
/// The index of the password field, which is shown as dots.
pub const PASSWORD_FIELD: usize = 2;

impl SignIn {
    /// A sheet with the homeserver and user filled in, and the password empty (after a failed sign-in).
    pub fn prefilled(homeserver: &str, user: &str, error: &str) -> SignIn {
        SignIn {
            values: [homeserver.to_string(), user.to_string(), String::new()],
            focus: PASSWORD_FIELD,
            error: error.to_string(),
            busy: false,
        }
    }

    pub fn type_text(&mut self, text: &str) {
        self.values[self.focus].push_str(text);
        self.error.clear();
    }

    pub fn backspace(&mut self) {
        self.values[self.focus].pop();
    }

    pub fn next_field(&mut self) {
        self.focus = (self.focus + 1) % self.values.len();
    }

    /// The checked homeserver base address, the user and the password. An error names the first field that is wrong.
    pub fn checked(&self) -> Result<(String, String, String), String> {
        let homeserver = protocol::homeserver_base(&self.values[0])?;
        let user = self.values[1].trim().to_string();
        if user.is_empty() {
            return Err("Enter the user name".into());
        }
        let password = self.values[2].clone();
        if password.is_empty() {
            return Err("Enter the password".into());
        }
        Ok((homeserver, user, password))
    }
}

/// The reactions the context menu offers: the menu label and the emoji sent.
pub const REACTIONS: [(&str, &str); 6] = [
    ("Love", "\u{2764}\u{FE0F}"),
    ("Like", "\u{1F44D}"),
    ("Dislike", "\u{1F44E}"),
    ("Haha", "\u{1F602}"),
    ("Emphasize", "\u{203C}\u{FE0F}"),
    ("Question", "\u{2753}"),
];

/// The emoji the picker offers with the words that find them, in rows of [`EMOJI_COLUMNS`].
pub const EMOJI_CATALOG: [(&str, &str); 64] = [
    ("\u{1F600}", "grinning smile happy"), ("\u{1F603}", "smile happy joy"),
    ("\u{1F604}", "laugh happy"), ("\u{1F601}", "beam grin"),
    ("\u{1F606}", "laugh squint"), ("\u{1F605}", "sweat relief"),
    ("\u{1F602}", "joy laugh tears lol"), ("\u{1F642}", "slight smile"),
    ("\u{1F609}", "wink"), ("\u{1F60A}", "blush smile"),
    ("\u{1F60D}", "love heart eyes"), ("\u{1F618}", "kiss"),
    ("\u{1F61C}", "tongue wink silly"), ("\u{1F914}", "think thinking hmm"),
    ("\u{1F610}", "neutral meh"), ("\u{1F62E}", "surprised wow"),
    ("\u{1F622}", "cry sad tear"), ("\u{1F62D}", "sob cry"),
    ("\u{1F620}", "angry mad"), ("\u{1F631}", "scream shock fear"),
    ("\u{1F634}", "sleep tired zzz"), ("\u{1F60E}", "cool sunglasses"),
    ("\u{1F973}", "party celebrate"), ("\u{1F64F}", "pray thanks please"),
    ("\u{1F44D}", "thumbs up like yes ok"), ("\u{1F44E}", "thumbs down dislike no"),
    ("\u{1F44F}", "clap applause"), ("\u{1F64C}", "raised hands hooray"),
    ("\u{1F44B}", "wave hello bye"), ("\u{1F4AA}", "strong muscle flex"),
    ("\u{2764}\u{FE0F}", "heart love red"), ("\u{1F494}", "broken heart"),
    ("\u{1F525}", "fire hot"), ("\u{2728}", "sparkles shine"),
    ("\u{1F389}", "tada party celebrate"), ("\u{1F381}", "gift present"),
    ("\u{2705}", "check done yes"), ("\u{274C}", "cross no wrong"),
    ("\u{2753}", "question"), ("\u{203C}\u{FE0F}", "exclamation emphasis"),
    ("\u{1F607}", "angel halo"), ("\u{1F923}", "rofl laugh floor"),
    ("\u{1F60B}", "yum tasty"), ("\u{1F917}", "hug"),
    ("\u{1F644}", "eye roll"), ("\u{1F62C}", "grimace awkward"),
    ("\u{1F633}", "flushed embarrassed"), ("\u{1F97A}", "pleading puppy"),
    ("\u{1F621}", "rage furious"), ("\u{1F92F}", "mind blown"),
    ("\u{1F91D}", "handshake deal"), ("\u{1F440}", "eyes look"),
    ("\u{1F648}", "see no evil monkey"), ("\u{1F4AF}", "hundred perfect"),
    ("\u{2615}", "coffee tea"), ("\u{1F355}", "pizza food"),
    ("\u{1F382}", "cake birthday"), ("\u{1F31E}", "sun sunny"),
    ("\u{1F308}", "rainbow"), ("\u{2B50}", "star"),
    ("\u{1F436}", "dog puppy"), ("\u{1F431}", "cat kitten"),
    ("\u{1F680}", "rocket launch ship"), ("\u{1F4A1}", "idea light bulb"),
];
/// How many emoji a row of the picker holds.
pub const EMOJI_COLUMNS: usize = 8;

/// The most recently used emoji the picker puts first.
pub const RECENT_EMOJI_MAX: usize = 8;

/// The catalogue emoji whose words contain every word of `query` (any case). An empty query finds all of them, in
/// catalogue order.
pub fn emoji_matches(query: &str) -> Vec<&'static str> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    EMOJI_CATALOG
        .iter()
        .filter(|(emoji, keywords)| words.iter().all(|w| keywords.contains(w.as_str()) || emoji.contains(w.as_str())))
        .map(|(emoji, _)| *emoji)
        .collect()
}

/// The per-conversation choices the user makes: pinned, alerts hidden, and marked as unread. Kept as room ids and saved
/// in the app's preferences as one id per line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomPrefs {
    pub pinned: Vec<String>,
    pub muted: Vec<String>,
    pub unread_marks: Vec<String>,
    /// The emoji picked most recently, newest first.
    pub recent_emoji: Vec<String>,
}

impl RoomPrefs {
    /// Reads a saved list: one room id per line, blank lines ignored.
    pub fn decode(text: &str) -> Vec<String> {
        text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
    }

    /// The text a list is saved as.
    pub fn encode(list: &[String]) -> String {
        list.join("\n")
    }
}

/// Toggles `id` in `list`. True when it is in the list afterwards.
fn toggle(list: &mut Vec<String>, id: &str) -> bool {
    match list.iter().position(|x| x == id) {
        Some(i) => {
            list.remove(i);
            false
        }
        None => {
            list.push(id.to_string());
            true
        }
    }
}

/// The most conversations that can be pinned at once.
pub const PIN_LIMIT: usize = 9;

/// What the composer's text becomes when it is sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Compose {
    /// Answer the message with this id.
    Reply(String),
    /// Replace the words of the message with this id.
    Edit(String),
}

/// What to send for the composer: a plain text, a reply or an edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outgoing {
    Text(String),
    Reply { text: String, to: String },
    Edit { text: String, target: String },
}

/// What a one-line sheet asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptKind {
    /// The user ids of the people to message: one starts a direct conversation, several a group.
    NewMessage,
    /// A new name for the room with this id.
    Rename(String),
}

/// A sheet with one text field: New Message, or the room name in Conversation Details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub kind: PromptKind,
    pub value: String,
    pub error: String,
    pub busy: bool,
}

impl Prompt {
    pub fn new_message() -> Prompt {
        Prompt { kind: PromptKind::NewMessage, value: "@".into(), error: String::new(), busy: false }
    }

    pub fn rename(room: &str, current: &str) -> Prompt {
        Prompt { kind: PromptKind::Rename(room.to_string()), value: current.to_string(), error: String::new(), busy: false }
    }

    /// The people a checked New Message value names.
    pub fn users(checked: &str) -> Vec<String> {
        checked.split(',').filter(|u| !u.is_empty()).map(str::to_string).collect()
    }

    pub fn title(&self) -> &'static str {
        match self.kind {
            PromptKind::NewMessage => "New Message",
            PromptKind::Rename(_) => "Rename Conversation",
        }
    }

    pub fn label(&self) -> &'static str {
        match self.kind {
            PromptKind::NewMessage => "To (Matrix user ids, separated by commas)",
            PromptKind::Rename(_) => "Name",
        }
    }

    pub fn ok_label(&self) -> &'static str {
        match self.kind {
            PromptKind::NewMessage => "Start",
            PromptKind::Rename(_) => "Rename",
        }
    }

    /// The checked value: the user ids joined by commas for New Message (see [`Prompt::users`]), a non-empty name for
    /// Rename.
    pub fn checked(&self) -> Result<String, String> {
        match self.kind {
            PromptKind::NewMessage => protocol::checked_user_ids(&self.value).map(|ids| ids.join(",")),
            PromptKind::Rename(_) => {
                let name = self.value.trim();
                if name.is_empty() {
                    Err("Enter a name".into())
                } else {
                    Ok(name.to_string())
                }
            }
        }
    }
}

/// What a message-info panel lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoKind {
    /// The words of the message before each edit.
    Edits,
    /// Who reacted with which emoji.
    Reactions,
}

impl InfoKind {
    pub fn title(self) -> &'static str {
        match self {
            InfoKind::Edits => "Edit History",
            InfoKind::Reactions => "Reactions",
        }
    }
}

/// The most lines a message-info panel lists; older entries are left out.
pub const INFO_ROWS_MAX: usize = 10;

/// A panel over the window. The sign-in sheet is separate because it blocks everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Overlay {
    #[default]
    None,
    Prompt(Prompt),
    /// Conversation Details: the name and the members of the room on screen.
    Details,
    /// Settings: the account and Sign Out.
    Settings,
    /// Asks before leaving the conversation with this room id.
    Confirm(String),
    /// The emoji picker. With a message id, a choice reacts to that message; without, it goes into the composer.
    Emoji(Option<String>),
    /// The edits or the reactions of the message with this id, in the room on screen.
    Info(InfoKind, String),
}

/// The app's state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Messages {
    /// The signed-in user's id. Empty until a sign-in succeeds.
    pub user_id: String,
    /// The homeserver base address of the signed-in account, for Settings.
    pub homeserver: String,
    pub rooms: Vec<Room>,
    /// The room on screen.
    pub selected: Option<String>,
    /// The text being typed in the composer.
    pub composer: String,
    /// The line under the header: what the app last reported.
    pub status: String,
    pub sign_in: Option<SignIn>,
    /// True once the first sync has been applied. Only messages after that are notified.
    pub primed: bool,
    pub prefs: RoomPrefs,
    /// The search field's text, and whether it has the keyboard (otherwise the composer does).
    pub search: String,
    pub search_focus: bool,
    /// Set while the composer holds a reply or an edit.
    pub compose: Option<Compose>,
    pub overlay: Overlay,
    /// What is typed in the emoji picker's search field.
    pub emoji_query: String,
}

impl Messages {
    pub fn signed_in(&self) -> bool {
        !self.user_id.is_empty()
    }

    pub fn selected_room(&self) -> Option<&Room> {
        let id = self.selected.as_deref()?;
        self.rooms.iter().find(|r| r.id == id)
    }

    /// The unread notifications of every room, for the dock badge. A conversation marked as unread counts as one when
    /// the server counts none; one with alerts hidden is not counted.
    pub fn total_unread(&self) -> u64 {
        self.rooms
            .iter()
            .filter(|r| !self.is_muted(&r.id))
            .map(|r| if r.unread == 0 && self.is_marked_unread(&r.id) { 1 } else { r.unread })
            .sum()
    }

    pub fn is_pinned(&self, id: &str) -> bool {
        self.prefs.pinned.iter().any(|x| x == id)
    }

    pub fn is_muted(&self, id: &str) -> bool {
        self.prefs.muted.iter().any(|x| x == id)
    }

    pub fn is_marked_unread(&self, id: &str) -> bool {
        self.prefs.unread_marks.iter().any(|x| x == id)
    }

    /// Pins or unpins a conversation. True when it is pinned afterwards. A new pin goes after the others; past
    /// [`PIN_LIMIT`] it is refused with the reason.
    pub fn toggle_pin(&mut self, id: &str) -> Result<bool, String> {
        if !self.is_pinned(id) && self.prefs.pinned.len() >= PIN_LIMIT {
            return Err(format!("You can pin up to {PIN_LIMIT} conversations"));
        }
        Ok(toggle(&mut self.prefs.pinned, id))
    }

    /// True when the pinned conversation `id` has a neighbour to swap places with, above (`up`) or below it.
    pub fn can_move_pin(&self, id: &str, up: bool) -> bool {
        match self.prefs.pinned.iter().position(|x| x == id) {
            Some(i) if up => i > 0,
            Some(i) => i + 1 < self.prefs.pinned.len(),
            None => false,
        }
    }

    /// Moves the pinned conversation `id` one place up or down among the pins. False when it cannot move.
    pub fn move_pin(&mut self, id: &str, up: bool) -> bool {
        if !self.can_move_pin(id, up) {
            return false;
        }
        let Some(i) = self.prefs.pinned.iter().position(|x| x == id) else { return false };
        let j = if up { i - 1 } else { i + 1 };
        self.prefs.pinned.swap(i, j);
        true
    }

    /// Hides or shows the alerts of a conversation. True when they are hidden afterwards.
    pub fn toggle_mute(&mut self, id: &str) -> bool {
        toggle(&mut self.prefs.muted, id)
    }

    /// Marks a conversation as unread, or takes the mark off. Marking the one on screen closes it, so the mark is not
    /// cleared again at once.
    pub fn toggle_unread_mark(&mut self, id: &str) -> bool {
        let marked = toggle(&mut self.prefs.unread_marks, id);
        if marked && self.selected.as_deref() == Some(id) {
            self.selected = None;
            self.compose = None;
        }
        marked
    }

    /// Applies the unread flag the server holds for a conversation. A mark never lands on the conversation on screen:
    /// it is being read. True when the local mark changed.
    pub fn set_unread_mark(&mut self, id: &str, marked: bool) -> bool {
        let present = self.is_marked_unread(id);
        if marked == present || (marked && self.selected.as_deref() == Some(id)) {
            return false;
        }
        toggle(&mut self.prefs.unread_marks, id);
        true
    }

    /// The conversations the list shows: those matching the search, the pinned ones first in the order they were pinned
    /// (and moved to), the rest in their order.
    pub fn visible_rooms(&self) -> Vec<&Room> {
        let query = self.search.trim().to_lowercase();
        let mut rooms: Vec<&Room> = self.rooms.iter().filter(|r| r.matches(&self.user_id, &query)).collect();
        rooms.sort_by_key(|r| self.prefs.pinned.iter().position(|x| *x == r.id).unwrap_or(usize::MAX));
        rooms
    }

    /// The emoji the open picker shows: with no search, the recent ones first and then the rest of the catalogue; with
    /// a search, the emoji it finds.
    pub fn emoji_choices(&self) -> Vec<String> {
        let found = emoji_matches(&self.emoji_query);
        if !self.emoji_query.trim().is_empty() {
            return found.into_iter().map(str::to_string).collect();
        }
        let recent: Vec<&str> =
            self.prefs.recent_emoji.iter().map(String::as_str).filter(|e| found.iter().any(|f| *f == *e)).collect();
        recent.iter().copied().chain(found.iter().copied().filter(|e| !recent.iter().any(|r| r == e))).map(str::to_string).collect()
    }

    /// The lines of a message-info panel for the message `id` of the room on screen: its versions, oldest first (the
    /// current one marked), or each emoji with the people who gave it. At most [`INFO_ROWS_MAX`] lines.
    pub fn info_lines(&self, kind: InfoKind, id: &str) -> Vec<String> {
        let Some(message) = self.selected_room().and_then(|r| r.timeline.iter().find(|m| m.id == id)) else { return Vec::new() };
        let mut lines: Vec<String> = match kind {
            InfoKind::Edits => {
                let total = message.history.len() + 1;
                message
                    .history
                    .iter()
                    .map(preview)
                    .chain(std::iter::once(preview(&message.body)))
                    .enumerate()
                    .map(|(i, text)| if i + 1 == total { format!("{}. {text} (current)", i + 1) } else { format!("{}. {text}", i + 1) })
                    .collect()
            }
            InfoKind::Reactions => message
                .reactions
                .iter()
                .map(|r| {
                    let who: Vec<String> = r
                        .users
                        .iter()
                        .map(|(user, _)| if *user == self.user_id { "You".to_string() } else { protocol::display_name(user).to_string() })
                        .collect();
                    format!("{}  {}", r.key, who.join(", "))
                })
                .collect(),
        };
        if lines.len() > INFO_ROWS_MAX {
            lines.drain(..lines.len() - INFO_ROWS_MAX);
        }
        lines
    }

    /// Opens the picker with an empty search, for a message (a reaction) or for the composer.
    pub fn open_emoji(&mut self, message: Option<String>) {
        self.emoji_query.clear();
        self.overlay = Overlay::Emoji(message);
    }

    /// Notes that `emoji` was picked: it moves to the front of the recent ones, which keep [`RECENT_EMOJI_MAX`].
    pub fn note_emoji_used(&mut self, emoji: &str) {
        self.prefs.recent_emoji.retain(|e| e != emoji);
        self.prefs.recent_emoji.insert(0, emoji.to_string());
        self.prefs.recent_emoji.truncate(RECENT_EMOJI_MAX);
    }

    /// Forgets a conversation that was left or deleted.
    pub fn remove_room(&mut self, id: &str) {
        self.rooms.retain(|r| r.id != id);
        self.prefs.pinned.retain(|x| x != id);
        self.prefs.muted.retain(|x| x != id);
        self.prefs.unread_marks.retain(|x| x != id);
        if self.selected.as_deref() == Some(id) {
            self.selected = None;
            self.compose = None;
            self.overlay = Overlay::None;
        }
    }

    /// Puts a conversation that was just made on screen, adding it to the list until a sync describes it.
    pub fn show_new_room(&mut self, id: &str) {
        self.room_index(id);
        self.selected = Some(id.to_string());
        self.compose = None;
    }

    /// Signs out locally: forgets the account, every conversation and everything typed.
    pub fn sign_out(&mut self) {
        let prefs = std::mem::take(&mut self.prefs);
        *self = Messages { prefs, ..Messages::default() };
    }

    /// Starts a reply to the message `id` of the room on screen. False when there is no such message.
    pub fn begin_reply(&mut self, id: &str) -> bool {
        if !self.selected_room().is_some_and(|r| r.timeline.iter().any(|m| m.id == id)) {
            return false;
        }
        self.compose = Some(Compose::Reply(id.to_string()));
        true
    }

    /// Starts editing the user's own text message `id`: its words go into the composer. False for anyone else's message
    /// or one that is not text.
    pub fn begin_edit(&mut self, id: &str) -> bool {
        let me = self.user_id.clone();
        let Some(room) = self.selected_room() else { return false };
        let Some(message) = room.timeline.iter().find(|m| m.id == id) else { return false };
        let Body::Text(words) = &message.body else { return false };
        if message.sender != me {
            return false;
        }
        let words = words.clone();
        self.composer = words;
        self.compose = Some(Compose::Edit(id.to_string()));
        true
    }

    /// Drops a reply or an edit in progress; an edit also empties the composer.
    pub fn cancel_compose(&mut self) {
        if matches!(self.compose, Some(Compose::Edit(_))) {
            self.composer.clear();
        }
        self.compose = None;
    }

    /// The line above the composer while it holds a reply or an edit.
    pub fn banner(&self) -> Option<String> {
        match self.compose.as_ref()? {
            Compose::Reply(id) => {
                let (who, what) = self.selected_room()?.quote(id)?;
                Some(format!("Replying to {who}: {what}"))
            }
            Compose::Edit(_) => Some("Editing message".into()),
        }
    }

    /// What the composer sends, with the composer and the reply or edit state cleared. `None` when there is nothing to
    /// send, or an edit changes nothing.
    pub fn take_outgoing(&mut self) -> Option<Outgoing> {
        let compose = self.compose.take();
        let text = self.take_composer()?;
        match compose {
            None => Some(Outgoing::Text(text)),
            Some(Compose::Reply(to)) => Some(Outgoing::Reply { text, to }),
            Some(Compose::Edit(target)) => {
                let same = self
                    .selected_room()
                    .and_then(|r| r.timeline.iter().find(|m| m.id == target))
                    .is_some_and(|m| m.body == Body::Text(text.clone()));
                if same {
                    None
                } else {
                    Some(Outgoing::Edit { text, target })
                }
            }
        }
    }

    /// Applies one sync answer and returns the notifications for messages that just arrived from other people.
    /// Rooms are kept in order of their newest message, newest first.
    pub fn apply_sync(&mut self, sync: &Sync) -> Vec<Notice> {
        let mut notices = Vec::new();
        for update in &sync.rooms {
            let index = self.room_index(&update.id);
            let viewing = self.selected.as_deref() == Some(update.id.as_str());
            let alerts = self.primed && !self.prefs.muted.iter().any(|x| *x == update.id);
            apply_room(&mut self.rooms[index], update, &self.user_id, alerts, viewing, &mut notices);
            if let Some(flag) = update.marked_unread {
                self.set_unread_mark(&update.id, flag);
            }
        }
        self.primed = true;
        self.rooms.sort_by(|a, b| b.last_ts.cmp(&a.last_ts).then_with(|| a.name.cmp(&b.name)));
        notices
    }

    /// Puts `id` on screen. The room's unread counts clear here; the returned value is the read marker to send, see
    /// [`Messages::take_read_marker`].
    pub fn select(&mut self, id: &str) -> Option<(String, String)> {
        if self.selected.as_deref() != Some(id) {
            self.compose = None;
        }
        self.selected = Some(id.to_string());
        self.prefs.unread_marks.retain(|x| x != id);
        self.take_read_marker()
    }

    /// The read marker of the room on screen: its newest event, when that is not already marked read. Clears the room's
    /// unread counts and records the marker, so each event is marked once. `None` when nothing needs sending.
    pub fn take_read_marker(&mut self) -> Option<(String, String)> {
        let id = self.selected.clone()?;
        let room = self.rooms.iter_mut().find(|r| r.id == id)?;
        room.unread = 0;
        room.highlights = 0;
        let newest = room.timeline.last()?.id.clone();
        if room.read_up_to.as_deref() == Some(newest.as_str()) {
            return None;
        }
        room.read_up_to = Some(newest.clone());
        Some((id, newest))
    }

    /// The text to send: the composer, trimmed, when it has any. The composer is emptied.
    pub fn take_composer(&mut self) -> Option<String> {
        let text = self.composer.trim().to_string();
        self.composer.clear();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    fn room_index(&mut self, id: &str) -> usize {
        match self.rooms.iter().position(|r| r.id == id) {
            Some(index) => index,
            None => {
                self.rooms.push(Room::named(id));
                self.rooms.len() - 1
            }
        }
    }
}

/// Folds one room's part of a sync into the room. A message is added once, by its event id.
fn apply_room(room: &mut Room, update: &RoomUpdate, me: &str, primed: bool, viewing: bool, notices: &mut Vec<Notice>) {
    if let Some(name) = &update.name {
        room.name = name.clone();
    }
    room.unread = update.unread;
    room.highlights = update.highlights;
    if let Some(typing) = &update.typing {
        room.typing = typing.iter().filter(|user| user.as_str() != me).cloned().collect();
    }
    for event in &update.events {
        if room.timeline.iter().any(|m| m.id == event.id) {
            continue;
        }
        if primed && !viewing && event.sender != me {
            notices.push(notice(&room.name, event));
        }
        room.last_ts = room.last_ts.max(event.ts);
        room.timeline.push(Message {
            id: event.id.clone(),
            sender: event.sender.clone(),
            ts: event.ts,
            body: event.body.clone(),
            reply_to: event.reply_to.clone(),
            edited: false,
            history: Vec::new(),
            reactions: Vec::new(),
        });
    }
    apply_changes(room, update);
    if room.timeline.len() > TIMELINE_MAX {
        let extra = room.timeline.len() - TIMELINE_MAX;
        room.timeline.drain(..extra);
    }
    for receipt in &update.receipts {
        if receipt.user_id == me {
            room.read_up_to = Some(receipt.event_id.clone());
        }
    }
}

/// Folds the edits, reactions, removals and membership changes of a room update into the room.
fn apply_changes(room: &mut Room, update: &RoomUpdate) {
    for edit in &update.edits {
        if let Some(m) = room.timeline.iter_mut().find(|m| m.id == edit.target && m.sender == edit.sender) {
            if m.body != Body::Redacted {
                if m.body != edit.body {
                    let before = std::mem::replace(&mut m.body, edit.body.clone());
                    m.history.push(before);
                }
                m.edited = true;
            }
        }
    }
    for reaction in &update.reactions {
        if let Some(m) = room.timeline.iter_mut().find(|m| m.id == reaction.target) {
            m.add_reaction(&reaction.key, &reaction.sender, &reaction.id);
        }
    }
    for removed in &update.redactions {
        if let Some(m) = room.timeline.iter_mut().find(|m| m.id == *removed) {
            m.body = Body::Redacted;
            m.reactions.clear();
            m.history.clear();
            m.edited = false;
        } else {
            for m in &mut room.timeline {
                if m.remove_reaction(removed) {
                    break;
                }
            }
        }
    }
    for change in &update.members {
        room.members.retain(|m| m.user_id != change.user_id);
        if change.membership == "join" || change.membership == "invite" {
            room.members.push(Member {
                user_id: change.user_id.clone(),
                name: change.name.clone(),
                invited: change.membership == "invite",
            });
        }
    }
}

fn notice(room: &str, event: &Event) -> Notice {
    Notice { title: room.to_string(), body: format!("{}: {}", protocol::display_name(&event.sender), preview(&event.body)) }
}

/// Milliseconds in a day: a message's day is its server time over this.
const DAY_MS: u64 = 86_400_000;

/// The day a server time (milliseconds since the epoch) falls on, in days since the epoch. UTC: the days are the
/// desktop's zone's when the zone is UTC, which is what the lab machines use.
pub fn day_of(ts: u64) -> i64 {
    (ts / DAY_MS) as i64
}

/// True when a message at `ts` starts a new day in the timeline: the first message, or one on another day than the one
/// before it.
pub fn starts_day(prev: Option<u64>, ts: u64) -> bool {
    prev.map_or(true, |p| day_of(p) != day_of(ts))
}

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// The heading of a day in the timeline: "Today", "Yesterday", or the date, given today's day.
pub fn day_label(day: i64, today: i64) -> String {
    match today - day {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => {
            let (y, m, d) = civil_from_days(day);
            format!("{d} {} {y}", MONTHS[(m - 1) as usize])
        }
    }
}

/// Civil (year, month 1-12, day) of a day count since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// The typing line for the people typing in a room, named by their user ids. `None` when nobody is typing.
pub fn typing_text(users: &[String]) -> Option<String> {
    let names: Vec<&str> = users.iter().map(|u| protocol::display_name(u)).collect();
    match names.as_slice() {
        [] => None,
        [one] => Some(format!("{one} is typing...")),
        [a, b] => Some(format!("{a} and {b} are typing...")),
        _ => Some("Several people are typing...".into()),
    }
}

/// The text a message shows as a preview: its words, or a short description for a picture or a file.
pub fn preview(body: &Body) -> String {
    match body {
        Body::Text(s) | Body::Notice(s) | Body::Emote(s) => clip(s),
        Body::Image { name, .. } => format!("Sent an image: {name}"),
        Body::Encrypted => "Sent an encrypted message".into(),
        Body::Other(kind) => format!("Sent a {}", kind.trim_start_matches("m.")),
        Body::Redacted => "Message removed".into(),
    }
}

/// `text` shortened to the preview length, on one line, with an ellipsis when it was cut.
fn clip(text: &str) -> String {
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= PREVIEW_CHARS {
        return one_line;
    }
    let head: String = one_line.chars().take(PREVIEW_CHARS).collect();
    format!("{head}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: &str, sender: &str, ts: u64, body: Body) -> Event {
        Event { id: id.into(), sender: sender.into(), ts, body, reply_to: None }
    }

    fn update(id: &str, events: Vec<Event>, unread: u64) -> RoomUpdate {
        RoomUpdate { id: id.into(), name: None, events, unread, ..RoomUpdate::default() }
    }

    fn sync(rooms: Vec<RoomUpdate>) -> Sync {
        Sync { next_batch: "n".into(), rooms }
    }

    fn signed_in() -> Messages {
        Messages { user_id: "@me:x".into(), ..Messages::default() }
    }

    #[test]
    fn the_first_sync_adds_history_without_notifications() {
        let mut m = signed_in();
        let history = sync(vec![update("!a:x", vec![ev("$1", "@bob:x", 10, Body::Text("old".into()))], 0)]);
        assert!(m.apply_sync(&history).is_empty());
        assert_eq!(m.rooms[0].timeline.len(), 1);
        assert!(m.primed);
    }

    #[test]
    fn later_messages_from_others_notify_and_my_own_do_not() {
        let mut m = signed_in();
        m.apply_sync(&sync(vec![update("!a:x", vec![], 0)]));
        let next = sync(vec![update(
            "!a:x",
            vec![ev("$2", "@bob:x", 20, Body::Text("hi there".into())), ev("$3", "@me:x", 21, Body::Text("mine".into()))],
            1,
        )]);
        let notices = m.apply_sync(&next);
        assert_eq!(notices, vec![Notice { title: "!a:x".into(), body: "bob: hi there".into() }]);
        assert_eq!(m.rooms[0].unread, 1);
    }

    #[test]
    fn a_message_is_added_once_even_when_a_sync_repeats_it() {
        let mut m = signed_in();
        m.apply_sync(&sync(vec![update("!a:x", vec![], 0)]));
        let same = sync(vec![update("!a:x", vec![ev("$2", "@bob:x", 20, Body::Text("a".into()))], 0)]);
        m.apply_sync(&same);
        assert!(m.apply_sync(&same).is_empty());
        assert_eq!(m.rooms[0].timeline.len(), 1);
    }

    #[test]
    fn a_room_being_viewed_does_not_notify() {
        let mut m = signed_in();
        m.apply_sync(&sync(vec![update("!a:x", vec![], 0)]));
        m.select("!a:x");
        let next = sync(vec![update("!a:x", vec![ev("$2", "@bob:x", 20, Body::Text("a".into()))], 0)]);
        assert!(m.apply_sync(&next).is_empty());
    }

    #[test]
    fn rooms_are_ordered_by_their_newest_message() {
        let mut m = signed_in();
        m.apply_sync(&sync(vec![
            update("!old:x", vec![ev("$1", "@a:x", 5, Body::Text("a".into()))], 0),
            update("!new:x", vec![ev("$2", "@a:x", 50, Body::Text("b".into()))], 0),
        ]));
        assert_eq!(m.rooms[0].id, "!new:x");
        assert_eq!(m.rooms[1].id, "!old:x");
    }

    #[test]
    fn a_name_from_a_sync_replaces_the_id() {
        let mut m = signed_in();
        let mut named = update("!a:x", vec![], 0);
        named.name = Some("Family".into());
        m.apply_sync(&sync(vec![named]));
        assert_eq!(m.rooms[0].name, "Family");
    }

    #[test]
    fn the_read_marker_is_sent_once_per_newest_event() {
        let mut m = signed_in();
        m.apply_sync(&sync(vec![update("!a:x", vec![ev("$1", "@bob:x", 1, Body::Text("a".into()))], 2)]));
        assert_eq!(m.select("!a:x"), Some(("!a:x".into(), "$1".into())));
        assert_eq!(m.rooms[0].unread, 0, "opening the room clears its count");
        assert_eq!(m.take_read_marker(), None, "already marked");
        m.apply_sync(&sync(vec![update("!a:x", vec![ev("$2", "@bob:x", 2, Body::Text("b".into()))], 1)]));
        assert_eq!(m.take_read_marker(), Some(("!a:x".into(), "$2".into())));
    }

    #[test]
    fn a_receipt_from_this_user_records_the_read_position() {
        let mut m = signed_in();
        let mut r = update("!a:x", vec![ev("$1", "@bob:x", 1, Body::Text("a".into()))], 0);
        r.receipts.push(protocol::Receipt { event_id: "$1".into(), user_id: "@me:x".into(), ts: 2 });
        m.apply_sync(&sync(vec![r]));
        m.select("!a:x");
        assert_eq!(m.rooms[0].read_up_to.as_deref(), Some("$1"));
        assert_eq!(m.take_read_marker(), None);
    }

    #[test]
    fn the_timeline_keeps_the_newest_messages() {
        let mut m = signed_in();
        let events: Vec<Event> = (0..TIMELINE_MAX + 5).map(|i| ev(&format!("${i}"), "@b:x", i as u64, Body::Text("t".into()))).collect();
        m.apply_sync(&sync(vec![update("!a:x", events, 0)]));
        assert_eq!(m.rooms[0].timeline.len(), TIMELINE_MAX);
        assert_eq!(m.rooms[0].timeline[0].id, "$5");
    }

    #[test]
    fn previews_describe_every_kind_of_message() {
        assert_eq!(preview(&Body::Image { name: "cat.png".into(), mxc: "mxc://x/y".into() }), "Sent an image: cat.png");
        assert_eq!(preview(&Body::Encrypted), "Sent an encrypted message");
        assert_eq!(preview(&Body::Other("m.file".into())), "Sent a file");
        assert_eq!(preview(&Body::Redacted), "Message removed");
        assert_eq!(preview(&Body::Text("a\n  b".into())), "a b", "one line");
        let long = "x".repeat(PREVIEW_CHARS + 10);
        assert!(preview(&Body::Text(long)).ends_with("..."));
    }

    #[test]
    fn the_composer_sends_trimmed_text_and_clears() {
        let mut m = signed_in();
        m.composer = "  hello  ".into();
        assert_eq!(m.take_composer().as_deref(), Some("hello"));
        assert!(m.composer.is_empty());
        m.composer = "   ".into();
        assert_eq!(m.take_composer(), None);
    }

    #[test]
    fn the_sign_in_sheet_checks_each_field() {
        let mut s = SignIn::default();
        s.values = ["matrix.org".into(), " alice ".into(), "secret".into()];
        assert_eq!(s.checked(), Ok(("https://matrix.org".into(), "alice".into(), "secret".into())));
        s.values[1] = "  ".into();
        assert_eq!(s.checked(), Err("Enter the user name".into()));
        s.values[1] = "alice".into();
        s.values[2].clear();
        assert_eq!(s.checked(), Err("Enter the password".into()));
        s.values[0] = "http://plain".into();
        assert!(s.checked().unwrap_err().contains("https"));
    }

    #[test]
    fn typing_goes_to_the_focused_field_and_tab_moves_on() {
        let mut s = SignIn::default();
        s.type_text("abc");
        s.backspace();
        s.next_field();
        s.type_text("bob");
        assert_eq!(s.values, ["ab".to_string(), "bob".to_string(), String::new()]);
        s.next_field();
        s.next_field();
        assert_eq!(s.focus, 0, "tab wraps to the first field");
    }

    #[test]
    fn a_prefilled_sheet_starts_on_the_password() {
        let s = SignIn::prefilled("https://m", "@a:m", "wrong");
        assert_eq!(s.focus, PASSWORD_FIELD);
        assert_eq!(s.values[2], "");
        assert_eq!(s.error, "wrong");
    }

    #[test]
    fn a_new_day_starts_at_the_first_message_and_at_each_midnight() {
        let day = DAY_MS;
        assert!(starts_day(None, 5));
        assert!(!starts_day(Some(day + 1), day + 2), "the same day");
        assert!(starts_day(Some(day - 1), day), "across midnight");
        assert_eq!(day_of(2 * DAY_MS + 7), 2);
    }

    #[test]
    fn day_headings_are_today_yesterday_or_the_date() {
        let today = day_of(1_800_000_000_000);
        assert_eq!(day_label(today, today), "Today");
        assert_eq!(day_label(today - 1, today), "Yesterday");
        assert_eq!(day_label(0, 20_000), "1 January 1970");
        assert_eq!(day_label(19_723, 20_000), "1 January 2024", "day 19723 is 2024-01-01");
        assert_eq!(day_label(19_997, 20_000), "1 October 2024", "274 days after 2024-01-01 in a leap year");
    }

    #[test]
    fn typing_names_one_two_or_several_people() {
        assert_eq!(typing_text(&[]), None);
        assert_eq!(typing_text(&["@bob:x".into()]).as_deref(), Some("bob is typing..."));
        assert_eq!(typing_text(&["@bob:x".into(), "@ann:x".into()]).as_deref(), Some("bob and ann are typing..."));
        let many = ["@a:x", "@b:x", "@c:x"].map(String::from);
        assert_eq!(typing_text(&many).as_deref(), Some("Several people are typing..."));
    }

    #[test]
    fn a_typing_update_names_the_others_and_an_absent_one_keeps_the_last() {
        let mut m = signed_in();
        m.apply_sync(&sync(vec![update("!a:x", vec![], 0)]));
        let mut typing = update("!a:x", vec![], 0);
        typing.typing = Some(vec!["@bob:x".into(), "@me:x".into()]);
        m.apply_sync(&sync(vec![typing]));
        assert_eq!(m.rooms[0].typing, vec!["@bob:x".to_string()], "my own typing is not shown");
        m.apply_sync(&sync(vec![update("!a:x", vec![], 0)]));
        assert_eq!(m.rooms[0].typing.len(), 1, "a sync without a typing update keeps the list");
        let mut stopped = update("!a:x", vec![], 0);
        stopped.typing = Some(vec![]);
        m.apply_sync(&sync(vec![stopped]));
        assert!(m.rooms[0].typing.is_empty());
    }

    #[test]
    fn the_unread_total_adds_every_room() {
        let mut m = signed_in();
        m.apply_sync(&sync(vec![update("!a:x", vec![], 2), update("!b:x", vec![], 3)]));
        assert_eq!(m.total_unread(), 5);
    }

    fn two_rooms() -> Messages {
        let mut m = signed_in();
        m.apply_sync(&sync(vec![
            update("!a:x", vec![ev("$1", "@bob:x", 10, Body::Text("lunch plans".into()))], 0),
            update("!b:x", vec![ev("$2", "@me:x", 20, Body::Text("mine".into()))], 0),
        ]));
        m
    }

    #[test]
    fn pinned_conversations_sort_first_and_keep_their_order() {
        let mut m = two_rooms();
        assert_eq!(m.visible_rooms().iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["!b:x", "!a:x"]);
        assert_eq!(m.toggle_pin("!a:x"), Ok(true));
        assert_eq!(m.visible_rooms()[0].id, "!a:x");
        assert_eq!(m.toggle_pin("!a:x"), Ok(false), "a second toggle unpins");
        assert_eq!(m.visible_rooms()[0].id, "!b:x");
    }

    #[test]
    fn pins_keep_the_order_they_were_made_in_and_can_be_moved() {
        let mut m = two_rooms();
        m.apply_sync(&sync(vec![update("!c:x", vec![ev("$3", "@a:x", 5, Body::Text("c".into()))], 0)]));
        m.toggle_pin("!a:x").unwrap();
        m.toggle_pin("!c:x").unwrap();
        let ids = |m: &Messages| m.visible_rooms().iter().map(|r| r.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&m), ["!a:x", "!c:x", "!b:x"], "pins in pin order, then the rest");
        assert!(!m.can_move_pin("!a:x", true) && m.can_move_pin("!a:x", false));
        assert!(!m.can_move_pin("!b:x", true), "an unpinned conversation does not move");
        assert!(m.move_pin("!c:x", true));
        assert_eq!(ids(&m), ["!c:x", "!a:x", "!b:x"]);
        assert!(!m.move_pin("!c:x", true), "already first");
        assert!(!m.move_pin("!a:x", false), "already last of the pins");
    }

    #[test]
    fn at_most_nine_conversations_can_be_pinned() {
        let mut m = signed_in();
        let updates: Vec<RoomUpdate> = (0..PIN_LIMIT + 1).map(|i| update(&format!("!r{i}:x"), vec![], 0)).collect();
        m.apply_sync(&sync(updates));
        for i in 0..PIN_LIMIT {
            assert_eq!(m.toggle_pin(&format!("!r{i}:x")), Ok(true));
        }
        let refused = m.toggle_pin(&format!("!r{PIN_LIMIT}:x"));
        assert!(refused.unwrap_err().contains('9'));
        assert!(!m.is_pinned(&format!("!r{PIN_LIMIT}:x")));
        assert_eq!(m.toggle_pin("!r0:x"), Ok(false), "a pin can always be taken off");
        assert_eq!(m.toggle_pin(&format!("!r{PIN_LIMIT}:x")), Ok(true), "which makes room");
    }

    #[test]
    fn the_search_matches_titles_and_loaded_messages() {
        let mut m = two_rooms();
        m.rooms[0].name = "Family".into();
        m.search = "FAMILY".into();
        assert_eq!(m.visible_rooms().len(), 1);
        m.search = "lunch".into();
        assert_eq!(m.visible_rooms()[0].id, "!a:x", "a word in a loaded message finds its conversation");
        m.search = "nothing like this".into();
        assert!(m.visible_rooms().is_empty());
        m.search = "  ".into();
        assert_eq!(m.visible_rooms().len(), 2, "blank search shows all");
    }

    #[test]
    fn hidden_alerts_skip_notifications_and_the_badge() {
        let mut m = signed_in();
        m.apply_sync(&sync(vec![update("!a:x", vec![], 0)]));
        m.toggle_mute("!a:x");
        let next = sync(vec![update("!a:x", vec![ev("$2", "@bob:x", 20, Body::Text("a".into()))], 3)]);
        assert!(m.apply_sync(&next).is_empty());
        assert_eq!(m.total_unread(), 0, "a muted conversation is not in the badge");
        assert_eq!(m.rooms[0].unread, 3, "but its own count still shows");
        assert!(!m.toggle_mute("!a:x"));
        assert_eq!(m.total_unread(), 3);
    }

    #[test]
    fn marking_as_unread_counts_once_and_opening_clears_it() {
        let mut m = two_rooms();
        assert!(m.toggle_unread_mark("!a:x"));
        assert_eq!(m.total_unread(), 1);
        m.select("!a:x");
        assert!(!m.is_marked_unread("!a:x"));
        assert_eq!(m.total_unread(), 0);
    }

    #[test]
    fn the_unread_flag_of_the_server_sets_and_clears_the_mark() {
        let mut m = two_rooms();
        let mut flagged = update("!a:x", vec![], 0);
        flagged.marked_unread = Some(true);
        m.apply_sync(&sync(vec![flagged.clone()]));
        assert!(m.is_marked_unread("!a:x"));
        assert!(!m.set_unread_mark("!a:x", true), "the same flag changes nothing");
        m.apply_sync(&sync(vec![update("!a:x", vec![], 0)]));
        assert!(m.is_marked_unread("!a:x"), "a sync without the flag leaves the mark");
        flagged.marked_unread = Some(false);
        m.apply_sync(&sync(vec![flagged]));
        assert!(!m.is_marked_unread("!a:x"));
    }

    #[test]
    fn the_server_does_not_mark_the_conversation_being_read() {
        let mut m = two_rooms();
        m.select("!a:x");
        let mut flagged = update("!a:x", vec![], 0);
        flagged.marked_unread = Some(true);
        m.apply_sync(&sync(vec![flagged]));
        assert!(!m.is_marked_unread("!a:x"));
        assert_eq!(m.selected.as_deref(), Some("!a:x"));
    }

    #[test]
    fn marking_the_open_conversation_closes_it() {
        let mut m = two_rooms();
        m.select("!a:x");
        m.begin_reply("$1");
        assert!(m.toggle_unread_mark("!a:x"));
        assert_eq!(m.selected, None);
        assert_eq!(m.compose, None);
        assert!(!m.toggle_unread_mark("!a:x"), "a second toggle takes the mark off");
    }

    #[test]
    fn preferences_round_trip_as_lines() {
        let list = vec!["!a:x".to_string(), "!b:x".to_string()];
        assert_eq!(RoomPrefs::decode(&RoomPrefs::encode(&list)), list);
        assert!(RoomPrefs::decode("\n  \n").is_empty());
    }

    #[test]
    fn a_removed_conversation_leaves_no_trace() {
        let mut m = two_rooms();
        m.select("!a:x");
        m.toggle_pin("!a:x").unwrap();
        m.toggle_mute("!a:x");
        m.overlay = Overlay::Details;
        m.remove_room("!a:x");
        assert_eq!(m.rooms.len(), 1);
        assert_eq!(m.selected, None);
        assert_eq!(m.overlay, Overlay::None);
        assert!(!m.is_pinned("!a:x") && !m.is_muted("!a:x"));
    }

    #[test]
    fn a_new_conversation_is_listed_and_selected_before_a_sync_names_it() {
        let mut m = two_rooms();
        m.show_new_room("!new:x");
        assert_eq!(m.selected.as_deref(), Some("!new:x"));
        assert!(m.rooms.iter().any(|r| r.id == "!new:x"));
    }

    #[test]
    fn shared_pictures_list_the_loaded_images_newest_first() {
        let mut m = signed_in();
        let image = |id: &str, ts: u64, name: &str| ev(id, "@bob:x", ts, Body::Image { name: name.into(), mxc: "mxc://x/y".into() });
        let events = vec![image("$1", 1, "a.png"), ev("$2", "@bob:x", 2, Body::Text("hi".into())), image("$3", 3, "b.png"), image("$4", 4, "c.png")];
        m.apply_sync(&sync(vec![update("!a:x", events, 0)]));
        assert_eq!(m.rooms[0].shared_pictures(), vec!["c.png".to_string(), "b.png".to_string(), "a.png".to_string()]);
        let mut gone = update("!a:x", vec![], 0);
        gone.redactions = vec!["$4".into()];
        m.apply_sync(&sync(vec![gone]));
        assert_eq!(m.rooms[0].shared_pictures(), vec!["b.png".to_string(), "a.png".to_string()], "a removed picture is not listed");
    }

    #[test]
    fn an_unnamed_room_is_called_after_the_others_in_it() {
        let mut m = signed_in();
        let mut u = update("!d:x", vec![], 0);
        u.members = vec![
            protocol::MemberUpdate { user_id: "@me:x".into(), name: None, membership: "join".into() },
            protocol::MemberUpdate { user_id: "@ann:x".into(), name: Some("Ann".into()), membership: "invite".into() },
        ];
        m.apply_sync(&sync(vec![u]));
        assert_eq!(m.rooms[0].title("@me:x"), "Ann");
        assert_eq!(m.rooms[0].members[1].label(), "Ann (invited)");
        m.rooms[0].name = "Trip".into();
        assert_eq!(m.rooms[0].title("@me:x"), "Trip", "a name wins");
    }

    #[test]
    fn a_leaving_member_is_dropped() {
        let mut m = signed_in();
        let mut u = update("!d:x", vec![], 0);
        u.members = vec![protocol::MemberUpdate { user_id: "@ann:x".into(), name: None, membership: "join".into() }];
        m.apply_sync(&sync(vec![u]));
        let mut gone = update("!d:x", vec![], 0);
        gone.members = vec![protocol::MemberUpdate { user_id: "@ann:x".into(), name: None, membership: "leave".into() }];
        m.apply_sync(&sync(vec![gone]));
        assert!(m.rooms[0].members.is_empty());
    }

    #[test]
    fn an_edit_replaces_the_words_of_the_senders_own_message_only() {
        let mut m = two_rooms();
        let mut u = update("!a:x", vec![], 0);
        u.edits = vec![
            protocol::Edit { target: "$1".into(), sender: "@eve:x".into(), body: Body::Text("forged".into()) },
            protocol::Edit { target: "$1".into(), sender: "@bob:x".into(), body: Body::Text("dinner plans".into()) },
        ];
        m.apply_sync(&sync(vec![u]));
        let message = &m.rooms.iter().find(|r| r.id == "!a:x").unwrap().timeline[0];
        assert_eq!(message.body, Body::Text("dinner plans".into()));
        assert!(message.edited);
    }

    #[test]
    fn an_edit_keeps_the_words_it_replaced() {
        let mut m = two_rooms();
        let edit = |text: &str| protocol::Edit { target: "$1".into(), sender: "@bob:x".into(), body: Body::Text(text.into()) };
        let mut u = update("!a:x", vec![], 0);
        u.edits = vec![edit("dinner plans"), edit("dinner plans"), edit("brunch plans")];
        m.apply_sync(&sync(vec![u]));
        let message = &m.rooms.iter().find(|r| r.id == "!a:x").unwrap().timeline[0];
        assert_eq!(message.history, vec![Body::Text("lunch plans".into()), Body::Text("dinner plans".into())]);
        assert_eq!(message.body, Body::Text("brunch plans".into()));
        m.select("!a:x");
        assert_eq!(
            m.info_lines(InfoKind::Edits, "$1"),
            ["1. lunch plans", "2. dinner plans", "3. brunch plans (current)"].map(String::from).to_vec()
        );
        let mut gone = update("!a:x", vec![], 0);
        gone.redactions = vec!["$1".into()];
        m.apply_sync(&sync(vec![gone]));
        let message = &m.rooms.iter().find(|r| r.id == "!a:x").unwrap().timeline[0];
        assert!(message.history.is_empty(), "removing a message forgets its earlier words");
    }

    #[test]
    fn the_reactions_panel_names_who_gave_each_emoji() {
        let mut m = two_rooms();
        let react = |id: &str, who: &str, key: &str| protocol::Annotation { id: id.into(), target: "$1".into(), key: key.into(), sender: who.into() };
        let mut u = update("!a:x", vec![], 0);
        u.reactions = vec![react("$r1", "@me:x", "+1"), react("$r2", "@bob:x", "+1"), react("$r3", "@ann:x", "ok")];
        m.apply_sync(&sync(vec![u]));
        assert!(m.info_lines(InfoKind::Reactions, "$1").is_empty(), "nothing is listed until the room is on screen");
        m.select("!a:x");
        assert_eq!(m.info_lines(InfoKind::Reactions, "$1"), vec!["+1  You, bob".to_string(), "ok  ann".to_string()]);
        assert!(m.info_lines(InfoKind::Reactions, "$none").is_empty());
        assert_eq!((InfoKind::Edits.title(), InfoKind::Reactions.title()), ("Edit History", "Reactions"));
    }

    #[test]
    fn a_long_history_lists_only_the_newest_versions_with_true_numbers() {
        let mut m = two_rooms();
        let mut u = update("!a:x", vec![], 0);
        u.edits = (0..INFO_ROWS_MAX + 4)
            .map(|i| protocol::Edit { target: "$1".into(), sender: "@bob:x".into(), body: Body::Text(format!("v{i}")) })
            .collect();
        m.apply_sync(&sync(vec![u]));
        m.select("!a:x");
        let lines = m.info_lines(InfoKind::Edits, "$1");
        assert_eq!(lines.len(), INFO_ROWS_MAX);
        assert!(lines[0].starts_with("6. "), "{lines:?}");
        assert!(lines[INFO_ROWS_MAX - 1].ends_with("(current)"));
    }

    #[test]
    fn reactions_group_by_emoji_and_a_removal_takes_one_back() {
        let mut m = two_rooms();
        let react = |id: &str, who: &str, key: &str| protocol::Annotation { id: id.into(), target: "$1".into(), key: key.into(), sender: who.into() };
        let mut u = update("!a:x", vec![], 0);
        u.reactions = vec![react("$r1", "@me:x", "+1"), react("$r2", "@bob:x", "+1"), react("$r3", "@bob:x", "ok"), react("$r1", "@me:x", "+1")];
        m.apply_sync(&sync(vec![u]));
        let message = m.rooms.iter().find(|r| r.id == "!a:x").unwrap().timeline[0].clone();
        assert_eq!(message.reactions.len(), 2);
        assert_eq!(message.reactions[0].users.len(), 2, "a repeated event id counts once");
        assert_eq!(message.reaction_of("@me:x", "+1"), Some("$r1"));
        assert_eq!(message.reaction_of("@me:x", "ok"), None);
        let mut back = update("!a:x", vec![], 0);
        back.redactions = vec!["$r3".into()];
        m.apply_sync(&sync(vec![back]));
        let message = &m.rooms.iter().find(|r| r.id == "!a:x").unwrap().timeline[0];
        assert_eq!(message.reactions.len(), 1, "an emptied reaction disappears");
    }

    #[test]
    fn removing_a_message_blanks_it_and_its_reactions() {
        let mut m = two_rooms();
        let mut u = update("!a:x", vec![], 0);
        u.reactions = vec![protocol::Annotation { id: "$r".into(), target: "$1".into(), key: "+1".into(), sender: "@me:x".into() }];
        m.apply_sync(&sync(vec![u]));
        let mut gone = update("!a:x", vec![], 0);
        gone.redactions = vec!["$1".into()];
        m.apply_sync(&sync(vec![gone]));
        let message = &m.rooms.iter().find(|r| r.id == "!a:x").unwrap().timeline[0];
        assert_eq!(message.body, Body::Redacted);
        assert!(message.reactions.is_empty());
    }

    #[test]
    fn a_reply_quotes_its_target_and_sends_with_the_relation() {
        let mut m = two_rooms();
        m.select("!a:x");
        assert!(!m.begin_reply("$nope"));
        assert!(m.begin_reply("$1"));
        assert_eq!(m.banner().as_deref(), Some("Replying to bob: lunch plans"));
        m.composer = " sure ".into();
        assert_eq!(m.take_outgoing(), Some(Outgoing::Reply { text: "sure".into(), to: "$1".into() }));
        assert_eq!(m.compose, None);
        assert_eq!(m.banner(), None);
    }

    #[test]
    fn only_my_own_text_messages_can_be_edited() {
        let mut m = two_rooms();
        m.select("!a:x");
        assert!(!m.begin_edit("$1"), "bob's message");
        m.select("!b:x");
        assert!(m.begin_edit("$2"));
        assert_eq!(m.composer, "mine", "the words come back into the composer");
        assert_eq!(m.banner().as_deref(), Some("Editing message"));
        m.composer = "mine, edited".into();
        assert_eq!(m.take_outgoing(), Some(Outgoing::Edit { text: "mine, edited".into(), target: "$2".into() }));
    }

    #[test]
    fn an_edit_that_changes_nothing_is_not_sent_and_cancel_empties_the_composer() {
        let mut m = two_rooms();
        m.select("!b:x");
        m.begin_edit("$2");
        assert_eq!(m.take_outgoing(), None);
        m.begin_edit("$2");
        m.cancel_compose();
        assert!(m.composer.is_empty());
        assert_eq!(m.compose, None);
        m.begin_reply("$2");
        m.composer = "draft".into();
        m.cancel_compose();
        assert_eq!(m.composer, "draft", "cancelling a reply keeps what was typed");
    }

    #[test]
    fn plain_text_sends_as_text() {
        let mut m = two_rooms();
        m.composer = "hi".into();
        assert_eq!(m.take_outgoing(), Some(Outgoing::Text("hi".into())));
        assert_eq!(m.take_outgoing(), None);
    }

    #[test]
    fn the_sheet_checks_what_it_asks_for() {
        let mut p = Prompt::new_message();
        assert!(p.checked().is_err(), "a bare @ is not a user id");
        p.value = "@bob:example.org".into();
        assert_eq!(p.checked(), Ok("@bob:example.org".into()));
        p.value = "@bob:example.org, @ann:example.org".into();
        assert_eq!(
            Prompt::users(&p.checked().unwrap()),
            vec!["@bob:example.org".to_string(), "@ann:example.org".to_string()]
        );
        let mut r = Prompt::rename("!a:x", "Old");
        r.value = "  ".into();
        assert_eq!(r.checked(), Err("Enter a name".into()));
        r.value = " New ".into();
        assert_eq!(r.checked(), Ok("New".into()));
        assert_eq!((p.title(), r.title()), ("New Message", "Rename Conversation"));
    }

    #[test]
    fn signing_out_keeps_only_the_preferences() {
        let mut m = two_rooms();
        m.toggle_pin("!a:x").unwrap();
        m.composer = "x".into();
        m.sign_out();
        assert!(!m.signed_in());
        assert!(m.rooms.is_empty() && m.composer.is_empty());
        assert!(m.is_pinned("!a:x"));
    }

    #[test]
    fn the_emoji_picker_fills_whole_rows() {
        assert_eq!(EMOJI_CATALOG.len() % EMOJI_COLUMNS, 0);
        assert_eq!(REACTIONS.len(), 6);
        let mut seen: Vec<&str> = EMOJI_CATALOG.iter().map(|(e, _)| *e).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), EMOJI_CATALOG.len(), "no emoji twice");
    }

    #[test]
    fn a_search_finds_emoji_by_every_word() {
        assert_eq!(emoji_matches("").len(), EMOJI_CATALOG.len());
        assert_eq!(emoji_matches("THUMBS up"), vec!["\u{1F44D}"]);
        assert!(emoji_matches("party").contains(&"\u{1F389}") && emoji_matches("party").contains(&"\u{1F973}"));
        assert!(emoji_matches("no such word").is_empty());
        assert_eq!(emoji_matches("\u{1F525}"), vec!["\u{1F525}"], "the emoji itself finds itself");
    }

    #[test]
    fn the_picker_puts_recent_emoji_first_and_a_search_replaces_the_list() {
        let mut m = signed_in();
        m.open_emoji(None);
        assert_eq!(m.emoji_choices().len(), EMOJI_CATALOG.len());
        m.note_emoji_used("\u{1F525}");
        m.note_emoji_used("\u{1F389}");
        m.note_emoji_used("\u{1F525}");
        assert_eq!(m.prefs.recent_emoji, vec!["\u{1F525}".to_string(), "\u{1F389}".to_string()], "newest first, no repeats");
        let choices = m.emoji_choices();
        assert_eq!(&choices[..2], ["\u{1F525}".to_string(), "\u{1F389}".to_string()]);
        assert_eq!(choices.len(), EMOJI_CATALOG.len(), "a recent emoji is not listed twice");
        m.emoji_query = "cat".into();
        assert_eq!(m.emoji_choices(), vec!["\u{1F431}".to_string()]);
        m.open_emoji(Some("$1".into()));
        assert!(m.emoji_query.is_empty(), "opening the picker clears the search");
        assert_eq!(m.overlay, Overlay::Emoji(Some("$1".into())));
    }

    #[test]
    fn only_the_latest_emoji_are_remembered() {
        let mut m = signed_in();
        for (emoji, _) in EMOJI_CATALOG.iter().take(RECENT_EMOJI_MAX + 3) {
            m.note_emoji_used(emoji);
        }
        assert_eq!(m.prefs.recent_emoji.len(), RECENT_EMOJI_MAX);
        assert_eq!(m.prefs.recent_emoji[0], EMOJI_CATALOG[RECENT_EMOJI_MAX + 2].0);
    }
}
