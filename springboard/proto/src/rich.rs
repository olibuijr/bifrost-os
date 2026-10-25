//! Wire types for user notifications (`UNUserNotificationCenter`), sharing (`NativeSharingServicePicker`) and typed drag and
//! drop (`NativeDraggingSource` / `NativeDraggingDestination`). Tags: `ToApp` 50 to 59, `ToShell` 0x140 to 0x14f. The enum
//! variants live in `ToApp` / `ToShell`; this module holds their payload types and the codec for them.
//!
//! Every list and string is capped on encode and refused (`BadValue`) on decode, so a packet stays below `MAX_PACKET`.

use super::{f32_, str_, u32_, DecodeError, GrantId, Reader, ToApp, ToShell, UiRect};

/// `UNNotificationDefaultActionIdentifier`: the user opened the notification itself.
pub const NOTIF_DEFAULT_ACTION: &str = "bifrost.notification.default";
/// `UNNotificationDismissActionIdentifier`: the user dismissed a notification whose category asked to hear about it.
pub const NOTIF_DISMISS_ACTION: &str = "bifrost.notification.dismiss";

pub const MAX_NOTIF_CATEGORIES: usize = 16;
pub const MAX_NOTIF_ACTIONS: usize = 10;
pub const MAX_NOTIF_ID: usize = 128;
pub const MAX_NOTIF_TEXT: usize = 256;
pub const MAX_NOTIF_IDS: usize = 64;
/// `userInfo` entries of one request, and the longest value.
pub const MAX_NOTIF_USER_INFO: usize = 16;
pub const MAX_NOTIF_USER_INFO_VALUE: usize = 512;
/// Attachments of one request, and the longest path.
pub const MAX_NOTIF_ATTACHMENTS: usize = 10;
pub const MAX_NOTIF_ATTACHMENT_PATH: usize = 1024;

/// A text-reply action (`UNTextInputNotificationAction`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextInput {
    pub button_title: String,
    pub placeholder: String,
}

/// `UNNotificationAction`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotifAction {
    pub id: String,
    pub title: String,
    /// `.destructive`: drawn red
    pub destructive: bool,
    /// `.foreground`: choosing it brings the app to the front
    pub foreground: bool,
    /// set for `UNTextInputNotificationAction`
    pub text_input: Option<TextInput>,
    /// the name of an icon from the shell's icon set, drawn with the action; empty for none (`UNNotificationActionIcon`)
    pub icon: String,
}

impl NotifAction {
    pub fn button(id: &str, title: &str) -> Self {
        NotifAction { id: id.into(), title: title.into(), destructive: false, foreground: false, text_input: None, icon: String::new() }
    }

    /// The same action with an icon (`UNNotificationActionIcon`).
    pub fn with_icon(mut self, name: &str) -> Self {
        self.icon = name.into();
        self
    }
}

/// `UNNotificationCategory`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotifCategory {
    pub id: String,
    pub actions: Vec<NotifAction>,
    /// `UNNotificationCategoryOptions` bits: `NOTIF_CATEGORY_CUSTOM_DISMISS` and the hidden-preview bits
    pub options: u8,
}

/// `UNNotificationRequest` with its content (`UNMutableNotificationContent`) and a time-interval trigger.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct NotifRequest {
    /// `identifier`: adding a request with an identifier already pending or delivered replaces it
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub body: String,
    /// `threadIdentifier`: notifications with the same thread stack together
    pub thread: String,
    /// `categoryIdentifier`; empty for none
    pub category: String,
    /// `UNTimeIntervalNotificationTrigger.timeInterval` in ms; 0 delivers now
    pub delay_ms: u32,
    /// `content.badge`: `Some(n)` sets the Dock badge to `n` (0 clears it), `None` leaves it
    pub badge: Option<u32>,
    /// `UNTimeIntervalNotificationTrigger(timeInterval:repeats:)`: fire every `delay_ms` until removed. Only with a delay.
    pub repeats: bool,
    /// `content.sound`: 0 none, 1 default, 2 named (`sound_name`), 3 default critical, 4 critical with `sound_volume`
    pub sound_kind: u8,
    /// the bundled sound file name for `sound_kind` 2
    pub sound_name: String,
    /// the audio volume of `sound_kind` 4, in percent
    pub sound_volume: u8,
    /// `content.userInfo`: string keys and values, returned with the response
    pub user_info: Vec<(String, String)>,
    /// `content.interruptionLevel`: 0 passive, 1 active (the default), 2 time sensitive, 3 critical
    pub interruption_level: u8,
    /// `content.relevanceScore` in thousandths, 0 to 1000
    pub relevance_milli: u16,
    /// `UNNotificationAttachment` images: absolute paths of PNG files in the app's data directory. The first one shows
    /// on the card as a thumbnail.
    pub attachments: Vec<String>,
}

/// `UNNotificationTrigger` of a scheduled request, sent with `ToShell::NotifySchedule`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifTrigger {
    /// `UNTimeIntervalNotificationTrigger`: after `seconds`, and every `seconds` when `repeats` (at least 60 then)
    Interval { seconds: u32, repeats: bool },
    /// `UNCalendarNotificationTrigger`: local `hour` (0..=23) and `minute` (0..=59) on the weekdays in `weekdays`, a bit
    /// set with bit 0 = Sunday .. bit 6 = Saturday (0 = every day); repeating or once
    Calendar { hour: u8, minute: u8, weekdays: u8, repeats: bool },
}

/// The longest interval, in seconds: its millisecond delay must fit a `u32` (about 49 days).
pub const MAX_NOTIF_INTERVAL_SECS: u32 = 4_294_967;
/// The shortest interval a repeating interval trigger may name, in seconds.
pub const MIN_NOTIF_REPEAT_SECS: u32 = 60;

/// The answer to `ToShell::NotifyRequest` (`add(_:withCompletionHandler:)`'s error).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifStatus {
    Ok = 0,
    /// the app has no `notifications` permission (`UNAuthorizationStatus.denied`)
    NotPermitted = 1,
    /// empty identifier, empty content, unknown category or oversized field (`UNErrorCodeContentProvidedInvalid`)
    Param = 2,
    /// too many pending requests
    Full = 3,
}

impl NotifStatus {
    pub fn from_wire(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Ok,
            1 => Self::NotPermitted,
            2 => Self::Param,
            3 => Self::Full,
            _ => return None,
        })
    }
}

/// `UNAuthorizationOptions` bits a `requestAuthorization` may carry: badge 1, sound 2, alert 4, critical alert 16,
/// provisional 64. Other bits make the message invalid.
pub const NOTIF_AUTH_MASK: u8 = NOTIF_AUTH_BADGE | NOTIF_AUTH_SOUND | NOTIF_AUTH_ALERT | NOTIF_AUTH_CRITICAL | NOTIF_AUTH_PROVISIONAL;
/// `UNAuthorizationOptions.badge`
pub const NOTIF_AUTH_BADGE: u8 = 1;
/// `UNAuthorizationOptions.sound`
pub const NOTIF_AUTH_SOUND: u8 = 2;
/// `UNAuthorizationOptions.alert`
pub const NOTIF_AUTH_ALERT: u8 = 4;
/// `UNAuthorizationOptions.criticalAlert` (needs the critical-alert grant, which the shell does not hand out)
pub const NOTIF_AUTH_CRITICAL: u8 = 16;
/// `UNAuthorizationOptions.provisional`: granted at once, delivered quietly, no prompt
pub const NOTIF_AUTH_PROVISIONAL: u8 = 64;

/// `UNNotificationPresentationOptions` bits a `willPresent` answer may carry: badge 1, sound 2, list 8, banner 16.
/// Other bits make the message invalid.
pub const NOTIF_PRESENT_MASK: u8 = NOTIF_PRESENT_BADGE | NOTIF_PRESENT_SOUND | NOTIF_PRESENT_LIST | NOTIF_PRESENT_BANNER;
/// `UNNotificationPresentationOptions.badge`
pub const NOTIF_PRESENT_BADGE: u8 = 1;
/// `UNNotificationPresentationOptions.sound`
pub const NOTIF_PRESENT_SOUND: u8 = 2;
/// `UNNotificationPresentationOptions.list` (Notification Center)
pub const NOTIF_PRESENT_LIST: u8 = 8;
/// `UNNotificationPresentationOptions.banner`
pub const NOTIF_PRESENT_BANNER: u8 = 16;

/// `UNNotificationCategoryOptions.customDismissAction`: the app hears `NOTIF_DISMISS_ACTION` when the user clears one of
/// its notifications of this category without opening it.
pub const NOTIF_CATEGORY_CUSTOM_DISMISS: u8 = 1;
/// `UNNotificationCategoryOptions.hiddenPreviewsShowTitle`: the title still shows when previews are hidden.
pub const NOTIF_CATEGORY_HIDDEN_SHOW_TITLE: u8 = 4;
/// `UNNotificationCategoryOptions.hiddenPreviewsShowSubtitle`: the subtitle still shows when previews are hidden.
pub const NOTIF_CATEGORY_HIDDEN_SHOW_SUBTITLE: u8 = 8;
/// Every category option bit the wire accepts.
pub const NOTIF_CATEGORY_MASK: u8 = NOTIF_CATEGORY_CUSTOM_DISMISS | NOTIF_CATEGORY_HIDDEN_SHOW_TITLE | NOTIF_CATEGORY_HIDDEN_SHOW_SUBTITLE;

/// What the user allowed for an app and whether Focus is on, as the settings query reports it. Sent as one byte of flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotifSettings {
    /// authorized (true) or denied (false). The manifest grant decides; there is no prompt.
    pub authorized: bool,
    /// banners, the lock screen and Notification Center rows (alert setting); off when the app is muted
    pub alert: bool,
    /// sound setting; follows the app's mute state
    pub sound: bool,
    /// badge setting; follows the app's mute state
    pub badge: bool,
    /// Do Not Disturb is on
    pub focus: bool,
}

impl NotifSettings {
    pub fn bits(self) -> u8 {
        (self.authorized as u8) | ((self.alert as u8) << 1) | ((self.sound as u8) << 2) | ((self.badge as u8) << 3) | ((self.focus as u8) << 4)
    }

    /// Unknown bits make the packet invalid.
    pub fn from_bits(v: u8) -> Option<Self> {
        if v & !0x1f != 0 {
            return None;
        }
        Some(NotifSettings { authorized: v & 1 != 0, alert: v & 2 != 0, sound: v & 4 != 0, badge: v & 8 != 0, focus: v & 16 != 0 })
    }
}

/// One thing to share or drag (`NativePasteboardWriting`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    Text(String),
    Url(String),
    /// a file the sender holds a document grant for (`ToApp::DocGranted`); the receiver gets its own grant
    File { grant: GrantId, name: String },
}

pub const MAX_ITEMS: usize = 8;
pub const MAX_PAYLOAD_TEXT: usize = 512;
pub const MAX_ITEM_TYPES: usize = 4;
pub const MAX_UTI: usize = 128;
pub const MAX_NAME: usize = 255;
pub const MAX_DRAG_TYPES: usize = 16;

impl Payload {
    /// The UTType the payload is on the pasteboard as (`public.utf8-plain-text`, `public.url`, or the file's own type).
    pub fn uti(&self) -> String {
        match self {
            Payload::Text(_) => "public.utf8-plain-text".into(),
            Payload::Url(_) => "public.url".into(),
            Payload::File { name, .. } => bifrost_ui::uttype::type_for_extension(name).unwrap_or("public.data").into(),
        }
    }
}

/// `NativePasteboardItem` of a dragging session: the types it is offered as and what it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DragItem {
    pub types: Vec<String>,
    pub payload: Payload,
}

impl DragItem {
    /// An item offered as its natural type.
    pub fn new(payload: Payload) -> Self {
        DragItem { types: vec![payload.uti()], payload }
    }
}

/// Bits of a drag operation mask (`NativeDragOperation`): what a source allows, and the one a destination chose (0 = none).
pub mod dragop {
    pub const COPY: u8 = 1;
    pub const LINK: u8 = 2;
    pub const MOVE: u8 = 4;
    pub const ALL: u8 = COPY | LINK | MOVE;
}

/// A drag image (`NativeDraggingItem.imageComponents`) as straight RGBA, at most `MAX_PREVIEW` pixels per side.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DragPreview {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

pub const MAX_PREVIEW: u32 = 32;

impl DragPreview {
    pub fn valid(&self) -> bool {
        self.w <= MAX_PREVIEW && self.h <= MAX_PREVIEW && self.rgba.len() == (self.w * self.h * 4) as usize
    }
}

/// Which `NativeDraggingDestination` call a `ToApp::DragDest` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragPhase {
    /// `draggingEntered:`
    Entered = 0,
    /// `draggingUpdated:`
    Updated = 1,
    /// `draggingExited:`
    Exited = 2,
    /// `performDragOperation:` (after `prepareForDragOperation:`); answer with `ToShell::DragPerformed`
    Perform = 3,
}

impl DragPhase {
    pub fn from_wire(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Entered,
            1 => Self::Updated,
            2 => Self::Exited,
            3 => Self::Perform,
            _ => return None,
        })
    }
}

// ---- codec helpers -------------------------------------------------------------------------------------------

fn put(b: &mut Vec<u8>, s: &str, max: usize) {
    let cut: String = s.chars().take(max).collect();
    str_(b, &cut);
}

fn get(r: &mut Reader, max: usize) -> Result<String, DecodeError> {
    let s = r.str()?;
    if s.chars().count() > max {
        return Err(DecodeError::BadValue);
    }
    Ok(s)
}

/// A system-service name or method name: at most this many characters.
pub const MAX_SERVICE_NAME: usize = 64;
/// A system-service JSON body (call, reply or event): at most this many characters.
pub const MAX_SERVICE_BODY: usize = crate::MAX_PACKET - 512;

/// The body as sent: a body over `MAX_SERVICE_BODY` bytes is replaced by `instead`, never cut (a cut JSON body
/// would arrive as a valid-looking, truncated reply).
fn fit_body<'a>(body: &'a str, instead: &'a str) -> &'a str {
    if body.len() > MAX_SERVICE_BODY {
        instead
    } else {
        body
    }
}
pub const MAX_DIST_INFO: usize = 32;
pub const MAX_DIST_VALUE: usize = 1024;

/// The `userInfo` pairs of a request, bounded like the request's own (`put_request`).
fn put_user_info(b: &mut Vec<u8>, info: &[(String, String)]) {
    let n = info.len().min(MAX_NOTIF_USER_INFO);
    u32_(b, n as u32);
    for (k, v) in &info[..n] {
        put(b, k, MAX_NOTIF_ID);
        put(b, v, MAX_NOTIF_USER_INFO_VALUE);
    }
}

fn get_user_info(r: &mut Reader) -> Result<Vec<(String, String)>, DecodeError> {
    let n = count(r, MAX_NOTIF_USER_INFO)?;
    let mut info = Vec::with_capacity(n);
    for _ in 0..n {
        info.push((get(r, MAX_NOTIF_ID)?, get(r, MAX_NOTIF_USER_INFO_VALUE)?));
    }
    Ok(info)
}

fn put_dist(b: &mut Vec<u8>, name: &str, info: &[(String, String)]) {
    put(b, name, MAX_NOTIF_ID);
    let n = info.len().min(MAX_DIST_INFO);
    u32_(b, n as u32);
    for (k, v) in &info[..n] {
        put(b, k, MAX_NOTIF_ID);
        put(b, v, MAX_DIST_VALUE);
    }
}

fn get_dist(r: &mut Reader) -> Result<(String, Vec<(String, String)>), DecodeError> {
    let name = get(r, MAX_NOTIF_ID)?;
    let n = count(r, MAX_DIST_INFO)?;
    let mut info = Vec::with_capacity(n);
    for _ in 0..n {
        info.push((get(r, MAX_NOTIF_ID)?, get(r, MAX_DIST_VALUE * 4)?));
    }
    Ok((name, info))
}

fn count(r: &mut Reader, max: usize) -> Result<usize, DecodeError> {
    let n = r.u32()? as usize;
    if n > max {
        return Err(DecodeError::BadValue);
    }
    Ok(n)
}

fn flag(r: &mut Reader) -> Result<bool, DecodeError> {
    match r.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(DecodeError::BadValue),
    }
}

fn put_ids(b: &mut Vec<u8>, ids: &[String]) {
    let n = ids.len().min(MAX_NOTIF_IDS);
    u32_(b, n as u32);
    for id in &ids[..n] {
        put(b, id, MAX_NOTIF_ID);
    }
}

fn get_ids(r: &mut Reader) -> Result<Vec<String>, DecodeError> {
    let n = count(r, MAX_NOTIF_IDS)?;
    (0..n).map(|_| get(r, MAX_NOTIF_ID)).collect()
}

fn put_categories(b: &mut Vec<u8>, list: &[NotifCategory]) {
    let n = list.len().min(MAX_NOTIF_CATEGORIES);
    u32_(b, n as u32);
    for c in &list[..n] {
        put(b, &c.id, MAX_NOTIF_ID);
        b.push(c.options);
        let m = c.actions.len().min(MAX_NOTIF_ACTIONS);
        u32_(b, m as u32);
        for a in &c.actions[..m] {
            put(b, &a.id, MAX_NOTIF_ID);
            put(b, &a.title, MAX_NOTIF_TEXT);
            put(b, &a.icon, MAX_NOTIF_ID);
            b.push(a.destructive as u8 | (a.foreground as u8) << 1 | (a.text_input.is_some() as u8) << 2);
            if let Some(t) = &a.text_input {
                put(b, &t.button_title, MAX_NOTIF_TEXT);
                put(b, &t.placeholder, MAX_NOTIF_TEXT);
            }
        }
    }
}

fn get_categories(r: &mut Reader) -> Result<Vec<NotifCategory>, DecodeError> {
    let n = count(r, MAX_NOTIF_CATEGORIES)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let id = get(r, MAX_NOTIF_ID)?;
        let options = r.u8()?;
        if options & !NOTIF_CATEGORY_MASK != 0 {
            return Err(DecodeError::BadValue);
        }
        let m = count(r, MAX_NOTIF_ACTIONS)?;
        let mut actions = Vec::with_capacity(m);
        for _ in 0..m {
            let aid = get(r, MAX_NOTIF_ID)?;
            let title = get(r, MAX_NOTIF_TEXT)?;
            let icon = get(r, MAX_NOTIF_ID)?;
            let f = r.u8()?;
            if f > 7 {
                return Err(DecodeError::BadValue);
            }
            let text_input = if f & 4 != 0 { Some(TextInput { button_title: get(r, MAX_NOTIF_TEXT)?, placeholder: get(r, MAX_NOTIF_TEXT)? }) } else { None };
            actions.push(NotifAction { id: aid, title, destructive: f & 1 != 0, foreground: f & 2 != 0, text_input, icon });
        }
        out.push(NotifCategory { id, actions, options });
    }
    Ok(out)
}

fn put_request(b: &mut Vec<u8>, q: &NotifRequest) {
    put(b, &q.id, MAX_NOTIF_ID);
    put(b, &q.title, MAX_NOTIF_TEXT);
    put(b, &q.subtitle, MAX_NOTIF_TEXT);
    put(b, &q.body, MAX_NOTIF_TEXT * 4);
    put(b, &q.thread, MAX_NOTIF_ID);
    put(b, &q.category, MAX_NOTIF_ID);
    u32_(b, q.delay_ms);
    match q.badge {
        Some(n) => {
            b.push(1);
            u32_(b, n);
        }
        None => b.push(0),
    }
    b.push(u8::from(q.repeats));
    b.push(q.sound_kind);
    put(b, &q.sound_name, MAX_NOTIF_ID);
    b.push(q.sound_volume);
    let n = q.user_info.len().min(MAX_NOTIF_USER_INFO);
    u32_(b, n as u32);
    for (k, v) in &q.user_info[..n] {
        put(b, k, MAX_NOTIF_ID);
        put(b, v, MAX_NOTIF_USER_INFO_VALUE);
    }
    b.push(q.interruption_level);
    b.extend_from_slice(&q.relevance_milli.to_le_bytes());
    let n = q.attachments.len().min(MAX_NOTIF_ATTACHMENTS);
    u32_(b, n as u32);
    for path in &q.attachments[..n] {
        put(b, path, MAX_NOTIF_ATTACHMENT_PATH);
    }
}

fn get_request(r: &mut Reader) -> Result<NotifRequest, DecodeError> {
    let id = get(r, MAX_NOTIF_ID)?;
    let title = get(r, MAX_NOTIF_TEXT)?;
    let subtitle = get(r, MAX_NOTIF_TEXT)?;
    let body = get(r, MAX_NOTIF_TEXT * 4)?;
    let thread = get(r, MAX_NOTIF_ID)?;
    let category = get(r, MAX_NOTIF_ID)?;
    let delay_ms = r.u32()?;
    let badge = if flag(r)? { Some(r.u32()?) } else { None };
    let repeats = flag(r)?;
    let sound_kind = r.u8()?;
    let sound_name = get(r, MAX_NOTIF_ID)?;
    let sound_volume = r.u8()?;
    if sound_kind > 4 || sound_volume > 100 {
        return Err(DecodeError::BadValue);
    }
    let n = count(r, MAX_NOTIF_USER_INFO)?;
    let mut user_info = Vec::with_capacity(n);
    for _ in 0..n {
        user_info.push((get(r, MAX_NOTIF_ID)?, get(r, MAX_NOTIF_USER_INFO_VALUE)?));
    }
    let interruption_level = r.u8()?;
    let relevance_milli = u16::from_le_bytes([r.u8()?, r.u8()?]);
    let n = count(r, MAX_NOTIF_ATTACHMENTS)?;
    let mut attachments = Vec::with_capacity(n);
    for _ in 0..n {
        attachments.push(get(r, MAX_NOTIF_ATTACHMENT_PATH)?);
    }
    if interruption_level > 3 || relevance_milli > 1000 {
        return Err(DecodeError::BadValue);
    }
    Ok(NotifRequest { id, title, subtitle, body, thread, category, delay_ms, badge, repeats, sound_kind, sound_name, sound_volume, user_info, interruption_level, relevance_milli, attachments })
}

/// A trigger: kind 0 is an interval (seconds u32, repeats), kind 1 a calendar (hour, minute, weekdays, repeats).
fn put_trigger(b: &mut Vec<u8>, t: &NotifTrigger) {
    match *t {
        NotifTrigger::Interval { seconds, repeats } => {
            b.push(0);
            u32_(b, seconds);
            b.push(u8::from(repeats));
        }
        NotifTrigger::Calendar { hour, minute, weekdays, repeats } => {
            b.push(1);
            b.extend_from_slice(&[hour, minute, weekdays, u8::from(repeats)]);
        }
    }
}

fn get_trigger(r: &mut Reader) -> Result<NotifTrigger, DecodeError> {
    match r.u8()? {
        0 => {
            let seconds = r.u32()?;
            let repeats = flag(r)?;
            if !(1..=MAX_NOTIF_INTERVAL_SECS).contains(&seconds) || (repeats && seconds < MIN_NOTIF_REPEAT_SECS) {
                return Err(DecodeError::BadValue);
            }
            Ok(NotifTrigger::Interval { seconds, repeats })
        }
        1 => {
            let hour = r.u8()?;
            let minute = r.u8()?;
            let weekdays = r.u8()?;
            let repeats = flag(r)?;
            if hour > 23 || minute > 59 || weekdays & !0x7f != 0 {
                return Err(DecodeError::BadValue);
            }
            Ok(NotifTrigger::Calendar { hour, minute, weekdays, repeats })
        }
        _ => Err(DecodeError::BadValue),
    }
}

fn put_payload(b: &mut Vec<u8>, p: &Payload) {
    match p {
        Payload::Text(s) => {
            b.push(0);
            put(b, s, MAX_PAYLOAD_TEXT);
        }
        Payload::Url(s) => {
            b.push(1);
            put(b, s, MAX_PAYLOAD_TEXT);
        }
        Payload::File { grant, name } => {
            b.push(2);
            b.extend_from_slice(grant);
            put(b, name, MAX_NAME);
        }
    }
}

fn get_payload(r: &mut Reader) -> Result<Payload, DecodeError> {
    Ok(match r.u8()? {
        0 => Payload::Text(get(r, MAX_PAYLOAD_TEXT)?),
        1 => Payload::Url(get(r, MAX_PAYLOAD_TEXT)?),
        2 => Payload::File { grant: r.grant()?, name: get(r, MAX_NAME)? },
        _ => return Err(DecodeError::BadValue),
    })
}

fn put_payloads(b: &mut Vec<u8>, list: &[Payload]) {
    let n = list.len().min(MAX_ITEMS);
    u32_(b, n as u32);
    for p in &list[..n] {
        put_payload(b, p);
    }
}

fn get_payloads(r: &mut Reader) -> Result<Vec<Payload>, DecodeError> {
    let n = count(r, MAX_ITEMS)?;
    (0..n).map(|_| get_payload(r)).collect()
}

fn put_types(b: &mut Vec<u8>, types: &[String], max: usize) {
    let kept: Vec<&String> = types.iter().filter(|t| !t.is_empty() && t.len() <= MAX_UTI).take(max).collect();
    u32_(b, kept.len() as u32);
    for t in kept {
        str_(b, t);
    }
}

fn get_types(r: &mut Reader, max: usize) -> Result<Vec<String>, DecodeError> {
    let n = count(r, max)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let t = r.str()?;
        if t.is_empty() || t.len() > MAX_UTI {
            return Err(DecodeError::BadValue);
        }
        out.push(t);
    }
    Ok(out)
}

fn put_items(b: &mut Vec<u8>, list: &[DragItem]) {
    let n = list.len().min(MAX_ITEMS);
    u32_(b, n as u32);
    for i in &list[..n] {
        put_types(b, &i.types, MAX_ITEM_TYPES);
        put_payload(b, &i.payload);
    }
}

fn get_items(r: &mut Reader) -> Result<Vec<DragItem>, DecodeError> {
    let n = count(r, MAX_ITEMS)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(DragItem { types: get_types(r, MAX_ITEM_TYPES)?, payload: get_payload(r)? });
    }
    Ok(out)
}

fn finite(v: f32) -> Result<f32, DecodeError> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(DecodeError::BadValue)
    }
}

// ---- messages ------------------------------------------------------------------------------------------------

pub(crate) mod tag {
    pub const NOTIFICATION_ACTION: u32 = 50;
    pub const NOTIFICATION_STATUS: u32 = 51;
    pub const NOTIFICATION_LIST: u32 = 52;
    pub const NOTIFICATION_SETTINGS: u32 = 56;
    pub const NOTIFICATION_AUTHORIZED: u32 = 57;
    pub const NOTIFICATION_WILL_PRESENT: u32 = 58;
    pub const SHARE_DONE: u32 = 53;
    pub const DRAG_DEST: u32 = 54;
    pub const DRAG_ENDED: u32 = 55;

    pub const NOTIFY_CATEGORIES: u32 = 0x140;
    pub const NOTIFY_REQUEST: u32 = 0x141;
    pub const NOTIFY_REMOVE: u32 = 0x142;
    pub const NOTIFY_QUERY: u32 = 0x143;
    pub const NOTIFY_SETTINGS: u32 = 0x14a;
    pub const NOTIFY_AUTHORIZE: u32 = 0x14b;
    pub const NOTIFY_PRESENT_REPLY: u32 = 0x14c;
    pub const NOTIFY_PRESENT_DELEGATE: u32 = 0x14d;
    pub const NOTIFY_SCHEDULE: u32 = 0x14e;
    pub const SHARE: u32 = 0x144;
    pub const DRAG_BEGIN: u32 = 0x145;
    pub const DRAG_REGISTER: u32 = 0x146;
    pub const DRAG_REPLY: u32 = 0x147;
    pub const DRAG_PERFORMED: u32 = 0x148;
    pub const DRAG_CANCEL: u32 = 0x149;
    pub const DIST_POST: u32 = 0x1d0;
    pub const DIST_NOTIFY: u32 = 0x1d1;
    pub const SERVICE_CALL: u32 = 0x1d2;
    pub const SERVICE_REPLY: u32 = 0x1d2;
    pub const SERVICE_EVENT: u32 = 0x1d3;
    pub const SELECTION_REQUEST: u32 = 0x1d4;
    pub const SELECTION_REPLY: u32 = 0x1d4;
}

/// Encodes the variants of `ToApp` that belong to this module (anything else writes nothing).
pub(crate) fn encode_app(m: &ToApp, b: &mut Vec<u8>) {
    match m {
        ToApp::NotificationAction { id, action, text, thread, category, user_info } => {
            u32_(b, tag::NOTIFICATION_ACTION);
            put(b, id, MAX_NOTIF_ID);
            put(b, action, MAX_NOTIF_ID);
            put(b, text, MAX_NOTIF_TEXT * 4);
            put(b, thread, MAX_NOTIF_ID);
            put(b, category, MAX_NOTIF_ID);
            put_user_info(b, user_info);
        }
        ToApp::NotificationStatus { request, status } => {
            u32_(b, tag::NOTIFICATION_STATUS);
            u32_(b, *request);
            b.push(*status as u8);
        }
        ToApp::NotificationList { request, delivered, ids } => {
            u32_(b, tag::NOTIFICATION_LIST);
            u32_(b, *request);
            b.push(*delivered as u8);
            put_ids(b, ids);
        }
        ToApp::NotificationSettings { request, settings } => {
            u32_(b, tag::NOTIFICATION_SETTINGS);
            u32_(b, *request);
            b.push(settings.bits());
        }
        ToApp::NotificationAuthorized { request, granted } => {
            u32_(b, tag::NOTIFICATION_AUTHORIZED);
            u32_(b, *request);
            b.push(*granted as u8);
        }
        ToApp::ShareDone { request, target } => {
            u32_(b, tag::SHARE_DONE);
            u32_(b, *request);
            put(b, target, MAX_NOTIF_ID);
        }
        ToApp::NotificationWillPresent { token, id, thread, category, title, body } => {
            u32_(b, tag::NOTIFICATION_WILL_PRESENT);
            u32_(b, *token);
            put(b, id, MAX_NOTIF_ID);
            put(b, thread, MAX_NOTIF_ID);
            put(b, category, MAX_NOTIF_ID);
            put(b, title, MAX_NOTIF_TEXT);
            put(b, body, MAX_NOTIF_TEXT);
        }
        ToApp::DistNotify { name, user_info } => {
            u32_(b, tag::DIST_NOTIFY);
            put_dist(b, name, user_info);
        }
        ToApp::ServiceReply { request, ok, body } => {
            u32_(b, tag::SERVICE_REPLY);
            u32_(b, *request);
            let too_big = body.len() > MAX_SERVICE_BODY;
            b.push((*ok && !too_big) as u8);
            put(b, if too_big { "body too large" } else { body }, MAX_SERVICE_BODY);
        }
        ToApp::ServiceEvent { service, event, body } => {
            u32_(b, tag::SERVICE_EVENT);
            put(b, service, MAX_SERVICE_NAME);
            put(b, event, MAX_SERVICE_NAME);
            put(b, fit_body(body, "{\"error\":\"body too large\"}"), MAX_SERVICE_BODY);
        }
        ToApp::DragDest { session, phase, x, y, allowed, items } => {
            u32_(b, tag::DRAG_DEST);
            u32_(b, *session);
            b.push(*phase as u8);
            f32_(b, *x);
            f32_(b, *y);
            b.push(*allowed);
            put_items(b, items);
        }
        ToApp::SelectionRequest { request } => {
            u32_(b, tag::SELECTION_REQUEST);
            u32_(b, *request);
        }
        ToApp::DragEnded { session, op } => {
            u32_(b, tag::DRAG_ENDED);
            u32_(b, *session);
            b.push(*op);
        }
        _ => {}
    }
}

pub(crate) fn decode_app(t: u32, r: &mut Reader) -> Result<ToApp, DecodeError> {
    Ok(match t {
        tag::NOTIFICATION_ACTION => ToApp::NotificationAction {
            id: get(r, MAX_NOTIF_ID)?,
            action: get(r, MAX_NOTIF_ID)?,
            text: get(r, MAX_NOTIF_TEXT * 4)?,
            thread: get(r, MAX_NOTIF_ID)?,
            category: get(r, MAX_NOTIF_ID)?,
            user_info: get_user_info(r)?,
        },
        tag::NOTIFICATION_STATUS => ToApp::NotificationStatus { request: r.u32()?, status: NotifStatus::from_wire(r.u8()?).ok_or(DecodeError::BadValue)? },
        tag::NOTIFICATION_LIST => ToApp::NotificationList { request: r.u32()?, delivered: flag(r)?, ids: get_ids(r)? },
        tag::NOTIFICATION_SETTINGS => ToApp::NotificationSettings { request: r.u32()?, settings: NotifSettings::from_bits(r.u8()?).ok_or(DecodeError::BadValue)? },
        tag::NOTIFICATION_AUTHORIZED => ToApp::NotificationAuthorized { request: r.u32()?, granted: flag(r)? },
        tag::NOTIFICATION_WILL_PRESENT => ToApp::NotificationWillPresent {
            token: r.u32()?,
            id: get(r, MAX_NOTIF_ID)?,
            thread: get(r, MAX_NOTIF_ID)?,
            category: get(r, MAX_NOTIF_ID)?,
            title: get(r, MAX_NOTIF_TEXT)?,
            body: get(r, MAX_NOTIF_TEXT)?,
        },
        tag::DIST_NOTIFY => {
            let (name, user_info) = get_dist(r)?;
            ToApp::DistNotify { name, user_info }
        }
        tag::SERVICE_REPLY => ToApp::ServiceReply { request: r.u32()?, ok: flag(r)?, body: get(r, MAX_SERVICE_BODY)? },
        tag::SERVICE_EVENT => ToApp::ServiceEvent {
            service: get(r, MAX_SERVICE_NAME)?,
            event: get(r, MAX_SERVICE_NAME)?,
            body: get(r, MAX_SERVICE_BODY)?,
        },
        tag::SELECTION_REQUEST => ToApp::SelectionRequest { request: r.u32()? },
        tag::SHARE_DONE => ToApp::ShareDone { request: r.u32()?, target: get(r, MAX_NOTIF_ID)? },
        tag::DRAG_DEST => {
            let session = r.u32()?;
            let phase = DragPhase::from_wire(r.u8()?).ok_or(DecodeError::BadValue)?;
            let x = finite(r.f32()?)?;
            let y = finite(r.f32()?)?;
            let allowed = r.u8()?;
            if allowed & !dragop::ALL != 0 {
                return Err(DecodeError::BadValue);
            }
            ToApp::DragDest { session, phase, x, y, allowed, items: get_items(r)? }
        }
        tag::DRAG_ENDED => {
            let session = r.u32()?;
            let op = r.u8()?;
            if op & !dragop::ALL != 0 {
                return Err(DecodeError::BadValue);
            }
            ToApp::DragEnded { session, op }
        }
        t => return Err(DecodeError::UnknownTag(t)),
    })
}

pub(crate) fn encode_shell(m: &ToShell, b: &mut Vec<u8>) {
    match m {
        ToShell::NotifyCategories(list) => {
            u32_(b, tag::NOTIFY_CATEGORIES);
            put_categories(b, list);
        }
        ToShell::NotifyRequest { request, req } => {
            u32_(b, tag::NOTIFY_REQUEST);
            u32_(b, *request);
            put_request(b, req);
        }
        ToShell::NotifyRemove { delivered, ids } => {
            u32_(b, tag::NOTIFY_REMOVE);
            b.push(*delivered as u8);
            put_ids(b, ids);
        }
        ToShell::NotifyQuery { request, delivered } => {
            u32_(b, tag::NOTIFY_QUERY);
            u32_(b, *request);
            b.push(*delivered as u8);
        }
        ToShell::NotifySettings { request } => {
            u32_(b, tag::NOTIFY_SETTINGS);
            u32_(b, *request);
        }
        ToShell::NotifyAuthorize { request, options } => {
            u32_(b, tag::NOTIFY_AUTHORIZE);
            u32_(b, *request);
            b.push(*options);
        }
        ToShell::NotifyPresentDelegate { enabled } => {
            u32_(b, tag::NOTIFY_PRESENT_DELEGATE);
            b.push(*enabled as u8);
        }
        ToShell::NotifyPresentReply { token, options } => {
            u32_(b, tag::NOTIFY_PRESENT_REPLY);
            u32_(b, *token);
            b.push(*options);
        }
        ToShell::NotifySchedule { request, req, trigger } => {
            u32_(b, tag::NOTIFY_SCHEDULE);
            u32_(b, *request);
            put_request(b, req);
            put_trigger(b, trigger);
        }
        ToShell::DistPost { name, user_info } => {
            u32_(b, tag::DIST_POST);
            put_dist(b, name, user_info);
        }
        ToShell::ServiceCall { request, service, method, body } => {
            u32_(b, tag::SERVICE_CALL);
            u32_(b, *request);
            put(b, service, MAX_SERVICE_NAME);
            put(b, method, MAX_SERVICE_NAME);
            put(b, fit_body(body, "null"), MAX_SERVICE_BODY);
        }
        ToShell::SelectionReply { request, text } => {
            u32_(b, tag::SELECTION_REPLY);
            u32_(b, *request);
            put(b, text, crate::MAX_SURROUNDING);
        }
        ToShell::Share { request, items, anchor } => {
            u32_(b, tag::SHARE);
            u32_(b, *request);
            f32_(b, anchor.x);
            f32_(b, anchor.y);
            f32_(b, anchor.w);
            f32_(b, anchor.h);
            put_payloads(b, items);
        }
        ToShell::DragBegin { session, allowed, items, preview } => {
            u32_(b, tag::DRAG_BEGIN);
            u32_(b, *session);
            b.push(*allowed);
            put_items(b, items);
            let (w, h, px) = if preview.valid() { (preview.w, preview.h, &preview.rgba[..]) } else { (0, 0, &[][..]) };
            u32_(b, w);
            u32_(b, h);
            b.extend_from_slice(px);
        }
        ToShell::DragRegister { types } => {
            u32_(b, tag::DRAG_REGISTER);
            put_types(b, types, MAX_DRAG_TYPES);
        }
        ToShell::DragReply { session, op } => {
            u32_(b, tag::DRAG_REPLY);
            u32_(b, *session);
            b.push(*op);
        }
        ToShell::DragPerformed { session, accepted } => {
            u32_(b, tag::DRAG_PERFORMED);
            u32_(b, *session);
            b.push(*accepted as u8);
        }
        ToShell::DragCancel { session } => {
            u32_(b, tag::DRAG_CANCEL);
            u32_(b, *session);
        }
        _ => {}
    }
}

pub(crate) fn decode_shell(t: u32, r: &mut Reader) -> Result<ToShell, DecodeError> {
    Ok(match t {
        tag::NOTIFY_CATEGORIES => ToShell::NotifyCategories(get_categories(r)?),
        tag::NOTIFY_REQUEST => ToShell::NotifyRequest { request: r.u32()?, req: get_request(r)? },
        tag::NOTIFY_REMOVE => ToShell::NotifyRemove { delivered: flag(r)?, ids: get_ids(r)? },
        tag::NOTIFY_QUERY => ToShell::NotifyQuery { request: r.u32()?, delivered: flag(r)? },
        tag::NOTIFY_SETTINGS => ToShell::NotifySettings { request: r.u32()? },
        tag::NOTIFY_AUTHORIZE => {
            let (request, options) = (r.u32()?, r.u8()?);
            if options & !NOTIF_AUTH_MASK != 0 {
                return Err(DecodeError::BadValue);
            }
            ToShell::NotifyAuthorize { request, options }
        }
        tag::NOTIFY_PRESENT_DELEGATE => ToShell::NotifyPresentDelegate { enabled: flag(r)? },
        tag::NOTIFY_PRESENT_REPLY => {
            let (token, options) = (r.u32()?, r.u8()?);
            if options & !NOTIF_PRESENT_MASK != 0 {
                return Err(DecodeError::BadValue);
            }
            ToShell::NotifyPresentReply { token, options }
        }
        tag::NOTIFY_SCHEDULE => {
            let request = r.u32()?;
            let req = get_request(r)?;
            let trigger = get_trigger(r)?;
            ToShell::NotifySchedule { request, req, trigger }
        }
        tag::DIST_POST => {
            let (name, user_info) = get_dist(r)?;
            ToShell::DistPost { name, user_info }
        }
        tag::SERVICE_CALL => ToShell::ServiceCall {
            request: r.u32()?,
            service: get(r, MAX_SERVICE_NAME)?,
            method: get(r, MAX_SERVICE_NAME)?,
            body: get(r, MAX_SERVICE_BODY)?,
        },
        tag::SELECTION_REPLY => ToShell::SelectionReply { request: r.u32()?, text: get(r, crate::MAX_SURROUNDING)? },
        tag::SHARE => {
            let request = r.u32()?;
            let anchor = UiRect { x: finite(r.f32()?)?, y: finite(r.f32()?)?, w: finite(r.f32()?)?, h: finite(r.f32()?)? };
            ToShell::Share { request, anchor, items: get_payloads(r)? }
        }
        tag::DRAG_BEGIN => {
            let session = r.u32()?;
            let allowed = r.u8()?;
            if allowed & !dragop::ALL != 0 {
                return Err(DecodeError::BadValue);
            }
            let items = get_items(r)?;
            let (w, h) = (r.u32()?, r.u32()?);
            if w > MAX_PREVIEW || h > MAX_PREVIEW {
                return Err(DecodeError::BadValue);
            }
            let rgba = r.take((w * h * 4) as usize)?.to_vec();
            ToShell::DragBegin { session, allowed, items, preview: DragPreview { w, h, rgba } }
        }
        tag::DRAG_REGISTER => ToShell::DragRegister { types: get_types(r, MAX_DRAG_TYPES)? },
        tag::DRAG_REPLY => {
            let session = r.u32()?;
            let op = r.u8()?;
            if op & !dragop::ALL != 0 || op.count_ones() > 1 {
                return Err(DecodeError::BadValue);
            }
            ToShell::DragReply { session, op }
        }
        tag::DRAG_PERFORMED => ToShell::DragPerformed { session: r.u32()?, accepted: flag(r)? },
        tag::DRAG_CANCEL => ToShell::DragCancel { session: r.u32()? },
        t => return Err(DecodeError::UnknownTag(t)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_rt(m: ToApp) {
        assert_eq!(ToApp::decode(&m.encode()), Ok(m));
    }
    fn shell_rt(m: ToShell) {
        assert_eq!(ToShell::decode(&m.encode()), Ok(m));
    }

    fn category() -> NotifCategory {
        NotifCategory {
            options: 0,
            id: "MESSAGE".into(),
            actions: vec![
                NotifAction::button("open", "Open"),
                NotifAction { destructive: true, ..NotifAction::button("delete", "Delete") },
                NotifAction { id: "reply".into(), title: "Reply".into(), destructive: false, foreground: true, text_input: Some(TextInput { button_title: "Send".into(), placeholder: "Message".into() }), icon: "reply".into() },
            ],
        }
    }

    #[test]
    fn notification_messages_round_trip() {
        shell_rt(ToShell::NotifyCategories(vec![category(), NotifCategory { options: 0, id: "EMPTY".into(), actions: vec![] }]));
        shell_rt(ToShell::NotifyRequest {
            request: 4,
            req: NotifRequest { id: "n1".into(), title: "Hi".into(), subtitle: "sub".into(), body: "b ü".into(), thread: "t".into(), category: "MESSAGE".into(), delay_ms: 1500, badge: Some(3), repeats: true, sound_kind: 4, sound_name: String::new(), sound_volume: 60, user_info: vec![("room".into(), "ü 7".into())], interruption_level: 2, relevance_milli: 750, attachments: vec!["/home/app/a.png".into()] },
        });
        shell_rt(ToShell::NotifyRequest { request: 6, req: NotifRequest { id: "n3".into(), sound_kind: 2, sound_name: "chime.wav".into(), ..Default::default() } });
        shell_rt(ToShell::NotifyRequest { request: 5, req: NotifRequest { id: "n2".into(), ..Default::default() } });
        shell_rt(ToShell::NotifyRemove { delivered: true, ids: vec!["a".into(), "b".into()] });
        shell_rt(ToShell::NotifyRemove { delivered: false, ids: vec![] });
        shell_rt(ToShell::NotifyQuery { request: 9, delivered: true });
        app_rt(ToApp::NotificationAction { id: "n1".into(), action: "reply".into(), text: "ok".into(), thread: "t".into(), category: "MESSAGE".into(), user_info: vec![("room".into(), "7 ü".into())] });
        app_rt(ToApp::NotificationAction { id: "n2".into(), action: NOTIF_DEFAULT_ACTION.into(), text: String::new(), thread: String::new(), category: String::new(), user_info: vec![] });
        app_rt(ToApp::NotificationStatus { request: 4, status: NotifStatus::Param });
        app_rt(ToApp::NotificationList { request: 9, delivered: false, ids: vec!["x".into()] });
    }

    #[test]
    fn notification_settings_round_trip_every_flag() {
        shell_rt(ToShell::NotifySettings { request: 12 });
        for bits in 0..32u8 {
            let settings = NotifSettings::from_bits(bits).unwrap();
            assert_eq!(settings.bits(), bits);
            app_rt(ToApp::NotificationSettings { request: bits as u32, settings });
        }
        let denied = NotifSettings { focus: true, ..Default::default() };
        assert_eq!(denied.bits(), 16);
    }

    #[test]
    fn notification_settings_reject_unknown_flag_bits() {
        assert_eq!(NotifSettings::from_bits(0x20), None);
        assert_eq!(NotifSettings::from_bits(0xff), None);
        let mut bytes = ToApp::NotificationSettings { request: 1, settings: NotifSettings { authorized: true, ..Default::default() } }.encode();
        let last = bytes.len() - 1;
        bytes[last] = 0x20;
        assert_eq!(ToApp::decode(&bytes), Err(DecodeError::BadValue));
    }

    #[test]
    fn notification_will_present_round_trips() {
        app_rt(ToApp::NotificationWillPresent {
            token: 7,
            id: "note-1".into(),
            thread: "t".into(),
            category: "chat".into(),
            title: "Hi".into(),
            body: "There".into(),
        });
        app_rt(ToApp::NotificationWillPresent { token: 0, id: String::new(), thread: String::new(), category: String::new(), title: String::new(), body: String::new() });
        shell_rt(ToShell::NotifyPresentDelegate { enabled: true });
        shell_rt(ToShell::NotifyPresentDelegate { enabled: false });
        for options in [0u8, 1, 2, 8, 16, NOTIF_PRESENT_MASK] {
            shell_rt(ToShell::NotifyPresentReply { token: 9, options });
        }
    }

    #[test]
    fn notification_category_options_round_trip_and_reject_unknown_bits() {
        let custom = NotifCategory { options: NOTIF_CATEGORY_CUSTOM_DISMISS, ..category() };
        shell_rt(ToShell::NotifyCategories(vec![custom]));
        let hidden = NotifCategory { options: NOTIF_CATEGORY_HIDDEN_SHOW_TITLE | NOTIF_CATEGORY_HIDDEN_SHOW_SUBTITLE, ..category() };
        shell_rt(ToShell::NotifyCategories(vec![hidden]));
        assert_eq!(NOTIF_CATEGORY_MASK, 13);
        // tag 4, count 4, id length 4 and "c" 1 bytes come first, so the options byte is at index 13
        let mut bytes = ToShell::NotifyCategories(vec![NotifCategory { id: "c".into(), actions: vec![], options: 0 }]).encode();
        assert_eq!(bytes[13], 0);
        bytes[13] = 0x80;
        assert_eq!(ToShell::decode(&bytes), Err(DecodeError::BadValue));
    }

    #[test]
    fn notification_action_icons_round_trip() {
        let icons = NotifCategory { id: "i".into(), actions: vec![NotifAction::button("a", "A").with_icon("trash"), NotifAction::button("b", "B")], options: 0 };
        shell_rt(ToShell::NotifyCategories(vec![icons.clone()]));
        assert_eq!(icons.actions[0].icon, "trash");
        assert!(icons.actions[1].icon.is_empty());
    }

    #[test]
    fn notification_attachments_are_bounded() {
        let mut bytes = ToShell::NotifyRequest { request: 1, req: NotifRequest { id: "a".into(), ..Default::default() } }.encode();
        // the attachment count is the last u32 of an empty request
        let at = bytes.len() - 4;
        bytes[at..].copy_from_slice(&(MAX_NOTIF_ATTACHMENTS as u32 + 1).to_le_bytes());
        assert_eq!(ToShell::decode(&bytes), Err(DecodeError::BadValue));
        let many = NotifRequest { attachments: vec!["/a.png".into(); 3], ..Default::default() };
        shell_rt(ToShell::NotifyRequest { request: 2, req: many });
    }

    #[test]
    fn notification_will_present_rejects_unknown_option_bits() {
        let mut bytes = ToShell::NotifyPresentReply { token: 1, options: 0 }.encode();
        let last = bytes.len() - 1;
        bytes[last] = 0x20;
        assert_eq!(ToShell::decode(&bytes), Err(DecodeError::BadValue));
        let mut bytes = ToShell::NotifyPresentDelegate { enabled: true }.encode();
        let last = bytes.len() - 1;
        bytes[last] = 2;
        assert_eq!(ToShell::decode(&bytes), Err(DecodeError::BadValue));
        assert_eq!(NOTIF_PRESENT_MASK, 27);
    }

    #[test]
    fn notification_authorization_round_trips() {
        for options in [0u8, 1, 2, 4, 16, 64, NOTIF_AUTH_MASK] {
            shell_rt(ToShell::NotifyAuthorize { request: 21, options });
        }
        app_rt(ToApp::NotificationAuthorized { request: 21, granted: true });
        app_rt(ToApp::NotificationAuthorized { request: 22, granted: false });
    }

    #[test]
    fn notification_authorization_rejects_unknown_option_bits() {
        assert_eq!(ToShell::NotifyAuthorize { request: 1, options: NOTIF_AUTH_MASK }.encode().last(), Some(&NOTIF_AUTH_MASK));
        let mut bytes = ToShell::NotifyAuthorize { request: 1, options: 2 }.encode();
        let last = bytes.len() - 1;
        bytes[last] = 0x08;
        assert_eq!(ToShell::decode(&bytes), Err(DecodeError::BadValue));
        let mut bytes = ToApp::NotificationAuthorized { request: 1, granted: true }.encode();
        let last = bytes.len() - 1;
        bytes[last] = 2;
        assert_eq!(ToApp::decode(&bytes), Err(DecodeError::BadValue));
    }

    #[test]
    fn notification_schedule_round_trips_both_triggers() {
        let req = NotifRequest { id: "s1".into(), title: "Standup".into(), body: "9:15".into(), ..Default::default() };
        shell_rt(ToShell::NotifySchedule { request: 7, req: req.clone(), trigger: NotifTrigger::Calendar { hour: 9, minute: 15, weekdays: 0b0111_1100, repeats: true } });
        shell_rt(ToShell::NotifySchedule { request: 8, req: req.clone(), trigger: NotifTrigger::Calendar { hour: 23, minute: 59, weekdays: 0, repeats: false } });
        shell_rt(ToShell::NotifySchedule { request: 9, req: req.clone(), trigger: NotifTrigger::Interval { seconds: 90, repeats: false } });
        shell_rt(ToShell::NotifySchedule { request: 10, req, trigger: NotifTrigger::Interval { seconds: 60, repeats: true } });
    }

    #[test]
    fn notification_schedule_rejects_bad_triggers() {
        let req = NotifRequest { id: "s1".into(), ..Default::default() };
        let bad = |trigger: NotifTrigger| ToShell::NotifySchedule { request: 1, req: req.clone(), trigger }.encode();
        // the trigger is the end of the packet: a calendar ends in hour, minute, weekdays, repeats
        let mut hour = bad(NotifTrigger::Calendar { hour: 9, minute: 15, weekdays: 2, repeats: false });
        let n = hour.len();
        hour[n - 4] = 24;
        assert_eq!(ToShell::decode(&hour), Err(DecodeError::BadValue));
        let mut weekday = bad(NotifTrigger::Calendar { hour: 9, minute: 15, weekdays: 2, repeats: false });
        let n = weekday.len();
        weekday[n - 2] = 0x80;
        assert_eq!(ToShell::decode(&weekday), Err(DecodeError::BadValue));
        assert_eq!(ToShell::decode(&bad(NotifTrigger::Interval { seconds: 0, repeats: false })), Err(DecodeError::BadValue));
        assert_eq!(ToShell::decode(&bad(NotifTrigger::Interval { seconds: 30, repeats: true })), Err(DecodeError::BadValue));
        assert_eq!(ToShell::decode(&bad(NotifTrigger::Interval { seconds: MAX_NOTIF_INTERVAL_SECS + 1, repeats: false })), Err(DecodeError::BadValue));
    }

    #[test]
    fn notification_limits_are_enforced() {
        // encode cuts, so an oversized request still decodes
        let big = ToShell::NotifyRequest { request: 1, req: NotifRequest { id: "i".repeat(1000), body: "b".repeat(100_000), ..Default::default() } };
        let bytes = big.encode();
        assert!(bytes.len() < crate::MAX_PACKET);
        let Ok(ToShell::NotifyRequest { req, .. }) = ToShell::decode(&bytes) else { panic!("decode") };
        assert_eq!(req.id.len(), MAX_NOTIF_ID);
        // a hand-built oversized count is refused
        let mut p = Vec::new();
        u32_(&mut p, tag::NOTIFY_CATEGORIES);
        u32_(&mut p, 17);
        assert_eq!(ToShell::decode(&p), Err(DecodeError::BadValue));
        // unknown flag bits on an action
        let mut ok = ToShell::NotifyCategories(vec![NotifCategory { options: 0, id: "c".into(), actions: vec![NotifAction::button("a", "A")] }]).encode();
        *ok.last_mut().unwrap() = 8;
        assert!(ToShell::decode(&ok).is_err());
        assert_eq!(ToApp::decode(&[51, 0, 0, 0, 1, 0, 0, 0, 9]), Err(DecodeError::BadValue));
    }

    #[test]
    fn share_round_trips_and_caps_items() {
        let items = vec![Payload::Text("hello".into()), Payload::Url("https://example.com".into()), Payload::File { grant: [7; 16], name: "a.png".into() }];
        shell_rt(ToShell::Share { request: 2, items, anchor: UiRect { x: 1.0, y: 2.0, w: 30.0, h: 20.0 } });
        app_rt(ToApp::ShareDone { request: 2, target: "is.olibuijr.notes.desktop".into() });
        shell_rt(ToShell::DistPost { name: "n".into(), user_info: vec![("k".into(), "v ü".into())] });
        app_rt(ToApp::DistNotify { name: "n".into(), user_info: vec![] });
        shell_rt(ToShell::ServiceCall { request: 7, service: "dock".into(), method: "badge".into(), body: r#"{"text":"3"}"#.into() });
        app_rt(ToApp::ServiceReply { request: 7, ok: true, body: "null".into() });
        app_rt(ToApp::ServiceReply { request: 8, ok: false, body: "no such method".into() });
        app_rt(ToApp::ServiceEvent { service: "now-playing".into(), event: "command".into(), body: r#"{"cmd":"pause"}"#.into() });
        app_rt(ToApp::ShareDone { request: 3, target: String::new() });
        let many = ToShell::Share { request: 1, items: (0..20).map(|i| Payload::Text(i.to_string())).collect(), anchor: UiRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 } };
        let Ok(ToShell::Share { items, .. }) = ToShell::decode(&many.encode()) else { panic!("decode") };
        assert_eq!(items.len(), MAX_ITEMS);
        let nan = ToShell::Share { request: 1, items: vec![], anchor: UiRect { x: f32::NAN, y: 0.0, w: 1.0, h: 1.0 } };
        assert_eq!(ToShell::decode(&nan.encode()), Err(DecodeError::BadValue));
    }

    #[test]
    fn drag_messages_round_trip() {
        let items = vec![
            DragItem::new(Payload::File { grant: [3; 16], name: "photo.png".into() }),
            DragItem { types: vec!["public.utf8-plain-text".into(), "public.text".into()], payload: Payload::Text("x".into()) },
        ];
        assert_eq!(items[0].types, vec!["public.png".to_string()]);
        let preview = DragPreview { w: 2, h: 2, rgba: vec![9; 16] };
        shell_rt(ToShell::DragBegin { session: 0x8000_0001, allowed: dragop::COPY | dragop::MOVE, items: items.clone(), preview });
        shell_rt(ToShell::DragRegister { types: vec!["public.png".into(), "public.text".into()] });
        shell_rt(ToShell::DragReply { session: 1, op: dragop::COPY });
        shell_rt(ToShell::DragReply { session: 1, op: 0 });
        shell_rt(ToShell::DragPerformed { session: 1, accepted: true });
        shell_rt(ToShell::DragCancel { session: 1 });
        for phase in [DragPhase::Entered, DragPhase::Updated, DragPhase::Exited, DragPhase::Perform] {
            app_rt(ToApp::DragDest { session: 1, phase, x: 10.5, y: 4.0, allowed: dragop::ALL, items: items.clone() });
        }
        app_rt(ToApp::DragEnded { session: 1, op: dragop::LINK });
    }

    #[test]
    fn drag_rejects_bad_values() {
        // an invalid preview is dropped on encode rather than sent
        let bad = ToShell::DragBegin { session: 1, allowed: 1, items: vec![], preview: DragPreview { w: 64, h: 64, rgba: vec![0; 10] } };
        let Ok(ToShell::DragBegin { preview, .. }) = ToShell::decode(&bad.encode()) else { panic!("decode") };
        assert_eq!((preview.w, preview.h, preview.rgba.len()), (0, 0, 0));
        // two operation bits in a reply, unknown bits in a mask, a non-finite point
        let mut two = ToShell::DragReply { session: 1, op: 0 }.encode();
        *two.last_mut().unwrap() = dragop::COPY | dragop::MOVE;
        assert_eq!(ToShell::decode(&two), Err(DecodeError::BadValue));
        let mut mask = ToApp::DragEnded { session: 1, op: 0 }.encode();
        *mask.last_mut().unwrap() = 8;
        assert_eq!(ToApp::decode(&mask), Err(DecodeError::BadValue));
        let nan = ToApp::DragDest { session: 1, phase: DragPhase::Updated, x: f32::NAN, y: 0.0, allowed: 1, items: vec![] };
        assert_eq!(ToApp::decode(&nan.encode()), Err(DecodeError::BadValue));
        // a packet with every field at its cap still fits
        let full = ToApp::DragDest {
            session: 1,
            phase: DragPhase::Perform,
            x: 0.0,
            y: 0.0,
            allowed: 7,
            items: (0..MAX_ITEMS).map(|_| DragItem { types: (0..MAX_ITEM_TYPES).map(|_| "u".repeat(MAX_UTI)).collect(), payload: Payload::Text("t".repeat(MAX_PAYLOAD_TEXT)) }).collect(),
        };
        assert!(full.encode().len() < crate::MAX_PACKET);
        let full_begin = ToShell::DragBegin { session: 1, allowed: 7, items: (0..MAX_ITEMS).map(|_| DragItem { types: (0..MAX_ITEM_TYPES).map(|_| "u".repeat(MAX_UTI)).collect(), payload: Payload::Text("t".repeat(MAX_PAYLOAD_TEXT)) }).collect(), preview: DragPreview { w: 32, h: 32, rgba: vec![1; 4096] } };
        assert!(full_begin.encode().len() < crate::MAX_PACKET);
    }
}
