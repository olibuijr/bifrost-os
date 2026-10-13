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
}

impl Room {
    fn named(id: &str) -> Room {
        Room { id: id.to_string(), name: id.to_string(), ..Room::default() }
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

/// The app's state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Messages {
    /// The signed-in user's id. Empty until a sign-in succeeds.
    pub user_id: String,
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
}

impl Messages {
    pub fn signed_in(&self) -> bool {
        !self.user_id.is_empty()
    }

    pub fn selected_room(&self) -> Option<&Room> {
        let id = self.selected.as_deref()?;
        self.rooms.iter().find(|r| r.id == id)
    }

    /// The unread notifications of every room, for the dock badge.
    pub fn total_unread(&self) -> u64 {
        self.rooms.iter().map(|r| r.unread).sum()
    }

    /// Applies one sync answer and returns the notifications for messages that just arrived from other people.
    /// Rooms are kept in order of their newest message, newest first.
    pub fn apply_sync(&mut self, sync: &Sync) -> Vec<Notice> {
        let mut notices = Vec::new();
        for update in &sync.rooms {
            let index = self.room_index(&update.id);
            let viewing = self.selected.as_deref() == Some(update.id.as_str());
            apply_room(&mut self.rooms[index], update, &self.user_id, self.primed, viewing, &mut notices);
        }
        self.primed = true;
        self.rooms.sort_by(|a, b| b.last_ts.cmp(&a.last_ts).then_with(|| a.name.cmp(&b.name)));
        notices
    }

    /// Puts `id` on screen. The room's unread counts clear here; the returned value is the read marker to send, see
    /// [`Messages::take_read_marker`].
    pub fn select(&mut self, id: &str) -> Option<(String, String)> {
        self.selected = Some(id.to_string());
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
        room.timeline.push(Message { id: event.id.clone(), sender: event.sender.clone(), ts: event.ts, body: event.body.clone() });
    }
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
        Event { id: id.into(), sender: sender.into(), ts, body }
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
}
