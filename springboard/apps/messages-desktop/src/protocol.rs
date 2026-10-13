//! The Matrix client-server wire format (the `/_matrix/client/v3` API): the URLs the app calls, the request bodies it
//! sends and the answers it reads. Nothing here opens a connection; [`crate::api`] does that. Every answer is parsed from
//! the JSON tree (`bifrost_core::json`), so the tests feed it the shapes a homeserver sends.

use bifrost_core::json::Json;

/// The path every client call starts with.
pub const CLIENT_API: &str = "/_matrix/client/v3";
/// The path media uploads go to.
pub const MEDIA_API: &str = "/_matrix/media/v3";
/// The longest sync long-poll asked for, in milliseconds. The shell's transport gives up on a silent socket after 25 s.
pub const SYNC_TIMEOUT_MS: u32 = 20_000;
/// The name a new login's device shows as on the account.
pub const DEVICE_NAME: &str = "BifrOSt Messages";

/// A signed-in session as the login answer gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login {
    pub user_id: String,
    pub access_token: String,
    pub device_id: String,
}

/// One sync answer: the token for the next sync and the joined rooms that changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sync {
    pub next_batch: String,
    pub rooms: Vec<RoomUpdate>,
}

/// What a sync says about one joined room.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomUpdate {
    pub id: String,
    /// The room's name (its `m.room.name`, or its canonical alias when it has none) when the sync carried one.
    pub name: Option<String>,
    /// The messages of the timeline, in server order. State events and other kinds are left out.
    pub events: Vec<Event>,
    /// Notifications the server counts as unread for this account.
    pub unread: u64,
    pub highlights: u64,
    /// Read receipts that came with the room (who has read up to which event).
    pub receipts: Vec<Receipt>,
    /// Who is typing in the room, when the sync carried a typing update (`None` when it did not, so the last one stands).
    pub typing: Option<Vec<String>>,
}

/// A message in a timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub id: String,
    pub sender: String,
    /// Server time in milliseconds since the Unix epoch.
    pub ts: u64,
    pub body: Body,
}

/// What a message says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Text(String),
    Notice(String),
    Emote(String),
    /// An image: its file name and its `mxc://` content URI.
    Image { name: String, mxc: String },
    /// An end-to-end encrypted message. The client does not decrypt yet, so the text is not shown.
    Encrypted,
    /// Another kind of message (a file, a video, ...), by its `msgtype`.
    Other(String),
    /// A message that was removed (its content is empty).
    Redacted,
}

/// A read receipt: `user_id` has read up to `event_id` at `ts`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub event_id: String,
    pub user_id: String,
    pub ts: u64,
}

/// What an image upload needs to say about the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageInfo<'a> {
    pub name: &'a str,
    pub mxc: &'a str,
    pub mime: &'a str,
    pub size: u64,
}

/// A homeserver address as the sign-in sheet takes it (`matrix.org`, `https://matrix.example.org/`). Gives the base URL
/// with `https://` and no trailing slash. Only https is accepted, because a sign-in sends the password.
pub fn homeserver_base(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    // The scheme is split off before the trailing slashes are trimmed, so `https://` alone leaves an empty host.
    let rest = match trimmed.split_once("://") {
        Some(("https", rest)) => rest,
        Some(_) => return Err("The homeserver must use https".into()),
        None => trimmed,
    };
    let host = rest.trim_end_matches('/');
    if host.is_empty() {
        return Err("Enter the homeserver address".into());
    }
    if host.contains('/') || host.contains(char::is_whitespace) {
        return Err("Enter the homeserver as a host name, such as matrix.org".into());
    }
    Ok(format!("https://{host}"))
}

/// Percent-encodes one URL path segment or query value. RFC 3986 unreserved characters pass; everything else, including
/// the `!`, `:` and `$` of room and event ids, is escaped.
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn api(base: &str, path: &str) -> String {
    format!("{base}{CLIENT_API}{path}")
}

pub fn login_url(base: &str) -> String {
    api(base, "/login")
}

/// The sync URL: a long-poll that answers at once for the first call (no `since`) and after the next change later.
pub fn sync_url(base: &str, since: Option<&str>) -> String {
    let mut url = api(base, &format!("/sync?timeout={SYNC_TIMEOUT_MS}"));
    if let Some(token) = since {
        url.push_str("&since=");
        url.push_str(&encode(token));
    }
    url
}

/// The PUT that sends a message. `txn` makes a retried send one message on the server.
pub fn send_url(base: &str, room: &str, txn: &str) -> String {
    api(base, &format!("/rooms/{}/send/m.room.message/{}", encode(room), encode(txn)))
}

/// The POST that marks `event` as read in `room`.
pub fn receipt_url(base: &str, room: &str, event: &str) -> String {
    api(base, &format!("/rooms/{}/receipt/m.read/{}", encode(room), encode(event)))
}

pub fn upload_url(base: &str, file_name: &str) -> String {
    format!("{base}{MEDIA_API}/upload?filename={}", encode(file_name))
}

/// The login request: a password login for `user` (a local part or a full Matrix id).
pub fn login_body(user: &str, password: &str) -> String {
    obj(vec![
        ("type", text("m.login.password")),
        ("identifier", obj(vec![("type", text("m.id.user")), ("user", text(user))])),
        ("password", text(password)),
        ("initial_device_display_name", text(DEVICE_NAME)),
    ])
    .encode()
}

/// The content of a plain text message.
pub fn text_content(body: &str) -> String {
    obj(vec![("msgtype", text("m.text")), ("body", text(body))]).encode()
}

/// The content of an image message, after the upload gave it an `mxc://` URI.
pub fn image_content(info: &ImageInfo) -> String {
    obj(vec![
        ("msgtype", text("m.image")),
        ("body", text(info.name)),
        ("url", text(info.mxc)),
        ("info", obj(vec![("mimetype", text(info.mime)), ("size", Json::Num(info.size as f64))])),
    ])
    .encode()
}

/// The MIME type of an image file the app sends, from its extension. `None` for anything else.
pub fn image_mime(name: &str) -> Option<&'static str> {
    let (_, ext) = name.rsplit_once('.')?;
    match ext.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// The name a user shows as: the local part of `@alice:example.org`, which is `alice`.
pub fn display_name(user_id: &str) -> &str {
    let rest = user_id.strip_prefix('@').unwrap_or(user_id);
    rest.split(':').next().unwrap_or(rest)
}

/// The error a failed call answered with (`errcode`, and `error` when present), when the body has one.
pub fn error_text(body: &Json) -> Option<String> {
    let code = body.str_field("errcode")?;
    Some(match body.str_field("error") {
        Some(text) => format!("{code}: {text}"),
        None => code,
    })
}

pub fn parse_login(body: &Json) -> Result<Login, String> {
    if body.str_field("errcode").as_deref() == Some("M_FORBIDDEN") {
        return Err("The user name or password is wrong".into());
    }
    if let Some(e) = error_text(body) {
        return Err(e);
    }
    Ok(Login {
        user_id: field(body, "user_id")?,
        access_token: field(body, "access_token")?,
        device_id: body.str_field("device_id").unwrap_or_default(),
    })
}

pub fn parse_sync(body: &Json) -> Result<Sync, String> {
    if let Some(e) = error_text(body) {
        return Err(e);
    }
    let next_batch = field(body, "next_batch")?;
    let mut rooms = Vec::new();
    if let Some(Json::Obj(joined)) = body.get("rooms").and_then(|r| r.get("join")) {
        for (id, room) in joined {
            rooms.push(parse_room(id, room));
        }
    }
    Ok(Sync { next_batch, rooms })
}

/// The `content_uri` of an upload answer.
pub fn parse_content_uri(body: &Json) -> Result<String, String> {
    if let Some(e) = error_text(body) {
        return Err(e);
    }
    let uri = field(body, "content_uri")?;
    if uri.starts_with("mxc://") {
        Ok(uri)
    } else {
        Err("the server did not return a media address".into())
    }
}

/// The event id of a send answer.
pub fn parse_event_id(body: &Json) -> Result<String, String> {
    if let Some(e) = error_text(body) {
        return Err(e);
    }
    field(body, "event_id")
}

fn field(body: &Json, key: &str) -> Result<String, String> {
    body.str_field(key).ok_or_else(|| format!("the server did not send {key}"))
}

fn text(s: &str) -> Json {
    Json::Str(s.to_string())
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The raw events of a sync section (`timeline`, `state`, `ephemeral`), or none.
fn raw_events(section: Option<&Json>) -> &[Json] {
    section.and_then(|s| s.get("events")).and_then(|e| e.as_array()).unwrap_or(&[])
}

fn parse_room(id: &str, room: &Json) -> RoomUpdate {
    let state = raw_events(room.get("state"));
    let timeline = raw_events(room.get("timeline"));
    let name = room_name(timeline).or_else(|| room_name(state));
    let counts = room.get("unread_notifications");
    let count = |key: &str| counts.and_then(|c| c.get(key)).and_then(|v| v.as_u64()).unwrap_or(0);
    RoomUpdate {
        id: id.to_string(),
        name,
        events: messages(timeline),
        unread: count("notification_count"),
        highlights: count("highlight_count"),
        receipts: receipts(raw_events(room.get("ephemeral"))),
        typing: typing(raw_events(room.get("ephemeral"))),
    }
}

/// The typing list of an ephemeral section: the user ids of the latest `m.typing` event. `None` when there is none.
fn typing(events: &[Json]) -> Option<Vec<String>> {
    let ev = events.iter().find(|e| e.str_field("type").as_deref() == Some("m.typing"))?;
    let ids = ev.get("content")?.get("user_ids")?.as_array()?;
    Some(ids.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
}

/// The last name a room's events set: `m.room.name`, or else `m.room.canonical_alias`. Empty names do not count.
fn room_name(events: &[Json]) -> Option<String> {
    let last = |kind: &str, key: &str| {
        events
            .iter()
            .filter(|e| e.str_field("type").as_deref() == Some(kind))
            .filter_map(|e| e.get("content")?.str_field(key))
            .filter(|v| !v.is_empty())
            .last()
    };
    last("m.room.name", "name").or_else(|| last("m.room.canonical_alias", "alias"))
}

fn messages(events: &[Json]) -> Vec<Event> {
    events.iter().filter_map(message).collect()
}

fn message(ev: &Json) -> Option<Event> {
    let kind = ev.str_field("type")?;
    let body = match kind.as_str() {
        "m.room.message" => message_body(ev.get("content")),
        "m.room.encrypted" => Body::Encrypted,
        _ => return None,
    };
    Some(Event {
        id: ev.str_field("event_id")?,
        sender: ev.str_field("sender")?,
        ts: ev.get("origin_server_ts").and_then(|v| v.as_u64()).unwrap_or(0),
        body,
    })
}

fn message_body(content: Option<&Json>) -> Body {
    let Some(content) = content else { return Body::Redacted };
    let text = |key: &str| content.str_field(key).unwrap_or_default();
    match content.str_field("msgtype").as_deref() {
        None => Body::Redacted,
        Some("m.text") => Body::Text(text("body")),
        Some("m.notice") => Body::Notice(text("body")),
        Some("m.emote") => Body::Emote(text("body")),
        Some("m.image") => Body::Image { name: text("body"), mxc: text("url") },
        Some(other) => Body::Other(other.to_string()),
    }
}

/// The read receipts of an ephemeral section: `content` maps each event id to `m.read`, then to the readers.
fn receipts(events: &[Json]) -> Vec<Receipt> {
    let mut out = Vec::new();
    for ev in events {
        if ev.str_field("type").as_deref() != Some("m.receipt") {
            continue;
        }
        let Some(Json::Obj(by_event)) = ev.get("content") else { continue };
        for (event_id, kinds) in by_event {
            let Some(Json::Obj(readers)) = kinds.get("m.read") else { continue };
            for (user, info) in readers {
                let ts = info.get("ts").and_then(|t| t.as_u64()).unwrap_or(0);
                out.push(Receipt { event_id: event_id.clone(), user_id: user.clone(), ts });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::json;

    fn parse(text: &str) -> Json {
        json::parse(text).expect("valid JSON")
    }

    #[test]
    fn homeserver_addresses_become_https_base_urls() {
        assert_eq!(homeserver_base("matrix.org").unwrap(), "https://matrix.org");
        assert_eq!(homeserver_base("  https://matrix.example.org/ ").unwrap(), "https://matrix.example.org");
        assert!(homeserver_base("http://plain.example.org").is_err(), "a password must not go over plain http");
        assert!(homeserver_base("").is_err());
        assert!(homeserver_base("https://example.org/path").is_err());
        assert!(homeserver_base("https://").is_err());
    }

    #[test]
    fn room_and_event_ids_are_percent_encoded() {
        assert_eq!(encode("!abc:matrix.org"), "%21abc%3Amatrix.org");
        assert_eq!(encode("$ev-1.x_y~z"), "%24ev-1.x_y~z");
        let url = send_url("https://m.example", "!r:m.example", "t1");
        assert_eq!(url, "https://m.example/_matrix/client/v3/rooms/%21r%3Am.example/send/m.room.message/t1");
    }

    #[test]
    fn sync_urls_carry_the_timeout_and_the_token() {
        assert_eq!(sync_url("https://m", None), "https://m/_matrix/client/v3/sync?timeout=20000");
        assert_eq!(sync_url("https://m", Some("s1_2")), "https://m/_matrix/client/v3/sync?timeout=20000&since=s1_2");
        assert_eq!(receipt_url("https://m", "!r:x", "$e"), "https://m/_matrix/client/v3/rooms/%21r%3Ax/receipt/m.read/%24e");
        assert_eq!(upload_url("https://m", "a b.png"), "https://m/_matrix/media/v3/upload?filename=a%20b.png");
    }

    #[test]
    fn login_body_is_a_password_login_with_a_user_identifier() {
        let body = parse(&login_body("alice", "p\"w"));
        assert_eq!(body.str_field("type").as_deref(), Some("m.login.password"));
        assert_eq!(body.get("identifier").and_then(|i| i.str_field("user")).as_deref(), Some("alice"));
        assert_eq!(body.str_field("password").as_deref(), Some("p\"w"), "quotes survive the encoding");
    }

    #[test]
    fn a_login_answer_gives_the_session() {
        let ok = parse(r#"{"user_id":"@alice:example.org","access_token":"tok","device_id":"DEV"}"#);
        assert_eq!(
            parse_login(&ok).unwrap(),
            Login { user_id: "@alice:example.org".into(), access_token: "tok".into(), device_id: "DEV".into() }
        );
        let wrong = parse(r#"{"errcode":"M_FORBIDDEN","error":"Invalid password"}"#);
        assert_eq!(parse_login(&wrong), Err("The user name or password is wrong".into()));
        let missing = parse(r#"{"user_id":"@a:b"}"#);
        assert!(parse_login(&missing).unwrap_err().contains("access_token"));
    }

    #[test]
    fn content_bodies_round_trip_through_the_parser() {
        let text = parse(&text_content("hi \"there\"\n"));
        assert_eq!(text.str_field("msgtype").as_deref(), Some("m.text"));
        assert_eq!(text.str_field("body").as_deref(), Some("hi \"there\"\n"));
        let info = ImageInfo { name: "cat.png", mxc: "mxc://m/abc", mime: "image/png", size: 42 };
        let image = parse(&image_content(&info));
        assert_eq!(image.str_field("url").as_deref(), Some("mxc://m/abc"));
        assert_eq!(image.get("info").and_then(|i| i.get("size")).and_then(|s| s.as_u64()), Some(42));
        assert_eq!(image.get("info").and_then(|i| i.str_field("mimetype")).as_deref(), Some("image/png"));
    }

    #[test]
    fn image_types_come_from_the_extension() {
        assert_eq!(image_mime("Photo.JPG"), Some("image/jpeg"));
        assert_eq!(image_mime("a.webp"), Some("image/webp"));
        assert_eq!(image_mime("notes.txt"), None);
        assert_eq!(image_mime("png"), None, "a name without an extension is not an image");
    }

    #[test]
    fn display_names_are_local_parts() {
        assert_eq!(display_name("@alice:example.org"), "alice");
        assert_eq!(display_name("bob"), "bob");
    }

    #[test]
    fn upload_and_send_answers() {
        let up = parse(r#"{"content_uri":"mxc://example.org/Xy"}"#);
        assert_eq!(parse_content_uri(&up).unwrap(), "mxc://example.org/Xy");
        assert!(parse_content_uri(&parse(r#"{"content_uri":"https://x"}"#)).is_err());
        let sent = parse(r#"{"event_id":"$ev1"}"#);
        assert_eq!(parse_event_id(&sent).unwrap(), "$ev1");
        let denied = parse(r#"{"errcode":"M_LIMIT_EXCEEDED","error":"slow down"}"#);
        assert_eq!(parse_event_id(&denied), Err("M_LIMIT_EXCEEDED: slow down".into()));
    }

    const SYNC: &str = r#"{
        "next_batch": "s100",
        "rooms": {
            "join": {
                "!room1:example.org": {
                    "state": {"events": [
                        {"type": "m.room.name", "content": {"name": "Family"}, "event_id": "$s1", "sender": "@a:example.org", "origin_server_ts": 1}
                    ]},
                    "timeline": {"events": [
                        {"type": "m.room.message", "event_id": "$e1", "sender": "@bob:example.org", "origin_server_ts": 1000,
                         "content": {"msgtype": "m.text", "body": "hello"}},
                        {"type": "m.room.member", "event_id": "$m1", "sender": "@bob:example.org", "origin_server_ts": 1001,
                         "content": {"membership": "join"}},
                        {"type": "m.room.message", "event_id": "$e2", "sender": "@alice:example.org", "origin_server_ts": 2000,
                         "content": {"msgtype": "m.image", "body": "cat.png", "url": "mxc://example.org/cat"}},
                        {"type": "m.room.encrypted", "event_id": "$e3", "sender": "@bob:example.org", "origin_server_ts": 3000,
                         "content": {"algorithm": "m.megolm.v1.aes-sha2"}},
                        {"type": "m.room.message", "event_id": "$e4", "sender": "@bob:example.org", "origin_server_ts": 4000,
                         "content": {}},
                        {"type": "m.room.message", "event_id": "$e5", "sender": "@bob:example.org", "origin_server_ts": 5000,
                         "content": {"msgtype": "m.file", "body": "a.pdf"}}
                    ]},
                    "ephemeral": {"events": [
                        {"type": "m.typing", "content": {"user_ids": []}},
                        {"type": "m.receipt", "content": {
                            "$e1": {"m.read": {"@alice:example.org": {"ts": 1500}}},
                            "$e2": {"m.read": {"@bob:example.org": {"ts": 2500}}}
                        }}
                    ]},
                    "unread_notifications": {"notification_count": 3, "highlight_count": 1}
                },
                "!room2:example.org": {
                    "timeline": {"events": []},
                    "unread_notifications": {"notification_count": 0, "highlight_count": 0}
                }
            },
            "invite": {}
        }
    }"#;

    #[test]
    fn a_sync_answer_gives_the_rooms_messages_and_counts() {
        let sync = parse_sync(&parse(SYNC)).unwrap();
        assert_eq!(sync.next_batch, "s100");
        assert_eq!(sync.rooms.len(), 2);
        let room = sync.rooms.iter().find(|r| r.id == "!room1:example.org").unwrap();
        assert_eq!(room.name.as_deref(), Some("Family"), "the state's name is the fallback when the timeline names nothing");
        assert_eq!(room.unread, 3);
        assert_eq!(room.highlights, 1);
        assert_eq!(room.events.len(), 5, "the member event is not a message; the empty one is kept as redacted");
        assert_eq!(room.events[0], Event { id: "$e1".into(), sender: "@bob:example.org".into(), ts: 1000, body: Body::Text("hello".into()) });
        assert_eq!(room.events[1].body, Body::Image { name: "cat.png".into(), mxc: "mxc://example.org/cat".into() });
        assert_eq!(room.events[2].body, Body::Encrypted);
        assert_eq!(room.events[3].body, Body::Redacted);
        assert_eq!(room.events[4].body, Body::Other("m.file".into()));
        assert_eq!(room.receipts.len(), 2);
        assert_eq!(room.typing, Some(Vec::new()), "an m.typing event with nobody typing is an answer, not a silence");
        assert!(room.receipts.contains(&Receipt { event_id: "$e1".into(), user_id: "@alice:example.org".into(), ts: 1500 }));
    }

    #[test]
    fn a_room_without_a_name_has_none_and_an_empty_one_has_no_events() {
        let sync = parse_sync(&parse(SYNC)).unwrap();
        let room = sync.rooms.iter().find(|r| r.id == "!room2:example.org").unwrap();
        assert_eq!(room.name, None);
        assert!(room.events.is_empty());
    }

    #[test]
    fn a_typing_update_lists_the_users_and_a_room_without_one_has_none() {
        let typing = parse(
            r#"{"next_batch":"s1","rooms":{"join":{"!t:x":{"timeline":{"events":[]},"ephemeral":{"events":[
                {"type":"m.typing","content":{"user_ids":["@bob:x","@me:x"]}}]}}}}}"#,
        );
        let sync = parse_sync(&typing).unwrap();
        assert_eq!(sync.rooms[0].typing, Some(vec!["@bob:x".to_string(), "@me:x".to_string()]));
        let quiet = parse(r#"{"next_batch":"s1","rooms":{"join":{"!q:x":{"timeline":{"events":[]}}}}}"#);
        assert_eq!(parse_sync(&quiet).unwrap().rooms[0].typing, None);
    }

    #[test]
    fn a_state_name_is_used_when_the_timeline_has_none_and_the_alias_is_the_last_fallback() {
        let named = parse(
            r#"{"next_batch":"x","rooms":{"join":{"!r:x":{
                "state":{"events":[{"type":"m.room.name","content":{"name":"Family"}}]},
                "timeline":{"events":[]}}}}}"#,
        );
        assert_eq!(parse_sync(&named).unwrap().rooms[0].name.as_deref(), Some("Family"));
        let alias = parse(
            r##"{"next_batch":"x","rooms":{"join":{"!r:x":{
                "state":{"events":[{"type":"m.room.canonical_alias","content":{"alias":"#dev:x"}}]},
                "timeline":{"events":[]}}}}}"##,
        );
        assert_eq!(parse_sync(&alias).unwrap().rooms[0].name.as_deref(), Some("#dev:x"));
    }

    #[test]
    fn the_last_timeline_name_wins() {
        let text = parse(
            r#"{"next_batch":"x","rooms":{"join":{"!r:x":{
                "timeline":{"events":[
                    {"type":"m.room.name","content":{"name":"Old"}},
                    {"type":"m.room.name","content":{"name":"New"}}]}}}}}"#,
        );
        assert_eq!(parse_sync(&text).unwrap().rooms[0].name.as_deref(), Some("New"));
    }

    #[test]
    fn a_sync_without_a_token_is_an_error_and_an_error_answer_says_why() {
        assert!(parse_sync(&parse(r#"{"rooms":{}}"#)).unwrap_err().contains("next_batch"));
        let denied = parse(r#"{"errcode":"M_UNKNOWN_TOKEN","error":"Unrecognised access token"}"#);
        assert_eq!(parse_sync(&denied), Err("M_UNKNOWN_TOKEN: Unrecognised access token".into()));
    }

    #[test]
    fn a_sync_with_no_joined_rooms_is_empty() {
        let sync = parse_sync(&parse(r#"{"next_batch":"n","rooms":{"invite":{"!i:x":{}}}}"#)).unwrap();
        assert_eq!(sync, Sync { next_batch: "n".into(), rooms: Vec::new() });
    }
}
