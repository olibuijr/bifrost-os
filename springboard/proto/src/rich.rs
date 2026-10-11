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
}

impl NotifAction {
    pub fn button(id: &str, title: &str) -> Self {
        NotifAction { id: id.into(), title: title.into(), destructive: false, foreground: false, text_input: None }
    }
}

/// `UNNotificationCategory`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotifCategory {
    pub id: String,
    pub actions: Vec<NotifAction>,
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
}

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

pub const MAX_DIST_INFO: usize = 32;
pub const MAX_DIST_VALUE: usize = 1024;

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
        let m = c.actions.len().min(MAX_NOTIF_ACTIONS);
        u32_(b, m as u32);
        for a in &c.actions[..m] {
            put(b, &a.id, MAX_NOTIF_ID);
            put(b, &a.title, MAX_NOTIF_TEXT);
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
        let m = count(r, MAX_NOTIF_ACTIONS)?;
        let mut actions = Vec::with_capacity(m);
        for _ in 0..m {
            let aid = get(r, MAX_NOTIF_ID)?;
            let title = get(r, MAX_NOTIF_TEXT)?;
            let f = r.u8()?;
            if f > 7 {
                return Err(DecodeError::BadValue);
            }
            let text_input = if f & 4 != 0 { Some(TextInput { button_title: get(r, MAX_NOTIF_TEXT)?, placeholder: get(r, MAX_NOTIF_TEXT)? }) } else { None };
            actions.push(NotifAction { id: aid, title, destructive: f & 1 != 0, foreground: f & 2 != 0, text_input });
        }
        out.push(NotifCategory { id, actions });
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
    Ok(NotifRequest { id, title, subtitle, body, thread, category, delay_ms, badge, repeats })
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
    pub const SHARE_DONE: u32 = 53;
    pub const DRAG_DEST: u32 = 54;
    pub const DRAG_ENDED: u32 = 55;

    pub const NOTIFY_CATEGORIES: u32 = 0x140;
    pub const NOTIFY_REQUEST: u32 = 0x141;
    pub const NOTIFY_REMOVE: u32 = 0x142;
    pub const NOTIFY_QUERY: u32 = 0x143;
    pub const SHARE: u32 = 0x144;
    pub const DRAG_BEGIN: u32 = 0x145;
    pub const DRAG_REGISTER: u32 = 0x146;
    pub const DRAG_REPLY: u32 = 0x147;
    pub const DRAG_PERFORMED: u32 = 0x148;
    pub const DRAG_CANCEL: u32 = 0x149;
    pub const DIST_POST: u32 = 0x1d0;
    pub const DIST_NOTIFY: u32 = 0x1d1;
}

/// Encodes the variants of `ToApp` that belong to this module (anything else writes nothing).
pub(crate) fn encode_app(m: &ToApp, b: &mut Vec<u8>) {
    match m {
        ToApp::NotificationAction { id, action, text, thread, category } => {
            u32_(b, tag::NOTIFICATION_ACTION);
            put(b, id, MAX_NOTIF_ID);
            put(b, action, MAX_NOTIF_ID);
            put(b, text, MAX_NOTIF_TEXT * 4);
            put(b, thread, MAX_NOTIF_ID);
            put(b, category, MAX_NOTIF_ID);
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
        ToApp::ShareDone { request, target } => {
            u32_(b, tag::SHARE_DONE);
            u32_(b, *request);
            put(b, target, MAX_NOTIF_ID);
        }
        ToApp::DistNotify { name, user_info } => {
            u32_(b, tag::DIST_NOTIFY);
            put_dist(b, name, user_info);
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
        },
        tag::NOTIFICATION_STATUS => ToApp::NotificationStatus { request: r.u32()?, status: NotifStatus::from_wire(r.u8()?).ok_or(DecodeError::BadValue)? },
        tag::NOTIFICATION_LIST => ToApp::NotificationList { request: r.u32()?, delivered: flag(r)?, ids: get_ids(r)? },
        tag::DIST_NOTIFY => {
            let (name, user_info) = get_dist(r)?;
            ToApp::DistNotify { name, user_info }
        }
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
        ToShell::DistPost { name, user_info } => {
            u32_(b, tag::DIST_POST);
            put_dist(b, name, user_info);
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
        tag::DIST_POST => {
            let (name, user_info) = get_dist(r)?;
            ToShell::DistPost { name, user_info }
        }
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
            id: "MESSAGE".into(),
            actions: vec![
                NotifAction::button("open", "Open"),
                NotifAction { destructive: true, ..NotifAction::button("delete", "Delete") },
                NotifAction { id: "reply".into(), title: "Reply".into(), destructive: false, foreground: true, text_input: Some(TextInput { button_title: "Send".into(), placeholder: "Message".into() }) },
            ],
        }
    }

    #[test]
    fn notification_messages_round_trip() {
        shell_rt(ToShell::NotifyCategories(vec![category(), NotifCategory { id: "EMPTY".into(), actions: vec![] }]));
        shell_rt(ToShell::NotifyRequest {
            request: 4,
            req: NotifRequest { id: "n1".into(), title: "Hi".into(), subtitle: "sub".into(), body: "b ü".into(), thread: "t".into(), category: "MESSAGE".into(), delay_ms: 1500, badge: Some(3), repeats: true },
        });
        shell_rt(ToShell::NotifyRequest { request: 5, req: NotifRequest { id: "n2".into(), ..Default::default() } });
        shell_rt(ToShell::NotifyRemove { delivered: true, ids: vec!["a".into(), "b".into()] });
        shell_rt(ToShell::NotifyRemove { delivered: false, ids: vec![] });
        shell_rt(ToShell::NotifyQuery { request: 9, delivered: true });
        app_rt(ToApp::NotificationAction { id: "n1".into(), action: "reply".into(), text: "ok".into(), thread: "t".into(), category: "MESSAGE".into() });
        app_rt(ToApp::NotificationStatus { request: 4, status: NotifStatus::Param });
        app_rt(ToApp::NotificationList { request: 9, delivered: false, ids: vec!["x".into()] });
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
        let mut ok = ToShell::NotifyCategories(vec![NotifCategory { id: "c".into(), actions: vec![NotifAction::button("a", "A")] }]).encode();
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
